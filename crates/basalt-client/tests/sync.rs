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

/// A different made-up word for every number: 0 is "Ba", 1 is "Be", and so on.
fn made_up(mut n: usize) -> String {
    const SYLLABLES: [&str; 20] = [
        "ba", "be", "bo", "da", "de", "do", "ka", "ke", "ko", "la", "le", "lo", "ma", "me", "mo",
        "ra", "re", "ro", "sa", "so",
    ];
    let mut word = String::new();
    loop {
        word.push_str(SYLLABLES[n % SYLLABLES.len()]);
        n /= SYLLABLES.len();
        if n == 0 {
            break;
        }
    }
    let mut chars = word.chars();
    let first = chars.next().unwrap().to_ascii_uppercase();
    std::iter::once(first).chain(chars).collect()
}

/// A feature-sized file that takes no space on the disk: marked sparse
/// before it is lengthened, as NTFS otherwise allocates every byte.
fn sparse_feature(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path).unwrap();
    #[cfg(windows)]
    {
        let marked = std::process::Command::new("fsutil")
            .args(["sparse", "setflag"])
            .arg(path)
            .output()
            .unwrap();
        assert!(marked.status.success(), "fsutil sparse setflag");
    }
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_len(basalt_host::media::index::MIN_FEATURE_BYTES + 1)
        .unwrap();
    path.to_path_buf()
}

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

/// The status has to come back.
///
/// It did not. `library_status` wrote two `self.config.lock()` calls as field
/// values of one struct literal, and a guard built inline that way is a
/// temporary that lives to the end of the whole statement — so the second lock
/// waited on the first, on the same thread, forever.
///
/// The damage was not one slow call. The window asks for this every two
/// seconds, so it wedged a thread each time while holding `library` and
/// `registry`: scans could no longer save their index, paired devices could no
/// longer authenticate, and eventually the app stopped responding altogether.
/// Every test in this file talked to the host over the protocol, and none of
/// them ever asked it for the status, which is how it shipped.
///
/// Run on a plain OS thread with its own runtime, and waited for with
/// `recv_timeout` rather than `tokio::time::timeout`.
///
/// That detail is load-bearing. The first version of this test used
/// `tokio::time::timeout` on a spawned task, and against the bug it hung
/// forever instead of failing: a thread deadlocked inside a worker never
/// returns to the scheduler, so it never hands back tokio's time driver and the
/// timeout it was supposed to trip never fires. `recv_timeout` blocks on an OS
/// primitive and cannot be starved that way.
///
/// Against the bug this reports `the status deadlocked on attempt 1` within ten
/// seconds, and the binary then hangs at exit, because the wedged thread can
/// never be reclaimed. The diagnosis is printed long before that, which is the
/// part that matters; there is no way to un-deadlock a thread to tidy up after.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asking_for_the_status_answers() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    fixture.host.enable_library_for_test();

    for attempt in 1..=2 {
        let host = Arc::clone(&fixture.host);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the call");
            let _ = tx.send(runtime.block_on(host.status(true)));
        });

        let status = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("the status deadlocked on attempt {attempt}"));
        assert!(status.library.enabled);
    }

    // The locks it takes have to be free afterwards, or the freeze simply moves
    // to whatever asks next. These are the two that were held.
    assert!(fixture.host.devices().is_empty());
    assert_eq!(fixture.host.library_items().len(), 0);
}

/// The index has to reach the disk.
///
/// Found by running the host over a real drive and looking in its config
/// folder: the scan logged success, the items were there over the wire, and no
/// `library-*.json` was ever written. Nothing caught it because every test
/// asked the running host what it had found, which it answers from memory.
///
/// The cost of getting this wrong is a full rescan of the whole drive at every
/// start — twenty-two seconds on the 500 GB disk this was found on.
#[tokio::test]
async fn a_scan_writes_the_index_to_disk() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    fixture.host.set_library_enabled(true).await.unwrap();

    let client = fixture.paired_client().await;
    assert_eq!(library_of(&client, 1).await.len(), 1);

    // The client is only told after the scan finishes, and the scan saves
    // before it announces, so by here the file is either there or never coming.
    let path = basalt_host::media::index::index_path(&fixture.dir, &fixture.dir.join("vault"));
    assert!(path.exists(), "no index written to {}", path.display());

    let saved = basalt_host::media::index::Library::load(&path);
    assert_eq!(
        saved.items.len(),
        1,
        "the index on disk has to hold what the scan found"
    );
    assert_eq!(saved.items[0].title, "Arrival");
}

