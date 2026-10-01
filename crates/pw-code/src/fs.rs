//! The project's files as the editor sees them: one folder at a time for the file tree, and every
//! file at once for Quick open. Both honour `.gitignore`.
//!
//! The tree shows ignored entries, dimmed (`target/`, `node_modules/`), because agents and builds
//! put things there that people still want to open. Quick open leaves them out.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Names never shown: VCS internals and OS litter.
const HIDDEN: &[&str] = &[".git", ".hg", ".svn", ".DS_Store", "Thumbs.db"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Matched by a `.gitignore` (or `.ignore`) rule.
    pub ignored: bool,
}

/// The entries of `root/rel`: folders first, then files, each case-insensitively A→Z.
///
/// `parent_ignored` says the folder itself is ignored, which makes everything in it ignored (ignore rules
/// match a folder, not what's below it), and spares reading rules in `node_modules`.
pub fn list_dir(root: &Path, rel: &Path, parent_ignored: bool) -> io::Result<Vec<Entry>> {
    let dir = root.join(rel);
    let mut entries: Vec<Entry> = std::fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if HIDDEN.contains(&name.as_str()) {
                return None;
            }
            let file_type = e.file_type().ok()?;
            // Follow symlinks to tell folders from files; a broken link shows as a file.
            let is_dir = file_type.is_dir() || (file_type.is_symlink() && e.path().is_dir());
            Some(Entry { name, is_dir, ignored: false })
        })
        .collect();
    if parent_ignored {
        entries.iter_mut().for_each(|e| e.ignored = true);
    } else {
        let kept = kept_names(&dir);
        for entry in &mut entries {
            entry.ignored = !kept.contains(&OsString::from(&entry.name));
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural_key(&a.name).cmp(&natural_key(&b.name))));
    Ok(entries)
}

/// The names in `dir` that ignore rules keep.
fn kept_names(dir: &Path) -> HashSet<OsString> {
    walker(dir)
        .max_depth(Some(1))
        .build()
        .flatten()
        .filter(|e| e.depth() == 1)
        .map(|e| e.file_name().to_owned())
        .collect()
}

fn walker(dir: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(dir);
    builder.hidden(false).require_git(false).follow_links(false).sort_by_file_name(|a, b| a.cmp(b));
    builder.filter_entry(|e| !HIDDEN.iter().any(|h| e.file_name() == *h));
    builder
}

fn natural_key(name: &str) -> (String, &str) {
    (name.to_lowercase(), name)
}

/// Every file under `root` that ignore rules keep, relative to it, up to `cap` of them. The flag says
/// whether the walk stopped at the cap.
pub fn index(root: &Path, cap: usize) -> (Vec<PathBuf>, bool) {
    let mut files = Vec::new();
    for entry in walker(root).build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) {
            continue;
        }
        if files.len() >= cap {
            return (files, true);
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            files.push(rel.to_path_buf());
        }
    }
    (files, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["src/nested", "target/debug", ".git/objects", "Docs"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["src/main.rs", "src/nested/deep.rs", "target/debug/app", "README.md", "b.txt", ".env", ".DS_Store"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(root.join("debug.log"), "").unwrap();
        dir
    }

    #[test]
    fn lists_folders_first_and_dims_ignored_entries() {
        let dir = fixture();
        let entries = list_dir(dir.path(), Path::new(""), false).unwrap();
        let shown: Vec<(&str, bool, bool)> = entries.iter().map(|e| (e.name.as_str(), e.is_dir, e.ignored)).collect();
        assert_eq!(
            shown,
            vec![
                ("Docs", true, false),
                ("src", true, false),
                ("target", true, true),
                (".env", false, false),
                (".gitignore", false, false),
                ("b.txt", false, false),
                ("debug.log", false, true),
                ("README.md", false, false),
            ]
        );
        let nested = list_dir(dir.path(), Path::new("target"), true).unwrap();
        assert_eq!(nested.len(), 1);
        assert!(nested[0].ignored, "children of an ignored folder are ignored too");
        assert!(!list_dir(dir.path(), Path::new("src"), false).unwrap().iter().any(|e| e.ignored));
        assert!(list_dir(dir.path(), Path::new("missing"), false).is_err());
    }

    #[test]
    fn index_skips_ignored_files_and_caps() {
        let dir = fixture();
        let (mut files, truncated) = index(dir.path(), 100);
        files.sort();
        let files: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(files, vec![".env", ".gitignore", "README.md", "b.txt", "src/main.rs", "src/nested/deep.rs"]);
        assert!(!truncated);
        let (files, truncated) = index(dir.path(), 2);
        assert_eq!(files.len(), 2);
        assert!(truncated);
    }
}
