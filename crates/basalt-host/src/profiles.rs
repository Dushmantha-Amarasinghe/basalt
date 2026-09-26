//! Profiles: who is watching, as opposed to which device.
//!
//! A device is paired once and gets into the drive. A profile is a person in
//! the household: their watch history and their starred files follow them from
//! the laptop to the television, which a device's own history cannot do.
//! Nobody has to use one — a device can carry on as itself, as before — and
//! any paired device may make one.
//!
//! **A PIN, not a password.** The device is already trusted to reach the
//! drive; the profile only says whose history this is. Four to eight digits is
//! what a family will actually remember, and what Netflix and Plex settled on
//! for the same job.
//!
//! **Stored like passwords anyway.** A PIN is hashed with Argon2id and a salt
//! of its own, and never leaves the host or appears in its window. Short PINs
//! are few, so the hash alone would not hold out against someone with the
//! config file; what holds out against guessing over the network is the
//! lockout, which doubles after every few wrong tries.
//!
//! **Sign-ins are tokens.** Signing in hands the device a random token, and
//! the host keeps only its hash — the same arrangement as device tokens. A
//! device that is remembered keeps its token; one that is not holds it only
//! until the app closes, and the host lets such tokens lapse after a day.

use std::collections::HashMap;

use basalt_proto::hex;
use basalt_proto::msg::ProfileView;
use serde::{Deserialize, Serialize};

use crate::error::HostError;

/// How long a sign-in nobody asked to remember lasts after its last use.
pub const SESSION_IDLE_SECS: i64 = 24 * 60 * 60;
/// How long a remembered sign-in lasts after its last use.
pub const REMEMBERED_IDLE_SECS: i64 = 180 * 24 * 60 * 60;
/// Profiles on one host.
pub const MAX_PROFILES: usize = 24;
/// Wrong PINs allowed before the profile starts to lock.
const FREE_TRIES: u32 = 4;
/// The first lock, doubling with every wrong try after it, to a limit.
const FIRST_LOCK_SECS: i64 = 30;
const LONGEST_LOCK_SECS: i64 = 15 * 60;
/// The avatar colours a device can offer.
pub const COLORS: u8 = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: u8,
    /// Argon2id, in the PHC string format. None once the host's owner reset
    /// it; the next sign-in chooses a new one.
    #[serde(default)]
    pub pin_hash: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub last_used: i64,
}

impl Profile {
    pub fn view(&self) -> ProfileView {
        ProfileView {
            id: self.id.clone(),
            name: self.name.clone(),
            color: self.color,
            has_pin: self.pin_hash.is_some(),
            last_used: self.last_used,
        }
    }
}

/// One device signed in to one profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileToken {
    /// Hex SHA-256 of the token. The token itself is never stored.
    pub token_hash: String,
    pub profile_id: String,
    /// Which device, by `Device::key`.
    pub device_key: String,
    /// Whether the device asked to stay signed in.
    pub remembered: bool,
    pub created_at: i64,
    pub last_used: i64,
}

/// Wrong PINs for one profile, kept in memory only: a restart forgetting them
/// costs an attacker a restart, which they cannot cause from a device.
#[derive(Debug, Default, Clone)]
struct Failures {
    count: u32,
    locked_until: i64,
}

#[derive(Debug, Default)]
pub struct ProfileBook {
    profiles: Vec<Profile>,
    tokens: Vec<ProfileToken>,
    failures: HashMap<String, Failures>,
}

pub fn hash_token(token: &str) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref())
}

/// Four to eight digits, and nothing else.
pub fn valid_pin(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

fn hash_pin(pin: &str) -> Result<String, HostError> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    // From the same source as every other secret the host makes.
    let bytes = basalt_net::pairing::random_bytes(16)
        .map_err(|e| HostError::BadRequest(format!("no randomness available: {e}")))?;
    let salt = SaltString::encode_b64(&bytes)
        .map_err(|e| HostError::BadRequest(format!("could not keep the PIN: {e}")))?;
    argon2::Argon2::default()
        .hash_password(pin.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| HostError::BadRequest(format!("could not keep the PIN: {e}")))
}

fn pin_matches(pin: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash).is_ok_and(|parsed| {
        argon2::Argon2::default()
            .verify_password(pin.as_bytes(), &parsed)
            .is_ok()
    })
}

