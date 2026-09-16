//! Paired devices, and the pairing window.
//!
//! Deliberately free of I/O and of any clock of its own: every method that
//! cares about time takes the current instant as an argument. That is what
//! makes the expiry and lockout rules testable without sleeping, and those two
//! rules are the only thing standing between a six-digit PIN and someone with
//! a script.

use std::time::Instant;

use basalt_net::pairing::{self, MAX_PIN_ATTEMPTS, PAIRING_WINDOW};
use basalt_proto::hex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, Result};

/// A device that has completed pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Hex SHA-256 of the device token.
    ///
    /// The token itself is never stored. The host only ever needs to recognise
    /// one, and a hash does that just as well while making the config file
    /// useless to anyone who reads it.
    pub token_hash: String,
    pub name: String,
    /// Unix seconds.
    pub paired_at: i64,
    pub last_seen: i64,
    pub writable: bool,
}

/// An open pairing window.
#[derive(Debug, Clone)]
struct PairingWindow {
    pin: String,
    opened: Instant,
    attempts: u32,
}

/// What the host will currently accept.
#[derive(Debug, Default)]
pub struct Registry {
    devices: Vec<Device>,
    pairing: Option<PairingWindow>,
}

/// Hashes a token for storage and comparison.
fn hash_token(token: &str) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Registry {
    pub fn new(devices: Vec<Device>) -> Self {
        Self {
            devices,
            pairing: None,
        }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    // -----------------------------------------------------------------------
    // Pairing
    // -----------------------------------------------------------------------

    /// Opens a pairing window and returns the PIN to show the user.
    ///
    /// Opening again replaces any window already open, which is what the user
    /// means when they press the button a second time: they want a PIN they
    /// can still read, not the one that is about to expire.
    pub fn open_pairing(&mut self, now: Instant) -> Result<String> {
        let pin = pairing::generate_pin()
            .map_err(|e| HostError::PairingRefused(format!("could not generate a PIN: {e}")))?;
        self.pairing = Some(PairingWindow {
            pin: pin.clone(),
            opened: now,
            attempts: 0,
        });
        Ok(pin)
    }

    pub fn close_pairing(&mut self) {
        self.pairing = None;
    }

    /// The PIN currently on display, if the window is still open.
    pub fn pairing_pin(&self, now: Instant) -> Option<&str> {
        self.pairing
            .as_ref()
            .filter(|w| now.duration_since(w.opened) < PAIRING_WINDOW)
            .map(|w| w.pin.as_str())
    }

    pub fn pairing_open(&self, now: Instant) -> bool {
        self.pairing_pin(now).is_some()
    }

    /// How long the open window has left.
    pub fn pairing_remaining(&self, now: Instant) -> Option<std::time::Duration> {
        let window = self.pairing.as_ref()?;
        PAIRING_WINDOW.checked_sub(now.duration_since(window.opened))
    }

    /// Verifies a pairing proof and, on success, issues a device token.
    ///
    /// `host_id` must be this host's own identity, not anything that arrived in
    /// a message — see [`basalt_net::pairing`] for why that distinction is the
    /// whole security of first contact.
    pub fn finish_pairing(
        &mut self,
        now: Instant,
        host_id: &str,
        client_nonce: &str,
        server_nonce: &str,
        proof: &str,
        device_name: &str,
    ) -> Result<String> {
        if !self.pairing_open(now) {
            // Clear an expired window rather than leaving it to rot, so the
            // host UI stops advertising a PIN that will never work.
            self.pairing = None;
            return Err(HostError::PairingRefused(
                "this host is not accepting new devices right now".into(),
            ));
        }

        let window = self.pairing.as_mut().expect("checked open just above");
        let ok = pairing::verify_proof(&window.pin, host_id, client_nonce, server_nonce, proof);

        if !ok {
            window.attempts += 1;
            let spent = window.attempts >= MAX_PIN_ATTEMPTS;
            if spent {
                // Closing rather than merely counting is the point: a fresh PIN
                // means the million guesses an attacker was working through are
                // all worthless, and the user has to be present to read the new
                // one off the host.
                self.pairing = None;
            }
            return Err(HostError::PairingRefused(if spent {
                "too many wrong PINs; pairing has been closed".into()
            } else {
                "that PIN is not right".into()
            }));
        }

        let token = pairing::random_token()
            .map_err(|e| HostError::PairingRefused(format!("could not issue a token: {e}")))?;
        let stamp = unix_now();
        self.devices.push(Device {
            token_hash: hash_token(&token),
            name: sanitise_device_name(device_name),
            paired_at: stamp,
            last_seen: stamp,
            writable: true,
        });
        // One PIN pairs one device. Leaving the window open would let anyone
        // who saw the screen pair as well.
        self.pairing = None;
        Ok(token)
    }

    // -----------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------

    /// Looks up a device by token, recording that it has been seen.
    pub fn authenticate(&mut self, token: &str) -> Option<Device> {
        let hash = hash_token(token);
        let device = self
            .devices
            .iter_mut()
            .find(|d| hex::constant_time_eq(d.token_hash.as_bytes(), hash.as_bytes()))?;
        device.last_seen = unix_now();
        Some(device.clone())
    }

    /// Removes a device. It cannot connect again without pairing afresh.
    pub fn revoke(&mut self, token_hash: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.token_hash != token_hash);
        self.devices.len() != before
    }

    pub fn set_writable(&mut self, token_hash: &str, writable: bool) -> bool {
        match self.devices.iter_mut().find(|d| d.token_hash == token_hash) {
            Some(d) => {
                d.writable = writable;
                true
            }
            None => false,
        }
    }
}

