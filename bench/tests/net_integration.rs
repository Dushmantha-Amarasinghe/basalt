//! End-to-end network tests.
//!
//! These drive the *real* server and the *real* client over real TCP and real
//! TLS on loopback. Nothing is mocked. A test that exercises a reimplementation
//! of the protocol proves only that the reimplementation works.
//!
//! What they are checking for is correctness, not speed — loopback says nothing
//! useful about throughput. Specifically: bytes arrive intact, errors stay
//! contained, and a hostile path never escapes the share root.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use basalt_bench::net::client::{Conn, build_tls_connector};
use basalt_bench::net::server::{ServerConfig, bind as bind_server, serve as serve_server};
use basalt_bench::net::{Op, STATUS_ERR, STATUS_OK, read_response_header, write_request};
use basalt_proto::frame::BatchReader;
use basalt_proto::manifest::BatchRequest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static TEST_ID: AtomicU32 = AtomicU32::new(0);

/// A running server plus the temp directory it serves.
struct Harness {
    plain: SocketAddr,
    tls: SocketAddr,
    root: PathBuf,
}

impl Harness {
    fn plain_addr(&self) -> String {
        self.plain.to_string()
    }
    fn tls_addr(&self) -> String {
        self.tls.to_string()
    }
}

/// Writes a small known corpus and starts a server over it.
///
/// Ports come from the OS via port 0 and are read back after binding. Probing
/// for a free port and binding it as a separate step is a race that `cargo
/// test`'s parallelism loses often enough to make the suite flaky.
async fn start_server() -> Harness {
    let id = TEST_ID.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("basalt-net-test-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::create_dir_all(root.join("media")).unwrap();

    std::fs::write(root.join("hello.txt"), b"hello world").unwrap();
    std::fs::write(root.join("empty.txt"), b"").unwrap();
    std::fs::write(root.join("docs/notes.txt"), "a note\n".repeat(500)).unwrap();
    std::fs::write(
        root.join("docs/report.txt"),
        "compressible text ".repeat(2000),
    )
    .unwrap();
    // Incompressible, so the compression policy has both cases to choose from.
    let noise: Vec<u8> = (0..40_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 16) as u8)
        .collect();
    std::fs::write(root.join("media/clip.bin"), &noise).unwrap();

    let any: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound = bind_server(ServerConfig {
        root: root.clone(),
        addr: any,
        tls_addr: any,
    })
    .await
    .expect("server should bind");

    // Sockets are already listening at this point, so there is no readiness
    // race: connections queue in the backlog until `serve` starts accepting.
    let plain = bound.plain_addr();
    let tls = bound.tls_addr();
    tokio::spawn(async move {
        let _ = serve_server(bound).await;
    });

    Harness { plain, tls, root }
}

/// Issues one request and returns `(status, body)`.
async fn request(conn: &mut Conn, op: Op, payload: &[u8]) -> (u8, Vec<u8>) {
    let io = conn.as_io();
    write_request(io, op, payload).await.unwrap();
    let (status, len) = read_response_header(io).await.unwrap();
    let mut body = vec![0u8; len as usize];
    if len > 0 {
        io.read_exact(&mut body).await.unwrap();
    }
    (status, body)
}

fn decode_batch(body: &[u8]) -> Vec<basalt_proto::frame::Entry> {
    BatchReader::new(body)
        .expect("batch header")
        .collect::<Result<_, _>>()
        .expect("batch body")
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn ping_round_trips() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();
    let (status, body) = request(&mut c, Op::Ping, b"").await;
    assert_eq!(status, STATUS_OK);
    assert!(body.is_empty());
}

#[tokio::test]
async fn get_file_returns_exact_bytes() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    for name in ["hello.txt", "empty.txt", "docs/notes.txt", "media/clip.bin"] {
        let (status, body) = request(&mut c, Op::GetFile, name.as_bytes()).await;
        assert_eq!(status, STATUS_OK, "{name} should be served");
        let expected = std::fs::read(h.root.join(name)).unwrap();
        assert_eq!(body, expected, "{name} came back with different bytes");
    }
}

