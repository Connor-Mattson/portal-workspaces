//! Syntax highlighting, one line at a time.
//!
//! Grammars are Sublime syntaxes run by `syntect`, from `two-face` (bat's set: syntect's defaults
//! plus TypeScript, TOML, Dockerfile, Kotlin, Swift, Zig and more). Rather than a stock colour theme,
//! scopes map to a small set of [`Class`]es, so the app colours code with its own palette.
//!
//! The editor asks for lines in order and says which line changed first. Parser state is kept at
//! the start of every [`CHUNK`]th line, so an edit re-parses from the chunk before it, not from the
//! top of the file.
//!
//! Parsing runs at a few dozen lines per millisecond, so a pass can be given a deadline: lines asked
//! for after it are left plain and the highlighter waits at the first of them, for the next pass to
//! pick up. Files over [`MAX_LINES`] aren't highlighted at all.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;
use std::time::Instant;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// Parser state is snapshotted every this many lines.
const CHUNK: usize = 64;
/// Longer lines (minified bundles) aren't highlighted; regex grammars get slow on them.
const MAX_LINE: usize = 4096;
/// Longer files show as plain text: reaching their end takes seconds of parsing, and the state kept
/// for them grows with their length. See ADR 0008.
pub const MAX_LINES: usize = 50_000;

/// What a piece of code is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Keyword,
    Operator,
    String,
    Escape,
    Number,
    Constant,
    Function,
    Macro,
    Type,
    Attribute,
    Parameter,
    Property,
    Tag,
    Comment,
    Punctuation,
    Heading,
    Emphasis,
    Strong,
    Link,
    Added,
    Removed,
    Invalid,
}

/// Scope prefixes and their class. The innermost scope that matches decides, and within one scope the
/// first rule wins, so specific rules come before general ones.
const RULES: &[(&str, Class)] = &[
    ("punctuation.definition.comment", Class::Comment),
    ("comment", Class::Comment),
    ("constant.character.escape", Class::Escape),
    ("punctuation.definition.string", Class::String),
    ("string.regexp", Class::Escape),
    ("string.other.link", Class::Link),
    ("string", Class::String),
    ("constant.numeric", Class::Number),
    ("constant", Class::Constant),
    ("variable.language", Class::Constant),
    ("variable.other.constant", Class::Constant),
    ("variable.parameter", Class::Parameter),
    ("variable.other.member", Class::Property),
    ("variable.other.property", Class::Property),
    ("variable.other.object.property", Class::Property),
    ("support.variable.property", Class::Property),
    ("support.type.property-name", Class::Property),
    ("meta.object-literal.key", Class::Property),
    ("entity.name.tag.yaml", Class::Property),
    ("entity.name.tag.toml", Class::Property),
    ("meta.mapping.key", Class::Property),
    ("variable.function", Class::Function),
    ("support.macro", Class::Macro),
    ("entity.name.macro", Class::Macro),
    ("support.function.macro", Class::Macro),
    ("entity.name.function", Class::Function),
    ("support.function", Class::Function),
    ("entity.name.tag", Class::Tag),
    ("entity.other.attribute-name", Class::Attribute),
    ("meta.annotation", Class::Attribute),
    ("meta.attribute", Class::Attribute),
    ("meta.decorator", Class::Attribute),
    ("punctuation.definition.annotation", Class::Attribute),
    ("entity.name.type", Class::Type),
    ("entity.name.class", Class::Type),
    ("entity.name.struct", Class::Type),
    ("entity.name.enum", Class::Type),
    ("entity.name.union", Class::Type),
    ("entity.name.trait", Class::Type),
    ("entity.name.interface", Class::Type),
    ("entity.name.impl", Class::Type),
    ("entity.name.namespace", Class::Type),
    ("entity.name.module", Class::Type),
    ("entity.other.inherited-class", Class::Type),
    ("support.type", Class::Type),
    ("support.class", Class::Type),
    ("storage.type.primitive", Class::Type),
    ("entity.name.section", Class::Heading),
    ("markup.heading", Class::Heading),
    ("meta.diff.header", Class::Heading),
    ("meta.diff.range", Class::Heading),
    ("keyword.operator.word", Class::Keyword),
    ("keyword.operator.logical.python", Class::Keyword),
    ("keyword.operator", Class::Operator),
    ("keyword", Class::Keyword),
    ("storage", Class::Keyword),
    ("markup.bold", Class::Strong),
    ("markup.italic", Class::Emphasis),
    ("markup.underline.link", Class::Link),
    ("markup.raw", Class::String),
    ("markup.quote", Class::Comment),
    ("markup.inserted", Class::Added),
    ("markup.deleted", Class::Removed),
    ("invalid", Class::Invalid),
    ("punctuation", Class::Punctuation),
];

