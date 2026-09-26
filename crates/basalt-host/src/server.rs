//! The serving loop and the operation dispatch.
//!
//! One task per connection, each strictly request-then-response. There is no
//! multiplexing inside a connection and there does not need to be: the client
//! opens a second connection when it wants to browse during a transfer, and
//! Phase 0 measured extra connections as free (1 stream 20.8 MB/s, 16 streams
//! 24.1 MB/s). Head-of-line blocking is solved by not sharing the line.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use basalt_net::framing::{read_request_raw, write_err, write_ok, write_response_header};
use basalt_net::identity::HostIdentity;
use basalt_net::tls::server_config;
use basalt_proto::codec::{Codec, CompressionPolicy};
use basalt_proto::frame::{BatchWriter, sanitize_relative_path};
use basalt_proto::manifest::BatchRequest;
use basalt_proto::msg::*;
use basalt_proto::ops::{MAX_READ_BYTES, Op, STATUS_OK};
use basalt_proto::{ErrorCode, PROTOCOL_VERSION};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::config::HostConfig;
use crate::error::{HostError, Result};
use crate::registry::{Device, PairingRequest, Registry};
use crate::traffic::Traffic;
use crate::uploads::{Uploads, is_temp_name};
use crate::vault::Vault;

/// Everything shared between connections.
pub struct Host {
    identity: HostIdentity,
    config_path: std::path::PathBuf,
    config: std::sync::Mutex<HostConfig>,
    vault: tokio::sync::RwLock<Option<Arc<Vault>>>,
    registry: std::sync::Mutex<Registry>,
    uploads: Uploads,
    traffic: Traffic,
    /// Watching the drive. Replaced whenever the vault is.
    watch: tokio::sync::RwLock<Option<Arc<crate::watch::Watch>>>,
    /// Bumped each time the watcher is replaced, so anything following it can
    /// move to the new one. See [`Host::keep_library_current`].
    watch_generation: tokio::sync::watch::Sender<u64>,
    /// Whether the chosen drive was missing the last time anyone looked.
    drive_lost: std::sync::atomic::AtomicBool,
    /// The media index, and whether a scan is running.
    library: std::sync::Mutex<crate::media::Library>,
    /// Every video, song and photo on the drive, and the newest files.
    collections: std::sync::Mutex<crate::media::collect::Stored>,
    /// Thumbnails being made at once. Two: a grid scrolled quickly asks for
    /// dozens, and making them all together would starve everything else the
    /// host is doing — a film being streamed, most of all.
    thumb_gate: tokio::sync::Semaphore,
    /// Thumbnails written, for trimming the cache every so often.
    thumbs_written: std::sync::atomic::AtomicUsize,
    scanning: std::sync::atomic::AtomicBool,
    /// A scan was asked for while one was already running.
    rescan_wanted: std::sync::atomic::AtomicBool,
    /// Videos that arrived while a scan was walking the drive.
    ///
    /// A scan replaces the whole index when it finishes, and one that had
    /// already passed a folder when a file landed in it would otherwise drop
    /// that file again — filed on arrival, then gone until the scan after.
    arrived_during_scan: std::sync::Mutex<Vec<String>>,
    /// When the last scan finished and how long it took, so the next one can
    /// be held off in proportion to what scanning this drive actually costs.
    last_scan: std::sync::Mutex<Option<(std::time::Instant, std::time::Duration)>>,
    /// Where each file has been watched to: each device's, and each
    /// profile's, own history.
    progress: std::sync::Mutex<crate::media::Progress>,
    /// The household's profiles and who is signed in to them.
    profiles: std::sync::Mutex<crate::profiles::ProfileBook>,
    /// Each profile's starred files, by profile id.
    stars: std::sync::Mutex<std::collections::HashMap<String, Vec<Star>>>,
}

impl Host {
    pub fn new(config: HostConfig, config_path: std::path::PathBuf) -> Result<Arc<Self>> {
        let identity = config.identity()?;
        let mut registry = Registry::new(config.devices.clone(), config.require_pin);
        let abandoned = registry.forget_abandoned(unix_now());

        // A drive that is not there is not a reason to refuse to start.
        //
        // This used to be `?`, and the app's startup turned the error into an
        // exit — so a host whose USB drive was unplugged simply vanished on
        // launch, with no window and nothing saying why. It now starts without
        // the drive, says so, and picks the drive up again when it returns.
        let vault = match &config.vault_path {
            Some(path) => match Vault::open(path, &config.vault_name) {
                Ok(vault) => Some(Arc::new(vault)),
                Err(e) => {
                    tracing::warn!(
                        "{} is not available ({e}); waiting for it to come back",
                        path.display()
                    );
                    None
                }
            },
            None => None,
        };
        let drive_lost = config.vault_path.is_some() && vault.is_none();

        // Starting the watcher must not stop a host from serving: a drive that
        // will not report changes is still a drive you can read.
        let watch = vault.as_ref().and_then(|vault| {
            crate::watch::Watch::start(vault.root())
                .inspect_err(|e| tracing::warn!("changes will not be live: {e}"))
                .ok()
        });

        let library = match (&config.vault_path, config.library_enabled) {
            (Some(root), true) => {
                crate::media::index::Library::load(&crate::media::index::index_path(
                    config_path.parent().unwrap_or(std::path::Path::new(".")),
                    root,
                ))
            }
            _ => crate::media::index::Library::default(),
        };

        let collections = match &config.vault_path {
            Some(root) => crate::media::collect::Stored::load(&crate::media::collect::stored_path(
                config_path.parent().unwrap_or(std::path::Path::new(".")),
                root,
            )),
            None => crate::media::collect::Stored::default(),
        };

        let config_dir = config_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        // Histories from before profiles began afresh: cleared, not read.
        let cleared = crate::media::Progress::clear_legacy(&config_dir);
        if cleared > 0 {
            tracing::info!("cleared {cleared} watch histories from before profiles");
        }
        let mut profiles = crate::profiles::ProfileBook::new(
            config.profiles.clone(),
            config.profile_tokens.clone(),
        );
        profiles.prune(unix_now());
        let stars = match &config.vault_path {
            Some(root) => load_stars(&stars_path(&config_dir, root)),
            None => Default::default(),
        };

        let progress = match &config.vault_path {
            Some(root) => crate::media::Progress::load(&crate::media::Progress::path_for(
                config_path.parent().unwrap_or(std::path::Path::new(".")),
                root,
            )),
            None => crate::media::Progress::default(),
        };

        let host = Arc::new(Self {
            identity,
            config_path,
            config: std::sync::Mutex::new(config),
            vault: tokio::sync::RwLock::new(vault),
            registry: std::sync::Mutex::new(registry),
            uploads: Uploads::default(),
            traffic: Traffic::default(),
            watch: tokio::sync::RwLock::new(watch),
            watch_generation: tokio::sync::watch::Sender::new(0),
            drive_lost: std::sync::atomic::AtomicBool::new(drive_lost),
            library: std::sync::Mutex::new(library),
            collections: std::sync::Mutex::new(collections),
            thumb_gate: tokio::sync::Semaphore::new(2),
            thumbs_written: std::sync::atomic::AtomicUsize::new(0),
            scanning: std::sync::atomic::AtomicBool::new(false),
            rescan_wanted: std::sync::atomic::AtomicBool::new(false),
            arrived_during_scan: std::sync::Mutex::new(Vec::new()),
            last_scan: std::sync::Mutex::new(None),
            progress: std::sync::Mutex::new(progress),
            profiles: std::sync::Mutex::new(profiles),
            stars: std::sync::Mutex::new(stars),
        });
        if abandoned > 0 {
            tracing::info!("let go of {abandoned} devices that never came back");
            host.persist()?;
        }
        Ok(host)
    }

    pub async fn watch(&self) -> Option<Arc<crate::watch::Watch>> {
        self.watch.read().await.clone()
    }

    /// Swaps the watcher, and tells anything following the old one to move.
    ///
    /// Dropping the old watcher is what stops it, so nothing may keep holding
    /// it: the rescan loop once did, and went on watching the previous drive —
    /// for ever — while the one actually being shared got no rescans at all.
    async fn replace_watch(&self, watch: Option<Arc<crate::watch::Watch>>) {
        *self.watch.write().await = watch;
        self.watch_generation
            .send_modify(|generation| *generation += 1);
    }

    /// Tells every connected client something changed.
    pub async fn announce(&self, change: basalt_proto::msg::Change) {
        if let Some(watch) = self.watch().await {
            watch.announce(change);
        }
    }

    // -----------------------------------------------------------------------
    // The media library
    // -----------------------------------------------------------------------

    pub fn library_enabled(&self) -> bool {
        self.config.lock().expect("config lock").library_enabled
    }

