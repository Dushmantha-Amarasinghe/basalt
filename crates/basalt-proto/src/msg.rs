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

/// Defaulting an absent boolean to true, for fields where the safe reading
/// of silence is "yes".
fn default_true() -> bool {
    true
}

/// Bumped on any breaking change to these bodies or to the op table.
///
/// 2 — pairing became a *request* the host displays, rather than a window the
/// host opens in advance, and the PIN became optional.
pub const PROTOCOL_VERSION: u16 = 3;

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
    /// Shown on the host beside the PIN, so the person reading it can see
    /// which machine is asking.
    #[serde(default)]
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairBeginResponse {
    /// 32 random bytes, hex.
    pub server_nonce: String,
    /// Identifies this attempt. The host has generated a PIN against it and is
    /// displaying both.
    pub request: String,
    /// Whether the host will check a PIN. When false the client may finish
    /// without one — the host has been set to let anyone on the network in.
    #[serde(default = "default_true")]
    pub requires_pin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairFinishRequest {
    /// The id from [`PairBeginResponse`].
    pub request: String,
    /// `HMAC-SHA256(pin, spki_hash ‖ client_nonce ‖ server_nonce)`, hex.
    /// Absent when the host said no PIN was required.
    #[serde(default)]
    pub proof: Option<String>,
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

// ---------------------------------------------------------------------------
// Watching
// ---------------------------------------------------------------------------

/// What happened to something on the drive.
///
/// Reported by watching the filesystem rather than by the host announcing its
/// own operations, and that is the important part: the drive is the truth. A
/// file deleted in Explorer, by another program, or by a second client all
/// arrive here identically, so no client can be looking at a listing the disk
/// has stopped agreeing with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// Something appeared. `path` is vault-relative.
    Created { path: String },
    /// Something is gone.
    Removed { path: String },
    /// Contents changed; the entry is still there.
    Modified { path: String },
    /// Moved or renamed within the vault.
    Renamed { from: String, to: String },
    /// Too many changes at once to report individually — reload everything.
    ///
    /// The watcher's buffer can overflow when something unpacks ten thousand
    /// files. Saying so plainly is far better than silently dropping events and
    /// leaving every client subtly wrong.
    Resynchronise,
    /// The media index finished changing.
    LibraryChanged,
}

impl Change {
    /// The directory a client would need to refresh, if any.
    ///
    /// A client only cares about a change if it is looking at the folder the
    /// change happened in — this is what lets it ignore the rest cheaply.
    pub fn parent_dirs(&self) -> Vec<String> {
        fn parent(path: &str) -> String {
            match path.rfind('/') {
                Some(cut) => path[..cut].to_string(),
                None => String::new(),
            }
        }
        match self {
            Change::Created { path } | Change::Removed { path } | Change::Modified { path } => {
                vec![parent(path)]
            }
            Change::Renamed { from, to } => {
                let (a, b) = (parent(from), parent(to));
                if a == b { vec![a] } else { vec![a, b] }
            }
            Change::Resynchronise | Change::LibraryChanged => Vec::new(),
        }
    }
}

/// Sent once to open a watch. Empty today; here so the shape can grow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WatchRequest {}

/// One streamed frame on a watch connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchEvent {
    pub change: Change,
}

// ---------------------------------------------------------------------------
// The media library
// ---------------------------------------------------------------------------

/// Asked for the whole index at once.
///
/// One request rather than paged: a personal library is thousands of items, not
/// millions, and the whole thing is a few hundred kilobytes of JSON. Paging
/// would cost a round trip per screen on a link where a round trip is 2.3 ms
/// and buy nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryRequest {
    /// Return nothing when the host's copy still matches this. Cheap polling.
    #[serde(default)]
    pub known_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryResponse {
    /// Bumped on every reindex, so a client can tell nothing changed.
    pub revision: u64,
    /// False when the library is switched off on the host.
    pub enabled: bool,
    /// True while a scan is running.
    pub scanning: bool,
    /// Absent when `known_revision` already matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<LibraryItem>>,
}

