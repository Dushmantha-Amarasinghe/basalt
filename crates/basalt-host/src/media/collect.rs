//! Every video, song and photo on the drive, sorted into what it is.
//!
//! What the Videos, Music, Photos and Recent sections show. Each device used
//! to find these for itself by listing the top of the drive and one level
//! down, once per connection — so a photo two folders deep never appeared,
//! and one uploaded after the device connected did not either until the app
//! was restarted. Now the host sorts them during the walk it already makes for
//! films, keeps them current as files come and go, and hands every device the
//! same answer.
//!
//! Sorted by name alone, like the rest of the app: a file called `.mp4` is a
//! video. Nothing is opened to decide — except a photo's header, once, for the
//! size it has to be laid out at.

use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};

use basalt_proto::media::{MediaKind, kind_of};
use basalt_proto::msg::{Collections, MediaFile};
use serde::{Deserialize, Serialize};

/// Files kept in each collection, newest first.
///
/// A bound on what one answer carries. A drive with more photos than this
/// shows the newest, and says it was cut short.
pub const MAX_PER_KIND: usize = 50_000;

/// Files in Recent.
pub const RECENT: usize = 300;

/// How old an empty partial upload has to be before it is swept away.
///
/// An upload that never got going — refused, or given up on before a byte
/// arrived. Nothing to resume, so an hour is plenty of margin.
pub const STALE_EMPTY_PART_SECS: i64 = 60 * 60;

/// How old any other partial upload has to be.
///
/// These can be resumed, so they are given a day: long enough for somebody
/// to come back to an upload the Wi-Fi interrupted.
pub const STALE_PART_SECS: i64 = 24 * 60 * 60;

/// Whether a partial upload has been left long enough to remove.
pub fn is_stale_part(size: u64, mtime: i64, now: i64) -> bool {
    let age = now - mtime;
    if size == 0 {
        age >= STALE_EMPTY_PART_SECS
    } else {
        age >= STALE_PART_SECS
    }
}

/// The collections as kept on disk, with the revision clients poll against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub collections: Collections,
}

impl Stored {
    /// Replaces the contents, bumping the revision only if anything differs.
    pub fn replace(&mut self, collections: Collections) -> bool {
        if self.collections == collections {
            return false;
        }
        self.collections = collections;
        self.revision += 1;
        true
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
        let json = serde_json::to_vec(self)?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, path)
    }
}

/// Where the collections live: beside `host.json`, keyed by drive like the
/// library index, so two drives never share one.
pub fn stored_path(config_dir: &Path, vault_root: &Path) -> PathBuf {
    let key = blake3::hash(vault_root.to_string_lossy().as_bytes()).to_hex();
    config_dir.join(format!("collections-{}.json", &key[..16]))
}

/// Collects files as a walk passes them.
#[derive(Debug, Default)]
pub struct Gatherer {
    videos: Vec<MediaFile>,
    music: Vec<MediaFile>,
    photos: Vec<MediaFile>,
    /// The newest files of any kind, as a min-heap so the oldest drops out.
    recent: BinaryHeap<std::cmp::Reverse<(i64, String, u64)>>,
}

impl Gatherer {
    /// Notes one file. `path` is vault-relative.
    pub fn note(&mut self, path: &str, size: u64, mtime: i64) {
        self.recent
            .push(std::cmp::Reverse((mtime, path.to_string(), size)));
        if self.recent.len() > RECENT {
            self.recent.pop();
        }
        let file = || MediaFile {
            path: path.to_string(),
            size,
            mtime,
            width: None,
            height: None,
        };
        match kind_of(path) {
            Some(MediaKind::Video) => self.videos.push(file()),
            Some(MediaKind::Audio) => self.music.push(file()),
            Some(MediaKind::Image) => self.photos.push(file()),
            None => {}
        }
    }

    /// The collections, with photo sizes filled in.
    ///
    /// `previous` supplies sizes already read, for any photo that has not
    /// changed since; `measure` reads the rest. A drive of ten thousand photos
    /// is read once, not on every scan.
    pub fn finish(
        self,
        previous: &Collections,
        measure: impl Fn(&str) -> Option<(u32, u32)>,
    ) -> Collections {
        let mut truncated = false;
        let mut sorted = |mut files: Vec<MediaFile>| {
            files.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
            if files.len() > MAX_PER_KIND {
                files.truncate(MAX_PER_KIND);
                truncated = true;
            }
            files
        };
        let videos = sorted(self.videos);
        let music = sorted(self.music);
        let mut photos = sorted(self.photos);
        measure_photos(&mut photos, previous, measure);

        let mut recent: Vec<MediaFile> = self
            .recent
            .into_iter()
            .map(|std::cmp::Reverse((mtime, path, size))| MediaFile {
                path,
                size,
                mtime,
                width: None,
                height: None,
            })
            .collect();
        recent.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));

        Collections {
            videos,
            music,
            photos,
            recent,
            truncated,
        }
    }
}