#[tokio::test]
async fn many_requests_reuse_one_connection() {
    // The shipped client keeps connections open. Request N must not be
    // affected by request N-1 leaving bytes on the wire.
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();
    let expected = std::fs::read(h.root.join("hello.txt")).unwrap();

    for i in 0..200 {
        let (status, body) = request(&mut c, Op::GetFile, b"hello.txt").await;
        assert_eq!(status, STATUS_OK, "request {i}");
        assert_eq!(body, expected, "request {i} desynchronised the stream");
    }
}

#[tokio::test]
async fn missing_file_reports_an_error_without_dropping_the_connection() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    let (status, body) = request(&mut c, Op::GetFile, b"nope.txt").await;
    assert_eq!(status, STATUS_ERR);
    assert!(!body.is_empty(), "an error should explain itself");

    // The connection must still be usable — one bad request cannot poison it.
    let (status, body) = request(&mut c, Op::GetFile, b"hello.txt").await;
    assert_eq!(status, STATUS_OK);
    assert_eq!(body, b"hello world");
}

#[tokio::test]
async fn traversal_attempts_are_refused_over_the_wire() {
    // The end-to-end version of the path-safety unit tests: a real peer asking
    // a real server for a file outside the share.
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    let attacks: &[&[u8]] = &[
        b"../../../Windows/win.ini",
        b"..\\..\\Windows\\win.ini",
        b"/Windows/win.ini",
        b"C:/Windows/win.ini",
        b"docs/../../secret.txt",
        b"\\\\evil\\share\\x",
        b"docs/../../../..",
    ];

    for attack in attacks {
        let (status, _) = request(&mut c, Op::GetFile, attack).await;
        assert_eq!(
            status,
            STATUS_ERR,
            "server served a path outside its root: {:?}",
            String::from_utf8_lossy(attack)
        );
    }

    // Still alive and serving legitimate requests.
    let (status, _) = request(&mut c, Op::GetFile, b"hello.txt").await;
    assert_eq!(status, STATUS_OK);
}

#[tokio::test]
async fn batch_returns_every_requested_file_intact() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    let paths = vec![
        "hello.txt".to_string(),
        "docs/notes.txt".to_string(),
        "docs/report.txt".to_string(),
        "media/clip.bin".to_string(),
        "empty.txt".to_string(),
    ];
    let req = serde_json::to_vec(&BatchRequest::new(paths.clone())).unwrap();
    let (status, body) = request(&mut c, Op::GetBatch, &req).await;
    assert_eq!(status, STATUS_OK);

    let entries = decode_batch(&body);
    assert_eq!(entries.len(), paths.len());

    for path in &paths {
        let entry = entries
            .iter()
            .find(|e| &e.path == path)
            .unwrap_or_else(|| panic!("{path} missing from the batch"));
        let expected = std::fs::read(h.root.join(path)).unwrap();
        assert_eq!(
            entry.data, expected,
            "{path} came back with different bytes"
        );
    }
}

#[tokio::test]
async fn compressed_and_uncompressed_batches_carry_identical_data() {
    // Compression must be invisible to the caller. If these ever diverge, the
    // codec is corrupting data.
    let h = start_server().await;
    let paths = vec![
        "hello.txt".to_string(),
        "docs/notes.txt".to_string(),
        "docs/report.txt".to_string(),
        "media/clip.bin".to_string(),
    ];

    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    let raw_req = serde_json::to_vec(&BatchRequest::uncompressed(paths.clone())).unwrap();
    let (_, raw_body) = request(&mut c, Op::GetBatch, &raw_req).await;
    let raw_entries = decode_batch(&raw_body);

    let zstd_req = serde_json::to_vec(&BatchRequest::new(paths.clone())).unwrap();
    let (_, zstd_body) = request(&mut c, Op::GetBatch, &zstd_req).await;
    let zstd_entries = decode_batch(&zstd_body);

    assert_eq!(raw_entries, zstd_entries, "compression altered the payload");
    assert!(
        zstd_body.len() < raw_body.len(),
        "the compressed batch ({} bytes) should be smaller than the raw one ({} bytes)",
        zstd_body.len(),
        raw_body.len()
    );
}

