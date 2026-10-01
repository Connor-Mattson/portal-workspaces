//! Terminals that finished or need you: raising, acknowledging, jumping to them and notifying
//! (see `crate::attention` and ADR 0012).

use std::time::Duration;

use iced::{Task, window};
use pw_model::{Mode, PaneId};

use crate::app::{App, Message};
use crate::attention::{self, Attention, Kind, PaneState, Source, Status};
use crate::editor::Focus;
use crate::notifier::NoteEvent;
use crate::workspace::WorkspaceView;

impl App {
    /// Whether you're looking at a terminal: it has the keys in a focused app window.
    fn is_watched(&self, pane: PaneId) -> bool {
        self.focused_window.is_some() && self.key_pane() == Some(pane)
    }

    pub(super) fn on_busy(&mut self, pane: PaneId) {
        let Some(rt) = self.sessions.get_mut(pane) else { return };
        rt.working = true;
        // It picked up again, so "it went quiet" is old news. What it said itself stays.
        if rt.attention.as_ref().is_some_and(|a| a.source == Source::Quiet) {
            rt.attention = None;
        }
    }

    pub(super) fn on_idle(&mut self, pane: PaneId, worked: Duration) -> Task<Message> {
        let Some(rt) = self.sessions.get_mut(pane) else { return Task::none() };
        rt.working = false;
        if rt.exited.is_some() {
            return Task::none();
        }
        let body = format!("Finished after {}", duration(worked));
        self.raise(pane, Kind::Finished, Source::Quiet, None, body)
    }

    pub(super) fn on_notify(&mut self, pane: PaneId, title: Option<String>, body: String) -> Task<Message> {
        let kind = attention::classify(title.as_deref(), &body);
        self.raise(pane, kind, Source::Signal, Some(body.clone()), body)
    }

    pub(super) fn on_bell(&mut self, pane: PaneId) -> Task<Message> {
        self.raise(pane, Kind::NeedsInput, Source::Signal, None, "Needs your input".to_owned())
    }

    /// Marks a terminal you aren't watching, and when the app isn't focused, asks for your attention
    /// (taskbar or dock) and posts a desktop notification saying `body`.
    fn raise(
        &mut self,
        pane: PaneId,
        kind: Kind,
        source: Source,
        message: Option<String>,
        body: String,
    ) -> Task<Message> {
        if self.is_watched(pane) {
            return Task::none();
        }
        self.attention_seq += 1;
        let new = Attention { kind, source, message, seq: self.attention_seq };
        let Some(rt) = self.sessions.get_mut(pane) else { return Task::none() };
        let rose = attention::raise(&mut rt.attention, new);
        rt.cache.clear();
        // A scripted run plays without focus; it mustn't notify you about its own terminals.
        if !rose || self.focused_window.is_some() || crate::devtools::scripted() {
            return Task::none();
        }
        let Some(ws) = self.workspace_of(pane) else { return Task::none() };
        if self.prefs.notifications {
            let summary = format!("{} · {}", ws.model.name, kind.label());
            self.notifier.post(pane, summary, body);
        }
        let window = self.window_of(pane).unwrap_or(self.main_window);
        window::request_user_attention(window, Some(window::UserAttention::Informational))
    }

    /// You're at the terminal with the keys now: whatever it wanted is seen.
    pub(super) fn acknowledge(&mut self) {
        if self.focused_window.is_none() {
            return;
        }
        if let Some(rt) = self.key_pane().and_then(|p| self.sessions.get_mut(p))
            && rt.attention.take().is_some()
        {
            rt.cache.clear();
        }
    }

    // ---- reading -------------------------------------------------------------------------------

    pub(crate) fn status_of(&self, panes: impl IntoIterator<Item = PaneId>) -> Status {
        Status::of(panes.into_iter().filter_map(|pane| {
            let rt = self.sessions.get(pane)?;
            Some(PaneState { pane, working: rt.working, attention: rt.attention.as_ref() })
        }))
    }