    pub fn library_revision(&self) -> u64 {
        self.library.lock().expect("library lock").revision
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn library_items(&self) -> Vec<basalt_proto::msg::LibraryItem> {
        self.library.lock().expect("library lock").items.clone()
    }

    /// Answers a client, omitting the index when it already has this revision.
    pub fn library_response(&self, known_revision: u64) -> basalt_proto::msg::LibraryResponse {
        let library = self.library.lock().expect("library lock");
        let enabled = self.library_enabled();
        basalt_proto::msg::LibraryResponse {
            revision: library.revision,
            enabled,
            scanning: self.is_scanning(),
            // Revision 0 means "never scanned", which a client cannot already
            // have — so it always gets the (empty) list rather than silence.
            items: if enabled && (known_revision != library.revision || library.revision == 0) {
                Some(library.items.clone())
            } else {
                None
            },
            sections: self.sections(),
        }
    }

    /// Which library sections devices show.
    pub fn sections(&self) -> basalt_proto::msg::Sections {
        self.config.lock().expect("config lock").sections
    }

    /// Changes which sections devices show, and tells them.
    pub async fn set_sections(&self, sections: basalt_proto::msg::Sections) -> Result<()> {
        self.config.lock().expect("config lock").sections = sections;
        self.persist()?;
        // The library answer carries the sections, so asking every device to
        // fetch it again is how they hear.
        self.announce(basalt_proto::msg::Change::LibraryChanged)
            .await;
        Ok(())
    }

    /// A picture of a video or photo: from the cache, or made now.
    ///
    /// Anything that cannot be pictured — no mpv beside the host, a file the
    /// decoders do not read — is "not found", which the device shows as its
    /// ordinary tile.
    pub async fn thumbnail(&self, rel: &str, size: u32) -> Result<Vec<u8>> {
        let vault = self
            .vault()
            .await
            .ok_or_else(|| HostError::Unavailable(self.vault_name()))?;
        let entry = vault.stat(rel)?;
        let path = vault.resolve(rel)?;
        let bucket = crate::media::thumbs::bucket(size);
        let dir = self.config_dir().join("thumbs");
        let cached = crate::media::thumbs::cache_path(&dir, rel, entry.size, entry.mtime, bucket);
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }

        let _turn = self
            .thumb_gate
            .acquire()
            .await
            .map_err(|_| HostError::NotFound(rel.to_string()))?;
        // Somebody else may have made it while this waited its turn.
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let bytes = tokio::task::spawn_blocking(move || crate::media::thumbs::make(&path, bucket))
            .await
            .map_err(|e| HostError::BadRequest(format!("the thumbnail panicked: {e}")))?
            .map_err(|e| HostError::NotFound(e.to_string()))?;

        if let Some(parent) = cached.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&cached, &bytes).await;
        let written = self
            .thumbs_written
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if written % 500 == 499 {
            tokio::task::spawn_blocking(move || crate::media::thumbs::trim(&dir));
        }
        Ok(bytes)
    }

    /// Answers a client, omitting the collections when it already has them.
    pub fn collections_response(
        &self,
        known_revision: u64,
    ) -> basalt_proto::msg::CollectionsResponse {
        let stored = self.collections.lock().expect("collections lock");
        basalt_proto::msg::CollectionsResponse {
            revision: stored.revision,
            scanning: self.is_scanning(),
            // Revision 0 is "never walked", which no client can already have.
            collections: (known_revision != stored.revision || stored.revision == 0)
                .then(|| stored.collections.clone()),
        }
    }

    fn collections_path(&self) -> Option<std::path::PathBuf> {
        Some(crate::media::collect::stored_path(
            self.config_dir(),
            &self.vault_path()?,
        ))
    }

    fn save_collections(&self) {
        let snapshot = self.collections.lock().expect("collections lock").clone();
        if let Some(path) = self.collections_path()
            && let Err(e) = snapshot.save(&path)
        {
            tracing::warn!("could not save the collections: {e}");
        }
    }

    /// Adds files that just arrived to the collections.
    async fn collect_arrivals(&self, vault: &Arc<Vault>, paths: &[String]) -> bool {
        let (vault, paths) = (Arc::clone(vault), paths.to_vec());
        let existing = self
            .collections
            .lock()
            .expect("collections lock")
            .collections
            .clone();
        let next = tokio::task::spawn_blocking(move || {
            let arrived: Vec<basalt_proto::msg::MediaFile> = paths
                .iter()
                .filter(|path| {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    !crate::uploads::is_temp_name(name)
                        && !crate::media::parse::is_system(path)
                        && basalt_proto::media::kind_of(path).is_some()
                })
                .filter_map(|path| {
                    let entry = vault.stat(path).ok()?;
                    (entry.kind == basalt_proto::msg::EntryKind::File).then(|| {
                        basalt_proto::msg::MediaFile {
                            path: path.clone(),
                            size: entry.size,
                            mtime: entry.mtime,
                            width: None,
                            height: None,
                        }
                    })
                })
                .collect();
            if arrived.is_empty() {
                return None;
            }
            Some(crate::media::collect::add(&existing, &arrived, |rel| {
                vault
                    .resolve(rel)
                    .ok()
                    .and_then(|p| crate::media::collect::photo_size(&p))
            }))
        })
        .await
        .ok()
        .flatten();
        let Some(next) = next else {
            return false;
        };
        let changed = self
            .collections
            .lock()
            .expect("collections lock")
            .replace(next);
        if changed {
            self.save_collections();
        }
        changed
    }

    /// Drops files and folders that went away from the collections, at once.
    ///
    /// No walk needed: a path that is gone is gone, and everything under a
    /// folder that is gone went with it.
    async fn file_removals(&self, paths: Vec<String>) {
        let changed = {
            let mut stored = self.collections.lock().expect("collections lock");
            let next = crate::media::collect::remove(&stored.collections, &paths);
            stored.replace(next)
        };
        if changed {
            self.save_collections();
            self.announce(basalt_proto::msg::Change::LibraryChanged)
                .await;
        }
    }

    fn library_path(&self) -> Option<std::path::PathBuf> {
        Some(self.library_path_for(&self.vault_path()?))
    }

    fn config_dir(&self) -> &std::path::Path {
        self.config_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
    }

    fn library_path_for(&self, root: &std::path::Path) -> std::path::PathBuf {
        crate::media::index::index_path(self.config_dir(), root)
    }

    /// Where artwork for one item is cached.
    fn art_path(&self, id: &str) -> Option<std::path::PathBuf> {
        // Ids are hex from a hash, so nothing here can walk out of the folder —
        // but checked anyway, because this value arrived over the wire and a
        // path built from one is exactly where traversal bugs live.
        if !id.chars().all(|c| c.is_ascii_alphanumeric()) || id.is_empty() || id.len() > 64 {
            return None;
        }
        Some(crate::media::art::art_path(self.config_path.parent()?, id))
    }

    pub fn art(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.art_path(id)?).ok()
    }

    // -----------------------------------------------------------------------
    // Watch progress
    // -----------------------------------------------------------------------

    fn progress_path(&self) -> Option<std::path::PathBuf> {
        Some(crate::media::Progress::path_for(
            self.config_dir(),
            &self.vault_path()?,
        ))
    }

    /// Records, forgets, then answers with everything this device should see.
    ///
    /// One call rather than three, because a client reporting its position also
    /// wants the current list — and doing both in one round trip means it sees
    /// its own update rather than a stale answer that races it.
    ///
    /// `owner` is whose history this is: the signed-in profile's, or the
    /// device's own (see [`owner_of`]). There is no shared history any more —
    /// a profile is what carries a place from one device to another.
    pub fn progress(
        &self,
        request: basalt_proto::msg::ProgressRequest,
        owner: Option<&str>,
    ) -> basalt_proto::msg::ProgressResponse {
        let Some(owner) = owner else {
            return basalt_proto::msg::ProgressResponse {
                entries: Vec::new(),
            };
        };
        let now = unix_now();
        let (entries, changed) = {
            let mut progress = self.progress.lock().expect("progress lock");
            let mut changed = false;
            if let Some(update) = request.update {
                progress.record_owned(owner, update, now);
                changed = true;
            }
            if let Some(path) = request.forget {
                changed |= progress.forget_owned(owner, &path);
            }
            (progress.all_for(owner), changed)
        };

        if changed {
            self.save_progress();
        }
        basalt_proto::msg::ProgressResponse { entries }
    }

    fn save_progress(&self) {
        let snapshot = self.progress.lock().expect("progress lock").clone();
        if let Some(path) = self.progress_path()
            && let Err(e) = snapshot.save(&path)
        {
            tracing::warn!("could not save watch progress: {e}");
        }
    }

    /// Turns the library on or off.
    ///
    /// Turning it off drops the index rather than hiding it: an index nobody
    /// asked for should not sit on disk, and rebuilding is a scan away.
    pub async fn set_library_enabled(self: &Arc<Self>, enabled: bool) -> Result<()> {
        self.config.lock().expect("config lock").library_enabled = enabled;
        self.persist()?;

        if enabled {
            self.start_scan();
        } else {
            self.library.lock().expect("library lock").items.clear();
            if let Some(path) = self.library_path() {
                let _ = std::fs::remove_file(path);
            }
            self.announce(basalt_proto::msg::Change::LibraryChanged)
                .await;
        }
        Ok(())
    }

    /// Turns the library on *without* scanning, to stand in for a restart.
    ///
    /// Only for tests. `set_library_enabled` scans as a side effect, which
    /// would hide the very thing the restart test is checking.
    #[doc(hidden)]
    pub fn enable_library_for_test(&self) {
        self.config.lock().expect("config lock").library_enabled = true;
    }

    // -----------------------------------------------------------------------
    // Profiles
    // -----------------------------------------------------------------------

    pub fn profile_views(&self) -> Vec<ProfileView> {
        self.profiles.lock().expect("profiles lock").views()
    }

    /// Every profile with the devices signed in to it, for the host's window.
    pub fn profiles_overview(&self) -> Vec<crate::ui::ProfileSummary> {
        let devices = self.devices();
        let book = self.profiles.lock().expect("profiles lock");
        book.profiles()
            .iter()
            .map(|profile| {
                let mut signed_in: Vec<crate::ui::ProfileDevice> = book
                    .tokens()
                    .iter()
                    .filter(|t| t.profile_id == profile.id)
                    .map(|t| crate::ui::ProfileDevice {
                        name: devices
                            .iter()
                            .find(|d| d.key() == t.device_key)
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| "A device no longer paired".into()),
                        remembered: t.remembered,
                        last_used: t.last_used,
                    })
                    .collect();
                signed_in.sort_by_key(|d| std::cmp::Reverse(d.last_used));
                crate::ui::ProfileSummary {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    color: profile.color,
                    has_pin: profile.pin_hash.is_some(),
                    created_at: profile.created_at,
                    last_used: profile.last_used,
                    devices: signed_in,
                }
            })
            .collect()
    }

    fn create_profile(
        &self,
        request: ProfileCreateRequest,
        device_key: &str,
    ) -> Result<(ProfileView, String)> {
        let (profile, token) = self.profiles.lock().expect("profiles lock").create(
            &request.name,
            &request.pin,
            request.color,
            device_key,
            request.remember,
            unix_now(),
        )?;
        self.persist()?;
        Ok((profile.view(), token))
    }

    fn sign_in_profile(
        &self,
        request: ProfileSignInRequest,
        device_key: &str,
    ) -> Result<(ProfileView, String)> {
        let (profile, token) = self.profiles.lock().expect("profiles lock").sign_in(
            &request.id,
            &request.pin,
            device_key,
            request.remember,
            unix_now(),
        )?;
        self.persist()?;
        Ok((profile.view(), token))
    }

    fn resolve_profile(&self, token: &str) -> Option<ProfileView> {
        self.profiles
            .lock()
            .expect("profiles lock")
            .resolve(token, unix_now())
            .map(|p| p.view())
    }

    /// Whether the profile a connection acts for is still signed in.
    ///
    /// A connection is told once which profile it works for, and holds on to
    /// that — so a profile removed on the host, or its PIN reset, would have
    /// gone on being used by every connection already open. Checked before
    /// anything that reads or writes a profile's history or stars.
    fn check_profile(&self, session: &mut Session) -> Result<()> {
        let Some((id, hash)) = &session.profile else {
            return Ok(());
        };
        let standing = self
            .profiles
            .lock()
            .expect("profiles lock")
            .still_signed_in(id, hash);
        if standing {
            Ok(())
        } else {
            session.profile = None;
            Err(HostError::SignedOut)
        }
    }

    fn sign_out_profile(&self, token: &str) -> Result<bool> {
        let ended = self.profiles.lock().expect("profiles lock").sign_out(token);
        if ended {
            self.persist()?;
        }
        Ok(ended)
    }

    /// Clears a profile's PIN from the host's window, for a forgotten one.
    /// Every device is signed out, and the next sign-in chooses a new PIN.
    pub fn reset_profile_pin(&self, id: &str) -> Result<bool> {
        let done = self.profiles.lock().expect("profiles lock").reset_pin(id);
        if done {
            self.persist()?;
        }
        Ok(done)
    }

    /// Removes a profile with its history and stars.
    pub fn remove_profile(&self, id: &str) -> Result<bool> {
        let removed = self.profiles.lock().expect("profiles lock").remove(id);
        if removed {
            self.persist()?;
            if self
                .progress
                .lock()
                .expect("progress lock")
                .drop_owner(&format!("profile:{id}"))
            {
                self.save_progress();
            }
            if self.stars.lock().expect("stars lock").remove(id).is_some() {
                self.save_stars();
            }
        }
        Ok(removed)
    }

    /// A profile's stars, replaced first when `set` is given.
    fn stars_of(&self, profile: &str, set: Option<Vec<Star>>) -> Vec<Star> {
        let (stars, changed) = {
            let mut all = self.stars.lock().expect("stars lock");
            let changed = set.is_some();
            if let Some(mut list) = set {
                list.truncate(MAX_STARS);
                all.insert(profile.to_string(), list);
            }
            (all.get(profile).cloned().unwrap_or_default(), changed)
        };
        if changed {
            self.save_stars();
        }
        stars
    }

    fn save_stars(&self) {
        let snapshot = self.stars.lock().expect("stars lock").clone();
        if let Some(root) = self.vault_path() {
            let path = stars_path(self.config_dir(), &root);
            let written = serde_json::to_vec(&snapshot)
                .map_err(std::io::Error::other)
                .and_then(|json| {
                    let temp = path.with_extension("tmp");
                    std::fs::write(&temp, json)?;
                    std::fs::rename(&temp, &path)
                });
            if let Err(e) = written {
                tracing::warn!("could not save stars: {e}");
            }
        }
    }

    /// Turns poster downloads on or off.
    ///
    /// Turning it off leaves the posters already downloaded where they are:
    /// they are on this machine already, and deleting them would be a second
    /// decision nobody asked for. Turning it on scans, which is what actually
    /// fetches them.
    pub async fn set_posters(self: &Arc<Self>, enabled: bool) -> Result<()> {
        self.config.lock().expect("config lock").posters = enabled;
        self.persist()?;
        if enabled {
            self.start_scan();
        }
        Ok(())
    }

    /// Stores the optional TMDb key and fetches whatever artwork it unlocks.
    ///
    /// Clearing it stops future lookups but leaves the posters already
    /// downloaded: they are on this machine already, and deleting them would
    /// be a second decision the user did not ask for.
    pub async fn set_tmdb_key(self: &Arc<Self>, key: &str) -> Result<()> {
        self.config.lock().expect("config lock").tmdb_key = key.trim().to_string();
        self.persist()?;
        self.start_scan();
        Ok(())
    }

    /// Keeps the index in step with the drive, from startup until the host stops.
    ///
    /// The watcher keeps *listings* live; without this the index would not be,
    /// and a film copied in would sit in Files but never appear under Movies
    /// until somebody pressed a button.
    ///
    /// **It follows the watcher, not a watcher.** It used to subscribe once, to
    /// whichever watcher existed when the server started — and when the drive
    /// was changed it kept holding the old one, so the old drive stayed watched
    /// and the one being shared got no rescans until the host restarted. A host
    /// set up for the first time had no watcher at startup at all, and its loop
    /// ended before the drive was even chosen. Now it moves to each new watcher
    /// as it appears, and waits when there is none.
    ///
    /// **Arrivals are filed at once; everything else waits for a scan.** A new
    /// video is filed on its own the moment the drive goes quiet — see
    /// [`crate::media::index::add`]. Removals, renames and new folders need the
    /// full walk, and that walk is rationed: see [`REST_MULTIPLE`].
    ///
    /// Also covers the case a restart would otherwise lose: the index on disk
    /// says nothing about what happened while the host was off.
    pub fn keep_library_current(self: &Arc<Self>) {
        use tokio::sync::broadcast::error::RecvError;

        /// Quiet time after the last change before acting on any of them.
        /// Copying a season is a change per episode; it is one event here.
        const SETTLE: std::time::Duration = std::time::Duration::from_secs(8);

        self.start_scan();

        let host = Arc::clone(self);
        tokio::spawn(async move {
            let mut generation = host.watch_generation.subscribe();
            loop {
                // Whatever watcher is current, subscribed to and then let go
                // of: holding it would keep the drive watched after the host
                // itself has moved on.
                let changes = host.watch().await.map(|watch| watch.subscribe());
                let Some(mut changes) = changes else {
                    if generation.changed().await.is_err() {
                        return;
                    }
                    continue;
                };

                'following: loop {
                    let first = tokio::select! {
                        replaced = generation.changed() => {
                            if replaced.is_err() {
                                return;
                            }
                            break 'following;
                        }
                        change = changes.recv() => change,
                    };

                    let mut batch = Batch::default();
                    match first {
                        Ok(change) => batch.note(&change),
                        Err(RecvError::Lagged(_)) => batch.needs_scan = true,
                        Err(RecvError::Closed) => break 'following,
                    }
                    if batch.is_empty() {
                        continue;
                    }

                    // Let the drive settle. Anything happening counts as not
                    // settled, including writes that change nothing about the
                    // library — a copy still in progress is exactly that.
                    loop {
                        match tokio::time::timeout(SETTLE, changes.recv()).await {
                            Ok(Ok(change)) => batch.note(&change),
                            Ok(Err(RecvError::Lagged(_))) => batch.needs_scan = true,
                            Ok(Err(RecvError::Closed)) | Err(_) => break,
                        }
                    }

                    if !batch.removed.is_empty() {
                        host.file_removals(std::mem::take(&mut batch.removed)).await;
                    }
                    if !batch.arrived.is_empty() {
                        host.file_arrivals(batch.arrived).await;
                    }
                    if batch.needs_scan {
                        // Waited out rather than skipped: the changes are real,
                        // and dropping them would leave the index wrong until
                        // something else happened to trigger a scan.
                        let wait = host.rest_before_next_scan();
                        if !wait.is_zero() {
                            tracing::debug!("holding the next scan off for {wait:?}");
                            tokio::time::sleep(wait).await;
                        }
                        host.start_scan();
                    }
                }
            }
        });
    }

    /// How long until another full scan is allowed.
    fn rest_before_next_scan(&self) -> std::time::Duration {
        let last = *self.last_scan.lock().expect("scan clock");
        last.and_then(|(finished, took)| {
            let gap = (took * REST_MULTIPLE).min(MAX_REST);
            gap.checked_sub(finished.elapsed())
        })
        .unwrap_or_default()
    }

    /// Files videos that just arrived, without walking the drive.
    async fn file_arrivals(self: &Arc<Self>, paths: Vec<String>) {
        let Some(vault) = self.vault().await else {
            return;
        };
        if self.collect_arrivals(&vault, &paths).await {
            self.announce(basalt_proto::msg::Change::LibraryChanged)
                .await;
        }
        if !self.library_enabled() {
            return;
        }
        // Remembered for a scan already walking the drive, which would
        // otherwise finish and replace the index without them.
        if self.is_scanning() {
            self.arrived_during_scan
                .lock()
                .expect("arrivals lock")
                .extend(paths.iter().cloned());
        }

        // Tried against the index as it stands, and retried if a scan swapped
        // it underneath — writing an index computed from a stale copy would
        // undo whatever that scan found.
        for _ in 0..3 {
            let (revision, existing) = {
                let library = self.library.lock().expect("library lock");
                (library.revision, library.items.clone())
            };
            let (vault, paths) = (Arc::clone(&vault), paths.clone());
            let added = tokio::task::spawn_blocking(move || {
                crate::media::index::add(
                    &existing,
                    &vault,
                    &paths,
                    basalt_catalog::Catalog::bundled(),
                )
            })
            .await
            .ok()
            .flatten();
            let Some(mut items) = added else {
                return;
            };
            self.dress(&mut items).await;

            let snapshot = {
                let mut library = self.library.lock().expect("library lock");
                if library.revision != revision {
                    continue;
                }
                let scanned_at = library.scanned_at;
                library.replace(items, scanned_at);
                library.clone()
            };
            self.save_library(&snapshot);
            tracing::info!("filed new arrivals without a scan");
            self.announce(basalt_proto::msg::Change::LibraryChanged)
                .await;
            return;
        }
    }

    /// Fetches posters for anything missing one and marks which items have
    /// one. Shared by a full scan and by arrivals, so a film added on its own
    /// gets its poster the same way.
    async fn dress(&self, items: &mut [basalt_proto::msg::LibraryItem]) {
        // Best effort and never fatal: no key, no network, or a title nobody
        // has a poster for each cost a poster and nothing more.
        let config_dir = self.config_dir().to_path_buf();
        let (posters, key) = {
            let config = self.config.lock().expect("config lock");
            (config.posters, config.tmdb_key.clone())
        };
        crate::media::art::enrich(&*items, posters, &key, &config_dir).await;

        // Marked after fetching, so an item whose poster just arrived is
        // already flagged in the index the client is about to be sent.
        let have = crate::media::art::cached(&config_dir);
        for item in items.iter_mut() {
            item.has_art = have.contains(&item.id);
        }
    }

    fn save_library(&self, snapshot: &crate::media::Library) {
        if let Some(path) = self.library_path()
            && let Err(e) = snapshot.save(&path)
        {
            tracing::warn!("could not save the library index: {e}");
        }
    }

    /// Rebuilds the index in the background.
    ///
    /// Completely, every time. That is what makes deleted files disappear
    /// without any separate bookkeeping to fall out of step — the scan is the
    /// truth and anything absent from it is gone by construction.
    pub fn start_scan(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;

        // Whether or not films are being recognised: the same walk is what
        // finds everything for Videos, Music and Photos.
        //
        // One scan at a time. A request while one runs is remembered rather
        // than dropped: the running scan may already have passed whatever
        // changed, so another follows it, after the usual rest.
        if self.scanning.swap(true, Ordering::SeqCst) {
            self.rescan_wanted.store(true, Ordering::SeqCst);
            return;
        }

        let host = Arc::clone(self);
        tokio::spawn(async move {
            let outcome = host.scan_once().await;
            host.scanning.store(false, Ordering::SeqCst);
            match outcome {
                Ok(true) => {
                    host.announce(basalt_proto::msg::Change::LibraryChanged)
                        .await;
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("the library scan failed: {e}"),
            }

            if host.rescan_wanted.swap(false, Ordering::SeqCst) {
                let wait = host.rest_before_next_scan();
                tokio::time::sleep(wait).await;
                host.start_scan();
            }
        });
    }

    async fn scan_once(&self) -> Result<bool> {
        let Some(vault) = self.vault().await else {
            return Ok(false);
        };
        self.arrived_during_scan
            .lock()
            .expect("arrivals lock")
            .clear();
        let began = std::time::Instant::now();
        let films = self.library_enabled();
        let now = unix_now();
        let walked = Arc::clone(&vault);
        let crate::media::index::Walk {
            mut items,
            gathered,
            stale_parts,
        } = tokio::task::spawn_blocking(move || crate::media::index::walk(&walked, films, now))
            .await
            .map_err(|e| HostError::BadRequest(format!("the scan panicked: {e}")))?;

        // Partial uploads nobody came back for. Looked at again first: one the
        // walk saw may have been resumed since. An upload still in progress
        // holds its file open, and Windows refuses to delete it.
        if !stale_parts.is_empty() {
            let swept = Arc::clone(&vault);
            let removed = tokio::task::spawn_blocking(move || {
                stale_parts
                    .iter()
                    .filter(|rel| {
                        let Ok(entry) = swept.stat(rel) else {
                            return false;
                        };
                        let Ok(path) = swept.resolve(rel) else {
                            return false;
                        };
                        crate::media::collect::is_stale_part(entry.size, entry.mtime, now)
                            && std::fs::remove_file(path).is_ok()
                    })
                    .count()
            })
            .await
            .unwrap_or(0);
            if removed > 0 {
                tracing::info!("removed {removed} partial uploads nobody came back for");
            }
        }

        // The collections, with photo sizes read for anything new.
        let previous = self
            .collections
            .lock()
            .expect("collections lock")
            .collections
            .clone();
        let measured = Arc::clone(&vault);
        let collections = tokio::task::spawn_blocking(move || {
            gathered.finish(&previous, |rel| {
                measured
                    .resolve(rel)
                    .ok()
                    .and_then(|p| crate::media::collect::photo_size(&p))
            })
        })
        .await
        .map_err(|e| HostError::BadRequest(format!("the scan panicked: {e}")))?;
        let mut collections_changed = self
            .collections
            .lock()
            .expect("collections lock")
            .replace(collections);

        // Anything that landed while the walk was under way, filed on top.
        let arrived = std::mem::take(&mut *self.arrived_during_scan.lock().expect("arrivals lock"));
        if !arrived.is_empty() {
            collections_changed |= self.collect_arrivals(&vault, &arrived).await;
        }
        if collections_changed {
            self.save_collections();
        }

        if !films {
            *self.last_scan.lock().expect("scan clock") =
                Some((std::time::Instant::now(), began.elapsed()));
            return Ok(collections_changed);
        }

        if !arrived.is_empty() {
            let base = items.clone();
            if let Some(with) = tokio::task::spawn_blocking(move || {
                crate::media::index::add(
                    &base,
                    &vault,
                    &arrived,
                    basalt_catalog::Catalog::bundled(),
                )
            })
            .await
            .ok()
            .flatten()
            {
                items = with;
            }
        }

        self.dress(&mut items).await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let (changed, snapshot) = {
            let mut library = self.library.lock().expect("library lock");
            let changed = library.replace(items, now);
            (changed, library.clone())
        };
        self.save_library(&snapshot);

        *self.last_scan.lock().expect("scan clock") =
            Some((std::time::Instant::now(), began.elapsed()));
        Ok(changed || collections_changed)
    }

    pub fn host_id(&self) -> &str {
        &self.identity.host_id
    }

    pub fn host_name(&self) -> String {
        self.config.lock().expect("config lock").host_name.clone()
    }

    pub fn vault_name(&self) -> String {
        self.config.lock().expect("config lock").vault_name.clone()
    }

    pub async fn vault(&self) -> Option<Arc<Vault>> {
        self.vault.read().await.clone()
    }

    /// Locks in a drive, replacing whatever was being served.
    pub async fn set_vault(self: &Arc<Self>, path: &std::path::Path, name: &str) -> Result<()> {
        // Opened before anything is written, so choosing a drive that cannot
        // be read leaves the host exactly as it was.
        let vault = Vault::open(path, name)?;
        {
            let mut config = self.config.lock().expect("config lock");
            config.vault_path = Some(path.to_path_buf());
            config.vault_name = name.to_string();
        }
        self.persist()?;
        self.attach(vault).await;
        Ok(())
    }

    /// Starts serving an opened drive: watcher, library and history.
    ///
    /// Shared by choosing a drive and by a drive coming back, so a USB drive
    /// plugged back in gets exactly what choosing it did.
    async fn attach(self: &Arc<Self>, vault: Vault) {
        use std::sync::atomic::Ordering;

        let vault = Arc::new(vault);
        *self.vault.write().await = Some(Arc::clone(&vault));
        self.drive_lost.store(false, Ordering::SeqCst);

        // A new watcher, and the old one dropped. That closes every watch
        // connection, and a client that reconnects gets changes for the drive
        // actually being served now.
        let watch = crate::watch::Watch::start(vault.root())
            .inspect_err(|e| tracing::warn!("changes will not be live: {e}"))
            .ok();
        self.replace_watch(watch).await;

        // A different drive is a different library, and a different history.
        //
        // The history used to stay behind: switching drives kept the previous
        // drive's resume points in memory and saved them under the new one's
        // name, so neither drive's file was right afterwards.
        let root = self
            .vault_path()
            .unwrap_or_else(|| vault.root().to_path_buf());
        let library = match self.library_enabled() {
            true => crate::media::index::Library::load(&self.library_path_for(&root)),
            false => crate::media::index::Library::default(),
        };
        *self.library.lock().expect("library lock") = library;
        *self.progress.lock().expect("progress lock") = crate::media::Progress::load(
            &crate::media::Progress::path_for(self.config_dir(), &root),
        );
        *self.collections.lock().expect("collections lock") = crate::media::collect::Stored::load(
            &crate::media::collect::stored_path(self.config_dir(), &root),
        );
        *self.stars.lock().expect("stars lock") = load_stars(&stars_path(self.config_dir(), &root));

        self.start_scan();
        self.announce(basalt_proto::msg::Change::Resynchronise)
            .await;
    }

    /// Notices the chosen drive going away and coming back.
    ///
    /// A host whose drive is unplugged keeps running and keeps being found —
    /// devices see it, and its window says what happened — and the moment the
    /// drive is back it is served again, with nobody having to do anything.
    pub fn keep_drive_attached(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;

        /// Often enough that plugging a drive back in feels immediate.
        const EVERY: std::time::Duration = std::time::Duration::from_secs(3);

        let host = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(EVERY).await;
                let (path, name) = {
                    let config = host.config.lock().expect("config lock");
                    match &config.vault_path {
                        Some(path) => (path.clone(), config.vault_name.clone()),
                        None => continue,
                    }
                };

                // Off the runtime: asking about a disconnected network drive
                // can take Windows a long time to answer.
                let probe = path.clone();
                let present =
                    tokio::task::spawn_blocking(move || crate::drives::is_available(&probe))
                        .await
                        .unwrap_or(false);
                let lost = host.drive_lost.load(Ordering::SeqCst);

                if !present && !lost {
                    tracing::warn!("{} has gone; waiting for it to come back", path.display());
                    host.drive_lost.store(true, Ordering::SeqCst);
                    // Let go of it entirely: requests then say the drive is
                    // missing rather than failing with a path error, and the
                    // watcher stops holding a handle on a device Windows
                    // wants to release.
                    *host.vault.write().await = None;
                    host.replace_watch(None).await;
                } else if present && lost {
                    match Vault::open(&path, &name) {
                        Ok(vault) => {
                            tracing::info!("{} is back", path.display());
                            host.attach(vault).await;
                        }
                        Err(e) => tracing::debug!("{} is not readable yet: {e}", path.display()),
                    }
                }
            }
        });
    }

    /// Where the served drive lives, if one has been chosen.
    pub fn vault_path(&self) -> Option<std::path::PathBuf> {
        self.config.lock().expect("config lock").vault_path.clone()
    }

    pub fn port(&self) -> u16 {
        self.config.lock().expect("config lock").port
    }

    /// Renames the host. Clients see this before they pair.
    pub fn set_host_name(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HostError::BadRequest("a host needs a name".into()));
        }
        self.config.lock().expect("config lock").host_name = name.chars().take(64).collect();
        self.persist()
    }

    /// Whether Windows starts this host at login.
    ///
    /// Read from the registry rather than from the config, because the user can
    /// turn it off in Task Manager's Startup tab without this app ever running.
    /// The config field is only a mirror, and the registry is the truth.
    pub fn start_with_windows(&self) -> bool {
        crate::autostart::is_enabled()
    }

    pub fn set_start_with_windows(&self, enabled: bool) -> Result<()> {
        crate::autostart::set_enabled(enabled)?;
        self.config.lock().expect("config lock").start_with_windows = enabled;
        self.persist()
    }

    /// Everything the host's own window needs to draw itself once.
    pub async fn status(&self, serving: bool) -> crate::ui::HostStatus {
        // Off the async runtime. Both of these are Windows calls against the
        // drive, and a drive that has spun down — or that a scan is currently
        // hammering — can leave them sitting for seconds. The interface asks
        // for this every two seconds, so blocking a runtime thread here means
        // blocking it more or less permanently.
        let vault = match self.vault().await {
            Some(vault) => tokio::task::spawn_blocking(move || {
                let (free, total) = vault.space();
                crate::ui::VaultView {
                    path: crate::drives::display(vault.root()),
                    name: vault.name().to_string(),
                    free,
                    total,
                    available: crate::drives::is_available(vault.root()),
                }
            })
            .await
            .ok(),
            // Chosen, but not there. Shown as the drive it is, marked missing,
            // rather than as no drive at all — which would drop the window
            // back to setup as if the choice had never been made.
            None => {
                let config = self.config.lock().expect("config lock");
                config.vault_path.as_ref().map(|path| crate::ui::VaultView {
                    path: crate::drives::display(path),
                    name: config.vault_name.clone(),
                    free: 0,
                    total: 0,
                    available: false,
                })
            }
        };

        let (host_name, port, require_pin, sections) = {
            let config = self.config.lock().expect("config lock");
            (
                config.host_name.clone(),
                config.port,
                config.require_pin,
                config.sections,
            )
        };
        let profiles = self.profiles_overview();

        // Both taken before the struct is built, for the reason spelled out on
        // `library_status`: a guard written inline as a field value is a
        // temporary that outlives its field and is still held while every later
        // field is evaluated. `device_count` used to hold `registry` across the
        // `library_status()` call below, so a deadlock in there took the device
        // list and every incoming connection down with it.
        let device_count = self.registry.lock().expect("registry lock").device_count();
        let library = self.library_status();
        let addresses = basalt_net::discovery::local_addresses()
            .into_iter()
            .map(|ip| ip.to_string())
            .collect();

        crate::ui::HostStatus {
            host_id: self.identity.host_id.clone(),
            host_name,
            port,
            require_pin,
            start_with_windows: self.start_with_windows(),
            vault,
            addresses,
            device_count,
            library,
            profiles,
            sections,
            serving,
            problem: None,
        }
    }

    /// What the host's own window shows about the index.
    ///
    /// Every lock is taken once, into a local, **before** the struct is built.
    ///
    /// This function used to lock `config` twice inside the struct literal —
    /// once for `enabled` and once for `has_key`. A guard created in a struct
    /// expression is a temporary, and a temporary lives to the end of the whole
    /// statement, not to the end of its field: the first guard was still held
    /// when the second lock was attempted, and `std::sync::Mutex` is not
    /// reentrant, so the thread waited on itself forever.
    ///
    /// It froze the entire app rather than one call. The window asks for the
    /// status every two seconds, and each attempt wedged another thread while
    /// holding `library` — which also stopped every scan from saving its index,
    /// and `registry`, which stopped paired devices from connecting at all.
    fn library_status(&self) -> crate::ui::LibraryStatus {
        let (enabled, posters, has_key) = {
            let config = self.config.lock().expect("config lock");
            (
                config.library_enabled,
                config.posters,
                !config.tmdb_key.trim().is_empty(),
            )
        };
        let scanning = self.is_scanning();
        let (videos, music, photos) = {
            let stored = self.collections.lock().expect("collections lock");
            let c = &stored.collections;
            (c.videos.len(), c.music.len(), c.photos.len())
        };
        let library = self.library.lock().expect("library lock");

        crate::ui::LibraryStatus {
            videos,
            music,
            photos,
            enabled,
            scanning,
            films: library
                .items
                .iter()
                .filter(|i| i.kind == basalt_proto::msg::LibraryKind::Film)
                .count(),
            series: library
                .items
                .iter()
                .filter(|i| i.kind == basalt_proto::msg::LibraryKind::Series)
                .count(),
            uncertain: library
                .items
                .iter()
                .filter(|i| i.confidence < basalt_proto::msg::CONFIDENT)
                .count(),
            with_art: library.items.iter().filter(|i| i.has_art).count(),
            posters,
            has_key,
            scanned_at: library.scanned_at,
        }
    }

    /// Devices waiting to be let in, with the PIN each was given.
    pub fn pending_pairings(&self) -> Vec<PairingRequest> {
        self.registry
            .lock()
            .expect("registry lock")
            .pending(Instant::now())
    }

    /// Refuses a waiting request.
    pub fn deny_pairing(&self, id: &str) -> bool {
        self.registry.lock().expect("registry lock").deny(id)
    }

    pub fn require_pin(&self) -> bool {
        self.registry.lock().expect("registry lock").require_pin()
    }

    /// Turns the PIN requirement on or off.
    ///
    /// With it off, anyone on this network who finds the host can read the
    /// drive. That is a real decision, so the host app says so in as many words
    /// rather than presenting it as a preference.
    pub fn set_require_pin(&self, require: bool) -> Result<()> {
        self.registry
            .lock()
            .expect("registry lock")
            .set_require_pin(require);
        self.config.lock().expect("config lock").require_pin = require;
        self.persist()
    }

    /// Everything each device has moved since the host started.
    pub fn traffic(&self) -> std::collections::HashMap<String, crate::traffic::DeviceTraffic> {
        self.traffic.snapshot()
    }

    /// What this host broadcasts about itself.
    pub fn beacon(&self) -> basalt_net::discovery::Beacon {
        let config = self.config.lock().expect("config lock");
        basalt_net::discovery::Beacon {
            host_id: self.identity.host_id.clone(),
            host_name: config.host_name.clone(),
            vault: config.vault_name.clone(),
            port: config.port,
            requires_pin: config.require_pin,
            has_vault: config.vault_path.is_some(),
        }
    }

    /// Renames a device in the list.
    pub fn rename_device(&self, token_hash: &str, name: &str) -> Result<bool> {
        let changed = self
            .registry
            .lock()
            .expect("registry lock")
            .rename(token_hash, name);
        if changed {
            self.persist()?;
        }
        Ok(changed)
    }

    pub fn devices(&self) -> Vec<Device> {
        self.registry
            .lock()
            .expect("registry lock")
            .devices()
            .to_vec()
    }

    /// Grants or withdraws write access for one device.
    ///
    /// Takes effect on the device's next connection: the grant is copied into
    /// the session at authentication so it does not have to be looked up on
    /// every single request.
    pub fn set_writable(&self, token_hash: &str, writable: bool) -> Result<bool> {
        let changed = self
            .registry
            .lock()
            .expect("registry lock")
            .set_writable(token_hash, writable);
        if changed {
            self.persist()?;
        }
        Ok(changed)
    }

    pub fn revoke(&self, token_hash: &str) -> Result<bool> {
        let removed = {
            let mut registry = self.registry.lock().expect("registry lock");
            let key = registry.device(token_hash).map(|d| d.key().to_string());
            let removed = registry.revoke(token_hash);
            if let Some(key) = key.filter(|_| removed) {
                self.profiles
                    .lock()
                    .expect("profiles lock")
                    .forget_device(&key);
            }
            removed
        };
        if removed {
            self.traffic.forget(token_hash);
            self.persist()?;
        }
        Ok(removed)
    }

    /// Copies the live device list into the config and writes it out.
    fn persist(&self) -> Result<()> {
        let snapshot = {
            let devices = self
                .registry
                .lock()
                .expect("registry lock")
                .devices()
                .to_vec();
            let (profiles, tokens) = {
                let book = self.profiles.lock().expect("profiles lock");
                (book.profiles().to_vec(), book.tokens().to_vec())
            };
            let mut config = self.config.lock().expect("config lock");
            config.devices = devices;
            config.profiles = profiles;
            config.profile_tokens = tokens;
            config.clone()
        };
        snapshot.save(&self.config_path)
    }

    /// Says a failure caused by the drive disappearing as exactly that.
    ///
    /// The drive monitor notices within seconds and lets go of it, but in
    /// those seconds every request still reached the drive and failed on its
    /// own terms — "was not found", with nothing after it — and a device
    /// showed an empty folder rather than a missing drive. Only failures are
    /// checked, so this costs nothing while everything works.
    async fn explain(&self, e: HostError) -> HostError {
        if !matches!(
            e,
            HostError::NotFound(_) | HostError::Io(_) | HostError::Denied(_)
        ) {
            return e;
        }
        let Some(vault) = self.vault().await else {
            return e;
        };
        let root = vault.root().to_path_buf();
        let gone = tokio::task::spawn_blocking(move || !crate::drives::is_available(&root))
            .await
            .unwrap_or(false);
        if gone {
            HostError::Unavailable(format!(
                "{} is not connected to the host right now",
                vault.name()
            ))
        } else {
            e
        }
    }

    async fn require_vault(&self) -> Result<Arc<Vault>> {
        if let Some(vault) = self.vault().await {
            return Ok(vault);
        }
        let config = self.config.lock().expect("config lock");
        Err(match config.vault_path {
            Some(_) => HostError::Unavailable(format!(
                "{} is not connected to the host right now",
                config.vault_name
            )),
            None => HostError::Denied("this host has not been given a drive to share yet".into()),
        })
    }
}

