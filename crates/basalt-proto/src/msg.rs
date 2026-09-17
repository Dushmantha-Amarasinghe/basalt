//! Control-message bodies.
//!
//! Control messages are JSON. They are small, infrequent, and being able to
//! read one in a log is worth more than the bytes a binary encoding would save
//! — the things that have to be fast (file bodies, batch streams, upload
//! chunks) never travel as JSON.
//!
//! Byte strings are carried as lowercase hex rather than base64, because they
//! turn up in logs and in the pairing UI and hex is unambiguous to read aloud.

use serde::{Deserialize, Serialize};

use crate::hex;
use crate::{ProtoError, Result};

/// Bumped on any breaking change to these bodies or to the op table.
pub const PROTOCOL_VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloRequest {
    pub protocol: u16,
    /// Human name for the connecting machine, shown on the host's device list.
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloResponse {
    pub protocol: u16,
    /// What the user called this vault.
    pub vault: String,
    /// Hex SPKI SHA-256 of the host's certificate — its permanent identity.
    pub host_id: String,
    /// Whether the host is currently accepting new pairings.
    pub pairing_open: bool,
    pub host_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairBeginRequest {
    /// 32 random bytes, hex.
    pub client_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairBeginResponse {
    /// 32 random bytes, hex.
    pub server_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairFinishRequest {
    /// `HMAC-SHA256(pin, spki_hash ‖ client_nonce ‖ server_nonce)`, hex.
    pub proof: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairFinishResponse {
    /// 32 random bytes, hex. The client stores this and presents it forever
    /// after.
    pub token: String,
    pub vault: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub vault: String,
    pub device_name: String,
    /// Whether this device may write. Read-only devices are a host-side
    /// setting; the client uses this to grey out the actions rather than
    /// letting them fail at the end of a long upload.
    pub writable: bool,
}

// ---------------------------------------------------------------------------
// Browsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    /// Zero for directories: computing a directory's recursive size means
    /// walking it, which is not something a listing can afford.
    pub size: u64,
    /// Unix seconds. Negative for files dated before 1970, which do exist.
    pub mtime: i64,
    /// Windows' read-only attribute. Surfaced so the client can explain why a
    /// delete will fail before attempting it.
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    /// Relative to the vault root. Empty means the root itself.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResponse {
    pub entry: DirEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceResponse {
    pub free: u64,
    pub total: u64,
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    pub path: String,
    pub offset: u64,
    /// Bytes wanted. The host may return fewer at the end of the file; it
    /// never returns more.
    pub length: u64,
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteBeginRequest {
    pub path: String,
    pub size: u64,
    /// Refuse rather than replace when the destination exists.
    #[serde(default)]
    pub overwrite: bool,
    /// Resume an upload from a previous session. The host answers with how
    /// many bytes it already holds.
    #[serde(default)]
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteBeginResponse {
    pub upload: String,
    /// Where to start sending. Non-zero when resuming.
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteCommitRequest {
    pub upload: String,
    /// BLAKE3 of the whole file, hex. The host recomputes it over what it
    /// actually received and refuses the commit on a mismatch, so a silently
    /// corrupted upload cannot replace a good file.
    pub blake3: String,
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteAbortRequest {
    pub upload: String,
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MkdirRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveRequest {
    pub path: String,
    /// Required for a non-empty directory, so a mis-click cannot erase a tree.
    #[serde(default)]
    pub recursive: bool,
}

// ---------------------------------------------------------------------------
// Upload chunks — binary, not JSON
// ---------------------------------------------------------------------------

/// Bytes of upload identifier at the head of a [`crate::ops::Op::WriteChunk`]
/// payload.
pub const UPLOAD_ID_BYTES: usize = 16;

/// Fixed-size prefix on a chunk: the upload id followed by the offset.
pub const CHUNK_HEADER_BYTES: usize = UPLOAD_ID_BYTES + 8;

/// Builds a `WriteChunk` payload.
///
/// This one message is binary rather than JSON because it is the only control
/// op that carries bulk data. Hex-encoding a 4 MiB chunk into a JSON string
/// would inflate it by a third and cost a copy in each direction, on the
/// machine measured at 260 MB/s of zstd — throwing that away on encoding would
/// be indefensible.
pub fn encode_chunk(upload: &[u8; UPLOAD_ID_BYTES], offset: u64, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CHUNK_HEADER_BYTES + data.len());
    out.extend_from_slice(upload);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Splits a `WriteChunk` payload back into its parts.
pub fn decode_chunk(payload: &[u8]) -> Result<([u8; UPLOAD_ID_BYTES], u64, &[u8])> {
    if payload.len() < CHUNK_HEADER_BYTES {
        return Err(ProtoError::Malformed(format!(
            "chunk payload is {} bytes, needs at least {CHUNK_HEADER_BYTES}",
            payload.len()
        )));
    }
    let mut upload = [0u8; UPLOAD_ID_BYTES];
    upload.copy_from_slice(&payload[..UPLOAD_ID_BYTES]);
    let offset = u64::from_le_bytes(
        payload[UPLOAD_ID_BYTES..CHUNK_HEADER_BYTES]
            .try_into()
            .expect("slice is 8 bytes"),
    );
    Ok((upload, offset, &payload[CHUNK_HEADER_BYTES..]))
}

/// Parses a hex upload id back into bytes.
pub fn parse_upload_id(s: &str) -> Result<[u8; UPLOAD_ID_BYTES]> {
    let bytes = hex::decode(s)?;
    if bytes.len() != UPLOAD_ID_BYTES {
        return Err(ProtoError::Malformed(format!(
            "upload id is {} bytes, expected {UPLOAD_ID_BYTES}",
            bytes.len()
        )));
    }
    let mut out = [0u8; UPLOAD_ID_BYTES];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_round_trip() {
        let id = [7u8; UPLOAD_ID_BYTES];
        let data = vec![9u8; 1000];
        let encoded = encode_chunk(&id, 4096, &data);
        let (back_id, offset, back_data) = decode_chunk(&encoded).unwrap();
        assert_eq!(back_id, id);
        assert_eq!(offset, 4096);
        assert_eq!(back_data, &data[..]);
    }

    #[test]
    fn an_empty_chunk_is_legal() {
        // The last chunk of a file whose size is an exact multiple of the chunk
        // size carries no data, and must not be treated as malformed.
        let encoded = encode_chunk(&[1u8; UPLOAD_ID_BYTES], 0, &[]);
        let (_, offset, data) = decode_chunk(&encoded).unwrap();
        assert_eq!(offset, 0);
        assert!(data.is_empty());
    }

    #[test]
    fn a_truncated_chunk_is_rejected_rather_than_panicking() {
        for len in 0..CHUNK_HEADER_BYTES {
            assert!(
                decode_chunk(&vec![0u8; len]).is_err(),
                "{len}-byte payload must be rejected"
            );
        }
    }

    #[test]
    fn upload_ids_parse_from_hex_and_reject_the_wrong_length() {
        let id = [0xABu8; UPLOAD_ID_BYTES];
        assert_eq!(parse_upload_id(&hex::encode(&id)).unwrap(), id);
        assert!(parse_upload_id("abcd").is_err());
        assert!(parse_upload_id("").is_err());
        assert!(parse_upload_id("zz").is_err());
    }

    #[test]
    fn a_listing_round_trips_through_json() {
        let response = ListResponse {
            entries: vec![
                DirEntry {
                    name: "Films".into(),
                    kind: EntryKind::Dir,
                    size: 0,
                    mtime: 1_700_000_000,
                    readonly: false,
                },
                DirEntry {
                    name: "notes.txt".into(),
                    kind: EntryKind::File,
                    size: 1024,
                    mtime: -86_400,
                    readonly: true,
                },
            ],
        };
        let json = serde_json::to_string(&response).unwrap();
        let back: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entries, response.entries);
    }

    #[test]
    fn optional_write_fields_default_when_an_older_client_omits_them() {
        let req: WriteBeginRequest = serde_json::from_str(r#"{"path":"a.bin","size":10}"#).unwrap();
        assert!(!req.overwrite);
        assert_eq!(req.resume, None);
    }

    #[test]
    fn a_listing_entry_survives_a_missing_readonly_flag() {
        let e: DirEntry =
            serde_json::from_str(r#"{"name":"a","kind":"file","size":1,"mtime":0}"#).unwrap();
        assert!(!e.readonly);
    }
}
