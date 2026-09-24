//! Where each file has been watched to.
//!
//! On the host, not on the device doing the watching. That is the whole point:
//! a resume point that does not follow you from the laptop to the living room
//! is a bookmark, not a Continue watching. It also means a client reinstall
//! loses nothing.
//!
//! Keyed by vault-relative path rather than by library item, because an episode
//! is watched and a series is not — and because a file that the index has never
//! recognised is still something somebody can be halfway through.
//!
//! Bounded. A drive browsed for years would otherwise accumulate an entry per
//! file ever opened, so the oldest are dropped once there are too many. What is
//! lost is a resume point for something untouched in months, which is the least
//! costly thing here to lose.
//!
//! **Shared, or one per device — and always both.** A household may want one
//! history, so a film started on the laptop is finished on the television; or
//! a history each, so one person's episodes do not move another's place. The
//! host's owner chooses. Every update is written to the shared history *and*
//! to the device's own, and the setting only decides which one a device is
//! shown — so switching either way loses nothing and needs no migration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use basalt_proto::msg::Watched;
use serde::{Deserialize, Serialize};

/// Entries kept before the oldest are dropped, in each history.
pub const MAX_ENTRIES: usize = 2_000;

/// Device histories kept before the least recently used is dropped.
///
/// A host is paired with a household's devices, not hundreds of them; this is
/// a ceiling on a file that anything on the network could otherwise grow.
pub const MAX_DEVICES: usize = 64;

/// Everything watched on one drive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    /// The history every device shares. Named as it was before devices had
    /// histories of their own, so an older file still reads.
    #[serde(default)]
    entries: History,
    /// Each device's own history, by its key — see `Device::key`.
    #[serde(default)]
    devices: HashMap<String, History>,
}

/// One history: where each file was watched to.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct History(HashMap<String, Watched>);

impl Progress {
    /// Where this lives: beside the host's config, keyed by drive.
    pub fn path_for(config_dir: &Path, vault_root: &Path) -> PathBuf {
        let key = blake3::hash(vault_root.to_string_lossy().as_bytes()).to_hex();
        config_dir.join(format!("progress-{}.json", &key[..16]))
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // Losing resume points costs a little convenience and nothing
                // else, so there is no reason to stop the host over it.
                tracing::warn!("watch progress did not parse ({e}); starting over");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec(self)?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, path)
    }

    /// Records where something got to, in the shared history.
    pub fn record(&mut self, update: Watched, now: i64) {
        self.record_for(None, update, now);
    }

    /// Records where something got to, in the shared history and — when a
    /// device is named — in that device's own.
    ///
    /// Finishing something *keeps* the entry rather than dropping it: "watched"
    /// is worth knowing, and it is what stops a finished episode reappearing in
    /// Continue watching every time the list is rebuilt.
    pub fn record_for(&mut self, device: Option<&str>, mut update: Watched, now: i64) {
        if update.path.trim().is_empty() {
            return;
        }
        update.fraction = update.fraction.clamp(0.0, 1.0);
        update.position = update.position.max(0.0);
        update.duration = update.duration.max(0.0);
        update.updated_at = now;

        self.entries.record(update.clone());
        if let Some(device) = device {
            if !self.devices.contains_key(device) {
                self.make_room_for_a_device();
            }
            self.devices
                .entry(device.to_string())
                .or_default()
                .record(update);
        }
    }

    /// Forgets a file in the shared history.
    pub fn forget(&mut self, path: &str) -> bool {
        self.forget_for(None, path)
    }

    /// Forgets a file in the shared history and in a device's own.
    ///
    /// From both, because whichever one the device is being shown, "remove
    /// from Continue watching" has to mean it is gone from what it sees — and
    /// from what it would see if the setting changed tomorrow.
    pub fn forget_for(&mut self, device: Option<&str>, path: &str) -> bool {
        let mut removed = self.entries.0.remove(path).is_some();
        if let Some(history) = device.and_then(|d| self.devices.get_mut(d)) {
            removed |= history.0.remove(path).is_some();
        }
        removed
    }

    /// Drops entries for files that are no longer on the drive, everywhere.
    pub fn retain_existing(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let mut changed = self.entries.retain(&exists);
        for history in self.devices.values_mut() {
            changed |= history.retain(&exists);
        }
        changed
    }

    /// The shared history, newest first.
    pub fn all(&self) -> Vec<Watched> {
        self.entries.newest_first()
    }

    /// One device's own history, newest first. Empty for a device that has
    /// never watched anything.
    pub fn all_for(&self, device: &str) -> Vec<Watched> {
        self.devices
            .get(device)
            .map(History::newest_first)
            .unwrap_or_default()
    }

    pub fn get(&self, path: &str) -> Option<&Watched> {
        self.entries.0.get(path)
    }

    pub fn get_for(&self, device: &str, path: &str) -> Option<&Watched> {
        self.devices.get(device)?.0.get(path)
    }

    pub fn len(&self) -> usize {
        self.entries.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.0.is_empty()
    }

    /// How many devices have a history of their own.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Drops the device history used longest ago, if there are too many.
    fn make_room_for_a_device(&mut self) {
        while self.devices.len() >= MAX_DEVICES {
            let stalest = self
                .devices
                .iter()
                .min_by_key(|(key, history)| (history.newest(), (*key).clone()))
                .map(|(key, _)| key.clone());
            match stalest {
                Some(key) => {
                    self.devices.remove(&key);
                }
                None => return,
            }
        }
    }
}