/// Starred items a profile may keep.
const MAX_STARS: usize = 5_000;

/// Where stars live: beside `host.json`, keyed by drive.
fn stars_path(config_dir: &std::path::Path, vault_root: &std::path::Path) -> std::path::PathBuf {
    let key = blake3::hash(vault_root.to_string_lossy().as_bytes()).to_hex();
    config_dir.join(format!("stars-{}.json", &key[..16]))
}

fn load_stars(path: &std::path::Path) -> std::collections::HashMap<String, Vec<Star>> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How much of the time a drive may spend being scanned.
///
/// The rest before the next full scan is this multiple of how long the last
/// one took, so the limit tunes itself to the drive: a small library scans in a
/// blink and stays live, while a whole drive that takes ninety seconds earns a
/// quarter of an hour to itself. Nothing else works for both — a fixed gap is
/// either too slow for a folder of films or far too eager for a 4 TB disk on an
/// old laptop, where a drive something else writes to continuously would
/// otherwise mean scanning continuously.
const REST_MULTIPLE: u32 = 10;

/// However costly a scan was, it never earns more rest than this.
const MAX_REST: std::time::Duration = std::time::Duration::from_secs(900);

/// What one settled burst of changes asks of the library.
#[derive(Debug, Default, PartialEq, Eq)]
struct Batch {
    /// Media that appeared, to be filed on its own.
    arrived: Vec<String>,
    /// Media and folders that went, dropped from the collections at once.
    removed: Vec<String>,
    /// Something only a full walk can account for: a removal, a new folder,
    /// or the watcher losing track.
    needs_scan: bool,
}