/// Trims a device name to something safe to display.
///
/// The name arrives from the network and ends up in a list on the host and in
/// log lines. Control characters could rewrite a terminal; an unbounded string
/// could push everything else off the screen.
fn sanitise_device_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "Unnamed device".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_ID: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const OTHER_HOST: &str = "bb00000000000000000000000000000000000000000000000000000000000002";

    fn nonces() -> (String, String) {
        (
            pairing::random_nonce().unwrap(),
            pairing::random_nonce().unwrap(),
        )
    }

    /// Pairs a device, returning its token.
    fn pair(registry: &mut Registry, now: Instant, host_id: &str) -> Result<String> {
        let pin = registry
            .pairing_pin(now)
            .expect("window is open")
            .to_string();
        let (c, s) = nonces();
        let proof = pairing::compute_proof(&pin, host_id, &c, &s).unwrap();
        registry.finish_pairing(now, host_id, &c, &s, &proof, "Laptop A")
    }

    #[test]
    fn a_new_registry_accepts_nobody() {
        let mut registry = Registry::default();
        let now = Instant::now();
        assert!(!registry.pairing_open(now));
        assert_eq!(registry.device_count(), 0);
        assert!(registry.authenticate("anything").is_none());
    }

    #[test]
    fn the_happy_path_pairs_and_then_authenticates() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();

        let token = pair(&mut registry, now, HOST_ID).unwrap();
        assert_eq!(registry.device_count(), 1);

        let device = registry.authenticate(&token).expect("the token works");
        assert_eq!(device.name, "Laptop A");
        assert!(device.writable);
    }

    #[test]
    fn the_raw_token_is_never_stored() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        let token = pair(&mut registry, now, HOST_ID).unwrap();

        assert_ne!(registry.devices()[0].token_hash, token);
        assert!(
            !serde_json::to_string(registry.devices())
                .unwrap()
                .contains(&token),
            "a token must not be recoverable from what is written to disk"
        );
    }

    #[test]
    fn a_wrong_pin_is_refused_and_pairs_nobody() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();

        let (c, s) = nonces();
        let wrong = pairing::compute_proof("000000", HOST_ID, &c, &s).unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &c, &s, &wrong, "Intruder")
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    // The attack the salt exists to stop, all the way through the registry: a
    // proof computed against someone else's public key must not pair.
    #[test]
    fn a_proof_bound_to_another_key_does_not_pair() {
        let mut registry = Registry::default();
        let now = Instant::now();
        let pin = registry.open_pairing(now).unwrap();

        let (c, s) = nonces();
        let proof = pairing::compute_proof(&pin, OTHER_HOST, &c, &s).unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &c, &s, &proof, "Impostor")
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    #[test]
    fn five_wrong_pins_close_the_window() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();

        for attempt in 1..=MAX_PIN_ATTEMPTS {
            let (c, s) = nonces();
            let wrong = pairing::compute_proof("000001", HOST_ID, &c, &s).unwrap();
            assert!(
                registry
                    .finish_pairing(now, HOST_ID, &c, &s, &wrong, "Intruder")
                    .is_err(),
                "attempt {attempt}"
            );
        }
        assert!(
            !registry.pairing_open(now),
            "the window must close once the attempts are spent"
        );
    }

    #[test]
    fn the_right_pin_after_a_lockout_is_too_late() {
        let mut registry = Registry::default();
        let now = Instant::now();
        let pin = registry.open_pairing(now).unwrap();

        for _ in 0..MAX_PIN_ATTEMPTS {
            let (c, s) = nonces();
            let wrong = pairing::compute_proof("000001", HOST_ID, &c, &s).unwrap();
            let _ = registry.finish_pairing(now, HOST_ID, &c, &s, &wrong, "Intruder");
        }

        let (c, s) = nonces();
        let right = pairing::compute_proof(&pin, HOST_ID, &c, &s).unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &c, &s, &right, "Intruder")
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    #[test]
    fn an_expired_window_refuses_the_right_pin() {
        let mut registry = Registry::default();
        let opened = Instant::now();
        let pin = registry.open_pairing(opened).unwrap();

        let later = opened + PAIRING_WINDOW + std::time::Duration::from_secs(1);
        assert!(!registry.pairing_open(later));

        let (c, s) = nonces();
        let proof = pairing::compute_proof(&pin, HOST_ID, &c, &s).unwrap();
        assert!(
            registry
                .finish_pairing(later, HOST_ID, &c, &s, &proof, "Laptop A")
                .is_err()
        );
    }

    #[test]
    fn the_window_is_still_open_a_second_before_it_expires() {
        let mut registry = Registry::default();
        let opened = Instant::now();
        registry.open_pairing(opened).unwrap();
        let just_before = opened + PAIRING_WINDOW - std::time::Duration::from_secs(1);
        assert!(registry.pairing_open(just_before));
        assert!(pair(&mut registry, just_before, HOST_ID).is_ok());
    }

    #[test]
    fn pairing_closes_after_one_device_so_a_pin_is_not_reusable() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        pair(&mut registry, now, HOST_ID).unwrap();

        assert!(!registry.pairing_open(now));
        assert_eq!(registry.device_count(), 1);
    }

    #[test]
    fn reopening_replaces_the_pin() {
        let mut registry = Registry::default();
        let now = Instant::now();
        let first = registry.open_pairing(now).unwrap();
        let second = registry.open_pairing(now).unwrap();
        assert_eq!(registry.pairing_pin(now), Some(second.as_str()));
        // Astronomically unlikely to collide, and a real failure if it does.
        assert_ne!(first, second);
    }

    #[test]
    fn two_devices_pair_independently() {
        let mut registry = Registry::default();
        let now = Instant::now();

        registry.open_pairing(now).unwrap();
        let first = pair(&mut registry, now, HOST_ID).unwrap();
        registry.open_pairing(now).unwrap();
        let second = pair(&mut registry, now, HOST_ID).unwrap();

        assert_ne!(first, second);
        assert_eq!(registry.device_count(), 2);
        assert!(registry.authenticate(&first).is_some());
        assert!(registry.authenticate(&second).is_some());
    }

    #[test]
    fn a_revoked_device_cannot_come_back() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        let token = pair(&mut registry, now, HOST_ID).unwrap();

        let hash = registry.devices()[0].token_hash.clone();
        assert!(registry.revoke(&hash));
        assert!(registry.authenticate(&token).is_none());
        assert_eq!(registry.device_count(), 0);
        assert!(!registry.revoke(&hash), "revoking twice changes nothing");
    }

    #[test]
    fn a_device_can_be_made_read_only() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        let token = pair(&mut registry, now, HOST_ID).unwrap();

        let hash = registry.devices()[0].token_hash.clone();
        assert!(registry.set_writable(&hash, false));
        assert!(!registry.authenticate(&token).unwrap().writable);
    }

    #[test]
    fn a_made_up_token_authenticates_nobody() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        pair(&mut registry, now, HOST_ID).unwrap();

        assert!(registry.authenticate("").is_none());
        assert!(registry.authenticate("deadbeef").is_none());
        assert!(registry.authenticate(&"0".repeat(64)).is_none());
    }

    #[test]
    fn connecting_updates_when_the_device_was_last_seen() {
        let mut registry = Registry::default();
        let now = Instant::now();
        registry.open_pairing(now).unwrap();
        let token = pair(&mut registry, now, HOST_ID).unwrap();

        registry.devices[0].last_seen = 0;
        registry.authenticate(&token).unwrap();
        assert!(registry.devices()[0].last_seen > 0);
    }

    #[test]
    fn device_names_are_cleaned_up_before_they_are_stored() {
        assert_eq!(sanitise_device_name("  Laptop A  "), "Laptop A");
        assert_eq!(sanitise_device_name(""), "Unnamed device");
        assert_eq!(sanitise_device_name("   "), "Unnamed device");
        assert_eq!(sanitise_device_name("a\r\nb\x1b[2J"), "ab[2J");
        assert!(sanitise_device_name(&"x".repeat(500)).len() <= 64);
    }

    #[test]
    fn the_remaining_time_counts_down_and_then_stops() {
        let mut registry = Registry::default();
        let opened = Instant::now();
        registry.open_pairing(opened).unwrap();

        assert_eq!(registry.pairing_remaining(opened), Some(PAIRING_WINDOW));
        let halfway = opened + PAIRING_WINDOW / 2;
        assert_eq!(
            registry.pairing_remaining(halfway),
            Some(PAIRING_WINDOW / 2)
        );
        assert_eq!(
            registry.pairing_remaining(opened + PAIRING_WINDOW * 2),
            None
        );
    }
}
