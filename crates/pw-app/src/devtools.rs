//! A scripted driver for checking the UI without a human at the keyboard.
//!
//! Set `PORTAL_WORKSPACES_DEV_SCRIPT` to a `;`-separated list of steps and the app plays them after boot,
//! saving screenshots of its own window:
//!
//! ```text
//! PORTAL_WORKSPACES_DEV_SCRIPT="wait 2; mode editor; open src/main.rs; wait 1; shot /tmp/pw/a.png; quit"
//! ```
//!
//! Steps:
//! - `wait <seconds>`, `shot <path.png>` (the main window), `quit` (asks about unsaved files), `cancel`
//!   (closes a sheet), `discard` (the unsaved sheet's "Don't save").
//! - `workspace <n>` (1-based), `mode editor|agents`, `drawer`, `system` (opens or closes the system profile),
//!   `shortcuts`, `attend` (goes to the terminal that has needed you longest, like Ctrl+Shift+I).
//! - Editor mode: `open <path>` (a pinned tab), `click <path>` (a tree row: previews a file, or opens or closes a
//!   folder), `split`, `terminal`, `explorer`, `find <query>`, `replace`, `quick-open <query>`, `save`.
//! - Input: `type <text>` (into the focused code editor; `\n` is Enter), `term <text>` (written to the terminal
//!   that has the keys; `\n` is Enter), `key <combo>` (e.g. `ctrl+shift+m`, `escape`, `down`, `ctrl+s`,
//!   `ctrl+backslash`).
//! - Terminal menu: `menu <x> <y>` (opens the right-click menu of the terminal that has the keys, at a point in
//!   the main window), `menu copy|paste|select-all|close`.
//! - `divider <ratio>`: drags the active grid's first divider to `ratio` (0 to 1), as a mouse would;
//!   `release` lets go of it.
//! - `panic`: panics on the UI thread, to check the crash report in the log file.
//!
//! A script with a step it doesn't know quits right away, with an error in the log.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::futures::future::{Either, select};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::{Point, Subscription, Task, window};
use pw_model::{Axis, Mode};

use crate::app::{App, EditorMsg, ExplorerMsg, Message};
use crate::background::{blocking, sleep};
use crate::keymap::Action;
use crate::ui::modal::ModalMsg;
use crate::ui::term_menu::{MenuMsg, TermMenu};

pub const SCRIPT_VAR: &str = "PORTAL_WORKSPACES_DEV_SCRIPT";

/// How long a shot waits after a drawn frame before capturing it.
const SETTLE: Duration = Duration::from_millis(150);
/// A shot that never lands (no frame drawn) stops holding up the script after this long.
const SHOT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
enum Step {
    Wait(Duration),
    Shot(PathBuf),
    /// Several messages, in order.
    Send(Vec<Message>),
}

/// Whether a script is playing. The main window then opens on top, so a screenshot shows what was drawn.
pub fn scripted() -> bool {
    std::env::var_os(SCRIPT_VAR).is_some()
}

/// The task playing the script in `PORTAL_WORKSPACES_DEV_SCRIPT`, if set.
pub fn from_env() -> Option<Task<Message>> {
    let script = std::env::var(SCRIPT_VAR).ok()?;
    let steps = match parse(&script) {
        Ok(steps) => steps,
        // Quit rather than sit there: whoever runs a script is waiting for it to finish.
        Err(step) => {
            tracing::error!(%step, "unknown dev script step");
            return Some(iced::exit());
        }
    };
    tracing::info!(steps = steps.len(), "playing dev script");
    // Give the window a moment to open and settle.
    let start = Task::future(sleep(Duration::from_millis(500))).discard();
    Some(steps.into_iter().fold(start, |task, step| task.chain(play(step))))
}

