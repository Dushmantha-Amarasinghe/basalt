//! Test corpus generation.
//!
//! The compression benchmark is the headline measurement of Phase 0, and it is
//! only worth anything if the test data compresses like real data. Random bytes
//! would make compression look useless; a repeating byte would make it look
//! miraculous. Neither tells us what will happen to an actual drive.
//!
//! So the generator produces four families with deliberately different
//! compressibility, matching what actually sits on a media/documents drive:
//!
//! | Family    | Models                       | Expected zstd-1 ratio |
//! |-----------|------------------------------|-----------------------|
//! | `prose`   | documents, subtitles, notes  | ~2.5–3.5x             |
//! | `code`    | source trees, configs, logs  | ~3–5x                 |
//! | `json`    | databases, metadata, exports | ~5–10x                |
//! | `binary`  | video, JPEG, archives        | ~1.0x (incompressible)|
//!
//! Everything is generated from a seeded PRNG, so a corpus is reproducible:
//! the same `--seed` gives byte-identical files on any machine, which makes
//! results comparable across runs and across the two machines.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::stats::fmt_bytes;

/// Deterministic, fast, and good enough for generating test payloads.
/// Not cryptographic — nothing here needs it.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the zero state, which xorshift cannot escape.
        Self(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    #[inline]
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        if hi <= lo {
            lo
        } else {
            lo + self.below(hi - lo)
        }
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        let (chunks, rem) = buf.as_chunks_mut::<8>();
        for c in chunks {
            *c = self.next_u64().to_le_bytes();
        }
        if !rem.is_empty() {
            let bytes = self.next_u64().to_le_bytes();
            rem.copy_from_slice(&bytes[..rem.len()]);
        }
    }
}

/// What kind of content to synthesise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    Prose,
    Code,
    Json,
    Binary,
}

impl Flavour {
    pub fn extension(self) -> &'static str {
        match self {
            Flavour::Prose => "txt",
            Flavour::Code => "rs",
            Flavour::Json => "json",
            // .bin is deliberately *not* in the pre-compressed extension table,
            // which forces the entropy sampler to do the work. That is what we
            // want to measure.
            Flavour::Binary => "bin",
        }
    }

    pub fn all() -> [Flavour; 4] {
        [
            Flavour::Prose,
            Flavour::Code,
            Flavour::Json,
            Flavour::Binary,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            Flavour::Prose => "prose",
            Flavour::Code => "code",
            Flavour::Json => "json",
            Flavour::Binary => "binary",
        }
    }
}

const WORDS: &[&str] = &[
    "the",
    "drive",
    "network",
    "transfer",
    "wireless",
    "latency",
    "throughput",
    "buffer",
    "stream",
    "packet",
    "index",
    "cache",
    "sector",
    "spindle",
    "request",
    "response",
    "client",
    "host",
    "session",
    "handshake",
    "certificate",
    "device",
    "volume",
    "directory",
    "metadata",
    "thumbnail",
    "playback",
    "resume",
    "chunk",
    "parallel",
    "compression",
    "entropy",
    "sample",
    "measure",
    "median",
    "baseline",
    "protocol",
    "connection",
    "bandwidth",
    "saturate",
    "queue",
    "scheduler",
    "seek",
    "sequential",
    "random",
    "buffered",
    "flush",
    "commit",
    "consistent",
    "and",
    "of",
    "to",
    "in",
    "for",
    "with",
    "that",
    "which",
    "when",
    "from",
    "into",
    "over",
];

const IDENTIFIERS: &[&str] = &[
    "buffer", "reader", "writer", "count", "offset", "length", "result", "handle", "entry", "path",
    "codec", "stream", "policy", "session", "client", "host", "index", "cache",
];

const TYPES: &[&str] = &[
    "u64", "usize", "String", "Vec<u8>", "bool", "i64", "PathBuf", "f64",
];

// Per-section seed salts, so the three corpus sections do not generate
// correlated content from the same base seed.
const SALT_SMALL: u64 = 0x5A17_0001;
const SALT_WIDE: u64 = 0x5A17_0002;

