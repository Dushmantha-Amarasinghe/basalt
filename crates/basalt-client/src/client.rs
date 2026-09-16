//! The client API, as one object.
//!
//! Everything the interface needs and nothing it does not: pair, reconnect,
//! browse, transfer. Each call takes a connection from the pool and gives it
//! back, so a download and a folder listing never wait on each other.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use basalt_proto::msg::{DirEntry, HelloResponse};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::pool::Pool;
use crate::session::{Session, SessionInfo};
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
    device_name: String,
}

impl Basalt {
    pub fn open(store_path: PathBuf) -> Result<Self> {
        let store = ClientStore::load(&store_path)?;
        let device_name = store
            .device_name
            .clone()
            .unwrap_or_else(crate::store::device_name);
        Ok(Self {
            store_path,
            store: std::sync::Mutex::new(store),
            pool: tokio::sync::RwLock::new(None),
            info: std::sync::Mutex::new(None),
            device_name,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
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

    /// Looks at a host without pairing, so the user can confirm what they found.
    pub async fn probe(&self, address: &str) -> Result<HelloResponse> {
        let addr = basalt_net::socket::resolve(address, basalt_net::DEFAULT_PORT).await?;
        Session::probe(addr, &self.device_name).await
    }

    /// Pairs with a host and stays connected to it.
    pub async fn pair(&self, address: &str, pin: &str) -> Result<SessionInfo> {
        let addr = basalt_net::socket::resolve(address, basalt_net::DEFAULT_PORT).await?;
        let (session, token) = Session::pair(addr, pin, &self.device_name).await?;
        let info = session.info().clone();

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
            store.device_name = Some(self.device_name.clone());
        }
        self.save_store()?;

        // The connection pairing opened is already authenticated, so it goes
        // straight into the pool rather than being thrown away and redialled.
        let pool = Pool::with_session(addr, &info.host_id, &token, &self.device_name, session);
        *self.pool.write().await = Some(pool);
        *self.info.lock().expect("info lock") = Some(info.clone());
        Ok(info)
    }

    /// Reconnects to a host already paired with.
    ///
    /// `address` overrides the remembered one, for the case where the router
    /// has handed the host a different one.
    pub async fn connect(&self, host_id: &str, address: Option<&str>) -> Result<SessionInfo> {
        let known = self
            .store
            .lock()
            .expect("store lock")
            .find(host_id)
            .cloned()
            .ok_or(ClientError::NotConnected)?;

        let target = address
            .map(str::to_string)
            .or_else(|| known.last_address.clone())
            .ok_or(ClientError::HostNotFound)?;
        let addr = basalt_net::socket::resolve(&target, basalt_net::DEFAULT_PORT).await?;

        let session =
            Session::connect(addr, &known.host_id, &known.token, &self.device_name).await?;
        let info = session.info().clone();

        {
            let mut store = self.store.lock().expect("store lock");
            store.note_address(host_id, &addr.to_string());
        }
        // A failure to write the address cache must not fail the connection —
        // it is an optimisation, and the client works without it.
        let _ = self.save_store();

        let pool = Pool::with_session(
            addr,
            &known.host_id,
            &known.token,
            &self.device_name,
            session,
        );
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

    /// Unpairs from a host on this side. The host keeps its record until it is
    /// revoked there too.
    pub async fn forget(&self, host_id: &str) -> Result<()> {
        let current = self.status().map(|i| i.host_id);
        if current.as_deref() == Some(host_id) {
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

    pub async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;
        let result = lease.read_range(path, offset, length).await;
        lease.check(result)
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
        lease.check(result)
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

        let mut done = 0u64;
        while done < total {
            if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                drop(file);
                tokio::fs::remove_file(&temp).await.ok();
                return Err(ClientError::Protocol("cancelled".into()));
            }

            let want = CHUNK_BYTES.min(total - done);
            let result = lease.read_range_into(remote, done, want, &mut file).await;
            let got = lease.check(result)?;
            if got == 0 {
                return Err(ClientError::Protocol(format!(
                    "the host stopped sending {remote} at {done} of {total} bytes"
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

        // Hashed up front, off the runtime, so the host can verify the whole
        // file at commit rather than trusting a length.
        let path_for_hash = local.to_path_buf();
        let digest = tokio::task::spawn_blocking(move || hash_file(&path_for_hash))
            .await
            .map_err(|e| ClientError::Protocol(format!("hashing failed: {e}")))??;

        let pool = self.pool().await?;
        let mut lease = pool.acquire().await?;

        let begin = {
            let result = lease.write_begin(remote, total, overwrite, None).await;
            lease.check(result)?
        };

        let mut file = tokio::fs::File::open(local).await?;
        let mut sent = begin.offset;
        if sent > 0 {
            file.seek(std::io::SeekFrom::Start(sent)).await?;
        }

        let mut buf = vec![0u8; CHUNK_BYTES as usize];
        while sent < total {
            if cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                let _ = lease.write_abort(&begin.upload).await;
                return Err(ClientError::Protocol("cancelled".into()));
            }

            let want = (CHUNK_BYTES.min(total - sent)) as usize;
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                let _ = lease.write_abort(&begin.upload).await;
                return Err(ClientError::Protocol(format!(
                    "{} is shorter than it said it was",
                    local.display()
                )));
            }

            let result = lease.write_chunk(&begin.upload, sent, &buf[..n]).await;
            lease.check(result)?;
            sent += n as u64;

            if let Some(report) = &progress {
                report(Progress {
                    kind: TransferKind::Upload,
                    path: remote.to_string(),
                    transferred: sent,
                    total,
                });
            }
        }

        let result = lease.write_commit(&begin.upload, &digest, mtime).await;
        lease.check(result)?;
        Ok(total)
    }
}

/// BLAKE3 of a whole file, read in blocks so a film never lands in memory.
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
