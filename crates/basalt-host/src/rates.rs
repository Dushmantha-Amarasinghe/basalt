//! Turning byte counters into speeds, honestly.
//!
//! [`crate::traffic`] deliberately refuses to compute a rate, because it does
//! not know over what interval to divide. This is the piece that does know: it
//! keeps the previous snapshot and the instant it was taken, and divides the
//! difference by the time that actually elapsed between the two readings.
//!
//! That distinction is not pedantry. The client once displayed 35 MB/s over a
//! link measured at 22.7 MB/s, because one side sent 250 ms of bytes and the
//! other divided by its own 120 ms timer. Anything that divides by an assumed
//! interval is inventing a number. So a poll that arrives early, late, or not
//! for a minute all produce a correct answer here, because the divisor is
//! measured rather than assumed.

use std::collections::HashMap;
use std::time::Instant;

use crate::traffic::DeviceTraffic;

/// Bytes per second, each direction, for one device.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rate {
    pub send: f64,
    pub receive: f64,
}

/// Intervals shorter than this are not treated as a measurement.
///
/// Two polls a millisecond apart would divide a handful of bytes by almost
/// nothing and report a fantastical speed. Below the floor the previous answer
/// stands and the baseline is left alone, so the next poll measures across the
/// whole gap instead.
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Remembers enough to answer "how fast, since you last asked?".
#[derive(Debug, Default)]
pub struct Rates {
    previous: HashMap<String, DeviceTraffic>,
    taken: Option<Instant>,
    last: HashMap<String, Rate>,
}

impl Rates {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a snapshot and returns the rate since the one before it.
    ///
    /// The first call has nothing to compare against and reports zero, which is
    /// the only truthful answer available.
    pub fn sample(
        &mut self,
        now: Instant,
        snapshot: HashMap<String, DeviceTraffic>,
    ) -> &HashMap<String, Rate> {
        let elapsed = self.taken.map(|t| now.duration_since(t));

        match elapsed {
            // Too soon to have measured anything; leave the baseline in place
            // so the next call spans the whole gap rather than a sliver of it.
            Some(gap) if gap < MIN_INTERVAL => return &self.last,
            Some(gap) => {
                let seconds = gap.as_secs_f64();
                let mut rates = HashMap::with_capacity(snapshot.len());
                for (device, current) in &snapshot {
                    let before = self.previous.get(device).copied().unwrap_or_default();
                    rates.insert(
                        device.clone(),
                        Rate {
                            // Saturating rather than wrapping or absolute: a
                            // counter that went backwards is not something
                            // this module can explain, and reporting zero for
                            // one interval is the only answer that does not
                            // invent a number. The next sample re-baselines
                            // and is correct again.
                            send: current.sent.saturating_sub(before.sent) as f64 / seconds,
                            receive: current.received.saturating_sub(before.received) as f64
                                / seconds,
                        },
                    );
                }
                self.last = rates;
            }
            // Nothing to compare against yet. Zero for everything present,
            // rather than an empty map, so a caller sees the same shape on the
            // first poll as on every later one.
            None => {
                self.last = snapshot
                    .keys()
                    .map(|d| (d.clone(), Rate::default()))
                    .collect();
            }
        }

        self.previous = snapshot;
        self.taken = Some(now);
        &self.last
    }

    /// The rate last computed for one device, zero if it has none.
    pub fn of(&self, device: &str) -> Rate {
        self.last.get(device).copied().unwrap_or_default()
    }

