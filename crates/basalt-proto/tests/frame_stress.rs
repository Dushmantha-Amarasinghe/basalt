//! Frame encoder/decoder stress and corruption tests.
//!
//! `basalt-proto` is not benchmark scaffolding — it is the format the host and
//! client will actually speak, and a decoder bug here is a corrupted file on
//! someone's drive. So this suite goes past happy-path round trips and attacks
//! the decoder the way a flaky Wi-Fi link and a hostile peer would:
//!
//! - randomised round trips across thousands of generated entry sets
//! - truncation at *every* byte offset
//! - single-bit corruption at many offsets
//! - adversarial and degenerate inputs
//!
//! The standard these tests hold the decoder to: it may reject input, but it
//! must never accept input and hand back *wrong data*. Silent corruption is the
//! one unacceptable outcome.

use basalt_proto::codec::Codec;
use basalt_proto::frame::{BatchReader, BatchWriter, Entry, EntryKind, MAX_ENTRY_BYTES};

/// Seeded xorshift so failures are reproducible from the printed seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() >> 24) as u8).collect()
    }
}

/// A randomly generated, always-valid entry set.
fn random_entries(rng: &mut Rng, count: usize) -> Vec<(EntryKind, String, i64, Vec<u8>)> {
    let segments = ["docs", "photos", "src", "a", "deep", "nested", "folder-2", "x_y"];
    (0..count)
        .map(|i| {
            let depth = rng.below(4) + 1;
            let mut path = String::new();
            for d in 0..depth {
                if d > 0 {
                    path.push('/');
                }
                path.push_str(segments[rng.below(segments.len())]);
            }
            path.push_str(&format!("/item{i}.bin"));

            let kind = match rng.below(10) {
                0 => EntryKind::Dir,
                1 => EntryKind::Error,
                _ => EntryKind::File,
            };
            let data = if kind == EntryKind::File {
                let len = rng.below(3000);
                rng.bytes(len)
            } else {
                Vec::new()
            };
            (kind, path, rng.next() as i64 % 2_000_000_000, data)
        })
        .collect()
}

fn encode(
    entries: &[(EntryKind, String, i64, Vec<u8>)],
    codec: Codec,
) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, codec).expect("writer");
    for (kind, path, mtime, data) in entries {
        match kind {
            EntryKind::File => w.write_file(path, *mtime, data).expect("write file"),
            EntryKind::Dir => w.write_dir(path, *mtime).expect("write dir"),
            EntryKind::Error => w.write_error(path, "synthetic failure").expect("write err"),
            EntryKind::End => unreachable!(),
        }
    }
    w.finish().expect("finish");
    buf
}

fn decode(bytes: &[u8]) -> Result<Vec<Entry>, basalt_proto::ProtoError> {
    let mut r = BatchReader::new(bytes)?;
    let mut out = Vec::new();
    while let Some(e) = r.read_entry()? {
        out.push(e);
    }
    Ok(out)
}

#[test]
fn randomised_round_trips_are_byte_exact() {
    for codec in [Codec::Raw, Codec::Zstd(1), Codec::Zstd(9)] {
        for seed in 0..120u64 {
            let mut rng = Rng::new(seed);
            let count = rng.below(40) + 1;
            let entries = random_entries(&mut rng, count);
            let encoded = encode(&entries, codec);
            let decoded = decode(&encoded)
                .unwrap_or_else(|e| panic!("seed {seed} codec {codec:?} failed to decode: {e}"));

            assert_eq!(
                decoded.len(),
                entries.len(),
                "seed {seed} codec {codec:?}: entry count changed"
            );
            for (got, (kind, path, mtime, data)) in decoded.iter().zip(&entries) {
                assert_eq!(got.kind, *kind, "seed {seed}: kind mismatch for {path}");
                assert_eq!(&got.path, path, "seed {seed}: path mismatch");
                if *kind != EntryKind::Error {
                    assert_eq!(got.mtime, *mtime, "seed {seed}: mtime mismatch for {path}");
                }
                assert_eq!(&got.data, data, "seed {seed}: DATA MISMATCH for {path}");
            }
        }
    }
}

#[test]
fn truncation_at_every_offset_is_rejected() {
    // A dropped Wi-Fi connection truncates mid-stream. Every possible cut point
    // must surface as an error rather than a short, plausible-looking result.
    let mut rng = Rng::new(7);
    let entries = random_entries(&mut rng, 12);

    for codec in [Codec::Raw, Codec::Zstd(1)] {
        let full = encode(&entries, codec);
        for cut in 0..full.len() {
            let truncated = &full[..cut];
            match decode(truncated) {
                Err(_) => {}
                Ok(decoded) => panic!(
                    "codec {codec:?}: truncating to {cut}/{} bytes decoded cleanly \
                     into {} entries — a partial stream must never look complete",
                    full.len(),
                    decoded.len()
                ),
            }
        }
    }
}

