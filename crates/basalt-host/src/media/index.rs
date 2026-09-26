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

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use basalt_proto::msg::{Episode, LibraryItem, LibraryKind, Season};
use serde::{Deserialize, Serialize};

use basalt_catalog::{Catalog, Kind, Lookup};

use super::parse::{self, Candidate, Parsed};
use super::subs;
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

/// Subtitle files remembered during one scan.
///
/// A ceiling for the same reason the directory count has one: a drive nobody
/// organised for us might hold any number of these, and the matching below is
/// linear in this list for every video.
pub const MAX_SUBTITLES: usize = 20_000;

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

/// Confidence given to a title the catalogue confirms.
///
/// Above everything the parser can reach on shape alone, because this is the
/// only signal that is not a guess: somebody released a film called that.
pub const VERIFIED: u8 = 99;

/// Confidence given to a film newer than the catalogue.
///
/// Below [`basalt_proto::msg::CONFIDENT`] on purpose, so the interface marks it
/// as a guess rather than asserting it.
pub const UNVERIFIED: u8 = 60;

/// Decides whether a candidate belongs in the library, and how sure to be.
///
/// **Series** are taken on their shape. `SxxExx` and a `Season NN` folder are
/// strong evidence of their own, and plenty of real series — regional ones,
/// anime under a romanised title, things somebody recorded off air — will
/// never be in a catalogue. The catalogue only raises confidence here.
///
/// **Films** have to exist. Three things, in order:
///
/// 1. Something besides the title says "film": a year, a `Movies` folder, or
///    release tags. Without one, a video is a video, as it always was.
/// 2. The catalogue knows a film by that title from about that year. That is
///    what keeps meeting recordings out: *Literature Review* was never
///    released, whatever folder it is in.
/// 3. One-word titles need more than a year. *Home* and *Meeting* are both
///    real, recent films, so `Home 2025.mp4` among somebody's videos has to
///    show something a home video would not — release tags, a library folder,
///    or a folder named after it.
///
/// The one exception: a film from the catalogue's own year or later that
/// carries release tags is kept, marked as unsure. It may simply be newer than
/// the snapshot, and dropping a film somebody just downloaded would be the
/// wrong way to be cautious.
///
/// Without a catalogue at all, the old rule stands: a year or a library folder.
pub fn decide(candidate: Candidate, catalog: Option<&Catalog>) -> Option<Parsed> {
    let Candidate {
        mut parsed,
        evidence,
    } = candidate;

    if parsed.is_episode() {
        if let Some(catalog) = catalog
            && catalog.find(Kind::Series, &parsed.title, parsed.year) == Lookup::Found
        {
            parsed.confidence = parsed.confidence.max(VERIFIED);
        }
        return Some(parsed);
    }

    let Some(catalog) = catalog else {
        return (parsed.year.is_some() || evidence.media_folder).then_some(parsed);
    };

    let corroborated = parsed.year.is_some() || evidence.media_folder || evidence.release_markers;
    if !corroborated {
        return None;
    }

    let one_word = parsed.title.split_whitespace().count() < 2;
    let beyond_year = evidence.media_folder || evidence.release_markers || evidence.named_folder;

    let found = catalog.find(Kind::Film, &parsed.title, parsed.year) == Lookup::Found;
    if found && (!one_word || beyond_year) {
        parsed.confidence = parsed.confidence.max(VERIFIED);
        return Some(parsed);
    }

    let newer_than_catalog = parsed
        .year
        .is_some_and(|year| year + 1 >= catalog.snapshot_year());
    if newer_than_catalog && evidence.release_markers {
        parsed.confidence = parsed.confidence.min(UNVERIFIED);
        return Some(parsed);
    }
    None
}

/// Reads one path into something the library can hold, or nothing.
fn identify(path: &str, catalog: Option<&Catalog>) -> Option<Parsed> {
    decide(parse::candidate(path)?, catalog)
}

/// Walks the vault and builds the index.
///
/// Synchronous and blocking: it is disk-bound and belongs on a blocking thread,
/// not in the async runtime.
pub fn scan(vault: &Vault) -> Vec<LibraryItem> {
    scan_within(vault, MAX_DIRS, MAX_DURATION, Catalog::bundled())
}