/// The bug this exists for: a host restarted with the library already on used
/// the index it had saved and never looked again, so anything added while it
/// was off stayed invisible until somebody pressed a button.
#[tokio::test]
async fn a_host_that_starts_with_the_library_on_scans_without_being_asked() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));

    // Enabled directly on the config, as a restart would find it — not via
    // `set_library_enabled`, which scans as a side effect and would hide this.
    fixture.host.enable_library_for_test();
    fixture.host.keep_library_current();

    let client = fixture.paired_client().await;
    let items = library_of(&client, 1).await;
    assert_eq!(items.len(), 1, "a restart has to rescan; got {items:?}");
    assert_eq!(items[0].title, "Arrival");
}

/// The other half: the watcher keeps listings live, and has to keep the index
/// live too, or a film copied in sits in Files and never reaches Movies.
#[tokio::test]
async fn a_film_copied_in_reaches_the_library_on_its_own() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    fixture.host.set_library_enabled(true).await.unwrap();

    let client = fixture.paired_client().await;
    assert_eq!(library_of(&client, 1).await.len(), 1);

    settle().await;
    put_feature(&fixture.vault_path("films/Dune.2021.mkv"));

    // Nothing asks for a rescan here. The host notices on its own.
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    while std::time::Instant::now() < deadline {
        let items = client.library(0).await.unwrap().items.unwrap_or_default();
        if items.len() == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the new film never reached the library");
}

#[tokio::test]
async fn a_deleted_film_leaves_the_library_on_its_own() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    put_feature(&fixture.vault_path("films/Dune.2021.mkv"));
    fixture.host.set_library_enabled(true).await.unwrap();

    let client = fixture.paired_client().await;
    assert_eq!(library_of(&client, 2).await.len(), 2);

    settle().await;
    std::fs::remove_file(fixture.vault_path("films/Dune.2021.mkv")).unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    while std::time::Instant::now() < deadline {
        let items = client.library(0).await.unwrap().items.unwrap_or_default();
        if items.len() == 1 {
            assert_eq!(items[0].title, "Arrival");
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the deleted film never left the library");
}

// ---------------------------------------------------------------------------
// Collections: Videos, Music, Photos and Recent
// ---------------------------------------------------------------------------

async fn collections_where(
    client: &Arc<Basalt>,
    want: impl Fn(&basalt_proto::msg::Collections) -> bool,
) -> basalt_proto::msg::Collections {
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let mut last = basalt_proto::msg::Collections::default();
    while std::time::Instant::now() < deadline {
        let response = client.collections(0).await.expect("the collections answer");
        if let Some(collections) = response.collections {
            if want(&collections) {
                return collections;
            }
            last = collections;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    last
}

fn names(files: &[basalt_proto::msg::MediaFile]) -> Vec<&str> {
    files.iter().map(|f| f.path.as_str()).collect()
}

/// A photo at the top of the drive never reached Photos when it arrived
/// after the device connected, and one three folders down never did at all.
/// The host sorts the whole drive, recognition on or off.
#[tokio::test]
async fn every_photo_song_and_video_is_found_at_any_depth() {
    let fixture = start_host().await;
    std::fs::create_dir_all(fixture.vault_path("Photos/2024/Trip/Day 2")).unwrap();
    std::fs::create_dir_all(fixture.vault_path("Music/Artist/Album")).unwrap();
    image::RgbImage::new(40, 30)
        .save(fixture.vault_path("Photos/2024/Trip/Day 2/beach.png"))
        .unwrap();
    std::fs::write(
        fixture.vault_path("Music/Artist/Album/01 Opening.flac"),
        b"flac",
    )
    .unwrap();
    std::fs::write(fixture.vault_path("films/clip.m2ts"), b"video").unwrap();

    let client = fixture.paired_client().await;
    assert!(!fixture.host.library_enabled(), "recognition stays off");
    let c = collections_where(&client, |c| !c.photos.is_empty()).await;

    assert_eq!(names(&c.photos), ["Photos/2024/Trip/Day 2/beach.png"]);
    assert_eq!(
        (c.photos[0].width, c.photos[0].height),
        (Some(40), Some(30))
    );
    assert_eq!(names(&c.music), ["Music/Artist/Album/01 Opening.flac"]);
    assert_eq!(names(&c.videos), ["films/clip.m2ts"]);
    assert!(
        names(&c.recent).contains(&"notes.txt"),
        "Recent is every kind of file"
    );
}

#[tokio::test]
async fn a_photo_added_or_deleted_shows_without_a_scan() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    collections_where(&client, |_| true).await;
    settle().await;

    image::RgbImage::new(8, 8)
        .save(fixture.vault_path("26_05_12_19_37_04.png"))
        .unwrap();
    let c = collections_where(&client, |c| !c.photos.is_empty()).await;
    assert_eq!(names(&c.photos), ["26_05_12_19_37_04.png"]);

    std::fs::remove_file(fixture.vault_path("26_05_12_19_37_04.png")).unwrap();
    let c = collections_where(&client, |c| c.photos.is_empty()).await;
    assert!(c.photos.is_empty(), "the deleted photo is still listed");
}

/// An upload that never got going left an empty partial file on the drive,
/// hidden and there for good. The walk tidies those away — but not one that
/// is recent enough to be resumed.
#[tokio::test]
async fn abandoned_partial_uploads_are_swept_and_recent_ones_kept() {
    let fixture = start_host().await;
    let old = fixture.vault_path(".basalt-00112233445566778899aabbccddeeff.part");
    let fresh = fixture.vault_path("films/.basalt-ffeeddccbbaa99887766554433221100.part");
    std::fs::write(&old, b"").unwrap();
    std::fs::write(&fresh, b"half a film").unwrap();
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(two_hours_ago)
        .unwrap();

    let client = fixture.paired_client().await;
    fixture.host.start_scan();
    collections_where(&client, |_| true).await;

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while old.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!old.exists(), "the abandoned empty upload is still there");
    assert!(fresh.exists(), "a resumable upload was removed");
}

#[tokio::test]
async fn devices_hear_which_sections_to_show() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert_eq!(
        client.library(0).await.unwrap().sections,
        basalt_proto::msg::Sections::default()
    );

    let chosen = basalt_proto::msg::Sections {
        music: false,
        photos: false,
        ..Default::default()
    };
    fixture.host.set_sections(chosen).await.unwrap();
    assert_eq!(client.library(0).await.unwrap().sections, chosen);
}

