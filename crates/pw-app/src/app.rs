//! The application: state, messages, update and subscriptions (Elm architecture).

mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{Key, Modifiers};
use iced::widget::Space;
use iced::widget::pane_grid;
use iced::{Element, Subscription, Task, clipboard, event, time, window};
use pw_model::{Axis, PaneId, PersistedState, Preset, ProfileId, SCHEMA_VERSION, UiPrefs, Workspace, WorkspaceId};
use pw_term::{GridSize, MouseButton, MouseEvent, MouseEventKind, SelectionKind, TermEvent};

use crate::fonts::CellMetrics;
use crate::keymap::{self, Action};
use crate::persist::Saver;
use crate::sessions::Sessions;
use crate::ui;
use crate::ui::modal::{Editor, Modal, ModalMsg};
use crate::ui::profile_editor::{self, ProfileEditor};
use crate::ui::terminal::TermMsg;
use crate::usage::Usage;
use crate::workspace::WorkspaceView;

pub use windows::WindowMsg;

/// How long after the last change state is written to disk.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);
/// How often live shells are asked where they are (for restoring cwds).
const CWD_POLL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum Message {
    /// Output, title, bell or exit from a terminal's IO thread.
    Term(PaneId, TermEvent),
    /// Mouse and size events from a terminal canvas.
    Terminal(PaneId, TermMsg),
    KeyPressed {
        window: window::Id,
        key: Key,
        modifiers: Modifiers,
        text: Option<String>,
    },
    Action(Action),
    SelectWorkspace(WorkspaceId),
    MoveWorkspace(WorkspaceId, isize),
    EditWorkspace(WorkspaceId),
    PaneClicked(pane_grid::Pane),
    PaneDragged(pane_grid::DragEvent),
    PaneResized(pane_grid::ResizeEvent),
    SplitPane(PaneId, Axis),
    ClosePane(PaneId),
    MaximizePane(PaneId),
    RestartPane(PaneId),
    /// Moves a pane into its own window.
    DetachPane(PaneId),
    /// Puts a detached pane back into its workspace's grid.
    DockPane(PaneId),
    NewPane,
    ApplyPreset(Preset),
    Modal(ModalMsg),
    Paste(Option<String>),
    /// A reading from the usage monitor.
    Usage(pw_usage::Update),
    RefreshUsage,
    /// Asks an idle profile's CLI to renew its expired sign-in.
    RenewProfile(ProfileId),
    AddProfile,
    EditProfile(ProfileId),
    ToggleUsage,
    SaveTick,
    CwdTick,
    Window(WindowMsg),
}

pub struct App {
    pub(crate) workspaces: Vec<WorkspaceView>,
    pub(crate) active: Option<WorkspaceId>,
    pub(crate) sessions: Sessions,
    pub(crate) usage: Usage,
    pub(crate) prefs: UiPrefs,
    pub(crate) metrics: CellMetrics,
    pub(crate) modal: Option<Modal>,
    pub(crate) main_window: window::Id,
    /// Detached terminals' windows (see `windows`).
    pub(crate) pane_windows: HashMap<window::Id, PaneId>,
    /// The app window that last had focus. Keys go to its terminal.
    pub(crate) key_window: window::Id,
    dirty: bool,
    saver: Saver,
}

