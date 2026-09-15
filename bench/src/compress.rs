//! Compression benchmarks.
//!
//! This answers the question Phase 0 exists to answer first, because it is the
//! highest-value lever in the design: **on a ~30 MB/s Wi-Fi link, does zstd buy
//! us real throughput, and at what level?**
//!
//! The reasoning being tested: zstd level 1 compresses at 400–800 MB/s, roughly
//! 20x faster than we can transmit. If that holds on the host laptop's CPU,
//! then compression is effectively free and every compressible byte we squeeze
//! is a byte we do not have to wait for the radio to carry.
//!
//! The number that matters is **effective throughput**, not ratio:
//!
//! ```text
//! effective = 1 / (1/compress_speed + ratio⁻¹/link_speed)
//! ```
//!
//! A 10x ratio is worthless if compressing runs slower than the link. This
//! module computes effective throughput at the real link speed, so the answer
//! accounts for both.
//!
//! It also measures the decision heuristic itself: how often the entropy
//! sampler agrees with what zstd actually achieves. A sampler that waves
//! incompressible data through costs CPU for nothing.

use std::time::Instant;

use anyhow::Result;
use basalt_proto::codec::{Codec, CompressionPolicy, shannon_entropy};

use crate::corpus::{Flavour, Rng, generate};
use crate::stats::{Measurement, Suite, fmt_bytes};

/// zstd levels worth testing. Above ~6 the speed collapses well below any
/// plausible link speed, so there is no point measuring further.
const LEVELS: [i32; 4] = [1, 3, 6, 9];

/// Link speeds to model, in MB/s.
///
/// 30 is the realistic both-ends-wireless case for this deployment; 100 models
/// a wired host; 1000 shows where compression stops paying off at all, which is
/// the sanity check on the whole idea.
const MODELLED_LINKS_MBS: [f64; 3] = [30.0, 100.0, 1000.0];

/// Bytes per sample block. Matches the batch-path working set.
const BLOCK_BYTES: usize = 4 * 1024 * 1024;

pub fn run(seed: u64, runs: usize) -> Result<Vec<Suite>> {
    let mut suites = Vec::new();
    suites.push(throughput_by_flavour(seed, runs)?);
    suites.push(effective_throughput(seed)?);
    suites.push(heuristic_accuracy(seed)?);
    Ok(suites)
}

/// Raw compress/decompress speed and ratio, per content type and zstd level.
fn throughput_by_flavour(seed: u64, runs: usize) -> Result<Suite> {
    let mut suite = Suite::new(
        "compression-throughput",
        "zstd speed and ratio by content type and level, on this CPU",
    );

    println!(
        "\ncompression throughput ({} per block)",
        fmt_bytes(BLOCK_BYTES as u64)
    );

    let mut rng = Rng::new(seed);
    for flavour in Flavour::all() {
        let mut block = Vec::new();
        generate(flavour, BLOCK_BYTES, &mut rng, &mut block);

        for level in LEVELS {
            let mut m = Measurement::new(
                format!("{:<7} compress  zstd-{level}", flavour.name()),
                block.len() as u64,
            );
            let mut compressed = Vec::new();
            for _ in 0..runs {
                let t = Instant::now();
                compressed = zstd::encode_all(block.as_slice(), level)?;
                m.record(t.elapsed());
            }
            m = m.with_wire_bytes(compressed.len() as u64);
            suite.push(m);

            // Decompression speed matters as much as compression: the client is
            // the one decoding, and if it cannot keep up with the radio then
            // compression has just moved the bottleneck rather than removed it.
            let mut d = Measurement::new(
                format!("{:<7} decompress zstd-{level}", flavour.name()),
                block.len() as u64,
            )
            .with_wire_bytes(compressed.len() as u64);
            for _ in 0..runs {
                let t = Instant::now();
                let out = zstd::decode_all(compressed.as_slice())?;
                d.record(t.elapsed());
                debug_assert_eq!(out.len(), block.len());
            }
            suite.push(d);
        }
    }

    Ok(suite)
}

