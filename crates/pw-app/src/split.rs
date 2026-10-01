//! Converting between the model's split trees and iced's live `pane_grid` state. Agent terminals and
//! editor groups both use these.

use iced::widget::pane_grid::{self, Configuration, Node, Pane};
use pw_model::{Axis, SplitTree};

use crate::keymap::Direction;

pub fn to_iced_axis(axis: Axis) -> pane_grid::Axis {
    match axis {
        Axis::Horizontal => pane_grid::Axis::Horizontal,
        Axis::Vertical => pane_grid::Axis::Vertical,
    }
}

/// A live grid laid out as `tree`.
pub fn grid<Id: Copy + PartialEq>(tree: &SplitTree<Id>) -> pane_grid::State<Id> {
    pane_grid::State::with_configuration(configuration(tree))
}

fn configuration<Id: Copy + PartialEq>(node: &SplitTree<Id>) -> Configuration<Id> {
    match node {
        SplitTree::Pane { id } => Configuration::Pane(*id),
        SplitTree::Split { axis, ratio, a, b } => Configuration::Split {
            axis: to_iced_axis(*axis),
            ratio: *ratio,
            a: Box::new(configuration(a)),
            b: Box::new(configuration(b)),
        },
    }
}

/// The tree a live grid is laid out as.
pub fn tree<Id: Copy + PartialEq>(grid: &pane_grid::State<Id>) -> SplitTree<Id> {
    from_node(grid.layout(), grid)
}

fn from_node<Id: Copy + PartialEq>(node: &Node, grid: &pane_grid::State<Id>) -> SplitTree<Id> {
    match node {
        Node::Pane(handle) => SplitTree::pane(*grid.get(*handle).expect("layout pane has state")),
        Node::Split { axis, ratio, a, b, .. } => SplitTree::split(
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

/// The handle of the pane showing `id`.
pub fn handle<Id: PartialEq>(grid: &pane_grid::State<Id>, id: &Id) -> Option<Pane> {
    grid.iter().find(|(_, p)| *p == id).map(|(h, _)| *h)
}

/// The pane next to `from` in `direction`.
pub fn adjacent<Id: Copy + PartialEq>(grid: &pane_grid::State<Id>, from: &Id, direction: Direction) -> Option<Id> {
    let from = handle(grid, from)?;
    let direction = match direction {
        Direction::Left => pane_grid::Direction::Left,
        Direction::Right => pane_grid::Direction::Right,
        Direction::Up => pane_grid::Direction::Up,
        Direction::Down => pane_grid::Direction::Down,
    };
    grid.adjacent(from, direction).and_then(|h| grid.get(h).copied())
}
