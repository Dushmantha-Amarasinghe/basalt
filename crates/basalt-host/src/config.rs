//! What the host remembers between runs.
//!
//! Its identity above all: regenerating the certificate would change the host
//! id, and every paired device would refuse to connect to what it would
//! correctly regard as a different machine. Losing this file means pairing
//! everything again.

use std::path::{Path, PathBuf};

use basalt_net::HostIdentity;
use basalt_proto::hex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, Result};
use crate::registry::Device;

/// An absent boolean reads as true, for the fields where that is the safe way
/// to read silence.
fn default_true() -> bool {
    true
}

/// On-disk host state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    /// Hex DER of the certificate and its private key.
    pub cert_der: String,
    pub key_der: String,

    /// The drive or folder being served. `None` until one is chosen.
    pub vault_path: Option<PathBuf>,
    pub vault_name: String,
    /// What this machine calls itself, shown to clients before pairing.
    pub host_name: String,
    pub port: u16,

    /// Whether pairing asks for a PIN.
    ///
    /// Defaults to on, including for a config written before this field
    /// existed. With it off, anyone on the network can read the drive — not a
    /// default to choose on somebody's behalf.
    #[serde(default = "default_true")]
    pub require_pin: bool,

    /// Whether the host starts with Windows. Mirrors the registry entry, so
    /// the app can draw the switch without reading the registry every time.
    #[serde(default)]
    pub start_with_windows: bool,

    /// Whether to recognise films and series on the drive.
    ///
    /// Off unless asked for. Scanning somebody's drive and filing what is on it
    /// is work they did not request, and a library they may not want — so an
    /// upgrade must not quietly start doing it.
    #[serde(default)]
    pub library_enabled: bool,

    /// Whether to fetch artwork for what the library has recognised.
    ///
    /// Off by default, and the switch is the consent. A lookup tells a third
    /// party what is on the drive, and a list of titles is a list of what
    /// somebody watches — that must never start happening because of an
    /// upgrade. It used to be the TMDb key below that gated this, simply
    /// because a key was required; posters need no key now, so the decision
    /// had to become a decision of its own rather than quietly disappear.
    #[serde(default)]
    pub posters: bool,

    /// TMDb key, for titles the keyless source does not have. Optional.
    ///
    /// Only consulted when [`Self::posters`] is on and the free source found
    /// nothing, so this widens coverage rather than unlocking the feature.
    #[serde(default)]
    pub tmdb_key: String,

    /// Whether each device sees its own watch history rather than the one
    /// they all share. Off by default: one history is what lets a film started
    /// on one screen be finished on another, which is the point of keeping it
    /// on the host at all.
    #[serde(default)]
    pub progress_per_device: bool,

    #[serde(default)]
    pub devices: Vec<Device>,
}

impl HostConfig {
    /// Builds a config around a freshly generated identity.
    pub fn create(host_name: &str) -> Result<Self> {
        let identity = HostIdentity::generate(host_name)
            .map_err(|e| HostError::BadRequest(format!("could not create an identity: {e}")))?;
        Ok(Self {
            cert_der: hex::encode(&identity.cert_der),
            key_der: hex::encode(&identity.key_der),
            vault_path: None,
            vault_name: "Vault".to_string(),
            host_name: host_name.to_string(),
            port: basalt_net::DEFAULT_PORT,
            require_pin: true,
            start_with_windows: false,
            library_enabled: false,
            posters: false,
            tmdb_key: String::new(),
            progress_per_device: false,
            devices: Vec::new(),
        })
    }

    pub fn identity(&self) -> Result<HostIdentity> {
        let cert = hex::decode(&self.cert_der)?;
        let key = hex::decode(&self.key_der)?;
        HostIdentity::from_der(cert, key)
            .map_err(|e| HostError::BadRequest(format!("stored identity is unusable: {e}")))
    }

