//! An open file: what it is, how it relates to the copy on disk, and its undo history.
//!
//! A document's text lives in the views (tabs) that show it, all kept identical by
//! `EditorView::change`. The document decides what disk changes mean (see `on_disk_read`) and
//! never overwrites either side silently.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pw_code::{Indent, Language};

use super::history::History;

/// Larger files aren't opened, rather than freezing the editor.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Why a file can't be shown as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    Missing,
    Folder,
    TooLarge,
    Binary,
    NotUtf8,
    Io(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenError::Missing => "it doesn't exist",
            OpenError::Folder => "it's a folder",
            OpenError::TooLarge => "it's larger than 8 MB",
            OpenError::Binary => "it isn't a text file",
            OpenError::NotUtf8 => "it isn't UTF-8 text",
            OpenError::Io(e) => e,
        })
    }
}

impl OpenError {
    /// Whether the file exists but isn't text, so another app may open it.
    pub fn openable_elsewhere(&self) -> bool {
        matches!(self, OpenError::TooLarge | OpenError::Binary | OpenError::NotUtf8)
    }
}

/// What a change on disk means for an open document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// Disk matches what we last saw (including our own saves).
    Unchanged,
    /// No local edits: show this text (one undoable step), then call [`Document::synced`].
    Changed(String),
    /// Local edits and disk both changed. Nothing was overwritten; the user picks.
    Conflict,
    Deleted,
}

#[derive(Debug)]
pub struct Document {
    /// Relative to the workspace root.
    pub path: PathBuf,
    pub language: Language,
    pub indent: Indent,
    /// Unsaved edits.
    pub dirty: bool,
    /// The file changed on disk while there were unsaved edits.
    pub conflict: bool,
    /// The file is gone from disk.
    pub deleted: bool,
    pub history: History,
    /// The text as last read from or written to disk (LF line endings).
    disk: String,
    /// The file used CRLF; it is edited as LF and written back as CRLF.
    crlf: bool,
    /// Bumped on every change to the text, so derived data (find results) knows when to redo.
    revision: u64,
    /// Changes whenever the document is loaded or synced with the disk, so a disk read that started
    /// before then knows it's stale (see [`super::disk`]).
    stamp: u64,
}

impl Document {
    /// Reads a file, returning the document and its text.
    pub fn load(root: &Path, path: &Path) -> Result<(Document, String), OpenError> {
        let text = read_text(&root.join(path))?;
        let (text, crlf) = normalize(text);
        let first_line = text.lines().next();
        let language = Language::detect(path, first_line);
        let indent = pw_code::editing::detect_indent(&text, language.indent);
        let doc = Document {
            path: path.to_path_buf(),
            language,
            indent,
            dirty: false,
            conflict: false,
            deleted: false,
            history: History::default(),
            disk: text.clone(),
            crlf,
            revision: 0,
            stamp: super::disk::stamp(),
        };
        Ok((doc, text))
    }

