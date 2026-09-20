//! Matching subtitle files to the films and episodes they belong to.
//!
//! Nobody names these consistently. The same season folder will hold
//! `Show.S01E01.srt` beside `Show S01E01 [group].en.srt` beside a `Subs/`
//! directory with `2_English.srt` inside it, and all three are the subtitles
//! for episode one. So this matches on evidence rather than on a convention:
//!
//! 1. **The same name.** `Arrival (2016).mkv` and `Arrival (2016).en.srt`.
//!    Strongest, and the common case.
//! 2. **The same episode.** Anything carrying the same `SxxExx` in the same
//!    folder, or in a `Subs`/`Subtitles` folder beside it. This is what
//!    catches the releases where the subtitle name shares nothing else with
//!    the video's.
//! 3. **A folder named after the video.** `Subs/Arrival (2016)/3_English.srt`,
//!    which is how several rippers lay it out.
//!
//! Deliberately *not* matched: any subtitle in the same folder when there is
//! only one video. That rule reads well and is wrong exactly when it matters —
//! a season folder with one leftover episode picks up the wrong language, and
//! a wrong subtitle is worse than none.

use std::collections::HashMap;

use super::parse;

/// Extensions treated as subtitles.
///
/// `idx` is left out on purpose: it is the index half of a `sub`/`idx` pair
/// and listing both would offer the same subtitle twice.
const SUBTITLE: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "sup"];

/// Folders that hold subtitles for the videos beside them.
const SUB_FOLDERS: &[&str] = &["subs", "subtitles", "sub", "subtitle"];

/// One subtitle file, ready to be offered in the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtitle {
    /// Vault-relative path.
    pub path: String,
    /// What to call it: `English`, `English (SDH)`, or the filename when
    /// nothing better can be read out of it.
    pub label: String,
}

pub fn is_subtitle(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            SUBTITLE.contains(&ext.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

/// The filename without its extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Whether this path sits in a folder set aside for subtitles.
fn in_sub_folder(path: &str) -> bool {
    folder(path)
        .split('/')
        .any(|segment| SUB_FOLDERS.contains(&segment.trim().to_ascii_lowercase().as_str()))
}

/// A normalised form for comparing names: letters and digits only.
///
/// Releases differ by punctuation constantly — `Show.S01E01` against
/// `Show S01E01` — and none of it carries meaning.
fn key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Reads a language out of what is left of a subtitle's name.
///
/// `Show.S01E01.en.forced.srt` next to `Show.S01E01.mkv` leaves `en.forced`,
/// which is exactly the useful part. Two-and three-letter codes are expanded
/// because `en` on a menu is worse than `English`.
pub fn label_for(sub_path: &str, video_stem: &str) -> String {
    let name = stem(sub_path);
    let extra = suffix_after(name, video_stem)
        .map(|rest| rest.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(name);

    // `3_English` — the numbering rippers use inside a `Subs` folder.
    let extra = extra
        .split_once('_')
        .filter(|(head, _)| head.chars().all(|c| c.is_ascii_digit()))
        .map_or(extra, |(_, tail)| tail);

    let words: Vec<String> = extra
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| expand(&w.to_ascii_lowercase()).to_string())
        .collect();

    // Nothing beyond the video's own name: there is no language to report,
    // and repeating the film's title in its subtitle menu says nothing.
    if words.is_empty() || key(&words.join("")) == key(video_stem) {
        "Subtitles".to_string()
    } else {
        words.join(" ")
    }
}

/// What is left of `name` once the video's name has been read off the front.
///
/// Compared through [`key`] rather than literally, because the two spell the
/// same title differently as a matter of course: `Outlander.S01E01.eng.srt`
/// sits beside `Outlander S01E01.mkv`, and a literal `strip_prefix` leaves the
/// whole filename — so the subtitle ends up labelled `Outlander S01e01
/// English` instead of `English`.
fn suffix_after<'a>(name: &'a str, video_stem: &str) -> Option<&'a str> {
    let want = key(video_stem);
    if want.is_empty() {
        return None;
    }

    let mut matched = 0usize;
    for (at, c) in name.char_indices() {
        if matched == want.len() {
            return Some(&name[at..]);
        }
        if !c.is_alphanumeric() {
            continue;
        }
        let lowered: String = c.to_lowercase().collect();
        if !want[matched..].starts_with(&lowered) {
            return None;
        }
        matched += lowered.len();
    }
    // The whole name was the video's name, with nothing appended.
    (matched == want.len()).then_some("")
}

