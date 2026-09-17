//! Turning a path into a film or an episode.
//!
//! Everything here is a heuristic, and the design follows from admitting that.
//! Each guess carries a confidence, nothing is asserted that the evidence does
//! not support, and the interface can show the weak matches to a person instead
//! of quietly filing a home video under a feature film.
//!
//! **The folder is better evidence than the filename.** `Breaking Bad/Season
//! 01/ep1.mkv` has a useless filename and an unambiguous path, and that shape
//! is the norm rather than the exception. So the path is read as a whole, from
//! the outside in, and a `Season NN` folder is treated as the strongest single
//! signal available — it tells you both that this is a series *and* what the
//! series is called, which is the folder above it.
//!
//! Deliberately no network, no ffmpeg, no configuration. This runs on a drive
//! nobody has organised for us.

/// Extensions treated as watchable video.
const VIDEO: &[&str] = &[
    "mkv", "mp4", "avi", "mov", "m4v", "webm", "wmv", "flv", "ts", "m2ts", "mpg", "mpeg", "vob",
    "divx", "ogv", "rmvb", "asf", "3gp",
];

/// Release noise: everything after the first of these is not part of a title.
///
/// Ordered longest-first where prefixes collide, so `web-dl` is not half-eaten
/// by `web`.
const NOISE: &[&str] = &[
    "2160p",
    "1080p",
    "1440p",
    "720p",
    "576p",
    "480p",
    "360p",
    "4k",
    "8k",
    "uhd",
    "hdr10",
    "hdr",
    "dolby",
    "dv",
    "bluray",
    "blu-ray",
    "brrip",
    "bdrip",
    "bdremux",
    "remux",
    "web-dl",
    "webdl",
    "webrip",
    "web",
    "hdtv",
    "pdtv",
    "dvdrip",
    "dvdscr",
    "hdrip",
    "cam",
    "ts",
    "tc",
    "x264",
    "x265",
    "h264",
    "h265",
    "h",
    "avc",
    "hevc",
    "av1",
    "xvid",
    "divx",
    "mpeg2",
    "10bit",
    "8bit",
    "aac",
    "ac3",
    "eac3",
    "dd5",
    "ddp5",
    "ddp",
    "dts",
    "dtshd",
    "truehd",
    "atmos",
    "flac",
    "mp3",
    "opus",
    "5",
    "7",
    "multi",
    "dual",
    "dubbed",
    "subbed",
    "subs",
    "proper",
    "repack",
    "extended",
    "unrated",
    "uncut",
    "remastered",
    "imax",
    "limited",
    "internal",
    "complete",
    "season",
    "hybrid",
    "sdr",
    "hq",
    "amzn",
    "nf",
    "dsnp",
    "hmax",
    "atvp",
    "hulu",
    "ita",
    "eng",
    "jpn",
];

/// Folders and files that are not the feature.
const EXTRAS: &[&str] = &[
    "sample",
    "trailer",
    "trailers",
    "extras",
    "featurette",
    "featurettes",
    "behind the scenes",
    "behindthescenes",
    "deleted scenes",
    "deletedscenes",
    "interview",
    "interviews",
    "bloopers",
    "shorts",
    "other",
    "scenes",
    "bonus",
    "making of",
    "makingof",
    "proof",
    "screens",
];

/// What a path turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub title: String,
    pub year: Option<u16>,
    /// Present exactly when this is an episode.
    pub season: Option<u16>,
    pub episode: Option<u16>,
    /// 0–100. See [`basalt_proto::msg::CONFIDENT`] for where the line sits.
    pub confidence: u8,
}

impl Parsed {
    pub fn is_episode(&self) -> bool {
        self.season.is_some() && self.episode.is_some()
    }
}

/// Whether a filename is a video worth indexing.
pub fn is_video(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => VIDEO.contains(&ext.to_ascii_lowercase().as_str()),
        _ => false,
    }
}

/// Whether any part of this path marks it as an extra rather than the feature.
///
/// Checked over the whole path, not just the filename, because these usually
/// arrive as a folder: `Arrival (2016)/Featurettes/making-of.mkv`.
pub fn is_extra(path: &str) -> bool {
    path.split('/').any(|segment| {
        let cleaned = segment.to_ascii_lowercase();
        let stem = cleaned
            .rsplit_once('.')
            .map_or(cleaned.as_str(), |(s, _)| s);
        let normalised = stem.replace(['.', '_', '-'], " ");
        let normalised = normalised.trim();
        // Exactly the word, or ending in it — `Arrival-sample`. Deliberately
        // *not* "starts with", which would file Trailer Park Boys as a trailer.
        EXTRAS
            .iter()
            .any(|extra| normalised == *extra || normalised.ends_with(&format!(" {extra}")))
    })
}