impl History {
    fn record(&mut self, update: Watched) {
        // An external player only reveals a byte offset, and a player that
        // reads ahead reports further than it has played. Never let a weaker
        // signal pull a known position backwards within the same session.
        if let Some(existing) = self.0.get(&update.path)
            && update.duration == 0.0
            && existing.duration > 0.0
            && update.fraction < existing.fraction
        {
            return;
        }
        self.0.insert(update.path.clone(), update);
        self.prune();
    }

    fn retain(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.0.len();
        self.0.retain(|path, _| exists(path));
        self.0.len() != before
    }

    fn newest_first(&self) -> Vec<Watched> {
        let mut entries: Vec<Watched> = self.0.values().cloned().collect();
        entries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.path.cmp(&b.path))
        });
        entries
    }

    /// When anything in here was last watched.
    fn newest(&self) -> i64 {
        self.0.values().map(|w| w.updated_at).max().unwrap_or(0)
    }

    /// Keeps the newest [`MAX_ENTRIES`] and drops the rest.
    fn prune(&mut self) {
        if self.0.len() <= MAX_ENTRIES {
            return;
        }
        let mut by_age: Vec<(String, i64)> = self
            .0
            .iter()
            .map(|(path, w)| (path.clone(), w.updated_at))
            .collect();
        by_age.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        for (path, _) in by_age.into_iter().skip(MAX_ENTRIES) {
            self.0.remove(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watched(path: &str, fraction: f64) -> Watched {
        Watched {
            path: path.into(),
            fraction,
            position: 0.0,
            duration: 0.0,
            updated_at: 0,
        }
    }

    fn timed(path: &str, fraction: f64, duration: f64) -> Watched {
        Watched {
            path: path.into(),
            fraction,
            position: fraction * duration,
            duration,
            updated_at: 0,
        }
    }

    #[test]
    fn something_watched_is_remembered_and_can_be_read_back() {
        let mut progress = Progress::default();
        progress.record(watched("a.mkv", 0.4), 100);

        let entry = progress.get("a.mkv").expect("recorded");
        assert_eq!(entry.fraction, 0.4);
        assert_eq!(
            entry.updated_at, 100,
            "the host stamps the time, not the client"
        );
    }

    #[test]
    fn watching_further_replaces_the_earlier_point() {
        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.2, 7200.0), 100);
        progress.record(timed("a.mkv", 0.6, 7200.0), 200);

        assert_eq!(progress.get("a.mkv").unwrap().fraction, 0.6);
        assert_eq!(progress.len(), 1, "one file, one entry");
    }

    /// An external player reads ahead of what it is showing, and reports only
    /// a byte offset. That estimate must never drag a known position backwards.
    #[test]
    fn a_byte_estimate_does_not_overwrite_a_known_position_backwards() {
        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.60, 7200.0), 100);
        progress.record(watched("a.mkv", 0.45), 200);

        assert_eq!(
            progress.get("a.mkv").unwrap().fraction,
            0.60,
            "the weaker signal must not win"
        );
    }

    #[test]
    fn a_byte_estimate_further_on_is_still_accepted() {
        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.30, 7200.0), 100);
        progress.record(watched("a.mkv", 0.75), 200);
        assert_eq!(progress.get("a.mkv").unwrap().fraction, 0.75);
    }

    /// Starting a film again is a deliberate act and has to be possible.
    #[test]
    fn a_known_position_can_be_moved_back_by_a_player_that_knows_the_duration() {
        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.80, 7200.0), 100);
        progress.record(timed("a.mkv", 0.02, 7200.0), 200);
        assert_eq!(progress.get("a.mkv").unwrap().fraction, 0.02);
    }

    #[test]
    fn a_nonsense_fraction_is_brought_into_range() {
        let mut progress = Progress::default();
        progress.record(watched("a.mkv", 4.2), 1);
        assert_eq!(progress.get("a.mkv").unwrap().fraction, 1.0);

        progress.record(timed("b.mkv", -3.0, 10.0), 1);
        assert_eq!(progress.get("b.mkv").unwrap().fraction, 0.0);
    }

    #[test]
    fn an_empty_path_is_not_recorded() {
        let mut progress = Progress::default();
        progress.record(watched("", 0.5), 1);
        progress.record(watched("   ", 0.5), 1);
        assert!(progress.is_empty());
    }

    #[test]
    fn forgetting_removes_it_and_says_whether_there_was_anything() {
        let mut progress = Progress::default();
        progress.record(watched("a.mkv", 0.5), 1);

        assert!(progress.forget("a.mkv"));
        assert!(progress.get("a.mkv").is_none());
        assert!(
            !progress.forget("a.mkv"),
            "forgetting twice changes nothing"
        );
    }

    /// A finished film keeps its entry — that is what stops it reappearing in
    /// Continue watching every time the list is rebuilt.
    #[test]
    fn finishing_something_keeps_the_record() {
        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.99, 7200.0), 1);

        let entry = progress.get("a.mkv").expect("still recorded");
        assert!(entry.finished());
        assert!(!entry.in_progress());
    }

    #[test]
    fn entries_come_back_newest_first() {
        let mut progress = Progress::default();
        progress.record(watched("old.mkv", 0.5), 100);
        progress.record(watched("new.mkv", 0.5), 300);
        progress.record(watched("middle.mkv", 0.5), 200);

        let paths: Vec<String> = progress.all().into_iter().map(|w| w.path).collect();
        assert_eq!(paths, ["new.mkv", "middle.mkv", "old.mkv"]);
    }

    #[test]
    fn a_file_that_left_the_drive_leaves_the_record() {
        let mut progress = Progress::default();
        progress.record(watched("kept.mkv", 0.5), 1);
        progress.record(watched("gone.mkv", 0.5), 1);

        assert!(progress.retain_existing(|path| path == "kept.mkv"));
        assert_eq!(progress.len(), 1);
        assert!(progress.get("gone.mkv").is_none());

        assert!(
            !progress.retain_existing(|_| true),
            "nothing removed means nothing to save"
        );
    }

    /// Years of browsing must not grow an unbounded file.
    #[test]
    fn the_oldest_entries_are_dropped_once_there_are_too_many() {
        let mut progress = Progress::default();
        for i in 0..MAX_ENTRIES + 50 {
            progress.record(watched(&format!("f{i}.mkv"), 0.5), i as i64);
        }

        assert_eq!(progress.len(), MAX_ENTRIES);
        assert!(progress.get("f0.mkv").is_none(), "the oldest goes first");
        assert!(
            progress
                .get(&format!("f{}.mkv", MAX_ENTRIES + 49))
                .is_some(),
            "the newest stays"
        );
    }

    #[test]
    fn it_survives_a_round_trip_to_disk() {
        let dir = std::env::temp_dir().join(format!("basalt-progress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.json");

        let mut progress = Progress::default();
        progress.record(timed("a.mkv", 0.4, 7200.0), 100);
        progress.save(&path).unwrap();

        let back = Progress::load(&path);
        assert_eq!(back.get("a.mkv").unwrap().fraction, 0.4);
        assert_eq!(back.get("a.mkv").unwrap().duration, 7200.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_starts_over_rather_than_failing() {
        let dir = std::env::temp_dir().join(format!("basalt-progress-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.json");
        std::fs::write(&path, b"{ not json").unwrap();

        assert!(Progress::load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_drives_do_not_share_a_progress_file() {
        let dir = Path::new("/tmp");
        assert_ne!(
            Progress::path_for(dir, Path::new(r"E:\")),
            Progress::path_for(dir, Path::new(r"F:\"))
        );
    }

    // -----------------------------------------------------------------------
    // One history each
    // -----------------------------------------------------------------------

    #[test]
    fn a_device_has_its_own_history_and_the_shared_one_is_kept_too() {
        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), timed("a.mkv", 0.4, 7200.0), 100);
        progress.record_for(Some("tv"), timed("b.mkv", 0.7, 3600.0), 200);

        let laptop: Vec<String> = progress
            .all_for("laptop")
            .into_iter()
            .map(|w| w.path)
            .collect();
        let tv: Vec<String> = progress.all_for("tv").into_iter().map(|w| w.path).collect();
        assert_eq!(laptop, ["a.mkv"]);
        assert_eq!(tv, ["b.mkv"]);
        assert_eq!(
            progress.len(),
            2,
            "both went into the shared history as well"
        );
    }

    #[test]
    fn one_device_watching_further_does_not_move_anothers_place() {
        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), timed("a.mkv", 0.2, 7200.0), 100);
        progress.record_for(Some("tv"), timed("a.mkv", 0.9, 7200.0), 200);

        assert_eq!(progress.get_for("laptop", "a.mkv").unwrap().fraction, 0.2);
        assert_eq!(progress.get_for("tv", "a.mkv").unwrap().fraction, 0.9);
        assert_eq!(
            progress.get("a.mkv").unwrap().fraction,
            0.9,
            "shared: the latest"
        );
    }

    #[test]
    fn a_device_that_never_watched_anything_has_nothing() {
        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), watched("a.mkv", 0.5), 1);
        assert!(progress.all_for("phone").is_empty());
    }

    #[test]
    fn forgetting_on_a_device_clears_it_from_both_histories() {
        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), watched("a.mkv", 0.5), 1);
        assert!(progress.forget_for(Some("laptop"), "a.mkv"));
        assert!(progress.get("a.mkv").is_none());
        assert!(progress.get_for("laptop", "a.mkv").is_none());
    }

    #[test]
    fn a_file_that_left_the_drive_leaves_every_history() {
        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), watched("gone.mkv", 0.5), 1);
        progress.record_for(Some("laptop"), watched("kept.mkv", 0.5), 1);
        assert!(progress.retain_existing(|path| path == "kept.mkv"));
        assert!(progress.get_for("laptop", "gone.mkv").is_none());
        assert!(progress.get_for("laptop", "kept.mkv").is_some());
    }

    /// Anything on the network can pair when the PIN is off, so the number of
    /// histories has a ceiling like everything else here.
    #[test]
    fn the_device_used_longest_ago_goes_first_once_there_are_too_many() {
        let mut progress = Progress::default();
        for i in 0..MAX_DEVICES {
            progress.record_for(
                Some(&format!("d{i:03}")),
                watched("a.mkv", 0.5),
                i as i64 + 10,
            );
        }
        progress.record_for(Some("newcomer"), watched("a.mkv", 0.5), 10_000);

        assert_eq!(progress.device_count(), MAX_DEVICES);
        assert!(
            progress.all_for("d000").is_empty(),
            "the stalest was dropped"
        );
        assert!(!progress.all_for("newcomer").is_empty());
    }

    /// A file written before devices had histories still reads, as the
    /// shared history it always was.
    #[test]
    fn a_file_from_before_device_histories_still_reads() {
        let dir = std::env::temp_dir().join(format!("basalt-progress-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.json");
        std::fs::write(
            &path,
            br#"{"entries":{"a.mkv":{"path":"a.mkv","fraction":0.5,"position":60.0,"duration":120.0,"updatedAt":7}}}"#,
        )
        .unwrap();

        let progress = Progress::load(&path);
        assert_eq!(progress.get("a.mkv").map(|w| w.fraction), Some(0.5));
        assert_eq!(progress.device_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_histories_survive_a_round_trip_to_disk() {
        let dir = std::env::temp_dir().join(format!("basalt-progress-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.json");

        let mut progress = Progress::default();
        progress.record_for(Some("laptop"), timed("a.mkv", 0.4, 7200.0), 100);
        progress.save(&path).unwrap();

        let back = Progress::load(&path);
        assert_eq!(back.get_for("laptop", "a.mkv").unwrap().fraction, 0.4);
        assert_eq!(back.get("a.mkv").unwrap().fraction, 0.4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
