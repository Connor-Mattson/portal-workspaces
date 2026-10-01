//! The application: state, messages, update and subscriptions (Elm architecture).

mod attention;
mod editor;
mod explorer;
mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{Space, pane_grid, stack};
use iced::{Element, Subscription, Task, clipboard, event, time, window};
use pw_model::{
    Axis, Mode, PaneId, PersistedState, Preset, ProfileId, SCHEMA_VERSION, UiPrefs, Workspace, WorkspaceId,
};
use pw_term::{GridSize, MouseButton, MouseEvent, MouseEventKind, SelectionKind, TermEvent};

use crate::devtools::{Dev, DevMsg};
use crate::editor::Focus;
use crate::editor::watch::FsHub;
use crate::fonts::CellMetrics;
use crate::keymap::{self, Action, Context};
use crate::notifier::{NoteEvent, Notifier};
use crate::persist::Saver;
use crate::sessions::Sessions;
use crate::system::SystemProfile;
use crate::ui;
use crate::ui::modal::{Modal, ModalMsg, WorkspaceSheet};
use crate::ui::profile_editor::{self, ProfileEditor};
use crate::ui::term_menu::{MenuMsg, TermMenu};
use crate::ui::terminal::TermMsg;
use crate::usage::Usage;
use crate::workspace::WorkspaceView;

pub use editor::{Clicked, EditorMsg};
pub use explorer::ExplorerMsg;
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
    /// The terminal's right-click menu.
    TermMenu(MenuMsg),
    /// A reading from the usage monitor.
    Usage(pw_usage::Update),
    RefreshUsage,
    /// Asks an idle profile's CLI to renew its expired sign-in.
    RenewProfile(ProfileId),
    AddProfile,
    EditProfile(ProfileId),
    ToggleUsage,
    /// A reading from the system monitor.
    System(Box<pw_system::Sample>),
    ToggleSystem,
    SaveTick,
    CwdTick,
    Window(WindowMsg),
    /// Agents or Editor, for the active workspace.
    SetMode(Mode),
    Editor(EditorMsg),
    Explorer(ExplorerMsg),
    /// Files changed under a workspace's watched folders.
    Fs(WorkspaceId, Vec<PathBuf>),
    /// The scripted UI driver (see `devtools`).
    Dev(DevMsg),
    /// The desktop notification was shown, clicked or closed.
    Note(NoteEvent),
    ToggleNotifications,
}

