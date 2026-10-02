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
        match find_ffmpeg(config_dir) {
            Some(ffmpeg) => Self::detect_with(ffmpeg).await,
            None => Capability::default(),
        }
    }

    /// The same, with ffmpeg already found.
    pub async fn detect_with(ffmpeg: PathBuf) -> Self {
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

/// How many conversions at once a machine is ever tried with when measuring.
/// A consumer NVIDIA card allows about this many encoders at a time, and
/// a household watches a few films at once at most.
pub const MOST_MEASURED: u32 = 6;

/// How much faster than real time each conversion has to run to count as
/// keeping up. Not exactly real time: a busy moment in a film, or the machine
/// doing something else, would then make it stutter.
const KEEPS_UP: f64 = 1.15;

/// What a machine was measured to manage, kept between runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    /// What converted, as people say it: "NVIDIA graphics".
    pub by: String,
    /// 4K films it can convert at once and keep up. Zero: not even one.
    pub at_once: u32,
    /// How much faster than real time one conversion ran.
    pub speed: f64,
    /// When, in Unix seconds.
    pub at: i64,
}

/// A conversion under way, as the host's window lists it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Active {
    /// The device it is for.
    pub device: String,
    /// The film, vault-relative.
    pub file: String,
    /// When it started, in Unix seconds.
    pub since: i64,
}

/// The conversions running, the route that last worked, and the settings.
pub struct Converter {
    capability: tokio::sync::OnceCell<Capability>,
    config_dir: PathBuf,
    /// ffmpeg as the installer placed it, when the host app says where.
    given: Mutex<Option<PathBuf>>,
    running: std::sync::Arc<AtomicUsize>,
    /// Tried first: what worked last time.
    preferred: Mutex<Option<Route>>,
    /// Switched on in the host's settings. On unless turned off.
    enabled: std::sync::atomic::AtomicBool,
    /// Conversions at once, chosen by hand. None: as measured.
    by_hand: Mutex<Option<u32>>,
    /// What this machine was measured to manage.
    measured: Mutex<Option<Measured>>,
    measuring: std::sync::atomic::AtomicBool,
    active: std::sync::Arc<Mutex<Vec<(u64, Active)>>>,
    next_id: std::sync::atomic::AtomicU64,
}

/// Why a conversion could not start.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("this host has no way to convert video")]
    Unable,
    #[error("video conversion is switched off on this host")]
    Off,
    #[error("this host's computer is too slow to convert video as it is watched")]
    TooSlow,
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