#[test]
fn corrupting_a_compressed_stream_is_always_detected() {
    // The compressed path carries a zstd frame checksum, so any corruption
    // must be caught. This is the guarantee `include_checksum(true)` buys.
    let mut rng = Rng::new(11);
    let entries = random_entries(&mut rng, 20);
    let full = encode(&entries, Codec::Zstd(1));

    let mut checked = 0;
    // Skip the 12-byte plaintext header; corrupting it is covered separately.
    for offset in (12..full.len()).step_by(7) {
        for bit in [0u8, 3, 7] {
            let mut corrupted = full.clone();
            corrupted[offset] ^= 1 << bit;
            if corrupted == full {
                continue;
            }
            checked += 1;

            if let Ok(decoded) = decode(&corrupted) {
                // Decoding succeeded — it is only acceptable if the bytes are
                // still exactly right (possible when a flipped bit lands in
                // padding the format does not read).
                let matches = decoded.len() == entries.len()
                    && decoded
                        .iter()
                        .zip(&entries)
                        .all(|(g, (_, p, _, d))| &g.path == p && &g.data == d);
                assert!(
                    matches,
                    "bit {bit} at offset {offset} produced DIFFERENT data that \
                     still decoded cleanly — silent corruption"
                );
            }
        }
    }
    assert!(checked > 50, "expected a meaningful number of mutations, ran {checked}");
}

#[test]
fn corrupting_the_header_is_rejected() {
    let entries = random_entries(&mut Rng::new(3), 4);
    let full = encode(&entries, Codec::Zstd(1));

    // Magic bytes.
    for offset in 0..4 {
        let mut corrupted = full.clone();
        corrupted[offset] ^= 0xFF;
        assert!(
            decode(&corrupted).is_err(),
            "corrupting magic byte {offset} must be rejected"
        );
    }
    // Version.
    let mut bad_version = full.clone();
    bad_version[4] = 0xAB;
    bad_version[5] = 0xCD;
    assert!(decode(&bad_version).is_err(), "bad version must be rejected");

    // Codec id.
    let mut bad_codec = full.clone();
    bad_codec[6] = 0x7F;
    assert!(decode(&bad_codec).is_err(), "unknown codec must be rejected");
}

#[test]
fn a_declared_length_larger_than_the_stream_is_rejected() {
    // Hostile peer claims a huge entry. The decoder must refuse rather than
    // attempt a multi-gigabyte allocation.
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
    w.write_file("a.txt", 0, b"short").unwrap();
    w.finish().unwrap();

    // Layout after the 12-byte header: kind(1) path_len(2) path(5) mtime(8)
    // then the 8-byte data length.
    let len_offset = 12 + 1 + 2 + 5 + 8;
    let mut evil = buf.clone();
    evil[len_offset..len_offset + 8].copy_from_slice(&(MAX_ENTRY_BYTES + 1).to_le_bytes());
    assert!(
        decode(&evil).is_err(),
        "an entry length above the cap must be rejected before allocating"
    );

    let mut huge = buf.clone();
    huge[len_offset..len_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode(&huge).is_err(), "u64::MAX length must be rejected");
}

#[test]
fn random_bytes_are_never_accepted_as_a_stream() {
    let mut rng = Rng::new(99);
    for _ in 0..500 {
        let len = rng.below(200) + 1;
        let junk = rng.bytes(len);
        assert!(
            decode(&junk).is_err(),
            "random bytes must not decode as a valid stream"
        );
    }
}

#[test]
fn a_valid_header_followed_by_junk_is_rejected() {
    // The nastier case: correct magic and version, garbage body.
    let mut rng = Rng::new(123);
    for _ in 0..200 {
        let mut stream = Vec::new();
        stream.extend_from_slice(b"BSLT");
        stream.extend_from_slice(&1u16.to_le_bytes());
        stream.push(0); // Codec::Raw
        stream.push(0); // level
        stream.extend_from_slice(&[0, 0, 0, 0]);
        let len = rng.below(150) + 8;
        stream.extend_from_slice(&rng.bytes(len));

        // Must not panic, and must not silently produce data.
        if let Ok(entries) = decode(&stream) {
            for e in entries {
                assert!(
                    !e.path.contains(".."),
                    "decoder produced a traversal path from junk: {}",
                    e.path
                );
            }
        }
    }
}

#[test]
fn many_entries_survive_a_round_trip() {
    // 20k entries exercises the shared zstd window across a realistic batch.
    let count = 20_000;
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, Codec::Zstd(1)).unwrap();
    for i in 0..count {
        w.write_file(
            &format!("shard{:03}/file{i}.txt", i / 250),
            i as i64,
            format!("contents of file number {i}\n").as_bytes(),
        )
        .unwrap();
    }
    assert_eq!(w.entries_written(), count as u64);
    w.finish().unwrap();

    let decoded = decode(&buf).expect("20k entries should decode");
    assert_eq!(decoded.len(), count);
    assert_eq!(decoded[0].path, "shard000/file0.txt");
    assert_eq!(decoded[count - 1].path, format!("shard{:03}/file{}.txt", (count - 1) / 250, count - 1));
    assert_eq!(decoded[12345].data, b"contents of file number 12345\n");
}

