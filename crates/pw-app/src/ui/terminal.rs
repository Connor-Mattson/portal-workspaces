//! Draws one terminal and turns mouse input on it into grid-level messages.
//!
//! Each pane owns a geometry cache. It is cleared only when that terminal changed, so a pane
//! that isn't producing output costs nothing to redraw, however busy its neighbours are.

use std::time::{Duration, Instant};

use iced::advanced::text::Shaping;
use iced::keyboard::Modifiers;
use iced::mouse::{self, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::text::{Alignment, LineHeight};
use iced::{Point, Rectangle, Renderer, Size, Theme, alignment, window};
use pw_model::PaneId;
use pw_term::{CursorShape, GridPoint, GridSize, Mods, MouseButton, Snapshot};

use crate::app::Message;
use crate::fonts::{self, CellMetrics};
use crate::sessions::PaneRuntime;
use crate::theme::to_color;
use crate::ui::term_menu;

/// Inner padding between the pane edge and the text grid.
const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 4.0;
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Lines scrolled per wheel notch.
const WHEEL_LINES: f32 = 3.0;

#[derive(Debug, Clone)]
pub enum TermMsg {
    Resize(GridSize),
    /// `menu_at` is where the right-click menu goes if this click opens it (window coordinates).
    MouseDown {
        point: GridPoint,
        right_half: bool,
        button: MouseButton,
        clicks: u8,
        mods: Mods,
        menu_at: Point,
    },
    MouseDrag {
        point: GridPoint,
        right_half: bool,
        button: MouseButton,
        mods: Mods,
    },
    MouseUp {
        point: GridPoint,
        button: MouseButton,
        mods: Mods,
    },
    Wheel {
        lines: i32,
        point: GridPoint,
        mods: Mods,
    },
}

pub struct TerminalCanvas<'a> {
    pub pane: PaneId,
    pub rt: &'a PaneRuntime,
    pub focused: bool,
    pub metrics: CellMetrics,
}

#[derive(Default)]
pub struct State {
    pressed: Option<MouseButton>,
    last_click: Option<(Instant, GridPoint, u8)>,
    last_drag: Option<(GridPoint, bool)>,
    scroll_remainder: f32,
    modifiers: Modifiers,
}

impl TerminalCanvas<'_> {
    fn grid_for(&self, bounds: Rectangle) -> GridSize {
        let m = self.metrics;
        GridSize {
            cols: ((bounds.width - 2.0 * PAD_X) / m.width).floor().max(2.0) as u16,
            rows: ((bounds.height - 2.0 * PAD_Y) / m.height).floor().max(1.0) as u16,
            cell_width: m.width.round() as u16,
            cell_height: m.height as u16,
        }
    }

    /// Grid cell under `position` (clamped into the grid), and whether it's the cell's right half.
    fn cell_at(&self, bounds: Rectangle, position: Point) -> (GridPoint, bool) {
        let size = self.rt.session.as_ref().map_or_else(|| self.grid_for(bounds), |s| s.size());
        let x = ((position.x - bounds.x - PAD_X) / self.metrics.width).max(0.0);
        let y = ((position.y - bounds.y - PAD_Y) / self.metrics.height).max(0.0);
        let col = (x as usize).min(size.cols.saturating_sub(1) as usize);
        let row = (y as usize).min(size.rows.saturating_sub(1) as usize);
        (GridPoint { row, col }, x.fract() >= 0.5)
    }

    fn publish(&self, msg: TermMsg) -> canvas::Action<Message> {
        canvas::Action::publish(Message::Terminal(self.pane, msg)).and_capture()
    }
}

fn mods(m: Modifiers) -> Mods {
    Mods { shift: m.shift(), ctrl: m.control(), alt: m.alt(), logo: m.logo() }
}

fn button(b: mouse::Button) -> Option<MouseButton> {
    match b {
        mouse::Button::Left => Some(MouseButton::Left),
        mouse::Button::Middle => Some(MouseButton::Middle),
        mouse::Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

impl canvas::Program<Message> for TerminalCanvas<'_> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            // Layout changes always come with a redraw, so this is where size changes are noticed.
            // During a divider drag the app defers them, and the old grid is drawn until it's let go.
            Event::Window(window::Event::RedrawRequested(_)) => {
                let session = self.rt.session.as_ref()?;
                let grid = self.grid_for(bounds);
                (session.target_size() != grid)
                    .then(|| canvas::Action::publish(Message::Terminal(self.pane, TermMsg::Resize(grid))))
            }
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(b)) => {
                let position = cursor.position_in(bounds).map(|p| Point::new(p.x + bounds.x, p.y + bounds.y))?;
                let button = button(*b)?;
                let (point, right_half) = self.cell_at(bounds, position);
                let now = Instant::now();
                let clicks = match state.last_click {
                    Some((at, p, n)) if p == point && now - at < MULTI_CLICK && button == MouseButton::Left => {
                        n % 3 + 1
                    }
                    _ => 1,
                };
                state.last_click = Some((now, point, clicks));
                state.pressed = Some(button);
                state.last_drag = Some((point, right_half));
                Some(self.publish(TermMsg::MouseDown {
                    point,
                    right_half,
                    button,
                    clicks,
                    mods: mods(state.modifiers),
                    menu_at: term_menu::place(position, bounds),
                }))
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let button = state.pressed?;
                let (point, right_half) = self.cell_at(bounds, *position);
                if state.last_drag == Some((point, right_half)) {
                    return None;
                }
                state.last_drag = Some((point, right_half));
                Some(self.publish(TermMsg::MouseDrag { point, right_half, button, mods: mods(state.modifiers) }))
            }
            Event::Mouse(mouse::Event::ButtonReleased(b)) => {
                let button = button(*b)?;
                if state.pressed != Some(button) {
                    return None;
                }
                state.pressed = None;
                let position = cursor.position().unwrap_or(bounds.position());
                let (point, _) = self.cell_at(bounds, position);
                Some(self.publish(TermMsg::MouseUp { point, button, mods: mods(state.modifiers) }))
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let position = cursor.position_in(bounds).map(|p| Point::new(p.x + bounds.x, p.y + bounds.y))?;
                let lines = match delta {
                    ScrollDelta::Lines { y, .. } => y * WHEEL_LINES,
                    ScrollDelta::Pixels { y, .. } => y / self.metrics.height,
                } + state.scroll_remainder;
                let whole = lines.trunc();
                state.scroll_remainder = lines - whole;
                if whole == 0.0 {
                    return Some(canvas::Action::capture());
                }
                let (point, _) = self.cell_at(bounds, position);
                Some(self.publish(TermMsg::Wheel { lines: whole as i32, point, mods: mods(state.modifiers) }))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.rt.cache.draw(renderer, bounds.size(), |frame| {
            if let Some(session) = &self.rt.session {
                paint(frame, session.snapshot(self.focused), self.metrics);
            }
        });
        vec![geometry]
    }

    fn mouse_interaction(&self, _state: &State, bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
        if cursor.is_over(bounds) { mouse::Interaction::Text } else { mouse::Interaction::default() }
    }
}

