//! The application: state, messages, update and subscriptions (Elm architecture).

use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{Key, Modifiers};
use iced::widget::pane_grid;
use iced::{Element, Size, Subscription, Task, clipboard, event, time, window};
use pw_model::{Axis, PaneId, PersistedState, Preset, SCHEMA_VERSION, UiPrefs, WindowGeometry, Workspace, WorkspaceId};
use pw_term::{GridSize, MouseButton, MouseEvent, MouseEventKind, SelectionKind, TermEvent};

use crate::fonts::CellMetrics;
use crate::keymap::{self, Action};
use crate::persist::Saver;
use crate::sessions::Sessions;
use crate::ui;
use crate::ui::modal::{Editor, Modal, ModalMsg};
use crate::ui::terminal::TermMsg;
use crate::workspace::WorkspaceView;

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
    NewPane,
    ApplyPreset(Preset),
    Modal(ModalMsg),
    Paste(Option<String>),
    SaveTick,
    CwdTick,
    WindowResized(Size),
    CloseRequested(window::Id),
}

pub struct App {
    pub(crate) workspaces: Vec<WorkspaceView>,
    pub(crate) active: Option<WorkspaceId>,
    pub(crate) sessions: Sessions,
    pub(crate) prefs: UiPrefs,
    pub(crate) metrics: CellMetrics,
    pub(crate) modal: Option<Modal>,
    dirty: bool,
    saver: Saver,
}

impl App {
    pub fn boot(state: PersistedState, saver: Saver) -> (Self, Task<Message>) {
        let mut app = Self {
            workspaces: state.workspaces.into_iter().map(WorkspaceView::new).collect(),
            active: None,
            sessions: Sessions::default(),
            metrics: CellMetrics::for_size(state.ui.font_size),
            prefs: state.ui,
            modal: None,
            dirty: false,
            saver,
        };
        match state.active {
            Some(id) => app.activate(id),
            None => app.modal = Some(Modal::Editor(Editor::new_workspace())),
        }
        let task = if app.modal.is_some() { ui::modal::focus_first_field() } else { Task::none() };
        (app, task)
    }