    /// Loads the config, creating one if this is the first run.
    pub fn load_or_create(path: &Path, host_name: &str) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                // Never silently replace a config that failed to parse: the
                // identity inside it is the only thing tying paired devices to
                // this machine, and overwriting it would quietly unpair them
                // all. Better to stop and say so.
                HostError::BadRequest(format!(
                    "{} did not parse ({e}). Move it aside to start fresh, \
                     but every paired device will have to pair again.",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::create(host_name)?;
                config.save(path)?;
                Ok(config)
            }
            Err(e) => Err(HostError::Io(e)),
        }
    }

    /// Writes the config, replacing any previous copy atomically.
    ///
    /// Via a temporary file and a rename, so an interrupted write cannot leave
    /// a half-written identity behind — which would be indistinguishable from
    /// a corrupt one and would cost every pairing.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| HostError::BadRequest(format!("could not encode the config: {e}")))?;

        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }
}

/// Where the host keeps its state.
///
/// `%APPDATA%\Basalt\host.json` on Windows, falling back to the working
/// directory anywhere the variable is missing. Beside the executable would be
/// worse: program directories are often read-only, and the failure would only
/// show up on the first save.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Basalt").join("host.json")
}

/// This machine's name, for display before pairing.
pub fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Basalt Host".to_string())
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
            "basalt-config-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    #[test]
    fn a_created_config_has_a_usable_identity() {
        let config = HostConfig::create("laptop-b").unwrap();
        let identity = config.identity().unwrap();
        assert_eq!(identity.host_id.len(), 64);
        assert_eq!(config.port, basalt_net::DEFAULT_PORT);
        assert!(config.vault_path.is_none());
    }

    #[test]
    fn saving_and_loading_preserves_the_identity() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");

        let first = HostConfig::load_or_create(&path, "laptop-b").unwrap();
        let second = HostConfig::load_or_create(&path, "laptop-b").unwrap();

        assert_eq!(
            first.identity().unwrap().host_id,
            second.identity().unwrap().host_id,
            "restarting the host must not unpair every device"
        );
    }

    #[test]
    fn the_config_file_is_created_on_first_run() {
        let dir = temp_dir();
        let path = dir.0.join("nested").join("host.json");
        assert!(!path.exists());
        HostConfig::load_or_create(&path, "laptop-b").unwrap();
        assert!(path.exists(), "the parent directory is created too");
    }

    #[test]
    fn a_corrupt_config_is_reported_rather_than_replaced() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");
        std::fs::write(&path, b"{ this is not json").unwrap();

        let err = HostConfig::load_or_create(&path, "laptop-b").unwrap_err();
        assert!(
            format!("{err}").contains("pair again"),
            "the message must explain the cost, got: {err}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{ this is not json",
            "the original file must be left alone"
        );
    }

    #[test]
    fn devices_and_vault_settings_survive_a_round_trip() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");

        let mut config = HostConfig::create("laptop-b").unwrap();
        config.vault_path = Some(PathBuf::from("E:/"));
        config.vault_name = "Films".into();
        config.devices.push(Device {
            token_hash: "abc123".into(),
            name: "Laptop A".into(),
            paired_at: 1_700_000_000,
            last_seen: 1_700_000_500,
            writable: false,
            device_id: "0123456789abcdef0123456789abcdef".into(),
            named_by_host: true,
        });
        config.save(&path).unwrap();

        let back = HostConfig::load_or_create(&path, "ignored").unwrap();
        assert_eq!(back.vault_path, Some(PathBuf::from("E:/")));
        assert_eq!(back.vault_name, "Films");
        assert_eq!(back.devices.len(), 1);
        assert_eq!(back.devices[0].name, "Laptop A");
        assert!(!back.devices[0].writable);
        assert_eq!(
            back.devices[0].device_id,
            "0123456789abcdef0123456789abcdef"
        );
        assert!(back.devices[0].named_by_host);
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");
        HostConfig::create("laptop-b").unwrap().save(&path).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "found {leftovers:?}");
    }

    #[test]
    fn an_older_config_without_a_device_list_still_loads() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");
        let config = HostConfig::create("laptop-b").unwrap();

        let mut value = serde_json::to_value(&config).unwrap();
        value.as_object_mut().unwrap().remove("devices");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let back = HostConfig::load_or_create(&path, "laptop-b").unwrap();
        assert!(back.devices.is_empty());
    }

    #[test]
    fn the_default_path_is_absolute_and_named() {
        let path = default_path();
        assert!(path.ends_with("Basalt/host.json") || path.ends_with("Basalt\\host.json"));
    }

    #[test]
    fn the_machine_always_has_some_name() {
        assert!(!machine_name().is_empty());
    }
}
