//! Basalt desktop shell.
//!
//! A thin layer of commands over [`basalt_client::Basalt`]. Everything with
//! any judgement in it lives in that crate, where it can be tested against a
//! real host in one process; this file only translates between Tauri's world
//! and that API.
//!
//! The window is frameless because the app draws its own title bar (see
//! `TitleBar.tsx`), and transparency is off: the design is solid graphite, and
//! a transparent window would cost compositing work for an effect the palette
//! never uses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use basalt_client::client::{Cancel, ProgressFn};
use basalt_client::proxy::MediaProxy;
use basalt_client::{Basalt, HostSummary, Status, TransferEvent, UiError};
use tauri::{Emitter, Manager, State};

/// Every command answers with this: the value, or an error the interface can
/// branch on. `UiError` and the response types live in `basalt-client` so their
/// JSON field names are covered by tests — see that crate's `ui` module for why
/// that matters more than it looks.
type Answer<T> = Result<T, UiError>;

struct AppState {
    client: Arc<Basalt>,
    proxy: tokio::sync::Mutex<Option<Arc<MediaProxy>>>,
    transfers: Mutex<HashMap<String, Cancel>>,
}

impl AppState {
    /// Starts the media proxy the first time something needs a URL.
    ///
    /// Lazily, because it is only useful once connected, and starting it at
    /// launch would mean a listening socket for an app that may never play
    /// anything.
    async fn proxy(&self) -> Answer<Arc<MediaProxy>> {
        let mut slot = self.proxy.lock().await;
        if let Some(proxy) = slot.as_ref() {
            return Ok(Arc::clone(proxy));
        }
        let proxy = Arc::new(MediaProxy::start(Arc::clone(&self.client)).await?);
        *slot = Some(Arc::clone(&proxy));
        Ok(proxy)
    }
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

#[tauri::command]
fn status(state: State<'_, AppState>) -> Status {
    status_of(&state.client)
}

#[tauri::command]
async fn probe(state: State<'_, AppState>, address: String) -> Answer<HostSummary> {
    Ok(state.client.probe(&address).await?.into())
}

#[tauri::command]
async fn pair(state: State<'_, AppState>, address: String, pin: String) -> Answer<Status> {
    state.client.pair(&address, &pin).await?;
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn connect_saved(state: State<'_, AppState>) -> Answer<Status> {
    state.client.connect_saved().await?;
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn connect_to(
    state: State<'_, AppState>,
    host_id: String,
    address: Option<String>,
) -> Answer<Status> {
    state.client.connect(&host_id, address.as_deref()).await?;
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>) -> Answer<Status> {
    state.client.disconnect().await;
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn forget_host(state: State<'_, AppState>, host_id: String) -> Answer<Status> {
    state.client.forget(&host_id).await?;
    Ok(status_of(&state.client))
}

fn status_of(client: &Arc<Basalt>) -> Status {
    Status::new(
        client.status(),
        !client.known_hosts().is_empty(),
        client.device_name(),
    )
}

// ---------------------------------------------------------------------------
// Browsing
// ---------------------------------------------------------------------------

#[tauri::command]
async fn list_dir(
    state: State<'_, AppState>,
    path: String,
) -> Answer<Vec<basalt_proto::msg::DirEntry>> {
    Ok(state.client.list(&path).await?)
}

#[tauri::command]
async fn space(state: State<'_, AppState>) -> Answer<(u64, u64)> {
    Ok(state.client.space().await?)
}

#[tauri::command]
async fn make_dir(state: State<'_, AppState>, path: String) -> Answer<()> {
    Ok(state.client.mkdir(&path).await?)
}

#[tauri::command]
async fn rename_entry(state: State<'_, AppState>, from: String, to: String) -> Answer<()> {
    Ok(state.client.rename(&from, &to).await?)
}

#[tauri::command]
async fn remove_entry(
    state: State<'_, AppState>,
    path: String,
    recursive: bool,
) -> Answer<()> {
    Ok(state.client.remove(&path, recursive).await?)
}

/// A URL the player, image viewer or PDF viewer can open directly.
#[tauri::command]
async fn media_url(state: State<'_, AppState>, path: String) -> Answer<String> {
    Ok(state.proxy().await?.url_for(&path))
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

/// Reports progress to the interface, throttled.
///
/// A 4 MiB chunk over a 22.7 MB/s link arrives about six times a second, which
/// is already a sensible rate for a progress bar — but a cached or local-speed
/// transfer would fire far faster and flood the event channel for no visible
/// benefit. The throttle costs nothing and bounds it.
fn progress_reporter(
    app: tauri::AppHandle,
    id: String,
    name: String,
    kind: &'static str,
) -> ProgressFn {
    let started = std::time::Instant::now();
    let last = Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));

    Arc::new(move |p: basalt_client::Progress| {
        let complete = p.transferred >= p.total;
        {
            let mut last = last.lock().expect("throttle lock");
            if !complete && last.elapsed() < std::time::Duration::from_millis(120) {
                return;
            }
            *last = std::time::Instant::now();
        }

        let seconds = started.elapsed().as_secs_f64();
        let _ = app.emit(
            "basalt://transfer",
            TransferEvent {
                id: id.clone(),
                kind,
                name: name.clone(),
                path: p.path.clone(),
                transferred: p.transferred,
                total: p.total,
                status: if complete { "done" } else { "active" },
                rate: if seconds > 0.0 {
                    p.transferred as f64 / seconds
                } else {
                    0.0
                },
            },
        );
    })
}

#[tauri::command]
async fn download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    remote: String,
    local: String,
    id: String,
) -> Answer<u64> {
    let name = remote.rsplit('/').next().unwrap_or(&remote).to_string();
    let cancel = Cancel::new();
    state
        .transfers
        .lock()
        .expect("transfers lock")
        .insert(id.clone(), cancel.clone());

    let report = progress_reporter(app, id.clone(), name, "download");
    let result = state
        .client
        .download(&remote, &PathBuf::from(local), Some(report), Some(cancel))
        .await;

    state.transfers.lock().expect("transfers lock").remove(&id);
    Ok(result?)
}

#[tauri::command]
async fn upload(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    local: String,
    remote: String,
    overwrite: bool,
    id: String,
) -> Answer<u64> {
    let name = remote.rsplit('/').next().unwrap_or(&remote).to_string();
    let cancel = Cancel::new();
    state
        .transfers
        .lock()
        .expect("transfers lock")
        .insert(id.clone(), cancel.clone());

    let report = progress_reporter(app, id.clone(), name, "upload");
    let result = state
        .client
        .upload(
            &PathBuf::from(local),
            &remote,
            overwrite,
            Some(report),
            Some(cancel),
        )
        .await;

    state.transfers.lock().expect("transfers lock").remove(&id);
    Ok(result?)
}

#[tauri::command]
fn cancel_transfer(state: State<'_, AppState>, id: String) -> bool {
    match state.transfers.lock().expect("transfers lock").get(&id) {
        Some(cancel) => {
            cancel.cancel();
            true
        }
        None => false,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store_path = basalt_client::store::default_path();
            let client = Arc::new(Basalt::open(store_path)?);

            // Reconnect in the background rather than blocking the window.
            // A host that is asleep must not mean an app that will not open.
            {
                let client = Arc::clone(&client);
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let connected = client.connect_saved().await.is_ok();
                    let _ = handle.emit("basalt://status", status_of(&client));
                    if !connected {
                        tracing_note("no saved host reachable at startup");
                    }
                });
            }

            app.manage(AppState {
                client,
                proxy: tokio::sync::Mutex::new(None),
                transfers: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            probe,
            pair,
            connect_saved,
            connect_to,
            disconnect,
            forget_host,
            list_dir,
            space,
            make_dir,
            rename_entry,
            remove_entry,
            media_url,
            download,
            upload,
            cancel_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Basalt");
}

/// Startup diagnostics go to stderr, which is visible in a debug build and
/// harmless in a release one.
fn tracing_note(message: &str) {
    eprintln!("basalt: {message}");
}
