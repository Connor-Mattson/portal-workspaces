//! Terminal engine for Portal Workspaces.
//!
//! A [`Session`] is one shell running in a PTY, parsed by `alacritty_terminal` on its own IO
//! thread. The GUI talks to it through plain types only: it sends [`KeyInput`]s, bytes and
//! resizes, gets [`TermEvent`]s through a callback, and draws [`Snapshot`]s. No alacritty types
//! leak out of this crate, so the renderer and the emulator can change independently.

mod activity;
pub mod color;
pub mod cwd;
mod events;
pub mod input;
pub mod mouse;
pub mod notify;
pub mod session;
pub mod snapshot;
mod tap;

pub use activity::ActivityConfig;
pub use color::{Palette, Rgb};
pub use events::TermEvent;
pub use input::{Key, KeyInput, Mods, NamedKey};
pub use mouse::{MouseButton, MouseEvent, MouseEventKind};
pub use session::{GridPoint, GridSize, SelectionKind, Session, SessionConfig, SpawnError};
pub use snapshot::{BgRun, CursorShape, CursorSnap, Row, Snapshot, TextRun};
