//! Basalt wire protocol.
//!
//! Two pieces live here because both the host and the client need to agree on
//! them exactly, and because the Phase 0 benchmark harness measures the real
//! thing rather than a stand-in:
//!
//! - [`frame`] — the batch stream format. One request, one response, thousands
//!   of files. This is what removes the per-file round trip that makes naive
//!   file servers crawl on directories full of small files.
//! - [`codec`] — deciding whether a given payload is worth compressing. On a
//!   ~30 MB/s Wi-Fi link, zstd runs roughly 20x faster than we can transmit, so
//!   compressing compressible bytes is close to free. Compressing *already*
//!   compressed bytes is pure waste, so the policy has to tell them apart
//!   cheaply.

pub mod codec;
pub mod frame;
pub mod manifest;

pub use codec::{Codec, CompressionPolicy};
pub use frame::{Entry, EntryKind, StreamHeader};
pub use manifest::BatchRequest;

/// Wire format magic. Bumped only on breaking layout changes.
pub const MAGIC: [u8; 4] = *b"BSLT";

/// Wire format version.
pub const VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("bad magic: expected {expected:?}, got {got:?}")]
    BadMagic { expected: [u8; 4], got: [u8; 4] },

    #[error("unsupported protocol version {0} (this build speaks {VERSION})")]
    UnsupportedVersion(u16),

    #[error("unknown codec id {0}")]
    UnknownCodec(u8),

    #[error("unknown entry kind {0}")]
    UnknownEntryKind(u8),

    #[error("path is not valid utf-8")]
    InvalidPath,

    #[error("path escapes the share root: {0}")]
    UnsafePath(String),

    #[error("frame declares {declared} bytes, which exceeds the {limit} byte limit")]
    FrameTooLarge { declared: u64, limit: u64 },
}

pub type Result<T> = std::result::Result<T, ProtoError>;
