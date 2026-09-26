//! Small pictures of videos and photos, made on the host.
//!
//! On the host because the host has the file. A device making its own would
//! stream part of every video across the Wi-Fi to take one frame of it, on
//! every device, every time; here it is a short read from a local disk, done
//! once, and every device gets the same picture.
//!
//! **Photos** in the formats a camera or a screenshot produces are decoded and
//! shrunk directly. **Videos**, and photos in formats nothing else here reads
//! (HEIC from a phone), go through mpv — the same library the client plays
//! with, loaded from a DLL beside the host. It writes one frame from about 15%
//! in, which is past the logos and cold opens most of the time; a frame that
//! comes out nearly black is tried again further in.
//!
//! Everything made is kept, keyed by the file's path, size and modification
//! time, so a changed file gets a new picture and an unchanged one is never
//! made twice.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use basalt_proto::media::{MediaKind, kind_of};

/// The long side of a grid thumbnail.
pub const GRID: u32 = 320;

/// The long side of a picture made for viewing, for photos a device cannot
/// open itself.
pub const VIEW: u32 = 1600;

/// Mean brightness, out of 255, below which a frame counts as black.
const DARK: f64 = 22.0;

/// Where into a video to look, in turn, until a frame is not black.
const POSITIONS: [&str; 3] = ["15%", "35%", "55%"];

/// How long one frame may take before mpv is given up on.
const MPV_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

/// Pictures kept before the oldest are cleared out.
const MAX_CACHED: usize = 40_000;

/// The size actually made for a size asked for: one of two.
pub fn bucket(asked: u32) -> u32 {
    if asked > 480 { VIEW } else { GRID }
}

/// The cache file for one picture.
pub fn cache_path(dir: &Path, rel: &str, size: u64, mtime: i64, bucket: u32) -> PathBuf {
    let key = blake3::hash(format!("{rel}\n{size}\n{mtime}\n{bucket}").as_bytes()).to_hex();
    dir.join(&key[..2]).join(format!("{}.jpg", &key[..32]))
}

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("{0} is not a video or a photo")]
    NotMedia(String),
    #[error("could not make a picture of {0}")]
    Failed(String),
    #[error("video pictures need mpv, which is not installed beside the host")]
    NoMpv,
}

/// Makes a picture of `path` at `bucket` pixels on its long side, as JPEG.
///
/// Blocking: call from a blocking thread.
pub fn make(path: &Path, bucket: u32) -> Result<Vec<u8>, ThumbError> {
    let name = path.to_string_lossy().to_string();
    match kind_of(&name) {
        Some(MediaKind::Image) => match decode_photo(path) {
            Some(photo) => encode(&shrink(photo, bucket)),
            // HEIC, AVIF and anything else the decoders here do not read.
            None => frame(path, bucket, &["0"]),
        },
        Some(MediaKind::Video) => frame(path, bucket, &POSITIONS),
        _ => Err(ThumbError::NotMedia(name)),
    }
}

/// A photo, turned the way its camera said it should be.
fn decode_photo(path: &Path) -> Option<image::DynamicImage> {
    use image::ImageDecoder;

    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut photo = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        photo.apply_orientation(orientation);
    }
    Some(photo)
}

fn shrink(photo: image::DynamicImage, bucket: u32) -> image::DynamicImage {
    if photo.width().max(photo.height()) <= bucket {
        return photo;
    }
    // Triangle rather than Lanczos: at thumbnail sizes the difference is
    // invisible and it is several times quicker on a 48-megapixel photo.
    photo.resize(bucket, bucket, image::imageops::FilterType::Triangle)
}

fn encode(photo: &image::DynamicImage) -> Result<Vec<u8>, ThumbError> {
    let mut out = Vec::new();
    let rgb = photo.to_rgb8();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
        .encode_image(&rgb)
        .map_err(|e| ThumbError::Failed(e.to_string()))?;
    Ok(out)
}

/// Mean brightness of a picture, 0 to 255.
fn brightness(photo: &image::DynamicImage) -> f64 {
    let small = photo.thumbnail(32, 32).to_luma8();
    let pixels = small.as_raw();
    if pixels.is_empty() {
        return 0.0;
    }
    pixels.iter().map(|&p| f64::from(p)).sum::<f64>() / pixels.len() as f64
}

/// One frame, through mpv, trying each position until one is not black.
fn frame(path: &Path, bucket: u32, positions: &[&str]) -> Result<Vec<u8>, ThumbError> {
    let mpv = Mpv::get().ok_or(ThumbError::NoMpv)?;
    let mut darkest = None;
    for at in positions {
        let Some(picture) = mpv.grab(path, at) else {
            continue;
        };
        let picture = shrink(picture, bucket);
        if brightness(&picture) >= DARK {
            return encode(&picture);
        }
        darkest.get_or_insert(picture);
    }
    // All black — a film that really does open on black for half its length,
    // or a recording of a dark room. Shown as it is rather than not at all.
    match darkest {
        Some(picture) => encode(&picture),
        None => Err(ThumbError::Failed(path.display().to_string())),
    }
}

