//! An open file: what it is, how it relates to the copy on disk, and its undo history.
//!
//! A document's text lives in the views (tabs) that show it, all kept identical by
//! `EditorView::change`. The document decides what disk changes mean (see `on_disk_change`) and
//! never overwrites either side silently.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

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
            std::fs::create_dir_all(parent)?;
        }
        let bytes = if self.crlf { text.replace('\n', "\r\n") } else { text.to_owned() };
        std::fs::write(&full, bytes)?;
        self.synced(text.to_owned());
        Ok(())
    }

    /// The text now matches the disk (after a save or a reload).
    pub fn synced(&mut self, disk: String) {
        self.disk = disk;
        self.dirty = false;
        self.conflict = false;
        self.deleted = false;
    }

    /// Looks at the file on disk after an outside change. `current` is the text being shown.
    pub fn on_disk_change(&mut self, root: &Path, current: &str) -> Reload {
        let text = match read_text(&root.join(&self.path)) {
            Ok(text) => normalize(text).0,
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
        if text == current {
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
        read_text(&root.join(&self.path)).ok().map(|t| normalize(t).0)
    }

    /// The document moved (renamed in the tree).
    pub fn moved_to(&mut self, path: PathBuf) {
        self.path = path;
    }
}

fn read_text(full: &Path) -> Result<String, OpenError> {
    let meta = std::fs::metadata(full).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => OpenError::Missing,
        _ => OpenError::Io(e.to_string()),
    })?;
    if meta.is_dir() {
        return Err(OpenError::Folder);
    }
    if meta.len() > MAX_BYTES {
        return Err(OpenError::TooLarge);
    }
    let bytes = std::fs::read(full).map_err(|e| match e.kind() {
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
        assert_eq!(doc.on_disk_change(dir.path(), "a\nc\n"), Reload::Unchanged);
    }

    #[test]
    fn disk_changes_reload_conflict_or_report_deletion() {
        let (dir, mut doc, text) = fixture("a.txt", "one\n");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "two\n").unwrap();
        assert_eq!(doc.on_disk_change(dir.path(), &text), Reload::Changed("two\n".into()));
        doc.synced("two\n".into());

        doc.changed(true);
        std::fs::write(&path, "three\n").unwrap();
        assert_eq!(doc.on_disk_change(dir.path(), "two!\n"), Reload::Conflict);
        assert!(doc.conflict);
        assert_eq!(doc.disk_text(dir.path()).as_deref(), Some("three\n"));

        // Keep mine: saving resolves it.
        doc.save(dir.path(), "two!\n").unwrap();
        assert!(!doc.conflict && !doc.dirty);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(doc.on_disk_change(dir.path(), "two!\n"), Reload::Deleted);
        assert!(doc.deleted);
    }

    #[test]
    fn disk_catching_up_with_edits_clears_dirty() {
        let (dir, mut doc, _) = fixture("a.txt", "one\n");
        doc.changed(true);
        std::fs::write(dir.path().join("a.txt"), "same\n").unwrap();
        assert_eq!(doc.on_disk_change(dir.path(), "same\n"), Reload::Unchanged);
        assert!(!doc.dirty);
    }
}
