//! Stable identifiers. They survive restarts, so persisted layouts can refer to panes.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Short form is plenty for logs.
                write!(f, "{}", &self.0.simple().to_string()[..8])
            }
        }
    };
}

id_type!(
    /// Identifies a workspace (one project in the drawer).
    WorkspaceId
);
id_type!(
    /// Identifies a terminal pane. A running session is keyed by this id.
    PaneId
);
