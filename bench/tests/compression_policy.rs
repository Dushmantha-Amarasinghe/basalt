//! Regression guards on the compression conclusion.
//!
//! Phase 0 concluded that wire compression is the highest-value lever in the
//! whole design: zstd-1 runs ~20x faster than a ~30 MB/s Wi-Fi link, so
//! compressible data effectively transfers several times faster.
//!
//! That conclusion depends on two things staying true, and both are easy to
//! break by accident later:
//!
//! 1. **The corpus still models reality.** If the generator drifts so that
//!    "prose" stops compressing like prose, every compression number becomes
//!    meaningless while still looking plausible.
//! 2. **The policy still sorts compressible from incompressible.** Raising the
//!    default level, or breaking the entropy sampler, silently turns a win into
//!    a loss.
//!
//! These tests pin both. They are intentionally loose on absolute numbers —
//! a test machine under load must not fail the build — but tight on direction.

use basalt_bench::corpus::{Corpus, CorpusSpec, Flavour, Rng, generate};
use basalt_proto::codec::{Codec, CompressionPolicy, is_precompressed_extension, shannon_entropy};

/// Link speed the design targets, in MB/s (both ends on Wi-Fi).
const TARGET_LINK_MBS: f64 = 30.0;

fn block(flavour: Flavour, size: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut buf = Vec::new();
    generate(flavour, size, &mut rng, &mut buf);
    buf
}

fn zstd_ratio(data: &[u8], level: i32) -> f64 {
    let compressed = zstd::encode_all(data, level).expect("compress");
    data.len() as f64 / compressed.len() as f64
}

/// Compression ratio plus the *fastest* observed compression rate, in MB/s.
///
/// Best-of-N rather than a single run. `cargo test` runs these in parallel with
/// everything else, so any individual run can be descheduled mid-measurement;
/// the fastest run is the one least contaminated by that. The question being
/// asked is "can this CPU compress faster than the link", which is a question
/// about capability, not about average throughput under test-suite load.
///
/// Without this the test is flaky, and a flaky test in the pre-commit hook is
/// worse than no test: it teaches you to reach for `--no-verify`.
fn ratio_and_peak_mbs(data: &[u8], level: i32) -> (f64, f64) {
    let mut ratio = 1.0;
    let mut best_mbs: f64 = 0.0;
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let compressed = zstd::encode_all(data, level).expect("compress");
        let elapsed = start.elapsed().as_secs_f64();
        ratio = data.len() as f64 / compressed.len() as f64;
        best_mbs = best_mbs.max((data.len() as f64 / 1e6) / elapsed.max(1e-9));
    }
    (ratio, best_mbs)
}

// --- the corpus still models reality ------------------------------------

#[test]
fn text_flavours_still_compress_like_text() {
    // Bounds are wide because the exact ratio is not the point; the point is
    // that these remain meaningfully compressible.
    let cases = [
        (Flavour::Prose, 1.8, 8.0),
        (Flavour::Code, 2.0, 15.0),
        (Flavour::Json, 2.5, 20.0),
    ];
    for (flavour, lo, hi) in cases {
        let ratio = zstd_ratio(&block(flavour, 1 << 20, 1), 1);
        assert!(
            ratio > lo && ratio < hi,
            "{} compressed {ratio:.2}x, expected between {lo}x and {hi}x — \
             the corpus no longer models this content type",
            flavour.name()
        );
    }
}

#[test]
fn binary_flavour_is_still_incompressible() {
    // If this ever starts compressing, every "compression wins" number in the
    // report is inflated, because the incompressible control arm is broken.
    let ratio = zstd_ratio(&block(Flavour::Binary, 1 << 20, 2), 1);
    assert!(
        ratio < 1.05,
        "binary compressed {ratio:.2}x — it must stay incompressible or the \
         benchmark flatters compression"
    );
}

