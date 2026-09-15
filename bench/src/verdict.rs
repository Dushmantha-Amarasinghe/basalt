//! The answer, in plain English.
//!
//! Everything else in this crate produces tables. Tables are useful for
//! re-checking a decision later, but they are not what anyone actually wants
//! after running a benchmark. This module reduces the whole exercise to one
//! question — *is the custom protocol worth building, or would Windows file
//! sharing do?* — and answers it in a sentence.
//!
//! The comparisons are deliberately unflattering to Basalt:
//!
//! - On small files it compares against SMB's **best parallel** result, not its
//!   sequential one. Beating a strawman would prove nothing.
//! - On large files it compares raw link throughput, where there is no clever
//!   trick available to either side and the answer should be roughly a tie.

use crate::stats::Suite;

/// One head-to-head result.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub what: String,
    pub basalt_mbs: f64,
    pub smb_mbs: f64,
    /// Plain-language note about what this particular row means.
    pub note: String,
}

impl Comparison {
    /// How many times faster Basalt is. Below 1.0 means SMB won.
    pub fn speedup(&self) -> f64 {
        if self.smb_mbs <= 0.0 {
            return 0.0;
        }
        self.basalt_mbs / self.smb_mbs
    }

    pub fn winner(&self) -> &'static str {
        let s = self.speedup();
        if s > 1.1 {
            "Basalt"
        } else if s < 0.9 {
            "Windows"
        } else {
            "tie"
        }
    }
}

/// Overall recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recommendation {
    /// Clear win on the case that matters. Build it.
    Build,
    /// Wins, but not by enough to justify months of work on the transport.
    Marginal,
    /// Windows file sharing is as good or better. Do not build a custom
    /// transport; build the app on top of SMB instead.
    UseSmb,
    /// Not enough data to say.
    Inconclusive,
}

impl Recommendation {
    pub fn headline(self) -> &'static str {
        match self {
            Recommendation::Build => "BUILD IT — the custom protocol is clearly faster",
            Recommendation::Marginal => "MARGINAL — faster, but not by much",
            Recommendation::UseSmb => "USE WINDOWS SHARING — the custom protocol is not faster",
            Recommendation::Inconclusive => "INCONCLUSIVE — some measurements are missing",
        }
    }
}

fn best_in<'a>(suites: &'a [Suite], suite_name: &str) -> Option<&'a crate::stats::Measurement> {
    suites
        .iter()
        .find(|s| s.name == suite_name)?
        .measurements
        .iter()
        .filter(|m| m.bytes > 0)
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()))
}

fn labelled<'a>(
    suites: &'a [Suite],
    suite_name: &str,
    contains: &str,
) -> Option<&'a crate::stats::Measurement> {
    suites
        .iter()
        .find(|s| s.name == suite_name)?
        .measurements
        .iter()
        .find(|m| m.label.contains(contains))
}

/// Builds the head-to-head table from the network and SMB result sets.
pub fn compare(net: &[Suite], smb: &[Suite]) -> Vec<Comparison> {
    let mut out = Vec::new();

    // Bulk transfer. Both sides are just moving bytes over the same radio, so
    // a rough tie is the expected and acceptable result here.
    if let (Some(b), Some(s)) = (
        best_in(net, "raw-throughput"),
        labelled(smb, "smb-large-file", "sequential read"),
    ) {
        out.push(Comparison {
            what: "One big file".into(),
            basalt_mbs: b.throughput_mbs(),
            smb_mbs: s.throughput_mbs(),
            note: "Moving a movie or a big archive. A tie here is fine — \
                   neither side can beat the radio."
                .into(),
        });
    }

    // Many small files. This is the case the batch protocol exists for, and the
    // one the whole decision turns on.
    if let (Some(b), Some(s)) = (
        labelled(net, "small-files", "zstd"),
        best_in(smb, "smb-small-files"),
    ) {
        out.push(Comparison {
            what: "Thousands of small files".into(),
            basalt_mbs: b.throughput_mbs(),
            smb_mbs: s.throughput_mbs(),
            note: "Copying a folder full of documents, photos or code. This is \
                   where a custom protocol should win, and the number that \
                   decides whether to build one."
                .into(),
        });
    }

    // Opening a big folder. Not decisive, but it is what browsing feels like.
    if let (Some(b), Some(s)) = (
        labelled(net, "latency", "ping"),
        labelled(smb, "smb-listing", "names + metadata"),
    ) {
        let _ = (b, s); // Different units; reported separately rather than as a ratio.
    }

    out
}

