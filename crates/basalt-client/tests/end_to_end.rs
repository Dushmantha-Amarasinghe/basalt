//! Host and client, in one process, over real sockets and real TLS.
//!
//! These are the tests that would have caught every integration bug the unit
//! tests cannot see: a mismatch between what one side writes and the other
//! reads, an operation allowed before authentication, a path check that holds
//! in the vault but is bypassed by the wire format.
//!
//! Everything binds to port 0 and reads back whichever port the OS gave it.
//! Probing for a free port and binding it separately is a race, and under
//! `cargo test`'s parallelism that race loses often enough to make a suite
//! flaky.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use basalt_client::{Basalt, ClientError};
use basalt_host::config::HostConfig;
use basalt_host::server::{self, Host};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "basalt-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A running host, its vault on disk, and everything cleaned up on drop.
struct Fixture {
    dir: PathBuf,
    host: Arc<Host>,
    addr: SocketAddr,
    client_store: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.client_store);
    }
}

impl Fixture {
    fn vault_path(&self, rel: &str) -> PathBuf {
        self.dir.join("vault").join(rel)
    }

    fn address(&self) -> String {
        self.addr.to_string()
    }

    fn client(&self) -> Arc<Basalt> {
        Arc::new(Basalt::open(self.client_store.clone()).expect("a fresh client store"))
    }

    /// Pairs a client the way a person does.
    ///
    /// Asking is what makes the host generate and display a PIN, so the first
    /// attempt is expected to come back asking for one — exactly as the app
    /// does, which then shows a PIN field.
    async fn paired_client(&self) -> Arc<Basalt> {
        let client = self.client();
        self.pair(&client).await.expect("pairing succeeds");
        client
    }

    /// Runs the whole two-step pairing against this host.
    async fn pair(&self, client: &Arc<Basalt>) -> Result<(), ClientError> {
        let requires_pin = client.begin_pairing(self.addr).await?;
        let pin = if requires_pin {
            Some(self.displayed_pin().expect("the host displays a PIN"))
        } else {
            None
        };
        client.finish_pairing(pin.as_deref()).await.map(|_| ())
    }

    /// The PIN the host is currently showing, whoever it is for.
    fn displayed_pin(&self) -> Option<String> {
        self.host.pending_pairings().into_iter().find_map(|r| r.pin)
    }
}

async fn start_host() -> Fixture {
    let dir = unique("e2e");
    let vault = dir.join("vault");
    std::fs::create_dir_all(vault.join("films")).unwrap();
    std::fs::create_dir_all(vault.join("docs")).unwrap();
    std::fs::create_dir_all(vault.join("empty")).unwrap();
    std::fs::write(vault.join("notes.txt"), b"hello from the vault").unwrap();
    std::fs::write(vault.join("films").join("short.mkv"), sample_bytes(50_000)).unwrap();
    for i in 0..20 {
        std::fs::write(
            vault.join("docs").join(format!("doc-{i:02}.txt")),
            format!("document number {i}, with some repeated filler text. ").repeat(20),
        )
        .unwrap();
    }

    let config_path = dir.join("host.json");
    let mut config = HostConfig::create("test-host").unwrap();
    config.vault_path = Some(vault.clone());
    config.vault_name = "Test Vault".into();
    config.save(&config_path).unwrap();

    let host = Host::new(config, config_path).unwrap();
    let bound = server::bind(Arc::clone(&host), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = bound.addr();
    tokio::spawn(server::serve(bound));

    Fixture {
        dir,
        host,
        addr,
        client_store: unique("client-store").with_extension("json"),
    }
}

/// Deterministic bytes that do not compress to nothing, so compression paths
/// are actually exercised rather than short-circuited.
fn sample_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + i / 7) % 251) as u8).collect()
}

fn blake3_of(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pairing_then_browsing_works_end_to_end() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let info = client.status().expect("connected after pairing");
    assert_eq!(info.vault, "Test Vault");
    assert_eq!(info.host_id, fixture.host.host_id());
    assert!(info.writable);

    let names: Vec<_> = client
        .list("")
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["docs", "empty", "films", "notes.txt"]);
}

