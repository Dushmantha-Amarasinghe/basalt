//! Finding out whether a newer Basalt exists, and fetching it safely.
//!
//! Both apps ship from one repository, so one release carries two installers
//! and each app has to recognise its own — see [`Product`].
//!
//! **What is downloaded is verified before it is run.** A release publishes a
//! `.sha256` beside each installer, and an installer whose digest does not
//! match is deleted rather than offered. Anything less would make the update
//! path the easiest way to get code onto somebody's machine.
//!
//! Nothing here installs anything. It reports what is available, fetches it,
//! and hands back a path; starting an installer is the shell's business,
//! because only the shell knows how to close its own window first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

mod version;
pub use version::{Version, is_newer};

/// Which installer out of a release belongs to this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Host,
    Client,
}

impl Product {
    /// The marker in an asset's filename.
    ///
    /// Matched on rather than the whole name so a release can rename its
    /// installers — for a version number, say — without every older copy of
    /// the app losing the ability to find them.
    fn marker(self) -> &'static str {
        match self {
            Product::Host => "host",
            Product::Client => "client",
        }
    }
}

/// Where releases are published.
pub const OWNER: &str = "Dushmantha-Amarasinghe";
pub const REPO: &str = "basalt";

/// How long any one request may take to begin answering.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a download may deliver nothing at all before it is given up on.
///
/// A stall timeout rather than a deadline on the whole transfer: the client
/// installer is thirty-five megabytes and some connections are genuinely
/// slow, so any total limit generous enough to be fair is far too long to be
/// useful. What is never legitimate is a socket that has stopped sending.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub answered {0}")]
    Status(u16),
    #[error("the release could not be read: {0}")]
    Malformed(String),
    #[error("this release has no installer for this app")]
    NoInstaller,
    #[error("the download did not match its checksum and was discarded")]
    ChecksumMismatch,
    #[error("the download stopped part way and did not resume")]
    Stalled,
    #[error("{0}")]
    Io(String),
}

type Result<T> = std::result::Result<T, UpdateError>;

/// A release newer than what is running.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// `1.2.0`, without the leading `v`.
    pub version: String,
    /// What the release notes say, as written.
    pub notes: String,
    /// The release page, for anyone who would rather read it there.
    pub page_url: String,
    pub installer_name: String,
    pub installer_url: String,
    pub installer_bytes: u64,
    /// The published digest. Absent means the download cannot be verified,
    /// which [`fetch`] treats as a refusal rather than a warning.
    pub checksum_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize, Clone)]
struct GhAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("Basalt/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| UpdateError::Network(e.to_string()))
}

/// The newest published release, or `None` when it is not newer than `current`.
pub async fn check(product: Product, current: &str) -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest");
    let response = client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| UpdateError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(UpdateError::Status(response.status().as_u16()));
    }
    let release: GhRelease = response
        .json()
        .await
        .map_err(|e| UpdateError::Malformed(e.to_string()))?;

    Ok(newer_than(release, product, current))
}

/// Turns a release into an offer, or nothing when there is nothing to offer.
///
/// Separated from the request so the decision is testable without a network:
/// which release counts, which asset belongs to this app, and whether the
/// version is actually an advance.
fn newer_than(release: GhRelease, product: Product, current: &str) -> Option<Release> {
    // A draft is not published and a prerelease was not offered to everyone.
    if release.draft || release.prerelease {
        return None;
    }
    if !is_newer(&release.tag_name, current) {
        return None;
    }

    let installer = pick(&release.assets, product)?;
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == format!("{}.sha256", installer.name))
        .map(|asset| asset.browser_download_url.clone());

    Some(Release {
        version: release.tag_name.trim_start_matches('v').to_string(),
        notes: release.body.trim().to_string(),
        page_url: release.html_url,
        installer_name: installer.name.clone(),
        installer_url: installer.browser_download_url.clone(),
        installer_bytes: installer.size,
        checksum_url: checksum,
    })
}

/// The installer in this release that belongs to this app.
fn pick(assets: &[GhAsset], product: Product) -> Option<&GhAsset> {
    assets.iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.ends_with(".exe") && name.contains(product.marker())
    })
}

/// Downloads an installer and returns its path, having checked its digest.
///
/// `progress` is called with bytes received and the total expected.
pub async fn fetch(
    release: &Release,
    into: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};

    let Some(checksum_url) = release.checksum_url.as_deref() else {
        // Refused rather than warned about. An unverified installer is the
        // one thing an update path must never hand to somebody.
        return Err(UpdateError::ChecksumMismatch);
    };

    let http = client()?;
    let expected = digest_from(
        &http
            .get(checksum_url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| UpdateError::Network(e.to_string()))?
            .text()
            .await
            .map_err(|e| UpdateError::Network(e.to_string()))?,
    )
    .ok_or_else(|| UpdateError::Malformed("the checksum file made no sense".into()))?;

    std::fs::create_dir_all(into).map_err(|e| UpdateError::Io(e.to_string()))?;
    let target = into.join(&release.installer_name);
    let part = target.with_extension("part");

    let response = http
        .get(&release.installer_url)
        .send()
        .await
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::Status(response.status().as_u16()));
    }

    let total = response.content_length().unwrap_or(release.installer_bytes);
    let mut file = std::fs::File::create(&part).map_err(|e| UpdateError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut had = 0u64;
    let mut stream = response.bytes_stream();

    loop {
        let next = tokio::time::timeout(STALL_TIMEOUT, stream.next()).await;
        let chunk = match next {
            Err(_) => {
                let _ = std::fs::remove_file(&part);
                return Err(UpdateError::Stalled);
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                let _ = std::fs::remove_file(&part);
                return Err(UpdateError::Network(e.to_string()));
            }
            Ok(Some(Ok(chunk))) => chunk,
        };

        use std::io::Write;
        file.write_all(&chunk)
            .map_err(|e| UpdateError::Io(e.to_string()))?;
        hasher.update(&chunk);
        had += chunk.len() as u64;
        progress(had, total);
    }
    drop(file);

    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&expected) {
        // Removed, not left lying about. A file that failed its check is not
        // something to leave on disk where somebody might run it by hand.
        let _ = std::fs::remove_file(&part);
        return Err(UpdateError::ChecksumMismatch);
    }

    let _ = std::fs::remove_file(&target);
    std::fs::rename(&part, &target).map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(target)
}

