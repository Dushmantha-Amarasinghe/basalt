//! The update path run against the real release, end to end.
//!
//! Ignored by default, and deliberately so: these reach GitHub, and a suite
//! that cannot run on a train is a suite people stop running. Everything they
//! cover that can be decided offline — which asset belongs to which app, which
//! tag is newer, how a digest file is read — is covered by the unit tests in
//! `lib.rs` and `version.rs`, which always run.
//!
//! What is left is the part no mock can vouch for: that the published release
//! is actually shaped the way the app expects, and that the installer sitting
//! on it matches the checksum published beside it. Run them after publishing:
//!
//! ```text
//! cargo test -p basalt-update -- --ignored --nocapture
//! ```

use basalt_update::{Product, check, fetch};

/// A version far enough behind that any real release is an advance.
const ANCIENT: &str = "0.0.1";

#[tokio::test]
#[ignore = "reaches GitHub"]
async fn the_published_release_offers_an_installer_to_both_apps() {
    for product in [Product::Host, Product::Client] {
        let offer = check(product, ANCIENT)
            .await
            .expect("the releases API answered")
            .unwrap_or_else(|| panic!("{product:?} was offered nothing"));

        println!(
            "{product:?}: v{} · {} · {} bytes",
            offer.version, offer.installer_name, offer.installer_bytes
        );

        assert!(offer.installer_bytes > 0, "{product:?} installer is empty");
        assert!(
            offer.checksum_url.is_some(),
            "{product:?} has no .sha256 beside its installer, so the app will \
             refuse the download rather than install something unverified",
        );
        assert!(
            !offer.notes.is_empty(),
            "{product:?} has no release notes, and the About panel shows them \
             as the reason to update",
        );
    }
}

/// The whole download: bytes, digest, and the file left on disk.
#[tokio::test]
#[ignore = "downloads tens of megabytes from GitHub"]
async fn the_published_installer_matches_its_published_checksum() {
    let offer = check(Product::Host, ANCIENT)
        .await
        .expect("the releases API answered")
        .expect("a release was offered");

    let into = std::env::temp_dir().join("basalt-update-test");
    std::fs::create_dir_all(&into).expect("a place to download to");

    let mut last = 0u64;
    let path = fetch(&offer, &into, |had, total| {
        // Progress must arrive, and must not run past the total — the About
        // panel divides by it.
        assert!(had <= total || total == 0, "{had} of {total}");
        last = had;
    })
    .await
    .expect("the installer downloaded and verified");

    let landed = std::fs::metadata(&path).expect("the installer is on disk");
    assert_eq!(landed.len(), offer.installer_bytes);
    assert_eq!(last, offer.installer_bytes, "progress stopped short");
    assert!(
        !path.to_string_lossy().ends_with(".part"),
        "a half-written download was left where a finished one should be",
    );

    std::fs::remove_file(&path).ok();
}
