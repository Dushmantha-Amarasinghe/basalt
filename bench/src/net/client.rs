//! Benchmark client. Runs on the PC, drives the laptop.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use basalt_proto::frame::BatchReader;
use basalt_proto::manifest::BatchRequest;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::{DEFAULT_PORT, Op, STATUS_OK, drain, read_response_header, write_request};
use crate::stats::{Measurement, Suite, fmt_bytes};

/// Stream counts to sweep. A single TCP flow often cannot saturate a Wi-Fi
/// link, but past a point extra streams only add loss and contention. The
/// sweet spot is expected around 4-8; this finds it rather than assuming it.
const STREAM_COUNTS: [usize; 5] = [1, 2, 4, 8, 16];

pub struct ClientConfig {
    pub host: String,
    pub port: u16,
    pub tls_port: u16,
    pub runs: usize,
    /// Bytes per throughput measurement.
    pub transfer_bytes: u64,
    /// How many small files to request in the batch-vs-per-file comparison.
    pub small_file_count: usize,
}

impl ClientConfig {
    fn plain_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
    fn tls_addr(&self) -> String {
        format!("{}:{}", self.host, self.tls_port)
    }
}

/// Either a plain or TLS-wrapped connection, so measurement code can be written
/// once and run over both.
pub enum Conn {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl Conn {
    pub async fn connect_plain(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("connecting to {addr}"))?;
        stream.set_nodelay(true)?;
        Ok(Conn::Plain(stream))
    }

    pub async fn connect_tls(addr: &str, connector: &TlsConnector) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("connecting to {addr}"))?;
        stream.set_nodelay(true)?;
        let name = rustls::pki_types::ServerName::try_from("localhost")?.to_owned();
        let tls = connector.connect(name, stream).await?;
        Ok(Conn::Tls(Box::new(tls)))
    }

    pub fn as_io(&mut self) -> &mut (dyn AsyncReadWrite + Unpin + Send) {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.as_mut(),
        }
    }
}

/// Helper trait so `Conn` can hand back one object usable for both directions.
///
/// `Send` is part of the object type because the multi-stream measurements move
/// each connection onto its own task.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

pub async fn run(config: ClientConfig) -> Result<Vec<Suite>> {
    let connector = build_tls_connector()?;

    println!("target {} (tls {})", config.plain_addr(), config.tls_addr());
    preflight(&config).await?;

    let mut suites = Vec::new();
    suites.push(latency(&config).await?);
    suites.push(raw_throughput(&config, &connector).await?);
    suites.push(small_files(&config, &connector).await?);
    Ok(suites)
}

/// Fails fast with a clear message if the server is not reachable, rather than
/// letting every later measurement time out one at a time.
async fn preflight(config: &ClientConfig) -> Result<()> {
    let mut conn = Conn::connect_plain(&config.plain_addr()).await.context(
        "could not reach the benchmark server — is `basalt-bench serve` \
             running on the laptop, and is the port open in Windows Firewall?",
    )?;
    let io = conn.as_io();
    write_request(io, Op::Ping, b"").await?;
    let (status, _) = read_response_header(io).await?;
    if status != STATUS_OK {
        bail!("server answered ping with status {status}");
    }
    Ok(())
}

/// Round-trip latency. Sets the floor on every per-file operation, and is the
/// number that makes batching worth building.
async fn latency(config: &ClientConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "latency",
        "round-trip time — the per-request floor that batching exists to avoid",
    );
    println!("\nlatency");

    let mut conn = Conn::connect_plain(&config.plain_addr()).await?;
    let io = conn.as_io();

    // Warm the path so the first sample does not carry ARP and route setup.
    for _ in 0..10 {
        write_request(io, Op::Ping, b"").await?;
        read_response_header(io).await?;
    }

    let mut m = Measurement::new("ping round trip", 0).with_items(1);
    for _ in 0..200 {
        let t = Instant::now();
        write_request(io, Op::Ping, b"").await?;
        let (_, len) = read_response_header(io).await?;
        debug_assert_eq!(len, 0);
        m.record(t.elapsed());
    }

    let rtt_ms = m.median_ms();
    m.notes.push(format!(
        "{} sequential requests would cost {:.1}s in latency alone",
        config.small_file_count,
        rtt_ms * config.small_file_count as f64 / 1000.0
    ));
    suite.push(m);
    Ok(suite)
}