impl App {
    pub fn boot(state: PersistedState, saver: Saver) -> (Self, Task<Message>) {
        let (main_window, opened) = window::open(windows::main_settings(state.ui.window));
        let mut app = Self {
            workspaces: state.workspaces.into_iter().map(WorkspaceView::new).collect(),
            active: None,
            sessions: Sessions::default(),
            usage: Usage::new(state.usage_profiles),
            metrics: CellMetrics::for_size(state.ui.font_size),
            prefs: state.ui,
            modal: None,
            main_window,
            pane_windows: HashMap::new(),
            key_window: main_window,
            dirty: false,
            saver,
        };
        let mut tasks = vec![opened.discard()];
        match state.active {
            // Restored detached windows open after the main one; it keeps the focus.
            Some(id) => tasks.push(app.activate(id).chain(window::gain_focus(main_window))),
            None => {
                app.modal = Some(Modal::Editor(Editor::new_workspace()));
                tasks.push(ui::modal::focus_first_field());
            }
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self, window: window::Id) -> String {
        if let Some(&pane) = self.pane_windows.get(&window) {
            let label = self.sessions.get(pane).map(ui::pane_label).unwrap_or_default();
            return match self.workspace_of(pane) {
                Some(ws) => format!("{label} — {} — Portal Workspaces", ws.model.name),
                None => format!("{label} — Portal Workspaces"),
            };
        }
        match self.active_view() {
            Some(ws) => format!("{} — Portal Workspaces", ws.model.name),
            None => "Portal Workspaces".to_owned(),
        }
    }

    pub fn view(&self, window: window::Id) -> Element<'_, Message> {
        if window == self.main_window {
            ui::view(self)
        } else if let Some(&pane) = self.pane_windows.get(&window) {
            ui::pane_window::view(self, pane)
        } else {
            // A window on its way out (a terminal, just docked).
            Space::new().into()
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            self.sessions.events().map(|(id, event)| Message::Term(id, event)),
            self.usage.events().map(Message::Usage),
            event::listen_with(on_runtime_event),
            time::every(CWD_POLL).map(|_| Message::CwdTick),
        ];
        // The save timer only exists while there is something to save, so an idle app has no
        // timers but the cwd poll.
        if self.dirty {
            subs.push(time::every(SAVE_DEBOUNCE).map(|_| Message::SaveTick));
        }
        Subscription::batch(subs)
    }

    // ---- queries -------------------------------------------------------------------------

    pub(crate) fn active_view(&self) -> Option<&WorkspaceView> {
        self.workspaces.iter().find(|w| Some(w.id()) == self.active)
    }

    fn active_mut(&mut self) -> Option<&mut WorkspaceView> {
        let active = self.active?;
        self.workspaces.iter_mut().find(|w| w.id() == active)
    }

