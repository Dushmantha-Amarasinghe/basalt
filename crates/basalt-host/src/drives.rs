//! The drives this machine could share.
//!
//! The setup screen asks one question — which drive? — and a question is much
//! easier to answer from a list than from a file picker. So the host enumerates
//! volumes itself and shows them with their labels and sizes, and the picker is
//! only there for the case the list cannot cover: a folder on a drive rather
//! than the whole of it.
//!
//! Windows only in substance. The non-Windows arm returns nothing so the crate
//! still builds and tests elsewhere.

use std::path::{Path, PathBuf};

/// A volume offered on the setup screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// `E:\`, ready to hand to [`crate::vault::Vault::open`].
    pub path: PathBuf,
    /// The volume label, or an empty string if it has none.
    pub label: String,
    /// `fixed`, `removable`, `network`, `cdrom`, or `other`.
    pub kind: &'static str,
    pub free: u64,
    pub total: u64,
}

impl Drive {
    /// What to call this drive when it has nothing better.
    ///
    /// A blank label is common on a freshly formatted USB stick, and "Local
    /// Disk (E:)" reads better than "(E:)" on its own.
    pub fn display_name(&self) -> String {
        let letter = self
            .path
            .to_string_lossy()
            .chars()
            .next()
            .unwrap_or('?')
            .to_string();
        if self.label.trim().is_empty() {
            let noun = match self.kind {
                "removable" => "Removable Disk",
                "network" => "Network Drive",
                "cdrom" => "Disc Drive",
                _ => "Local Disk",
            };
            format!("{noun} ({letter}:)")
        } else {
            format!("{} ({}:)", self.label.trim(), letter)
        }
    }
}

/// Every mounted volume, in drive-letter order.
///
/// Volumes that will not report a size are still listed: an empty card reader
/// slot is worth showing as unavailable rather than silently omitting, because
/// a user who expects to see E: and does not would otherwise have no idea why.
pub fn list() -> Vec<Drive> {
    #[cfg(windows)]
    {
        windows_drives()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Whether a path can actually be served right now.
///
/// Used before locking one in, so choosing a drive that has been unplugged
/// since the list was drawn fails with something the user can act on rather
/// than a vault that opens onto nothing.
pub fn is_available(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// A path the way a person writes it.
///
/// A canonical Windows path comes back as `\\?\D:\`, which is correct and looks
/// like something has gone wrong. The host's window showed exactly that under
/// the drive's name.
pub fn display(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

#[cfg(windows)]
fn windows_drives() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE,
    };

    // SAFETY: no arguments, no out-parameters; a bitmask of present letters.
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();

    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let letter = (b'A' + index as u8) as char;
        let root = format!("{letter}:\\");
        let wide = wide(&root);

        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let kind = match unsafe { GetDriveTypeW(wide.as_ptr()) } {
            DRIVE_FIXED => "fixed",
            DRIVE_REMOVABLE => "removable",
            DRIVE_REMOTE => "network",
            DRIVE_CDROM => "cdrom",
            _ => "other",
        };

        let path = PathBuf::from(&root);
        let (free, total) = crate::space::for_path(&path);
        drives.push(Drive {
            path,
            label: volume_label(&wide).unwrap_or_default(),
            kind,
            free,
            total,
        });
    }

    drives
}

#[cfg(windows)]
fn volume_label(root: &[u16]) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let mut label = [0u16; 256];

    // SAFETY: `root` is NUL-terminated; `label` is written with at most its own
    // length; every other out-parameter is null, which this API accepts.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        // An empty card reader slot fails here. Not an error worth reporting —
        // the drive is listed with no label and no size, which is the truth.
        return None;
    }

    let end = label.iter().position(|&c| c == 0).unwrap_or(label.len());
    Some(String::from_utf16_lossy(&label[..end]))
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_has_at_least_one_drive() {
        let drives = list();
        if cfg!(windows) {
            assert!(!drives.is_empty(), "Windows always has a system drive");
            assert!(
                drives.iter().any(|d| d.kind == "fixed" && d.total > 0),
                "at least one fixed volume should report a size"
            );
        }
    }

    #[test]
    fn every_drive_is_a_root_path() {
        for drive in list() {
            let text = drive.path.to_string_lossy().into_owned();
            assert_eq!(text.len(), 3, "expected a root like E:\\, got {text}");
            assert!(text.ends_with('\\'), "{text}");
        }
    }

    #[test]
    fn a_drive_without_a_label_still_has_something_to_call_it() {
        let drive = Drive {
            path: PathBuf::from("E:\\"),
            label: String::new(),
            kind: "removable",
            free: 0,
            total: 0,
        };
        assert_eq!(drive.display_name(), "Removable Disk (E:)");
    }

    #[test]
    fn a_label_is_preferred_and_trimmed() {
        let drive = Drive {
            path: PathBuf::from("E:\\"),
            label: "  Films  ".into(),
            kind: "fixed",
            free: 0,
            total: 0,
        };
        assert_eq!(drive.display_name(), "Films (E:)");
    }

    #[test]
    fn a_path_is_shown_the_way_a_person_writes_it() {
        assert_eq!(display(Path::new(r"\\?\D:\")), r"D:\");
        assert_eq!(display(Path::new(r"\\?\D:\Films")), r"D:\Films");
        assert_eq!(display(Path::new(r"\\?\UNC\nas\share")), r"\\nas\share");
        assert_eq!(display(Path::new(r"E:\")), r"E:\");
    }

    #[test]
    fn a_drive_that_is_not_there_is_not_available() {
        assert!(!is_available(Path::new(
            "Z:\\definitely\\not\\mounted\\8f2a"
        )));
        assert!(is_available(&std::env::temp_dir()));
    }

    #[test]
    fn a_file_is_not_a_drive() {
        // `Vault::open` would refuse this anyway, but saying so at the point of
        // choosing gives a much better message than "could not open the vault".
        let file = std::env::temp_dir().join("basalt-drives-probe.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(!is_available(&file));
        let _ = std::fs::remove_file(&file);
    }
}
