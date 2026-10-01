//! Undo and redo for a document.
//!
//! Iced's editor has no history, so we keep one. Edits are grouped the way people expect: a run
//! of typing is one step, and a new step starts after a pause, when the kind of edit changes
//! (typing → deleting), on a paste or new line, or when the cursor was moved in between. Each
//! step stores the whole text before it, which can't drift out of sync with the text the way an
//! operation log can. Steps are capped in number and in bytes, oldest dropped first, so a large
//! file's history stays bounded.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iced::widget::text_editor::{Action, Cursor, Edit};

/// Most steps kept.
const LIMIT: usize = 500;
/// Most bytes of snapshots kept per document.
const BUDGET: usize = 32 * 1024 * 1024;
/// A pause this long starts a new step.
const PAUSE: Duration = Duration::from_millis(900);

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub text: String,
    pub cursor: Cursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Word,
    Space,
    Newline,
    Delete,
    Paste,
    /// Programmatic changes (reload from disk, comment toggling, indenting). Always its own step.
    Bulk,
}

impl EditKind {
    pub fn of(action: &Action) -> Option<Self> {
        let Action::Edit(edit) = action else { return None };
        Some(match edit {
            Edit::Insert(c) if c.is_whitespace() => EditKind::Space,
            Edit::Insert(_) => EditKind::Word,
            Edit::Enter => EditKind::Newline,
            Edit::Backspace | Edit::Delete => EditKind::Delete,
            Edit::Paste(_) => EditKind::Paste,
            Edit::Indent | Edit::Unindent => EditKind::Bulk,
        })
    }
}

#[derive(Debug, Default)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    /// Bytes held by `undo` and `redo`.
    bytes: usize,
    /// The last edit: when, what kind, and where it left the cursor.
    last: Option<(Instant, EditKind, Cursor)>,
}

impl History {
    /// Called before an edit is applied. `current` is only evaluated when a new step starts.
    pub fn before_edit(&mut self, kind: EditKind, cursor: Cursor, current: impl FnOnce() -> Snapshot) {
        let now = Instant::now();
        let continues = self.last.as_ref().is_some_and(|(at, last_kind, left_at)| {
            now.duration_since(*at) < PAUSE
                && *left_at == cursor
                && kind != EditKind::Bulk
                && kind != EditKind::Paste
                // Typing a space after a word continues it; a word after spaces starts anew.
                && (*last_kind == kind || (*last_kind == EditKind::Word && kind == EditKind::Space))
        });
        if !continues {
            self.push_undo(current());
            self.bytes -= self.redo.iter().map(|s| s.text.len()).sum::<usize>();
            self.redo.clear();
        }
        self.last = Some((now, kind, cursor));
    }

    /// Records where the edit left the cursor, so the next edit can tell if it moved.
    pub fn after_edit(&mut self, cursor: Cursor) {
        if let Some(last) = &mut self.last {
            last.2 = cursor;
        }
    }

    /// Ends the current step, so the next edit starts a new one.
    pub fn seal(&mut self) {
        self.last = None;
    }

    /// The state to go back to, given the current one (which becomes redoable).
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let target = self.undo.pop_back()?;
        self.bytes = self.bytes - target.text.len() + current.text.len();
        self.redo.push(current);
        self.last = None;
        Some(target)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let target = self.redo.pop()?;
        self.bytes -= target.text.len();
        self.push_undo(current);
        self.last = None;
        Some(target)
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        self.bytes += snapshot.text.len();
        self.undo.push_back(snapshot);
        while self.undo.len() > LIMIT || (self.bytes > BUDGET && self.undo.len() > 1) {
            let dropped = self.undo.pop_front().expect("non-empty");
            self.bytes -= dropped.text.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::Position;

    fn at(column: usize) -> Cursor {
        Cursor { position: Position { line: 0, column }, selection: None }
    }

    fn snap(text: &str) -> Snapshot {
        Snapshot { text: text.to_owned(), cursor: at(text.len()) }
    }

    #[test]
    fn typing_groups_into_one_step() {
        let mut h = History::default();
        let mut text = String::new();
        for c in "hello world".chars() {
            let kind = if c == ' ' { EditKind::Space } else { EditKind::Word };
            h.before_edit(kind, at(text.len()), || snap(&text));
            text.push(c);
            h.after_edit(at(text.len()));
        }
        // "hello" + " " is one step; "world" after a space starts the second.
        let back = h.undo(snap(&text)).unwrap();
        assert_eq!(back.text, "hello ");
        let back = h.undo(snap(&back.text)).unwrap();
        assert_eq!(back.text, "");
        assert!(h.undo(snap("")).is_none());
        assert_eq!(h.redo(snap("")).unwrap().text, "hello ");
    }

    #[test]
    fn moving_the_cursor_or_sealing_starts_a_new_step() {
        let mut h = History::default();
        h.before_edit(EditKind::Word, at(0), || snap(""));
        h.after_edit(at(1));
        // The user clicked elsewhere before typing again.
        h.before_edit(EditKind::Word, at(0), || snap("a"));
        h.after_edit(at(1));
        h.seal();
        h.before_edit(EditKind::Word, at(1), || snap("ba"));
        h.after_edit(at(2));
        assert_eq!(h.undo(snap("bac")).unwrap().text, "ba");
        assert_eq!(h.undo(snap("ba")).unwrap().text, "a");
    }

    #[test]
    fn new_edits_clear_redo() {
        let mut h = History::default();
        h.before_edit(EditKind::Paste, at(0), || snap(""));
        let _ = h.undo(snap("pasted"));
        h.before_edit(EditKind::Word, at(0), || snap(""));
        assert!(h.redo(snap("x")).is_none());
        assert_eq!(h.bytes, 0);
    }

    #[test]
    fn history_is_bounded_by_bytes() {
        let mut h = History::default();
        let big = "x".repeat(BUDGET / 3);
        for _ in 0..5 {
            h.before_edit(EditKind::Bulk, at(0), || snap(&big));
        }
        assert!(h.bytes <= BUDGET);
        assert_eq!(h.undo.len(), 3);
    }
}