/// Keeps the cache from growing without end: past its limit, the pictures
/// made longest ago go first.
pub fn trim(dir: &Path) {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|shard| {
            std::fs::read_dir(shard.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= MAX_CACHED {
        return;
    }
    files.sort();
    let excess = files.len() - MAX_CACHED;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// mpv, loaded from its DLL
// ---------------------------------------------------------------------------

type Handle = *mut std::ffi::c_void;

/// The handful of libmpv's functions this needs.
struct Mpv {
    create: unsafe extern "C" fn() -> Handle,
    set_option_string:
        unsafe extern "C" fn(Handle, *const std::ffi::c_char, *const std::ffi::c_char) -> i32,
    initialize: unsafe extern "C" fn(Handle) -> i32,
    command: unsafe extern "C" fn(Handle, *const *const std::ffi::c_char) -> i32,
    wait_event: unsafe extern "C" fn(Handle, f64) -> *const Event,
    terminate_destroy: unsafe extern "C" fn(Handle),
    /// Kept loaded for as long as the functions above are used.
    _library: libloading::Library,
}

/// The start of `mpv_event`: only its kind is read.
#[repr(C)]
struct Event {
    id: i32,
}

const EVENT_SHUTDOWN: i32 = 1;
const EVENT_END_FILE: i32 = 7;

static MPV: OnceLock<Option<Mpv>> = OnceLock::new();
static MPV_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Where the DLL is, when the default — beside the executable, in `lib` —
/// is not right. Set once, before the first picture.
pub fn set_mpv_path(path: PathBuf) {
    let _ = MPV_PATH.set(path);
}

/// Whether video pictures can be made at all.
pub fn mpv_available() -> bool {
    Mpv::get().is_some()
}

impl Mpv {
    fn get() -> Option<&'static Mpv> {
        MPV.get_or_init(|| {
            let path = MPV_PATH.get().cloned().or_else(default_mpv_path)?;
            match unsafe { Mpv::load(&path) } {
                Ok(mpv) => {
                    tracing::info!("video thumbnails through {}", path.display());
                    Some(mpv)
                }
                Err(e) => {
                    tracing::warn!(
                        "no video thumbnails: {} would not load ({e})",
                        path.display()
                    );
                    None
                }
            }
        })
        .as_ref()
    }

    /// # Safety
    /// The DLL has to be libmpv, with the C API the declarations above name.
    unsafe fn load(path: &Path) -> Result<Mpv, libloading::Error> {
        let library = unsafe { libloading::Library::new(path)? };
        unsafe {
            Ok(Mpv {
                create: *library.get(b"mpv_create\0")?,
                set_option_string: *library.get(b"mpv_set_option_string\0")?,
                initialize: *library.get(b"mpv_initialize\0")?,
                command: *library.get(b"mpv_command\0")?,
                wait_event: *library.get(b"mpv_wait_event\0")?,
                terminate_destroy: *library.get(b"mpv_terminate_destroy\0")?,
                _library: library,
            })
        }
    }

    /// Writes one frame of `path`, from `at` in, and reads it back.
    fn grab(&self, path: &Path, at: &str) -> Option<image::DynamicImage> {
        let out = std::env::temp_dir().join(format!(
            "basalt-frame-{}-{}",
            std::process::id(),
            FRAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&out).ok()?;
        let result = self.run(path, at, &out);
        let picture = result
            .then(|| first_file(&out))
            .flatten()
            .and_then(|file| image::open(file).ok());
        let _ = std::fs::remove_dir_all(&out);
        picture
    }

    /// One mpv, one file, one frame. Returns whether it finished in time.
    fn run(&self, path: &Path, at: &str, out: &Path) -> bool {
        let handle = unsafe { (self.create)() };
        if handle.is_null() {
            return false;
        }
        let options: &[(&str, String)] = &[
            // Nothing on screen, no sound, nothing read from the user's own
            // mpv set-up, which might do anything.
            ("config", "no".into()),
            ("terminal", "no".into()),
            ("msg-level", "all=no".into()),
            ("load-scripts", "no".into()),
            ("ytdl", "no".into()),
            ("audio", "no".into()),
            ("sub", "no".into()),
            ("osd-level", "0".into()),
            ("hwdec", "no".into()),
            // The frame, as a file.
            ("vo", "image".into()),
            ("vo-image-format", "jpg".into()),
            ("vo-image-jpeg-quality", "90".into()),
            ("vo-image-outdir", out.to_string_lossy().into_owned()),
            ("frames", "1".into()),
            ("start", at.to_string()),
            // To the nearest keyframe: exact would decode everything from
            // there to the point, which is seconds on a 4K film for no
            // difference anyone could see in a thumbnail.
            ("hr-seek", "no".into()),
            // Enough to show a still, not to play it: an image is shown for
            // a moment and then counts as finished.
            ("image-display-duration", "0".into()),
            ("keep-open", "no".into()),
            ("idle", "no".into()),
        ];
        let ok = options.iter().all(|(name, value)| {
            let (Ok(name), Ok(value)) = (
                std::ffi::CString::new(*name),
                std::ffi::CString::new(value.as_str()),
            ) else {
                return false;
            };
            unsafe { (self.set_option_string)(handle, name.as_ptr(), value.as_ptr()) >= 0 }
        });

        let finished = ok && unsafe { (self.initialize)(handle) } >= 0 && {
            let file = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok();
            let loadfile = std::ffi::CString::new("loadfile").expect("no nul");
            match file {
                Some(file) => {
                    let args = [loadfile.as_ptr(), file.as_ptr(), std::ptr::null()];
                    let started = unsafe { (self.command)(handle, args.as_ptr()) } >= 0;
                    started && self.until_finished(handle)
                }
                None => false,
            }
        };
        unsafe { (self.terminate_destroy)(handle) };
        finished
    }

    fn until_finished(&self, handle: Handle) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < MPV_TIMEOUT {
            let event = unsafe { (self.wait_event)(handle, 0.5) };
            if event.is_null() {
                continue;
            }
            let id = unsafe { (*event).id };
            if id == EVENT_END_FILE || id == EVENT_SHUTDOWN {
                return true;
            }
        }
        false
    }
}

// libmpv handles are thread-safe, and the function pointers are plain data.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

static FRAME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn first_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
}

/// `lib/libmpv-2.dll` beside the executable, which is where the installer
/// puts it.
fn default_mpv_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join("lib").join("libmpv-2.dll");
    candidate.exists().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("basalt-thumbs-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn two_sizes_are_made() {
        assert_eq!(bucket(0), GRID);
        assert_eq!(bucket(320), GRID);
        assert_eq!(bucket(480), GRID);
        assert_eq!(bucket(1200), VIEW);
        assert_eq!(bucket(4000), VIEW);
    }

    #[test]
    fn a_changed_file_is_a_different_picture() {
        let dir = Path::new("cache");
        let a = cache_path(dir, "Photos/a.jpg", 100, 1, GRID);
        assert_eq!(a, cache_path(dir, "Photos/a.jpg", 100, 1, GRID));
        assert_ne!(a, cache_path(dir, "Photos/a.jpg", 101, 1, GRID));
        assert_ne!(a, cache_path(dir, "Photos/a.jpg", 100, 2, GRID));
        assert_ne!(a, cache_path(dir, "Photos/a.jpg", 100, 1, VIEW));
        assert!(a.starts_with(dir), "never outside the cache folder");
    }

    #[test]
    fn a_photo_is_shrunk_to_its_long_side_and_keeps_its_shape() {
        let dir = temp("shrink");
        let path = dir.join("wide.png");
        image::RgbImage::from_pixel(1200, 800, image::Rgb([200, 120, 40]))
            .save(&path)
            .unwrap();
        let bytes = make(&path, GRID).unwrap();
        let picture = image::load_from_memory(&bytes).unwrap();
        assert_eq!((picture.width(), picture.height()), (320, 213));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_small_photo_is_not_blown_up() {
        let dir = temp("small");
        let path = dir.join("icon.png");
        image::RgbImage::from_pixel(64, 48, image::Rgb([10, 200, 90]))
            .save(&path)
            .unwrap();
        let picture = image::load_from_memory(&make(&path, GRID).unwrap()).unwrap();
        assert_eq!((picture.width(), picture.height()), (64, 48));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(matches!(
            make(Path::new("notes.docx"), GRID),
            Err(ThumbError::NotMedia(_))
        ));
    }

    #[test]
    fn black_is_told_from_a_picture() {
        let black = image::DynamicImage::ImageRgb8(image::RgbImage::new(40, 30));
        let lit = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            40,
            30,
            image::Rgb([180, 160, 140]),
        ));
        assert!(brightness(&black) < DARK);
        assert!(brightness(&lit) > DARK);
    }

    #[test]
    fn the_cache_is_trimmed_to_its_limit_oldest_first() {
        // Exercised at a tiny scale by the same code path: nothing is removed
        // while under the limit.
        let dir = temp("trim");
        let shard = dir.join("ab");
        std::fs::create_dir_all(&shard).unwrap();
        std::fs::write(shard.join("one.jpg"), b"x").unwrap();
        trim(&dir);
        assert!(shard.join("one.jpg").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only where the DLL is to hand: point `BASALT_LIBMPV` at it.
    #[test]
    fn a_video_frame_comes_through_mpv() {
        let (Ok(dll), Ok(video)) = (
            std::env::var("BASALT_LIBMPV"),
            std::env::var("BASALT_TEST_VIDEO"),
        ) else {
            eprintln!("skipped: set BASALT_LIBMPV and BASALT_TEST_VIDEO");
            return;
        };
        set_mpv_path(PathBuf::from(dll));
        let bytes = make(Path::new(&video), GRID).expect("a frame");
        let picture = image::load_from_memory(&bytes).unwrap();
        assert_eq!(picture.width().max(picture.height()), GRID);
        assert!(brightness(&picture) >= DARK);
    }
}
