//! SMB baseline.
//!
//! This is the measurement the Phase 0 gate is defined against: **does the
//! custom protocol actually beat Windows file sharing on this link?**
//!
//! It matters because the honest answer might be no. SMB3 already pipelines,
//! compounds requests, leases, and uses large I/O sizes. On a LAN, a tuned SMB
//! share is genuinely fast, and it is already installed. If Basalt cannot match
//! it, the case for the custom protocol collapses down to the UI — which is a
//! real reason to build, but a very different project from the one planned.
//!
//! The comparison is deliberately apples-to-apples: the same corpus, the same
//! file counts, the same measurement code as [`crate::net`]. The only thing
//! that changes is the transport. Anything else would be measuring the harness
//! rather than the protocols.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::corpus::Flavour;
use crate::stats::{Measurement, Suite, fmt_bytes, fmt_duration_ms};

pub struct SmbConfig {
    /// UNC path to the share, e.g. `\\LAPTOP\bench-corpus`.
    pub share: PathBuf,
    pub runs: usize,
    /// How many small files to read. Match the `net` run for a fair comparison.
    pub small_files: usize,
    /// Bytes to read from the large file.
    pub large_bytes: u64,
}

pub fn run(config: &SmbConfig) -> Result<Vec<Suite>> {
    anyhow::ensure!(
        config.share.is_dir(),
        "cannot reach {}. Share the corpus directory on the laptop and check \
         the UNC path — you may need to open it once in Explorer to authenticate.",
        config.share.display()
    );

    println!("\nSMB baseline against {}", config.share.display());
    println!(
        "  NOTE: these numbers come from the same corpus and the same code as\n\
         \x20 the `net` run, so they are directly comparable."
    );

    Ok(vec![
        large_file(config)?,
        small_files(config)?,
        listing(config)?,
    ])
}

/// Sequential read of one large file — the streaming and bulk-copy case.
fn large_file(config: &SmbConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "smb-large-file",
        "reading one large file over SMB — compare against net raw throughput",
    );

    // Binary, so SMB compression (if the server negotiates it) cannot flatter
    // the baseline on data our own protocol would also decline to compress.
    let path = config
        .share
        .join("large")
        .join(format!("binary.{}", Flavour::Binary.extension()));

    let available = std::fs::metadata(&path)
        .with_context(|| {
            format!(
                "{} not found — is the corpus generated on the server?",
                path.display()
            )
        })?
        .len();
    let to_read = config.large_bytes.min(available);

    println!("\nlarge file ({} per run)", fmt_bytes(to_read));

    let mut m = Measurement::new("sequential read", to_read);
    for _ in 0..config.runs {
        let start = Instant::now();
        let read = read_file(&path, to_read)?;
        m.record(start.elapsed());
        debug_assert_eq!(read, to_read);
    }
    suite.push(m);

    Ok(suite)
}

/// Many small files — the case the batch protocol exists for.
fn small_files(config: &SmbConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "smb-small-files",
        "reading many small files over SMB — compare against the batched request",
    );

    let paths = discover_small_files(&config.share, config.small_files)?;
    anyhow::ensure!(
        !paths.is_empty(),
        "no small files under {}. Generate the corpus on the server first.",
        config.share.display()
    );

    let total_bytes: u64 = paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    println!(
        "\nsmall files ({} files, {})",
        paths.len(),
        fmt_bytes(total_bytes)
    );

    // Sequential is what a naive copy does, and what `net`'s per-file arm does.
    let mut sequential =
        Measurement::new("sequential, one at a time", total_bytes).with_items(paths.len() as u64);
    for _ in 0..config.runs.min(3) {
        let start = Instant::now();
        for p in &paths {
            read_file(p, u64::MAX)?;
        }
        sequential.record(start.elapsed());
    }
    suite.push(sequential);

    // Parallel is what Explorer and robocopy /MT effectively do, and is the
    // fairer comparison against a batched request. Leaving it out would let
    // Basalt win against a strawman.
    for threads in [4usize, 8, 16] {
        let mut m = Measurement::new(format!("{threads:>2} threads in parallel"), total_bytes)
            .with_items(paths.len() as u64);
        for _ in 0..config.runs.min(3) {
            let start = Instant::now();
            read_parallel(&paths, threads)?;
            m.record(start.elapsed());
        }
        suite.push(m);
    }

    if let Some(best) = suite
        .measurements
        .iter()
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()))
    {
        println!(
            "  -> SMB's best small-file result: {} at {:.1} MB/s ({})",
            best.label.trim(),
            best.throughput_mbs(),
            fmt_duration_ms(best.median_ms())
        );
        println!("     This is the number the batched request has to beat.");
    }

    Ok(suite)
}

/// Directory enumeration over SMB.
fn listing(config: &SmbConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "smb-listing",
        "enumerating a large directory over SMB — compare against the mirrored index",
    );

    let dir = config.share.join("wide");
    if !dir.is_dir() {
        println!("\nno wide/ directory on the share; skipping listing");
        return Ok(suite);
    }

    let count = std::fs::read_dir(&dir)?.count();
    println!("\ndirectory listing ({count} entries)");

    let mut names = Measurement::new("names only", 0).with_items(count as u64);
    for _ in 0..config.runs.min(3) {
        let start = Instant::now();
        let n = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).count();
        names.record(start.elapsed());
        debug_assert_eq!(n, count);
    }
    suite.push(names);

    let mut with_meta = Measurement::new("names + metadata", 0).with_items(count as u64);
    for _ in 0..config.runs.min(3) {
        let start = Instant::now();
        for entry in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
            let _ = entry.metadata();
        }
        with_meta.record(start.elapsed());
    }
    with_meta.notes.push(
        "every visit to this folder costs this much over SMB; the mirrored \
         index turns it into a local query"
            .into(),
    );
    suite.push(with_meta);

    Ok(suite)
}

