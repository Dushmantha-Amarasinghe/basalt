//! Batch stream framing.
//!
//! The problem this solves: fetching 10,000 small files one request at a time
//! costs 10,000 round trips and 10,000 disk seeks. On a Wi-Fi link with ~2 ms
//! RTT that is 20 seconds of pure latency before a single useful byte moves.
//!
//! The batch stream collapses that into one request and one response. The host
//! walks the requested paths in on-disk order and emits a single continuous
//! stream of length-prefixed entries.
//!
//! The important detail is that compression wraps the *entire body*, not each
//! file individually. A shared zstd window across thousands of similar files
//! (source trees, logs, documents) compresses dramatically better than the same
//! files compressed one at a time, because the dictionary carries over.
//!
//! ```text
//! ┌─────────────── header (12 bytes, always plaintext) ───────────────┐
//! │ magic "BSLT" │ version u16 │ codec u8 │ level i8 │ flags u16 │ rsv │
//! └───────────────────────────────────────────────────────────────────┘
//! ┌─────────── body (one zstd stream, or raw bytes) ──────────────────┐
//! │ entry │ entry │ entry │ … │ End                                   │
//! └───────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Large files do not travel this path — they use ranged single-file requests
//! so they can be split across parallel streams and resumed. [`MAX_ENTRY_BYTES`]
//! enforces that split.

use std::io::{BufReader, Read, Write};

use crate::codec::Codec;
use crate::{MAGIC, ProtoError, Result, VERSION};

/// Largest single entry accepted in a batch stream (64 MiB).
///
/// This is a framing guard, not a tuning knob: anything this large belongs on
/// the ranged single-file path where it can be parallelised and resumed. It
/// also bounds decoder allocation against a malicious or corrupt length field.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Longest path accepted on the wire.
pub const MAX_PATH_BYTES: usize = 4096;

const HEADER_BYTES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    End = 0,
    File = 1,
    Dir = 2,
    /// The host could not read this entry. Carried inline so one unreadable
    /// file does not abort a 10,000-file transfer.
    Error = 3,
}

impl EntryKind {
    fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(EntryKind::End),
            1 => Ok(EntryKind::File),
            2 => Ok(EntryKind::Dir),
            3 => Ok(EntryKind::Error),
            other => Err(ProtoError::UnknownEntryKind(other)),
        }
    }
}

/// One decoded entry from a batch stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    /// Relative, `/`-separated, already validated by [`sanitize_relative_path`].
    pub path: String,
    /// Unix seconds.
    pub mtime: i64,
    /// File contents for [`EntryKind::File`], empty otherwise.
    pub data: Vec<u8>,
    /// Message for [`EntryKind::Error`], `None` otherwise.
    pub error: Option<String>,
}

/// Parsed stream header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamHeader {
    pub version: u16,
    pub codec: Codec,
    pub flags: u16,
}

impl StreamHeader {
    pub fn new(codec: Codec) -> Self {
        Self {
            version: VERSION,
            codec,
            flags: 0,
        }
    }

    fn write_to<W: Write>(&self, w: &mut W) -> Result<()> {
        let mut buf = [0u8; HEADER_BYTES];
        buf[0..4].copy_from_slice(&MAGIC);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6] = self.codec.id();
        buf[7] = self.codec.level() as u8;
        buf[8..10].copy_from_slice(&self.flags.to_le_bytes());
        // buf[10..12] reserved, zero.
        w.write_all(&buf)?;
        Ok(())
    }

    fn read_from<R: Read>(r: &mut R) -> Result<Self> {
        let mut buf = [0u8; HEADER_BYTES];
        r.read_exact(&mut buf)?;

        let magic: [u8; 4] = buf[0..4].try_into().expect("slice is 4 bytes");
        if magic != MAGIC {
            return Err(ProtoError::BadMagic {
                expected: MAGIC,
                got: magic,
            });
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != VERSION {
            return Err(ProtoError::UnsupportedVersion(version));
        }
        let codec = Codec::from_wire(buf[6], buf[7] as i8)?;
        let flags = u16::from_le_bytes([buf[8], buf[9]]);
        Ok(Self {
            version,
            codec,
            flags,
        })
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

enum Sink<W: Write> {
    Raw(W),
    Zstd(Box<zstd::stream::write::Encoder<'static, W>>),
}

impl<W: Write> Write for Sink<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Sink::Raw(w) => w.write(buf),
            Sink::Zstd(e) => e.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sink::Raw(w) => w.flush(),
            Sink::Zstd(e) => e.flush(),
        }
    }
}

