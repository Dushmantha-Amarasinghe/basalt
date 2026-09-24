//! Paired devices, and the requests waiting to become one.
//!
//! Deliberately free of I/O and of any clock of its own: every method that
//! cares about time takes the current instant as an argument. That is what
//! makes the expiry and lockout rules testable without sleeping, and those two
//! rules are the only thing standing between a six-digit PIN and someone with
//! a script.
//!
//! **How pairing works now.** A client asks to pair; the host records that as a
//! *request*, generates a PIN for it, and displays both — the device's name and
//! the number to read across. Earlier this was the other way round: the host
//! opened a window in advance and the user went and fetched a PIN before
//! touching the client. Requests are better because the host can say who is
//! asking.
//!
//! When the PIN is switched off, a request is granted as soon as it arrives.
//! That is a real decision with a real cost — anyone on the network can then
//! read the drive — so it defaults to on and the host says plainly what turning
//! it off means.

use std::time::Instant;

use basalt_net::pairing::{self, MAX_PIN_ATTEMPTS, PAIRING_WINDOW};
use basalt_proto::hex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, Result};

/// Requests held at once, so a flood cannot fill memory or bury the real one.
const MAX_PENDING: usize = 8;

/// How long a device with no id may go unseen before it is let go.
///
/// Devices paired before ids existed cannot be told apart once they pair
/// again, so the host collected them: the same machine listed three times,
/// two of those never to connect again. Every device still in use says what
/// it is on its next connection and is kept for good; a record that has had a
/// month to do that and has not is one nobody is coming back for.
const FORGOTTEN_AFTER_SECONDS: i64 = 30 * 24 * 60 * 60;

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
    /// The id the device made for itself, the same every time it pairs.
    ///
    /// Empty for a device paired before devices had one, until it next
    /// connects and says what it is.
    #[serde(default)]
    pub device_id: String,
    /// Set once somebody renames the device on the host. After that, the name
    /// the device calls itself no longer replaces the one it was given here.
    #[serde(default)]
    pub named_by_host: bool,
}

impl Device {
    /// What anything kept per device is filed under.
    ///
    /// The device's own id when it has one, because that survives pairing
    /// again; the token only for a device too old to have sent an id.
    pub fn key(&self) -> &str {
        if self.device_id.is_empty() {
            &self.token_hash
        } else {
            &self.device_id
        }
    }
}

/// A client waiting to be let in.
#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub id: String,
    pub device_name: String,
    /// See [`Device::device_id`]. Empty from an older client.
    pub device_id: String,
    /// `None` when the host is not asking for one.
    pub pin: Option<String>,
    pub opened: Instant,
    pub attempts: u32,
    /// The nonces this attempt is bound to.
    pub client_nonce: String,
    pub server_nonce: String,
}

impl PairingRequest {
    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.opened) >= PAIRING_WINDOW
    }

    pub fn remaining(&self, now: Instant) -> std::time::Duration {
        PAIRING_WINDOW.saturating_sub(now.duration_since(self.opened))
    }
}

