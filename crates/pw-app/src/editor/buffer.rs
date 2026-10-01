//! Editing a document through one of its views: typing with pairs, smart Enter, indenting,
//! comments, moving lines, undo. The language's rules come from `pw_code::editing`.
//!
//! A `Buffer` is a short-lived pairing of a [`Document`] and a view's [`Content`]. The editor
//! creates one inside `EditorView::change`, which then copies the result to the document's other
//! views.

use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
use pw_code::editing::{self, Enter};
use pw_code::text::{Pos, TextEdit};

use super::document::Document;
use super::history::{EditKind, Snapshot};
use crate::ui::editor::code_editor::Content;

pub struct Buffer<'a> {
    pub doc: &'a mut Document,
    pub view: &'a mut Content,
}

impl Buffer<'_> {
    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.view.text(), cursor: self.view.cursor() }
    }

    /// Applies an editor action, recording history for edits.
    pub fn perform(&mut self, action: Action) {
        let Some(kind) = EditKind::of(&action) else {
            self.view.perform(action);
            return;
        };
        let view = &*self.view;
        self.doc.history.before_edit(kind, view.cursor(), || Snapshot { text: view.text(), cursor: view.cursor() });
        self.view.perform(action);
        self.doc.history.after_edit(self.view.cursor());
        self.doc.changed(true);
    }

    pub fn undo(&mut self) {
        if let Some(target) = self.doc.history.undo(self.snapshot()) {
            self.restore(target);
        }
    }

    pub fn redo(&mut self) {
        if let Some(target) = self.doc.history.redo(self.snapshot()) {
            self.restore(target);
        }
    }

    fn restore(&mut self, target: Snapshot) {
        let current = self.view.text();
        replace_text(self.view, &current, &target.text);
        self.view.move_to(clamp_cursor(self.view, target.cursor));
        let dirty = !self.doc.matches_disk(&target.text);
        self.doc.changed(dirty);
    }

    /// Replaces the whole text as one undoable step, touching only the part that differs (so the
    /// view doesn't jump). The cursor stays on its text.
    pub fn set_text(&mut self, text: &str) {
        let current = self.view.text();
        if current == text {
            return;
        }
        let view = &*self.view;
        self.doc
            .history
            .before_edit(EditKind::Bulk, view.cursor(), || Snapshot { text: current.clone(), cursor: view.cursor() });
        let cursor = self.view.cursor();
        replace_text(self.view, &current, text);
        let edit = TextEdit::between(&current, text).expect("texts differ");
        let shift = |p: Position| to_position(edit.shift(to_pos(p)));
        let moved = Cursor { position: shift(cursor.position), selection: cursor.selection.map(shift) };
        self.view.move_to(clamp_cursor(self.view, moved));
        self.doc.history.after_edit(self.view.cursor());
        self.doc.history.seal();
        let dirty = !self.doc.matches_disk(text);
        self.doc.changed(dirty);
    }

    /// Selects a span on one line (find results), scrolling it into view.
    pub fn select(&mut self, line: usize, range: std::ops::Range<usize>) {
        let cursor = Cursor {
            position: Position { line, column: range.end },
            selection: Some(Position { line, column: range.start }),
        };
        self.view.move_to(clamp_cursor(self.view, cursor));
    }

    /// Characters around the cursor on its line: (before, after).
    fn around_cursor(&self) -> (Option<char>, Option<char>) {
        let Position { line, column } = self.view.cursor().position;
        let Some(text) = self.view.line(line).map(|l| l.text) else { return (None, None) };
        let column = column.min(text.len());
        (text[..column].chars().next_back(), text[column..].chars().next())
    }

    /// Typing a character with pairs: brackets and quotes close themselves, typing a closer that's
    /// already there steps over it, and with a selection a bracket or quote wraps it.
    pub fn type_char(&mut self, c: char) {
        let lang = self.doc.language;
        if let Some(selected) = self.view.selection().filter(|s| !s.is_empty()) {
            match lang.closer(c) {
                Some(close) if !selected.contains('\n') || matches!(c, '(' | '[' | '{') => {
                    self.perform(Action::Edit(Edit::Paste(Arc::new(format!("{c}{selected}{close}")))));
                }
                _ => self.perform(Action::Edit(Edit::Insert(c))),
            }
            return;
        }
        let (before, after) = self.around_cursor();
        if editing::steps_over(lang, c, after) {
            self.perform(Action::Move(Motion::Right));
        } else if editing::should_autoclose(lang, c, before, after)
            && let Some(close) = lang.closer(c)
        {
            self.perform(Action::Edit(Edit::Insert(c)));
            self.perform(Action::Edit(Edit::Insert(close)));
            self.perform(Action::Move(Motion::Left));
        } else {
            self.perform(Action::Edit(Edit::Insert(c)));
        }
    }

    /// Backspace, which also removes the closer of an empty pair (`(|)`) and a whole indent level in
    /// leading whitespace.
    pub fn backspace(&mut self) {
        let cursor = self.view.cursor();
        if cursor.selection.is_none() {
            let (before, after) = self.around_cursor();
            if editing::deletes_pair(self.doc.language, before, after) {
                self.perform(Action::Edit(Edit::Delete));
            } else if let pw_code::Indent::Spaces(n) = self.doc.indent {
                let Position { line, column } = cursor.position;
                let text = self.view.line(line).map(|l| l.text.into_owned()).unwrap_or_default();
                let leading = &text[..column.min(text.len())];
                if column > 0 && leading.bytes().all(|b| b == b' ') {
                    let remove = match column % n as usize {
                        0 => n as usize,
                        r => r,
                    };
                    for _ in 1..remove {
                        self.perform(Action::Edit(Edit::Backspace));
                    }
                }
            }
        }
        self.perform(Action::Edit(Edit::Backspace));
    }

    /// Enter: keeps the line's indentation, indents after an opener, and between a pair puts the
    /// closer on its own line below.
    pub fn newline(&mut self) {
        let indent = self.current_indent();
        let unit = self.doc.indent.unit();
        let cursor = self.view.cursor();
        if cursor.selection.is_some() {
            self.perform(Action::Edit(Edit::Enter));
            self.insert_text(&indent);
            return;
        }
        let Position { line, column } = cursor.position;
        let text = self.view.line(line).map(|l| l.text.into_owned()).unwrap_or_default();
        let (before, after) = text.split_at(column.min(text.len()));
        let rule = editing::enter(self.doc.language, before, after);
        // Spaces right after the cursor would end up in front of the next line's text.
        let trailing = after.len() - after.trim_start().len();
        self.perform(Action::Edit(Edit::Enter));
        for _ in 0..trailing {
            self.perform(Action::Edit(Edit::Delete));
        }
        match rule {
            Enter::Keep => self.insert_text(&indent),
            Enter::Indent => self.insert_text(&format!("{indent}{unit}")),
            Enter::Split => {
                self.insert_text(&format!("{indent}{unit}\n{indent}"));
                self.perform(Action::Move(Motion::Up));
                self.perform(Action::Move(Motion::End));
            }
        }
    }

    fn insert_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.perform(Action::Edit(Edit::Paste(Arc::new(text.to_owned()))));
        }
    }

    /// Leading whitespace of the cursor's line.
    fn current_indent(&self) -> String {
        let line = self.view.cursor().position.line;
        self.view
            .line(line)
            .map(|l| l.text.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
            .unwrap_or_default()
    }

    /// Tab: indents the selected lines, or inserts one indent unit at the cursor.
    pub fn indent(&mut self) {
        let unit = self.doc.indent.unit();
        if self.selected_lines().is_none() {
            let column = self.view.cursor().position.column;
            let insert = match self.doc.indent {
                // Up to the next tab stop.
                pw_code::Indent::Spaces(n) => " ".repeat(n as usize - column % n as usize),
                pw_code::Indent::Tabs => unit,
            };
            self.insert_text(&insert);
            return;
        }
        self.edit_lines(|line| if line.trim().is_empty() { line.to_owned() } else { format!("{unit}{line}") });
    }

    /// Shift+Tab: outdents the cursor's (or selected) lines.
    pub fn unindent(&mut self) {
        let indent = self.doc.indent;
        self.edit_lines(|line| editing::outdent(line, indent).to_owned());
    }

    /// Comments the cursor's (or selected) lines out, or back in.
    pub fn toggle_comment(&mut self) {
        let (first, last) = self.line_span();
        let lines: Vec<String> =
            (first..=last).filter_map(|i| self.view.line(i).map(|l| l.text.into_owned())).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let Some(toggled) = editing::toggle_comment(self.doc.language, &refs) else { return };
        let mut toggled = toggled.into_iter();
        self.edit_lines(|_| toggled.next().unwrap_or_default());
    }

    /// Moves the cursor's (or selected) lines up or down by one, as one undoable step.
    pub fn move_lines(&mut self, up: bool) {
        let (first, last) = self.line_span();
        let count = self.view.line_count();
        if (up && first == 0) || (!up && last + 1 >= count) {
            return;
        }
        let mut lines: Vec<String> = self.view.with_lines(|l| l.iter().map(|s| (*s).to_owned()).collect());
        if up {
            lines[first - 1..=last].rotate_left(1);
        } else {
            lines[first..=last + 1].rotate_right(1);
        }
        self.rewrite_keeping_cursor(&lines.join("\n"), if up { -1 } else { 1 });
    }

    /// Duplicates the cursor's (or selected) lines. Going down, the cursor follows the copy.
    pub fn duplicate_lines(&mut self, down: bool) {
        let (first, last) = self.line_span();
        let mut lines: Vec<String> = self.view.with_lines(|l| l.iter().map(|s| (*s).to_owned()).collect());
        let copy: Vec<String> = lines[first..=last].to_vec();
        lines.splice(last + 1..last + 1, copy);
        let shift = if down { (last - first + 1) as isize } else { 0 };
        self.rewrite_keeping_cursor(&lines.join("\n"), shift);
    }

    /// Replaces the text, then puts the cursor and selection back, `shift` lines away.
    fn rewrite_keeping_cursor(&mut self, text: &str, shift: isize) {
        let cursor = self.view.cursor();
        let moved = |p: Position| Position { line: p.line.saturating_add_signed(shift), ..p };
        self.set_text(text);
        let target = Cursor { position: moved(cursor.position), selection: cursor.selection.map(moved) };
        self.view.move_to(clamp_cursor(self.view, target));
    }

    /// The selection's line range when it spans more than one line.
    fn selected_lines(&self) -> Option<(usize, usize)> {
        let cursor = self.view.cursor();
        let anchor = cursor.selection?;
        (anchor.line != cursor.position.line).then(|| self.line_span())
    }

    /// First and last line touched by the cursor or selection. A selection ending at the start of a
    /// line doesn't include it.
    fn line_span(&self) -> (usize, usize) {
        let cursor = self.view.cursor();
        let (a, b) = match cursor.selection {
            Some(anchor) if (anchor.line, anchor.column) < (cursor.position.line, cursor.position.column) => {
                (anchor, cursor.position)
            }
            Some(anchor) => (cursor.position, anchor),
            None => (cursor.position, cursor.position),
        };
        let last = if b.line > a.line && b.column == 0 { b.line - 1 } else { b.line };
        (a.line, last)
    }

    /// Rewrites each line in the cursor's span as one undoable step, then selects the result.
    fn edit_lines(&mut self, mut f: impl FnMut(&str) -> String) {
        let (first, last) = self.line_span();
        let text = self.view.text();
        let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let last = last.min(lines.len().saturating_sub(1));
        let mut changed = false;
        for line in &mut lines[first..=last] {
            let new = f(line);
            changed |= new != *line;
            *line = new;
        }
        if !changed {
            return;
        }
        let single = self.view.cursor().selection.is_none() && first == last;
        let before = self.view.cursor();
        let old_len = self.view.line(first).map_or(0, |l| l.text.len());
        self.set_text(&lines.join("\n"));
        if single {
            // Keep the caret on its text as the line's start changes.
            let delta = lines[first].len() as isize - old_len as isize;
            let column = before.position.column.saturating_add_signed(delta);
            self.view.move_to(clamp_cursor(
                self.view,
                Cursor { position: Position { line: first, column }, selection: None },
            ));
        } else {
            self.view.move_to(Cursor {
                position: Position { line: last, column: lines[last].len() },
                selection: Some(Position { line: first, column: 0 }),
            });
        }
    }
}

