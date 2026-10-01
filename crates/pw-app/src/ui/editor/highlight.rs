//! Syntax colours for the code editor, from `pw_code::syntax`.
//!
//! Iced keeps one highlighter per editor widget and says which line changed first. A group's
//! widget shows different tabs over time, so the parse state lives with each tab ([`Parse`]) and the
//! widget's highlighter switches to the shown tab's. Coming back to a tab resumes where its
//! highlighting stopped; its view still holds the colours of the lines already done.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use iced::advanced::text::highlighter::{Format, Highlighter};
use iced::{Font, Theme};
use pw_code::syntax::{self, Class};

use super::code_editor::Budgeted;
use crate::fonts;
use crate::theme;

/// One tab's parse state, shared with the widget's highlighter while the tab is shown. It belongs to
/// the tab's view: the view's lines carry the colours this state has produced so far.
#[derive(Clone, Default)]
pub struct Parse(Rc<RefCell<Option<(&'static str, syntax::Highlighter)>>>);

impl Parse {
    /// Runs `f` on the highlighter for `token`, starting over if the tab's syntax changed.
    fn with<T>(&self, token: &'static str, f: impl FnOnce(&mut syntax::Highlighter) -> T) -> T {
        let mut slot = self.0.borrow_mut();
        if slot.as_ref().is_none_or(|(t, _)| *t != token) {
            *slot = Some((token, syntax::Highlighter::new(token)));
        }
        f(&mut slot.as_mut().expect("just set").1)
    }
}

#[derive(Clone)]
pub struct Settings {
    /// The language's syntax token (see `pw_code::Language`), or none for a file too long to highlight.
    pub syntax: &'static str,
    /// The tab shown (`Tab::id`).
    pub tab: u64,
    /// The tab's parse state (`Tab::parse`).
    pub parse: Parse,
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        // A tab's `parse` is always the same one.
        self.syntax == other.syntax && self.tab == other.tab
    }
}

pub struct CodeHighlighter {
    syntax: &'static str,
    parse: Parse,
}

impl CodeHighlighter {
    fn with<T>(&self, f: impl FnOnce(&mut syntax::Highlighter) -> T) -> T {
        self.parse.with(self.syntax, f)
    }
}

impl Highlighter for CodeHighlighter {
    type Settings = Settings;
    type Highlight = Class;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Class)>;

    fn new(settings: &Settings) -> Self {
        Self { syntax: settings.syntax, parse: settings.parse.clone() }
    }

    fn update(&mut self, settings: &Settings) {
        *self = Self::new(settings);
    }

    fn change_line(&mut self, line: usize) {
        self.with(|h| h.change_line(line));
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.with(|h| h.highlight_line(line)).into_iter()
    }

    fn current_line(&self) -> usize {
        // Plain text: nothing to do, ever.
        self.with(|h| if h.is_plain() { usize::MAX } else { h.current_line() })
    }
}

impl Budgeted for CodeHighlighter {
    fn set_deadline(&mut self, deadline: Instant) {
        self.with(|h| h.set_deadline(deadline));
    }
}

pub fn format(class: &Class, _theme: &Theme) -> Format<Font> {
    let (color, bold, italic) = theme::syntax(*class);
    let font = (bold || italic).then(|| fonts::mono(bold, italic));
    Format { color: Some(color), font }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(tab: u64, parse: &Parse) -> Settings {
        Settings { syntax: "rs", tab, parse: parse.clone() }
    }

    #[test]
    fn switching_back_to_a_tab_resumes_its_highlighting() {
        let (a, b) = (Parse::default(), Parse::default());
        let mut h = CodeHighlighter::new(&settings(1, &a));
        for _ in 0..100 {
            let _ = h.highlight_line("let a = 1;");
        }
        h.update(&settings(2, &b));
        assert_eq!(h.current_line(), 0, "another tab starts at its own place");
        let _ = h.highlight_line("fn b() {}");
        h.update(&settings(1, &a));
        assert_eq!(h.current_line(), 100, "not from line 0");
        // A widget made afresh for the tab (say, after a split) picks up the same state.
        assert_eq!(CodeHighlighter::new(&settings(1, &a)).current_line(), 100);
    }

    #[test]
    fn a_changed_syntax_starts_over() {
        let a = Parse::default();
        let mut h = CodeHighlighter::new(&settings(1, &a));
        let _ = h.highlight_line("let a = 1;");
        // The file grew past the limit, or was renamed to another language.
        h.update(&Settings { syntax: "py", ..settings(1, &a) });
        assert_eq!(h.current_line(), 0);
        h.update(&Settings { syntax: "", ..settings(1, &a) });
        assert_eq!(h.current_line(), usize::MAX);
    }
}
