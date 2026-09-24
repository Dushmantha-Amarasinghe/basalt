//! A host and its drive, over the drive's whole life: missing at startup,
//! unplugged while serving, plugged back in, and swapped for another.
//!
//! Real timers and a real filesystem watcher, so these take seconds rather
//! than milliseconds. They are the only way to see the failures they guard:
//! a host that exited on launch because its USB drive was out, and a library
//! that went on watching a drive it no longer served.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use basalt_host::config::HostConfig;
use basalt_host::server::Host;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A folder of its own, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "basalt-drive-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A host whose chosen drive is `vault`, which may or may not exist yet.
fn host_on(scratch: &Scratch, vault: &Path, library: bool) -> Arc<Host> {
    let config_path = scratch.path("host.json");
    let mut config = HostConfig::create("test-host").unwrap();
    config.vault_path = Some(vault.to_path_buf());
    config.vault_name = "Media".into();
    config.library_enabled = library;
    config.save(&config_path).unwrap();
    Host::new(config, config_path).expect("a host starts whatever state its drive is in")
}

/// Writes a file big enough to count as a feature, without writing its bytes.
fn put_feature(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    file.set_len(basalt_host::media::index::MIN_FEATURE_BYTES + 1)
        .unwrap();
}

/// Waits for something to become true, or fails saying what never did.
async fn eventually(what: &str, within: Duration, mut check: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("{what} did not happen within {within:?}");
}

#[tokio::test]
async fn a_missing_drive_does_not_stop_the_host() {
    let scratch = Scratch::new();
    let gone = scratch.path("unplugged");
    let host = host_on(&scratch, &gone, false);

    let status = host.status(true).await;
    let vault = status.vault.expect("the chosen drive is still reported");
    assert!(!vault.available);
    assert_eq!(vault.name, "Media");
    assert!(host.vault().await.is_none());
}

#[tokio::test]
async fn a_drive_that_arrives_after_startup_is_served() {
    let scratch = Scratch::new();
    let later = scratch.path("plugged-in-later");
    let host = host_on(&scratch, &later, false);
    host.keep_drive_attached();

    std::fs::create_dir_all(&later).unwrap();
    std::fs::write(later.join("notes.txt"), b"hello").unwrap();

    eventually(
        "the drive being picked up",
        Duration::from_secs(10),
        async || host.vault().await.is_some(),
    )
    .await;
    let vault = host.status(true).await.vault.unwrap();
    assert!(vault.available);
}

#[tokio::test]
async fn a_drive_unplugged_while_serving_is_let_go_and_taken_back() {
    let scratch = Scratch::new();
    let drive = scratch.path("usb");
    std::fs::create_dir_all(&drive).unwrap();
    let host = host_on(&scratch, &drive, false);
    host.keep_drive_attached();
    assert!(host.vault().await.is_some());

    std::fs::remove_dir_all(&drive).unwrap();
    eventually(
        "the drive being let go",
        Duration::from_secs(10),
        async || host.vault().await.is_none(),
    )
    .await;
    assert!(!host.status(true).await.vault.unwrap().available);

    std::fs::create_dir_all(&drive).unwrap();
    eventually(
        "the drive being taken back",
        Duration::from_secs(10),
        async || host.vault().await.is_some(),
    )
    .await;
    assert!(host.status(true).await.vault.unwrap().available);
}

/// The path shown in the host's window is the one a person would write, not
/// Windows' canonical form of it.
#[tokio::test]
async fn the_drive_is_shown_by_its_ordinary_path() {
    let scratch = Scratch::new();
    let drive = scratch.path("share");
    std::fs::create_dir_all(&drive).unwrap();
    let host = host_on(&scratch, &drive, false);
    let shown = host.status(true).await.vault.unwrap().path;
    assert!(!shown.starts_with(r"\\?\"), "{shown}");
}

/// A video landing on a drive chosen after the host started is filed without
/// anybody pressing Scan.
///
/// This is the one that was broken: the loop that keeps the library current
/// subscribed to the first drive's watcher and never let go, so the drive
/// actually being shared got nothing until a restart.
#[tokio::test]
async fn the_library_follows_the_drive_it_is_switched_to() {
    let scratch = Scratch::new();
    let first = scratch.path("first");
    let second = scratch.path("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();

    let host = host_on(&scratch, &first, true);
    host.keep_library_current();
    host.set_vault(&second, "Second").await.unwrap();

    // Once the switch has settled, so only the watcher can find what comes
    // next — not a scan that happened to still be walking.
    eventually(
        "the switch's scan finishing",
        Duration::from_secs(20),
        async || !host.is_scanning(),
    )
    .await;
    // Long enough for any scan the switch queued to have run too.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!host.is_scanning());
    assert!(host.library_items().is_empty());

    put_feature(&second.join("Downloads/Blade.Runner.2049.2017.1080p.BluRay.x264.mkv"));
    eventually(
        "the new film being filed",
        Duration::from_secs(30),
        async || {
            host.library_items()
                .iter()
                .any(|item| item.title == "Blade Runner 2049")
        },
    )
    .await;
}

/// The same for a host that had no drive at all when it started — whose loop
/// used to end before the drive was ever chosen.
#[tokio::test]
async fn the_library_follows_a_drive_chosen_after_startup() {
    let scratch = Scratch::new();
    let config_path = scratch.path("host.json");
    let mut config = HostConfig::create("test-host").unwrap();
    config.library_enabled = true;
    config.save(&config_path).unwrap();
    let host = Host::new(config, config_path).unwrap();
    host.keep_library_current();

    let drive = scratch.path("chosen");
    std::fs::create_dir_all(&drive).unwrap();
    host.set_vault(&drive, "Chosen").await.unwrap();
    eventually(
        "the first scan finishing",
        Duration::from_secs(20),
        async || !host.is_scanning(),
    )
    .await;
    // Long enough for any scan the switch queued to have run too.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!host.is_scanning());

    put_feature(&drive.join("Films/Blade Runner 2049 (2017)/Blade Runner 2049 (2017).mkv"));
    eventually(
        "the new film being filed",
        Duration::from_secs(30),
        async || {
            host.library_items()
                .iter()
                .any(|item| item.title == "Blade Runner 2049")
        },
    )
    .await;
}
