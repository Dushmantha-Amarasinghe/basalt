//! The shapes the interface receives.
//!
//! These live here rather than in the Tauri shell for one reason: the shell is
//! outside the Cargo workspace, so nothing in it is covered by `cargo test` or
//! by the pre-commit hook. A field renamed here and not in `api.ts` would
//! produce an app that runs, connects, and shows nothing — with no test
//! anywhere to catch it.
//!
//! **Every struct here is `camelCase` on the wire.** Tauri converts command
//! *arguments* from JavaScript's camelCase to Rust's snake_case automatically,
//! but it does not touch what comes back: a response is serialised exactly as
//! serde declares it. So `host_id` would arrive in JavaScript as `host_id`
//! while `api.ts` reads `hostId`, and every field would silently be
//! `undefined`. The tests below exist to make that impossible to reintroduce.

use serde::Serialize;

use crate::session::SessionInfo;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub host_id: Option<String>,
    pub host_name: Option<String>,
    pub vault: Option<String>,
    pub writable: bool,
    pub address: Option<String>,
    /// Whether this device has ever paired with anything. Distinguishes "the
    /// host is asleep" from "you have not set this up yet", which are entirely
    /// different screens.
    pub has_paired: bool,
    pub device_name: String,
}

impl Status {
    /// Builds the status from the live session and the paired host on disk.
    ///
    /// **Which vault this device is paired with is a property of the store,
    /// not of the connection.** Reading it from the session alone meant that
    /// the moment the host was unreachable, the app forgot which vault it even
    /// belonged to: `host_id` went null, and the "Forget this vault" button —
    /// whose entire purpose is to escape a vault you can no longer reach —
    /// silently did nothing, because it had no id to act on.
    ///
    /// So identity falls back to `saved`. Only `address` and `writable` stay
    /// session-only, and deliberately: the address is where the host answered
    /// *today*, which the settings screen says in as many words, and claiming
    /// write access this device cannot currently exercise would be a guess
    /// dressed as a fact.
    pub fn new(
        info: Option<SessionInfo>,
        saved: Option<&crate::store::KnownHost>,
        device_name: &str,
    ) -> Self {
        Self {
            connected: info.is_some(),
            host_id: info
                .as_ref()
                .map(|i| i.host_id.clone())
                .or_else(|| saved.map(|h| h.host_id.clone())),
            host_name: info
                .as_ref()
                .map(|i| i.host_name.clone())
                .or_else(|| saved.map(|h| h.host_name.clone())),
            vault: info
                .as_ref()
                .map(|i| i.vault.clone())
                .or_else(|| saved.map(|h| h.vault.clone())),
            writable: info.as_ref().is_some_and(|i| i.writable),
            address: info.as_ref().map(|i| i.address.to_string()),
            has_paired: saved.is_some(),
            device_name: device_name.to_string(),
        }
    }
}

/// A host found on the network, ready to be shown in a list.
///
/// This replaces typing an address. Everything needed to draw a row and decide
/// what tapping it does is here: whether it wants a PIN, whether it has a drive
/// to share yet, and whether this device already knows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHost {
    pub host_id: String,
    pub host_name: String,
    pub vault: String,
    /// Where it answered from, `ip:port`.
    ///
    /// Shown small, and never typed. It is here because the identity is what
    /// gets pinned and the address is only how this device reached it today.
    pub address: String,
    pub requires_pin: bool,
    /// False until somebody has chosen a drive on that machine.
    pub has_vault: bool,
    /// Whether this device has already paired with it.
    pub paired: bool,
}

impl DiscoveredHost {
    pub fn new(found: &basalt_net::discovery::Found, paired: bool) -> Self {
        Self {
            host_id: found.beacon.host_id.clone(),
            host_name: found.beacon.host_name.clone(),
            vault: found.beacon.vault.clone(),
            address: found.address.to_string(),
            requires_pin: found.beacon.requires_pin,
            has_vault: found.beacon.has_vault,
            paired,
        }
    }
}