/// Everything one walk of the drive found.
#[derive(Debug, Default)]
pub struct Walk {
    /// Films and series, when recognising them is on.
    pub items: Vec<LibraryItem>,
    /// Every media file, for the collections.
    pub gathered: crate::media::collect::Gatherer,
    /// Partial uploads left long enough to sweep away, vault-relative.
    pub stale_parts: Vec<String>,
}

/// The whole walk: films, collections and leftovers, in one pass.
pub fn walk(vault: &Vault, films: bool, now: i64) -> Walk {
    walk_within(
        vault,
        MAX_DIRS,
        MAX_DURATION,
        Catalog::bundled(),
        films,
        now,
    )
}

/// Films only, with the limits passed in so a test can reach them.
pub fn scan_within(
    vault: &Vault,
    max_dirs: usize,
    max_duration: std::time::Duration,
    catalog: Option<&Catalog>,
) -> Vec<LibraryItem> {
    walk_within(vault, max_dirs, max_duration, catalog, true, 0).items
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
///
/// **Collections ride along.** Every media file the walk passes is noted for
/// Videos, Music, Photos and Recent — including inside `Extras` and `Sample`
/// folders, which are skipped for films but are still videos somebody may
/// want to find. Partial uploads left behind are noted too, so the one walk
/// that reads every folder is also what tidies them away.
pub fn walk_within(
    vault: &Vault,
    max_dirs: usize,
    max_duration: std::time::Duration,
    catalog: Option<&Catalog>,
    films: bool,
    now: i64,
) -> Walk {
    let started = std::time::Instant::now();
    let mut found = Vec::new();
    let mut walk = Walk::default();
    // Collected alongside the videos, in the same walk. A second pass looking
    // for them would mean reading every directory on the drive twice.
    let mut subtitles: Vec<String> = Vec::new();
    // Each folder with whether it is inside an extras folder, where nothing
    // is a film but everything is still a file.
    let mut queue = std::collections::VecDeque::from([(String::new(), false)]);
    let mut visited = 0usize;

    while let Some((dir, in_extras)) = queue.pop_front() {
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
                    if !parse::is_system(&path) {
                        let extras = in_extras || parse::is_extra(&path);
                        queue.push_back((path, extras));
                    }
                }
                basalt_proto::msg::EntryKind::File => {
                    if crate::uploads::is_temp_name(&entry.name) {
                        if now > 0
                            && crate::media::collect::is_stale_part(entry.size, entry.mtime, now)
                        {
                            walk.stale_parts.push(path);
                        }
                        continue;
                    }
                    walk.gathered.note(&path, entry.size, entry.mtime);
                    if !films || in_extras {
                        continue;
                    }
                    // Before the size floor: a subtitle is a few kilobytes,
                    // and the floor exists to keep home videos out of the
                    // library, not to throw away the subtitles for a film.
                    if subs::is_subtitle(&entry.name) {
                        if subtitles.len() < MAX_SUBTITLES {
                            subtitles.push(path);
                        }
                        continue;
                    }
                    if entry.size < MIN_FEATURE_BYTES {
                        continue;
                    }
                    if let Some(parsed) = identify(&path, catalog) {
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

    walk.items = group(found, &subtitles);
    // Logged because this is the one expensive thing the host does, and when
    // somebody says it has stopped responding this line is what says whether a
    // scan was the reason.
    tracing::info!(
        "scanned {visited} directories in {:?}, found {} items",
        started.elapsed(),
        walk.items.len()
    );
    walk
}

/// Adds videos that just arrived to an index, without walking the drive.
///
/// A full scan of a whole drive is minutes of disk on the machines this runs
/// on, so after one the host rests before the next — which meant a new episode
/// could take a quarter of an hour to appear under TV Series. Everything needed
/// to file one new video is right beside it, so it is filed at once, and the
/// full scan stays what it is good at: noticing what went away.
///
/// Returns the new index, or `None` when nothing in it changed.
pub fn add(
    existing: &[LibraryItem],
    vault: &Vault,
    paths: &[String],
    catalog: Option<&Catalog>,
) -> Option<Vec<LibraryItem>> {
    let mut grouping = Grouping::from_items(existing);
    let mut added = false;

    for path in paths {
        let name = path.rsplit('/').next().unwrap_or(path);
        if crate::uploads::is_temp_name(name) || parse::is_system(path) {
            continue;
        }
        let Ok(entry) = vault.stat(path) else {
            // Gone again, or never there. The next full scan will agree.
            continue;
        };
        if entry.kind != basalt_proto::msg::EntryKind::File || entry.size < MIN_FEATURE_BYTES {
            continue;
        }
        let Some(parsed) = identify(path, catalog) else {
            continue;
        };
        let tracks = subs::for_video(path, &subtitles_near(vault, path))
            .into_iter()
            .map(track)
            .collect();
        grouping.fold(
            Found {
                path: path.clone(),
                size: entry.size,
                mtime: entry.mtime,
                parsed,
            },
            tracks,
        );
        added = true;
    }

    if !added {
        return None;
    }
    let items = grouping.finish();
    (items != existing).then_some(items)
}

/// Subtitle files beside a video, and in the subtitle folders next to it —
/// everywhere [`subs::for_video`] would look.
fn subtitles_near(vault: &Vault, video: &str) -> Vec<String> {
    use basalt_proto::msg::EntryKind;

    let join = |dir: &str, name: &str| {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{dir}/{name}")
        }
    };
    let dir = video.rsplit_once('/').map_or("", |(dir, _)| dir);

    let mut found = Vec::new();
    let mut sub_folders = Vec::new();
    for entry in vault.list(dir).unwrap_or_default() {
        match entry.kind {
            EntryKind::File if subs::is_subtitle(&entry.name) => {
                found.push(join(dir, &entry.name));
            }
            EntryKind::Dir if subs::is_sub_folder(&entry.name) => {
                sub_folders.push(join(dir, &entry.name));
            }
            _ => {}
        }
    }

    // Into each `Subs` folder, and the folders inside it: `Subs/Arrival/3_English.srt`.
    for folder in sub_folders {
        for entry in vault.list(&folder).unwrap_or_default() {
            let path = join(&folder, &entry.name);
            match entry.kind {
                EntryKind::File if subs::is_subtitle(&entry.name) => found.push(path),
                EntryKind::Dir => found.extend(
                    vault
                        .list(&path)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|e| e.kind == EntryKind::File && subs::is_subtitle(&e.name))
                        .map(|e| join(&path, &e.name)),
                ),
                _ => {}
            }
        }
    }
    found
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

/// The host's subtitle record, as the wire spells it.
fn track(found: subs::Subtitle) -> basalt_proto::msg::SubtitleTrack {
    basalt_proto::msg::SubtitleTrack {
        path: found.path,
        label: found.label,
    }
}

/// Folds the files found into films and series.
fn group(found: Vec<Found>, subtitles: &[String]) -> Vec<LibraryItem> {
    // Matched once, up front, rather than per item: a season folder holds
    // thirty of each and doing it inside the loop would be quadratic.
    let videos: Vec<String> = found.iter().map(|f| f.path.clone()).collect();
    let mut by_video = subs::map_all(&videos, subtitles);

    let mut grouping = Grouping::default();
    for entry in found {
        let tracks = by_video
            .remove(&entry.path)
            .unwrap_or_default()
            .into_iter()
            .map(track)
            .collect();
        grouping.fold(entry, tracks);
    }
    grouping.finish()
}

/// Films and series being assembled, before they are tidied into an index.
///
/// Its own type so that a full scan and a single new file go through exactly
/// the same rules: an episode added on its own must land where a scan would
/// have put it, or the next scan would move it.
#[derive(Default)]
struct Grouping {
    films: Vec<LibraryItem>,
    // Ordered so the output is stable between scans, which keeps the revision
    // from bumping just because a hash map iterated differently.
    series: BTreeMap<String, LibraryItem>,
}

impl Grouping {
    /// Starts from an index that already exists.
    fn from_items(items: &[LibraryItem]) -> Self {
        let mut grouping = Grouping::default();
        for item in items {
            match item.kind {
                LibraryKind::Film => grouping.films.push(item.clone()),
                LibraryKind::Series => {
                    grouping.series.insert(item.id.clone(), item.clone());
                }
            }
        }
        grouping
    }

    fn fold(&mut self, entry: Found, subtitles: Vec<basalt_proto::msg::SubtitleTrack>) {
        let parsed = entry.parsed;
        if parsed.title.is_empty() {
            return;
        }

        if parsed.is_episode() {
            // The year is part of a series' identity when the path gives one,
            // so two different shows that share a name stay two shows. Copies
            // that carry no year are folded into the dated one in `finish`.
            let id = item_id(LibraryKind::Series, &parsed.title, parsed.year);
            let item = self
                .series
                .entry(id.clone())
                .or_insert_with(|| LibraryItem {
                    id,
                    subtitles: Vec::new(),
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
                subtitles,
                path: entry.path,
                title: None,
                size: entry.size,
                added: entry.mtime,
            });
        } else {
            let id = item_id(LibraryKind::Film, &parsed.title, parsed.year);
            // The same film at two qualities is one film. The larger file wins
            // as the one to play; both stay visible in Files.
            match self.films.iter_mut().find(|f| f.id == id) {
                Some(existing) => {
                    existing.added = existing.added.max(entry.mtime);
                    if entry.size > existing.size {
                        existing.size = entry.size;
                        // The subtitles follow the copy that will be played.
                        existing.subtitles = subtitles;
                        existing.path = Some(entry.path);
                    }
                }
                None => self.films.push(LibraryItem {
                    id,
                    kind: LibraryKind::Film,
                    title: parsed.title,
                    year: parsed.year,
                    subtitles,
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

    /// Merges, orders and totals everything into a finished index.
    fn finish(mut self) -> Vec<LibraryItem> {
        self.merge_undated_series();

        let mut items: Vec<LibraryItem> = self.films;
        for mut item in self.series.into_values() {
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

    /// Folds undated episodes into the one dated series of that name.
    ///
    /// Series used to be identified by title alone, so two different shows
    /// with the same name — a remake and its original — became one show with
    /// both sets of episodes in it. Now a year keeps them apart. But most
    /// episodes carry no year at all, and those must still join their show:
    /// when exactly one dated series has that name, they belong to it. When
    /// there are two, there is no telling which, and they stay on their own
    /// rather than being filed under the wrong show.
    fn merge_undated_series(&mut self) {
        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        for (id, item) in &self.series {
            by_title
                .entry(basalt_catalog::normalise(&item.title))
                .or_default()
                .push(id.clone());
        }

        for ids in by_title.into_values() {
            let dated: Vec<&String> = ids
                .iter()
                .filter(|id| self.series[*id].year.is_some())
                .collect();
            let [target] = dated.as_slice() else {
                continue;
            };
            let target = (*target).clone();
            let undated: Vec<String> = ids
                .iter()
                .filter(|id| self.series[*id].year.is_none())
                .cloned()
                .collect();
            for id in undated {
                let undated = self.series.remove(&id).expect("listed above");
                let into = self.series.get_mut(&target).expect("still present");
                into.confidence = into.confidence.min(undated.confidence);
                into.added = into.added.max(undated.added);
                for season in undated.seasons {
                    match into.seasons.iter_mut().find(|s| s.number == season.number) {
                        Some(existing) => existing.episodes.extend(season.episodes),
                        None => into.seasons.push(season),
                    }
                }
            }
        }
    }
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

        let items = scan_within(
            &vault_of(&dir),
            10,
            std::time::Duration::from_secs(30),
            None,
        );
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
            "Shows/Northwind/Season 01/Northwind S01E09 The Reckoning.mkv",
        );
        // A second copy elsewhere, and deliberately the larger of the two.
        let spare = dir.0.join("Spare/Northwind S01E09 The Reckoning.mkv");
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

    /// Writes a small file, as a subtitle is.
    fn put_small(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Subtitles have to survive the size floor that keeps home videos out.
    ///
    /// They are a few kilobytes. Checking the floor before checking whether a
    /// file is a subtitle would discard every one of them, and the library
    /// would report that a drive full of subtitles had none.
    #[test]
    fn a_scan_finds_the_subtitles_beside_what_it_indexes() {
        let dir = temp_dir();
        put(&dir.0, "Films/Arrival (2016).mkv");
        put_small(
            &dir.0,
            "Films/Arrival (2016).en.srt",
            b"1
",
        );
        put_small(
            &dir.0,
            "Films/Arrival (2016).fr.srt",
            b"1
",
        );

        put(&dir.0, "Shows/Northwind/Season 01/Northwind S01E01.mkv");
        put_small(
            &dir.0,
            "Shows/Northwind/Season 01/Subs/Northwind.S01E01.eng.srt",
            b"1
",
        );

        let items = scan(&vault_of(&dir));
        let film = items
            .iter()
            .find(|i| i.title == "Arrival")
            .expect("the film");
        assert_eq!(
            film.subtitles
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            ["English", "French"],
        );

        let show = items
            .iter()
            .find(|i| i.title == "Northwind")
            .expect("the series");
        let episode = &show.seasons[0].episodes[0];
        assert_eq!(episode.subtitles.len(), 1, "got {:?}", episode.subtitles);
        assert_eq!(episode.subtitles[0].label, "English");

        // And a subtitle is never mistaken for something to watch.
        assert!(!items.iter().any(|i| i.title.contains("srt")));
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
        put(&dir.0, "Films/Blade.Runner.2049.2017.mkv");
        let before = scan(&vault_of(&dir))[0].id.clone();

        std::fs::create_dir_all(dir.0.join("Archive")).unwrap();
        std::fs::rename(
            dir.0.join("Films/Blade.Runner.2049.2017.mkv"),
            dir.0.join("Archive/Blade.Runner.2049.2017.mkv"),
        )
        .unwrap();

        let after = scan(&vault_of(&dir));
        assert_eq!(after[0].id, before);
        assert_eq!(
            after[0].path.as_deref(),
            Some("Archive/Blade.Runner.2049.2017.mkv")
        );
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

    // -----------------------------------------------------------------------
    // Telling a film from a recording
    // -----------------------------------------------------------------------

    /// A catalogue small enough to reason about, so these tests say what the
    /// rules do rather than what one snapshot of Wikidata happens to hold.
    fn catalogue() -> Catalog {
        let mut b = basalt_catalog::Builder::new(20260920);
        b.add(Kind::Film, "Night Harbour", Some((2024, 2024)));
        b.add(Kind::Film, "Paper Boats", Some((2018, 2018)));
        b.add(Kind::Film, "Lantern", Some((2019, 2019)));
        b.add(Kind::Film, "Home", Some((2025, 2025)));
        b.add(Kind::Series, "Northwind", Some((2021, 2023)));
        b.finish()
    }

    fn scan_with(dir: &TempDir, catalog: &Catalog) -> Vec<LibraryItem> {
        scan_within(&vault_of(dir), MAX_DIRS, MAX_DURATION, Some(catalog))
    }

    fn titles(items: &[LibraryItem]) -> Vec<&str> {
        items.iter().map(|i| i.title.as_str()).collect()
    }

    /// The report this exists for: a term of lecture recordings, all in a
    /// folder with the year in its name, filed under Movies.
    #[test]
    fn recordings_in_a_dated_folder_are_not_films() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Semester 2025/Literature Review-20251014 100532-Meeting Recording.mp4",
        );
        put(
            &dir.0,
            "Semester 2025/Network Basics-20250927 090711-Meeting Recording.mp4",
        );
        put(&dir.0, "Semester 2025/Physical Sep 2025.mp4");
        assert!(scan_with(&dir, &catalogue()).is_empty());
    }

    #[test]
    fn a_released_film_is_kept_and_trusted() {
        let dir = temp_dir();
        put(&dir.0, "Downloads/Night.Harbour.2024.1080p.WEB-DL.x265.mkv");
        put(&dir.0, "Movies/Paper Boats (2018).mkv");
        let items = scan_with(&dir, &catalogue());
        assert_eq!(titles(&items), ["Night Harbour", "Paper Boats"]);
        assert!(items.iter().all(|i| i.confidence == VERIFIED));
    }

    #[test]
    fn a_title_nobody_released_is_not_a_film_even_in_a_movies_folder() {
        let dir = temp_dir();
        put(&dir.0, "Movies/Our Trip To The Coast 2024.mkv");
        assert!(scan_with(&dir, &catalogue()).is_empty());
    }

    /// A one-word title with a year is a home video far more often than it is
    /// the film of that name.
    #[test]
    fn a_one_word_title_needs_more_than_a_year() {
        let dir = temp_dir();
        put(&dir.0, "Videos/Home 2025.mp4");
        assert!(scan_with(&dir, &catalogue()).is_empty());

        for shown in [
            "Downloads/Home.2025.1080p.WEBRip.mkv",
            "Films/Home (2025).mkv",
            "Home (2025)/Home.mkv",
        ] {
            let dir = temp_dir();
            put(&dir.0, shown);
            assert_eq!(titles(&scan_with(&dir, &catalogue())), ["Home"], "{shown}");
        }
    }

    #[test]
    fn the_year_has_to_agree_with_the_release() {
        let dir = temp_dir();
        put(&dir.0, "Movies/Night Harbour (1999).mkv");
        assert!(scan_with(&dir, &catalogue()).is_empty());
    }

    /// Newer than the catalogue and tagged like a release: kept, but marked as
    /// a guess.
    #[test]
    fn a_tagged_film_newer_than_the_catalogue_is_kept_as_unsure() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Downloads/Tide Line 2026 2160p WEB-DL DDP5.1 H.265.mkv",
        );
        let items = scan_with(&dir, &catalogue());
        assert_eq!(titles(&items), ["Tide Line"]);
        assert_eq!(items[0].confidence, UNVERIFIED);
        assert!(items[0].confidence < basalt_proto::msg::CONFIDENT);
    }

    #[test]
    fn an_old_unknown_title_is_not_rescued_by_its_tags() {
        let dir = temp_dir();
        put(&dir.0, "Downloads/Tide Line 2011 1080p BluRay x264.mkv");
        assert!(scan_with(&dir, &catalogue()).is_empty());
    }

    /// Series are taken on their shape; the catalogue only adds confidence.
    #[test]
    fn a_series_is_kept_whether_or_not_the_catalogue_knows_it() {
        let dir = temp_dir();
        put(&dir.0, "Shows/Northwind/Season 01/Northwind S01E01.mkv");
        put(
            &dir.0,
            "Shows/Harbour Lights/Season 01/Harbour Lights S01E01.mkv",
        );
        let items = scan_with(&dir, &catalogue());
        assert_eq!(titles(&items), ["Harbour Lights", "Northwind"]);
        assert!(items[0].confidence < VERIFIED);
        assert_eq!(items[1].confidence, VERIFIED);
    }

    #[test]
    fn without_a_catalogue_the_old_rule_stands() {
        let dir = temp_dir();
        put(&dir.0, "Movies/Our Trip To The Coast.mkv");
        put(&dir.0, "Clips/Something 2024.mkv");
        put(&dir.0, "Clips/No Year At All.mkv");
        let items = scan_within(&vault_of(&dir), MAX_DIRS, MAX_DURATION, None);
        assert_eq!(titles(&items), ["Our Trip To The Coast", "Something"]);
    }

    // -----------------------------------------------------------------------
    // Series that share a name
    // -----------------------------------------------------------------------

    #[test]
    fn two_shows_with_one_name_and_different_years_stay_apart() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Shows/Harbour Lights (2004)/Season 01/Harbour Lights S01E01.mkv",
        );
        put(
            &dir.0,
            "Shows/Harbour Lights (2019)/Season 01/Harbour Lights S01E01.mkv",
        );
        let items = scan_with(&dir, &catalogue());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].year, Some(2004));
        assert_eq!(items[1].year, Some(2019));
    }

    #[test]
    fn undated_episodes_join_the_one_dated_show_of_that_name() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Shows/Harbour Lights (2019)/Season 01/Harbour Lights S01E01.mkv",
        );
        put(&dir.0, "Loose/Harbour.Lights.S01E02.mkv");
        let items = scan_with(&dir, &catalogue());
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].year, Some(2019));
        assert_eq!(items[0].seasons[0].episodes.len(), 2);
    }

    #[test]
    fn undated_episodes_are_not_guessed_into_one_of_two_dated_shows() {
        let dir = temp_dir();
        put(
            &dir.0,
            "Shows/Harbour Lights (2004)/Season 01/Harbour Lights S01E01.mkv",
        );
        put(
            &dir.0,
            "Shows/Harbour Lights (2019)/Season 01/Harbour Lights S01E01.mkv",
        );
        put(&dir.0, "Loose/Harbour.Lights.S01E02.mkv");
        assert_eq!(scan_with(&dir, &catalogue()).len(), 3);
    }

    // -----------------------------------------------------------------------
    // Adding one arrival without a scan
    // -----------------------------------------------------------------------

    #[test]
    fn a_new_episode_joins_its_series_without_a_scan() {
        let dir = temp_dir();
        put(&dir.0, "Shows/Northwind/Season 01/Northwind S01E01.mkv");
        let cat = catalogue();
        let before = scan_with(&dir, &cat);

        put(&dir.0, "Shows/Northwind/Season 01/Northwind S01E02.mkv");
        put_small(
            &dir.0,
            "Shows/Northwind/Season 01/Northwind S01E02.en.srt",
            b"1",
        );
        let after = add(
            &before,
            &vault_of(&dir),
            &["Shows/Northwind/Season 01/Northwind S01E02.mkv".to_string()],
            Some(&cat),
        )
        .expect("the index changed");

        assert_eq!(after.len(), 1);
        let episodes = &after[0].seasons[0].episodes;
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[1].number, 2);
        assert_eq!(
            episodes[1].subtitles.len(),
            1,
            "the subtitle beside it came too"
        );
        // And a full scan would have produced exactly this.
        assert_eq!(after, scan_with(&dir, &cat));
    }

    #[test]
    fn a_new_film_is_filed_by_the_same_rules_as_a_scan() {
        let dir = temp_dir();
        let cat = catalogue();
        put(&dir.0, "Downloads/Night.Harbour.2024.1080p.WEB-DL.mkv");
        put(
            &dir.0,
            "Semester 2025/Lecture 4-20251014 100532-Meeting Recording.mp4",
        );
        let after = add(
            &[],
            &vault_of(&dir),
            &[
                "Downloads/Night.Harbour.2024.1080p.WEB-DL.mkv".to_string(),
                "Semester 2025/Lecture 4-20251014 100532-Meeting Recording.mp4".to_string(),
            ],
            Some(&cat),
        )
        .expect("the index changed");
        assert_eq!(titles(&after), ["Night Harbour"]);
    }

    #[test]
    fn nothing_worth_filing_leaves_the_index_alone() {
        let dir = temp_dir();
        let cat = catalogue();
        put(&dir.0, "Shows/Northwind/Season 01/Northwind S01E01.mkv");
        let before = scan_with(&dir, &cat);

        put_small(
            &dir.0,
            "Shows/Northwind/Season 01/Northwind S01E03.mkv",
            b"tiny",
        );
        put(&dir.0, "Shows/Northwind/Season 01/.basalt-00aa.part");
        let paths = [
            "Shows/Northwind/Season 01/Northwind S01E03.mkv".to_string(),
            "Shows/Northwind/Season 01/.basalt-00aa.part".to_string(),
            "Shows/Northwind/Season 01/gone.mkv".to_string(),
            // Already in the index: filing it again changes nothing.
            "Shows/Northwind/Season 01/Northwind S01E01.mkv".to_string(),
        ];
        assert!(add(&before, &vault_of(&dir), &paths, Some(&cat)).is_none());
    }
}