/// Fills in photo sizes: from `previous` where the file is unchanged, and
/// from `measure` otherwise.
fn measure_photos(
    photos: &mut [MediaFile],
    previous: &Collections,
    measure: impl Fn(&str) -> Option<(u32, u32)>,
) {
    let known: HashMap<&str, &MediaFile> = previous
        .photos
        .iter()
        .map(|p| (p.path.as_str(), p))
        .collect();
    for photo in photos {
        let size = match known.get(photo.path.as_str()) {
            Some(old) if old.size == photo.size && old.mtime == photo.mtime => {
                old.width.zip(old.height)
            }
            _ => measure(&photo.path),
        };
        if let Some((w, h)) = size {
            photo.width = Some(w);
            photo.height = Some(h);
        }
    }
}

/// Files that arrived since the last walk, added without walking again.
///
/// Anything already listed under the same path is replaced, so a file
/// written twice is listed once, at its newest.
pub fn add(
    existing: &Collections,
    arrived: &[MediaFile],
    measure: impl Fn(&str) -> Option<(u32, u32)>,
) -> Collections {
    let mut next = existing.clone();
    for file in arrived {
        let target = match kind_of(&file.path) {
            Some(MediaKind::Video) => Some(&mut next.videos),
            Some(MediaKind::Audio) => Some(&mut next.music),
            Some(MediaKind::Image) => Some(&mut next.photos),
            None => None,
        };
        let mut file = file.clone();
        if let Some(list) = target {
            if kind_of(&file.path) == Some(MediaKind::Image)
                && let Some((w, h)) = measure(&file.path)
            {
                file.width = Some(w);
                file.height = Some(h);
            }
            list.retain(|f| f.path != file.path);
            list.insert(0, file.clone());
            list.truncate(MAX_PER_KIND);
        }
        next.recent.retain(|f| f.path != file.path);
        next.recent.insert(
            0,
            MediaFile {
                width: None,
                height: None,
                ..file
            },
        );
        next.recent.truncate(RECENT);
    }
    next
}

/// Files and folders that went away. A folder takes everything under it.
pub fn remove(existing: &Collections, gone: &[String]) -> Collections {
    let under = |path: &str| {
        gone.iter().any(|g| {
            path == g
                || (path.len() > g.len()
                    && path.starts_with(g.as_str())
                    && path.as_bytes()[g.len()] == b'/')
        })
    };
    let keep = |files: &[MediaFile]| -> Vec<MediaFile> {
        files.iter().filter(|f| !under(&f.path)).cloned().collect()
    };
    Collections {
        videos: keep(&existing.videos),
        music: keep(&existing.music),
        photos: keep(&existing.photos),
        recent: keep(&existing.recent),
        truncated: existing.truncated,
    }
}

