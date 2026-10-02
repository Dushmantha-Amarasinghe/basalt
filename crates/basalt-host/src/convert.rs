//! Video converted as it is watched, for a device that cannot play the file.
//!
//! A phone whose hardware decoder stops at 1440p cannot play a 4K episode:
//! decoded in software it managed eight frames a second and fell seconds
//! behind its sound. The host converts it instead, as it is watched: the
//! picture becomes 1080p H.264, which every phone decodes in hardware, while
//! the sound and the subtitles are copied across untouched. Nothing is written
//! to disk; ffmpeg's output goes straight down the connection.
//!
//! **The machine does the work it is built for.** A graphics chip has a video
//! engine that decodes and encodes far faster than its processor can, and
//! uses little power doing it. So the fastest route this machine has is
//! tried first, and the next if it fails:
//!
//! 1. NVIDIA, decoding, scaling and encoding on the card.
//! 2. Intel Quick Sync, the same on the processor's graphics.
//! 3. Decoding on the graphics, scaling in software, encoding on whichever
//!    encoder there is (NVIDIA, Intel, AMD).
//! 4. Software throughout, which a modern processor can manage for one stream
//!    and an old one cannot.
//!
//! A route that fails does so in the first moments, before any picture has
//! been sent, so falling through costs a second at most. The one that worked
//! is remembered and tried first next time.
//!
//! **Times stay the film's own.** A conversion starts where it is asked to,
//! and keeps the film's timestamps, so a player showing it says 41:30 at
//! 41:30, and seeking further is a new conversion from there.
//!
//! **ffmpeg is found, not assumed.** Beside the host, or downloaded into the
//! host's own folder, or on the computer's path; without it the host says it
//! cannot convert and the device plays the file as best it can.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdout, Command};

/// Conversions at once. A consumer NVIDIA card allows a handful of encoders
/// at a time, and each stream is a full decode of a 4K film: two is what an
/// ordinary machine can be trusted with while it also does everything else.
pub const MAX_AT_ONCE: usize = 2;

/// The picture's width after conversion. 1080p: sharp on any phone, and
/// within what every phone's decoder takes.
pub const WIDTH: u32 = 1920;

/// Bytes read from ffmpeg per chunk sent.
pub const CHUNK: usize = 256 * 1024;

/// How long a route has to produce its first bytes before it counts as having
/// failed. Generous: a 4K film's first frame through software takes a moment.
const FIRST_BYTES: std::time::Duration = std::time::Duration::from_secs(12);

/// One way of converting, from decoding to encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Nvidia,
    Intel,
    /// Decoded on the graphics, scaled in software, encoded by `Encoder`.
    Hybrid(Encoder),
    Software,
}

/// A hardware encoder this machine has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Nvidia,
    Intel,
    Amd,
}

impl Encoder {
    fn name(self) -> &'static str {
        match self {
            Encoder::Nvidia => "h264_nvenc",
            Encoder::Intel => "h264_qsv",
            Encoder::Amd => "h264_amf",
        }
    }
}

impl Route {
    /// What the route is called where people read it.
    pub fn describe(self) -> &'static str {
        match self {
            Route::Nvidia => "NVIDIA graphics",
            Route::Intel => "Intel graphics",
            Route::Hybrid(Encoder::Nvidia) => "graphics, with NVIDIA encoding",
            Route::Hybrid(Encoder::Intel) => "graphics, with Intel encoding",
            Route::Hybrid(Encoder::Amd) => "graphics, with AMD encoding",
            Route::Software => "the processor",
        }
    }
}

