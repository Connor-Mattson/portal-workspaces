//! The code editor widget: a line-number gutter, a current-line band, bracket matching, find
//! highlights, and a scrollbar that shows itself on hover or scroll.
//!
//! It is a fork of iced 0.14's `text_editor` (MIT), by way of Portal Papers. The gutter and the
//! scrollbar need the editor's scroll position and line layout, which iced keeps private. Editing
//! still goes through iced's `Action`s and its cosmic-text editor, so typing, selection and IME
//! behave exactly like iced's widget. See `docs/decisions/0008-code-editor-widget.md`.
//!
//! Nothing here polls: the scrollbar's fade schedules redraws only while it is fading, like the
//! caret blink. Highlighting gets a slice of each frame ([`HIGHLIGHT_BUDGET`]); while visible lines are
//! still waiting for colour, the editor asks for another frame, and stops once they're done.

use std::cell::{Cell, RefCell};
use std::ops::{DerefMut, Range};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::graphics::text::cosmic_text;
use iced::advanced::input_method::{self, InputMethod};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::editor::Editor as _;
use iced::advanced::text::highlighter::{Format, Highlighter};
use iced::advanced::text::{self, LineHeight, Renderer as _, Text, Wrapping};
use iced::advanced::widget::{self, Widget, operation, tree};
use iced::advanced::{Clipboard, Shell, clipboard, mouse};
use iced::keyboard;
use iced::widget::text_editor::{Action, Binding, Cursor, Edit, KeyPress, Line, Selection, Status};
use iced::{Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Theme, alignment, window};
use pw_code::Language;

/// JetBrains Mono's advance is 600/1000 em.
const CHAR_WIDTH: f32 = 0.6;
/// Space left of the line numbers.
const GUTTER_LEFT: f32 = 14.0;
/// Space between the line numbers and the text.
const GUTTER_GAP: f32 = 18.0;
const PAD_TOP: f32 = 8.0;
const PAD_BOTTOM: f32 = 8.0;
/// Room right of the text, so the scrollbar never covers it.
const PAD_RIGHT: f32 = 18.0;
const TRACK_WIDTH: f32 = 12.0;
const THUMB_WIDTH: f32 = 6.0;
const THUMB_WIDTH_HOVER: f32 = 8.0;
const MIN_THUMB: f32 = 28.0;
/// How long the scrollbar stays after the last scroll (or after the pointer leaves).
const LINGER: Duration = Duration::from_millis(900);
const LINGER_AFTER_HOVER: Duration = Duration::from_millis(350);
const FADE: Duration = Duration::from_millis(220);
const FRAME: Duration = Duration::from_millis(16);
const CARET_BLINK_MILLIS: u128 = 500;
/// How long one frame may spend parsing for colours. The rest waits for the next frame, so jumping to
/// the end of a long file shows it at once and colours it over the following frames.
const HIGHLIGHT_BUDGET: Duration = Duration::from_millis(8);

/// A highlighter that can stop at a deadline and pick up where it stopped on the next pass.
pub trait Budgeted: Highlighter {
    fn set_deadline(&mut self, deadline: Instant);
}

type RawEditor = <iced::Renderer as text::Renderer>::Editor;

// ---- content -------------------------------------------------------------------------------

/// The text of an editor, with its cursor, selection and layout.
pub struct Content {
    editor: RefCell<RawEditor>,
    /// `Some` while edits are recorded (see [`record_edits`](Self::record_edits)), holding the text from before
    /// the first edit once there is one.
    recording: Option<Option<String>>,
    /// The first line edited since the highlighter last heard. Iced keeps only the latest edit's line, which
    /// loses an earlier, higher one when several edits land before the next layout (a mirrored view that
    /// isn't shown keeps collecting them), and the highlighter outlives the widget (it belongs to the tab).
    changed_from: Cell<Option<usize>>,
}

impl Content {
    pub fn with_text(text: &str) -> Self {
        Self { editor: RefCell::new(RawEditor::with_text(text)), recording: None, changed_from: Cell::new(None) }
    }

    pub fn perform(&mut self, action: Action) {
        if !matches!(action, Action::Edit(_)) {
            self.editor.get_mut().perform(action);
            return;
        }
        // Only `Edit`s change the text. The copy is taken once, before the first.
        if matches!(self.recording, Some(None)) {
            self.recording = Some(Some(self.text()));
        }
        let top = |c: Cursor| c.selection.map_or(c.position.line, |s| s.line.min(c.position.line));
        let before = top(self.cursor());
        self.editor.get_mut().perform(action);
        let first = before.min(top(self.cursor()));
        self.changed_from.set(Some(self.changed_from.get().map_or(first, |line| line.min(first))));
    }

    /// Starts noting whether the text changes, so mirroring can tell an edit from a scroll or a cursor move
    /// without copying the text for every action.
    pub fn record_edits(&mut self) {
        self.recording = Some(None);
    }

    /// Stops recording. The text from before the first edit since [`record_edits`](Self::record_edits), or
    /// `None` if nothing was edited.
    pub fn recorded(&mut self) -> Option<String> {
        self.recording.take().flatten()
    }

