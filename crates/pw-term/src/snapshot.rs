//! A render-ready copy of the visible screen.
//!
//! Capturing holds the terminal lock only for one pass over the visible cells. Cells are merged
//! into runs so the renderer draws a handful of text and rectangle primitives per row instead of
//! one per cell.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, point_to_viewport};
use alacritty_terminal::vte::ansi::CursorShape as AlacrittyShape;

use crate::color::{Palette, Rgb};
use crate::events::Listener;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// Already drawn into the cell's colors; the renderer draws nothing extra.
    Block,
    /// Outline, used when the pane isn't focused.
    HollowBlock,
    Beam,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnap {
    pub row: usize,
    pub col: usize,
    /// 2 over a wide character.
    pub cells: usize,
    pub shape: CursorShape,
    pub color: Rgb,
}

/// A span of same-styled text. ASCII merges into long runs; any other character gets a run of
/// its own so fallback-font glyphs with odd advances can't push later text off the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub col: usize,
    /// Cells covered (a wide character covers 2).
    pub cells: usize,
    pub text: String,
    pub fg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}

/// A span of cells whose background differs from the terminal background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgRun {
    pub col: usize,
    pub cells: usize,
    pub color: Rgb,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub backgrounds: Vec<BgRun>,
    pub text: Vec<TextRun>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: Vec<Row>,
    pub background: Rgb,
    pub cursor: Option<CursorSnap>,
    /// Lines scrolled back into history (0 = following the live output).
    pub display_offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: Rgb,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}

pub(crate) fn capture(term: &Term<Listener>, palette: &Palette, focused: bool) -> Snapshot {
    let content = term.renderable_content();
    let cols = term.columns();
    let lines = term.screen_lines();
    let offset = content.display_offset;
    let overrides = content.colors;
    let background =
        overrides[alacritty_terminal::vte::ansi::NamedColor::Background].map_or(palette.background, Rgb::from);
    let cursor_color = overrides[alacritty_terminal::vte::ansi::NamedColor::Cursor].map_or(palette.cursor, Rgb::from);

    let cursor_point = content.cursor.point;
    let cursor_shape = match content.cursor.shape {
        AlacrittyShape::Hidden => None,
        _ if !focused => Some(CursorShape::HollowBlock),
        AlacrittyShape::Block => Some(CursorShape::Block),
        AlacrittyShape::HollowBlock => Some(CursorShape::HollowBlock),
        AlacrittyShape::Beam => Some(CursorShape::Beam),
        AlacrittyShape::Underline => Some(CursorShape::Underline),
    };
    let alacritty_shape = content.cursor.shape;
    let selection = content.selection;

    let mut rows = vec![Row::default(); lines];
    let mut cursor = None;
    let mut pending: Option<(usize, TextRun, Style)> = None;

    let flush = |pending: &mut Option<(usize, TextRun, Style)>, rows: &mut Vec<Row>| {
        if let Some((row, run, _)) = pending.take() {
            rows[row].text.push(run);
        }
    };

    for indexed in content.display_iter {
        let cell = indexed.cell;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let Some(vp) = point_to_viewport(offset, indexed.point) else { continue };
        let (row, col) = (vp.line, vp.column.0);
        if row >= lines {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut fg = palette.resolve(cell.fg, overrides);
        let mut bg = palette.resolve(cell.bg, overrides);
        if cell.flags.contains(Flags::DIM) {
            fg = fg.mix(bg, 0.4);
        }
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let point = Point::new(indexed.point.line, indexed.point.column);
        if selection.is_some_and(|s| s.contains_cell(&indexed, cursor_point, alacritty_shape)) {
            bg = palette.selection;
        }
        if point == cursor_point
            && let Some(shape) = cursor_shape
        {
            if shape == CursorShape::Block {
                fg = bg;
                bg = cursor_color;
            }
            cursor = Some(CursorSnap { row, col, cells: width, shape, color: cursor_color });
        }

        if bg != background {
            let bgs = &mut rows[row].backgrounds;
            match bgs.last_mut() {
                Some(last) if last.color == bg && last.col + last.cells == col => last.cells += width,
                _ => bgs.push(BgRun { col, cells: width, color: bg }),
            }
        }

        let style = Style {
            fg,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            strikeout: cell.flags.contains(Flags::STRIKEOUT),
        };
        let c = cell.c;
        let decorated = style.underline || style.strikeout;
        if (c == ' ' || c == '\t' || c == '\0') && !decorated {
            flush(&mut pending, &mut rows);
            continue;
        }
        let c = if c == '\t' || c == '\0' { ' ' } else { c };
        let zerowidth = cell.zerowidth();
        let simple = c.is_ascii() && zerowidth.is_none() && width == 1;

        if simple
            && let Some((prow, run, pstyle)) = pending.as_mut()
            && *prow == row
            && *pstyle == style
            && run.col + run.cells == col
        {
            run.text.push(c);
            run.cells += 1;
            continue;
        }
        flush(&mut pending, &mut rows);
        let mut text = String::with_capacity(8);
        text.push(c);
        text.extend(zerowidth.into_iter().flatten());
        let run = TextRun {
            col,
            cells: width,
            text,
            fg: style.fg,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            strikeout: style.strikeout,
        };
        if simple {
            pending = Some((row, run, style));
        } else {
            rows[row].text.push(run);
        }
    }
    flush(&mut pending, &mut rows);

    Snapshot { cols, rows, background, cursor, display_offset: offset }
}

impl Snapshot {
    /// The visible text, one line per row with trailing spaces trimmed. Handy for tests.
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|row| {
                let mut line = String::new();
                for run in &row.text {
                    while line.chars().count() < run.col {
                        line.push(' ');
                    }
                    line.push_str(&run.text);
                }
                line.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