#[tokio::test]
async fn a_wrong_pin_is_refused_and_pairs_nothing() {
    let fixture = start_host().await;
    let client = fixture.client();

    // Asking makes the host display a PIN.
    assert!(client.begin_pairing(fixture.addr).await.unwrap());
    let real = fixture.displayed_pin().unwrap();
    let wrong = if real == "000000" { "111111" } else { "000000" };

    let err = client.finish_pairing(Some(wrong)).await.unwrap_err();
    assert_eq!(err.kind(), "pairing", "got: {err}");
    assert!(client.known_hosts().is_empty());
    assert!(!client.is_connected());
}

// The whole point of a request: the host can say *who* is asking, beside the
// number to read across.
#[tokio::test]
async fn asking_to_pair_shows_the_request_on_the_host() {
    let fixture = start_host().await;
    assert!(fixture.host.pending_pairings().is_empty());

    let client = fixture.client();
    assert!(client.begin_pairing(fixture.addr).await.unwrap());

    let pending = fixture.host.pending_pairings();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].device_name, client.device_name());
    assert!(pending[0].pin.is_some(), "a PIN to read across");
}

#[tokio::test]
async fn a_pin_pairs_exactly_one_device() {
    let fixture = start_host().await;
    let client = fixture.client();
    client.begin_pairing(fixture.addr).await.unwrap();
    let pin = fixture.displayed_pin().unwrap();
    client.finish_pairing(Some(&pin)).await.unwrap();

    // The same number, tried by somebody else, is spent.
    let second = Arc::new(Basalt::open(unique("second").with_extension("json")).unwrap());
    second.begin_pairing(fixture.addr).await.unwrap();
    assert!(
        second.finish_pairing(Some(&pin)).await.is_err(),
        "a PIN belongs to one request and is consumed by it"
    );
}

// The setting the user asked for: no PIN, and connecting is one click.
#[tokio::test]
async fn with_the_pin_switched_off_pairing_needs_nothing_typed() {
    let fixture = start_host().await;
    fixture.host.set_require_pin(false).unwrap();

    let client = fixture.client();
    assert!(
        !client.begin_pairing(fixture.addr).await.unwrap(),
        "the host should not be asking for one"
    );
    let info = client.finish_pairing(None).await.expect("nothing to type");

    assert_eq!(info.vault, "Test Vault");
    assert!(!client.list("").await.unwrap().is_empty());
}

#[tokio::test]
async fn the_host_can_refuse_a_request_before_it_completes() {
    let fixture = start_host().await;
    let client = fixture.client();
    client.begin_pairing(fixture.addr).await.unwrap();

    let request = fixture.host.pending_pairings().remove(0);
    let pin = request.pin.clone().unwrap();
    assert!(fixture.host.deny_pairing(&request.id));

    assert!(
        client.finish_pairing(Some(&pin)).await.is_err(),
        "a refused request must not be completable"
    );
}

#[tokio::test]
async fn an_unpaired_client_cannot_touch_the_drive() {
    let fixture = start_host().await;

    // Probing is deliberately allowed: it is how the client shows the user
    // what it found before they commit to pairing.
    let hello = fixture
        .client()
        .probe(&fixture.address())
        .await
        .expect("probing needs no token");
    assert_eq!(hello.host_id, fixture.host.host_id());
    assert_eq!(hello.vault, "Test Vault");

    // But nothing beyond it.
    let unpaired = fixture.client();
    assert!(matches!(
        unpaired.list("").await,
        Err(ClientError::NotConnected)
    ));
}

#[tokio::test]
async fn a_paired_client_reconnects_without_pairing_again() {
    let fixture = start_host().await;
    let first = fixture.paired_client().await;
    let host_id = first.status().unwrap().host_id;
    first.disconnect().await;

    // A brand new client object reading the same store, standing in for the
    // app being closed and reopened.
    let reopened = fixture.client();
    let info = reopened.connect_saved().await.expect("reconnects silently");
    assert_eq!(info.host_id, host_id);
    assert!(!reopened.list("").await.unwrap().is_empty());
}

#[tokio::test]
async fn a_revoked_device_stops_working() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.list("").await.is_ok());

    let hash = fixture.host.devices()[0].token_hash.clone();
    assert!(fixture.host.revoke(&hash).unwrap());
    client.disconnect().await;

    let err = client.connect_saved().await.unwrap_err();
    assert_eq!(err.kind(), "unpaired", "got: {err}");
}

