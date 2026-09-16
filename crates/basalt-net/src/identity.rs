//! The host's cryptographic identity.
//!
//! A Basalt host is identified by the SHA-256 of its certificate's
//! **SubjectPublicKeyInfo** — the public key together with its algorithm — not
//! by the certificate as a whole. Hashing the whole certificate would work
//! until the day the certificate is reissued, at which point every paired
//! device would refuse to connect and the only fix would be re-pairing all of
//! them. The key can outlive any number of certificates.
//!
//! There is no certificate authority anywhere in this design and there should
//! not be. On a home network there is nobody to be an authority; what matters
//! is that the machine answering today is the same machine that answered when
//! the user typed the PIN, and a pinned key says exactly that and nothing more.

use basalt_proto::hex;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::{NetError, Result};

/// A host certificate and its private key, with the identity already computed.
#[derive(Debug, Clone)]
pub struct HostIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    /// Hex SHA-256 of the SPKI. This is what a client pins.
    pub host_id: String,
}

impl HostIdentity {
    /// Generates a fresh identity.
    ///
    /// The subject names are cosmetic: no client validates them, because
    /// hostnames on a home LAN are assigned by a router and change. The pin is
    /// the whole of the authentication.
    pub fn generate(host_name: &str) -> Result<Self> {
        let names = vec![host_name.to_string(), "basalt".to_string()];
        let cert = rcgen::generate_simple_self_signed(names)
            .map_err(|e| NetError::Crypto(format!("generating certificate: {e}")))?;

        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.signing_key.serialize_der();
        let host_id = host_id_from_cert(&cert_der)?;

        Ok(Self {
            cert_der,
            key_der,
            host_id,
        })
    }

    /// Rebuilds an identity from stored DER, recomputing the id rather than
    /// trusting a stored copy of it.
    pub fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Self> {
        let host_id = host_id_from_cert(&cert_der)?;
        Ok(Self {
            cert_der,
            key_der,
            host_id,
        })
    }

    pub fn certificate(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn private_key(&self) -> Result<PrivateKeyDer<'static>> {
        PrivateKeyDer::try_from(self.key_der.clone())
            .map_err(|e| NetError::Crypto(format!("reading private key: {e}")))
    }
}

/// Computes the host id for a DER-encoded certificate.
pub fn host_id_from_cert(cert_der: &[u8]) -> Result<String> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert_der)
        .map_err(|e| NetError::Crypto(format!("parsing certificate: {e}")))?;
    let spki = parsed.tbs_certificate.subject_pki.raw;
    let digest = ring::digest::digest(&ring::digest::SHA256, spki);
    Ok(hex::encode(digest.as_ref()))
}

/// Shortens a host id for display.
///
/// The full 64 characters are unreadable and nobody compares them. Eight is
/// enough for a person to spot that two hosts are different, and the value
/// being compared for real is always the full one.
pub fn short_id(host_id: &str) -> String {
    host_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_identity_has_a_full_length_hex_id() {
        let id = HostIdentity::generate("laptop-b").unwrap();
        assert_eq!(id.host_id.len(), 64, "sha-256 is 32 bytes of hex");
        assert!(id.host_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!id.cert_der.is_empty());
        assert!(!id.key_der.is_empty());
    }

    #[test]
    fn the_id_is_stable_across_reloads() {
        let id = HostIdentity::generate("laptop-b").unwrap();
        let reloaded = HostIdentity::from_der(id.cert_der.clone(), id.key_der.clone()).unwrap();
        assert_eq!(reloaded.host_id, id.host_id);
    }

    #[test]
    fn two_hosts_get_different_ids() {
        let a = HostIdentity::generate("laptop-b").unwrap();
        let b = HostIdentity::generate("laptop-b").unwrap();
        assert_ne!(
            a.host_id, b.host_id,
            "the same subject name must not produce the same identity"
        );
    }

    // The reason the SPKI is hashed rather than the certificate: a reissued
    // certificate over the same key has to keep the same identity, or every
    // paired device would have to pair again.
    #[test]
    fn reissuing_a_certificate_over_the_same_key_keeps_the_identity() {
        let key = rcgen::KeyPair::generate().unwrap();
        let expected = {
            let mut params = rcgen::CertificateParams::new(vec!["basalt".to_string()]).unwrap();
            params.distinguished_name = rcgen::DistinguishedName::new();
            let cert = params.self_signed(&key).unwrap();
            host_id_from_cert(cert.der()).unwrap()
        };

        let mut params = rcgen::CertificateParams::new(vec!["something-else".to_string()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        let reissued = params.self_signed(&key).unwrap();

        assert_eq!(host_id_from_cert(reissued.der()).unwrap(), expected);
    }

    #[test]
    fn the_private_key_loads_back_into_rustls() {
        let id = HostIdentity::generate("laptop-b").unwrap();
        assert!(id.private_key().is_ok());
    }

    #[test]
    fn garbage_is_not_mistaken_for_a_certificate() {
        assert!(host_id_from_cert(b"not a certificate").is_err());
        assert!(host_id_from_cert(&[]).is_err());
    }

    #[test]
    fn short_ids_are_eight_characters_and_prefix_the_full_one() {
        let id = HostIdentity::generate("laptop-b").unwrap();
        let short = short_id(&id.host_id);
        assert_eq!(short.len(), 8);
        assert!(id.host_id.starts_with(&short));
    }
}