fn clean_name(name: &str) -> Result<String, HostError> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(HostError::BadRequest("a profile needs a name".into()));
    }
    if name.chars().count() > 32 {
        return Err(HostError::BadRequest(
            "that name is longer than 32 letters".into(),
        ));
    }
    Ok(name)
}

impl ProfileBook {
    pub fn new(profiles: Vec<Profile>, tokens: Vec<ProfileToken>) -> Self {
        Self {
            profiles,
            tokens,
            failures: HashMap::new(),
        }
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn tokens(&self) -> &[ProfileToken] {
        &self.tokens
    }

    pub fn views(&self) -> Vec<ProfileView> {
        self.profiles.iter().map(Profile::view).collect()
    }

    pub fn find(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Makes a profile and signs the device in to it. Returns the token.
    pub fn create(
        &mut self,
        name: &str,
        pin: &str,
        color: u8,
        device_key: &str,
        remember: bool,
        now: i64,
    ) -> Result<(Profile, String), HostError> {
        let name = clean_name(name)?;
        if !valid_pin(pin) {
            return Err(HostError::BadRequest("a PIN is 4 to 8 digits".into()));
        }
        if self
            .profiles
            .iter()
            .any(|p| p.name.to_lowercase() == name.to_lowercase())
        {
            return Err(HostError::Exists(format!(
                "there is already a profile called {name}"
            )));
        }
        if self.profiles.len() >= MAX_PROFILES {
            return Err(HostError::BadRequest(format!(
                "a host keeps up to {MAX_PROFILES} profiles"
            )));
        }
        let id = basalt_net::pairing::random_token()
            .map_err(|e| HostError::BadRequest(format!("no randomness available: {e}")))?[..16]
            .to_string();
        let profile = Profile {
            id,
            name,
            color: color % COLORS,
            pin_hash: Some(hash_pin(pin)?),
            created_at: now,
            last_used: now,
        };
        self.profiles.push(profile.clone());
        let token = self.issue(&profile.id, device_key, remember, now)?;
        Ok((profile, token))
    }

    /// Checks a PIN and signs the device in. Returns the token.
    ///
    /// A profile whose PIN was reset takes the PIN given as its new one.
    pub fn sign_in(
        &mut self,
        id: &str,
        pin: &str,
        device_key: &str,
        remember: bool,
        now: i64,
    ) -> Result<(Profile, String), HostError> {
        let failures = self.failures.entry(id.to_string()).or_default();
        if failures.locked_until > now {
            let wait = failures.locked_until - now;
            return Err(HostError::Denied(format!(
                "too many wrong PINs; try again in {}",
                if wait >= 60 {
                    format!("{} minutes", (wait + 59) / 60)
                } else {
                    format!("{wait} seconds")
                }
            )));
        }
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| HostError::NotFound("that profile".into()))?;

        match self.profiles[index].pin_hash.clone() {
            Some(hash) => {
                if !pin_matches(pin, &hash) {
                    let failures = self.failures.entry(id.to_string()).or_default();
                    failures.count += 1;
                    if failures.count > FREE_TRIES {
                        let doublings = (failures.count - FREE_TRIES - 1).min(10);
                        let lock = (FIRST_LOCK_SECS << doublings).min(LONGEST_LOCK_SECS);
                        failures.locked_until = now + lock;
                    }
                    return Err(HostError::Denied("that PIN is not right".into()));
                }
            }
            None => {
                if !valid_pin(pin) {
                    return Err(HostError::BadRequest("a PIN is 4 to 8 digits".into()));
                }
                self.profiles[index].pin_hash = Some(hash_pin(pin)?);
            }
        }
        self.failures.remove(id);
        self.profiles[index].last_used = now;
        let profile = self.profiles[index].clone();
        let token = self.issue(id, device_key, remember, now)?;
        Ok((profile, token))
    }

    /// A new token for a device, replacing whatever it held for this profile.
    fn issue(
        &mut self,
        profile_id: &str,
        device_key: &str,
        remember: bool,
        now: i64,
    ) -> Result<String, HostError> {
        let token = basalt_net::pairing::random_token()
            .map_err(|e| HostError::BadRequest(format!("no randomness available: {e}")))?;
        self.tokens
            .retain(|t| !(t.profile_id == profile_id && t.device_key == device_key));
        self.tokens.push(ProfileToken {
            token_hash: hash_token(&token),
            profile_id: profile_id.to_string(),
            device_key: device_key.to_string(),
            remembered: remember,
            created_at: now,
            last_used: now,
        });
        Ok(token)
    }

    /// The profile a token stands for, if it still does. Notes its use.
    pub fn resolve(&mut self, token: &str, now: i64) -> Option<Profile> {
        let hash = hash_token(token);
        let entry = self.tokens.iter_mut().find(|t| t.token_hash == hash)?;
        let limit = if entry.remembered {
            REMEMBERED_IDLE_SECS
        } else {
            SESSION_IDLE_SECS
        };
        if now - entry.last_used > limit {
            return None;
        }
        entry.last_used = now;
        let id = entry.profile_id.clone();
        let profile = self.profiles.iter_mut().find(|p| p.id == id)?;
        profile.last_used = now;
        Some(profile.clone())
    }

    /// Whether a sign-in a connection is acting on still stands.
    pub fn still_signed_in(&self, profile_id: &str, token_hash: &str) -> bool {
        self.tokens
            .iter()
            .any(|t| t.token_hash == token_hash && t.profile_id == profile_id)
    }

    /// Ends one sign-in.
    pub fn sign_out(&mut self, token: &str) -> bool {
        let hash = hash_token(token);
        let before = self.tokens.len();
        self.tokens.retain(|t| t.token_hash != hash);
        self.tokens.len() != before
    }

    /// Clears a profile's PIN and signs it out everywhere. The next sign-in
    /// on any device chooses a new PIN.
    pub fn reset_pin(&mut self, id: &str) -> bool {
        let Some(profile) = self.profiles.iter_mut().find(|p| p.id == id) else {
            return false;
        };
        profile.pin_hash = None;
        self.tokens.retain(|t| t.profile_id != id);
        self.failures.remove(id);
        true
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<bool, HostError> {
        let name = clean_name(name)?;
        if self
            .profiles
            .iter()
            .any(|p| p.id != id && p.name.to_lowercase() == name.to_lowercase())
        {
            return Err(HostError::Exists(format!(
                "there is already a profile called {name}"
            )));
        }
        Ok(match self.profiles.iter_mut().find(|p| p.id == id) {
            Some(profile) => {
                profile.name = name;
                true
            }
            None => false,
        })
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        self.tokens.retain(|t| t.profile_id != id);
        self.failures.remove(id);
        self.profiles.len() != before
    }

    /// Signs a device out of every profile: it was unpaired.
    pub fn forget_device(&mut self, device_key: &str) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| t.device_key != device_key);
        self.tokens.len() != before
    }

