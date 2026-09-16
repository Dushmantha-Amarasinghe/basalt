//! PIN pairing.
//!
//! First contact is the only moment this design is vulnerable. Afterwards the
//! client has the host's public key pinned and a machine in the middle cannot
//! do anything useful. So the whole job here is to make sure the key that gets
//! pinned belongs to the host the user is looking at, and not to whatever else
//! answered.
//!
//! The mechanism:
//!
//! ```text
//! key   = PBKDF2-HMAC-SHA256(pin, salt = host_id, 120_000 iterations)
//! proof = HMAC-SHA256(key, client_nonce ‖ server_nonce)
//! ```
//!
//! Two properties matter, and both come from where the host id sits.
//!
//! **It is the salt, so the proof is bound to a specific public key.** A client
//! derives its key from the certificate it actually received. A machine in the
//! middle presents its own certificate, so the client's key is derived from
//! *that* key's hash, and the proof it produces is worthless against the real
//! host — which derives from its own. Forwarding the exchange does not work
//! either, for the same reason. Without this the attack is trivial: intercept
//! first contact, present your own certificate, and the client pins you
//! forever.
//!
//! **PBKDF2 rather than using the PIN directly as a key.** Six digits is a
//! million possibilities, which is nothing to brute force offline if someone
//! captures a proof. At 120,000 iterations, searching the whole space costs
//! around 10^11 HMAC operations instead of 10^6. Pairing happens once and the
//! derivation takes about a tenth of a second, so this is paid for by nobody.
//!
//! A PAKE such as SPAKE2 would remove the offline attack rather than merely
//! pricing it, and is the right answer if this ever leaves the LAN. It is a
//! considerably larger piece of machinery and, with a three-minute window and a
//! five-attempt lockout in front of it, it is not what is protecting anyone
//! here.

use std::num::NonZeroU32;

use basalt_proto::hex;
use ring::rand::SecureRandom;

use crate::{NetError, Result};

/// Iterations for the PIN key derivation.
///
/// Chosen so that deriving once is imperceptible during pairing while a brute
/// force over the six-digit space is not worth starting.
const PBKDF2_ITERATIONS: u32 = 120_000;

/// Digits in a pairing PIN.
pub const PIN_DIGITS: usize = 6;

/// Bytes in a nonce, a device token, or a derived key.
pub const NONCE_BYTES: usize = 32;
pub const TOKEN_BYTES: usize = 32;
const KEY_BYTES: usize = 32;

/// Wrong PINs accepted before the host closes pairing and generates a new one.
pub const MAX_PIN_ATTEMPTS: u32 = 5;

/// How long a pairing window stays open.
pub const PAIRING_WINDOW: std::time::Duration = std::time::Duration::from_secs(180);

/// Fills a buffer from the system CSPRNG.
pub fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; len];
    ring::rand::SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| NetError::Crypto("the system random number generator failed".into()))?;
    Ok(out)
}

/// A fresh nonce, hex encoded.
pub fn random_nonce() -> Result<String> {
    Ok(hex::encode(&random_bytes(NONCE_BYTES)?))
}

/// A fresh device token, hex encoded.
pub fn random_token() -> Result<String> {
    Ok(hex::encode(&random_bytes(TOKEN_BYTES)?))
}

/// Generates a uniformly distributed PIN.
///
/// Rejection sampling rather than `% 1_000_000`: the modulo is biased, because
/// 2^32 is not a multiple of a million, and low PINs would come up very
/// slightly more often. The bias is tiny and the fix is four lines, so there is
/// no argument for taking it.
pub fn generate_pin() -> Result<String> {
    const RANGE: u32 = 1_000_000;
    let limit = u32::MAX - (u32::MAX % RANGE);
    loop {
        let bytes = random_bytes(4)?;
        let value = u32::from_le_bytes(bytes.try_into().expect("asked for 4 bytes"));
        if value < limit {
            return Ok(format!("{:0>width$}", value % RANGE, width = PIN_DIGITS));
        }
    }
}

/// Whether a string is a well-formed PIN.
pub fn is_valid_pin(pin: &str) -> bool {
    pin.len() == PIN_DIGITS && pin.chars().all(|c| c.is_ascii_digit())
}

/// Computes the pairing proof.
///
/// Both ends run this identically; the host compares its own result against
/// what arrived. `host_id` is hex and must be the id of the certificate the
/// *client actually received*, never one supplied in a message — taking it from
/// a message is precisely the hole this is here to close.
pub fn compute_proof(
    pin: &str,
    host_id: &str,
    client_nonce: &str,
    server_nonce: &str,
) -> Result<String> {
    if !is_valid_pin(pin) {
        return Err(NetError::Crypto(format!(
            "a PIN is {PIN_DIGITS} digits; got {pin:?}"
        )));
    }
    let salt = hex::decode(host_id).map_err(|_| NetError::Crypto("host id is not hex".into()))?;
    if salt.is_empty() {
        return Err(NetError::Crypto("host id is empty".into()));
    }
    let client = hex::decode(client_nonce)
        .map_err(|_| NetError::Crypto("client nonce is not hex".into()))?;
    let server = hex::decode(server_nonce)
        .map_err(|_| NetError::Crypto("server nonce is not hex".into()))?;
    if client.len() != NONCE_BYTES || server.len() != NONCE_BYTES {
        return Err(NetError::Crypto(format!(
            "nonces must be {NONCE_BYTES} bytes"
        )));
    }

    let mut key = [0u8; KEY_BYTES];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PBKDF2_ITERATIONS).expect("iteration count is not zero"),
        &salt,
        pin.as_bytes(),
        &mut key,
    );

    let mut message = Vec::with_capacity(NONCE_BYTES * 2);
    message.extend_from_slice(&client);
    message.extend_from_slice(&server);

    let tag = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
        &message,
    );
    Ok(hex::encode(tag.as_ref()))
}

