//! One shell in a PTY, parsed on its own IO thread.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term, TermMode, viewport_to_point};
use alacritty_terminal::tty;

use crate::color::Palette;
use crate::events::{Listener, Shared, TermEvent};
use crate::input::{self, KeyInput};
use crate::mouse::{self, MouseEvent, MouseEventKind};
use crate::snapshot::{self, Snapshot};

/// Terminal size in cells, plus the cell size in pixels (reported to programs that ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    fn window_size(self) -> WindowSize {
        WindowSize {
            num_cols: self.cols.max(1),
            num_lines: self.rows.max(1),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows.max(1) as usize
    }

    fn columns(&self) -> usize {
        self.cols.max(1) as usize
    }
}

/// A cell position in the visible viewport (row 0 is the top line on screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridPoint {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character by character (single click and drag).
    Simple,
    /// Whole words (double click).
    Word,
    /// Whole lines (triple click).
    Line,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub cwd: PathBuf,
    pub size: GridSize,
    pub scrollback: usize,
    pub palette: Palette,
    /// Program and arguments. `None` runs the user's login shell the platform's usual way.
    pub shell: Option<(String, Vec<String>)>,
    pub env: HashMap<String, String>,
}

impl SessionConfig {
    pub fn new(cwd: PathBuf, size: GridSize) -> Self {
        let env = [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "PortalWorkspaces"),
            ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        Self { cwd, size, scrollback: 10_000, palette: Palette::default(), shell: None, env }
    }
}

#[derive(Debug)]
pub struct SpawnError(std::io::Error);

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not start shell: {}", self.0)
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// A running shell. Dropping it closes the PTY, which hangs up the shell.
pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    shared: Arc<Shared>,
    pid: u32,
    size: GridSize,
}

impl Session {
    /// Starts a shell. `sink` is called from the session's IO thread for every [`TermEvent`].
    pub fn spawn(config: SessionConfig, sink: impl Fn(TermEvent) + Send + Sync + 'static) -> Result<Self, SpawnError> {
        let size = config.size;
        let shared = Arc::new(Shared {
            sink: Arc::new(sink),
            dirty: false.into(),
            palette: config.palette,
            window_size: Mutex::new(size.window_size()),
            loop_tx: OnceLock::new(),
        });
        let listener = Listener(shared.clone());

        let term_config =
            term::Config { scrolling_history: config.scrollback, kitty_keyboard: true, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(term_config, &size, listener.clone())));

        let options = tty::Options {
            shell: config.shell.map(|(program, args)| tty::Shell::new(program, args)),
            working_directory: Some(config.cwd),
            drain_on_exit: false,
            env: config.env,
        };
        let pty = tty::new(&options, size.window_size(), 0).map_err(SpawnError)?;
        let pid = pty.child().id();

        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false).map_err(SpawnError)?;
        let sender = event_loop.channel();
        let _ = shared.loop_tx.set(sender.clone());
        // The IO thread ends on its own when the shell exits or we send Shutdown.
        drop(event_loop.spawn());

        Ok(Self { term, notifier: Notifier(sender), shared, pid, size })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// The shell's current working directory, if the OS lets us see it.
    pub fn current_dir(&self) -> Option<PathBuf> {
        crate::cwd::current_dir(self.pid)
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        *self.shared.window_size.lock().expect("window size lock") = size.window_size();
        self.term.lock().resize(size);
        self.notifier.on_resize(size.window_size());
    }

    /// Sends raw bytes to the program.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.notifier.0.send(Msg::Input(bytes));
        }
    }

    /// Sends a key press. Returns whether it produced any bytes.
    pub fn send_key(&self, key: &KeyInput) -> bool {
        let mode = self.mode();
        let Some(bytes) = input::encode(key, mode) else { return false };
        self.follow_output();
        self.write(bytes);
        true
    }

    pub fn paste(&self, text: &str) {
        let bytes = input::encode_paste(text, self.mode());
        self.follow_output();
        self.write(bytes);
    }

    /// Scrolls back to the live output and drops any selection, as terminals do on typing.
    fn follow_output(&self) {
        let mut term = self.term.lock();
        term.selection = None;
        term.scroll_display(Scroll::Bottom);
    }

    fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Whether the running program wants mouse events (then clicks and drags go to it).
    pub fn wants_mouse(&self) -> bool {
        mouse::reporting(self.mode())
    }

    /// Reports a mouse event to the program if it asked for it. Returns whether it was sent.
    pub fn report_mouse(&self, event: MouseEvent) -> bool {
        match mouse::encode(event, self.mode()) {
            Some(bytes) => {
                self.write(bytes);
                true
            }
            None => false,
        }
    }

    /// Mouse wheel by `lines` (positive = up). Goes to the program when it reports the mouse,
    /// becomes arrow keys in full-screen programs (`less`, `man`), and scrolls history otherwise.
    pub fn wheel(&self, lines: i32, point: GridPoint, mods: input::Mods) {
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        if mouse::reporting(mode) {
            let kind = if lines > 0 { MouseEventKind::WheelUp } else { MouseEventKind::WheelDown };
            for _ in 0..lines.unsigned_abs() {
                self.report_mouse(MouseEvent { kind, point, mods });
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let app = mode.contains(TermMode::APP_CURSOR);
            let seq: &[u8] = match (lines > 0, app) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(seq.repeat(lines.unsigned_abs() as usize));
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
        }
    }

    pub fn scroll_page(&self, up: bool) {
        self.term.lock().scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
    }

    pub fn start_selection(&self, kind: SelectionKind, at: GridPoint, right_half: bool) {
        let mut term = self.term.lock();
        let point = self.to_point(&term, at);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, side(right_half)));
    }

    pub fn update_selection(&self, at: GridPoint, right_half: bool) {
        let mut term = self.term.lock();
        let point = self.to_point(&term, at);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side(right_half));
        }
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    /// Tells the program about focus changes if it asked (focus reporting, mode 1004).
    pub fn set_focused(&self, focused: bool) {
        let mut term = self.term.lock();
        term.is_focused = focused;
        if term.mode().contains(TermMode::FOCUS_IN_OUT) {
            drop(term);
            self.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
        }
    }

    /// Copies the visible screen for drawing. Clears the pending-wakeup flag first, so output
    /// that arrives during or after the capture triggers a new [`TermEvent::Wakeup`].
    pub fn snapshot(&self, focused: bool) -> Snapshot {
        self.shared.dirty.store(false, Ordering::Release);
        let term = self.term.lock();
        snapshot::capture(&term, &self.shared.palette, focused)
    }

    fn to_point(&self, term: &Term<Listener>, at: GridPoint) -> Point {
        let row = at.row.min(term.screen_lines().saturating_sub(1));
        let col = at.col.min(term.columns().saturating_sub(1));
        viewport_to_point(term.grid().display_offset(), Point::new(row, Column(col)))
    }
}

fn side(right_half: bool) -> Side {
    if right_half { Side::Right } else { Side::Left }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}
