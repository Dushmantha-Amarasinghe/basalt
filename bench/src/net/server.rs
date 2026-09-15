//! Benchmark server. Runs on the laptop, over the drive under test.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use basalt_proto::codec::{Codec, CompressionPolicy};
use basalt_proto::frame::{BatchWriter, sanitize_relative_path};
use basalt_proto::manifest::BatchRequest;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::{CHUNK_BYTES, Op, STATUS_ERR, STATUS_OK, drain, read_request, write_response_header};
use crate::corpus::{Flavour, Rng, generate};

pub struct ServerConfig {
    pub root: PathBuf,
    pub addr: SocketAddr,
    /// Second listener offering the same protocol wrapped in TLS 1.3, so the
    /// client can measure the crypto cost on this machine directly.
    pub tls_addr: SocketAddr,
}

/// A server whose sockets are already bound but which is not yet accepting.
///
/// Splitting bind from serve exists for the integration tests: they pass port 0
/// and read back whichever ports the OS handed out. Probing for a free port and
/// then binding it separately is a race, and under `cargo test`'s parallelism
/// that race loses often enough to make the suite flaky.
pub struct BoundServer {
    plain: TcpListener,
    tls_listener: TcpListener,
    acceptor: TlsAcceptor,
    root: Arc<PathBuf>,
    plain_addr: SocketAddr,
    tls_addr: SocketAddr,
}

impl BoundServer {
    pub fn plain_addr(&self) -> SocketAddr {
        self.plain_addr
    }

    pub fn tls_addr(&self) -> SocketAddr {
        self.tls_addr
    }

    pub fn print_banner(&self) {
        println!("basalt-bench server");
        println!("  root      {}", self.root.display());
        println!("  plaintext {}", self.plain_addr);
        println!("  tls       {}", self.tls_addr);
        for ip in local_addresses() {
            println!("  reachable at {ip}");
        }
        println!("\nrun the client with:  basalt-bench net --host <this machine's IP>");
        println!("ctrl-c to stop\n");
    }
}

/// Binds both listeners and prepares TLS, without accepting yet.
pub async fn bind(config: ServerConfig) -> Result<BoundServer> {
    let root = config
        .root
        .canonicalize()
        .with_context(|| format!("resolving root {}", config.root.display()))?;

    let plain = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("binding {}", config.addr))?;
    let tls_listener = TcpListener::bind(config.tls_addr)
        .await
        .with_context(|| format!("binding {}", config.tls_addr))?;

    let plain_addr = plain.local_addr()?;
    let tls_addr = tls_listener.local_addr()?;

    Ok(BoundServer {
        plain,
        tls_listener,
        acceptor: build_tls_acceptor()?,
        root: Arc::new(root),
        plain_addr,
        tls_addr,
    })
}

pub async fn run(config: ServerConfig) -> Result<()> {
    let server = bind(config).await?;
    server.print_banner();
    serve(server).await
}

/// Accepts connections until cancelled.
pub async fn serve(server: BoundServer) -> Result<()> {
    let BoundServer {
        plain,
        tls_listener,
        acceptor,
        root,
        ..
    } = server;

    loop {
        tokio::select! {
            accepted = plain.accept() => {
                let (stream, peer) = accepted?;
                let root = Arc::clone(&root);
                tokio::spawn(async move {
                    tune_socket(&stream);
                    if let Err(e) = serve_connection(stream, root).await {
                        tracing::debug!("plaintext {peer}: {e}");
                    }
                });
            }
            accepted = tls_listener.accept() => {
                let (stream, peer) = accepted?;
                let root = Arc::clone(&root);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    tune_socket(&stream);
                    match acceptor.accept(stream).await {
                        Ok(tls) => {
                            if let Err(e) = serve_connection(tls, root).await {
                                tracing::debug!("tls {peer}: {e}");
                            }
                        }
                        Err(e) => tracing::debug!("tls handshake {peer}: {e}"),
                    }
                });
            }
        }
    }
}

/// Socket tuning applied to every connection.
///
/// `TCP_NODELAY` is on because the small-file and ping paths are latency-bound
/// and Nagle would add up to 40 ms per exchange. Bulk transfers write in large
/// chunks, so they never form the small segments Nagle exists to coalesce.
fn tune_socket(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
}