    pub fn move_to(&mut self, cursor: Cursor) {
        self.editor.get_mut().move_to(cursor);
    }

    pub fn cursor(&self) -> Cursor {
        self.editor.borrow().cursor()
    }

    pub fn line_count(&self) -> usize {
        self.editor.borrow().line_count()
    }

    pub fn line(&self, index: usize) -> Option<Line<'static>> {
        let editor = self.editor.borrow();
        let line = editor.line(index)?;
        Some(Line { text: line.text.into_owned().into(), ending: line.ending })
    }

    /// The whole text, lines joined with their endings.
    pub fn text(&self) -> String {
        #[cfg(test)]
        TEXT_COPIES.with(|n| n.set(n.get() + 1));
        self.with_lines(|lines| lines.join("\n"))
    }

    /// The selected text, if any.
    pub fn selection(&self) -> Option<String> {
        self.editor.borrow().copy()
    }

    /// Calls `f` with every line, without copying them.
    pub fn with_lines<T>(&self, f: impl FnOnce(&[&str]) -> T) -> T {
        let editor = self.editor.borrow();
        let lines: Vec<&str> = editor.buffer().lines.iter().map(|l| l.text()).collect();
        f(&lines)
    }
}

#[cfg(test)]
thread_local! {
    /// How many times this thread copied a whole text out of a [`Content`], for tests that check a path doesn't.
    pub static TEXT_COPIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Default for Content {
    fn default() -> Self {
        Self::with_text("")
    }
}

// ---- widget --------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub background: Color,
    pub text: Color,
    pub selection: Color,
    pub caret: Color,
    pub current_line: Color,
    pub line_number: Color,
    pub line_number_active: Color,
    pub bracket: Color,
    pub found: Color,
    pub found_current: Color,
    pub thumb: Color,
    pub thumb_active: Color,
}

type KeyBinding<'a, Message> = Box<dyn Fn(KeyPress) -> Option<Binding<Message>> + 'a>;

pub struct CodeEditor<'a, H: Highlighter, Message> {
    id: Option<widget::Id>,
    content: &'a Content,
    font: Font,
    text_size: f32,
    line_height: LineHeight,
    wrapping: Wrapping,
    key_binding: Option<KeyBinding<'a, Message>>,
    on_edit: Box<dyn Fn(Action) -> Message + 'a>,
    highlighter_settings: H::Settings,
    highlighter_format: fn(&H::Highlight, &Theme) -> Format<Font>,
    /// Decides bracket matching (comments and strings don't count).
    language: Language,
    /// Find results: (line, byte range).
    found: &'a [(usize, Range<usize>)],
    current_found: Option<usize>,
    style: Style,
}

impl<'a, H: Highlighter, Message> CodeEditor<'a, H, Message> {
    pub fn new(
        content: &'a Content,
        on_edit: impl Fn(Action) -> Message + 'a,
        settings: H::Settings,
        format: fn(&H::Highlight, &Theme) -> Format<Font>,
        style: Style,
    ) -> Self {
        Self {
            id: None,
            content,
            font: Font::MONOSPACE,
            text_size: 14.0,
            line_height: LineHeight::default(),
            wrapping: Wrapping::default(),
            key_binding: None,
            on_edit: Box::new(on_edit),
            highlighter_settings: settings,
            highlighter_format: format,
            language: Language::PLAIN,
            found: &[],
            current_found: None,
            style,
        }
    }

    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.text_size = size;
        self
    }

    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.line_height = line_height;
        self
    }

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    pub fn key_binding(mut self, f: impl Fn(KeyPress) -> Option<Binding<Message>> + 'a) -> Self {
        self.key_binding = Some(Box::new(f));
        self
    }

    pub fn language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    pub fn found(mut self, found: &'a [(usize, Range<usize>)], current: Option<usize>) -> Self {
        self.found = found;
        self.current_found = current;
        self
    }

    fn line_px(&self) -> f32 {
        self.line_height.to_absolute(Pixels(self.text_size)).0
    }

    fn geometry(&self, bounds: Rectangle) -> Geometry {
        let digits = self.content.line_count().max(1).ilog10() as f32 + 1.0;
        let gutter_width = GUTTER_LEFT + digits.max(3.0) * self.text_size * CHAR_WIDTH + GUTTER_GAP;
        let gutter = Rectangle { width: gutter_width.min(bounds.width), ..bounds };
        let text = Rectangle {
            x: bounds.x + gutter.width,
            y: bounds.y + PAD_TOP,
            width: (bounds.width - gutter.width - PAD_RIGHT).max(1.0),
            height: (bounds.height - PAD_TOP - PAD_BOTTOM).max(1.0),
        };
        let track = Rectangle {
            x: bounds.x + bounds.width - TRACK_WIDTH,
            y: bounds.y + 2.0,
            width: TRACK_WIDTH,
            height: (bounds.height - 4.0).max(0.0),
        };
        Geometry { bounds, line_px: self.line_px(), gutter, text, track }
    }

    fn scroll_metrics(&self, geo: &Geometry) -> ScrollMetrics {
        let editor = self.content.editor.borrow();
        ScrollMetrics::of(editor.buffer(), geo.text, self.wrapping, self.text_size * CHAR_WIDTH)
    }
}

