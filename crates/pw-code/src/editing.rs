//! Source-editing rules that know a language: bracket matching, auto-closed pairs, smart Enter,
//! comment toggling and indentation detection.
//!
//! Positions are `(line, byte column)` into the editor's lines. Strings and line comments are
//! recognised one line at a time ([`code_spans`]), which is right for the brackets and quotes
//! people type and cheap enough to run on every frame.

use crate::language::{Indent, Language};

/// How far bracket matching searches, in lines.
const MAX_LINES: usize = 1000;

/// Whether typing `open` between `before` and `after` should also insert its closer.
///
/// Brackets close in front of whitespace, the end of the line or closing punctuation, so typing `(` in
/// front of a word doesn't leave a stray `)`. Quotes also need a non-word character before them, so
/// `don't` stays one quote.
pub fn should_autoclose(lang: Language, open: char, before: Option<char>, after: Option<char>) -> bool {
    if lang.closer(open).is_none() {
        return false;
    }
    let free_after =
        after.is_none_or(|c| c.is_whitespace() || matches!(c, '}' | ']' | ')' | ',' | '.' | ';' | ':' | '>'));
    if is_quote(lang, open) {
        let free_before = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == open || c == '\\'));
        free_after && free_before
    } else {
        free_after && before != Some('\\')
    }
}

/// Whether typing `c` in front of `after` should step over it instead of inserting another.
pub fn steps_over(lang: Language, c: char, after: Option<char>) -> bool {
    after == Some(c) && (matches!(c, '}' | ']' | ')') || is_quote(lang, c))
}

/// Whether backspace between `before` and `after` should delete an empty pair.
pub fn deletes_pair(lang: Language, before: Option<char>, after: Option<char>) -> bool {
    before.and_then(|b| lang.closer(b)).is_some_and(|close| after == Some(close))
}

fn is_quote(lang: Language, c: char) -> bool {
    lang.quotes.contains(c)
}

/// Byte ranges of `line` that are code: outside strings and before a line comment. A string left open
/// at the end of the line runs to the end of it.
pub fn code_spans(lang: Language, line: &str) -> Vec<std::ops::Range<usize>> {
    scan(lang, line).0
}

/// Where a line's comment starts, if it has one outside strings.
pub fn comment_start(lang: Language, line: &str) -> Option<usize> {
    scan(lang, line).1
}

/// One pass over a line: its code spans, and where its line comment starts.
fn scan(lang: Language, line: &str) -> (Vec<std::ops::Range<usize>>, Option<usize>) {
    let mut spans = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut chars = line.char_indices();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(q) if c == q => {
                quote = None;
                start = i + c.len_utf8();
            }
            Some(_) => {}
            None if lang.line_comment.is_some_and(|lc| line[i..].starts_with(lc)) => {
                spans.push(start..i);
                spans.retain(|s| !s.is_empty());
                return (spans, Some(i));
            }
            None => {
                // A quote right after a word character is an apostrophe or a lifetime, not a string.
                let after_word = line[..i].chars().next_back().is_some_and(|p| p.is_alphanumeric() || p == '_');
                if is_quote(lang, c) && !(c == '\'' && after_word) {
                    spans.push(start..i);
                    quote = Some(c);
                }
            }
        }
    }
    if quote.is_none() {
        spans.push(start..line.len());
    }
    spans.retain(|s| !s.is_empty());
    (spans, None)
}

/// The bracket next to the cursor and its partner, both as `(line, byte column)`.
///
/// Looks at the character before the cursor first, then the one after it. Brackets in strings and
/// comments don't count. The search stops after [`MAX_LINES`] lines.
pub fn matching_bracket(lang: Language, lines: &[&str], line: usize, column: usize) -> Option<[(usize, usize); 2]> {
    let text = *lines.get(line)?;
    let spans = code_spans(lang, text);
    let is_code = |spans: &[std::ops::Range<usize>], i: usize| spans.iter().any(|s| s.contains(&i));
    let bytes = text.as_bytes();
    let at = [column.checked_sub(1), Some(column)]
        .into_iter()
        .flatten()
        .find(|&i| i < bytes.len() && bracket(bytes[i]).is_some() && is_code(&spans, i))?;
    let (partner, forward) = bracket(bytes[at])?;
    let own = bytes[at];
    let mut depth = 0usize;
    let mut visit = |b: u8| -> bool {
        if b == own {
            depth += 1;
        } else if b == partner {
            if depth == 0 {
                return true;
            }
            depth -= 1;
        }
        false
    };
    if forward {
        for (l, text) in lines.iter().enumerate().skip(line).take(MAX_LINES) {
            let spans = if l == line { spans.clone() } else { code_spans(lang, text) };
            for span in spans {
                let from = if l == line { span.start.max(at + 1) } else { span.start };
                for i in from..span.end {
                    if visit(text.as_bytes()[i]) {
                        return Some([(line, at), (l, i)]);
                    }
                }
            }
        }
    } else {
        for l in (line.saturating_sub(MAX_LINES)..=line).rev() {
            let text = lines[l];
            let spans = if l == line { spans.clone() } else { code_spans(lang, text) };
            for span in spans.into_iter().rev() {
                let to = if l == line { span.end.min(at) } else { span.end };
                for i in (span.start..to).rev() {
                    if visit(text.as_bytes()[i]) {
                        return Some([(line, at), (l, i)]);
                    }
                }
            }
        }
    }
    None
}

