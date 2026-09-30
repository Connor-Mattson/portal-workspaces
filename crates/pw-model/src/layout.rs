//! The split tree that arranges a workspace's terminal panes.
//!
//! The GUI keeps its own live copy (iced's `pane_grid`); this tree is the persisted and testable
//! form, and the place where layout rules such as the pane cap live.

use serde::{Deserialize, Serialize};

use crate::ids::PaneId;

/// A workspace holds at most this many terminals.
pub const MAX_PANES: usize = 8;

/// Direction of the dividing line of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// Divider runs horizontally: `a` above `b`.
    Horizontal,
    /// Divider runs vertically: `a` left of `b`.
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayoutNode {
    Split {
        axis: Axis,
        /// Share of the space given to `a`, in `(0, 1)`.
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
    Pane {
        id: PaneId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutFull;

impl std::fmt::Display for LayoutFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a workspace holds at most {MAX_PANES} terminals")
    }
}

impl std::error::Error for LayoutFull {}

impl LayoutNode {
    pub fn pane(id: PaneId) -> Self {
        Self::Pane { id }
    }

    pub fn split(axis: Axis, ratio: f32, a: LayoutNode, b: LayoutNode) -> Self {
        Self::Split { axis, ratio: clamp_ratio(ratio), a: Box::new(a), b: Box::new(b) }
    }

    /// Pane ids in reading order (depth-first, `a` before `b`).
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Pane { id } => out.push(*id),
            Self::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn pane_count(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Split { a, b, .. } => a.pane_count() + b.pane_count(),
        }
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Pane { id } => *id == pane,
            Self::Split { a, b, .. } => a.contains(pane) || b.contains(pane),
        }
    }

    /// Splits `target` in two, placing `new` after it (right or below). Enforces [`MAX_PANES`].
    ///
    /// Returns `Ok(false)` if `target` isn't in the tree.
    pub fn split_pane(&mut self, target: PaneId, axis: Axis, new: PaneId) -> Result<bool, LayoutFull> {
        if self.pane_count() >= MAX_PANES {
            return Err(LayoutFull);
        }
        Ok(self.split_inner(target, axis, new))
    }

    fn split_inner(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Self::Pane { id } if *id == target => {
                *self = Self::split(axis, 0.5, Self::pane(target), Self::pane(new));
                true
            }
            Self::Pane { .. } => false,
            Self::Split { a, b, .. } => a.split_inner(target, axis, new) || b.split_inner(target, axis, new),
        }
    }

    /// Removes `target`; its sibling takes the freed space. Returns `None` if the tree would be empty.
    pub fn close_pane(self, target: PaneId) -> Option<Self> {
        match self {
            Self::Pane { id } if id == target => None,
            pane @ Self::Pane { .. } => Some(pane),
            Self::Split { axis, ratio, a, b } => match (a.close_pane(target), b.close_pane(target)) {
                (Some(a), Some(b)) => Some(Self::Split { axis, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// Clamps every ratio into range. Used after loading a hand-edited or old state file.
    pub fn sanitized(self) -> Self {
        match self {
            pane @ Self::Pane { .. } => pane,
            Self::Split { axis, ratio, a, b } => Self::split(axis, ratio, a.sanitized(), b.sanitized()),
        }
    }
}

fn clamp_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() { ratio.clamp(0.05, 0.95) } else { 0.5 }
}

/// One-click arrangements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    Single,
    Columns(u8),
    Rows(u8),
    Grid { cols: u8, rows: u8 },
}

impl Preset {
    /// The presets offered in the UI, smallest first.
    pub const ALL: [Preset; 8] = [
        Preset::Single,
        Preset::Columns(2),
        Preset::Rows(2),
        Preset::Columns(3),
        Preset::Grid { cols: 2, rows: 2 },
        Preset::Grid { cols: 3, rows: 2 },
        Preset::Columns(4),
        Preset::Grid { cols: 4, rows: 2 },
    ];

    pub fn pane_count(self) -> usize {
        match self {
            Preset::Single => 1,
            Preset::Columns(n) | Preset::Rows(n) => n as usize,
            Preset::Grid { cols, rows } => cols as usize * rows as usize,
        }
    }

    pub fn label(self) -> String {
        match self {
            Preset::Single => "Single".into(),
            Preset::Columns(n) => format!("{n} columns"),
            Preset::Rows(n) => format!("{n} rows"),
            Preset::Grid { cols, rows } => format!("{cols} × {rows} grid"),
        }
    }

    /// Builds the tree for this preset from `panes`, which must hold exactly
    /// [`pane_count`](Self::pane_count) ids (clamped to [`MAX_PANES`]). Cells fill row by row.
    pub fn build(self, panes: &[PaneId]) -> LayoutNode {
        assert!(!panes.is_empty(), "a layout needs at least one pane");
        match self {
            Preset::Single => LayoutNode::pane(panes[0]),
            Preset::Columns(_) => even(Axis::Vertical, panes.iter().map(|&id| LayoutNode::pane(id)).collect()),
            Preset::Rows(_) => even(Axis::Horizontal, panes.iter().map(|&id| LayoutNode::pane(id)).collect()),
            Preset::Grid { cols, .. } => {
                let rows = panes
                    .chunks(cols.max(1) as usize)
                    .map(|row| even(Axis::Vertical, row.iter().map(|&id| LayoutNode::pane(id)).collect()))
                    .collect();
                even(Axis::Horizontal, rows)
            }
        }
    }
}

/// Lays `nodes` out along `axis` with equal space each, as a right-leaning chain of splits.
fn even(axis: Axis, mut nodes: Vec<LayoutNode>) -> LayoutNode {
    let n = nodes.len();
    let first = nodes.remove(0);
    if n == 1 {
        return first;
    }
    LayoutNode::split(axis, 1.0 / n as f32, first, even(axis, nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<PaneId> {
        (0..n).map(|_| PaneId::new()).collect()
    }

    #[test]
    fn split_adds_pane_after_target() {
        let p = ids(2);
        let mut tree = LayoutNode::pane(p[0]);
        assert_eq!(tree.split_pane(p[0], Axis::Vertical, p[1]), Ok(true));
        assert_eq!(tree.panes(), p);
    }

    #[test]
    fn split_unknown_target_is_noop() {
        let p = ids(2);
        let mut tree = LayoutNode::pane(p[0]);
        assert_eq!(tree.split_pane(PaneId::new(), Axis::Vertical, p[1]), Ok(false));
        assert_eq!(tree.pane_count(), 1);
    }

    #[test]
    fn split_refuses_ninth_pane() {
        let p = ids(MAX_PANES + 1);
        let mut tree = LayoutNode::pane(p[0]);
        for &id in &p[1..MAX_PANES] {
            let last = *tree.panes().last().unwrap();
            tree.split_pane(last, Axis::Vertical, id).unwrap();
        }
        assert_eq!(tree.pane_count(), MAX_PANES);
        assert_eq!(tree.split_pane(p[0], Axis::Horizontal, p[MAX_PANES]), Err(LayoutFull));
        assert_eq!(tree.pane_count(), MAX_PANES);
    }

    #[test]
    fn close_promotes_sibling() {
        let p = ids(3);
        let mut tree = LayoutNode::pane(p[0]);
        tree.split_pane(p[0], Axis::Vertical, p[1]).unwrap();
        tree.split_pane(p[1], Axis::Horizontal, p[2]).unwrap();
        let tree = tree.close_pane(p[1]).unwrap();
        assert_eq!(tree.panes(), vec![p[0], p[2]]);
        let tree = tree.close_pane(p[0]).unwrap();
        assert_eq!(tree, LayoutNode::pane(p[2]));
        assert_eq!(tree.close_pane(p[2]), None);
    }

    #[test]
    fn presets_hold_their_pane_count_in_order() {
        for preset in Preset::ALL {
            let p = ids(preset.pane_count());
            let tree = preset.build(&p);
            assert_eq!(tree.panes(), p, "{preset:?}");
            assert!(tree.pane_count() <= MAX_PANES);
        }
    }

    #[test]
    fn columns_are_even() {
        // Three columns: first split gives 1/3, the nested one splits the rest in half.
        let tree = Preset::Columns(3).build(&ids(3));
        let LayoutNode::Split { ratio, b, axis, .. } = tree else { panic!() };
        assert_eq!(axis, Axis::Vertical);
        assert!((ratio - 1.0 / 3.0).abs() < 1e-6);
        let LayoutNode::Split { ratio, .. } = *b else { panic!() };
        assert!((ratio - 0.5).abs() < 1e-6);
    }

    #[test]
    fn grid_is_rows_of_columns() {
        let p = ids(4);
        let tree = Preset::Grid { cols: 2, rows: 2 }.build(&p);
        let LayoutNode::Split { axis, a, b, .. } = tree else { panic!() };
        assert_eq!(axis, Axis::Horizontal);
        assert_eq!(a.panes(), vec![p[0], p[1]]);
        assert_eq!(b.panes(), vec![p[2], p[3]]);
    }

    #[test]
    fn ratios_are_clamped() {
        let p = ids(2);
        let tree = LayoutNode::Split {
            axis: Axis::Vertical,
            ratio: f32::NAN,
            a: Box::new(LayoutNode::pane(p[0])),
            b: Box::new(LayoutNode::Split {
                axis: Axis::Vertical,
                ratio: 7.0,
                a: Box::new(LayoutNode::pane(p[1])),
                b: Box::new(LayoutNode::pane(PaneId::new())),
            }),
        }
        .sanitized();
        let LayoutNode::Split { ratio, b, .. } = tree else { panic!() };
        assert_eq!(ratio, 0.5);
        let LayoutNode::Split { ratio, .. } = *b else { panic!() };
        assert_eq!(ratio, 0.95);
    }
}
