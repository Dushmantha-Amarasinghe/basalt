//! Operation codes and the request/response envelope.
//!
//! The envelope is the one Phase 0 measured, unchanged:
//!
//! ```text
//! request   [op u8][len u32 LE][payload len bytes]
//! response  [status u8][len u64 LE][payload len bytes]
//! ```
//!
//! Requests carry a 32-bit length because they are manifests and paths; a
//! response carries 64 because it may be a film. The split is deliberate — it
//! means a corrupt request length cannot ask for a terabyte of allocation.
//!
//! The framing itself lives in `basalt-net`, which has tokio. This module is
//! kept synchronous and dependency-light so the protocol vocabulary can be
//! tested without a runtime.

use crate::{ProtoError, Result};

/// Largest control payload accepted in one request.
///
/// Manifests for a large directory are the biggest thing that travels this
/// path. 64 MiB is far more than any real manifest and still small enough that
/// a hostile length field cannot exhaust memory.
pub const MAX_REQUEST_BYTES: u32 = 64 * 1024 * 1024;

/// Largest single ranged read a client may ask for (16 MiB).
///
/// Bigger reads do not go faster — the link runs at ~22.7 MB/s and 4 MiB is
/// already several times the bandwidth-delay product — but they do let one
/// request pin a large buffer on a host with 5.9 GB of RAM.
pub const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Op {
    /// Empty both ways. Keeps a pooled connection warm and measures RTT.
    Ping = 1,
    /// Version and capability exchange. Legal before authenticating.
    Hello = 2,
    /// Start pairing: exchange nonces.
    PairBegin = 3,
    /// Finish pairing: present the PIN proof, receive a device token.
    PairFinish = 4,
    /// Present a device token on an already-paired connection.
    Auth = 5,
    /// List one directory.
    List = 6,
    /// Metadata for one path.
    Stat = 7,
    /// Ranged read. The response payload is raw bytes, which is what makes
    /// seeking in a video work.
    Read = 8,
    /// Many small files as a single batch stream.
    ReadBatch = 9,
    /// Open an upload, or resume one that was interrupted.
    WriteBegin = 10,
    /// Append bytes to an open upload.
    WriteChunk = 11,
    /// Close an upload and verify its hash.
    WriteCommit = 12,
    /// Abandon an open upload and delete its partial file.
    WriteAbort = 13,
    Mkdir = 14,
    Rename = 15,
    Remove = 16,
    /// Free and total bytes on the vault's volume.
    Space = 17,
    /// Duplicate a file or folder **on the host**.
    ///
    /// Pasting could have been done by downloading and uploading again, which
    /// would cost two trips across a 22.7 MB/s link for a file that never
    /// needed to leave the drive. Copying host-side turns a two-minute
    /// operation on a 2 GB film into a disk-speed one.
    Copy = 18,
    /// Subscribe to changes on the drive. **Streams.**
    ///
    /// The one operation that does not answer once and stop. The host keeps the
    /// connection and writes a fresh OK response for every change it sees,
    /// until the client hangs up. That needs no new framing — a response is
    /// already self-delimiting — and it keeps the request/response shape of
    /// every other op intact rather than inventing multiplexing for one case.
    Watch = 19,
    /// The media index: films and series recognised on the drive.
    Library = 20,
    /// Poster or backdrop bytes for one library item.
    LibraryArt = 21,
    /// Where each file has been watched to. Reads, and optionally writes first.
    ///
    /// Kept on the host rather than per device on purpose: a resume point that
    /// does not follow you from the laptop to the living room is a bookmark,
    /// not a Continue watching.
    Progress = 22,
    /// This device is leaving: the host forgets it.
    ///
    /// Sent when somebody chooses "Forget this vault". It used to be forgotten
    /// on the device alone, and the host kept a record that would never connect
    /// again — one more stale row in its device list every time a device
    /// re-paired.
    Unpair = 23,
    /// Every video, music file and photo on the drive, and the most recently
    /// changed files — what the Videos, Music, Photos and Recent sections
    /// show. Found by the host's own walk of the whole drive, rather than by
    /// each device listing a couple of folders levels deep for itself.
    Collections = 24,
    /// A small preview image of a video or a photo, made by the host.
    Thumbnail = 25,
    /// The household's profiles, by name. Never their PINs.
    Profiles = 26,
    /// Makes a profile, and signs this device in to it.
    ProfileCreate = 27,
    /// Signs this device in to a profile, with its PIN.
    ProfileSignIn = 28,
    /// Which profile this connection acts for, or none: the device itself.
    ///
    /// Per connection, because a device holds several open at once and each
    /// has to say who it is working for.
    ProfileUse = 29,
    /// Ends a profile sign-in, on this device.
    ProfileSignOut = 30,
    /// The starred files of the profile this connection acts for.
    Stars = 31,
}