/// A photo's size as it is meant to be seen, from its header alone.
///
/// Turned for the camera's orientation: a portrait photo from a phone is
/// stored sideways with a note to turn it, and laying it out by the stored
/// size would put a tall picture in a wide space. `None` for anything the
/// header reader does not know, which the grid lays out at a common shape.
pub fn photo_size(path: &Path) -> Option<(u32, u32)> {
    use image::ImageDecoder;

    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    let turned = matches!(
        decoder.orientation().ok()?,
        image::metadata::Orientation::Rotate90
            | image::metadata::Orientation::Rotate270
            | image::metadata::Orientation::Rotate90FlipH
            | image::metadata::Orientation::Rotate270FlipH
    );
    Some(if turned { (h, w) } else { (w, h) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gathered(files: &[(&str, u64, i64)]) -> Collections {
        let mut g = Gatherer::default();
        for (path, size, mtime) in files {
            g.note(path, *size, *mtime);
        }
        g.finish(&Collections::default(), |_| None)
    }

    fn paths(files: &[MediaFile]) -> Vec<&str> {
        files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn sorts_every_file_into_its_kind_newest_first() {
        let c = gathered(&[
            ("26_05_12_19_37_04.png", 5_300_000, 300),
            ("Photos/2024/Trip/IMG_0001.HEIC", 3_000_000, 100),
            ("Music/Album/01 Opening.flac", 30_000_000, 200),
            ("Landman/S01/Landman.S01E01.mkv", 3_000_000_000, 150),
            ("Concert.m2ts", 9_000_000, 400),
            ("notes.docx", 15_000, 500),
        ]);
        assert_eq!(
            paths(&c.photos),
            ["26_05_12_19_37_04.png", "Photos/2024/Trip/IMG_0001.HEIC"]
        );
        assert_eq!(paths(&c.music), ["Music/Album/01 Opening.flac"]);
        assert_eq!(
            paths(&c.videos),
            ["Concert.m2ts", "Landman/S01/Landman.S01E01.mkv"]
        );
        // Recent is everything, whatever it is.
        assert_eq!(c.recent[0].path, "notes.docx");
        assert_eq!(c.recent.len(), 6);
        assert!(!c.truncated);
    }

    #[test]
    fn recent_keeps_only_the_newest() {
        let mut g = Gatherer::default();
        for i in 0..(RECENT as i64 + 50) {
            g.note(&format!("file{i}.txt"), 1, i);
        }
        let c = g.finish(&Collections::default(), |_| None);
        assert_eq!(c.recent.len(), RECENT);
        assert_eq!(c.recent[0].mtime, RECENT as i64 + 49);
        assert_eq!(c.recent.last().unwrap().mtime, 50);
    }

    #[test]
    fn a_collection_past_its_cap_says_so() {
        let mut g = Gatherer::default();
        for i in 0..(MAX_PER_KIND as i64 + 3) {
            g.note(&format!("p/{i}.jpg"), 1, i);
        }
        let c = g.finish(&Collections::default(), |_| None);
        assert_eq!(c.photos.len(), MAX_PER_KIND);
        assert!(c.truncated);
        assert_eq!(
            c.photos[0].mtime,
            MAX_PER_KIND as i64 + 2,
            "the newest are kept"
        );
    }

    #[test]
    fn photo_sizes_are_read_once() {
        let first = {
            let mut g = Gatherer::default();
            g.note("a.jpg", 10, 1);
            g.note("b.jpg", 20, 2);
            g.finish(&Collections::default(), |_| Some((4000, 3000)))
        };
        let asked = std::cell::RefCell::new(Vec::new());
        let second = {
            let mut g = Gatherer::default();
            g.note("a.jpg", 10, 1);
            // Changed since: read again.
            g.note("b.jpg", 25, 9);
            g.finish(&first, |p| {
                asked.borrow_mut().push(p.to_string());
                Some((3000, 4000))
            })
        };
        assert_eq!(*asked.borrow(), ["b.jpg"]);
        let a = second.photos.iter().find(|p| p.path == "a.jpg").unwrap();
        assert_eq!((a.width, a.height), (Some(4000), Some(3000)));
        let b = second.photos.iter().find(|p| p.path == "b.jpg").unwrap();
        assert_eq!((b.width, b.height), (Some(3000), Some(4000)));
    }

    #[test]
    fn arrivals_go_to_the_front_once() {
        let start = gathered(&[("old.png", 1, 10)]);
        let file = MediaFile {
            path: "new.png".into(),
            size: 5,
            mtime: 20,
            width: None,
            height: None,
        };
        let once = add(&start, std::slice::from_ref(&file), |_| Some((2, 1)));
        let twice = add(&once, &[file], |_| Some((2, 1)));
        assert_eq!(paths(&twice.photos), ["new.png", "old.png"]);
        assert_eq!(twice.photos[0].width, Some(2));
        assert_eq!(paths(&twice.recent), ["new.png", "old.png"]);
    }

    #[test]
    fn a_removed_folder_takes_everything_under_it_and_nothing_beside_it() {
        let start = gathered(&[
            ("Trip/a.jpg", 1, 1),
            ("Trip/Day 2/b.jpg", 1, 2),
            ("Trip Two/c.jpg", 1, 3),
            ("song.mp3", 1, 4),
        ]);
        let after = remove(&start, &["Trip".to_string(), "song.mp3".to_string()]);
        assert_eq!(paths(&after.photos), ["Trip Two/c.jpg"]);
        assert!(after.music.is_empty());
        assert_eq!(paths(&after.recent), ["Trip Two/c.jpg"]);
    }

    #[test]
    fn only_old_partial_uploads_are_stale() {
        let now = 1_000_000;
        assert!(
            !is_stale_part(0, now - 60, now),
            "just refused: give it time"
        );
        assert!(is_stale_part(0, now - STALE_EMPTY_PART_SECS, now));
        assert!(
            !is_stale_part(5_000, now - STALE_EMPTY_PART_SECS, now),
            "resumable"
        );
        assert!(is_stale_part(5_000, now - STALE_PART_SECS, now));
    }

    #[test]
    fn a_photo_is_measured_as_it_is_meant_to_be_seen() {
        let dir = std::env::temp_dir().join(format!("basalt-photo-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wide.png");
        image::RgbImage::new(64, 36).save(&path).unwrap();
        assert_eq!(photo_size(&path), Some((64, 36)));
        assert_eq!(photo_size(&dir.join("missing.png")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