/// A film or a series. Episodes hang off the series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    /// Stable across rescans: derived from the title and year, not the path, so
    /// moving a file does not orphan its artwork or its resume point.
    pub id: String,
    pub kind: LibraryKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// Vault-relative path of the file, for a film.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub size: u64,
    /// Unix seconds of the newest file in this item, for "recently added".
    #[serde(default)]
    pub added: i64,
    /// Seasons, for a series. Empty for a film.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<Season>,
    /// How sure the parser is, 0–100. Below `CONFIDENT` the interface should
    /// offer the user a chance to correct it rather than assert it.
    #[serde(default)]
    pub confidence: u8,
    /// Whether the host has a poster cached for this item.
    ///
    /// Here so a client knows whether to ask at all: without it, a library of
    /// five hundred with no artwork would be five hundred requests that all
    /// come back empty.
    #[serde(default)]
    pub has_art: bool,
}

/// Below this, a match is a guess worth showing the user.
pub const CONFIDENT: u8 = 70;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    Film,
    Series,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Season {
    pub number: u16,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub number: u16,
    /// Vault-relative path.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub added: i64,
}

/// Artwork for one item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtRequest {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Where things have been watched to
// ---------------------------------------------------------------------------

/// How far through a file somebody got.
///
/// **A fraction, not a timestamp.** The in-app player knows its position in
/// seconds; an external player does not report anything at all, and the only
/// signal available there is how far through the file it has read. Storing a
/// fraction makes both sources the same kind of answer, and seconds — when
/// they are known — come along beside it for the interface to display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watched {
    /// Vault-relative path of the file itself, not the library item: an
    /// episode is watched, a series is not.
    pub path: String,
    /// 0.0–1.0 through the file. Always present.
    pub fraction: f64,
    /// Seconds in. Zero when an external player was used and only the byte
    /// offset was observable.
    #[serde(default)]
    pub position: f64,
    /// Total seconds. Zero when unknown.
    #[serde(default)]
    pub duration: f64,
    /// Unix seconds of the last update, for ordering Continue watching.
    pub updated_at: i64,
}

/// Past this, the file counts as watched rather than in progress.
///
/// Credits run long. Stopping at 94% is finishing something, and offering to
/// resume it two minutes from the end is worse than offering nothing.
pub const FINISHED_AT: f64 = 0.94;

/// Before this, there is nothing worth resuming.
///
/// Opening something, watching the first minute of a two-hour film and
/// stopping is not a thing to be reminded of later.
pub const STARTED_AFTER: f64 = 0.01;

impl Watched {
    pub fn finished(&self) -> bool {
        self.fraction >= FINISHED_AT
    }