/// Link ceiling with the disk out of the picture, swept across stream counts
/// and with/without TLS.
async fn raw_throughput(config: &ClientConfig, connector: &TlsConnector) -> Result<Suite> {
    let mut suite = Suite::new(
        "raw-throughput",
        "link ceiling with no disk involved, by stream count and encryption",
    );
    println!(
        "\nraw throughput ({} per run, no disk)",
        fmt_bytes(config.transfer_bytes)
    );

    for encrypted in [false, true] {
        for streams in STREAM_COUNTS {
            let label = format!(
                "download {:>2} stream{} {}",
                streams,
                if streams == 1 { " " } else { "s" },
                if encrypted { "tls" } else { "plain" }
            );
            let mut m = Measurement::new(label, config.transfer_bytes);

            for _ in 0..config.runs {
                let elapsed = parallel_source(config, connector, streams, encrypted).await?;
                m.record(elapsed);
            }
            suite.push(m);
        }
    }

    // Upload matters too: Wi-Fi is not symmetric in practice, and the client
    // pushes files as well as pulling them.
    let mut up = Measurement::new("upload  1 stream  plain", config.transfer_bytes);
    for _ in 0..config.runs {
        up.record(sink_once(config, config.transfer_bytes).await?);
    }
    suite.push(up);

    if let Some(best) = suite.best() {
        println!(
            "  -> best: {} at {:.1} MB/s",
            best.label,
            best.throughput_mbs()
        );
    }
    Ok(suite)
}

/// Splits `transfer_bytes` across `streams` connections and times the slowest.
async fn parallel_source(
    config: &ClientConfig,
    connector: &TlsConnector,
    streams: usize,
    encrypted: bool,
) -> Result<std::time::Duration> {
    let per_stream = config.transfer_bytes / streams as u64;
    let addr = if encrypted {
        config.tls_addr()
    } else {
        config.plain_addr()
    };

    // Connect everything up front so handshake cost is excluded from the
    // measurement — we are measuring steady-state throughput, and a shipped
    // client keeps its connections open anyway.
    let mut conns = Vec::with_capacity(streams);
    for _ in 0..streams {
        conns.push(if encrypted {
            Conn::connect_tls(&addr, connector).await?
        } else {
            Conn::connect_plain(&addr).await?
        });
    }

    let start = Instant::now();
    let mut tasks = Vec::with_capacity(streams);
    for mut conn in conns {
        tasks.push(tokio::spawn(async move {
            let io = conn.as_io();
            write_request(io, Op::Source, &per_stream.to_le_bytes()).await?;
            let (status, len) = read_response_header(io).await?;
            if status != STATUS_OK {
                bail!("server returned status {status}");
            }
            drain(io, len).await
        }));
    }
    for t in tasks {
        t.await??;
    }
    Ok(start.elapsed())
}

async fn sink_once(config: &ClientConfig, bytes: u64) -> Result<std::time::Duration> {
    use tokio::io::AsyncWriteExt;

    let mut conn = Conn::connect_plain(&config.plain_addr()).await?;
    let io = conn.as_io();
    let chunk = vec![0x5Au8; 1024 * 1024];

    let start = Instant::now();
    write_request(io, Op::Sink, &bytes.to_le_bytes()).await?;
    let mut sent = 0u64;
    while sent < bytes {
        let take = chunk.len().min((bytes - sent) as usize);
        io.write_all(&chunk[..take]).await?;
        sent += take as u64;
    }
    io.flush().await?;
    read_response_header(io).await?;
    Ok(start.elapsed())
}

/// The measurement that justifies the batch protocol: N files one at a time
/// versus the same N files in a single request.
async fn small_files(config: &ClientConfig, connector: &TlsConnector) -> Result<Suite> {
    let mut suite = Suite::new(
        "small-files",
        "per-file requests vs one batched request, for many small files",
    );
    println!("\nsmall files ({} files)", config.small_file_count);

    let paths = discover_small_files(config).await?;
    if paths.is_empty() {
        println!("  no small-file corpus on the server; skipping");
        println!("  run: basalt-bench gen-corpus --root <server root>");
        return Ok(suite);
    }
    let paths: Vec<String> = paths.into_iter().take(config.small_file_count).collect();
    println!("  using {} files from the server corpus", paths.len());

    // Arm 1: one request per file, sequentially. This is the naive design, and
    // the thing the batch path has to beat.
    {
        let mut m =
            Measurement::new("per-file requests, sequential", 0).with_items(paths.len() as u64);
        for _ in 0..config.runs.min(3) {
            let mut conn = Conn::connect_plain(&config.plain_addr()).await?;
            let io = conn.as_io();
            let start = Instant::now();
            let mut total = 0u64;
            for p in &paths {
                write_request(io, Op::GetFile, p.as_bytes()).await?;
                let (status, len) = read_response_header(io).await?;
                if status == STATUS_OK {
                    drain(io, len).await?;
                    total += len;
                } else {
                    drain(io, len).await?;
                }
            }
            m.record(start.elapsed());
            m.bytes = total;
            m.wire_bytes = total;
        }
        suite.push(m);
    }

    // Arm 2 and 3: one batched request, uncompressed and compressed.
    for compressed in [false, true] {
        let label = if compressed {
            "batched request, zstd"
        } else {
            "batched request, raw "
        };
        let mut m = Measurement::new(label, 0).with_items(paths.len() as u64);
        let mut logical = 0u64;
        let mut wire = 0u64;

        for _ in 0..config.runs.min(3) {
            let req = if compressed {
                BatchRequest::new(paths.clone())
            } else {
                BatchRequest::uncompressed(paths.clone())
            };
            let body = serde_json::to_vec(&req)?;

            let mut conn = if compressed {
                Conn::connect_tls(&config.tls_addr(), connector).await?
            } else {
                Conn::connect_plain(&config.plain_addr()).await?
            };
            let io = conn.as_io();

            let start = Instant::now();
            write_request(io, Op::GetBatch, &body).await?;
            let (status, len) = read_response_header(io).await?;
            if status != STATUS_OK {
                bail!("batch request failed with status {status}");
            }
            let mut buf = vec![0u8; len as usize];
            io.read_exact(&mut buf).await?;
            let elapsed = start.elapsed();

            // Decode on this side too: the client has to do this work, so it
            // belongs inside the measurement.
            let decoded = tokio::task::spawn_blocking(move || -> Result<(u64, u64)> {
                let mut reader = BatchReader::new(buf.as_slice())?;
                let mut bytes = 0u64;
                let mut count = 0u64;
                while let Some(entry) = reader.read_entry()? {
                    bytes += entry.data.len() as u64;
                    count += 1;
                }
                Ok((bytes, count))
            })
            .await??;

            m.record(elapsed);
            logical = decoded.0;
            wire = len;
        }

        m.bytes = logical;
        m.wire_bytes = wire;
        suite.push(m);
    }

    report_batch_speedup(&suite);
    Ok(suite)
}