/// Turns a language code into a name, and leaves anything else alone.
fn expand(word: &str) -> String {
    let named = match word {
        "en" | "eng" | "english" => "English",
        "es" | "spa" | "spanish" => "Spanish",
        "fr" | "fre" | "fra" | "french" => "French",
        "de" | "ger" | "deu" | "german" => "German",
        "it" | "ita" | "italian" => "Italian",
        "pt" | "por" | "portuguese" => "Portuguese",
        "nl" | "dut" | "nld" | "dutch" => "Dutch",
        "ru" | "rus" | "russian" => "Russian",
        "ja" | "jpn" | "japanese" => "Japanese",
        "ko" | "kor" | "korean" => "Korean",
        "zh" | "chi" | "zho" | "chinese" => "Chinese",
        "ar" | "ara" | "arabic" => "Arabic",
        "hi" | "hin" | "hindi" => "Hindi",
        "si" | "sin" | "sinhala" => "Sinhala",
        "ta" | "tam" | "tamil" => "Tamil",
        "sv" | "swe" | "swedish" => "Swedish",
        "da" | "dan" | "danish" => "Danish",
        "no" | "nor" | "norwegian" => "Norwegian",
        "fi" | "fin" | "finnish" => "Finnish",
        "pl" | "pol" | "polish" => "Polish",
        "tr" | "tur" | "turkish" => "Turkish",
        "sdh" => "SDH",
        "forced" => "forced",
        other => return capitalise(other),
    };
    named.to_string()
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Picks the subtitles belonging to one video out of everything found.
///
/// `candidates` is every subtitle file on the drive, vault-relative.
pub fn for_video(video: &str, candidates: &[String]) -> Vec<Subtitle> {
    let video_stem = stem(video);
    let video_key = key(video_stem);
    let video_folder = folder(video);
    let episode = parse::parse(video).filter(|p| p.is_episode());

    let mut found: Vec<Subtitle> = Vec::new();
    for candidate in candidates {
        if !belongs(candidate, video, &video_key, video_folder, episode.as_ref()) {
            continue;
        }
        found.push(Subtitle {
            label: label_for(candidate, video_stem),
            path: candidate.clone(),
        });
    }

    // Stable, and by label so a menu reads alphabetically.
    found.sort_by(|a, b| a.label.cmp(&b.label).then(a.path.cmp(&b.path)));
    found.dedup_by(|a, b| a.path == b.path);
    found
}

fn belongs(
    candidate: &str,
    video: &str,
    video_key: &str,
    video_folder: &str,
    episode: Option<&parse::Parsed>,
) -> bool {
    // What the film is called, which is the file's name or — when the file is
    // a bare `movie.mkv` inside a titled folder — the folder's.
    let folder_name = video_folder.rsplit('/').next().unwrap_or("");
    if candidate == video {
        return false;
    }
    let candidate_key = key(stem(candidate));
    let candidate_folder = folder(candidate);

    // 1. Named after the video, with anything appended: `.en`, `.forced`.
    if candidate_key.starts_with(video_key) && !video_key.is_empty() {
        return true;
    }

    // Everything below requires being somewhere near the video: the same
    // folder, or a subtitle folder under it. Without that, one episode's
    // subtitles would be offered for the same episode of a different show.
    let near = candidate_folder == video_folder
        || (in_sub_folder(candidate) && candidate_folder.starts_with(video_folder));
    if !near {
        return false;
    }

    // 2. A folder named after the video: `Subs/Arrival (2016)/3_English.srt`.
    // Only inside a subtitle folder. Without that guard the "folder named
    // after the film" rule matches the folder the film merely *lives* in, so
    // every subtitle in `Films/` was claimed by every film in `Films/`.
    if in_sub_folder(candidate)
        && candidate_folder.rsplit('/').next().is_some_and(|leaf| {
            let leaf = key(leaf);
            leaf == video_key || (!folder_name.is_empty() && leaf == key(folder_name))
        })
    {
        return true;
    }

    // 3. The same episode number, however the rest of the name is spelled.
    if let Some(episode) = episode
        && let Some(theirs) = parse::episode_numbers(stem(candidate))
    {
        return theirs == (episode.season.unwrap_or(0), episode.episode.unwrap_or(0));
    }

    false
}

/// Every subtitle, grouped by the video it belongs to.
///
/// One pass rather than one scan per video: a season folder has thirty of
/// each, and matching them pairwise would be nine hundred comparisons for
/// what is really thirty.
pub fn map_all(videos: &[String], candidates: &[String]) -> HashMap<String, Vec<Subtitle>> {
    videos
        .iter()
        .map(|video| (video.clone(), for_video(video, candidates)))
        .filter(|(_, subs)| !subs.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn labels(subs: &[Subtitle]) -> Vec<&str> {
        subs.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn a_subtitle_is_recognised_by_its_extension() {
        for name in ["a.srt", "a.ASS", "a.ssa", "a.vtt", "a.sub", "a.sup"] {
            assert!(is_subtitle(name), "{name}");
        }
        for name in ["a.mkv", "a.idx", "a.txt", "srt", ".srt"] {
            assert!(!is_subtitle(name), "{name}");
        }
    }

    /// The common case, and the one that has to be exactly right.
    #[test]
    fn a_subtitle_named_after_the_film_belongs_to_it() {
        let subs = for_video(
            "Films/Arrival (2016).mkv",
            &paths(&[
                "Films/Arrival (2016).srt",
                "Films/Arrival (2016).en.srt",
                "Films/Arrival (2016).es.forced.srt",
                "Films/Sicario (2015).en.srt",
            ]),
        );
        assert_eq!(labels(&subs), ["English", "Spanish forced", "Subtitles"]);
        assert!(!subs.iter().any(|s| s.path.contains("Sicario")));
    }

    /// Punctuation differs between the video and its subtitles constantly.
    #[test]
    fn punctuation_between_the_two_names_does_not_matter() {
        let subs = for_video(
            "Shows/Show.S01E01.1080p.mkv",
            &paths(&["Shows/Show S01E01 1080p.eng.srt"]),
        );
        assert_eq!(subs.len(), 1, "got {subs:?}");
    }

    /// The case the "same name" rule cannot reach: a subtitle that shares the
    /// episode number and nothing else.
    #[test]
    fn a_subtitle_sharing_only_the_episode_number_still_belongs() {
        let subs = for_video(
            "Show/Season 1/Show_1080P_S01_E03.mkv",
            &paths(&[
                "Show/Season 1/Subs/Show.S01E03.WEB.en.srt",
                "Show/Season 1/Subs/Show.S01E04.WEB.en.srt",
            ]),
        );
        assert_eq!(subs.len(), 1, "only episode three, got {subs:?}");
        assert!(subs[0].path.ends_with("S01E03.WEB.en.srt"));
    }

    #[test]
    fn a_subs_folder_named_after_the_film_belongs_to_it() {
        let subs = for_video(
            "Films/Arrival (2016)/movie.mkv",
            &paths(&[
                "Films/Arrival (2016)/Subs/Arrival (2016)/3_English.srt",
                "Films/Arrival (2016)/Subs/Arrival (2016)/7_Spanish.srt",
            ]),
        );
        assert_eq!(labels(&subs), ["English", "Spanish"]);
    }

    /// The rule that reads well and is wrong: "the only subtitle in the
    /// folder must be the one". A season folder full of episodes would give
    /// every one of them the same subtitle.
    #[test]
    fn an_unrelated_subtitle_in_the_same_folder_is_not_claimed() {
        let subs = for_video(
            "Show/Season 1/Show S01E01.mkv",
            &paths(&["Show/Season 1/something entirely else.srt"]),
        );
        assert!(subs.is_empty(), "got {subs:?}");
    }

    /// Nearness matters: the same episode number of a different series must
    /// not be pulled in from elsewhere on the drive.
    #[test]
    fn the_same_episode_of_another_show_is_not_claimed() {
        let subs = for_video(
            "Outlander/Season 1/Outlander S01E01.mkv",
            &paths(&["Alien Earth/Season 1/Alien.Earth.S01E01.en.srt"]),
        );
        assert!(subs.is_empty(), "got {subs:?}");
    }

    #[test]
    fn a_language_code_becomes_a_language() {
        assert_eq!(label_for("x/Film.en.srt", "Film"), "English");
        assert_eq!(label_for("x/Film.fre.srt", "Film"), "French");
        assert_eq!(label_for("x/Film.en.sdh.srt", "Film"), "English SDH");
        assert_eq!(label_for("x/Film.si.srt", "Film"), "Sinhala");
        // Nothing recognisable: better the filename than a wrong guess.
        assert_eq!(label_for("x/Film.qq.srt", "Film"), "Qq");
        assert_eq!(label_for("x/12_Korean.srt", "Other"), "Korean");
    }

    #[test]
    fn mapping_many_videos_at_once_keeps_them_apart() {
        let videos = paths(&["S/Season 1/E01.mkv", "S/Season 1/E02.mkv"]);
        let subs = paths(&["S/Season 1/E01.en.srt", "S/Season 1/E02.en.srt"]);
        let map = map_all(&videos, &subs);
        assert_eq!(map.len(), 2);
        assert_eq!(map["S/Season 1/E01.mkv"][0].path, "S/Season 1/E01.en.srt");
        assert_eq!(map["S/Season 1/E02.mkv"][0].path, "S/Season 1/E02.en.srt");
    }

    #[test]
    fn a_video_with_no_subtitles_is_left_out_of_the_map() {
        let map = map_all(&paths(&["a.mkv"]), &paths(&["unrelated.srt"]));
        assert!(map.is_empty());
    }
}
