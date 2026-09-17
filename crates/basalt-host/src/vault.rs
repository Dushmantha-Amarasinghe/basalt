//! The locked-in drive, and every operation permitted on it.
//!
//! This is the security boundary of the whole system. Everything a client can
//! ask for arrives as a relative path from an untrusted source and is turned
//! into a real path here; if that translation is wrong, nothing else in the
//! design matters.
//!
//! Two independent checks run on every path, because each has a gap the other
//! covers:
//!
//! 1. [`basalt_proto::frame::sanitize_relative_path`] rejects the syntax —
//!    `..`, absolute paths, drive letters, UNC prefixes, NUL and control
//!    characters, reserved Windows device names, trailing dots and spaces.
//! 2. The resolved path is canonicalised and required to sit under the
//!    canonical root, which is what catches symlinks and junctions that look
//!    innocent until the filesystem resolves them.
//!
//! Syntax checking alone would be defeated by a junction; canonicalisation
//! alone would be defeated by a path that never gets created. Both, always.

use std::path::{Path, PathBuf};

use basalt_proto::frame::sanitize_relative_path;
use basalt_proto::msg::{DirEntry, EntryKind};

use crate::error::{HostError, Result, from_io};

/// One shared drive.
#[derive(Debug, Clone)]
pub struct Vault {
    /// Canonical, so prefix comparisons are meaningful.
    root: PathBuf,
    name: String,
    writable: bool,
}

impl Vault {
    /// Opens a directory as a vault.
    ///
    /// Canonicalising here rather than at each use is the point: every later
    /// comparison is against a path that has already had symlinks, `.`, `..`
    /// and short 8.3 names resolved out of it.
    pub fn open(root: impl AsRef<Path>, name: impl Into<String>) -> Result<Self> {
        let root = root.as_ref();
        let canonical = root
            .canonicalize()
            .map_err(|e| from_io(&root.display().to_string(), e))?;
        if !canonical.is_dir() {
            return Err(HostError::BadRequest(format!(
                "{} is not a directory",
                root.display()
            )));
        }
        Ok(Self {
            root: canonical,
            name: name.into(),
            writable: true,
        })
    }

    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    fn require_writable(&self) -> Result<()> {
        if self.writable {
            Ok(())
        } else {
            Err(HostError::Denied("this vault is read-only".into()))
        }
    }

