//! Hex encoding.
//!
//! Twenty lines rather than a dependency. Every byte string that crosses the
//! wire or lands in a config file goes through here — tokens, nonces, SPKI
//! hashes, BLAKE3 digests — so it is worth having it be obviously correct and
//! obviously constant-shaped.

use crate::{ProtoError, Result};

const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0F) as usize] as char);
    }
    out
}

pub fn decode(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(ProtoError::Malformed(format!(
            "hex string has an odd length ({})",
            s.len()
        )));
    }
    let (pairs, _) = s.as_bytes().as_chunks::<2>();
    let mut out = Vec::with_capacity(pairs.len());
    for [hi, lo] in pairs {
        out.push((nibble(*hi)? << 4) | nibble(*lo)?);
    }
    Ok(out)
}

fn nibble(c: u8) -> Result<u8> {
    Ok(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        other => {
            return Err(ProtoError::Malformed(format!(
                "{:?} is not a hex digit",
                other as char
            )));
        }
    })
}

/// Compares two byte strings without leaking where they first differ.
///
/// Tokens and pairing proofs are compared with this rather than `==`. On a LAN
/// the timing signal from an early-exit comparison is buried in milliseconds of
/// Wi-Fi jitter and almost certainly unexploitable — but "almost certainly" is
/// not a good reason to write the weaker version when the stronger one is three
/// lines.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_round_trips() {
        for case in [
            &b""[..],
            &b"\x00"[..],
            &b"\xff"[..],
            &b"\x00\x01\x02\xfe\xff"[..],
            &[0x5Au8; 32][..],
        ] {
            assert_eq!(decode(&encode(case)).unwrap(), case);
        }
    }

    #[test]
    fn output_is_lowercase_and_zero_padded() {
        assert_eq!(encode(&[0x0A, 0xB0]), "0ab0");
        assert_eq!(encode(&[0, 0, 0]), "000000");
    }

    #[test]
    fn uppercase_input_decodes_too() {
        assert_eq!(decode("DEADBEEF").unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(decode("abc").is_err(), "odd length");
        assert!(decode("zz").is_err(), "not hex");
        assert!(decode("ab cd").is_err(), "space is not hex");
        assert!(decode("ab\0").is_err(), "NUL is not hex");
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokeN"));
        assert!(!constant_time_eq(b"token", b"token "));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