/// Reads up to `limit` bytes and discards them.
fn read_file(path: &Path, limit: u64) -> Result<u64> {
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    while total < limit {
        // Clamp to the limit rather than reading a whole block past it.
        // Overshooting would move more bytes than the measurement counts,
        // which silently understates throughput.
        let want = buf.len().min((limit - total) as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}

fn read_parallel(paths: &[PathBuf], threads: usize) -> Result<u64> {
    let cursor = AtomicUsize::new(0);
    let total = AtomicUsize::new(0);
    let failure: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(index) else { return };
                    match read_file(path, u64::MAX) {
                        Ok(n) => {
                            total.fetch_add(n as usize, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let mut slot = failure.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(e.to_string());
                            }
                            return;
                        }
                    }
                }
            });
        }
    });

    if let Some(e) = failure.into_inner().unwrap() {
        anyhow::bail!("parallel SMB read failed: {e}");
    }
    Ok(total.load(Ordering::Relaxed) as u64)
}

/// Walks `small/` on the share, taking at most `limit` files.
fn discover_small_files(share: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    let small = share.join("small");
    if !small.is_dir() {
        return Ok(Vec::new());
    }

    let mut shards: Vec<PathBuf> = std::fs::read_dir(&small)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    shards.sort();

    let mut out = Vec::new();
    for shard in shards {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&shard)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        files.sort();
        for f in files {
            out.push(f);
            if out.len() >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Side-by-side verdict against the `net` results, printed after both have run.
pub fn print_comparison_hint() {
    println!(
        "\nComparing against the custom protocol:\n\
         \n\
           Open docs/benchmarks.md and put these side by side:\n\
         \n\
           · smb-large-file  vs  raw-throughput   -> does the protocol match SMB\n\
             on bulk transfer? It should be close; both are moving bytes over the\n\
             same radio.\n\
           · smb-small-files vs  small-files      -> the one that matters. Compare\n\
             SMB's *best* parallel result against the batched request, not against\n\
             the sequential arm. Beating a strawman proves nothing.\n\
           · smb-listing     vs  disk-listing     -> how much the mirrored index\n\
             is worth per folder visit.\n\
         \n\
           The gate: match or beat SMB on both workloads. If the batched request\n\
           does not clearly win on small files, the custom protocol is not\n\
           earning its keep and the plan needs revisiting.\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, CorpusSpec};

    /// A local directory shaped like the share, so the traversal and counting
    /// logic can be tested without an actual SMB server.
    fn local_corpus(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("basalt-smb-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Corpus::open(&root)
            .generate(
                CorpusSpec {
                    large_file_bytes: 128 * 1024,
                    small_file_count: 30,
                    small_file_bytes: 1024,
                    wide_entry_count: 25,
                    seed: 9,
                },
                true,
            )
            .unwrap();
        root
    }

    #[test]
    fn discovery_finds_small_files_and_honours_the_limit() {
        let root = local_corpus("discover");
        let all = discover_small_files(&root, 1000).unwrap();
        assert_eq!(all.len(), 30);
        assert!(all.iter().all(|p| p.is_file()));

        let capped = discover_small_files(&root, 10).unwrap();
        assert_eq!(capped.len(), 10);

        // Deterministic order, so repeat runs read the same set.
        let again = discover_small_files(&root, 10).unwrap();
        assert_eq!(capped, again);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_on_a_share_without_a_corpus_returns_nothing() {
        let empty = std::env::temp_dir().join("basalt-smb-empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(discover_small_files(&empty, 100).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn read_file_respects_its_limit() {
        let path = std::env::temp_dir().join(format!("basalt-smb-read-{}.bin", std::process::id()));
        std::fs::write(&path, vec![0u8; 5_000_000]).unwrap();

        assert_eq!(read_file(&path, u64::MAX).unwrap(), 5_000_000);
        // The limit is enforced per 1 MiB block, so it stops at the first block
        // boundary at or past the limit rather than exactly on it.
        let limited = read_file(&path, 2_000_000).unwrap();
        assert!(
            (2_000_000..=3_000_000).contains(&limited),
            "limit of 2 MB read {limited} bytes"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parallel_read_covers_every_file_once() {
        let root = local_corpus("parallel");
        let paths = discover_small_files(&root, 1000).unwrap();
        let expected: u64 = paths
            .iter()
            .map(|p| std::fs::metadata(p).unwrap().len())
            .sum();

        for threads in [1usize, 4, 16] {
            assert_eq!(
                read_parallel(&paths, threads).unwrap(),
                expected,
                "{threads} threads did not read every file exactly once"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_refuses_an_unreachable_share() {
        let config = SmbConfig {
            share: PathBuf::from(r"\\definitely-not-a-host\nope"),
            runs: 1,
            small_files: 10,
            large_bytes: 1024,
        };
        let err = run(&config).expect_err("should refuse");
        assert!(
            err.to_string().contains("cannot reach"),
            "error should explain the problem, got: {err}"
        );
    }

    #[test]
    fn the_baseline_runs_against_a_local_directory() {
        // Not SMB, but it exercises every code path end to end. The real run
        // points --share at a UNC path.
        let root = local_corpus("full");
        let config = SmbConfig {
            share: root.clone(),
            runs: 1,
            small_files: 20,
            large_bytes: 64 * 1024,
        };
        let suites = run(&config).expect("baseline should run");
        assert_eq!(suites.len(), 3);
        assert!(suites.iter().all(|s| !s.measurements.is_empty()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
