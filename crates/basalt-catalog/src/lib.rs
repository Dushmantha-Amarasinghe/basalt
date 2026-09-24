//! Which films and TV series actually exist.
//!
//! The parser can say "this path is shaped like a film". It cannot say "this
//! is a film". Run over a real drive, shape alone filed lecture captures and
//! meeting recordings as cinema: a recording in a folder with a year in its
//! name has a title and a year like anything else. What separates *Arrival*
//! from *Literature Review* is that one of them was released, and that is a
//! fact about the world rather than about the path.
//!
//! So this is every film and series title Wikidata knows, shipped inside the
//! host. Offline on purpose:
//!
//! - **No network.** A host on a LAN with no internet still identifies films.
//! - **Nothing leaves the machine.** Asking a service about every title on a
//!   drive tells that service what is on the drive.
//! - **No key and no account,** because zero configuration is the product.
//!
//! The data is Wikidata's, which is CC0, rebuilt by `tools/catalog-build` for
//! each release. It is only as fresh as that build, and
//! [`Catalog::snapshot_year`] is what lets the host make allowances for films
//! released after it.
//!
//! **Titles are stored as hashes, not strings.** A 44-bit hash of the kind and
//! the normalised title gives exact membership in a few megabytes instead of
//! twenty, and the chance that an unreleased title collides with a released
//! one is about one in twenty-five million per lookup.

use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Bits of each title hash that are kept.
///
/// Enough that a collision is not worth thinking about at this size, few
/// enough that the gaps between sorted hashes fit a four-byte varint.
const KEY_BITS: u32 = 44;

const MAGIC: &[u8; 4] = b"BSCT";
const VERSION: u8 = 1;
/// Magic, version, snapshot date, entry count.
const HEADER_BYTES: usize = 4 + 1 + 4 + 4;

/// The earliest year a byte can hold. Anything older is stored as this, which
/// only ever widens a match — the first films are from the 1880s.
const YEAR_BASE: u16 = 1870;

/// The catalogue that ships with the host. Empty in a checkout that has never
/// run the builder, which [`Catalog::bundled`] reports as no catalogue at all.
static BUNDLED: &[u8] = include_bytes!("../data/catalog.bin");

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("not a catalogue file")]
    NotACatalogue,
    #[error("catalogue version {0} is not one this build reads")]
    Version(u8),
    #[error("the catalogue is damaged: {0}")]
    Damaged(&'static str),
    #[error("the catalogue did not decompress: {0}")]
    Decompress(#[from] std::io::Error),
}

/// Whether a title names a film or a series.
///
/// Kept apart deliberately: *Fargo* is both, and a file that parses as an
/// episode must not be verified by the film of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Film,
    Series,
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Kind::Film => b'f',
            Kind::Series => b's',
        }
    }
}

/// What a lookup found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// Released under this title, in a year compatible with the one asked.
    Found,
    /// The title exists, but not from anywhere near that year. *Home* was
    /// released several times, none of them in the year a home video was shot.
    WrongYear,
    /// Nothing of this kind was ever released under this title.
    Missing,
}

/// Reduces a title to what is worth comparing.
///
/// Release names and catalogue entries disagree about everything that carries
/// no meaning: case, punctuation, spacing, accents, and `&` against `and`.
/// `Spider-Man: No Way Home` and `Spider.Man.No.Way.Home` must meet, and so
/// must *Amélie* and a file somebody typed as `Amelie`.
///
/// Compatibility decomposition (NFKD) rather than canonical, so full-width
/// letters, ligatures and superscripts fold to the plain letters people type.
/// Letters from any script survive — a Japanese title is compared as Japanese —
/// because stripping them would leave two unrelated titles both empty.
pub fn normalise(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        if c == '&' {
            out.push_str("and");
            continue;
        }
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// The spellings of one title worth trying.
///
/// A leading "The" is the one difference normalising cannot see through:
/// releases drop it (`Lord of the Rings The Two Towers`) and occasionally add
/// it, and either way the letters no longer line up. Nothing subtler is tried
/// — not "A" and "An", not a sequel written `2` against `II` — because every
/// extra spelling is another chance for a home video to match something.
pub fn spellings(title: &str) -> Vec<String> {
    const THE: &str = "the ";
    let base = title.trim();
    // Compared byte-wise and ASCII-only, so the slice below can never land
    // inside a character whatever the rest of the title is written in.
    let has_article = base.len() > THE.len()
        && base
            .get(..THE.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(THE));
    let other = if has_article {
        base[THE.len()..].trim_start().to_string()
    } else {
        format!("The {base}")
    };
    vec![base.to_string(), other]
}

/// A hash of one title of one kind.
fn key(kind: Kind, normalised: &str) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[kind.tag(), 0]);
    hasher.update(normalised.as_bytes());
    let digest = hasher.finalize();
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(head) >> (64 - KEY_BITS)
}

