//! Disk benchmarks.
//!
//! Two questions, one of which decides a core piece of the architecture.
//!
//! **How fast is sequential read?** Sets the ceiling for streaming a movie. The
//! plan expects 100–200 MB/s from a desktop HDD — comfortably above the ~30 MB/s
//! link, which is why the disk is not the bottleneck for large files.
//!
//! **How badly does concurrency hurt?** This is the one that matters. A
//! spinning platter has one head. Eight threads reading eight different files
//! drag it between eight places, and throughput can fall *below* the
//! single-threaded number. If that collapse is real, the host needs an I/O
//! scheduler that keeps disk concurrency low (2–4) while network concurrency
//! stays high, with a RAM buffer decoupling them. If it is not real, that whole
//! component can be dropped.
//!
//! Every read here bypasses the Windows file cache. Without that, the second
//! run of a 200 MB corpus on a 16 GB laptop is served entirely from RAM and the
//! curve measures memcpy rather than the drive.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::corpus::{Corpus, Flavour, Rng};
use crate::stats::{Measurement, Suite, fmt_bytes};
use crate::winio::{AlignedBuffer, DriveKind, UnbufferedFile, detect_drive_kind};

/// Thread counts to sweep for the concurrency curve.
const THREAD_COUNTS: [usize; 5] = [1, 2, 4, 8, 16];

/// Block sizes for sequential reads.
const BLOCK_SIZES: [usize; 5] = [
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
];

pub struct DiskConfig {
    pub root: PathBuf,
    pub runs: usize,
    /// Bytes to read per sequential measurement. Capped so a full sweep does
    /// not take an hour on a spinning disk.
    pub sequential_bytes: u64,
    /// How many small files to use for the concurrency curve.
    pub small_files: usize,
}

impl Default for DiskConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("bench-corpus"),
            runs: 3,
            sequential_bytes: 512 * 1024 * 1024,
            small_files: 2000,
        }
    }
}

pub fn run(config: &DiskConfig) -> Result<Vec<Suite>> {
    let corpus = Corpus::open(&config.root);
    anyhow::ensure!(
        corpus.is_generated(),
        "no corpus at {}. run: basalt-bench gen-corpus --root {}",
        config.root.display(),
        config.root.display()
    );

    let kind = detect_drive_kind(&config.root);
    println!("\ndrive under test: {}", describe(kind));
    if kind == DriveKind::Unknown {
        println!(
            "  (Windows would not answer the seek-penalty query — common on USB\n\
                bridges. The concurrency curve below settles it either way.)"
        );
    }

    Ok(vec![
        sequential(&corpus, config)?,
        concurrency_curve(&corpus, config, kind)?,
        access_order(&corpus, config)?,
        listing(&corpus, config)?,
    ])
}

fn describe(kind: DriveKind) -> &'static str {
    match kind {
        DriveKind::Spinning => "spinning (has a seek penalty)",
        DriveKind::Solid => "solid state (no seek penalty)",
        DriveKind::Unknown => "unknown",
    }
}

/// Sequential read at a range of block sizes, cache bypassed.
fn sequential(corpus: &Corpus, config: &DiskConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "disk-sequential",
        "unbuffered sequential read throughput by block size",
    );
    println!(
        "\nsequential read ({} per run, cache bypassed)",
        fmt_bytes(config.sequential_bytes)
    );

    let path = corpus.large_file(Flavour::Binary);
    let available = std::fs::metadata(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    let to_read = config.sequential_bytes.min(available);

    for block in BLOCK_SIZES {
        let mut m = Measurement::new(
            format!("unbuffered, {} blocks", fmt_bytes(block as u64)),
            to_read,
        );
        for _ in 0..config.runs {
            let start = Instant::now();
            let read = read_sequential_unbuffered(&path, block, to_read)?;
            m.record(start.elapsed());
            // Unbuffered reads cannot stop mid-sector, so a large block size
            // may overshoot the limit slightly. Report what was actually read
            // rather than what was asked for.
            m.bytes = read;
        }
        suite.push(m);
    }

    // Buffered, for contrast. This number is the page cache, not the drive —
    // it is here so the gap between the two is visible rather than a footnote.
    let mut cached = Measurement::new("buffered (page cache, for contrast)", to_read);
    for _ in 0..config.runs {
        let start = Instant::now();
        read_sequential_buffered(&path, to_read)?;
        cached.record(start.elapsed());
    }
    cached.notes.push(
        "served largely from RAM; shown only to make the cache effect visible. \
         Do not quote this as a disk speed."
            .into(),
    );
    suite.push(cached);

    if let Some(best) = suite
        .measurements
        .iter()
        .filter(|m| m.label.starts_with("unbuffered"))
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()))
    {
        println!(
            "  -> best block size: {} at {:.1} MB/s",
            best.label,
            best.throughput_mbs()
        );
    }
    Ok(suite)
}

