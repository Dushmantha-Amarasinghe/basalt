//! Basalt transport.
//!
//! Everything both ends need to talk to each other and nothing either end
//! needs on its own: the request/response envelope, TLS with a pinned public
//! key instead of a certificate authority, the PIN pairing exchange, and the
//! socket options Phase 0 measured.
//!
//! The deliberate omission is any notion of a filesystem or a user interface.
//! `basalt-host` and `basalt-client` each depend on this and not on each other,
//! which is what lets the integration tests run both in one process.

pub mod discovery;
pub mod framing;
pub mod identity;
pub mod pairing;
pub mod socket;
pub mod tls;

pub use basalt_proto::{ErrorCode, Op, WireError};
pub use discovery::{Beacon, Found};
pub use identity::HostIdentity;
pub use socket::DEFAULT_PORT;
pub use tls::Trust;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The peer answered, and said no.
    #[error("{0}")]
    Remote(basalt_proto::WireError),

    /// The peer said something that does not fit the protocol. Either a bug or
    /// something that is not a Basalt host.
    #[error("protocol: {0}")]
    Protocol(String),

    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),

    #[error("crypto: {0}")]
    Crypto(String),

    #[error(transparent)]
    Proto(#[from] basalt_proto::ProtoError),
}

impl NetError {
    /// The wire error code, when the failure came from the peer rather than
    /// from the link.
    ///
    /// The client uses this to tell apart the cases that need different
    /// handling: a rejected token means reconnect silently, a missing file
    /// means show it in place, and a dropped connection means retry.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            NetError::Remote(e) => Some(e.code),
            _ => None,
        }
    }

    /// Whether retrying on a fresh connection could plausibly work.
    ///
    /// A transport failure is worth retrying; a refusal from the far end is
    /// not, because the same request will be refused the same way.
    pub fn is_transient(&self) -> bool {
        matches!(self, NetError::Io(_) | NetError::Tls(_))
    }
}

pub type Result<T> = std::result::Result<T, NetError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_errors_carry_their_code_through() {
        let err = NetError::Remote(WireError::new(ErrorCode::NotFound, "gone"));
        assert_eq!(err.code(), Some(ErrorCode::NotFound));
        assert!(!err.is_transient());
    }

    #[test]
    fn a_dropped_connection_is_worth_retrying() {
        let err = NetError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert!(err.is_transient());
        assert_eq!(err.code(), None);
    }

    #[test]
    fn a_protocol_violation_is_not_worth_retrying() {
        assert!(!NetError::Protocol("nonsense".into()).is_transient());
    }
}
