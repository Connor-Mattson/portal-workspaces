//! Domain model for Portal Workspaces.
//!
//! Everything here is plain data and pure logic: no GUI, no PTYs, no threads. The app crate turns
//! this into widgets and the terminal crate turns panes into running shells.

pub mod editor;
pub mod ids;
pub mod layout;
pub mod state;
pub mod store;
pub mod usage;
pub mod workspace;

pub use editor::{EditorSession, GroupSpec, MAX_GROUPS, Mode, OpenFile, TerminalSpec};
pub use ids::{GroupId, PaneId, ProfileId, WorkspaceId};
pub use layout::{Axis, LayoutFull, LayoutNode, MAX_PANES, Preset, SplitTree};
pub use state::{PersistedState, SCHEMA_VERSION, UiPrefs, WindowGeometry};
pub use usage::{PollEnv, Provider, UsageProfile};
pub use workspace::{PaneSpec, Workspace};