/// Turns the comparisons into a recommendation.
pub fn recommend(comparisons: &[Comparison]) -> Recommendation {
    let small = comparisons
        .iter()
        .find(|c| c.what.contains("small files"))
        .map(|c| c.speedup());
    let large = comparisons
        .iter()
        .find(|c| c.what.contains("big file"))
        .map(|c| c.speedup());

    let (Some(small), Some(large)) = (small, large) else {
        return Recommendation::Inconclusive;
    };
    if small <= 0.0 || large <= 0.0 {
        return Recommendation::Inconclusive;
    }

    // Losing badly on bulk transfer disqualifies it regardless of the small-file
    // result: that is the common case and a regression there would be felt
    // every time a video is copied.
    if large < 0.8 {
        return Recommendation::UseSmb;
    }

    if small >= 2.0 {
        Recommendation::Build
    } else if small >= 1.25 {
        Recommendation::Marginal
    } else {
        Recommendation::UseSmb
    }
}

/// Prints the whole thing in plain language.
pub fn print(comparisons: &[Comparison]) {
    println!("\n{}", "=".repeat(70));
    println!("  RESULT: Basalt vs built-in Windows file sharing");
    println!("{}", "=".repeat(70));

    if comparisons.is_empty() {
        println!(
            "\n  No comparison could be made. That usually means the SMB half\n\
             \x20 did not run — check that the share is reachable from this PC."
        );
        return;
    }

    for c in comparisons {
        println!("\n  {}", c.what);
        println!("    Basalt          {:>8.1} MB/s", c.basalt_mbs);
        println!("    Windows sharing {:>8.1} MB/s", c.smb_mbs);
        let s = c.speedup();
        if s >= 1.0 {
            println!("    -> Basalt is {s:.2}x faster ({})", c.winner());
        } else {
            println!(
                "    -> Windows is {:.2}x faster ({})",
                1.0 / s.max(0.0001),
                c.winner()
            );
        }
        println!("    {}", c.note);
    }

    let rec = recommend(comparisons);
    println!("\n{}", "-".repeat(70));
    println!("  {}", rec.headline());
    println!("{}", "-".repeat(70));

    match rec {
        Recommendation::Build => println!(
            "\n  The custom protocol wins clearly on the case that matters most.\n\
             \x20 Building the transfer engine is justified.\n"
        ),
        Recommendation::Marginal => println!(
            "\n  It is faster, but not dramatically. The transfer engine is\n\
             \x20 probably still worth building, though the stronger reason to\n\
             \x20 continue is the app itself rather than raw speed.\n"
        ),
        Recommendation::UseSmb => println!(
            "\n  Windows file sharing already does this as well or better.\n\
             \x20 Recommendation: do not build a custom transfer protocol.\n\
             \x20 Build the app on top of the built-in sharing instead — you\n\
             \x20 still get the whole interface, media player and library, and\n\
             \x20 save months of work on the part that turned out not to help.\n"
        ),
        Recommendation::Inconclusive => println!(
            "\n  Some measurements are missing, so no conclusion can be drawn.\n\
             \x20 Both the `net` and `smb` halves need to complete.\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(what: &str, basalt: f64, smb: f64) -> Comparison {
        Comparison {
            what: what.into(),
            basalt_mbs: basalt,
            smb_mbs: smb,
            note: String::new(),
        }
    }

    #[test]
    fn speedup_is_a_plain_ratio() {
        assert!((cmp("x", 30.0, 15.0).speedup() - 2.0).abs() < 1e-9);
        assert!((cmp("x", 15.0, 30.0).speedup() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_zero_baseline_does_not_divide_by_zero() {
        assert_eq!(cmp("x", 30.0, 0.0).speedup(), 0.0);
    }

    #[test]
    fn winner_has_a_dead_band_around_parity() {
        assert_eq!(cmp("x", 100.0, 100.0).winner(), "tie");
        assert_eq!(cmp("x", 105.0, 100.0).winner(), "tie");
        assert_eq!(cmp("x", 150.0, 100.0).winner(), "Basalt");
        assert_eq!(cmp("x", 50.0, 100.0).winner(), "Windows");
    }

    #[test]
    fn a_clear_small_file_win_recommends_building() {
        let c = vec![
            cmp("One big file", 30.0, 29.0),
            cmp("Thousands of small files", 60.0, 20.0),
        ];
        assert_eq!(recommend(&c), Recommendation::Build);
    }

    #[test]
    fn no_small_file_advantage_recommends_smb() {
        // The honest outcome we have to be willing to report.
        let c = vec![
            cmp("One big file", 30.0, 30.0),
            cmp("Thousands of small files", 21.0, 20.0),
        ];
        assert_eq!(recommend(&c), Recommendation::UseSmb);
    }

    #[test]
    fn a_modest_win_is_marginal() {
        let c = vec![
            cmp("One big file", 30.0, 30.0),
            cmp("Thousands of small files", 30.0, 20.0),
        ];
        assert_eq!(recommend(&c), Recommendation::Marginal);
    }

    #[test]
    fn losing_badly_on_big_files_disqualifies_regardless_of_small_files() {
        // Bulk transfer is the common case. A big regression there is not worth
        // trading for a small-file win.
        let c = vec![
            cmp("One big file", 15.0, 30.0),
            cmp("Thousands of small files", 100.0, 20.0),
        ];
        assert_eq!(recommend(&c), Recommendation::UseSmb);
    }

    #[test]
    fn missing_measurements_are_inconclusive_not_a_guess() {
        assert_eq!(recommend(&[]), Recommendation::Inconclusive);
        assert_eq!(
            recommend(&[cmp("One big file", 30.0, 30.0)]),
            Recommendation::Inconclusive
        );
        assert_eq!(
            recommend(&[
                cmp("One big file", 30.0, 0.0),
                cmp("Thousands of small files", 30.0, 10.0),
            ]),
            Recommendation::Inconclusive
        );
    }

    #[test]
    fn every_recommendation_has_a_headline() {
        for r in [
            Recommendation::Build,
            Recommendation::Marginal,
            Recommendation::UseSmb,
            Recommendation::Inconclusive,
        ] {
            assert!(r.headline().len() > 10);
        }
    }

    #[test]
    fn printing_an_empty_comparison_does_not_panic() {
        print(&[]);
    }

    #[test]
    fn compare_pulls_the_right_measurements_out_of_the_suites() {
        use crate::stats::Measurement;
        use std::time::Duration;

        let mut net_raw = Suite::new("raw-throughput", "");
        let mut m = Measurement::new("download  4 streams plain", 100_000_000);
        m.record(Duration::from_millis(1000));
        net_raw.measurements.push(m);

        let mut net_small = Suite::new("small-files", "");
        let mut m = Measurement::new("batched request, zstd", 50_000_000);
        m.record(Duration::from_millis(1000));
        net_small.measurements.push(m);

        let mut smb_large = Suite::new("smb-large-file", "");
        let mut m = Measurement::new("sequential read", 50_000_000);
        m.record(Duration::from_millis(1000));
        smb_large.measurements.push(m);

        let mut smb_small = Suite::new("smb-small-files", "");
        let mut m = Measurement::new(" 8 threads in parallel", 25_000_000);
        m.record(Duration::from_millis(1000));
        smb_small.measurements.push(m);

        let comparisons = compare(&[net_raw, net_small], &[smb_large, smb_small]);
        assert_eq!(comparisons.len(), 2);
        // 100 MB/s vs 50 MB/s
        assert!((comparisons[0].speedup() - 2.0).abs() < 0.01);
        // 50 MB/s vs 25 MB/s
        assert!((comparisons[1].speedup() - 2.0).abs() < 0.01);
    }

    #[test]
    fn compare_returns_nothing_when_a_side_is_missing() {
        assert!(compare(&[], &[]).is_empty());
    }
}
