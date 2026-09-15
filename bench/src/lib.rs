//! Basalt Phase 0 measurement harness.
//!
//! Exposed as a library so the integration tests in `tests/` can drive the real
//! server and client rather than a reimplementation of them. A test that only
//! exercises a copy of the code proves nothing about what actually ships.

pub mod compress;
pub mod corpus;
pub mod disk;
pub mod net;
pub mod report;
pub mod smb;
pub mod stats;
pub mod winio;