fn paint(frame: &mut Frame, snap: Snapshot, m: CellMetrics) {
    frame.fill_rectangle(Point::ORIGIN, frame.size(), to_color(snap.background));
    // Column edges are rounded so adjacent backgrounds meet without seams.
    let x = |col: usize| (PAD_X + col as f32 * m.width).round();

    for (r, row) in snap.rows.into_iter().enumerate() {
        let y = PAD_Y + r as f32 * m.height;
        for bg in &row.backgrounds {
            let (x0, x1) = (x(bg.col), x(bg.col + bg.cells));
            frame.fill_rectangle(Point::new(x0, y), Size::new(x1 - x0, m.height), to_color(bg.color));
        }
        for run in row.text {
            let color = to_color(run.fg);
            let left = PAD_X + run.col as f32 * m.width;
            // Runs of characters the bundled font has at one cell wide need no shaping or fallback.
            let shaping = if run.text.chars().all(pw_term::fits_a_cell) { Shaping::Basic } else { Shaping::Advanced };
            frame.fill_text(Text {
                content: run.text,
                position: Point::new(left, y),
                max_width: f32::INFINITY,
                color,
                size: m.font_size.into(),
                line_height: LineHeight::Absolute(m.height.into()),
                font: fonts::mono(run.bold, run.italic),
                align_x: Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping,
            });
            let width = run.cells as f32 * m.width;
            if run.underline {
                frame.fill_rectangle(Point::new(left, y + m.height - 2.0), Size::new(width, 1.0), color);
            }
            if run.strikeout {
                frame.fill_rectangle(Point::new(left, y + (m.height / 2.0).round()), Size::new(width, 1.0), color);
            }
        }
    }

    if let Some(cursor) = snap.cursor {
        let origin = Point::new(x(cursor.col), PAD_Y + cursor.row as f32 * m.height);
        let width = x(cursor.col + cursor.cells) - origin.x;
        let color = to_color(cursor.color);
        match cursor.shape {
            CursorShape::Block => {}
            CursorShape::HollowBlock => {
                let rect =
                    Path::rectangle(Point::new(origin.x + 0.5, origin.y + 0.5), Size::new(width - 1.0, m.height - 1.0));
                frame.stroke(&rect, Stroke::default().with_color(color).with_width(1.0));
            }
            CursorShape::Beam => frame.fill_rectangle(origin, Size::new(2.0, m.height), color),
            CursorShape::Underline => {
                frame.fill_rectangle(Point::new(origin.x, origin.y + m.height - 2.0), Size::new(width, 2.0), color)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::graphics::text::cosmic_text::skrifa::{self, MetadataProvider, instance::Size as FontSize};

    use crate::fonts;

    /// `pw_term::fits_a_cell` decides which characters share a run drawn without fallback fonts. Each one must be
    /// in every style of the bundled monospace font, one cell (600/1000 em) wide.
    #[test]
    fn every_character_that_fits_a_cell_is_one_cell_in_every_mono_face() {
        let chars: Vec<char> = (0..=0x10ffff).filter_map(char::from_u32).filter(|&c| pw_term::fits_a_cell(c)).collect();
        assert!(chars.len() > 300);
        for data in &fonts::DATA[3..] {
            let font = skrifa::FontRef::new(data).unwrap();
            let location = skrifa::instance::LocationRef::default();
            let em = f32::from(font.metrics(FontSize::unscaled(), location).units_per_em);
            let charmap = font.charmap();
            let metrics = font.glyph_metrics(FontSize::unscaled(), location);
            for &c in &chars {
                let glyph = charmap.map(c).unwrap_or_else(|| panic!("U+{:04X} is missing", c as u32));
                assert_eq!(metrics.advance_width(glyph), Some(0.6 * em), "U+{:04X}", c as u32);
            }
        }
    }
}
