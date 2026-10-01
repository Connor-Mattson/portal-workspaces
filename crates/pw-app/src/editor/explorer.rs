//! The file tree's state: folder listings loaded as they're expanded, the selection, and the row
//! being typed into (a new file's name, or a rename).
//!
//! Only expanded folders are ever listed, and they're re-listed when the watcher reports a change
//! in them. Rows have a fixed height, so the view draws only the ones in sight.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use pw_code::fs::{self, Entry};

/// Height of one row in the tree, in logical pixels.
pub const ROW_HEIGHT: f32 = 24.0;

/// One folder's entries.
#[derive(Debug, Default)]
struct Listing {
    entries: Vec<Entry>,
    /// The folder itself is ignored, so everything in it is too.
    ignored: bool,
    error: Option<String>,
}

/// A visible row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Relative to the root.
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub ignored: bool,
}

/// A name being typed into the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    /// A new file or folder in `dir`.
    New {
        dir: PathBuf,
        folder: bool,
    },
    Rename(PathBuf),
}

#[derive(Debug, Default)]
pub struct Explorer {
    listings: HashMap<PathBuf, Listing>,
    pub expanded: BTreeSet<PathBuf>,
    pub selected: Option<PathBuf>,
    pub draft: Option<(Draft, String)>,
    pub error: Option<String>,
    /// The visible slice of the tree: scroll offset and height, in pixels.
    pub scroll: f32,
    pub viewport: f32,
}

impl Explorer {
    /// Lists the root and the folders that were open last time, in order, so ignore flags carry down.
    pub fn start(&mut self, root: &Path, expanded: &[PathBuf]) {
        self.load(root, Path::new(""));
        let mut dirs: Vec<&PathBuf> = expanded.iter().collect();
        dirs.sort_by_key(|p| p.components().count());
        for dir in dirs {
            if self.is_listed_dir(dir) {
                self.load(root, dir);
                self.expanded.insert(dir.clone());
            }
        }
    }

    /// Whether `path` is a folder in a loaded listing.
    fn is_listed_dir(&self, path: &Path) -> bool {
        self.entry(path).is_some_and(|e| e.is_dir)
    }

    fn entry(&self, path: &Path) -> Option<&Entry> {
        let parent = path.parent().unwrap_or(Path::new(""));
        let name = path.file_name()?.to_str()?;
        self.listings.get(parent)?.entries.iter().find(|e| e.name == name)
    }

    fn ignored(&self, path: &Path) -> bool {
        let parent = path.parent().unwrap_or(Path::new(""));
        self.listings.get(parent).is_some_and(|l| l.ignored) || self.entry(path).is_some_and(|e| e.ignored)
    }

    fn load(&mut self, root: &Path, dir: &Path) {
        let ignored = !dir.as_os_str().is_empty() && self.ignored(dir);
        let listing = match fs::list_dir(root, dir, ignored) {
            Ok(entries) => Listing { entries, ignored, error: None },
            Err(err) => Listing { entries: Vec::new(), ignored, error: Some(err.to_string()) },
        };
        self.listings.insert(dir.to_path_buf(), listing);
    }

