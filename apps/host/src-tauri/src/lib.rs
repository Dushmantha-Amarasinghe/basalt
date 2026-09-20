//! Basalt Host desktop shell.
//!
//! A thin layer of commands over [`basalt_host::Host`]. Everything with any
//! judgement in it — what a drive is, what a rate is, what shape the interface
//! receives — lives in `basalt-host`, where tests reach it. This file only
//! translates between Tauri's world and that API, and owns the one thing it
//! cannot: the serving task's lifetime.
//!
//! The window is frameless because the app draws its own title bar, matching
//! the client.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use basalt_host::rates::Rates;
use basalt_host::ui::{DeviceView, DriveView, HostStatus, PairingView, UiError};
use basalt_host::{Host, HostConfig, HostError};
use tauri::{Manager, State};

/// Every command answers with this: the value, or an error the interface can
/// branch on. Both live in `basalt-host` so their JSON field names are covered
/// by tests — see that crate's `ui` module for why that matters more than it
/// looks.
type Answer<T> = Result<T, UiError>;

struct AppState {
    host: Arc<Host>,
    /// Turns the host's byte counters into speeds over a measured interval.
    ///
    /// Lives here rather than in `Host` because it is a property of *this*
    /// observer: it means "since the window last asked", and a second observer
    /// asking on its own schedule would need its own.
    rates: Mutex<Rates>,
    serving: Arc<AtomicBool>,
    /// Why serving stopped, when it did.
    problem: Arc<Mutex<Option<String>>>,
}

// ---------------------------------------------------------------------------
// Status and setup
// ---------------------------------------------------------------------------

/// How long a command may take before the log mentions it.
///
/// The window asks for the status every two seconds, so anything slower than
/// this is already visible as a stutter. Naming it in the log turns "the app
/// feels stuck" into a line saying which call and for how long.
const SLOW_COMMAND: std::time::Duration = std::time::Duration::from_secs(2);

#[tauri::command]
async fn status(state: State<'_, AppState>) -> Answer<HostStatus> {
    let began = std::time::Instant::now();
    let mut status = state
        .host
        .status(state.serving.load(Ordering::Relaxed))
        .await;
    status.problem = state.problem.lock().expect("problem lock").clone();

    let took = began.elapsed();
    if took > SLOW_COMMAND {
        tracing::warn!("the status took {took:?}");
    } else {
        // Debug rather than info: one line every two seconds forever would
        // bury the things worth reading.
        tracing::debug!("status in {took:?}");
    }
    Ok(status)
}

