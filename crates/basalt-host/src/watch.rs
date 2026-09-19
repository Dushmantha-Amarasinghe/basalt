//! Watching the drive, so nobody is ever looking at a stale listing.
//!
//! The filesystem is the truth. Not the host's record of what it did — the
//! filesystem. A file deleted in Explorer, overwritten by another program, or
//! renamed by a second client all arrive here through the same door, which
//! means there is no class of change that quietly fails to propagate. Reporting
//! the host's *own* operations instead would have been simpler and would have
//! been wrong exactly when it mattered.
//!
//! Two things make this usable rather than a firehose:
//!
//! - **Coalescing.** Writing one file emits a burst of events; copying a folder
//!   emits thousands. Events are collected for [`SETTLE`] and deduplicated
//!   before anyone hears about them.
//! - **A ceiling.** Unpacking an archive can produce more changes than any
//!   client wants listed. Past [`MAX_BATCH`] the whole batch collapses into a
//!   single [`Change::Resynchronise`], which says "reload, I stopped counting"
//!   rather than dropping events and leaving clients subtly wrong.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use basalt_proto::msg::Change;
use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::broadcast;

/// How long to gather events before reporting them.
///
/// Long enough that saving a file is one change rather than four, short enough
/// that a delete feels instant. A person notices about a tenth of a second.
pub const SETTLE: Duration = Duration::from_millis(120);

/// Changes in one batch past which the batch becomes "reload everything".
pub const MAX_BATCH: usize = 64;

/// How many changes a slow client may fall behind before it is told to reload.
const CHANNEL_DEPTH: usize = 256;

/// A live view of what is happening on the drive.
pub struct Watch {
    sender: broadcast::Sender<Change>,
    /// Held because dropping it stops the watcher.
    _watcher: Box<dyn Watcher + Send + Sync>,
}

impl Watch {
    /// Starts watching `root`, recursively.
    pub fn start(root: &Path) -> crate::Result<Arc<Self>> {
        let (sender, _) = broadcast::channel(CHANNEL_DEPTH);
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();

        let mut watcher = notify::recommended_watcher(move |event| {
            // A closed receiver means the host is shutting down; nothing to say.
            let _ = raw_tx.send(event);
        })
        .map_err(|e| crate::HostError::BadRequest(format!("could not watch the drive: {e}")))?;

        watcher.watch(root, RecursiveMode::Recursive).map_err(|e| {
            crate::HostError::BadRequest(format!("could not watch {}: {e}", root.display()))
        })?;

        // The notify callback is not async, so its events cross into tokio
        // through a std channel drained on a blocking thread.
        {
            let sender = sender.clone();
            let root = root.to_path_buf();
            std::thread::spawn(move || coalesce(raw_rx, &root, &sender));
        }

        Ok(Arc::new(Self {
            sender,
            _watcher: Box::new(watcher),
        }))
    }

    /// A stream of changes. Every subscriber gets every change.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.sender.subscribe()
    }

    /// Announces something the watcher cannot see for itself.
    ///
    /// The media index living beside the drive rather than on it is the only
    /// such thing today.
    pub fn announce(&self, change: Change) {
        let _ = self.sender.send(change);
    }
}

/// Drains raw events, batches them, and publishes the result.
fn coalesce(
    raw: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    root: &Path,
    sender: &broadcast::Sender<Change>,
) {
    loop {
        // Block until something happens at all, so an idle drive costs nothing.
        let first = match raw.recv() {
            Ok(event) => event,
            // The watcher was dropped: the host is going away.
            Err(_) => return,
        };

        let mut batch = vec![first];
        let deadline = std::time::Instant::now() + SETTLE;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            match raw.recv_timeout(remaining) {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }

        for change in summarise(batch, root) {
            // No subscribers is the normal state when nobody is connected.
            let _ = sender.send(change);
        }
    }
}