    /// Whether this belongs in Continue watching.
    pub fn in_progress(&self) -> bool {
        self.fraction > STARTED_AFTER && !self.finished()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressRequest {
    /// Recorded before the answer is built, so a client that reports and reads
    /// in one call sees its own update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Watched>,
    /// Forget this path entirely — "watched" or "start again".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forget: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressResponse {
    pub entries: Vec<Watched>,
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
    fn a_change_names_the_folder_that_has_to_be_refreshed() {
        assert_eq!(
            Change::Created {
                path: "films/a.mkv".into()
            }
            .parent_dirs(),
            ["films"]
        );
        assert_eq!(
            Change::Removed {
                path: "a.mkv".into()
            }
            .parent_dirs(),
            [""],
            "something in the root belongs to the root"
        );
    }

    // A move between folders changes two listings, and a client looking at
    // either one is now wrong.
    #[test]
    fn a_move_between_folders_touches_both_of_them() {
        let change = Change::Renamed {
            from: "films/a.mkv".into(),
            to: "archive/a.mkv".into(),
        };
        assert_eq!(change.parent_dirs(), ["films", "archive"]);
    }

    #[test]
    fn a_rename_in_place_touches_one_folder_once() {
        let change = Change::Renamed {
            from: "films/a.mkv".into(),
            to: "films/b.mkv".into(),
        };
        assert_eq!(change.parent_dirs(), ["films"]);
    }

    #[test]
    fn a_resynchronise_belongs_to_no_particular_folder() {
        assert!(Change::Resynchronise.parent_dirs().is_empty());
        assert!(Change::LibraryChanged.parent_dirs().is_empty());
    }

    #[test]
    fn changes_round_trip_with_their_kind_tagged() {
        let change = Change::Renamed {
            from: "a".into(),
            to: "b".into(),
        };
        let json = serde_json::to_string(&change).unwrap();
        assert!(json.contains(r#""kind":"renamed""#), "{json}");
        assert_eq!(serde_json::from_str::<Change>(&json).unwrap(), change);
    }

    #[test]
    fn a_library_response_can_say_nothing_changed_without_sending_the_index() {
        let response = LibraryResponse {
            revision: 7,
            enabled: true,
            scanning: false,
            items: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("items"), "an unchanged index sends no items");
    }

    #[test]
    fn a_film_and_a_series_round_trip() {
        let items = vec![
            LibraryItem {
                id: "f1".into(),
                kind: LibraryKind::Film,
                title: "Arrival".into(),
                year: Some(2016),
                path: Some("films/Arrival.mkv".into()),
                size: 10,
                added: 1,
                seasons: Vec::new(),
                confidence: 95,
                has_art: false,
            },
            LibraryItem {
                id: "s1".into(),
                kind: LibraryKind::Series,
                title: "Breaking Bad".into(),
                year: Some(2008),
                path: None,
                size: 20,
                added: 2,
                seasons: vec![Season {
                    number: 1,
                    episodes: vec![Episode {
                        number: 1,
                        path: "shows/BB/S01/e1.mkv".into(),
                        title: None,
                        size: 20,
                        added: 2,
                    }],
                }],
                confidence: 88,
                has_art: false,
            },
        ];
        let json = serde_json::to_string(&items).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<LibraryItem>>(&json).unwrap(),
            items
        );
    }

    /// These cross into JavaScript, where a snake_case key silently reads as
    /// `undefined`. They happen to be single words today; this makes that a
    /// rule rather than an accident, because adding `episode_title` later
    /// would break the interface with no error anywhere.
    #[test]
    fn nothing_the_interface_reads_is_snake_case() {
        fn keys(value: serde_json::Value) -> Vec<String> {
            value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect()
        }

        let item = LibraryItem {
            id: "f1".into(),
            kind: LibraryKind::Film,
            title: "Arrival".into(),
            year: Some(2016),
            path: Some("a.mkv".into()),
            size: 1,
            added: 2,
            seasons: vec![Season {
                number: 1,
                episodes: vec![Episode {
                    number: 1,
                    path: "b.mkv".into(),
                    title: Some("Pilot".into()),
                    size: 3,
                    added: 4,
                }],
            }],
            confidence: 90,
            has_art: true,
        };
        let response = LibraryResponse {
            revision: 1,
            enabled: true,
            scanning: false,
            items: Some(vec![item.clone()]),
        };

        let mut all = keys(serde_json::to_value(&item).unwrap());
        all.extend(keys(serde_json::to_value(&response).unwrap()));
        all.extend(keys(
            serde_json::to_value(&item.seasons[0].episodes[0]).unwrap(),
        ));
        all.extend(keys(serde_json::to_value(&item.seasons[0]).unwrap()));
        all.extend(keys(
            serde_json::to_value(Change::Renamed {
                from: "a".into(),
                to: "b".into(),
            })
            .unwrap(),
        ));

        for key in all {
            assert!(
                !key.contains('_'),
                "{key} would arrive undefined in JavaScript"
            );
        }
    }

    fn watched(fraction: f64) -> Watched {
        Watched {
            path: "films/a.mkv".into(),
            fraction,
            position: 0.0,
            duration: 0.0,
            updated_at: 0,
        }
    }

    /// Credits run long, and offering to resume something two minutes from the
    /// end is worse than offering nothing at all.
    #[test]
    fn something_watched_to_the_credits_counts_as_finished() {
        assert!(watched(0.95).finished());
        assert!(watched(1.0).finished());
        assert!(!watched(0.5).finished());
        assert!(!watched(0.95).in_progress());
    }

    /// Opening a film, watching a minute and stopping is not something to be
    /// reminded about later.
    #[test]
    fn barely_started_is_not_in_progress() {
        assert!(!watched(0.0).in_progress());
        assert!(!watched(0.005).in_progress());
        assert!(watched(0.05).in_progress());
    }

    #[test]
    fn progress_is_camel_case_on_the_wire() {
        let json = serde_json::to_string(&watched(0.5)).unwrap();
        assert!(json.contains("updatedAt"), "{json}");
        assert!(!json.contains("updated_at"), "{json}");
    }

    #[test]
    fn a_progress_request_that_only_reads_sends_nothing_extra() {
        let json = serde_json::to_string(&ProgressRequest::default()).unwrap();
        assert_eq!(json, "{}", "reading is the common case and should be empty");
    }

    #[test]
    fn a_listing_entry_survives_a_missing_readonly_flag() {
        let e: DirEntry =
            serde_json::from_str(r#"{"name":"a","kind":"file","size":1,"mtime":0}"#).unwrap();
        assert!(!e.readonly);
    }
}