async fn serve_connection<S>(mut stream: S, root: Arc<PathBuf>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let (op, payload) = match read_request(&mut stream).await {
            Ok(v) => v,
            // Clean disconnect between requests.
            Err(_) => return Ok(()),
        };

        match op {
            Op::Ping => {
                write_response_header(&mut stream, STATUS_OK, 0).await?;
                stream.flush().await?;
            }
            Op::Source => {
                let n = payload_u64(&payload)?;
                write_response_header(&mut stream, STATUS_OK, n).await?;
                send_synthetic(&mut stream, n).await?;
                stream.flush().await?;
            }
            Op::Sink => {
                let n = payload_u64(&payload)?;
                drain(&mut stream, n).await?;
                write_response_header(&mut stream, STATUS_OK, 0).await?;
                stream.flush().await?;
            }
            Op::GetFile => {
                let rel = std::str::from_utf8(&payload).context("path was not utf-8")?;
                match resolve(&root, rel) {
                    Ok(path) => send_file(&mut stream, &path).await?,
                    Err(e) => send_error(&mut stream, &e.to_string()).await?,
                }
            }
            Op::GetBatch => {
                let req: BatchRequest =
                    serde_json::from_slice(&payload).context("bad batch manifest")?;
                send_batch(&mut stream, &root, req).await?;
            }
        }
    }
}

fn payload_u64(payload: &[u8]) -> Result<u64> {
    if payload.len() != 8 {
        bail!("expected an 8-byte payload, got {}", payload.len());
    }
    Ok(u64::from_le_bytes(
        payload.try_into().expect("checked length"),
    ))
}

/// Resolves a wire path under the root, refusing anything that escapes it.
///
/// Two independent checks, because either alone has a gap: the protocol-level
/// validator rejects traversal syntax, and the canonicalised prefix check
/// catches symlinks and junctions that only reveal themselves after resolution.
fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let safe = sanitize_relative_path(rel)?;
    let joined = root.join(&safe);
    let canonical = joined
        .canonicalize()
        .with_context(|| format!("{safe} not found"))?;
    if !canonical.starts_with(root) {
        bail!("{safe} resolves outside the share root");
    }
    Ok(canonical)
}