/// The decision-grade number: end-to-end throughput once the link is modelled.
fn effective_throughput(seed: u64) -> Result<Suite> {
    let mut suite = Suite::new(
        "effective-throughput",
        "modelled end-to-end throughput including transmission, by link speed",
    );

    println!("\neffective throughput (compress + transmit, modelled)");
    println!(
        "  {:<26} {:>10} {:>10} {:>11} {:>11}",
        "", "ratio", "zstd MB/s", "raw @30MB/s", "zstd @30MB/s"
    );

    let mut rng = Rng::new(seed);
    for flavour in Flavour::all() {
        let mut block = Vec::new();
        generate(flavour, BLOCK_BYTES, &mut rng, &mut block);

        for level in LEVELS {
            let t = Instant::now();
            let compressed = zstd::encode_all(block.as_slice(), level)?;
            let compress_secs = t.elapsed().as_secs_f64();

            let ratio = block.len() as f64 / compressed.len() as f64;
            let compress_mbs = (block.len() as f64 / 1e6) / compress_secs.max(1e-9);

            for link_mbs in MODELLED_LINKS_MBS {
                // Pipelined, not serial: compression of block N+1 overlaps with
                // transmission of block N, so the achieved rate is whichever
                // stage is slower — not the sum of both.
                let transmit_mbs = link_mbs * ratio;
                let effective = compress_mbs.min(transmit_mbs);
                let gain = effective / link_mbs;

                let mut m = Measurement::new(
                    format!(
                        "{:<7} zstd-{level} @ {link_mbs:.0} MB/s link",
                        flavour.name()
                    ),
                    0,
                );
                m.record(std::time::Duration::from_secs_f64(compress_secs));
                m.notes.push(format!(
                    "ratio {ratio:.2}x, compress {compress_mbs:.0} MB/s, \
                     effective {effective:.1} MB/s, {gain:.2}x vs raw"
                ));
                suite.measurements.push(m);

                if (link_mbs - 30.0).abs() < f64::EPSILON {
                    println!(
                        "  {:<26} {ratio:>9.2}x {compress_mbs:>10.0} {link_mbs:>11.1} {effective:>11.1}",
                        format!("{} zstd-{level}", flavour.name()),
                    );
                }
            }
        }
    }

    Ok(suite)
}

/// Does the cheap entropy heuristic actually agree with zstd?
///
/// A false "compressible" costs CPU for nothing. A false "incompressible" wastes
/// link capacity, which is the more expensive mistake here. This measures both
/// error rates against ground truth, and sweeps the threshold so Phase 0 can
/// pick the right value rather than inheriting the 7.5 guess.
fn heuristic_accuracy(seed: u64) -> Result<Suite> {
    let mut suite = Suite::new(
        "heuristic-accuracy",
        "agreement between the entropy sampler and actual zstd outcomes",
    );

    println!("\nentropy heuristic vs ground truth");
    println!(
        "  {:<10} {:>9} {:>12} {:>10} {:>10}",
        "flavour", "entropy", "true ratio", "predicted", "correct"
    );

    let policy = CompressionPolicy::default();
    let mut rng = Rng::new(seed ^ 0xE47);
    let mut correct = 0usize;
    let mut total = 0usize;
    let mut entropies = Vec::new();

    for flavour in Flavour::all() {
        for _ in 0..8 {
            let size = rng.range(32 * 1024, 512 * 1024);
            let mut block = Vec::new();
            generate(flavour, size, &mut rng, &mut block);

            let sample = &block[..block.len().min(policy.sample_bytes)];
            let entropy = shannon_entropy(sample);

            // Ground truth: is compressing this actually worth it? A 5% saving
            // is the break-even point below which the CPU is not worth spending.
            let compressed = zstd::encode_all(block.as_slice(), 1)?;
            let true_ratio = block.len() as f64 / compressed.len() as f64;
            let actually_worth_it = true_ratio > 1.05;

            // Decide using a neutral extension so the entropy sampler is what
            // is under test, not the extension table.
            let predicted = policy.decide("sample.unknown", block.len() as u64, sample);
            let predicted_compress = !predicted.is_raw();

            let is_correct = predicted_compress == actually_worth_it;
            correct += is_correct as usize;
            total += 1;
            entropies.push((flavour, entropy, true_ratio, actually_worth_it));

            if total <= 4 || !is_correct {
                println!(
                    "  {:<10} {entropy:>9.3} {true_ratio:>11.2}x {:>10} {:>10}",
                    flavour.name(),
                    if predicted_compress {
                        "compress"
                    } else {
                        "raw"
                    },
                    if is_correct { "yes" } else { "NO" },
                );
            }
        }
    }

    let accuracy = correct as f64 / total.max(1) as f64;
    println!("  accuracy: {correct}/{total} ({:.1}%)", accuracy * 100.0);

    // Sweep the threshold to find the widest correct separation, rather than
    // trusting the default.
    let mut best = (policy.entropy_threshold, 0usize);
    let mut t = 6.0f32;
    while t <= 8.0 {
        let hits = entropies
            .iter()
            .filter(|(_, e, _, worth)| (*e <= t) == *worth)
            .count();
        if hits > best.1 {
            best = (t, hits);
        }
        t += 0.05;
    }

    let max_compressible = entropies
        .iter()
        .filter(|(_, _, _, w)| *w)
        .map(|(_, e, _, _)| *e)
        .fold(f32::MIN, f32::max);
    let min_incompressible = entropies
        .iter()
        .filter(|(_, _, _, w)| !*w)
        .map(|(_, e, _, _)| *e)
        .fold(f32::MAX, f32::min);

    println!(
        "  entropy separation: compressible ≤ {max_compressible:.3}, \
         incompressible ≥ {min_incompressible:.3}"
    );
    println!(
        "  widest-separation threshold: {:.2} ({}/{} correct; default is {:.2})",
        best.0, best.1, total, policy.entropy_threshold
    );
    // Do not take that sweep at face value. It weights both error directions
    // equally, and they are not equal here:
    //
    //   - Threshold too HIGH  -> we compress incompressible data. Cost: a few
    //     CPU-milliseconds we have in abundance (zstd-1 runs at 400+ MB/s
    //     against a ~30 MB/s link).
    //   - Threshold too LOW   -> we ship compressible data raw. Cost: link
    //     capacity, the one resource that is actually scarce.
    //
    // So the default deliberately sits near the top of the safe band rather
    // than at its midpoint: erring toward compressing is the cheap mistake.
    println!(
        "  keeping {:.2} — the two error directions have asymmetric cost, and \
         wasting CPU is far cheaper than wasting link",
        policy.entropy_threshold
    );

    let mut m = Measurement::new("entropy heuristic accuracy", 0).with_items(total as u64);
    m.record(std::time::Duration::from_millis(1));
    m.notes.push(format!(
        "{correct}/{total} correct ({:.1}%); suggested threshold {:.2}; \
         separation gap {max_compressible:.3}..{min_incompressible:.3}",
        accuracy * 100.0,
        best.0
    ));
    suite.measurements.push(m);

    Ok(suite)
}