/// The partner of a bracket byte, and whether it lies forward.
fn bracket(b: u8) -> Option<(u8, bool)> {
    match b {
        b'{' => Some((b'}', true)),
        b'[' => Some((b']', true)),
        b'(' => Some((b')', true)),
        b'}' => Some((b'{', false)),
        b']' => Some((b'[', false)),
        b')' => Some((b'(', false)),
        _ => None,
    }
}

/// What Enter does at a cursor that splits a line into `before` and `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enter {
    /// Keep the line's indentation.
    Keep,
    /// Indent one level more: after `{`, `(`, `[` or a Python `:`.
    Indent,
    /// Between a bracket pair: indent the new line and move the closer to its own line below.
    Split,
}

pub fn enter(lang: Language, before: &str, after: &str) -> Enter {
    let end = before.trim_end();
    let end = comment_start(lang, end).map_or(end, |i| end[..i].trim_end());
    let opened = end.chars().next_back();
    let closes = after.trim_start().chars().next();
    match opened {
        Some(o @ ('{' | '(' | '[')) if closes == lang.closer(o) => Enter::Split,
        Some('{' | '(' | '[') => Enter::Indent,
        Some(':') if lang.name == "Python" => Enter::Indent,
        _ => Enter::Keep,
    }
}

/// Lines with comments toggled: commented out (keeping indentation) unless every non-blank line already
/// is, in which case they're uncommented. Languages without line comments wrap each line in a block
/// comment. `None` if the language has no comments.
pub fn toggle_comment(lang: Language, lines: &[&str]) -> Option<Vec<String>> {
    let (open, close) = match (lang.line_comment, lang.block_comment) {
        (Some(lc), _) => (lc, ""),
        (None, Some((open, close))) => (open, close),
        (None, None) => return None,
    };
    let is_commented = |l: &str| {
        let t = l.trim();
        t.starts_with(open) && t.ends_with(close)
    };
    let content: Vec<&&str> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let uncomment = !content.is_empty() && content.iter().all(|l| is_commented(l));
    // Comment markers go at the shallowest indentation of the block, so they line up.
    let column = content.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    Some(
        lines
            .iter()
            .map(|line| {
                if line.trim().is_empty() {
                    return (*line).to_owned();
                }
                if uncomment {
                    let indent_len = line.len() - line.trim_start().len();
                    let (indent, rest) = line.split_at(indent_len);
                    let rest = rest.trim_end();
                    let rest = rest.strip_prefix(open).unwrap_or(rest);
                    let rest = rest.strip_suffix(close).unwrap_or(rest);
                    let rest = rest.strip_prefix(' ').unwrap_or(rest);
                    let rest = if close.is_empty() { rest } else { rest.strip_suffix(' ').unwrap_or(rest) };
                    format!("{indent}{rest}")
                } else {
                    let (indent, rest) = line.split_at(column.min(line.len()));
                    let tail = if close.is_empty() { String::new() } else { format!(" {close}") };
                    format!("{indent}{open} {rest}{tail}")
                }
            })
            .collect(),
    )
}

/// How a text is indented: by tabs if most indented lines start with one, else by the most common step
/// between consecutive indentation levels. `fallback` when nothing is indented.
pub fn detect_indent(text: &str, fallback: Indent) -> Indent {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    let mut steps = [0usize; 9];
    let mut previous = 0usize;
    for line in text.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let width = line.len() - line.trim_start_matches(' ').len();
        if width > 0 {
            spaces += 1;
        }
        let step = width.abs_diff(previous);
        if (2..=8).contains(&step) {
            steps[step] += 1;
        }
        previous = width;
    }
    if tabs > spaces {
        return Indent::Tabs;
    }
    match steps.iter().enumerate().max_by_key(|&(n, count)| (*count, std::cmp::Reverse(n))) {
        Some((n, &count)) if count > 0 => Indent::Spaces(n as u8),
        _ if spaces == 0 && tabs > 0 => Indent::Tabs,
        _ => fallback,
    }
}

