//! How fast a transfer is going now, and how fast to plan on.
//!
//! Two numbers, because they answer two questions. **The speed shown** should
//! be the speed now: it used to be the average since the transfer began, which
//! is steady and wrong — a file that had the link to itself for a minute kept
//! reporting that speed long after a second transfer had taken half of it, and
//! the title bar, measuring the link as it was, disagreed with it. **The time
//! left** should not jump about with every hiccup, so it is planned on a
//! longer window: long enough to be calm, short enough to notice within
//! seconds that the link has changed.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The window the shown speed is measured over. The title bar's readout uses
/// the same, so the two agree.
pub const CURRENT: Duration = Duration::from_secs(2);

/// The window the time left is planned on.
pub const STEADY: Duration = Duration::from_secs(10);

/// Anything shorter than this is not enough to call a rate.
const SHORTEST: Duration = Duration::from_millis(50);

/// Bytes moved against time, for one transfer.
#[derive(Debug, Default)]
pub struct Meter {
    /// When, and how far through the transfer it had got then.
    samples: VecDeque<(Instant, u64)>,
}

impl Meter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes how many bytes of the transfer are done as of `now`.
    ///
    /// The first report only starts the clock. A resumed upload reports the
    /// part already on the host in its first figure, and counting that would
    /// show a speed no link ever had.
    pub fn record(&mut self, now: Instant, transferred: u64) {
        self.samples.push_back((now, transferred));

        // Keep the window, plus the one sample just before it, which is what
        // the oldest end of the window is measured from.
        let Some(cutoff) = now.checked_sub(STEADY) else {
            return;
        };
        while self.samples.len() > 2 && self.samples[1].0 <= cutoff {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the last [`CURRENT`].
    pub fn current(&self, now: Instant) -> f64 {
        self.over(CURRENT, now)
    }

    /// Bytes per second over the last [`STEADY`], for planning the time left.
    pub fn steady(&self, now: Instant) -> f64 {
        self.over(STEADY, now)
    }

    /// Bytes per second over a window ending now.
    ///
    /// Measured from the last sample at or before the start of the window, so
    /// a stall inside the window counts against the speed — it did happen —
    /// while a transfer younger than the window is measured over its own life.
    fn over(&self, window: Duration, now: Instant) -> f64 {
        let Some(&(last_at, last_bytes)) = self.samples.back() else {
            return 0.0;
        };
        let cutoff = now.checked_sub(window);
        let base = self
            .samples
            .iter()
            .rev()
            .find(|(at, _)| cutoff.is_some_and(|c| *at <= c))
            .or(self.samples.front())
            .copied();
        let Some((base_at, base_bytes)) = base else {
            return 0.0;
        };

        // To `now`, not to the last report: a transfer that has stopped
        // reporting has slowed down, and the speed should say so.
        let elapsed = now.max(last_at).duration_since(base_at);
        if elapsed < SHORTEST || last_bytes < base_bytes {
            return 0.0;
        }
        (last_bytes - base_bytes) as f64 / elapsed.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    /// Feeds a meter a steady rate for a while, one report per quarter second.
    fn steady(
        meter: &mut Meter,
        from: Instant,
        seconds: f64,
        bytes_per_second: u64,
        start: u64,
    ) -> (Instant, u64) {
        let steps = (seconds * 4.0) as u32;
        let mut at = from;
        let mut done = start;
        for _ in 0..steps {
            at += Duration::from_millis(250);
            done += bytes_per_second / 4;
            meter.record(at, done);
        }
        (at, done)
    }

    fn near(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() <= expected * 0.05
    }

    #[test]
    fn a_steady_transfer_reads_its_speed() {
        let mut meter = Meter::new();
        let start = Instant::now();
        meter.record(start, 0);
        let (now, _) = steady(&mut meter, start, 12.0, 20 * MB, 0);
        assert!(near(meter.current(now), 20e6), "{}", meter.current(now));
        assert!(near(meter.steady(now), 20e6), "{}", meter.steady(now));
    }

    /// The report this exists for: a second transfer halves the first, and the
    /// first went on showing its old average.
    #[test]
    fn the_shown_speed_follows_a_change_within_seconds() {
        let mut meter = Meter::new();
        let start = Instant::now();
        meter.record(start, 0);
        let (at, done) = steady(&mut meter, start, 30.0, 20 * MB, 0);
        let (now, _) = steady(&mut meter, at, 3.0, 10 * MB, done);

        assert!(near(meter.current(now), 10e6), "now {}", meter.current(now));
        // The planning figure moves too, just more calmly.
        let planned = meter.steady(now);
        assert!(planned < 18e6 && planned > 10e6, "planned {planned}");
    }

    #[test]
    fn the_first_report_only_starts_the_clock() {
        let mut meter = Meter::new();
        let start = Instant::now();
        // A resumed upload: three quarters were already on the host.
        meter.record(start, 750 * MB);
        assert_eq!(meter.current(start), 0.0);
        meter.record(start + Duration::from_secs(1), 770 * MB);
        assert!(near(meter.current(start + Duration::from_secs(1)), 20e6));
    }

    #[test]
    fn a_transfer_that_stops_reporting_slows_down() {
        let mut meter = Meter::new();
        let start = Instant::now();
        meter.record(start, 0);
        let (at, _) = steady(&mut meter, start, 5.0, 20 * MB, 0);
        let later = at + Duration::from_secs(1);
        assert!(meter.current(later) < 15e6, "{}", meter.current(later));
    }

    #[test]
    fn nothing_yet_is_no_speed_rather_than_a_guess() {
        let meter = Meter::new();
        assert_eq!(meter.current(Instant::now()), 0.0);
        assert_eq!(meter.steady(Instant::now()), 0.0);
    }

    #[test]
    fn memory_stays_bounded_however_long_it_runs() {
        let mut meter = Meter::new();
        let start = Instant::now();
        meter.record(start, 0);
        steady(&mut meter, start, 600.0, 20 * MB, 0);
        // Ten seconds of quarter-second reports, plus the one before them.
        assert!(meter.samples.len() <= 42, "{}", meter.samples.len());
    }
}
