//! The find (and replace) bar of an editor group.
//!
//! Matches are recomputed only when the query, the tab or its document's text changed (the
//! revision), so an open find bar costs one scan of the file per edit.

use pw_code::find::{self, Match};

use super::buffer::to_pos;
use crate::ui::editor::code_editor::Content;

#[derive(Debug, Default)]
pub struct Find {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub show_replace: bool,
    /// In document order.
    pub matches: Vec<Match>,
    /// The match selected in the editor, if any.
    pub current: Option<usize>,
    /// What `matches` was computed for: tab, revision, query, case.
    key: Option<(u64, u64, String, bool)>,
}

impl Find {
    /// Recomputes the matches if anything they depend on changed, and which one is selected.
    pub fn refresh(&mut self, view: Option<(u64, u64, &Content)>) {
        let Some((tab, revision, view)) = view else {
            self.matches.clear();
            self.current = None;
            self.key = None;
            return;
        };
        let key = (tab, revision, self.query.clone(), self.case_sensitive);
        if self.key.as_ref() != Some(&key) {
            self.matches = view.with_lines(|lines| find::find_all(lines, &self.query, self.case_sensitive));
            self.key = Some(key);
        }
        self.current = find::selected(&self.matches, selection(view));
    }

    /// The match after (or before) the cursor, wrapping around.
    pub fn step(&self, view: &Content, forward: bool) -> Option<Match> {
        find::step(&self.matches, selection(view), forward).map(|i| self.matches[i].clone())
    }

    pub fn replace_all(&self, view: &Content) -> String {
        view.with_lines(|lines| find::replace_all(lines, &self.matches, &self.replacement))
    }
}

/// The view's selection as ordered (start, end), or the caret twice.
pub fn selection(view: &Content) -> (pw_code::Pos, pw_code::Pos) {
    let cursor = view.cursor();
    let (a, b) = (to_pos(cursor.selection.unwrap_or(cursor.position)), to_pos(cursor.position));
    if a <= b { (a, b) } else { (b, a) }
}