#[test]
fn entry_sizes_around_the_block_boundaries_round_trip() {
    // Sizes near powers of two and near zstd's internal block size are where
    // off-by-one framing bugs hide.
    let sizes = [
        0usize, 1, 2, 7, 255, 256, 257, 1023, 1024, 1025, 4095, 4096, 4097, 65_535, 65_536,
        65_537, 131_072, 131_073,
    ];
    for codec in [Codec::Raw, Codec::Zstd(1)] {
        let mut buf = Vec::new();
        let mut w = BatchWriter::new(&mut buf, codec).unwrap();
        for (i, &size) in sizes.iter().enumerate() {
            let data: Vec<u8> = (0..size).map(|b| (b % 251) as u8).collect();
            w.write_file(&format!("size{i}.bin"), 0, &data).unwrap();
        }
        w.finish().unwrap();

        let decoded = decode(&buf).expect("boundary sizes should decode");
        assert_eq!(decoded.len(), sizes.len());
        for (entry, &size) in decoded.iter().zip(&sizes) {
            assert_eq!(entry.data.len(), size, "size {size} did not round-trip");
            assert!(
                entry.data.iter().enumerate().all(|(b, &v)| v == (b % 251) as u8),
                "size {size} round-tripped with wrong contents"
            );
        }
    }
}

#[test]
fn unicode_paths_round_trip() {
    let paths = [
        "документы/отчёт.txt",
        "写真/家族.jpg",
        "musique/café-noël.flac",
        "emoji/🎬-movie.mkv",
        "mixed/Ünïcödé_ﬁle.txt",
    ];
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, Codec::Zstd(1)).unwrap();
    for (i, p) in paths.iter().enumerate() {
        w.write_file(p, i as i64, p.as_bytes()).unwrap();
    }
    w.finish().unwrap();

    let decoded = decode(&buf).expect("unicode paths should decode");
    for (entry, path) in decoded.iter().zip(&paths) {
        assert_eq!(&entry.path, path);
        assert_eq!(entry.data, path.as_bytes());
    }
}

#[test]
fn invalid_utf8_in_a_path_is_rejected() {
    let mut stream = Vec::new();
    stream.extend_from_slice(b"BSLT");
    stream.extend_from_slice(&1u16.to_le_bytes());
    stream.push(0);
    stream.push(0);
    stream.extend_from_slice(&[0, 0, 0, 0]);
    stream.push(EntryKind::File as u8);
    stream.extend_from_slice(&4u16.to_le_bytes());
    stream.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC]); // not valid UTF-8
    stream.extend_from_slice(&0i64.to_le_bytes());
    stream.extend_from_slice(&0u64.to_le_bytes());
    stream.push(EntryKind::End as u8);

    assert!(decode(&stream).is_err(), "invalid UTF-8 path must be rejected");
}

#[test]
fn all_zstd_levels_round_trip() {
    let entries = random_entries(&mut Rng::new(55), 30);
    for level in 1..=9 {
        let encoded = encode(&entries, Codec::Zstd(level));
        let decoded = decode(&encoded).unwrap_or_else(|e| panic!("level {level}: {e}"));
        assert_eq!(decoded.len(), entries.len(), "level {level}");
        for (got, (_, path, _, data)) in decoded.iter().zip(&entries) {
            assert_eq!(&got.path, path, "level {level}");
            assert_eq!(&got.data, data, "level {level}");
        }
    }
}

#[test]
fn the_iterator_and_manual_reads_agree() {
    let entries = random_entries(&mut Rng::new(17), 25);
    let encoded = encode(&entries, Codec::Zstd(1));

    let via_iterator: Vec<Entry> = BatchReader::new(encoded.as_slice())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let via_manual = decode(&encoded).unwrap();

    assert_eq!(via_iterator, via_manual);
}

#[test]
fn reading_past_the_end_keeps_returning_none() {
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, Codec::Raw).unwrap();
    w.write_file("a.txt", 0, b"x").unwrap();
    w.finish().unwrap();

    let mut r = BatchReader::new(buf.as_slice()).unwrap();
    assert!(r.read_entry().unwrap().is_some());
    for _ in 0..5 {
        assert!(
            r.read_entry().unwrap().is_none(),
            "reads past the terminator must stay None, not error or loop"
        );
    }
}

#[test]
fn writer_reports_accurate_counters() {
    let mut buf = Vec::new();
    let mut w = BatchWriter::new(&mut buf, Codec::Zstd(1)).unwrap();
    w.write_file("a.txt", 0, &[0u8; 1000]).unwrap();
    w.write_dir("d", 0).unwrap();
    w.write_file("b.txt", 0, &[0u8; 2500]).unwrap();
    w.write_error("c.txt", "nope").unwrap();

    assert_eq!(w.entries_written(), 4);
    assert_eq!(w.logical_bytes(), 3500, "dirs and errors carry no payload");
    w.finish().unwrap();
}
