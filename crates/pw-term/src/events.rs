//! Bridges alacritty's event callbacks (fired on the IO thread) to the app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoopSender, Msg};

use crate::activity::Activity;
use crate::color::Palette;

/// Something the GUI needs to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// New output was parsed. Sent once, then not again until the next [`Session::snapshot`]
    /// (`crate::Session::snapshot`), so a flood of output costs the UI one message per frame.
    Wakeup,
    Title(String),
    ResetTitle,
    Bell,
    /// The shell exited, with its exit code when it has one.
    Exited(Option<i32>),
    /// The program asked to copy text (OSC 52).
    ClipboardStore(String),
    /// The program sent a desktop notification (OSC 9, 777 or 99): agents say they're done or
    /// need you this way.
    Notify {
        title: Option<String>,
        body: String,
    },
    /// The terminal has been printing steadily for a while (see [`crate::ActivityConfig`]).
    Busy,
    /// A busy terminal went quiet, after `worked` of output.
    Idle {
        worked: Duration,
    },
}

pub(crate) type Sink = Arc<dyn Fn(TermEvent) + Send + Sync>;

/// State shared between the session handle, the terminal and the IO thread.
pub(crate) struct Shared {
    pub sink: Sink,
    pub dirty: AtomicBool,
    pub palette: Palette,
    pub activity: Activity,
    pub window_size: Mutex<WindowSize>,
    /// Set right after the event loop is created; the terminal needs a listener before that.
    pub loop_tx: OnceLock<EventLoopSender>,
}

#[derive(Clone)]
pub(crate) struct Listener(pub Arc<Shared>);

impl Listener {
    fn reply(&self, text: String) {
        if let Some(tx) = self.0.loop_tx.get() {
            let _ = tx.send(Msg::Input(text.into_bytes().into()));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let shared = &self.0;
        match event {
            Event::Wakeup => {
                if !shared.dirty.swap(true, Ordering::AcqRel) {
                    (shared.sink)(TermEvent::Wakeup);
                }
            }
            Event::Title(title) => (shared.sink)(TermEvent::Title(title)),
            Event::ResetTitle => (shared.sink)(TermEvent::ResetTitle),
            Event::Bell => (shared.sink)(TermEvent::Bell),
            Event::ChildExit(status) => (shared.sink)(TermEvent::Exited(status.code())),
            Event::ClipboardStore(_, text) => (shared.sink)(TermEvent::ClipboardStore(text)),
            // Replies the terminal can't produce itself: device reports, color and size queries.
            Event::PtyWrite(text) => self.reply(text),
            Event::ColorRequest(index, format) => {
                let palette = &shared.palette;
                let color = match index {
                    0..=255 => palette.indexed(index as u8),
                    256 => palette.foreground,
                    257 => palette.background,
                    _ => palette.cursor,
                };
                self.reply(format(color.into()));
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *shared.window_size.lock().expect("window size lock");
                self.reply(format(size));
            }
            // Reading the clipboard from a program (OSC 52 paste) is deliberately unsupported.
            Event::ClipboardLoad(..) | Event::MouseCursorDirty | Event::CursorBlinkingChange | Event::Exit => {}
        }
    }
}
