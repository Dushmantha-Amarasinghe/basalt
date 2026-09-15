//! Compression policy.
//!
//! Context that makes this the highest-value component in the system: with both
//! the host laptop and the client on Wi-Fi, the link ceiling is roughly
//! 25–40 MB/s. zstd level 1 compresses at 400–800 MB/s — about 20x faster than
//! we can put bytes on the air. So for *compressible* data, compression is
//! effectively free throughput.
//!
//! The catch is that most bytes on a media drive are already compressed (H.264,
//! JPEG, ZIP). Running zstd over those burns CPU, usually *grows* the payload
//! slightly, and buys nothing. So the whole job of this module is to sort one
//! from the other as cheaply as possible.
//!
//! Two-stage decision, cheapest first:
//! 1. Extension lookup — free, catches the overwhelming majority of real files.
//! 2. Shannon entropy over a small head sample — ~microseconds, catches the
//!    rest (extensionless files, misnamed files, unknown formats).

use crate::{ProtoError, Result};

/// Compression applied to a payload or batch stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Send bytes untouched.
    Raw,
    /// zstd at the given level.
    Zstd(i32),
}

impl Codec {
    pub fn id(self) -> u8 {
        match self {
            Codec::Raw => 0,
            Codec::Zstd(_) => 1,
        }
    }

    /// zstd level, or 0 for raw. Carried on the wire so the receiver can report
    /// what actually happened; decompression does not need it.
    pub fn level(self) -> i8 {
        match self {
            Codec::Raw => 0,
            Codec::Zstd(l) => l as i8,
        }
    }

    pub fn from_wire(id: u8, level: i8) -> Result<Self> {
        match id {
            0 => Ok(Codec::Raw),
            1 => Ok(Codec::Zstd(level as i32)),
            other => Err(ProtoError::UnknownCodec(other)),
        }
    }

    pub fn is_raw(self) -> bool {
        matches!(self, Codec::Raw)
    }
}

/// Extensions whose contents are already entropy-coded. Compressing these is
/// always a loss, so we skip even the entropy sample.
///
/// Deliberately conservative: a false negative here just means we pay for one
/// cheap entropy sample, while a false positive means we ship compressible
/// bytes uncompressed over a slow link.
const PRECOMPRESSED_EXTENSIONS: &[&str] = &[
    // Video
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "m2ts", "vob",
    "3gp", "ogv", "mxf", "rmvb", "divx", "asf", // Audio
    "mp3", "aac", "m4a", "ogg", "oga", "opus", "flac", "wma", "ape", "alac", "mka", "ac3", "dts",
    // Images
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "jxl", "jp2", "bpg",
    // Archives / packages
    "zip", "rar", "7z", "gz", "bz2", "xz", "zst", "lz4", "lzma", "cab", "arj", "tgz", "tbz", "txz",
    "jar", "war", "apk", "ipa", "deb", "rpm", "pkg", "dmg", "msix", "appx", "nupkg", "whl",
    "crate", "xpi", "vsix", // Office / OpenDocument (zip containers)
    "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "kra", "ora",
    // Disc / VM images that are typically already compressed
    "iso", "vhdx", "qcow2", "squashfs", // Misc entropy-coded
    "pdf", "swf", "woff", "woff2", "crypt", "gpg", "aes",
];

/// How aggressively to compress, and how we decide.
#[derive(Debug, Clone, Copy)]
pub struct CompressionPolicy {
    /// zstd level used when we do compress. Level 1 is the default: on this
    /// link, throughput matters far more than the last few percent of ratio.
    pub level: i32,
    /// Shannon entropy above which a payload is treated as incompressible,
    /// in bits per byte (theoretical max 8.0).
    ///
    /// 7.5 is the working default. Truly random and entropy-coded data sits at
    /// 7.98+; English text lands near 4.5; source code and JSON near 5.0–5.5.
    /// Phase 0 calibrates this against the real corpus.
    pub entropy_threshold: f32,
    /// Bytes sampled from the head of a payload for the entropy estimate.
    pub sample_bytes: usize,
    /// Payloads at or below this size skip compression entirely — framing and
    /// zstd header overhead dominate and the absolute saving is negligible.
    pub min_size: u64,
    /// Skip the extension fast path and always run the entropy sample. Used by
    /// the benchmark to measure how well the heuristic agrees with reality.
    pub always_sample: bool,
}