/// A place among the conversions running, given back when dropped, and its
/// line in the list the host's window shows.
struct Slot {
    running: std::sync::Arc<AtomicUsize>,
    active: std::sync::Arc<Mutex<Vec<(u64, Active)>>>,
    id: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
        self.active
            .lock()
            .expect("active lock")
            .retain(|(id, _)| *id != self.id);
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

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Converter {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            capability: tokio::sync::OnceCell::new(),
            config_dir,
            given: Mutex::new(None),
            running: std::sync::Arc::new(AtomicUsize::new(0)),
            preferred: Mutex::new(None),
            enabled: std::sync::atomic::AtomicBool::new(true),
            by_hand: Mutex::new(None),
            measured: Mutex::new(None),
            measuring: std::sync::atomic::AtomicBool::new(false),
            active: std::sync::Arc::default(),
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Uses the ffmpeg the installer put in place. Before anything converts.
    pub fn use_ffmpeg(&self, path: PathBuf) {
        *self.given.lock().expect("ffmpeg lock") = Some(path);
    }

    /// Whether this is an installed host with its own ffmpeg, rather than a
    /// copy borrowing whatever the computer has, as tests and development do.
    pub fn installed(&self) -> bool {
        self.given.lock().expect("ffmpeg lock").is_some()
    }

    /// The settings, as saved: on or off, a number chosen by hand, and what
    /// was measured last time.
    pub fn restore(&self, enabled: bool, by_hand: Option<u32>, measured: Option<Measured>) {
        self.enabled.store(enabled, Ordering::SeqCst);
        *self.by_hand.lock().expect("limit lock") = by_hand;
        *self.measured.lock().expect("measured lock") = measured;
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_by_hand(&self, at_once: Option<u32>) {
        *self.by_hand.lock().expect("limit lock") = at_once.map(|n| n.min(MOST_MEASURED * 2));
    }

    pub fn by_hand(&self) -> Option<u32> {
        *self.by_hand.lock().expect("limit lock")
    }

    pub fn measured(&self) -> Option<Measured> {
        self.measured.lock().expect("measured lock").clone()
    }

    pub fn is_measuring(&self) -> bool {
        self.measuring.load(Ordering::SeqCst)
    }

    /// Conversions allowed at once: as chosen by hand, or as measured, or one
    /// until it has been.
    pub fn limit(&self) -> u32 {
        self.by_hand()
            .or_else(|| self.measured().map(|m| m.at_once))
            .unwrap_or(1)
    }

    /// The conversions running now, oldest first.
    pub fn active(&self) -> Vec<Active> {
        self.active
            .lock()
            .expect("active lock")
            .iter()
            .map(|(_, a)| a.clone())
            .collect()
    }

    /// What this machine can do, if it has been looked at yet.
    pub fn detected(&self) -> Option<Capability> {
        self.capability.get().cloned()
    }

    /// What this machine can do, found the first time anyone asks.
    pub async fn capability(&self) -> &Capability {
        self.capability
            .get_or_init(|| async {
                let given = self.given.lock().expect("ffmpeg lock").clone();
                match given.filter(|p| p.is_file()) {
                    Some(ffmpeg) => Capability::detect_with(ffmpeg).await,
                    None => Capability::detect(&self.config_dir).await,
                }
            })
            .await
    }

    /// Whether a conversion could start now, by the settings and the room.
    fn admit(&self) -> Result<(), ConvertError> {
        if !self.enabled() {
            return Err(ConvertError::Off);
        }
        let limit = self.limit() as usize;
        if limit == 0 {
            return Err(ConvertError::TooSlow);
        }
        // Not refused while measuring: somebody watching comes first, and a
        // measurement a little low is better than a film that will not play.
        if self.running.load(Ordering::SeqCst) >= limit {
            return Err(ConvertError::Busy);
        }
        Ok(())
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
        self.admit()?;
        let preferred = *self.preferred.lock().expect("route lock");
        Ok(preferred.unwrap_or(first))
    }

    /// Starts converting `input` from `start` seconds in, for `device`;
    /// `file` is how the host's window names it.
    pub async fn start(
        &self,
        input: &Path,
        start: f64,
        device: &str,
        file: &str,
    ) -> Result<Conversion, ConvertError> {
        let capability = self.capability().await;
        let Some(ffmpeg) = capability.ffmpeg.clone() else {
            return Err(ConvertError::Unable);
        };
        if capability.routes.is_empty() {
            return Err(ConvertError::Unable);
        }
        self.admit()?;
        // A place first, given back however this ends. Taken, then checked,
        // so two asking at once cannot both take the last one.
        if self.running.fetch_add(1, Ordering::SeqCst) >= self.limit() as usize {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return Err(ConvertError::Busy);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.active.lock().expect("active lock").push((
            id,
            Active {
                device: device.to_string(),
                file: file.to_string(),
                since: unix_now(),
            },
        ));
        let slot = Slot {
            running: std::sync::Arc::clone(&self.running),
            active: std::sync::Arc::clone(&self.active),
            id,
        };

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

    /// Measures how many 4K films this machine can convert at once and keep
    /// up: a demanding 4K 10-bit HEVC sample, converted once, then twice at
    /// the same time, and so on, until it no longer keeps up.
    ///
    /// Tens of seconds on a slow machine, once; the result is kept.
    pub async fn measure(&self) -> Result<Measured, String> {
        if self.measuring.swap(true, Ordering::SeqCst) {
            return Err("already measuring".into());
        }
        struct Done<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&self.measuring);

        let capability = self.capability().await;
        let ffmpeg = capability.ffmpeg.clone().ok_or("no ffmpeg here")?;
        let sample = make_sample(&ffmpeg, &self.config_dir.join("converter")).await?;

        // The first route that converts the sample at all. That first run also
        // wakes the graphics driver, which takes seconds the first time and
        // never again, so it is not what is timed: one more run is.
        let mut found = None;
        for &route in &capability.routes {
            if run_at_once(&ffmpeg, route, &sample, 1).await.is_ok() {
                found = Some(route);
                break;
            }
        }
        let route = found.ok_or("nothing here could convert the sample")?;
        let speed = run_at_once(&ffmpeg, route, &sample, 1).await?;
        let mut at_once = u32::from(speed >= KEEPS_UP);
        if at_once == 1 {
            for streams in 2..=MOST_MEASURED {
                match run_at_once(&ffmpeg, route, &sample, streams).await {
                    Ok(each) if each >= KEEPS_UP => at_once = streams,
                    _ => break,
                }
            }
        }
        let measured = Measured {
            by: route.describe().to_string(),
            at_once,
            speed: (speed * 10.0).round() / 10.0,
            at: unix_now(),
        };
        *self.preferred.lock().expect("route lock") = Some(route);
        *self.measured.lock().expect("measured lock") = Some(measured.clone());
        tracing::info!(
            "measured video conversion: {} at once on {}, {:.1}x real time for one",
            measured.at_once,
            measured.by,
            measured.speed
        );
        Ok(measured)
    }
}

/// Seconds of the measuring sample, and how many times it is played through.
const SAMPLE_SECONDS: f64 = 3.0;
const SAMPLE_LOOPS: u32 = 4;

/// A short, demanding 4K 10-bit HEVC film to measure with, made once with
/// whatever HEVC encoder this machine has, and kept.
///
/// A test picture with grain over it: plain colour bars decode far faster
/// than any real film, and would have promised more than the machine can do.
async fn make_sample(ffmpeg: &Path, dir: &Path) -> Result<PathBuf, String> {
    let sample = dir.join("sample-4k-hevc10.mkv");
    if std::fs::metadata(&sample).is_ok_and(|m| m.len() > 0) {
        return Ok(sample);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let partial = dir.join("sample-4k-hevc10.part.mkv");
    let encoders: [&[&str]; 4] = [
        &[
            "-c:v",
            "hevc_nvenc",
            "-profile:v",
            "main10",
            "-pix_fmt",
            "p010le",
        ],
        &[
            "-c:v",
            "hevc_qsv",
            "-profile:v",
            "main10",
            "-pix_fmt",
            "p010le",
        ],
        &[
            "-c:v",
            "hevc_amf",
            "-profile:v",
            "main10",
            "-pix_fmt",
            "p010le",
        ],
        &[
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
        ],
    ];
    for encoder in encoders {
        let made = quiet(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("testsrc2=size=3840x2160:rate=24000/1001,noise=alls=12:allf=t+u")
            .args(["-t", &SAMPLE_SECONDS.to_string()])
            .args(encoder)
            .args(["-b:v", "20M", "-f", "matroska"])
            .arg(&partial)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status();
        let ok = matches!(
            tokio::time::timeout(std::time::Duration::from_secs(300), made).await,
            Ok(Ok(status)) if status.success()
        );
        if ok && std::fs::rename(&partial, &sample).is_ok() {
            return Ok(sample);
        }
    }
    let _ = std::fs::remove_file(&partial);
    Err("no HEVC encoder here could make the sample".into())
}

/// Converts the sample `streams` times at once, and says how much faster than
/// real time each one ran, the slowest of them.
async fn run_at_once(
    ffmpeg: &Path,
    route: Route,
    sample: &Path,
    streams: u32,
) -> Result<f64, String> {
    let mut args = arguments(route, sample, 0.0, false);
    let input = args.iter().position(|a| a == "-i").ok_or("no input")?;
    args.splice(
        input..input,
        ["-stream_loop".to_string(), (SAMPLE_LOOPS - 1).to_string()],
    );

    let started = std::time::Instant::now();
    let mut running = Vec::new();
    for _ in 0..streams {
        let child = quiet(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("ffmpeg would not start: {e}"))?;
        running.push(child);
    }
    for mut child in running {
        let status = tokio::time::timeout(std::time::Duration::from_secs(300), child.wait())
            .await
            .map_err(|_| "took far too long".to_string())?
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("{route:?} failed with {streams} at once"));
        }
    }
    let content = SAMPLE_SECONDS * f64::from(SAMPLE_LOOPS);
    Ok(content / started.elapsed().as_secs_f64())
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

    #[tokio::test]
    async fn the_settings_decide_whether_a_conversion_may_start() {
        let converter = Converter::new(std::env::temp_dir());
        // One at once until measured.
        assert_eq!(converter.limit(), 1);
        assert!(converter.admit().is_ok());

        converter.set_enabled(false);
        assert!(matches!(converter.admit(), Err(ConvertError::Off)));
        converter.set_enabled(true);

        let slow = Measured {
            by: "the processor".into(),
            at_once: 0,
            speed: 0.6,
            at: 0,
        };
        converter.restore(true, None, Some(slow));
        assert!(matches!(converter.admit(), Err(ConvertError::TooSlow)));

        // Chosen by hand, over what was measured.
        converter.set_by_hand(Some(2));
        assert_eq!(converter.limit(), 2);
        converter.running.store(2, Ordering::SeqCst);
        assert!(matches!(converter.admit(), Err(ConvertError::Busy)));
        converter.running.store(0, Ordering::SeqCst);
        converter.set_by_hand(None);
        assert_eq!(converter.limit(), 0);
    }

    /// Measuring, for real: a demanding 4K 10-bit sample made and converted
    /// one, two, three at a time. Run by hand (`--ignored`): on a machine
    /// without a graphics encoder it takes minutes.
    #[tokio::test]
    #[ignore]
    async fn this_machine_is_measured() {
        let dir = std::env::temp_dir().join(format!("basalt-measure-{}", std::process::id()));
        let converter = Converter::new(dir.clone());
        if converter.capability().await.ffmpeg.is_none() {
            eprintln!("no ffmpeg here; skipped");
            return;
        }
        let started = std::time::Instant::now();
        let measured = converter.measure().await.expect("measures");
        eprintln!("measured in {:?}: {measured:?}", started.elapsed());
        assert!(measured.speed > 0.0);
        assert_eq!(converter.limit(), measured.at_once);
        assert!(!converter.is_measuring());
        // Kept, so the next time costs nothing.
        assert!(dir.join("converter").join("sample-4k-hevc10.mkv").is_file());
        let _ = std::fs::remove_dir_all(&dir);
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

        let mut conversion = converter
            .start(&input, 2.0, "Laptop", "in.mkv")
            .await
            .expect("converts");
        assert_eq!(converter.active().len(), 1, "listed while it runs");
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
        assert!(converter.active().is_empty(), "and its line in the list");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