impl Batch {
    fn is_empty(&self) -> bool {
        self.arrived.is_empty() && self.removed.is_empty() && !self.needs_scan
    }

    fn note(&mut self, change: &basalt_proto::msg::Change) {
        use crate::media::parse::{is_system, is_video};
        use basalt_proto::msg::Change;

        // Windows writes to its own folders constantly. A drive root shared
        // whole would otherwise be permanently "just changed", and the index
        // would scan for ever on a machine doing nothing a person would notice.
        let interesting = |path: &String| !is_system(path);
        // A path with no extension is most likely a folder, and a folder that
        // appears or goes may hold any number of videos.
        let folder = |path: &String| !path.rsplit('/').next().unwrap_or(path).contains('.');

        let media = |path: &String| basalt_proto::media::kind_of(path).is_some();

        match change {
            Change::Created { path } if interesting(path) && (is_video(path) || media(path)) => {
                if !self.arrived.contains(path) {
                    self.arrived.push(path.clone());
                }
            }
            Change::Created { path } if interesting(path) && folder(path) => {
                self.needs_scan = true;
            }
            Change::Removed { path } if interesting(path) && (media(path) || folder(path)) => {
                self.removed.push(path.clone());
                // Films need the walk to notice a removal; the collections
                // have already let it go.
                if is_video(path) || folder(path) {
                    self.needs_scan = true;
                }
            }
            Change::Renamed { from, to } => {
                if interesting(to) && (is_video(to) || media(to)) && !self.arrived.contains(to) {
                    self.arrived.push(to.clone());
                }
                if interesting(from) && (media(from) || folder(from)) {
                    self.removed.push(from.clone());
                }
                if interesting(from) && (is_video(from) || folder(from)) {
                    self.needs_scan = true;
                }
                // A folder renamed brings everything in it under a new name,
                // which only a walk can list.
                if interesting(to) && folder(to) {
                    self.needs_scan = true;
                }
            }
            Change::Resynchronise => self.needs_scan = true,
            Change::Created { .. }
            | Change::Removed { .. }
            | Change::Modified { .. }
            | Change::LibraryChanged => {}
        }
    }
}