fn play(step: Step) -> Task<Message> {
    match step {
        // Lazy: the timer starts when the step runs, not when the script is built.
        Step::Wait(duration) => Task::future(async move { sleep(duration).await }).discard(),
        Step::Send(messages) => messages.into_iter().fold(Task::none(), |task, m| task.chain(Task::done(m))),
        // Ask the app for a shot and hold the script until it's saved.
        Step::Shot(path) => {
            let (done, saved) = oneshot::channel();
            let request = ShotRequest { path, done: Arc::new(Mutex::new(Some(done))) };
            let wait = async move {
                let timeout = std::pin::pin!(sleep(SHOT_TIMEOUT));
                if let Either::Right(_) = select(saved, timeout).await {
                    tracing::warn!("screenshot timed out; continuing the script");
                }
            };
            Task::done(Message::Dev(DevMsg::Shot(request))).chain(Task::future(wait).discard())
        }
    }
}

#[derive(Debug, Clone)]
pub enum DevMsg {
    Shot(ShotRequest),
    /// The window drew a frame.
    Frame,
    /// Keeps an idle app redrawing while a shot waits for a frame.
    Nudge,
    /// A key press, as if typed into the main window.
    Key(Key, Modifiers),
    /// Text written to the terminal that has the keys.
    Term(String),
    /// Opens a file as a pinned tab in the focused group.
    Open(PathBuf),
    /// Opens the right-click menu of the terminal that has the keys, at a point in the main window.
    Menu(Point),
    /// Moves the active grid's first divider, as a drag does.
    Divider(f32),
    Panic,
}

/// A screenshot waiting for the next drawn frame. iced captures by re-rendering the last frame the window
/// drew, so capturing right after a change would show the state before it.
#[derive(Debug, Clone)]
pub struct ShotRequest {
    path: PathBuf,
    done: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

/// The app's side of the script: at most one pending screenshot.
#[derive(Debug, Default)]
pub struct Dev {
    pending: Option<ShotRequest>,
}

impl Dev {
    /// Frames (and a nudge timer) only while a shot waits, so an unscripted app gets nothing from here.
    pub fn subscription(&self) -> Subscription<Message> {
        match self.pending {
            Some(_) => Subscription::batch([
                window::frames().map(|_| Message::Dev(DevMsg::Frame)),
                iced::time::every(Duration::from_millis(250)).map(|_| Message::Dev(DevMsg::Nudge)),
            ]),
            None => Subscription::none(),
        }
    }
}

impl App {
    pub(crate) fn on_dev(&mut self, msg: DevMsg) -> Task<Message> {
        match msg {
            DevMsg::Nudge => Task::none(),
            DevMsg::Shot(request) => {
                self.dev.pending = Some(request);
                Task::none()
            }
            DevMsg::Frame => {
                let Some(ShotRequest { path, done }) = self.dev.pending.take() else { return Task::none() };
                let window = self.main_window;
                // Not right away: this very update re-lays out the UI. Let the redraw that follows land first.
                Task::future(sleep(SETTLE))
                    .then(move |()| window::screenshot(window))
                    .then(move |shot| {
                        let (path, done) = (path.clone(), done.clone());
                        Task::perform(blocking(move || save_png(&path, &shot)), move |result| {
                            if let Err(err) = result {
                                tracing::error!(%err, "screenshot failed");
                            }
                            if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                                let _ = done.send(());
                            }
                        })
                    })
                    .discard()
            }
            DevMsg::Key(key, modifiers) => {
                let text = match &key {
                    Key::Character(c) if !modifiers.control() && !modifiers.logo() => Some(c.to_string()),
                    Key::Named(Named::Space) => Some(" ".to_owned()),
                    _ => None,
                };
                self.update(Message::KeyPressed { window: self.main_window, key, modifiers, text })
            }
            DevMsg::Term(text) => {
                if let Some(session) =
                    self.key_pane().and_then(|p| self.sessions.get(p)).and_then(|rt| rt.session.as_ref())
                {
                    session.write(text.replace('\n', "\r").into_bytes());
                }
                Task::none()
            }
            DevMsg::Open(path) => self.open_file(&path, false, true),
            DevMsg::Panic => panic!("the dev script asked for a panic"),
            DevMsg::Divider(ratio) => {
                let split = self.active_view().and_then(|ws| ws.grid.as_ref()?.layout().splits().next().copied());
                match split {
                    Some(split) => {
                        self.update(Message::PaneResized(iced::widget::pane_grid::ResizeEvent { split, ratio }))
                    }
                    None => Task::none(),
                }
            }
            DevMsg::Menu(at) => {
                if let Some(pane) = self.key_pane() {
                    let can_copy =
                        self.sessions.get(pane).and_then(|rt| rt.session.as_ref()?.selection_text()).is_some();
                    self.term_menu = Some(TermMenu { window: self.main_window, pane, at, can_copy });
                }
                Task::none()
            }
        }
    }
}

