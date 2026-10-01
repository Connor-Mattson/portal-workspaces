//! Desktop notifications, for when a terminal needs you and the app isn't focused (ADR 0012).
//!
//! There's one notification, replaced in place, rather than a pile: it always says what's newest.
//! Posting talks to D-Bus (or the macOS notification center), so it happens on its own thread. On
//! Linux that thread then waits for a click, which jumps to the terminal; at most one waits.

use iced::Subscription;
use iced::futures::channel::mpsc::UnboundedSender;
use pw_model::PaneId;

use crate::inbox::{self, Inbox};

/// What became of the notification.
#[derive(Debug, Clone)]
pub enum NoteEvent {
    /// It's on screen under this id (later posts replace it).
    Shown(u32),
    /// The user clicked it.
    Clicked,
    /// It was dismissed or expired, or the server is gone.
    Closed,
}

pub struct Notifier {
    /// The notification on screen, if any.
    id: Option<u32>,
    /// A thread is waiting on it for a click.
    waiting: bool,
    /// The terminal the last notification was about.
    pub target: Option<PaneId>,
    tx: UnboundedSender<NoteEvent>,
    inbox: Inbox<NoteEvent>,
}

impl Default for Notifier {
    fn default() -> Self {
        let (tx, inbox) = inbox::channel("pw-notifications");
        #[cfg(target_os = "macos")]
        {
            // Unbundled builds (cargo run) have no identifier of their own; the default is used then.
            let _ = notify_rust::set_application("dev.portal.workspaces");
        }
        Self { id: None, waiting: false, target: None, tx, inbox }
    }
}

impl Notifier {
    pub fn events(&self) -> Subscription<NoteEvent> {
        self.inbox.subscription()
    }

    /// Shows `summary` / `body` about `pane`, replacing the notification already up.
    pub fn post(&mut self, pane: PaneId, summary: String, body: String) {
        self.target = Some(pane);
        let id = self.id;
        let wait = cfg!(not(target_os = "macos")) && !self.waiting;
        self.waiting |= wait;
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("pw-notify".into()).spawn(move || {
            let send = |event| {
                let _ = tx.unbounded_send(event);
            };
            match show(id, &summary, &body, wait, &send) {
                Ok(()) => {}
                Err(err) => {
                    tracing::warn!(%err, "desktop notification failed");
                    if wait {
                        send(NoteEvent::Closed);
                    }
                }
            }
        });
        if spawned.is_err() {
            self.waiting &= !wait;
        }
    }

    pub fn on_event(&mut self, event: &NoteEvent) {
        match event {
            NoteEvent::Shown(id) => self.id = Some(*id),
            NoteEvent::Clicked | NoteEvent::Closed => {
                self.id = None;
                self.waiting = false;
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn show(
    id: Option<u32>,
    summary: &str,
    body: &str,
    wait: bool,
    send: &impl Fn(NoteEvent),
) -> Result<(), notify_rust::error::Error> {
    let mut note = notify_rust::Notification::new();
    note.appname("Portal Workspaces")
        .icon("portal-workspaces")
        .hint(notify_rust::Hint::DesktopEntry("portal-workspaces".into()))
        .hint(notify_rust::Hint::Category("im.received".into()))
        .summary(summary)
        .body(body)
        .action("default", "Show");
    if let Some(id) = id {
        note.id(id);
    }
    let handle = note.show()?;
    send(NoteEvent::Shown(handle.id()));
    if wait {
        handle.wait_for_action(|action| {
            send(if action == "default" { NoteEvent::Clicked } else { NoteEvent::Closed });
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show(
    _id: Option<u32>,
    summary: &str,
    body: &str,
    _wait: bool,
    _send: &impl Fn(NoteEvent),
) -> Result<(), notify_rust::error::Error> {
    // Clicking brings the app forward; the terminal is then one ⌘I away.
    notify_rust::Notification::new().summary(summary).body(body).show().map(drop)
}
