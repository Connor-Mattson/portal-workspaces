//! Running terminals, keyed by pane.
//!
//! Sessions belong to the registry, not to whichever workspace is on screen. Switching
//! workspaces only changes what is drawn, so hidden terminals keep running. A session goes away
//! only when its pane is closed or its workspace deleted.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use iced::Subscription;
use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::widget::canvas;
use pw_model::PaneId;
use pw_term::{GridSize, Session, SessionConfig, TermEvent};

/// Everything the UI tracks about one pane's terminal.
pub struct PaneRuntime {
    pub session: Option<Session>,
    pub spawn_error: Option<String>,
    /// Title set by the running program (OSC 0/2).
    pub title: Option<String>,
    /// `Some(code)` once the shell has exited.
    pub exited: Option<Option<i32>>,
    /// Last known working directory: where it started, then refreshed from the live shell.
    pub cwd: PathBuf,
    /// Output arrived while the pane wasn't on screen.
    pub unseen_output: bool,
    /// The program rang the bell while the pane wasn't on screen.
    pub bell: bool,
    /// Drawn geometry; cleared only when something visible changed.
    pub cache: canvas::Cache,
}

impl PaneRuntime {
    pub fn is_running(&self) -> bool {
        self.session.is_some() && self.exited.is_none()
    }
}

type EventReceiver = UnboundedReceiver<(PaneId, TermEvent)>;

/// Delivers terminal events from every IO thread to the app as one subscription.
#[derive(Clone)]
struct Inbox(Arc<Mutex<Option<EventReceiver>>>);

impl Hash for Inbox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // There is exactly one inbox per app, so a constant identity is right.
        "pw-terminal-events".hash(state);
    }
}

pub struct Sessions {
    panes: HashMap<PaneId, PaneRuntime>,
    tx: UnboundedSender<(PaneId, TermEvent)>,
    inbox: Inbox,
}

impl Default for Sessions {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded();
        Self { panes: HashMap::new(), tx, inbox: Inbox(Arc::new(Mutex::new(Some(rx)))) }
    }
}

impl Sessions {
    /// Terminal events from all sessions.
    pub fn events(&self) -> Subscription<(PaneId, TermEvent)> {
        Subscription::run_with(self.inbox.clone(), |inbox: &Inbox| -> BoxStream<'static, (PaneId, TermEvent)> {
            match inbox.0.lock().expect("inbox lock").take() {
                Some(rx) => rx.boxed(),
                None => stream::empty().boxed(),
            }
        })
    }

    pub fn get(&self, id: PaneId) -> Option<&PaneRuntime> {
        self.panes.get(&id)
    }

    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut PaneRuntime> {
        self.panes.get_mut(&id)
    }

    pub fn contains(&self, id: PaneId) -> bool {
        self.panes.contains_key(&id)
    }

    /// Starts a shell for `id` in `cwd`, falling back to `fallback` and then `$HOME` if `cwd` is
    /// gone. Replaces any previous session for the pane (used for restart).
    pub fn spawn(&mut self, id: PaneId, cwd: &Path, fallback: &Path, size: GridSize) {
        let cwd = [cwd, fallback].into_iter().find(|p| p.is_dir()).map(Path::to_path_buf).unwrap_or_else(home_dir);
        let tx = self.tx.clone();
        let result = Session::spawn(SessionConfig::new(cwd.clone(), size), move |event| {
            let _ = tx.unbounded_send((id, event));
        });
        let (session, spawn_error) = match result {
            Ok(session) => (Some(session), None),
            Err(err) => {
                tracing::error!(pane = %id, %err, "shell failed to start");
                (None, Some(err.to_string()))
            }
        };
        self.panes.insert(
            id,
            PaneRuntime {
                session,
                spawn_error,
                title: None,
                exited: None,
                cwd,
                unseen_output: false,
                bell: false,
                cache: canvas::Cache::new(),
            },
        );
    }

    /// Ends the pane's shell (closing the PTY hangs it up).
    pub fn remove(&mut self, id: PaneId) {
        self.panes.remove(&id);
    }

    /// Refreshes every running pane's cwd from its shell. Returns whether any changed.
    pub fn refresh_cwds(&mut self) -> bool {
        let mut changed = false;
        for rt in self.panes.values_mut() {
            if let Some(cwd) = rt.session.as_ref().filter(|_| rt.exited.is_none()).and_then(Session::current_dir)
                && cwd != rt.cwd
            {
                rt.cwd = cwd;
                changed = true;
            }
        }
        changed
    }

    pub fn clear_all_caches(&self) {
        for rt in self.panes.values() {
            rt.cache.clear();
        }
    }
}

pub fn home_dir() -> PathBuf {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("/"))
}
