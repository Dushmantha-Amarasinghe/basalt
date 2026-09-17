//! Live sync and the media library, end to end.
//!
//! A real host, a real client, and a real filesystem. These drive the disk
//! directly rather than calling the host's own operations, because the design
//! treats the filesystem as the truth — a file deleted in Explorer has to
//! arrive at every client exactly like one deleted through the app, and only a
//! test that deletes it behind the app's back proves that.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use basalt_client::Basalt;
use basalt_host::{Host, HostConfig, server};
use basalt_proto::msg::{Change, LibraryItem, LibraryKind};

fn unique(prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "basalt-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

struct Fixture {
    dir: PathBuf,
    host: Arc<Host>,
    addr: SocketAddr,
    stores: std::sync::Mutex<Vec<PathBuf>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        for store in self.stores.lock().expect("stores lock").iter() {
            let _ = std::fs::remove_file(store);
        }
    }
}

impl Fixture {
    fn vault_path(&self, rel: &str) -> PathBuf {
        self.dir.join("vault").join(rel)
    }

    async fn paired_client(&self) -> Arc<Basalt> {
        let store = unique("sync-store").with_extension("json");
        self.stores.lock().expect("stores lock").push(store.clone());

        let client = Arc::new(Basalt::open(store).expect("a fresh store"));
        let requires_pin = client
            .begin_pairing(self.addr)
            .await
            .expect("pairing opens");
        let pin = requires_pin.then(|| {
            self.host
                .pending_pairings()
                .into_iter()
                .find_map(|r| r.pin)
                .expect("the host displays a PIN")
        });
        client
            .finish_pairing(pin.as_deref())
            .await
            .expect("pairing completes");
        client
    }
}

async fn start_host() -> Fixture {
    let dir = unique("sync");
    let vault = dir.join("vault");
    std::fs::create_dir_all(vault.join("films")).unwrap();
    std::fs::write(vault.join("notes.txt"), b"hello from the vault").unwrap();

    let config_path = dir.join("host.json");
    let mut config = HostConfig::create("sync-host").unwrap();
    config.vault_path = Some(vault);
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
        stores: std::sync::Mutex::new(Vec::new()),
    }
}

/// Collects the changes a client is told about.
#[derive(Clone, Default)]
struct Seen(Arc<std::sync::Mutex<Vec<Change>>>);

impl Seen {
    fn record(&self) -> impl Fn(Change) + Send + Sync + 'static {
        let inner = Arc::clone(&self.0);
        move |change| inner.lock().expect("seen lock").push(change)
    }

    fn all(&self) -> Vec<Change> {
        self.0.lock().expect("seen lock").clone()
    }

    /// Waits for a matching change, or gives up. Generous, because this is
    /// waiting on a real filesystem watcher rather than on a mock.
    async fn wait_for(&self, want: impl Fn(&Change) -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            if self.all().iter().any(&want) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }
}

/// Gives the watcher a moment to be listening before the drive is touched.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

fn removed(name: &str) -> impl Fn(&Change) -> bool + '_ {
    move |change| matches!(change, Change::Removed { path } if path == name)
}

fn appeared(name: &str) -> impl Fn(&Change) -> bool + '_ {
    move |change| matches!(change, Change::Created { path } | Change::Modified { path } if path == name)
}

// ---------------------------------------------------------------------------
// Live sync
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_file_deleted_on_the_drive_reaches_a_watching_client() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.list("").await.is_ok());

    let seen = Seen::default();
    let _watch = client.watch(seen.record());
    settle().await;

    std::fs::remove_file(fixture.vault_path("notes.txt")).unwrap();

    assert!(
        seen.wait_for(removed("notes.txt")).await,
        "a deletion must reach the client; saw {:?}",
        seen.all()
    );
}

#[tokio::test]
async fn a_file_created_outside_the_app_reaches_a_watching_client() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let seen = Seen::default();
    let _watch = client.watch(seen.record());
    settle().await;

    // Written directly, as another program or a second client would.
    std::fs::write(fixture.vault_path("films/new.mkv"), b"x").unwrap();

    assert!(
        seen.wait_for(appeared("films/new.mkv")).await,
        "saw {:?}",
        seen.all()
    );
}

#[tokio::test]
async fn a_rename_reaches_a_watching_client_as_both_halves() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let seen = Seen::default();
    let _watch = client.watch(seen.record());
    settle().await;

    std::fs::rename(
        fixture.vault_path("notes.txt"),
        fixture.vault_path("renamed.txt"),
    )
    .unwrap();

    assert!(
        seen.wait_for(appeared("renamed.txt")).await,
        "saw {:?}",
        seen.all()
    );
    assert!(
        seen.wait_for(removed("notes.txt")).await,
        "the folder the file left also has to be refreshed; saw {:?}",
        seen.all()
    );
}

/// A change made *through* the app propagates the same way as one made behind
/// its back, because both are only ever noticed on the disk.
#[tokio::test]
async fn a_deletion_through_the_app_reaches_every_other_client() {
    let fixture = start_host().await;
    let watcher = fixture.paired_client().await;
    let actor = fixture.paired_client().await;

    let seen = Seen::default();
    let _watch = watcher.watch(seen.record());
    settle().await;

    actor.remove("notes.txt", false).await.unwrap();

    assert!(
        seen.wait_for(removed("notes.txt")).await,
        "saw {:?}",
        seen.all()
    );
}