/// The cursor as 0-based (line, column in characters).
pub fn line_col(view: &Content) -> (usize, usize) {
    let Position { line, column } = view.cursor().position;
    let col = view.line(line).map_or(column, |l| l.text.get(..column).map_or(column, |s| s.chars().count()));
    (line, col)
}

/// Puts the cursor at a 0-based line and character column (clamped).
pub fn go_to(view: &mut Content, line: usize, column: usize) {
    let byte = view.line(line).map_or(0, |l| l.text.char_indices().nth(column).map_or(l.text.len(), |(i, _)| i));
    let cursor = Cursor { position: Position { line, column: byte }, selection: None };
    view.move_to(clamp_cursor(view, cursor));
}

/// Applies `edit` to a view whose text is the edit's old text, keeping its cursor on its text.
pub fn apply_edit(view: &mut Content, edit: &TextEdit) {
    let cursor = view.cursor();
    view.move_to(Cursor {
        position: to_position(edit.old_end),
        selection: (edit.start != edit.old_end).then_some(to_position(edit.start)),
    });
    if edit.inserted.is_empty() {
        view.perform(Action::Edit(Edit::Delete));
    } else {
        view.perform(Action::Edit(Edit::Paste(Arc::new(edit.inserted.clone()))));
    }
    let shift = |p: Position| to_position(edit.shift(to_pos(p)));
    view.move_to(clamp_cursor(
        view,
        Cursor { position: shift(cursor.position), selection: cursor.selection.map(shift) },
    ));
}

