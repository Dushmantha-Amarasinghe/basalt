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
pub mod index;
pub mod parse;
pub mod progress;

pub use index::{Library, scan};
pub use parse::{Parsed, is_video};
pub use progress::Progress;
