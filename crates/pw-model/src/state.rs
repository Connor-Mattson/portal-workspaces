//! Everything the app remembers between runs.

use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;
use crate::workspace::Workspace;

/// Bump when the on-disk shape changes, and teach [`crate::store::migrate`] the old shape.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    pub schema_version: u32,
    /// Drawer order.
    pub workspaces: Vec<Workspace>,
    pub active: Option<WorkspaceId>,
    #[serde(default)]
    pub ui: UiPrefs,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, workspaces: Vec::new(), active: None, ui: UiPrefs::default() }
    }
}

impl PersistedState {
    /// Repairs every workspace and points `active` at one that exists.
    pub fn repaired(mut self) -> Self {
        self.workspaces = self.workspaces.into_iter().map(Workspace::repaired).collect();
        if !self.active.is_some_and(|a| self.workspaces.iter().any(|w| w.id == a)) {
            self.active = self.workspaces.first().map(|w| w.id);
        }
        self.ui.font_size = self.ui.font_size.clamp(UiPrefs::MIN_FONT, UiPrefs::MAX_FONT);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub sidebar_collapsed: bool,
    pub font_size: f32,
    pub window: Option<WindowGeometry>,
}

impl UiPrefs {
    pub const MIN_FONT: f32 = 9.0;
    pub const MAX_FONT: f32 = 24.0;
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self { sidebar_collapsed: false, font_size: 13.0, window: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
}
