//! Finding text in a file: every match, stepping between them, replacing them.

use std::ops::Range;

use crate::text::Pos;

/// Matches are capped so a one-letter query in a long file stays cheap to show.
pub const MAX_MATCHES: usize = 10_000;

/// A match: its line and byte range in that line.
pub type Match = (usize, Range<usize>);

/// Every occurrence of `query` within single lines, without overlaps, in document order.
pub fn find_all(lines: &[&str], query: &str, case_sensitive: bool) -> Vec<Match> {
    let mut out = Vec::new();
    if query.is_empty() || query.contains('\n') {
        return out;
    }
    for (i, line) in lines.iter().enumerate() {
        let mut from = 0;
        while from < line.len() {
            let Some((start, end)) = find_in(&line[from..], query, case_sensitive) else { break };
            out.push((i, from + start..from + end));
            if out.len() >= MAX_MATCHES {
                return out;
            }
            from += end.max(start + 1);
            while !line.is_char_boundary(from) {
                from += 1;
            }
        }
    }
    out
}

/// The first match in `hay` as a byte range.
fn find_in(hay: &str, query: &str, case_sensitive: bool) -> Option<(usize, usize)> {
    if case_sensitive {
        return hay.find(query).map(|i| (i, i + query.len()));
    }
    hay.char_indices().find_map(|(i, _)| {
        let mut rest = hay[i..].chars();
        let mut len = 0;
        for q in query.chars() {
            let c = rest.next()?;
            if !c.to_lowercase().eq(q.to_lowercase()) {
                return None;
            }
            len += c.len_utf8();
        }
        Some((i, i + len))
    })
}

/// The index of the match after `selection` (or before it, going back), wrapping around.
/// `selection` is `(start, end)` of the cursor's selection, or the caret twice.
pub fn step(matches: &[Match], selection: (Pos, Pos), forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let (start, end) = selection;
    Some(if forward {
        matches.iter().position(|(l, r)| (*l, r.start) >= (end.line, end.column)).unwrap_or(0)
    } else {
        matches.iter().rposition(|(l, r)| (*l, r.start) < (start.line, start.column)).unwrap_or(matches.len() - 1)
    })
}

/// The index of the match that `selection` covers exactly, if any.
pub fn selected(matches: &[Match], selection: (Pos, Pos)) -> Option<usize> {
    let (start, end) = selection;
    (start.line == end.line && start != end)
        .then(|| matches.iter().position(|(l, r)| *l == start.line && *r == (start.column..end.column)))
        .flatten()
}

/// The text with every match replaced.
pub fn replace_all(lines: &[&str], matches: &[Match], replacement: &str) -> String {
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    for (line, range) in matches.iter().rev() {
        out[*line].replace_range(range.clone(), replacement);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_insensitively_by_default() {
        let lines = ["The theorem", "", "THEOREM é théorème"];
        assert_eq!(find_all(&lines, "theorem", false), vec![(0, 4..11), (2, 0..7)]);
        assert_eq!(find_all(&lines, "theorem", true), vec![(0, 4..11)]);
        assert_eq!(find_all(&lines, "É", false), vec![(2, 8..10), (2, 13..15)]);
        assert!(find_all(&lines, "", false).is_empty());
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(find_all(&["aaaa"], "aa", true), vec![(0, 0..2), (0, 2..4)]);
    }

    #[test]
    fn steps_wrap_and_selection_is_recognised() {
        let lines = ["x y x", "x"];
        let m = find_all(&lines, "x", true);
        let caret = |l, c| (Pos::new(l, c), Pos::new(l, c));
        assert_eq!(step(&m, caret(0, 0), true), Some(0));
        assert_eq!(step(&m, (Pos::new(0, 0), Pos::new(0, 1)), true), Some(1));
        assert_eq!(step(&m, caret(1, 1), true), Some(0));
        assert_eq!(step(&m, caret(0, 0), false), Some(2));
        assert_eq!(selected(&m, (Pos::new(0, 4), Pos::new(0, 5))), Some(1));
        assert_eq!(selected(&m, caret(0, 4)), None);
        assert_eq!(replace_all(&lines, &m, "zz"), "zz y zz\nzz");
        assert_eq!(step(&[], caret(0, 0), true), None);
    }
}
