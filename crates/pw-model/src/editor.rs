//! Editor mode: what a workspace's code editor looks like between runs.
//!
//! A workspace is shown either as its agent terminals ([`Mode::Agents`]) or as a code editor
//! ([`Mode::Editor`]): a file tree, up to [`MAX_GROUPS`] editor groups of tabs, and one terminal.
//! This is the persisted form; unsaved text is never stored here.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ids::{GroupId, PaneId};
use crate::layout::{LayoutFull, SplitTree};

/// A workspace shows at most this many editor groups side by side.
pub const MAX_GROUPS: usize = 4;

/// How a workspace is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The terminal grid.
    #[default]
    Agents,
    /// The code editor.
    Editor,
}

impl Mode {
    pub fn toggled(self) -> Self {
        match self {
            Mode::Agents => Mode::Editor,
            Mode::Editor => Mode::Agents,
        }
    }
}

/// An open tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenFile {
    /// Relative to the workspace root.
    pub path: PathBuf,
    /// The cursor, 0-based (column in characters).
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
    /// A preview tab is replaced by the next file opened from the tree, until it's edited or pinned.
    #[serde(default)]
    pub preview: bool,
}

impl OpenFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), line: 0, column: 0, preview: false }
    }
}

/// One editor group: a strip of tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupSpec {
    pub tabs: Vec<OpenFile>,
    /// The tab shown, by path.
    pub active: Option<PathBuf>,
}

/// The editor's own terminal. It isn't an agent pane, so it doesn't count toward `MAX_PANES`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalSpec {
    pub id: PaneId,
    /// Where its shell starts. Empty means the workspace root.
    #[serde(default)]
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSession {
    pub layout: SplitTree<GroupId>,
    /// Specs for exactly the groups in `layout`.
    pub groups: BTreeMap<GroupId, GroupSpec>,
    pub focused: GroupId,
    /// Folders open in the file tree, relative to the root.
    pub expanded: Vec<PathBuf>,
    pub show_explorer: bool,
    /// Width share of the file tree.
    pub explorer_ratio: f32,
    pub terminal: TerminalSpec,
    pub show_terminal: bool,
    /// Height share of the editor groups above the terminal.
    pub editor_ratio: f32,
}

impl Default for EditorSession {
    fn default() -> Self {
        let group = GroupId::new();
        Self {
            layout: SplitTree::pane(group),
            groups: BTreeMap::from([(group, GroupSpec::default())]),
            focused: group,
            expanded: Vec::new(),
            show_explorer: true,
            explorer_ratio: Self::EXPLORER_RATIO,
            terminal: TerminalSpec { id: PaneId::new(), cwd: PathBuf::new() },
            show_terminal: true,
            editor_ratio: Self::EDITOR_RATIO,
        }
    }
}

impl EditorSession {
    pub const EXPLORER_RATIO: f32 = 0.2;
    pub const EDITOR_RATIO: f32 = 0.7;
    pub const EXPLORER_RANGE: (f32, f32) = (0.08, 0.5);
    pub const EDITOR_RANGE: (f32, f32) = (0.15, 0.92);

    /// Splits `target`, adding `new` after it. Enforces [`MAX_GROUPS`].
    pub fn split(
        &mut self,
        target: GroupId,
        axis: crate::Axis,
        new: GroupId,
        spec: GroupSpec,
    ) -> Result<bool, LayoutFull> {
        let added = self.layout.split_leaf(target, axis, new, MAX_GROUPS)?;
        if added {
            self.groups.insert(new, spec);
        }
        Ok(added)
    }

    /// Makes a loaded session self-consistent: specs for exactly the groups in the layout, at most
    /// [`MAX_GROUPS`], only safe relative paths, no duplicate tabs, ratios in range.
    pub fn repaired(mut self) -> Self {
        let mut layout = self.layout.sanitized();
        while layout.pane_count() > MAX_GROUPS {
            let last = *layout.panes().last().expect("non-empty tree");
            layout = layout.close_pane(last).expect("more than one group left");
        }
        let mut specs = std::mem::take(&mut self.groups);
        self.groups =
            layout.panes().into_iter().map(|id| (id, specs.remove(&id).unwrap_or_default().repaired())).collect();
        if !layout.contains(self.focused) {
            self.focused = layout.panes()[0];
        }
        self.layout = layout;
        let mut seen = HashSet::new();
        self.expanded.retain(|p| is_safe_relative(p) && seen.insert(p.clone()));
        self.explorer_ratio = clamp(self.explorer_ratio, Self::EXPLORER_RANGE, Self::EXPLORER_RATIO);
        self.editor_ratio = clamp(self.editor_ratio, Self::EDITOR_RANGE, Self::EDITOR_RATIO);
        self
    }
}