/// Whether a year read off a path fits a release window.
///
/// A year either side is allowed: a film shown at festivals one year is
/// released the next, and releases are named after whichever year the group
/// saw. An unknown year on either side is not a disagreement, just less to go
/// on.
fn compatible(year: Option<u16>, first: u16, last: u16) -> bool {
    match year {
        None => true,
        Some(_) if first == 0 => true,
        Some(y) => y + 1 >= first && y <= last.saturating_add(1),
    }
}

/// Every title, as sorted hashes with the years each was released.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    keys: Vec<u64>,
    /// First year of release, 0 when unknown.
    first: Vec<u16>,
    /// Last year of release; equal to `first` for almost everything.
    last: Vec<u16>,
    /// When the data was taken, as `yyyymmdd`.
    snapshot: u32,
}

impl Catalog {
    /// The catalogue built into this binary, or `None` if it has none.
    ///
    /// Decoded once, on first use: the host only needs it when it scans, and
    /// a host that is never asked to recognise films never pays for it.
    pub fn bundled() -> Option<&'static Catalog> {
        static CELL: OnceLock<Option<Catalog>> = OnceLock::new();
        CELL.get_or_init(|| match Catalog::decode(BUNDLED) {
            Ok(catalog) if !catalog.is_empty() => Some(catalog),
            _ => None,
        })
        .as_ref()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// When the data was taken, as `yyyymmdd`.
    pub fn snapshot(&self) -> u32 {
        self.snapshot
    }

    /// The year the data was taken. A film from this year or later may simply
    /// be newer than the catalogue.
    pub fn snapshot_year(&self) -> u16 {
        (self.snapshot / 10_000) as u16
    }

    /// Looks one title up, exactly as spelled once normalised.
    pub fn lookup(&self, kind: Kind, title: &str, year: Option<u16>) -> Lookup {
        let normalised = normalise(title);
        if normalised.is_empty() {
            return Lookup::Missing;
        }
        let key = key(kind, &normalised);
        let start = self.keys.partition_point(|&k| k < key);

        let mut seen = false;
        for at in start..self.keys.len() {
            if self.keys[at] != key {
                break;
            }
            seen = true;
            if compatible(year, self.first[at], self.last[at]) {
                return Lookup::Found;
            }
        }
        if seen {
            Lookup::WrongYear
        } else {
            Lookup::Missing
        }
    }

    /// Looks a title up under each of its [`spellings`], best answer first.
    pub fn find(&self, kind: Kind, title: &str, year: Option<u16>) -> Lookup {
        let mut best = Lookup::Missing;
        for spelling in spellings(title) {
            match self.lookup(kind, &spelling, year) {
                Lookup::Found => return Lookup::Found,
                Lookup::WrongYear => best = Lookup::WrongYear,
                Lookup::Missing => {}
            }
        }
        best
    }

    /// Writes the catalogue in the form [`Catalog::decode`] reads.
    ///
    /// Columns rather than rows — every hash gap, then every first year, then
    /// every span — because like values sit together and compress far better
    /// that way.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.keys.len() * 7);
        let mut previous = 0u64;
        for &key in &self.keys {
            write_varint(&mut body, key - previous);
            previous = key;
        }
        for &first in &self.first {
            body.push(year_byte(first));
        }
        for (&first, &last) in self.first.iter().zip(&self.last) {
            body.push(last.saturating_sub(first).min(255) as u8);
        }

