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
    pub fn new(info: Option<SessionInfo>, has_paired: bool, device_name: &str) -> Self {
        Self {
            connected: info.is_some(),
            host_id: info.as_ref().map(|i| i.host_id.clone()),
            host_name: info.as_ref().map(|i| i.host_name.clone()),
            vault: info.as_ref().map(|i| i.vault.clone()),
            writable: info.as_ref().is_some_and(|i| i.writable),
            address: info.as_ref().map(|i| i.address.to_string()),
            has_paired,
            device_name: device_name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSummary {
    pub host_id: String,
    pub host_name: String,
    pub vault: String,
    pub pairing_open: bool,
}

impl From<basalt_proto::msg::HelloResponse> for HostSummary {
    fn from(hello: basalt_proto::msg::HelloResponse) -> Self {
        Self {
            host_id: hello.host_id,
            host_name: hello.host_name,
            vault: hello.vault,
            pairing_open: hello.pairing_open,
        }
    }
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
    /// Bytes per second over the whole transfer so far.
    pub rate: f64,
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
        let status = Status::new(None, false, "Laptop A");
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

    #[test]
    fn host_summary_matches_what_the_interface_reads() {
        let summary = HostSummary {
            host_id: "aa".into(),
            host_name: "laptop-b".into(),
            vault: "Vault".into(),
            pairing_open: true,
        };
        assert_eq!(
            keys(&summary),
            vec!["hostId", "hostName", "pairingOpen", "vault"]
        );
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
        };
        assert_eq!(
            keys(&event),
            vec![
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
        let status = Status::new(None, false, "Laptop A");
        let summary = HostSummary {
            host_id: "aa".into(),
            host_name: "b".into(),
            vault: "v".into(),
            pairing_open: false,
        };
        let event = TransferEvent {
            id: "t".into(),
            kind: "upload",
            name: "n".into(),
            path: "p".into(),
            transferred: 0,
            total: 0,
            status: "done",
            rate: 0.0,
        };

        for key in keys(&status)
            .iter()
            .chain(&keys(&summary))
            .chain(&keys(&event))
        {
            assert!(
                !key.contains('_'),
                "{key} would arrive undefined in JavaScript"
            );
        }
    }

    #[test]
    fn a_disconnected_status_says_so_without_inventing_details() {
        let status = Status::new(None, true, "Laptop A");
        assert!(!status.connected);
        assert!(status.host_id.is_none());
        assert!(status.vault.is_none());
        assert!(!status.writable, "no connection means no write access");
        assert!(status.has_paired, "but it still knows a host exists");
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
        let status = Status::new(Some(info), true, "Laptop A");
        assert!(status.connected);
        assert_eq!(status.host_id.as_deref(), Some("aabb"));
        assert_eq!(status.vault.as_deref(), Some("Films"));
        assert_eq!(status.address.as_deref(), Some("192.168.1.11:7742"));
        assert!(status.writable);
    }
}
