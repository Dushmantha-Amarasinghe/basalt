//! The client API, as one object.
//!
//! Everything the interface needs and nothing it does not: pair, reconnect,
//! browse, transfer. Each call takes a connection from the pool and gives it
//! back, so a download and a folder listing never wait on each other.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Duration;

use basalt_proto::msg::{
    Change, DirEntry, HelloResponse, LibraryResponse, ProgressRequest, Watched,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::pool::Pool;
use crate::session::{Me, PairChallenge, Session, SessionInfo};
use crate::store::{ClientStore, KnownHost};
use crate::{ClientError, Result};

/// Bytes per transfer chunk.
///
/// Four megabytes is comfortably above the bandwidth-delay product of a link
/// measured at 22.7 MB/s and 2.3 ms, so the pipe stays full, and small enough
/// that a host with 5.9 GB of RAM is never asked to hold much. Phase 0 found
/// write size barely mattered — about 12% across the whole range — so this is
/// chosen for memory rather than for speed.
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// Chunks allowed on the wire before waiting for the first to be answered.
///
/// A transfer used to send one chunk and wait for the host's answer before
/// reading the next, so the link sat idle while this machine read from its
/// disk, while the host wrote to its own, and while the answer came back.
/// Starting a second file filled those gaps, which is why two transfers went
/// faster together than one did alone. The host answers a connection's
/// requests in order, so several can be sent ahead with no change to the
/// protocol; three keeps the link busy through every one of those waits
/// without asking either end to hold much.
pub const IN_FLIGHT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    Download,
    Upload,
}

/// Where a transfer has got to.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Progress {
    pub kind: TransferKind,
    pub path: String,
    pub transferred: u64,
    pub total: u64,
}

/// Called as a transfer advances. Cheap: it fires once per chunk.
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Set to stop a transfer in flight.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct Basalt {
    store_path: PathBuf,
    store: std::sync::Mutex<ClientStore>,
    pool: tokio::sync::RwLock<Option<Pool>>,
    info: std::sync::Mutex<Option<SessionInfo>>,
    /// This device's name and id, as every host sees it.
    me: Me,
    /// A pairing in progress, held open between the two steps.
    ///
    /// The host generated its PIN when the first step arrived and is showing
    /// it now; reconnecting for the second step would produce a different one.
    pending: tokio::sync::Mutex<Option<(Session, PairChallenge)>>,
    /// Every payload byte that has crossed the link since the app started.
    ///
    /// One counter in one place rather than reporting from each call site,
    /// because the interface's throughput trace has to reflect *everything* on
    /// the link — a film being streamed through the media proxy moves far more
    /// data than any download, and a trace that only counted downloads would be
    /// wrong in exactly the moment someone is watching it.
    bytes_moved: std::sync::atomic::AtomicU64,
}

