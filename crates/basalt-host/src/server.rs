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
use basalt_net::pairing;
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
use crate::registry::{Device, Registry};
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
}

impl Host {
    pub fn new(config: HostConfig, config_path: std::path::PathBuf) -> Result<Arc<Self>> {
        let identity = config.identity()?;
        let registry = Registry::new(config.devices.clone());

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

    /// Opens a pairing window and returns the PIN to display.
    pub fn open_pairing(&self) -> Result<String> {
        self.registry
            .lock()
            .expect("registry lock")
            .open_pairing(Instant::now())
    }

    pub fn close_pairing(&self) {
        self.registry.lock().expect("registry lock").close_pairing();
    }

    pub fn pairing_open(&self) -> bool {
        self.registry
            .lock()
            .expect("registry lock")
            .pairing_open(Instant::now())
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
    client_nonce: Option<String>,
    server_nonce: Option<String>,
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

    loop {
        let (op, payload) = match read_request(&mut stream).await {
            Ok(v) => v,
            // A peer that goes away between requests is the normal end of a
            // pooled connection, not a failure.
            Err(_) => return Ok(()),
        };

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
            write_err(&mut stream, code, &e.to_string()).await?;
        }
    }
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
                    pairing_open: host.pairing_open(),
                    host_name: host.host_name(),
                },
            )
            .await?;
        }

        Op::PairBegin => {
            let req: PairBeginRequest = decode(payload)?;
            if !host.pairing_open() {
                return Err(HostError::PairingRefused(
                    "this host is not accepting new devices right now".into(),
                ));
            }
            let server_nonce = pairing::random_nonce()
                .map_err(|e| HostError::PairingRefused(format!("no randomness: {e}")))?;
            session.client_nonce = Some(req.client_nonce);
            session.server_nonce = Some(server_nonce.clone());
            reply(stream, &PairBeginResponse { server_nonce }).await?;
        }

        Op::PairFinish => {
            let req: PairFinishRequest = decode(payload)?;
            let (Some(client_nonce), Some(server_nonce)) =
                (session.client_nonce.clone(), session.server_nonce.clone())
            else {
                return Err(HostError::PairingRefused(
                    "pairing was not started on this connection".into(),
                ));
            };
            // Consume the nonces whatever happens, so one PairBegin buys
            // exactly one attempt rather than an unlimited number against the
            // same challenge.
            session.client_nonce = None;
            session.server_nonce = None;

            let token = {
                let mut registry = host.registry.lock().expect("registry lock");
                // The host id comes from this host's own identity. Taking it
                // from the request would hand an attacker the one value the
                // proof is supposed to bind.
                registry.finish_pairing(
                    Instant::now(),
                    host.host_id(),
                    &client_nonce,
                    &server_nonce,
                    &req.proof,
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