/// Encodes a batch stream.
///
/// Call [`BatchWriter::finish`] rather than dropping, otherwise the trailing
/// zstd frame is never flushed and the stream is truncated.
pub struct BatchWriter<W: Write> {
    sink: Sink<W>,
    codec: Codec,
    entries_written: u64,
    logical_bytes: u64,
}

impl<W: Write> BatchWriter<W> {
    pub fn new(mut inner: W, codec: Codec) -> Result<Self> {
        StreamHeader::new(codec).write_to(&mut inner)?;
        let sink = match codec {
            Codec::Raw => Sink::Raw(inner),
            Codec::Zstd(level) => {
                let mut encoder = zstd::stream::write::Encoder::new(inner, level)?;
                // Long-distance matching lets the dictionary reach back across
                // many files, which is exactly the win we are after for large
                // batches of similar small files.
                let _ = encoder.long_distance_matching(true);
                Sink::Zstd(Box::new(encoder))
            }
        };
        Ok(Self {
            sink,
            codec,
            entries_written: 0,
            logical_bytes: 0,
        })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    pub fn entries_written(&self) -> u64 {
        self.entries_written
    }

    /// Total uncompressed payload bytes handed to this writer.
    pub fn logical_bytes(&self) -> u64 {
        self.logical_bytes
    }

    pub fn write_file(&mut self, path: &str, mtime: i64, data: &[u8]) -> Result<()> {
        if data.len() as u64 > MAX_ENTRY_BYTES {
            return Err(ProtoError::FrameTooLarge {
                declared: data.len() as u64,
                limit: MAX_ENTRY_BYTES,
            });
        }
        self.write_common(EntryKind::File, path, mtime)?;
        self.sink.write_all(&(data.len() as u64).to_le_bytes())?;
        self.sink.write_all(data)?;
        self.entries_written += 1;
        self.logical_bytes += data.len() as u64;
        Ok(())
    }

    /// Streams a file body straight from a reader, so the host never holds a
    /// whole file in memory. `len` must match exactly what `src` yields.
    pub fn write_file_from<R: Read>(
        &mut self,
        path: &str,
        mtime: i64,
        len: u64,
        src: &mut R,
    ) -> Result<()> {
        if len > MAX_ENTRY_BYTES {
            return Err(ProtoError::FrameTooLarge {
                declared: len,
                limit: MAX_ENTRY_BYTES,
            });
        }
        self.write_common(EntryKind::File, path, mtime)?;
        self.sink.write_all(&len.to_le_bytes())?;
        let copied = std::io::copy(&mut src.take(len), &mut self.sink)?;
        if copied != len {
            return Err(ProtoError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("declared {len} bytes for {path} but source yielded {copied}"),
            )));
        }
        self.entries_written += 1;
        self.logical_bytes += len;
        Ok(())
    }

    pub fn write_dir(&mut self, path: &str, mtime: i64) -> Result<()> {
        self.write_common(EntryKind::Dir, path, mtime)?;
        self.entries_written += 1;
        Ok(())
    }

    /// Records a per-entry failure inline. One unreadable file must not abort
    /// a batch of thousands.
    pub fn write_error(&mut self, path: &str, message: &str) -> Result<()> {
        self.write_common(EntryKind::Error, path, 0)?;
        let msg = message.as_bytes();
        let msg = &msg[..msg.len().min(u16::MAX as usize)];
        self.sink.write_all(&(msg.len() as u16).to_le_bytes())?;
        self.sink.write_all(msg)?;
        self.entries_written += 1;
        Ok(())
    }

    fn write_common(&mut self, kind: EntryKind, path: &str, mtime: i64) -> Result<()> {
        let bytes = path.as_bytes();
        if bytes.len() > MAX_PATH_BYTES {
            return Err(ProtoError::UnsafePath(format!(
                "path exceeds {MAX_PATH_BYTES} bytes"
            )));
        }
        self.sink.write_all(&[kind as u8])?;
        self.sink.write_all(&(bytes.len() as u16).to_le_bytes())?;
        self.sink.write_all(bytes)?;
        self.sink.write_all(&mtime.to_le_bytes())?;
        Ok(())
    }

    /// Writes the terminator and flushes the codec. Returns the inner writer.
    pub fn finish(mut self) -> Result<W> {
        self.sink.write_all(&[EntryKind::End as u8])?;
        match self.sink {
            Sink::Raw(mut w) => {
                w.flush()?;
                Ok(w)
            }
            Sink::Zstd(e) => Ok(e.finish()?),
        }
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

enum Source<R: Read> {
    Raw(R),
    Zstd(Box<zstd::stream::read::Decoder<'static, BufReader<R>>>),
}

impl<R: Read> Read for Source<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Source::Raw(r) => r.read(buf),
            Source::Zstd(d) => d.read(buf),
        }
    }
}