struct Tables {
    set: SyntaxSet,
    rules: Vec<(Scope, Class)>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        set: two_face::syntax::extra_newlines(),
        rules: RULES.iter().map(|(s, c)| (Scope::new(s).expect("valid scope"), *c)).collect(),
    })
}

/// Loads the grammars (about 50 ms). Call it from a background thread before the first file opens, so
/// the first highlight doesn't wait.
pub fn warm() {
    let _ = tables();
}

/// Whether a syntax token (a language's `syntax` extension) names a known grammar.
pub fn has_syntax(token: &str) -> bool {
    find(token).is_some()
}

/// The syntax token to highlight a file of `lines` lines with: none (plain text) past [`MAX_LINES`].
pub fn token_for(token: &'static str, lines: usize) -> &'static str {
    if lines > MAX_LINES { "" } else { token }
}

fn find(token: &str) -> Option<&'static SyntaxReference> {
    if token.is_empty() {
        return None;
    }
    let set = &tables().set;
    set.find_syntax_by_extension(token).or_else(|| set.find_syntax_by_token(token))
}

/// The class of the innermost scope that has one.
#[cfg(test)]
fn classify(stack: &ScopeStack, rules: &[(Scope, Class)]) -> Option<Class> {
    stack.as_slice().iter().rev().find_map(|scope| class_of(*scope, rules))
}

/// The first rule matching one scope.
fn class_of(scope: Scope, rules: &[(Scope, Class)]) -> Option<Class> {
    rules.iter().find(|(prefix, _)| prefix.is_prefix_of(scope)).map(|(_, class)| *class)
}

/// `classify` with each scope's class cached. A scope resolves on its own (the innermost scope with
/// a class wins), so the results are the same, without walking the rules for every token.
#[derive(Default)]
struct Classes(HashMap<Scope, Option<Class>>);

impl Classes {
    fn classify(&mut self, stack: &ScopeStack, rules: &[(Scope, Class)]) -> Option<Class> {
        stack.as_slice().iter().rev().find_map(|scope| *self.0.entry(*scope).or_insert_with(|| class_of(*scope, rules)))
    }
}

type State = (ParseState, ScopeStack);

/// Highlights one file's lines in order. Plain text (no grammar) yields no spans.
pub struct Highlighter {
    syntax: Option<&'static SyntaxReference>,
    /// `snapshots[k]` is the state at the start of line `k * CHUNK`.
    snapshots: Vec<State>,
    /// The state at the start of `current`.
    working: Option<State>,
    current: usize,
    classes: Classes,
    /// Lines asked for after this are skipped (see [`set_deadline`](Self::set_deadline)).
    deadline: Option<Instant>,
    /// The deadline has passed in this pass, so the clock needn't be read again.
    out_of_time: bool,
}

impl Highlighter {
    /// A highlighter for a language's syntax token (see [`crate::Language`]'s `syntax`).
    pub fn new(token: &str) -> Self {
        let syntax = find(token);
        let initial = syntax.map(|s| (ParseState::new(s), ScopeStack::new()));
        Self {
            syntax,
            snapshots: initial.iter().cloned().collect(),
            working: initial,
            current: 0,
            classes: Classes::default(),
            deadline: None,
            out_of_time: false,
        }
    }

    pub fn is_plain(&self) -> bool {
        self.syntax.is_none()
    }

    /// The next line [`highlight_line`](Self::highlight_line) will be given.
    pub fn current_line(&self) -> usize {
        self.current
    }

