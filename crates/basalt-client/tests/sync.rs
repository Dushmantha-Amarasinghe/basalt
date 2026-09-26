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
