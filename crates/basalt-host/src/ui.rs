//! The shapes the host's interface receives.
//!
//! Here rather than in the Tauri shell for the same reason as the client's
//! equivalent: the shell is outside the Cargo workspace, so nothing in it is
//! covered by `cargo test` or the pre-commit hook. A field renamed here and not
//! in the interface would produce an app that runs and shows nothing, with no
//! test anywhere to catch it.
//!
//! **Every struct here is `camelCase` on the wire.** Tauri converts command
//! *arguments* from camelCase to snake_case automatically; it does nothing to
//! what comes back. `token_hash` would arrive in JavaScript as `token_hash`
//! while the interface reads `tokenHash`, and every field would silently be
//! `undefined`. The tests at the bottom assert the exact key set of each, so
//! that cannot be reintroduced quietly.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

use crate::drives::Drive;
use crate::rates::Rate;
use crate::registry::{Device, PairingRequest};
use crate::traffic::DeviceTraffic;

/// The drive being served.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    pub path: String,
    pub name: String,
    pub free: u64,
    pub total: u64,
    /// Whether the drive is still there.
    ///
    /// A USB drive gets unplugged, and the host should say so plainly rather
    /// than leaving a stale gauge on screen and failing every request with a
    /// path error.
    pub available: bool,
}

/// Everything the dashboard needs to draw itself once.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_id: String,
    pub host_name: String,
    pub port: u16,
    pub require_pin: bool,
    pub start_with_windows: bool,
    pub vault: Option<VaultView>,
    /// The addresses this host can be reached on, for the user's information
    /// only — nothing has to type one in any more.
    pub addresses: Vec<String>,
    pub device_count: usize,
    /// The media index, always present so the switch can be drawn.
    pub library: LibraryStatus,
    /// Whether the serving loop is actually accepting connections.
    pub serving: bool,
    /// Why it is not, when it is not.
    ///
    /// A host that quietly fails to bind — because another copy is already
    /// running, most likely — would otherwise sit there looking healthy while
    /// no client could ever reach it.
    pub problem: Option<String>,
}

/// How the media index is getting on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    pub enabled: bool,
    /// True while a scan is running, so the interface can say so rather than
    /// looking like it found nothing.
    pub scanning: bool,
    pub films: usize,
    pub series: usize,
    /// Items the parser was not sure about, worth a person's eye.
    pub uncertain: usize,
    /// Items with a poster downloaded.
    pub with_art: usize,
    /// Whether poster downloads are switched on.
    pub posters: bool,
    /// Whether a TMDb key has been supplied at all.
    ///
    /// The key itself never leaves the host — the interface only needs to know
    /// whether one is set, so it can say so without ever displaying it.
    pub has_key: bool,
    /// Unix seconds of the last completed scan, zero if never.
    pub scanned_at: i64,
}

/// A drive offered on the setup screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveView {
    pub path: String,
    pub name: String,
    /// The bare volume label, empty when it has none.
    ///
    /// Separate from `name` because they are for different things: the list
    /// needs "Films (E:)" to be unambiguous, while the share is better called
    /// "Films" — the drive letter is this machine's business, not the
    /// business of a laptop across the room.
    pub label: String,
    pub kind: String,
    pub free: u64,
    pub total: u64,
    /// False for an empty card reader slot or a disconnected network drive.
    pub ready: bool,
}

impl From<Drive> for DriveView {
    fn from(drive: Drive) -> Self {
        Self {
            name: drive.display_name(),
            // `total == 0` is how Windows reports a slot with no medium in it.
            ready: drive.total > 0,
            path: drive.path.to_string_lossy().into_owned(),
            label: drive.label.trim().to_string(),
            kind: drive.kind.to_string(),
            free: drive.free,
            total: drive.total,
        }
    }
}

/// One paired device, with what it has moved and how fast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    /// The device's token hash, which is its identity everywhere else too.
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    pub last_seen: i64,
    pub writable: bool,
    pub online: bool,
    pub connections: u32,
    pub sent: u64,
    pub received: u64,
    pub send_rate: f64,
    pub receive_rate: f64,
}

/// A device asking to be let in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    pub id: String,
    pub device_name: String,
    /// `None` when the host is not asking for a PIN, in which case there is
    /// nothing to read across and the request grants itself.
    pub pin: Option<String>,
    pub seconds_left: u64,
}

impl PairingView {
    pub fn new(request: &PairingRequest, now: Instant) -> Self {
        Self {
            id: request.id.clone(),
            device_name: request.device_name.clone(),
            pin: request.pin.clone(),
            seconds_left: request.remaining(now).as_secs(),
        }
    }
}