#[tokio::test]
async fn a_photo_thumbnail_is_made_once_and_kept() {
    let fixture = start_host().await;
    image::RgbImage::from_pixel(1200, 900, image::Rgb([30, 140, 200]))
        .save(fixture.vault_path("beach.png"))
        .unwrap();
    let client = fixture.paired_client().await;

    let first = client
        .thumbnail("beach.png", 320)
        .await
        .expect("a thumbnail");
    let picture = image::load_from_memory(&first).expect("a real image");
    assert_eq!((picture.width(), picture.height()), (320, 240));

    let cached: Vec<_> = walk_files(&fixture.dir.join("thumbs"));
    assert_eq!(cached.len(), 1, "kept for next time");
    assert_eq!(client.thumbnail("beach.png", 320).await.unwrap(), first);

    // A bigger picture, for viewing, is a separate one.
    let big = client.thumbnail("beach.png", 1600).await.unwrap();
    let picture = image::load_from_memory(&big).unwrap();
    assert_eq!(
        (picture.width(), picture.height()),
        (1200, 900),
        "never blown up"
    );
}

#[tokio::test]
async fn what_cannot_be_pictured_is_not_found() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    let err = client
        .thumbnail("notes.txt", 320)
        .await
        .expect_err("not media");
    assert_eq!(err.kind(), "notfound");
    let err = client
        .thumbnail("missing.png", 320)
        .await
        .expect_err("not there");
    assert_eq!(err.kind(), "notfound");
    // Nor can a path be used to picture something outside the drive.
    assert!(client.thumbnail("../host.json", 320).await.is_err());
}

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// The HD / 4K tag on a card: the size reaches devices with the library,
/// read from the name until the file itself has been measured.
#[tokio::test]
async fn a_film_and_its_episodes_arrive_with_their_picture_size() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.2160p.WEB-DL.mkv"));
    std::fs::create_dir_all(fixture.vault_path("tv/Breaking Bad/Season 01")).unwrap();
    put_feature(&fixture.vault_path("tv/Breaking Bad/Season 01/Breaking.Bad.S01E01.720p.mkv"));

    let client = fixture.paired_client().await;
    fixture.host.set_library_enabled(true).await.unwrap();
    let items = library_of(&client, 2).await;

    let film = items
        .iter()
        .find(|i| i.kind == LibraryKind::Film)
        .expect("the film");
    let size = film.resolution.expect("a size for the film");
    assert_eq!((size.width, size.height), (3840, 2160));

    let show = items
        .iter()
        .find(|i| i.kind == LibraryKind::Series)
        .expect("the show");
    let episode = &show.seasons[0].episodes[0];
    assert_eq!(episode.resolution.map(|r| r.height), Some(720));
}