impl Basalt {
    pub fn open(store_path: PathBuf) -> Result<Self> {
        let mut store = ClientStore::load(&store_path)?;
        let device_name = store
            .device_name
            .clone()
            .unwrap_or_else(crate::store::device_name);
        // Made once and kept for good. Saved straight away, so that a first
        // connection and the pairing after it cannot end up with two ids.
        let device_id = match store.device_id.clone() {
            Some(id) => id,
            None => {
                let id = crate::store::new_device_id()?;
                store.device_id = Some(id.clone());
                store.save(&store_path)?;
                id
            }
        };
        Ok(Self {
            store_path,
            store: std::sync::Mutex::new(store),
            pool: tokio::sync::RwLock::new(None),
            info: std::sync::Mutex::new(None),
            me: Me::new(device_name, device_id),
            pending: tokio::sync::Mutex::new(None),
            bytes_moved: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn device_name(&self) -> &str {
        &self.me.name
    }

    /// The id this device made for itself.
    pub fn device_id(&self) -> &str {
        &self.me.id
    }

    /// Total payload bytes moved since startup. Monotonic; callers take deltas.
    pub fn bytes_moved(&self) -> u64 {
        self.bytes_moved.load(Ordering::Relaxed)
    }

    fn count(&self, bytes: u64) {
        self.bytes_moved.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn known_hosts(&self) -> Vec<KnownHost> {
        self.store.lock().expect("store lock").hosts.clone()
    }

    /// What the client is currently connected to, if anything.
    pub fn status(&self) -> Option<SessionInfo> {
        self.info.lock().expect("info lock").clone()
    }

    pub fn is_connected(&self) -> bool {
        self.status().is_some()
    }

    fn save_store(&self) -> Result<()> {
        let snapshot = self.store.lock().expect("store lock").clone();
        snapshot.save(&self.store_path)
    }

    // -----------------------------------------------------------------------
    // Connecting
    // -----------------------------------------------------------------------

    /// Looks at a host without pairing.
    ///
    /// Only used by the command line now — the app discovers hosts rather than
    /// being told where one is — but it is the quickest way to answer "is
    /// anything listening at this address" when something is wrong.
    pub async fn probe(&self, address: &str) -> Result<HelloResponse> {
        let addr = basalt_net::socket::resolve(address, basalt_net::DEFAULT_PORT).await?;
        Session::probe(addr, &self.me).await
    }

    /// Pairs with a host at a known address, for the command line.
    pub async fn pair(&self, address: &str, pin: Option<&str>) -> Result<SessionInfo> {
        let addr = basalt_net::socket::resolve(address, basalt_net::DEFAULT_PORT).await?;
        self.pair_with(addr, pin).await
    }

    /// Every Basalt host answering on this network.
    ///
    /// The list a person picks from. Nothing here is trusted: a reply can claim
    /// anything, and the TLS pin decides the truth a moment later.
    pub async fn discover(&self) -> Result<Vec<basalt_net::discovery::Found>> {
        Ok(basalt_net::discovery::scan(basalt_net::discovery::SCAN_WINDOW).await?)
    }

    /// The same list, ready for a person to pick from.
    ///
    /// Marks the ones this device already knows, and puts them in a stable
    /// order. The order matters more than it sounds: the interface rescans on a
    /// timer, and a list that reshuffles itself between scans is one you cannot
    /// click on.
    pub async fn discover_hosts(&self) -> Result<Vec<crate::ui::DiscoveredHost>> {
        let found = self.discover().await?;
        let known: std::collections::HashSet<String> = self
            .known_hosts()
            .into_iter()
            .map(|host| host.host_id)
            .collect();

        let mut hosts: Vec<_> = found
            .iter()
            .map(|f| crate::ui::DiscoveredHost::new(f, known.contains(&f.beacon.host_id)))
            .collect();
        crate::ui::sort_hosts(&mut hosts);
        Ok(hosts)
    }

    /// Asks a host to pair, from the `ip:port` a discovered host reported.
    ///
    /// Resolution lives here rather than in the Tauri shell so the shell needs
    /// no knowledge of the network layer at all — and so that turning a bad
    /// address into a sensible error is covered by a test.
    pub async fn begin_pairing_at(&self, address: &str) -> Result<bool> {
        let addr = basalt_net::socket::resolve(address, basalt_net::DEFAULT_PORT).await?;
        self.begin_pairing(addr).await
    }

    /// Asks a host to pair, and says whether it wants a PIN.
    ///
    /// The host is displaying the request from this moment — with this device's
    /// name against the number to read across — so the interface can show a PIN
    /// field knowing one is on screen at the other end.
    pub async fn begin_pairing(&self, address: SocketAddr) -> Result<bool> {
        let (session, challenge) = Session::begin_pair(address, &self.me).await?;
        let requires_pin = challenge.requires_pin;
        *self.pending.lock().await = Some((session, challenge));
        Ok(requires_pin)
    }

    /// Completes the pairing begun by [`Basalt::begin_pairing`].
    ///
    /// Uses the session that request was made on, so the PIN the host is
    /// showing is the one being checked.
    pub async fn finish_pairing(&self, pin: Option<&str>) -> Result<SessionInfo> {
        let (mut session, challenge) = self
            .pending
            .lock()
            .await
            .take()
            .ok_or(ClientError::NotConnected)?;

        let token = match session.finish_pair(&challenge, pin).await {
            Ok(token) => token,
            Err(e) => {
                // A wrong PIN is worth another go against the same request —
                // the host is still showing it and still counting attempts.
                if matches!(e, ClientError::BadPin(_) | ClientError::PinRequired) {
                    *self.pending.lock().await = Some((session, challenge));
                }
                return Err(e);
            }
        };

        let info = session.info().clone();
        let addr = info.address;

        {
            let mut store = self.store.lock().expect("store lock");
            store.remember(KnownHost {
                host_id: info.host_id.clone(),
                token: token.clone(),
                vault: info.vault.clone(),
                host_name: info.host_name.clone(),
                last_address: Some(addr.to_string()),
                paired_at: unix_now(),
            });
            store.device_name = Some(self.me.name.clone());
        }
        self.save_store()?;

        // The connection pairing opened is already authenticated, so it goes
        // straight into the pool rather than being thrown away and redialled.
        let pool = Pool::with_session(addr, &info.host_id, &token, &self.me, session);
        *self.pool.write().await = Some(pool);
        *self.info.lock().expect("info lock") = Some(info.clone());
        Ok(info)
    }

    /// Abandons a pairing in progress.
    pub async fn cancel_pairing(&self) {
        *self.pending.lock().await = None;
    }

    /// Pairs in one call, for the command line and for tests.
    pub async fn pair_with(&self, address: SocketAddr, pin: Option<&str>) -> Result<SessionInfo> {
        self.begin_pairing(address).await?;
        self.finish_pairing(pin).await
    }

    /// Reconnects to a host already paired with.
    ///
    /// The remembered address is tried first because it usually still works and
    /// costs one round trip. If it does not, the network is asked where the
    /// pinned key is *now* — which is what makes a changed address a non-event
    /// rather than something the user has to go and look up.
    pub async fn connect(&self, host_id: &str, address: Option<&str>) -> Result<SessionInfo> {
        let known = self
            .store
            .lock()
            .expect("store lock")
            .find(host_id)
            .cloned()
            .ok_or(ClientError::NotConnected)?;

        let hint = match address
            .map(str::to_string)
            .or_else(|| known.last_address.clone())
        {
            Some(target) => basalt_net::socket::resolve(&target, basalt_net::DEFAULT_PORT)
                .await
                .ok(),
            None => None,
        };

        let mut session = None;
        if let Some(addr) = hint {
            match Session::connect(addr, &known.host_id, &known.token, &self.me).await {
                Ok(open) => session = Some(open),
                // Only a transport failure is worth looking elsewhere for. A
                // host that answered and said no — a revoked token, a key that
                // is not the pinned one — will say exactly the same thing at
                // whatever address discovery turns up, and searching would
                // turn a clear "you have been removed" into a vague "offline".
                Err(e) if !e.is_transient() => return Err(e),
                Err(_) => {}
            }
        }

        let session = match session {
            Some(session) => session,
            None => {
                // Ask the network. Only a host presenting the pinned key will
                // do, so a wrong answer costs a failed handshake and nothing
                // more.
                let found = basalt_net::discovery::find_host(
                    &known.host_id,
                    std::time::Duration::from_secs(3),
                )
                .await?
                .ok_or(ClientError::HostNotFound)?;

                Session::connect(found.address, &known.host_id, &known.token, &self.me).await?
            }
        };
        let addr = session.info().address;
        let info = session.info().clone();

        {
            let mut store = self.store.lock().expect("store lock");
            store.note_address(host_id, &addr.to_string());
        }
        // A failure to write the address cache must not fail the connection —
        // it is an optimisation, and the client works without it.
        let _ = self.save_store();

        let pool = Pool::with_session(addr, &known.host_id, &known.token, &self.me, session);
        *self.pool.write().await = Some(pool);
        *self.info.lock().expect("info lock") = Some(info.clone());
        Ok(info)
    }

    /// Reconnects to the most recently paired host. What the app does on start.
    pub async fn connect_saved(&self) -> Result<SessionInfo> {
        let primary = self
            .store
            .lock()
            .expect("store lock")
            .primary()
            .cloned()
            .ok_or(ClientError::HostNotFound)?;
        self.connect(&primary.host_id, None).await
    }

    pub async fn disconnect(&self) {
        if let Some(pool) = self.pool.write().await.take() {
            pool.clear();
        }
        *self.info.lock().expect("info lock") = None;
    }

    /// Unpairs from a host, on this side and — when it can be reached — on the
    /// host's too.
    ///
    /// It used to be this side only, and the host kept a record that would
    /// never connect again: every time a device was forgotten and paired
    /// afresh, the host's device list grew by one stale row. Telling the host
    /// is best effort. One that is switched off, or too old to understand the
    /// request, keeps the record, and lets go of it by itself in time.
    pub async fn forget(&self, host_id: &str) -> Result<()> {
        let current = self.status().map(|i| i.host_id);
        if current.as_deref() == Some(host_id) {
            if let Ok(pool) = self.pool().await
                && let Ok(mut lease) = pool.acquire().await
            {
                let _ = lease.unpair().await;
                // Never back into the pool: the host has just revoked the
                // token this connection authenticated with.
                lease.discard();
            }
            self.disconnect().await;
        }
        self.store.lock().expect("store lock").forget(host_id);
        self.save_store()
    }

    async fn pool(&self) -> Result<Pool> {
        self.pool
            .read()
            .await
            .clone()
            .ok_or(ClientError::NotConnected)
    }

    // -----------------------------------------------------------------------
    // Browsing
    // -----------------------------------------------------------------------

    pub async fn list(&self, path: &str) -> Result<Vec<DirEntry>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.list(path).await;
        lease.check(result)
    }

    pub async fn stat(&self, path: &str) -> Result<DirEntry> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.stat(path).await;
        lease.check(result)
    }

    pub async fn space(&self) -> Result<(u64, u64)> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.space().await;
        lease.check(result)
    }

    // -----------------------------------------------------------------------
    // The media library
    // -----------------------------------------------------------------------

    /// The index, or a revision marker if this client already has it.
    pub async fn library(&self, known_revision: u64) -> Result<LibraryResponse> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.library(known_revision).await;
        lease.check(result)
    }