#[tokio::test]
async fn two_clients_both_hear_the_same_change() {
    let fixture = start_host().await;
    let first = fixture.paired_client().await;
    let second = fixture.paired_client().await;

    let a = Seen::default();
    let b = Seen::default();
    let _one = first.watch(a.record());
    let _two = second.watch(b.record());
    settle().await;

    std::fs::remove_file(fixture.vault_path("notes.txt")).unwrap();

    assert!(
        a.wait_for(removed("notes.txt")).await,
        "first: {:?}",
        a.all()
    );
    assert!(
        b.wait_for(removed("notes.txt")).await,
        "second: {:?}",
        b.all()
    );
}

#[tokio::test]
async fn dropping_the_handle_stops_the_watch() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let seen = Seen::default();
    let watch = client.watch(seen.record());
    settle().await;
    drop(watch);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let before = seen.all().len();
    std::fs::remove_file(fixture.vault_path("notes.txt")).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert_eq!(
        seen.all().len(),
        before,
        "a dropped handle must stop delivering"
    );
}

// ---------------------------------------------------------------------------
// The media library
// ---------------------------------------------------------------------------

/// Big enough to count as a feature rather than a sample.
fn put_feature(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    file.set_len(basalt_host::media::index::MIN_FEATURE_BYTES + 1)
        .unwrap();
}

/// Waits for a scan to finish and returns what it found.
async fn library_of(client: &Arc<Basalt>, expect: usize) -> Vec<LibraryItem> {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut last = Vec::new();
    while std::time::Instant::now() < deadline {
        let response = client.library(0).await.expect("the library answers");
        if !response.scanning {
            last = response.items.unwrap_or_default();
            if last.len() == expect {
                return last;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    last
}

#[tokio::test]
async fn the_library_is_off_until_it_is_switched_on() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    let client = fixture.paired_client().await;

    let response = client.library(0).await.unwrap();
    assert!(
        !response.enabled,
        "scanning somebody's drive is not something to start doing uninvited"
    );
    assert!(response.items.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn switching_the_library_on_finds_films_and_series() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.1080p.BluRay-SPARKS.mkv"));
    put_feature(&fixture.vault_path("shows/Breaking Bad/Season 01/S01E01.mkv"));
    put_feature(&fixture.vault_path("shows/Breaking Bad/Season 01/S01E02.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();

    let items = library_of(&client, 2).await;
    assert_eq!(items.len(), 2, "one film and one series, got {items:?}");

    let film = items
        .iter()
        .find(|i| i.kind == LibraryKind::Film)
        .expect("a film");
    assert_eq!(film.title, "Arrival");
    assert_eq!(film.year, Some(2016));

    let show = items
        .iter()
        .find(|i| i.kind == LibraryKind::Series)
        .expect("a series");
    assert_eq!(show.title, "Breaking Bad");
    assert_eq!(show.seasons[0].episodes.len(), 2);
}

/// The reindexing asked for by name: what is gone has to go.
#[tokio::test]
async fn a_deleted_film_leaves_the_library_on_the_next_scan() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    put_feature(&fixture.vault_path("films/Dune.2021.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();
    assert_eq!(library_of(&client, 2).await.len(), 2);

    std::fs::remove_file(fixture.vault_path("films/Dune.2021.mkv")).unwrap();
    fixture.host.start_scan();

    let after = library_of(&client, 1).await;
    assert_eq!(after.len(), 1, "the deleted film is still listed");
    assert_eq!(after[0].title, "Arrival");
}

#[tokio::test]
async fn a_new_film_joins_the_library_on_the_next_scan() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();
    assert_eq!(library_of(&client, 1).await.len(), 1);

    put_feature(&fixture.vault_path("films/Dune.2021.mkv"));
    fixture.host.start_scan();

    assert_eq!(library_of(&client, 2).await.len(), 2);
}

#[tokio::test]
async fn switching_the_library_off_empties_it() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();
    assert_eq!(library_of(&client, 1).await.len(), 1);

    fixture.host.set_library_enabled(false).await.unwrap();
    let response = client.library(0).await.unwrap();
    assert!(!response.enabled);
    assert!(response.items.unwrap_or_default().is_empty());
}

/// Polling has to be cheap: an unchanged index sends nothing back.
#[tokio::test]
async fn asking_again_with_the_current_revision_sends_no_items() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();
    library_of(&client, 1).await;

    let first = client.library(0).await.unwrap();
    assert!(first.items.is_some());

    let again = client.library(first.revision).await.unwrap();
    assert_eq!(again.revision, first.revision);
    assert!(
        again.items.is_none(),
        "an unchanged index must not be sent twice"
    );
}

#[tokio::test]
async fn a_watching_client_is_told_when_the_library_changes() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    let client = fixture.paired_client().await;

    let seen = Seen::default();
    let _watch = client.watch(seen.record());
    settle().await;

    fixture.host.set_library_enabled(true).await.unwrap();

    assert!(
        seen.wait_for(|change| matches!(change, Change::LibraryChanged))
            .await,
        "saw {:?}",
        seen.all()
    );
}
