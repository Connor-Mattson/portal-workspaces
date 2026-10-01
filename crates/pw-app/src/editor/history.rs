//! Undo and redo for a document.
//!
//! Iced's editor has no history, so we keep one. Edits are grouped the way people expect: a run
//! of typing is one step, and a new step starts after a pause, when the kind of edit changes
//! (typing → deleting), on a paste or new line, or when the cursor was moved in between.
//!
//! The step being typed holds the whole text from before it. When the next step starts, it's
//! closed into a [`Step`]: just the span that differs between the text before and after it,
//! found by comparing the two (not by logging the widget's operations, which could drift out of
//! sync with the text). So a closed step costs the bytes it changed, not the file's size, and a
//! big file keeps as many steps as a small one. Applying a step first checks the text still
//! has what the step expects; if it doesn't, the history is dropped rather than applied to the
//! wrong text. Steps are capped in number and in bytes, oldest dropped first.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iced::widget::text_editor::{Action, Cursor, Edit};
use pw_code::text::{common_prefix, common_suffix};

/// Most steps kept.
const LIMIT: usize = 500;
/// Most bytes kept per document, the open step's whole text included.
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

/// One closed step: at byte `at`, `before` became `after`. The rest of the text was the same on
/// both sides, and is `len` bytes long with `after` in place.
#[derive(Debug, Clone)]
struct Step {
    at: usize,
    before: String,
    after: String,
    /// The whole text's length after the step.
    len: usize,
    cursor_before: Cursor,
    cursor_after: Cursor,
}

impl Step {
    /// The step that turned `before` into `after`, or `None` if nothing changed.
    fn between(before: &Snapshot, after: &str, cursor_after: Cursor) -> Option<Self> {
        let (old, new) = (&before.text, after);
        if old == new {
            return None;
        }
        let at = common_prefix(old, new);
        let suffix = common_suffix(&old[at..], &new[at..]);
        Some(Self {
            at,
            before: old[at..old.len() - suffix].to_owned(),
            after: new[at..new.len() - suffix].to_owned(),
            len: new.len(),
            cursor_before: before.cursor,
            cursor_after,
        })
    }

    fn bytes(&self) -> usize {
        self.before.len() + self.after.len()
    }

    /// `text` with the step undone, or `None` if `text` isn't what the step left.
    fn undo(&self, text: &str) -> Option<String> {
        (text.len() == self.len).then_some(())?;
        swap(text, self.at, &self.after, &self.before)
    }

    /// `text` with the step redone, or `None` if `text` isn't what it was before the step.
    fn redo(&self, text: &str) -> Option<String> {
        (text.len() + self.after.len() == self.len + self.before.len()).then_some(())?;
        swap(text, self.at, &self.before, &self.after)
    }
}

/// `text` with `from` at `at` replaced by `to`, if `from` is there.
fn swap(text: &str, at: usize, from: &str, to: &str) -> Option<String> {
    (text.get(at..at + from.len())? == from).then_some(())?;
    let mut out = String::with_capacity(text.len() - from.len() + to.len());
    out.push_str(&text[..at]);
    out.push_str(to);
    out.push_str(&text[at + from.len()..]);
    Some(out)
}

#[derive(Debug, Default)]
pub struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    /// The text before the step being made (the newest one, not yet closed into a [`Step`]).
    open: Option<Snapshot>,
    /// Bytes held by `undo`, `redo` and `open`.
    bytes: usize,
    /// The last edit: when, what kind, and where it left the cursor.
    last: Option<(Instant, EditKind, Cursor)>,
}

