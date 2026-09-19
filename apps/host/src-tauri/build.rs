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

    // Without this the stamp would be baked in once and never refreshed,
    // which is worse than having none at all.
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");

    tauri_build::build()
}