fn read_sequential_unbuffered(path: &Path, block: usize, limit: u64) -> Result<u64> {
    let file = UnbufferedFile::open(path)?;
    let mut buf = AlignedBuffer::new(block)?;
    let mut total = 0u64;
    while total < limit {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}

fn read_sequential_buffered(path: &Path, limit: u64) -> Result<u64> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    while total < limit {
        // Clamp, so the bytes moved match the bytes the measurement counts.
        let want = buf.len().min((limit - total) as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}

/// **The measurement that decides whether the I/O scheduler gets built.**
///
/// Reads the same set of small files with an increasing number of threads. On
/// flash, throughput should climb with depth. On a platter, it should peak
/// early and then fall as the head thrashes.
fn concurrency_curve(corpus: &Corpus, config: &DiskConfig, kind: DriveKind) -> Result<Suite> {
    let mut suite = Suite::new(
        "disk-concurrency",
        "small-file read throughput vs thread count — decides the I/O scheduler's queue depth",
    );

    let mut paths = corpus.small_file_paths()?;
    paths.truncate(config.small_files);
    anyhow::ensure!(!paths.is_empty(), "the corpus has no small files");

    let absolute: Vec<PathBuf> = paths.iter().map(|p| corpus.root.join(p)).collect();
    let total_bytes: u64 = absolute
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    println!(
        "\nconcurrency curve ({} files, {}, cache bypassed)",
        absolute.len(),
        fmt_bytes(total_bytes)
    );

    for threads in THREAD_COUNTS {
        let mut m = Measurement::new(
            format!(
                "{threads:>2} thread{}",
                if threads == 1 { " " } else { "s" }
            ),
            total_bytes,
        )
        .with_items(absolute.len() as u64);

        for _ in 0..config.runs {
            let start = Instant::now();
            read_parallel(&absolute, threads)?;
            m.record(start.elapsed());
        }
        suite.push(m);
    }

    interpret_curve(&suite, kind);
    Ok(suite)
}

/// Reads every path using `threads` workers pulling from a shared cursor.
fn read_parallel(paths: &[PathBuf], threads: usize) -> Result<u64> {
    let cursor = AtomicUsize::new(0);
    let total = AtomicUsize::new(0);
    let failure: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                // One buffer per worker, reused across files. 1 MiB covers any
                // small file in a single read.
                let mut buf = match AlignedBuffer::new(1024 * 1024) {
                    Ok(b) => b,
                    Err(e) => {
                        *failure.lock().unwrap() = Some(e.to_string());
                        return;
                    }
                };
                loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(index) else { return };
                    match UnbufferedFile::open(path).and_then(|f| f.read_all(&mut buf)) {
                        Ok(n) => {
                            total.fetch_add(n as usize, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let mut slot = failure.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(format!("{}: {e}", path.display()));
                            }
                            return;
                        }
                    }
                }
            });
        }
    });

    if let Some(e) = failure.into_inner().unwrap() {
        anyhow::bail!("parallel read failed: {e}");
    }
    Ok(total.load(Ordering::Relaxed) as u64)
}

/// What the concurrency curve says about this drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveVerdict {
    /// Throughput falls as threads are added. The host needs an I/O scheduler
    /// that caps disk concurrency.
    Thrashing,
    /// Adding threads changes almost nothing; the disk is not the constraint.
    NoBenefit,
    /// Throughput rises and holds. A plain bounded queue is sufficient.
    ScalesCleanly,
}

