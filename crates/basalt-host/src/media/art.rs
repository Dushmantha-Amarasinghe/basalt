//! Fetching posters, when the user has asked for it.
//!
//! Off until it is switched on, and switched on by a person rather than by an
//! upgrade. Looking a title up means telling a third party what is on
//! somebody's drive, and a list of filenames is a list of what a person
//! watches. No key is needed any more, so the *switch* is the consent — it
//! would have been easy to let this start happening silently once the key
//! stopped being required, and that would have been the wrong trade.
//!
//! Two sources, tried in order:
//!
//! 1. **Free Movie DB**, which needs no key at all and is what makes this work
//!    out of the box. It answers from JustWatch's catalogue, which covers
//!    silent film through last week.
//! 2. **TMDb**, if the user has pasted a key. Only consulted when the first
//!    found nothing, so a key is a widening of coverage rather than a
//!    requirement.
//!
//! **Artwork never decides anything.** What is a film, what is an episode and
//! what it is called are settled by [`super::parse`] from the path alone,
//! before this module is asked for a picture. That separation is deliberate:
//! the search here is fuzzy enough to answer `video 7` with *Scream 7* and
//! `MOV 1308` with *Scary Movie*, so letting it name things would mean a
//! library confidently full of the wrong titles. It is given a decision and
//! asked only for the image that goes with it — and even then it has to prove
//! the match before the image is kept. See [`matches`].
//!
//! **Best effort throughout.** A title neither source has heard of, a network
//! that is down, a service that has gone away: each costs one missing poster
//! and nothing else. Nothing here can fail a scan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use basalt_proto::msg::{LibraryItem, LibraryKind};
use serde::Deserialize;

/// Where cached posters live, under the host's config directory.
pub const CACHE_DIR: &str = "art";

/// Which size to pull. `w500` is about 40 KB and twice the width any card in
/// the interface draws, so it stays sharp on a high-density screen without
/// spending a megabyte per film.
const POSTER_SIZE: &str = "w500";

const API: &str = "https://api.themoviedb.org/3";
const IMAGES: &str = "https://image.tmdb.org/t/p";

/// The keyless source.
///
/// Its documented IMDb endpoints return an error for every input — only this
/// one answers — but it happens to carry everything wanted here: title, year,
/// whether it is a film or a series, and posters at several widths.
const FREE_MOVIE_DB: &str = "https://imdb.iamidiotareyoutoo.com/justwatch";

/// Requests in flight at once.
///
/// TMDb asks for restraint rather than publishing a hard limit. Four is quick
/// over a library of a few hundred and polite enough to stay welcome.
const CONCURRENCY: usize = 4;

/// How long any one request may take before it is abandoned.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The poster file for one item.
pub fn art_path(config_dir: &Path, id: &str) -> PathBuf {
    config_dir.join(CACHE_DIR).join(format!("{id}.jpg"))
}

/// Whether a poster has already been fetched for this item.
pub fn has_art(config_dir: &Path, id: &str) -> bool {
    art_path(config_dir, id).is_file()
}

/// The ids that already have a poster.
pub fn cached(config_dir: &Path) -> HashSet<String> {
    let mut found = HashSet::new();
    let Ok(entries) = std::fs::read_dir(config_dir.join(CACHE_DIR)) else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".jpg") {
            found.insert(id.to_string());
        }
    }
    found
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    poster_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FreeMovieDbResponse {
    #[serde(default)]
    description: Vec<FreeMovieDbResult>,
}

#[derive(Debug, Deserialize)]
struct FreeMovieDbResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    year: Option<u16>,
    /// `MOVIE` or `SHOW`.
    #[serde(rename = "type", default)]
    kind: String,
    /// Widest first, as the service returns them.
    #[serde(default)]
    photo_url: Vec<String>,
}

/// A title reduced to what is worth comparing.
///
/// Case, punctuation and spacing all differ between a release name and a
/// catalogue entry without meaning anything: `Northwind Fall` and
/// `Northwind: Fall` are the same programme.
///
/// The catalogue's normalisation, so a poster and a film are matched by the
/// same rule that decided the film was real. The version that used to live
/// here kept accents, so `Amelie` never met the catalogue's *Amélie*.
fn normalise(title: &str) -> String {
    basalt_catalog::normalise(title)
}