    pub fn name(&self) -> String {
        file_name(&self.path)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn crlf(&self) -> bool {
        self.crlf
    }

    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Records that the text changed, and whether it now differs from the saved text.
    pub fn changed(&mut self, dirty: bool) {
        self.dirty = dirty;
        self.revision += 1;
    }

    pub fn matches_disk(&self, text: &str) -> bool {
        self.disk == text
    }

    /// Writes `text`, keeping the file's line endings. Saving through a conflict keeps these edits.
    pub fn save(&mut self, root: &Path, text: &str) -> io::Result<()> {
        let full = root.join(&self.path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = if self.crlf { text.replace('\n', "\r\n") } else { text.to_owned() };
        write_file(&full, bytes.as_bytes())?;
        self.synced(text.to_owned());
        Ok(())
    }

    /// The text now matches the disk (after a save or a reload).
    pub fn synced(&mut self, disk: String) {
        self.disk = disk;
        self.dirty = false;
        self.conflict = false;
        self.deleted = false;
        self.stamp = super::disk::stamp();
    }

    /// Reads `root/path` as a document's text (LF line endings). Blocking: the editor calls it off
    /// the UI thread for disk changes.
    pub fn read(root: &Path, path: &Path) -> Result<String, OpenError> {
        read_text(&root.join(path)).map(|text| normalize(text).0)
    }

    /// Decides what a fresh [`read`](Self::read) of the file means, after an outside change. It must
    /// have started after the document's last sync (same [`stamp`](Self::stamp)). `current` gives the
    /// text being shown, and is only called when the disk differs from the last text synced.
    pub fn on_disk_read(&mut self, read: Result<String, OpenError>, current: impl FnOnce() -> String) -> Reload {
        let text = match read {
            Ok(text) => text,
            Err(OpenError::Missing) => {
                self.deleted = true;
                return Reload::Deleted;
            }
            // Unreadable for now (mid-write, or no longer text): wait for the next change.
            Err(_) => return Reload::Unchanged,
        };
        self.deleted = false;
        if text == self.disk {
            return Reload::Unchanged;
        }
        if text == current() {
            self.synced(text);
            return Reload::Unchanged;
        }
        if self.dirty {
            self.conflict = true;
            return Reload::Conflict;
        }
        Reload::Changed(text)
    }

    /// The disk version, to resolve a conflict in its favour (the edits stay undoable).
    pub fn disk_text(&self, root: &Path) -> Option<String> {
        Self::read(root, &self.path).ok()
    }

    /// The document moved (renamed in the tree).
    pub fn moved_to(&mut self, path: PathBuf) {
        self.path = path;
    }
}

/// Writes `bytes` to `full` so that a crash mid-save leaves the old file or the new one, never a
/// truncated mix: a temporary file in the same folder, synced, then renamed over the original.
///
/// The original's permissions and owner carry over, and a file we may not write stays unwritten. A
/// symlink is written through to its target and stays a link. Where a rename would change what the
/// file is, it's written in place instead (not atomically): a file with other hard links (a rename
/// would split it from them), one whose owner we can't keep, and a dangling symlink.
fn write_file(full: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = match fs::symlink_metadata(full) {
        Ok(meta) if meta.file_type().is_symlink() => match fs::canonicalize(full) {
            Ok(target) => target,
            Err(_) => return write_in_place(full, bytes),
        },
        _ => full.to_path_buf(),
    };
    let original = match fs::metadata(&target) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(meta) = &original {
        if meta.nlink() > 1 {
            return write_in_place(&target, bytes);
        }
        // Renaming over a read-only file would succeed; saving must fail as writing it would.
        fs::OpenOptions::new().write(true).open(&target)?;
    }

    let tmp = temp_path(&target);
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let written = (|| {
        if let Some(meta) = &original {
            // The owner first: changing it can clear the mode's setuid bits.
            if std::os::unix::fs::fchown(&file, Some(meta.uid()), Some(meta.gid())).is_err() {
                return Ok(false);
            }
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &target).map(|()| true)
    })();
    drop(file);
    match written {
        Ok(true) => {
            // Make the rename itself durable. Not every filesystem can sync a folder; the data is safe anyway.
            if let Some(dir) = target.parent() {
                let _ = fs::File::open(dir).and_then(|d| d.sync_all());
            }
            Ok(())
        }
        Ok(false) => {
            let _ = fs::remove_file(&tmp);
            write_in_place(&target, bytes)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_in_place(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// A hidden sibling of `target` no other save uses.
fn temp_path(target: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = std::ffi::OsString::from(".");
    name.push(target.file_name().unwrap_or_default());
    name.push(format!(".{}-{n}.pw-save", std::process::id()));
    target.with_file_name(name)
}

fn read_text(full: &Path) -> Result<String, OpenError> {
    let meta = fs::metadata(full).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => OpenError::Missing,
        _ => OpenError::Io(e.to_string()),
    })?;
    if meta.is_dir() {
        return Err(OpenError::Folder);
    }
    if meta.len() > MAX_BYTES {
        return Err(OpenError::TooLarge);
    }
    let bytes = fs::read(full).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => OpenError::Missing,
        _ => OpenError::Io(e.to_string()),
    })?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(OpenError::Binary);
    }
    String::from_utf8(bytes).map_err(|_| OpenError::NotUtf8)
}

/// LF text and whether the original used CRLF.
fn normalize(text: String) -> (String, bool) {
    if text.contains("\r\n") { (text.replace("\r\n", "\n"), true) } else { (text, false) }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_disk_change(doc: &mut Document, root: &Path, current: &str) -> Reload {
        let read = Document::read(root, &doc.path);
        doc.on_disk_read(read, || current.to_owned())
    }

    fn fixture(name: &str, text: &str) -> (tempfile::TempDir, Document, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), text).unwrap();
        let (doc, text) = Document::load(dir.path(), Path::new(name)).unwrap();
        (dir, doc, text)
    }

    #[test]
    fn loads_text_and_refuses_the_rest() {
        let (dir, doc, text) = fixture("main.rs", "fn main() {\r\n    x();\r\n}\r\n");
        assert_eq!(text, "fn main() {\n    x();\n}\n");
        assert!(doc.crlf());
        assert_eq!(doc.language.name, "Rust");
        assert_eq!(doc.indent, Indent::Spaces(4));
        std::fs::write(dir.path().join("bin"), [0u8, 1, 2]).unwrap();
        std::fs::write(dir.path().join("latin1"), [0xe9u8, b'a']).unwrap();
        let err = |p: &str| Document::load(dir.path(), Path::new(p)).unwrap_err();
        assert_eq!(err("bin"), OpenError::Binary);
        assert_eq!(err("latin1"), OpenError::NotUtf8);
        assert_eq!(err("gone"), OpenError::Missing);
        assert_eq!(err("."), OpenError::Folder);
    }

    #[test]
    fn saves_keep_line_endings() {
        let (dir, mut doc, _) = fixture("a.txt", "a\r\nb\r\n");
        doc.changed(true);
        doc.save(dir.path(), "a\nc\n").unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "a\r\nc\r\n");
        assert!(!doc.dirty);
        // Our own save shows up as a disk change: nothing happens.
        assert_eq!(on_disk_change(&mut doc, dir.path(), "a\nc\n"), Reload::Unchanged);
    }

    #[test]
    fn saves_replace_the_file_atomically_keeping_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut doc, _) = fixture("run.sh", "echo one\n");
        let path = dir.path().join("run.sh");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        doc.save(dir.path(), "echo two\n").unwrap();
        let meta = fs::metadata(&path).unwrap();
        assert_ne!(meta.ino(), inode, "a new file was renamed into place");
        assert_eq!(meta.permissions().mode() & 0o7777, 0o751);
        assert_eq!(fs::read_to_string(&path).unwrap(), "echo two\n");
        // No temporary file is left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn saves_write_through_symlinks_and_hard_links() {
        let (dir, mut doc, _) = fixture("real.txt", "one\n");
        std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();
        let (mut linked, _) = Document::load(dir.path(), Path::new("link.txt")).unwrap();
        linked.save(dir.path(), "two\n").unwrap();
        assert!(fs::symlink_metadata(dir.path().join("link.txt")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dir.path().join("real.txt")).unwrap(), "two\n");

        // A hard-linked file is written in place, so both names keep showing the same text.
        fs::hard_link(dir.path().join("real.txt"), dir.path().join("hard.txt")).unwrap();
        doc.save(dir.path(), "three\n").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("hard.txt")).unwrap(), "three\n");
        assert_eq!(fs::metadata(dir.path().join("real.txt")).unwrap().nlink(), 2);
    }

    #[test]
    fn failed_saves_leave_the_original_intact() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut doc, _) = fixture("a.txt", "one\n");
        let path = dir.path().join("a.txt");
        // Root writes anyway; there is nothing to test then.
        let probe = dir.path().join("probe");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let unprivileged = fs::write(&probe, "").is_err();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        if !unprivileged {
            return;
        }

        // A read-only file stays read-only, though its folder would allow replacing it.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        doc.changed(true);
        assert!(doc.save(dir.path(), "two\n").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");
        assert!(doc.dirty);

        // The temporary file can't be created: the original is untouched.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = doc.save(dir.path(), "two\n");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn disk_changes_reload_conflict_or_report_deletion() {
        let (dir, mut doc, text) = fixture("a.txt", "one\n");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "two\n").unwrap();
        assert_eq!(on_disk_change(&mut doc, dir.path(), &text), Reload::Changed("two\n".into()));
        doc.synced("two\n".into());

        doc.changed(true);
        std::fs::write(&path, "three\n").unwrap();
        assert_eq!(on_disk_change(&mut doc, dir.path(), "two!\n"), Reload::Conflict);
        assert!(doc.conflict);
        assert_eq!(doc.disk_text(dir.path()).as_deref(), Some("three\n"));

        // Keep mine: saving resolves it.
        doc.save(dir.path(), "two!\n").unwrap();
        assert!(!doc.conflict && !doc.dirty);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(on_disk_change(&mut doc, dir.path(), "two!\n"), Reload::Deleted);
        assert!(doc.deleted);
    }

    #[test]
    fn disk_catching_up_with_edits_clears_dirty() {
        let (dir, mut doc, _) = fixture("a.txt", "one\n");
        doc.changed(true);
        std::fs::write(dir.path().join("a.txt"), "same\n").unwrap();
        assert_eq!(on_disk_change(&mut doc, dir.path(), "same\n"), Reload::Unchanged);
        assert!(!doc.dirty);
    }
}