/// Classifies a concurrency curve given throughputs in ascending thread order.
///
/// Kept separate from the printing so the decision itself can be tested. The
/// distinction that matters, and which an earlier version got backwards:
/// thrashing is throughput *falling as threads increase*, measured peak against
/// the highest thread count. The global minimum is nearly always the 1-thread
/// baseline, which is merely un-parallelised — comparing against it reports
/// thrashing on a drive that scales perfectly.
pub fn classify_curve(throughputs: &[f64]) -> CurveVerdict {
    let Some(&first) = throughputs.first() else {
        return CurveVerdict::NoBenefit;
    };
    let Some(&last) = throughputs.last() else {
        return CurveVerdict::NoBenefit;
    };
    let peak = throughputs.iter().copied().fold(f64::MIN, f64::max);

    // No usable signal: a single sample, or every run measuring nothing. Report
    // "no benefit" rather than inventing a collapse out of zeroes.
    if throughputs.len() < 2 || peak <= 0.0 {
        return CurveVerdict::NoBenefit;
    }

    if last / peak < 0.7 {
        CurveVerdict::Thrashing
    } else if peak / first.max(0.001) < 1.3 {
        CurveVerdict::NoBenefit
    } else {
        CurveVerdict::ScalesCleanly
    }
}

fn interpret_curve(suite: &Suite, kind: DriveKind) {
    let Some(single) = suite.measurements.first() else {
        return;
    };
    let Some(best) = suite
        .measurements
        .iter()
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()))
    else {
        return;
    };
    // Thrashing means throughput *falling as threads are added*, so the
    // comparison has to be peak against the highest thread count. Comparing
    // against the global minimum would flag the 1-thread baseline — which is
    // simply un-parallelised, not thrashing — and reach the opposite
    // conclusion on a drive that scales perfectly well.
    let peak_index = suite
        .measurements
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.throughput_mbs().total_cmp(&b.throughput_mbs()))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let peak_threads = THREAD_COUNTS.get(peak_index).copied().unwrap_or(1);
    let max_threads = THREAD_COUNTS.last().copied().unwrap_or(1);

    let Some(tail) = suite.measurements.last() else {
        return;
    };

    let scaling = best.throughput_mbs() / single.throughput_mbs().max(0.001);
    let tail_ratio = tail.throughput_mbs() / best.throughput_mbs().max(0.001);

    println!(
        "  -> peak at {peak_threads} threads ({:.1} MB/s), {scaling:.2}x the single-threaded rate",
        best.throughput_mbs()
    );

    let throughputs: Vec<f64> = suite
        .measurements
        .iter()
        .map(|m| m.throughput_mbs())
        .collect();
    match classify_curve(&throughputs) {
        CurveVerdict::Thrashing => {
            println!(
                "  -> by {max_threads} threads throughput has fallen to {:.0}% of peak — \
                 the head is thrashing.",
                tail_ratio * 100.0
            );
            println!(
                "     This is the case for the I/O scheduler: cap disk concurrency\n\
                 \x20    near {peak_threads}, keep network concurrency high, decouple with RAM."
            );
        }
        CurveVerdict::NoBenefit => {
            println!(
                "  -> parallelism barely helps ({scaling:.2}x from 1 to {peak_threads} threads).\n\
                 \x20    Something other than the disk is the limit here."
            );
        }
        CurveVerdict::ScalesCleanly => {
            println!(
                "  -> scales cleanly: still {:.0}% of peak at {max_threads} threads, no collapse.",
                tail_ratio * 100.0
            );
            // Deliberately says nothing about read *ordering*. Queue depth and
            // ordering are separate questions measured by separate sections,
            // and on the real drive the answers differed: concurrency scaled
            // cleanly (no scheduler needed) while sorting was still worth
            // 2.71x. Claiming "no scheduler needed" here used to read as
            // "ordering does not matter", contradicting the very next section.
            println!(
                "     A plain bounded queue at {peak_threads} is enough here — no need to\n\
                 \x20    throttle or reorder for the sake of the head. See the access\n\
                 \x20    order section below for whether sorting still pays."
            );
        }
    }

    println!(
        "  -> Windows reported {}; suggested default queue depth {}",
        describe(kind),
        kind.suggested_queue_depth()
    );
}

