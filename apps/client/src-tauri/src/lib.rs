//! Basalt desktop shell.
//!
//! A thin layer of commands over [`basalt_client::Basalt`]. Everything with
//! any judgement in it lives in that crate, where it can be tested against a
//! real host in one process; this file only translates between Tauri's world
//! and that API.
//!
//! The window is frameless because the app draws its own title bar (see
//! `TitleBar.tsx`), and **transparent** — not for any visual effect, but
//! because that is how video gets on screen. libmpv renders into the native
//! window behind the webview, so the page has to be able to get out of its
//! way. `body` stays opaque, so nothing about the app looks any different;
//! only the player makes itself see-through, and mpv shows through the hole.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use basalt_client::client::{Cancel, ProgressFn, WatchHandle};
use basalt_client::proxy::MediaProxy;
use basalt_client::{Basalt, DiscoveredHost, Status, TransferEvent, UiError};
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
    /// The live-change subscription. Replaced whenever the app reconnects, and
    /// the old one stops the moment it is dropped.
    watch: Mutex<Option<WatchHandle>>,
    /// Paths handed to a player outside this app.
    ///
    /// The proxy records how far anything has *read*, which is the only
    /// resume signal an external player gives away — but it is a poor one,
    /// because a player reads ahead of what it is showing. The app's own
    /// player reports real positions, so folding its read-ahead in as well
    /// would push Continue watching minutes past where anybody has watched.
    /// Only paths in here contribute that estimate.
    external: Mutex<std::collections::HashSet<String>>,
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

/// Every Basalt host answering on this network.
///
/// This is what replaced typing an address. It takes about a second — the scan
/// window — so the interface shows the previous list while it runs rather than
/// emptying itself on every sweep.
#[tauri::command]
async fn discover(state: State<'_, AppState>) -> Answer<Vec<DiscoveredHost>> {
    Ok(state.client.discover_hosts().await?)
}

/// Asks a host to pair, and reports whether it wants a PIN.
///
/// From this moment the host is displaying the request — this device's name
/// against the number to read across — so the interface can show a PIN field
/// knowing one is on screen at the other end.
#[tauri::command]
async fn begin_pairing(state: State<'_, AppState>, address: String) -> Answer<bool> {
    Ok(state.client.begin_pairing_at(&address).await?)
}

/// Completes the request begun above, on the same session.
///
/// `pin` is empty when the host did not ask for one. Two calls rather than one
/// because they answer different questions, and because a single call would
/// mean typing a PIN at a host that might not be there.
#[tauri::command]
async fn finish_pairing(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    pin: String,
) -> Answer<Status> {
    let pin = pin.trim();
    state
        .client
        .finish_pairing(if pin.is_empty() { None } else { Some(pin) })
        .await?;
    start_watching(&state.client, &app);
    Ok(status_of(&state.client))
}

/// Abandons a request, for when the user backs out of the PIN screen.
///
/// Worth doing rather than letting it lapse: the host is showing a card with
/// this device's name on it, and leaving it there for three minutes after
/// somebody changed their mind is untidy at best and confusing at worst.
#[tauri::command]
async fn cancel_pairing(state: State<'_, AppState>) -> Answer<()> {
    state.client.cancel_pairing().await;
    Ok(())
}

