//! How big each film and episode's picture is, for the HD / FHD / 4K tag.
//!
//! Measured from the file — its header, read through mpv — because the name
//! is not reliable: plenty of files say nothing, and some say `1080p` on an
//! upscaled 720p. The name is still read, as the answer until a file has been
//! measured and for one that cannot be.
//!
//! Measured once. The answer is kept by path, size and modification time, so
//! a rescan costs nothing and a replaced file is measured again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use basalt_proto::msg::{LibraryItem, Resolution};
use serde::{Deserialize, Serialize};

/// What a release name says about its picture, when it says anything.
pub fn from_name(path: &str) -> Option<Resolution> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let tokens: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let at = |w: u32, h: u32| {
        Some(Resolution {
            width: w,
            height: h,
        })
    };

    for token in &tokens {
        match *token {
            "2160p" | "4k" | "uhd" | "2160" => return at(3840, 2160),
            "1440p" | "2k" => return at(2560, 1440),
            "1080p" | "1080i" | "fhd" => return at(1920, 1080),
            "720p" => return at(1280, 720),
            "576p" | "576i" => return at(1024, 576),
            "480p" | "480i" => return at(854, 480),
            _ => {}
        }
        // `1920x1080`.
        if let Some((w, h)) = token.split_once('x')
            && let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
            && (320..=16_384).contains(&w)
            && (240..=16_384).contains(&h)
        {
            return at(w, h);
        }
    }
    None
}

/// What has been measured, kept beside the library index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Measured {
    /// By [`key`]. `None` for a file that could not be read, so it is not
    /// tried again on every scan.
    #[serde(default)]
    sizes: HashMap<String, Option<Resolution>>,
}

/// One file's identity: its path, and enough to notice it being replaced.
pub fn key(path: &str, size: u64, mtime: i64) -> String {
    format!("{path}\n{size}\n{mtime}")
}

impl Measured {
    pub fn path_for(config_dir: &Path, vault_root: &Path) -> PathBuf {
        let key = blake3::hash(vault_root.to_string_lossy().as_bytes()).to_hex();
        config_dir.join(format!("resolutions-{}.json", &key[..16]))
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(self)?)?;
        std::fs::rename(&temp, path)
    }

    /// Whether this file has been looked at, successfully or not.
    pub fn knows(&self, key: &str) -> bool {
        self.sizes.contains_key(key)
    }

    pub fn record(&mut self, key: String, size: Option<Resolution>) {
        self.sizes.insert(key, size);
    }

    /// Drops files the library no longer has, so the list does not grow for
    /// ever as files come and go.
    pub fn retain(&mut self, keep: &std::collections::HashSet<String>) {
        self.sizes.retain(|k, _| keep.contains(k));
    }

    /// The size to show: measured if it was, the name's otherwise.
    fn lookup(&self, path: &str, size: u64, mtime: i64) -> Option<Resolution> {
        match self.sizes.get(&key(path, size, mtime)) {
            Some(Some(measured)) => Some(*measured),
            _ => from_name(path),
        }
    }

    /// Writes a size onto every film and episode.
    pub fn apply(&self, items: &mut [LibraryItem]) {
        for item in items {
            if let Some(path) = &item.path {
                item.resolution = self.lookup(path, item.size, item.added);
            }
            for season in &mut item.seasons {
                for episode in &mut season.episodes {
                    episode.resolution = self.lookup(&episode.path, episode.size, episode.added);
                }
            }
        }
    }
}

/// Every file in the library, as `(key, path)`, once each.
pub fn files_of(items: &[LibraryItem]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for item in items {
        if let Some(path) = &item.path {
            out.push((key(path, item.size, item.added), path.clone()));
        }
        for season in &item.seasons {
            for episode in &season.episodes {
                out.push((
                    key(&episode.path, episode.size, episode.added),
                    episode.path.clone(),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(w: u32, h: u32) -> Option<Resolution> {
        Some(Resolution {
            width: w,
            height: h,
        })
    }

    #[test]
    fn reads_what_release_names_say() {
        assert_eq!(
            from_name("Night.Harbour.S01E01.2160p.WEB-DL.x265.mkv"),
            res(3840, 2160)
        );
        assert_eq!(from_name("Coastline_1080P_S01_E01.mp4"), res(1920, 1080));
        assert_eq!(from_name("Arrival (2016) [720p].mkv"), res(1280, 720));
        assert_eq!(from_name("Film.UHD.BluRay.mkv"), res(3840, 2160));
        assert_eq!(from_name("Recording_1920x1080.mp4"), res(1920, 1080));
    }

    #[test]
    fn says_nothing_when_the_name_does_not() {
        assert_eq!(from_name("Night Harbour.mkv"), None);
        // Numbers that are not a picture size.
        assert_eq!(from_name("Blade Runner 2049 (2017).mkv"), None);
        assert_eq!(from_name("Episode 1080.mkv"), None);
    }

    #[test]
    fn a_measured_size_beats_the_name_and_a_failed_one_falls_back() {
        let mut measured = Measured::default();
        let path = "tv/Show.S01E01.1080p.mkv";
        measured.record(key(path, 10, 5), res(1280, 720));
        assert_eq!(measured.lookup(path, 10, 5), res(1280, 720));
        // Replaced since: not what was measured.
        assert_eq!(measured.lookup(path, 11, 6), res(1920, 1080));
        measured.record(key(path, 11, 6), None);
        assert!(measured.knows(&key(path, 11, 6)));
        assert_eq!(measured.lookup(path, 11, 6), res(1920, 1080));
    }
}