struct Geometry {
    bounds: Rectangle,
    line_px: f32,
    gutter: Rectangle,
    text: Rectangle,
    track: Rectangle,
}

/// Where the view is, in visual (wrapped) lines.
struct ScrollMetrics {
    /// Visual line where each buffer line starts, plus the total at the end.
    starts: Vec<f32>,
    offset: f32,
    visible: f32,
}

impl ScrollMetrics {
    fn of(buffer: &cosmic_text::Buffer, text: Rectangle, wrapping: Wrapping, char_width: f32) -> Self {
        let line_height = buffer.metrics().line_height.max(1.0);
        let mut starts = Vec::with_capacity(buffer.lines.len() + 1);
        let mut total = 0.0;
        for line in &buffer.lines {
            starts.push(total);
            total += match line.layout_opt() {
                Some(layout) => layout.len().max(1) as f32,
                // Not laid out yet (far from the view): estimate the wrapping. The font is
                // monospace, so this is close.
                None if wrapping != Wrapping::None => {
                    (line.text().chars().count() as f32 * char_width / text.width).ceil().max(1.0)
                }
                None => 1.0,
            };
        }
        starts.push(total);
        let scroll = buffer.scroll();
        let offset = starts.get(scroll.line).copied().unwrap_or(0.0) + scroll.vertical / line_height;
        Self { starts, offset, visible: text.height / line_height }
    }

    fn total(&self) -> f32 {
        self.starts.last().copied().unwrap_or(0.0)
    }

    fn max_offset(&self) -> f32 {
        (self.total() - self.visible).max(0.0)
    }

    fn overflows(&self) -> bool {
        self.max_offset() > 0.5
    }

    fn thumb(&self, track: Rectangle) -> Rectangle {
        let height =
            (track.height * self.visible / self.total().max(1.0)).clamp(MIN_THUMB.min(track.height), track.height);
        let progress = (self.offset / self.max_offset().max(f32::EPSILON)).clamp(0.0, 1.0);
        Rectangle { y: track.y + (track.height - height) * progress, height, ..track }
    }

    /// The offset that puts the thumb's top at `y`.
    fn offset_for_thumb_top(&self, track: Rectangle, y: f32) -> f32 {
        let thumb = self.thumb(track);
        let room = (track.height - thumb.height).max(1.0);
        ((y - track.y) / room).clamp(0.0, 1.0) * self.max_offset()
    }

    fn y_of_line(&self, track: Rectangle, line: usize) -> f32 {
        let start = self.starts.get(line).copied().unwrap_or(0.0);
        track.y + track.height * start / self.total().max(1.0)
    }
}

// ---- state ---------------------------------------------------------------------------------

pub struct State<H: Highlighter> {
    focus: Option<Focus>,
    preedit: Option<input_method::Preedit>,
    last_click: Option<mouse::Click>,
    drag: Option<Drag>,
    partial_scroll: f32,
    hovered: bool,
    over_track: bool,
    /// The scrollbar shows until then, then fades.
    scrollbar_until: Option<Instant>,
    scrollbar_alpha: f32,
    highlighter: RefCell<H>,
    highlighter_settings: H::Settings,
    highlighter_format_address: usize,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Text,
    Gutter,
    /// `grab` is where in the thumb it was grabbed; `requested` the offset asked for so far
    /// (the editor applies it after this event batch).
    Scrollbar {
        grab: f32,
        requested: f32,
    },
}

#[derive(Debug, Clone)]
struct Focus {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl Focus {
    fn now() -> Self {
        let now = Instant::now();
        Self { updated_at: now, now, is_window_focused: true }
    }

    fn is_caret_visible(&self) -> bool {
        self.is_window_focused && ((self.now - self.updated_at).as_millis() / CARET_BLINK_MILLIS).is_multiple_of(2)
    }
}

impl<H: Highlighter> State<H> {
    fn dragging_scrollbar(&self) -> bool {
        matches!(self.drag, Some(Drag::Scrollbar { .. }))
    }

    fn show_scrollbar_for(&mut self, linger: Duration) {
        let until = Instant::now() + linger;
        self.scrollbar_until = Some(self.scrollbar_until.map_or(until, |u| u.max(until)));
    }

    /// Updates the scrollbar's opacity for this frame and schedules the next one while fading.
    fn animate_scrollbar(&mut self, now: Instant) -> Option<Instant> {
        if self.hovered || self.dragging_scrollbar() {
            self.scrollbar_alpha = 1.0;
            return None;
        }
        let Some(until) = self.scrollbar_until else {
            self.scrollbar_alpha = 0.0;
            return None;
        };
        if now < until {
            self.scrollbar_alpha = 1.0;
            return Some(until);
        }
        let faded = (now - until).as_secs_f32() / FADE.as_secs_f32();
        if faded >= 1.0 {
            self.scrollbar_alpha = 0.0;
            self.scrollbar_until = None;
            None
        } else {
            self.scrollbar_alpha = 1.0 - faded;
            Some(now + FRAME)
        }
    }
}

impl<H: Highlighter> operation::Focusable for State<H> {
    fn is_focused(&self) -> bool {
        self.focus.is_some()
    }