/// Reads a path and says what it is, or nothing if it is not a video.
///
/// `path` is vault-relative with forward slashes, as the wire spells it.
pub fn parse(path: &str) -> Option<Parsed> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let file = segments.last()?;
    if !is_video(file) || is_extra(path) {
        return None;
    }

    let stem = file.rsplit_once('.').map_or(*file, |(s, _)| s);
    let folders = &segments[..segments.len() - 1];

    match episode_numbers(stem) {
        Some((season, episode)) => Some(episode_from(stem, folders, season, episode)),
        None => match season_folder(folders) {
            // A `Season NN` folder with an unreadable filename is still clearly
            // an episode of something — the folder above names the series, and
            // the episode number is the best that can be recovered.
            Some((depth, season)) => {
                let number = trailing_number(stem).unwrap_or(0);
                let mut parsed = series_named(folders, depth);
                parsed.season = Some(season);
                parsed.episode = Some(number);
                parsed.confidence = if number > 0 { 78 } else { 45 };
                Some(parsed)
            }
            None => Some(film_from(stem, folders)),
        },
    }
}

// ---------------------------------------------------------------------------
// Episodes
// ---------------------------------------------------------------------------

/// `S01E02`, `1x02`, `S01.E02`, `Season 1 Episode 2`.
///
/// Works on characters throughout. Slicing the lowercased string by an index
/// found while walking characters would panic the moment a title contains an
/// accent, which is not a theoretical concern on a film library.
fn episode_numbers(stem: &str) -> Option<(u16, u16)> {
    let chars: Vec<char> = stem.to_ascii_lowercase().chars().collect();

    // sNNeNN, with anything non-alphanumeric allowed between the two halves.
    for i in 0..chars.len() {
        if chars[i] != 's' {
            continue;
        }
        let Some((season, mut j)) = read_number(&chars, i + 1) else {
            continue;
        };
        if j == i + 1 {
            continue;
        }
        while j < chars.len() && !chars[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j < chars.len()
            && chars[j] == 'e'
            && let Some((episode, end)) = read_number(&chars, j + 1)
            && end > j + 1
        {
            return Some((season, episode));
        }
    }

    // NNxNN
    for i in 0..chars.len() {
        if chars[i] != 'x' || i == 0 {
            continue;
        }
        let start = chars[..i]
            .iter()
            .rposition(|c| !c.is_ascii_digit())
            .map_or(0, |at| at + 1);
        if start == i {
            continue;
        }
        let season: u16 = match chars[start..i].iter().collect::<String>().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if let Some((episode, end)) = read_number(&chars, i + 1)
            && end > i + 1
        {
            return Some((season, episode));
        }
    }

    // "season 1 episode 2", spelled out.
    let word_at = |chars: &[char], word: &[char]| -> Option<usize> {
        // A name shorter than the word it is being searched for has no match —
        // and `saturating_sub` alone would still offer index 0 and slice past
        // the end, which is a panic on every short filename.
        if chars.len() < word.len() {
            return None;
        }
        (0..=chars.len() - word.len()).find(|&i| &chars[i..i + word.len()] == word)
    };
    let season_word: Vec<char> = "season ".chars().collect();
    let episode_word: Vec<char> = "episode ".chars().collect();
    if let Some(s) = word_at(&chars, &season_word)
        && let Some((season, _)) = read_number(&chars, s + season_word.len())
        && let Some(e) = word_at(&chars, &episode_word)
        && let Some((episode, _)) = read_number(&chars, e + episode_word.len())
        && episode > 0
    {
        return Some((season, episode));
    }

    None
}

/// Reads digits from `at`, returning the value and where it stopped.
fn read_number(chars: &[char], at: usize) -> Option<(u16, usize)> {
    let mut end = at;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if end == at {
        return Some((0, at));
    }
    let text: String = chars[at..end].iter().collect();
    Some((text.parse().ok()?, end))
}

/// A `Season 01` / `S01` / `Series 2` folder, as a depth from the end.
fn season_folder(folders: &[&str]) -> Option<(usize, u16)> {
    for (back, folder) in folders.iter().rev().enumerate() {
        let lower = folder.trim().to_ascii_lowercase();
        // "Specials" is season zero everywhere that matters.
        if lower == "specials" || lower == "special" {
            return Some((back, 0));
        }
        // Longest prefix first, or `season 1` matches the bare `s` rule and
        // leaves "eason 1", which is not a number.
        for prefix in ["season", "series", "s"] {
            if let Some(rest) = lower.strip_prefix(prefix) {
                let digits = rest.trim_start_matches([' ', '.', '_', '-']);
                if !digits.is_empty()
                    && digits.chars().all(|c| c.is_ascii_digit())
                    && let Ok(number) = digits.parse::<u16>()
                {
                    return Some((back, number));
                }
            }
        }
    }
    None
}

/// The last run of digits in a name, for `ep1.mkv` or `Episode 04.mkv`.
///
/// Digits attached to a word count: `e04` is episode four, and that spelling is
/// common enough inside a `Season NN` folder to be worth reading.
fn trailing_number(stem: &str) -> Option<u16> {
    let chars: Vec<char> = stem.chars().collect();
    let end = chars.iter().rposition(|c| c.is_ascii_digit())? + 1;
    let start = chars[..end]
        .iter()
        .rposition(|c| !c.is_ascii_digit())
        .map_or(0, |at| at + 1);
    chars[start..end].iter().collect::<String>().parse().ok()
}

/// Builds an episode, taking the series name from the best available place.
fn episode_from(stem: &str, folders: &[&str], season: u16, episode: u16) -> Parsed {
    // A `Season NN` folder is the strongest signal there is: it says both that
    // this is a series and, one level up, what it is called.
    if let Some((depth, _)) = season_folder(folders) {
        let mut parsed = series_named(folders, depth);
        parsed.season = Some(season);
        parsed.episode = Some(episode);
        parsed.confidence = 95;
        return parsed;
    }

    // Otherwise the filename's own prefix, falling back to the folder when the
    // filename begins with the marker and carries no title at all.
    let from_name = clean_title(&cut_at_marker(stem));
    let (title, year, confidence) = if from_name.is_empty() {
        let folder = folders.last().copied().unwrap_or_default();
        (clean_title(folder), year_in(folder), 80)
    } else {
        (from_name, year_in(stem), 88)
    };

    Parsed {
        title,
        year,
        season: Some(season),
        episode: Some(episode),
        confidence,
    }
}

/// A series named by the folder `depth` levels above the file's own folder.
fn series_named(folders: &[&str], depth: usize) -> Parsed {
    // `depth` counts back from the file's folder to the `Season NN` one; the
    // series folder is one further out again.
    let index = folders.len().saturating_sub(depth + 2);
    let folder = folders.get(index).copied().unwrap_or_default();
    Parsed {
        title: clean_title(folder),
        year: year_in(folder),
        season: None,
        episode: None,
        confidence: 0,
    }
}

/// Everything before the `SxxExx` / `1x02` marker.
///
/// Returns an owned string built from characters, not a slice: the search runs
/// over the lowercased form and a byte slice at a character index would split a
/// multi-byte character and panic.
fn cut_at_marker(stem: &str) -> String {
    let original: Vec<char> = stem.chars().collect();
    let chars: Vec<char> = stem.to_ascii_lowercase().chars().collect();

    for i in 0..chars.len() {
        let marker = (chars[i] == 's'
            && matches!(read_number(&chars, i + 1), Some((_, end)) if end > i + 1))
            || (chars[i] == 'x'
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && matches!(read_number(&chars, i + 1), Some((_, end)) if end > i + 1));
        if !marker {
            continue;
        }
        let start = if chars[i] == 'x' {
            chars[..i]
                .iter()
                .rposition(|c| !c.is_ascii_digit())
                .map_or(0, |at| at + 1)
        } else {
            i
        };
        return original[..start].iter().collect();
    }
    stem.to_string()
}

// ---------------------------------------------------------------------------
// Films
// ---------------------------------------------------------------------------

fn film_from(stem: &str, folders: &[&str]) -> Parsed {
    let from_name = clean_title(stem);
    let name_year = year_in(stem);
    let folder = folders.last().copied().unwrap_or_default();
    let folder_year = year_in(folder);

    // `Arrival (2016)/movie.mkv` — the folder is the only thing that knows.
    let generic = from_name.is_empty()
        || matches!(
            from_name.to_ascii_lowercase().as_str(),
            "movie" | "film" | "video" | "main" | "index" | "vts 01 1" | "title00"
        );

    if generic && !folder.is_empty() {
        return Parsed {
            title: clean_title(folder),
            year: folder_year,
            season: None,
            episode: None,
            confidence: if folder_year.is_some() { 85 } else { 60 },
        };
    }

    let year = name_year.or(folder_year);
    Parsed {
        title: from_name,
        year,
        // A year is the difference between "probably a film" and "some video".
        confidence: if year.is_some() { 90 } else { 55 },
        season: None,
        episode: None,
    }
}

// ---------------------------------------------------------------------------
// Shared cleaning
// ---------------------------------------------------------------------------

/// A four-digit year that could plausibly be one.
fn year_in(text: &str) -> Option<u16> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = None;
    for i in 0..chars.len() {
        if !chars[i].is_ascii_digit() {
            continue;
        }
        // Not part of a longer run of digits, which would be a resolution or a
        // release id rather than a year.
        if i > 0 && chars[i - 1].is_ascii_digit() {
            continue;
        }
        if i + 4 > chars.len() || !chars[i..i + 4].iter().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if i + 4 < chars.len() && chars[i + 4].is_ascii_digit() {
            continue;
        }
        let value: u16 = chars[i..i + 4].iter().collect::<String>().parse().ok()?;
        if (1900..=2099).contains(&value) {
            // The last plausible year wins: `Blade Runner 2049 (2017)` is a
            // 2017 film, and the title keeps its 2049.
            found = Some(value);
        }
    }
    found
}