// The check that makes the pin worth having: a host that is not the one this
// device paired with must be refused, even though it is a perfectly valid
// Basalt host with a perfectly valid certificate.
#[tokio::test]
async fn a_different_host_at_the_same_address_is_refused() {
    let first = start_host().await;
    let client = first.paired_client().await;
    let host_id = client.status().unwrap().host_id;
    client.disconnect().await;

    let impostor = start_host().await;
    let err = client
        .connect(&host_id, Some(&impostor.address()))
        .await
        .unwrap_err();

    assert!(
        err.is_transient() || matches!(err, ClientError::WrongHost { .. }),
        "connecting to the wrong host must fail, got: {err}"
    );
    assert!(!client.is_connected());
}

// ---------------------------------------------------------------------------
// Browsing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listings_carry_kind_size_and_time() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let entries = client.list("").await.unwrap();
    let notes = entries.iter().find(|e| e.name == "notes.txt").unwrap();
    assert_eq!(notes.kind, basalt_proto::msg::EntryKind::File);
    assert_eq!(notes.size, "hello from the vault".len() as u64);
    assert!(notes.mtime > 1_600_000_000, "a real timestamp");

    let films = entries.iter().find(|e| e.name == "films").unwrap();
    assert_eq!(films.kind, basalt_proto::msg::EntryKind::Dir);
}

#[tokio::test]
async fn an_empty_folder_lists_as_empty_rather_than_failing() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.list("empty").await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_folder_reports_not_found() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let err = client.list("nowhere").await.unwrap_err();
    assert_eq!(err.kind(), "notfound", "got: {err}");
}

#[tokio::test]
async fn the_vault_reports_its_free_space() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let (free, total) = client.space().await.unwrap();
    assert!(total > 0);
    assert!(free <= total);
}

// Traversal is rejected in the vault by unit tests; this proves it is also
// rejected when it arrives over the wire, where a different code path parses it.
#[tokio::test]
async fn traversal_over_the_wire_is_refused() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    for escape in [
        "../",
        "../../Windows",
        "films/../../..",
        "C:/Windows",
        "\\\\server\\share",
        "films/../../host.json",
    ] {
        let result = client.list(escape).await;
        assert!(result.is_err(), "{escape:?} must not list");
    }

    // And the file just outside the vault really is unreachable.
    assert!(fixture.dir.join("host.json").exists());
    assert!(client.read_range("../host.json", 0, 10).await.is_err());
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ranged_reads_land_on_the_right_bytes() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let whole = sample_bytes(50_000);

    for (offset, length) in [
        (0u64, 1u64),
        (0, 50_000),
        (1, 4096),
        (25_000, 25_000),
        (49_999, 1),
        (10_000, 100_000),
    ] {
        let got = client
            .read_range("films/short.mkv", offset, length)
            .await
            .unwrap();
        let end = ((offset + length) as usize).min(whole.len());
        assert_eq!(
            got,
            &whole[offset as usize..end],
            "offset {offset} length {length}"
        );
    }
}

#[tokio::test]
async fn a_read_past_the_end_is_empty_rather_than_an_error() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(
        client
            .read_range("films/short.mkv", 50_000, 1000)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_oversized_read_is_refused_rather_than_attempted() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let err = client
        .read_range("films/short.mkv", 0, 1 << 30)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "error", "got: {err}");
}

// The batch path is the largest measured win in the system (7.6x), so it gets
// its own end-to-end proof rather than only unit coverage of the framing.
#[tokio::test]
async fn a_batch_fetches_many_files_in_one_round_trip() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let paths: Vec<String> = (0..20).map(|i| format!("docs/doc-{i:02}.txt")).collect();
    let entries = client.read_batch(paths.clone()).await.unwrap();
    assert_eq!(entries.len(), 20);

    for entry in &entries {
        assert_eq!(entry.kind, basalt_proto::EntryKind::File);
        let expected = std::fs::read(fixture.vault_path(&entry.path)).unwrap();
        assert_eq!(entry.data, expected, "{}", entry.path);
    }
}