/// Whether a search result is the thing that was asked for.
///
/// **Deliberately strict.** This is a fuzzy search over a large catalogue, and
/// it always has an answer: asked for `video 7` it offers *Scream 7*, asked for
/// `MOV 1308` it offers *Scary Movie*. Both look entirely plausible in a
/// response and would hang a real film's poster on somebody's screen recording.
///
/// So the title has to match exactly once punctuation and case are set aside,
/// the kind has to agree, and a year — when both sides have one — has to be
/// within a year, because a release is often dated to the year either side of a
/// catalogue's. Being too strict costs a poster that could have been found.
/// Being too loose costs the user's trust in everything else on the screen.
fn matches(result: &FreeMovieDbResult, item: &LibraryItem) -> bool {
    let wanted_kind = match item.kind {
        LibraryKind::Film => "MOVIE",
        LibraryKind::Series => "SHOW",
    };
    if !result.kind.eq_ignore_ascii_case(wanted_kind) {
        return false;
    }
    if normalise(&result.title) != normalise(&item.title) {
        return false;
    }
    match (item.year, result.year) {
        (Some(ours), Some(theirs)) => ours.abs_diff(theirs) <= 1,
        _ => true,
    }
}

/// Fetches whatever is missing, and returns how many posters arrived.
///
/// Only items with no cached poster are looked up, so a rescan of a settled
/// library makes no requests at all.
pub async fn enrich(
    items: &[LibraryItem],
    wanted_at_all: bool,
    key: &str,
    config_dir: &Path,
) -> usize {
    if !wanted_at_all {
        return 0;
    }
    let have = cached(config_dir);
    let wanted: Vec<&LibraryItem> = items.iter().filter(|i| !have.contains(&i.id)).collect();
    if wanted.is_empty() {
        return 0;
    }

    if let Err(e) = std::fs::create_dir_all(config_dir.join(CACHE_DIR)) {
        tracing::warn!("no artwork cache: {e}");
        return 0;
    }

    let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("could not start the artwork client: {e}");
            return 0;
        }
    };

    // A few at a time, not all at once. A library of five hundred titles would
    // otherwise open five hundred connections, which is both rude to a free
    // service and a good way to be rate-limited into getting nothing at all.
    let mut added = 0usize;
    let mut running = tokio::task::JoinSet::new();
    let mut queue = wanted.into_iter();

    loop {
        while running.len() < CONCURRENCY {
            let Some(item) = queue.next() else { break };
            let client = client.clone();
            let key = key.to_string();
            let item = item.clone();
            let dir = config_dir.to_path_buf();
            running.spawn(async move { fetch_one(&client, &key, &item, &dir).await });
        }

        match running.join_next().await {
            Some(Ok(true)) => added += 1,
            Some(Ok(false)) => {}
            // A panicked fetch costs one poster, not the whole run.
            Some(Err(e)) => tracing::debug!("a poster fetch failed: {e}"),
            None => break,
        }
    }

    if added > 0 {
        tracing::info!("fetched {added} poster(s)");
    }
    added
}

/// One title: search, then download. True when a poster was saved.
async fn fetch_one(
    client: &reqwest::Client,
    key: &str,
    item: &LibraryItem,
    config_dir: &Path,
) -> bool {
    // The keyless source first, so this works for somebody who has pasted
    // nothing. TMDb is asked only when that found nothing, which makes a key a
    // way to widen coverage rather than the price of entry.
    let url = match free_movie_db(client, item).await {
        Some(url) => url,
        None => match search(client, key, item).await {
            Some(poster) => format!("{IMAGES}/{POSTER_SIZE}{poster}"),
            None => return false,
        },
    };

    let bytes = match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::debug!("poster for {} did not download: {e}", item.title);
                return false;
            }
        },
        Ok(response) => {
            tracing::debug!("poster for {}: {}", item.title, response.status());
            return false;
        }
        Err(e) => {
            tracing::debug!("poster for {}: {e}", item.title);
            return false;
        }
    };

    // Through a temporary file and a rename, so a download interrupted halfway
    // cannot leave a truncated image that would then be treated as cached and
    // never retried.
    let target = art_path(config_dir, &item.id);
    let temp = target.with_extension("part");
    if std::fs::write(&temp, &bytes).is_err() {
        return false;
    }
    std::fs::rename(&temp, &target).is_ok()
}