#[tokio::test]
async fn one_bad_path_does_not_abort_the_whole_batch() {
    // The reason errors are carried inline: a single unreadable file must not
    // cost the user the other 9,999.
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    let paths = vec![
        "hello.txt".to_string(),
        "does-not-exist.txt".to_string(),
        "../../escape.txt".to_string(),
        "docs/notes.txt".to_string(),
    ];
    let req = serde_json::to_vec(&BatchRequest::new(paths)).unwrap();
    let (status, body) = request(&mut c, Op::GetBatch, &req).await;
    assert_eq!(status, STATUS_OK, "the batch itself should still succeed");

    let entries = decode_batch(&body);
    let good: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == basalt_proto::frame::EntryKind::File)
        .collect();
    let bad: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == basalt_proto::frame::EntryKind::Error)
        .collect();

    assert_eq!(good.len(), 2, "both readable files should arrive");
    assert_eq!(bad.len(), 2, "both failures should be reported inline");
    assert!(bad.iter().all(|e| e.error.is_some()));
}

#[tokio::test]
async fn source_delivers_the_exact_byte_count() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    for n in [0u64, 1, 1023, 1024 * 1024, 3_000_000] {
        let (status, body) = request(&mut c, Op::Source, &n.to_le_bytes()).await;
        assert_eq!(status, STATUS_OK);
        assert_eq!(
            body.len() as u64,
            n,
            "asked for {n} bytes, got {}",
            body.len()
        );
    }
}

#[tokio::test]
async fn sink_accepts_the_exact_byte_count() {
    let h = start_server().await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();

    for n in [0usize, 1, 5000, 2_000_000] {
        let io = c.as_io();
        write_request(io, Op::Sink, &(n as u64).to_le_bytes())
            .await
            .unwrap();
        io.write_all(&vec![0xA5u8; n]).await.unwrap();
        io.flush().await.unwrap();
        let (status, len) = read_response_header(io).await.unwrap();
        assert_eq!(status, STATUS_OK, "sink of {n} bytes");
        assert_eq!(len, 0);
    }
}

#[tokio::test]
async fn tls_carries_the_same_data_as_plaintext() {
    let h = start_server().await;
    let connector = build_tls_connector().unwrap();

    let mut plain = Conn::connect_plain(&h.plain_addr()).await.unwrap();
    let mut tls = Conn::connect_tls(&h.tls_addr(), &connector).await.unwrap();

    for name in ["hello.txt", "docs/report.txt", "media/clip.bin"] {
        let (ps, pb) = request(&mut plain, Op::GetFile, name.as_bytes()).await;
        let (ts, tb) = request(&mut tls, Op::GetFile, name.as_bytes()).await;
        assert_eq!(ps, ts, "{name}: status differed between plaintext and TLS");
        assert_eq!(pb, tb, "{name}: bytes differed between plaintext and TLS");
    }
}

#[tokio::test]
async fn tls_rejects_a_plaintext_client() {
    // Sanity check that the TLS listener really is doing TLS. A plaintext
    // request should never be answered as though it were valid.
    let h = start_server().await;
    let mut stream = tokio::net::TcpStream::connect(h.tls).await.unwrap();
    write_request(&mut stream, Op::Ping, b"").await.unwrap();

    let result =
        tokio::time::timeout(Duration::from_secs(2), read_response_header(&mut stream)).await;
    match result {
        Ok(Ok(_)) => panic!("TLS listener answered an unencrypted request"),
        Ok(Err(_)) | Err(_) => {}
    }
}

#[tokio::test]
async fn concurrent_clients_do_not_interfere() {
    let h = start_server().await;
    let expected = std::fs::read(h.root.join("docs/report.txt")).unwrap();

    let mut tasks = Vec::new();
    for client in 0..16 {
        let addr = h.plain_addr();
        let expected = expected.clone();
        tasks.push(tokio::spawn(async move {
            let mut c = Conn::connect_plain(&addr).await.unwrap();
            for round in 0..10 {
                let (status, body) = request(&mut c, Op::GetFile, b"docs/report.txt").await;
                assert_eq!(status, STATUS_OK, "client {client} round {round}");
                assert_eq!(
                    body, expected,
                    "client {client} round {round} got interleaved data"
                );
            }
        }));
    }
    for t in tasks {
        t.await.expect("a concurrent client panicked");
    }
}