#[tokio::test]
async fn one_bad_path_does_not_destroy_a_whole_batch() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let entries = client
        .read_batch(vec![
            "docs/doc-00.txt".into(),
            "docs/missing.txt".into(),
            "../escaped.txt".into(),
            "docs/doc-01.txt".into(),
        ])
        .await
        .unwrap();

    let files: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == basalt_proto::EntryKind::File)
        .collect();
    let errors: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == basalt_proto::EntryKind::Error)
        .collect();

    assert_eq!(files.len(), 2, "the readable files still arrive");
    assert_eq!(errors.len(), 2, "the rest arrive as inline errors");
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_download_arrives_byte_for_byte() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let dest = fixture.dir.join("downloaded.mkv");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = {
        let seen = Arc::clone(&seen);
        Arc::new(move |p: basalt_client::Progress| {
            seen.lock().unwrap().push((p.transferred, p.total));
        }) as basalt_client::client::ProgressFn
    };

    let bytes = client
        .download("films/short.mkv", &dest, Some(recorder), None)
        .await
        .unwrap();

    assert_eq!(bytes, 50_000);
    assert_eq!(
        blake3_of(&std::fs::read(&dest).unwrap()),
        blake3_of(&sample_bytes(50_000))
    );

    let progress = seen.lock().unwrap().clone();
    assert!(!progress.is_empty(), "progress must be reported");
    let (last, total) = *progress.last().unwrap();
    assert_eq!(last, total, "the final report must be complete");
}

#[tokio::test]
async fn a_download_leaves_no_partial_file_behind() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let dest = fixture.dir.join("clean.mkv");

    client
        .download("films/short.mkv", &dest, None, None)
        .await
        .unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(&fixture.dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".part"))
        .collect();
    assert!(leftovers.is_empty(), "found {leftovers:?}");
}

#[tokio::test]
async fn an_upload_arrives_byte_for_byte() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let source = fixture.dir.join("to-upload.bin");
    let data = sample_bytes(300_000);
    std::fs::write(&source, &data).unwrap();

    client
        .upload(&source, "films/uploaded.bin", false, None, None)
        .await
        .unwrap();

    let landed = std::fs::read(fixture.vault_path("films/uploaded.bin")).unwrap();
    assert_eq!(blake3_of(&landed), blake3_of(&data));
}

#[tokio::test]
async fn an_empty_file_uploads_and_downloads() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let source = fixture.dir.join("empty-source.bin");
    std::fs::write(&source, b"").unwrap();
    client
        .upload(&source, "empty.bin", false, None, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(fixture.vault_path("empty.bin"))
            .unwrap()
            .len(),
        0
    );

    let dest = fixture.dir.join("empty-back.bin");
    assert_eq!(
        client
            .download("empty.bin", &dest, None, None)
            .await
            .unwrap(),
        0
    );
    assert!(dest.exists());
}

#[tokio::test]
async fn an_upload_refuses_to_replace_a_file_unless_asked() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let source = fixture.dir.join("source.txt");
    std::fs::write(&source, b"replacement").unwrap();

    let err = client
        .upload(&source, "notes.txt", false, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "exists", "got: {err}");
    assert_eq!(
        std::fs::read(fixture.vault_path("notes.txt")).unwrap(),
        b"hello from the vault"
    );

    client
        .upload(&source, "notes.txt", true, None, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.vault_path("notes.txt")).unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn an_upload_cannot_be_aimed_outside_the_vault() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let source = fixture.dir.join("payload.txt");
    std::fs::write(&source, b"payload").unwrap();

    for escape in [
        "../escaped.txt",
        "C:/Windows/Temp/escaped.txt",
        "films/../../x",
    ] {
        assert!(
            client
                .upload(&source, escape, true, None, None)
                .await
                .is_err(),
            "{escape} must be refused"
        );
    }
    assert!(!fixture.dir.join("escaped.txt").exists());
}

