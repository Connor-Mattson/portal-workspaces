//! A workspace as the UI holds it: the model plus iced's live pane-grid state, and its editor once
//! it has been shown in Editor mode.

use iced::widget::pane_grid::{self, Pane};
use pw_model::{Axis, LayoutFull, LayoutNode, MAX_PANES, PaneId, Preset, Workspace};

use crate::editor::EditorView;
use crate::keymap::Direction;
use crate::split::{self, to_iced_axis};

pub struct WorkspaceView {
    /// Name, root and ids. Its `layout`, `panes` and `focused` are refreshed by [`Self::to_model`].
    pub model: Workspace,
    /// The live layout. `None` when every pane has been closed.
    pub grid: Option<pane_grid::State<PaneId>>,
    /// A pane in the grid.
    pub focused: Option<PaneId>,
    /// Panes popped out into their own windows. They're not in the grid but count toward the cap.
    pub detached: Vec<PaneId>,
    /// Shells are started the first time the workspace is shown, not at app launch.
    pub spawned: bool,
    /// The code editor, built the first time the workspace is shown in Editor mode.
    pub editor: Option<EditorView>,
}

impl WorkspaceView {
    pub fn new(model: Workspace) -> Self {
        let grid = model.layout.as_ref().map(split::grid);
        Self { focused: model.focused, detached: model.detached.clone(), grid, model, spawned: false, editor: None }
    }

    pub fn id(&self) -> pw_model::WorkspaceId {
        self.model.id
    }