pub struct App {
    pub(crate) workspaces: Vec<WorkspaceView>,
    pub(crate) active: Option<WorkspaceId>,
    pub(crate) sessions: Sessions,
    pub(crate) usage: Usage,
    pub(crate) system: SystemProfile,
    pub(crate) prefs: UiPrefs,
    pub(crate) metrics: CellMetrics,
    pub(crate) modal: Option<Modal>,
    /// A terminal's right-click menu, while it's open.
    pub(crate) term_menu: Option<TermMenu>,
    pub(crate) main_window: window::Id,
    /// Detached terminals' windows (see `windows`).
    pub(crate) pane_windows: HashMap<window::Id, PaneId>,
    /// The app window that last had focus. Keys go to its terminal.
    pub(crate) key_window: window::Id,
    /// The app window that has the system's focus now; `None` while you're in another app.
    pub(crate) focused_window: Option<window::Id>,
    /// Desktop notifications for terminals that want you (see `app::attention`).
    pub(crate) notifier: Notifier,
    /// Orders attention app-wide, so the oldest is answered first.
    attention_seq: u64,
    /// Watches the folders the editors show.
    pub(crate) fs: FsHub,
    /// The last tab or tree row clicked, for double clicks.
    pub(crate) last_click: Option<(Clicked, Instant)>,
    /// The scripted UI driver's pending screenshot.
    pub(crate) dev: Dev,
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
            system: SystemProfile::new(),
            metrics: CellMetrics::for_size(state.ui.font_size),
            prefs: state.ui,
            modal: None,
            term_menu: None,
            main_window,
            pane_windows: HashMap::new(),
            key_window: main_window,
            focused_window: None,
            notifier: Notifier::default(),
            attention_seq: 0,
            fs: FsHub::default(),
            last_click: None,
            dev: Dev::default(),
            dirty: false,
            saver,
        };
        let mut tasks = vec![opened.discard()];
        match state.active {
            // Restored detached windows open after the main one; it keeps the focus.
            Some(id) => tasks.push(app.activate(id).chain(window::gain_focus(main_window))),
            None => {
                app.modal = Some(Modal::WorkspaceSheet(WorkspaceSheet::new_workspace()));
                tasks.push(ui::modal::focus_first_field());
            }
        }
        app.sync_system();
        tasks.extend(crate::devtools::from_env());
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
        let title = match self.active_view() {
            Some(ws) => format!("{} — Portal Workspaces", ws.model.name),
            None => "Portal Workspaces".to_owned(),
        };
        // The taskbar shows how many workspaces want you.
        match self.attention_count() {
            0 => title,
            n => format!("({n}) {title}"),
        }
    }

    pub fn view(&self, window: window::Id) -> Element<'_, Message> {
        let content = if window == self.main_window {
            ui::view(self)
        } else if let Some(&pane) = self.pane_windows.get(&window) {
            ui::pane_window::view(self, pane)
        } else {
            // A window on its way out (a terminal, just docked).
            Space::new().into()
        };
        // Always a stack, so opening the menu doesn't rebuild the window's widget state (focus, drags).
        let menu = match &self.term_menu {
            Some(menu) if menu.window == window => ui::term_menu::view(menu),
            _ => Space::new().into(),
        };
        stack![content, menu].into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            self.sessions.events().map(|(id, event)| Message::Term(id, event)),
            self.usage.events().map(Message::Usage),
            self.system.events().map(|sample| Message::System(Box::new(sample))),
            self.fs.events().map(|(id, paths)| Message::Fs(id, paths)),
            self.notifier.events().map(Message::Note),
            event::listen_with(on_runtime_event),
            time::every(CWD_POLL).map(|_| Message::CwdTick),
            self.dev.subscription(),
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

    pub(crate) fn active_mut(&mut self) -> Option<&mut WorkspaceView> {
        let active = self.active?;
        self.workspaces.iter_mut().find(|w| w.id() == active)
    }

    fn position(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id() == id)
    }

    /// The workspace a terminal belongs to: in its grid, detached, or its editor's.
    pub(crate) fn workspace_of(&self, pane: PaneId) -> Option<&WorkspaceView> {
        self.workspaces.iter().find(|ws| ws.owns(pane))
    }

    /// The main window's focused pane.
    fn focused_pane(&self) -> Option<PaneId> {
        self.active_view()?.focused
    }

    /// The terminal that gets the keys: the detached terminal whose window has focus, or in the main
    /// window the grid's focused pane (Agents) or the editor's terminal when it has the keys (Editor).
    pub(crate) fn key_pane(&self) -> Option<PaneId> {
        if let Some(&pane) = self.pane_windows.get(&self.key_window) {
            return Some(pane);
        }
        let ws = self.active_view()?;
        match ws.model.mode {
            Mode::Agents => ws.focused,
            Mode::Editor => {
                let editor = ws.editor.as_ref()?;
                (editor.focus == Focus::Terminal && editor.show_terminal).then_some(editor.terminal.id)
            }
        }
    }

    /// What has the keys, for shortcuts.
    fn context(&self) -> Context {
        if self.key_window != self.main_window {
            return Context::Terminal;
        }
        match self.editor().map(|e| e.focus) {
            Some(Focus::Editor) => Context::Editor,
            Some(Focus::Explorer) => Context::Explorer,
            Some(Focus::Terminal) | None => Context::Terminal,
        }
    }

    /// Whether a pane is drawn as focused (solid cursor, accent frame).
    pub(crate) fn has_focus(&self, pane: PaneId) -> bool {
        self.key_pane() == Some(pane)
    }

    /// Whether a terminal is currently drawn: in its own window, or in the active workspace's grid
    /// (Agents) or terminal panel (Editor).
    fn is_visible(&self, pane: PaneId) -> bool {
        self.window_of(pane).is_some()
            || self.active_view().is_some_and(|ws| match ws.model.mode {
                Mode::Agents => match ws.maximized() {
                    Some(max) => max == pane,
                    None => ws.handle(pane).is_some(),
                },
                Mode::Editor => ws.editor.as_ref().is_some_and(|e| e.show_terminal && e.terminal.id == pane),
            })
    }

    pub(crate) fn default_grid(&self) -> GridSize {
        GridSize {
            cols: 80,
            rows: 24,
            cell_width: self.metrics.width.round() as u16,
            cell_height: self.metrics.height as u16,
        }
    }

    // ---- state changes -------------------------------------------------------------------

    pub(crate) fn touch(&mut self) {
        self.dirty = true;
    }

    /// Samples the system only while the profile is on screen: the drawer's section is open, or
    /// the collapsed drawer's rail shows it. Also hands over the shells to total per workspace.
    fn sync_system(&mut self) {
        self.system.set_active(self.prefs.sidebar_collapsed || self.prefs.system_expanded);
        self.system.track(self.sessions.pids());
    }

    /// Runs `change`, then tells the terminals that lost and gained the keys.
    pub(crate) fn refocus(&mut self, change: impl FnOnce(&mut Self)) {
        let before = self.key_pane();
        change(self);
        let after = self.key_pane();
        // Even when the keys stayed put, you may have just come back to them.
        self.acknowledge();
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
            // Hidden panes kept running without redrawing; their caches are stale.
            self.sessions.get(pane).expect("just ensured").cache.clear();
        }
        self.touch();
        let closed: Vec<PaneId> = detached.into_iter().filter(|p| self.window_of(*p).is_none()).collect();
        let mut tasks: Vec<Task<Message>> = closed.into_iter().map(|p| self.open_window(p)).collect();
        if self.active_view().is_some_and(|ws| ws.model.mode == Mode::Editor) {
            tasks.push(self.start_editor());
        }
        Task::batch(tasks)
    }

    /// Gives a terminal the keys: focuses it in the grid or the editor, or makes its window the key
    /// window.
    fn set_focus(&mut self, pane: PaneId) {
        if self.window_of(pane).is_none() && self.editor().is_some_and(|e| e.terminal.id == pane) {
            // The click that got here already took the keys from the code editor widget.
            let _ = self.set_editor_focus(Focus::Terminal);
            return;
        }
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
        for pane in ws.all_pane_ids().into_iter().chain([ws.editor_terminal()]) {
            self.sessions.remove(pane);
        }
        self.fs.unwatch(id);
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
        if self.dismiss_term_menu(&message) {
            return Task::none();
        }
        match message {
            Message::Term(pane, event) => return self.on_term_event(pane, event),
            Message::Terminal(pane, msg) => return self.on_terminal(pane, msg),
            Message::KeyPressed { window, key, modifiers, text } => return self.on_key(window, key, modifiers, text),
            Message::Action(action) => return self.perform(action),
            Message::SelectWorkspace(id) => return self.select_workspace(id),
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
                    self.modal = Some(Modal::WorkspaceSheet(WorkspaceSheet::edit(&ws.model)));
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
            Message::TermMenu(msg) => return self.on_term_menu(msg),
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
            Message::System(sample) => {
                self.system.apply(*sample);
                // Terminals opened since the last reading are counted from the next one.
                self.system.track(self.sessions.pids());
            }
            Message::ToggleSystem => {
                self.prefs.system_expanded = !self.prefs.system_expanded;
                self.sync_system();
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
            Message::SetMode(mode) => return self.set_mode(mode),
            Message::Editor(msg) => return self.on_editor(msg),
            Message::Explorer(msg) => return self.on_explorer(msg),
            Message::Fs(id, paths) => self.on_fs(id, paths),
            Message::Dev(msg) => return self.on_dev(msg),
            Message::Note(event) => return self.on_note(event),
            Message::ToggleNotifications => self.toggle_notifications(),
        }
        Task::none()
    }

    /// Quits, first asking about unsaved files.
    pub(crate) fn request_quit(&mut self) -> Task<Message> {
        let files = self.unsaved();
        if files.is_empty() {
            return self.quit();
        }
        self.modal = Some(Modal::Unsaved { files, then: ui::modal::Pending::Quit, error: None });
        self.raise_main()
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
                }
            }
            TermEvent::Title(title) => rt.title = Some(title).filter(|t| !t.trim().is_empty()),
            TermEvent::ResetTitle => rt.title = None,
            TermEvent::Bell => return self.on_bell(pane),
            TermEvent::Notify { title, body } => return self.on_notify(pane, title, body),
            TermEvent::Busy => self.on_busy(pane),
            TermEvent::Idle { worked } => return self.on_idle(pane, worked),
            TermEvent::Exited(code) => {
                rt.exited = Some(code);
                rt.working = false;
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
        let window = self.window_of(pane).unwrap_or(self.main_window);
        let Some(rt) = self.sessions.get_mut(pane) else { return Task::none() };
        let Some(session) = rt.session.as_mut() else { return Task::none() };
        // Shift overrides mouse reporting so you can always select text, as in other terminals.
        let report = |mods: pw_term::Mods| session.wants_mouse() && !mods.shift;
        let mut task = Task::none();
        match msg {
            TermMsg::Resize(size) => session.resize(size),
            TermMsg::MouseDown { point, right_half, button, clicks, mods, menu_at } => {
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
                } else {
                    let can_copy = session.selection_text().is_some();
                    self.term_menu = Some(TermMenu { window, pane, at: menu_at, can_copy });
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
        let context = self.context();
        if let Some(action) = keymap::action(&key, modifiers, context) {
            return self.perform(action);
        }
        // A sheet in the main window doesn't stop the detached terminals.
        if self.modal.is_some() && window == self.main_window {
            return match key {
                Key::Named(Named::Escape) => self.on_modal(ModalMsg::Cancel),
                Key::Named(Named::Enter) => self.on_modal(ModalMsg::Submit),
                _ => Task::none(),
            };
        }
        if window == self.main_window && self.editor().is_some() {
            if self.editor().is_some_and(|e| e.quick_open.is_some()) {
                return self.editor_key(&key).unwrap_or_else(Task::none);
            }
            match context {
                Context::Explorer => return self.explorer_key(&key),
                Context::Editor => return self.editor_key(&key).unwrap_or_else(Task::none),
                Context::Terminal => {}
            }
        }
        let Some(pane) = self.key_pane() else { return Task::none() };
        let Some(rt) = self.sessions.get(pane) else { return Task::none() };
        if keymap::copies_selection(&key, modifiers)
            && let Some(text) = rt.session.as_ref().and_then(|s| s.take_visible_selection())
        {
            rt.cache.clear();
            return clipboard::write(text);
        }
        if rt.exited.is_some() || rt.session.is_none() {
            if key == Key::Named(Named::Enter) {
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
            | Action::SelectAll
            | Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::FontBigger
            | Action::FontSmaller
            | Action::FontReset => self.modal.is_none() || !in_main,
            // Grid and editor actions mean nothing in a detached terminal's window.
            Action::Split(_)
            | Action::Focus(_)
            | Action::ToggleMaximize
            | Action::ToggleMode
            | Action::ToggleTerminal
            | Action::ToggleExplorer
            | Action::QuickOpen
            | Action::Save
            | Action::Undo
            | Action::Redo
            | Action::Find
            | Action::Replace
            | Action::FindNext
            | Action::FindPrev
            | Action::ToggleComment
            | Action::MoveLines { .. }
            | Action::DuplicateLines { .. }
            | Action::NextTab
            | Action::PrevTab => self.modal.is_none() && in_main,
            _ => self.modal.is_none(),
        };
        if !allowed {
            return Task::none();
        }
        if in_main
            && self.modal.is_none()
            && let Some(task) = self.perform_in_editor(action)
        {
            return task;
        }
        match action {
            Action::NewWorkspace => {
                self.modal = Some(Modal::WorkspaceSheet(WorkspaceSheet::new_workspace()));
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
                self.sync_system();
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
            Action::SelectAll => {
                if let Some(pane) = self.key_pane() {
                    return self.select_all(pane);
                }
            }
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
            Action::Quit => return self.request_quit(),
            Action::NextAttention => return self.jump_to_next(),
            Action::ToggleMode => {
                let mode = self.active_view().map_or(Mode::Agents, |ws| ws.model.mode.toggled());
                return self.set_mode(mode);
            }
            // Editor shortcuts outside Editor mode.
            Action::ToggleTerminal
            | Action::ToggleExplorer
            | Action::QuickOpen
            | Action::Save
            | Action::Undo
            | Action::Redo
            | Action::Find
            | Action::Replace
            | Action::FindNext
            | Action::FindPrev
            | Action::ToggleComment
            | Action::MoveLines { .. }
            | Action::DuplicateLines { .. }
            | Action::NextTab
            | Action::PrevTab => {}
        }
        Task::none()
    }

    /// Selects a terminal's whole history and screen.
    fn select_all(&self, pane: PaneId) -> Task<Message> {
        let Some(rt) = self.sessions.get(pane) else { return Task::none() };
        let Some(session) = &rt.session else { return Task::none() };
        session.select_all();
        rt.cache.clear();
        // Like a selection made with the mouse, it becomes the primary selection.
        session.selection_text().map_or_else(Task::none, clipboard::write_primary)
    }

    /// Closes the terminal menu on anything but its own messages and background updates: a key, a
    /// click outside it, a layout or focus change. Returns whether the message is used up: Escape
    /// only closes the menu (an agent would take it as "stop").
    fn dismiss_term_menu(&mut self, message: &Message) -> bool {
        let Some(menu) = &self.term_menu else { return false };
        let keep = match message {
            Message::TermMenu(_)
            | Message::Term(..)
            | Message::Paste(_)
            | Message::Usage(_)
            | Message::System(_)
            | Message::SaveTick
            | Message::CwdTick
            | Message::Fs(..)
            | Message::Note(_)
            | Message::Dev(_) => true,
            // The rest of the right-click that opened it.
            Message::Terminal(_, TermMsg::MouseDrag { .. } | TermMsg::MouseUp { .. }) => true,
            // Its window may report focus after the click.
            Message::Window(WindowMsg::Focused(id)) => *id == menu.window,
            _ => false,
        };
        if keep {
            return false;
        }
        self.term_menu = None;
        matches!(message, Message::KeyPressed { key: Key::Named(Named::Escape), .. })
    }

    fn on_term_menu(&mut self, msg: MenuMsg) -> Task<Message> {
        let Some(menu) = self.term_menu.take() else { return Task::none() };
        match msg {
            MenuMsg::Copy => {
                let text = self.sessions.get(menu.pane).and_then(|rt| rt.session.as_ref()?.selection_text());
                if let Some(text) = text {
                    return clipboard::write(text);
                }
            }
            // The right-click gave the terminal the keys, so the paste lands in it.
            MenuMsg::Paste => return clipboard::read().map(Message::Paste),
            MenuMsg::SelectAll => return self.select_all(menu.pane),
            MenuMsg::Close => {}
        }
        Task::none()
    }

    fn on_modal(&mut self, msg: ModalMsg) -> Task<Message> {
        match (&mut self.modal, msg) {
            (modal, ModalMsg::Cancel) => {
                // The very first run has nothing behind the workspace sheet; keep it up.
                if !self.workspaces.is_empty() || !matches!(modal, Some(Modal::WorkspaceSheet(_))) {
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
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Name(name)) => editor.name = name,
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Root(root)) => {
                editor.root = root;
                editor.error = None;
            }
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Preset(preset)) => editor.preset = preset,
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Browse) => {
                let start = editor
                    .resolved_root()
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| crate::sessions::home_dir().to_path_buf());
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
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Browsed(Some(path))) => editor.set_root(path),
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Submit) => match editor.validate() {
                Ok((name, root)) => {
                    let (target, preset) = (editor.target, editor.preset);
                    self.modal = None;
                    match target.and_then(|id| self.workspaces.iter_mut().find(|w| w.id() == id)) {
                        Some(ws) => {
                            ws.model.name = name;
                            // A new folder means a new file tree; keep the editor only while it holds edits.
                            if ws.model.root != root && ws.editor.as_ref().is_none_or(|e| e.dirty_paths().is_empty()) {
                                ws.editor = None;
                                ws.model.editor.expanded.clear();
                            }
                            ws.model.root = root;
                            let id = ws.id();
                            if self.active == Some(id) && ws.model.mode == Mode::Editor {
                                self.touch();
                                return self.start_editor();
                            }
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
            (Some(Modal::WorkspaceSheet(editor)), ModalMsg::Delete) => {
                if let Some(id) = editor.target {
                    self.modal = Some(Modal::ConfirmDelete(id));
                }
            }
            (Some(Modal::ConfirmDelete(id)), ModalMsg::Submit) => {
                let id = *id;
                self.modal = None;
                let closed = self.delete_workspace(id);
                if self.workspaces.is_empty() {
                    self.modal = Some(Modal::WorkspaceSheet(WorkspaceSheet::new_workspace()));
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
            (Some(Modal::Unsaved { files, then, .. }), ModalMsg::Submit) => {
                let (files, then) = (files.clone(), then.clone());
                let failed = self.save_files(&files);
                if failed.is_empty() {
                    self.modal = None;
                    return self.finish_pending(then);
                }
                if let Some(Modal::Unsaved { error, .. }) = &mut self.modal {
                    *error = Some(format!("Couldn't save {}", failed.join("; ")));
                }
            }
            (Some(Modal::Unsaved { then, .. }), ModalMsg::Discard) => {
                let then = then.clone();
                self.modal = None;
                return self.finish_pending(then);
            }
            (Some(Modal::ConfirmTrash { workspace, path }), ModalMsg::Submit) => {
                let (workspace, path) = (*workspace, path.clone());
                self.modal = None;
                if self.active == Some(workspace) {
                    self.trash(&path);
                    self.sync_watches(workspace);
                }
                return self.set_editor_focus(Focus::Explorer);
            }
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
            window::Event::Unfocused => Some(WindowMsg::Unfocused(id)),
            window::Event::Resized(size) => Some(WindowMsg::Resized(id, size)),
            window::Event::CloseRequested => Some(WindowMsg::CloseRequested(id)),
            window::Event::Closed => Some(WindowMsg::Closed(id)),
            _ => None,
        }
        .map(Message::Window),
        _ => None,
    }
}