impl History {
    /// Called before an edit is applied. `current` is only evaluated when a new step starts.
    pub fn before_edit(&mut self, kind: EditKind, cursor: Cursor, current: impl FnOnce() -> Snapshot) {
        let now = Instant::now();
        let continues = self.open.is_some()
            && self.last.as_ref().is_some_and(|(at, last_kind, left_at)| {
                now.duration_since(*at) < PAUSE
                    && *left_at == cursor
                    && kind != EditKind::Bulk
                    && kind != EditKind::Paste
                    // Typing a space after a word continues it; a word after spaces starts anew.
                    && (*last_kind == kind || (*last_kind == EditKind::Word && kind == EditKind::Space))
            });
        if !continues {
            let current = current();
            self.close(&current);
            self.bytes += current.text.len();
            self.open = Some(current);
            self.bytes -= self.redo.drain(..).map(|s| s.bytes()).sum::<usize>();
            self.trim();
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
        self.close(&current);
        self.last = None;
        let step = self.undo.pop_back()?;
        let Some(text) = step.undo(&current.text) else { return self.lost() };
        let cursor = step.cursor_before;
        self.redo.push(step);
        Some(Snapshot { text, cursor })
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        self.close(&current);
        self.last = None;
        let step = self.redo.pop()?;
        let Some(text) = step.redo(&current.text) else { return self.lost() };
        let cursor = step.cursor_after;
        self.undo.push_back(step);
        Some(Snapshot { text, cursor })
    }

    /// Closes the open step, which `current` ends. Redoing it puts the cursor where its last
    /// edit left it, if that's known.
    fn close(&mut self, current: &Snapshot) {
        let Some(open) = self.open.take() else { return };
        self.bytes -= open.text.len();
        let cursor = self.last.map_or(current.cursor, |(_, _, left_at)| left_at);
        if let Some(step) = Step::between(&open, &current.text, cursor) {
            self.bytes += step.bytes();
            self.undo.push_back(step);
        }
    }

    /// The text isn't what the history expects: something changed it without going through
    /// here. Applying a step now would garble it, so forget everything instead.
    fn lost(&mut self) -> Option<Snapshot> {
        tracing::warn!("undo history no longer matches the text; dropping it");
        *self = Self::default();
        None
    }

    fn trim(&mut self) {
        while self.undo.len() > LIMIT || (self.bytes > BUDGET && !self.undo.is_empty()) {
            let dropped = self.undo.pop_front().expect("non-empty");
            self.bytes -= dropped.bytes();
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

    fn at_column(column: usize) -> Cursor {
        at(column)
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
        assert_eq!(h.bytes, "pasted".len());
        h.before_edit(EditKind::Word, at(0), || snap(""));
        assert!(h.redo(snap("x")).is_none());
        // Only the new step ("" → "x") is left.
        assert_eq!(h.bytes, 1);
        assert_eq!(h.undo(snap("x")).unwrap().text, "");
    }

    /// Drives a history the way `Buffer` does, remembering every text a step ended on.
    struct Session {
        h: History,
        text: String,
        states: Vec<String>,
    }

    impl Session {
        fn new(text: &str) -> Self {
            Self { h: History::default(), text: text.to_owned(), states: vec![text.to_owned()] }
        }

        /// Replaces `delete` bytes at `at` with `insert`, the cursor moving from `from` to `to`.
        fn edit(&mut self, kind: EditKind, (from, to): (usize, usize), at: usize, delete: usize, insert: &str) {
            let text = &self.text;
            self.h.before_edit(kind, at_column(from), || Snapshot { text: text.clone(), cursor: at_column(from) });
            self.text.replace_range(at..at + delete, insert);
            self.h.after_edit(at_column(to));
        }

        /// One step: typing, a paste, deleting, a bulk edit or a reload from disk (the whole
        /// text replaced, as `Buffer::set_text` does).
        fn step(&mut self, kind: EditKind, at: usize, delete: usize, insert: &str) {
            self.h.seal();
            let at = self.boundary(at);
            match kind {
                // A key at a time, each continuing the step.
                EditKind::Word => {
                    for (i, c) in insert.char_indices() {
                        self.edit(kind, (at + i, at + i + c.len_utf8()), at + i, 0, &c.to_string());
                    }
                }
                // Backspaces from the end of the span.
                EditKind::Delete => {
                    let mut end = self.boundary(at + delete);
                    while end > at {
                        let start = self.text[..end].char_indices().next_back().map_or(0, |(i, _)| i);
                        self.edit(kind, (end, start), start, end - start, "");
                        end = start;
                    }
                }
                _ => {
                    let delete = self.boundary(at + delete) - at;
                    self.edit(kind, (at, at + insert.len()), at, delete, insert);
                }
            }
            self.states.push(self.text.clone());
        }

        /// The nearest char boundary at or before `at`.
        fn boundary(&self, at: usize) -> usize {
            (0..=at.min(self.text.len())).rev().find(|&i| self.text.is_char_boundary(i)).unwrap_or(0)
        }

        fn undo(&mut self) -> Option<String> {
            let back = self.h.undo(snap(&self.text))?;
            self.text = back.text.clone();
            Some(back.text)
        }

        fn redo(&mut self) -> Option<String> {
            let forward = self.h.redo(snap(&self.text))?;
            self.text = forward.text.clone();
            Some(forward.text)
        }

        /// Undoes to the start and redoes to the end, checking every text on the way.
        fn round_trip(&mut self) {
            for want in self.states.clone().iter().rev().skip(1) {
                assert!(self.undo().as_ref() == Some(want), "undo didn't restore a step");
            }
            assert!(self.undo().is_none());
            for want in self.states.clone().iter().skip(1) {
                assert!(self.redo().as_ref() == Some(want), "redo didn't restore a step");
            }
            assert!(self.redo().is_none());
        }
    }

    fn mixed_session(start: &str) -> Session {
        let mut s = Session::new(start);
        let mid = start.len() / 2;
        s.step(EditKind::Word, 0, 0, "let x");
        s.step(EditKind::Space, 5, 0, " ");
        s.step(EditKind::Paste, mid, 0, "pasted text\nover lines\n");
        s.step(EditKind::Delete, mid + 3, 4, "");
        s.step(EditKind::Newline, 2, 0, "\n    ");
        // Comment toggling, indenting: one span replaced.
        s.step(EditKind::Bulk, 0, 6, "// let");
        // Reloaded from disk with something else entirely.
        let reloaded = s.text.replace('e', "é");
        let len = s.text.len();
        s.step(EditKind::Bulk, 0, len, &reloaded);
        // Typing multibyte text after it.
        s.step(EditKind::Word, 1, 0, "日本");
        s
    }

    #[test]
    fn undo_and_redo_round_trip_on_a_small_document() {
        mixed_session("fn main() {\n    println!(\"hi\");\n}\n").round_trip();
        mixed_session("").round_trip();
    }

    #[test]
    fn undo_and_redo_round_trip_on_a_large_document() {
        let line = "fn example() { let value = compute(42); }\n";
        let big = line.repeat(crate::editor::document::MAX_BYTES as usize / line.len());
        let mut s = mixed_session(&big);
        for i in 0..20 {
            s.step(EditKind::Word, i * 1000, 0, "x");
        }
        // Every step fits (the newest is still open): each costs what it changed, the reload its
        // two whole texts.
        assert_eq!(s.h.undo.len(), s.states.len() - 2);
        assert!(s.h.bytes <= BUDGET);
        s.round_trip();
    }

    #[test]
    fn undo_then_new_edits_then_undo_again() {
        let mut s = Session::new("abc");
        s.step(EditKind::Word, 3, 0, "def");
        s.step(EditKind::Word, 0, 0, "x");
        assert_eq!(s.undo().unwrap(), "abcdef");
        s.states.pop();
        s.step(EditKind::Paste, 3, 0, "123");
        s.round_trip();
    }

    #[test]
    fn history_is_bounded_by_bytes() {
        let mut s = Session::new("");
        let chunk = "x".repeat(BUDGET / 10);
        for _ in 0..8 {
            s.step(EditKind::Paste, 0, 0, &chunk);
        }
        assert!(s.h.bytes <= BUDGET);
        // The open step holds the whole text before it (seven chunks); closed steps, one each.
        assert_eq!(s.h.undo.len(), 3);
        while s.undo().is_some() {}
        // The open step and the three kept undone: four chunks are left.
        assert_eq!(s.text.len(), 4 * chunk.len());
    }

    #[test]
    fn a_text_changed_behind_its_back_drops_the_history_instead_of_garbling() {
        let mut s = Session::new("abc");
        s.step(EditKind::Paste, 3, 0, "def");
        s.step(EditKind::Paste, 0, 0, "123");
        assert_eq!(s.undo().unwrap(), "abcdef");
        // Not "abcdef": something edited the text without the history seeing it.
        s.text = "abcdXf".into();
        assert!(s.undo().is_none());
        assert!(s.redo().is_none());
        assert_eq!(s.h.bytes, 0);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use iced::widget::text_editor::Position;

    /// `cargo test -p pw-app history_cost -- --ignored --nocapture`: a long session on a file at
    /// `MAX_BYTES`, one small edit per step.
    #[test]
    #[ignore]
    fn history_cost() {
        let at = Cursor { position: Position { line: 0, column: 0 }, selection: None };
        let line = "fn example() { let value = compute(42); }\n";
        let mut text = line.repeat(8 * 1024 * 1024 / line.len());
        let mut h = History::default();
        let started = Instant::now();
        let steps = 200;
        for i in 0..steps {
            h.seal();
            let snapshot = text.clone();
            h.before_edit(EditKind::Word, at, || Snapshot { text: snapshot, cursor: at });
            let offset = (i * 7919 * line.len()) % (text.len() - 1);
            text.insert(offset, 'x');
        }
        let elapsed = started.elapsed();
        let held = h.bytes;
        let mut undone = 0;
        let undo_started = Instant::now();
        let mut current = text.clone();
        while let Some(back) = h.undo(Snapshot { text: current.clone(), cursor: at }) {
            current = back.text;
            undone += 1;
        }
        println!(
            "{steps} steps on {} MB: {:?} per step, {} MB held, {undone} undoable, undo {:?} each",
            text.len() / (1024 * 1024),
            elapsed / steps as u32,
            held as f64 / (1024.0 * 1024.0),
            undo_started.elapsed() / undone.max(1) as u32,
        );
    }
}
