//! Comparing one version with another.
//!
//! Small on purpose. A release tag here is `v1.2.0` and nothing more exotic,
//! so a semver crate would be a dependency earning its place by handling
//! cases this project will never publish.

/// Three numbers, which is all a Basalt tag ever is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// Reads `v1.2.0`, `1.2.0`, or `1.2` — anything else is not a version.
    ///
    /// Trailing text is refused rather than ignored: `1.2.0-rc1` is not the
    /// same release as `1.2.0`, and treating it as one would offer a release
    /// candidate to everybody.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        if text.is_empty() {
            return None;
        }

        let mut parts = text.split('.');
        let mut number = || -> Option<u32> {
            match parts.next() {
                Some(part) => part.parse().ok(),
                // A missing patch is zero: `1.2` is `1.2.0`.
                None => Some(0),
            }
        };

        let major = number()?;
        let minor = number()?;
        let patch = number()?;
        // Anything left over means this was not a plain version.
        if parts.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch))
    }
}

/// Whether `candidate` is a release worth offering to someone on `current`.
///
/// Unreadable on either side means no. An update prompt is an interruption,
/// and one raised because a tag could not be parsed is an interruption with
/// nothing behind it.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(new), Some(old)) => new > old,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_read_with_or_without_its_v() {
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse(" 1.2.3 "), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("1.2"), Some(Version(1, 2, 0)));
    }

    #[test]
    fn anything_that_is_not_a_version_is_refused() {
        for text in ["", "v", "latest", "1.2.3.4", "1.x.0", "1.2.0-rc1", "beta"] {
            assert_eq!(Version::parse(text), None, "{text}");
        }
    }

    #[test]
    fn newer_means_newer() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(is_newer("v2.0.0", "1.99.99"));
    }

    #[test]
    fn the_same_or_older_is_not_an_update() {
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        // Ten is after nine, which string comparison gets wrong.
        assert!(!is_newer("v1.0.9", "1.0.10"));
        assert!(is_newer("v1.0.10", "1.0.9"));
    }

    #[test]
    fn an_unreadable_version_never_prompts() {
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("v1.0.1", "unknown"));
    }
}
