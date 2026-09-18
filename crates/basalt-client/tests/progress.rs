//! Watch progress, end to end.
//!
//! The point being proved is that the resume point lives on the *host*: a
//! second device, with its own store and its own token, sees where the first
//! one got to. A test that used one client would pass with the position kept
//! in memory on that client and prove nothing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use basalt_client::Basalt;
use basalt_host::{Host, HostConfig, server};
use basalt_proto::msg::{ProgressRequest, Watched};

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
    async fn paired_client(&self) -> Arc<Basalt> {
        let store = unique("progress-store").with_extension("json");
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
    let dir = unique("progress");
    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("a.mkv"), b"x").unwrap();

    let config_path = dir.join("host.json");
    let mut config = HostConfig::create("progress-host").unwrap();
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

fn at(path: &str, fraction: f64, duration: f64) -> ProgressRequest {
    ProgressRequest {
        update: Some(Watched {
            path: path.into(),
            fraction,
            position: fraction * duration,
            duration,
            updated_at: 0,
        }),
        forget: None,
    }
}

#[tokio::test]
async fn nothing_has_been_watched_to_begin_with() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let entries = client.progress(ProgressRequest::default()).await.unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn a_position_reported_comes_straight_back() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    let entries = client
        .progress(at("films/a.mkv", 0.4, 7200.0))
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "reporting and reading is one round trip");
    assert_eq!(entries[0].path, "films/a.mkv");
    assert!((entries[0].fraction - 0.4).abs() < 1e-9);
    assert!(entries[0].updated_at > 0, "the host stamps the time");
}

/// The reason progress lives on the host at all.
#[tokio::test]
async fn a_second_device_sees_where_the_first_one_got_to() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let living_room = fixture.paired_client().await;

    laptop
        .progress(at("films/a.mkv", 0.55, 7200.0))
        .await
        .unwrap();

    let entries = living_room
        .progress(ProgressRequest::default())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert!((entries[0].fraction - 0.55).abs() < 1e-9);
}

#[tokio::test]
async fn watching_further_moves_the_point_rather_than_adding_another() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    client
        .progress(at("films/a.mkv", 0.2, 7200.0))
        .await
        .unwrap();
    let entries = client
        .progress(at("films/a.mkv", 0.8, 7200.0))
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert!((entries[0].fraction - 0.8).abs() < 1e-9);
}

#[tokio::test]
async fn forgetting_something_removes_it() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    client
        .progress(at("films/a.mkv", 0.4, 7200.0))
        .await
        .unwrap();
    let entries = client
        .progress(ProgressRequest {
            update: None,
            forget: Some("films/a.mkv".into()),
        })
        .await
        .unwrap();

    assert!(entries.is_empty());
}

#[tokio::test]
async fn several_files_come_back_newest_first() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;

    client.progress(at("first.mkv", 0.3, 100.0)).await.unwrap();
    // The host stamps in whole seconds, so two updates in the same second
    // would tie; a pause makes the ordering deterministic.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    client.progress(at("second.mkv", 0.3, 100.0)).await.unwrap();

    let entries = client.progress(ProgressRequest::default()).await.unwrap();
    assert_eq!(entries[0].path, "second.mkv");
}

/// A restart must not lose where anyone was.
#[tokio::test]
async fn progress_survives_the_host_restarting() {
    let fixture = start_host().await;
    let client = fixture.paired_client().await;
    client
        .progress(at("films/a.mkv", 0.42, 7200.0))
        .await
        .unwrap();

    // A second host over the same config directory is what a restart looks
    // like from the outside.
    let config_path = fixture.dir.join("host.json");
    let config = HostConfig::load_or_create(&config_path, "progress-host").unwrap();
    let restarted = Host::new(config, config_path).unwrap();

    let entries = restarted.progress(ProgressRequest::default()).entries;
    assert_eq!(entries.len(), 1, "a restart must not lose the resume point");
    assert!((entries[0].fraction - 0.42).abs() < 1e-9);
}