impl Op {
    /// The highest opcode this build knows about.
    ///
    /// Adding a variant means bumping this, and the tests below fail loudly if
    /// it is forgotten — `from_u8(LAST + 1)` would start succeeding, which is
    /// exactly the signal that the table and the enum have drifted apart.
    pub const LAST: u8 = Op::Stars as u8;

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Op::Ping,
            2 => Op::Hello,
            3 => Op::PairBegin,
            4 => Op::PairFinish,
            5 => Op::Auth,
            6 => Op::List,
            7 => Op::Stat,
            8 => Op::Read,
            9 => Op::ReadBatch,
            10 => Op::WriteBegin,
            11 => Op::WriteChunk,
            12 => Op::WriteCommit,
            13 => Op::WriteAbort,
            14 => Op::Mkdir,
            15 => Op::Rename,
            16 => Op::Remove,
            17 => Op::Space,
            18 => Op::Copy,
            19 => Op::Watch,
            20 => Op::Library,
            21 => Op::LibraryArt,
            22 => Op::Progress,
            23 => Op::Unpair,
            24 => Op::Collections,
            25 => Op::Thumbnail,
            26 => Op::Profiles,
            27 => Op::ProfileCreate,
            28 => Op::ProfileSignIn,
            29 => Op::ProfileUse,
            30 => Op::ProfileSignOut,
            31 => Op::Stars,
            other => return Err(ProtoError::UnknownOp(other)),
        })
    }

    /// Whether this operation may be used before the connection has presented
    /// a valid device token.
    ///
    /// Everything not on this list touches the drive and must be refused until
    /// the peer has proved who it is. Expressed as a match rather than a flag
    /// on each variant so that adding an op without thinking about it fails
    /// closed: a new variant is authenticated unless it is named here.
    pub fn allowed_unauthenticated(self) -> bool {
        matches!(
            self,
            Op::Ping | Op::Hello | Op::PairBegin | Op::PairFinish | Op::Auth
        )
    }
}

pub const STATUS_OK: u8 = 0;
pub const STATUS_ERR: u8 = 1;

/// Why a request failed.
///
/// A code rather than only a message, because the client reacts differently to
/// each: a missing file is shown in place, an expired token triggers a silent
/// reconnect, and a denied path is a bug worth reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The path does not exist.
    NotFound,
    /// The path exists but the host refused it — outside the vault, or locked
    /// by another process.
    Denied,
    /// The target already exists and the caller did not ask to overwrite.
    Exists,
    /// A directory that must be empty is not.
    NotEmpty,
    /// No valid device token on this connection.
    Unauthenticated,
    /// The request did not parse, or its fields were out of range.
    BadRequest,
    /// The host is not in pairing mode, the PIN was wrong, or it has locked out.
    PairingRefused,
    /// Something the filesystem reported that does not map to anything above.
    Io,
    /// A newer client asked for something this host does not implement.
    Unsupported,
    /// The host is running, but the drive it shares is not connected.
    ///
    /// Its own code so a client can say so and keep checking, rather than
    /// showing an empty folder — which is what a generic failure looked like.
    Unavailable,
    /// The profile sign-in this connection presented has ended: signed out on
    /// another device, the profile removed, or its PIN reset on the host.
    SignedOut,
    /// A code from a newer host than this build knows, read as a plain
    /// failure rather than as a reply that will not parse at all.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
}

impl WireError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for WireError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_opcode_round_trips() {
        // Walking the numeric range rather than a hand-written list, so a new
        // variant that someone forgets to add to `from_u8` is caught here.
        for raw in 1..=Op::LAST {
            let op = Op::from_u8(raw).unwrap_or_else(|_| panic!("opcode {raw} is unmapped"));
            assert_eq!(op as u8, raw);
        }
    }

    #[test]
    fn unknown_opcodes_are_rejected() {
        assert!(Op::from_u8(0).is_err());
        assert!(Op::from_u8(255).is_err());
        // The guard against adding a variant and forgetting `LAST`: if this
        // starts parsing, the enum has grown past what the tests cover.
        assert!(Op::from_u8(Op::LAST + 1).is_err());
    }

    #[test]
    fn only_the_handshake_is_allowed_before_authenticating() {
        for op in [Op::Ping, Op::Hello, Op::PairBegin, Op::PairFinish, Op::Auth] {
            assert!(op.allowed_unauthenticated(), "{op:?} is part of connecting");
        }
        for op in [
            Op::List,
            Op::Stat,
            Op::Read,
            Op::ReadBatch,
            Op::WriteBegin,
            Op::WriteChunk,
            Op::WriteCommit,
            Op::WriteAbort,
            Op::Mkdir,
            Op::Rename,
            Op::Remove,
            Op::Space,
            Op::Copy,
            Op::Watch,
            Op::Library,
            Op::LibraryArt,
            Op::Progress,
            Op::Unpair,
            Op::Collections,
            Op::Thumbnail,
            Op::Profiles,
            Op::ProfileCreate,
            Op::ProfileSignIn,
            Op::ProfileUse,
            Op::ProfileSignOut,
            Op::Stars,
        ] {
            assert!(
                !op.allowed_unauthenticated(),
                "{op:?} touches the drive and must require a token"
            );
        }
    }

    #[test]
    fn a_code_this_build_does_not_know_still_reads() {
        let err: WireError =
            serde_json::from_str(r#"{"code":"something_newer","message":"hm"}"#).unwrap();
        assert_eq!(err.code, ErrorCode::Unknown);
        assert_eq!(err.message, "hm");
    }

    #[test]
    fn wire_errors_round_trip_through_json() {
        let err = WireError::new(ErrorCode::NotFound, "photos/missing.jpg");
        let json = serde_json::to_string(&err).unwrap();
        let back: WireError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, ErrorCode::NotFound);
        assert_eq!(back.message, "photos/missing.jpg");
    }
}