/// A token that opens with a plausible year: `2016`, `(2016)`, `2020-RARBG`.
fn opens_with_year(word: &str) -> bool {
    let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
    let digits: String = bare.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.len() == 4 && matches!(digits.parse::<u16>(), Ok(y) if (1900..=2099).contains(&y))
}

/// Strips separators, release noise and anything after it.
///
/// Two rules here look obvious and are not:
///
/// - **The release year is the *last* year in the name, not the first.** `Blade
///   Runner 2049 (2017)` has two, and cutting at the first renames the film.
/// - **A trailing `-GROUP` is not stripped on sight.** Release names put the
///   group after the noise, so cutting at the noise already removes it — and a
///   rule that removes it directly turns `Spider-Man` into `Spider`.
pub fn clean_title(raw: &str) -> String {
    let spaced = raw
        .replace(['.', '_'], " ")
        .replace(['[', ']', '{', '}'], " ");
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let release_year = words.iter().rposition(|word| opens_with_year(word));

    let mut kept: Vec<String> = Vec::new();
    for (i, word) in words.iter().enumerate() {
        // Never cut everything away: a film whose title *is* a year keeps it.
        if Some(i) == release_year && !kept.is_empty() {
            break;
        }
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
        if NOISE.contains(&bare.to_ascii_lowercase().as_str()) {
            break;
        }
        if bare.is_empty() {
            continue;
        }
        kept.push(bare.to_string());
    }

    kept.join(" ")
        .trim_matches(|c: char| c == '-' || c == ' ')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film(path: &str) -> Parsed {
        let parsed = parse(path).unwrap_or_else(|| panic!("{path} should parse"));
        assert!(!parsed.is_episode(), "{path} parsed as an episode");
        parsed
    }

    fn episode(path: &str) -> Parsed {
        let parsed = parse(path).unwrap_or_else(|| panic!("{path} should parse"));
        assert!(parsed.is_episode(), "{path} did not parse as an episode");
        parsed
    }

    // -----------------------------------------------------------------------
    // What counts as a video at all
    // -----------------------------------------------------------------------

    #[test]
    fn only_video_extensions_are_indexed() {
        assert!(is_video("a.mkv"));
        assert!(is_video("A.MP4"), "extensions are not case sensitive");
        assert!(!is_video("a.srt"));
        assert!(!is_video("a.jpg"));
        assert!(!is_video("mkv"), "no extension at all");
        assert!(!is_video(".mkv"), "a dotfile is not a video called nothing");
    }

    #[test]
    fn extras_are_left_out_wherever_they_appear_in_the_path() {
        assert!(is_extra("Arrival (2016)/Featurettes/making-of.mkv"));
        assert!(is_extra("films/Arrival-sample.mkv"));
        assert!(is_extra("films/Behind The Scenes/a.mkv"));
        assert!(parse("films/Arrival (2016)/trailer.mkv").is_none());
    }

    // "Trailer Park Boys" is a series, not a trailer.
    #[test]
    fn a_title_that_merely_contains_an_extras_word_is_kept() {
        assert!(!is_extra("Trailer Park Boys/Season 01/S01E01.mkv"));
        assert!(!is_extra("films/The Sample Size (2012).mkv"));
    }

    // -----------------------------------------------------------------------
    // Films
    // -----------------------------------------------------------------------

    #[test]
    fn a_scene_release_becomes_a_clean_title_and_year() {
        let parsed = film("films/Arrival.2016.1080p.BluRay.x264-SPARKS.mkv");
        assert_eq!(parsed.title, "Arrival");
        assert_eq!(parsed.year, Some(2016));
        assert!(parsed.confidence >= basalt_proto::msg::CONFIDENT);
    }

    #[test]
    fn a_bracketed_year_ends_the_title() {
        let parsed = film("films/Blade Runner (1982) [2160p] [HDR].mkv");
        assert_eq!(parsed.title, "Blade Runner");
        assert_eq!(parsed.year, Some(1982));
    }

    /// The folder is the only thing that knows, and that arrangement is common
    /// enough that failing it would be embarrassing.
    #[test]
    fn a_film_named_only_by_its_folder_is_still_recognised() {
        let parsed = film("films/Arrival (2016)/movie.mkv");
        assert_eq!(parsed.title, "Arrival");
        assert_eq!(parsed.year, Some(2016));
    }

    #[test]
    fn a_dvd_rip_with_a_meaningless_filename_uses_the_folder() {
        let parsed = film("films/The Thing (1982)/VTS_01_1.mkv");
        assert_eq!(parsed.title, "The Thing");
        assert_eq!(parsed.year, Some(1982));
    }

    /// `Blade Runner 2049 (2017)` — one of these numbers is the title and the
    /// other is the year, and getting it backwards renames the film.
    #[test]
    fn a_year_in_the_title_is_not_mistaken_for_the_release_year() {
        let parsed = film("films/Blade Runner 2049 (2017) 2160p.mkv");
        assert_eq!(parsed.title, "Blade Runner 2049");
        assert_eq!(parsed.year, Some(2017));
    }

    #[test]
    fn a_film_whose_title_is_a_year_survives() {
        let parsed = film("films/2012.2009.1080p.BluRay.mkv");
        assert_eq!(parsed.title, "2012");
        assert_eq!(parsed.year, Some(2009));
    }

    #[test]
    fn a_film_with_no_year_is_kept_but_held_less_confidently() {
        let parsed = film("films/Some Home Video.mkv");
        assert_eq!(parsed.title, "Some Home Video");
        assert_eq!(parsed.year, None);
        assert!(
            parsed.confidence < basalt_proto::msg::CONFIDENT,
            "worth showing to a person rather than asserting"
        );
    }

    #[test]
    fn a_resolution_is_never_read_as_a_year() {
        assert_eq!(year_in("1080p"), None);
        assert_eq!(year_in("S01E01"), None);
        assert_eq!(year_in("Arrival 2016"), Some(2016));
    }

    // -----------------------------------------------------------------------
    // Episodes
    // -----------------------------------------------------------------------

    #[test]
    fn the_standard_marker_is_read() {
        let parsed = episode("shows/Breaking.Bad.S01E07.1080p.mkv");
        assert_eq!(parsed.title, "Breaking Bad");
        assert_eq!((parsed.season, parsed.episode), (Some(1), Some(7)));
    }

    #[test]
    fn the_older_markers_are_read_too() {
        assert_eq!(episode_numbers("show.1x02.mkv"), Some((1, 2)));
        assert_eq!(episode_numbers("show.s01.e02"), Some((1, 2)));
        assert_eq!(episode_numbers("Show Season 3 Episode 11"), Some((3, 11)));
        assert_eq!(episode_numbers("show.S2024E05"), Some((2024, 5)));
    }

    /// The case the whole path-first design exists for: the filename says
    /// nothing at all and the folders say everything.
    #[test]
    fn a_useless_filename_under_a_season_folder_still_resolves() {
        let parsed = episode("Shows/Breaking Bad/Season 01/ep1.mkv");
        assert_eq!(parsed.title, "Breaking Bad");
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
    }

    #[test]
    fn a_season_folder_beats_a_filename_that_repeats_the_show_name() {
        let parsed = episode("Shows/Breaking Bad (2008)/Season 01/Breaking.Bad.S01E01.mkv");
        assert_eq!(parsed.title, "Breaking Bad");
        assert_eq!(parsed.year, Some(2008));
        assert_eq!(parsed.confidence, 95);
    }

    #[test]
    fn the_short_season_folder_spelling_works() {
        let parsed = episode("Shows/The Wire/S03/e04.mkv");
        assert_eq!(parsed.title, "The Wire");
        assert_eq!((parsed.season, parsed.episode), (Some(3), Some(4)));
    }

    #[test]
    fn specials_are_season_zero() {
        let parsed = episode("Shows/Firefly/Specials/Making.S00E01.mkv");
        assert_eq!(parsed.title, "Firefly");
        assert_eq!(parsed.season, Some(0));
    }

    #[test]
    fn an_episode_loose_in_a_folder_takes_its_name_from_the_filename() {
        let parsed = episode("random/The.Office.US.S02E03.HDTV.XviD-GROUP.mkv");
        assert_eq!(parsed.title, "The Office US");
        assert_eq!((parsed.season, parsed.episode), (Some(2), Some(3)));
    }

    #[test]
    fn an_episode_whose_filename_is_only_a_marker_borrows_the_folder_name() {
        let parsed = episode("Shows/Severance/S01E02.mkv");
        assert_eq!(parsed.title, "Severance");
    }

    #[test]
    fn a_season_folder_with_an_unnumbered_file_is_flagged_rather_than_guessed() {
        let parsed = episode("Shows/Lost/Season 2/pilot.mkv");
        assert_eq!(parsed.title, "Lost");
        assert_eq!(parsed.season, Some(2));
        assert_eq!(parsed.episode, Some(0));
        assert!(
            parsed.confidence < basalt_proto::msg::CONFIDENT,
            "an episode number that had to be invented is not a confident match"
        );
    }

    // -----------------------------------------------------------------------
    // Title cleaning
    // -----------------------------------------------------------------------

    #[test]
    fn release_noise_is_stripped() {
        assert_eq!(
            clean_title("Arrival.2016.1080p.BluRay.x264-SPARKS"),
            "Arrival"
        );
        assert_eq!(clean_title("The_Matrix_1999_REMUX"), "The Matrix");
        assert_eq!(
            clean_title("Dune Part Two 2024 2160p DV HDR"),
            "Dune Part Two"
        );
    }

    #[test]
    fn a_release_group_suffix_is_dropped() {
        assert_eq!(clean_title("Some Film 2020-RARBG"), "Some Film");
    }

    // A hyphen inside a title is not a release group.
    #[test]
    fn a_hyphenated_title_is_not_truncated() {
        assert_eq!(clean_title("Spider-Man"), "Spider-Man");
        assert_eq!(
            clean_title("X-Men Days of Future Past"),
            "X-Men Days of Future Past"
        );
    }

    #[test]
    fn cleaning_never_produces_stray_punctuation() {
        for raw in [
            "Arrival...2016",
            "  The  Thing  ",
            "[Group] Show - 01 [1080p]",
            "a.b.c.1080p",
        ] {
            let cleaned = clean_title(raw);
            assert!(
                !cleaned.starts_with(' ') && !cleaned.ends_with(' '),
                "{cleaned:?}"
            );
            assert!(!cleaned.contains("  "), "{cleaned:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Robustness
    // -----------------------------------------------------------------------

    #[test]
    fn nothing_here_panics_on_awkward_input() {
        for path in [
            "",
            "/",
            ".mkv",
            "a/",
            "S01E01.mkv",
            "////a.mkv",
            "…/ünïcödé (2020).mkv",
            &format!("{}.mkv", "x".repeat(5000)),
            "1x.mkv",
            "sE.mkv",
            "Season /a.mkv",
        ] {
            let _ = parse(path);
        }
    }

    #[test]
    fn unicode_titles_survive() {
        let parsed = film("films/Amélie (2001).mkv");
        assert_eq!(parsed.title, "Amélie");
        assert_eq!(parsed.year, Some(2001));
    }

    #[test]
    fn a_non_video_is_not_parsed_at_all() {
        assert!(parse("films/Arrival.2016.srt").is_none());
        assert!(parse("films/poster.jpg").is_none());
    }
}