impl Default for CompressionPolicy {
    fn default() -> Self {
        Self {
            level: 1,
            entropy_threshold: 7.5,
            sample_bytes: 64 * 1024,
            min_size: 512,
            always_sample: false,
        }
    }
}

impl CompressionPolicy {
    /// Never compress anything. Used as the control arm in benchmarks.
    pub fn disabled() -> Self {
        Self {
            level: 0,
            entropy_threshold: -1.0,
            sample_bytes: 0,
            min_size: u64::MAX,
            always_sample: false,
        }
    }

    pub fn with_level(mut self, level: i32) -> Self {
        self.level = level;
        self
    }

    /// Decide how to encode one payload.
    ///
    /// `sample` should be the first [`Self::sample_bytes`] of the payload (less
    /// is fine for short payloads). `size` is the full payload length, which
    /// may be much larger than the sample.
    pub fn decide(&self, path: &str, size: u64, sample: &[u8]) -> Codec {
        if self.min_size == u64::MAX || size < self.min_size {
            return Codec::Raw;
        }
        if !self.always_sample && is_precompressed_extension(path) {
            return Codec::Raw;
        }
        if sample.is_empty() {
            // No evidence either way. Compressing is the cheaper mistake: worst
            // case zstd adds a few bytes of header.
            return Codec::Zstd(self.level);
        }
        if shannon_entropy(sample) > self.entropy_threshold {
            Codec::Raw
        } else {
            Codec::Zstd(self.level)
        }
    }
}

/// True if the path's extension names an already-compressed format.
pub fn is_precompressed_extension(path: &str) -> bool {
    let Some(ext) = path.rsplit('.').next() else {
        return false;
    };
    // `rsplit` yields the whole string when there is no '.', which is not an
    // extension. Guard against treating "README" as extension "README".
    if ext.len() == path.len() {
        return false;
    }
    if ext.is_empty() || ext.len() > 8 {
        return false;
    }
    let mut buf = [0u8; 8];
    let bytes = ext.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        buf[i] = b.to_ascii_lowercase();
    }
    let lower = std::str::from_utf8(&buf[..bytes.len()]).unwrap_or("");
    // Linear scan over ~100 short strings, once per file. The table is grouped
    // by media type for readability rather than sorted, so this is not a
    // binary search; at this call rate the difference is unmeasurable.
    PRECOMPRESSED_EXTENSIONS.contains(&lower)
}

/// Shannon entropy of a byte slice, in bits per byte (0.0 ..= 8.0).
///
/// This is the cheap discriminator between "worth compressing" and "already
/// compressed". A 256-entry histogram plus one pass of `log2` is far faster
/// than trial-compressing, and on this link the decision only needs to be
/// directionally right.
pub fn shannon_entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let mut histogram = [0u32; 256];
    for &b in data {
        histogram[b as usize] += 1;
    }
    let len = data.len() as f32;
    let mut entropy = 0.0f32;
    for &count in &histogram {
        if count == 0 {
            continue;
        }
        let p = count as f32 / len;
        entropy -= p * p.log2();
    }
    entropy
}

/// Measured compression outcome, for reporting "2.3x effective" in the UI and
/// for the Phase 0 report.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct CompressionStats {
    pub logical_bytes: u64,
    pub wire_bytes: u64,
    pub files_compressed: u64,
    pub files_raw: u64,
}

impl CompressionStats {
    pub fn record(&mut self, codec: Codec, logical: u64, wire: u64) {
        self.logical_bytes += logical;
        self.wire_bytes += wire;
        if codec.is_raw() {
            self.files_raw += 1;
        } else {
            self.files_compressed += 1;
        }
    }