impl GroupSpec {
    fn repaired(mut self) -> Self {
        let mut seen = HashSet::new();
        self.tabs.retain(|t| is_safe_relative(&t.path) && seen.insert(t.path.clone()));
        // At most one preview tab per group.
        let last_preview = self.tabs.iter().rposition(|t| t.preview);
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            tab.preview = Some(i) == last_preview;
        }
        if !self.active.as_ref().is_some_and(|a| self.tabs.iter().any(|t| &t.path == a)) {
            self.active = self.tabs.first().map(|t| t.path.clone());
        }
        self
    }
}

/// A non-empty path below the root: no root, no `..`, no `.`.
pub fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn clamp(value: f32, (min, max): (f32, f32), default: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { default }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;

    #[test]
    fn default_has_one_empty_focused_group() {
        let s = EditorSession::default();
        assert_eq!(s.layout.panes(), vec![s.focused]);
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.clone().repaired(), s);
    }

    #[test]
    fn split_enforces_the_group_cap() {
        let mut s = EditorSession::default();
        for _ in 1..MAX_GROUPS {
            let last = *s.layout.panes().last().unwrap();
            assert_eq!(s.split(last, Axis::Vertical, GroupId::new(), GroupSpec::default()), Ok(true));
        }
        assert_eq!(s.split(s.focused, Axis::Vertical, GroupId::new(), GroupSpec::default()), Err(LayoutFull));
        assert_eq!(s.groups.len(), MAX_GROUPS);
    }

    #[test]
    fn repair_drops_unsafe_and_duplicate_tabs() {
        let mut s = EditorSession::default();
        let spec = s.groups.get_mut(&s.focused).unwrap();
        spec.tabs =
            ["src/a.rs", "/etc/passwd", "../x", "src/a.rs", "b.rs", "./c.rs"].into_iter().map(OpenFile::new).collect();
        spec.tabs[4].preview = true;
        spec.tabs[0].preview = true;
        spec.active = Some("../x".into());
        s.expanded = vec!["src".into(), "src".into(), "/abs".into(), "".into()];
        let s = s.repaired();
        let spec = &s.groups[&s.focused];
        let paths: Vec<_> = spec.tabs.iter().map(|t| t.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["src/a.rs", "b.rs"]);
        assert_eq!(spec.tabs.iter().filter(|t| t.preview).count(), 1);
        assert!(spec.tabs[1].preview);
        assert_eq!(spec.active, Some("src/a.rs".into()));
        assert_eq!(s.expanded, vec![PathBuf::from("src")]);
    }

    #[test]
    fn repair_matches_groups_to_layout_and_clamps() {
        let mut s = EditorSession::default();
        let extra = GroupId::new();
        let orphan = GroupId::new();
        s.layout = SplitTree::split(Axis::Vertical, 0.5, s.layout.clone(), SplitTree::pane(extra));
        s.groups.insert(orphan, GroupSpec::default());
        s.focused = orphan;
        s.explorer_ratio = f32::NAN;
        s.editor_ratio = 4.0;
        let s = s.repaired();
        assert_eq!(s.groups.keys().copied().collect::<HashSet<_>>(), s.layout.panes().into_iter().collect());
        assert!(s.groups.contains_key(&extra));
        assert_ne!(s.focused, orphan);
        assert_eq!(s.explorer_ratio, EditorSession::EXPLORER_RATIO);
        assert_eq!(s.editor_ratio, EditorSession::EDITOR_RANGE.1);
    }

    #[test]
    fn repair_caps_groups() {
        let mut s = EditorSession::default();
        for _ in 0..MAX_GROUPS + 2 {
            let tree = std::mem::replace(&mut s.layout, SplitTree::pane(GroupId::new()));
            s.layout = SplitTree::split(Axis::Vertical, 0.5, tree, SplitTree::pane(GroupId::new()));
        }
        let s = s.repaired();
        assert_eq!(s.layout.pane_count(), MAX_GROUPS);
        assert_eq!(s.groups.len(), MAX_GROUPS);
    }

    #[test]
    fn mode_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Mode::Editor).unwrap(), "\"editor\"");
        assert_eq!(Mode::Agents.toggled(), Mode::Editor);
    }
}