#[test]
fn entropy_separates_the_flavours_cleanly() {
    // The heuristic only works if there is a wide gap between compressible and
    // incompressible entropy. Narrowing that gap makes the threshold fragile.
    let compressible_max = [Flavour::Prose, Flavour::Code, Flavour::Json]
        .into_iter()
        .map(|f| shannon_entropy(&block(f, 1 << 18, 3)))
        .fold(f32::MIN, f32::max);
    let incompressible = shannon_entropy(&block(Flavour::Binary, 1 << 18, 4));

    assert!(
        incompressible - compressible_max > 1.5,
        "entropy gap collapsed: compressible tops out at {compressible_max:.2}, \
         incompressible sits at {incompressible:.2}"
    );
    assert!(
        compressible_max < CompressionPolicy::default().entropy_threshold,
        "compressible content ({compressible_max:.2}) crossed the threshold \
         ({:.2}) and would now be sent raw",
        CompressionPolicy::default().entropy_threshold
    );
    assert!(
        incompressible > CompressionPolicy::default().entropy_threshold,
        "incompressible content ({incompressible:.2}) fell below the threshold \
         and would now be pointlessly compressed"
    );
}

// --- the policy still makes the right call -------------------------------

#[test]
fn policy_agrees_with_ground_truth_across_the_corpus() {
    let policy = CompressionPolicy::default();
    let mut wrong = Vec::new();

    for flavour in Flavour::all() {
        for seed in 0..12u64 {
            let data = block(flavour, 64 * 1024 + (seed as usize * 4096), seed + 100);
            let sample = &data[..data.len().min(policy.sample_bytes)];

            let truth = zstd_ratio(&data, 1) > 1.05;
            // Neutral extension so the entropy sampler is what is under test.
            let decided = !policy
                .decide("probe.unknown", data.len() as u64, sample)
                .is_raw();

            if decided != truth {
                wrong.push(format!(
                    "{} seed {seed}: decided {}, truth {}",
                    flavour.name(),
                    if decided { "compress" } else { "raw" },
                    if truth {
                        "compressible"
                    } else {
                        "incompressible"
                    }
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "the entropy heuristic disagreed with zstd on {} cases:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

#[test]
fn compression_still_beats_the_link_for_compressible_data() {
    // The headline claim, pinned. Model: compression of block N+1 overlaps
    // transmission of block N, so the achieved rate is whichever stage is
    // slower — hence `min`, not a sum.
    let policy = CompressionPolicy::default();

    for flavour in [Flavour::Prose, Flavour::Code, Flavour::Json] {
        let data = block(flavour, 4 * 1024 * 1024, 7);
        let (ratio, compress_mbs) = ratio_and_peak_mbs(&data, policy.level);
        let effective = compress_mbs.min(TARGET_LINK_MBS * ratio);

        assert!(
            effective > TARGET_LINK_MBS * 1.5,
            "{}: effective throughput {effective:.1} MB/s barely beats the raw \
             link ({TARGET_LINK_MBS} MB/s). ratio {ratio:.2}x, compressor \
             {compress_mbs:.0} MB/s — compression is no longer worth it",
            flavour.name()
        );
    }
}

#[test]
fn the_default_level_is_fast_enough_to_stay_ahead_of_the_link() {
    // Guards against someone raising the default level for a better ratio.
    // At level 9 the compressor drops near the link speed and the pipeline
    // stalls on CPU instead of the radio.
    let policy = CompressionPolicy::default();
    let data = block(Flavour::Prose, 4 * 1024 * 1024, 11);
    let (_, mbs) = ratio_and_peak_mbs(&data, policy.level);

    // Deliberately conservative: a loaded machine must not fail the build.
    // This only catches a genuinely unsuitable default.
    assert!(
        mbs > TARGET_LINK_MBS * 2.0,
        "zstd level {} ran at {mbs:.0} MB/s, too close to the {TARGET_LINK_MBS} \
         MB/s link — the compressor would become the bottleneck",
        policy.level
    );
}

#[test]
fn policy_never_compresses_known_media_formats() {
    // Even with a trivially compressible body, extension wins. This is the
    // cheap fast path that keeps a movie library from burning CPU.
    let policy = CompressionPolicy::default();
    let compressible_body = vec![b'a'; 128 * 1024];

    for name in [
        "movie.mp4",
        "clip.mkv",
        "song.mp3",
        "photo.jpg",
        "photo.HEIC",
        "archive.zip",
        "backup.7z",
        "doc.pdf",
        "sheet.xlsx",
        "image.webp",
        "disk.iso",
    ] {
        assert_eq!(
            policy.decide(name, 100 << 20, &compressible_body),
            Codec::Raw,
            "{name} should skip compression on the extension fast path"
        );
    }
}

#[test]
fn extension_table_entries_are_well_formed() {
    // A stray uppercase or dotted entry would silently never match, because
    // lookups are done on a lowercased, dot-stripped extension.
    for name in [
        "mp4", "mkv", "jpg", "zip", "7z", "pdf", "flac", "webp", "docx", "iso",
    ] {
        assert!(
            is_precompressed_extension(&format!("f.{name}")),
            "{name} should be recognised"
        );
        assert!(
            is_precompressed_extension(&format!("f.{}", name.to_uppercase())),
            "{name} should be recognised case-insensitively"
        );
    }

    for name in ["txt", "rs", "json", "md", "csv", "log", "xml", "sql", "bin"] {
        assert!(
            !is_precompressed_extension(&format!("f.{name}")),
            "{name} is compressible and must NOT be on the skip list"
        );
    }
}

#[test]
fn policy_round_trips_every_flavour() {
    let policy = CompressionPolicy::default();
    for flavour in Flavour::all() {
        let data = block(flavour, 512 * 1024, 21);
        let sample = &data[..data.len().min(policy.sample_bytes)];
        let codec = policy.decide(
            &format!("f.{}", flavour.extension()),
            data.len() as u64,
            sample,
        );
        let wire = match codec {
            Codec::Raw => data.clone(),
            Codec::Zstd(l) => zstd::encode_all(data.as_slice(), l).unwrap(),
        };
        let back = match codec {
            Codec::Raw => wire,
            Codec::Zstd(_) => zstd::decode_all(wire.as_slice()).unwrap(),
        };
        assert_eq!(
            back,
            data,
            "{} did not survive the round trip",
            flavour.name()
        );
    }
}

// --- the corpus is reproducible ------------------------------------------

#[test]
fn the_same_seed_produces_identical_bytes() {
    // Results are only comparable across machines and across days if the
    // corpus is reproducible. If this breaks, every recorded benchmark
    // silently stops being a valid baseline.
    for flavour in Flavour::all() {
        let a = block(flavour, 256 * 1024, 4242);
        let b = block(flavour, 256 * 1024, 4242);
        assert_eq!(a, b, "{} is not reproducible from its seed", flavour.name());
    }
}

#[test]
fn different_seeds_produce_different_bytes() {
    for flavour in Flavour::all() {
        let a = block(flavour, 64 * 1024, 1);
        let b = block(flavour, 64 * 1024, 2);
        assert_ne!(
            a,
            b,
            "{} ignored its seed — the corpus is not actually varying",
            flavour.name()
        );
    }
}

#[test]
fn generating_a_small_corpus_produces_the_expected_shape() {
    let root = std::env::temp_dir().join(format!("basalt-corpus-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let spec = CorpusSpec {
        large_file_bytes: 64 * 1024,
        small_file_count: 40,
        small_file_bytes: 2048,
        wide_entry_count: 50,
        seed: 777,
    };
    let corpus = Corpus::open(&root);
    corpus.generate(spec, true).expect("corpus should generate");

    assert!(corpus.is_generated(), "corpus should be marked complete");

    // Only the binary flavour is generated at full size; the other three are
    // companions and deliberately smaller, so that a weak laptop is not made to
    // format gigabytes of synthetic prose nothing ever reads.
    for flavour in Flavour::all() {
        let path = corpus.large_file(flavour);
        let len = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{} missing: {e}", path.display()))
            .len();
        assert!(len > 0, "{} is empty", path.display());

        if flavour == Flavour::Binary {
            assert!(
                len >= spec.large_file_bytes,
                "the measured large file {} is {len} bytes, expected at least {}",
                path.display(),
                spec.large_file_bytes
            );
        }
    }

    let small = corpus.small_file_paths().expect("listing small files");
    assert_eq!(small.len(), spec.small_file_count);
    assert!(small.iter().all(|p| p.starts_with("small/shard")));

    let wide = std::fs::read_dir(corpus.wide_dir()).unwrap().count();
    assert_eq!(wide, spec.wide_entry_count);

    // Regenerating without --force must be a no-op rather than a rewrite.
    corpus
        .generate(spec, false)
        .expect("second generate should skip");

    let _ = std::fs::remove_dir_all(&root);
}