    /// The grid's panes in reading order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout().map(|l| l.panes()).unwrap_or_default()
    }

    /// The grid's panes, then the detached ones.
    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        let mut ids = self.pane_ids();
        ids.extend(&self.detached);
        ids
    }

    /// Panes in the grid.
    pub fn grid_len(&self) -> usize {
        self.grid.as_ref().map_or(0, pane_grid::State::len)
    }

    /// Every terminal of the workspace, detached ones included.
    pub fn pane_count(&self) -> usize {
        self.grid_len() + self.detached.len()
    }

    pub fn can_split(&self) -> bool {
        self.pane_count() < MAX_PANES
    }

    /// Whether `preset` can be applied: it replaces the grid, and the detached panes stay.
    pub fn fits(&self, preset: Preset) -> bool {
        preset.pane_count() + self.detached.len() <= MAX_PANES
    }

    /// The editor's terminal (its session exists once the editor showed it).
    pub fn editor_terminal(&self) -> PaneId {
        self.model.editor.terminal.id
    }

    /// Whether a terminal belongs to this workspace: in the grid, detached, or the editor's.
    pub fn owns(&self, pane: PaneId) -> bool {
        self.is_detached(pane) || self.handle(pane).is_some() || self.editor_terminal() == pane
    }

    pub fn is_detached(&self, id: PaneId) -> bool {
        self.detached.contains(&id)
    }

    /// The pane-grid handle for a pane id.
    pub fn handle(&self, id: PaneId) -> Option<Pane> {
        split::handle(self.grid.as_ref()?, &id)
    }

    pub fn layout(&self) -> Option<LayoutNode> {
        self.grid.as_ref().map(split::tree)
    }

    /// Splits the focused pane (or `target`) and focuses the new one.
    pub fn split(&mut self, target: Option<PaneId>, axis: Axis, new: PaneId) -> Result<(), LayoutFull> {
        if !self.can_split() {
            return Err(LayoutFull);
        }
        match (self.grid.as_mut(), target.or(self.focused)) {
            (None, _) => {
                self.grid = Some(pane_grid::State::new(new).0);
            }
            (Some(grid), target) => {
                let handle = target
                    .and_then(|t| grid.iter().find(|(_, p)| **p == t).map(|(h, _)| *h))
                    .or_else(|| grid.iter().next().map(|(h, _)| *h))
                    .expect("grid is never empty");
                grid.restore();
                grid.split(to_iced_axis(axis), handle, new);
            }
        }
        self.focused = Some(new);
        Ok(())
    }

    /// Removes a pane from the layout, moving focus to its sibling.
    pub fn close(&mut self, id: PaneId) {
        let Some(handle) = self.handle(id) else { return };
        let grid = self.grid.as_mut().expect("handle implies grid");
        if grid.len() == 1 {
            self.grid = None;
            self.focused = None;
            return;
        }
        if let Some((_, sibling)) = grid.close(handle)
            && self.focused == Some(id)
        {
            self.focused = grid.get(sibling).copied();
        }
    }

    /// Takes a pane out of the grid to show in its own window. Focus moves to its sibling.
    pub fn detach(&mut self, id: PaneId) -> bool {
        if self.handle(id).is_none() {
            return false;
        }
        // The others would stay hidden behind a pane that's gone.
        if let Some(grid) = self.grid.as_mut() {
            grid.restore();
        }
        self.close(id);
        self.detached.push(id);
        true
    }

    /// Puts a detached pane back into the grid, next to the focused pane, and focuses it.
    pub fn dock(&mut self, id: PaneId) -> bool {
        let Some(index) = self.detached.iter().position(|p| *p == id) else { return false };
        self.detached.remove(index);
        self.split(None, Axis::Vertical, id).expect("a detached pane already counted toward the cap");
        true
    }

    /// Replaces the grid's layout with `preset`, reusing existing panes in reading order. Returns the
    /// ids of panes that were created (need shells) and dropped (need closing). Detached panes stay;
    /// callers check [`Self::fits`] first.
    pub fn apply_preset(&mut self, preset: Preset) -> (Vec<PaneId>, Vec<PaneId>) {
        let current = self.pane_ids();
        let want = preset.pane_count().min(MAX_PANES);
        let mut ids: Vec<PaneId> = current.iter().copied().take(want).collect();
        let created: Vec<PaneId> = (ids.len()..want).map(|_| PaneId::new()).collect();
        ids.extend(&created);
        let dropped = current.iter().copied().skip(want).collect();
        self.grid = Some(split::grid(&preset.build(&ids)));
        if !self.focused.is_some_and(|f| ids.contains(&f)) {
            self.focused = ids.first().copied();
        }
        (created, dropped)
    }

    /// The layout currently matching a preset exactly (ratios aside), if any.
    pub fn matching_preset(&self) -> Option<Preset> {
        let layout = self.layout()?;
        let ids = layout.panes();
        Preset::ALL.into_iter().filter(|p| p.pane_count() == ids.len()).find(|p| same_shape(&p.build(&ids), &layout))
    }

    pub fn adjacent(&self, direction: Direction) -> Option<PaneId> {
        split::adjacent(self.grid.as_ref()?, &self.focused?, direction)
    }

    pub fn toggle_maximize(&mut self, id: PaneId) {
        let handle = self.handle(id);
        let Some(grid) = self.grid.as_mut() else { return };
        match (grid.maximized(), handle) {
            (Some(_), _) => grid.restore(),
            (None, Some(h)) if grid.len() > 1 => grid.maximize(h),
            _ => {}
        }
    }

    pub fn maximized(&self) -> Option<PaneId> {
        let grid = self.grid.as_ref()?;
        grid.maximized().and_then(|h| grid.get(h).copied())
    }

    /// The persisted form. `cwd_of` supplies each terminal's current directory.
    pub fn to_model(&self, cwd_of: impl Fn(PaneId) -> Option<std::path::PathBuf>) -> Workspace {
        let mut model = self.model.clone();
        if let Some(editor) = &self.editor {
            model.editor = editor.to_session(cwd_of(editor.terminal.id));
        }
        model.layout = self.layout();
        model.focused = self.focused;
        model.detached = self.detached.clone();
        model.panes = self
            .all_pane_ids()
            .into_iter()
            .map(|id| {
                let cwd = cwd_of(id)
                    .or_else(|| self.model.panes.get(&id).map(|p| p.cwd.clone()))
                    .unwrap_or_else(|| model.root.clone());
                (id, pw_model::PaneSpec { cwd })
            })
            .collect();
        model
    }
}

