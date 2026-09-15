//! Batch request types.

use serde::{Deserialize, Serialize};

/// A request for many files in one round trip.
///
/// The host is free to reorder the response: it sorts the requested paths into
/// on-disk order before reading, so a spinning disk performs one broadly
/// sequential sweep instead of thousands of random seeks. Clients must key off
/// the path in each entry rather than assuming request order is preserved.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchRequest {
    /// Relative, `/`-separated paths. Validated host-side before use.
    pub paths: Vec<String>,

    /// Whether the client can decode a compressed body. When false the host
    /// must answer with [`crate::Codec::Raw`].
    #[serde(default = "default_true")]
    pub accept_compression: bool,

    /// Preferred zstd level. The host may clamp this — a busy host serving
    /// several devices will refuse to spend level-19 CPU on one of them.
    #[serde(default)]
    pub preferred_level: Option<i32>,
}

fn default_true() -> bool {
    true
}

impl BatchRequest {
    pub fn new(paths: Vec<String>) -> Self {
        Self {
            paths,
            accept_compression: true,
            preferred_level: None,
        }
    }

    /// Control arm for benchmarks: request the same files with compression off.
    pub fn uncompressed(paths: Vec<String>) -> Self {
        Self {
            paths,
            accept_compression: false,
            preferred_level: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_compression_defaults_to_true_when_absent() {
        let req: BatchRequest = serde_json::from_str(r#"{"paths":["a.txt"]}"#).unwrap();
        assert!(req.accept_compression);
        assert_eq!(req.paths, vec!["a.txt"]);
        assert_eq!(req.preferred_level, None);
    }

    #[test]
    fn request_round_trips_through_json() {
        let req = BatchRequest {
            paths: vec!["a.txt".into(), "b/c.rs".into()],
            accept_compression: false,
            preferred_level: Some(3),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: BatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.paths, req.paths);
        assert!(!back.accept_compression);
        assert_eq!(back.preferred_level, Some(3));
    }
}