    /// Resolves a path that must already exist.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.is_empty() {
            return Ok(self.root.clone());
        }
        let safe = sanitize_relative_path(rel)?;
        let canonical = self
            .root
            .join(&safe)
            .canonicalize()
            .map_err(|e| from_io(&safe, e))?;
        self.contain(&canonical, &safe)?;
        Ok(canonical)
    }

    /// Resolves a path that does not exist yet, such as an upload target or a
    /// new folder.
    ///
    /// The parent must exist and must be inside the vault; the final component
    /// is appended afterwards. Canonicalising the parent is what matters — it
    /// is where a junction would be hiding.
    pub fn resolve_new(&self, rel: &str) -> Result<PathBuf> {
        let safe = sanitize_relative_path(rel)?;
        let (parent, name) = match safe.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), safe.clone()),
        };

        let parent_path = if parent.is_empty() {
            self.root.clone()
        } else {
            let canonical = self
                .root
                .join(&parent)
                .canonicalize()
                .map_err(|e| from_io(&parent, e))?;
            self.contain(&canonical, &parent)?;
            canonical
        };

        Ok(parent_path.join(name))
    }

    /// Rejects anything that resolved outside the vault.
    fn contain(&self, resolved: &Path, label: &str) -> Result<()> {
        if resolved.starts_with(&self.root) {
            Ok(())
        } else {
            Err(HostError::Denied(format!(
                "{label} resolves outside the vault"
            )))
        }
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub fn list(&self, rel: &str) -> Result<Vec<DirEntry>> {
        let dir = self.resolve(rel)?;
        let reader = std::fs::read_dir(&dir).map_err(|e| from_io(rel, e))?;

        let mut entries = Vec::new();
        for item in reader {
            // One unreadable entry must not fail the whole listing. A folder
            // containing a file Windows has locked is common, and a directory
            // that refuses to open at all is much worse than one missing a row.
            let Ok(item) = item else { continue };
            let Ok(meta) = item.metadata() else { continue };
            let Ok(name) = item.file_name().into_string() else {
                continue;
            };

            entries.push(DirEntry {
                name,
                kind: if meta.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                },
                // A directory's size would mean walking it, which a listing
                // cannot afford; the client shows a dash.
                size: if meta.is_dir() { 0 } else { meta.len() },
                mtime: mtime_of(&meta),
                readonly: meta.permissions().readonly(),
            });
        }

        // Sorted server-side so the order is stable across requests even
        // though the client sorts again for display. An unstable listing makes
        // a refresh look like the folder changed.
        entries.sort_by_key(|e| e.name.to_lowercase());
        Ok(entries)
    }

    pub fn stat(&self, rel: &str) -> Result<DirEntry> {
        let path = self.resolve(rel)?;
        let meta = std::fs::metadata(&path).map_err(|e| from_io(rel, e))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.name)
            .to_string();
        Ok(DirEntry {
            name,
            kind: if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime: mtime_of(&meta),
            readonly: meta.permissions().readonly(),
        })
    }

    /// Reads a byte range.
    ///
    /// Returns fewer bytes than asked for at the end of the file, and an empty
    /// vector for an offset past the end — the same shape as an HTTP range
    /// request, which is what the media player is built on.
    pub fn read_range(&self, rel: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};

        let path = self.resolve(rel)?;
        let mut file = std::fs::File::open(&path).map_err(|e| from_io(rel, e))?;
        let size = file.metadata().map_err(|e| from_io(rel, e))?.len();

        if offset >= size {
            return Ok(Vec::new());
        }
        let want = length.min(size - offset);

        file.seek(SeekFrom::Start(offset))
            .map_err(|e| from_io(rel, e))?;
        let mut buf = vec![0u8; want as usize];
        file.read_exact(&mut buf).map_err(|e| from_io(rel, e))?;
        Ok(buf)
    }

    // -----------------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------------

    pub fn mkdir(&self, rel: &str) -> Result<()> {
        self.require_writable()?;
        let path = self.resolve_new(rel)?;
        if path.exists() {
            return Err(HostError::Exists(rel.to_string()));
        }
        std::fs::create_dir(&path).map_err(|e| from_io(rel, e))
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.require_writable()?;
        let source = self.resolve(from)?;
        let target = self.resolve_new(to)?;
        if target.exists() {
            return Err(HostError::Exists(to.to_string()));
        }
        std::fs::rename(&source, &target).map_err(|e| from_io(from, e))
    }

    /// Duplicates a file or folder inside the vault.
    ///
    /// Host-side, because the alternative is downloading and uploading again:
    /// two crossings of a 22.7 MB/s link for bytes that never needed to leave
    /// the drive. A 2 GB film goes from about three minutes to disk speed.
    pub fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.require_writable()?;
        let source = self.resolve(from)?;
        let target = self.resolve_new(to)?;

        if target.exists() {
            return Err(HostError::Exists(to.to_string()));
        }
        // Copying a folder into itself would recurse until the disk filled.
        // `resolve` has already canonicalised both, so this comparison is
        // against real paths rather than the strings the client sent.
        if target.starts_with(&source) {
            return Err(HostError::Denied(format!(
                "{to} is inside {from}, so copying would never finish"
            )));
        }

        let meta = std::fs::metadata(&source).map_err(|e| from_io(from, e))?;
        if meta.is_dir() {
            copy_tree(&source, &target).map_err(|e| from_io(from, e))
        } else {
            std::fs::copy(&source, &target)
                .map(|_| ())
                .map_err(|e| from_io(from, e))
        }
    }

    pub fn remove(&self, rel: &str, recursive: bool) -> Result<()> {
        self.require_writable()?;
        let path = self.resolve(rel)?;
        // Removing the vault root would delete the share itself. Nothing in
        // the client offers it, which is exactly why the guard belongs here.
        if path == self.root {
            return Err(HostError::Denied("the vault root cannot be removed".into()));
        }

        let meta = std::fs::metadata(&path).map_err(|e| from_io(rel, e))?;
        if !meta.is_dir() {
            return std::fs::remove_file(&path).map_err(|e| from_io(rel, e));
        }

        if recursive {
            return std::fs::remove_dir_all(&path).map_err(|e| from_io(rel, e));
        }
        match std::fs::remove_dir(&path) {
            Ok(()) => Ok(()),
            // Windows reports a non-empty directory as `Other`, so the kind
            // cannot be matched on. Checking the directory ourselves gives the
            // client a code it can act on instead of a generic IO failure.
            Err(e) => {
                let has_children = std::fs::read_dir(&path)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
                if has_children {
                    Err(HostError::NotEmpty(rel.to_string()))
                } else {
                    Err(from_io(rel, e))
                }
            }
        }
    }

    /// Free and total bytes on the volume holding the vault.
    pub fn space(&self) -> (u64, u64) {
        crate::space::for_path(&self.root)
    }
}

