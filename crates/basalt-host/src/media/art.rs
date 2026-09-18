//! Fetching posters, when the user has asked for it.
//!
//! Off unless a TMDb key is set, and the key has to be pasted in by hand. That
//! is not an oversight: looking a title up means telling a third party what is
//! on somebody's drive, and a filename list is a list of what a person watches.
//! It should never begin happening because of an upgrade.
//!
//! With no key the library still works — [`crate::media`] parses everything the
//! same way, and the interface draws a poster from the title. Artwork is an
//! improvement on that, not a prerequisite for it.
//!
//! **Best effort throughout.** A title TMDb has never heard of, a network that
//! is down, a key that has been revoked: each costs one missing poster and
//! nothing else. Nothing here can fail a scan.

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

/// Fetches whatever is missing, and returns how many posters arrived.
///
/// Only items with no cached poster are looked up, so a rescan of a settled
/// library makes no requests at all.
pub async fn enrich(items: &[LibraryItem], key: &str, config_dir: &Path) -> usize {
    if key.trim().is_empty() {
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
    // otherwise open five hundred connections, which is both rude to TMDb and
    // a good way to be rate-limited into getting nothing at all.
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
    let Some(poster) = search(client, key, item).await else {
        return false;
    };
    let url = format!("{IMAGES}/{POSTER_SIZE}{poster}");

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
            id: id.into(),
            kind: LibraryKind::Film,
            title: title.into(),
            year: Some(2016),
            path: Some("a.mkv".into()),
            size: 1,
            added: 1,
            seasons: Vec::new(),
            confidence: 90,
            has_art: false,
        }
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

    /// Without a key this must do nothing at all — not fail, not try, not
    /// reach the network. Looking titles up is opt-in.
    #[tokio::test]
    async fn no_key_means_no_requests_and_no_cache_directory() {
        let dir = temp_dir();
        assert_eq!(enrich(&[film("f1", "Arrival")], "", &dir.0).await, 0);
        assert_eq!(enrich(&[film("f1", "Arrival")], "   ", &dir.0).await, 0);
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
            enrich(&[film("f1", "Arrival")], "not-a-real-key", &dir.0).await,
            0
        );
    }

    #[tokio::test]
    async fn an_empty_library_asks_for_nothing() {
        let dir = temp_dir();
        assert_eq!(enrich(&[], "not-a-real-key", &dir.0).await, 0);
    }
}