async fn send_error<S: AsyncWrite + Unpin>(stream: &mut S, msg: &str) -> Result<()> {
    let bytes = msg.as_bytes();
    write_response_header(stream, STATUS_ERR, bytes.len() as u64).await?;
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn send_file<S: AsyncWrite + Unpin>(stream: &mut S, path: &Path) -> Result<()> {
    let data = match tokio::fs::read(path).await {
        Ok(d) => d,
        Err(e) => return send_error(stream, &e.to_string()).await,
    };
    write_response_header(stream, STATUS_OK, data.len() as u64).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}

/// Streams synthetic bytes with no disk involvement, so [`Op::Source`] measures
/// the link and the crypto in isolation.
async fn send_synthetic<S: AsyncWrite + Unpin>(stream: &mut S, total: u64) -> Result<()> {
    let mut rng = Rng::new(0xF00D);
    let mut chunk = Vec::new();
    // Incompressible, so a compressing TLS layer or NIC cannot flatter the
    // result by shrinking the payload.
    generate(Flavour::Binary, CHUNK_BYTES, &mut rng, &mut chunk);

    let mut sent = 0u64;
    while sent < total {
        let take = chunk.len().min((total - sent) as usize);
        stream.write_all(&chunk[..take]).await?;
        sent += take as u64;
    }
    Ok(())
}

/// Serves a whole manifest as one batch stream.
///
/// Two things happen here that are the entire point of the batch path:
///
/// 1. Paths are sorted before reading, so a spinning disk performs one broadly
///    sequential sweep instead of thousands of independent seeks.
/// 2. The whole body goes through a single zstd context, so the dictionary is
///    shared across every file in the batch.
///
/// The framing runs on a blocking thread because zstd is CPU-bound and would
/// otherwise stall the async runtime.
async fn send_batch<S: AsyncWrite + Unpin>(
    stream: &mut S,
    root: &Path,
    req: BatchRequest,
) -> Result<()> {
    let policy = CompressionPolicy::default();
    let codec = if req.accept_compression {
        Codec::Zstd(req.preferred_level.unwrap_or(policy.level).clamp(1, 9))
    } else {
        Codec::Raw
    };

    let root = root.to_path_buf();
    let body = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        // Sorting is what turns random seeks into a sweep. Directory order
        // correlates strongly with on-disk layout for a freshly written tree.
        let mut paths = req.paths;
        paths.sort();

        let mut out = Vec::with_capacity(8 * 1024 * 1024);
        let mut w = BatchWriter::new(&mut out, codec)?;
        for (index, rel) in paths.iter().enumerate() {
            // Every path written into the stream must itself be valid, including
            // the ones attached to errors. Writing the caller's raw path into an
            // error entry means the *decoder* rejects it, which kills the whole
            // batch — the exact opposite of what inline errors are for. So a
            // path we cannot sanitise gets a safe placeholder keyed by its index
            // in the request, and the original goes in the message where it is
            // inert.
            let label =
                sanitize_relative_path(rel).unwrap_or_else(|_| format!("!rejected/{index:06}"));

            match resolve(&root, rel) {
                Ok(path) => match std::fs::read(&path) {
                    Ok(data) => {
                        let mtime = std::fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        w.write_file(&label, mtime, &data)?;
                    }
                    // One unreadable file must not abort a batch of thousands.
                    Err(e) => w.write_error(&label, &e.to_string())?,
                },
                Err(e) => w.write_error(&label, &e.to_string())?,
            }
        }
        w.finish()?;
        Ok(out)
    })
    .await??;

    write_response_header(stream, STATUS_OK, body.len() as u64).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

/// Self-signed certificate, generated fresh on every start.
///
/// Benchmark-only. The shipping design pins the SPKI hash at pairing time and
/// binds it to a PIN-derived HMAC challenge; none of that affects throughput,
/// which is all this harness is measuring.
fn build_tls_acceptor() -> Result<TlsAcceptor> {
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "basalt-bench".to_string(),
    ])?;

    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.signing_key.serialize_der())
        .map_err(|e| anyhow::anyhow!("serialising private key: {e}"))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Best-effort list of this machine's LAN addresses, so the user does not have
/// to go hunting in `ipconfig`.
fn local_addresses() -> Vec<String> {
    let Ok(out) = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-NetIPAddress -AddressFamily IPv4 | \
             Where-Object { $_.IPAddress -notlike '127.*' -and \
             $_.IPAddress -notlike '169.254.*' }).IPAddress",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("basalt-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("ok.txt"), b"hello").unwrap();
        std::fs::write(dir.join("sub").join("nested.txt"), b"nested").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn resolve_accepts_paths_inside_the_root() {
        let root = temp_root();
        assert!(resolve(&root, "ok.txt").is_ok());
        assert!(resolve(&root, "sub/nested.txt").is_ok());
        assert!(
            resolve(&root, "sub\\nested.txt").is_ok(),
            "backslashes normalise"
        );
    }

    #[test]
    fn resolve_rejects_traversal_out_of_the_root() {
        let root = temp_root();
        for bad in [
            "../outside.txt",
            "sub/../../escape",
            "/etc/passwd",
            "C:/Windows/win.ini",
        ] {
            assert!(
                resolve(&root, bad).is_err(),
                "{bad} must not resolve inside the share"
            );
        }
    }

    #[test]
    fn resolve_rejects_missing_files() {
        let root = temp_root();
        assert!(resolve(&root, "does-not-exist.txt").is_err());
    }

    #[test]
    fn payload_u64_requires_exactly_eight_bytes() {
        assert_eq!(payload_u64(&1234u64.to_le_bytes()).unwrap(), 1234);
        assert!(payload_u64(&[0u8; 4]).is_err());
        assert!(payload_u64(&[0u8; 9]).is_err());
    }

    #[test]
    fn tls_acceptor_builds() {
        assert!(build_tls_acceptor().is_ok());
    }
}