/// Sorted versus shuffled access, which is the design question behind the batch
/// path sorting its manifest before reading.
fn access_order(corpus: &Corpus, config: &DiskConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "disk-access-order",
        "reading a file set in on-disk order vs shuffled — justifies sorting the batch manifest",
    );

    let mut paths = corpus.small_file_paths()?;
    paths.truncate(config.small_files);
    let sorted: Vec<PathBuf> = paths.iter().map(|p| corpus.root.join(p)).collect();

    let mut shuffled = sorted.clone();
    let mut rng = Rng::new(0x5EED);
    for i in (1..shuffled.len()).rev() {
        shuffled.swap(i, rng.below(i + 1));
    }

    let total_bytes: u64 = sorted
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    println!("\naccess order ({} files, 2 threads)", sorted.len());

    for (label, set) in [
        ("sorted (on-disk order)", &sorted),
        ("shuffled (random)", &shuffled),
    ] {
        let mut m = Measurement::new(label, total_bytes).with_items(set.len() as u64);
        for _ in 0..config.runs {
            let start = Instant::now();
            read_parallel(set, 2)?;
            m.record(start.elapsed());
        }
        suite.push(m);
    }

    if let (Some(a), Some(b)) = (suite.measurements.first(), suite.measurements.get(1)) {
        let gain = a.throughput_mbs() / b.throughput_mbs().max(0.001);
        println!(
            "  -> sorted is {gain:.2}x {} than shuffled",
            if gain >= 1.0 { "faster" } else { "slower" }
        );
        if gain > 1.15 {
            println!("     Sorting the batch manifest before reading is worth it.");
        } else {
            println!(
                "     Sorting buys little here — likely a freshly written corpus\n\
                 \x20    with good locality, or flash. Re-check on a fragmented drive."
            );
        }
    }

    Ok(suite)
}