/// Replaces `old` with `new` in `view` by editing only the differing middle.
fn replace_text(view: &mut Content, old: &str, new: &str) {
    if let Some(edit) = TextEdit::between(old, new) {
        view.move_to(Cursor {
            position: to_position(edit.old_end),
            selection: (edit.start != edit.old_end).then_some(to_position(edit.start)),
        });
        if edit.inserted.is_empty() {
            view.perform(Action::Edit(Edit::Delete));
        } else {
            view.perform(Action::Edit(Edit::Paste(Arc::new(edit.inserted))));
        }
    }
}

pub fn to_pos(p: Position) -> Pos {
    Pos::new(p.line, p.column)
}

pub fn to_position(p: Pos) -> Position {
    Position { line: p.line, column: p.column }
}

/// Keeps a cursor inside the text, on character boundaries.
pub fn clamp_cursor(view: &Content, cursor: Cursor) -> Cursor {
    let clamp = |p: Position| {
        let last = view.line_count().saturating_sub(1);
        let line = p.line.min(last);
        let text = view.line(line).map(|l| l.text).unwrap_or_default();
        let mut column = p.column.min(text.len());
        while !text.is_char_boundary(column) {
            column -= 1;
        }
        Position { line, column }
    };
    Cursor { position: clamp(cursor.position), selection: cursor.selection.map(clamp) }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn with_buffer(name: &str, text: &str, f: impl FnOnce(&mut Buffer)) -> String {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), text).unwrap();
        let (mut doc, text) = Document::load(dir.path(), Path::new(name)).unwrap();
        let mut view = Content::with_text(&text);
        let mut buffer = Buffer { doc: &mut doc, view: &mut view };
        f(&mut buffer);
        view.text()
    }

    fn caret(buffer: &mut Buffer, line: usize, column: usize) {
        buffer.view.move_to(Cursor { position: Position { line, column }, selection: None });
    }

    #[test]
    fn pairs_close_and_step_over() {
        let out = with_buffer("a.rs", "\n", |b| {
            for c in "f(\"x".chars() {
                b.type_char(c);
            }
            b.type_char('"');
            b.type_char(')');
            b.type_char(';');
        });
        assert_eq!(out, "f(\"x\");\n");
    }

    #[test]
    fn backspace_removes_empty_pairs_and_indent_levels() {
        let out = with_buffer("a.rs", "\n", |b| {
            b.type_char('(');
            b.backspace();
        });
        assert_eq!(out, "\n");
        // Indented by 4: backspace in leading spaces removes one level.
        let out = with_buffer("a.rs", "fn x() {\n    a\n        y\n}\n", |b| {
            caret(b, 2, 8);
            b.backspace();
        });
        assert_eq!(out, "fn x() {\n    a\n    y\n}\n");
    }

    #[test]
    fn enter_splits_pairs_and_indents() {
        let out = with_buffer("a.rs", "fn main() {}\n", |b| {
            caret(b, 0, 11);
            b.newline();
            b.type_char('x');
        });
        assert_eq!(out, "fn main() {\n    x\n}\n");
        let out = with_buffer("a.py", "def f():\n", |b| {
            caret(b, 0, 8);
            b.newline();
            b.type_char('y');
        });
        assert_eq!(out, "def f():\n    y\n");
    }

    #[test]
    fn comments_indents_and_moves_lines() {
        let out = with_buffer("a.py", "a = 1\nb = 2\n", |b| {
            b.view.move_to(Cursor {
                position: Position { line: 1, column: 2 },
                selection: Some(Position { line: 0, column: 0 }),
            });
            b.toggle_comment();
        });
        assert_eq!(out, "# a = 1\n# b = 2\n");
        let out = with_buffer("a.rs", "one\ntwo\n", |b| {
            caret(b, 0, 1);
            b.move_lines(false);
            b.indent();
        });
        assert_eq!(out, "two\no   ne\n");
        let out = with_buffer("a.rs", "one\n", |b| {
            caret(b, 0, 0);
            b.duplicate_lines(true);
            assert_eq!(line_col(b.view), (1, 0));
        });
        assert_eq!(out, "one\none\n");
    }

    #[test]
    fn undo_restores_and_tracks_dirty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
        let (mut doc, text) = Document::load(dir.path(), Path::new("a.txt")).unwrap();
        let mut view = Content::with_text(&text);
        let mut b = Buffer { doc: &mut doc, view: &mut view };
        caret(&mut b, 0, 2);
        b.type_char('!');
        assert!(b.doc.dirty);
        b.undo();
        assert_eq!(b.view.text(), "hi");
        assert!(!b.doc.dirty);
        b.redo();
        assert_eq!(b.view.text(), "hi!");
    }

    #[test]
    fn edits_replay_into_another_view_keeping_its_cursor() {
        let mut a = Content::with_text("one\ntwo\nthree");
        let mut b = Content::with_text("one\ntwo\nthree");
        b.move_to(Cursor { position: Position { line: 2, column: 2 }, selection: None });
        let before = a.text();
        a.move_to(Cursor { position: Position { line: 0, column: 3 }, selection: None });
        a.perform(Action::Edit(Edit::Paste(Arc::new("\ninserted".into()))));
        let edit = TextEdit::between(&before, &a.text()).unwrap();
        apply_edit(&mut b, &edit);
        assert_eq!(b.text(), a.text());
        assert_eq!(b.cursor().position, Position { line: 3, column: 2 });
    }
}
