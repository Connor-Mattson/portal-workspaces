//! A workspace: one project, its root directory and its terminal layout.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::{PaneId, WorkspaceId};
use crate::layout::{LayoutNode, MAX_PANES, Preset};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneSpec {
    /// Where the pane's shell starts. Updated from the live shell's cwd while the app runs.
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// The project directory. New panes start here.
    pub root: PathBuf,
    /// `None` when every pane has been closed.
    pub layout: Option<LayoutNode>,
    pub panes: BTreeMap<PaneId, PaneSpec>,
    pub focused: Option<PaneId>,
}

impl Workspace {
    /// A new workspace laid out as `preset`, every pane starting in `root`.
    pub fn new(name: impl Into<String>, root: PathBuf, preset: Preset) -> Self {
        let ids: Vec<PaneId> = (0..preset.pane_count().clamp(1, MAX_PANES)).map(|_| PaneId::new()).collect();
        let panes = ids.iter().map(|&id| (id, PaneSpec { cwd: root.clone() })).collect();
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            layout: Some(preset.build(&ids)),
            focused: ids.first().copied(),
            root,
            panes,
        }
    }

    pub fn pane_count(&self) -> usize {
        self.layout.as_ref().map_or(0, LayoutNode::pane_count)
    }

    /// Makes a loaded workspace self-consistent: specs exist for exactly the panes in the layout,
    /// ratios are in range, the focus points at a real pane and the pane cap holds.
    pub fn repaired(mut self) -> Self {
        let mut layout = self.layout.take().map(LayoutNode::sanitized);
        while let Some(tree) = layout.as_ref().filter(|t| t.pane_count() > MAX_PANES) {
            let last = *tree.panes().last().expect("non-empty tree");
            layout = layout.and_then(|t| t.close_pane(last));
        }
        let live = layout.as_ref().map(LayoutNode::panes).unwrap_or_default();
        let root = self.root.clone();
        let mut specs = std::mem::take(&mut self.panes);
        self.panes =
            live.iter().map(|id| (*id, specs.remove(id).unwrap_or_else(|| PaneSpec { cwd: root.clone() }))).collect();
        if !self.focused.is_some_and(|f| live.contains(&f)) {
            self.focused = live.first().copied();
        }
        self.layout = layout;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis;

    #[test]
    fn new_workspace_has_specs_for_every_pane() {
        let ws = Workspace::new("demo", "/tmp".into(), Preset::Grid { cols: 2, rows: 2 });
        assert_eq!(ws.pane_count(), 4);
        assert_eq!(ws.panes.len(), 4);
        assert!(ws.panes.values().all(|p| p.cwd.as_path() == std::path::Path::new("/tmp")));
        assert_eq!(ws.focused, ws.layout.as_ref().unwrap().panes().first().copied());
    }

    #[test]
    fn repair_drops_orphan_specs_and_fills_missing_ones() {
        let mut ws = Workspace::new("demo", "/root".into(), Preset::Single);
        let orphan = PaneId::new();
        ws.panes.insert(orphan, PaneSpec { cwd: "/x".into() });
        let first = ws.focused.unwrap();
        let added = PaneId::new();
        ws.layout.as_mut().unwrap().split_pane(first, Axis::Vertical, added).unwrap();
        ws.focused = Some(orphan);

        let ws = ws.repaired();
        assert!(!ws.panes.contains_key(&orphan));
        assert_eq!(ws.panes[&added].cwd, PathBuf::from("/root"));
        assert_eq!(ws.focused, Some(first));
    }

    #[test]
    fn repair_enforces_pane_cap() {
        let mut ws = Workspace::new("demo", "/".into(), Preset::Grid { cols: 4, rows: 2 });
        let first = ws.focused.unwrap();
        // Bypass split_pane's cap check to simulate a hand-edited file.
        let extra = PaneId::new();
        let tree = ws.layout.take().unwrap();
        ws.layout = Some(LayoutNode::split(Axis::Vertical, 0.5, tree, LayoutNode::pane(extra)));
        let ws = ws.repaired();
        assert_eq!(ws.pane_count(), MAX_PANES);
        assert!(ws.panes.contains_key(&first));
    }
}