/// Directory enumeration, which the metadata index exists to avoid repeating.
fn listing(corpus: &Corpus, config: &DiskConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "disk-listing",
        "enumerating a large directory — the cost the metadata index removes",
    );

    let dir = corpus.wide_dir();
    let count = std::fs::read_dir(&dir)?.count();
    println!("\ndirectory listing ({count} entries)");

    let mut names = Measurement::new("names only", 0).with_items(count as u64);
    for _ in 0..config.runs {
        let start = Instant::now();
        let n = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).count();
        names.record(start.elapsed());
        debug_assert_eq!(n, count);
    }
    suite.push(names);

    // With metadata, which is what a file browser actually needs to render a
    // row: size, modified time, and whether it is a directory.
    let mut with_meta = Measurement::new("names + metadata", 0).with_items(count as u64);
    for _ in 0..config.runs {
        let start = Instant::now();
        let mut bytes = 0u64;
        for entry in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
            if let Ok(meta) = entry.metadata() {
                bytes += meta.len();
            }
        }
        with_meta.record(start.elapsed());
        debug_assert!(bytes > 0);
    }
    with_meta.notes.push(
        "this is the per-visit cost a file browser pays without an index; the \
         mirrored SQLite index reduces it to a local query"
            .into(),
    );
    suite.push(with_meta);

    Ok(suite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::CorpusSpec;

    fn tiny_corpus(tag: &str) -> (PathBuf, Corpus) {
        let root =
            std::env::temp_dir().join(format!("basalt-disk-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let corpus = Corpus::open(&root);
        corpus
            .generate(
                CorpusSpec {
                    large_file_bytes: 256 * 1024,
                    small_file_count: 20,
                    small_file_bytes: 2048,
                    wide_entry_count: 30,
                    seed: 5,
                },
                true,
            )
            .unwrap();
        (root, corpus)
    }

    #[test]
    fn unbuffered_sequential_read_returns_the_whole_file() {
        let (root, corpus) = tiny_corpus("seq");
        let path = corpus.large_file(Flavour::Binary);
        let size = std::fs::metadata(&path).unwrap().len();

        for block in [4096usize, 64 * 1024, 1024 * 1024] {
            let read = read_sequential_unbuffered(&path, block, size).unwrap();
            assert_eq!(read, size, "block size {block} read {read} of {size} bytes");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parallel_read_covers_every_file_exactly_once() {
        let (root, corpus) = tiny_corpus("par");
        let paths: Vec<PathBuf> = corpus
            .small_file_paths()
            .unwrap()
            .iter()
            .map(|p| corpus.root.join(p))
            .collect();
        let expected: u64 = paths
            .iter()
            .map(|p| std::fs::metadata(p).unwrap().len())
            .sum();

        // Regardless of thread count, the shared cursor must hand out every
        // file once — no duplicates, no skips.
        for threads in [1usize, 2, 4, 8, 16, 32] {
            let total = read_parallel(&paths, threads).unwrap();
            assert_eq!(
                total, expected,
                "{threads} threads read {total} bytes, expected {expected}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parallel_read_reports_a_missing_file_rather_than_hanging() {
        let missing = vec![std::env::temp_dir().join("basalt-not-a-real-file.bin")];
        assert!(read_parallel(&missing, 4).is_err());
    }

    #[test]
    fn parallel_read_of_an_empty_set_succeeds() {
        assert_eq!(read_parallel(&[], 8).unwrap(), 0);
    }

    #[test]
    fn a_curve_that_keeps_climbing_is_not_thrashing() {
        // Regression guard. These are the real numbers from an NVMe run:
        // 1 -> 16 threads, rising then holding. An earlier version compared the
        // peak against the global minimum, saw the 1-thread baseline at 16% of
        // peak, and reported head thrashing on a drive with no head at all.
        let nvme = [89.5, 197.0, 381.9, 571.1, 541.0];
        assert_eq!(classify_curve(&nvme), CurveVerdict::ScalesCleanly);
    }

    #[test]
    fn a_curve_that_collapses_is_thrashing() {
        // What a spinning disk should look like: peaks early, then the head
        // starts servicing four readers at once and throughput falls away.
        let hdd = [45.0, 52.0, 38.0, 19.0, 11.0];
        assert_eq!(classify_curve(&hdd), CurveVerdict::Thrashing);
    }

    #[test]
    fn a_flat_curve_reports_no_benefit() {
        let flat = [100.0, 101.0, 99.0, 100.5, 100.0];
        assert_eq!(classify_curve(&flat), CurveVerdict::NoBenefit);
    }

    #[test]
    fn classify_curve_handles_degenerate_input() {
        assert_eq!(classify_curve(&[]), CurveVerdict::NoBenefit);
        assert_eq!(classify_curve(&[50.0]), CurveVerdict::NoBenefit);
        // All zeroes must not divide by zero or panic.
        assert_eq!(classify_curve(&[0.0, 0.0, 0.0]), CurveVerdict::NoBenefit);
    }

    #[test]
    fn a_curve_that_peaks_then_partially_recovers_is_still_judged_on_the_tail() {
        // Peak 80 at index 1, tail 30 -> 37% of peak. Thrashing, even though
        // the tail is above the 1-thread baseline.
        let curve = [25.0, 80.0, 60.0, 40.0, 30.0];
        assert_eq!(classify_curve(&curve), CurveVerdict::Thrashing);
    }

    #[test]
    fn run_refuses_to_proceed_without_a_corpus() {
        let config = DiskConfig {
            root: std::env::temp_dir().join("basalt-no-corpus-here"),
            ..Default::default()
        };
        let err = run(&config).expect_err("should refuse");
        assert!(
            err.to_string().contains("gen-corpus"),
            "the error should say how to fix it, got: {err}"
        );
    }

    #[test]
    fn buffered_and_unbuffered_reads_agree_on_length() {
        let (root, corpus) = tiny_corpus("agree");
        let path = corpus.large_file(Flavour::Prose);
        let size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(read_sequential_buffered(&path, size).unwrap(), size);
        assert_eq!(
            read_sequential_unbuffered(&path, 65536, size).unwrap(),
            size
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
