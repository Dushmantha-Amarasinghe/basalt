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

/// Two devices on their own keep separate places: sharing one is what a
/// profile is for.
#[tokio::test]
async fn devices_on_their_own_keep_separate_histories() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let tv = fixture.paired_client().await;

    laptop
        .progress(at("films/a.mkv", 0.4, 7200.0))
        .await
        .unwrap();
    tv.progress(at("films/b.mkv", 0.7, 3600.0)).await.unwrap();

    let on_laptop = laptop.progress(ProgressRequest::default()).await.unwrap();
    let on_tv = tv.progress(ProgressRequest::default()).await.unwrap();
    assert_eq!(on_laptop.len(), 1);
    assert_eq!(on_laptop[0].path, "films/a.mkv");
    assert_eq!(on_tv.len(), 1);
    assert_eq!(on_tv[0].path, "films/b.mkv");
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

    let device = restarted.devices()[0].key().to_string();
    let entries = restarted
        .progress(ProgressRequest::default(), Some(&device))
        .entries;
    assert_eq!(entries.len(), 1, "a restart must not lose the resume point");
    assert!((entries[0].fraction - 0.42).abs() < 1e-9);
}

/// A device keeps its history when it pairs again, because it is filed under
/// the device's own id rather than the token that pairing replaces.
#[tokio::test]
async fn a_device_that_pairs_again_keeps_its_history() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    laptop
        .progress(at("films/a.mkv", 0.4, 7200.0))
        .await
        .unwrap();

    // Pairing again from the same store: same device, new token.
    laptop.disconnect().await;
    let requires_pin = laptop.begin_pairing(fixture.addr).await.unwrap();
    let pin = requires_pin.then(|| {
        fixture
            .host
            .pending_pairings()
            .into_iter()
            .find_map(|r| r.pin)
            .unwrap()
    });
    laptop.finish_pairing(pin.as_deref()).await.unwrap();

    let seen = laptop.progress(ProgressRequest::default()).await.unwrap();
    assert_eq!(seen.len(), 1, "its place is still there");
    assert_eq!(fixture.host.devices().len(), 1);
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

/// The reason profiles exist: a film started on one device is carried on from
/// another, by the same person.
#[tokio::test]
async fn a_profile_carries_a_place_from_one_device_to_another() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let tv = fixture.paired_client().await;

    let maya = laptop
        .create_profile("Maya", "4821", 2, false)
        .await
        .unwrap();
    assert_eq!(maya.name, "Maya");
    laptop
        .progress(at("films/a.mkv", 0.55, 7200.0))
        .await
        .unwrap();

    // The television on its own sees nothing of it.
    assert!(
        tv.progress(ProgressRequest::default())
            .await
            .unwrap()
            .is_empty()
    );

    tv.sign_in_profile(&maya.id, "4821", false).await.unwrap();
    let seen = tv.progress(ProgressRequest::default()).await.unwrap();
    assert_eq!(seen.len(), 1);
    assert!((seen[0].fraction - 0.55).abs() < 1e-9);
}

#[tokio::test]
async fn every_connection_acts_for_the_profile() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    laptop
        .create_profile("Maya", "4821", 0, false)
        .await
        .unwrap();

    // Several at once, so the pool opens several connections, and each has
    // to act for Maya rather than for the device.
    let reports = (0..6).map(|i| {
        let laptop = Arc::clone(&laptop);
        tokio::spawn(async move {
            laptop
                .progress(at(&format!("films/{i}.mkv"), 0.3, 1000.0))
                .await
                .unwrap()
        })
    });
    for report in reports {
        report.await.unwrap();
    }
    let seen = laptop.progress(ProgressRequest::default()).await.unwrap();
    assert_eq!(seen.len(), 6);

    laptop.sign_out_profile().await.unwrap();
    assert!(
        laptop
            .progress(ProgressRequest::default())
            .await
            .unwrap()
            .is_empty(),
        "on its own, the device has its own, empty, history"
    );
}

#[tokio::test]
async fn a_wrong_pin_is_refused() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let tv = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, false)
        .await
        .unwrap();

    let err = tv
        .sign_in_profile(&maya.id, "1111", false)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "denied", "{err}");
    assert!(tv.identity().await.profile.is_none());
}

#[tokio::test]
async fn profiles_are_listed_by_name_and_never_with_a_pin() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    laptop
        .create_profile("Maya", "4821", 3, false)
        .await
        .unwrap();
    laptop
        .create_profile("Sam", "9090", 5, false)
        .await
        .unwrap();

    let tv = fixture.paired_client().await;
    let listed = tv.profiles().await.unwrap();
    let names: Vec<_> = listed.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Maya", "Sam"]);
    let json = serde_json::to_string(&listed).unwrap();
    assert!(!json.contains("4821") && !json.contains("argon2"), "{json}");
}

