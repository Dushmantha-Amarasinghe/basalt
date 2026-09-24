//! Basalt client.
//!
//! The half that runs on the machine doing the browsing: pairing, a pool of
//! authenticated connections, transfers with progress and resume, and a local
//! HTTP proxy so a media player can seek through a file on the host as if it
//! were local.
//!
//! Nothing here knows about a user interface. The Tauri shell is a thin layer
//! of commands over [`Basalt`], which is what lets the whole client be tested
//! against a real host in one process.

pub mod client;
pub mod players;
pub mod pool;
pub mod proxy;
pub mod rate;
pub mod session;
pub mod store;
pub mod ui;

pub use client::{Basalt, Progress, TransferKind};
pub use pool::Pool;
pub use session::{Session, SessionInfo};
pub use store::{ClientStore, KnownHost};
pub use ui::{DiscoveredHost, Status, TransferEvent, UiError};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Net(#[from] basalt_net::NetError),

    #[error(transparent)]
    Proto(#[from] basalt_proto::ProtoError),

    #[error("{0}")]
    Config(String),

    #[error("protocol: {0}")]
    Protocol(String),

    /// The key presented did not match the pinned one. Either the host was
    /// reset, or something is pretending to be it.
    #[error(
        "this is not the host this device paired with \
         (expected {expected}…, got {got}…). If you reset the host, pair again."
    )]
    WrongHost { expected: String, got: String },

    #[error("this host speaks protocol {theirs}, this app speaks {ours}. Update both ends.")]
    Incompatible { ours: u16, theirs: u16 },

    #[error("the host is not accepting new devices. Open pairing on the host and try again.")]
    PairingClosed,

    /// The host asked for a PIN and none was supplied. Not a failure — the
    /// interface asks for one and tries again.
    #[error("this host asks for a PIN. It is showing one now.")]
    PinRequired,

    #[error("{0}")]
    BadPin(String),

    #[error("not connected to a host")]
    NotConnected,

    #[error("no paired host was found on this network")]
    HostNotFound,
}

impl ClientError {
    /// The wire code, when the host was the one that said no.
    pub fn code(&self) -> Option<basalt_proto::ErrorCode> {
        match self {
            ClientError::Net(e) => e.code(),
            _ => None,
        }
    }

    /// A short machine-readable kind, for the UI to branch on.
    ///
    /// The interface needs to tell these apart without matching on prose:
    /// `offline` gets a reconnecting banner, `unpaired` sends the user back to
    /// pairing, and `notfound` is drawn in place.
    pub fn kind(&self) -> &'static str {
        use basalt_proto::ErrorCode as E;
        match self {
            ClientError::Io(_) => "offline",
            ClientError::NotConnected | ClientError::HostNotFound => "offline",
            ClientError::WrongHost { .. } => "wronghost",
            ClientError::Incompatible { .. } => "incompatible",
            ClientError::PairingClosed | ClientError::BadPin(_) => "pairing",
            ClientError::PinRequired => "pinrequired",
            ClientError::Net(e) => match e.code() {
                Some(E::NotFound) => "notfound",
                Some(E::Denied) => "denied",
                Some(E::Exists) => "exists",
                Some(E::NotEmpty) => "notempty",
                Some(E::Unauthenticated) => "unpaired",
                Some(E::PairingRefused) => "pairing",
                Some(E::Unavailable) => "unavailable",
                Some(_) => "error",
                None if e.is_transient() => "offline",
                None => "error",
            },
            _ => "error",
        }
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[cfg(test)]
mod tests {
    use super::*;
    use basalt_proto::{ErrorCode, WireError};

    fn remote(code: ErrorCode) -> ClientError {
        ClientError::Net(basalt_net::NetError::Remote(WireError::new(code, "x")))
    }

    #[test]
    fn remote_codes_survive_the_trip_up_from_the_wire() {
        assert_eq!(
            remote(ErrorCode::NotFound).code(),
            Some(ErrorCode::NotFound)
        );
        assert_eq!(ClientError::NotConnected.code(), None);
    }

    #[test]
    fn each_failure_gets_the_kind_the_interface_needs() {
        assert_eq!(remote(ErrorCode::NotFound).kind(), "notfound");
        assert_eq!(remote(ErrorCode::Denied).kind(), "denied");
        assert_eq!(remote(ErrorCode::Exists).kind(), "exists");
        assert_eq!(remote(ErrorCode::NotEmpty).kind(), "notempty");
        assert_eq!(remote(ErrorCode::Unauthenticated).kind(), "unpaired");
        assert_eq!(remote(ErrorCode::PairingRefused).kind(), "pairing");

        assert_eq!(
            ClientError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)).kind(),
            "offline"
        );
        assert_eq!(ClientError::NotConnected.kind(), "offline");
        assert_eq!(ClientError::BadPin("no".into()).kind(), "pairing");
        assert_eq!(
            ClientError::WrongHost {
                expected: "aa".into(),
                got: "bb".into()
            }
            .kind(),
            "wronghost"
        );
    }

    #[test]
    fn the_wrong_host_message_says_what_to_do_about_it() {
        let err = ClientError::WrongHost {
            expected: "aabbccdd".into(),
            got: "11223344".into(),
        };
        let text = format!("{err}");
        assert!(text.contains("aabbccdd"));
        assert!(text.contains("11223344"));
        assert!(text.contains("pair again"));
    }

    #[test]
    fn a_version_mismatch_names_both_versions() {
        let text = format!("{}", ClientError::Incompatible { ours: 1, theirs: 7 });
        assert!(text.contains('1') && text.contains('7'));
    }
}