/// Removes one level of indentation from the start of `line`.
pub fn outdent(line: &str, indent: Indent) -> &str {
    let unit = indent.unit();
    line.strip_prefix(unit.as_str()).or_else(|| line.strip_prefix('\t')).unwrap_or_else(|| {
        let spaces = line.len() - line.trim_start_matches(' ').len();
        &line[spaces.min(unit.len().max(1))..]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn lang(file: &str) -> Language {
        Language::detect(Path::new(file), None)
    }

    #[test]
    fn code_spans_skip_strings_and_comments() {
        let rs = lang("a.rs");
        let line = r#"let s = "a (b"; // ) c"#;
        let spans = code_spans(rs, line);
        assert_eq!(spans, vec![0..8, 14..16]);
        assert_eq!(comment_start(rs, line), Some(16));
        // Escaped quotes stay inside the string.
        assert_eq!(code_spans(rs, r#"f("a\"b", x)"#), vec![0..2, 8..12]);
        // Lifetimes aren't strings.
        assert_eq!(code_spans(rs, "fn f<'a>(x: &'a str)"), vec![0..20]);
        assert_eq!(comment_start(lang("a.py"), "x = '#' # real"), Some(8));
        assert_eq!(comment_start(rs, r#""http://x""#), None);
    }

    #[test]
    fn matches_brackets_across_lines_skipping_strings_and_comments() {
        let rs = lang("a.rs");
        let lines = ["fn f(a: &str) {", r#"    g(")"); // }"#, "}"];
        assert_eq!(matching_bracket(rs, &lines, 0, 15), Some([(0, 14), (2, 0)]));
        assert_eq!(matching_bracket(rs, &lines, 2, 0), Some([(2, 0), (0, 14)]));
        // `g(")")`: the paren after the cursor at 5 pairs with the one after the string.
        assert_eq!(matching_bracket(rs, &lines, 1, 5), Some([(1, 5), (1, 9)]));
        assert_eq!(matching_bracket(rs, &lines, 0, 1), None);
        assert_eq!(matching_bracket(rs, &["( ["], 0, 1), None);
    }

    #[test]
    fn autoclose_rules() {
        let rs = lang("a.rs");
        let py = lang("a.py");
        assert!(should_autoclose(rs, '(', Some('f'), None));
        assert!(should_autoclose(rs, '{', Some(' '), Some(')')));
        assert!(!should_autoclose(rs, '(', Some(' '), Some('x')));
        assert!(should_autoclose(rs, '"', Some('('), Some(')')));
        assert!(!should_autoclose(rs, '"', Some('a'), None));
        assert!(!should_autoclose(rs, '\'', None, None));
        assert!(!should_autoclose(py, '\'', Some('n'), Some(' ')));
        assert!(should_autoclose(py, '\'', Some('='), None));
        assert!(steps_over(rs, ')', Some(')')));
        assert!(steps_over(rs, '"', Some('"')));
        assert!(!steps_over(rs, 'x', Some('x')));
        assert!(deletes_pair(rs, Some('('), Some(')')));
        assert!(!deletes_pair(rs, Some('('), Some(']')));
    }

    #[test]
    fn enter_indents_after_openers_and_splits_pairs() {
        let rs = lang("a.rs");
        assert_eq!(enter(rs, "fn main() {", "}"), Enter::Split);
        assert_eq!(enter(rs, "fn main() { // start", ""), Enter::Indent);
        assert_eq!(enter(rs, "let x = 1;", ""), Enter::Keep);
        assert_eq!(enter(lang("a.py"), "def f():", ""), Enter::Indent);
        assert_eq!(enter(rs, "x:", ""), Enter::Keep);
    }

    #[test]
    fn toggles_line_and_block_comments() {
        let rs = lang("a.rs");
        let out = toggle_comment(rs, &["    a();", "", "      b();"]).unwrap();
        assert_eq!(out, vec!["    // a();", "", "    //   b();"]);
        let back: Vec<&str> = out.iter().map(String::as_str).collect();
        assert_eq!(toggle_comment(rs, &back).unwrap(), vec!["    a();", "", "      b();"]);
        let html = lang("a.html");
        let out = toggle_comment(html, &["<p>x</p>"]).unwrap();
        assert_eq!(out, vec!["<!-- <p>x</p> -->"]);
        assert_eq!(toggle_comment(html, &[out[0].as_str()]).unwrap(), vec!["<p>x</p>"]);
        assert_eq!(toggle_comment(lang("a.diff"), &["x"]), None);
    }

    #[test]
    fn detects_indentation() {
        assert_eq!(detect_indent("a\n  b\n    c\n  d\n", Indent::Spaces(4)), Indent::Spaces(2));
        assert_eq!(detect_indent("a {\n    b\n        c\n}\n", Indent::Spaces(2)), Indent::Spaces(4));
        assert_eq!(detect_indent("a\n\tb\n\t\tc\n", Indent::Spaces(4)), Indent::Tabs);
        assert_eq!(detect_indent("flat\ntext\n", Indent::Spaces(3)), Indent::Spaces(3));
    }

    #[test]
    fn outdents_one_level() {
        assert_eq!(outdent("        x", Indent::Spaces(4)), "    x");
        assert_eq!(outdent("  x", Indent::Spaces(4)), "x");
        assert_eq!(outdent("\tx", Indent::Spaces(4)), "x");
        assert_eq!(outdent("x", Indent::Tabs), "x");
    }
}