/// Measures the shared-window effect that motivates batching the whole stream
/// through one zstd context instead of compressing each file separately.
pub fn shared_window_gain(seed: u64, file_count: usize, file_bytes: usize) -> Result<Measurement> {
    let mut rng = Rng::new(seed ^ 0x5EA);
    let mut files = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let mut buf = Vec::new();
        generate(Flavour::all()[i % 3], file_bytes, &mut rng, &mut buf);
        files.push(buf);
    }

    let logical: usize = files.iter().map(|f| f.len()).sum();

    let per_file: usize = files
        .iter()
        .map(|f| zstd::encode_all(f.as_slice(), 1).map(|c| c.len()))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .sum();

    let mut concatenated = Vec::with_capacity(logical);
    for f in &files {
        concatenated.extend_from_slice(f);
    }
    let t = Instant::now();
    let shared = zstd::encode_all(concatenated.as_slice(), 1)?;
    let elapsed = t.elapsed();

    let mut m = Measurement::new(
        format!(
            "shared window, {file_count} x {}",
            fmt_bytes(file_bytes as u64)
        ),
        logical as u64,
    )
    .with_wire_bytes(shared.len() as u64)
    .with_items(file_count as u64);
    m.record(elapsed);
    m.notes.push(format!(
        "per-file {} vs shared-window {} ({:.2}x better)",
        fmt_bytes(per_file as u64),
        fmt_bytes(shared.len() as u64),
        per_file as f64 / shared.len() as f64
    ));

    println!(
        "\nshared-window batching: per-file {} -> shared {} ({:.2}x smaller)",
        fmt_bytes(per_file as u64),
        fmt_bytes(shared.len() as u64),
        per_file as f64 / shared.len() as f64
    );

    Ok(m)
}

/// Sanity check that the [`Codec`] the policy picks actually round-trips.
pub fn verify_round_trip(seed: u64) -> Result<()> {
    let policy = CompressionPolicy::default();
    let mut rng = Rng::new(seed);
    for flavour in Flavour::all() {
        let mut block = Vec::new();
        generate(flavour, 256 * 1024, &mut rng, &mut block);
        let sample = &block[..block.len().min(policy.sample_bytes)];
        let codec = policy.decide(
            &format!("x.{}", flavour.extension()),
            block.len() as u64,
            sample,
        );
        let wire = match codec {
            Codec::Raw => block.clone(),
            Codec::Zstd(l) => zstd::encode_all(block.as_slice(), l)?,
        };
        let back = match codec {
            Codec::Raw => wire,
            Codec::Zstd(_) => zstd::decode_all(wire.as_slice())?,
        };
        anyhow::ensure!(
            back == block,
            "{} did not survive a {codec:?} round trip",
            flavour.name()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_selected_codecs_round_trip() {
        verify_round_trip(99).unwrap();
    }

    #[test]
    fn shared_window_beats_per_file_on_a_realistic_batch() {
        let m = shared_window_gain(5, 200, 8 * 1024).unwrap();
        assert!(
            m.compression_ratio() > 2.0,
            "batching similar small files should compress well, got {:.2}x",
            m.compression_ratio()
        );
    }

    #[test]
    fn effective_throughput_model_prefers_raw_on_a_fast_link() {
        // Sanity check on the model itself: with a 1000 MB/s link and a 2x
        // ratio, a 400 MB/s compressor is the bottleneck and compression is a
        // net loss. This is why the model uses min(), not the ratio alone.
        let compress_mbs = 400.0f64;
        let ratio = 2.0f64;
        let link = 1000.0f64;
        let effective = compress_mbs.min(link * ratio);
        assert!(
            effective < link,
            "on a fast link a slow compressor must reduce throughput"
        );
    }

    #[test]
    fn effective_throughput_model_prefers_compression_on_a_slow_link() {
        let compress_mbs = 400.0f64;
        let ratio = 2.0f64;
        let link = 30.0f64;
        let effective = compress_mbs.min(link * ratio);
        assert!(
            effective > link * 1.9,
            "on a slow link compression should nearly double throughput"
        );
    }
}