/// Puts a discovered list in an order that does not move under the cursor.
///
/// Hosts already paired with first, then ones with a drive to share, then by
/// name. The host id breaks ties so two machines with the same name — which
/// happens, `DESKTOP-4F2A` twice — never swap places between scans.
pub fn sort_hosts(hosts: &mut [DiscoveredHost]) {
    hosts.sort_by(|a, b| {
        b.paired
            .cmp(&a.paired)
            .then(b.has_vault.cmp(&a.has_vault))
            .then_with(|| a.host_name.to_lowercase().cmp(&b.host_name.to_lowercase()))
            .then_with(|| a.host_id.cmp(&b.host_id))
    });
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEvent {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    pub transferred: u64,
    pub total: u64,
    pub status: &'static str,
    /// Bytes per second now: over the last couple of seconds, the same window
    /// the title bar's speed is measured over. See [`crate::rate`].
    pub rate: f64,
    /// Bytes per second to plan the time left on: a longer window, so the
    /// estimate is calm rather than jumping with every hiccup.
    pub eta_rate: f64,
}

/// An error the interface can branch on.
///
/// `kind` is a short machine-readable tag — `offline`, `notfound`, `denied`,
/// `unpaired` — so the UI never has to match on prose to decide whether to
/// show a reconnecting banner or an empty folder.
#[derive(Debug, Clone, Serialize)]
pub struct UiError {
    pub kind: String,
    pub message: String,
}

impl From<crate::ClientError> for UiError {
    fn from(e: crate::ClientError) -> Self {
        Self {
            kind: e.kind().to_string(),
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
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

    /// The exact field names `api.ts` reads. If this list and the struct ever
    /// disagree, the app connects and then shows nothing.
    #[test]
    fn status_matches_what_the_interface_reads() {
        let status = Status::new(None, None, "Laptop A");
        assert_eq!(
            keys(&status),
            vec![
                "address",
                "connected",
                "deviceName",
                "hasPaired",
                "hostId",
                "hostName",
                "vault",
                "writable",
            ]
        );
    }

    fn found(host_id: &str, requires_pin: bool, has_vault: bool) -> basalt_net::discovery::Found {
        basalt_net::discovery::Found {
            beacon: basalt_net::discovery::Beacon {
                host_id: host_id.into(),
                host_name: "laptop-b".into(),
                vault: "Films".into(),
                port: 7742,
                requires_pin,
                has_vault,
            },
            address: "192.168.1.90:7742".parse().unwrap(),
        }
    }

    #[test]
    fn a_discovered_host_matches_what_the_interface_reads() {
        let host = DiscoveredHost::new(&found("aa", true, true), false);
        assert_eq!(
            keys(&host),
            vec![
                "address",
                "hasVault",
                "hostId",
                "hostName",
                "paired",
                "requiresPin",
                "vault",
            ]
        );
    }

    /// The address comes from the datagram's sender, not from the payload — a
    /// host cannot talk a client into connecting somewhere else by lying in
    /// its beacon.
    #[test]
    fn a_discovered_host_carries_the_address_it_answered_from() {
        let host = DiscoveredHost::new(&found("aa", true, true), false);
        assert_eq!(host.address, "192.168.1.90:7742");
        assert_eq!(host.host_name, "laptop-b");
        assert_eq!(host.vault, "Films");
        assert!(host.requires_pin);
        assert!(host.has_vault);
        assert!(!host.paired);
    }

    #[test]
    fn a_host_with_no_drive_yet_says_so() {
        // Worth listing anyway: seeing the machine and being told it has no
        // drive is a much better answer than an empty list.
        let host = DiscoveredHost::new(&found("aa", false, false), true);
        assert!(!host.has_vault);
        assert!(!host.requires_pin);
        assert!(host.paired);
    }

    fn host(name: &str, host_id: &str, paired: bool, has_vault: bool) -> DiscoveredHost {
        DiscoveredHost {
            host_id: host_id.into(),
            host_name: name.into(),
            vault: "Vault".into(),
            address: "192.168.1.90:7742".into(),
            requires_pin: true,
            has_vault,
            paired,
        }
    }

    #[test]
    fn a_host_already_paired_with_comes_first() {
        let mut hosts = vec![
            host("Zebra", "cc", false, true),
            host("Apple", "aa", true, true),
        ];
        sort_hosts(&mut hosts);
        assert_eq!(hosts[0].host_name, "Apple");
    }

    #[test]
    fn a_host_with_no_drive_sinks_below_one_that_has_one() {
        let mut hosts = vec![
            host("Apple", "aa", false, false),
            host("Zebra", "cc", false, true),
        ];
        sort_hosts(&mut hosts);
        assert_eq!(hosts[0].host_name, "Zebra");
    }

    #[test]
    fn otherwise_they_are_in_name_order_regardless_of_case() {
        let mut hosts = vec![
            host("zebra", "cc", false, true),
            host("Apple", "aa", false, true),
            host("Mango", "bb", false, true),
        ];
        sort_hosts(&mut hosts);
        let names: Vec<_> = hosts.iter().map(|h| h.host_name.as_str()).collect();
        assert_eq!(names, ["Apple", "Mango", "zebra"]);
    }

    /// The reason the sort exists: the interface rescans on a timer, and rows
    /// that swap places between scans cannot be clicked reliably.
    #[test]
    fn two_machines_with_the_same_name_keep_a_stable_order() {
        let mut first = vec![
            host("DESKTOP-4F2A", "bb", false, true),
            host("DESKTOP-4F2A", "aa", false, true),
        ];
        let mut second = vec![
            host("DESKTOP-4F2A", "aa", false, true),
            host("DESKTOP-4F2A", "bb", false, true),
        ];
        sort_hosts(&mut first);
        sort_hosts(&mut second);
        assert_eq!(first[0].host_id, "aa");
        assert_eq!(second[0].host_id, "aa");
    }

    #[test]
    fn sorting_an_empty_list_is_fine() {
        let mut hosts: Vec<DiscoveredHost> = Vec::new();
        sort_hosts(&mut hosts);
        assert!(hosts.is_empty());
    }

    #[test]
    fn transfer_events_match_what_the_interface_reads() {
        let event = TransferEvent {
            id: "t1".into(),
            kind: "download",
            name: "a.mkv".into(),
            path: "films/a.mkv".into(),
            transferred: 1,
            total: 2,
            status: "active",
            rate: 3.0,
            eta_rate: 2.5,
        };
        assert_eq!(
            keys(&event),
            vec![
                "etaRate",
                "id",
                "kind",
                "name",
                "path",
                "rate",
                "status",
                "total",
                "transferred"
            ]
        );
    }

    #[test]
    fn errors_match_what_the_interface_reads() {
        let error = UiError::from(crate::ClientError::NotConnected);
        assert_eq!(keys(&error), vec!["kind", "message"]);
        assert_eq!(error.kind, "offline");
    }

    // Nothing may be snake_case. A single underscore is the whole bug.
    #[test]
    fn no_field_anywhere_reaches_javascript_in_snake_case() {
        let status = Status::new(None, None, "Laptop A");
        let event = TransferEvent {
            id: "t".into(),
            kind: "upload",
            name: "n".into(),
            path: "p".into(),
            transferred: 0,
            total: 0,
            status: "done",
            rate: 0.0,
            eta_rate: 0.0,
        };

        let host = DiscoveredHost::new(&found("aa", true, true), false);

        for key in keys(&status)
            .iter()
            .chain(&keys(&event))
            .chain(&keys(&host))
        {
            assert!(
                !key.contains('_'),
                "{key} would arrive undefined in JavaScript"
            );
        }
    }

    fn saved_host() -> crate::store::KnownHost {
        crate::store::KnownHost {
            host_id: "aabb".into(),
            token: "t".into(),
            vault: "Films".into(),
            host_name: "laptop-b".into(),
            last_address: Some("192.168.1.11:7742".into()),
            paired_at: 0,
            identity: Default::default(),
        }
    }

    /// Offline, it must still know *which* vault it is paired with.
    ///
    /// The regression this exists for: identity was read from the live
    /// session, so losing the host meant `host_id` went null — and "Forget
    /// this vault", whose whole job is escaping a vault you cannot reach,
    /// found no id to act on and silently did nothing. Twice reported, and
    /// invisible both times, because a button that does nothing looks
    /// identical to a button that is broken.
    #[test]
    fn a_disconnected_status_still_knows_the_vault_it_is_paired_with() {
        let host = saved_host();
        let status = Status::new(None, Some(&host), "Laptop A");
        assert!(!status.connected);
        assert!(status.has_paired);
        assert_eq!(
            status.host_id.as_deref(),
            Some("aabb"),
            "without this the vault cannot be forgotten"
        );
        assert_eq!(status.vault.as_deref(), Some("Films"));
        assert_eq!(status.host_name.as_deref(), Some("laptop-b"));
    }

    /// The other half: what it must *not* claim while disconnected.
    #[test]
    fn a_disconnected_status_invents_no_connection_details() {
        let host = saved_host();
        let status = Status::new(None, Some(&host), "Laptop A");
        assert!(
            status.address.is_none(),
            "the address is where the host answered today, not a memory"
        );
        assert!(!status.writable, "no connection means no write access");
    }

    #[test]
    fn a_status_with_nothing_paired_is_empty() {
        let status = Status::new(None, None, "Laptop A");
        assert!(!status.connected);
        assert!(!status.has_paired);
        assert!(status.host_id.is_none());
    }

    #[test]
    fn a_connected_status_carries_the_session_through() {
        let info = SessionInfo {
            host_id: "aabb".into(),
            host_name: "laptop-b".into(),
            vault: "Films".into(),
            writable: true,
            address: "192.168.1.11:7742".parse().unwrap(),
        };
        let host = saved_host();
        let status = Status::new(Some(info), Some(&host), "Laptop A");
        assert!(status.connected);
        assert_eq!(status.host_id.as_deref(), Some("aabb"));
        assert_eq!(status.vault.as_deref(), Some("Films"));
        assert_eq!(status.address.as_deref(), Some("192.168.1.11:7742"));
        assert!(status.writable);
    }
}