/// Asks the keyless source for a poster, and only takes one it can vouch for.
///
/// Returns a complete URL, unlike TMDb's, which hands back a path to be joined
/// to an image host.
async fn free_movie_db(client: &reqwest::Client, item: &LibraryItem) -> Option<String> {
    let url = format!("{FREE_MOVIE_DB}?q={}", urlencode(&item.title));
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        tracing::debug!("looking up {}: {}", item.title, response.status());
        return None;
    }
    let body: FreeMovieDbResponse = response.json().await.ok()?;

    // Every candidate is checked, not just the first. The service orders by
    // its own relevance, which puts the popular remake above the film actually
    // asked for — `Nosferatu 1922` answers with the 2024 one first.
    let found = body
        .description
        .iter()
        .find(|result| matches(result, item))?;

    // Widest first, and the widest on offer is around 592px — ample for a card
    // and still small enough to be worth caching per title.
    found.photo_url.first().cloned()
}

async fn search(client: &reqwest::Client, key: &str, item: &LibraryItem) -> Option<String> {
    let endpoint = match item.kind {
        LibraryKind::Film => "search/movie",
        LibraryKind::Series => "search/tv",
    };
    // The year is the strongest disambiguator there is — two films share a
    // title far more often than a title and a year together.
    let year_param = match (item.kind, item.year) {
        (LibraryKind::Film, Some(year)) => format!("&year={year}"),
        (LibraryKind::Series, Some(year)) => format!("&first_air_date_year={year}"),
        _ => String::new(),
    };

    let url = format!(
        "{API}/{endpoint}?api_key={}&query={}{year_param}",
        urlencode(key),
        urlencode(&item.title),
    );

    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        tracing::debug!("looking up {}: {}", item.title, response.status());
        return None;
    }
    let body: SearchResponse = response.json().await.ok()?;
    body.results.into_iter().find_map(|r| r.poster_path)
}