/// Joins the device list with its counters and its current speeds.
///
/// Three sources, keyed by token hash: the registry says who exists, traffic
/// says what they have moved, rates say how fast right now. A device with no
/// entry in the other two has simply never connected since the host started,
/// which is zeros rather than an omission.
pub fn devices_view(
    devices: &[Device],
    traffic: &HashMap<String, DeviceTraffic>,
    rates: &HashMap<String, Rate>,
) -> Vec<DeviceView> {
    devices
        .iter()
        .map(|device| {
            let moved = traffic.get(&device.token_hash).copied().unwrap_or_default();
            let rate = rates.get(&device.token_hash).copied().unwrap_or_default();
            DeviceView {
                id: device.token_hash.clone(),
                name: device.name.clone(),
                paired_at: device.paired_at,
                last_seen: device.last_seen,
                writable: device.writable,
                // An open connection is the only honest definition of online:
                // `last_seen` only says when a device authenticated, and a
                // laptop that closed its lid an hour ago would still look
                // recent.
                online: moved.connections > 0,
                connections: moved.connections,
                sent: moved.sent,
                received: moved.received,
                send_rate: rate.send,
                receive_rate: rate.receive,
            }
        })
        .collect()
}

/// An error the interface can branch on.
#[derive(Debug, Clone, Serialize)]
pub struct UiError {
    pub kind: String,
    pub message: String,
}