    /// Effective throughput multiplier: how many logical bytes we moved per
    /// byte actually put on the wire. 1.0 means compression bought nothing.
    pub fn ratio(&self) -> f64 {
        if self.wire_bytes == 0 {
            return 1.0;
        }
        self.logical_bytes as f64 / self.wire_bytes as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_of_uniform_data_is_zero() {
        assert_eq!(shannon_entropy(&[0u8; 4096]), 0.0);
    }

    #[test]
    fn entropy_of_all_byte_values_is_eight() {
        let data: Vec<u8> = (0..=255u8).cycle().take(256 * 16).collect();
        assert!((shannon_entropy(&data) - 8.0).abs() < 0.001);
    }

    #[test]
    fn entropy_of_english_text_is_low() {
        let text = b"the quick brown fox jumps over the lazy dog. \
                     pack my box with five dozen liquor jugs. \
                     how vexingly quick daft zebras jump!";
        let e = shannon_entropy(text);
        assert!(e > 3.0 && e < 5.5, "expected text entropy 3..5.5, got {e}");
    }

    #[test]
    fn entropy_of_empty_slice_is_zero() {
        assert_eq!(shannon_entropy(&[]), 0.0);
    }

    #[test]
    fn precompressed_extensions_are_detected_case_insensitively() {
        assert!(is_precompressed_extension("holiday.MP4"));
        assert!(is_precompressed_extension("a/b/c.jpg"));
        assert!(is_precompressed_extension("archive.7z"));
        assert!(!is_precompressed_extension("notes.txt"));
        assert!(!is_precompressed_extension("main.rs"));
    }

    #[test]
    fn extensionless_paths_are_not_precompressed() {
        // Regression guard: `rsplit('.')` returns the whole string when there
        // is no dot, which must not be read as an extension.
        assert!(!is_precompressed_extension("README"));
        assert!(!is_precompressed_extension("Makefile"));
        assert!(!is_precompressed_extension(""));
    }

    #[test]
    fn policy_skips_known_compressed_extensions() {
        let policy = CompressionPolicy::default();
        // Low-entropy body, but the extension says it is already compressed —
        // the fast path should win and avoid even sampling.
        let sample = vec![b'a'; 4096];
        assert_eq!(policy.decide("movie.mp4", 10_000_000, &sample), Codec::Raw);
    }

    #[test]
    fn policy_compresses_low_entropy_payloads() {
        let policy = CompressionPolicy::default();
        let sample = vec![b'a'; 4096];
        assert_eq!(policy.decide("notes.txt", 10_000, &sample), Codec::Zstd(1));
    }

    #[test]
    fn policy_skips_high_entropy_payloads_despite_unknown_extension() {
        let policy = CompressionPolicy::default();
        // Deterministic pseudo-random bytes: high entropy, unknown extension.
        let sample: Vec<u8> = (0..65536u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 16) as u8)
            .collect();
        assert_eq!(policy.decide("blob.unknown", 1 << 20, &sample), Codec::Raw);
    }

    #[test]
    fn policy_skips_tiny_payloads() {
        let policy = CompressionPolicy::default();
        assert_eq!(policy.decide("notes.txt", 16, b"aaaa"), Codec::Raw);
    }

    #[test]
    fn disabled_policy_never_compresses() {
        let policy = CompressionPolicy::disabled();
        let sample = vec![b'a'; 4096];
        assert_eq!(policy.decide("notes.txt", 1 << 20, &sample), Codec::Raw);
    }

    #[test]
    fn always_sample_bypasses_the_extension_table() {
        let mut policy = CompressionPolicy::default();
        policy.always_sample = true;
        // A .mp4 whose body is actually trivially compressible: with the fast
        // path bypassed, entropy decides and we compress.
        let sample = vec![b'a'; 4096];
        assert_eq!(policy.decide("fake.mp4", 1 << 20, &sample), Codec::Zstd(1));
    }

    #[test]
    fn codec_round_trips_through_the_wire_encoding() {
        for codec in [Codec::Raw, Codec::Zstd(1), Codec::Zstd(3), Codec::Zstd(19)] {
            let decoded = Codec::from_wire(codec.id(), codec.level()).unwrap();
            assert_eq!(codec, decoded);
        }
    }

    #[test]
    fn unknown_codec_id_is_rejected() {
        assert!(Codec::from_wire(200, 0).is_err());
    }

    #[test]
    fn compression_stats_report_the_effective_multiplier() {
        let mut stats = CompressionStats::default();
        stats.record(Codec::Zstd(1), 1000, 250);
        stats.record(Codec::Raw, 1000, 1000);
        assert_eq!(stats.logical_bytes, 2000);
        assert_eq!(stats.wire_bytes, 1250);
        assert_eq!(stats.files_compressed, 1);
        assert_eq!(stats.files_raw, 1);
        assert!((stats.ratio() - 1.6).abs() < 1e-9);
    }

    #[test]
    fn empty_stats_report_a_neutral_ratio() {
        assert_eq!(CompressionStats::default().ratio(), 1.0);
    }
}