    fn focus(&mut self) {
        self.focus = Some(Focus::now());
    }

    fn unfocus(&mut self) {
        self.focus = None;
    }
}

// ---- widget impl ---------------------------------------------------------------------------

impl<H, Message> Widget<Message, Theme, iced::Renderer> for CodeEditor<'_, H, Message>
where
    H: Budgeted,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<H>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            focus: None,
            preedit: None,
            last_click: None,
            drag: None,
            partial_scroll: 0.0,
            hovered: false,
            over_track: false,
            scrollbar_until: None,
            scrollbar_alpha: 0.0,
            highlighter: RefCell::new(H::new(&self.highlighter_settings)),
            highlighter_settings: self.highlighter_settings.clone(),
            highlighter_format_address: self.highlighter_format as usize,
        })
    }

    fn size(&self) -> Size<Length> {
        Size { width: Length::Fill, height: Length::Fill }
    }

    fn layout(&mut self, tree: &mut widget::Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<H>>();
        if state.highlighter_format_address != self.highlighter_format as usize {
            state.highlighter.borrow_mut().change_line(0);
            state.highlighter_format_address = self.highlighter_format as usize;
        }
        if state.highlighter_settings != self.highlighter_settings {
            state.highlighter.borrow_mut().update(&self.highlighter_settings);
            state.highlighter_settings = self.highlighter_settings.clone();
        }
        let size = limits.width(Length::Fill).height(Length::Fill).max();
        let geo = self.geometry(Rectangle::with_size(size));
        self.content.editor.borrow_mut().update(
            geo.text.size(),
            self.font,
            Pixels(self.text_size),
            self.line_height,
            self.wrapping,
            state.highlighter.borrow_mut().deref_mut(),
        );
        if let Some(line) = self.content.changed_from.take() {
            state.highlighter.borrow_mut().change_line(line);
        }
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State<H>>();
        let geo = self.geometry(layout.bounds());

        match event {
            Event::Window(window::Event::Unfocused) => {
                if let Some(focus) = &mut state.focus {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                if let Some(focus) = &mut state.focus {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(focus) = &mut state.focus
                    && focus.is_window_focused
                {
                    focus.now = *now;
                    let millis = CARET_BLINK_MILLIS - (focus.now - focus.updated_at).as_millis() % CARET_BLINK_MILLIS;
                    shell.request_redraw_at(focus.now + Duration::from_millis(millis as u64));
                }
                if let Some(at) = state.animate_scrollbar(*now) {
                    shell.request_redraw_at(at);
                }
                // The last frame ran out of time before the visible lines were coloured.
                let last = last_visible_line(self.content.editor.borrow().buffer(), geo.text.height);
                if state.highlighter.borrow().current_line() <= last {
                    shell.request_redraw();
                }
                shell.request_input_method(&self.input_method(state, &geo));
                return;
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hovered = cursor.is_over(geo.bounds);
                let over_track = hovered && cursor.is_over(geo.track);
                if hovered != state.hovered || over_track != state.over_track {
                    if state.hovered && !hovered {
                        state.show_scrollbar_for(LINGER_AFTER_HOVER);
                    }
                    state.hovered = hovered;
                    state.over_track = over_track;
                    shell.request_redraw();
                }
            }
            _ => {}
        }

        if self.scrollbar(state, event, &geo, cursor, shell) {
            shell.capture_event();
            return;
        }

        let Some(update) = Update::from_event(event, state, &geo, cursor, self.key_binding.as_deref()) else {
            return;
        };
        match update {
            Update::Click(click) => {
                let action = match click.kind() {
                    mouse::click::Kind::Single => Action::Click(click.position()),
                    mouse::click::Kind::Double => Action::SelectWord,
                    mouse::click::Kind::Triple => Action::SelectLine,
                };
                state.focus = Some(Focus::now());
                state.last_click = Some(click);
                state.drag = matches!(click.kind(), mouse::click::Kind::Single).then_some(Drag::Text);
                shell.publish((self.on_edit)(action));
                shell.capture_event();
            }
            Update::GutterClick(y) => {
                // Selects the whole line; dragging extends by lines.
                state.focus = Some(Focus::now());
                state.last_click = None;
                state.drag = Some(Drag::Gutter);
                shell.publish((self.on_edit)(Action::Click(Point::new(0.0, y))));
                shell.publish((self.on_edit)(Action::SelectLine));
                shell.capture_event();
            }
            Update::Drag(position) => {
                shell.publish((self.on_edit)(Action::Drag(position)));
            }
            Update::Release => state.drag = None,
            Update::Scroll(lines) => {
                let lines = lines + state.partial_scroll;
                state.partial_scroll = lines.fract();
                if lines.trunc() != 0.0 {
                    shell.publish((self.on_edit)(Action::Scroll { lines: lines as i32 }));
                }
                state.show_scrollbar_for(LINGER);
                shell.request_redraw();
                shell.capture_event();
            }
            Update::InputMethod(update) => match update {
                Ime::Toggle(is_open) => {
                    state.preedit = is_open.then(input_method::Preedit::new);
                    shell.request_redraw();
                }
                Ime::Preedit { content, selection } => {
                    state.preedit =
                        Some(input_method::Preedit { content, selection, text_size: Some(Pixels(self.text_size)) });
                    shell.request_redraw();
                }
                Ime::Commit(text) => {
                    shell.publish((self.on_edit)(Action::Edit(Edit::Paste(Arc::new(text)))));
                }
            },
            Update::Binding(binding) => {
                if !matches!(binding, Binding::Unfocus) {
                    shell.capture_event();
                }
                apply_binding(binding, self.content, state, &*self.on_edit, clipboard, shell);
                if let Some(focus) = &mut state.focus {
                    focus.updated_at = Instant::now();
                }
            }
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<H>>();
        let geo = self.geometry(layout.bounds());
        let style = self.style;
        let line_px = geo.line_px;

        let mut editor = self.content.editor.borrow_mut();
        let mut highlighter = state.highlighter.borrow_mut();
        highlighter.set_deadline(Instant::now() + HIGHLIGHT_BUDGET);
        editor.highlight(self.font, highlighter.deref_mut(), |h| (self.highlighter_format)(h, theme));
        drop(highlighter);
        let editor = &*editor;
        let buffer = editor.buffer();
        let cursor = editor.cursor();
        let text = geo.text;
        let origin = text.position() - Point::ORIGIN;
        // The text area's rows, full width (gutter included): what scrolls.
        let rows = Rectangle { x: geo.bounds.x, width: geo.bounds.width, ..text };

        renderer.fill_quad(renderer::Quad { bounds: geo.bounds, ..Default::default() }, style.background);

        // Buffer lines in view: (line, top relative to the text area, height).
        let visible = visible_lines(buffer, text.height);
        let first = visible.first().map_or(0, |v| v.0);
        let last = visible.last().map_or(0, |v| v.0);

        // Current line.
        let caret = matches!(editor.selection(), Selection::Caret(_));
        if state.focus.is_some()
            && caret
            && let Some(&(_, top, height)) = visible.iter().find(|v| v.0 == cursor.position.line)
            && let Some(band) = rows.intersection(&Rectangle { y: text.y + top, height, ..rows })
        {
            renderer.fill_quad(renderer::Quad { bounds: band, ..Default::default() }, style.current_line);
        }

        // Find results.
        for (i, (line, range)) in self.found.iter().enumerate() {
            if *line < first || *line > last {
                continue;
            }
            let current = self.current_found == Some(i);
            for rect in span_rects(buffer, *line, range.clone()) {
                if let Some(bounds) = text.intersection(&(rect + origin)) {
                    let quad = renderer::Quad {
                        bounds,
                        border: Border {
                            color: if current { style.found_current } else { Color::TRANSPARENT },
                            width: if current { 1.0 } else { 0.0 },
                            radius: 2.0.into(),
                        },
                        ..Default::default()
                    };
                    renderer.fill_quad(quad, if current { alpha(style.found_current, 0.28) } else { style.found });
                }
            }
        }

        // Bracket pair at the caret.
        if state.focus.is_some() && caret {
            let lines: Vec<&str> = buffer.lines.iter().map(|l| l.text()).collect();
            let pair =
                pw_code::editing::matching_bracket(self.language, &lines, cursor.position.line, cursor.position.column);
            for (line, column) in pair.into_iter().flatten() {
                for rect in span_rects(buffer, line, column..column + 1) {
                    if let Some(bounds) = text.intersection(&(rect + origin)) {
                        let quad = renderer::Quad {
                            bounds,
                            border: Border { color: style.bracket, width: 1.0, radius: 2.0.into() },
                            ..Default::default()
                        };
                        renderer.fill_quad(quad, alpha(style.bracket, 0.16));
                    }
                }
            }
        }

        // Selection.
        if let Selection::Range(ranges) = editor.selection() {
            let color = if state.focus.is_some() { style.selection } else { alpha(style.selection, 0.55) };
            for range in ranges.into_iter().filter_map(|r| text.intersection(&(r + origin))) {
                renderer.fill_quad(renderer::Quad { bounds: range, ..Default::default() }, color);
            }
        }

        // Text.
        if !editor.is_empty() {
            renderer.fill_editor(editor, text.position(), style.text, text);
        }

        // Caret.
        if let Some(focus) = &state.focus
            && let Selection::Caret(position) = editor.selection()
            && focus.is_caret_visible()
            && state.preedit.is_none()
        {
            let caret = Rectangle::new(position + origin, Size::new(2.0, line_px));
            if let Some(bounds) = text.intersection(&caret) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: Border { radius: 1.0.into(), ..Border::default() },
                        ..Default::default()
                    },
                    style.caret,
                );
            }
        }

        // Gutter: line numbers.
        let gutter_rows = Rectangle { x: geo.gutter.x, width: geo.gutter.width, ..text };
        let number_right = geo.gutter.x + geo.gutter.width - GUTTER_GAP;
        for &(line, top, _) in &visible {
            let color = if line == cursor.position.line { style.line_number_active } else { style.line_number };
            renderer.fill_text(
                Text {
                    content: (line + 1).to_string(),
                    bounds: Size::new(number_right - geo.gutter.x, line_px),
                    size: Pixels(self.text_size),
                    line_height: self.line_height,
                    font: self.font,
                    align_x: text::Alignment::Right,
                    align_y: alignment::Vertical::Top,
                    shaping: text::Shaping::Basic,
                    wrapping: Wrapping::None,
                },
                Point::new(number_right, text.y + top),
                color,
                gutter_rows,
            );
        }

        // Scrollbar, over everything.
        if state.scrollbar_alpha > 0.0 {
            let metrics = ScrollMetrics::of(buffer, text, self.wrapping, self.text_size * CHAR_WIDTH);
            if metrics.overflows() {
                renderer.with_layer(geo.bounds, |renderer| {
                    self.draw_scrollbar(renderer, state, &geo, &metrics, cursor.position.line);
                });
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State<H>>();
        let geo = self.geometry(layout.bounds());
        if state.dragging_scrollbar() || (cursor.is_over(geo.track) && state.scrollbar_alpha > 0.0) {
            mouse::Interaction::Idle
        } else if matches!(state.drag, Some(Drag::Text)) || cursor.is_over(geo.text) {
            mouse::Interaction::Text
        } else if cursor.is_over(geo.gutter) {
            mouse::Interaction::Pointer
        } else if cursor.is_over(geo.bounds) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State<H>>();
        operation.focusable(self.id.as_ref(), layout.bounds(), state);
    }
}

impl<H: Highlighter, Message> CodeEditor<'_, H, Message> {
    /// Scrollbar presses and drags. Returns whether it used the event.
    fn scrollbar(
        &self,
        state: &mut State<H>,
        event: &Event,
        geo: &Geometry,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        match (event, state.drag) {
            (Event::Mouse(mouse::Event::CursorMoved { .. }), Some(Drag::Scrollbar { grab, requested })) => {
                let Some(position) = cursor.position() else { return true };
                let metrics = self.scroll_metrics(geo);
                let target = metrics.offset_for_thumb_top(geo.track, position.y - grab);
                let lines = (target - requested).round();
                if lines != 0.0 {
                    shell.publish((self.on_edit)(Action::Scroll { lines: lines as i32 }));
                    state.drag = Some(Drag::Scrollbar { grab, requested: requested + lines });
                }
                true
            }
            (Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)), Some(Drag::Scrollbar { .. })) => {
                state.drag = None;
                state.show_scrollbar_for(LINGER_AFTER_HOVER);
                shell.request_redraw();
                true
            }
            (Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)), _)
                if state.scrollbar_alpha > 0.0 && cursor.is_over(geo.track) =>
            {
                let metrics = self.scroll_metrics(geo);
                let Some(position) = cursor.position() else { return false };
                if !metrics.overflows() {
                    return false;
                }
                let thumb = metrics.thumb(geo.track);
                let mut requested = metrics.offset;
                let grab = if thumb.contains(position) {
                    position.y - thumb.y
                } else {
                    // Jump so the thumb is centered on the pointer, then keep dragging.
                    let grab = thumb.height / 2.0;
                    let lines = (metrics.offset_for_thumb_top(geo.track, position.y - grab) - requested).round();
                    if lines != 0.0 {
                        shell.publish((self.on_edit)(Action::Scroll { lines: lines as i32 }));
                        requested += lines;
                    }
                    grab
                };
                state.drag = Some(Drag::Scrollbar { grab, requested });
                shell.request_redraw();
                true
            }
            _ => false,
        }
    }

    fn draw_scrollbar(
        &self,
        renderer: &mut iced::Renderer,
        state: &State<H>,
        geo: &Geometry,
        metrics: &ScrollMetrics,
        cursor_line: usize,
    ) {
        let style = self.style;
        let a = state.scrollbar_alpha;
        let track = geo.track;
        let active = state.over_track || state.dragging_scrollbar();
        let quad = |bounds: Rectangle, radius: f32| renderer::Quad {
            bounds,
            border: Border { radius: radius.into(), ..Border::default() },
            ..Default::default()
        };
        if active {
            renderer.fill_quad(quad(track, 0.0), alpha(style.thumb, 0.05 * a));
        }
        // Marks: find results and the caret line.
        let mark_x = track.x + track.width - 5.0;
        for (line, _) in self.found {
            let y = metrics.y_of_line(track, *line);
            renderer.fill_quad(
                quad(Rectangle::new(Point::new(mark_x, y), Size::new(4.0, 2.0)), 1.0),
                alpha(style.found_current, 0.8 * a),
            );
        }
        let y = metrics.y_of_line(track, cursor_line);
        renderer.fill_quad(
            quad(Rectangle::new(Point::new(track.x + 2.0, y), Size::new(track.width - 4.0, 2.0)), 1.0),
            alpha(style.line_number_active, 0.7 * a),
        );

        let width = if active { THUMB_WIDTH_HOVER } else { THUMB_WIDTH };
        let thumb = metrics.thumb(track);
        let thumb = Rectangle { x: track.x + track.width - width - 2.0, width, ..thumb };
        let color = if active { style.thumb_active } else { style.thumb };
        renderer.fill_quad(quad(thumb, width / 2.0), alpha(color, color.a * a));
    }

    fn input_method<'b>(&self, state: &'b State<H>, geo: &Geometry) -> InputMethod<&'b str> {
        let Some(Focus { is_window_focused: true, .. }) = &state.focus else {
            return InputMethod::Disabled;
        };
        let editor = self.content.editor.borrow();
        let position = match editor.selection() {
            Selection::Caret(position) => position,
            Selection::Range(ranges) => ranges.first().copied().unwrap_or_default().position(),
        };
        InputMethod::Enabled {
            cursor: Rectangle::new(position + (geo.text.position() - Point::ORIGIN), Size::new(1.0, self.line_px())),
            purpose: input_method::Purpose::Normal,
            preedit: state.preedit.as_ref().map(input_method::Preedit::as_ref),
        }
    }
}