/// Percent-encodes a query value.
///
/// Small by hand rather than another dependency: the only things that reach it
/// are a title and a key, and the set of characters that must not travel raw in
/// a query string is short and fixed.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
            "basalt-art-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn film(id: &str, title: &str) -> LibraryItem {
        LibraryItem {
            resolution: None,
            id: id.into(),
            kind: LibraryKind::Film,
            title: title.into(),
            year: Some(2016),
            path: Some("a.mkv".into()),
            size: 1,
            added: 1,
            seasons: Vec::new(),
            subtitles: Vec::new(),
            confidence: 90,
            has_art: false,
        }
    }

    fn result(title: &str, year: Option<u16>, kind: &str) -> FreeMovieDbResult {
        FreeMovieDbResult {
            title: title.into(),
            year,
            kind: kind.into(),
            photo_url: vec!["https://example.test/p.jpg".into()],
        }
    }

    fn series(title: &str, year: Option<u16>) -> LibraryItem {
        LibraryItem {
            resolution: None,
            kind: LibraryKind::Series,
            title: title.into(),
            year,
            path: None,
            ..film("s1", title)
        }
    }

    /// Every one of these is an answer the service really gave to a filename
    /// that was not a film. Accepting any of them hangs a film's poster on
    /// somebody's screen recording, which looks far more broken than no poster.
    #[test]
    fn a_plausible_wrong_answer_is_refused() {
        let asked = film("f1", "video 7");
        assert!(!matches(&result("Scream 7", Some(2026), "MOVIE"), &asked));

        let asked = film("f2", "Day 23");
        assert!(!matches(
            &result("Disclosure Day", Some(2026), "MOVIE"),
            &asked
        ));

        let asked = film("f3", "MOV 1308");
        assert!(!matches(
            &result("Scary Movie", Some(2026), "MOVIE"),
            &asked
        ));
    }

    /// A film is not its series and a series is not its film, whatever the
    /// title says.
    #[test]
    fn the_wrong_kind_is_refused() {
        let asked = film("f1", "The Quiet Coast");
        assert!(!matches(
            &result("The Quiet Coast", Some(2014), "SHOW"),
            &asked
        ));
    }

    /// The same title, a different decade: a real risk for remakes, and the
    /// service offers the newest first regardless of what was asked.
    #[test]
    fn the_wrong_year_is_refused() {
        let asked = LibraryItem {
            resolution: None,
            year: Some(1922),
            ..film("f1", "Nosferatu")
        };
        assert!(!matches(&result("Nosferatu", Some(2024), "MOVIE"), &asked));
        assert!(matches(&result("Nosferatu", Some(1922), "MOVIE"), &asked));
        // A release dated a year either side of the catalogue is still it.
        assert!(matches(&result("Nosferatu", Some(1923), "MOVIE"), &asked));
    }

    /// And the ones that should be accepted, differing only in how they were
    /// written down.
    #[test]
    fn the_right_answer_is_taken_through_punctuation_and_case() {
        // `Northwind Fall` off the filename, `Northwind: Fall` in the catalogue.
        assert!(matches(
            &result("Northwind: Fall", Some(2025), "SHOW"),
            &series("Northwind Fall", None)
        ));
        assert!(matches(
            &result("SALT", Some(2022), "SHOW"),
            &series("Salt", None)
        ));
        // Accents are not a difference either: a filename typed without one
        // is the same film.
        assert!(matches(
            &result("Amélie", Some(2001), "MOVIE"),
            &LibraryItem {
                resolution: None,
                year: Some(2001),
                ..film("f9", "Amelie")
            }
        ));
        // No year on our side is not a mismatch, just less to go on.
        assert!(matches(
            &result("The Quiet Coast", Some(2014), "SHOW"),
            &series("The Quiet Coast", None)
        ));
    }

    #[test]
    fn a_title_with_spaces_and_punctuation_survives_a_query_string() {
        assert_eq!(urlencode("Blade Runner 2049"), "Blade%20Runner%202049");
        assert_eq!(urlencode("WALL·E"), "WALL%C2%B7E");
        assert_eq!(urlencode("Am\u{e9}lie"), "Am%C3%A9lie");
    }

    #[test]
    fn characters_that_would_break_the_query_are_escaped() {
        // An unescaped `&` would silently truncate the title and look up the
        // wrong film; an unescaped `#` would drop everything after it.
        assert_eq!(urlencode("Fire & Ice"), "Fire%20%26%20Ice");
        assert_eq!(urlencode("#Alive"), "%23Alive");
        assert_eq!(urlencode("a=b"), "a%3Db");
    }

    #[test]
    fn unreserved_characters_are_left_alone() {
        assert_eq!(urlencode("Spider-Man_2.0~x"), "Spider-Man_2.0~x");
    }

    #[test]
    fn the_cache_starts_empty_and_notices_what_arrives() {
        let dir = temp_dir();
        assert!(cached(&dir.0).is_empty());
        assert!(!has_art(&dir.0, "f1"));

        std::fs::create_dir_all(dir.0.join(CACHE_DIR)).unwrap();
        std::fs::write(art_path(&dir.0, "f1"), b"jpeg").unwrap();

        assert!(has_art(&dir.0, "f1"));
        assert_eq!(cached(&dir.0), HashSet::from(["f1".to_string()]));
    }

    #[test]
    fn a_stray_file_in_the_cache_is_not_read_as_an_id() {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.0.join(CACHE_DIR)).unwrap();
        std::fs::write(dir.0.join(CACHE_DIR).join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.0.join(CACHE_DIR).join("f2.jpg.part"), b"x").unwrap();

        assert!(cached(&dir.0).is_empty(), "only finished jpegs count");
    }

    /// Switched off, this must do nothing at all — not fail, not try, not
    /// reach the network, not even make a folder.
    ///
    /// The switch carries the whole consent now that no key is needed. When a
    /// key was required, forgetting this check would simply have done nothing;
    /// today it would silently start sending every title on somebody's drive
    /// to a stranger the moment they upgraded.
    #[tokio::test]
    async fn switched_off_means_no_requests_and_no_cache_directory() {
        let dir = temp_dir();
        assert_eq!(enrich(&[film("f1", "Arrival")], false, "", &dir.0).await, 0);
        assert_eq!(
            enrich(&[film("f1", "Arrival")], false, "a-key", &dir.0).await,
            0,
            "a key present but the switch off is still off"
        );
        assert!(
            !dir.0.join(CACHE_DIR).exists(),
            "nothing should be created before anyone opts in"
        );
    }

    #[tokio::test]
    async fn an_item_that_already_has_a_poster_is_not_looked_up_again() {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.0.join(CACHE_DIR)).unwrap();
        std::fs::write(art_path(&dir.0, "f1"), b"jpeg").unwrap();

        // A key that would fail if it were ever used, so a request would show
        // up as a hang or an error rather than passing quietly.
        assert_eq!(
            enrich(&[film("f1", "Arrival")], true, "not-a-real-key", &dir.0).await,
            0
        );
    }

    #[tokio::test]
    async fn an_empty_library_asks_for_nothing() {
        let dir = temp_dir();
        assert_eq!(enrich(&[], true, "not-a-real-key", &dir.0).await, 0);
    }
}
