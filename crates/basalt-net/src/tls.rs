//! TLS configuration for both ends.
//!
//! The client does not use the system trust store and does not check
//! hostnames. Both would be wrong here, not merely unnecessary:
//!
//! - There is no certificate authority that could vouch for a laptop in
//!   someone's house, so trust-store validation would reject every host
//!   unconditionally.
//! - The host's address is whatever the router handed out this week, so a
//!   hostname check would fail for reasons that have nothing to do with
//!   security and would train people to disable it.
//!
//! What replaces them is stricter than either: the client requires the exact
//! public key it saw when the user typed the PIN. A certificate signed by a
//! real authority for the right hostname is still refused if the key is not the
//! pinned one.

use std::sync::{Arc, Mutex, Once};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::identity::{HostIdentity, host_id_from_cert};
use crate::{NetError, Result};

/// Installs the ring crypto provider exactly once.
///
/// rustls 0.23 requires a process-wide provider and returns an error if one is
/// already installed. Doing it behind a `Once` means library code can call this
/// freely without the second call being an error, and without every caller
/// having to remember.
pub fn init_crypto() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Failure means something else installed a provider first, which is
        // fine: any provider that can do TLS 1.3 works for us.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Builds the host's TLS acceptor from its stored identity.
pub fn server_config(identity: &HostIdentity) -> Result<TlsAcceptor> {
    init_crypto();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate()], identity.private_key()?)
        .map_err(|e| NetError::Crypto(format!("building the server config: {e}")))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// What the client will accept from the far end.
#[derive(Debug, Clone)]
pub enum Trust {
    /// Require this exact host id. Normal operation.
    Pinned(String),
    /// Accept whatever is presented and remember its id.
    ///
    /// Only legal during pairing, where there is nothing to compare against
    /// yet. The PIN proof is what decides whether the key that turned up is
    /// the right one; see [`crate::pairing`].
    FirstContact,
}

/// A verifier that authenticates by public key rather than by authority.
#[derive(Debug)]
pub struct PinVerifier {
    trust: Trust,
    /// The id actually presented, recorded for the pairing flow to read back.
    seen: Mutex<Option<String>>,
}

impl PinVerifier {
    pub fn new(trust: Trust) -> Self {
        Self {
            trust,
            seen: Mutex::new(None),
        }
    }

    /// The host id of the certificate the server presented, once a handshake
    /// has completed.
    pub fn seen_host_id(&self) -> Option<String> {
        self.seen.lock().ok().and_then(|g| g.clone())
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let presented = host_id_from_cert(end_entity).map_err(|e| {
            rustls::Error::General(format!("could not read the server's public key: {e}"))
        })?;

        if let Trust::Pinned(expected) = &self.trust {
            // Length is fixed and both sides are hex, so a plain comparison
            // leaks nothing an attacker does not already have: the host id is
            // public by design.
            if &presented != expected {
                return Err(rustls::Error::General(format!(
                    "this is not the host you paired with \
                     (expected {}…, got {}…)",
                    &expected[..8.min(expected.len())],
                    &presented[..8.min(presented.len())],
                )));
            }
        }

        if let Ok(mut slot) = self.seen.lock() {
            *slot = Some(presented);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &verification_algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &verification_algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        verification_algorithms().supported_schemes()
    }
}

fn verification_algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
    rustls::crypto::ring::default_provider().signature_verification_algorithms
}

/// Builds a connector, and hands back the verifier so the caller can read the
/// host id that was presented.
pub fn client_config(trust: Trust) -> (TlsConnector, Arc<PinVerifier>) {
    init_crypto();
    let verifier = Arc::new(PinVerifier::new(trust));
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    (TlsConnector::from(Arc::new(config)), verifier)
}

/// The name sent in SNI.
///
/// Constant on purpose. The server ignores it and the client does not verify
/// it, so sending the IP address would only mean the handshake looked different
/// depending on how the host was reached.
pub fn sni_name() -> ServerName<'static> {
    ServerName::try_from("basalt").expect("a constant, valid DNS name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_the_provider_twice_is_harmless() {
        init_crypto();
        init_crypto();
    }

    #[test]
    fn a_server_config_builds_from_a_generated_identity() {
        let id = HostIdentity::generate("laptop-b").unwrap();
        assert!(server_config(&id).is_ok());
    }

    #[test]
    fn first_contact_records_whatever_it_is_shown() {
        init_crypto();
        let id = HostIdentity::generate("laptop-b").unwrap();
        let verifier = PinVerifier::new(Trust::FirstContact);
        assert!(verifier.seen_host_id().is_none());

        verifier
            .verify_server_cert(
                &id.certificate(),
                &[],
                &sni_name(),
                &[],
                UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_700_000_000)),
            )
            .expect("first contact accepts any key");

        assert_eq!(
            verifier.seen_host_id().as_deref(),
            Some(id.host_id.as_str())
        );
    }

    #[test]
    fn a_pinned_verifier_accepts_the_key_it_pinned() {
        init_crypto();
        let id = HostIdentity::generate("laptop-b").unwrap();
        let verifier = PinVerifier::new(Trust::Pinned(id.host_id.clone()));
        assert!(
            verifier
                .verify_server_cert(
                    &id.certificate(),
                    &[],
                    &sni_name(),
                    &[],
                    UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_700_000_000)),
                )
                .is_ok()
        );
    }

    #[test]
    fn a_pinned_verifier_refuses_a_different_key() {
        init_crypto();
        let real = HostIdentity::generate("laptop-b").unwrap();
        let impostor = HostIdentity::generate("laptop-b").unwrap();
        let verifier = PinVerifier::new(Trust::Pinned(real.host_id.clone()));

        let err = verifier
            .verify_server_cert(
                &impostor.certificate(),
                &[],
                &sni_name(),
                &[],
                UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_700_000_000)),
            )
            .expect_err("a different key must be refused");

        assert!(
            format!("{err}").contains("not the host you paired with"),
            "the message should say what went wrong, got: {err}"
        );
    }

    #[test]
    fn a_certificate_that_does_not_parse_is_refused() {
        init_crypto();
        let verifier = PinVerifier::new(Trust::FirstContact);
        assert!(
            verifier
                .verify_server_cert(
                    &CertificateDer::from(b"garbage".to_vec()),
                    &[],
                    &sni_name(),
                    &[],
                    UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_700_000_000)),
                )
                .is_err()
        );
    }

    #[test]
    fn the_verifier_advertises_some_signature_schemes() {
        init_crypto();
        let verifier = PinVerifier::new(Trust::FirstContact);
        assert!(!verifier.supported_verify_schemes().is_empty());
    }
}
