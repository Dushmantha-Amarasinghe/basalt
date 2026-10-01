//! Recognising films and series on a drive nobody organised for us.
//!
//! [`parse`] reads a path and says what it thinks it is, with a confidence.
//! [`index`] walks the vault, folds the results into items, and persists them.
//!
//! The whole thing is off unless the user turns it on, because scanning a drive
//! is work they did not ask for and a library they may not want. When it is on,
//! every scan rebuilds the index completely — which is what makes deletions
//! disappear without any separate bookkeeping to go wrong.

pub mod art;
pub mod collect;
pub mod index;
pub mod parse;
pub mod progress;
pub mod quality;
pub mod subs;
pub mod thumbs;

pub use index::{Library, scan};

/// The revision that follows `previous`: later than it, and never earlier
/// than now, in milliseconds.
///
/// Revisions are how a client asks "anything new?", and the host answers
/// "no" when the number it is given is its own. A plain counter per drive
/// made that answer wrong after the host changed drive: the new drive's
/// library could be on the same count as the old one, and a client holding
/// the old drive's films was told it already had this one's. Taken from the
/// clock, the numbers of two drives do not meet, and they still only ever go
/// up for one. Milliseconds stay far inside what JavaScript holds exactly.
pub fn next_revision(previous: u64) -> u64 {
    (previous + 1).max(now_millis())
}

/// A revision loaded from disk, made at least as late as now, so it cannot
/// equal anything a client was given for a drive in use before it. Zero is
/// kept: it means "never scanned" and is always answered in full.
pub fn reloaded_revision(saved: u64) -> u64 {
    if saved == 0 {
        0
    } else {
        saved.max(now_millis())
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
pub use parse::{Parsed, is_video};
pub use progress::Progress;