    pub async fn art(&self, id: &str) -> Result<Vec<u8>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.art(id).await;
        lease.check(result)
    }

    /// Reports where something got to, and reads back everything watched.
    ///
    /// One call for both because a client that has just reported its position
    /// also wants the fresh list, and two calls would race each other.
    pub async fn progress(&self, request: ProgressRequest) -> Result<Vec<Watched>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.progress(request).await;
        Ok(lease.check(result)?.entries)
    }

    // -----------------------------------------------------------------------
    // Watching
    // -----------------------------------------------------------------------

    /// Calls `on_change` for everything that happens on the drive.
    ///
    /// Runs on its own connection, because a watch holds one open indefinitely
    /// and borrowing from the pool would starve everything else.
    ///
    /// Reconnects on its own. A watch that stops when the Wi-Fi hiccups is
    /// worse than no watch at all — it leaves the interface confidently showing
    /// a listing that has since changed, with nothing to suggest otherwise. So
    /// a reconnection also reports [`Change::Resynchronise`], because changes
    /// certainly happened while it was away and there is no way to know which.
    pub fn watch<F>(self: &Arc<Self>, on_change: F) -> WatchHandle
    where
        F: Fn(Change) + Send + Sync + 'static,
    {
        let stop = Arc::new(tokio::sync::Notify::new());
        let client = Arc::clone(self);
        let signal = Arc::clone(&stop);

        const FIRST_RETRY: Duration = Duration::from_millis(250);
        const SLOWEST_RETRY: Duration = Duration::from_secs(10);
        /// A connection that lasted this long is evidence the host is well.
        const HEALTHY: Duration = Duration::from_secs(30);

        let task = tokio::spawn(async move {
            let mut backoff = FIRST_RETRY;
            // The first connection is not a reconnection, so it does not claim
            // anything was missed.
            let mut reconnecting = false;

            loop {
                if reconnecting {
                    on_change(Change::Resynchronise);
                }

                let started = std::time::Instant::now();
                if client.watch_once(&on_change, &signal).await.is_ok() {
                    // The caller asked it to stop.
                    return;
                }
                reconnecting = true;

                // Only a connection that *lasted* resets the backoff. Resetting
                // after every attempt would leave it permanently at the first
                // step, which is a retry storm against a host that is off.
                if started.elapsed() >= HEALTHY {
                    backoff = FIRST_RETRY;
                }
                tokio::select! {
                    _ = signal.notified() => return,
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(SLOWEST_RETRY);
            }
        });

        WatchHandle {
            stop,
            task: Some(task),
        }
    }

    /// One watch connection, for as long as it lasts.
    async fn watch_once<F>(&self, on_change: &F, stop: &tokio::sync::Notify) -> Result<()>
    where
        F: Fn(Change) + Send + Sync,
    {
        let (addr, host_id, token) = {
            let pool = self.pool().await?;
            let known = self
                .store
                .lock()
                .expect("store lock")
                .find(pool.host_id())
                .cloned()
                .ok_or(ClientError::NotConnected)?;
            (pool.address(), known.host_id, known.token)
        };

        let mut session = Session::connect(addr, &host_id, &token, &self.me).await?;
        session.watch_begin().await?;

        loop {
            tokio::select! {
                _ = stop.notified() => return Ok(()),
                change = session.watch_next() => on_change(change?),
            }
        }
    }

    pub async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.read_range(path, offset, length).await;
        let data = lease.check(result)?;
        self.count(data.len() as u64);
        Ok(data)
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.mkdir(path).await;
        lease.check(result)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.rename(from, to).await;
        lease.check(result)
    }

    /// Duplicates a path on the host. Nothing crosses the link but the request.
    pub async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.copy(from, to).await;
        lease.check(result)
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.remove(path, recursive).await;
        lease.check(result)
    }

    /// Fetches many small files in one round trip.
    ///
    /// The measured difference against asking for them one at a time is 7.6x,
    /// which is the largest single win in the system. Anything that needs more
    /// than a handful of files should come through here.
    pub async fn read_batch(&self, paths: Vec<String>) -> Result<Vec<basalt_proto::Entry>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.read_batch(paths).await;
        let entries = lease.check(result)?;
        self.count(entries.iter().map(|e| e.data.len() as u64).sum());
        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // Transfers
    // -----------------------------------------------------------------------

    /// Downloads a file, reporting progress as it goes.
    ///
    /// Written to a `.part` file and renamed at the end, for the same reason
    /// uploads are: a transfer interrupted three quarters of the way through a
    /// film must not leave something that looks like a playable file.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let entry = self.stat(remote).await?;
        let total = entry.size;

        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temp = local.with_extension(format!(
            "{}part",
            local
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}."))
                .unwrap_or_default()
        ));

        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let mut file = tokio::fs::File::create(&temp).await?;

        let outcome: Result<u64> = async {
            // Ranges asked for and not yet read back, oldest first.
            let mut asked: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
            let mut requested = 0u64;
            let mut done = 0u64;

            while done < total {
                while asked.len() < IN_FLIGHT && requested < total {
                    let want = CHUNK_BYTES.min(total - requested);
                    lease.send_read(remote, requested, want).await?;
                    asked.push_back(want);
                    requested += want;
                }
                if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                    return Err(ClientError::Protocol("cancelled".into()));
                }

                let expected = asked.pop_front().expect("a range is always in flight here");
                let got = lease.receive_read_into(&mut file).await?;
                self.count(got);
                // The ranges after this one were asked for assuming this one
                // was whole. A short answer means the file changed underneath
                // the download, and carrying on would stitch two versions of
                // it together.
                if got != expected {
                    return Err(ClientError::Protocol(format!(
                        "{remote} changed while it was downloading ({done} of {total} bytes)"
                    )));
                }
                done += got;

                if let Some(report) = &progress {
                    report(Progress {
                        kind: TransferKind::Download,
                        path: remote.to_string(),
                        transferred: done,
                        total,
                    });
                }
            }
            Ok(done)
        }
        .await;

        let done = match outcome {
            Ok(done) => done,
            Err(e) => {
                // Answers may still be on their way down this connection, and
                // nothing can tell them apart from the next request's. It
                // cannot be used again.
                lease.discard();
                drop(file);
                tokio::fs::remove_file(&temp).await.ok();
                return Err(e);
            }
        };

        file.flush().await?;
        drop(file);
        // `rename` refuses to replace an existing file on Windows, so an
        // overwrite has to remove the old one first.
        if local.exists() {
            tokio::fs::remove_file(local).await?;
        }
        tokio::fs::rename(&temp, local).await?;
        Ok(done)
    }

    /// Uploads a file, resuming if the host already holds part of it.
    ///
    /// Hashed as it is read for sending, rather than in a pass of its own
    /// before the first byte goes: that pass read the whole file once more
    /// and held the transfer back while it did.
    pub async fn upload(
        &self,
        local: &Path,
        remote: &str,
        overwrite: bool,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let meta = tokio::fs::metadata(local).await?;
        let total = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;

        let begin = {
            let result = lease.write_begin(remote, total, overwrite, None).await;
            lease.check(result)?
        };

        let mut file = tokio::fs::File::open(local).await?;
        let mut buf = vec![0u8; CHUNK_BYTES as usize];
        let mut hasher = blake3::Hasher::new();

        // The digest covers the whole file, so a part the host already holds
        // is read through the hash here, without being sent again.
        let mut sent = 0u64;
        while sent < begin.offset {
            let want = (CHUNK_BYTES.min(begin.offset - sent)) as usize;
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(ClientError::Protocol(format!(
                    "{} is shorter than the part already uploaded",
                    local.display()
                )));
            }
            hasher.update(&buf[..n]);
            sent += n as u64;
        }

        let outcome: Result<()> = async {
            // Chunks sent and not yet confirmed written, oldest first.
            let mut unconfirmed: std::collections::VecDeque<u64> =
                std::collections::VecDeque::new();
            let mut confirmed = sent;

            while confirmed < total {
                while unconfirmed.len() < IN_FLIGHT && sent < total {
                    if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                        return Err(ClientError::Protocol("cancelled".into()));
                    }
                    let want = (CHUNK_BYTES.min(total - sent)) as usize;
                    let n = file.read(&mut buf[..want]).await?;
                    if n == 0 {
                        return Err(ClientError::Protocol(format!(
                            "{} is shorter than it said it was",
                            local.display()
                        )));
                    }
                    hasher.update(&buf[..n]);
                    lease.send_chunk(&begin.upload, sent, &buf[..n]).await?;
                    unconfirmed.push_back(n as u64);
                    sent += n as u64;
                }

                lease.confirm_chunk().await?;
                let n = unconfirmed
                    .pop_front()
                    .expect("a chunk is always in flight here");
                self.count(n);
                confirmed += n;

                // Progress is what the host has confirmed writing, not what
                // has merely left this machine.
                if let Some(report) = &progress {
                    report(Progress {
                        kind: TransferKind::Upload,
                        path: remote.to_string(),
                        transferred: confirmed,
                        total,
                    });
                }
            }
            Ok(())
        }
        .await;

        if let Err(e) = outcome {
            // Confirmations may still be on their way back, and would be read
            // as the answer to whatever this connection is asked next.
            lease.discard();
            drop(lease);
            // Cancelling, or a file that was not what it claimed, ends the
            // upload. Anything else — the link dropping — leaves the partial
            // file on the host, so trying again carries on rather than
            // starting over.
            let deliberate = matches!(&e, ClientError::Protocol(_));
            if deliberate && let Ok(mut other) = pool.acquire().await {
                let _ = other.write_abort(&begin.upload).await;
            }
            return Err(e);
        }

        let digest = hasher.finalize().to_hex().to_string();
        let result = lease.write_commit(&begin.upload, &digest, mtime).await;
        lease.check(result)?;
        Ok(total)
    }
}