    /// Opens or closes a folder.
    pub fn toggle(&mut self, root: &Path, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.load(root, dir);
            self.expanded.insert(dir.to_path_buf());
        }
    }

    pub fn expand(&mut self, root: &Path, dir: &Path) {
        if !self.expanded.contains(dir) {
            self.toggle(root, dir);
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.listings.retain(|dir, _| dir.as_os_str().is_empty());
    }

    /// Opens every folder above `path`, so its row shows, and selects it.
    pub fn reveal(&mut self, root: &Path, path: &Path) {
        let mut dirs: Vec<&Path> = path.ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()).collect();
        dirs.reverse();
        for dir in dirs {
            self.expand(root, dir);
        }
        self.selected = Some(path.to_path_buf());
    }

    /// Re-lists the given folders if they're loaded (after the watcher saw changes in them). Folders
    /// that disappeared close.
    pub fn refresh(&mut self, root: &Path, dirs: &BTreeSet<PathBuf>) {
        // Parents first, so ignore flags carry down.
        let mut dirs: Vec<&PathBuf> = dirs.iter().filter(|d| self.listings.contains_key(*d)).collect();
        dirs.sort_by_key(|p| p.components().count());
        for dir in dirs {
            if !dir.as_os_str().is_empty() && !root.join(dir).is_dir() {
                self.expanded.retain(|e| !e.starts_with(dir));
                self.listings.retain(|l, _| !l.starts_with(dir));
                continue;
            }
            self.load(root, dir);
        }
    }

    /// Re-lists everything loaded.
    pub fn reload(&mut self, root: &Path) {
        let dirs: BTreeSet<PathBuf> = self.listings.keys().cloned().collect();
        self.refresh(root, &dirs);
    }

    /// Folders to watch: the root and every open folder.
    pub fn watched(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(Path::new("")).chain(self.expanded.iter().map(PathBuf::as_path))
    }

    /// The visible rows, depth-first.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        self.push_rows(Path::new(""), 0, &mut rows);
        rows
    }

    fn push_rows(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let Some(listing) = self.listings.get(dir) else { return };
        for entry in &listing.entries {
            let path = dir.join(&entry.name);
            let expanded = entry.is_dir && self.expanded.contains(&path);
            rows.push(Row {
                path: path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
                ignored: entry.ignored,
            });
            if expanded {
                self.push_rows(&path, depth + 1, rows);
            }
        }
    }

    /// Why a folder couldn't be listed.
    pub fn listing_error(&self, dir: &Path) -> Option<&str> {
        self.listings.get(dir)?.error.as_deref()
    }

    /// The folder new files go in: the selected folder, or the selected file's folder, or the root.
    pub fn target_dir(&self) -> PathBuf {
        match &self.selected {
            Some(p) if self.is_listed_dir(p) => p.clone(),
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or_default(),
            None => PathBuf::new(),
        }
    }

    /// Moves the selection by `delta` rows.
    pub fn step(&mut self, delta: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|s| rows.iter().position(|r| &r.path == s));
        let next = match current {
            Some(i) => i.saturating_add_signed(delta).min(rows.len() - 1),
            None => 0,
        };
        self.selected = Some(rows[next].path.clone());
        self.scroll_to(next);
    }

    /// Scrolls so row `index` is in view.
    fn scroll_to(&mut self, index: usize) {
        let top = index as f32 * ROW_HEIGHT;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_HEIGHT > self.scroll + self.viewport {
            self.scroll = top + ROW_HEIGHT - self.viewport;
        }
    }

    /// A path renamed or moved: carries expansion and selection along.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        let moved = |p: &PathBuf| match p.strip_prefix(from) {
            Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
            Ok(rest) => to.join(rest),
            Err(_) => p.clone(),
        };
        self.expanded = self.expanded.iter().map(moved).collect();
        self.selected = self.selected.as_ref().map(moved);
        self.listings.retain(|dir, _| !dir.starts_with(from));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for d in ["src/ui", "target/debug"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        for f in ["src/main.rs", "src/ui/view.rs", "target/debug/app", "Cargo.toml"] {
            std::fs::write(dir.path().join(f), "").unwrap();
        }
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        dir
    }

    fn names(e: &Explorer) -> Vec<String> {
        e.rows().iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.name)).collect()
    }

    #[test]
    fn expands_lazily_and_flattens() {
        let dir = fixture();
        let mut e = Explorer::default();
        e.start(dir.path(), &["src".into(), "src/ui".into(), "gone".into()]);
        assert_eq!(names(&e), vec!["src", "  ui", "    view.rs", "  main.rs", "target", ".gitignore", "Cargo.toml"]);
        e.toggle(dir.path(), Path::new("src"));
        assert_eq!(names(&e), vec!["src", "target", ".gitignore", "Cargo.toml"]);
        e.toggle(dir.path(), Path::new("target"));
        let rows = e.rows();
        assert!(rows.iter().find(|r| r.name == "debug").unwrap().ignored, "ignored folders' children are ignored");
        let watched: Vec<&Path> = e.watched().collect();
        assert!(watched.contains(&Path::new("target")) && watched.contains(&Path::new("src/ui")));
    }

    #[test]
    fn reveal_selection_and_target_dir() {
        let dir = fixture();
        let mut e = Explorer::default();
        e.start(dir.path(), &[]);
        e.reveal(dir.path(), Path::new("src/ui/view.rs"));
        assert!(e.expanded.contains(Path::new("src/ui")));
        assert_eq!(e.target_dir(), PathBuf::from("src/ui"));
        e.selected = Some("src".into());
        assert_eq!(e.target_dir(), PathBuf::from("src"));
        e.viewport = ROW_HEIGHT * 2.0;
        e.step(3);
        assert_eq!(e.selected, Some(PathBuf::from("src/main.rs")));
        assert!(e.scroll > 0.0);
    }

    #[test]
    fn refresh_picks_up_changes_and_drops_removed_folders() {
        let dir = fixture();
        let mut e = Explorer::default();
        e.start(dir.path(), &["src/ui".into(), "src".into()]);
        std::fs::write(dir.path().join("src/new.rs"), "").unwrap();
        std::fs::remove_dir_all(dir.path().join("src/ui")).unwrap();
        e.refresh(dir.path(), &BTreeSet::from(["src".into(), "src/ui".into()]));
        assert_eq!(names(&e), vec!["src", "  main.rs", "  new.rs", "target", ".gitignore", "Cargo.toml"]);
        assert!(!e.expanded.contains(Path::new("src/ui")));
    }
}