/// Turns a burst of raw events into the smallest set of changes that describes
/// it.
fn summarise(events: Vec<notify::Result<notify::Event>>, root: &Path) -> Vec<Change> {
    let mut created = HashSet::new();
    let mut removed = HashSet::new();
    let mut modified = HashSet::new();
    let mut overflowed = false;

    for event in events {
        let event = match event {
            Ok(event) => event,
            // The platform telling us it lost events. The only honest response
            // is to admit the gap.
            Err(_) => {
                overflowed = true;
                continue;
            }
        };

        for path in &event.paths {
            let Some(rel) = relative(root, path) else {
                continue;
            };
            // Windows writes to its own folders constantly, and a drive shared
            // whole includes all of them. Left in, that churn counts towards
            // `MAX_BATCH` and tips ordinary windows into `Resynchronise` — and
            // a resynchronise is the one change no filter downstream can
            // narrow, so every client reloads and the host rescans, over and
            // over, for files nobody can see. Dropped here, at the source,
            // rather than by each consumer deciding separately.
            if crate::media::parse::is_system(&rel) {
                continue;
            }
            match event.kind {
                EventKind::Create(_) => {
                    created.insert(rel);
                }
                EventKind::Remove(_) => {
                    removed.insert(rel);
                }
                EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    // Windows reports a rename as two paths, and which is which
                    // is not reliably ordered. Whether each end still exists
                    // is, so ask the disk rather than guessing.
                    if root.join(&rel).exists() {
                        created.insert(rel);
                    } else {
                        removed.insert(rel);
                    }
                }
                EventKind::Modify(_) => {
                    modified.insert(rel);
                }
                _ => {}
            }
        }
    }

    // A path both created and removed inside one window settled somewhere: a
    // temporary file, or an atomic replace. Report what the disk says now.
    let settled: Vec<String> = created.intersection(&removed).cloned().collect();
    for path in settled {
        created.remove(&path);
        removed.remove(&path);
        if root.join(&path).exists() {
            modified.insert(path);
        }
    }
    // Saying a file both appeared and changed is noise; appearing covers it.
    for path in &created {
        modified.remove(path);
    }

    let total = created.len() + removed.len() + modified.len();
    if overflowed || total > MAX_BATCH {
        return vec![Change::Resynchronise];
    }

    let mut out = Vec::with_capacity(total);
    out.extend(removed.into_iter().map(|path| Change::Removed { path }));
    out.extend(created.into_iter().map(|path| Change::Created { path }));
    out.extend(modified.into_iter().map(|path| Change::Modified { path }));
    out
}