impl<'a, H, Message> From<CodeEditor<'a, H, Message>> for Element<'a, Message, Theme, iced::Renderer>
where
    H: Budgeted,
    Message: 'a,
{
    fn from(editor: CodeEditor<'a, H, Message>) -> Self {
        Self::new(editor)
    }
}

// ---- helpers -------------------------------------------------------------------------------

fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

/// Buffer lines in view: (line, top relative to the text area, height in pixels).
fn visible_lines(buffer: &cosmic_text::Buffer, height: f32) -> Vec<(usize, f32, f32)> {
    let line_height = buffer.metrics().line_height;
    let scroll = buffer.scroll();
    let mut top = -scroll.vertical;
    let mut out = Vec::new();
    for (i, line) in buffer.lines.iter().enumerate().skip(scroll.line) {
        if top > height {
            break;
        }
        let Some(layout) = line.layout_opt() else { break };
        let h = layout.len().max(1) as f32 * line_height;
        out.push((i, top, h));
        top += h;
    }
    out
}

/// The last line iced's `Editor::highlight` colours, worked out the same way, so the editor asks for
/// another frame exactly while that pass has lines left.
fn last_visible_line(buffer: &cosmic_text::Buffer, height: f32) -> usize {
    let scroll = buffer.scroll();
    let mut window = (height / buffer.metrics().line_height).ceil() as i32;
    buffer
        .lines
        .iter()
        .enumerate()
        .skip(scroll.line)
        .find_map(|(i, line)| {
            let rows = line.layout_opt().map_or(1, Vec::len) as i32;
            if window > rows {
                window -= rows;
                None
            } else {
                Some(i)
            }
        })
        .unwrap_or(buffer.lines.len().saturating_sub(1))
}

