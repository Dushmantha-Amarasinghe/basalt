//! Adversarial path validation tests.
//!
//! Every path in a batch stream arrives from the network and then gets joined
//! to a real directory on a real drive. This is the single place where a bug
//! turns into "a peer on your Wi-Fi wrote a file into `C:\Windows\System32`",
//! so it gets tested like a security boundary rather than a string utility.
//!
//! The invariant under test: **anything `sanitize_relative_path` accepts, when
//! joined to a root, must stay inside that root.** Rejecting a legitimate path
//! is an inconvenience. Accepting a malicious one is a compromise.

use std::path::{Component, Path, PathBuf};

use basalt_proto::frame::sanitize_relative_path;

/// Paths that must never be accepted, with why they are dangerous.
const MUST_REJECT: &[(&str, &str)] = &[
    // --- classic traversal ---
    ("..", "bare parent"),
    ("../", "parent with separator"),
    ("../etc/passwd", "leading traversal"),
    ("a/../../b", "traversal through a valid segment"),
    ("a/b/../../../c", "multi-level escape"),
    ("....//", "dot padding"),
    ("..\\..\\windows", "backslash traversal"),
    ("a/..\\..\\b", "mixed separator traversal"),
    ("./../x", "traversal after a no-op segment"),
    // --- absolute paths ---
    ("/etc/passwd", "unix absolute"),
    ("/", "root"),
    ("\\windows\\system32", "windows absolute"),
    ("\\", "windows root"),
    // --- drive-qualified ---
    ("C:/Windows/win.ini", "drive absolute"),
    ("C:\\Windows\\win.ini", "drive absolute, backslashes"),
    ("c:windows", "drive relative"),
    ("Z:/", "drive root"),
    ("a:b", "single-letter drive"),
    // --- UNC / network ---
    ("//server/share", "UNC"),
    ("\\\\server\\share", "UNC, backslashes"),
    ("//./PhysicalDrive0", "device namespace"),
    ("//?/C:/Windows", "extended-length prefix"),
    // --- NUL and control characters ---
    ("a\0b", "embedded NUL"),
    ("\0", "bare NUL"),
    ("a\nb", "newline"),
    ("a\rb", "carriage return"),
    ("a\tb", "tab"),
    ("a\x7fb", "delete"),
    ("a\x1bb", "escape"),
    // --- Windows reserved device names ---
    ("con", "CON device"),
    ("CON", "CON device, uppercase"),
    ("CoN", "CON device, mixed case"),
    ("nul", "NUL device"),
    ("nul.txt", "NUL device with extension"),
    ("prn", "PRN device"),
    ("aux", "AUX device"),
    ("com1", "COM1 device"),
    ("COM9.log", "COM9 with extension"),
    ("lpt1", "LPT1 device"),
    ("a/con/b", "device name nested mid-path"),
    ("dir/nul", "device name as leaf"),
    // --- trailing dots and spaces (Windows strips these silently) ---
    ("evil.exe.", "trailing dot"),
    ("evil.exe ", "trailing space"),
    ("evil.exe...", "multiple trailing dots"),
    ("dir./file.txt", "trailing dot on a directory"),
    ("dir /file.txt", "trailing space on a directory"),
    // --- degenerate ---
    ("", "empty"),
    (".", "current directory"),
    ("./", "current directory with separator"),
    ("././.", "repeated no-ops"),
    ("//", "empty segments only"),
];

/// Paths that are awkward but legitimate and must be accepted.
const MUST_ACCEPT: &[(&str, &str)] = &[
    ("a.txt", "a.txt"),
    ("dir/file.txt", "dir/file.txt"),
    ("a/b/c/d/e/f.bin", "a/b/c/d/e/f.bin"),
    ("dir\\file.txt", "dir/file.txt"),
    ("./dir/file.txt", "dir/file.txt"),
    ("dir//file.txt", "dir/file.txt"),
    ("dir/./file.txt", "dir/file.txt"),
    // Device-like names that are NOT reserved.
    ("console.txt", "console.txt"),
    ("communication.md", "communication.md"),
    ("com0.txt", "com0.txt"),
    ("com10.txt", "com10.txt"),
    ("nulled.txt", "nulled.txt"),
    ("auxiliary/notes.txt", "auxiliary/notes.txt"),
    ("prnt.txt", "prnt.txt"),
    // Dots in sensible places.
    ("file.tar.gz", "file.tar.gz"),
    (".gitignore", ".gitignore"),
    ("dir/.hidden", "dir/.hidden"),
    ("..hidden.txt", "..hidden.txt"),
    // Unicode.
    ("документы/отчёт.txt", "документы/отчёт.txt"),
    ("写真/家族.jpg", "写真/家族.jpg"),
    ("emoji/🎬.mkv", "emoji/🎬.mkv"),
    ("café/naïve.txt", "café/naïve.txt"),
    // Spaces and punctuation in the middle are fine.
    (
        "My Documents/Report v2 (final).docx",
        "My Documents/Report v2 (final).docx",
    ),
    ("a-b_c/d+e=f.txt", "a-b_c/d+e=f.txt"),
];