/// Generates approximately `target` bytes of the given flavour into `out`.
///
/// Output length is approximate for text flavours: the generator stops at the
/// first line boundary past the target rather than truncating mid-token, so the
/// content stays well-formed and compresses realistically.
pub fn generate(flavour: Flavour, target: usize, rng: &mut Rng, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(target + 256);
    match flavour {
        Flavour::Binary => {
            out.resize(target, 0);
            rng.fill(out);
        }
        Flavour::Prose => {
            while out.len() < target {
                let words = rng.range(8, 20);
                for i in 0..words {
                    if i > 0 {
                        out.push(b' ');
                    }
                    out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
                }
                out.extend_from_slice(b".\n");
            }
        }
        Flavour::Code => {
            let mut depth = 0usize;
            while out.len() < target {
                let indent = "    ".repeat(depth.min(3));
                match rng.below(6) {
                    0 => {
                        out.extend_from_slice(
                            format!(
                                "{indent}pub fn {}_{}(&self, {}: {}) -> {} {{\n",
                                IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                                rng.below(1000),
                                IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                                TYPES[rng.below(TYPES.len())],
                                TYPES[rng.below(TYPES.len())],
                            )
                            .as_bytes(),
                        );
                        depth += 1;
                    }
                    1 if depth > 0 => {
                        depth -= 1;
                        out.extend_from_slice(format!("{indent}}}\n\n").as_bytes());
                    }
                    2 => out.extend_from_slice(
                        format!(
                            "{indent}let {} = self.{}.len();\n",
                            IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                            IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                        )
                        .as_bytes(),
                    ),
                    3 => out.extend_from_slice(
                        format!(
                            "{indent}// {} the {} before we {} it\n",
                            WORDS[rng.below(WORDS.len())],
                            WORDS[rng.below(WORDS.len())],
                            WORDS[rng.below(WORDS.len())],
                        )
                        .as_bytes(),
                    ),
                    4 => out.extend_from_slice(
                        format!(
                            "{indent}if {} > {} {{ return Err(Error::TooLarge); }}\n",
                            IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                            rng.below(100_000),
                        )
                        .as_bytes(),
                    ),
                    _ => out.extend_from_slice(
                        format!(
                            "{indent}self.{}.push({});\n",
                            IDENTIFIERS[rng.below(IDENTIFIERS.len())],
                            rng.below(10_000),
                        )
                        .as_bytes(),
                    ),
                }
            }
        }
        Flavour::Json => {
            out.extend_from_slice(b"[\n");
            while out.len() < target {
                out.extend_from_slice(
                    format!(
                        "  {{\"id\": {}, \"name\": \"{}_{}\", \"size\": {}, \"mtime\": {}, \
                         \"kind\": \"{}\", \"indexed\": {}, \"checksum\": \"{:016x}\"}},\n",
                        rng.below(1_000_000),
                        WORDS[rng.below(WORDS.len())],
                        rng.below(10_000),
                        rng.below(50_000_000),
                        1_700_000_000u64 + rng.below(50_000_000) as u64,
                        ["file", "dir", "link"][rng.below(3)],
                        rng.below(2) == 0,
                        rng.next_u64(),
                    )
                    .as_bytes(),
                );
            }
            out.extend_from_slice(b"]\n");
        }
    }
}

/// Layout of a generated corpus.
pub struct Corpus {
    pub root: PathBuf,
}

/// Knobs for corpus size. `quick` trades fidelity for turnaround while
/// iterating on the harness.
#[derive(Debug, Clone, Copy)]
pub struct CorpusSpec {
    /// Bytes per large file, one per flavour.
    pub large_file_bytes: u64,
    /// Number of small files, spread across flavours.
    pub small_file_count: usize,
    /// Approximate bytes per small file.
    pub small_file_bytes: usize,
    /// Entries in the single wide directory used for the listing benchmark.
    pub wide_entry_count: usize,
    pub seed: u64,
}

impl CorpusSpec {
    /// Full-fidelity corpus.
    ///
    /// Sized for a low-end laptop writing to a USB hard drive, which is the
    /// machine this actually has to run on. Three things keep it small without
    /// weakening the measurements:
    ///
    /// - The disk benchmark reads unbuffered, so files do **not** need to
    ///   exceed RAM to defeat the page cache. 512 MiB is plenty.
    /// - Only the `binary` large file is ever measured; the other three
    ///   flavours exist for the compression tests, which generate their own
    ///   data in memory. See [`Corpus::generate_large`].
    /// - Creating files is a metadata operation, and on a spinning USB drive
    ///   100,000 of them takes many minutes. 20,000 measures directory
    ///   enumeration just as well.
    pub fn full() -> Self {
        Self {
            large_file_bytes: 512 * 1024 * 1024,
            small_file_count: 10_000,
            small_file_bytes: 20 * 1024,
            wide_entry_count: 20_000,
            seed: 0xBA5A17,
        }
    }

    /// Small corpus for smoke-testing the harness itself.
    pub fn quick() -> Self {
        Self {
            large_file_bytes: 256 * 1024 * 1024,
            small_file_count: 2_000,
            small_file_bytes: 20 * 1024,
            wide_entry_count: 10_000,
            seed: 0xBA5A17,
        }
    }