/// Decodes a batch stream produced by [`BatchWriter`].
pub struct BatchReader<R: Read> {
    source: Source<R>,
    header: StreamHeader,
    finished: bool,
    max_entry_bytes: u64,
}

impl<R: Read> BatchReader<R> {
    pub fn new(mut inner: R) -> Result<Self> {
        let header = StreamHeader::read_from(&mut inner)?;
        let source = match header.codec {
            Codec::Raw => Source::Raw(inner),
            Codec::Zstd(_) => Source::Zstd(Box::new(zstd::stream::read::Decoder::new(inner)?)),
        };
        Ok(Self {
            source,
            header,
            finished: false,
            max_entry_bytes: MAX_ENTRY_BYTES,
        })
    }

    pub fn header(&self) -> StreamHeader {
        self.header
    }

    /// Reads the next entry, or `None` at the end of the stream.
    pub fn read_entry(&mut self) -> Result<Option<Entry>> {
        if self.finished {
            return Ok(None);
        }

        let mut kind_byte = [0u8; 1];
        self.source.read_exact(&mut kind_byte)?;
        let kind = EntryKind::from_u8(kind_byte[0])?;
        if kind == EntryKind::End {
            self.finished = true;
            return Ok(None);
        }

        let path = self.read_path()?;
        let mtime = self.read_i64()?;

        let (data, error) = match kind {
            EntryKind::File => {
                let len = self.read_u64()?;
                if len > self.max_entry_bytes {
                    return Err(ProtoError::FrameTooLarge {
                        declared: len,
                        limit: self.max_entry_bytes,
                    });
                }
                let mut data = vec![0u8; len as usize];
                self.source.read_exact(&mut data)?;
                (data, None)
            }
            EntryKind::Error => {
                let mut len_buf = [0u8; 2];
                self.source.read_exact(&mut len_buf)?;
                let len = u16::from_le_bytes(len_buf) as usize;
                let mut msg = vec![0u8; len];
                self.source.read_exact(&mut msg)?;
                let msg = String::from_utf8(msg).map_err(|_| ProtoError::InvalidPath)?;
                (Vec::new(), Some(msg))
            }
            EntryKind::Dir => (Vec::new(), None),
            EntryKind::End => unreachable!("handled above"),
        };

        Ok(Some(Entry {
            kind,
            path,
            mtime,
            data,
            error,
        }))
    }

    fn read_path(&mut self) -> Result<String> {
        let mut len_buf = [0u8; 2];
        self.source.read_exact(&mut len_buf)?;
        let len = u16::from_le_bytes(len_buf) as usize;
        if len > MAX_PATH_BYTES {
            return Err(ProtoError::UnsafePath(format!(
                "path exceeds {MAX_PATH_BYTES} bytes"
            )));
        }
        let mut buf = vec![0u8; len];
        self.source.read_exact(&mut buf)?;
        let raw = String::from_utf8(buf).map_err(|_| ProtoError::InvalidPath)?;
        // Validate on the way in. A path that arrives over the network is
        // untrusted input and will be joined against a local root.
        sanitize_relative_path(&raw)
    }