// ---------------------------------------------------------------------------
// The host changing its mind about a device
// ---------------------------------------------------------------------------

/// Collects what a watch says besides changes.
#[derive(Clone, Default)]
struct Notices(Arc<std::sync::Mutex<Vec<String>>>);

impl Notices {
    fn record(&self) -> impl Fn(basalt_client::WatchNotice) + Send + Sync + 'static {
        let inner = Arc::clone(&self.0);
        move |notice| {
            let said = match notice {
                basalt_client::WatchNotice::Removed(e) => format!("removed: {e}"),
                basalt_client::WatchNotice::AccessChanged => "access".to_string(),
            };
            inner.lock().expect("notices lock").push(said);
        }
    }

    async fn wait_for(&self, want: impl Fn(&str) -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if self.0.lock().expect("notices lock").iter().any(|n| want(n)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }
}

// What was reported: a device made read-only went on uploading and deleting,
// and one removed went on browsing, because both were read when it connected.
#[tokio::test]
async fn a_device_made_read_only_is_refused_on_the_connections_it_already_has() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.mkdir("before").await.is_ok());
    let hash = fixture.host.devices()[0].token_hash.clone();

    assert!(fixture.host.set_writable(&hash, false).unwrap());
    let err = client.mkdir("after").await.expect_err("read-only now");
    assert_eq!(err.kind(), "denied", "got: {err}");
    assert!(client.list("").await.is_ok(), "reading goes on");

    assert!(fixture.host.set_writable(&hash, true).unwrap());
    assert!(client.mkdir("allowed-again").await.is_ok());

    assert!(fixture.host.revoke(&hash).unwrap());
    let err = client.list("").await.expect_err("removed now");
    assert_eq!(err.kind(), "unpaired", "got: {err}");
}

// Nothing happened on the device when it was removed: nobody was clicking, and
// the one connection it had open, the watch, was never told.
#[tokio::test]
async fn a_watching_device_hears_at_once_that_its_access_changed_or_it_was_removed() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    assert!(client.status().unwrap().writable);
    let hash = fixture.host.devices()[0].token_hash.clone();

    let notices = Notices::default();
    let _watch = client.watch_with(|_| {}, notices.record());
    settle().await;

    assert!(fixture.host.set_writable(&hash, false).unwrap());
    assert!(
        notices.wait_for(|n| n == "access").await,
        "made read-only, the device must hear of it"
    );
    assert!(
        !client.status().unwrap().writable,
        "and what it shows must follow"
    );

    settle().await;
    assert!(fixture.host.revoke(&hash).unwrap());
    assert!(
        notices
            .wait_for(|n| n.starts_with("removed:") && n.contains("removed this device"))
            .await,
        "removed, the device must hear of it"
    );
    assert!(
        client.known_hosts().is_empty(),
        "the pairing is dropped here"
    );
    assert!(!client.is_connected());
}

// ---------------------------------------------------------------------------
// Changing drive
// ---------------------------------------------------------------------------

/// Pairs an existing client with another host, leaving it connected there.
async fn pair_with(client: &Arc<Basalt>, fixture: &Fixture) {
    let requires_pin = client
        .begin_pairing(fixture.addr)
        .await
        .expect("pairing opens");
    let pin = requires_pin.then(|| {
        fixture
            .host
            .pending_pairings()
            .into_iter()
            .find_map(|r| r.pin)
            .expect("the host displays a PIN")
    });
    client
        .finish_pairing(pin.as_deref())
        .await
        .expect("pairing completes");
}

/// The titles a host gives a client holding `known`, scanning or not.
async fn titles_for(client: &Arc<Basalt>, known: u64) -> Option<Vec<String>> {
    let response = client.library(known).await.expect("the library answers");
    response
        .items
        .map(|items| items.into_iter().map(|i| i.title).collect())
}