/// A bound but not yet accepting server.
///
/// Splitting bind from serve is what lets tests pass port 0 and read back
/// whichever port the OS handed out. Probing for a free port and binding it
/// separately is a race, and under `cargo test`'s parallelism that race loses
/// often enough to make a suite flaky.
pub struct BoundServer {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    host: Arc<Host>,
    addr: SocketAddr,
}

impl BoundServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn host(&self) -> &Arc<Host> {
        &self.host
    }
}

pub async fn bind(host: Arc<Host>, addr: SocketAddr) -> Result<BoundServer> {
    let acceptor = server_config(&host.identity)
        .map_err(|e| HostError::BadRequest(format!("could not start TLS: {e}")))?;
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    Ok(BoundServer {
        listener,
        acceptor,
        host,
        addr,
    })
}

/// Accepts connections until the future is dropped.
pub async fn serve(server: BoundServer) -> Result<()> {
    let BoundServer {
        listener,
        acceptor,
        host,
        ..
    } = server;

    // Keep the media index in step with the drive, from now until the host
    // stops. Does nothing at all when the library is switched off.
    host.keep_library_current();
    // And keep serving the drive through it being unplugged and plugged back.
    host.keep_drive_attached();

    // Announce on the local network for as long as this host is serving, so
    // clients never have to be told an address. A failure here is not fatal —
    // a host nobody can discover is still a host somebody can reach directly.
    {
        let host = Arc::clone(&host);
        tokio::spawn(async move {
            if let Err(e) = basalt_net::discovery::respond(move || host.beacon()).await {
                tracing::warn!("discovery is not running: {e}");
            }
        });
    }

    loop {
        let (stream, peer) = listener.accept().await?;
        basalt_net::socket::tune(&stream);
        let acceptor = acceptor.clone();
        let host = Arc::clone(&host);

        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls) => {
                    if let Err(e) = serve_connection(tls, host).await {
                        tracing::debug!("connection from {peer} ended: {e}");
                    }
                }
                // A failed handshake is usually a port scanner or a browser
                // finding the port, not a bug. Worth a line, not a warning.
                Err(e) => tracing::debug!("tls handshake with {peer} failed: {e}"),
            }
        });
    }
}