    fn position(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id() == id)
    }

    /// The workspace a pane belongs to, in its grid or detached.
    pub(crate) fn workspace_of(&self, pane: PaneId) -> Option<&WorkspaceView> {
        self.workspaces.iter().find(|ws| ws.is_detached(pane) || ws.handle(pane).is_some())
    }

    /// The main window's focused pane.
    fn focused_pane(&self) -> Option<PaneId> {
        self.active_view()?.focused
    }

    /// The pane that gets the keys: the detached terminal whose window has focus, or the main window's
    /// focused pane.
    fn key_pane(&self) -> Option<PaneId> {
        match self.pane_windows.get(&self.key_window) {
            Some(&pane) => Some(pane),
            None => self.focused_pane(),
        }
    }

    /// Whether a pane is drawn as focused (solid cursor, accent frame).
    pub(crate) fn has_focus(&self, pane: PaneId) -> bool {
        self.key_pane() == Some(pane)
    }

    /// Whether a pane is currently drawn: in the active workspace's grid, or in its own window.
    fn is_visible(&self, pane: PaneId) -> bool {
        self.window_of(pane).is_some()
            || self.active_view().is_some_and(|ws| match ws.maximized() {
                Some(max) => max == pane,
                None => ws.handle(pane).is_some(),
            })
    }

    fn default_grid(&self) -> GridSize {
        GridSize {
            cols: 80,
            rows: 24,
            cell_width: self.metrics.width.round() as u16,
            cell_height: self.metrics.height as u16,
        }
    }

    // ---- state changes -------------------------------------------------------------------

    fn touch(&mut self) {
        self.dirty = true;
    }

    /// Runs `change`, then tells the terminals that lost and gained the keys.
    fn refocus(&mut self, change: impl FnOnce(&mut Self)) {
        let before = self.key_pane();
        change(self);
        let after = self.key_pane();
        if before == after {
            return;
        }
        for (pane, focused) in [(before, false), (after, true)] {
            if let Some(rt) = pane.and_then(|p| self.sessions.get(p)) {
                rt.cache.clear();
                if let Some(session) = &rt.session {
                    session.set_focused(focused);
                }
            }
        }
    }

    /// Shows a workspace, starting its shells (and opening its detached terminals' windows) the first
    /// time.
    fn activate(&mut self, id: WorkspaceId) -> Task<Message> {
        self.refocus(|app| app.active = Some(id));
        let size = self.default_grid();
        let Some(ws) = self.workspaces.iter_mut().find(|w| w.id() == id) else { return Task::none() };
        ws.spawned = true;
        let root = ws.model.root.clone();
        let panes: Vec<(PaneId, PathBuf)> = ws
            .all_pane_ids()
            .into_iter()
            .map(|p| (p, ws.model.panes.get(&p).map_or_else(|| root.clone(), |s| s.cwd.clone())))
            .collect();
        let detached = ws.detached.clone();
        let key_pane = self.key_pane();
        for (pane, cwd) in panes {
            if !self.sessions.contains(pane) {
                self.sessions.spawn(pane, &cwd, &root, size);
                if let Some(session) = self.sessions.get(pane).and_then(|rt| rt.session.as_ref()) {
                    session.set_focused(Some(pane) == key_pane);
                }
            }
            let rt = self.sessions.get_mut(pane).expect("just ensured");
            rt.unseen_output = false;
            rt.bell = false;
            // Hidden panes kept running without redrawing; their caches are stale.
            rt.cache.clear();
        }
        self.touch();
        let closed: Vec<PaneId> = detached.into_iter().filter(|p| self.window_of(*p).is_none()).collect();
        Task::batch(closed.into_iter().map(|p| self.open_window(p)).collect::<Vec<_>>())
    }

    /// Gives a pane the keys: focuses it in the grid, or makes its window the key window.
    fn set_focus(&mut self, pane: PaneId) {
        let window = self.window_of(pane).unwrap_or(self.main_window);
        let moved = window == self.main_window && self.focused_pane() != Some(pane);
        self.refocus(|app| {
            app.key_window = window;
            if let Some(ws) = app.active_mut().filter(|_| moved) {
                ws.focused = Some(pane);
            }
        });
        if moved {
            self.touch();
        }
    }

    fn split(&mut self, target: Option<PaneId>, axis: Axis) {
        let size = self.default_grid();
        let Some(ws) = self.active_mut() else { return };
        let root = ws.model.root.clone();
        let source = target.or(ws.focused);
        let new = PaneId::new();
        if ws.split(source, axis, new).is_err() {
            return;
        }
        // A new pane opens where the pane it was split from currently is.
        let cwd = source.and_then(|s| self.sessions.get(s)).map_or_else(|| root.clone(), |rt| rt.cwd.clone());
        self.sessions.spawn(new, &cwd, &root, size);
        self.set_focus(new);
        self.sessions.clear_all_caches();
        self.touch();
    }

    /// Closes a pane in the grid or in its own window, ending its shell.
    fn close_pane(&mut self, pane: PaneId) -> Task<Message> {
        let window = self.window_of(pane);
        if let Some(id) = window {
            self.forget_window(id);
        }
        self.refocus(|app| {
            if let Some(ws) = app.workspaces.iter_mut().find(|ws| ws.is_detached(pane) || ws.handle(pane).is_some()) {
                ws.detached.retain(|p| *p != pane);
                ws.close(pane);
            }
            app.sessions.remove(pane);
        });
        self.touch();
        window.map_or_else(Task::none, window::close)
    }

    fn apply_preset(&mut self, preset: Preset) {
        let size = self.default_grid();
        let Some(ws) = self.active_mut().filter(|ws| ws.fits(preset)) else { return };
        let root = ws.model.root.clone();
        let (created, dropped) = ws.apply_preset(preset);
        for pane in dropped {
            self.sessions.remove(pane);
        }
        for pane in created {
            self.sessions.spawn(pane, &root, &root, size);
        }
        self.sessions.clear_all_caches();
        self.touch();
    }

    fn delete_workspace(&mut self, id: WorkspaceId) -> Task<Message> {
        let Some(index) = self.position(id) else { return Task::none() };
        let mut tasks = Vec::new();
        for pane in self.workspaces[index].detached.clone() {
            if let Some(window) = self.window_of(pane) {
                self.forget_window(window);
                tasks.push(window::close(window));
            }
        }
        let ws = self.workspaces.remove(index);
        for pane in ws.all_pane_ids() {
            self.sessions.remove(pane);
        }
        if self.active == Some(id) {
            self.active = None;
            let next = self.workspaces.get(index.min(self.workspaces.len().saturating_sub(1))).map(WorkspaceView::id);
            if let Some(next) = next {
                tasks.push(self.activate(next));
            }
        }
        self.touch();
        Task::batch(tasks)
    }

    fn set_font_size(&mut self, size: f32) {
        let size = size.clamp(UiPrefs::MIN_FONT, UiPrefs::MAX_FONT);
        self.prefs.font_size = size;
        self.metrics = CellMetrics::for_size(size);
        self.sessions.clear_all_caches();
        self.touch();
    }

    fn persisted(&self) -> PersistedState {
        PersistedState {
            schema_version: SCHEMA_VERSION,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| ws.to_model(|pane| self.sessions.get(pane).map(|rt| rt.cwd.clone())))
                .collect(),
            active: self.active,
            ui: self.prefs.clone(),
            usage_profiles: self.usage.profiles().to_vec(),
        }
    }

    // ---- update --------------------------------------------------------------------------

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Term(pane, event) => return self.on_term_event(pane, event),
            Message::Terminal(pane, msg) => return self.on_terminal(pane, msg),
            Message::KeyPressed { window, key, modifiers, text } => return self.on_key(window, key, modifiers, text),
            Message::Action(action) => return self.perform(action),
            Message::SelectWorkspace(id) => {
                if self.active != Some(id) {
                    return self.activate(id);
                }
            }
            Message::MoveWorkspace(id, delta) => {
                if let Some(from) = self.position(id) {
                    let to = from.saturating_add_signed(delta).min(self.workspaces.len() - 1);
                    let ws = self.workspaces.remove(from);
                    self.workspaces.insert(to, ws);
                    self.touch();
                }
            }
            Message::EditWorkspace(id) => {
                if let Some(ws) = self.workspaces.iter().find(|w| w.id() == id) {
                    self.modal = Some(Modal::Editor(Editor::edit(&ws.model)));
                    return Task::batch([self.raise_main(), ui::modal::focus_first_field()]);
                }
            }
            Message::PaneClicked(handle) => {
                let pane = self.active_view().and_then(|ws| ws.grid.as_ref()?.get(handle).copied());
                if let Some(pane) = pane {
                    self.set_focus(pane);
                }
            }
            Message::PaneDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                if let Some(grid) = self.active_mut().and_then(|ws| ws.grid.as_mut()) {
                    grid.drop(pane, target);
                    self.sessions.clear_all_caches();
                    self.touch();
                }
            }
            Message::PaneDragged(_) => {}
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                if let Some(grid) = self.active_mut().and_then(|ws| ws.grid.as_mut()) {
                    grid.resize(split, ratio);
                    self.touch();
                }
            }
            Message::SplitPane(pane, axis) => self.split(Some(pane), axis),
            Message::ClosePane(pane) => return self.close_pane(pane),
            Message::MaximizePane(pane) => {
                if let Some(ws) = self.active_mut() {
                    ws.toggle_maximize(pane);
                }
                self.set_focus(pane);
                self.sessions.clear_all_caches();
            }
            Message::RestartPane(pane) => self.restart(pane),
            Message::DetachPane(pane) => return self.detach(pane),
            Message::DockPane(pane) => return self.dock(pane),
            Message::NewPane => self.split(None, Axis::Vertical),
            Message::ApplyPreset(preset) => {
                let Some(ws) = self.active_view().filter(|ws| ws.fits(preset)) else { return Task::none() };
                let closing = ws.grid_len().saturating_sub(preset.pane_count());
                let running = ws
                    .pane_ids()
                    .iter()
                    .skip(preset.pane_count())
                    .any(|p| self.sessions.get(*p).is_some_and(|rt| rt.is_running()));
                if running {
                    self.modal = Some(Modal::ConfirmPreset { workspace: ws.id(), preset, closing });
                } else {
                    self.apply_preset(preset);
                }
            }
            Message::Modal(msg) => return self.on_modal(msg),
            Message::Paste(Some(text)) => {
                if let Some(session) =
                    self.key_pane().and_then(|p| self.sessions.get(p)).and_then(|rt| rt.session.as_ref())
                {
                    session.paste(&text);
                }
            }
            Message::Paste(None) => {}
            Message::Usage(update) => self.usage.apply(update),
            Message::RefreshUsage => self.usage.refresh(),
            Message::RenewProfile(id) => self.usage.renew(id),
            Message::AddProfile => {
                self.modal = Some(Modal::Profile(ProfileEditor::new(self.usage.profiles())));
                return profile_editor::focus();
            }
            Message::EditProfile(id) => {
                if let Some(profile) = self.usage.profile(id) {
                    self.modal = Some(Modal::Profile(ProfileEditor::edit(profile, self.usage.profiles())));
                    return profile_editor::focus();
                }
            }
            Message::ToggleUsage => {
                self.prefs.usage_expanded = !self.prefs.usage_expanded;
                self.touch();
            }
            Message::SaveTick => {
                self.saver.save(self.persisted());
                self.dirty = false;
            }
            Message::CwdTick => {
                if self.sessions.refresh_cwds() {
                    self.touch();
                }
            }
            Message::Window(msg) => return self.on_window(msg),
        }
        Task::none()
    }

    /// Saves and exits. A daemon doesn't exit when its windows close, so this is the only way out.
    fn quit(&mut self) -> Task<Message> {
        self.sessions.refresh_cwds();
        self.saver.save_and_wait(self.persisted());
        iced::exit()
    }

    fn restart(&mut self, pane: PaneId) {
        let size = self.default_grid();
        let Some(ws) = self.workspace_of(pane) else { return };
        let root = ws.model.root.clone();
        let cwd = self.sessions.get(pane).map_or_else(|| root.clone(), |rt| rt.cwd.clone());
        self.sessions.spawn(pane, &cwd, &root, size);
        if let Some(session) = self.sessions.get(pane).and_then(|rt| rt.session.as_ref()) {
            session.set_focused(self.has_focus(pane));
        }
    }

    fn on_term_event(&mut self, pane: PaneId, event: TermEvent) -> Task<Message> {
        let visible = self.is_visible(pane);
        let Some(rt) = self.sessions.get_mut(pane) else { return Task::none() };
        match event {
            TermEvent::Wakeup => {
                if visible {
                    rt.cache.clear();
                } else {
                    rt.unseen_output = true;
                }
            }
            TermEvent::Title(title) => rt.title = Some(title).filter(|t| !t.trim().is_empty()),
            TermEvent::ResetTitle => rt.title = None,
            TermEvent::Bell => {
                if !visible {
                    rt.bell = true;
                }
            }
            TermEvent::Exited(code) => {
                rt.exited = Some(code);
                rt.cache.clear();
            }
            TermEvent::ClipboardStore(text) => return clipboard::write(text),
        }
        Task::none()
    }

    fn on_terminal(&mut self, pane: PaneId, msg: TermMsg) -> Task<Message> {
        if let TermMsg::MouseDown { .. } = msg {
            self.set_focus(pane);
        }
        let Some(rt) = self.sessions.get_mut(pane) else { return Task::none() };
        let Some(session) = rt.session.as_mut() else { return Task::none() };
        // Shift overrides mouse reporting so you can always select text, as in other terminals.
        let report = |mods: pw_term::Mods| session.wants_mouse() && !mods.shift;
        let mut task = Task::none();
        match msg {
            TermMsg::Resize(size) => session.resize(size),
            TermMsg::MouseDown { point, right_half, button, clicks, mods } => {
                if report(mods) {
                    session.report_mouse(MouseEvent { kind: MouseEventKind::Press(button), point, mods });
                } else if button == MouseButton::Left {
                    let kind = match clicks {
                        1 => SelectionKind::Simple,
                        2 => SelectionKind::Word,
                        _ => SelectionKind::Line,
                    };
                    session.start_selection(kind, point, right_half);
                } else if button == MouseButton::Middle {
                    task = clipboard::read_primary().map(Message::Paste);
                }
            }
            TermMsg::MouseDrag { point, right_half, button, mods } => {
                if report(mods) {
                    session.report_mouse(MouseEvent { kind: MouseEventKind::Drag(button), point, mods });
                } else if button == MouseButton::Left {
                    session.update_selection(point, right_half);
                }
            }
            TermMsg::MouseUp { point, button, mods } => {
                if report(mods) {
                    session.report_mouse(MouseEvent { kind: MouseEventKind::Release(button), point, mods });
                } else if button == MouseButton::Left
                    && let Some(text) = session.selection_text()
                {
                    // X11/Wayland convention: selecting copies to the primary selection.
                    task = clipboard::write_primary(text);
                }
            }
            TermMsg::Wheel { lines, point, mods } => session.wheel(lines, point, mods),
        }
        rt.cache.clear();
        task
    }

    fn on_key(&mut self, window: window::Id, key: Key, modifiers: Modifiers, text: Option<String>) -> Task<Message> {
        // Keys come from the focused window, which normally already is the key window.
        if window != self.key_window && (window == self.main_window || self.pane_windows.contains_key(&window)) {
            self.refocus(|app| app.key_window = window);
        }
        if let Some(action) = keymap::action(&key, modifiers) {
            return self.perform(action);
        }
        // A sheet in the main window doesn't stop the detached terminals.
        if self.modal.is_some() && window == self.main_window {
            return match key {
                Key::Named(iced::keyboard::key::Named::Escape) => self.on_modal(ModalMsg::Cancel),
                Key::Named(iced::keyboard::key::Named::Enter) => self.on_modal(ModalMsg::Submit),
                _ => Task::none(),
            };
        }
        let Some(pane) = self.key_pane() else { return Task::none() };
        let Some(rt) = self.sessions.get(pane) else { return Task::none() };
        if rt.exited.is_some() || rt.session.is_none() {
            if key == Key::Named(iced::keyboard::key::Named::Enter) {
                self.restart(pane);
            }
            return Task::none();
        }
        if let (Some(session), Some(input)) = (&rt.session, keymap::to_terminal(&key, modifiers, text.as_deref()))
            && session.send_key(&input)
        {
            rt.cache.clear();
        }
        Task::none()
    }

    fn perform(&mut self, action: Action) -> Task<Message> {
        let in_main = self.key_window == self.main_window;
        let allowed = match action {
            Action::Quit | Action::ShowShortcuts => true,
            // A detached terminal keeps working under a sheet in the main window.
            Action::ClosePane
            | Action::ToggleDetach
            | Action::Copy
            | Action::Paste
            | Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::FontBigger
            | Action::FontSmaller
            | Action::FontReset => self.modal.is_none() || !in_main,
            // Grid actions mean nothing in a detached terminal's window.
            Action::Split(_) | Action::Focus(_) | Action::ToggleMaximize => self.modal.is_none() && in_main,
            _ => self.modal.is_none(),
        };
        if !allowed {
            return Task::none();
        }
        match action {
            Action::NewWorkspace => {
                self.modal = Some(Modal::Editor(Editor::new_workspace()));
                return Task::batch([self.raise_main(), ui::modal::focus_first_field()]);
            }
            Action::EditWorkspace => {
                if let Some(id) = self.active {
                    return self.update(Message::EditWorkspace(id));
                }
            }
            Action::SelectWorkspace(index) => {
                if let Some(id) = self.workspaces.get(index).map(WorkspaceView::id) {
                    return self.update(Message::SelectWorkspace(id));
                }
            }
            Action::NextWorkspace | Action::PrevWorkspace => {
                let n = self.workspaces.len();
                if n > 0 {
                    let current = self.active.and_then(|a| self.position(a)).unwrap_or(0);
                    let next = if action == Action::NextWorkspace { (current + 1) % n } else { (current + n - 1) % n };
                    let id = self.workspaces[next].id();
                    return self.update(Message::SelectWorkspace(id));
                }
            }
            Action::ToggleSidebar => {
                self.prefs.sidebar_collapsed = !self.prefs.sidebar_collapsed;
                self.touch();
            }
            Action::Split(axis) => self.split(None, axis),
            Action::ClosePane => {
                if let Some(pane) = self.key_pane() {
                    return self.close_pane(pane);
                }
            }
            Action::ToggleDetach => match self.key_pane() {
                Some(pane) if !in_main => return self.dock(pane),
                Some(pane) => return self.detach(pane),
                None => {}
            },
            Action::Focus(direction) => {
                if let Some(pane) = self.active_view().and_then(|ws| ws.adjacent(direction)) {
                    self.set_focus(pane);
                }
            }
            Action::ToggleMaximize => {
                if let Some(pane) = self.focused_pane() {
                    return self.update(Message::MaximizePane(pane));
                }
            }
            Action::Copy => {
                let text = self.key_pane().and_then(|p| self.sessions.get(p)?.session.as_ref()?.selection_text());
                if let Some(text) = text {
                    return clipboard::write(text);
                }
            }
            Action::Paste => return clipboard::read().map(Message::Paste),
            Action::FontBigger => self.set_font_size(self.prefs.font_size + 1.0),
            Action::FontSmaller => self.set_font_size(self.prefs.font_size - 1.0),
            Action::FontReset => self.set_font_size(UiPrefs::default().font_size),
            Action::ScrollPageUp | Action::ScrollPageDown => {
                if let Some(rt) = self.key_pane().and_then(|p| self.sessions.get(p)) {
                    if let Some(session) = &rt.session {
                        session.scroll_page(action == Action::ScrollPageUp);
                    }
                    rt.cache.clear();
                }
            }
            Action::ShowShortcuts => {
                self.modal = match self.modal {
                    Some(Modal::Shortcuts) => None,
                    _ => Some(Modal::Shortcuts),
                };
                return self.raise_main();
            }
            Action::Quit => return self.quit(),
        }
        Task::none()
    }

    fn on_modal(&mut self, msg: ModalMsg) -> Task<Message> {
        match (&mut self.modal, msg) {
            (modal, ModalMsg::Cancel) => {
                // The very first run has nothing behind the workspace sheet; keep it up.
                if !self.workspaces.is_empty() || !matches!(modal, Some(Modal::Editor(_))) {
                    self.modal = None;
                }
            }
            (Some(Modal::Profile(editor)), ModalMsg::Profile(msg)) => editor.update(msg),
            (Some(Modal::Profile(editor)), ModalMsg::Submit) => match editor.validate() {
                Ok(profile) => {
                    self.modal = None;
                    self.usage.upsert(profile);
                    self.touch();
                }
                Err(error) => editor.error = Some(error),
            },
            (Some(Modal::Profile(editor)), ModalMsg::Delete) => {
                let id = editor.id;
                self.modal = None;
                self.usage.remove(id);
                self.touch();
            }
            (Some(Modal::Editor(editor)), ModalMsg::Name(name)) => editor.name = name,
            (Some(Modal::Editor(editor)), ModalMsg::Root(root)) => {
                editor.root = root;
                editor.error = None;
            }
            (Some(Modal::Editor(editor)), ModalMsg::Preset(preset)) => editor.preset = preset,
            (Some(Modal::Editor(editor)), ModalMsg::Browse) => {
                let start = editor.resolved_root().filter(|p| p.is_dir()).unwrap_or_else(crate::sessions::home_dir);
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose a project folder")
                            .set_directory(start)
                            .pick_folder()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    |path| Message::Modal(ModalMsg::Browsed(path)),
                );
            }
            (Some(Modal::Editor(editor)), ModalMsg::Browsed(Some(path))) => editor.set_root(path),
            (Some(Modal::Editor(editor)), ModalMsg::Submit) => match editor.validate() {
                Ok((name, root)) => {
                    let (target, preset) = (editor.target, editor.preset);
                    self.modal = None;
                    match target.and_then(|id| self.workspaces.iter_mut().find(|w| w.id() == id)) {
                        Some(ws) => {
                            ws.model.name = name;
                            ws.model.root = root;
                        }
                        None => {
                            let ws = WorkspaceView::new(Workspace::new(name, root, preset));
                            let id = ws.id();
                            self.workspaces.push(ws);
                            self.touch();
                            return self.activate(id);
                        }
                    }
                    self.touch();
                }
                Err(error) => editor.error = Some(error),
            },
            (Some(Modal::Editor(editor)), ModalMsg::Delete) => {
                if let Some(id) = editor.target {
                    self.modal = Some(Modal::ConfirmDelete(id));
                }
            }
            (Some(Modal::ConfirmDelete(id)), ModalMsg::Submit) => {
                let id = *id;
                self.modal = None;
                let closed = self.delete_workspace(id);
                if self.workspaces.is_empty() {
                    self.modal = Some(Modal::Editor(Editor::new_workspace()));
                    return Task::batch([closed, ui::modal::focus_first_field()]);
                }
                return closed;
            }
            (Some(Modal::ConfirmPreset { workspace, preset, .. }), ModalMsg::Submit) => {
                let (workspace, preset) = (*workspace, *preset);
                self.modal = None;
                if self.active == Some(workspace) {
                    self.apply_preset(preset);
                }
            }
            (Some(Modal::Shortcuts), ModalMsg::Submit) => self.modal = None,
            _ => {}
        }
        Task::none()
    }
}

/// Global events: keys no widget used, and focus, resizes and close requests from every window.
fn on_runtime_event(event: iced::Event, status: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, text, .. })
            if status == event::Status::Ignored =>
        {
            Some(Message::KeyPressed { window: id, key, modifiers, text: text.map(|t| t.to_string()) })
        }
        iced::Event::Window(event) => match event {
            window::Event::Focused => Some(WindowMsg::Focused(id)),
            window::Event::Resized(size) => Some(WindowMsg::Resized(id, size)),
            window::Event::CloseRequested => Some(WindowMsg::CloseRequested(id)),
            window::Event::Closed => Some(WindowMsg::Closed(id)),
            _ => None,
        }
        .map(Message::Window),
        _ => None,
    }
}