// What was reported: after changing drive, Movies still showed the last
// drive's films. Here the client holds the first host's revision when it asks
// the second, which with a counter per drive was the same number: the second
// host answered "you have it", and the first host's films stayed.
#[tokio::test]
async fn another_host_never_answers_with_the_last_hosts_films() {
    let first = start_host().await;
    let second = start_host().await;
    put_feature(&first.vault_path("films/Arrival.2016.mkv"));
    put_feature(&second.vault_path("films/Dune.2021.mkv"));
    first.host.set_library_enabled(true).await.unwrap();
    second.host.set_library_enabled(true).await.unwrap();

    let client = first.paired_client().await;
    let items = library_of(&client, 1).await;
    assert_eq!(items[0].title, "Arrival");
    let held = client.library(0).await.unwrap().revision;
    let first_id = client.status().unwrap().host_id;

    // Change drive: on to the second host, without disconnecting first.
    pair_with(&client, &second).await;
    assert_eq!(library_of(&client, 1).await[0].title, "Dune");
    assert_eq!(
        titles_for(&client, held).await,
        Some(vec!["Dune".to_string()]),
        "the first host's revision must not count as having the second's films"
    );

    // And back again, holding the second's.
    let held = client.library(0).await.unwrap().revision;
    client
        .connect(&first_id, Some(&first.addr.to_string()))
        .await
        .expect("back to the first");
    assert_eq!(
        titles_for(&client, held).await,
        Some(vec!["Arrival".to_string()])
    );
}