        let compressed = zstd::encode_all(&body[..], 19).expect("zstd into memory cannot fail");
        let mut out = Vec::with_capacity(HEADER_BYTES + compressed.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.snapshot.to_le_bytes());
        out.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    /// Reads what [`Catalog::encode`] wrote, checking everything it can.
    ///
    /// An empty input is an empty catalogue rather than an error: that is what
    /// a checkout carries before the builder has ever been run.
    pub fn decode(bytes: &[u8]) -> Result<Catalog, CatalogError> {
        if bytes.is_empty() {
            return Ok(Catalog::default());
        }
        if bytes.len() < HEADER_BYTES || &bytes[..4] != MAGIC {
            return Err(CatalogError::NotACatalogue);
        }
        if bytes[4] != VERSION {
            return Err(CatalogError::Version(bytes[4]));
        }
        let snapshot = u32::from_le_bytes(bytes[5..9].try_into().expect("four bytes"));
        let count = u32::from_le_bytes(bytes[9..13].try_into().expect("four bytes")) as usize;

        let body = zstd::decode_all(&bytes[HEADER_BYTES..])?;
        let mut at = 0usize;
        let mut keys = Vec::with_capacity(count);
        let mut previous = 0u64;
        for _ in 0..count {
            let (gap, used) =
                read_varint(&body[at..]).ok_or(CatalogError::Damaged("a hash ran off the end"))?;
            at += used;
            let key = previous
                .checked_add(gap)
                .ok_or(CatalogError::Damaged("hashes overflowed"))?;
            keys.push(key);
            previous = key;
        }
        if body.len() != at + 2 * count {
            return Err(CatalogError::Damaged(
                "the year columns are the wrong length",
            ));
        }
        let first: Vec<u16> = body[at..at + count].iter().map(|&b| byte_year(b)).collect();
        let last: Vec<u16> = body[at + count..]
            .iter()
            .zip(&first)
            .map(|(&span, &first)| if first == 0 { 0 } else { first + span as u16 })
            .collect();

        Ok(Catalog {
            keys,
            first,
            last,
            snapshot,
        })
    }
}

/// Collects titles and turns them into a [`Catalog`].
#[derive(Debug, Default)]
pub struct Builder {
    entries: Vec<(u64, u16, u16)>,
    snapshot: u32,
}

impl Builder {
    /// `snapshot` is the date the data was taken, as `yyyymmdd`.
    pub fn new(snapshot: u32) -> Self {
        Builder {
            entries: Vec::new(),
            snapshot,
        }
    }

    /// Adds one title. `years` is the first and last year of release, when
    /// known; a title with no normalisable letters at all is skipped.
    pub fn add(&mut self, kind: Kind, title: &str, years: Option<(u16, u16)>) {
        let normalised = normalise(title);
        if normalised.is_empty() {
            return;
        }
        let (first, last) = match years {
            Some((a, b)) => (a.min(b).max(YEAR_BASE), a.max(b).max(YEAR_BASE)),
            None => (0, 0),
        };
        self.entries.push((key(kind, &normalised), first, last));
    }

    pub fn finish(mut self) -> Catalog {
        self.entries.sort_unstable();
        self.entries.dedup();
        let mut catalog = Catalog {
            keys: Vec::with_capacity(self.entries.len()),
            first: Vec::with_capacity(self.entries.len()),
            last: Vec::with_capacity(self.entries.len()),
            snapshot: self.snapshot,
        };
        for (key, first, last) in self.entries {
            catalog.keys.push(key);
            catalog.first.push(first);
            // Stored as a span of at most 255 years, so clamp here rather than
            // let the round trip quietly disagree with what was built.
            catalog
                .last
                .push(if first == 0 { 0 } else { last.min(first + 255) });
        }
        catalog
    }
}

fn year_byte(year: u16) -> u8 {
    if year == 0 {
        0
    } else {
        (year.saturating_sub(YEAR_BASE) + 1).min(255) as u8
    }
}

