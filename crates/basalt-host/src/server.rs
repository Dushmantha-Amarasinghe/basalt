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

use basalt_net::framing::{read_request, write_err, write_ok, write_response_header};
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
}

impl Host {
    pub fn new(config: HostConfig, config_path: std::path::PathBuf) -> Result<Arc<Self>> {
        let identity = config.identity()?;
        let registry = Registry::new(config.devices.clone(), config.require_pin);

        let vault = match &config.vault_path {
            Some(path) => Some(Arc::new(Vault::open(path, &config.vault_name)?)),
            None => None,
        };

        Ok(Arc::new(Self {
            identity,
            config_path,
            config: std::sync::Mutex::new(config),
            vault: tokio::sync::RwLock::new(vault),
            registry: std::sync::Mutex::new(registry),
            uploads: Uploads::default(),
            traffic: Traffic::default(),
        }))
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
    pub async fn set_vault(&self, path: &std::path::Path, name: &str) -> Result<()> {
        let vault = Arc::new(Vault::open(path, name)?);
        *self.vault.write().await = Some(vault);
        {
            let mut config = self.config.lock().expect("config lock");
            config.vault_path = Some(path.to_path_buf());
            config.vault_name = name.to_string();
        }
        self.persist()
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
        let vault = self.vault().await.map(|vault| {
            let (free, total) = vault.space();
            crate::ui::VaultView {
                path: vault.root().to_string_lossy().into_owned(),
                name: vault.name().to_string(),
                free,
                total,
                available: crate::drives::is_available(vault.root()),
            }
        });

        let (host_name, port, require_pin) = {
            let config = self.config.lock().expect("config lock");
            (config.host_name.clone(), config.port, config.require_pin)
        };

        crate::ui::HostStatus {
            host_id: self.identity.host_id.clone(),
            host_name,
            port,
            require_pin,
            start_with_windows: self.start_with_windows(),
            vault,
            addresses: basalt_net::discovery::local_addresses()
                .into_iter()
                .map(|ip| ip.to_string())
                .collect(),
            device_count: self.registry.lock().expect("registry lock").device_count(),
            serving,
            problem: None,
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
        let removed = self
            .registry
            .lock()
            .expect("registry lock")
            .revoke(token_hash);
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
            let mut config = self.config.lock().expect("config lock");
            config.devices = devices;
            config.clone()
        };
        snapshot.save(&self.config_path)
    }

    async fn require_vault(&self) -> Result<Arc<Vault>> {
        self.vault().await.ok_or_else(|| {
            HostError::Denied("this host has not been given a drive to share yet".into())
        })
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
}

impl Session {
    /// The token hash this connection authenticated with, for accounting.
    fn device_key(&self) -> Option<String> {
        self.device.as_ref().map(|d| d.token_hash.clone())
    }
}

impl Session {
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
        let (op, payload) = match read_request(&mut stream).await {
            Ok(v) => v,
            // A peer that goes away between requests is the normal end of a
            // pooled connection, not a failure.
            Err(_) => break Ok(()),
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
            let _req: HelloRequest = decode(payload)?;
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
            let request = {
                let mut registry = host.registry.lock().expect("registry lock");
                registry.begin_pairing(Instant::now(), &req.device_name, &req.client_nonce)?
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
                    &req.device_name,
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
            let device = {
                let mut registry = host.registry.lock().expect("registry lock");
                registry.authenticate(&req.token)
            };
            let Some(device) = device else {
                return Err(HostError::Unauthenticated);
            };
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
    }
    Ok(())
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