/// Copies a directory and everything under it.
///
/// Iterative rather than recursive: a deeply nested tree would otherwise be
/// able to exhaust the stack, and a drive full of someone else's folders is
/// not something to take on trust.
fn copy_tree(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut pending = vec![(source.to_path_buf(), target.to_path_buf())];

    while let Some((from, to)) = pending.pop() {
        std::fs::create_dir_all(&to)?;
        for entry in std::fs::read_dir(&from)? {
            // One unreadable entry does not abandon the copy, for the same
            // reason a listing tolerates one: a single locked file in a large
            // folder should not mean nothing gets copied.
            let Ok(entry) = entry else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            let destination = to.join(entry.file_name());

            if meta.is_dir() {
                pending.push((entry.path(), destination));
            } else {
                std::fs::copy(entry.path(), destination)?;
            }
        }
    }
    Ok(())
}

/// Modification time as Unix seconds, zero when the filesystem will not say.
fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    let Ok(time) = meta.modified() else { return 0 };
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // Files dated before 1970 exist, usually from a bad clock. Reporting
        // the negative offset is more honest than clamping to the epoch.
        Err(e) => -(e.duration().as_secs() as i64),
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

    /// A vault with a small tree in it, in a uniquely named temp directory.
    fn temp_vault() -> TempVault {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let dir = std::env::temp_dir().join(format!(
            "basalt-vault-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("films")).unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        std::fs::write(dir.join("notes.txt"), b"hello world").unwrap();
        std::fs::write(dir.join("films").join("a.mkv"), vec![7u8; 5000]).unwrap();

        let vault = Vault::open(&dir, "Vault").unwrap();
        TempVault { dir, vault }
    }

    #[test]
    fn opening_a_missing_directory_fails_rather_than_creating_it() {
        let missing = std::env::temp_dir().join("basalt-does-not-exist-9e3f");
        assert!(Vault::open(&missing, "Vault").is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn opening_a_file_as_a_vault_is_refused() {
        let t = temp_vault();
        assert!(Vault::open(t.dir.join("notes.txt"), "Vault").is_err());
    }

    #[test]
    fn the_root_lists() {
        let t = temp_vault();
        let names: Vec<_> = t
            .vault
            .list("")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["empty", "films", "notes.txt"]);
    }

    #[test]
    fn listings_carry_kind_and_size() {
        let t = temp_vault();
        let entries = t.vault.list("").unwrap();
        let notes = entries.iter().find(|e| e.name == "notes.txt").unwrap();
        assert_eq!(notes.kind, EntryKind::File);
        assert_eq!(notes.size, 11);

        let films = entries.iter().find(|e| e.name == "films").unwrap();
        assert_eq!(films.kind, EntryKind::Dir);
        assert_eq!(films.size, 0, "a directory reports no size");
    }

    #[test]
    fn subdirectories_list() {
        let t = temp_vault();
        let entries = t.vault.list("films").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.mkv");
    }

    #[test]
    fn listing_something_that_is_not_there_is_not_found() {
        let t = temp_vault();
        assert_eq!(
            t.vault.list("nope").unwrap_err().code(),
            basalt_proto::ErrorCode::NotFound
        );
    }

    // --- containment -------------------------------------------------------

    #[test]
    fn traversal_cannot_escape_the_vault() {
        let t = temp_vault();
        for escape in [
            "../outside.txt",
            "films/../../outside.txt",
            "..",
            "../",
            "films/../..",
        ] {
            assert!(
                t.vault.resolve(escape).is_err(),
                "{escape:?} must not resolve"
            );
            assert!(
                t.vault.resolve_new(escape).is_err(),
                "{escape:?} must not resolve as a new path either"
            );
        }
    }

    #[test]
    fn absolute_and_device_paths_are_refused() {
        let t = temp_vault();
        for bad in [
            "C:/Windows/win.ini",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "con",
            "nul.txt",
            "a\0b",
        ] {
            assert!(t.vault.resolve(bad).is_err(), "{bad:?} must not resolve");
            assert!(
                t.vault.resolve_new(bad).is_err(),
                "{bad:?} must not resolve as a new path"
            );
        }
    }

    #[test]
    fn an_empty_path_means_the_root_itself() {
        let t = temp_vault();
        assert_eq!(t.vault.resolve("").unwrap(), *t.vault.root());
    }

    #[test]
    fn backslashes_are_accepted_because_windows_clients_send_them() {
        let t = temp_vault();
        assert!(t.vault.resolve("films\\a.mkv").is_ok());
    }

    #[test]
    fn a_new_path_resolves_under_an_existing_parent() {
        let t = temp_vault();
        let target = t.vault.resolve_new("films/new.mkv").unwrap();
        assert!(target.starts_with(t.vault.root()));
        assert!(!target.exists());
    }

    #[test]
    fn a_new_path_under_a_missing_parent_is_not_found() {
        let t = temp_vault();
        assert_eq!(
            t.vault.resolve_new("nowhere/new.mkv").unwrap_err().code(),
            basalt_proto::ErrorCode::NotFound
        );
    }

    // --- ranged reads ------------------------------------------------------

    #[test]
    fn a_whole_file_reads_back() {
        let t = temp_vault();
        assert_eq!(
            t.vault.read_range("notes.txt", 0, 1000).unwrap(),
            b"hello world"
        );
    }

    #[test]
    fn ranges_land_on_the_right_bytes() {
        let t = temp_vault();
        assert_eq!(t.vault.read_range("notes.txt", 0, 5).unwrap(), b"hello");
        assert_eq!(t.vault.read_range("notes.txt", 6, 5).unwrap(), b"world");
        assert_eq!(t.vault.read_range("notes.txt", 5, 1).unwrap(), b" ");
    }

    #[test]
    fn a_range_that_runs_off_the_end_is_truncated_not_padded() {
        let t = temp_vault();
        // The player asks for a fixed window and expects a short answer at the
        // end of the file; padding with zeros would corrupt the last frame.
        assert_eq!(t.vault.read_range("notes.txt", 8, 999).unwrap(), b"rld");
    }

    #[test]
    fn a_range_starting_past_the_end_is_empty_rather_than_an_error() {
        let t = temp_vault();
        assert!(t.vault.read_range("notes.txt", 11, 10).unwrap().is_empty());
        assert!(
            t.vault
                .read_range("notes.txt", 9_999, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_offset_of_a_file_reads_correctly() {
        let t = temp_vault();
        let whole = t.vault.read_range("films/a.mkv", 0, u64::MAX).unwrap();
        assert_eq!(whole.len(), 5000);
        for (offset, len) in [(0, 1), (1, 4096), (4999, 1), (2500, 2500), (4096, 1000)] {
            let part = t.vault.read_range("films/a.mkv", offset, len).unwrap();
            let end = ((offset + len) as usize).min(whole.len());
            assert_eq!(
                part,
                &whole[offset as usize..end],
                "offset {offset} len {len}"
            );
        }
    }

    // --- mutations ---------------------------------------------------------

    #[test]
    fn directories_are_created_and_refuse_to_collide() {
        let t = temp_vault();
        t.vault.mkdir("fresh").unwrap();
        assert!(t.dir.join("fresh").is_dir());
        assert_eq!(
            t.vault.mkdir("fresh").unwrap_err().code(),
            basalt_proto::ErrorCode::Exists
        );
    }

    #[test]
    fn renaming_moves_a_file_and_refuses_to_overwrite() {
        let t = temp_vault();
        t.vault.rename("notes.txt", "films/notes.txt").unwrap();
        assert!(t.dir.join("films").join("notes.txt").exists());
        assert!(!t.dir.join("notes.txt").exists());

        std::fs::write(t.dir.join("other.txt"), b"x").unwrap();
        assert_eq!(
            t.vault
                .rename("other.txt", "films/notes.txt")
                .unwrap_err()
                .code(),
            basalt_proto::ErrorCode::Exists
        );
    }

    #[test]
    fn renaming_cannot_move_a_file_out_of_the_vault() {
        let t = temp_vault();
        assert!(t.vault.rename("notes.txt", "../escaped.txt").is_err());
        assert!(
            t.dir.join("notes.txt").exists(),
            "the file must still be here"
        );
    }

    #[test]
    fn files_and_empty_directories_are_removed() {
        let t = temp_vault();
        t.vault.remove("notes.txt", false).unwrap();
        assert!(!t.dir.join("notes.txt").exists());
        t.vault.remove("empty", false).unwrap();
        assert!(!t.dir.join("empty").exists());
    }

    #[test]
    fn a_full_directory_needs_the_recursive_flag() {
        let t = temp_vault();
        assert_eq!(
            t.vault.remove("films", false).unwrap_err().code(),
            basalt_proto::ErrorCode::NotEmpty
        );
        assert!(t.dir.join("films").exists(), "nothing was deleted");

        t.vault.remove("films", true).unwrap();
        assert!(!t.dir.join("films").exists());
    }

    #[test]
    fn the_vault_root_cannot_be_removed() {
        let t = temp_vault();
        assert!(t.vault.remove("", true).is_err());
        assert!(t.dir.exists());
    }

    // --- copying -----------------------------------------------------------

    #[test]
    fn a_file_copies_and_leaves_the_original() {
        let t = temp_vault();
        t.vault.copy("notes.txt", "notes-copy.txt").unwrap();
        assert_eq!(
            std::fs::read(t.dir.join("notes-copy.txt")).unwrap(),
            b"hello world"
        );
        assert!(t.dir.join("notes.txt").exists(), "the original stays put");
    }

    #[test]
    fn a_folder_copies_with_everything_under_it() {
        let t = temp_vault();
        std::fs::create_dir_all(t.dir.join("films").join("nested").join("deep")).unwrap();
        std::fs::write(
            t.dir
                .join("films")
                .join("nested")
                .join("deep")
                .join("x.txt"),
            b"buried",
        )
        .unwrap();

        t.vault.copy("films", "films-copy").unwrap();

        assert_eq!(
            std::fs::read(t.dir.join("films-copy").join("a.mkv"))
                .unwrap()
                .len(),
            5000
        );
        assert_eq!(
            std::fs::read(
                t.dir
                    .join("films-copy")
                    .join("nested")
                    .join("deep")
                    .join("x.txt")
            )
            .unwrap(),
            b"buried"
        );
    }

    #[test]
    fn copying_onto_something_that_exists_is_refused() {
        let t = temp_vault();
        assert_eq!(
            t.vault.copy("notes.txt", "films").unwrap_err().code(),
            basalt_proto::ErrorCode::Exists
        );
    }

    // Without the containment check this fills the drive.
    #[test]
    fn a_folder_cannot_be_copied_inside_itself() {
        let t = temp_vault();
        assert!(t.vault.copy("films", "films/inner").is_err());
        assert!(!t.dir.join("films").join("inner").exists());
    }

    #[test]
    fn copying_cannot_read_or_write_outside_the_vault() {
        let t = temp_vault();
        assert!(t.vault.copy("../outside.txt", "stolen.txt").is_err());
        assert!(t.vault.copy("notes.txt", "../escaped.txt").is_err());
        assert!(!t.dir.parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn copying_something_missing_is_not_found() {
        let t = temp_vault();
        assert_eq!(
            t.vault.copy("nope.txt", "copy.txt").unwrap_err().code(),
            basalt_proto::ErrorCode::NotFound
        );
    }

    #[test]
    fn an_empty_folder_copies_as_an_empty_folder() {
        let t = temp_vault();
        t.vault.copy("empty", "empty-copy").unwrap();
        assert!(t.dir.join("empty-copy").is_dir());
        assert_eq!(
            std::fs::read_dir(t.dir.join("empty-copy")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_read_only_vault_refuses_every_mutation() {
        let t = temp_vault();
        let ro = t.vault.clone().read_only();
        assert!(!ro.writable());
        assert!(ro.mkdir("x").is_err());
        assert!(ro.rename("notes.txt", "x.txt").is_err());
        assert!(ro.remove("notes.txt", false).is_err());
        assert!(ro.copy("notes.txt", "x.txt").is_err());
        // Reading still works — that is the point of read-only.
        assert!(ro.list("").is_ok());
        assert!(ro.read_range("notes.txt", 0, 5).is_ok());
    }

    #[test]
    fn stat_describes_a_single_entry() {
        let t = temp_vault();
        let entry = t.vault.stat("films/a.mkv").unwrap();
        assert_eq!(entry.name, "a.mkv");
        assert_eq!(entry.size, 5000);
        assert_eq!(entry.kind, EntryKind::File);
    }

    #[test]
    fn space_reports_a_plausible_volume() {
        let t = temp_vault();
        let (free, total) = t.vault.space();
        assert!(total > 0, "a mounted volume has a size");
        assert!(free <= total);
    }
}
