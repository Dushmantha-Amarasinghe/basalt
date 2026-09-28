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
    /// When this device last connected to it. The host to reopen is the one
    /// used last, not the one paired last: choosing another drive from the
    /// list must stick, even when the drive chosen was paired long ago.
    #[serde(default)]
    pub used_at: i64,
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

    /// The host to reconnect to on startup: the one used last, or paired
    /// last if that was later.
    pub fn primary(&self) -> Option<&KnownHost> {
        self.hosts.iter().max_by_key(|h| h.used_at.max(h.paired_at))
    }

    /// Records that this device has just connected to `host_id`.
    pub fn note_used(&mut self, host_id: &str, now: i64) {
        if let Some(host) = self.find_mut(host_id) {
            host.used_at = now;
        }
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

/// This device's lasting id: the same after Basalt is uninstalled and
/// installed again, its data cleared, or its package renamed.
///
/// A random id kept in the app's own storage went with that storage, and every
/// reinstall appeared on the host as another device of the same name. This is
/// worked out from something the system keeps for the device instead: on
/// Android, its ID for apps signed with Basalt's key, passed in as `hint`
/// because only the Java side can read it; on Windows, the installation's
/// MachineGuid together with the user's profile, so two people's accounts on
/// one PC stay two devices.
///
/// Only a one-way hash of it leaves the device, in the same form as the random
/// ids before it. It lets the host recognise a device it has seen; it proves
/// nothing, and never lets a device in without pairing. `None` when there is
/// nothing to work from, and the random id is used as before.
pub fn lasting_device_id(hint: Option<&str>) -> Option<String> {
    let source = match hint.map(str::trim) {
        Some(hint) if usable_hint(hint) => hint.to_string(),
        _ => system_hint()?,
    };
    let hash = blake3::derive_key("Basalt 2026-09-28 device id", source.as_bytes());
    Some(hash[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// Not empty, and not the one ANDROID_ID a batch of early phones all shared.
fn usable_hint(hint: &str) -> bool {
    !hint.is_empty() && hint != "9774d56d682e549c"
}

#[cfg(windows)]
fn system_hint() -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let subkey = wide(r"SOFTWARE\Microsoft\Cryptography");
    let name = wide("MachineGuid");
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: both strings are NUL-terminated and outlive the call, and the
    // buffer and its size are consistent.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let chars = (size as usize / 2).saturating_sub(1).min(buffer.len());
    let machine = String::from_utf16_lossy(&buffer[..chars]);
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let machine = machine.trim();
    (!machine.is_empty()).then(|| format!("{machine}|{}", profile.to_lowercase()))
}

#[cfg(not(windows))]
fn system_hint() -> Option<String> {
    None
}

/// A fresh device id: sixteen random bytes, as hex.
pub fn new_device_id() -> Result<String> {
    basalt_net::pairing::random_nonce()
        .map(|nonce| nonce[..32].to_string())
        .map_err(|e| ClientError::Protocol(format!("no randomness available: {e}")))
}

/// This device's name, as it will appear on the host.
pub fn device_name() -> String {
    #[cfg(target_os = "android")]
    if let Some(name) = android::device_name() {
        return name;
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Basalt Client".to_string())
}

/// A phone has no computer name. What it does have is its maker and model,
/// which is how people tell phones apart in a list: "Google Pixel 7 Pro".
#[cfg(target_os = "android")]
mod android {
    unsafe extern "C" {
        fn __system_property_get(
            name: *const std::ffi::c_char,
            value: *mut std::ffi::c_char,
        ) -> i32;
    }

    fn property(name: &str) -> Option<String> {
        let name = std::ffi::CString::new(name).ok()?;
        // PROP_VALUE_MAX is 92 bytes, terminator included.
        let mut value = [0 as std::ffi::c_char; 92];
        let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        if len <= 0 {
            return None;
        }
        let text = unsafe { std::ffi::CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        (!text.is_empty()).then_some(text)
    }

    pub fn device_name() -> Option<String> {
        let model = property("ro.product.model")?;
        let maker = property("ro.product.manufacturer").unwrap_or_default();
        // "Pixel 7 Pro" from Google, but "SM-S918B" from Samsung: the maker
        // goes in front unless the model already says it.
        let maker = capitalise(&maker);
        if maker.is_empty() || model.to_lowercase().starts_with(&maker.to_lowercase()) {
            Some(model)
        } else {
            Some(format!("{maker} {model}"))
        }
    }

    fn capitalise(word: &str) -> String {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }
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
            used_at: 0,
            identity: Default::default(),
        }
    }

    /// Switching to a drive paired long ago must survive the app reopening,
    /// rather than falling back to whichever was paired most recently.
    #[test]
    fn the_drive_used_last_is_the_one_reopened() {
        let mut store = ClientStore::default();
        store.remember(host("old", 100));
        store.remember(host("new", 200));
        assert_eq!(store.primary().unwrap().host_id, "new");
        store.note_used("old", 300);
        assert_eq!(store.primary().unwrap().host_id, "old");
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

    /// The same device is the same id however often its storage is wiped;
    /// another device is another id; and only a hash leaves the device.
    #[test]
    fn a_lasting_id_follows_the_device_not_its_storage() {
        let phone = lasting_device_id(Some("3f2a9c0d1b7e4a55")).unwrap();
        assert_eq!(phone, lasting_device_id(Some("3f2a9c0d1b7e4a55")).unwrap());
        assert_eq!(phone.len(), 32);
        assert!(phone.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!phone.contains("3f2a9c0d1b7e4a55"));
        assert_ne!(phone, lasting_device_id(Some("0a0b0c0d0e0f1011")).unwrap());
    }

    /// The id every one of a batch of early Android phones reported is no
    /// way to tell phones apart, and an empty one is none at all.
    #[test]
    fn a_useless_hint_is_not_used() {
        assert!(!usable_hint("9774d56d682e549c"));
        assert!(!usable_hint(""));
        assert!(usable_hint("3f2a9c0d1b7e4a55"));
    }

    /// A Windows PC always has an installation id to work from, so its apps
    /// never fall back to a random one.
    #[cfg(windows)]
    #[test]
    fn a_windows_pc_has_a_lasting_id() {
        let id = lasting_device_id(None).expect("MachineGuid is readable");
        assert_eq!(id, lasting_device_id(None).unwrap());
    }
}