#[tokio::test]
async fn a_round_trip_through_the_host_preserves_a_larger_file() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    // Larger than one chunk, so the chunking itself is exercised.
    let data = sample_bytes(9 * 1024 * 1024);
    let source = fixture.dir.join("big-source.bin");
    std::fs::write(&source, &data).unwrap();

    client
        .upload(&source, "big.bin", false, None, None)
        .await
        .unwrap();

    let dest = fixture.dir.join("big-back.bin");
    client.download("big.bin", &dest, None, None).await.unwrap();

    assert_eq!(
        blake3_of(&std::fs::read(&dest).unwrap()),
        blake3_of(&data),
        "a file must survive a full round trip unchanged"
    );
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn folders_are_created_renamed_and_removed() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    client.mkdir("fresh").await.unwrap();
    assert!(fixture.vault_path("fresh").is_dir());

    client.rename("fresh", "renamed").await.unwrap();
    assert!(fixture.vault_path("renamed").is_dir());
    assert!(!fixture.vault_path("fresh").exists());

    client.remove("renamed", false).await.unwrap();
    assert!(!fixture.vault_path("renamed").exists());
}

#[tokio::test]
async fn a_full_folder_needs_the_recursive_flag() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let err = client.remove("docs", false).await.unwrap_err();
    assert_eq!(err.kind(), "notempty", "got: {err}");
    assert!(fixture.vault_path("docs").exists(), "nothing was deleted");

    client.remove("docs", true).await.unwrap();
    assert!(!fixture.vault_path("docs").exists());
}

#[tokio::test]
async fn copying_happens_on_the_host_without_moving_bytes_over_the_link() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let before = client.bytes_moved();
    client
        .copy("films/short.mkv", "films/copy.mkv")
        .await
        .unwrap();
    let after = client.bytes_moved();

    assert_eq!(
        std::fs::read(fixture.vault_path("films/copy.mkv")).unwrap(),
        sample_bytes(50_000)
    );
    assert!(
        fixture.vault_path("films/short.mkv").exists(),
        "the original stays put"
    );
    assert!(
        after - before < 1000,
        "a 50 KB copy moved {} bytes over the link; it should move only the request",
        after - before
    );
}

#[tokio::test]
async fn copying_a_folder_brings_everything_under_it() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    client.copy("docs", "docs-copy").await.unwrap();

    let copied = client.list("docs-copy").await.unwrap();
    assert_eq!(copied.len(), 20);
    assert_eq!(
        std::fs::read(fixture.vault_path("docs-copy/doc-00.txt")).unwrap(),
        std::fs::read(fixture.vault_path("docs/doc-00.txt")).unwrap()
    );
}

#[tokio::test]
async fn copying_is_refused_where_it_should_be() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    // Onto something that exists.
    assert_eq!(
        client.copy("notes.txt", "docs").await.unwrap_err().kind(),
        "exists"
    );
    // Into itself, which would never finish.
    assert!(client.copy("docs", "docs/inner").await.is_err());
    // Out of the vault.
    assert!(client.copy("notes.txt", "../escaped.txt").await.is_err());
    assert!(!fixture.dir.join("escaped.txt").exists());
    // From outside it.
    assert!(client.copy("../host.json", "stolen.json").await.is_err());
    assert!(!fixture.vault_path("stolen.json").exists());
}

// Moving is a rename, which is what makes drag-and-drop and cut-and-paste
// instant rather than a transfer in each direction.
#[tokio::test]
async fn moving_a_file_into_a_folder_is_instant_and_keeps_its_contents() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let before = client.bytes_moved();
    client.rename("notes.txt", "docs/notes.txt").await.unwrap();
    let after = client.bytes_moved();

    assert!(!fixture.vault_path("notes.txt").exists());
    assert_eq!(
        std::fs::read(fixture.vault_path("docs/notes.txt")).unwrap(),
        b"hello from the vault"
    );
    assert!(after - before < 1000, "a move must not transfer the file");
}

