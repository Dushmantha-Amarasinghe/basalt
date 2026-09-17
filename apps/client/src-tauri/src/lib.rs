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

/// Bytes that crossed the link, and how long that took.
///
/// A rate cannot be reconstructed from the bytes alone: whoever receives this
/// has no reliable way to know how long the window was, and guessing from
/// arrival times is what produced a display reading 35 MB/s over a 22 MB/s
/// link.
#[derive(Debug, Clone, Copy, serde::Serialize)]
struct ByteWindow {
    bytes: u64,
    millis: f64,
}

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
async fn stat_entry(
    state: State<'_, AppState>,
    path: String,
) -> Answer<basalt_proto::msg::DirEntry> {
    Ok(state.client.stat(&path).await?)
}

#[tauri::command]
async fn copy_entry(state: State<'_, AppState>, from: String, to: String) -> Answer<()> {
    Ok(state.client.copy(&from, &to).await?)
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

/// Hands a file to a player that can actually decode it.
///
/// **Streamed, not downloaded.** The player is given a URL from the media
/// proxy and seeks through it with range requests, so a 3 GB episode starts
/// playing at once instead of after two minutes of copying, and nothing is
/// written to this machine's disk.
///
/// Falls back to downloading only when no streaming-capable player is
/// installed, and says which of the two happened so the interface can explain
/// the wait.
#[tauri::command]
async fn open_externally(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    remote: String,
    id: String,
) -> Answer<OpenResult> {
    if let Some(player) = basalt_client::players::find() {
        let url = state.proxy().await?.url_for(&remote);
        basalt_client::players::launch(&player, &url).map_err(|e| UiError {
            kind: "error".into(),
            message: format!("could not start {}: {e}", player.name),
        })?;
        return Ok(OpenResult {
            player: player.name.to_string(),
            streamed: true,
        });
    }

    // Nothing installed that takes a URL. Copy it out and let Windows decide
    // what opens it — slower, and honest about being slower.
    use tauri_plugin_opener::OpenerExt;

    let name = remote.rsplit('/').next().unwrap_or(&remote).to_string();
    let dir = std::env::temp_dir().join("Basalt");
    std::fs::create_dir_all(&dir).map_err(|e| UiError::from(basalt_client::ClientError::Io(e)))?;
    let local = dir.join(&name);

    let cancel = Cancel::new();
    state
        .transfers
        .lock()
        .expect("transfers lock")
        .insert(id.clone(), cancel.clone());

    let report = progress_reporter(app.clone(), id.clone(), name, "download");
    let result = state
        .client
        .download(&remote, &local, Some(report), Some(cancel))
        .await;
    state.transfers.lock().expect("transfers lock").remove(&id);
    result?;

    let shown = local.display().to_string();
    app.opener()
        .open_path(shown.clone(), None::<&str>)
        .map_err(|e| UiError {
            kind: "error".into(),
            message: format!("could not open {shown}: {e}"),
        })?;
    Ok(OpenResult {
        player: "the default app".into(),
        streamed: false,
    })
}

/// Which player took the file, and whether it was streamed or copied first.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenResult {
    player: String,
    streamed: bool,
}

/// Whether a player that can stream a URL is installed.
#[tauri::command]
fn external_player() -> Option<String> {
    basalt_client::players::find().map(|p| p.name.to_string())
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
    // WebView2 refuses to start playback with sound unless the page has a
    // recent user gesture. Clicking a file in the list *is* one, but the
    // `<video>` element is created afterwards, during a React render, and by
    // then the activation has often lapsed — so a film would open showing a
    // still frame, or play with no audio, for no reason the user could see.
    //
    // Set before the webview exists, which is why it is the first thing here.
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--autoplay-policy=no-user-gesture-required",
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store_path = basalt_client::store::default_path();
            let client = Arc::new(Basalt::open(store_path)?);

            // Feed the throughput trace from the one counter that sees every
            // byte — downloads, uploads, listings and, above all, a film being
            // streamed through the media proxy. Emitting only when something
            // moved means an idle app sends nothing at all.
            {
                let client = Arc::clone(&client);
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut last_total = 0u64;
                    let mut last_at = std::time::Instant::now();
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_millis(125));
                    loop {
                        interval.tick().await;
                        let total = client.bytes_moved();
                        let delta = total.saturating_sub(last_total);
                        let elapsed = last_at.elapsed();
                        last_total = total;
                        last_at = std::time::Instant::now();

                        // The measured interval travels with the bytes. The
                        // interface used to divide this by its own tick length
                        // instead, and reported every speed about twice what it
                        // really was.
                        if delta > 0 {
                            let _ = handle.emit(
                                "basalt://bytes",
                                ByteWindow {
                                    bytes: delta,
                                    millis: elapsed.as_secs_f64() * 1000.0,
                                },
                            );
                        }
                    }
                });
            }

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
            stat_entry,
            copy_entry,
            space,
            make_dir,
            rename_entry,
            remove_entry,
            media_url,
            download,
            upload,
            open_externally,
            external_player,
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
