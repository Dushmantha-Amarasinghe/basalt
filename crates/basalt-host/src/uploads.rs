//! Uploads in progress.
//!
//! An upload writes to a temporary file beside its destination and is renamed
//! into place only after its BLAKE3 matches. Two things follow from that, and
//! both matter on a Wi-Fi link that drops:
//!
//! - A failed or abandoned upload never replaces a good file. The worst case is
//!   a `.part` file left behind, not a truncated original.
//! - Resuming is just reopening the temporary file and reading its length, so
//!   it survives the host restarting and not only the connection dropping.
//!
//! The temporary file sits next to the destination rather than in a scratch
//! directory so that the final step is a rename within one volume, which is
//! atomic. A rename across volumes is a copy, and a copy of a 2 GB film at the
//! end of an upload would be both slow and a second chance to fail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use basalt_proto::hex;
use basalt_proto::msg::UPLOAD_ID_BYTES;
use tokio::io::AsyncWriteExt;

use crate::error::{HostError, Result, from_io};
use crate::vault::Vault;

/// One upload the host is holding open.
#[derive(Debug, Clone)]
pub struct Upload {
    /// Vault-relative destination.
    pub rel: String,
    pub target: PathBuf,
    pub temp: PathBuf,
    /// What the client said the finished file will be.
    pub size: u64,
    /// What has actually landed on disk.
    pub received: u64,
    pub overwrite: bool,
}

/// Every upload this host currently has open.
///
/// A single lock covers the map and is held across the write of one chunk.
/// That serialises simultaneous uploads, which sounds worse than it is: the
/// link runs at ~22.7 MB/s and is the bottleneck by a wide margin, so two
/// uploads sharing it were never going to proceed in parallel anyway.
#[derive(Debug, Default)]
pub struct Uploads {
    open: tokio::sync::Mutex<HashMap<[u8; UPLOAD_ID_BYTES], Upload>>,
}

/// The name of the temporary file for an upload.
///
/// The leading dot keeps it out of the way in Explorer, and the id makes two
/// uploads of the same destination impossible to confuse.
fn temp_name(id: &[u8; UPLOAD_ID_BYTES]) -> String {
    format!(".basalt-{}.part", hex::encode(id))
}

/// Whether a filename is one of ours.
///
/// Used to hide partial uploads from listings: a half-written file appearing in
/// the browser with a nonsense name looks like corruption.
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with(".basalt-") && name.ends_with(".part")
}

impl Uploads {
    /// Opens an upload, or reopens one that was interrupted.
    pub async fn begin(
        &self,
        vault: &Vault,
        rel: &str,
        size: u64,
        overwrite: bool,
        resume: Option<&str>,
    ) -> Result<([u8; UPLOAD_ID_BYTES], u64)> {
        let target = vault.resolve_new(rel)?;
        if !overwrite && target.exists() {
            return Err(HostError::Exists(rel.to_string()));
        }
        if target.is_dir() {
            return Err(HostError::Denied(format!("{rel} is a directory")));
        }

        let id = match resume {
            Some(hexed) => basalt_proto::msg::parse_upload_id(hexed)?,
            None => {
                let bytes = basalt_net::pairing::random_bytes(UPLOAD_ID_BYTES)
                    .map_err(|e| HostError::BadRequest(format!("could not open an upload: {e}")))?;
                let mut id = [0u8; UPLOAD_ID_BYTES];
                id.copy_from_slice(&bytes);
                id
            }
        };

        let parent = target
            .parent()
            .ok_or_else(|| HostError::BadRequest(format!("{rel} has no parent directory")))?;
        let temp = parent.join(temp_name(&id));

        // Resuming reads the length off disk rather than trusting anything
        // remembered in memory, so it works identically after a host restart.
        let received = match tokio::fs::metadata(&temp).await {
            Ok(meta) => meta.len(),
            Err(_) => {
                tokio::fs::File::create(&temp)
                    .await
                    .map_err(|e| from_io(rel, e))?;
                0
            }
        };

        // A resumed upload cannot be longer than the file it claims to be;
        // that would mean the client is resuming against a different file.
        if received > size {
            tokio::fs::remove_file(&temp).await.ok();
            return Err(HostError::BadRequest(format!(
                "the partial upload for {rel} is larger than the file it claims to be"
            )));
        }

        self.open.lock().await.insert(
            id,
            Upload {
                rel: rel.to_string(),
                target,
                temp,
                size,
                received,
                overwrite,
            },
        );
        Ok((id, received))
    }

