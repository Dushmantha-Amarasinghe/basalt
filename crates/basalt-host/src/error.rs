//! Host-side errors, and how they become wire errors.
//!
//! The mapping is kept in one place because the client behaves differently for
//! each code — a missing file is drawn in place, a denied path is reported as a
//! bug, a rejected token triggers a silent reconnect — and a mis-classified
//! error means the wrong behaviour rather than a wrong message.

use basalt_proto::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("{0} was not found")]
    NotFound(String),

    #[error("{0}")]
    Denied(String),

    #[error("{0} already exists")]
    Exists(String),

    #[error("{0} is not empty")]
    NotEmpty(String),

    #[error("{0}")]
    BadRequest(String),

    #[error("not authenticated")]
    Unauthenticated,

    #[error("{0}")]
    PairingRefused(String),

    /// The chosen drive is not connected.
    #[error("{0}")]
    Unavailable(String),

    /// A profile sign-in that has ended.
    #[error("signed out of this profile")]
    SignedOut,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Proto(#[from] basalt_proto::ProtoError),

    #[error(transparent)]
    Net(#[from] basalt_net::NetError),
}

impl HostError {
    pub fn code(&self) -> ErrorCode {
        match self {
            HostError::NotFound(_) => ErrorCode::NotFound,
            HostError::Denied(_) => ErrorCode::Denied,
            HostError::Exists(_) => ErrorCode::Exists,
            HostError::NotEmpty(_) => ErrorCode::NotEmpty,
            HostError::BadRequest(_) => ErrorCode::BadRequest,
            HostError::Unauthenticated => ErrorCode::Unauthenticated,
            HostError::PairingRefused(_) => ErrorCode::PairingRefused,
            // A path that escapes the vault arrives here as a protocol error
            // from the sanitiser, and "denied" is the honest description: the
            // request was well formed and refused on purpose.
            HostError::Proto(basalt_proto::ProtoError::UnsafePath(_)) => ErrorCode::Denied,
            HostError::Proto(_) | HostError::Net(_) => ErrorCode::BadRequest,
            HostError::Unavailable(_) => ErrorCode::Unavailable,
            HostError::SignedOut => ErrorCode::SignedOut,
            HostError::Io(_) => ErrorCode::Io,
        }
    }
}

/// Turns a filesystem error into the closest host error, keeping the path.
///
/// Worth doing rather than letting everything fall through as `Io`: "not
/// found" and "access denied" are the two the user sees most and the two they
/// can act on.
pub fn from_io(path: &str, e: std::io::Error) -> HostError {
    match e.kind() {
        std::io::ErrorKind::NotFound => HostError::NotFound(path.to_string()),
        std::io::ErrorKind::PermissionDenied => {
            HostError::Denied(format!("{path}: permission denied"))
        }
        std::io::ErrorKind::AlreadyExists => HostError::Exists(path.to_string()),
        _ => HostError::Io(e),
    }
}

pub type Result<T> = std::result::Result<T, HostError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_maps_to_its_own_code() {
        assert_eq!(HostError::NotFound("a".into()).code(), ErrorCode::NotFound);
        assert_eq!(HostError::Denied("a".into()).code(), ErrorCode::Denied);
        assert_eq!(HostError::Exists("a".into()).code(), ErrorCode::Exists);
        assert_eq!(HostError::NotEmpty("a".into()).code(), ErrorCode::NotEmpty);
        assert_eq!(
            HostError::Unauthenticated.code(),
            ErrorCode::Unauthenticated
        );
        assert_eq!(
            HostError::BadRequest("a".into()).code(),
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn a_missing_drive_has_its_own_code_and_says_so_plainly() {
        let err = HostError::Unavailable("Media is not connected to the host right now".into());
        assert_eq!(err.code(), ErrorCode::Unavailable);
        assert_eq!(
            err.to_string(),
            "Media is not connected to the host right now"
        );
    }

    #[test]
    fn an_escaping_path_is_reported_as_denied_not_as_a_bad_request() {
        let err = HostError::Proto(basalt_proto::ProtoError::UnsafePath("../etc".into()));
        assert_eq!(err.code(), ErrorCode::Denied);
    }

    #[test]
    fn filesystem_errors_keep_their_meaning_and_their_path() {
        let err = from_io(
            "photos/a.jpg",
            std::io::Error::from(std::io::ErrorKind::NotFound),
        );
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(format!("{err}").contains("photos/a.jpg"));

        let err = from_io(
            "locked.bin",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        assert_eq!(err.code(), ErrorCode::Denied);
        assert!(format!("{err}").contains("locked.bin"));
    }

    #[test]
    fn an_unrecognised_filesystem_error_stays_generic() {
        let err = from_io("x", std::io::Error::from(std::io::ErrorKind::WouldBlock));
        assert_eq!(err.code(), ErrorCode::Io);
    }
}