/// Per-connection state.
#[derive(Default)]
struct Session {
    device: Option<Device>,
    /// The profile this connection acts for, when it has signed in to one,
    /// and the hash of the sign-in it acts on.
    profile: Option<(String, String)>,
    /// What the device said about itself when it connected: its name and id.
    hello: Option<HelloRequest>,
}

impl Session {
    /// The token hash this connection authenticated with, for accounting.
    fn device_key(&self) -> Option<String> {
        self.device.as_ref().map(|d| d.token_hash.clone())
    }
}

impl Session {
    /// Whose history and stars this connection works with: the profile's
    /// when signed in to one, the device's own otherwise.
    fn owner(&self) -> Option<String> {
        match (&self.profile, &self.device) {
            (Some((profile, _)), _) => Some(format!("profile:{profile}")),
            (None, Some(device)) => Some(device.key().to_string()),
            (None, None) => None,
        }
    }

    fn writable(&self) -> bool {
        self.device.as_ref().is_some_and(|d| d.writable)
    }
}

pub async fn serve_connection<S>(mut stream: S, host: Arc<Host>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut session = Session::default();
    // Undone when this connection ends, however it ends.
    let mut counted: Option<String> = None;

    let result = loop {
        let (raw, payload) = match read_request_raw(&mut stream).await {
            Ok(v) => v,
            // A peer that goes away between requests is the normal end of a
            // pooled connection, not a failure.
            Err(_) => break Ok(()),
        };
        // Something a newer device asks for that this host cannot do. Said
        // so, and the connection kept: the device can carry on without it.
        let Ok(op) = basalt_proto::ops::Op::from_u8(raw) else {
            write_err(
                &mut stream,
                ErrorCode::Unsupported,
                "this host does not do that yet; update Basalt Host",
            )
            .await?;
            continue;
        };

        // Register the connection the moment it has a device to attribute it
        // to, so the host can show how many each one holds open.
        if counted.is_none()
            && let Some(key) = session.device_key()
        {
            host.traffic.connected(&key);
            counted = Some(key);
        }

        if !op.allowed_unauthenticated() && session.device.is_none() {
            write_err(
                &mut stream,
                ErrorCode::Unauthenticated,
                "present a device token first",
            )
            .await?;
            continue;
        }

        if let Err(e) = dispatch(&mut stream, &host, &mut session, op, &payload).await {
            // A refusal is an answer, not a reason to hang up: the client is
            // pooling this connection and will use it again.
            let e = host.explain(e).await;
            let code = e.code();
            tracing::debug!("{op:?} failed: {e}");
            if let Err(e) = write_err(&mut stream, code, &e.to_string()).await {
                break Err(e.into());
            }
        }
    };

    if let Some(key) = counted {
        host.traffic.disconnected(&key);
    }
    result
}

