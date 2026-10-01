//! Files with awkward names and in awkward states, through the real protocol.
//!
//! A drive is not a test fixture: it has names in every script, symbols that
//! mean something in a URL, folders deeper than Windows' old path limit,
//! thousands of files in one place, files another program holds open, and
//! links that point somewhere else entirely. Every one of these must list,
//! read, move and delete as it would in Explorer, or fail with a reason a
//! person can act on — never vanish, hang, or take the folder down with it.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use basalt_client::Basalt;
use basalt_host::{Host, HostConfig, server};

fn unique(prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "basalt-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

struct Fixture {
    dir: PathBuf,
    host: Arc<Host>,
    addr: SocketAddr,
    store: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Read-only files would stop the folder being removed.
        clear_readonly(&self.dir);
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.store);
    }
}

fn clear_readonly(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                clear_readonly(&path);
            }
            let mut perms = meta.permissions();
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }
    }
}

impl Fixture {
    fn vault(&self) -> PathBuf {
        self.dir.join("vault")
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.vault().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

async fn start() -> (Fixture, Arc<Basalt>) {
    let dir = unique("files");
    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    let config_path = dir.join("host.json");
    let mut config = HostConfig::create("files-host").unwrap();
    config.vault_path = Some(vault);
    config.vault_name = "Awkward Drive".into();
    config.save(&config_path).unwrap();

    let host = Host::new(config, config_path).unwrap();
    let bound = server::bind(Arc::clone(&host), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = bound.addr();
    tokio::spawn(server::serve(bound));

    let store = unique("files-store").with_extension("json");
    let client = Arc::new(Basalt::open(store.clone()).expect("a fresh store"));
    let requires_pin = client.begin_pairing(addr).await.expect("pairing opens");
    let pin = requires_pin.then(|| {
        host.pending_pairings()
            .into_iter()
            .find_map(|r| r.pin)
            .expect("the host displays a PIN")
    });
    client
        .finish_pairing(pin.as_deref())
        .await
        .expect("pairing completes");

    (
        Fixture {
            dir,
            host,
            addr,
            store,
        },
        client,
    )
}

fn names(entries: &[basalt_proto::msg::DirEntry]) -> Vec<String> {
    entries.iter().map(|e| e.name.clone()).collect()
}

/// Names a person might give a file, every one of them legal on Windows.
const NAMES: &[&str] = &[
    "Café — résumé.txt",
    "日本語のファイル.txt",
    "සිංහල ලේඛනය.txt",
    "Ελληνικά.txt",
    "עברית.txt",
    "Holiday 🎬 2026.mp4",
    "a#b%c&d+e.txt",
    "50% off (final) [v2] {copy}.txt",
    "semi;colon,comma=equals@at.txt",
    "it's 'quoted'.txt",
    "tilde~and`backtick^caret.txt",
    "   leading spaces.txt",
    "many.dots.in.the.name.tar.gz",
    "no extension",
    ".hidden-style dotfile",
    "%20 looks encoded.txt",
];

#[tokio::test]
async fn every_kind_of_name_lists_reads_renames_and_deletes() {
    let (fixture, client) = start().await;
    for (i, name) in NAMES.iter().enumerate() {
        fixture.put(&format!("names/{name}"), format!("file {i}").as_bytes());
    }

    let listed = names(&client.list("names").await.expect("the folder lists"));
    for name in NAMES {
        assert!(
            listed.contains(&name.to_string()),
            "{name} is missing from {listed:?}"
        );
    }

    for (i, name) in NAMES.iter().enumerate() {
        let path = format!("names/{name}");
        let bytes = client
            .read_range(&path, 0, 64)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(bytes, format!("file {i}").as_bytes(), "{name} reads back");

        let renamed = format!("names/renamed {i} {name}");
        client
            .rename(&path, &renamed)
            .await
            .unwrap_or_else(|e| panic!("rename {name}: {e}"));
        client
            .copy(&renamed, &format!("names/copy {i} {name}"))
            .await
            .unwrap_or_else(|e| panic!("copy {name}: {e}"));
        client
            .remove(&renamed, false)
            .await
            .unwrap_or_else(|e| panic!("delete {name}: {e}"));
    }
    let left = names(&client.list("names").await.unwrap());
    assert_eq!(
        left.len(),
        NAMES.len(),
        "one copy of each remains: {left:?}"
    );
}

#[tokio::test]
async fn awkward_names_upload_and_download_whole() {
    let (fixture, client) = start().await;
    let local = fixture.dir.join("local");
    std::fs::create_dir_all(&local).unwrap();
    client.mkdir("up").await.expect("a folder to upload into");

    for (i, name) in NAMES.iter().enumerate() {
        let source = local.join(format!("source-{i}"));
        std::fs::write(&source, format!("upload {i}")).unwrap();
        client
            .upload(&source, &format!("up/{name}"), false, None, None)
            .await
            .unwrap_or_else(|e| panic!("upload {name}: {e}"));

        let back = local.join(format!("back-{i}"));
        client
            .download(&format!("up/{name}"), &back, None, None)
            .await
            .unwrap_or_else(|e| panic!("download {name}: {e}"));
        assert_eq!(
            std::fs::read(&back).unwrap(),
            format!("upload {i}").as_bytes(),
            "{name}"
        );
    }
}

// Windows refuses these characters in a name. An upload from a phone, where
// they are legal, must be refused with a reason, not end in a half-written
// file or an error nobody can read.
#[tokio::test]
async fn a_name_windows_cannot_hold_is_refused_clearly() {
    let (fixture, client) = start().await;
    let source = fixture.dir.join("source.txt");
    std::fs::write(&source, b"x").unwrap();
    client.mkdir("bad").await.unwrap();
    for bad in [
        "what?.txt",
        "a:b.txt",
        "pipe|name.txt",
        "star*.txt",
        "quote\".txt",
        "less<more>.txt",
    ] {
        let err = client
            .upload(&source, &format!("bad/{bad}"), false, None, None)
            .await
            .expect_err(bad);
        let message = err.to_string();
        assert!(!message.is_empty(), "{bad}");
        assert!(
            !message.to_lowercase().contains("os error"),
            "{bad}: a raw system error is not a reason a person can act on: {message}"
        );
    }
    // And nothing half-made is left behind.
    if let Ok(left) = client.list("bad").await {
        assert!(left.is_empty(), "{:?}", names(&left));
    }
}

// Explorer renames `IMG_001.JPG` to `IMG_001.jpg` without complaint; Windows
// sees the same name and a careless rename either refuses or does nothing.
#[tokio::test]
async fn a_rename_that_only_changes_capitals_takes() {
    let (fixture, client) = start().await;
    fixture.put("photos/IMG_001.JPG", b"jpeg");
    client
        .rename("photos/IMG_001.JPG", "photos/IMG_001.jpg")
        .await
        .expect("a change of case is a rename");
    assert_eq!(
        names(&client.list("photos").await.unwrap()),
        vec!["IMG_001.jpg"]
    );
}

// Past the 260 characters Windows long refused, and forty folders deep.
#[tokio::test]
async fn long_and_deep_paths_work_end_to_end() {
    let (fixture, client) = start().await;
    let long_dir = format!("long/{}", "a folder with a long name ".repeat(5).trim());
    let deep_dir = (0..40)
        .map(|i| format!("level{i}"))
        .collect::<Vec<_>>()
        .join("/");
    let long_file = format!("{long_dir}/{}/{}.txt", "x".repeat(120), "y".repeat(100));
    let deep_file = format!("{deep_dir}/bottom.txt");
    assert!(fixture.vault().join(&long_file).to_string_lossy().len() > 300);

    for (rel, body) in [
        (&long_file, b"long".as_slice()),
        (&deep_file, b"deep".as_slice()),
    ] {
        let parent = rel.rsplit_once('/').unwrap().0;
        // One folder at a time, as Explorer and the apps make them.
        let mut made = String::new();
        for part in parent.split('/') {
            if !made.is_empty() {
                made.push('/');
            }
            made.push_str(part);
            if client.stat(&made).await.is_err() {
                client
                    .mkdir(&made)
                    .await
                    .unwrap_or_else(|e| panic!("mkdir {made}: {e}"));
            }
        }
        let source = fixture.dir.join("source");
        std::fs::write(&source, body).unwrap();
        client
            .upload(&source, rel, false, None, None)
            .await
            .unwrap_or_else(|e| panic!("upload: {e}"));
        let listed = names(
            &client
                .list(parent)
                .await
                .unwrap_or_else(|e| panic!("list: {e}")),
        );
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(client.read_range(rel, 0, 10).await.unwrap(), body);
        client
            .rename(rel, &format!("{parent}/moved.txt"))
            .await
            .unwrap();
        client
            .remove(&format!("{parent}/moved.txt"), false)
            .await
            .unwrap();
    }
    client
        .remove("level0", true)
        .await
        .expect("a deep tree deletes whole");
    assert!(!fixture.vault().join("level0").exists());
}

#[tokio::test]
async fn ten_thousand_files_in_one_folder_list_whole_and_quickly() {
    let (fixture, client) = start().await;
    let dir = fixture.vault().join("many");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..10_000 {
        std::fs::write(dir.join(format!("file {i:05}.jpg")), b"").unwrap();
    }
    let started = Instant::now();
    let listed = client.list("many").await.expect("the folder lists");
    let took = started.elapsed();
    assert_eq!(listed.len(), 10_000, "every file, none dropped");
    assert!(took < Duration::from_secs(5), "listing took {took:?}");
}

#[tokio::test]
async fn an_empty_file_and_a_five_gigabyte_one_read_correctly() {
    let (fixture, client) = start().await;
    fixture.put("sizes/empty.txt", b"");
    let big = fixture.vault().join("sizes/big.bin");
    {
        std::fs::File::create(&big).unwrap();
        // Sparse: five gigabytes on paper, nothing on the disk. NTFS only
        // leaves the length unallocated for a file marked sparse first.
        #[cfg(windows)]
        {
            let marked = std::process::Command::new("fsutil")
                .args(["sparse", "setflag"])
                .arg(&big)
                .output()
                .unwrap();
            assert!(marked.status.success(), "fsutil sparse setflag");
        }
        let file = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
        file.set_len(5 * 1024 * 1024 * 1024).unwrap();
    }
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
        file.seek(SeekFrom::End(-4)).unwrap();
        file.write_all(b"tail").unwrap();
    }

    let listed = client.list("sizes").await.unwrap();
    let size = |name: &str| listed.iter().find(|e| e.name == name).unwrap().size;
    assert_eq!(size("empty.txt"), 0);
    assert_eq!(size("big.bin"), 5 * 1024 * 1024 * 1024);

    assert!(
        client
            .read_range("sizes/empty.txt", 0, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let end = 5 * 1024 * 1024 * 1024 - 4;
    assert_eq!(
        client.read_range("sizes/big.bin", end, 100).await.unwrap(),
        b"tail",
        "an offset past four gigabytes is read where it is"
    );

    let local = fixture.dir.join("empty-back.txt");
    client
        .download("sizes/empty.txt", &local, None, None)
        .await
        .expect("an empty file downloads");
    assert_eq!(std::fs::metadata(&local).unwrap().len(), 0);
}

// Another program holding a file open must cost that file, with a reason —
// not the folder it is in.
#[cfg(windows)]
#[tokio::test]
async fn a_file_another_program_holds_still_lists_and_reports_why_it_cannot_open() {
    use std::os::windows::fs::OpenOptionsExt;
    let (fixture, client) = start().await;
    fixture.put("busy/held.txt", b"held");
    fixture.put("busy/free.txt", b"free");
    let _held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(fixture.vault().join("busy/held.txt"))
        .unwrap();

    let listed = names(&client.list("busy").await.expect("the folder still lists"));
    assert!(
        listed.contains(&"held.txt".to_string()) && listed.contains(&"free.txt".to_string()),
        "{listed:?}"
    );
    assert_eq!(
        client.read_range("busy/free.txt", 0, 10).await.unwrap(),
        b"free"
    );

    let err = client
        .read_range("busy/held.txt", 0, 10)
        .await
        .expect_err("a held file cannot be read");
    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("in use")
            || message.contains("another program")
            || message.contains("locked"),
        "the reason names the cause: {err}"
    );
}

// Explorer deletes a read-only file after asking; so does Basalt, which has
// already asked by the time it gets here.
#[tokio::test]
async fn a_read_only_file_can_be_renamed_and_deleted() {
    let (fixture, client) = start().await;
    fixture.put("ro/locked-down.txt", b"ro");
    let path = fixture.vault().join("ro/locked-down.txt");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).unwrap();

    let listed = client.list("ro").await.unwrap();
    assert!(listed[0].readonly, "shown as read-only");
    client
        .rename("ro/locked-down.txt", "ro/renamed.txt")
        .await
        .expect("renames");
    client
        .remove("ro/renamed.txt", false)
        .await
        .expect("deletes");
    assert!(client.list("ro").await.unwrap().is_empty());
}

// A link inside the shared drive pointing outside it is the classic way out
// of a shared folder. Listed as what it is, and never followed out.
#[cfg(windows)]
#[tokio::test]
async fn a_folder_link_pointing_outside_the_drive_is_not_followed() {
    let (fixture, client) = start().await;
    let outside = fixture.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
    let link = fixture.vault().join("escape");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "mklink: {}",
        String::from_utf8_lossy(&made.stderr)
    );

    assert!(
        client.list("escape").await.is_err(),
        "the link is not a way out"
    );
    assert!(client.read_range("escape/secret.txt", 0, 10).await.is_err());
    let _ = std::fs::remove_dir(&link);
}

// Keep the fixture's host alive for the whole test even where it is unused.
#[allow(dead_code)]
fn _keep(f: &Fixture) -> (&Arc<Host>, SocketAddr) {
    (&f.host, f.addr)
}

// Windows' hidden and system items: marked in a listing, so the apps can
// leave them out as Explorer does, and never searched for photos or films.
#[cfg(windows)]
#[tokio::test]
async fn hidden_items_are_marked_and_kept_out_of_the_collections() {
    let (fixture, client) = start().await;
    fixture.put("desktop.ini", b"[.ShellClassInfo]");
    fixture.put("AppData/cache/thumb 1.jpg", b"not a photo anyone took");
    fixture.put("Photos/beach.jpg", b"a real photo");
    for target in ["desktop.ini", "AppData"] {
        let made = std::process::Command::new("attrib")
            .args(["+h", "+s"])
            .arg(fixture.vault().join(target))
            .output()
            .unwrap();
        assert!(made.status.success(), "attrib {target}");
    }

    let listed = client.list("").await.unwrap();
    let hidden = |name: &str| listed.iter().find(|e| e.name == name).map(|e| e.hidden);
    assert_eq!(hidden("desktop.ini"), Some(true));
    assert_eq!(hidden("AppData"), Some(true));
    assert_eq!(
        hidden("Photos"),
        Some(false),
        "an ordinary folder is not hidden"
    );

    // Asked to look again, then waited for.
    fixture.host.start_scan();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut photos = Vec::new();
    while Instant::now() < deadline {
        let response = client.collections(0).await.expect("the collections answer");
        if !response.scanning {
            photos = response
                .collections
                .map(|c| c.photos.into_iter().map(|p| p.path).collect::<Vec<_>>())
                .unwrap_or_default();
            if !photos.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        photos,
        vec!["Photos/beach.jpg".to_string()],
        "nothing from a hidden folder"
    );
}