fn parse(script: &str) -> Result<Vec<Step>, String> {
    script
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|step| {
            let (name, arg) = step.split_once(' ').map_or((step, ""), |(n, a)| (n, a.trim()));
            let send = |m: Message| Ok(Step::Send(vec![m]));
            let action = |a| send(Message::Action(a));
            let editor = |m| send(Message::Editor(m));
            let err = || step.to_owned();
            match name {
                "wait" => arg.parse::<f64>().map(|s| Step::Wait(Duration::from_secs_f64(s))).map_err(|_| err()),
                "shot" if !arg.is_empty() => Ok(Step::Shot(PathBuf::from(arg))),
                "quit" => action(Action::Quit),
                "cancel" => send(Message::Modal(ModalMsg::Cancel)),
                "discard" => send(Message::Modal(ModalMsg::Discard)),
                "workspace" => match arg.parse::<usize>() {
                    Ok(n) if n > 0 => action(Action::SelectWorkspace(n - 1)),
                    _ => Err(err()),
                },
                "mode" => match arg {
                    "editor" => send(Message::SetMode(Mode::Editor)),
                    "agents" => send(Message::SetMode(Mode::Agents)),
                    _ => Err(err()),
                },
                "drawer" => action(Action::ToggleSidebar),
                "system" => send(Message::ToggleSystem),
                "shortcuts" => action(Action::ShowShortcuts),
                "attend" => action(Action::NextAttention),
                "open" if !arg.is_empty() => send(Message::Dev(DevMsg::Open(PathBuf::from(arg)))),
                "click" if !arg.is_empty() => send(Message::Explorer(ExplorerMsg::Clicked(PathBuf::from(arg)))),
                "split" => action(Action::Split(Axis::Vertical)),
                "terminal" => action(Action::ToggleTerminal),
                "explorer" => action(Action::ToggleExplorer),
                "save" => action(Action::Save),
                "replace" => action(Action::Replace),
                "find" => Ok(Step::Send(vec![
                    Message::Action(Action::Find),
                    Message::Editor(EditorMsg::FindQuery(arg.to_owned())),
                ])),
                "quick-open" => Ok(Step::Send(vec![
                    Message::Action(Action::QuickOpen),
                    Message::Editor(EditorMsg::QuickQuery(arg.to_owned())),
                ])),
                "type" if !arg.is_empty() => Ok(Step::Send(
                    unescape(arg)
                        .chars()
                        .map(|c| Message::Editor(if c == '\n' { EditorMsg::Newline } else { EditorMsg::Type(c) }))
                        .collect(),
                )),
                "term" if !arg.is_empty() => send(Message::Dev(DevMsg::Term(unescape(arg)))),
                "key" => key(arg).map(|(k, m)| Step::Send(vec![Message::Dev(DevMsg::Key(k, m))])).ok_or_else(err),
                "escape" => editor(EditorMsg::Escape),
                "panic" => send(Message::Dev(DevMsg::Panic)),
                "release" => send(Message::DividerReleased),
                "divider" => arg.parse().map(|r| Step::Send(vec![Message::Dev(DevMsg::Divider(r))])).map_err(|_| err()),
                "menu" => match arg {
                    "copy" => send(Message::TermMenu(MenuMsg::Copy)),
                    "paste" => send(Message::TermMenu(MenuMsg::Paste)),
                    "select-all" => send(Message::TermMenu(MenuMsg::SelectAll)),
                    "close" => send(Message::TermMenu(MenuMsg::Close)),
                    _ => match arg.split_once(' ').map(|(x, y)| (x.trim().parse::<f32>(), y.trim().parse::<f32>())) {
                        Some((Ok(x), Ok(y))) => send(Message::Dev(DevMsg::Menu(Point::new(x, y)))),
                        _ => Err(err()),
                    },
                },
                _ => Err(err()),
            }
        })
        .collect()
}

