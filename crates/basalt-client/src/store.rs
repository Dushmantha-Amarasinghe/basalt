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
    /// Who uses this device with this host: a remembered profile, the last
    /// one signed in to, or the device on its own.
    #[serde(default)]
    pub identity: Identity,
}

/// Who this device signs in as, per host.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Identity {
    /// A profile to sign straight back in to, when "remember me" was ticked.
    #[serde(default)]
    pub profile: Option<SavedProfile>,
    /// The last profile signed in to here, shown first after signing out so
    /// getting back in is a tap and a PIN.
    #[serde(default)]
    pub last_profile: Option<String>,
    /// "Always continue as this device": no question on start.
    #[serde(default)]
    pub always_device: bool,
}

/// A remembered profile sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: u8,
    /// The sign-in token the host issued. Never the PIN: a remembered
    /// profile skips the PIN, and a PIN kept here would unlock it anywhere
    /// the file was copied to.
    pub token: String,
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
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes)
                .map(Self::revealed)
                .map_err(|e| {
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
        let json = serde_json::to_vec_pretty(&self.clone().sealed())
            .map_err(|e| ClientError::Config(format!("could not encode the store: {e}")))?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// Every token encrypted for this Windows user, as it goes to disk.
    ///
    /// The file sat in AppData with the tokens in plain text, so anything that
    /// could read the file could use them from anywhere. Sealed, they only
    /// open for the same user on the same machine.
    fn sealed(mut self) -> Self {
        for host in &mut self.hosts {
            host.token = secret::seal(&host.token);
            if let Some(profile) = &mut host.identity.profile {
                profile.token = secret::seal(&profile.token);
            }
        }
        self
    }

    /// Tokens as the program uses them. A file written before sealing reads
    /// as it is, and is sealed the next time it is saved.
    fn revealed(mut self) -> Self {
        for host in &mut self.hosts {
            host.token = secret::open(&host.token);
            if let Some(profile) = &mut host.identity.profile {
                profile.token = secret::open(&profile.token);
            }
        }
        self
    }

    pub fn find_mut(&mut self, host_id: &str) -> Option<&mut KnownHost> {
        self.hosts.iter_mut().find(|h| h.host_id == host_id)
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

/// Tokens at rest, encrypted with Windows' own per-user protection (DPAPI).
mod secret {
    const PREFIX: &str = "dpapi:";

    pub fn seal(plain: &str) -> String {
        if plain.is_empty() || plain.starts_with(PREFIX) {
            return plain.to_string();
        }
        match platform::protect(plain.as_bytes()) {
            Some(sealed) => format!("{PREFIX}{}", basalt_proto::hex::encode(&sealed)),
            // Nothing to seal with: kept as it was, as it always had been.
            None => plain.to_string(),
        }
    }

    pub fn open(stored: &str) -> String {
        let Some(hex) = stored.strip_prefix(PREFIX) else {
            return stored.to_string();
        };
        // One that will not open — the file copied to another account or
        // machine — is no token at all, and the host says so on connecting.
        basalt_proto::hex::decode(hex)
            .ok()
            .and_then(|bytes| platform::unprotect(&bytes))
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default()
    }

    #[cfg(windows)]
    mod platform {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        };

        fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
            // SAFETY: the API allocated `cbData` bytes at `pbData`, which are
            // copied out before the allocation is handed back.
            let bytes =
                unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
            unsafe { LocalFree(blob.pbData.cast()) };
            bytes
        }

        pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
            let input = CRYPT_INTEGER_BLOB {
                cbData: u32::try_from(plain.len()).ok()?,
                pbData: plain.as_ptr().cast_mut(),
            };
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            // SAFETY: both blobs are valid for the call; nothing else is passed.
            let ok = unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            (ok != 0).then(|| take(output))
        }

        pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
            let input = CRYPT_INTEGER_BLOB {
                cbData: u32::try_from(sealed.len()).ok()?,
                pbData: sealed.as_ptr().cast_mut(),
            };
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            // SAFETY: as above.
            let ok = unsafe {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            (ok != 0).then(|| take(output))
        }
    }

    #[cfg(not(windows))]
    mod platform {
        pub fn protect(_: &[u8]) -> Option<Vec<u8>> {
            None
        }
        pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
            None
        }
    }
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

    #[test]
    fn tokens_are_sealed_on_disk_and_read_back_as_they_were() {
        let dir = TempDir(std::env::temp_dir().join(format!("basalt-seal-{}", std::process::id())));
        let path = dir.0.join("client.json");
        let mut store = ClientStore::default();
        let mut paired = host("aa", 1);
        paired.token = "secret-device-token".into();
        paired.identity.profile = Some(SavedProfile {
            id: "p1".into(),
            name: "Maya".into(),
            color: 2,
            token: "secret-profile-token".into(),
        });
        store.remember(paired);
        store.save(&path).unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("secret-device-token"), "sealed on disk");
        assert!(!on_disk.contains("secret-profile-token"), "sealed on disk");

        let back = ClientStore::load(&path).unwrap();
        let host = back.find("aa").unwrap();
        assert_eq!(host.token, "secret-device-token");
        assert_eq!(
            host.identity.profile.as_ref().unwrap().token,
            "secret-profile-token"
        );
    }

    #[test]
    fn a_file_from_before_sealing_still_reads() {
        let dir =
            TempDir(std::env::temp_dir().join(format!("basalt-plain-{}", std::process::id())));
        std::fs::create_dir_all(&dir.0).unwrap();
        let path = dir.0.join("client.json");
        std::fs::write(
            &path,
            r#"{"hosts":[{"host_id":"aa","token":"plain-token","vault":"V","host_name":"H","last_address":null,"paired_at":1}]}"#,
        )
        .unwrap();
        let store = ClientStore::load(&path).unwrap();
        assert_eq!(store.find("aa").unwrap().token, "plain-token");
        assert!(store.find("aa").unwrap().identity.profile.is_none());
    }

    fn host(id: &str, paired_at: i64) -> KnownHost {
        KnownHost {
            host_id: id.into(),
            token: format!("token-for-{id}"),
            vault: "Vault".into(),
            host_name: "laptop-b".into(),
            last_address: None,
            paired_at,
            identity: Default::default(),
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