/// Checks a proof without leaking, through timing, how much of it was right.
pub fn verify_proof(
    pin: &str,
    host_id: &str,
    client_nonce: &str,
    server_nonce: &str,
    presented: &str,
) -> bool {
    match compute_proof(pin, host_id, client_nonce, server_nonce) {
        Ok(expected) => hex::constant_time_eq(expected.as_bytes(), presented.as_bytes()),
        Err(_) => false,
    }
}

/// Formats a PIN for display as `123 456`.
pub fn format_pin(pin: &str) -> String {
    if pin.len() == PIN_DIGITS {
        format!("{} {}", &pin[..3], &pin[3..])
    } else {
        pin.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_A: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const HOST_B: &str = "bb00000000000000000000000000000000000000000000000000000000000002";

    fn nonces() -> (String, String) {
        (random_nonce().unwrap(), random_nonce().unwrap())
    }

    #[test]
    fn generated_pins_are_six_digits() {
        for _ in 0..50 {
            let pin = generate_pin().unwrap();
            assert_eq!(pin.len(), PIN_DIGITS);
            assert!(pin.chars().all(|c| c.is_ascii_digit()), "{pin}");
            assert!(is_valid_pin(&pin));
        }
    }

    #[test]
    fn generated_pins_are_not_all_the_same() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            seen.insert(generate_pin().unwrap());
        }
        assert!(seen.len() > 25, "PIN generation looks degenerate");
    }

    #[test]
    fn leading_zeros_survive_formatting() {
        assert_eq!(format_pin("000123"), "000 123");
        assert!(is_valid_pin("000000"));
    }

    #[test]
    fn malformed_pins_are_rejected() {
        for bad in ["", "12345", "1234567", "12345a", "12 345", "abcdef"] {
            assert!(!is_valid_pin(bad), "{bad:?} must not be a valid PIN");
        }
    }

    #[test]
    fn the_right_pin_verifies() {
        let (c, s) = nonces();
        let proof = compute_proof("123456", HOST_A, &c, &s).unwrap();
        assert!(verify_proof("123456", HOST_A, &c, &s, &proof));
    }

    #[test]
    fn a_wrong_pin_does_not_verify() {
        let (c, s) = nonces();
        let proof = compute_proof("123456", HOST_A, &c, &s).unwrap();
        assert!(!verify_proof("123457", HOST_A, &c, &s, &proof));
        assert!(!verify_proof("000000", HOST_A, &c, &s, &proof));
    }

    // The attack this whole module exists to stop: someone in the middle
    // presents their own certificate during first contact, and the client pins
    // them instead of the host.
    #[test]
    fn a_proof_made_against_another_key_is_worthless() {
        let (c, s) = nonces();
        let against_impostor = compute_proof("123456", HOST_B, &c, &s).unwrap();
        assert!(
            !verify_proof("123456", HOST_A, &c, &s, &against_impostor),
            "a proof bound to a different public key must not verify"
        );
    }

    #[test]
    fn a_captured_proof_cannot_be_replayed_against_new_nonces() {
        let (c1, s1) = nonces();
        let captured = compute_proof("123456", HOST_A, &c1, &s1).unwrap();
        let (c2, s2) = nonces();
        assert!(!verify_proof("123456", HOST_A, &c2, &s2, &captured));
        // Changing only one side is enough to break it.
        assert!(!verify_proof("123456", HOST_A, &c1, &s2, &captured));
        assert!(!verify_proof("123456", HOST_A, &c2, &s1, &captured));
    }

    #[test]
    fn the_same_inputs_always_give_the_same_proof() {
        let (c, s) = nonces();
        let a = compute_proof("246810", HOST_A, &c, &s).unwrap();
        let b = compute_proof("246810", HOST_A, &c, &s).unwrap();
        assert_eq!(a, b, "both ends must be able to compute this independently");
    }

    #[test]
    fn malformed_inputs_are_errors_rather_than_silent_failures() {
        let (c, s) = nonces();
        assert!(compute_proof("12345", HOST_A, &c, &s).is_err(), "short pin");
        assert!(
            compute_proof("123456", "zz", &c, &s).is_err(),
            "bad host id"
        );
        assert!(
            compute_proof("123456", "", &c, &s).is_err(),
            "empty host id"
        );
        assert!(
            compute_proof("123456", HOST_A, "abcd", &s).is_err(),
            "short nonce"
        );
    }

    #[test]
    fn verification_of_garbage_returns_false_rather_than_panicking() {
        let (c, s) = nonces();
        assert!(!verify_proof("123456", HOST_A, &c, &s, ""));
        assert!(!verify_proof("123456", HOST_A, &c, &s, "not hex"));
        assert!(!verify_proof("bad", HOST_A, &c, &s, "whatever"));
    }

    #[test]
    fn nonces_and_tokens_are_distinct_every_time() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20 {
            assert!(seen.insert(random_nonce().unwrap()));
            assert!(seen.insert(random_token().unwrap()));
        }
    }

    #[test]
    fn tokens_are_full_length() {
        let token = random_token().unwrap();
        assert_eq!(hex::decode(&token).unwrap().len(), TOKEN_BYTES);
    }
}
