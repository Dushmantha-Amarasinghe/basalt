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
    Change, DirEntry, HelloResponse, LibraryResponse, ProfileCreateRequest, ProfileSignInRequest,
    ProfileView, ProgressRequest, Watched,
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
    /// Who is using the device right now: a profile, or the device itself.
    identity: std::sync::Mutex<Current>,
    /// Every payload byte that has crossed the link since the app started.
    ///
    /// One counter in one place rather than reporting from each call site,
    /// because the interface's throughput trace has to reflect *everything* on
    /// the link — a film being streamed through the media proxy moves far more
    /// data than any download, and a trace that only counted downloads would be
    /// wrong in exactly the moment someone is watching it.
    bytes_moved: std::sync::atomic::AtomicU64,
    /// Connections being attempted now. See [`Basalt::is_connecting`].
    connecting: std::sync::atomic::AtomicUsize,
}

/// Counts one connection attempt for as long as it lasts, however it ends.
struct Attempt<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> Attempt<'a> {
    fn begin(count: &'a std::sync::atomic::AtomicUsize) -> Self {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Basalt {
    /// Opens the client for the device it runs on, with the device's lasting
    /// id: see [`crate::store::lasting_device_id`]. `hint` is Android's id for
    /// the app, which only the Java side can read; on Windows it is `None` and
    /// the system is asked instead.
    ///
    /// The apps open this way. [`Basalt::open`] keeps a random id, which is
    /// what tests and the command line want: several clients on one machine,
    /// each its own device.
    pub fn open_as_this_device(store_path: PathBuf, hint: Option<&str>) -> Result<Self> {
        let client = Self::open(store_path)?;
        if let Some(id) = crate::store::lasting_device_id(hint) {
            let mut store = client.store.lock().expect("store lock");
            if store.device_id.as_deref() != Some(id.as_str()) {
                // A copy that paired with a random id moves to the lasting one;
                // the host learns it on the next connection.
                store.device_id = Some(id.clone());
                store.save(&client.store_path)?;
            }
            drop(store);
            return Ok(Self {
                me: Me::new(client.me.name.clone(), id),
                ..client
            });
        }
        Ok(client)
    }

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
            identity: std::sync::Mutex::new(Current::default()),
            bytes_moved: std::sync::atomic::AtomicU64::new(0),
            connecting: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Whether a connection to a host is being attempted right now.
    ///
    /// Lets the window tell "still trying" from "tried, and nothing answered":
    /// at startup the first attempt takes a moment, and showing "waiting for
    /// the drive" for that moment on every launch would be crying wolf.
    pub fn is_connecting(&self) -> bool {
        self.connecting.load(std::sync::atomic::Ordering::SeqCst) > 0
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

    /// The host `connect_saved` goes to: the one used last.
    pub fn primary_host(&self) -> Option<KnownHost> {
        self.store.lock().expect("store lock").primary().cloned()
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
                used_at: unix_now(),
                identity: Default::default(),
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
    /// Connects to a host this device has paired with.
    ///
    /// A host that answers but no longer knows this device has removed it:
    /// its pairing is dropped here as well, and [`ClientError::Removed`] says
    /// so, naming the host and the drive.
    pub async fn connect(&self, host_id: &str, address: Option<&str>) -> Result<SessionInfo> {
        let _attempt = Attempt::begin(&self.connecting);
        match self.connect_known(host_id, address).await {
            Err(e) if e.kind() == "unpaired" => Err(self.removed(host_id).await),
            other => other,
        }
    }

    /// Drops a host that has removed this device, and says which it was.
    pub async fn removed(&self, host_id: &str) -> ClientError {
        self.drop_removed(host_id)
            .await
            .unwrap_or_else(|| ClientError::Removed {
                host_name: "The host".into(),
                vault: "the drive".into(),
            })
    }

    /// [`Basalt::removed`], but `None` when the pairing had already been
    /// dropped. Several connections can learn of one removal at once, a
    /// watch and the one replacing it, and only the first can still name the
    /// host: the rest must not report it again without the names.
    async fn drop_removed(&self, host_id: &str) -> Option<ClientError> {
        // Found and forgotten under one lock, so exactly one caller gets it.
        let known = {
            let mut store = self.store.lock().expect("store lock");
            let known = store.find(host_id).cloned();
            store.forget(host_id);
            known
        }?;
        if self.status().map(|i| i.host_id).as_deref() == Some(host_id) {
            self.disconnect().await;
        }
        let _ = self.save_store();
        Some(ClientError::Removed {
            host_name: known.host_name,
            vault: known.vault,
        })
    }

    async fn connect_known(&self, host_id: &str, address: Option<&str>) -> Result<SessionInfo> {
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
            store.note_used(host_id, unix_now());
        }
        // A failure to write the address cache must not fail the connection —
        // it is an optimisation, and the client works without it.
        let _ = self.save_store();

        let pool = Pool::with_session(addr, &known.host_id, &known.token, &self.me, session);
        // The same host again, after a dropped connection or the host
        // restarting: whoever was using the device still is. It used to start
        // over from what was saved, so somebody who had chosen "this device"
        // for now, or a profile without "remember", was asked who is watching
        // in the middle of watching, every time the Wi-Fi dropped.
        let previous = self.identity.lock().expect("identity lock").clone();
        let carried_over =
            previous.chosen && !previous.ended && previous.host.as_deref() == Some(host_id);
        // Otherwise straight back in as whoever was here last time, when that
        // was remembered. Checked with the host on first use; one that has
        // ended leaves the device on its own and the app asks who is watching.
        let mut current = match &known.identity.profile {
            _ if carried_over => {
                pool.set_profile(previous.token.clone());
                previous
            }
            Some(saved) => {
                pool.set_profile(Some(saved.token.clone()));
                Current {
                    profile: Some(ProfileView {
                        id: saved.id.clone(),
                        name: saved.name.clone(),
                        color: saved.color,
                        has_pin: true,
                        last_used: 0,
                    }),
                    chosen: true,
                    token: Some(saved.token.clone()),
                    ..Current::default()
                }
            }
            None => Current {
                chosen: known.identity.always_device,
                ..Current::default()
            },
        };
        current.host = Some(host_id.to_string());
        *self.identity.lock().expect("identity lock") = current;
        *self.pool.write().await = Some(pool);
        *self.info.lock().expect("info lock") = Some(info.clone());
        Ok(info)
    }

    /// Reconnects to the host used last. What the app does on start.
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

    /// Disconnects on purpose. Whoever was using the device is forgotten too,
    /// unless remembered: only a connection that dropped carries them over.
    pub async fn disconnect(&self) {
        if let Some(pool) = self.pool.write().await.take() {
            pool.clear();
        }
        *self.info.lock().expect("info lock") = None;
        *self.identity.lock().expect("identity lock") = Current::default();
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

    // -----------------------------------------------------------------------
    // Profiles
    // -----------------------------------------------------------------------

    /// Who is using the device, and whether that still needs asking.
    ///
    /// Asks the host once when a remembered sign-in has not been checked
    /// yet, so a profile removed or signed out elsewhere is noticed here
    /// rather than on the first thing somebody tries to do.
    pub async fn identity(&self) -> IdentityState {
        if let Ok(pool) = self.pool().await {
            // Asked outright, not left to the pool: a connection already told
            // which profile it is for does not ask again by itself.
            if let Some(token) = pool.profile_token()
                && let Ok(mut lease) = pool.acquire().await
            {
                let result = lease.profile_use(Some(&token)).await;
                let _ = lease.check(result);
            }
            if pool.take_profile_ended() {
                let mut current = self.identity.lock().expect("identity lock");
                current.profile = None;
                current.chosen = false;
                current.ended = true;
                current.token = None;
                drop(current);
                self.update_identity(|identity| identity.profile = None);
            }
        }
        let current = self.identity.lock().expect("identity lock").clone();
        let last_profile = self
            .current_host()
            .and_then(|host| host.identity.last_profile);
        IdentityState {
            profile: current.profile,
            choose: !current.chosen,
            ended: current.ended,
            last_profile,
        }
    }

    /// The household's profiles.
    pub async fn profiles(&self) -> Result<Vec<ProfileView>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.profiles().await;
        Ok(lease.check(result)?.profiles)
    }

    /// Makes a profile and signs in to it.
    pub async fn create_profile(
        &self,
        name: &str,
        pin: &str,
        color: u8,
        remember: bool,
    ) -> Result<ProfileView> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease
            .profile_create(&ProfileCreateRequest {
                name: name.to_string(),
                pin: pin.to_string(),
                color,
                remember,
            })
            .await;
        let session = lease.check(result)?;
        drop(lease);
        self.adopt_profile(&pool, session, remember)
    }

    /// Signs in to a profile with its PIN.
    pub async fn sign_in_profile(
        &self,
        id: &str,
        pin: &str,
        remember: bool,
    ) -> Result<ProfileView> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease
            .profile_sign_in(&ProfileSignInRequest {
                id: id.to_string(),
                pin: pin.to_string(),
                remember,
            })
            .await;
        let session = lease.check(result)?;
        drop(lease);
        self.adopt_profile(&pool, session, remember)
    }

    fn adopt_profile(
        &self,
        pool: &Pool,
        session: basalt_proto::msg::ProfileSession,
        remember: bool,
    ) -> Result<ProfileView> {
        pool.set_profile(Some(session.token.clone()));
        let profile = session.profile;
        *self.identity.lock().expect("identity lock") = Current {
            profile: Some(profile.clone()),
            chosen: true,
            ended: false,
            token: Some(session.token.clone()),
            host: Some(pool.host_id().to_string()),
        };
        let saved = remember.then(|| crate::store::SavedProfile {
            id: profile.id.clone(),
            name: profile.name.clone(),
            color: profile.color,
            token: session.token,
        });
        let id = profile.id.clone();
        self.update_identity(move |identity| {
            identity.profile = saved;
            identity.last_profile = Some(id);
            identity.always_device = false;
        });
        Ok(profile)
    }

    /// Signs out of the profile, here. The device carries on as itself, and
    /// the app asks who is watching.
    pub async fn sign_out_profile(&self) -> Result<()> {
        let pool = self.pool().await?;
        if let Some(token) = pool.profile_token()
            && let Ok(mut lease) = pool.acquire().await
        {
            let result = lease.profile_sign_out(&token).await;
            // Best effort: signed out here whatever the host said.
            let _ = lease.check(result);
        }
        pool.set_profile(None);
        *self.identity.lock().expect("identity lock") = Current::default();
        self.update_identity(|identity| identity.profile = None);
        Ok(())
    }

    /// Carries on as the device itself, and with `always`, stops asking.
    pub async fn continue_as_device(&self, always: bool) -> Result<()> {
        let pool = self.pool().await?;
        if pool.profile_token().is_some() {
            pool.set_profile(None);
        }
        *self.identity.lock().expect("identity lock") = Current {
            chosen: true,
            host: Some(pool.host_id().to_string()),
            ..Current::default()
        };
        self.update_identity(move |identity| {
            identity.profile = None;
            identity.always_device = always;
        });
        Ok(())
    }

    /// The signed-in profile's stars, replaced first when `set` is given.
    pub async fn stars(
        &self,
        set: Option<Vec<basalt_proto::msg::Star>>,
    ) -> Result<Vec<basalt_proto::msg::Star>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.stars(set).await;
        Ok(lease.check(result)?.stars)
    }

    /// Starts converting a video on the host, from `start` seconds in.
    ///
    /// On a connection of its own, as watching for changes is: the
    /// conversion holds it for as long as it runs, and the pool's
    /// connections stay free for everything else meanwhile.
    pub async fn convert(&self, path: &str, start: f64) -> Result<Converting> {
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
        let started = session.convert_begin(path, start).await?;
        Ok(Converting {
            by: started.by,
            session,
        })
    }

    /// Whether the host could convert a video now, and what would: asked
    /// before switching to a conversion, so a host that cannot costs only
    /// the question.
    pub async fn convert_check(&self, path: &str) -> Result<basalt_proto::msg::ConvertStarted> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.convert_check(path).await;
        lease.check(result)
    }

    /// The subtitles for one video, and others that might be meant for it.
    pub async fn subtitles(&self, path: &str) -> Result<basalt_proto::msg::SubtitlesResponse> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.subtitles(path).await;
        lease.check(result)
    }

    fn current_host(&self) -> Option<KnownHost> {
        let id = self.status()?.host_id;
        self.store.lock().expect("store lock").find(&id).cloned()
    }

    /// Changes what is remembered for the connected host, and saves it.
    fn update_identity(&self, change: impl FnOnce(&mut crate::store::Identity)) {
        let Some(id) = self.status().map(|info| info.host_id) else {
            return;
        };
        {
            let mut store = self.store.lock().expect("store lock");
            if let Some(host) = store.find_mut(&id) {
                change(&mut host.identity);
            }
        }
        if let Err(e) = self.save_store() {
            tracing::warn!("could not save who is signed in: {e}");
        }
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

    /// Every video, song and photo on the drive, sorted by the host.
    pub async fn collections(
        &self,
        known_revision: u64,
    ) -> Result<basalt_proto::msg::CollectionsResponse> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.collections(known_revision).await;
        lease.check(result)
    }

    /// A preview image of a video or photo, made by the host. JPEG.
    pub async fn thumbnail(&self, path: &str, size: u32) -> Result<Vec<u8>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.thumbnail(path, size).await;
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
        self.watch_with(on_change, |_| {})
    }

    /// [`Basalt::watch`], and `on_notice` when the host changes its mind about
    /// this device.
    ///
    /// The watch is the one connection open while nobody is doing anything,
    /// so it is how that is noticed at once rather than on the next click.
    /// The host ends it when the device's access changes, and connecting
    /// again says what the access now is. Removed, the pairing is dropped
    /// here, as [`Basalt::connect`] does, and the watch ends: retrying would
    /// only be refused again.
    pub fn watch_with<F, N>(self: &Arc<Self>, on_change: F, on_notice: N) -> WatchHandle
    where
        F: Fn(Change) + Send + Sync + 'static,
        N: Fn(WatchNotice) + Send + Sync + 'static,
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
                match client.watch_once(&on_change, &on_notice, &signal).await {
                    // The caller asked it to stop.
                    Ok(()) => return,
                    Err(e) if e.kind() == "unpaired" => {
                        if let Some(host_id) = client.status().map(|i| i.host_id)
                            && let Some(removed) = client.drop_removed(&host_id).await
                        {
                            on_notice(WatchNotice::Removed(removed));
                        }
                        return;
                    }
                    Err(_) => {}
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
    async fn watch_once<F, N>(
        &self,
        on_change: &F,
        on_notice: &N,
        stop: &tokio::sync::Notify,
    ) -> Result<()>
    where
        F: Fn(Change) + Send + Sync,
        N: Fn(WatchNotice) + Send + Sync,
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
        // What the host says about access now, which is newer than what the
        // rest of the app was told when it connected.
        let writable = session.info().writable;
        let changed = {
            let mut info = self.info.lock().expect("info lock");
            match info.as_mut() {
                Some(info) if info.host_id == host_id && info.writable != writable => {
                    info.writable = writable;
                    true
                }
                _ => false,
            }
        };
        if changed {
            on_notice(WatchNotice::AccessChanged);
        }
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

        let mut file = tokio::fs::File::create(&temp).await?;
        let done = match self
            .download_into(remote, total, &mut file, progress, cancel)
            .await
        {
            Ok(done) => done,
            Err(e) => {
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

    /// Downloads into a file somebody else has already opened.
    ///
    /// For Android, where a download is saved through the system — into
    /// Downloads, or a folder the user picked — and what the app gets is an
    /// open file rather than a path. Whoever opened it decides what happens
    /// to it if this fails; nothing here deletes it.
    pub async fn download_to_file(
        &self,
        remote: &str,
        file: std::fs::File,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let total = self.stat(remote).await?.size;
        let mut file = tokio::fs::File::from_std(file);
        let done = self
            .download_into(remote, total, &mut file, progress, cancel)
            .await?;
        file.flush().await?;
        file.sync_all().await.ok();
        Ok(done)
    }

    /// The download itself: every byte of `remote`, written to `file`.
    async fn download_into(
        &self,
        remote: &str,
        total: u64,
        file: &mut tokio::fs::File,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;

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
                let got = lease.receive_read_into(&mut *file).await?;
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

        if outcome.is_err() {
            // Answers may still be on their way down this connection, and
            // nothing can tell them apart from the next request's. It cannot
            // be used again.
            lease.discard();
        }
        outcome
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
        // A folder has a size and opens as nothing. Handed to `File::open` on
        // Windows it fails as "access is denied", which is what dropping a
        // folder used to say. Folders go through `upload_tree`.
        if meta.is_dir() {
            return Err(ClientError::Protocol(format!(
                "{} is a folder",
                local.display()
            )));
        }
        let total = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        // Opened before the host is asked for anything. The other way round, a
        // file that would not open still started an upload on the host, and
        // left an empty partial file on the drive when this side gave up.
        let file = tokio::fs::File::open(local).await?;
        self.upload_open(
            file,
            total,
            mtime,
            &local.display().to_string(),
            remote,
            overwrite,
            progress,
            cancel,
        )
        .await
    }

    /// Uploads a file somebody else has already opened.
    ///
    /// For Android, where a file chosen from the phone — the gallery, the
    /// file picker, something shared from another app — arrives as an open
    /// file with no path. Its size is given rather than read from it: some
    /// sources hand over a stream that cannot say how long it is.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_file(
        &self,
        file: std::fs::File,
        size: u64,
        mtime: Option<i64>,
        remote: &str,
        overwrite: bool,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let file = tokio::fs::File::from_std(file);
        self.upload_open(
            file, size, mtime, remote, remote, overwrite, progress, cancel,
        )
        .await
    }

    /// The upload itself, from an open file of a known size.
    #[allow(clippy::too_many_arguments)]
    async fn upload_open(
        &self,
        mut file: tokio::fs::File,
        total: u64,
        mtime: Option<i64>,
        label: &str,
        remote: &str,
        overwrite: bool,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<u64> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;

        // Once more on a fresh connection if the first could not even start.
        // Nothing has been sent at this point, so there is nothing to undo,
        // and a connection the system closed while the app was in the
        // background is the usual reason — not the host, and not the file.
        let begin = match lease.write_begin(remote, total, overwrite, None).await {
            Ok(begin) => begin,
            Err(e) if e.is_transient() => {
                lease.discard();
                drop(lease);
                lease = pool.acquire().await?;
                let result = lease.write_begin(remote, total, overwrite, None).await;
                lease.check(result)?
            }
            Err(e) => return Err(lease.check::<()>(Err(e)).unwrap_err()),
        };
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
                    "{label} is shorter than the part already uploaded"
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
                    let n = read_full(&mut file, &mut buf[..want]).await?;
                    if n == 0 {
                        return Err(ClientError::Protocol(format!(
                            "{label} is shorter than it said it was"
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

/// A conversion under way on the host.
pub struct Converting {
    /// What is converting it, as people say it.
    pub by: String,
    session: Session,
}

impl Converting {
    /// The next piece of the converted film, or `None` at its end.
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>> {
        self.session.convert_next().await
    }
}

/// Who is using the device, as this run knows it.
#[derive(Debug, Clone, Default)]
struct Current {
    profile: Option<ProfileView>,
    /// Whether somebody has chosen — a profile, or the device — or it was
    /// remembered. Until then the app asks.
    chosen: bool,
    /// The host ended the profile's sign-in since the app last looked.
    ended: bool,
    /// The profile's sign-in, remembered or not, so a reconnection to the
    /// same host carries on with it.
    token: Option<String>,
    /// The host this was chosen on. Another host asks again.
    host: Option<String>,
}

/// Who is using the device, for the app.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityState {
    pub profile: Option<ProfileView>,
    /// Ask who is watching.
    pub choose: bool,
    /// Signed out by the host: removed, reset, or signed out elsewhere.
    pub ended: bool,
    /// The profile last signed in to here, to show first.
    pub last_profile: Option<String>,
}

/// What a folder upload did.
#[derive(Debug, Clone, Default)]
pub struct TreeUpload {
    /// Files that arrived.
    pub files: usize,
    /// Their bytes.
    pub bytes: u64,
    /// Files that did not, with why — vault-relative, as they would have been.
    pub failed: Vec<(String, String)>,
}

/// Everything under a local folder, parents before children.
#[derive(Debug, Default)]
struct LocalTree {
    /// Folders, relative to the root, with `/` between parts.
    dirs: Vec<String>,
    /// Files: relative path, full local path, size.
    files: Vec<(String, PathBuf, u64)>,
    /// Anything that could not be read, and why.
    unreadable: Vec<(String, String)>,
}

/// Walks a local folder without following links.
///
/// Junctions and symbolic links are skipped rather than followed: Windows
/// profiles are full of junctions that point back up the tree, and following
/// one uploads the same files forever.
fn walk_local(root: &Path) -> LocalTree {
    let mut tree = LocalTree::default();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                tree.unreadable.push((rel.clone(), e.to_string()));
                continue;
            }
        };
        let mut children: Vec<_> = entries.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if kind.is_dir() {
                tree.dirs.push(child.clone());
                pending.push((entry.path(), child));
            } else if kind.is_file() {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                tree.files.push((child, entry.path(), size));
            }
        }
    }
    // Parents first, so every folder exists before anything is put in it.
    tree.dirs.sort_by_key(|d| d.matches('/').count());
    tree.files.sort_by(|a, b| a.0.cmp(&b.0));
    tree
}

impl Basalt {
    /// Uploads a whole folder: its folders, then its files, as one transfer.
    ///
    /// Progress is reported across the lot — bytes of every file together —
    /// so the interface shows one bar for a season of episodes rather than
    /// ten. One file failing does not stop the rest; each is reported at the
    /// end. Cancelling stops the file in flight and everything after it.
    pub async fn upload_tree(
        &self,
        local: &Path,
        remote: &str,
        progress: Option<ProgressFn>,
        cancel: Option<Cancel>,
    ) -> Result<TreeUpload> {
        let root = local.to_path_buf();
        let tree = tokio::task::spawn_blocking(move || walk_local(&root))
            .await
            .map_err(|e| ClientError::Protocol(format!("could not read the folder: {e}")))?;

        let join = |rel: &str| {
            if rel.is_empty() {
                remote.to_string()
            } else {
                format!("{remote}/{rel}")
            }
        };
        let mut report = TreeUpload {
            failed: tree
                .unreadable
                .iter()
                .map(|(rel, why)| (join(rel), why.clone()))
                .collect(),
            ..TreeUpload::default()
        };

        // The folder itself, then its folders. One that is already there is
        // fine: uploading into an existing folder is how adding to one works.
        for dir in std::iter::once(String::new()).chain(tree.dirs.iter().cloned()) {
            match self.mkdir(&join(&dir)).await {
                Ok(()) => {}
                Err(e) if e.kind() == "exists" => {}
                Err(e) => return Err(e),
            }
        }

        let total: u64 = tree.files.iter().map(|(_, _, size)| size).sum();
        let mut done = 0u64;
        for (rel, path, size) in &tree.files {
            if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                return Err(ClientError::Protocol("cancelled".into()));
            }
            let target = join(rel);
            // Each file's progress, placed after everything before it — and
            // never "complete" until the last file is, because the interface
            // takes transferred == total as done.
            let file_progress = progress.as_ref().map(|outer| {
                let outer = Arc::clone(outer);
                let base = done;
                let whole = remote.to_string();
                let ceiling = total.saturating_sub(1);
                Arc::new(move |p: Progress| {
                    outer(Progress {
                        kind: TransferKind::Upload,
                        path: whole.clone(),
                        transferred: (base + p.transferred).min(ceiling),
                        total,
                    })
                }) as ProgressFn
            });
            match self
                .upload(path, &target, false, file_progress, cancel.clone())
                .await
            {
                Ok(bytes) => {
                    report.files += 1;
                    report.bytes += bytes;
                }
                Err(e) => {
                    if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                        return Err(e);
                    }
                    report.failed.push((target, e.to_string()));
                }
            }
            done += size;
        }

        if let Some(outer) = &progress {
            outer(Progress {
                kind: TransferKind::Upload,
                path: remote.to_string(),
                transferred: total,
                total,
            });
        }
        Ok(report)
    }
}

/// Reads until `buf` is full or the file ends, and says how much it got.
///
/// A single read of a file hands back at most two megabytes, whatever was
/// asked for, so every "four-megabyte" chunk used to go out as two — twice
/// the round trips the chunk size was chosen to avoid.
async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
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

/// What a watch says besides changes to the drive.
#[derive(Debug)]
pub enum WatchNotice {
    /// The host removed this device. The pairing has been dropped here too,
    /// and the watch has ended; the error names the host and drive.
    Removed(ClientError),
    /// The host changed what this device may do; [`Basalt::status`] has it.
    AccessChanged,
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
