//! Measurement primitives.
//!
//! Benchmarks lie easily, so the rules here are deliberate:
//!
//! - Report the **median**, not the mean. One antivirus scan or one HDD
//!   re-seek turns a mean into fiction.
//! - Always report spread (p95 and min/max). A median of 30 MB/s with a p95 of
//!   200 ms tells a very different story from one with a p95 of 31 MB/s.
//! - Never report a single run.

use std::fmt;
use std::time::Duration;

use serde::Serialize;

/// A set of timed runs of the same operation moving the same number of bytes.
#[derive(Debug, Clone, Serialize)]
pub struct Measurement {
    pub label: String,
    /// Logical bytes moved per run (before any compression).
    pub bytes: u64,
    /// Bytes actually put on the wire or read from disk per run. Equal to
    /// `bytes` when nothing is compressed.
    pub wire_bytes: u64,
    /// Items (files, entries) per run, when the operation is item-shaped.
    pub items: u64,
    pub durations_ms: Vec<f64>,
    pub notes: Vec<String>,
}

impl Measurement {
    pub fn new(label: impl Into<String>, bytes: u64) -> Self {
        Self {
            label: label.into(),
            bytes,
            wire_bytes: bytes,
            items: 0,
            durations_ms: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn with_items(mut self, items: u64) -> Self {
        self.items = items;
        self
    }

    pub fn with_wire_bytes(mut self, wire: u64) -> Self {
        self.wire_bytes = wire;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn record(&mut self, elapsed: Duration) {
        self.durations_ms.push(elapsed.as_secs_f64() * 1000.0);
    }

    fn sorted_ms(&self) -> Vec<f64> {
        let mut v = self.durations_ms.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    pub fn runs(&self) -> usize {
        self.durations_ms.len()
    }

    pub fn median_ms(&self) -> f64 {
        percentile(&self.sorted_ms(), 0.50)
    }

    pub fn p95_ms(&self) -> f64 {
        percentile(&self.sorted_ms(), 0.95)
    }

    pub fn min_ms(&self) -> f64 {
        self.sorted_ms().first().copied().unwrap_or(0.0)
    }

    pub fn max_ms(&self) -> f64 {
        self.sorted_ms().last().copied().unwrap_or(0.0)
    }

    /// Throughput in MB/s (10^6 bytes, matching how transfer speeds are
    /// normally quoted) based on the median run.
    pub fn throughput_mbs(&self) -> f64 {
        let ms = self.median_ms();
        if ms <= 0.0 || self.bytes == 0 {
            return 0.0;
        }
        (self.bytes as f64 / 1e6) / (ms / 1000.0)
    }

    /// Bytes actually transmitted per second. Below [`Self::throughput_mbs`]
    /// whenever compression is doing something.
    pub fn wire_throughput_mbs(&self) -> f64 {
        let ms = self.median_ms();
        if ms <= 0.0 || self.wire_bytes == 0 {
            return 0.0;
        }
        (self.wire_bytes as f64 / 1e6) / (ms / 1000.0)
    }

    pub fn items_per_sec(&self) -> f64 {
        let ms = self.median_ms();
        if ms <= 0.0 || self.items == 0 {
            return 0.0;
        }
        self.items as f64 / (ms / 1000.0)
    }

    /// Logical bytes per wire byte. 1.0 means compression bought nothing.
    pub fn compression_ratio(&self) -> f64 {
        if self.wire_bytes == 0 {
            return 1.0;
        }
        self.bytes as f64 / self.wire_bytes as f64
    }

    /// Relative spread of the middle of the distribution. Above ~0.25 means the
    /// machine was busy and the number should not be trusted.
    pub fn instability(&self) -> f64 {
        let med = self.median_ms();
        if med <= 0.0 {
            return 0.0;
        }
        (self.p95_ms() - med) / med
    }

    pub fn is_stable(&self) -> bool {
        self.instability() < 0.25
    }
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = rank - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Human-readable byte count using binary units.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes == 0 {
        return "0 B".into();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Human-readable duration.
pub fn fmt_duration_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{:.0} µs", ms * 1000.0)
    } else if ms < 1000.0 {
        format!("{ms:.1} ms")
    } else if ms < 60_000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        let total = ms / 1000.0;
        format!("{}m {:.0}s", (total / 60.0) as u64, total % 60.0)
    }
}

/// A named group of measurements sharing one question, e.g. "how many parallel
/// streams saturate this link?".
#[derive(Debug, Clone, Serialize)]
pub struct Suite {
    pub name: String,
    pub description: String,
    pub measurements: Vec<Measurement>,
}

impl Suite {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            measurements: Vec::new(),
        }
    }

    pub fn push(&mut self, m: Measurement) {
        println!("  {m}");
        self.measurements.push(m);
    }

    /// The measurement with the highest logical throughput.
    pub fn best(&self) -> Option<&Measurement> {
        self.measurements.iter().max_by(|a, b| {
            a.throughput_mbs()
                .partial_cmp(&b.throughput_mbs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

impl fmt::Display for Measurement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:<34} ", self.label)?;
        if self.bytes > 0 {
            write!(f, "{:>8.1} MB/s ", self.throughput_mbs())?;
        }
        write!(f, "{:>10}", fmt_duration_ms(self.median_ms()))?;
        if self.items > 0 {
            write!(f, "  {:>9.0} items/s", self.items_per_sec())?;
        }
        let ratio = self.compression_ratio();
        if ratio > 1.001 {
            write!(f, "  {ratio:.2}x compressed")?;
        }
        if self.runs() > 1 && !self.is_stable() {
            write!(f, "  ⚠ unstable (p95 {})", fmt_duration_ms(self.p95_ms()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurement_with(ms: &[f64], bytes: u64) -> Measurement {
        let mut m = Measurement::new("t", bytes);
        for &v in ms {
            m.record(Duration::from_secs_f64(v / 1000.0));
        }
        m
    }

    #[test]
    fn percentile_interpolates_between_samples() {
        let v = vec![10.0, 20.0, 30.0, 40.0];
        assert!((percentile(&v, 0.0) - 10.0).abs() < 1e-9);
        assert!((percentile(&v, 1.0) - 40.0).abs() < 1e-9);
        assert!((percentile(&v, 0.5) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn percentile_handles_degenerate_inputs() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[7.0], 0.95), 7.0);
    }

    #[test]
    fn median_ignores_a_single_outlier() {
        // The whole reason we report median rather than mean: one 10x stall
        // must not move the headline number.
        let m = measurement_with(&[100.0, 101.0, 99.0, 100.0, 1000.0], 0);
        assert!((m.median_ms() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn throughput_is_computed_from_the_median() {
        // 100 MB in 1000 ms = 100 MB/s.
        let m = measurement_with(&[1000.0], 100_000_000);
        assert!((m.throughput_mbs() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn wire_throughput_reflects_compression() {
        let mut m = measurement_with(&[1000.0], 100_000_000);
        m.wire_bytes = 25_000_000;
        assert!((m.throughput_mbs() - 100.0).abs() < 1e-6);
        assert!((m.wire_throughput_mbs() - 25.0).abs() < 1e-6);
        assert!((m.compression_ratio() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn zero_duration_or_zero_bytes_does_not_divide_by_zero() {
        assert_eq!(Measurement::new("empty", 0).throughput_mbs(), 0.0);
        assert_eq!(measurement_with(&[0.0], 1000).throughput_mbs(), 0.0);
        assert_eq!(Measurement::new("empty", 0).items_per_sec(), 0.0);
        assert_eq!(Measurement::new("empty", 0).compression_ratio(), 1.0);
    }

    #[test]
    fn instability_flags_a_noisy_run() {
        let steady = measurement_with(&[100.0, 100.0, 101.0, 100.0, 99.0], 1000);
        assert!(steady.is_stable());

        let noisy = measurement_with(&[100.0, 100.0, 100.0, 100.0, 900.0], 1000);
        assert!(!noisy.is_stable(), "a 9x p95 outlier must be flagged");
    }

    #[test]
    fn items_per_sec_is_computed() {
        let m = measurement_with(&[1000.0], 0).with_items(10_000);
        assert!((m.items_per_sec() - 10_000.0).abs() < 1e-6);
    }

    #[test]
    fn byte_formatting_is_readable() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1024), "1.00 KiB");
        assert_eq!(fmt_bytes(1536), "1.50 KiB");
        assert_eq!(fmt_bytes(20 * 1024), "20.0 KiB");
        assert_eq!(fmt_bytes(500 * 1024), "500 KiB");
        assert_eq!(fmt_bytes(4 * 1024 * 1024 * 1024), "4.00 GiB");
    }

    #[test]
    fn duration_formatting_picks_sensible_units() {
        assert_eq!(fmt_duration_ms(0.25), "250 µs");
        assert_eq!(fmt_duration_ms(12.5), "12.5 ms");
        assert_eq!(fmt_duration_ms(2500.0), "2.50 s");
        assert_eq!(fmt_duration_ms(90_000.0), "1m 30s");
    }

    #[test]
    fn suite_best_picks_the_highest_throughput() {
        let mut s = Suite::new("streams", "how many streams saturate the link");
        let mut slow = Measurement::new("1 stream", 100_000_000);
        slow.record(Duration::from_millis(1000));
        let mut fast = Measurement::new("4 streams", 100_000_000);
        fast.record(Duration::from_millis(400));
        s.push(slow);
        s.push(fast);
        assert_eq!(s.best().unwrap().label, "4 streams");
    }
}