#[tokio::test]
async fn a_remembered_profile_is_signed_back_in_after_reconnecting() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, true)
        .await
        .unwrap();
    laptop
        .progress(at("films/a.mkv", 0.2, 1000.0))
        .await
        .unwrap();

    laptop.disconnect().await;
    laptop.connect_saved().await.unwrap();
    let identity = laptop.identity().await;
    assert_eq!(identity.profile.map(|p| p.id), Some(maya.id));
    assert!(!identity.choose, "nothing to ask");
    assert_eq!(
        laptop
            .progress(ProgressRequest::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_sign_in_not_remembered_asks_again_after_reconnecting() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, false)
        .await
        .unwrap();

    laptop.disconnect().await;
    laptop.connect_saved().await.unwrap();
    let identity = laptop.identity().await;
    assert!(identity.profile.is_none());
    assert!(identity.choose);
    assert_eq!(identity.last_profile, Some(maya.id), "shown first");
}

#[tokio::test]
async fn always_as_this_device_stops_asking() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    laptop.continue_as_device(true).await.unwrap();

    laptop.disconnect().await;
    laptop.connect_saved().await.unwrap();
    let identity = laptop.identity().await;
    assert!(identity.profile.is_none());
    assert!(!identity.choose);
}

#[tokio::test]
async fn a_profile_removed_on_the_host_signs_the_device_out() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, true)
        .await
        .unwrap();
    laptop
        .progress(at("films/a.mkv", 0.2, 1000.0))
        .await
        .unwrap();

    assert!(fixture.host.remove_profile(&maya.id).unwrap());
    let identity = laptop.identity().await;
    assert!(identity.profile.is_none());
    assert!(
        identity.ended && identity.choose,
        "the app asks again, and says why"
    );
    // Carrying on works, as the device.
    assert!(
        laptop
            .progress(ProgressRequest::default())
            .await
            .unwrap()
            .is_empty()
    );

    // And it is not signed back in next time.
    laptop.disconnect().await;
    laptop.connect_saved().await.unwrap();
    assert!(laptop.identity().await.profile.is_none());
}

#[tokio::test]
async fn a_reset_pin_is_chosen_again_at_the_next_sign_in() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, true)
        .await
        .unwrap();

    assert!(fixture.host.reset_profile_pin(&maya.id).unwrap());
    assert!(laptop.identity().await.ended);
    let listed = laptop.profiles().await.unwrap();
    assert!(!listed[0].has_pin);

    laptop
        .sign_in_profile(&maya.id, "2468", true)
        .await
        .unwrap();
    laptop.sign_out_profile().await.unwrap();
    assert!(
        laptop
            .sign_in_profile(&maya.id, "4821", false)
            .await
            .is_err()
    );
    assert!(
        laptop
            .sign_in_profile(&maya.id, "2468", false)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn stars_follow_the_profile() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let tv = fixture.paired_client().await;

    // On its own, a device keeps its stars itself.
    assert_eq!(laptop.stars(None).await.unwrap_err().kind(), "denied");

    let maya = laptop
        .create_profile("Maya", "4821", 0, false)
        .await
        .unwrap();
    let star = basalt_proto::msg::Star {
        path: "films/a.mkv".into(),
        name: "a.mkv".into(),
        kind: "file".into(),
    };
    laptop.stars(Some(vec![star.clone()])).await.unwrap();

    tv.sign_in_profile(&maya.id, "4821", false).await.unwrap();
    assert_eq!(tv.stars(None).await.unwrap(), vec![star]);
}

#[tokio::test]
async fn the_host_lists_each_profile_with_the_devices_signed_in() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let tv = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 0, true)
        .await
        .unwrap();
    tv.sign_in_profile(&maya.id, "4821", false).await.unwrap();

    let overview = fixture.host.profiles_overview();
    assert_eq!(overview.len(), 1);
    assert_eq!(overview[0].devices.len(), 2);
    assert_eq!(
        overview[0].devices.iter().filter(|d| d.remembered).count(),
        1
    );

    // Unpairing a device signs it out of the profile too.
    tv.forget(&tv.status().unwrap().host_id).await.unwrap();
    assert_eq!(fixture.host.profiles_overview()[0].devices.len(), 1);
}

#[tokio::test]
async fn profiles_survive_the_host_restarting() {
    let fixture = start_host().await;
    let laptop = fixture.paired_client().await;
    let maya = laptop
        .create_profile("Maya", "4821", 1, true)
        .await
        .unwrap();

    let config = HostConfig::load_or_create(&fixture.dir.join("host.json"), "x").unwrap();
    assert_eq!(config.profiles.len(), 1);
    assert_eq!(config.profiles[0].id, maya.id);
    let saved = std::fs::read_to_string(fixture.dir.join("host.json")).unwrap();
    assert!(!saved.contains("4821"), "the PIN is never written down");
}