// The same, with one host changing its own drive: its new drive's index was
// numbered like the old one's, so a client was told it already had it.
#[tokio::test]
async fn a_host_that_changes_drive_gives_the_new_drives_films() {
    let fixture = start_host().await;
    put_feature(&fixture.vault_path("films/Arrival.2016.mkv"));
    fixture.host.set_library_enabled(true).await.unwrap();
    let client = fixture.paired_client().await;
    assert_eq!(library_of(&client, 1).await[0].title, "Arrival");
    let held = client.library(0).await.unwrap().revision;

    let other = unique("sync-other-drive");
    put_feature(&other.join("films/Dune.2021.mkv"));
    fixture
        .host
        .set_vault(&other, "Other Drive")
        .await
        .expect("the host takes the new drive");

    // The scan of the new drive, then the question a watching client asks.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut titles = None;
    while std::time::Instant::now() < deadline {
        let response = client.library(held).await.expect("the library answers");
        if !response.scanning {
            titles = response
                .items
                .map(|items| items.into_iter().map(|i| i.title).collect::<Vec<_>>());
            if titles.as_deref() == Some(&["Dune".to_string()][..]) {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_dir_all(&other);
    assert_eq!(
        titles,
        Some(vec!["Dune".to_string()]),
        "a client holding the old drive's revision gets the new drive's films"
    );
}

// A drive changed while the last one was still being scanned. The scan used to
// finish anyway and put the old drive's films into the new drive's library,
// and save them under its name, until the scan after; and that scan waited out
// the usual rest first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scan_of_the_last_drive_never_lands_on_the_new_one() {
    let fixture = start_host().await;
    let base = sparse_feature(&fixture.dir.join("base.mkv"));
    // Enough folders that the walk is still going when the drive changes.
    for i in 0..1_000 {
        let film = fixture.vault_path(&format!(
            "films/The {} (2026)/The {} (2026) 1080p WEB-DL.mkv",
            made_up(i),
            made_up(i)
        ));
        std::fs::create_dir_all(film.parent().unwrap()).unwrap();
        std::fs::hard_link(&base, &film).unwrap();
        for extra in ["Featurettes", "Interviews", "Scenes", "Trailers"] {
            std::fs::create_dir(film.parent().unwrap().join(extra)).unwrap();
        }
    }
    let client = fixture.paired_client().await;

    let other = fixture.dir.join("other-drive");
    sparse_feature(&other.join("films/Dune.2021.mkv"));
    // The walk at start-up first, so the one looking for films starts at once
    // rather than being queued behind it.
    while client.library(0).await.unwrap().scanning {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    fixture.host.set_library_enabled(true).await.unwrap();
    assert!(
        fixture.host.is_scanning(),
        "the first drive is being scanned"
    );
    // Under way, rather than merely asked for.
    tokio::time::sleep(Duration::from_millis(30)).await;
    fixture
        .host
        .set_vault(&other, "Other Drive")
        .await
        .expect("the host takes the new drive");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut last = Vec::new();
    while std::time::Instant::now() < deadline {
        let response = client.library(0).await.expect("the library answers");
        last = response
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.title)
            .collect::<Vec<_>>();
        assert!(
            !last.iter().any(|t| t.starts_with("The ")),
            "the last drive's films showed on the new one: {} of them",
            last.len()
        );
        if !response.scanning && last == ["Dune"] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(last, ["Dune"], "the new drive's films, in good time");

    // And on disk, under the new drive's name, nothing of the old one.
    let saved = basalt_host::media::index::index_path(&fixture.dir, &other);
    let text = std::fs::read_to_string(&saved).unwrap_or_default();
    assert!(text.contains("Dune"), "the new drive's library is saved");
    assert!(
        !text.contains("WEB-DL"),
        "the old drive's films were saved as the new one's"
    );
}

// ---------------------------------------------------------------------------
// A large library
// ---------------------------------------------------------------------------

// Two thousand films and two hundred series of ten episodes: a big collection,
// but a real one. The scan has to finish in reasonable time, and the list the
// apps are sent has to stay a size a phone can take in.
#[tokio::test]
async fn a_large_library_scans_quickly_and_travels_light() {
    let fixture = start_host().await;
    // Four thousand two hundred files of feature size would be two hundred
    // gigabytes. They are hard links to a few sparse files instead: the same
    // size to the scanner, and almost nothing on the disk. (Windows allows
    // 1023 links to one file.)
    let bases: Vec<PathBuf> = (0..5)
        .map(|n| sparse_feature(&fixture.dir.join(format!("base-{n}.mkv"))))
        .collect();
    let mut made = 0usize;
    let mut link = |rel: String| {
        let target = fixture.vault_path(&rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::hard_link(&bases[made / 1_000], &target).unwrap();
        made += 1;
    };
    // Invented titles, so none is in the catalogue: they are kept as films
    // because they are this year's and carry release tags, which is how a new
    // download looks. Letters rather than numbers, as "Film 105" would rightly
    // read as an episode.
    for i in 0..2_000 {
        link(format!("films/The {} (2026) 1080p WEB-DL.mkv", made_up(i)));
    }
    for s in 0..200 {
        let title = format!("House of {}", made_up(s));
        for e in 1..=10 {
            link(format!("shows/{title}/Season 01/{title} S01E{e:02}.mkv"));
        }
    }
    let client = fixture.paired_client().await;

    let started = std::time::Instant::now();
    fixture.host.set_library_enabled(true).await.unwrap();
    let deadline = started + Duration::from_secs(120);
    let mut response = client.library(0).await.unwrap();
    while (response.scanning || response.items.as_ref().map_or(0, |i| i.len()) < 2_200)
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
        response = client.library(0).await.unwrap();
    }
    let took = started.elapsed();
    let items = response.items.unwrap_or_default();
    let films = items.iter().filter(|i| i.kind == LibraryKind::Film).count();
    let series = items
        .iter()
        .filter(|i| i.kind == LibraryKind::Series)
        .count();
    assert_eq!((films, series), (2_000, 200), "every film and series");
    assert!(took < Duration::from_secs(60), "the scan took {took:?}");

    let bytes = serde_json::to_vec(&items).unwrap().len();
    println!("large library: scanned in {took:?}, {bytes} bytes for the whole list");
    assert!(bytes < 8 * 1024 * 1024, "the list is {bytes} bytes");

    // Asked again with what it has: nothing to send.
    let again = client.library(response.revision).await.unwrap();
    assert!(
        again.items.is_none(),
        "an unchanged library is not sent twice"
    );
}

// ---------------------------------------------------------------------------
// Subtitles for a video played from Files
// ---------------------------------------------------------------------------

// A video played from Files, with the library switched off, and its subtitle
// nowhere near it: in Downloads, named for another release of the same film.
// It used to get no subtitles at all, even one sitting right beside it.
#[tokio::test]
async fn a_video_from_files_gets_its_subtitles_wherever_they_are() {
    let fixture = start_host().await;
    std::fs::create_dir_all(fixture.vault_path("Downloads")).unwrap();
    std::fs::create_dir_all(fixture.vault_path("Random/Stuff")).unwrap();
    std::fs::write(
        fixture.vault_path("Random/Stuff/Arrival.2016.1080p.BluRay.mkv"),
        b"not really a film",
    )
    .unwrap();
    std::fs::write(
        fixture.vault_path("Downloads/Arrival.2016.720p.WEB-DL.en.srt"),
        b"1\n00:00:01,000 --> 00:00:02,000\nHello\n",
    )
    .unwrap();
    std::fs::write(
        fixture.vault_path("Random/Stuff/Arrival.2016.1080p.BluRay.es.srt"),
        b"1\n00:00:01,000 --> 00:00:02,000\nHola\n",
    )
    .unwrap();
    std::fs::write(
        fixture.vault_path("Downloads/Some.Other.Film.2001.en.srt"),
        b"x",
    )
    .unwrap();
    let client = fixture.paired_client().await;

    // The one beside it is found whether or not a scan has run yet.
    let early = client
        .subtitles("Random/Stuff/Arrival.2016.1080p.BluRay.mkv")
        .await
        .unwrap();
    assert!(early.tracks.iter().any(|t| t.label == "Spanish"));

    // After a scan, the one in Downloads too.
    fixture.host.start_scan();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let found = client
            .subtitles("Random/Stuff/Arrival.2016.1080p.BluRay.mkv")
            .await
            .unwrap();
        if found.tracks.len() == 2 || std::time::Instant::now() > deadline {
            let labels: Vec<_> = found.tracks.iter().map(|t| t.label.as_str()).collect();
            assert_eq!(labels, ["English", "Spanish"]);
            assert!(
                !found.others.iter().any(|o| o.path.contains("Other.Film")),
                "an unrelated film's subtitle is not offered"
            );
            break;
        }
    }

    // Outside the drive is refused, as everything is.
    assert!(client.subtitles("../outside.mkv").await.is_err());
}

// ---------------------------------------------------------------------------
// Converting on the host
// ---------------------------------------------------------------------------

/// Makes a short test video with ffmpeg, or says there is no ffmpeg here.
fn test_video(at: &Path, size: &str) -> bool {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("testsrc2=size={size}:rate=24"))
        .args(["-f", "lavfi", "-i", "sine=frequency=440", "-t", "6"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
        .arg(at)
        .status()
        .is_ok_and(|s| s.success())
}

// A video converted on the host as it is watched: asked for from a point in
// it, sent as Matroska, straight through and to the end. Through the proxy,
// it is what the player opens.
#[tokio::test]
async fn a_video_is_converted_as_it_is_watched() {
    let fixture = start_host().await;
    if !test_video(
        &fixture.vault_path("Films/Big.Picture.2024.mkv"),
        "2560x1440",
    ) {
        eprintln!("no ffmpeg here; skipped");
        return;
    }
    let client = fixture.paired_client().await;

    let mut converting = match client.convert("Films/Big.Picture.2024.mkv", 2.0).await {
        Ok(converting) => converting,
        Err(e) if e.kind() == "unavailable" => {
            eprintln!("this machine cannot convert: {e}");
            return;
        }
        Err(e) => panic!("the conversion did not start: {e}"),
    };
    assert!(!converting.by.is_empty(), "it says what is converting");
    let mut film = Vec::new();
    while let Some(piece) = converting.next().await.unwrap() {
        film.extend(piece);
    }
    assert!(
        film.len() > 10_000,
        "a film came back: {} bytes",
        film.len()
    );
    assert_eq!(&film[..4], &[0x1a, 0x45, 0xdf, 0xa3], "as Matroska");

    // The same through the proxy, as the player opens it.
    let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(&client))
        .await
        .unwrap();
    let url = format!("{}?convert=1", proxy.url_for("Films/Big.Picture.2024.mkv"));
    let (head, body) = http_get(&url).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        !head.contains("Content-Length"),
        "made as it is sent: {head}"
    );
    assert_eq!(&body[..4], &[0x1a, 0x45, 0xdf, 0xa3]);
    let status = proxy.conversion("Films/Big.Picture.2024.mkv").unwrap();
    assert_eq!(status.by.as_deref(), Some(converting.by.as_str()));

    // Nothing outside the drive.
    assert!(client.convert("../outside.mkv", 0.0).await.is_err());
}

/// A plain HTTP GET, read to the end: the head, and the body.
async fn http_get(url: &str) -> (String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (authority, path) = rest.split_once('/').unwrap();
    let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
    stream
        .write_all(format!("GET /{path} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut all = Vec::new();
    stream.read_to_end(&mut all).await.unwrap();
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8_lossy(&all[..split]).to_string(),
        all[split + 4..].to_vec(),
    )
}