/// `\n` and `\t` in script text.
fn unescape(text: &str) -> String {
    text.replace("\\n", "\n").replace("\\t", "\t")
}

/// A key combination such as `ctrl+shift+m`, `escape` or `alt+up`.
fn key(combo: &str) -> Option<(Key, Modifiers)> {
    let mut modifiers = Modifiers::empty();
    let mut parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let last = parts.pop().filter(|k| !k.is_empty())?;
    for part in parts {
        modifiers |= match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Modifiers::CTRL,
            "shift" => Modifiers::SHIFT,
            "alt" => Modifiers::ALT,
            "cmd" | "super" | "logo" => Modifiers::LOGO,
            _ => return None,
        };
    }
    let named = match last.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(Named::Enter),
        "escape" | "esc" => Some(Named::Escape),
        "tab" => Some(Named::Tab),
        "backspace" => Some(Named::Backspace),
        "delete" => Some(Named::Delete),
        "space" => Some(Named::Space),
        "up" => Some(Named::ArrowUp),
        "down" => Some(Named::ArrowDown),
        "left" => Some(Named::ArrowLeft),
        "right" => Some(Named::ArrowRight),
        "home" => Some(Named::Home),
        "end" => Some(Named::End),
        "pageup" => Some(Named::PageUp),
        "pagedown" => Some(Named::PageDown),
        "f2" => Some(Named::F2),
        "f3" => Some(Named::F3),
        _ => None,
    };
    // Characters that can't be written in a combo, or are awkward in a script.
    let character = match last.to_ascii_lowercase().as_str() {
        "backslash" => Some("\\"),
        "slash" => Some("/"),
        "plus" => Some("+"),
        "minus" => Some("-"),
        "comma" => Some(","),
        "semicolon" => Some(";"),
        _ => None,
    };
    let key = match (named, character) {
        (Some(named), _) => Key::Named(named),
        (None, Some(c)) => Key::Character(c.into()),
        (None, None) if last.chars().count() == 1 => {
            // Shifted letters arrive upper-case, as from a real keyboard.
            let c = if modifiers.shift() { last.to_uppercase() } else { last.to_lowercase() };
            Key::Character(c.into())
        }
        (None, None) => return None,
    };
    Some((key, modifiers))
}

fn save_png(path: &Path, shot: &window::Screenshot) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&shot.rgba)).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_script() {
        let steps = parse("wait 1.5; shot /tmp/a.png ;; workspace 2; mode editor; type a\\nb; quit").unwrap();
        assert_eq!(steps.len(), 6);
        assert!(matches!(steps[0], Step::Wait(d) if d == Duration::from_millis(1500)));
        assert!(matches!(&steps[1], Step::Shot(p) if p == Path::new("/tmp/a.png")));
        assert!(matches!(&steps[4], Step::Send(m) if m.len() == 3));
    }

    #[test]
    fn rejects_unknown_steps() {
        assert_eq!(parse("wait 1; dance").err().as_deref(), Some("dance"));
        assert!(parse("workspace 0").is_err());
        assert!(parse("shot").is_err());
        assert!(parse("mode sideways").is_err());
        assert!(parse("key ctrl+").is_err());
    }

    #[test]
    fn parses_key_combos() {
        let (key, mods) = key("ctrl+shift+m").unwrap();
        assert_eq!(key, Key::Character("M".into()));
        assert_eq!(mods, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(super::key("escape").unwrap().0, Key::Named(Named::Escape));
        assert_eq!(super::key("alt+up").unwrap(), (Key::Named(Named::ArrowUp), Modifiers::ALT));
        assert_eq!(super::key("ctrl+backslash").unwrap().0, Key::Character("\\".into()));
        assert!(super::key("hyper+x").is_none());
    }
}
