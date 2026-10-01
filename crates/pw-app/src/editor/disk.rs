//! Disk changes, read off the UI thread.
//!
//! The watcher reports batches of changed paths (ADR 0009). Re-listing their folders and reading the
//! open files among them is I/O, so a batch goes in three steps: the editor plans what to read
//! ([`EditorView::plan_disk`], no I/O), a background thread reads it ([`Plan::read`]), and the editor
//! applies what came back ([`EditorView::apply_disk`]).
//!
//! Each result carries the stamp of the listing or document it would update, taken when the batch was
//! planned. A document saved, reloaded or reopened since has a new stamp, so its result is stale and the
//! file is read again; a folder listed again since keeps its newer listing. Edits made meanwhile need no
//! stamp: the document decides with the text as it is then, so a dirty one gets the conflict banner, never
//! a reload.
//!
//! One batch per workspace runs at a time ([`Queue`]). Paths reported meanwhile wait, and go together in
//! the next.
//!
//! [`EditorView::plan_disk`]: super::EditorView::plan_disk
//! [`EditorView::apply_disk`]: super::EditorView::apply_disk

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::document::{Document, OpenError};
use super::explorer::{self, Relist, Relisted};

/// A new value on every call, so a listing or a document can tell whether it changed since a read began.
pub fn stamp() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What a batch reads.
pub struct Plan {
    pub(super) root: PathBuf,
    pub(super) dirs: Vec<Relist>,
    /// Open documents, with their stamps.
    pub(super) files: Vec<(PathBuf, u64)>,
}

/// What a batch read.
#[derive(Clone)]
pub struct Scan {
    pub(super) dirs: Vec<(Relist, Relisted)>,
    pub(super) files: Vec<(PathBuf, u64, Result<String, OpenError>)>,
}

impl fmt::Debug for Scan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scan").field("dirs", &self.dirs.len()).field("files", &self.files.len()).finish()
    }
}

impl Plan {
    /// Does the reading. Blocking.
    pub fn read(self) -> Scan {
        let dirs = explorer::relist(&self.root, self.dirs);
        let files = self
            .files
            .into_iter()
            .map(|(path, stamp)| {
                let read = Document::read(&self.root, &path);
                (path, stamp, read)
            })
            .collect();
        Scan { dirs, files }
    }
}

/// Changed paths waiting for a batch, and whether one is running.
#[derive(Debug, Default)]
pub struct Queue {
    pending: BTreeSet<PathBuf>,
    running: bool,
}

impl Queue {
    pub fn push(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.pending.extend(paths);
    }

    /// The paths for the next batch, unless one is running or nothing waits. Call
    /// [`finished`](Self::finished) when its results are in.
    pub fn start(&mut self) -> Option<BTreeSet<PathBuf>> {
        if self.running || self.pending.is_empty() {
            return None;
        }
        self.running = true;
        Some(std::mem::take(&mut self.pending))
    }

    pub fn finished(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_batch_runs_at_a_time_and_the_next_takes_everything_that_waited() {
        let mut queue = Queue::default();
        assert_eq!(queue.start(), None);
        queue.push(["a".into()]);
        assert_eq!(queue.start(), Some(BTreeSet::from(["a".into()])));
        queue.push(["b".into()]);
        queue.push(["c".into(), "b".into()]);
        assert_eq!(queue.start(), None, "a batch is still running");
        queue.finished();
        assert_eq!(queue.start(), Some(BTreeSet::from(["b".into(), "c".into()])));
        queue.finished();
        assert_eq!(queue.start(), None);
    }
}
