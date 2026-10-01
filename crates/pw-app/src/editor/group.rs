//! An editor group: a strip of tabs, the find bar over them, and a note about the last file that
//! couldn't be opened.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::find::Find;
use crate::ui::editor::code_editor::Content;
use crate::ui::editor::highlight::Parse;

/// One open tab: a view (cursor, scroll, layout) of a document.
pub struct Tab {
    /// Relative to the workspace root; the key of its document.
    pub path: PathBuf,
    pub view: Content,
    /// A preview tab is replaced by the next file opened from the tree, until it's edited or pinned.
    pub preview: bool,
    /// Unique for the app's lifetime, so a group's highlighter knows when it shows another tab.
    pub id: u64,
    /// How far the view's colours have got, kept so showing the tab again resumes there.
    pub parse: Parse,
}

impl Tab {
    pub fn new(path: PathBuf, text: &str, preview: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        Self { path, view: Content::with_text(text), preview, id, parse: Parse::default() }
    }
}

/// A file the group tried to open but can't show as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub path: PathBuf,
    pub message: String,
    /// The file exists, so the system's default app might open it.
    pub external: bool,
}

#[derive(Default)]
pub struct Group {
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    pub find: Option<Find>,
    pub notice: Option<Notice>,
}

impl Group {
    pub fn position(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.path == path)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active?)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active?)
    }

    pub fn active_path(&self) -> Option<&Path> {
        self.active_tab().map(|t| t.path.as_path())
    }

    /// Adds a tab next to the active one (replacing the preview tab when opening another preview) and
    /// makes it active. Returns the path of the preview tab it replaced.
    pub fn insert(&mut self, tab: Tab) -> Option<PathBuf> {
        let mut replaced = None;
        let mut at = self.active.map_or(self.tabs.len(), |i| i + 1);
        if tab.preview
            && let Some(i) = self.tabs.iter().position(|t| t.preview)
        {
            replaced = Some(self.tabs.remove(i).path);
            at = i;
        }
        let at = at.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active = Some(at);
        replaced
    }

    /// Removes a tab; the neighbour to the left becomes active (or the right one, from the first).
    pub fn remove(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.saturating_sub(1)),
            other => other,
        };
        Some(tab)
    }

    /// Cycles the active tab.
    pub fn cycle(&mut self, forward: bool) {
        let n = self.tabs.len();
        if let Some(active) = self.active.filter(|_| n > 1) {
            self.active = Some(if forward { (active + 1) % n } else { (active + n - 1) % n });
        }
    }

    /// For a tab whose file name another tab shares: the fewest trailing folders of its path that tell it apart
    /// (`src` for `a/src/lib.rs` beside `b/lib.rs`, `a/src` beside `b/src/lib.rs`).
    pub fn folder_hint(&self, index: usize) -> Option<String> {
        let path = &self.tabs.get(index)?.path;
        let name = path.file_name()?;
        let folders = |p: &Path| -> Vec<String> {
            let mut parts: Vec<String> =
                p.parent().into_iter().flat_map(Path::iter).map(|c| c.to_string_lossy().into_owned()).collect();
            parts.reverse();
            parts
        };
        let mine = folders(path);
        let others: Vec<Vec<String>> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, t)| *i != index && t.path.file_name() == Some(name))
            .map(|(_, t)| folders(&t.path))
            .collect();
        if others.is_empty() {
            return None;
        }
        // Grow the suffix until no other tab ends the same way (or the whole path is shown).
        let depth =
            (1..=mine.len()).find(|&k| others.iter().all(|o| o.get(..k) != mine.get(..k))).unwrap_or(mine.len());
        let mut shown: Vec<&str> = mine[..depth].iter().map(String::as_str).collect();
        shown.reverse();
        Some(if shown.is_empty() { "./".to_owned() } else { format!("{}/", shown.join("/")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(g: &Group) -> Vec<&str> {
        g.tabs.iter().map(|t| t.path.to_str().unwrap()).collect()
    }

    #[test]
    fn tabs_open_next_to_the_active_one_and_close_leftwards() {
        let mut g = Group::default();
        for p in ["a", "c"] {
            g.insert(Tab::new(p.into(), "", false));
        }
        g.active = Some(0);
        g.insert(Tab::new("b".into(), "", false));
        assert_eq!(names(&g), vec!["a", "b", "c"]);
        assert_eq!(g.active, Some(1));
        g.remove(1);
        assert_eq!(g.active, Some(0));
        g.remove(0);
        assert_eq!(g.active, Some(0));
        assert_eq!(names(&g), vec!["c"]);
        g.remove(0);
        assert_eq!(g.active, None);
    }

    #[test]
    fn a_preview_replaces_the_previous_preview() {
        let mut g = Group::default();
        g.insert(Tab::new("pinned".into(), "", false));
        assert_eq!(g.insert(Tab::new("p1".into(), "", true)), None);
        assert_eq!(g.insert(Tab::new("p2".into(), "", true)), Some("p1".into()));
        assert_eq!(names(&g), vec!["pinned", "p2"]);
        g.cycle(true);
        assert_eq!(g.active, Some(0));
        g.cycle(false);
        assert_eq!(g.active, Some(1));
    }

    #[test]
    fn same_named_tabs_show_the_folders_that_tell_them_apart() {
        let mut g = Group::default();
        for p in ["crates/pw-code/src/lib.rs", "crates/pw-model/src/lib.rs", "lib.rs", "src/main.rs", "x/mod.rs"] {
            g.insert(Tab::new(p.into(), "", false));
        }
        let hint = |g: &Group, path: &str| g.folder_hint(g.position(Path::new(path)).unwrap());
        assert_eq!(hint(&g, "crates/pw-code/src/lib.rs").as_deref(), Some("pw-code/src/"));
        assert_eq!(hint(&g, "crates/pw-model/src/lib.rs").as_deref(), Some("pw-model/src/"));
        assert_eq!(hint(&g, "lib.rs").as_deref(), Some("./"));
        assert_eq!(hint(&g, "src/main.rs"), None);
        g.insert(Tab::new("y/mod.rs".into(), "", false));
        assert_eq!(hint(&g, "x/mod.rs").as_deref(), Some("x/"));
    }
}
