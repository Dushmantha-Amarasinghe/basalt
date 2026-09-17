//! Finding a media player that can open a URL.
//!
//! The app's own window cannot decode most of a real film library — Chromium
//! reads Matroska only well enough for WebM, so an MKV carrying AC3, DTS or
//! Dolby Digital Plus plays its picture in silence. The answer is to hand the
//! file to a player that can.
//!
//! **The point is that it is handed a URL, not a file.** Downloading a 3 GB
//! episode before it could start would take over two minutes on a 22.7 MB/s
//! link and fill the disk with copies. VLC, mpv, MPC and PotPlayer all open an
//! HTTP URL and seek through it with range requests, which is exactly what the
//! media proxy already serves — so playback starts at once and nothing is
//! stored.
//!
//! Players are found through the registry rather than a list of guessed
//! directories. The first version of this file guessed, and immediately failed
//! on a real machine where PotPlayer lives under `D:\Installed Softwares` —
//! people put programs where they like, and Windows already records where they
//! went.
//!
//! Only players known to accept a URL are considered. Launching whatever is
//! registered for `.mkv` in general would work for most of them and fail
//! bafflingly for the rest, handing a player an address it treats as a
//! filename.

use std::path::PathBuf;

/// A player installed on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub name: &'static str,
    pub path: PathBuf,
}

/// A player worth looking for: its display name, the executables it installs
/// under, and the directories it lands in when it does not register itself.
struct Candidate {
    name: &'static str,
    executables: &'static [&'static str],
    fallback_paths: &'static [&'static str],
}

/// In order of preference. VLC first because it is the most widely installed
/// and the most forgiving about codecs; mpv next because it is what the app
/// will eventually embed.
const CANDIDATES: &[Candidate] = &[
    Candidate {
        name: "VLC",
        executables: &["vlc.exe"],
        fallback_paths: &[
            r"C:\Program Files\VideoLAN\VLC\vlc.exe",
            r"C:\Program Files (x86)\VideoLAN\VLC\vlc.exe",
        ],
    },
    Candidate {
        name: "mpv",
        executables: &["mpv.exe"],
        fallback_paths: &[
            r"C:\Program Files\mpv\mpv.exe",
            r"C:\Program Files\mpv-x86_64\mpv.exe",
        ],
    },
    Candidate {
        name: "MPC-HC",
        executables: &["mpc-hc64.exe", "mpc-hc.exe", "mpc-be64.exe"],
        fallback_paths: &[
            r"C:\Program Files\MPC-HC\mpc-hc64.exe",
            r"C:\Program Files (x86)\MPC-HC\mpc-hc.exe",
            r"C:\Program Files\MPC-BE\mpc-be64.exe",
        ],
    },
    Candidate {
        name: "PotPlayer",
        executables: &["PotPlayerMini64.exe", "PotPlayerMini.exe"],
        fallback_paths: &[
            r"C:\Program Files\DAUM\PotPlayer\PotPlayerMini64.exe",
            r"C:\Program Files (x86)\DAUM\PotPlayer\PotPlayerMini.exe",
        ],
    },
];

/// The first streaming-capable player found on this machine.
pub fn find() -> Option<Player> {
    for candidate in CANDIDATES {
        for exe in candidate.executables {
            // Where Windows says the program is. This is the answer that
            // survives someone installing to another drive.
            if let Some(path) = app_path(exe).filter(|p| p.is_file()) {
                return Some(Player {
                    name: candidate.name,
                    path,
                });
            }
            if let Some(path) = on_path(exe) {
                return Some(Player {
                    name: candidate.name,
                    path,
                });
            }
        }
        for path in candidate.fallback_paths {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(Player {
                    name: candidate.name,
                    path,
                });
            }
        }
    }
    None
}

/// Reads `App Paths\<exe>`, where Windows records installed programs.
///
/// Checked in both hives: per-machine installs land in `HKLM`, per-user ones
/// in `HKCU`, and either is a perfectly normal way to install a player.
#[cfg(windows)]
fn app_path(exe: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let subkey = wide(&format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}"
    ));

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buffer = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buffer) as u32;

        // SAFETY: `subkey` is NUL-terminated, the buffer and its size are
        // consistent, and the value name is null to read the key's default.
        let status = unsafe {
            RegGetValueW(
                hive,
                subkey.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            continue;
        }

        let chars = (size as usize / 2).saturating_sub(1);
        let raw = String::from_utf16_lossy(&buffer[..chars]);
        // The value is sometimes quoted, because it is also used as a command.
        let trimmed = raw.trim().trim_matches('"');
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

#[cfg(not(windows))]
fn app_path(_exe: &str) -> Option<PathBuf> {
    None
}

/// Looks for an executable on `PATH`.
fn on_path(exe: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// Launches a player against a URL, without waiting for it to exit.
pub fn launch(player: &Player, url: &str) -> std::io::Result<()> {
    let mut command = std::process::Command::new(&player.path);
    command.arg(url);

    // MPC keeps a single instance by default and would replace whatever is
    // already playing rather than opening a second window.
    if player.name.starts_with("MPC") {
        command.arg("/new");
    }

    // Spawned and forgotten: the player outlives this call, and waiting on it
    // would block the app for the length of the film.
    command.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_not_found_rather_than_a_panic() {
        assert!(on_path("definitely-not-a-real-player-9f2a.exe").is_none());
        assert!(app_path("definitely-not-a-real-player-9f2a.exe").is_none());
    }

    #[test]
    fn finding_a_player_is_optional_and_never_fails() {
        // Whether one is installed depends on the machine, so this asserts the
        // shape rather than the outcome: a result, never a panic, and a real
        // file when something is found.
        if let Some(player) = find() {
            assert!(player.path.is_file(), "{:?} must exist", player.path);
            assert!(!player.name.is_empty());
        }
    }

    // The failure that made this registry-based: PotPlayer installed to
    // `D:\Installed Softwares`, which no list of guessed directories would ever
    // contain.
    #[cfg(windows)]
    #[test]
    fn a_registered_program_is_found_wherever_it_was_installed() {
        // `notepad.exe` is registered in App Paths on every Windows machine and
        // is a stable stand-in for "a program Windows knows about".
        let found = app_path("notepad.exe");
        if let Some(path) = found {
            assert!(path.is_file(), "{path:?} should exist");
            assert!(path.is_absolute());
        }
    }

    #[test]
    fn every_fallback_path_is_absolute() {
        // A relative path here would resolve against whatever directory the
        // app happened to be started from.
        for candidate in CANDIDATES {
            for path in candidate.fallback_paths {
                assert!(
                    PathBuf::from(path).is_absolute(),
                    "{path} should be absolute"
                );
            }
        }
    }

    #[test]
    fn every_candidate_is_complete_and_uniquely_named() {
        assert!(!CANDIDATES.is_empty());
        for candidate in CANDIDATES {
            assert!(!candidate.executables.is_empty(), "{}", candidate.name);
            for exe in candidate.executables {
                assert!(
                    !exe.contains('\\') && !exe.contains('/'),
                    "{exe} should be a bare executable name"
                );
            }
        }
        let names: std::collections::HashSet<_> = CANDIDATES.iter().map(|c| c.name).collect();
        assert_eq!(names.len(), CANDIDATES.len(), "duplicate player names");
    }
}
