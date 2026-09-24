//! What the client remembers about hosts it has paired with.
//!
//! Two values per host and they are not the same kind of thing. The **host id**
//! is the public key it must present; losing it would mean trusting whatever
//! answers next time. The **token** is a secret that proves this device is
//! allowed in. Both are useless without the other, and the file holding them
//! deserves the same care as a password manager's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{ClientError, Result};

/// One host this device has paired with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownHost {
    /// Hex SHA-256 of the host's SPKI. Pinned; nothing else is accepted.
    pub host_id: String,
    /// The device token issued at pairing.
    pub token: String,
    pub vault: String,
    pub host_name: String,
    /// Where it answered last time, so reconnecting does not wait on
    /// discovery. Only ever a hint: the pin is what decides trust, so a stale
    /// address is a failed connection and never a wrong one.
    pub last_address: Option<String>,
    pub paired_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientStore {
    #[serde(default)]
    pub hosts: Vec<KnownHost>,
    /// What this device calls itself on the host's device list.
    #[serde(default)]
    pub device_name: Option<String>,
    /// The id this device made for itself, so hosts know it when it comes
    /// back. Made the first time the app opens and never changed.
    #[serde(default)]
    pub device_id: Option<String>,
}

impl ClientStore {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                ClientError::Config(format!(
                    "{} did not parse ({e}). Move it aside to start fresh, \
                     but this device will have to pair again.",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ClientError::Io(e)),
        }
    }

    /// Writes via a temporary file and a rename, so an interrupted save cannot
    /// leave a half-written token behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| ClientError::Config(format!("could not encode the store: {e}")))?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    pub fn find(&self, host_id: &str) -> Option<&KnownHost> {
        self.hosts.iter().find(|h| h.host_id == host_id)
    }

    /// Records a pairing, replacing any earlier one for the same host.
    ///
    /// Replacing rather than appending matters: pairing again after a host was
    /// reset would otherwise leave the old, now-rejected token in front of the
    /// new one, and every connection would fail until someone noticed.
    pub fn remember(&mut self, host: KnownHost) {
        self.hosts.retain(|h| h.host_id != host.host_id);
        self.hosts.push(host);
    }

    pub fn forget(&mut self, host_id: &str) -> bool {
        let before = self.hosts.len();
        self.hosts.retain(|h| h.host_id != host_id);
        self.hosts.len() != before
    }

    /// Updates the cached address for a host, if it is known.
    pub fn note_address(&mut self, host_id: &str, address: &str) {
        if let Some(host) = self.hosts.iter_mut().find(|h| h.host_id == host_id) {
            host.last_address = Some(address.to_string());
        }
    }

    /// The host to reconnect to on startup: the most recently paired one.
    pub fn primary(&self) -> Option<&KnownHost> {
        self.hosts.iter().max_by_key(|h| h.paired_at)
    }
}

/// Where the client keeps its store.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Basalt").join("client.json")
}

/// A fresh device id: sixteen random bytes, as hex.
pub fn new_device_id() -> Result<String> {
    basalt_net::pairing::random_nonce()
        .map(|nonce| nonce[..32].to_string())
        .map_err(|e| ClientError::Protocol(format!("no randomness available: {e}")))
}

/// This device's name, as it will appear on the host.
pub fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Basalt Client".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "basalt-store-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn host(id: &str, paired_at: i64) -> KnownHost {
        KnownHost {
            host_id: id.into(),
            token: format!("token-for-{id}"),
            vault: "Vault".into(),
            host_name: "laptop-b".into(),
            last_address: None,
            paired_at,
        }
    }

    #[test]
    fn a_missing_store_is_empty_rather_than_an_error() {
        let dir = temp_dir();
        let store = ClientStore::load(&dir.0.join("nothing-here.json")).unwrap();
        assert!(store.hosts.is_empty());
    }

    #[test]
    fn a_store_round_trips() {
        let dir = temp_dir();
        let path = dir.0.join("client.json");

        let mut store = ClientStore::default();
        store.remember(host("aa", 100));
        store.device_name = Some("Laptop A".into());
        store.save(&path).unwrap();

        let back = ClientStore::load(&path).unwrap();
        assert_eq!(back.hosts.len(), 1);
        assert_eq!(back.find("aa").unwrap().token, "token-for-aa");
        assert_eq!(back.device_name.as_deref(), Some("Laptop A"));
    }

    #[test]
    fn a_corrupt_store_is_reported_rather_than_replaced() {
        let dir = temp_dir();
        let path = dir.0.join("client.json");
        std::fs::write(&path, b"not json at all").unwrap();

        let err = ClientStore::load(&path).unwrap_err();
        assert!(format!("{err}").contains("pair again"));
        assert_eq!(std::fs::read(&path).unwrap(), b"not json at all");
    }

    // The bug this prevents: pairing again after the host was reset leaves the
    // stale token in front of the new one and nothing ever connects.
    #[test]
    fn pairing_again_replaces_the_old_token_rather_than_shadowing_it() {
        let mut store = ClientStore::default();
        store.remember(host("aa", 100));

        let mut renewed = host("aa", 200);
        renewed.token = "the-new-token".into();
        store.remember(renewed);

        assert_eq!(store.hosts.len(), 1);
        assert_eq!(store.find("aa").unwrap().token, "the-new-token");
    }

    #[test]
    fn several_hosts_coexist() {
        let mut store = ClientStore::default();
        store.remember(host("aa", 100));
        store.remember(host("bb", 200));
        assert_eq!(store.hosts.len(), 2);
        assert_eq!(store.find("aa").unwrap().token, "token-for-aa");
        assert_eq!(store.find("bb").unwrap().token, "token-for-bb");
        assert!(store.find("cc").is_none());
    }

    #[test]
    fn the_primary_host_is_the_most_recently_paired() {
        let mut store = ClientStore::default();
        store.remember(host("old", 100));
        store.remember(host("new", 500));
        store.remember(host("middle", 300));
        assert_eq!(store.primary().unwrap().host_id, "new");
    }

    #[test]
    fn an_empty_store_has_no_primary() {
        assert!(ClientStore::default().primary().is_none());
    }

    #[test]
    fn forgetting_removes_exactly_one_host() {
        let mut store = ClientStore::default();
        store.remember(host("aa", 100));
        store.remember(host("bb", 200));

        assert!(store.forget("aa"));
        assert_eq!(store.hosts.len(), 1);
        assert!(!store.forget("aa"), "forgetting twice changes nothing");
    }

    #[test]
    fn the_cached_address_is_updated_and_survives_a_save() {
        let dir = temp_dir();
        let path = dir.0.join("client.json");

        let mut store = ClientStore::default();
        store.remember(host("aa", 100));
        store.note_address("aa", "192.168.1.11:7742");
        store.note_address("unknown", "192.168.1.99:7742");
        store.save(&path).unwrap();

        let back = ClientStore::load(&path).unwrap();
        assert_eq!(
            back.find("aa").unwrap().last_address.as_deref(),
            Some("192.168.1.11:7742")
        );
    }

    #[test]
    fn an_older_store_without_the_optional_fields_still_loads() {
        let store: ClientStore = serde_json::from_str("{}").unwrap();
        assert!(store.hosts.is_empty());
        assert!(store.device_name.is_none());
    }

    #[test]
    fn this_device_always_has_a_name() {
        assert!(!device_name().is_empty());
    }
}