/// Rectangles (relative to the text area) covering a byte range of a line, one per wrapped row.
fn span_rects(buffer: &cosmic_text::Buffer, line: usize, range: Range<usize>) -> Vec<Rectangle> {
    let start = cosmic_text::Cursor::new(line, range.start);
    let end = cosmic_text::Cursor::new(line, range.end);
    buffer
        .layout_runs()
        .skip_while(|run| run.line_i < line)
        .take_while(|run| run.line_i == line)
        .filter_map(|run| {
            let (x, width) = run.highlight(start, end)?;
            (width > 0.0).then(|| Rectangle::new(Point::new(x, run.line_top), Size::new(width, run.line_height)))
        })
        .collect()
}

fn apply_binding<H: Highlighter, Message>(
    binding: Binding<Message>,
    content: &Content,
    state: &mut State<H>,
    on_edit: &dyn Fn(Action) -> Message,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
) {
    let mut publish = |action| shell.publish(on_edit(action));
    match binding {
        Binding::Unfocus => {
            state.focus = None;
            state.drag = None;
            shell.request_redraw();
        }
        Binding::Copy => {
            if let Some(selection) = content.selection() {
                clipboard.write(clipboard::Kind::Standard, selection);
            }
        }
        Binding::Cut => {
            if let Some(selection) = content.selection() {
                clipboard.write(clipboard::Kind::Standard, selection);
                publish(Action::Edit(Edit::Delete));
            }
        }
        Binding::Paste => {
            if let Some(contents) = clipboard.read(clipboard::Kind::Standard) {
                publish(Action::Edit(Edit::Paste(Arc::new(contents))));
            }
        }
        Binding::Move(motion) => publish(Action::Move(motion)),
        Binding::Select(motion) => publish(Action::Select(motion)),
        Binding::SelectWord => publish(Action::SelectWord),
        Binding::SelectLine => publish(Action::SelectLine),
        Binding::SelectAll => publish(Action::SelectAll),
        Binding::Insert(c) => publish(Action::Edit(Edit::Insert(c))),
        Binding::Enter => publish(Action::Edit(Edit::Enter)),
        Binding::Backspace => publish(Action::Edit(Edit::Backspace)),
        Binding::Delete => publish(Action::Edit(Edit::Delete)),
        Binding::Sequence(sequence) => {
            for binding in sequence {
                apply_binding(binding, content, state, on_edit, clipboard, shell);
            }
        }
        Binding::Custom(message) => shell.publish(message),
    }
}