    fn read_u64(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.source.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_i64(&mut self) -> Result<i64> {
        let mut buf = [0u8; 8];
        self.source.read_exact(&mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }
}

impl<R: Read> Iterator for BatchReader<R> {
    type Item = Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_entry().transpose()
    }
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Windows device names that are reserved regardless of extension. Creating a
/// file with one of these names can hang or hit a device instead of the disk.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Validates a path received over the wire before it is joined to a local root.
///
/// Rejects absolute paths, drive letters, UNC prefixes, `..` traversal, NUL and
/// other control bytes, trailing dots/spaces (silently stripped by Windows,
/// which can be used to dodge extension checks), and reserved device names.
///
/// Returns the path normalised to `/` separators with redundant components
/// removed.
pub fn sanitize_relative_path(path: &str) -> Result<String> {
    let unsafe_path = |why: &str| ProtoError::UnsafePath(format!("{path}: {why}"));

    if path.is_empty() {
        return Err(unsafe_path("empty"));
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(unsafe_path("too long"));
    }
    if path.contains('\0') {
        return Err(unsafe_path("contains NUL"));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(unsafe_path("contains a control character"));
    }

    let normalised = path.replace('\\', "/");

    if normalised.starts_with('/') {
        return Err(unsafe_path("absolute"));
    }
    if normalised.starts_with("//") {
        return Err(unsafe_path("UNC prefix"));
    }
    // "C:" or "C:/..." — a drive-relative or drive-absolute path.
    let bytes = normalised.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(unsafe_path("drive letter"));
    }

    let mut parts: Vec<&str> = Vec::new();
    for segment in normalised.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err(unsafe_path("contains '..'")),
            s => {
                // Windows strips trailing dots and spaces, so "evil.exe." and
                // "evil.exe " both resolve to "evil.exe". Reject rather than
                // normalise, so the name we validate is the name on disk.
                if s.ends_with('.') || s.ends_with(' ') {
                    return Err(unsafe_path("segment ends with a dot or space"));
                }
                let stem = s.split('.').next().unwrap_or(s).to_ascii_lowercase();
                if WINDOWS_RESERVED.contains(&stem.as_str()) {
                    return Err(unsafe_path("reserved Windows device name"));
                }
                parts.push(s);
            }
        }
    }

    if parts.is_empty() {
        return Err(unsafe_path("resolves to nothing"));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(codec: Codec, entries: &[(&str, &[u8])]) -> Vec<Entry> {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, codec).unwrap();
        for (path, data) in entries {
            w.write_file(path, 1_700_000_000, data).unwrap();
        }
        w.finish().unwrap();

        let mut r = BatchReader::new(buf.as_slice()).unwrap();
        assert_eq!(r.header().codec, codec);
        let mut out = Vec::new();
        while let Some(e) = r.read_entry().unwrap() {
            out.push(e);
        }
        out
    }

    #[test]
    fn raw_stream_round_trips() {
        let entries = round_trip(Codec::Raw, &[("a.txt", b"hello"), ("b/c.txt", b"world")]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "a.txt");
        assert_eq!(entries[0].data, b"hello");
        assert_eq!(entries[1].path, "b/c.txt");
        assert_eq!(entries[1].data, b"world");
    }

    #[test]
    fn zstd_stream_round_trips() {
        let entries = round_trip(Codec::Zstd(1), &[("a.txt", b"hello"), ("b.txt", b"world")]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].data, b"hello");
        assert_eq!(entries[1].data, b"world");
    }

    #[test]
    fn empty_stream_round_trips() {
        assert_eq!(round_trip(Codec::Zstd(1), &[]).len(), 0);
    }

    #[test]
    fn empty_files_round_trip() {
        let entries = round_trip(Codec::Raw, &[("empty.txt", b"")]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].data.is_empty());
    }

    #[test]
    fn shared_window_beats_per_file_compression() {
        // The core reason the body is one zstd stream rather than one stream
        // per file: 500 similar small files share a dictionary.
        let file = b"fn main() { println!(\"hello, world\"); } // boilerplate line\n";
        let paths: Vec<String> = (0..500).map(|i| format!("src/file{i}.rs")).collect();

        let mut batched = Vec::new();
        let mut w = BatchWriter::new(&mut batched, Codec::Zstd(1)).unwrap();
        for p in &paths {
            w.write_file(p, 0, file).unwrap();
        }
        w.finish().unwrap();

        let per_file: usize = paths
            .iter()
            .map(|_| zstd::encode_all(&file[..], 1).unwrap().len())
            .sum();

        assert!(
            batched.len() * 4 < per_file,
            "shared-window batch ({} bytes) should be far smaller than \
             per-file compression ({} bytes)",
            batched.len(),
            per_file
        );
    }

    #[test]
    fn dirs_and_errors_round_trip() {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Zstd(1)).unwrap();
        w.write_dir("photos", 42).unwrap();
        w.write_error("locked.bin", "access denied").unwrap();
        w.write_file("ok.txt", 7, b"fine").unwrap();
        w.finish().unwrap();

        let entries: Vec<Entry> = BatchReader::new(buf.as_slice())
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();

        assert_eq!(entries[0].kind, EntryKind::Dir);
        assert_eq!(entries[0].mtime, 42);
        assert_eq!(entries[1].kind, EntryKind::Error);
        assert_eq!(entries[1].error.as_deref(), Some("access denied"));
        assert_eq!(entries[2].kind, EntryKind::File);
        assert_eq!(entries[2].data, b"fine");
    }

    #[test]
    fn write_file_from_streams_without_buffering() {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
        let body = vec![7u8; 100_000];
        w.write_file_from("big.bin", 1, body.len() as u64, &mut body.as_slice())
            .unwrap();
        assert_eq!(w.logical_bytes(), 100_000);
        w.finish().unwrap();

        let entries: Vec<Entry> = BatchReader::new(buf.as_slice())
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(entries[0].data, body);
    }

    #[test]
    fn write_file_from_rejects_a_short_source() {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
        let short = vec![0u8; 10];
        let err = w.write_file_from("x.bin", 0, 1000, &mut short.as_slice());
        assert!(
            err.is_err(),
            "declaring more bytes than the source has must fail"
        );
    }

    #[test]
    fn oversized_entries_are_rejected_on_write() {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
        let err = w.write_file_from("huge.bin", 0, MAX_ENTRY_BYTES + 1, &mut std::io::empty());
        assert!(matches!(err, Err(ProtoError::FrameTooLarge { .. })));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let err = BatchReader::new(&b"NOPEnotastream"[..]);
        assert!(matches!(err, Err(ProtoError::BadMagic { .. })));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut header = Vec::new();
        header.extend_from_slice(&MAGIC);
        header.extend_from_slice(&999u16.to_le_bytes());
        header.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        assert!(matches!(
            BatchReader::new(header.as_slice()),
            Err(ProtoError::UnsupportedVersion(999))
        ));
    }

    #[test]
    fn truncated_stream_is_an_error_not_a_silent_success() {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
        w.write_file("a.txt", 0, b"some content here").unwrap();
        w.finish().unwrap();

        buf.truncate(buf.len() - 6);
        let mut r = BatchReader::new(buf.as_slice()).unwrap();
        // Either the entry read fails, or it succeeds and the *next* read hits
        // EOF before the End marker. Both are errors; neither is a clean end.
        let first = r.read_entry();
        let outcome = match first {
            Err(_) => Err(()),
            Ok(_) => r.read_entry().map_err(|_| ()),
        };
        assert!(
            outcome.is_err(),
            "a truncated stream must not decode cleanly"
        );
    }

    // --- path safety -------------------------------------------------------

    #[test]
    fn safe_relative_paths_are_accepted_and_normalised() {
        assert_eq!(sanitize_relative_path("a/b/c.txt").unwrap(), "a/b/c.txt");
        assert_eq!(sanitize_relative_path("a\\b\\c.txt").unwrap(), "a/b/c.txt");
        assert_eq!(
            sanitize_relative_path("./a//b/./c.txt").unwrap(),
            "a/b/c.txt"
        );
        assert_eq!(sanitize_relative_path("file.txt").unwrap(), "file.txt");
    }

    #[test]
    fn traversal_is_rejected() {
        for p in [
            "../secrets",
            "a/../../etc/passwd",
            "..",
            "a/..",
            "..\\..\\windows\\system32",
        ] {
            assert!(
                sanitize_relative_path(p).is_err(),
                "traversal path {p:?} must be rejected"
            );
        }
    }

    #[test]
    fn absolute_and_drive_paths_are_rejected() {
        for p in [
            "/etc/passwd",
            "C:/Windows",
            "c:windows",
            "\\\\server\\share",
            "//server/share",
        ] {
            assert!(
                sanitize_relative_path(p).is_err(),
                "absolute path {p:?} must be rejected"
            );
        }
    }

    #[test]
    fn control_characters_and_nul_are_rejected() {
        assert!(sanitize_relative_path("a\0b").is_err());
        assert!(sanitize_relative_path("a\nb").is_err());
        assert!(sanitize_relative_path("a\tb").is_err());
    }

    #[test]
    fn windows_reserved_device_names_are_rejected() {
        for p in ["con", "PRN", "nul.txt", "a/aux/b", "COM1.log", "lpt9"] {
            assert!(
                sanitize_relative_path(p).is_err(),
                "reserved device name {p:?} must be rejected"
            );
        }
    }

    #[test]
    fn trailing_dots_and_spaces_are_rejected() {
        // Windows silently strips these, so "evil.exe." lands as "evil.exe".
        assert!(sanitize_relative_path("evil.exe.").is_err());
        assert!(sanitize_relative_path("evil.exe ").is_err());
        assert!(sanitize_relative_path("dir./file").is_err());
    }

    #[test]
    fn empty_and_degenerate_paths_are_rejected() {
        assert!(sanitize_relative_path("").is_err());
        assert!(sanitize_relative_path(".").is_err());
        assert!(sanitize_relative_path("./././").is_err());
    }

    #[test]
    fn a_malicious_path_on_the_wire_is_caught_at_decode_time() {
        // The writer does not validate — a hostile or buggy peer could emit
        // anything. The reader must be the checkpoint.
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
        w.write_file("../../etc/passwd", 0, b"pwned").unwrap();
        w.finish().unwrap();

        let mut r = BatchReader::new(buf.as_slice()).unwrap();
        assert!(matches!(r.read_entry(), Err(ProtoError::UnsafePath(_))));
    }
}