    /// Every terminal of a workspace: its grid, detached ones and the editor's.
    pub(crate) fn workspace_status(&self, ws: &WorkspaceView) -> Status {
        self.status_of(ws.all_pane_ids().into_iter().chain([ws.editor_terminal()]))
    }

    /// How many workspaces have a terminal that finished or needs you.
    pub(crate) fn attention_count(&self) -> usize {
        self.workspaces.iter().filter(|ws| self.workspace_status(ws).kind.is_some()).count()
    }

    /// The terminal to go to first: the one that has needed you longest, else the one that finished
    /// first.
    fn next_attention(&self) -> Option<PaneId> {
        self.workspaces
            .iter()
            .filter_map(|ws| {
                let status = self.workspace_status(ws);
                Some((status.kind?, status.first?))
            })
            // The most urgent kind, then the oldest.
            .max_by(|(ka, (sa, _)), (kb, (sb, _))| ka.cmp(kb).then(sb.cmp(sa)))
            .map(|(_, (_, pane))| pane)
    }

    // ---- going there ---------------------------------------------------------------------------

    pub(super) fn jump_to_next(&mut self) -> Task<Message> {
        match self.next_attention() {
            Some(pane) => self.jump_to(pane),
            None => Task::none(),
        }
    }

    /// Opening a workspace that wants something goes straight to the terminal that wants it, unless
    /// the one with the keys there wants something too.
    pub(super) fn select_workspace(&mut self, id: pw_model::WorkspaceId) -> Task<Message> {
        let Some(ws) = self.workspaces.iter().find(|w| w.id() == id) else { return Task::none() };
        let focused_wants = ws.focused.and_then(|p| self.sessions.get(p)).is_some_and(|rt| rt.attention.is_some());
        match self.workspace_status(ws).first {
            Some((_, pane)) if !focused_wants && self.window_of(pane).is_none() => self.jump_to(pane),
            _ if self.active != Some(id) => self.activate(id),
            _ => Task::none(),
        }
    }

    /// Shows a terminal and gives it the keys: its workspace, mode and window, out of a maximized
    /// neighbor's way, with the app brought forward.
    pub(super) fn jump_to(&mut self, pane: PaneId) -> Task<Message> {
        let Some(ws) = self.workspace_of(pane) else { return Task::none() };
        let id = ws.id();
        let mut tasks = Vec::new();
        if self.active != Some(id) {
            tasks.push(self.activate(id));
        }
        if let Some(window) = self.window_of(pane) {
            self.refocus(|app| app.key_window = window);
            tasks.push(window::gain_focus(window));
            return Task::batch(tasks);
        }
        let in_editor = self.active_view().is_some_and(|ws| ws.editor_terminal() == pane);
        let mode = if in_editor { Mode::Editor } else { Mode::Agents };
        tasks.push(self.set_mode(mode));
        if in_editor {
            self.show_editor_terminal();
            tasks.push(self.set_editor_focus(Focus::Terminal));
        } else {
            if let Some(ws) = self.active_mut()
                && ws.maximized().is_some_and(|max| max != pane)
            {
                ws.toggle_maximize(pane);
                self.sessions.clear_all_caches();
            }
            self.set_focus(pane);
        }
        tasks.push(window::gain_focus(self.main_window));
        Task::batch(tasks)
    }

    // ---- desktop notifications -----------------------------------------------------------------

    pub(super) fn on_note(&mut self, event: NoteEvent) -> Task<Message> {
        self.notifier.on_event(&event);
        match (event, self.notifier.target) {
            (NoteEvent::Clicked, Some(pane)) if self.sessions.contains(pane) => self.jump_to(pane),
            (NoteEvent::Clicked, _) => window::gain_focus(self.main_window),
            _ => Task::none(),
        }
    }

    pub(super) fn toggle_notifications(&mut self) {
        self.prefs.notifications = !self.prefs.notifications;
        self.touch();
    }
}

/// "12s", "4m 12s", "1h 3m".
fn duration(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(Duration::from_secs(12)), "12s");
        assert_eq!(duration(Duration::from_secs(252)), "4m 12s");
        assert_eq!(duration(Duration::from_secs(3780)), "1h 3m");
    }
}