    /// Rough total on disk. Only the binary large file is full size; see
    /// [`Corpus::generate_large`].
    pub fn total_bytes(&self) -> u64 {
        let companions = 3 * (32 * 1024 * 1024).min(self.large_file_bytes);
        self.large_file_bytes
            + companions
            + (self.small_file_count * self.small_file_bytes) as u64
            + (self.wide_entry_count * 64) as u64
    }
}

impl Corpus {
    pub fn large_dir(&self) -> PathBuf {
        self.root.join("large")
    }
    pub fn small_dir(&self) -> PathBuf {
        self.root.join("small")
    }
    pub fn wide_dir(&self) -> PathBuf {
        self.root.join("wide")
    }
    pub fn large_file(&self, flavour: Flavour) -> PathBuf {
        self.large_dir()
            .join(format!("{}.{}", flavour.name(), flavour.extension()))
    }

    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// True if a previously generated corpus looks complete.
    pub fn is_generated(&self) -> bool {
        self.root.join(".basalt-corpus").is_file()
    }

    /// Relative paths of every small file, in creation order.
    pub fn small_file_paths(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let dir = self.small_dir();
        for shard in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            let shard_name = shard.file_name().to_string_lossy().into_owned();
            for f in fs::read_dir(shard.path())? {
                let f = f?;
                if f.file_type()?.is_file() {
                    out.push(format!(
                        "small/{shard_name}/{}",
                        f.file_name().to_string_lossy()
                    ));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Generates the corpus, skipping any part that already exists.
    pub fn generate(&self, spec: CorpusSpec, force: bool) -> Result<()> {
        if self.is_generated() && !force {
            println!(
                "corpus already present at {} (use --force to regenerate)",
                self.root.display()
            );
            return Ok(());
        }

        println!(
            "generating corpus at {} (~{})",
            self.root.display(),
            fmt_bytes(spec.total_bytes())
        );

        fs::create_dir_all(self.large_dir())?;
        fs::create_dir_all(self.small_dir())?;
        fs::create_dir_all(self.wide_dir())?;

        self.generate_large(spec)?;
        self.generate_small(spec)?;
        self.generate_wide(spec)?;

        fs::write(
            self.root.join(".basalt-corpus"),
            format!(
                "seed={}\nlarge_bytes={}\nsmall_count={}\nwide_count={}\n",
                spec.seed, spec.large_file_bytes, spec.small_file_count, spec.wide_entry_count
            ),
        )?;
        println!("corpus ready");
        Ok(())
    }

    fn generate_large(&self, spec: CorpusSpec) -> Result<()> {
        const CHUNK: usize = 8 * 1024 * 1024;

        // Only the binary flavour is actually measured — the disk sequential
        // read and the SMB large-file baseline both use it, chosen because it
        // is incompressible and so cannot be flattered by SMB compression. The
        // other three exist only so the tree looks realistic and so the
        // buffered/unbuffered agreement test has something to read.
        //
        // Generating them at full size would triple the wait for no gain, and
        // they are the *slow* ones: prose, code and json are built by string
        // formatting, while binary is a PRNG fill. On a weak laptop CPU that
        // difference is minutes, not seconds.
        const COMPANION_BYTES: u64 = 32 * 1024 * 1024;

        for (i, flavour) in Flavour::all().into_iter().enumerate() {
            let target = if flavour == Flavour::Binary {
                spec.large_file_bytes
            } else {
                COMPANION_BYTES.min(spec.large_file_bytes)
            };

            let path = self.large_file(flavour);
            if path.is_file() && path.metadata()?.len() >= target {
                continue;
            }
            print!("  large/{:<8} {:>9} ", flavour.name(), fmt_bytes(target));
            std::io::stdout().flush().ok();

            let mut rng = Rng::new(spec.seed ^ (i as u64) << 32);
            let mut file =
                File::create(&path).with_context(|| format!("creating {}", path.display()))?;
            let mut buf = Vec::with_capacity(CHUNK + 4096);
            let mut written = 0u64;
            while written < target {
                generate(flavour, CHUNK, &mut rng, &mut buf);
                let remaining = (target - written) as usize;
                let take = buf.len().min(remaining);
                file.write_all(&buf[..take])?;
                written += take as u64;
            }
            file.sync_all()?;
            println!("done");
        }
        Ok(())
    }

    fn generate_small(&self, spec: CorpusSpec) -> Result<()> {
        // Shard across subdirectories: a single directory with 10,000 entries
        // behaves very differently from a realistic tree, and we measure the
        // pathological single-directory case separately in `wide/`.
        const PER_SHARD: usize = 250;

        print!("  small/   {} files ", spec.small_file_count);
        std::io::stdout().flush().ok();

        let mut rng = Rng::new(spec.seed ^ SALT_SMALL);
        let mut buf = Vec::new();
        for i in 0..spec.small_file_count {
            let shard = i / PER_SHARD;
            let shard_dir = self.small_dir().join(format!("shard{shard:03}"));
            if i % PER_SHARD == 0 {
                fs::create_dir_all(&shard_dir)?;
            }
            let flavour = Flavour::all()[i % 4];
            // Vary sizes so the workload is not artificially uniform.
            let size = rng.range(spec.small_file_bytes / 2, spec.small_file_bytes * 2);
            generate(flavour, size, &mut rng, &mut buf);
            let path = shard_dir.join(format!("{i:05}.{}", flavour.extension()));
            fs::write(&path, &buf).with_context(|| format!("writing {}", path.display()))?;
        }
        println!("done");
        Ok(())
    }

    fn generate_wide(&self, spec: CorpusSpec) -> Result<()> {
        print!("  wide/    {} entries ", spec.wide_entry_count);
        std::io::stdout().flush().ok();

        let dir = self.wide_dir();
        let mut rng = Rng::new(spec.seed ^ SALT_WIDE);
        for i in 0..spec.wide_entry_count {
            let path = dir.join(format!("entry_{i:06}.dat"));
            if path.is_file() {
                continue;
            }
            // Tiny bodies: this corpus exists to measure directory enumeration
            // and per-file overhead, not bandwidth.
            let mut body = [0u8; 64];
            rng.fill(&mut body);
            let mut f = BufWriter::new(File::create(&path)?);
            f.write_all(&body)?;
        }
        println!("done");
        Ok(())
    }
}

/// Resolves the corpus root, defaulting next to the workspace.
pub fn default_root() -> PathBuf {
    Path::new("bench-corpus").to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio(flavour: Flavour, size: usize) -> f64 {
        let mut rng = Rng::new(42);
        let mut buf = Vec::new();
        generate(flavour, size, &mut rng, &mut buf);
        let compressed = zstd::encode_all(buf.as_slice(), 1).unwrap();
        buf.len() as f64 / compressed.len() as f64
    }

    #[test]
    fn rng_is_deterministic_for_a_given_seed() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn rng_does_not_collapse_to_zero() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn rng_fill_handles_non_multiple_of_eight() {
        let mut rng = Rng::new(1);
        let mut buf = [0u8; 13];
        rng.fill(&mut buf);
        assert!(
            buf.iter().any(|&b| b != 0),
            "fill must write the tail bytes"
        );
    }

    #[test]
    fn rng_below_zero_does_not_panic() {
        assert_eq!(Rng::new(1).below(0), 0);
    }

    #[test]
    fn generated_text_flavours_compress_like_real_text() {
        // The point of the corpus: if these ratios are wrong, every compression
        // conclusion drawn in Phase 0 is wrong too.
        let prose = ratio(Flavour::Prose, 512 * 1024);
        assert!(
            (1.8..6.0).contains(&prose),
            "prose should compress ~2-4x, got {prose:.2}x"
        );

        let code = ratio(Flavour::Code, 512 * 1024);
        assert!(
            (2.0..12.0).contains(&code),
            "code should compress ~3-6x, got {code:.2}x"
        );

        let json = ratio(Flavour::Json, 512 * 1024);
        assert!(json > 2.5, "json should compress well, got {json:.2}x");
    }

    #[test]
    fn generated_binary_is_incompressible() {
        let bin = ratio(Flavour::Binary, 512 * 1024);
        assert!(
            bin < 1.05,
            "binary must not compress, got {bin:.2}x — the corpus would \
             flatter compression and invalidate the benchmark"
        );
    }

    #[test]
    fn generated_sizes_land_near_the_target() {
        let mut rng = Rng::new(3);
        let mut buf = Vec::new();
        for flavour in Flavour::all() {
            generate(flavour, 100_000, &mut rng, &mut buf);
            assert!(
                buf.len() >= 100_000 && buf.len() < 100_000 + 4096,
                "{}: expected ~100000 bytes, got {}",
                flavour.name(),
                buf.len()
            );
        }
    }

    #[test]
    fn generate_clears_previous_contents() {
        let mut rng = Rng::new(3);
        let mut buf = vec![0xAAu8; 500_000];
        generate(Flavour::Prose, 1000, &mut rng, &mut buf);
        assert!(buf.len() < 5000, "buffer must be reset, not appended to");
    }
}