async fn dispatch<S>(
    stream: &mut S,
    host: &Arc<Host>,
    session: &mut Session,
    op: Op,
    payload: &[u8],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match op {
        Op::Ping => write_ok(stream, &[]).await?,

        Op::Hello => {
            // Kept for the rest of the connection: pairing and authenticating
            // both want to know which device this is.
            session.hello = Some(decode(payload)?);
            reply(
                stream,
                &HelloResponse {
                    protocol: PROTOCOL_VERSION,
                    vault: host.vault_name(),
                    host_id: host.host_id().to_string(),
                    pairing_open: true,
                    host_name: host.host_name(),
                },
            )
            .await?;
        }

        Op::PairBegin => {
            let req: PairBeginRequest = decode(payload)?;
            // The host records the attempt and displays it, rather than needing
            // a window opened in advance. That is what lets it show *which*
            // machine is asking, beside the number to read across.
            let hello = session.hello.as_ref();
            let name = match req.device_name.trim() {
                "" => hello.map_or("", |h| h.device_name.as_str()),
                name => name,
            };
            let device_id = match req.device_id.trim() {
                "" => hello.map_or("", |h| h.device_id.as_str()),
                id => id,
            };
            let request = {
                let mut registry = host.registry.lock().expect("registry lock");
                registry.begin_pairing_for(Instant::now(), name, device_id, &req.client_nonce)?
            };
            reply(
                stream,
                &PairBeginResponse {
                    server_nonce: request.server_nonce.clone(),
                    request: request.id.clone(),
                    requires_pin: request.pin.is_some(),
                },
            )
            .await?;
        }

        Op::PairFinish => {
            let req: PairFinishRequest = decode(payload)?;
            let token = {
                let mut registry = host.registry.lock().expect("registry lock");
                // The host id comes from this host's own identity. Taking it
                // from the request would hand an attacker the one value the
                // proof is supposed to bind.
                registry.finish_pairing(
                    Instant::now(),
                    host.host_id(),
                    &req.request,
                    req.proof.as_deref(),
                )?
            };
            host.persist()?;

            // Pairing also authenticates this connection, so the client can go
            // straight on to browsing without a second round trip.
            session.device = {
                let mut registry = host.registry.lock().expect("registry lock");
                registry.authenticate(&token)
            };

            reply(
                stream,
                &PairFinishResponse {
                    token,
                    vault: host.vault_name(),
                },
            )
            .await?;
        }

        Op::Auth => {
            let req: AuthRequest = decode(payload)?;
            let (device, changed) = {
                let mut registry = host.registry.lock().expect("registry lock");
                match registry.authenticate(&req.token) {
                    Some(device) => {
                        // Whatever the device says about itself now — an id
                        // it did not have when it paired, a new name — is
                        // taken in on every connection.
                        let changed = session.hello.as_ref().is_some_and(|hello| {
                            registry.observe(
                                &device.token_hash,
                                &hello.device_id,
                                &hello.device_name,
                            )
                        });
                        let current = registry.device(&device.token_hash).cloned();
                        (current, changed)
                    }
                    None => (None, false),
                }
            };
            let Some(device) = device else {
                return Err(HostError::Unauthenticated);
            };
            if changed {
                host.persist()?;
            }
            let response = AuthResponse {
                vault: host.vault_name(),
                device_name: device.name.clone(),
                writable: device.writable,
            };
            session.device = Some(device);
            reply(stream, &response).await?;
        }

        Op::List => {
            let req: ListRequest = decode(payload)?;
            let vault = host.require_vault().await?;
            let path = req.path.clone();
            let mut entries = tokio::task::spawn_blocking(move || vault.list(&path))
                .await
                .map_err(join)??;
            // Partial uploads are an implementation detail. Showing them makes
            // a transfer in progress look like corruption on the drive.
            entries.retain(|e| !is_temp_name(&e.name));
            reply(stream, &ListResponse { entries }).await?;
        }

        Op::Stat => {
            let req: StatRequest = decode(payload)?;
            let vault = host.require_vault().await?;
            let path = req.path.clone();
            let entry = tokio::task::spawn_blocking(move || vault.stat(&path))
                .await
                .map_err(join)??;
            reply(stream, &StatResponse { entry }).await?;
        }

        Op::Space => {
            let vault = host.require_vault().await?;
            let (free, total) = tokio::task::spawn_blocking(move || vault.space())
                .await
                .map_err(join)?;
            reply(stream, &SpaceResponse { free, total }).await?;
        }

        Op::Read => {
            let req: ReadRequest = decode(payload)?;
            if req.length > MAX_READ_BYTES {
                return Err(HostError::BadRequest(format!(
                    "a single read is limited to {MAX_READ_BYTES} bytes"
                )));
            }
            let vault = host.require_vault().await?;
            let data = tokio::task::spawn_blocking(move || {
                vault.read_range(&req.path, req.offset, req.length)
            })
            .await
            .map_err(join)??;

            // Raw bytes, no envelope: this is the path a video player seeks
            // through and every byte of overhead is paid thousands of times.
            write_response_header(stream, STATUS_OK, data.len() as u64).await?;
            stream.write_all(&data).await?;
            stream.flush().await?;
            if let Some(key) = session.device_key() {
                host.traffic.sent(&key, data.len() as u64);
            }
        }

        Op::ReadBatch => {
            let req: BatchRequest = decode(payload)?;
            let vault = host.require_vault().await?;
            let body = tokio::task::spawn_blocking(move || build_batch(&vault, req))
                .await
                .map_err(join)??;
            write_response_header(stream, STATUS_OK, body.len() as u64).await?;
            stream.write_all(&body).await?;
            stream.flush().await?;
            if let Some(key) = session.device_key() {
                host.traffic.sent(&key, body.len() as u64);
            }
        }

        Op::WriteBegin => {
            let req: WriteBeginRequest = decode(payload)?;
            require_write(session)?;
            let vault = host.require_vault().await?;
            let (id, offset) = host
                .uploads
                .begin(
                    &vault,
                    &req.path,
                    req.size,
                    req.overwrite,
                    req.resume.as_deref(),
                )
                .await?;
            reply(
                stream,
                &WriteBeginResponse {
                    upload: basalt_proto::hex::encode(&id),
                    offset,
                },
            )
            .await?;
        }

        Op::WriteChunk => {
            require_write(session)?;
            let (id, offset, data) = decode_chunk(payload)?;
            host.uploads.write_chunk(&id, offset, data).await?;
            if let Some(key) = session.device_key() {
                host.traffic.received(&key, data.len() as u64);
            }
            write_ok(stream, &[]).await?;
        }

        Op::WriteCommit => {
            let req: WriteCommitRequest = decode(payload)?;
            require_write(session)?;
            let id = parse_upload_id(&req.upload)?;
            host.uploads.commit(&id, &req.blake3, req.mtime).await?;
            write_ok(stream, &[]).await?;
        }

        Op::WriteAbort => {
            let req: WriteAbortRequest = decode(payload)?;
            require_write(session)?;
            let id = parse_upload_id(&req.upload)?;
            host.uploads.abort(&id).await?;
            write_ok(stream, &[]).await?;
        }

        Op::Mkdir => {
            let req: MkdirRequest = decode(payload)?;
            require_write(session)?;
            let vault = host.require_vault().await?;
            tokio::task::spawn_blocking(move || vault.mkdir(&req.path))
                .await
                .map_err(join)??;
            write_ok(stream, &[]).await?;
        }

        Op::Rename => {
            let req: RenameRequest = decode(payload)?;
            require_write(session)?;
            let vault = host.require_vault().await?;
            tokio::task::spawn_blocking(move || vault.rename(&req.from, &req.to))
                .await
                .map_err(join)??;
            write_ok(stream, &[]).await?;
        }

        Op::Copy => {
            let req: CopyRequest = decode(payload)?;
            require_write(session)?;
            let vault = host.require_vault().await?;
            tokio::task::spawn_blocking(move || vault.copy(&req.from, &req.to))
                .await
                .map_err(join)??;
            write_ok(stream, &[]).await?;
        }

        Op::Remove => {
            let req: RemoveRequest = decode(payload)?;
            require_write(session)?;
            let vault = host.require_vault().await?;
            tokio::task::spawn_blocking(move || vault.remove(&req.path, req.recursive))
                .await
                .map_err(join)??;
            write_ok(stream, &[]).await?;
        }

        // The one op that does not answer once. This borrows the connection
        // for as long as the client wants it and writes a response per change.
        Op::Watch => {
            let _: WatchRequest = decode(payload).unwrap_or_default();
            stream_changes(stream, host).await?;
        }

        Op::Library => {
            let req: LibraryRequest = decode(payload).unwrap_or_default();
            reply(stream, &host.library_response(req.known_revision)).await?;
        }

        Op::Progress => {
            let request: ProgressRequest = decode(payload).unwrap_or_default();
            host.check_profile(session)?;
            reply(stream, &host.progress(request, session.owner().as_deref())).await?;
        }

        Op::Unpair => {
            let Some(device) = session.device.take() else {
                return Err(HostError::Unauthenticated);
            };
            host.revoke(&device.token_hash)?;
            write_ok(stream, &[]).await?;
        }

        Op::Profiles => {
            reply(
                stream,
                &ProfilesResponse {
                    profiles: host.profile_views(),
                },
            )
            .await?;
        }

        Op::ProfileCreate | Op::ProfileSignIn => {
            let device_key = session
                .device
                .as_ref()
                .map(|d| d.key().to_string())
                .ok_or(HostError::Unauthenticated)?;
            let (profile, token) = if op == Op::ProfileCreate {
                host.create_profile(decode(payload)?, &device_key)?
            } else {
                host.sign_in_profile(decode(payload)?, &device_key)?
            };
            session.profile = Some((profile.id.clone(), crate::profiles::hash_token(&token)));
            reply(stream, &ProfileSession { profile, token }).await?;
        }

        Op::ProfileUse => {
            let req: ProfileUseRequest = decode(payload).unwrap_or_default();
            let (profile, hash) = match req.token {
                None => (None, None),
                Some(token) => (
                    Some(host.resolve_profile(&token).ok_or(HostError::SignedOut)?),
                    Some(crate::profiles::hash_token(&token)),
                ),
            };
            session.profile = profile.as_ref().zip(hash).map(|(p, h)| (p.id.clone(), h));
            reply(stream, &ProfileUseResponse { profile }).await?;
        }

        Op::ProfileSignOut => {
            let req: ProfileSignOutRequest = decode(payload)?;
            host.sign_out_profile(&req.token)?;
            session.profile = None;
            write_ok(stream, &[]).await?;
        }

        Op::Stars => {
            let req: StarsRequest = decode(payload).unwrap_or_default();
            // A device on its own keeps its stars itself, as it always has.
            host.check_profile(session)?;
            let profile = session.profile.clone().map(|(id, _)| id).ok_or_else(|| {
                HostError::Denied("stars are kept on the host for profiles only".into())
            })?;
            reply(
                stream,
                &StarsResponse {
                    stars: host.stars_of(&profile, req.set),
                },
            )
            .await?;
        }

        Op::Collections => {
            let req: basalt_proto::msg::CollectionsRequest = decode(payload).unwrap_or_default();
            reply(stream, &host.collections_response(req.known_revision)).await?;
        }

        Op::Thumbnail => {
            let req: basalt_proto::msg::ThumbnailRequest = decode(payload)?;
            let bytes = host.thumbnail(&req.path, req.size).await?;
            write_ok(stream, &bytes).await?;
        }

        Op::LibraryArt => {
            let req: ArtRequest = decode(payload)?;
            match host.art(&req.id) {
                Some(bytes) => write_ok(stream, &bytes).await?,
                None => return Err(HostError::NotFound(format!("artwork for {}", req.id))),
            }
        }
    }
    Ok(())
}

