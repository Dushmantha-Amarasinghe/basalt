//! The library: what a scan found, and how it changes when the drive does.
//!
//! Rebuilt from scratch on every scan rather than patched incrementally, and
//! that is the whole answer to "remove what is no longer there". A scan
//! produces the complete truth about the drive; anything absent from it is gone
//! by construction. Incremental removal would mean maintaining a second idea of
//! what exists and keeping it in step, which is exactly the kind of bookkeeping
//! that drifts.
//!
//! What *is* preserved across a rebuild is everything a person contributed —
//! artwork fetched, and later their corrections — keyed by an id derived from
//! the title rather than the path. So reorganising a drive keeps the poster.
//!
//! Stored as JSON beside the host's config. A personal library is thousands of
//! items, and the whole index is a few hundred kilobytes; a database earns its
//! place at a scale this will not reach. (A mirrored SQLite index was cut from
//! this project once before, for mirroring *every file*. This is not that: it
//! holds derived metadata for media only.)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use basalt_proto::msg::{Episode, LibraryItem, LibraryKind, Season};
use serde::{Deserialize, Serialize};

use super::parse::{self, Parsed};
use crate::vault::Vault;

/// Directories descended into during one scan.
///
/// A ceiling rather than a depth limit: a library is nested three or four deep
/// but arbitrarily wide, and the thing worth preventing is a scan that never
/// ends on a drive full of something unexpected.
pub const MAX_DIRS: usize = 40_000;

/// And a clock, because the count is not the thing that hurts.
///
/// Forty thousand directories took twenty seconds on a fast SSD and would take
/// minutes on the external drive this is actually for — on the kind of old
/// laptop it runs on, that is the machine being unusable rather than a scan
/// being slow. Whichever limit is reached first stops the walk, and what has
/// been found by then is kept: a partial library is worth having, and the next
/// scan starts again from the top anyway.
pub const MAX_DURATION: std::time::Duration = std::time::Duration::from_secs(90);

/// Files smaller than this are not features.
///
/// Trailers and junk hide under a hundred megabytes; so does a home video, but
/// a home video is not what this screen is for. Files below the line stay
/// browsable in Files, they are simply not filed as cinema.
pub const MIN_FEATURE_BYTES: u64 = 50 * 1024 * 1024;

/// The whole index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    /// Bumped whenever the contents change, so a client can poll cheaply.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub items: Vec<LibraryItem>,
    /// Unix seconds of the last completed scan.
    #[serde(default)]
    pub scanned_at: i64,
}