#[tokio::test]
async fn a_read_only_device_can_browse_but_not_change_anything() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.mkdir("while-writable").await.is_ok());

    // Demote the device, then reconnect: the grant is read at authentication,
    // so an open session keeps the access it was given.
    let hash = fixture.host.devices()[0].token_hash.clone();
    assert!(fixture.host.set_writable(&hash, false).unwrap());
    client.disconnect().await;

    let info = client.connect_saved().await.unwrap();
    assert!(
        !info.writable,
        "the client must be told up front, so the interface can grey the actions out \
         rather than failing at the end of a long upload"
    );

    // Reading still works — that is the whole point of read-only.
    assert!(client.list("").await.is_ok());
    assert!(client.read_range("notes.txt", 0, 5).await.is_ok());

    // Writing does not, through any of the doors.
    let source = fixture.dir.join("blocked.txt");
    std::fs::write(&source, b"nope").unwrap();
    for (label, result) in [
        ("mkdir", client.mkdir("blocked").await),
        ("rename", client.rename("notes.txt", "moved.txt").await),
        ("remove", client.remove("notes.txt", false).await),
        ("copy", client.copy("notes.txt", "copied.txt").await),
        (
            "upload",
            client
                .upload(&source, "blocked.txt", true, None, None)
                .await
                .map(|_| ()),
        ),
    ] {
        let err = result.expect_err("{label} must be refused");
        assert_eq!(err.kind(), "denied", "{label} gave: {err}");
    }

    assert!(fixture.vault_path("notes.txt").exists());
    assert!(!fixture.vault_path("blocked").exists());
    assert!(!fixture.vault_path("blocked.txt").exists());
}

// ---------------------------------------------------------------------------
// The media proxy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_media_proxy_serves_ranges_like_a_web_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(&client))
        .await
        .unwrap();

    let url = proxy.url_for("films/short.mkv");
    assert!(url.starts_with("http://127.0.0.1:"));

    // A range request, exactly as a video element would send it.
    let mut socket = tokio::net::TcpStream::connect(proxy.addr()).await.unwrap();
    let path = url.split_once("127.0.0.1").unwrap().1;
    let path = path.split_once('/').unwrap().1;
    socket
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: localhost\r\nRange: bytes=100-199\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();

    let mut response = Vec::new();
    socket.read_to_end(&mut response).await.unwrap();

    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("headers end");
    let headers = String::from_utf8_lossy(&response[..split]).to_string();
    let body = &response[split + 4..];

    assert!(headers.contains("206 Partial Content"), "{headers}");
    assert!(
        headers.contains("Content-Range: bytes 100-199/50000"),
        "{headers}"
    );
    assert!(headers.contains("Accept-Ranges: bytes"), "{headers}");
    assert_eq!(body, &sample_bytes(50_000)[100..200]);
}

/// One HTTP request against the proxy, returning the headers and body.
async fn proxy_request(
    proxy: &basalt_client::proxy::MediaProxy,
    path: &str,
    range: Option<&str>,
) -> (String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let url = proxy.url_for(path);
    let target = url.split_once("127.0.0.1").unwrap().1;
    let target = target.split_once('/').unwrap().1;

    let mut socket = tokio::net::TcpStream::connect(proxy.addr()).await.unwrap();
    let mut request = format!("GET /{target} HTTP/1.1\r\nHost: localhost\r\n");
    if let Some(range) = range {
        request.push_str(&format!("Range: {range}\r\n"));
    }
    request.push_str("\r\n");
    socket.write_all(request.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    socket.read_to_end(&mut response).await.unwrap();
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("headers end");
    (
        String::from_utf8_lossy(&response[..split]).to_string(),
        response[split + 4..].to_vec(),
    )
}

/// The exact sequence a video element performs: open with an unbounded range,
/// read the tail to find the index, then jump to the middle when the user
/// drags the scrubber. Every byte is checked against the file on disk, because
/// a proxy that is off by one produces a video that plays and is subtly wrong.
#[tokio::test]
async fn the_media_proxy_is_byte_exact_through_a_players_whole_sequence() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(&client))
        .await
        .unwrap();

    let whole = sample_bytes(50_000);

    // 1. Opening request: everything from the start.
    let (headers, body) = proxy_request(&proxy, "films/short.mkv", Some("bytes=0-")).await;
    assert!(headers.contains("206 Partial Content"), "{headers}");
    assert!(
        headers.contains("/50000"),
        "the full length must be advertised"
    );
    assert_eq!(body, &whole[..body.len()]);
    assert!(!body.is_empty());

    // 2. The tail, which is how a player finds the index of a file that was
    //    not written for streaming.
    let (headers, body) = proxy_request(&proxy, "films/short.mkv", Some("bytes=-4096")).await;
    assert!(
        headers.contains("Content-Range: bytes 45904-49999/50000"),
        "{headers}"
    );
    assert_eq!(body, &whole[45_904..]);

    // 3. A seek into the middle.
    let (headers, body) = proxy_request(&proxy, "films/short.mkv", Some("bytes=20000-29999")).await;
    assert!(
        headers.contains("Content-Range: bytes 20000-29999/50000"),
        "{headers}"
    );
    assert_eq!(body, &whole[20_000..30_000]);

    // 4. Every boundary, since off-by-one is the whole risk here.
    for (from, to) in [(0usize, 0usize), (49_999, 49_999), (0, 49_999), (1, 2)] {
        let (_, body) = proxy_request(
            &proxy,
            "films/short.mkv",
            Some(&format!("bytes={from}-{to}")),
        )
        .await;
        assert_eq!(body, &whole[from..=to], "range {from}-{to}");
    }
}