    pub fn title(&self) -> String {
        match self.active_view() {
            Some(ws) => format!("{} — Portal Workspaces", ws.model.name),
            None => "Portal Workspaces".to_owned(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        ui::view(self)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            self.sessions.events().map(|(id, event)| Message::Term(id, event)),
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

    fn focused_pane(&self) -> Option<PaneId> {
        self.active_view()?.focused
    }

    /// Whether a pane is currently drawn.
    fn is_visible(&self, pane: PaneId) -> bool {
        self.active_view().is_some_and(|ws| match ws.maximized() {
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

    /// Shows a workspace, starting its shells the first time.
    fn activate(&mut self, id: WorkspaceId) {
        if let Some(old) = self.focused_pane().and_then(|p| self.sessions.get(p)).and_then(|rt| rt.session.as_ref()) {
            old.set_focused(false);
        }
        self.active = Some(id);
        let size = self.default_grid();
        let Some(ws) = self.workspaces.iter_mut().find(|w| w.id() == id) else { return };
        ws.spawned = true;
        let root = ws.model.root.clone();
        let panes: Vec<(PaneId, PathBuf)> = ws
            .pane_ids()
            .into_iter()
            .map(|p| (p, ws.model.panes.get(&p).map_or_else(|| root.clone(), |s| s.cwd.clone())))
            .collect();
        let focused = ws.focused;
        for (pane, cwd) in panes {
            if !self.sessions.contains(pane) {
                self.sessions.spawn(pane, &cwd, &root, size);
            }
            let rt = self.sessions.get_mut(pane).expect("just ensured");
            rt.unseen_output = false;
            rt.bell = false;
            // Hidden panes kept running without redrawing; their caches are stale.
            rt.cache.clear();
            if let Some(session) = &rt.session {
                session.set_focused(Some(pane) == focused);
            }
        }
        self.touch();
    }

    fn set_focus(&mut self, pane: PaneId) {
        let Some(ws) = self.active_mut() else { return };
        let previous = ws.focused.replace(pane);
        if previous == Some(pane) {
            return;
        }
        for (id, focused) in [(previous, false), (Some(pane), true)] {
            if let Some(rt) = id.and_then(|id| self.sessions.get(id)) {
                rt.cache.clear();
                if let Some(session) = &rt.session {
                    session.set_focused(focused);
                }
            }
        }
        self.touch();
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

    fn close_pane(&mut self, pane: PaneId) {
        let Some(ws) = self.active_mut() else { return };
        ws.close(pane);
        let focus = ws.focused;
        self.sessions.remove(pane);
        if let Some(rt) = focus.and_then(|f| self.sessions.get(f)) {
            rt.cache.clear();
            if let Some(session) = &rt.session {
                session.set_focused(true);
            }
        }
        self.touch();
    }

    fn apply_preset(&mut self, preset: Preset) {
        let size = self.default_grid();
        let Some(ws) = self.active_mut() else { return };
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

    fn delete_workspace(&mut self, id: WorkspaceId) {
        let Some(index) = self.position(id) else { return };
        let ws = self.workspaces.remove(index);
        for pane in ws.pane_ids() {
            self.sessions.remove(pane);
        }
        if self.active == Some(id) {
            self.active = None;
            let next = self.workspaces.get(index.min(self.workspaces.len().saturating_sub(1))).map(WorkspaceView::id);
            if let Some(next) = next {
                self.activate(next);
            }
        }
        self.touch();
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
        }
    }

    // ---- update --------------------------------------------------------------------------

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Term(pane, event) => return self.on_term_event(pane, event),
            Message::Terminal(pane, msg) => return self.on_terminal(pane, msg),
            Message::KeyPressed { key, modifiers, text } => return self.on_key(key, modifiers, text),
            Message::Action(action) => return self.perform(action),
            Message::SelectWorkspace(id) => {
                if self.active != Some(id) {
                    self.activate(id);
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
                    return ui::modal::focus_first_field();
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
            Message::ClosePane(pane) => self.close_pane(pane),
            Message::MaximizePane(pane) => {
                if let Some(ws) = self.active_mut() {
                    ws.toggle_maximize(pane);
                }
                self.set_focus(pane);
                self.sessions.clear_all_caches();
            }
            Message::RestartPane(pane) => self.restart(pane),
            Message::NewPane => self.split(None, Axis::Vertical),
            Message::ApplyPreset(preset) => {
                let Some(ws) = self.active_view() else { return Task::none() };
                let closing = ws.pane_count().saturating_sub(preset.pane_count());
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
                    self.focused_pane().and_then(|p| self.sessions.get(p)).and_then(|rt| rt.session.as_ref())
                {
                    session.paste(&text);
                }
            }
            Message::Paste(None) => {}
            Message::SaveTick => {
                self.saver.save(self.persisted());
                self.dirty = false;
            }
            Message::CwdTick => {
                if self.sessions.refresh_cwds() {
                    self.touch();
                }
            }
            Message::WindowResized(size) => {
                self.prefs.window = Some(WindowGeometry { width: size.width, height: size.height });
                self.touch();
            }
            Message::CloseRequested(id) => return self.quit(Some(id)),
        }
        Task::none()
    }

    fn quit(&mut self, window: Option<window::Id>) -> Task<Message> {
        self.sessions.refresh_cwds();
        self.saver.save_and_wait(self.persisted());
        match window {
            Some(id) => window::close(id),
            None => iced::exit(),
        }
    }

    fn restart(&mut self, pane: PaneId) {
        let size = self.default_grid();
        let Some(ws) = self.active_view() else { return };
        let root = ws.model.root.clone();
        let focused = ws.focused == Some(pane);
        let cwd = self.sessions.get(pane).map_or_else(|| root.clone(), |rt| rt.cwd.clone());
        self.sessions.spawn(pane, &cwd, &root, size);
        if let Some(session) = self.sessions.get(pane).and_then(|rt| rt.session.as_ref()) {
            session.set_focused(focused);
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

    fn on_key(&mut self, key: Key, modifiers: Modifiers, text: Option<String>) -> Task<Message> {
        if let Some(action) = keymap::action(&key, modifiers) {
            return self.perform(action);
        }
        if self.modal.is_some() {
            return match key {
                Key::Named(iced::keyboard::key::Named::Escape) => self.on_modal(ModalMsg::Cancel),
                Key::Named(iced::keyboard::key::Named::Enter) => self.on_modal(ModalMsg::Submit),
                _ => Task::none(),
            };
        }
        let Some(pane) = self.focused_pane() else { return Task::none() };
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
        // While a sheet is open only app-level actions make sense.
        if self.modal.is_some() && !matches!(action, Action::Quit | Action::ShowShortcuts) {
            return Task::none();
        }
        match action {
            Action::NewWorkspace => {
                self.modal = Some(Modal::Editor(Editor::new_workspace()));
                return ui::modal::focus_first_field();
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
                if let Some(pane) = self.focused_pane() {
                    self.close_pane(pane);
                }
            }
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
                let text = self.focused_pane().and_then(|p| self.sessions.get(p)?.session.as_ref()?.selection_text());
                if let Some(text) = text {
                    return clipboard::write(text);
                }
            }
            Action::Paste => return clipboard::read().map(Message::Paste),
            Action::FontBigger => self.set_font_size(self.prefs.font_size + 1.0),
            Action::FontSmaller => self.set_font_size(self.prefs.font_size - 1.0),
            Action::FontReset => self.set_font_size(UiPrefs::default().font_size),
            Action::ScrollPageUp | Action::ScrollPageDown => {
                if let Some(rt) = self.focused_pane().and_then(|p| self.sessions.get(p)) {
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
            }
            Action::Quit => return self.quit(None),
        }
        Task::none()
    }

    fn on_modal(&mut self, msg: ModalMsg) -> Task<Message> {
        match (&mut self.modal, msg) {
            (_, ModalMsg::Cancel) => {
                // The very first run has nothing behind the sheet; keep it up.
                if !self.workspaces.is_empty() {
                    self.modal = None;
                }
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
                            self.activate(id);
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
                self.delete_workspace(id);
                if self.workspaces.is_empty() {
                    self.modal = Some(Modal::Editor(Editor::new_workspace()));
                    return ui::modal::focus_first_field();
                }
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

/// Global events: keys no widget used, window resizes and close requests.
fn on_runtime_event(event: iced::Event, status: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, text, .. })
            if status == event::Status::Ignored =>
        {
            Some(Message::KeyPressed { key, modifiers, text: text.map(|t| t.to_string()) })
        }
        iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
        iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested(id)),
        _ => None,
    }
}
