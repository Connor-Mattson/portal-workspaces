//! Code editing knowledge for Portal Workspaces' Editor mode.
//!
//! No GUI types and no threads: languages, syntax highlighting, editing rules, find, directory
//! listings and fuzzy matching, each unit-tested. The app turns them into an editor.

pub mod editing;
pub mod find;
pub mod fs;
pub mod fuzzy;
pub mod language;
pub mod syntax;
pub mod text;

pub use language::{Indent, Language, Tint};
pub use text::{Pos, TextEdit};