/// BLAKE3 of a whole file, read in blocks so a film never lands in memory.
///
/// What an upload's streamed hash has to agree with.
#[cfg(test)]
fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Keeps a watch running. Dropping it stops the watch.
///
/// A handle rather than a detached task on purpose: a subscription that
/// outlives whatever asked for it is how the client came to start eight
/// uploads from one drop, and the fix there was the same — make the lifetime
/// something the caller holds.
pub struct WatchHandle {
    stop: Arc<tokio::sync::Notify>,
    /// Taken by `stop`, so `Drop` knows it has already been dealt with.
    task: Option<tokio::task::JoinHandle<()>>,
}

impl WatchHandle {
    /// Stops watching and waits for the task to finish.
    pub async fn stop(mut self) {
        self.stop.notify_waiters();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.notify_waiters();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_token_starts_unset_and_latches() {
        let cancel = Cancel::new();
        assert!(!cancel.is_cancelled());
        cancel.cancel();
        assert!(cancel.is_cancelled());

        // Clones share the flag: the UI holds one and the transfer the other.
        let copy = cancel.clone();
        assert!(copy.is_cancelled());
    }

    #[test]
    fn a_fresh_client_is_not_connected() {
        let path =
            std::env::temp_dir().join(format!("basalt-client-{}-none.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let client = Basalt::open(path.clone()).unwrap();
        assert!(!client.is_connected());
        assert!(client.status().is_none());
        assert!(client.known_hosts().is_empty());
        assert!(!client.device_name().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn operations_without_a_connection_say_so() {
        let path =
            std::env::temp_dir().join(format!("basalt-client-{}-ops.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let client = Basalt::open(path.clone()).unwrap();

        assert!(matches!(
            client.list("").await,
            Err(ClientError::NotConnected)
        ));
        assert!(matches!(
            client.mkdir("x").await,
            Err(ClientError::NotConnected)
        ));
        assert!(matches!(
            client.connect_saved().await,
            Err(ClientError::HostNotFound)
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hashing_matches_blake3_of_the_same_bytes() {
        let path = std::env::temp_dir().join(format!("basalt-hash-{}.bin", std::process::id()));
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();

        assert_eq!(
            hash_file(&path).unwrap(),
            blake3::hash(&data).to_hex().to_string()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn hashing_an_empty_file_works() {
        let path = std::env::temp_dir().join(format!("basalt-empty-{}.bin", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            blake3::hash(b"").to_hex().to_string()
        );
        let _ = std::fs::remove_file(&path);
    }
}