#[tokio::test]
async fn the_media_proxy_answers_a_plain_request_and_a_bad_range() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(&client))
        .await
        .unwrap();

    // No Range header at all: a 200 with the length, which is what a player
    // uses to decide whether it can seek.
    let (headers, _) = proxy_request(&proxy, "films/short.mkv", None).await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
    assert!(headers.contains("Accept-Ranges: bytes"), "{headers}");
    assert!(
        headers.contains("Content-Type: video/x-matroska"),
        "{headers}"
    );

    // A range past the end has its own status, and players rely on it.
    let (headers, _) = proxy_request(&proxy, "films/short.mkv", Some("bytes=999999-")).await;
    assert!(headers.contains("416 Range Not Satisfiable"), "{headers}");
    assert!(
        headers.contains("Content-Range: bytes */50000"),
        "{headers}"
    );
}

#[tokio::test]
async fn the_media_proxy_refuses_a_request_without_its_token() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(&client))
        .await
        .unwrap();

    let mut socket = tokio::net::TcpStream::connect(proxy.addr()).await.unwrap();
    socket
        .write_all(b"GET /wrong-token/films/short.mkv HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();

    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    assert!(
        response.starts_with("HTTP/1.1 404"),
        "another program on this machine must not be able to read the vault: {response}"
    );
}

// ---------------------------------------------------------------------------
// Connection handling
// ---------------------------------------------------------------------------

// The reason there is a pool at all: a long transfer must not stop the
// interface from working.
#[tokio::test]
async fn browsing_works_while_a_transfer_is_running() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let data = sample_bytes(8 * 1024 * 1024);
    std::fs::write(fixture.vault_path("films/large.bin"), &data).unwrap();

    let downloader = {
        let client = Arc::clone(&client);
        let dest = fixture.dir.join("concurrent.bin");
        tokio::spawn(async move { client.download("films/large.bin", &dest, None, None).await })
    };

    // Several listings while the download is in flight. If these shared a
    // connection with the transfer, they would block until it finished.
    for _ in 0..5 {
        assert!(!client.list("docs").await.unwrap().is_empty());
    }

    assert_eq!(downloader.await.unwrap().unwrap(), 8 * 1024 * 1024);
}

#[tokio::test]
async fn many_requests_reuse_connections_rather_than_opening_one_each() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    for _ in 0..50 {
        client.list("").await.unwrap();
    }
    // Nothing to assert on the host side without instrumentation; what this
    // proves is that a pooled connection survives repeated use, which is where
    // a framing bug would show up as a desynchronised stream.
    assert_eq!(client.list("").await.unwrap().len(), 4);
}

#[tokio::test]
async fn the_host_survives_a_client_that_disappears_mid_request() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    // Half a request, then hang up.
    let mut raw = tokio::net::TcpStream::connect(fixture.addr).await.unwrap();
    use tokio::io::AsyncWriteExt;
    raw.write_all(&[6u8, 200, 0, 0, 0]).await.unwrap();
    drop(raw);

    // Something that is not TLS at all.
    let mut junk = tokio::net::TcpStream::connect(fixture.addr).await.unwrap();
    junk.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    drop(junk);

    // The host must still be serving.
    assert!(client.list("").await.is_ok());
}
