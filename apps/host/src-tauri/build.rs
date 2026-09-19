use std::process::Command;

/// Stamps the build with the commit it came from and the day it was made.
///
/// Every build calls itself 0.1.0, which made "is this the new one?"
/// unanswerable: an installer copied between machines carries the copy's date,
/// not the build's, so the file on disk lies about its age. Somebody chasing a
/// bug that was already fixed spent a round trip on it.
///
/// Both values are also written to the log at startup, so a log always says
/// exactly which binary produced it.
fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|out| !out.stdout.is_empty());

    let stamp = format!("{commit}{}", if dirty { "+" } else { "" });
    println!("cargo:rustc-env=BASALT_COMMIT={stamp}");
    println!(
        "cargo:rustc-env=BASALT_BUILT={}",
        // Seconds since the epoch, formatted by the app. No date crate here:
        // a build script should not pull a dependency to print one string.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    // Rerun on every build, by naming a path that does not exist: cargo treats
    // a missing dependency as changed.
    //
    // The obvious version of this watched `.git/HEAD`, and it was wrong in the
    // one case the stamp exists for. Committing writes `.git/refs/heads/<branch>`
    // and leaves `HEAD` alone, so the build script never reran and the binary
    // went on claiming the commit before the fix — a stamp that lies is worse
    // than no stamp, since it is trusted.
    //
    // Watching the resolved ref and `packed-refs` too would fix that case and
    // still get the date wrong for any build made without committing. The only
    // stamp that is always true is one recomputed every time. It costs this one
    // crate a relink per build, and it is rebuilt for a release anyway.
    println!("cargo:rerun-if-changed=.build-stamp-always-reruns");

    tauri_build::build()
}