    /// Drops sign-ins past their time. Returns whether anything went.
    pub fn prune(&mut self, now: i64) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| {
            let limit = if t.remembered {
                REMEMBERED_IDLE_SECS
            } else {
                SESSION_IDLE_SECS
            };
            now - t.last_used <= limit
        });
        self.tokens.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> ProfileBook {
        ProfileBook::new(Vec::new(), Vec::new())
    }

    #[test]
    fn a_profile_is_made_and_signed_in_to() {
        let mut b = book();
        let (profile, token) = b.create("  Maya ", "4821", 3, "dev-a", true, 100).unwrap();
        assert_eq!(profile.name, "Maya");
        assert!(
            profile
                .pin_hash
                .as_deref()
                .unwrap()
                .starts_with("$argon2id$")
        );
        assert!(!profile.pin_hash.as_deref().unwrap().contains("4821"));
        assert_eq!(b.resolve(&token, 101).unwrap().id, profile.id);
    }

    #[test]
    fn names_and_pins_are_checked() {
        let mut b = book();
        assert!(b.create("", "1234", 0, "d", false, 1).is_err());
        assert!(b.create("Maya", "12", 0, "d", false, 1).is_err());
        assert!(b.create("Maya", "12ab", 0, "d", false, 1).is_err());
        assert!(b.create("Maya", "123456789", 0, "d", false, 1).is_err());
        b.create("Maya", "1234", 0, "d", false, 1).unwrap();
        assert!(
            b.create("maya", "5678", 0, "d", false, 1).is_err(),
            "names are unique"
        );
    }

    #[test]
    fn the_right_pin_signs_in_and_a_wrong_one_does_not() {
        let mut b = book();
        let (p, _) = b.create("Maya", "4821", 0, "dev-a", false, 1).unwrap();
        assert!(b.sign_in(&p.id, "0000", "dev-b", false, 2).is_err());
        let (_, token) = b.sign_in(&p.id, "4821", "dev-b", false, 3).unwrap();
        assert!(b.resolve(&token, 4).is_some());
    }

    #[test]
    fn guessing_locks_the_profile_for_longer_each_time() {
        let mut b = book();
        let (p, _) = b.create("Maya", "4821", 0, "d", false, 0).unwrap();
        for _ in 0..FREE_TRIES {
            assert!(b.sign_in(&p.id, "0000", "d", false, 10).is_err());
        }
        // The next wrong one locks it; now even the right PIN waits.
        assert!(b.sign_in(&p.id, "0000", "d", false, 10).is_err());
        let err = b.sign_in(&p.id, "4821", "d", false, 20).unwrap_err();
        assert!(err.to_string().contains("try again"), "{err}");

        // Another wrong one once the lock is over locks it for twice as long.
        let after_first = 10 + FIRST_LOCK_SECS + 1;
        assert!(b.sign_in(&p.id, "0000", "d", false, after_first).is_err());
        assert!(
            b.sign_in(&p.id, "4821", "d", false, after_first + FIRST_LOCK_SECS + 1)
                .is_err()
        );
        let after_second = after_first + FIRST_LOCK_SECS * 2 + 1;
        assert!(b.sign_in(&p.id, "4821", "d", false, after_second).is_ok());

        // The right PIN clears the count: one slip afterwards is just a slip.
        assert!(
            b.sign_in(&p.id, "0000", "d", false, after_second + 1)
                .is_err()
        );
        assert!(
            b.sign_in(&p.id, "4821", "d", false, after_second + 2)
                .is_ok()
        );
    }

    #[test]
    fn a_session_lapses_and_a_remembered_one_lasts() {
        let mut b = book();
        let (_, short) = b.create("Maya", "4821", 0, "a", false, 0).unwrap();
        let (_, long) = b.create("Sam", "4821", 0, "a", true, 0).unwrap();
        let later = SESSION_IDLE_SECS + 10;
        assert!(b.resolve(&short, later).is_none());
        assert!(b.resolve(&long, later).is_some());
        assert!(b.prune(later));
        assert_eq!(b.tokens().len(), 1);
    }

    #[test]
    fn signing_out_ends_only_that_sign_in() {
        let mut b = book();
        let (p, on_a) = b.create("Maya", "4821", 0, "a", true, 0).unwrap();
        let (_, on_b) = b.sign_in(&p.id, "4821", "b", true, 1).unwrap();
        assert!(b.sign_out(&on_a));
        assert!(b.resolve(&on_a, 2).is_none());
        assert!(b.resolve(&on_b, 2).is_some());
    }

    #[test]
    fn signing_in_again_on_a_device_replaces_its_old_token() {
        let mut b = book();
        let (p, first) = b.create("Maya", "4821", 0, "a", true, 0).unwrap();
        let (_, second) = b.sign_in(&p.id, "4821", "a", true, 1).unwrap();
        assert!(b.resolve(&first, 2).is_none());
        assert!(b.resolve(&second, 2).is_some());
        assert_eq!(b.tokens().len(), 1);
    }

    #[test]
    fn a_reset_pin_signs_out_everywhere_and_the_next_sign_in_sets_one() {
        let mut b = book();
        let (p, token) = b.create("Maya", "4821", 0, "a", true, 0).unwrap();
        assert!(b.reset_pin(&p.id));
        assert!(b.resolve(&token, 1).is_none());
        assert!(!b.find(&p.id).unwrap().view().has_pin);
        b.sign_in(&p.id, "7777", "a", true, 2).unwrap();
        assert!(
            b.sign_in(&p.id, "4821", "a", true, 3).is_err(),
            "the old PIN is gone"
        );
        assert!(b.sign_in(&p.id, "7777", "a", true, 4).is_ok());
    }

    #[test]
    fn removing_a_profile_ends_its_sign_ins() {
        let mut b = book();
        let (p, token) = b.create("Maya", "4821", 0, "a", true, 0).unwrap();
        assert!(b.remove(&p.id));
        assert!(b.resolve(&token, 1).is_none());
        assert!(b.profiles().is_empty());
    }

    #[test]
    fn an_unpaired_device_is_signed_out_of_everything() {
        let mut b = book();
        let (_, token) = b.create("Maya", "4821", 0, "a", true, 0).unwrap();
        b.create("Sam", "4821", 0, "b", true, 0).unwrap();
        assert!(b.forget_device("a"));
        assert!(b.resolve(&token, 1).is_none());
        assert_eq!(b.tokens().len(), 1);
    }
}