/// Writes one response per change until the client hangs up.
///
/// A lagging subscriber is told to reload rather than quietly skipped. The
/// alternative — dropping the changes it missed — leaves that client showing a
/// listing the drive stopped agreeing with, which is the single failure this
/// whole mechanism exists to prevent.
async fn stream_changes<S>(stream: &mut S, host: &Arc<Host>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut events = match host.watch().await {
        Some(watch) => watch.subscribe(),
        None => {
            return Err(HostError::Denied(
                "this host has not been given a drive to share yet".into(),
            ));
        }
    };

    loop {
        let change = match events.recv().await {
            Ok(change) => change,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                tracing::debug!("a watcher fell {missed} changes behind");
                basalt_proto::msg::Change::Resynchronise
            }
            // The watcher stopped, which means the vault was replaced.
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Ok(());
            }
        };

        // A write failure here is the client hanging up, which is how a watch
        // is meant to end.
        if reply(stream, &WatchEvent { change }).await.is_err() {
            return Ok(());
        }
    }
}

fn require_write(session: &Session) -> Result<()> {
    if session.writable() {
        Ok(())
    } else {
        Err(HostError::Denied(
            "this device is paired read-only".to_string(),
        ))
    }
}

fn decode<T: serde::de::DeserializeOwned>(payload: &[u8]) -> Result<T> {
    serde_json::from_slice(payload)
        .map_err(|e| HostError::BadRequest(format!("request did not parse: {e}")))
}

async fn reply<S, T>(stream: &mut S, value: &T) -> Result<()>
where
    S: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    let body = serde_json::to_vec(value)
        .map_err(|e| HostError::BadRequest(format!("could not encode the reply: {e}")))?;
    write_ok(stream, &body).await?;
    Ok(())
}

fn join(e: tokio::task::JoinError) -> HostError {
    HostError::BadRequest(format!("a background task failed: {e}"))
}

/// Builds one batch stream covering a whole manifest.
///
/// The two things that make this worth 7.6x over per-file requests:
///
/// 1. The manifest is sorted before anything is read, so a spinning disk does
///    one broadly sequential sweep instead of thousands of independent seeks.
///    Measured 2.71x on the real USB drive.
/// 2. The whole body goes through one zstd context, so the dictionary carries
///    across every file. Measured 2.23x on a real corpus.
fn build_batch(vault: &Vault, req: BatchRequest) -> Result<Vec<u8>> {
    let policy = CompressionPolicy::default();
    let codec = if req.accept_compression {
        // Clamped at 9 because level 9 on the host measured 19.6 MB/s — below
        // the link — so a client asking for more would slow itself down.
        Codec::Zstd(req.preferred_level.unwrap_or(policy.level).clamp(1, 9))
    } else {
        Codec::Raw
    };

    let mut paths = req.paths;
    paths.sort();

    let mut out = Vec::with_capacity(4 * 1024 * 1024);
    let mut writer = BatchWriter::new(&mut out, codec)?;

    for (index, rel) in paths.iter().enumerate() {
        // Every path written into the stream has to be valid, including the
        // ones attached to errors. Writing a caller's raw path into an error
        // entry means the *decoder* rejects it, which kills the entire batch —
        // the opposite of what inline errors are for. A path that will not
        // sanitise gets a placeholder keyed by its position, and the original
        // goes in the message where it is inert.
        let label = sanitize_relative_path(rel).unwrap_or_else(|_| format!("!rejected/{index:06}"));

        match vault.resolve(rel) {
            Ok(path) => match std::fs::read(&path) {
                Ok(data) => {
                    let mtime = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    writer.write_file(&label, mtime, &data)?;
                }
                // One unreadable file must not abort a batch of thousands.
                Err(e) => writer.write_error(&label, &e.to_string())?,
            },
            Err(e) => writer.write_error(&label, &e.to_string())?,
        }
    }

    writer.finish()?;
    Ok(out)
}