#[tokio::test]
async fn concurrent_batches_stay_independent() {
    let h = start_server().await;

    let mut tasks = Vec::new();
    for client in 0..8 {
        let addr = h.plain_addr();
        let root = h.root.clone();
        tasks.push(tokio::spawn(async move {
            let mut c = Conn::connect_plain(&addr).await.unwrap();
            let paths = vec!["hello.txt".to_string(), "docs/notes.txt".to_string()];
            let req = serde_json::to_vec(&BatchRequest::new(paths.clone())).unwrap();
            let (status, body) = request(&mut c, Op::GetBatch, &req).await;
            assert_eq!(status, STATUS_OK, "client {client}");
            let entries = decode_batch(&body);
            assert_eq!(entries.len(), 2, "client {client}");
            for e in entries {
                let expected = std::fs::read(root.join(&e.path)).unwrap();
                assert_eq!(e.data, expected, "client {client}: {} corrupted", e.path);
            }
        }));
    }
    for t in tasks {
        t.await.expect("a concurrent batch client panicked");
    }
}

#[tokio::test]
async fn a_malformed_request_does_not_crash_the_server() {
    let h = start_server().await;

    // Garbage opcode.
    {
        let mut stream = tokio::net::TcpStream::connect(h.plain).await.unwrap();
        stream.write_all(&[0xEE, 0, 0, 0, 0]).await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = [0u8; 16];
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
    }
    // Declared payload far larger than what follows.
    {
        let mut stream = tokio::net::TcpStream::connect(h.plain).await.unwrap();
        stream.write_all(&[Op::GetFile as u8]).await.unwrap();
        stream.write_all(&u32::MAX.to_le_bytes()).await.unwrap();
        stream.write_all(b"short").await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = [0u8; 16];
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
    }
    // Connection closed mid-header.
    {
        let mut stream = tokio::net::TcpStream::connect(h.plain).await.unwrap();
        stream.write_all(&[Op::GetFile as u8, 0xFF]).await.unwrap();
        drop(stream);
    }

    // The server must still be healthy after all of that.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();
    let (status, body) = request(&mut c, Op::GetFile, b"hello.txt").await;
    assert_eq!(status, STATUS_OK, "server did not survive malformed input");
    assert_eq!(body, b"hello world");
}

#[tokio::test]
async fn a_large_batch_round_trips() {
    // Closer to the real workload: many small files in one request.
    let h = start_server().await;
    let dir = h.root.join("many");
    std::fs::create_dir_all(&dir).unwrap();

    let mut paths = Vec::new();
    for i in 0..1000 {
        let name = format!("many/file{i:04}.txt");
        std::fs::write(h.root.join(&name), format!("contents of file {i}\n")).unwrap();
        paths.push(name);
    }

    let mut c = Conn::connect_plain(&h.plain_addr()).await.unwrap();
    let req = serde_json::to_vec(&BatchRequest::new(paths.clone())).unwrap();
    let (status, body) = request(&mut c, Op::GetBatch, &req).await;
    assert_eq!(status, STATUS_OK);

    let entries = decode_batch(&body);
    assert_eq!(entries.len(), 1000);
    for entry in &entries {
        let i: usize = entry
            .path
            .trim_start_matches("many/file")
            .trim_end_matches(".txt")
            .parse()
            .unwrap();
        assert_eq!(
            entry.data,
            format!("contents of file {i}\n").as_bytes(),
            "{} has the wrong contents",
            entry.path
        );
    }
}

#[test]
fn test_corpus_paths_are_relative() {
    // Guard against a refactor that starts sending absolute paths, which the
    // server would rightly refuse.
    for p in ["hello.txt", "docs/notes.txt", "media/clip.bin"] {
        assert!(!Path::new(p).is_absolute());
    }
}