    /// Appends one chunk.
    ///
    /// The offset must be exactly where the last chunk ended. Accepting
    /// anything else would mean a dropped chunk could leave a hole full of
    /// zeros in the middle of a file that still passed its length check — the
    /// hash would catch it at commit, but only after the whole upload.
    pub async fn write_chunk(
        &self,
        id: &[u8; UPLOAD_ID_BYTES],
        offset: u64,
        data: &[u8],
    ) -> Result<u64> {
        let mut open = self.open.lock().await;
        let upload = open
            .get_mut(id)
            .ok_or_else(|| HostError::BadRequest("no such upload".into()))?;

        if offset != upload.received {
            return Err(HostError::BadRequest(format!(
                "chunk for {} arrived at offset {offset}, expected {}",
                upload.rel, upload.received
            )));
        }
        if upload.received + data.len() as u64 > upload.size {
            return Err(HostError::BadRequest(format!(
                "{} would exceed the {} bytes it was declared to be",
                upload.rel, upload.size
            )));
        }

        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&upload.temp)
            .await
            .map_err(|e| from_io(&upload.rel, e))?;
        file.write_all(data)
            .await
            .map_err(|e| from_io(&upload.rel, e))?;
        file.flush().await.map_err(|e| from_io(&upload.rel, e))?;

        upload.received += data.len() as u64;
        Ok(upload.received)
    }

    /// Verifies and moves the file into place.
    pub async fn commit(
        &self,
        id: &[u8; UPLOAD_ID_BYTES],
        expected_hash: &str,
        mtime: Option<i64>,
    ) -> Result<()> {
        let upload = {
            let mut open = self.open.lock().await;
            open.remove(id)
                .ok_or_else(|| HostError::BadRequest("no such upload".into()))?
        };

        if upload.received != upload.size {
            tokio::fs::remove_file(&upload.temp).await.ok();
            return Err(HostError::BadRequest(format!(
                "{} is {} bytes, expected {}",
                upload.rel, upload.received, upload.size
            )));
        }

        let temp = upload.temp.clone();
        let actual = tokio::task::spawn_blocking(move || hash_file(&temp))
            .await
            .map_err(|e| HostError::BadRequest(format!("hashing failed: {e}")))?
            .map_err(|e| from_io(&upload.rel, e))?;

        if !hex::constant_time_eq(actual.as_bytes(), expected_hash.as_bytes()) {
            // A file that does not hash is not written. Leaving the original
            // intact is the entire reason for the temporary file.
            tokio::fs::remove_file(&upload.temp).await.ok();
            return Err(HostError::BadRequest(format!(
                "{} did not survive the transfer intact",
                upload.rel
            )));
        }

        // Checked again here, not just when the upload opened.
        //
        // `begin` looks at the destination before a single byte has moved, and
        // on a slow link minutes pass before the file lands — plenty of time
        // for something else to take the name. Worse, `rename` on Windows
        // *replaces* silently, so without this an upload that asked not to
        // overwrite anything would quietly destroy whatever arrived while it
        // was in flight. Eight simultaneous uploads of one film demonstrated
        // it: all eight declared `overwrite: false` and all eight landed, each
        // one overwriting the last.
        if !upload.overwrite && upload.target.exists() {
            tokio::fs::remove_file(&upload.temp).await.ok();
            return Err(HostError::Exists(upload.rel.clone()));
        }

        // Anything that goes wrong from here takes the partial file with it,
        // rather than leaving a `.part` on the drive that the listing hides.
        if let Err(e) = tokio::fs::rename(&upload.temp, &upload.target).await {
            tokio::fs::remove_file(&upload.temp).await.ok();
            return Err(from_io(&upload.rel, e));
        }

        if let Some(seconds) = mtime {
            set_mtime(&upload.target, seconds);
        }
        Ok(())
    }

    /// Abandons an upload and removes its partial file.
    pub async fn abort(&self, id: &[u8; UPLOAD_ID_BYTES]) -> Result<()> {
        let upload = self.open.lock().await.remove(id);
        if let Some(upload) = upload {
            tokio::fs::remove_file(&upload.temp).await.ok();
        }
        Ok(())
    }

    pub async fn count(&self) -> usize {
        self.open.lock().await.len()
    }

    pub async fn get(&self, id: &[u8; UPLOAD_ID_BYTES]) -> Option<Upload> {
        self.open.lock().await.get(id).cloned()
    }
}

