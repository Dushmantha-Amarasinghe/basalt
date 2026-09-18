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

#[tauri::command]
async fn status(state: State<'_, AppState>) -> Answer<HostStatus> {
    let mut status = state
        .host
        .status(state.serving.load(Ordering::Relaxed))
        .await;
    status.problem = state.problem.lock().expect("problem lock").clone();
    Ok(status)
}

#[tauri::command]
fn list_drives() -> Vec<DriveView> {
    basalt_host::drives::list()
        .into_iter()
        .map(DriveView::from)
        .collect()
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
fn devices(state: State<'_, AppState>) -> Vec<DeviceView> {
    let traffic = state.host.traffic();
    let mut rates = state.rates.lock().expect("rates lock");
    let sampled = rates.sample(std::time::Instant::now(), traffic.clone());
    basalt_host::ui::devices_view(&state.host.devices(), &traffic, sampled)
}

#[tauri::command]
fn revoke_device(state: State<'_, AppState>, id: String) -> Answer<bool> {
    let removed = state.host.revoke(&id)?;
    if removed {
        state.rates.lock().expect("rates lock").forget(&id);
    }
    Ok(removed)
}

#[tauri::command]
fn rename_device(state: State<'_, AppState>, id: String, name: String) -> Answer<bool> {
    Ok(state.host.rename_device(&id, &name)?)
}

#[tauri::command]
fn set_device_writable(state: State<'_, AppState>, id: String, writable: bool) -> Answer<bool> {
    Ok(state.host.set_writable(&id, writable)?)
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

#[tauri::command]
fn pending_pairings(state: State<'_, AppState>) -> Vec<PairingView> {
    let now = std::time::Instant::now();
    state
        .host
        .pending_pairings()
        .iter()
        .map(|request| PairingView::new(request, now))
        .collect()
}

#[tauri::command]
fn deny_pairing(state: State<'_, AppState>, id: String) -> bool {
    state.host.deny_pairing(&id)
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
async fn set_tmdb_key(state: State<'_, AppState>, key: String) -> Answer<HostStatus> {
    state.host.set_tmdb_key(&key).await?;
    status(state).await
}

#[tauri::command]
fn open_vault_folder(state: State<'_, AppState>, app: tauri::AppHandle) -> Answer<()> {
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

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "basalt_host=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = basalt_host::config::default_path();
            let config =
                HostConfig::load_or_create(&config_path, &basalt_host::config::machine_name())?;
            let port = config.port;
            let host = Host::new(config, config_path)?;

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

            build_tray(app.handle())?;

            // Windows started this, not the user: stay out of the way. The
            // host serves whether or not anyone is looking at its window, and
            // the tray icon is there when they want it.
            if basalt_host::autostart::launched_at_startup() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

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
            set_tmdb_key,
            open_vault_folder,
        ])
        .run(tauri::generate_context!())
        .expect("could not start Basalt Host");
}