#[test]
fn dangerous_paths_are_rejected() {
    let mut accepted = Vec::new();
    for (path, why) in MUST_REJECT {
        if let Ok(result) = sanitize_relative_path(path) {
            accepted.push(format!("{path:?} ({why}) -> accepted as {result:?}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "these dangerous paths were accepted:\n  {}",
        accepted.join("\n  ")
    );
}

#[test]
fn legitimate_paths_are_accepted_and_normalised() {
    for (input, expected) in MUST_ACCEPT {
        match sanitize_relative_path(input) {
            Ok(got) => assert_eq!(
                &got, expected,
                "{input:?} normalised to {got:?}, expected {expected:?}"
            ),
            Err(e) => panic!("{input:?} should be accepted but was rejected: {e}"),
        }
    }
}

/// The property that actually matters, checked against the real filesystem
/// path logic rather than against string rules.
#[test]
fn accepted_paths_never_escape_the_root() {
    let root = Path::new("/srv/basalt/share");

    for (input, _) in MUST_ACCEPT {
        let safe = sanitize_relative_path(input).expect("should be accepted");
        let joined = root.join(&safe);

        // Resolve `.` and `..` lexically, the way a filesystem would, and
        // confirm we are still under the root.
        let mut resolved = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::ParentDir => {
                    assert!(
                        resolved.starts_with(root),
                        "{input:?} walked above the root during resolution"
                    );
                    resolved.pop();
                }
                Component::CurDir => {}
                other => resolved.push(other.as_os_str()),
            }
        }
        assert!(
            resolved.starts_with(root),
            "{input:?} resolved to {resolved:?}, which is outside {root:?}"
        );
    }
}

#[test]
fn no_accepted_path_contains_traversal_or_separators_that_bite() {
    for (input, _) in MUST_ACCEPT {
        let safe = sanitize_relative_path(input).unwrap();
        assert!(!safe.contains('\\'), "{input:?} kept a backslash: {safe:?}");
        assert!(
            !safe.starts_with('/'),
            "{input:?} became absolute: {safe:?}"
        );
        assert!(
            !safe.contains("//"),
            "{input:?} kept an empty segment: {safe:?}"
        );
        assert!(
            !safe.split('/').any(|s| s == ".." || s == "."),
            "{input:?} kept a traversal segment: {safe:?}"
        );
        assert!(!safe.is_empty(), "{input:?} normalised to nothing");
    }
}

#[test]
fn sanitising_is_idempotent() {
    // Running an already-sanitised path through again must not change it.
    // If it did, some layer applying the check twice would behave differently
    // from a layer applying it once.
    for (input, _) in MUST_ACCEPT {
        let once = sanitize_relative_path(input).unwrap();
        let twice = sanitize_relative_path(&once)
            .unwrap_or_else(|e| panic!("{input:?} -> {once:?} was rejected on re-check: {e}"));
        assert_eq!(once, twice, "{input:?} is not idempotent");
    }
}

#[test]
fn generated_traversal_permutations_are_all_rejected() {
    // Machine-generated combinations catch gaps a hand-written list misses.
    let prefixes = ["", "a/", "a/b/", "./", "x/./"];
    let travs = ["..", "..\\", "../", "%2e%2e"];
    let suffixes = ["", "/etc/passwd", "\\windows", "/x"];

    let mut leaked = Vec::new();
    for p in prefixes {
        for t in travs {
            for s in suffixes {
                let path = format!("{p}{t}{s}");
                if let Ok(result) = sanitize_relative_path(&path) {
                    // "%2e%2e" is a URL encoding, not a filesystem traversal —
                    // it is a literal directory name here and is legitimately
                    // accepted. Anything containing a real `..` is not.
                    if result.split('/').any(|seg| seg == "..") {
                        leaked.push(format!("{path:?} -> {result:?}"));
                    }
                }
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "traversal survived sanitisation:\n  {}",
        leaked.join("\n  ")
    );
}

#[test]
fn long_paths_are_bounded() {
    let deep = (0..2000)
        .map(|i| format!("dir{i}"))
        .collect::<Vec<_>>()
        .join("/");
    assert!(
        sanitize_relative_path(&deep).is_err(),
        "an over-long path must be rejected rather than passed to the OS"
    );

    let long_segment = "a".repeat(10_000);
    assert!(
        sanitize_relative_path(&long_segment).is_err(),
        "an over-long single segment must be rejected"
    );
}

#[test]
fn every_rejection_explains_itself() {
    // Error messages end up in logs and in the host's access view. A bare
    // "invalid path" would make a genuine misconfiguration impossible to debug.
    for (path, _) in MUST_REJECT {
        if let Err(e) = sanitize_relative_path(path) {
            let msg = e.to_string();
            assert!(
                msg.len() > 10,
                "rejection of {path:?} produced an unhelpful message: {msg:?}"
            );
        }
    }
}

#[test]
fn null_bytes_cannot_truncate_a_path() {
    // The classic C-string truncation attack: "safe.txt\0../../evil" looks
    // harmless to a length-aware check and becomes "safe.txt" to any API that
    // stops at NUL. Rejecting outright is the only safe answer.
    for path in [
        "safe.txt\0../../evil",
        "\0../../evil",
        "dir/safe\0/../../evil",
    ] {
        assert!(
            sanitize_relative_path(path).is_err(),
            "{path:?} must be rejected — NUL can truncate the path downstream"
        );
    }
}