    /// Starts a pass that stops parsing at `deadline`: from then on lines get no spans and the
    /// highlighter stays where it stopped, so the next pass resumes there.
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = Some(deadline);
        self.out_of_time = false;
    }

    /// Line `line` changed: resume from the snapshot at or before it.
    pub fn change_line(&mut self, line: usize) {
        if self.snapshots.is_empty() {
            self.current = line.min(self.current);
            return;
        }
        let k = (line / CHUNK).min(self.snapshots.len() - 1);
        self.snapshots.truncate(k + 1);
        self.working = Some(self.snapshots[k].clone());
        self.current = k * CHUNK;
    }

    /// The classified spans of the next line (byte ranges, in order, not overlapping). Past the
    /// deadline, none, and the line is left for the next pass.
    pub fn highlight_line(&mut self, line: &str) -> Vec<(Range<usize>, Class)> {
        let Some((parser, stack)) = self.working.as_mut() else {
            self.current += 1;
            return Vec::new();
        };
        self.out_of_time = self.out_of_time || self.deadline.is_some_and(|d| Instant::now() >= d);
        if self.out_of_time {
            return Vec::new();
        }
        let index = self.current;
        self.current += 1;
        if index.is_multiple_of(CHUNK) && index / CHUNK == self.snapshots.len() {
            self.snapshots.push((parser.clone(), stack.clone()));
        }
        let tables = tables();
        // Newline grammars expect the line ending; long lines are parsed only up to their start, so the
        // state still carries on to the next line.
        let mut owned = String::with_capacity(line.len().min(MAX_LINE) + 1);
        owned.push_str(if line.len() > MAX_LINE { "" } else { line });
        owned.push('\n');
        let Ok(ops) = parser.parse_line(&owned, &tables.set) else {
            return Vec::new();
        };
        let mut spans: Vec<(Range<usize>, Class)> = Vec::new();
        let mut last = 0;
        let push = |range: Range<usize>, class: Option<Class>, spans: &mut Vec<(Range<usize>, Class)>| {
            let range = range.start.min(line.len())..range.end.min(line.len());
            let Some(class) = class.filter(|_| !range.is_empty()) else { return };
            match spans.last_mut() {
                Some((prev, c)) if *c == class && prev.end == range.start => prev.end = range.end,
                _ => spans.push((range, class)),
            }
        };
        for (at, op) in ops {
            push(last..at, self.classes.classify(stack, &tables.rules), &mut spans);
            let _ = stack.apply(&op);
            last = at;
        }
        push(last..owned.len(), self.classes.classify(stack, &tables.rules), &mut spans);
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::TABLE;

    fn classes(h: &mut Highlighter, line: &str) -> Vec<(String, Class)> {
        h.highlight_line(line).into_iter().map(|(r, c)| (line[r].to_owned(), c)).collect()
    }

    fn class_of(spans: &[(String, Class)], text: &str) -> Option<Class> {
        spans.iter().find(|(t, _)| t.contains(text)).map(|(_, c)| *c)
    }

    #[test]
    fn every_language_has_a_grammar() {
        for spec in TABLE {
            assert!(spec.syntax.is_empty() || has_syntax(spec.syntax), "no grammar for {}", spec.name);
        }
    }

    #[test]
    fn classifies_rust() {
        let mut h = Highlighter::new("rs");
        let spans = classes(&mut h, r#"fn main() { let x = "hi\n"; println!("{x}"); // done"#);
        assert_eq!(class_of(&spans, "fn"), Some(Class::Keyword));
        assert_eq!(class_of(&spans, "main"), Some(Class::Function));
        assert_eq!(class_of(&spans, "hi"), Some(Class::String));
        assert_eq!(class_of(&spans, "\\n"), Some(Class::Escape));
        assert_eq!(class_of(&spans, "println"), Some(Class::Macro));
        assert_eq!(class_of(&spans, "// done"), Some(Class::Comment));
    }

    #[test]
    fn carries_state_across_lines_and_resumes_after_a_change() {
        let mut h = Highlighter::new("py");
        let lines = ["x = '''start", "still string", "end'''", "y = 1"];
        let out: Vec<_> = lines.iter().map(|l| classes(&mut h, l)).collect();
        assert_eq!(out[1], vec![("still string".to_owned(), Class::String)]);
        assert_eq!(class_of(&out[3], "1"), Some(Class::Number));
        // Line 1 changes: parsing resumes from the snapshot before it with the right state.
        h.change_line(1);
        assert_eq!(h.current_line(), 0);
        let _ = classes(&mut h, lines[0]);
        assert_eq!(classes(&mut h, "again"), vec![("again".to_owned(), Class::String)]);
    }

    #[test]
    fn snapshots_bound_reparsing() {
        let mut h = Highlighter::new("rs");
        for _ in 0..CHUNK * 3 + 5 {
            h.highlight_line("let a = 1;");
        }
        h.change_line(CHUNK * 2 + 10);
        assert_eq!(h.current_line(), CHUNK * 2);
    }

    #[test]
    fn plain_text_has_no_spans() {
        let mut h = Highlighter::new("");
        assert!(h.is_plain());
        assert!(h.highlight_line("fn main() {}").is_empty());
        assert_eq!(h.current_line(), 1);
    }

    #[test]
    fn spans_stay_inside_the_line() {
        let mut h = Highlighter::new("md");
        for line in ["# Title", "*em* and **strong** `code`", ""] {
            for (range, _) in h.highlight_line(line) {
                assert!(range.end <= line.len() && range.start < range.end);
            }
        }
    }

    #[test]
    fn cached_classes_match_uncached_ones_across_grammars() {
        let samples = [
            ("rs", include_str!("syntax.rs")),
            (
                "py",
                "import os\n@dataclass\nclass A(B):\n    def f(self, x: int = 1) -> str:\n        return f'{x!r}' # hi\n",
            ),
            ("md", "# Title\n\n*em* **strong** [link](http://x) `code`\n> quote\n- item\n"),
            (
                "ts",
                "export const f = async <T,>(a: T): Promise<T> => { /* c */ return `${a}`; };\nclass X extends Y {}\n",
            ),
            ("html", "<!doctype html>\n<div class=\"a\" id='b'><script>let x = 1;</script><!-- c --></div>\n"),
            ("json", "{\"a\": [1, 2.5e3, true, null, \"s\\n\"]}\n"),
            ("toml", "[package]\nname = \"x\" # c\nversion = 1\n"),
            ("sh", "#!/bin/sh\nfor f in *.rs; do echo \"$f\" | grep -c fn; done\n"),
            ("diff", "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n"),
        ];
        let tables = tables();
        let mut cached = Classes::default();
        let mut compared = 0;
        for (token, text) in samples {
            let syntax = find(token).unwrap_or_else(|| panic!("no grammar for {token}"));
            let mut parser = ParseState::new(syntax);
            let mut stack = ScopeStack::new();
            for line in text.lines() {
                for (_, op) in parser.parse_line(&format!("{line}\n"), &tables.set).unwrap() {
                    let _ = stack.apply(&op);
                    // Twice: once filling the cache, once reading it.
                    for _ in 0..2 {
                        assert_eq!(cached.classify(&stack, &tables.rules), classify(&stack, &tables.rules));
                    }
                    compared += 1;
                }
            }
        }
        assert!(compared > 500, "only {compared} stacks compared");
    }

    #[test]
    fn a_deadline_pauses_and_the_next_pass_resumes_where_it_stopped() {
        let lines = ["x = '''start", "still string", "end'''", "y = 1"];
        let mut h = Highlighter::new("py");
        let _ = classes(&mut h, lines[0]);
        // Out of time: nothing is parsed, and the highlighter waits at line 1.
        h.set_deadline(Instant::now());
        assert!(h.highlight_line(lines[1]).is_empty());
        assert!(h.highlight_line(lines[2]).is_empty());
        assert_eq!(h.current_line(), 1);
        // The next pass starts at line 1, still inside the string.
        h.set_deadline(Instant::now() + std::time::Duration::from_secs(60));
        assert_eq!(classes(&mut h, lines[1]), vec![("still string".to_owned(), Class::String)]);
        let _ = classes(&mut h, lines[2]);
        assert_eq!(class_of(&classes(&mut h, lines[3]), "1"), Some(Class::Number));
        assert_eq!(h.current_line(), 4);
    }

    #[test]
    fn a_kept_highlighter_resumes_instead_of_starting_over() {
        // What a tab keeps while another is shown: coming back continues where it was.
        let mut h = Highlighter::new("rs");
        for _ in 0..CHUNK * 2 {
            h.highlight_line("let a = 1;");
        }
        assert_eq!(h.current_line(), CHUNK * 2);
        // An edit further down goes back only to its chunk.
        h.change_line(CHUNK + 3);
        assert_eq!(h.current_line(), CHUNK);
    }

    #[test]
    fn long_files_are_plain_text() {
        assert_eq!(token_for("rs", MAX_LINES), "rs");
        assert_eq!(token_for("rs", MAX_LINES + 1), "");
        assert!(Highlighter::new(token_for("rs", MAX_LINES + 1)).is_plain());
    }
}