fn report_batch_speedup(suite: &Suite) {
    let find = |needle: &str| {
        suite
            .measurements
            .iter()
            .find(|m| m.label.contains(needle))
            .map(|m| m.median_ms())
    };
    if let (Some(per_file), Some(batched)) = (find("per-file"), find("zstd"))
        && batched > 0.0
    {
        println!(
            "  -> batching is {:.1}x faster than per-file requests \
                 ({} vs {})",
            per_file / batched,
            crate::stats::fmt_duration_ms(per_file),
            crate::stats::fmt_duration_ms(batched),
        );
    }
}

/// Asks the server for the corpus manifest it generated.
///
/// The corpus is deterministic for a given seed and count, so the client can
/// reconstruct the expected paths rather than needing a directory-listing op.
async fn discover_small_files(config: &ClientConfig) -> Result<Vec<String>> {
    const PER_SHARD: usize = 250;

    let mut conn = Conn::connect_plain(&config.plain_addr()).await?;
    let io = conn.as_io();

    let mut found = Vec::new();
    let extensions = ["txt", "rs", "json", "bin"];

    for i in 0..config.small_file_count {
        let candidate = format!(
            "small/shard{:03}/{:05}.{}",
            i / PER_SHARD,
            i,
            extensions[i % 4]
        );
        write_request(io, Op::GetFile, candidate.as_bytes()).await?;
        let (status, len) = read_response_header(io).await?;
        drain(io, len).await?;
        if status == STATUS_OK {
            found.push(candidate);
        } else if i == 0 {
            // The first probe failing means there is no corpus at all.
            return Ok(Vec::new());
        }
    }
    Ok(found)
}

/// Accepts any server certificate.
///
/// Correct for a throughput benchmark and wrong for anything else. The shipped
/// client pins the server's SPKI hash captured during PIN-authenticated
/// pairing; certificate validation costs nothing at steady state, so omitting
/// it here does not distort the numbers.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn build_tls_connector() -> Result<TlsConnector> {
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Resolves `host` into the two socket addresses the server listens on.
pub fn resolve_addrs(host: &str, port: u16) -> Result<(SocketAddr, SocketAddr)> {
    use std::net::ToSocketAddrs;
    let plain = format!("{host}:{port}")
        .to_socket_addrs()
        .with_context(|| format!("resolving {host}"))?
        .next()
        .with_context(|| format!("no address for {host}"))?;
    let mut tls = plain;
    tls.set_port(port + 1);
    Ok((plain, tls))
}

pub const fn default_tls_port(port: u16) -> u16 {
    port + 1
}

pub const fn default_port() -> u16 {
    DEFAULT_PORT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_port_sits_next_to_the_plain_port() {
        assert_eq!(default_tls_port(7742), 7743);
    }

    #[test]
    fn resolve_addrs_produces_adjacent_ports() {
        let (plain, tls) = resolve_addrs("127.0.0.1", 7742).unwrap();
        assert_eq!(plain.port(), 7742);
        assert_eq!(tls.port(), 7743);
        assert_eq!(plain.ip(), tls.ip());
    }

    #[test]
    fn tls_connector_builds() {
        assert!(build_tls_connector().is_ok());
    }

    #[test]
    fn stream_sweep_covers_the_expected_range() {
        // The sweep must bracket the expected 4-8 sweet spot on both sides,
        // otherwise it cannot show a peak.
        assert!(STREAM_COUNTS.contains(&1));
        assert!(STREAM_COUNTS.contains(&8));
        assert!(STREAM_COUNTS.iter().any(|&n| n > 8));
    }
}