fn same_shape(a: &LayoutNode, b: &LayoutNode) -> bool {
    match (a, b) {
        (LayoutNode::Pane { .. }, LayoutNode::Pane { .. }) => true,
        (LayoutNode::Split { axis: xa, a: aa, b: ab, .. }, LayoutNode::Split { axis: xb, a: ba, b: bb, .. }) => {
            xa == xb && same_shape(aa, ba) && same_shape(ab, bb)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(preset: Preset) -> WorkspaceView {
        WorkspaceView::new(Workspace::new("t", "/tmp".into(), preset))
    }

    #[test]
    fn layout_round_trips_through_pane_grid() {
        for preset in Preset::ALL {
            let ws = view(preset);
            assert_eq!(ws.layout(), ws.model.layout, "{preset:?}");
            assert_eq!(ws.matching_preset(), Some(preset));
        }
    }

    #[test]
    fn split_is_capped() {
        let mut ws = view(Preset::Grid { cols: 4, rows: 2 });
        assert!(!ws.can_split());
        assert_eq!(ws.split(None, Axis::Vertical, PaneId::new()), Err(LayoutFull));
        assert_eq!(ws.pane_count(), MAX_PANES);
    }

    #[test]
    fn close_last_pane_empties_the_workspace_and_split_revives_it() {
        let mut ws = view(Preset::Single);
        let only = ws.focused.unwrap();
        ws.close(only);
        assert_eq!(ws.pane_count(), 0);
        assert_eq!(ws.layout(), None);
        let new = PaneId::new();
        ws.split(None, Axis::Vertical, new).unwrap();
        assert_eq!(ws.pane_ids(), vec![new]);
        assert_eq!(ws.focused, Some(new));
    }

    #[test]
    fn preset_reuses_panes_and_reports_changes() {
        let mut ws = view(Preset::Columns(3));
        let before = ws.pane_ids();
        let (created, dropped) = ws.apply_preset(Preset::Grid { cols: 2, rows: 2 });
        assert_eq!(created.len(), 1);
        assert!(dropped.is_empty());
        assert_eq!(&ws.pane_ids()[..3], &before[..]);

        let (created, dropped) = ws.apply_preset(Preset::Single);
        assert!(created.is_empty());
        assert_eq!(dropped.len(), 3);
        assert_eq!(ws.pane_ids(), vec![before[0]]);
    }

    #[test]
    fn detached_panes_count_toward_the_cap_and_dock_back() {
        let mut ws = view(Preset::Grid { cols: 4, rows: 2 });
        let ids = ws.pane_ids();
        ws.focused = Some(ids[1]);
        ws.toggle_maximize(ids[1]);
        assert!(ws.detach(ids[1]));
        assert_eq!(ws.maximized(), None);
        assert_eq!(ws.grid_len(), MAX_PANES - 1);
        assert_eq!(ws.pane_count(), MAX_PANES);
        assert!(!ws.can_split());
        assert!(!ws.fits(Preset::Grid { cols: 4, rows: 2 }));
        assert!(ws.fits(Preset::Columns(4)));
        assert_ne!(ws.focused, Some(ids[1]));
        assert!(!ws.detach(ids[1]), "already detached");

        let model = ws.to_model(|_| None);
        assert_eq!(model.detached, vec![ids[1]]);
        assert!(model.panes.contains_key(&ids[1]));
        assert_eq!(model.clone().repaired(), model);

        assert!(ws.dock(ids[1]));
        assert!(ws.detached.is_empty());
        assert_eq!(ws.grid_len(), MAX_PANES);
        assert_eq!(ws.focused, Some(ids[1]));
    }

    #[test]
    fn detaching_the_last_pane_empties_the_grid() {
        let mut ws = view(Preset::Single);
        let only = ws.focused.unwrap();
        assert!(ws.detach(only));
        assert_eq!(ws.grid_len(), 0);
        assert_eq!(ws.all_pane_ids(), vec![only]);
        let (created, dropped) = ws.apply_preset(Preset::Columns(2));
        assert_eq!((created.len(), dropped.len()), (2, 0));
        assert!(ws.dock(only));
        assert_eq!(ws.pane_count(), 3);
    }

    #[test]
    fn close_moves_focus_to_sibling() {
        let mut ws = view(Preset::Columns(2));
        let ids = ws.pane_ids();
        ws.focused = Some(ids[1]);
        ws.close(ids[1]);
        assert_eq!(ws.focused, Some(ids[0]));
    }
}