impl Library {
    pub fn find(&self, id: &str) -> Option<&LibraryItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Replaces the contents, bumping the revision only if anything differs.
    ///
    /// The comparison matters: a scan that finds nothing new must not make
    /// every connected client reload, and a periodic rescan of an untouched
    /// drive is the common case.
    pub fn replace(&mut self, items: Vec<LibraryItem>, now: i64) -> bool {
        self.scanned_at = now;
        if self.items == items {
            return false;
        }
        self.items = items;
        self.revision += 1;
        true
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // A corrupt index costs a rescan, nothing more, so there is no
                // reason to stop the host over it the way a corrupt identity
                // would.
                tracing::warn!("the library index did not parse ({e}); rebuilding");
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
}

/// Where the index lives: beside `host.json`, not on the drive.
///
/// Keyed by the vault's path so swapping one USB drive for another does not
/// silently merge two libraries into one.
pub fn index_path(config_dir: &Path, vault_root: &Path) -> PathBuf {
    let key = blake3::hash(vault_root.to_string_lossy().as_bytes()).to_hex();
    config_dir.join(format!("library-{}.json", &key[..16]))
}

/// One file found on the drive, before it becomes part of anything.
#[derive(Debug, Clone)]
struct Found {
    path: String,
    size: u64,
    mtime: i64,
    parsed: Parsed,
}

/// Walks the vault and builds the index.
///
/// Synchronous and blocking: it is disk-bound and belongs on a blocking thread,
/// not in the async runtime.
pub fn scan(vault: &Vault) -> Vec<LibraryItem> {
    scan_within(vault, MAX_DIRS, MAX_DURATION)
}

/// The walk, with its limits passed in so a test can reach them.
///
/// **Breadth-first, and that is the point.** This used to be a `Vec` with
/// `pop()`, which is last-in-first-out: the walk dived into whichever top-level
/// folder sorted last and followed it all the way down. Combined with a ceiling
/// on directories, that meant one deep branch could swallow the entire budget
/// before the walk ever looked at the others — a drive with films in `Films`
/// and a deep tree in `Projects` reported no films at all, which is exactly
/// what was seen on a real drive: forty thousand directories visited, four
/// items found.
///
/// A queue fixes it for the shape libraries actually have. Media sits three or
/// four levels down, so breadth-first reaches all of it early and the ceiling
/// now truncates the deepest corners — the part least likely to hold a film —
/// rather than everything after the first one.
pub fn scan_within(
    vault: &Vault,
    max_dirs: usize,
    max_duration: std::time::Duration,
) -> Vec<LibraryItem> {
    let started = std::time::Instant::now();
    let mut found = Vec::new();
    let mut queue = std::collections::VecDeque::from([String::new()]);
    let mut visited = 0usize;

    while let Some(dir) = queue.pop_front() {
        visited += 1;
        if visited > max_dirs {
            tracing::warn!(
                "stopped after {max_dirs} directories with {} still to look at",
                queue.len()
            );
            break;
        }
        // Checked every so often rather than every directory: the clock itself
        // is cheap, but not as cheap as not reading it.
        if visited.is_multiple_of(64) && started.elapsed() > max_duration {
            tracing::warn!(
                "stopped after {:?} with {} directories still to look at",
                started.elapsed(),
                queue.len()
            );
            break;
        }

        // Every read goes through the vault, so the scan cannot reach outside
        // the one security boundary in the system.
        let entries = match vault.list(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                // One unreadable folder is normal and not worth a line each.
                // The root failing is the whole scan failing, and saying so is
                // the difference between "no films on this drive" and "this
                // host cannot read the drive at all" — which looked identical
                // from the outside.
                if dir.is_empty() {
                    tracing::warn!("the vault root could not be read: {e}");
                }
                continue;
            }
        };

        if dir.is_empty() {
            // Reported because an empty root and a drive full of films look
            // the same in the result: nothing found. This line tells them
            // apart without anyone having to guess which it was.
            tracing::info!("the vault root has {} entries", entries.len());
        }

        for entry in entries {
            let path = if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            };

            match entry.kind {
                basalt_proto::msg::EntryKind::Dir => {
                    if !parse::is_extra(&path) && !parse::is_system(&path) {
                        queue.push_back(path);
                    }
                }
                basalt_proto::msg::EntryKind::File => {
                    if entry.size < MIN_FEATURE_BYTES {
                        continue;
                    }
                    if let Some(parsed) = parse::parse(&path) {
                        found.push(Found {
                            path,
                            size: entry.size,
                            mtime: entry.mtime,
                            parsed,
                        });
                    }
                }
            }
        }
    }

    let items = group(found);
    // Logged because this is the one expensive thing the host does, and when
    // somebody says it has stopped responding this line is what says whether a
    // scan was the reason.
    tracing::info!(
        "scanned {visited} directories in {:?}, found {} items",
        started.elapsed(),
        items.len()
    );
    items
}

/// A stable id from the title and year.
///
/// Deliberately not the path: moving a film to a different folder must not
/// orphan its artwork or its resume point, and it is the same film.
pub fn item_id(kind: LibraryKind, title: &str, year: Option<u16>) -> String {
    let prefix = match kind {
        LibraryKind::Film => "f",
        LibraryKind::Series => "s",
    };
    let key = format!(
        "{prefix}:{}:{}",
        title.to_lowercase(),
        year.map_or(String::new(), |y| y.to_string())
    );
    format!("{prefix}{}", &blake3::hash(key.as_bytes()).to_hex()[..16])
}

