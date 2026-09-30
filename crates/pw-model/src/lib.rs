//! Domain model for Portal Workspaces.
//!
//! Everything here is plain data and pure logic: no GUI, no PTYs, no threads. The app crate turns
//! this into widgets and the terminal crate turns panes into running shells.

pub mod ids;
pub mod layout;
pub mod state;
pub mod store;
pub mod workspace;

pub use ids::{PaneId, WorkspaceId};
pub use layout::{Axis, LayoutFull, LayoutNode, MAX_PANES, Preset};
pub use state::{PersistedState, SCHEMA_VERSION, UiPrefs, WindowGeometry};
pub use workspace::{PaneSpec, Workspace};