/// The arguments for one conversion: everything after `ffmpeg`.
///
/// `start` is seconds into the film; `subtitles` says whether its subtitle
/// tracks come along (they are left behind only when a first try with them
/// failed).
pub fn arguments(route: Route, input: &Path, start: f64, subtitles: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"]
        .into_iter()
        .map(String::from)
        .collect();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    // Decoding.
    match route {
        Route::Nvidia => push(&["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]),
        Route::Intel => push(&["-hwaccel", "qsv", "-hwaccel_output_format", "qsv"]),
        // Decoded on the NVIDIA card and brought back, for software scaling:
        // asked for as CUDA, because a D3D11 device ties the NVIDIA encoder
        // to whichever adapter decoded, which on a laptop is the Intel one.
        Route::Hybrid(Encoder::Nvidia) => push(&["-hwaccel", "cuda"]),
        Route::Hybrid(_) => push(&["-hwaccel", "d3d11va"]),
        Route::Software => {}
    }
    // Before the input: a jump straight to that point, not decoding up to it.
    if start > 0.0 {
        push(&["-ss", &format!("{start:.3}")]);
    }
    push(&["-i", &input.to_string_lossy()]);

    // The picture, and the sound and subtitles as they are.
    push(&["-map", "0:v:0", "-map", "0:a?"]);
    if subtitles {
        push(&["-map", "0:s?"]);
    }
    push(&["-c:a", "copy"]);
    if subtitles {
        // MP4's own subtitle format has no place in Matroska; its text does.
        let mp4 = matches!(
            input
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("mp4" | "m4v" | "mov")
        );
        push(&["-c:s", if mp4 { "srt" } else { "copy" }]);
    }

    let scale = format!("{WIDTH}:-2");
    match route {
        Route::Nvidia => push(&[
            "-vf",
            &format!("scale_cuda={scale}:format=nv12"),
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p4",
        ]),
        Route::Intel => push(&[
            "-vf",
            &format!("vpp_qsv=w={WIDTH}:h=-1:format=nv12"),
            "-c:v",
            "h264_qsv",
            "-preset",
            "veryfast",
        ]),
        Route::Hybrid(encoder) => push(&[
            "-vf",
            &format!("scale={scale},format=nv12"),
            "-c:v",
            encoder.name(),
        ]),
        Route::Software => push(&[
            "-vf",
            &format!("scale={scale},format=yuv420p"),
            "-c:v",
            "libx264",
            "-preset",
            "superfast",
        ]),
    }
    push(&[
        // Plenty for 1080p on a phone, and a ceiling so a busy scene does not
        // outrun the Wi-Fi.
        "-b:v",
        "8M",
        "-maxrate",
        "10M",
        "-bufsize",
        "16M",
        // A keyframe every two seconds: a player joining or seeking within
        // what it has does not wait long for a picture.
        "-g",
        "48",
        // The film's own timestamps, from wherever it starts.
        "-copyts",
        "-avoid_negative_ts",
        "disabled",
        "-f",
        "matroska",
        "-live",
        "1",
        "pipe:1",
    ]);
    args
}

/// Where ffmpeg is, if anywhere.
///
/// In order: a path given in `BASALT_FFMPEG`; beside the host, or in its
/// `lib` folder; in the host's own folder, where a download would put it;
/// and finally the computer's path.
pub fn find_ffmpeg(config_dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(given) = std::env::var_os("BASALT_FFMPEG") {
        places.push(PathBuf::from(given));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        places.push(dir.join(name));
        places.push(dir.join("lib").join(name));
    }
    places.push(config_dir.join("converter").join(name));
    if let Some(found) = places.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// A command that shows no console window on Windows.
fn quiet(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Whether ffmpeg can use an encoder on this machine: a second of a test
/// picture, encoded and thrown away. An encoder can be built in and still
/// have no hardware to run on.
async fn encoder_works(ffmpeg: &Path, encoder: &str) -> bool {
    let run = quiet(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=24",
            "-t",
            "1",
            "-c:v",
            encoder,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(20), run).await,
        Ok(Ok(status)) if status.success()
    )
}

/// What this machine can convert with, found once.
#[derive(Debug, Clone, Default)]
pub struct Capability {
    pub ffmpeg: Option<PathBuf>,
    /// The routes to try, fastest first. Empty without ffmpeg.
    pub routes: Vec<Route>,
}

impl Capability {
    /// Finds ffmpeg and tries each hardware encoder. A few seconds, once.
    pub async fn detect(config_dir: &Path) -> Self {
        let Some(ffmpeg) = find_ffmpeg(config_dir) else {
            return Capability::default();
        };
        let mut encoders = Vec::new();
        for encoder in [Encoder::Nvidia, Encoder::Intel, Encoder::Amd] {
            if encoder_works(&ffmpeg, encoder.name()).await {
                encoders.push(encoder);
            }
        }
        let software = encoder_works(&ffmpeg, "libx264").await;
        Capability {
            routes: routes_for(&encoders, software),
            ffmpeg: Some(ffmpeg),
        }
    }

    /// Whether anything can convert here.
    pub fn can_convert(&self) -> bool {
        self.ffmpeg.is_some() && !self.routes.is_empty()
    }
}

/// The routes worth trying with these encoders, fastest first.
pub fn routes_for(encoders: &[Encoder], software: bool) -> Vec<Route> {
    let mut routes = Vec::new();
    if encoders.contains(&Encoder::Nvidia) {
        routes.push(Route::Nvidia);
    }
    if encoders.contains(&Encoder::Intel) {
        routes.push(Route::Intel);
    }
    for &encoder in encoders {
        routes.push(Route::Hybrid(encoder));
    }
    if software {
        routes.push(Route::Software);
    }
    routes
}

/// The conversions running, and the route that last worked.
pub struct Converter {
    capability: tokio::sync::OnceCell<Capability>,
    config_dir: PathBuf,
    running: std::sync::Arc<AtomicUsize>,
    /// Tried first: what worked last time.
    preferred: Mutex<Option<Route>>,
}

/// Why a conversion could not start.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("this host has no way to convert video")]
    Unable,
    #[error("this host is already converting as much as it can")]
    Busy,
    #[error("the video could not be converted: {0}")]
    Failed(String),
}

/// A conversion under way: ffmpeg's output, and ffmpeg itself, which is
/// stopped when this is dropped.
pub struct Conversion {
    pub route: Route,
    child: Child,
    stdout: ChildStdout,
    /// Bytes already read while checking the route worked.
    first: Vec<u8>,
    _slot: Slot,
}

/// A place among the conversions running, given back when dropped.
struct Slot(std::sync::Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Conversion {
    /// The next part of the converted film, or `None` when it has all been
    /// sent.
    pub async fn next(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        if !self.first.is_empty() {
            return Ok(Some(std::mem::take(&mut self.first)));
        }
        let mut buffer = vec![0u8; CHUNK];
        let read = self.stdout.read(&mut buffer).await?;
        if read == 0 {
            let _ = self.child.wait().await;
            return Ok(None);
        }
        buffer.truncate(read);
        Ok(Some(buffer))
    }
}

impl Converter {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            capability: tokio::sync::OnceCell::new(),
            config_dir,
            running: std::sync::Arc::new(AtomicUsize::new(0)),
            preferred: Mutex::new(None),
        }
    }

    /// What this machine can do, found the first time anyone asks.
    pub async fn capability(&self) -> &Capability {
        self.capability
            .get_or_init(|| Capability::detect(&self.config_dir))
            .await
    }

    /// What would convert a file now, without converting it: the fastest
    /// route there is, or why there is none or no room for another.
    pub async fn check(&self) -> Result<Route, ConvertError> {
        let capability = self.capability().await;
        let first = capability
            .routes
            .first()
            .copied()
            .filter(|_| capability.ffmpeg.is_some())
            .ok_or(ConvertError::Unable)?;
        if self.running.load(Ordering::SeqCst) >= MAX_AT_ONCE {
            return Err(ConvertError::Busy);
        }
        let preferred = *self.preferred.lock().expect("route lock");
        Ok(preferred.unwrap_or(first))
    }

    /// Starts converting `input` from `start` seconds in.
    pub async fn start(&self, input: &Path, start: f64) -> Result<Conversion, ConvertError> {
        let capability = self.capability().await;
        let Some(ffmpeg) = capability.ffmpeg.clone() else {
            return Err(ConvertError::Unable);
        };
        if capability.routes.is_empty() {
            return Err(ConvertError::Unable);
        }
        // A place first, given back however this ends.
        if self.running.fetch_add(1, Ordering::SeqCst) >= MAX_AT_ONCE {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return Err(ConvertError::Busy);
        }
        let slot = Slot(std::sync::Arc::clone(&self.running));

        let mut routes = capability.routes.clone();
        if let Some(preferred) = *self.preferred.lock().expect("route lock")
            && let Some(at) = routes.iter().position(|r| *r == preferred)
        {
            routes.remove(at);
            routes.insert(0, preferred);
        }

        let mut last_error = String::from("no route worked");
        let mut slot = Some(slot);
        for route in routes {
            // With subtitles first, and without if they were what failed.
            for subtitles in [true, false] {
                match try_route(&ffmpeg, route, input, start, subtitles).await {
                    Ok((child, stdout, first)) => {
                        *self.preferred.lock().expect("route lock") = Some(route);
                        tracing::info!("converting {} on {}", input.display(), route.describe());
                        return Ok(Conversion {
                            route,
                            child,
                            stdout,
                            first,
                            _slot: slot.take().expect("one slot"),
                        });
                    }
                    Err(e) => {
                        tracing::debug!("{route:?} (subtitles {subtitles}) did not convert: {e}");
                        last_error = e;
                    }
                }
            }
        }
        Err(ConvertError::Failed(last_error))
    }
}

/// Runs one route until it produces its first bytes, or fails.
async fn try_route(
    ffmpeg: &Path,
    route: Route,
    input: &Path,
    start: f64,
    subtitles: bool,
) -> Result<(Child, ChildStdout, Vec<u8>), String> {
    let mut child = quiet(ffmpeg)
        .args(arguments(route, input, start, subtitles))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("ffmpeg would not start: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no output from ffmpeg")?;
    let mut stderr = child.stderr.take().ok_or("no errors from ffmpeg")?;

    let mut first = vec![0u8; CHUNK];
    let read = tokio::time::timeout(FIRST_BYTES, stdout.read(&mut first)).await;
    match read {
        Ok(Ok(n)) if n > 0 => {
            first.truncate(n);
            // What it says from here on is not read; drained so a full pipe
            // never stalls it.
            tokio::spawn(async move {
                let mut sink = Vec::new();
                let _ = stderr.read_to_end(&mut sink).await;
            });
            Ok((child, stdout, first))
        }
        Ok(_) => {
            let mut said = String::new();
            let _ = stderr.read_to_string(&mut said).await;
            let _ = child.kill().await;
            Err(said.lines().last().unwrap_or("ffmpeg stopped").to_string())
        }
        Err(_) => {
            let _ = child.kill().await;
            Err("nothing came out in time".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fastest_routes_come_first() {
        let routes = routes_for(&[Encoder::Nvidia, Encoder::Intel], true);
        assert_eq!(
            routes,
            [
                Route::Nvidia,
                Route::Intel,
                Route::Hybrid(Encoder::Nvidia),
                Route::Hybrid(Encoder::Intel),
                Route::Software,
            ]
        );
        assert_eq!(routes_for(&[], false), []);
        assert_eq!(
            routes_for(&[Encoder::Amd], false),
            [Route::Hybrid(Encoder::Amd)]
        );
    }

    #[test]
    fn a_conversion_keeps_sound_and_subtitles_and_the_films_own_times() {
        let args = arguments(
            Route::Nvidia,
            Path::new("D:/Films/Arrival.mkv"),
            2490.5,
            true,
        );
        let line = args.join(" ");
        assert!(line.contains("-hwaccel cuda -hwaccel_output_format cuda"));
        // Jumped to before the input is opened, not decoded up to.
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let input = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss < input);
        assert_eq!(args[ss + 1], "2490.500");
        assert!(line.contains("-map 0:v:0 -map 0:a? -map 0:s?"));
        assert!(line.contains("-c:a copy"));
        assert!(line.contains("-c:s copy"));
        assert!(line.contains("scale_cuda=1920:-2:format=nv12"));
        assert!(line.contains("-copyts"));
        assert!(line.ends_with("-f matroska -live 1 pipe:1"));
    }

    #[test]
    fn mp4_subtitles_become_text_and_can_be_left_behind() {
        let mp4 = arguments(Route::Software, Path::new("a.mp4"), 0.0, true).join(" ");
        assert!(mp4.contains("-c:s srt"));
        assert!(
            !mp4.contains("-ss"),
            "the start is not given when it is the start"
        );
        let none = arguments(Route::Software, Path::new("a.mkv"), 0.0, false).join(" ");
        assert!(!none.contains("0:s"));
        assert!(!none.contains("-c:s"));
    }

    #[test]
    fn each_route_decodes_and_encodes_where_it_says() {
        let line = |route| arguments(route, Path::new("a.mkv"), 0.0, true).join(" ");
        assert!(line(Route::Intel).contains("vpp_qsv=w=1920:h=-1:format=nv12 -c:v h264_qsv"));
        assert!(line(Route::Hybrid(Encoder::Amd)).contains("-hwaccel d3d11va"));
        assert!(line(Route::Hybrid(Encoder::Amd)).contains("-c:v h264_amf"));
        // Not D3D11 for the NVIDIA encoder: it would be tied to the wrong card.
        let nvidia = line(Route::Hybrid(Encoder::Nvidia));
        assert!(nvidia.contains("-hwaccel cuda -i"));
        assert!(!nvidia.contains("d3d11va"));
        assert!(line(Route::Software).contains("-c:v libx264 -preset superfast"));
        assert!(!line(Route::Software).contains("-hwaccel"));
    }

    /// The real thing, when this machine has ffmpeg: a few seconds of a test
    /// picture converted through whatever route works here, and read back.
    #[tokio::test]
    async fn a_test_picture_converts_on_this_machine() {
        let dir = std::env::temp_dir().join(format!("basalt-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let converter = Converter::new(dir.clone());
        let Some(ffmpeg) = converter.capability().await.ffmpeg.clone() else {
            eprintln!("no ffmpeg here; skipped");
            return;
        };
        let input = dir.join("in.mkv");
        let made = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("testsrc2=size=2560x1440:rate=24")
            .args(["-f", "lavfi", "-i", "sine=frequency=440", "-t", "6"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(made.success());

        let mut conversion = converter.start(&input, 2.0).await.expect("converts");
        let mut bytes = 0usize;
        while let Some(chunk) = conversion.next().await.unwrap() {
            bytes += chunk.len();
        }
        assert!(bytes > 10_000, "something came out: {bytes} bytes");
        drop(conversion);
        assert_eq!(
            converter.running.load(Ordering::SeqCst),
            0,
            "its place is given back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
