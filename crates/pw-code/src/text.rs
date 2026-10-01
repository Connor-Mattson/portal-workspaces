//! Positions in text, and the one edit that turns one text into another.
//!
//! When the same file is open in two editor groups, each has its own view (cursor, scroll). An
//! edit in one is replayed in the other as the smallest changed span ([`TextEdit::between`]), and
//! the other view's cursor is carried along ([`TextEdit::shift`]) so it stays on the same text.

/// A line and a byte column in that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

impl Pos {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// Replacing the text between `start` and `old_end` with `inserted`, which ends at `new_end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: Pos,
    pub old_end: Pos,
    pub new_end: Pos,
    pub inserted: String,
}

impl TextEdit {
    /// The smallest edit that turns `old` into `new`: everything between their common prefix and
    /// common suffix. `None` when they are equal.
    pub fn between(old: &str, new: &str) -> Option<TextEdit> {
        if old == new {
            return None;
        }
        let prefix = common_prefix(old, new);
        let suffix = common_suffix(&old[prefix..], &new[prefix..]);
        let inserted = new[prefix..new.len() - suffix].to_owned();
        Some(TextEdit {
            start: position_of(old, prefix),
            old_end: position_of(old, old.len() - suffix),
            new_end: position_of(new, new.len() - suffix),
            inserted,
        })
    }

    /// Where `p` (a position in the old text) is in the new text. Positions before the edit stay,
    /// positions after it move with the text around them, and positions inside the replaced span
    /// go to its start.
    pub fn shift(&self, p: Pos) -> Pos {
        if p <= self.start {
            p
        } else if p < self.old_end {
            self.start
        } else if p.line == self.old_end.line {
            Pos::new(self.new_end.line, self.new_end.column + (p.column - self.old_end.column))
        } else {
            Pos::new(p.line - self.old_end.line + self.new_end.line, p.column)
        }
    }
}

/// Length in bytes of the longest common prefix, on a char boundary.
pub fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    while !a.is_char_boundary(n) || !b.is_char_boundary(n) {
        n -= 1;
    }
    n
}

/// Length in bytes of the longest common suffix, on a char boundary.
pub fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = a.bytes().rev().zip(b.bytes().rev()).take_while(|(x, y)| x == y).count();
    while !a.is_char_boundary(a.len() - n) || !b.is_char_boundary(b.len() - n) {
        n -= 1;
    }
    n
}

/// The line and byte column of a byte offset.
pub fn position_of(text: &str, offset: usize) -> Pos {
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column = before.rfind('\n').map_or(offset, |i| offset - i - 1);
    Pos::new(line, column)
}

/// The byte offset of a position, clamped to the text.
pub fn offset_of(text: &str, p: Pos) -> usize {
    let mut start = 0;
    for _ in 0..p.line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let mut offset = (start + p.column).min(end);
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &str, edit: &TextEdit) -> String {
        let (a, b) = (offset_of(old, edit.start), offset_of(old, edit.old_end));
        format!("{}{}{}", &old[..a], edit.inserted, &old[b..])
    }

    #[test]
    fn between_finds_the_changed_span_and_reapplies() {
        let cases = [
            ("hello world", "hello brave world"),
            ("a\nb\nc", "a\nc"),
            ("fn x() {}\n", "fn x() {\n    y();\n}\n"),
            ("héllo", "hállo"),
            ("", "new"),
            ("gone", ""),
            ("aaa", "aaaa"),
        ];
        for (old, new) in cases {
            let edit = TextEdit::between(old, new).unwrap();
            assert_eq!(apply(old, &edit), new, "{old:?} → {new:?}");
        }
        assert_eq!(TextEdit::between("same", "same"), None);
    }

    #[test]
    fn positions_after_an_edit_follow_their_text() {
        let old = "one\ntwo three\nfour";
        let new = "one\ntwo and a half\nmore three\nfour";
        let edit = TextEdit::between(old, new).unwrap();
        // Before the edit: unchanged.
        assert_eq!(edit.shift(Pos::new(0, 2)), Pos::new(0, 2));
        // The "h" of "three" on line 1 moved to line 2, column 6.
        assert_eq!(edit.shift(Pos::new(1, 5)), Pos::new(2, 6));
        // A cursor right where text was inserted stays in front of it.
        assert_eq!(edit.shift(Pos::new(1, 4)), Pos::new(1, 4));
        // A later line shifts down by one.
        assert_eq!(edit.shift(Pos::new(2, 3)), Pos::new(3, 3));
    }

    #[test]
    fn positions_inside_a_deletion_collapse_to_its_start() {
        let edit = TextEdit::between("abcdef", "af").unwrap();
        assert_eq!(edit.shift(Pos::new(0, 3)), Pos::new(0, 1));
        assert_eq!(edit.shift(Pos::new(0, 6)), Pos::new(0, 2));
    }

    #[test]
    fn offsets_round_trip_and_clamp() {
        let text = "ab\ncdé\n";
        for offset in [0, 1, 2, 3, 5, 7, 8] {
            assert_eq!(offset_of(text, position_of(text, offset)), offset);
        }
        assert_eq!(offset_of(text, Pos::new(1, 99)), 7);
        assert_eq!(offset_of(text, Pos::new(9, 0)), text.len());
        // Mid-character columns snap back.
        assert_eq!(offset_of(text, Pos::new(1, 3)), 5);
    }
}