/// Folds the files found into films and series.
fn group(found: Vec<Found>) -> Vec<LibraryItem> {
    let mut films: Vec<LibraryItem> = Vec::new();
    // Ordered so the output is stable between scans, which keeps the revision
    // from bumping just because a hash map iterated differently.
    let mut series: BTreeMap<String, LibraryItem> = BTreeMap::new();

    for entry in found {
        let parsed = entry.parsed;
        if parsed.title.is_empty() {
            continue;
        }

        if parsed.is_episode() {
            let id = item_id(LibraryKind::Series, &parsed.title, None);
            let item = series.entry(id.clone()).or_insert_with(|| LibraryItem {
                id,
                kind: LibraryKind::Series,
                title: parsed.title.clone(),
                year: parsed.year,
                path: None,
                size: 0,
                added: 0,
                seasons: Vec::new(),
                confidence: parsed.confidence,
                has_art: false,
            });

            // A series is only as trustworthy as its least certain episode.
            item.confidence = item.confidence.min(parsed.confidence);
            item.year = item.year.or(parsed.year);
            // Size is summed at the end, from the episodes that survive
            // deduplication — adding it here would count a second copy twice.
            item.added = item.added.max(entry.mtime);

            let number = parsed.season.unwrap_or(0);
            let season = match item.seasons.iter_mut().find(|s| s.number == number) {
                Some(season) => season,
                None => {
                    item.seasons.push(Season {
                        number,
                        episodes: Vec::new(),
                    });
                    item.seasons.last_mut().expect("just pushed")
                }
            };
            season.episodes.push(Episode {
                number: parsed.episode.unwrap_or(0),
                path: entry.path,
                title: None,
                size: entry.size,
                added: entry.mtime,
            });
        } else {
            let id = item_id(LibraryKind::Film, &parsed.title, parsed.year);
            // The same film at two qualities is one film. The larger file wins
            // as the one to play; both stay visible in Files.
            match films.iter_mut().find(|f| f.id == id) {
                Some(existing) => {
                    existing.added = existing.added.max(entry.mtime);
                    if entry.size > existing.size {
                        existing.size = entry.size;
                        existing.path = Some(entry.path);
                    }
                }
                None => films.push(LibraryItem {
                    id,
                    kind: LibraryKind::Film,
                    title: parsed.title,
                    year: parsed.year,
                    path: Some(entry.path),
                    size: entry.size,
                    added: entry.mtime,
                    seasons: Vec::new(),
                    confidence: parsed.confidence,
                    has_art: false,
                }),
            }
        }
    }

    let mut items: Vec<LibraryItem> = films;
    for mut item in series.into_values() {
        for season in &mut item.seasons {
            // By number, then largest first, so the copy kept below is the
            // better one — the same rule films already used.
            season.episodes.sort_by(|a, b| {
                a.number
                    .cmp(&b.number)
                    .then(b.size.cmp(&a.size))
                    .then(a.path.cmp(&b.path))
            });
            // The same episode twice is one episode.
            //
            // This deduplicated on *path*, which can never collide, so a
            // second copy of an episode anywhere on the drive got a row of its
            // own: a sixteen-episode season listed seventeen, with one of them
            // appearing twice. Both files are still there in Files; the
            // library shows the episode once and plays the larger.
            season.episodes.dedup_by(|a, b| a.number == b.number);
        }
        item.seasons.sort_by_key(|s| s.number);
        item.size = item
            .seasons
            .iter()
            .flat_map(|season| &season.episodes)
            .map(|episode| episode.size)
            .sum();
        items.push(item);
    }

    // A stable order, so two scans of an unchanged drive compare equal and the
    // revision does not move.
    items.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.year.cmp(&b.year))
            .then(a.id.cmp(&b.id))
    });
    items
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "basalt-index-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// Writes a file big enough to count as a feature.
    fn put(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MIN_FEATURE_BYTES + 1).unwrap();
    }

    fn vault_of(dir: &TempDir) -> Vault {
        Vault::open(&dir.0, "Test").unwrap()
    }

    /// A deep branch must not be able to eat the whole budget.
    ///
    /// The real report: a drive with films on it indexed to nothing. The walk
    /// was last-in-first-out, so it dived into the top-level folder that sorted
    /// last and followed it down until the directory ceiling stopped it — the
    /// films, one level from the root, were never reached. On the drive it was
    /// found on that was forty thousand directories visited and four items
    /// found.
    ///
    /// `Zzz` sorts after `Films`, so under the old order it was taken first.
    #[test]
    fn a_deep_branch_does_not_hide_the_films_beside_it() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.1080p.BluRay-SPARKS.mkv");

        // Deeper than the budget, so a walk that goes down before it goes
        // across can never come back up to look at `Films`.
        let mut deep = dir.0.join("Zzz");
        for level in 0..40 {
            deep = deep.join(format!("level-{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();

        let items = scan_within(&vault_of(&dir), 10, std::time::Duration::from_secs(30));
        assert_eq!(
            items.len(),
            1,
            "the film one level down has to be found before a deep branch, got {items:?}"
        );
        assert_eq!(items[0].title, "Arrival");
    }

    /// A spare copy of an episode is not a second episode.
    ///
    /// Seen on a real drive: a season folder of sixteen episodes listed
    /// seventeen, because one episode also existed loose in an unrelated
    /// folder. Films already collapsed duplicates and episodes did not.
    #[test]
    fn the_same_episode_twice_is_listed_once() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Shows/Outlander/Season 01/Outlander S01E09 The Reckoning.mkv",
        );
        // A second copy elsewhere, and deliberately the larger of the two.
        let spare = dir.0.join("Spare/Outlander S01E09 The Reckoning.mkv");
        std::fs::create_dir_all(spare.parent().unwrap()).unwrap();
        std::fs::File::create(&spare)
            .unwrap()
            .set_len(MIN_FEATURE_BYTES * 3)
            .unwrap();

        let items = scan(&vault_of(&dir));
        assert_eq!(items.len(), 1, "one series, got {items:?}");
        let episodes = &items[0].seasons[0].episodes;
        assert_eq!(episodes.len(), 1, "one episode, got {episodes:?}");
        // The larger copy is the one worth playing.
        assert!(
            episodes[0].path.starts_with("Spare/"),
            "got {:?}",
            episodes[0].path
        );
        // And the series size counts it once.
        assert_eq!(items[0].size, MIN_FEATURE_BYTES * 3);
    }

    #[test]
    fn films_and_series_are_told_apart() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.1080p.BluRay-SPARKS.mkv");
        put(&dir.0, "Shows/Breaking Bad/Season 01/S01E01.mkv");
        put(&dir.0, "Shows/Breaking Bad/Season 01/S01E02.mkv");

        let items = scan(&vault_of(&dir));
        assert_eq!(items.len(), 2);

        let film = items.iter().find(|i| i.kind == LibraryKind::Film).unwrap();
        assert_eq!(film.title, "Arrival");
        assert_eq!(film.year, Some(2016));

        let show = items
            .iter()
            .find(|i| i.kind == LibraryKind::Series)
            .unwrap();
        assert_eq!(show.title, "Breaking Bad");
        assert_eq!(show.seasons.len(), 1);
        assert_eq!(show.seasons[0].episodes.len(), 2);
    }

    #[test]
    fn episodes_of_one_series_gather_under_it_across_seasons() {
        let dir = temp_dir();
        put(&dir.0, "Shows/The Wire/Season 01/S01E01.mkv");
        put(&dir.0, "Shows/The Wire/Season 02/S02E01.mkv");
        put(&dir.0, "Shows/The Wire/Season 02/S02E02.mkv");

        let items = scan(&vault_of(&dir));
        assert_eq!(items.len(), 1, "one series, not three");
        assert_eq!(items[0].seasons.len(), 2);
        assert_eq!(items[0].seasons[1].episodes.len(), 2);
    }

    #[test]
    fn episodes_are_ordered_by_number_not_by_the_order_found() {
        let dir = temp_dir();
        for n in [3, 1, 10, 2] {
            put(&dir.0, &format!("Shows/X/Season 01/S01E{n:02}.mkv"));
        }
        let items = scan(&vault_of(&dir));
        let numbers: Vec<u16> = items[0].seasons[0]
            .episodes
            .iter()
            .map(|e| e.number)
            .collect();
        assert_eq!(numbers, [1, 2, 3, 10], "10 sorts after 2, not after 1");
    }

    #[test]
    fn the_same_film_at_two_qualities_is_one_item() {
        let dir = temp_dir();
        let big = dir.0.join("Films/Arrival.2016.2160p.mkv");
        std::fs::create_dir_all(big.parent().unwrap()).unwrap();
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MIN_FEATURE_BYTES * 4)
            .unwrap();
        put(&dir.0, "Films/Arrival.2016.1080p.mkv");

        let items = scan(&vault_of(&dir));
        assert_eq!(items.len(), 1);
        assert!(
            items[0].path.as_deref().unwrap().contains("2160p"),
            "the better copy is the one to play"
        );
    }

    #[test]
    fn extras_and_small_files_are_left_out() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival (2016)/Arrival.2016.mkv");
        put(&dir.0, "Films/Arrival (2016)/Featurettes/making-of.mkv");

        // Under the size floor, so not a feature however it is named.
        let small = dir.0.join("Films/Tiny.2020.mkv");
        std::fs::write(&small, b"x").unwrap();

        let items = scan(&vault_of(&dir));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Arrival");
    }

    #[test]
    fn subtitles_and_artwork_beside_a_film_are_ignored() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        put(&dir.0, "Films/Arrival.2016.srt");
        put(&dir.0, "Films/poster.jpg");

        assert_eq!(scan(&vault_of(&dir)).len(), 1);
    }

    #[test]
    fn an_empty_drive_indexes_to_nothing_rather_than_failing() {
        let dir = temp_dir();
        assert!(scan(&vault_of(&dir)).is_empty());
    }

    // -----------------------------------------------------------------------
    // Reindexing — the part the user asked for by name
    // -----------------------------------------------------------------------

    /// A rescan is the complete truth, so anything deleted is gone by
    /// construction rather than by remembering to remove it.
    #[test]
    fn a_deleted_film_disappears_on_the_next_scan() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        put(&dir.0, "Films/Dune.2021.mkv");
        assert_eq!(scan(&vault_of(&dir)).len(), 2);

        std::fs::remove_file(dir.0.join("Films/Dune.2021.mkv")).unwrap();
        let after = scan(&vault_of(&dir));
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].title, "Arrival");
    }

    #[test]
    fn a_deleted_episode_leaves_the_series_with_the_rest() {
        let dir = temp_dir();
        put(&dir.0, "Shows/X/Season 01/S01E01.mkv");
        put(&dir.0, "Shows/X/Season 01/S01E02.mkv");
        std::fs::remove_file(dir.0.join("Shows/X/Season 01/S01E02.mkv")).unwrap();

        let items = scan(&vault_of(&dir));
        assert_eq!(items[0].seasons[0].episodes.len(), 1);
    }

    #[test]
    fn a_series_whose_last_episode_went_disappears_entirely() {
        let dir = temp_dir();
        put(&dir.0, "Shows/X/Season 01/S01E01.mkv");
        assert_eq!(scan(&vault_of(&dir)).len(), 1);

        std::fs::remove_file(dir.0.join("Shows/X/Season 01/S01E01.mkv")).unwrap();
        assert!(scan(&vault_of(&dir)).is_empty());
    }

    #[test]
    fn a_new_file_appears_on_the_next_scan() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        put(&dir.0, "Films/Dune.2021.mkv");
        assert_eq!(scan(&vault_of(&dir)).len(), 2);
    }

    /// Moving a file must not orphan the artwork or the resume point: it is the
    /// same film, so it keeps its id.
    #[test]
    fn moving_a_film_keeps_its_identity() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        let before = scan(&vault_of(&dir))[0].id.clone();

        std::fs::create_dir_all(dir.0.join("Archive")).unwrap();
        std::fs::rename(
            dir.0.join("Films/Arrival.2016.mkv"),
            dir.0.join("Archive/Arrival.2016.mkv"),
        )
        .unwrap();

        let after = scan(&vault_of(&dir));
        assert_eq!(after[0].id, before);
        assert_eq!(after[0].path.as_deref(), Some("Archive/Arrival.2016.mkv"));
    }

    // -----------------------------------------------------------------------
    // Revisions
    // -----------------------------------------------------------------------

    #[test]
    fn a_scan_that_found_nothing_new_does_not_bump_the_revision() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        let vault = vault_of(&dir);

        let mut library = Library::default();
        assert!(library.replace(scan(&vault), 1));
        let revision = library.revision;

        assert!(
            !library.replace(scan(&vault), 2),
            "an unchanged drive must not make every client reload"
        );
        assert_eq!(library.revision, revision);
        assert_eq!(library.scanned_at, 2, "but the scan time still moves");
    }

    #[test]
    fn a_changed_drive_bumps_the_revision() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        let vault = vault_of(&dir);

        let mut library = Library::default();
        library.replace(scan(&vault), 1);
        let revision = library.revision;

        put(&dir.0, "Films/Dune.2021.mkv");
        assert!(library.replace(scan(&vault), 2));
        assert_eq!(library.revision, revision + 1);
    }

    #[test]
    fn scanning_twice_produces_the_same_order() {
        let dir = temp_dir();
        for name in ["Zulu.1964", "Arrival.2016", "Dune.2021", "Alien.1979"] {
            put(&dir.0, &format!("Films/{name}.mkv"));
        }
        let vault = vault_of(&dir);
        let first: Vec<String> = scan(&vault).into_iter().map(|i| i.id).collect();
        let second: Vec<String> = scan(&vault).into_iter().map(|i| i.id).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn items_come_back_in_alphabetical_order() {
        let dir = temp_dir();
        for name in ["Zulu.1964", "Arrival.2016", "dune.2021"] {
            put(&dir.0, &format!("Films/{name}.mkv"));
        }
        let titles: Vec<String> = scan(&vault_of(&dir)).into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["Arrival", "dune", "Zulu"], "case-insensitive");
    }

    // -----------------------------------------------------------------------
    // Identity and storage
    // -----------------------------------------------------------------------

    #[test]
    fn two_drives_do_not_share_an_index_file() {
        let dir = temp_dir();
        let a = index_path(&dir.0, Path::new(r"E:\"));
        let b = index_path(&dir.0, Path::new(r"F:\"));
        assert_ne!(a, b, "swapping USB drives must not merge two libraries");
    }

    #[test]
    fn the_index_survives_a_round_trip_to_disk() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival.2016.mkv");
        let path = dir.0.join("library.json");

        let mut library = Library::default();
        library.replace(scan(&vault_of(&dir)), 99);
        library.save(&path).unwrap();

        let back = Library::load(&path);
        assert_eq!(back.revision, library.revision);
        assert_eq!(back.items, library.items);
        assert_eq!(back.scanned_at, 99);
    }

    #[test]
    fn a_corrupt_index_rebuilds_rather_than_stopping_the_host() {
        let dir = temp_dir();
        let path = dir.0.join("library.json");
        std::fs::write(&path, b"{ not json").unwrap();

        let library = Library::load(&path);
        assert!(library.items.is_empty());
        assert_eq!(library.revision, 0);
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = temp_dir();
        let path = dir.0.join("library.json");
        Library::default().save(&path).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "found {leftovers:?}");
    }

    #[test]
    fn a_film_and_a_series_of_the_same_name_get_different_ids() {
        assert_ne!(
            item_id(LibraryKind::Film, "Fargo", Some(1996)),
            item_id(LibraryKind::Series, "Fargo", Some(1996))
        );
    }

    #[test]
    fn identity_ignores_case_so_a_rename_does_not_orphan_artwork() {
        assert_eq!(
            item_id(LibraryKind::Film, "Arrival", Some(2016)),
            item_id(LibraryKind::Film, "arrival", Some(2016))
        );
    }
}