/// BLAKE3 of a whole file, read in blocks so a film does not land in memory.
fn hash_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Best effort: keep the modification time the client reported.
///
/// Failure is ignored deliberately. A file that arrived intact but kept the
/// wrong timestamp is a successful upload, and refusing it over a cosmetic
/// detail would be absurd.
fn set_mtime(path: &Path, seconds: i64) {
    let Ok(file) = std::fs::File::options().write(true).open(path) else {
        return;
    };
    let time = if seconds >= 0 {
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(seconds as u64))
    } else {
        std::time::UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(seconds.unsigned_abs()))
    };
    if let Some(time) = time {
        let _ = file.set_modified(time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempVault {
        dir: PathBuf,
        vault: Vault,
    }

    impl Drop for TempVault {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_vault() -> TempVault {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let dir = std::env::temp_dir().join(format!(
            "basalt-upload-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let vault = Vault::open(&dir, "Vault").unwrap();
        TempVault { dir, vault }
    }

    fn blake3_of(data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }

    #[tokio::test]
    async fn a_whole_upload_lands_in_place() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let data = b"the quick brown fox".to_vec();

        let (id, offset) = uploads
            .begin(&t.vault, "sub/fox.txt", data.len() as u64, false, None)
            .await
            .unwrap();
        assert_eq!(offset, 0);

        uploads.write_chunk(&id, 0, &data).await.unwrap();
        uploads.commit(&id, &blake3_of(&data), None).await.unwrap();

        assert_eq!(
            std::fs::read(t.dir.join("sub").join("fox.txt")).unwrap(),
            data
        );
        assert_eq!(uploads.count().await, 0);
    }

    #[tokio::test]
    async fn chunks_accumulate_in_order() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();

        let (id, _) = uploads
            .begin(&t.vault, "big.bin", data.len() as u64, false, None)
            .await
            .unwrap();

        let mut sent = 0usize;
        for chunk in data.chunks(1024) {
            uploads.write_chunk(&id, sent as u64, chunk).await.unwrap();
            sent += chunk.len();
        }
        uploads.commit(&id, &blake3_of(&data), None).await.unwrap();
        assert_eq!(std::fs::read(t.dir.join("big.bin")).unwrap(), data);
    }

    #[tokio::test]
    async fn a_chunk_at_the_wrong_offset_is_refused() {
        let t = temp_vault();
        let uploads = Uploads::default();

        let (id, _) = uploads
            .begin(&t.vault, "a.bin", 100, false, None)
            .await
            .unwrap();
        uploads.write_chunk(&id, 0, &[1u8; 10]).await.unwrap();

        // A gap would leave zeros inside the file.
        assert!(uploads.write_chunk(&id, 50, &[2u8; 10]).await.is_err());
        // So would rewinding.
        assert!(uploads.write_chunk(&id, 0, &[3u8; 10]).await.is_err());
        assert_eq!(uploads.get(&id).await.unwrap().received, 10);
    }

    #[tokio::test]
    async fn an_upload_cannot_exceed_its_declared_size() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "a.bin", 10, false, None)
            .await
            .unwrap();
        assert!(uploads.write_chunk(&id, 0, &[0u8; 11]).await.is_err());
    }

    #[tokio::test]
    async fn committing_early_fails_and_leaves_nothing_behind() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "short.bin", 100, false, None)
            .await
            .unwrap();
        uploads.write_chunk(&id, 0, &[1u8; 10]).await.unwrap();

        assert!(
            uploads
                .commit(&id, &blake3_of(&[1u8; 10]), None)
                .await
                .is_err()
        );
        assert!(!t.dir.join("short.bin").exists());
    }

    // The property the temporary file exists for.
    #[tokio::test]
    async fn a_corrupted_upload_does_not_replace_the_original() {
        let t = temp_vault();
        std::fs::write(t.dir.join("precious.txt"), b"the original").unwrap();
        let uploads = Uploads::default();

        let corrupt = b"tampered with".to_vec();
        let (id, _) = uploads
            .begin(&t.vault, "precious.txt", corrupt.len() as u64, true, None)
            .await
            .unwrap();
        uploads.write_chunk(&id, 0, &corrupt).await.unwrap();

        // Commit with the hash of what the client *meant* to send.
        assert!(
            uploads
                .commit(&id, &blake3_of(b"what was intended"), None)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(t.dir.join("precious.txt")).unwrap(),
            b"the original"
        );
    }

    #[tokio::test]
    async fn a_failed_commit_cleans_up_its_partial_file() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "a.bin", 4, false, None)
            .await
            .unwrap();
        uploads.write_chunk(&id, 0, b"abcd").await.unwrap();
        let temp = uploads.get(&id).await.unwrap().temp;

        assert!(
            uploads
                .commit(&id, &blake3_of(b"efgh"), None)
                .await
                .is_err()
        );
        assert!(!temp.exists(), "the partial file must not be left behind");
    }

    #[tokio::test]
    async fn an_interrupted_upload_resumes_where_it_stopped() {
        let t = temp_vault();
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();

        // First attempt: half the file, then the connection dies.
        let first = Uploads::default();
        let (id, _) = first
            .begin(&t.vault, "resumed.bin", data.len() as u64, false, None)
            .await
            .unwrap();
        first.write_chunk(&id, 0, &data[..2000]).await.unwrap();
        drop(first);

        // A whole new Uploads, standing in for the host having restarted.
        let second = Uploads::default();
        let (same_id, offset) = second
            .begin(
                &t.vault,
                "resumed.bin",
                data.len() as u64,
                false,
                Some(&hex::encode(&id)),
            )
            .await
            .unwrap();
        assert_eq!(same_id, id);
        assert_eq!(offset, 2000, "resume must pick up from what is on disk");

        second.write_chunk(&id, 2000, &data[2000..]).await.unwrap();
        second.commit(&id, &blake3_of(&data), None).await.unwrap();
        assert_eq!(std::fs::read(t.dir.join("resumed.bin")).unwrap(), data);
    }

    #[tokio::test]
    async fn aborting_removes_the_partial_file() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "abandoned.bin", 100, false, None)
            .await
            .unwrap();
        let temp = uploads.get(&id).await.unwrap().temp;
        uploads.write_chunk(&id, 0, &[1u8; 10]).await.unwrap();

        uploads.abort(&id).await.unwrap();
        assert!(!temp.exists());
        assert_eq!(uploads.count().await, 0);
        assert!(uploads.write_chunk(&id, 10, &[0u8; 1]).await.is_err());
    }

    // What eight duplicate uploads of one file actually did: `rename` on
    // Windows replaces silently, so all eight landed and each overwrote the
    // last — despite every one of them declaring `overwrite: false`. The
    // destination is now re-checked at commit, so only the first lands.
    #[tokio::test]
    async fn losing_the_race_for_a_destination_leaves_nothing_behind() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let data = b"the same file, eight times".to_vec();

        // Eight uploads opened before any of them commits, exactly as
        // simultaneous drops would.
        let mut ids = Vec::new();
        for _ in 0..8 {
            let (id, _) = uploads
                .begin(&t.vault, "contested.bin", data.len() as u64, false, None)
                .await
                .unwrap();
            uploads.write_chunk(&id, 0, &data).await.unwrap();
            ids.push(id);
        }

        let digest = blake3_of(&data);
        let mut winners = 0;
        for id in &ids {
            if uploads.commit(id, &digest, None).await.is_ok() {
                winners += 1;
            }
        }

        assert_eq!(
            winners, 1,
            "only the first may land; the rest asked not to overwrite"
        );
        assert_eq!(std::fs::read(t.dir.join("contested.bin")).unwrap(), data);

        let leftovers: Vec<_> = std::fs::read_dir(&t.dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| is_temp_name(n))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the seven that lost left {leftovers:?} on the drive"
        );
    }

    #[tokio::test]
    async fn an_existing_destination_needs_the_overwrite_flag() {
        let t = temp_vault();
        std::fs::write(t.dir.join("taken.txt"), b"x").unwrap();
        let uploads = Uploads::default();

        assert!(
            uploads
                .begin(&t.vault, "taken.txt", 10, false, None)
                .await
                .is_err()
        );
        assert!(
            uploads
                .begin(&t.vault, "taken.txt", 10, true, None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn an_upload_cannot_be_aimed_outside_the_vault() {
        let t = temp_vault();
        let uploads = Uploads::default();
        for escape in ["../escaped.bin", "C:/Windows/evil.bin", "sub/../../x.bin"] {
            assert!(
                uploads
                    .begin(&t.vault, escape, 10, true, None)
                    .await
                    .is_err(),
                "{escape} must be refused"
            );
        }
    }

    #[tokio::test]
    async fn an_upload_cannot_overwrite_a_directory() {
        let t = temp_vault();
        let uploads = Uploads::default();
        assert!(
            uploads
                .begin(&t.vault, "sub", 10, true, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_empty_file_uploads() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "empty.txt", 0, false, None)
            .await
            .unwrap();
        uploads.commit(&id, &blake3_of(b""), None).await.unwrap();
        assert!(t.dir.join("empty.txt").exists());
        assert_eq!(std::fs::metadata(t.dir.join("empty.txt")).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn the_modification_time_is_carried_over() {
        let t = temp_vault();
        let uploads = Uploads::default();
        let (id, _) = uploads
            .begin(&t.vault, "dated.txt", 2, false, None)
            .await
            .unwrap();
        uploads.write_chunk(&id, 0, b"hi").await.unwrap();
        uploads
            .commit(&id, &blake3_of(b"hi"), Some(1_600_000_000))
            .await
            .unwrap();

        let meta = std::fs::metadata(t.dir.join("dated.txt")).unwrap();
        let seconds = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(seconds, 1_600_000_000);
    }

    #[tokio::test]
    async fn operations_on_an_unknown_upload_are_refused() {
        let uploads = Uploads::default();
        let id = [9u8; UPLOAD_ID_BYTES];
        assert!(uploads.write_chunk(&id, 0, b"x").await.is_err());
        assert!(uploads.commit(&id, &blake3_of(b"x"), None).await.is_err());
        // Aborting something that is already gone is not an error: a client
        // cleaning up after a crash should not have to care.
        assert!(uploads.abort(&id).await.is_ok());
    }

    #[test]
    fn partial_files_are_recognisable_so_they_can_be_hidden() {
        assert!(is_temp_name(
            ".basalt-00112233445566778899aabbccddeeff.part"
        ));
        assert!(!is_temp_name("holiday.mp4"));
        assert!(!is_temp_name("notes.part"));
        assert!(!is_temp_name(".basalt-something.txt"));
    }
}