#[tauri::command]
async fn connect_saved(state: State<'_, AppState>, app: tauri::AppHandle) -> Answer<Status> {
    state.client.connect_saved().await?;
    start_watching(&state.client, &app);
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn connect_to(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    host_id: String,
    address: Option<String>,
) -> Answer<Status> {
    state.client.connect(&host_id, address.as_deref()).await?;
    start_watching(&state.client, &app);
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>) -> Answer<Status> {
    // Dropped, not left running: a watch with nothing to watch is a retry loop.
    state.watch.lock().expect("watch lock").take();
    state.client.disconnect().await;
    Ok(status_of(&state.client))
}

#[tauri::command]
async fn forget_host(state: State<'_, AppState>, host_id: String) -> Answer<Status> {
    state.watch.lock().expect("watch lock").take();
    state.client.forget(&host_id).await?;
    Ok(status_of(&state.client))
}

fn status_of(client: &Arc<Basalt>) -> Status {
    // The paired host on disk, which is what the window needs in order to name
    // the vault — or to forget it — while nothing is answering.
    let saved = client.known_hosts();
    let live = client.status();
    let paired = live
        .as_ref()
        .and_then(|info| saved.iter().find(|host| host.host_id == info.host_id))
        .or_else(|| saved.first());

    Status::new(live.clone(), paired, client.device_name())
}

// ---------------------------------------------------------------------------
// Live changes
// ---------------------------------------------------------------------------

/// Subscribes to the host's changes and forwards them to the window.
///
/// Replaces any watch already running, so reconnecting does not leave two
/// subscriptions delivering everything twice.
fn start_watching(client: &Arc<Basalt>, app: &tauri::AppHandle) {
    let emitter = app.clone();
    let handle = client.watch(move |change| {
        let _ = emitter.emit("basalt://change", change);
    });
    if let Some(state) = app.try_state::<AppState>() {
        // Assigning drops the previous handle, which stops the old watch.
        *state.watch.lock().expect("watch lock") = Some(handle);
    }
}

// ---------------------------------------------------------------------------
// The media library
// ---------------------------------------------------------------------------

/// The host's index of films and series.
///
/// `knownRevision` lets the answer be "nothing has changed", which is what
/// makes it cheap to ask after every change event.
#[tauri::command]
async fn library(
    state: State<'_, AppState>,
    known_revision: u64,
) -> Answer<basalt_proto::msg::LibraryResponse> {
    Ok(state.client.library(known_revision).await?)
}

/// A poster, as a data URL an `<img>` can use directly.
///
/// A data URL rather than a byte array because the alternative is shipping
/// megabytes of JSON-encoded numbers across the IPC boundary and rebuilding a
/// blob on the other side. Posters are ~40 KB and cached by the interface, so
/// the base64 overhead is paid once per title.
#[tauri::command]
async fn library_art(state: State<'_, AppState>, id: String) -> Answer<Option<String>> {
    match state.client.art(&id).await {
        Ok(bytes) if !bytes.is_empty() => {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(Some(format!("data:image/jpeg;base64,{encoded}")))
        }
        // A missing poster is not an error worth a banner: the interface draws
        // its own instead.
        _ => Ok(None),
    }
}

/// Records where something got to, and reads back everything watched.
///
/// Also folds in whatever the media proxy has seen an external player read,
/// which is the only resume signal a player like PotPlayer gives away. It runs
/// ahead of what is actually on screen by however much the player buffered, so
/// it is an estimate — good enough for Continue watching, and much better than
/// having nothing for anything that will not play in the window.
#[tauri::command]
async fn watch_progress(
    state: State<'_, AppState>,
    update: Option<basalt_proto::msg::Watched>,
    forget: Option<String>,
) -> Answer<Vec<basalt_proto::msg::Watched>> {
    // Whatever an external player read since the last poll, reported first so
    // the answer already includes it.
    if let Some(proxy) = state.proxy.lock().await.as_ref() {
        let external = state.external.lock().expect("external lock").clone();
        for (path, reach) in proxy.take_reach() {
            let fraction = reach.fraction();
            // Read-ahead is only a resume signal for a player this app cannot
            // see. Its own reports what it is actually showing.
            if fraction <= 0.0 || !external.contains(&path) {
                continue;
            }
            let _ = state
                .client
                .progress(basalt_proto::msg::ProgressRequest {
                    update: Some(basalt_proto::msg::Watched {
                        path,
                        fraction,
                        // Seconds are unknowable from a byte offset, and the
                        // host is careful not to let this overwrite a real
                        // position with a weaker guess.
                        position: 0.0,
                        duration: 0.0,
                        updated_at: 0,
                    }),
                    forget: None,
                })
                .await;
        }
    }

    Ok(state
        .client
        .progress(basalt_proto::msg::ProgressRequest { update, forget })
        .await?)
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
async fn remove_entry(state: State<'_, AppState>, path: String, recursive: bool) -> Answer<()> {
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
    let last = Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
    let meter = Mutex::new(basalt_client::rate::Meter::new());

    Arc::new(move |p: basalt_client::Progress| {
        let now = std::time::Instant::now();
        // Every report goes into the meter, throttled or not: the rate is only
        // as good as the samples behind it.
        let (rate, eta_rate) = {
            let mut meter = meter.lock().expect("meter lock");
            meter.record(now, p.transferred);
            (meter.current(now), meter.steady(now))
        };

        let complete = p.transferred >= p.total;
        {
            let mut last = last.lock().expect("throttle lock");
            if !complete && last.elapsed() < std::time::Duration::from_millis(120) {
                return;
            }
            *last = now;
        }

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
                rate,
                eta_rate,
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
        state
            .external
            .lock()
            .expect("external lock")
            .insert(remote.clone());
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
// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

/// Which app this is, for picking the right installer out of a release.
///
/// Both apps are published from one repository, so a release carries two
/// installers and each has to recognise its own.
const PRODUCT: basalt_update::Product = basalt_update::Product::Client;

/// What is running now, as the release tags spell it.
#[tauri::command]
async fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether a newer release exists. `None` means this is the newest.
///
/// A failure to reach GitHub is an error rather than "no update": the two
/// mean different things to somebody who just pressed the button, and
/// reporting the first as the second is how an app quietly stops updating.
#[tauri::command]
async fn check_update() -> Answer<Option<basalt_update::Release>> {
    basalt_update::check(PRODUCT, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| UiError {
            kind: "error".into(),
            message: e.to_string(),
        })
}

/// Downloads an offered release, reporting progress, and returns its path.
///
/// The file is verified against the checksum published beside it before this
/// returns; an installer that fails is deleted rather than handed back.
#[tauri::command]
async fn download_update(
    app: tauri::AppHandle,
    release: basalt_update::Release,
) -> Answer<String> {
    let into = std::env::temp_dir().join("Basalt Updates");
    let emitter = app.clone();
    let path = basalt_update::fetch(&release, &into, move |had, total| {
        let _ = emitter.emit("basalt://update-progress", (had, total));
    })
    .await
    .map_err(|e| UiError {
        kind: "error".into(),
        message: e.to_string(),
    })?;

    Ok(path.to_string_lossy().into_owned())
}

/// Starts the installer and stands aside.
///
/// The app has to go: an installer cannot replace files that are open, and
/// NSIS will silently skip the executable of a running program — which is
/// exactly how somebody ends up "updating" and finding the same version.
#[tauri::command]
async fn install_update(app: tauri::AppHandle, path: String) -> Answer<()> {
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| UiError {
            kind: "error".into(),
            message: format!("could not start the installer: {e}"),
        })?;

    // A moment for the installer to be up before this window disappears,
    // so the screen is never empty with nothing apparently happening.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        app.exit(0);
    });
    Ok(())
}

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
        .plugin(tauri_plugin_libmpv::init())
        .setup(|app| {
            let store_path = basalt_client::store::default_path();
            let client = Arc::new(Basalt::open(store_path)?);

            // Registered **before** anything else in this closure.
            //
            // The window is created from the config and starts loading as soon
            // as it exists, and the first thing the interface does is ask for
            // the connection status. If that call lands before the state is
            // managed, Tauri answers "state not managed", the interface has
            // nothing to show, and the app sits on its splash screen looking
            // exactly like a crash. Spawning tasks first was enough to lose
            // that race.
            app.manage(AppState {
                client: Arc::clone(&client),
                proxy: tokio::sync::Mutex::new(None),
                transfers: Mutex::new(HashMap::new()),
                watch: Mutex::new(None),
                external: Mutex::new(std::collections::HashSet::new()),
            });

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
                    let mut interval = tokio::time::interval(std::time::Duration::from_millis(125));
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
            //
            // The watch starts only once a connection exists, and its handle is
            // kept in the app state: dropping it stops the watch, and a watch
            // that outlived what asked for it is exactly the shape of bug that
            // once turned one dropped file into eight uploads.
            {
                let client = Arc::clone(&client);
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let connected = client.connect_saved().await.is_ok();
                    let _ = handle.emit("basalt://status", status_of(&client));
                    if connected {
                        start_watching(&client, &handle);
                    } else {
                        eprintln!("basalt: no saved host reachable at startup");
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            check_update,
            download_update,
            install_update,
            status,
            discover,
            begin_pairing,
            finish_pairing,
            cancel_pairing,
            connect_saved,
            library,
            library_art,
            watch_progress,
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
