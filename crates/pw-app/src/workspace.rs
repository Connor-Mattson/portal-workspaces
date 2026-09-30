//! A workspace as the UI holds it: the model plus iced's live pane-grid state.

use iced::widget::pane_grid::{self, Configuration, Node, Pane};
use pw_model::{Axis, LayoutFull, LayoutNode, MAX_PANES, PaneId, Preset, Workspace};

use crate::keymap::Direction;

pub struct WorkspaceView {
    /// Name, root and ids. Its `layout`, `panes` and `focused` are refreshed by [`Self::to_model`].
    pub model: Workspace,
    /// The live layout. `None` when every pane has been closed.
    pub grid: Option<pane_grid::State<PaneId>>,
    pub focused: Option<PaneId>,
    /// Shells are started the first time the workspace is shown, not at app launch.
    pub spawned: bool,
}

impl WorkspaceView {
    pub fn new(model: Workspace) -> Self {
        let grid = model.layout.as_ref().map(|l| pane_grid::State::with_configuration(to_configuration(l)));
        Self { focused: model.focused, grid, model, spawned: false }
    }

    pub fn id(&self) -> pw_model::WorkspaceId {
        self.model.id
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout().map(|l| l.panes()).unwrap_or_default()
    }

    pub fn pane_count(&self) -> usize {
        self.grid.as_ref().map_or(0, pane_grid::State::len)
    }

    pub fn can_split(&self) -> bool {
        self.pane_count() < MAX_PANES
    }

    /// The pane-grid handle for a pane id.
    pub fn handle(&self, id: PaneId) -> Option<Pane> {
        self.grid.as_ref()?.iter().find(|(_, p)| **p == id).map(|(h, _)| *h)
    }

    pub fn layout(&self) -> Option<LayoutNode> {
        let grid = self.grid.as_ref()?;
        Some(from_node(grid.layout(), grid))
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

    /// Replaces the layout with `preset`, reusing existing panes in reading order. Returns the
    /// ids of panes that were created (need shells) and dropped (need closing).
    pub fn apply_preset(&mut self, preset: Preset) -> (Vec<PaneId>, Vec<PaneId>) {
        let current = self.pane_ids();
        let want = preset.pane_count().min(MAX_PANES);
        let mut ids: Vec<PaneId> = current.iter().copied().take(want).collect();
        let created: Vec<PaneId> = (ids.len()..want).map(|_| PaneId::new()).collect();
        ids.extend(&created);
        let dropped = current.iter().copied().skip(want).collect();
        self.grid = Some(pane_grid::State::with_configuration(to_configuration(&preset.build(&ids))));
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
        let grid = self.grid.as_ref()?;
        let from = self.handle(self.focused?)?;
        let direction = match direction {
            Direction::Left => pane_grid::Direction::Left,
            Direction::Right => pane_grid::Direction::Right,
            Direction::Up => pane_grid::Direction::Up,
            Direction::Down => pane_grid::Direction::Down,
        };
        grid.adjacent(from, direction).and_then(|h| grid.get(h).copied())
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

    /// The persisted form. `cwd_of` supplies each pane's current directory.
    pub fn to_model(&self, cwd_of: impl Fn(PaneId) -> Option<std::path::PathBuf>) -> Workspace {
        let mut model = self.model.clone();
        model.layout = self.layout();
        model.focused = self.focused;
        model.panes = self
            .pane_ids()
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

fn to_iced_axis(axis: Axis) -> pane_grid::Axis {
    match axis {
        Axis::Horizontal => pane_grid::Axis::Horizontal,
        Axis::Vertical => pane_grid::Axis::Vertical,
    }
}

fn to_configuration(node: &LayoutNode) -> Configuration<PaneId> {
    match node {
        LayoutNode::Pane { id } => Configuration::Pane(*id),
        LayoutNode::Split { axis, ratio, a, b } => Configuration::Split {
            axis: to_iced_axis(*axis),
            ratio: *ratio,
            a: Box::new(to_configuration(a)),
            b: Box::new(to_configuration(b)),
        },
    }
}

fn from_node(node: &Node, grid: &pane_grid::State<PaneId>) -> LayoutNode {
    match node {
        Node::Pane(handle) => LayoutNode::pane(*grid.get(*handle).expect("layout pane has state")),
        Node::Split { axis, ratio, a, b, .. } => LayoutNode::split(
            match axis {
                pane_grid::Axis::Horizontal => Axis::Horizontal,
                pane_grid::Axis::Vertical => Axis::Vertical,
            },
            *ratio,
            from_node(a, grid),
            from_node(b, grid),
        ),
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
    fn close_moves_focus_to_sibling() {
        let mut ws = view(Preset::Columns(2));
        let ids = ws.pane_ids();
        ws.focused = Some(ids[1]);
        ws.close(ids[1]);
        assert_eq!(ws.focused, Some(ids[0]));
    }
}
