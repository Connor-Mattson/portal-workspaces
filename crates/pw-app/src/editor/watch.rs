//! Watching the folders the editor shows, for changes made by agents, builds and `git`.
//!
//! Watches are targeted and non-recursive: the root, every folder open in the file tree, and the
//! folders of open files. A recursive watch would walk `node_modules` and `target` and can run
//! out of inotify watches. Raw events go to one coalescing thread, which waits for 150 ms of quiet
//! and delivers one batch of changed paths per workspace, so an agent rewriting ten files, or a
//! `git checkout`, is one update rather than hundreds. Nothing is polled.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use iced::Subscription;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use pw_model::WorkspaceId;

use crate::inbox::{self, Inbox};

const QUIET: Duration = Duration::from_millis(150);

struct Watch {
    watcher: RecommendedWatcher,
    dirs: BTreeSet<PathBuf>,
}

pub struct FsHub {
    watches: HashMap<WorkspaceId, Watch>,
    raw: mpsc::Sender<(WorkspaceId, PathBuf)>,
    inbox: Inbox<(WorkspaceId, Vec<PathBuf>)>,
}

impl Default for FsHub {
    fn default() -> Self {
        let (out, inbox) = inbox::channel("pw-fs-events");
        let (raw, rx) = mpsc::channel::<(WorkspaceId, PathBuf)>();
        std::thread::Builder::new()
            .name("fs-coalesce".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut batch: HashMap<WorkspaceId, BTreeSet<PathBuf>> = HashMap::new();
                    batch.entry(first.0).or_default().insert(first.1);
                    while let Ok((ws, path)) = rx.recv_timeout(QUIET) {
                        batch.entry(ws).or_default().insert(path);
                    }
                    for (ws, paths) in batch {
                        if out.unbounded_send((ws, paths.into_iter().collect())).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("spawn fs coalescing thread");
        Self { watches: HashMap::new(), raw, inbox }
    }
}

impl FsHub {
    /// Batches of changed absolute paths, per workspace.
    pub fn events(&self) -> Subscription<(WorkspaceId, Vec<PathBuf>)> {
        self.inbox.subscription()
    }

    /// Watches exactly `dirs` (absolute) for `workspace`, adding and removing watches as needed.
    pub fn watch(&mut self, workspace: WorkspaceId, dirs: BTreeSet<PathBuf>) {
        if !self.watches.contains_key(&workspace) {
            let raw = self.raw.clone();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    if !in_vcs_dir(&path) {
                        let _ = raw.send((workspace, path));
                    }
                }
            });
            match watcher {
                Ok(watcher) => {
                    self.watches.insert(workspace, Watch { watcher, dirs: BTreeSet::new() });
                }
                Err(err) => {
                    tracing::warn!(%err, "file watching unavailable");
                    return;
                }
            }
        }
        let watch = self.watches.get_mut(&workspace).expect("just ensured");
        for gone in watch.dirs.difference(&dirs) {
            let _ = watch.watcher.unwatch(gone);
        }
        let mut watched = BTreeSet::new();
        for dir in dirs {
            if watch.dirs.contains(&dir) {
                watched.insert(dir);
                continue;
            }
            match watch.watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watched.insert(dir);
                }
                Err(err) => tracing::debug!(%err, dir = %dir.display(), "can't watch folder"),
            }
        }
        watch.dirs = watched;
    }

    pub fn unwatch(&mut self, workspace: WorkspaceId) {
        self.watches.remove(&workspace);
    }
}

/// `.git` internals change on every git command; reacting to them would only add noise.
fn in_vcs_dir(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(n) if n == ".git"))
}