/// Reads the digest out of a `.sha256` file.
///
/// Handles both shapes these come in: the bare digest, and the
/// `<digest>  <filename>` that `sha256sum` writes.
pub fn digest_from(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?;
    let token = token.trim_start_matches('*');
    let looks_right = token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit());
    looks_right.then(|| token.to_ascii_lowercase())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GhAsset {
        GhAsset {
            name: name.into(),
            browser_download_url: format!("https://example.test/{name}"),
            size: 10,
        }
    }

    fn release(tag: &str, names: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            body: "notes".into(),
            html_url: "https://example.test/release".into(),
            draft: false,
            prerelease: false,
            assets: names.iter().map(|n| asset(n)).collect(),
        }
    }

    /// One release carries both installers, and each app must take its own.
    #[test]
    fn each_app_finds_its_own_installer() {
        let assets = [
            asset("Basalt-Client-1.1.0-setup.exe"),
            asset("Basalt-Host-1.1.0-setup.exe"),
        ];
        assert_eq!(
            pick(&assets, Product::Client).unwrap().name,
            "Basalt-Client-1.1.0-setup.exe"
        );
        assert_eq!(
            pick(&assets, Product::Host).unwrap().name,
            "Basalt-Host-1.1.0-setup.exe"
        );
    }

    #[test]
    fn a_release_missing_this_app_offers_nothing() {
        let only_host = release("v2.0.0", &["Basalt-Host-2.0.0-setup.exe"]);
        assert!(newer_than(only_host, Product::Client, "1.0.0").is_none());
    }

    #[test]
    fn a_draft_or_prerelease_is_not_offered() {
        let mut draft = release("v2.0.0", &["Basalt-Client-2.0.0-setup.exe"]);
        draft.draft = true;
        assert!(newer_than(draft, Product::Client, "1.0.0").is_none());

        let mut early = release("v2.0.0", &["Basalt-Client-2.0.0-setup.exe"]);
        early.prerelease = true;
        assert!(newer_than(early, Product::Client, "1.0.0").is_none());
    }

    #[test]
    fn the_same_version_is_not_an_update() {
        let same = release("v1.0.0", &["Basalt-Client-1.0.0-setup.exe"]);
        assert!(newer_than(same, Product::Client, "1.0.0").is_none());
    }

    #[test]
    fn a_newer_release_carries_its_notes_and_its_checksum() {
        let found = newer_than(
            release(
                "v1.1.0",
                &[
                    "Basalt-Client-1.1.0-setup.exe",
                    "Basalt-Client-1.1.0-setup.exe.sha256",
                ],
            ),
            Product::Client,
            "1.0.0",
        )
        .expect("an update");

        assert_eq!(found.version, "1.1.0");
        assert_eq!(found.notes, "notes");
        assert!(found.checksum_url.unwrap().ends_with(".sha256"));
    }

    /// The checksum belongs to *this* installer, not to whichever one is
    /// listed first — a release holds two of each.
    #[test]
    fn the_checksum_matches_the_installer_it_belongs_to() {
        let found = newer_than(
            release(
                "v1.1.0",
                &[
                    "Basalt-Host-1.1.0-setup.exe",
                    "Basalt-Host-1.1.0-setup.exe.sha256",
                    "Basalt-Client-1.1.0-setup.exe",
                    "Basalt-Client-1.1.0-setup.exe.sha256",
                ],
            ),
            Product::Client,
            "1.0.0",
        )
        .expect("an update");
        assert!(found.checksum_url.unwrap().contains("Client"));
    }

    #[test]
    fn a_digest_is_read_in_either_shape() {
        let bare = "a".repeat(64);
        assert_eq!(digest_from(&bare), Some(bare.clone()));
        assert_eq!(
            digest_from(&format!("{bare}  installer.exe")),
            Some(bare.clone())
        );
        // What `sha256sum` writes in binary mode.
        assert_eq!(digest_from(&format!("{bare} *installer.exe")), Some(bare));
    }

    #[test]
    fn nonsense_is_not_mistaken_for_a_digest() {
        assert_eq!(digest_from(""), None);
        assert_eq!(digest_from("not a hash"), None);
        assert_eq!(digest_from(&"a".repeat(63)), None, "too short");
        assert_eq!(digest_from(&"z".repeat(64)), None, "not hex");
    }
}