fn byte_year(byte: u8) -> u16 {
    if byte == 0 {
        0
    } else {
        YEAR_BASE + byte as u16 - 1
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let low = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

fn read_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, &byte) in bytes.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Catalog {
        let mut builder = Builder::new(20260920);
        builder.add(Kind::Film, "Arrival", Some((2016, 2016)));
        builder.add(Kind::Film, "Blade Runner 2049", Some((2017, 2017)));
        builder.add(Kind::Film, "Home", Some((2009, 2009)));
        builder.add(Kind::Film, "Home", Some((2015, 2015)));
        builder.add(Kind::Film, "Amélie", Some((2001, 2001)));
        builder.add(Kind::Film, "Spider-Man: No Way Home", Some((2021, 2021)));
        builder.add(
            Kind::Film,
            "The Lord of the Rings: The Two Towers",
            Some((2002, 2002)),
        );
        builder.add(Kind::Film, "Fast & Furious", Some((2009, 2009)));
        builder.add(Kind::Film, "Nosferatu", Some((1922, 1922)));
        builder.add(Kind::Film, "Nosferatu", Some((2024, 2024)));
        builder.add(Kind::Film, "A Film With No Date", None);
        builder.add(Kind::Series, "Breaking Bad", Some((2008, 2013)));
        builder.add(Kind::Series, "Fargo", Some((2014, 2024)));
        builder.add(Kind::Film, "Fargo", Some((1996, 1996)));
        builder.finish()
    }

    #[test]
    fn a_released_title_is_found_through_punctuation_case_and_accents() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Film, "arrival", Some(2016)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "Spider.Man.No.Way.Home", Some(2021)),
            Lookup::Found
        );
        assert_eq!(c.lookup(Kind::Film, "Amelie", None), Lookup::Found);
        assert_eq!(c.lookup(Kind::Film, "AMÉLIE", Some(2001)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "Fast and Furious", Some(2009)),
            Lookup::Found
        );
    }

    #[test]
    fn a_recording_is_not_found() {
        let c = sample();
        for junk in [
            "Literature Review-20251014 100532-Meeting Recording",
            "Physical Sep",
            "video 7",
            "Day 23",
            "MOV 1308",
        ] {
            assert_eq!(
                c.lookup(Kind::Film, junk, Some(2025)),
                Lookup::Missing,
                "{junk}"
            );
        }
    }

    #[test]
    fn a_year_either_side_still_counts() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Film, "Arrival", Some(2015)), Lookup::Found);
        assert_eq!(c.lookup(Kind::Film, "Arrival", Some(2017)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "Arrival", Some(2019)),
            Lookup::WrongYear
        );
    }

    /// Several films share a title. The year picks between them, and a year
    /// that matches none of them is not rescued by the others' existence.
    #[test]
    fn a_shared_title_is_judged_by_each_release_separately() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Film, "Home", Some(2015)), Lookup::Found);
        assert_eq!(c.lookup(Kind::Film, "Home", Some(2009)), Lookup::Found);
        assert_eq!(c.lookup(Kind::Film, "Home", Some(2012)), Lookup::WrongYear);
        assert_eq!(c.lookup(Kind::Film, "Nosferatu", Some(1922)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "Nosferatu", Some(1970)),
            Lookup::WrongYear
        );
    }

    #[test]
    fn no_year_on_either_side_is_not_a_disagreement() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Film, "Home", None), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "A Film With No Date", Some(1950)),
            Lookup::Found
        );
    }

    #[test]
    fn a_series_spans_every_year_it_ran() {
        let c = sample();
        assert_eq!(
            c.lookup(Kind::Series, "Breaking Bad", Some(2008)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(Kind::Series, "Breaking Bad", Some(2011)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(Kind::Series, "Breaking Bad", Some(2020)),
            Lookup::WrongYear
        );
    }

    #[test]
    fn films_and_series_are_kept_apart() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Series, "Arrival", None), Lookup::Missing);
        assert_eq!(c.lookup(Kind::Film, "Breaking Bad", None), Lookup::Missing);
        // Both exist, and each is found only as itself.
        assert_eq!(c.lookup(Kind::Film, "Fargo", Some(1996)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Series, "Fargo", Some(1996)),
            Lookup::WrongYear
        );
    }

    #[test]
    fn a_dropped_or_added_article_is_tried() {
        let c = sample();
        assert_eq!(
            c.lookup(Kind::Film, "Lord of the Rings The Two Towers", Some(2002)),
            Lookup::Missing,
            "a plain lookup is exact"
        );
        assert_eq!(
            c.find(Kind::Film, "Lord of the Rings The Two Towers", Some(2002)),
            Lookup::Found
        );
        assert_eq!(c.find(Kind::Film, "The Arrival", Some(2016)), Lookup::Found);
    }

    #[test]
    fn an_empty_or_punctuation_only_title_is_never_found() {
        let c = sample();
        assert_eq!(c.lookup(Kind::Film, "", None), Lookup::Missing);
        assert_eq!(c.lookup(Kind::Film, " -_. ", None), Lookup::Missing);
    }

    #[test]
    fn normalising_keeps_other_scripts_and_folds_everything_meaningless() {
        assert_eq!(normalise("Spider-Man: No Way Home"), "spidermannowayhome");
        assert_eq!(normalise("Amélie"), "amelie");
        assert_eq!(normalise("WALL·E"), "walle");
        assert_eq!(normalise("Ｆｕｌｌ Ｗｉｄｔｈ"), "fullwidth");
        assert_eq!(normalise("Fast & Furious"), "fastandfurious");
        assert_eq!(normalise("千と千尋の神隠し"), "千と千尋の神隠し");
        assert_eq!(normalise("Кин-дза-дза!"), "киндзадза");
    }

    #[test]
    fn spellings_try_the_article_both_ways() {
        assert_eq!(spellings("The Office"), ["The Office", "Office"]);
        assert_eq!(spellings("Office"), ["Office", "The Office"]);
        assert_eq!(spellings("THE OFFICE"), ["THE OFFICE", "OFFICE"]);
        // "A" and "An" are left alone on purpose.
        assert_eq!(
            spellings("An Education"),
            ["An Education", "The An Education"]
        );
        // A title that is only the article, or starts with a word that merely
        // begins with it, is not stripped.
        assert_eq!(spellings("Theory"), ["Theory", "The Theory"]);
        assert_eq!(spellings("The"), ["The", "The The"]);
        // Never splits a character, whatever follows.
        assert_eq!(spellings("The Überfall"), ["The Überfall", "Überfall"]);
    }

    #[test]
    fn a_catalogue_survives_its_own_encoding() {
        let c = sample();
        let back = Catalog::decode(&c.encode()).expect("round trip");
        assert_eq!(back.len(), c.len());
        assert_eq!(back.snapshot(), 20260920);
        assert_eq!(back.snapshot_year(), 2026);
        assert_eq!(back.keys, c.keys);
        assert_eq!(back.first, c.first);
        assert_eq!(back.last, c.last);
        assert_eq!(
            back.lookup(Kind::Film, "Arrival", Some(2016)),
            Lookup::Found
        );
    }

    #[test]
    fn duplicates_are_stored_once() {
        let mut builder = Builder::new(1);
        builder.add(Kind::Film, "Arrival", Some((2016, 2016)));
        builder.add(Kind::Film, "ARRIVAL", Some((2016, 2016)));
        builder.add(Kind::Film, "Arrival.", Some((2016, 2016)));
        assert_eq!(builder.finish().len(), 1);
    }

    #[test]
    fn nothing_is_an_empty_catalogue_rather_than_an_error() {
        let c = Catalog::decode(&[]).expect("empty is fine");
        assert!(c.is_empty());
        assert_eq!(c.lookup(Kind::Film, "Arrival", None), Lookup::Missing);
    }

    #[test]
    fn a_damaged_file_is_refused_rather_than_misread() {
        assert!(matches!(
            Catalog::decode(b"nope"),
            Err(CatalogError::NotACatalogue)
        ));
        let mut wrong = sample().encode();
        wrong[4] = 99;
        assert!(matches!(
            Catalog::decode(&wrong),
            Err(CatalogError::Version(99))
        ));
        let good = sample().encode();
        let truncated = &good[..good.len() - 5];
        assert!(Catalog::decode(truncated).is_err());
    }

    #[test]
    fn the_earliest_and_latest_years_survive_a_round_trip() {
        let mut builder = Builder::new(1);
        // The oldest surviving film, and a series running long enough that its
        // span needs most of a byte.
        builder.add(Kind::Film, "Roundhay Garden Scene", Some((1888, 1888)));
        builder.add(Kind::Series, "Long Runner", Some((1953, 2026)));
        let c = Catalog::decode(&builder.finish().encode()).unwrap();
        assert_eq!(
            c.lookup(Kind::Film, "Roundhay Garden Scene", Some(1888)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(Kind::Film, "Roundhay Garden Scene", Some(1950)),
            Lookup::WrongYear
        );
        assert_eq!(
            c.lookup(Kind::Series, "Long Runner", Some(1953)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(Kind::Series, "Long Runner", Some(2026)),
            Lookup::Found
        );
    }

    #[test]
    fn varints_round_trip_at_the_edges() {
        for value in [
            0u64,
            1,
            127,
            128,
            16_383,
            16_384,
            u64::from(u32::MAX),
            (1 << KEY_BITS) - 1,
        ] {
            let mut out = Vec::new();
            write_varint(&mut out, value);
            assert_eq!(read_varint(&out), Some((value, out.len())));
        }
        assert_eq!(read_varint(&[0x80, 0x80]), None);
    }

    /// The shipped file, when there is one, must be a real catalogue.
    #[test]
    fn the_bundled_catalogue_decodes_and_knows_the_classics() {
        if BUNDLED.is_empty() {
            return;
        }
        let c = Catalog::bundled().expect("the bundled catalogue decodes");
        assert!(c.len() > 300_000, "only {} titles", c.len());
        assert!(c.snapshot_year() >= 2026);
        assert_eq!(c.lookup(Kind::Film, "Arrival", Some(2016)), Lookup::Found);
        assert_eq!(
            c.lookup(Kind::Film, "Blade Runner 2049", Some(2017)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(Kind::Series, "Breaking Bad", Some(2008)),
            Lookup::Found
        );
        assert_eq!(
            c.lookup(
                Kind::Film,
                "Literature Review-20251014 100532-Meeting Recording",
                Some(2025)
            ),
            Lookup::Missing
        );
    }
}