/// A path inside the vault, as the wire spells it: relative, forward slashes.
///
/// Anything outside the root is dropped rather than reported. The watcher
/// should never see such a path, and a change leaking an absolute path would
/// tell a client where the drive is mounted — which is the host's business.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let text = rel.to_string_lossy().replace('\\', "/");
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "basalt-watch-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn event(kind: EventKind, paths: &[PathBuf]) -> notify::Result<notify::Event> {
        Ok(notify::Event {
            kind,
            paths: paths.to_vec(),
            attrs: Default::default(),
        })
    }

    #[test]
    fn paths_are_reported_relative_to_the_vault_with_forward_slashes() {
        let root = Path::new(r"D:\Films");
        assert_eq!(
            relative(root, Path::new(r"D:\Films\2024\a.mkv")).as_deref(),
            Some("2024/a.mkv")
        );
    }

    // A change carrying an absolute path would tell a client where the drive is
    // mounted, which is the host's business and nobody else's.
    #[test]
    fn a_path_outside_the_vault_is_not_reported_at_all() {
        let root = Path::new(r"D:\Films");
        assert_eq!(relative(root, Path::new(r"D:\Other\a.mkv")), None);
        assert_eq!(
            relative(root, root),
            None,
            "the root itself is not an entry"
        );
    }

    #[test]
    fn a_created_file_is_reported_once() {
        let dir = temp_dir();
        let path = dir.0.join("a.mkv");
        std::fs::write(&path, b"x").unwrap();

        let changes = summarise(
            vec![
                event(
                    EventKind::Create(CreateKind::File),
                    std::slice::from_ref(&path),
                ),
                event(
                    EventKind::Create(CreateKind::File),
                    std::slice::from_ref(&path),
                ),
            ],
            &dir.0,
        );
        assert_eq!(
            changes,
            vec![Change::Created {
                path: "a.mkv".into()
            }]
        );
    }

    // Saving a file emits a burst. One change is what a person did.
    #[test]
    fn a_burst_of_writes_to_one_file_collapses() {
        let dir = temp_dir();
        let path = dir.0.join("a.txt");
        std::fs::write(&path, b"x").unwrap();

        let events: Vec<_> = (0..20)
            .map(|_| {
                event(
                    EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
                    std::slice::from_ref(&path),
                )
            })
            .collect();
        assert_eq!(
            summarise(events, &dir.0),
            vec![Change::Modified {
                path: "a.txt".into()
            }]
        );
    }

    #[test]
    fn a_removed_file_is_reported_as_removed() {
        let dir = temp_dir();
        let changes = summarise(
            vec![event(
                EventKind::Remove(RemoveKind::File),
                &[dir.0.join("gone.mkv")],
            )],
            &dir.0,
        );
        assert_eq!(
            changes,
            vec![Change::Removed {
                path: "gone.mkv".into()
            }]
        );
    }

    /// Windows reports a rename as two name events whose order is not reliable.
    /// Asking the disk which end still exists is the only thing that always
    /// works.
    #[test]
    fn a_rename_becomes_a_removal_and_a_creation_decided_by_the_disk() {
        let dir = temp_dir();
        let to = dir.0.join("new.mkv");
        std::fs::write(&to, b"x").unwrap();
        let from = dir.0.join("old.mkv");

        let changes = summarise(
            vec![
                event(
                    EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                    &[from],
                ),
                event(EventKind::Modify(ModifyKind::Name(RenameMode::To)), &[to]),
            ],
            &dir.0,
        );
        assert!(changes.contains(&Change::Removed {
            path: "old.mkv".into()
        }));
        assert!(changes.contains(&Change::Created {
            path: "new.mkv".into()
        }));
    }

    // An editor writing through a temporary file creates and removes it inside
    // one window. Reporting a file that is not there would be a lie.
    #[test]
    fn a_temporary_file_that_came_and_went_is_not_reported() {
        let dir = temp_dir();
        let temp = dir.0.join("a.txt.tmp");

        let changes = summarise(
            vec![
                event(
                    EventKind::Create(CreateKind::File),
                    std::slice::from_ref(&temp),
                ),
                event(EventKind::Remove(RemoveKind::File), &[temp]),
            ],
            &dir.0,
        );
        assert!(changes.is_empty(), "got {changes:?}");
    }

    #[test]
    fn an_atomic_replace_is_reported_as_a_modification() {
        let dir = temp_dir();
        let path = dir.0.join("host.json");
        std::fs::write(&path, b"x").unwrap();

        let changes = summarise(
            vec![
                event(
                    EventKind::Remove(RemoveKind::File),
                    std::slice::from_ref(&path),
                ),
                event(EventKind::Create(CreateKind::File), &[path]),
            ],
            &dir.0,
        );
        assert_eq!(
            changes,
            vec![Change::Modified {
                path: "host.json".into()
            }],
            "the file is still there, so it changed rather than appeared"
        );
    }

    #[test]
    fn a_file_that_appeared_is_not_also_reported_as_modified() {
        let dir = temp_dir();
        let path = dir.0.join("a.mkv");
        std::fs::write(&path, b"x").unwrap();

        let changes = summarise(
            vec![
                event(
                    EventKind::Create(CreateKind::File),
                    std::slice::from_ref(&path),
                ),
                event(
                    EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
                    &[path],
                ),
            ],
            &dir.0,
        );
        assert_eq!(
            changes,
            vec![Change::Created {
                path: "a.mkv".into()
            }]
        );
    }

    /// Unpacking an archive must not become ten thousand messages. Admitting
    /// the gap is the only answer that leaves every client correct.
    #[test]
    fn an_enormous_batch_becomes_one_reload() {
        let dir = temp_dir();
        let events: Vec<_> = (0..MAX_BATCH + 1)
            .map(|i| {
                event(
                    EventKind::Create(CreateKind::File),
                    &[dir.0.join(format!("f{i}.mkv"))],
                )
            })
            .collect();
        assert_eq!(summarise(events, &dir.0), vec![Change::Resynchronise]);
    }

    /// Windows churning through its own folders must not tip an ordinary
    /// window over the ceiling.
    ///
    /// A drive shared whole includes `$RECYCLE.BIN`, `System Volume
    /// Information` and the rest, all of which Windows writes to constantly
    /// and none of which any client is looking at. Counted, they overflow the
    /// batch and it collapses to `Resynchronise` — the one change no filter
    /// downstream can narrow, so every client reloads its listing and the host
    /// rescans the drive, repeatedly, for files nobody can see.
    #[test]
    fn churn_in_windows_own_folders_is_not_reported_and_cannot_overflow() {
        let dir = temp_dir();
        let mut events: Vec<_> = (0..MAX_BATCH * 4)
            .map(|i| {
                event(
                    EventKind::Modify(ModifyKind::Any),
                    &[dir.0.join(format!("$RECYCLE.BIN/noise{i}.tmp"))],
                )
            })
            .collect();
        events.push(event(
            EventKind::Create(CreateKind::File),
            &[dir.0.join("Films/Arrival.mkv")],
        ));

        assert_eq!(
            summarise(events, &dir.0),
            vec![Change::Created {
                path: "Films/Arrival.mkv".into()
            }],
            "the one real change has to survive, and survive as itself"
        );
    }

    #[test]
    fn a_batch_at_the_limit_is_still_reported_in_full() {
        let dir = temp_dir();
        let events: Vec<_> = (0..MAX_BATCH)
            .map(|i| {
                event(
                    EventKind::Create(CreateKind::File),
                    &[dir.0.join(format!("f{i}.mkv"))],
                )
            })
            .collect();
        assert_eq!(summarise(events, &dir.0).len(), MAX_BATCH);
    }

    /// The platform saying it lost events. Pretending otherwise would leave
    /// clients quietly wrong, which is the one outcome worth avoiding.
    #[test]
    fn a_dropped_event_forces_a_reload_rather_than_being_ignored() {
        let dir = temp_dir();
        let changes = summarise(
            vec![Err(notify::Error::generic("the queue overflowed"))],
            &dir.0,
        );
        assert_eq!(changes, vec![Change::Resynchronise]);
    }

    #[tokio::test]
    async fn a_real_file_appearing_reaches_a_subscriber() {
        let dir = temp_dir();
        let watch = Watch::start(&dir.0).expect("watching a temp directory");
        let mut events = watch.subscribe();

        // The watcher needs a moment to be listening before the change happens.
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(dir.0.join("appeared.mkv"), b"hello").unwrap();

        let change = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("a change within five seconds")
            .expect("the channel stays open");

        assert!(
            matches!(&change, Change::Created { path } | Change::Modified { path } if path == "appeared.mkv"),
            "got {change:?}"
        );
    }

    #[tokio::test]
    async fn announcing_reaches_subscribers_without_touching_the_disk() {
        let dir = temp_dir();
        let watch = Watch::start(&dir.0).unwrap();
        let mut events = watch.subscribe();

        watch.announce(Change::LibraryChanged);
        assert_eq!(events.recv().await.unwrap(), Change::LibraryChanged);
    }
}