enum Update<Message> {
    Click(mouse::Click),
    /// A press in the gutter, at this y relative to the text area.
    GutterClick(f32),
    Drag(Point),
    Release,
    Scroll(f32),
    InputMethod(Ime),
    Binding(Binding<Message>),
}

enum Ime {
    Toggle(bool),
    Preedit { content: String, selection: Option<Range<usize>> },
    Commit(String),
}

impl<Message> Update<Message> {
    fn from_event<H: Highlighter>(
        event: &Event,
        state: &State<H>,
        geo: &Geometry,
        cursor: mouse::Cursor,
        key_binding: Option<&dyn Fn(KeyPress) -> Option<Binding<Message>>>,
    ) -> Option<Self> {
        let origin = geo.text.position() - Point::ORIGIN;
        match event {
            Event::Mouse(event) => match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if let Some(position) = cursor.position_over(geo.bounds) {
                        if geo.gutter.contains(position) {
                            return Some(Update::GutterClick(position.y - geo.text.y));
                        }
                        let click = mouse::Click::new(position - origin, mouse::Button::Left, state.last_click);
                        Some(Update::Click(click))
                    } else if state.focus.is_some() {
                        Some(Update::Binding(Binding::Unfocus))
                    } else {
                        None
                    }
                }
                mouse::Event::ButtonReleased(mouse::Button::Left) => state.drag.is_some().then_some(Update::Release),
                mouse::Event::CursorMoved { .. } => match state.drag {
                    Some(Drag::Text) => Some(Update::Drag(cursor.position()? - origin)),
                    Some(Drag::Gutter) => {
                        let position = cursor.position()? - origin;
                        Some(Update::Drag(Point::new(0.0, position.y)))
                    }
                    _ => None,
                },
                mouse::Event::WheelScrolled { delta } if cursor.is_over(geo.bounds) => {
                    Some(Update::Scroll(match delta {
                        mouse::ScrollDelta::Lines { y, .. } if y.abs() > 0.0 => y.signum() * -(y.abs() * 3.0).max(1.0),
                        mouse::ScrollDelta::Lines { .. } => 0.0,
                        // Trackpads: follow the fingers, one line per line height of travel.
                        mouse::ScrollDelta::Pixels { y, .. } => -y / geo.line_px,
                    }))
                }
                _ => None,
            },
            Event::InputMethod(event) => match event {
                input_method::Event::Opened | input_method::Event::Closed => {
                    Some(Update::InputMethod(Ime::Toggle(matches!(event, input_method::Event::Opened))))
                }
                input_method::Event::Preedit(content, selection) if state.focus.is_some() => {
                    Some(Update::InputMethod(Ime::Preedit { content: content.clone(), selection: selection.clone() }))
                }
                input_method::Event::Commit(content) if state.focus.is_some() => {
                    Some(Update::InputMethod(Ime::Commit(content.clone())))
                }
                _ => None,
            },
            Event::Keyboard(keyboard::Event::KeyPressed {
                key, modified_key, physical_key, modifiers, text, ..
            }) => {
                let status = if state.focus.is_some() {
                    Status::Focused { is_hovered: cursor.is_over(geo.bounds) }
                } else {
                    Status::Active
                };
                let press = KeyPress {
                    key: key.clone(),
                    modified_key: modified_key.clone(),
                    physical_key: *physical_key,
                    modifiers: *modifiers,
                    text: text.clone(),
                    status,
                };
                match key_binding {
                    Some(f) => f(press),
                    None => Binding::from_key_press(press),
                }
                .map(Update::Binding)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_round_trips_text() {
        let content = Content::with_text("a\n\nb\n");
        assert_eq!(content.text(), "a\n\nb\n");
        assert_eq!(content.line_count(), 4);
        assert_eq!(content.line(2).unwrap().text, "b");
        assert_eq!(content.with_lines(|l| l.len()), 4);
    }

    #[test]
    fn edits_report_the_first_line_they_touched_until_taken() {
        use iced::widget::text_editor::Position;
        let mut content = Content::with_text("a\nb\nc\nd\ne\nf");
        let at = |content: &mut Content, line| {
            content.move_to(Cursor { position: Position { line, column: 0 }, selection: None });
        };
        assert_eq!(content.changed_from.take(), None);
        // Several edits before the next layout (a mirrored view that isn't shown): the topmost counts,
        // where iced alone would keep only the last one's.
        at(&mut content, 4);
        content.perform(Action::Edit(Edit::Insert('x')));
        at(&mut content, 1);
        content.perform(Action::Edit(Edit::Insert('y')));
        at(&mut content, 5);
        content.perform(Action::Edit(Edit::Insert('z')));
        content.perform(Action::Move(iced::widget::text_editor::Motion::Up));
        assert_eq!(content.changed_from.take(), Some(1));
        assert_eq!(content.changed_from.take(), None);
        // Backspace at a line's start joins it to the line above.
        at(&mut content, 3);
        content.perform(Action::Edit(Edit::Backspace));
        assert_eq!(content.changed_from.take(), Some(2));
    }
}