impl From<crate::HostError> for UiError {
    fn from(e: crate::HostError) -> Self {
        Self {
            kind: match &e {
                crate::HostError::NotFound(_) => "notfound",
                crate::HostError::Denied(_) => "denied",
                crate::HostError::PairingRefused(_) => "pairing",
                crate::HostError::Exists(_) => "exists",
                _ => "error",
            }
            .to_string(),
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn keys(value: &impl Serialize) -> Vec<String> {
        let json = serde_json::to_value(value).unwrap();
        let mut keys: Vec<String> = json
            .as_object()
            .expect("these are all structs")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    fn device(hash: &str, name: &str) -> Device {
        Device {
            token_hash: hash.into(),
            name: name.into(),
            paired_at: 1_700_000_000,
            last_seen: 1_700_000_500,
            writable: true,
            device_id: String::new(),
            named_by_host: false,
        }
    }

    // -----------------------------------------------------------------------
    // The wire contract
    // -----------------------------------------------------------------------

    #[test]
    fn the_status_is_camel_case() {
        let status = HostStatus {
            host_id: "aa".into(),
            host_name: "Laptop B".into(),
            port: 7799,
            require_pin: true,
            start_with_windows: false,
            vault: None,
            addresses: vec!["192.168.1.90".into()],
            device_count: 1,
            library: LibraryStatus {
                enabled: false,
                scanning: false,
                films: 0,
                series: 0,
                uncertain: 0,
                with_art: 0,
                posters: false,
                has_key: false,
                scanned_at: 0,
            },
            serving: true,
            problem: None,
        };
        assert_eq!(
            keys(&status),
            [
                "addresses",
                "deviceCount",
                "hostId",
                "hostName",
                "library",
                "port",
                "problem",
                "requirePin",
                "serving",
                "startWithWindows",
                "vault",
            ]
        );
    }

    #[test]
    fn the_library_status_is_camel_case() {
        let status = LibraryStatus {
            enabled: true,
            scanning: true,
            films: 1,
            series: 2,
            uncertain: 3,
            with_art: 1,
            posters: true,
            has_key: true,
            scanned_at: 4,
        };
        assert_eq!(
            keys(&status),
            [
                "enabled",
                "films",
                "hasKey",
                "posters",
                "scannedAt",
                "scanning",
                "series",
                "uncertain",
                "withArt"
            ]
        );
    }

    #[test]
    fn the_vault_is_camel_case() {
        let vault = VaultView {
            path: "E:\\".into(),
            name: "Films".into(),
            free: 1,
            total: 2,
            available: true,
        };
        assert_eq!(keys(&vault), ["available", "free", "name", "path", "total"]);
    }

    #[test]
    fn a_drive_is_camel_case() {
        let view = DriveView::from(Drive {
            path: PathBuf::from("E:\\"),
            label: "Films".into(),
            kind: "removable",
            free: 1,
            total: 2,
        });
        assert_eq!(
            keys(&view),
            ["free", "kind", "label", "name", "path", "ready", "total"]
        );
    }

    #[test]
    fn the_label_is_offered_separately_from_the_list_name() {
        let view = DriveView::from(Drive {
            path: PathBuf::from("E:\\"),
            label: "  Films  ".into(),
            kind: "removable",
            free: 1,
            total: 2,
        });
        assert_eq!(view.name, "Films (E:)", "unambiguous in a list");
        assert_eq!(view.label, "Films", "and a better name for the share");
    }

    #[test]
    fn a_drive_without_a_label_offers_an_empty_one() {
        let view = DriveView::from(Drive {
            path: PathBuf::from("E:\\"),
            label: String::new(),
            kind: "removable",
            free: 1,
            total: 2,
        });
        assert!(
            view.label.is_empty(),
            "so the interface falls back to the name"
        );
    }

    #[test]
    fn a_device_is_camel_case() {
        let views = devices_view(
            &[device("aa", "Laptop A")],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            keys(&views[0]),
            [
                "connections",
                "id",
                "lastSeen",
                "name",
                "online",
                "pairedAt",
                "receiveRate",
                "received",
                "sendRate",
                "sent",
                "writable",
            ]
        );
    }

    #[test]
    fn a_pairing_request_is_camel_case() {
        let request = PairingRequest {
            id: "r1".into(),
            device_name: "Laptop A".into(),
            device_id: String::new(),
            pin: Some("169241".into()),
            opened: Instant::now(),
            attempts: 0,
            client_nonce: "n".into(),
            server_nonce: "s".into(),
        };
        let view = PairingView::new(&request, Instant::now());
        assert_eq!(keys(&view), ["deviceName", "id", "pin", "secondsLeft"]);
    }

    // -----------------------------------------------------------------------
    // Joining the three sources
    // -----------------------------------------------------------------------

    #[test]
    fn a_device_with_an_open_connection_is_online() {
        let traffic = HashMap::from([(
            "aa".to_string(),
            DeviceTraffic {
                sent: 10,
                received: 20,
                connections: 2,
            },
        )]);
        let rates = HashMap::from([(
            "aa".to_string(),
            Rate {
                send: 1_000.0,
                receive: 5.0,
            },
        )]);

        let views = devices_view(&[device("aa", "Laptop A")], &traffic, &rates);
        assert!(views[0].online);
        assert_eq!(views[0].connections, 2);
        assert_eq!(views[0].sent, 10);
        assert_eq!(views[0].received, 20);
        assert_eq!(views[0].send_rate, 1_000.0);
        assert_eq!(views[0].receive_rate, 5.0);
    }

    // `last_seen` is not liveness: a laptop that shut its lid an hour ago still
    // has a recent timestamp, and showing it as online would be a lie the user
    // would act on.
    #[test]
    fn a_recently_seen_device_with_no_connection_is_offline() {
        let traffic = HashMap::from([(
            "aa".to_string(),
            DeviceTraffic {
                sent: 900,
                received: 0,
                connections: 0,
            },
        )]);
        let views = devices_view(&[device("aa", "Laptop A")], &traffic, &HashMap::new());
        assert!(!views[0].online);
        assert_eq!(views[0].sent, 900, "what it moved is still worth showing");
    }

    #[test]
    fn a_device_that_has_never_connected_shows_zeros_rather_than_vanishing() {
        let views = devices_view(
            &[device("aa", "Laptop A")],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(views.len(), 1);
        assert!(!views[0].online);
        assert_eq!(views[0].sent, 0);
        assert_eq!(views[0].send_rate, 0.0);
    }

    #[test]
    fn counters_belonging_to_a_removed_device_are_not_shown() {
        let traffic = HashMap::from([(
            "gone".to_string(),
            DeviceTraffic {
                sent: 1,
                received: 1,
                connections: 1,
            },
        )]);
        let views = devices_view(&[device("aa", "Laptop A")], &traffic, &HashMap::new());
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, "aa");
    }

    #[test]
    fn devices_keep_the_order_the_registry_gave_them() {
        let views = devices_view(
            &[device("aa", "First"), device("bb", "Second")],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(views[0].name, "First");
        assert_eq!(views[1].name, "Second");
    }

    // -----------------------------------------------------------------------
    // Drives
    // -----------------------------------------------------------------------

    #[test]
    fn an_empty_card_reader_slot_is_listed_but_not_ready() {
        let view = DriveView::from(Drive {
            path: PathBuf::from("F:\\"),
            label: String::new(),
            kind: "removable",
            free: 0,
            total: 0,
        });
        assert!(!view.ready);
        assert_eq!(view.name, "Removable Disk (F:)");
    }

    #[test]
    fn a_pairing_request_counts_down_in_whole_seconds() {
        let opened = Instant::now();
        let request = PairingRequest {
            id: "r1".into(),
            device_name: "Laptop A".into(),
            device_id: String::new(),
            pin: Some("169241".into()),
            opened,
            attempts: 0,
            client_nonce: "n".into(),
            server_nonce: "s".into(),
        };
        let fresh = PairingView::new(&request, opened);
        assert!(fresh.seconds_left > 0);

        let late = PairingView::new(
            &request,
            opened + basalt_net::pairing::PAIRING_WINDOW + std::time::Duration::from_secs(1),
        );
        assert_eq!(late.seconds_left, 0);
    }

    #[test]
    fn a_request_without_a_pin_says_so_rather_than_showing_an_empty_string() {
        let request = PairingRequest {
            id: "r1".into(),
            device_name: "Laptop A".into(),
            device_id: String::new(),
            pin: None,
            opened: Instant::now(),
            attempts: 0,
            client_nonce: "n".into(),
            server_nonce: "s".into(),
        };
        let json = serde_json::to_value(PairingView::new(&request, Instant::now())).unwrap();
        assert!(json["pin"].is_null());
    }

    // -----------------------------------------------------------------------
    // Errors
    // -----------------------------------------------------------------------

    #[test]
    fn errors_carry_a_tag_the_interface_can_branch_on() {
        let denied = UiError::from(crate::HostError::Denied("nope".into()));
        assert_eq!(denied.kind, "denied");
        assert_eq!(
            denied.message,
            format!("{}", crate::HostError::Denied("nope".into()))
        );

        let missing = UiError::from(crate::HostError::NotFound("x".into()));
        assert_eq!(missing.kind, "notfound");
    }
}