/// What the host will currently accept.
#[derive(Debug, Default)]
pub struct Registry {
    devices: Vec<Device>,
    pending: Vec<PairingRequest>,
    /// Whether a request has to prove it knows a PIN.
    require_pin: bool,
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
    pub fn new(devices: Vec<Device>, require_pin: bool) -> Self {
        Self {
            devices,
            pending: Vec::new(),
            require_pin,
        }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// One device, by the hash of its token.
    pub fn device(&self, token_hash: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.token_hash == token_hash)
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn require_pin(&self) -> bool {
        self.require_pin
    }

    /// Changes whether a PIN is asked for.
    ///
    /// Any request already waiting is dropped. A request created under one rule
    /// must not be completed under another — turning the PIN off mid-attempt
    /// would otherwise let a waiting request through without one.
    pub fn set_require_pin(&mut self, require: bool) {
        self.require_pin = require;
        self.pending.clear();
    }

    // -----------------------------------------------------------------------
    // Pairing requests
    // -----------------------------------------------------------------------

    /// Requests still worth showing, oldest first.
    pub fn pending(&self, now: Instant) -> Vec<PairingRequest> {
        self.pending
            .iter()
            .filter(|r| !r.expired(now))
            .cloned()
            .collect()
    }

    fn forget_expired(&mut self, now: Instant) {
        self.pending.retain(|r| !r.expired(now));
    }

    /// Records a client asking to pair, and generates its PIN.
    ///
    /// For a client that did not say what device it is; see
    /// [`Registry::begin_pairing_for`].
    pub fn begin_pairing(
        &mut self,
        now: Instant,
        device_name: &str,
        client_nonce: &str,
    ) -> Result<PairingRequest> {
        self.begin_pairing_for(now, device_name, "", client_nonce)
    }

    /// Records a device asking to pair, and generates its PIN.
    pub fn begin_pairing_for(
        &mut self,
        now: Instant,
        device_name: &str,
        device_id: &str,
        client_nonce: &str,
    ) -> Result<PairingRequest> {
        self.forget_expired(now);

        // A client that retries should replace its own waiting request rather
        // than adding another, or one machine reconnecting a few times would
        // fill the host's screen with itself. Recognised by its id when it has
        // one, since two machines may well share a name.
        let name = sanitise_device_name(device_name);
        let device_id = sanitise_device_id(device_id);
        if device_id.is_empty() {
            self.pending.retain(|r| r.device_name != name);
        } else {
            self.pending.retain(|r| r.device_id != device_id);
        }

        if self.pending.len() >= MAX_PENDING {
            return Err(HostError::PairingRefused(
                "too many devices are trying to pair at once".into(),
            ));
        }

        let pin =
            if self.require_pin {
                Some(pairing::generate_pin().map_err(|e| {
                    HostError::PairingRefused(format!("could not generate a PIN: {e}"))
                })?)
            } else {
                None
            };
        let server_nonce = pairing::random_nonce()
            .map_err(|e| HostError::PairingRefused(format!("no randomness: {e}")))?;
        let id = pairing::random_token()
            .map_err(|e| HostError::PairingRefused(format!("no randomness: {e}")))?;

        let request = PairingRequest {
            id,
            device_name: name,
            device_id,
            pin,
            opened: now,
            attempts: 0,
            client_nonce: client_nonce.to_string(),
            server_nonce,
        };
        self.pending.push(request.clone());
        Ok(request)
    }

    /// Drops a request, for when the user refuses it on the host.
    pub fn deny(&mut self, id: &str) -> bool {
        let before = self.pending.len();
        self.pending.retain(|r| r.id != id);
        self.pending.len() != before
    }

    /// Completes a request and issues a device token.
    ///
    /// `host_id` must be this host's own identity, not anything that arrived in
    /// a message — see [`basalt_net::pairing`] for why that distinction is the
    /// whole security of first contact.
    ///
    /// The device is recorded under the name it gave when it asked — the one
    /// shown beside the PIN, and so the one the person at the host agreed to.
    /// It used to take the name from this final message instead, and clients
    /// sent the *host's* name there, so every device in the list was called
    /// after the machine it was connecting to.
    ///
    /// A device that pairs again replaces its old record rather than adding a
    /// second one. It keeps whatever the host had decided about it — whether
    /// it may write, and a name given here — because pairing again is the same
    /// device getting a new key, not a new device.
    pub fn finish_pairing(
        &mut self,
        now: Instant,
        host_id: &str,
        request_id: &str,
        proof: Option<&str>,
    ) -> Result<String> {
        self.forget_expired(now);

        let index = self
            .pending
            .iter()
            .position(|r| r.id == request_id)
            .ok_or_else(|| {
                HostError::PairingRefused(
                    "that pairing request has expired. Try connecting again.".into(),
                )
            })?;

        if let Some(pin) = self.pending[index].pin.clone() {
            let request = &self.pending[index];
            let ok = proof.is_some_and(|proof| {
                pairing::verify_proof(
                    &pin,
                    host_id,
                    &request.client_nonce,
                    &request.server_nonce,
                    proof,
                )
            });

            if !ok {
                self.pending[index].attempts += 1;
                let spent = self.pending[index].attempts >= MAX_PIN_ATTEMPTS;
                if spent {
                    // Dropping the request rather than merely counting is the
                    // point: a new one means a new PIN, so the guesses an
                    // attacker was working through are all worthless, and
                    // somebody has to be at the host to read the new number.
                    self.pending.remove(index);
                }
                return Err(HostError::PairingRefused(if spent {
                    "too many wrong PINs. Try connecting again for a new one.".into()
                } else {
                    "that PIN is not right".into()
                }));
            }
        }

        let token = pairing::random_token()
            .map_err(|e| HostError::PairingRefused(format!("could not issue a token: {e}")))?;
        let stamp = unix_now();
        let request = self.pending.remove(index);

        let known = (!request.device_id.is_empty())
            .then(|| {
                self.devices
                    .iter_mut()
                    .find(|d| d.device_id == request.device_id)
            })
            .flatten();
        match known {
            Some(device) => {
                device.token_hash = hash_token(&token);
                device.paired_at = stamp;
                device.last_seen = stamp;
                if !device.named_by_host {
                    device.name = request.device_name;
                }
            }
            None => self.devices.push(Device {
                token_hash: hash_token(&token),
                name: request.device_name,
                paired_at: stamp,
                last_seen: stamp,
                writable: true,
                device_id: request.device_id,
                named_by_host: false,
            }),
        }
        Ok(token)
    }

    /// Takes in what a connecting device says about itself.
    ///
    /// A device paired before ids existed learns its id here, which is what
    /// lets it be recognised if it ever pairs again. And a device that has been
    /// renamed shows its new name — unless somebody named it on the host, in
    /// which case their name stands.
    ///
    /// Returns whether anything changed, so the caller knows to save.
    pub fn observe(&mut self, token_hash: &str, device_id: &str, device_name: &str) -> bool {
        let Some(device) = self.devices.iter_mut().find(|d| d.token_hash == token_hash) else {
            return false;
        };
        let mut changed = false;

        let device_id = sanitise_device_id(device_id);
        if device.device_id.is_empty() && !device_id.is_empty() {
            device.device_id = device_id;
            changed = true;
        }

        let name = device_name.trim();
        if !name.is_empty() && !device.named_by_host {
            let name = sanitise_device_name(name);
            if device.name != name {
                device.name = name;
                changed = true;
            }
        }
        changed
    }

    /// Lets go of devices with no id that have not connected in a month.
    ///
    /// Returns how many went.
    pub fn forget_abandoned(&mut self, now_unix: i64) -> usize {
        let before = self.devices.len();
        self.devices.retain(|d| {
            !d.device_id.is_empty() || now_unix - d.last_seen < FORGOTTEN_AFTER_SECONDS
        });
        before - self.devices.len()
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

    pub fn rename(&mut self, token_hash: &str, name: &str) -> bool {
        match self.devices.iter_mut().find(|d| d.token_hash == token_hash) {
            Some(d) => {
                d.name = sanitise_device_name(name);
                d.named_by_host = true;
                true
            }
            None => false,
        }
    }
}

/// Keeps a device id to something that is plainly an id.
///
/// It arrives from the network and becomes a key for files on the host, so
/// anything but a short run of hex is treated as no id at all.
fn sanitise_device_id(id: &str) -> String {
    let id = id.trim();
    if (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit()) {
        id.to_ascii_lowercase()
    } else {
        String::new()
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

    fn with_pin() -> Registry {
        Registry::new(Vec::new(), true)
    }

    fn without_pin() -> Registry {
        Registry::new(Vec::new(), false)
    }

    /// Runs a whole pairing the way a client does, returning the token.
    fn pair(registry: &mut Registry, now: Instant, host_id: &str, name: &str) -> Result<String> {
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, name, &nonce)?;
        let proof = request.pin.as_ref().map(|pin| {
            pairing::compute_proof(pin, host_id, &request.client_nonce, &request.server_nonce)
                .unwrap()
        });
        registry.finish_pairing(now, host_id, &request.id, proof.as_deref())
    }

    #[test]
    fn a_new_registry_accepts_nobody() {
        let mut registry = with_pin();
        assert_eq!(registry.device_count(), 0);
        assert!(registry.pending(Instant::now()).is_empty());
        assert!(registry.authenticate("anything").is_none());
    }

    #[test]
    fn the_happy_path_pairs_and_then_authenticates() {
        let mut registry = with_pin();
        let now = Instant::now();
        let token = pair(&mut registry, now, HOST_ID, "Laptop A").unwrap();

        assert_eq!(registry.device_count(), 1);
        let device = registry.authenticate(&token).expect("the token works");
        assert_eq!(device.name, "Laptop A");
        assert!(device.writable);
    }

    // The host has to be able to show who is asking, and what number to read
    // across. That is the whole reason requests exist.
    #[test]
    fn a_request_is_visible_on_the_host_with_its_pin() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        registry.begin_pairing(now, "Laptop A", &nonce).unwrap();

        let pending = registry.pending(now);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].device_name, "Laptop A");
        let pin = pending[0].pin.as_ref().expect("a PIN to display");
        assert_eq!(pin.len(), 6);
    }

    #[test]
    fn with_the_pin_switched_off_a_request_carries_none_and_pairs_at_once() {
        let mut registry = without_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();

        let request = registry.begin_pairing(now, "Laptop A", &nonce).unwrap();
        assert!(request.pin.is_none(), "nothing to read across");

        let token = registry
            .finish_pairing(now, HOST_ID, &request.id, None)
            .unwrap();
        assert!(registry.authenticate(&token).is_some());
    }

    #[test]
    fn with_the_pin_on_a_request_without_a_proof_is_refused() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Intruder", &nonce).unwrap();

        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, None)
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    #[test]
    fn a_wrong_pin_is_refused_and_pairs_nobody() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Intruder", &nonce).unwrap();

        let wrong = pairing::compute_proof(
            "000000",
            HOST_ID,
            &request.client_nonce,
            &request.server_nonce,
        )
        .unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, Some(&wrong))
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    /// The attack the salt exists to stop.
    ///
    /// Someone in the middle presents their own certificate during first
    /// contact. The client computes its proof against *that* key, so the proof
    /// is worthless against the real host — which verifies with its own.
    ///
    /// The first version of this test computed and verified with the same id,
    /// which proved nothing at all. The two must differ, and only here.
    #[test]
    fn a_proof_bound_to_another_key_does_not_pair() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Impostor", &nonce).unwrap();
        let pin = request.pin.clone().unwrap();

        // The right PIN, bound to the wrong key.
        let proof = pairing::compute_proof(
            &pin,
            OTHER_HOST,
            &request.client_nonce,
            &request.server_nonce,
        )
        .unwrap();

        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, Some(&proof))
                .is_err(),
            "a proof bound to a different public key must not pair"
        );
        assert_eq!(registry.device_count(), 0);

        // And the same PIN bound to the right key still works, so the test
        // above is failing for the reason it claims rather than by accident.
        let good =
            pairing::compute_proof(&pin, HOST_ID, &request.client_nonce, &request.server_nonce)
                .unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, Some(&good))
                .is_ok()
        );
    }

    #[test]
    fn five_wrong_pins_discard_the_request() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Intruder", &nonce).unwrap();

        let wrong = pairing::compute_proof(
            "000001",
            HOST_ID,
            &request.client_nonce,
            &request.server_nonce,
        )
        .unwrap();
        for attempt in 1..=MAX_PIN_ATTEMPTS {
            assert!(
                registry
                    .finish_pairing(now, HOST_ID, &request.id, Some(&wrong))
                    .is_err(),
                "attempt {attempt}"
            );
        }
        assert!(
            registry.pending(now).is_empty(),
            "the request must be gone once the attempts are spent"
        );
    }

    #[test]
    fn the_right_pin_after_a_lockout_is_too_late() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Intruder", &nonce).unwrap();
        let pin = request.pin.clone().unwrap();

        let wrong = pairing::compute_proof(
            "000001",
            HOST_ID,
            &request.client_nonce,
            &request.server_nonce,
        )
        .unwrap();
        for _ in 0..MAX_PIN_ATTEMPTS {
            let _ = registry.finish_pairing(now, HOST_ID, &request.id, Some(&wrong));
        }

        let right =
            pairing::compute_proof(&pin, HOST_ID, &request.client_nonce, &request.server_nonce)
                .unwrap();
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, Some(&right))
                .is_err()
        );
        assert_eq!(registry.device_count(), 0);
    }

    #[test]
    fn a_request_expires() {
        let mut registry = with_pin();
        let opened = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(opened, "Laptop A", &nonce).unwrap();
        let pin = request.pin.clone().unwrap();

        let later = opened + PAIRING_WINDOW + std::time::Duration::from_secs(1);
        assert!(registry.pending(later).is_empty());

        let proof =
            pairing::compute_proof(&pin, HOST_ID, &request.client_nonce, &request.server_nonce)
                .unwrap();
        assert!(
            registry
                .finish_pairing(later, HOST_ID, &request.id, Some(&proof))
                .is_err()
        );
    }

    #[test]
    fn a_request_is_still_good_a_second_before_it_expires() {
        let mut registry = with_pin();
        let opened = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(opened, "Laptop A", &nonce).unwrap();
        let pin = request.pin.clone().unwrap();

        let just_before = opened + PAIRING_WINDOW - std::time::Duration::from_secs(1);
        let proof =
            pairing::compute_proof(&pin, HOST_ID, &request.client_nonce, &request.server_nonce)
                .unwrap();
        assert!(
            registry
                .finish_pairing(just_before, HOST_ID, &request.id, Some(&proof))
                .is_ok()
        );
    }

    // One machine reconnecting a few times must not fill the host's screen
    // with itself.
    #[test]
    fn a_device_retrying_replaces_its_own_request() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();

        let first = registry.begin_pairing(now, "Laptop A", &nonce).unwrap();
        let second = registry.begin_pairing(now, "Laptop A", &nonce).unwrap();

        assert_eq!(registry.pending(now).len(), 1);
        assert_ne!(first.id, second.id);
        assert_eq!(registry.pending(now)[0].id, second.id);
    }

    #[test]
    fn different_devices_each_get_their_own_request() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        registry.begin_pairing(now, "Laptop A", &nonce).unwrap();
        registry.begin_pairing(now, "Phone", &nonce).unwrap();
        assert_eq!(registry.pending(now).len(), 2);
    }

    #[test]
    fn a_flood_of_requests_is_refused_rather_than_burying_the_real_one() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        for i in 0..MAX_PENDING {
            registry
                .begin_pairing(now, &format!("Device {i}"), &nonce)
                .unwrap();
        }
        assert!(registry.begin_pairing(now, "One too many", &nonce).is_err());
        assert_eq!(registry.pending(now).len(), MAX_PENDING);
    }

    #[test]
    fn a_request_can_be_refused_at_the_host() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Someone", &nonce).unwrap();

        assert!(registry.deny(&request.id));
        assert!(registry.pending(now).is_empty());
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, None)
                .is_err()
        );
    }

    // A request created while a PIN was required must not become PIN-less
    // because the switch was flipped while it waited.
    #[test]
    fn changing_the_pin_setting_discards_waiting_requests() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(now, "Laptop A", &nonce).unwrap();

        registry.set_require_pin(false);
        assert!(registry.pending(now).is_empty());
        assert!(
            registry
                .finish_pairing(now, HOST_ID, &request.id, None)
                .is_err(),
            "a request made under the old rule must not complete under the new one"
        );
    }

    #[test]
    fn the_raw_token_is_never_stored() {
        let mut registry = with_pin();
        let token = pair(&mut registry, Instant::now(), HOST_ID, "Laptop A").unwrap();

        assert_ne!(registry.devices()[0].token_hash, token);
        assert!(
            !serde_json::to_string(registry.devices())
                .unwrap()
                .contains(&token),
            "a token must not be recoverable from what is written to disk"
        );
    }

    #[test]
    fn two_devices_pair_independently() {
        let mut registry = with_pin();
        let now = Instant::now();
        let first = pair(&mut registry, now, HOST_ID, "Laptop A").unwrap();
        let second = pair(&mut registry, now, HOST_ID, "Phone").unwrap();

        assert_ne!(first, second);
        assert_eq!(registry.device_count(), 2);
        assert!(registry.authenticate(&first).is_some());
        assert!(registry.authenticate(&second).is_some());
    }

    #[test]
    fn a_revoked_device_cannot_come_back() {
        let mut registry = with_pin();
        let token = pair(&mut registry, Instant::now(), HOST_ID, "Laptop A").unwrap();

        let hash = registry.devices()[0].token_hash.clone();
        assert!(registry.revoke(&hash));
        assert!(registry.authenticate(&token).is_none());
        assert!(!registry.revoke(&hash), "revoking twice changes nothing");
    }

    #[test]
    fn a_device_can_be_made_read_only_and_renamed() {
        let mut registry = with_pin();
        let token = pair(&mut registry, Instant::now(), HOST_ID, "Laptop A").unwrap();
        let hash = registry.devices()[0].token_hash.clone();

        assert!(registry.set_writable(&hash, false));
        assert!(registry.rename(&hash, "  Study laptop  "));

        let device = registry.authenticate(&token).unwrap();
        assert!(!device.writable);
        assert_eq!(device.name, "Study laptop");
    }

    #[test]
    fn a_made_up_token_authenticates_nobody() {
        let mut registry = with_pin();
        pair(&mut registry, Instant::now(), HOST_ID, "Laptop A").unwrap();

        assert!(registry.authenticate("").is_none());
        assert!(registry.authenticate("deadbeef").is_none());
        assert!(registry.authenticate(&"0".repeat(64)).is_none());
    }

    #[test]
    fn connecting_updates_when_the_device_was_last_seen() {
        let mut registry = with_pin();
        let token = pair(&mut registry, Instant::now(), HOST_ID, "Laptop A").unwrap();

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
    fn a_request_counts_down() {
        let mut registry = with_pin();
        let opened = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing(opened, "Laptop A", &nonce).unwrap();

        assert_eq!(request.remaining(opened), PAIRING_WINDOW);
        assert_eq!(
            request.remaining(opened + PAIRING_WINDOW / 2),
            PAIRING_WINDOW / 2
        );
        assert!(request.remaining(opened + PAIRING_WINDOW * 2).is_zero());
        assert!(request.expired(opened + PAIRING_WINDOW * 2));
    }

    // -----------------------------------------------------------------------
    // Knowing a device when it comes back
    // -----------------------------------------------------------------------

    const DEVICE_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DEVICE_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// Pairs the way a current client does: with its own id.
    fn pair_as(registry: &mut Registry, now: Instant, name: &str, id: &str) -> String {
        let nonce = pairing::random_nonce().unwrap();
        let request = registry.begin_pairing_for(now, name, id, &nonce).unwrap();
        let proof = request.pin.as_ref().map(|pin| {
            pairing::compute_proof(pin, HOST_ID, &request.client_nonce, &request.server_nonce)
                .unwrap()
        });
        registry
            .finish_pairing(now, HOST_ID, &request.id, proof.as_deref())
            .unwrap()
    }

    /// The bug this replaces: devices were recorded under whatever the last
    /// pairing message said, and clients sent the host's own name there.
    #[test]
    fn a_device_is_listed_under_the_name_it_asked_with() {
        let mut registry = with_pin();
        let token = pair_as(&mut registry, Instant::now(), "Laptop A", DEVICE_A);
        let device = registry.authenticate(&token).unwrap();
        assert_eq!(device.name, "Laptop A");
        assert_eq!(device.device_id, DEVICE_A);
    }

    /// Pairing again is the same device with a new key, not a second device.
    #[test]
    fn pairing_again_replaces_the_device_rather_than_adding_it() {
        let mut registry = with_pin();
        let now = Instant::now();
        let first = pair_as(&mut registry, now, "Laptop A", DEVICE_A);
        assert!(registry.set_writable(&hash_token(&first), false));

        let second = pair_as(&mut registry, now, "Laptop A", DEVICE_A);
        assert_eq!(registry.device_count(), 1);
        assert!(
            registry.authenticate(&first).is_none(),
            "the old key is gone"
        );
        let device = registry.authenticate(&second).expect("the new key works");
        assert!(!device.writable, "what the host decided about it stands");
    }

    #[test]
    fn two_machines_that_share_a_name_are_two_devices() {
        let mut registry = with_pin();
        let now = Instant::now();
        pair_as(&mut registry, now, "Laptop", DEVICE_A);
        pair_as(&mut registry, now, "Laptop", DEVICE_B);
        assert_eq!(registry.device_count(), 2);
    }

    #[test]
    fn a_waiting_request_is_replaced_by_the_same_device_not_the_same_name() {
        let mut registry = with_pin();
        let now = Instant::now();
        let nonce = pairing::random_nonce().unwrap();
        registry
            .begin_pairing_for(now, "Laptop", DEVICE_A, &nonce)
            .unwrap();
        registry
            .begin_pairing_for(now, "Laptop", DEVICE_B, &nonce)
            .unwrap();
        assert_eq!(registry.pending(now).len(), 2);
        registry
            .begin_pairing_for(now, "Laptop, renamed", DEVICE_A, &nonce)
            .unwrap();
        assert_eq!(registry.pending(now).len(), 2);
    }

    /// Devices paired before ids existed say what they are when they next
    /// connect, and from then on are recognised like any other.
    #[test]
    fn a_device_from_before_ids_learns_its_id_and_name_on_connecting() {
        let mut registry = with_pin();
        let now = Instant::now();
        let token = pair(&mut registry, now, HOST_ID, "The Host's Own Name").unwrap();
        let hash = hash_token(&token);
        assert!(registry.device(&hash).unwrap().device_id.is_empty());

        assert!(registry.observe(&hash, DEVICE_A, "Laptop A"));
        let device = registry.device(&hash).unwrap();
        assert_eq!(device.device_id, DEVICE_A);
        assert_eq!(device.name, "Laptop A");
        assert!(
            !registry.observe(&hash, DEVICE_A, "Laptop A"),
            "nothing new"
        );

        pair_as(&mut registry, now, "Laptop A", DEVICE_A);
        assert_eq!(registry.device_count(), 1);
    }

    #[test]
    fn a_name_given_on_the_host_outlives_the_one_the_device_uses() {
        let mut registry = with_pin();
        let now = Instant::now();
        let token = pair_as(&mut registry, now, "DESKTOP-7Q2", DEVICE_A);
        let hash = hash_token(&token);
        assert!(registry.rename(&hash, "Kitchen laptop"));

        assert!(!registry.observe(&hash, DEVICE_A, "DESKTOP-7Q2"));
        assert_eq!(registry.device(&hash).unwrap().name, "Kitchen laptop");

        let again = pair_as(&mut registry, now, "DESKTOP-7Q2", DEVICE_A);
        assert_eq!(
            registry.authenticate(&again).unwrap().name,
            "Kitchen laptop"
        );
    }

    #[test]
    fn devices_without_an_id_are_let_go_after_a_month_unseen() {
        let now = 1_800_000_000;
        let day = 24 * 60 * 60;
        let device = |name: &str, id: &str, seen: i64| Device {
            token_hash: format!("{name}-hash"),
            name: name.into(),
            paired_at: seen,
            last_seen: seen,
            writable: true,
            device_id: id.into(),
            named_by_host: false,
        };
        let mut registry = Registry::new(
            vec![
                device("abandoned", "", now - 31 * day),
                device("recent", "", now - day),
                device("known", DEVICE_A, now - 400 * day),
            ],
            true,
        );
        assert_eq!(registry.forget_abandoned(now), 1);
        let names: Vec<&str> = registry.devices().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["recent", "known"]);
    }

    #[test]
    fn anything_that_is_not_plainly_an_id_is_treated_as_none() {
        assert_eq!(sanitise_device_id(DEVICE_A), DEVICE_A);
        assert_eq!(
            sanitise_device_id("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            DEVICE_A
        );
        assert_eq!(sanitise_device_id("abc"), "");
        assert_eq!(sanitise_device_id("../../../etc/passwd/aaaaaaaaaa"), "");
        assert_eq!(sanitise_device_id(&"a".repeat(200)), "");
    }

    #[test]
    fn a_device_is_filed_under_its_id_once_it_has_one() {
        let mut registry = with_pin();
        let now = Instant::now();
        let token = pair(&mut registry, now, HOST_ID, "Old laptop").unwrap();
        let hash = hash_token(&token);
        assert_eq!(registry.device(&hash).unwrap().key(), hash);
        registry.observe(&hash, DEVICE_B, "Old laptop");
        assert_eq!(registry.device(&hash).unwrap().key(), DEVICE_B);
    }
}
