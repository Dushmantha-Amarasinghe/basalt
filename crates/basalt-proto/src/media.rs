//! What kind of file something is, by its name.
//!
//! One list, shared by the host that sorts the drive and anything else that
//! needs to agree with it. The client once kept three lists of its own —
//! one for the player, one for its sections, one for thumbnails — and they
//! disagreed: a `.m2ts` played but never appeared under Videos.

/// Videos: anything the player opens that has pictures.
pub const VIDEO: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "m4v", "webm", "wmv", "flv", "ts", "m2ts", "mts", "mpg", "mpeg",
    "vob", "divx", "ogv", "rmvb", "asf", "3gp",
];

/// Music and other sound.
pub const AUDIO: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "aac", "ogg", "oga", "opus", "wma", "aiff", "aif", "alac", "ape",
    "wv", "mka",
];

/// Photos and pictures.
pub const IMAGE: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "webp", "avif", "bmp", "heic", "heif", "tif", "tiff",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

/// The extension, lower-cased, without the dot. Empty when there is none.
pub fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// What a file is, from its name alone.
pub fn kind_of(name: &str) -> Option<MediaKind> {
    let ext = extension(name);
    let ext = ext.as_str();
    if VIDEO.contains(&ext) {
        Some(MediaKind::Video)
    } else if AUDIO.contains(&ext) {
        Some(MediaKind::Audio)
    } else if IMAGE.contains(&ext) {
        Some(MediaKind::Image)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_by_extension_whatever_the_case() {
        assert_eq!(kind_of("Films/Arrival.2016.MKV"), Some(MediaKind::Video));
        assert_eq!(kind_of("Concert.m2ts"), Some(MediaKind::Video));
        assert_eq!(kind_of("Music/Album/01 Track.flac"), Some(MediaKind::Audio));
        assert_eq!(kind_of("26_05_12_19_37_04.png"), Some(MediaKind::Image));
        assert_eq!(kind_of("IMG_0001.HEIC"), Some(MediaKind::Image));
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(kind_of("notes.docx"), None);
        assert_eq!(kind_of("Makefile"), None);
        // A dotfile has no extension: `.mp4` is a name, not a video.
        assert_eq!(kind_of(".mp4"), None);
        assert_eq!(kind_of("film.mp4.part"), None);
    }
}