    /// Drops a device, for when it is removed.
    pub fn forget(&mut self, device: &str) {
        self.previous.remove(device);
        self.last.remove(device);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn moved(sent: u64, received: u64) -> DeviceTraffic {
        DeviceTraffic {
            sent,
            received,
            connections: 1,
        }
    }

    fn snapshot(pairs: &[(&str, DeviceTraffic)]) -> HashMap<String, DeviceTraffic> {
        pairs
            .iter()
            .map(|(name, traffic)| ((*name).to_string(), *traffic))
            .collect()
    }

    #[test]
    fn the_first_sample_has_nothing_to_compare_against() {
        let mut rates = Rates::new();
        let now = Instant::now();
        let result = rates.sample(now, snapshot(&[("a", moved(1_000_000, 0))]));
        assert_eq!(result.get("a"), Some(&Rate::default()));
    }

    #[test]
    fn a_rate_is_the_difference_over_the_measured_interval() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));

        // Ten megabytes in two seconds is five megabytes a second, whatever
        // the caller's timer thinks its own period is.
        let result = rates.sample(
            start + Duration::from_secs(2),
            snapshot(&[("a", moved(10_000_000, 0))]),
        );
        assert_eq!(result["a"].send, 5_000_000.0);
        assert_eq!(result["a"].receive, 0.0);
    }

    /// The bug this module exists to prevent.
    #[test]
    fn a_late_poll_does_not_inflate_the_speed() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));

        // The caller meant to poll every second and was four seconds late.
        // Dividing by the intended period would report four times the truth.
        let result = rates.sample(
            start + Duration::from_secs(4),
            snapshot(&[("a", moved(4_000_000, 0))]),
        );
        assert_eq!(result["a"].send, 1_000_000.0);
    }

    #[test]
    fn both_directions_are_counted_separately() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));
        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(300, 700))]),
        );
        assert_eq!(result["a"].send, 300.0);
        assert_eq!(result["a"].receive, 700.0);
    }

    #[test]
    fn two_polls_a_moment_apart_keep_the_previous_answer() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));
        rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(1_000, 0))]),
        );

        let result = rates.sample(
            start + Duration::from_millis(1_010),
            snapshot(&[("a", moved(1_005, 0))]),
        );
        assert_eq!(
            result["a"].send, 1_000.0,
            "a sliver of an interval is not a measurement"
        );
    }

    #[test]
    fn the_baseline_survives_an_interval_too_short_to_use() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));
        // Ignored, and must not become the new baseline.
        rates.sample(
            start + Duration::from_millis(10),
            snapshot(&[("a", moved(10, 0))]),
        );

        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(1_000, 0))]),
        );
        assert_eq!(
            result["a"].send, 1_000.0,
            "measured across the whole second, not the remainder of it"
        );
    }

    #[test]
    fn an_idle_device_reports_zero_rather_than_its_total() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(9_000_000, 0))]));
        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(9_000_000, 0))]),
        );
        assert_eq!(result["a"].send, 0.0);
    }

    /// A counter can only go backwards through a bug, since revoking a device
    /// forgets its key outright. So the honest answer is zero for that one
    /// interval — not a negative rate, and not the new total either, which
    /// would be guessing that the counter restarted rather than jumped.
    #[test]
    fn a_counter_that_went_backwards_reports_zero_and_then_recovers() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(5_000, 5_000))]));

        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(10, 10))]),
        );
        assert_eq!(result["a"].send, 0.0);
        assert_eq!(result["a"].receive, 0.0);

        // Re-baselined: the interval after it measures normally again.
        let result = rates.sample(
            start + Duration::from_secs(2),
            snapshot(&[("a", moved(1_010, 10))]),
        );
        assert_eq!(result["a"].send, 1_000.0);
    }

    #[test]
    fn a_device_that_appeared_between_samples_counts_from_zero() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));
        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(0, 0)), ("b", moved(2_000, 0))]),
        );
        assert_eq!(result["b"].send, 2_000.0);
    }

    #[test]
    fn a_device_that_vanished_is_not_reported_at_all() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0)), ("b", moved(0, 0))]));
        let result = rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(1, 0))]),
        );
        assert!(!result.contains_key("b"));
    }

    #[test]
    fn forgetting_a_device_clears_both_its_baseline_and_its_rate() {
        let mut rates = Rates::new();
        let start = Instant::now();
        rates.sample(start, snapshot(&[("a", moved(0, 0))]));
        rates.sample(
            start + Duration::from_secs(1),
            snapshot(&[("a", moved(1_000, 0))]),
        );
        assert_eq!(rates.of("a").send, 1_000.0);

        rates.forget("a");
        assert_eq!(rates.of("a"), Rate::default());
    }

    #[test]
    fn asking_about_an_unknown_device_is_zero_not_a_panic() {
        let rates = Rates::new();
        assert_eq!(rates.of("nobody"), Rate::default());
    }
}