/// The drives this machine could share.
///
/// On a blocking thread, and `async` so Tauri keeps it off the main one.
/// Enumerating volumes means asking Windows about every drive letter present,
/// and a disconnected network drive or an empty optical bay can leave
/// `GetVolumeInformationW` sitting there for tens of seconds. On the main
/// thread that is a frozen window; here it is a slow list.
#[tauri::command]
async fn list_drives() -> Vec<DriveView> {
    tokio::task::spawn_blocking(|| {
        basalt_host::drives::list()
            .into_iter()
            .map(DriveView::from)
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Locks in a drive or folder.
///
/// Checked here rather than only inside the vault, so choosing a drive that has
/// been unplugged since the list was drawn says so plainly instead of failing
/// later with a path error on every request.
#[tauri::command]
async fn choose_vault(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> Answer<HostStatus> {
    let path = std::path::PathBuf::from(&path);
    if !basalt_host::drives::is_available(&path) {
        return Err(UiError::from(HostError::NotFound(format!(
            "{} is not there any more. Plug it back in, or pick another drive.",
            path.display()
        ))));
    }

    let name = if name.trim().is_empty() {
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_string()
    } else {
        name.trim().to_string()
    };

    state.host.set_vault(&path, &name).await?;
    status(state).await
}

#[tauri::command]
async fn set_host_name(state: State<'_, AppState>, name: String) -> Answer<HostStatus> {
    state.host.set_host_name(&name)?;
    status(state).await
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

/// The device list, with what each has moved and how fast right now.
///
/// Sampling the rates here — on the same call that draws them — is what keeps
/// the speeds honest: the interval measured is exactly the one between two
/// readings, whatever the interface's polling loop actually managed.
#[tauri::command]
async fn devices(state: State<'_, AppState>) -> Answer<Vec<DeviceView>> {
    let traffic = state.host.traffic();
    let devices = state.host.devices();
    // The guard is taken and dropped without an await in between, which is what
    // keeps a plain mutex safe inside an async command.
    let mut rates = state.rates.lock().expect("rates lock");
    let sampled = rates.sample(std::time::Instant::now(), traffic.clone());
    Ok(basalt_host::ui::devices_view(&devices, &traffic, sampled))
}

#[tauri::command]
async fn revoke_device(state: State<'_, AppState>, id: String) -> Answer<bool> {
    let removed = state.host.revoke(&id)?;
    if removed {
        state.rates.lock().expect("rates lock").forget(&id);
    }
    Ok(removed)
}

#[tauri::command]
async fn rename_device(state: State<'_, AppState>, id: String, name: String) -> Answer<bool> {
    Ok(state.host.rename_device(&id, &name)?)
}

#[tauri::command]
async fn set_device_writable(
    state: State<'_, AppState>,
    id: String,
    writable: bool,
) -> Answer<bool> {
    Ok(state.host.set_writable(&id, writable)?)
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

#[tauri::command]
async fn pending_pairings(state: State<'_, AppState>) -> Answer<Vec<PairingView>> {
    let now = std::time::Instant::now();
    Ok(state
        .host
        .pending_pairings()
        .iter()
        .map(|request| PairingView::new(request, now))
        .collect())
}

#[tauri::command]
async fn deny_pairing(state: State<'_, AppState>, id: String) -> Answer<bool> {
    Ok(state.host.deny_pairing(&id))
}

#[tauri::command]
async fn set_require_pin(state: State<'_, AppState>, require: bool) -> Answer<HostStatus> {
    state.host.set_require_pin(require)?;
    status(state).await
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[tauri::command]
async fn set_start_with_windows(state: State<'_, AppState>, enabled: bool) -> Answer<HostStatus> {
    state.host.set_start_with_windows(enabled)?;
    status(state).await
}

/// Turns the media index on or off.
///
/// Switching it on starts a scan; switching it off drops the index rather than
/// hiding it, because an index nobody asked for should not sit on disk.
#[tauri::command]
async fn set_library_enabled(state: State<'_, AppState>, enabled: bool) -> Answer<HostStatus> {
    state.host.set_library_enabled(enabled).await?;
    status(state).await
}

/// Rebuilds the index now.
///
/// A scan is complete every time, so this is also how anything deleted behind
/// the app's back leaves the library.
#[tauri::command]
async fn rescan_library(state: State<'_, AppState>) -> Answer<HostStatus> {
    state.host.start_scan();
    status(state).await
}

/// Stores the TMDb key and fetches whatever artwork it unlocks.
///
/// The key only ever travels inwards. `status` reports whether one is set, not
/// what it is, so it cannot be read back out of the interface.
#[tauri::command]
async fn set_posters(state: State<'_, AppState>, enabled: bool) -> Answer<HostStatus> {
    state.host.set_posters(enabled).await?;
    status(state).await
}

#[tauri::command]
async fn set_tmdb_key(state: State<'_, AppState>, key: String) -> Answer<HostStatus> {
    state.host.set_tmdb_key(&key).await?;
    status(state).await
}

#[tauri::command]
async fn open_vault_folder(state: State<'_, AppState>, app: tauri::AppHandle) -> Answer<()> {
    use tauri_plugin_opener::OpenerExt;

    let path = state
        .host
        .vault_path()
        .ok_or_else(|| HostError::NotFound("no drive has been chosen yet".into()))?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| UiError::from(HostError::BadRequest(format!("could not open it: {e}"))))
}

// ---------------------------------------------------------------------------
// The tray
// ---------------------------------------------------------------------------

/// Brings the window back, wherever it was.
fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open Basalt Host", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit and stop sharing", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    TrayIconBuilder::with_id("basalt-host")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| tauri::Error::AssetNotFound("the bundled window icon".into()))?,
        )
        .tooltip("Basalt Host — sharing a drive")
        .menu(&menu)
        // The menu belongs on right-click only, so a left click can do the
        // obvious thing instead of opening a two-item list.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

/// Sends the log to a file beside the config, and returns its path.
///
/// A release build sets `windows_subsystem = "windows"`, which means there is
/// no console and anything written to stdout goes nowhere at all. That made a
/// host misbehaving on another machine completely undiagnosable — the first
/// report of trouble had nothing to look at but guesswork.
///
/// One file, truncated at each start. A host that has been running for a month
/// should not have a log nobody will ever read; what matters is the session
/// that went wrong, and that is the one still open.
fn start_logging() -> Option<std::path::PathBuf> {
    let path = basalt_host::config::default_path()
        .parent()?
        .join("host.log");
    std::fs::create_dir_all(path.parent()?).ok()?;

    let file = std::fs::File::create(&path).ok()?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "basalt_host=debug,basalt_host_lib=debug".into()),
        )
        .init();

    // A panic on a background thread otherwise vanishes without trace, and a
    // panic is exactly the thing somebody reporting "it stopped responding"
    // needs recorded.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));

    Some(path)
}

/// Notices when the window stops answering, and writes it down.
///
/// "Not responding" is Windows saying a program has not collected its messages
/// for a few seconds, which on this app means the main thread is stuck in
/// something. From the outside that is indistinguishable from a crash, a slow
/// drive, or a command that never returns, and guessing between those cost two
/// rebuilds already.
///
/// So: post a do-nothing closure to the main thread every couple of seconds and
/// see whether it comes back. If it does not, the main thread is the problem and
/// the log says so with a timestamp. If it keeps coming back while the window
/// still shows nothing, the main thread is fine and the fault is in a command —
/// which is the other half of the answer, and just as useful.
fn watch_main_thread(app: tauri::AppHandle) {
    use std::time::{Duration, Instant};

    /// How often to ask.
    const PING: Duration = Duration::from_secs(2);
    /// How long an unanswered ping means trouble. Windows uses five seconds for
    /// the same judgement, so this agrees with what the user is shown.
    const STALL: Duration = Duration::from_secs(5);

    std::thread::spawn(move || {
        let mut stuck_since: Option<Instant> = None;
        loop {
            std::thread::sleep(PING);

            let (tx, rx) = std::sync::mpsc::channel();
            // Only fails once the app is shutting down, which is this thread's
            // cue to stop rather than a fault to report.
            if app
                .run_on_main_thread(move || {
                    let _ = tx.send(());
                })
                .is_err()
            {
                return;
            }

            match (rx.recv_timeout(STALL), stuck_since) {
                (Ok(()), Some(since)) => {
                    tracing::warn!("the window answered again after {:?}", since.elapsed());
                    stuck_since = None;
                }
                (Ok(()), None) => {}
                (Err(_), Some(since)) => {
                    tracing::error!("the window is still stuck, {:?} now", since.elapsed())
                }
                (Err(_), None) => {
                    tracing::error!("the window has stopped answering; the main thread is stuck");
                    stuck_since = Some(Instant::now() - STALL);
                }
            }
        }
    });
}

/// Which build this is: the commit, and when it was made.
///
/// Every build calls itself 0.1.0, so the version alone never answered "am I
/// running the new one?". This does.
fn build_stamp() -> String {
    let seconds: i64 = env!("BASALT_BUILT").parse().unwrap_or(0);
    // Whole days since the epoch, turned into a date by hand. A date crate for
    // one string in one label is not a trade worth making.
    let (year, month, day) = civil_from_days(seconds / 86_400);
    format!(
        "{} · built {year:04}-{month:02}-{day:02}",
        env!("BASALT_COMMIT")
    )
}

/// Days since 1970-01-01 to a calendar date. Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[tauri::command]
async fn build_info() -> String {
    build_stamp()
}

/// Opens the folder holding the config and the log.
///
/// Because "send me the log" should not require anyone to know what %APPDATA%
/// is, nor to be told a path over chat and paste it into a Run box.
#[tauri::command]
async fn open_log_folder(app: tauri::AppHandle) -> Answer<()> {
    use tauri_plugin_opener::OpenerExt;

    let folder = basalt_host::config::default_path()
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| HostError::NotFound("the settings folder".into()))?;
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|e| UiError::from(HostError::BadRequest(format!("could not open it: {e}"))))
}

pub fn run() {
    let log = start_logging();
    tracing::info!(
        "Basalt Host {} ({}) starting; log at {:?}",
        env!("CARGO_PKG_VERSION"),
        build_stamp(),
        log
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Logged step by step because this runs on the main thread, and a
            // main thread that never finishes here is a window that never
            // responds. Silence used to leave no way to tell which step it was.
            watch_main_thread(app.handle().clone());

            let config_path = basalt_host::config::default_path();
            tracing::info!("reading {}", config_path.display());
            let config =
                HostConfig::load_or_create(&config_path, &basalt_host::config::machine_name())?;
            let port = config.port;
            tracing::info!(
                vault = ?config.vault_path,
                library = config.library_enabled,
                devices = config.devices.len(),
                "opening the vault"
            );
            let host = Host::new(config, config_path)?;
            tracing::info!("vault open");

            let serving = Arc::new(AtomicBool::new(false));
            let problem = Arc::new(Mutex::new(None));

            // Registered **before** anything else in this closure.
            //
            // The window starts loading as soon as it exists, and the first
            // thing the interface does is ask for the status. If that call
            // lands before the state is managed, Tauri answers "state not
            // managed" and the app sits on a blank screen looking exactly like
            // a crash. Spawning a task first was once enough to lose that race
            // in the client.
            app.manage(AppState {
                host: Arc::clone(&host),
                rates: Mutex::new(Rates::new()),
                serving: Arc::clone(&serving),
                problem: Arc::clone(&problem),
            });

            // Serve on every interface, for as long as the app is open. The
            // beacon that lets clients find this machine is started by `serve`
            // itself, so there is nothing else to wire up here.
            {
                let host = Arc::clone(&host);
                tauri::async_runtime::spawn(async move {
                    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
                    let outcome = match basalt_host::bind(host, addr).await {
                        Ok(bound) => {
                            serving.store(true, Ordering::Relaxed);
                            tracing::info!("serving on {}", bound.addr());
                            basalt_host::serve(bound).await
                        }
                        Err(e) => Err(e),
                    };

                    serving.store(false, Ordering::Relaxed);
                    let message = match outcome {
                        Err(basalt_host::HostError::Io(e))
                            if e.kind() == std::io::ErrorKind::AddrInUse =>
                        {
                            format!(
                                "Port {port} is already taken. Another copy of Basalt Host is \
                                 probably already running."
                            )
                        }
                        Err(e) => format!("Sharing stopped: {e}"),
                        Ok(()) => "Sharing stopped.".to_string(),
                    };
                    tracing::error!("{message}");
                    *problem.lock().expect("problem lock") = Some(message);
                });
            }

            // Its own step in the log because it talks to the notification
            // area, and the notification area belongs to Explorer — which at
            // login is the busiest process on the machine.
            tracing::info!("building the tray icon");
            build_tray(app.handle())?;
            tracing::info!("tray icon built");

            // Windows started this, not the user: stay out of the way. The
            // host serves whether or not anyone is looking at its window, and
            // the tray icon is there when they want it.
            if basalt_host::autostart::launched_at_startup() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                tracing::info!("started by Windows, so the window stays hidden");
            }

            tracing::info!("ready");
            Ok(())
        })
        // Closing the window keeps the drive shared.
        //
        // This app is a server that happens to have a window. Someone tidying
        // their taskbar should not silently disconnect a laptop mid-transfer,
        // so the close button hides; Quit, in the tray menu, stops sharing.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            status,
            list_drives,
            choose_vault,
            set_host_name,
            devices,
            revoke_device,
            rename_device,
            set_device_writable,
            pending_pairings,
            deny_pairing,
            set_require_pin,
            set_start_with_windows,
            set_library_enabled,
            rescan_library,
            set_posters,
            set_tmdb_key,
            build_info,
            open_log_folder,
            open_vault_folder,
        ])
        .run(tauri::generate_context!())
        .expect("could not start Basalt Host");
}
