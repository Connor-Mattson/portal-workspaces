//! Editor mode in the app: switching modes, the code editor and its groups, find, quick open, saving,
//! the terminal panel and disk changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use iced::widget::pane_grid;
use iced::widget::text_editor::{Action as EditAction, Edit};
use pw_model::{Axis, GroupId, Mode, WorkspaceId};

use crate::app::{App, Message};
use crate::background;
use crate::editor::disk;
use crate::editor::find::Find;
use crate::editor::quick_open::QuickOpen;
use crate::editor::{EditorView, Focus, Region};
use crate::keymap::{Action, Direction};
use crate::ui::editor as view;
use crate::ui::modal::{Modal, Pending};

/// Two clicks on the same tab or tree row within this time are a double click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Quick open indexes at most this many files.
const INDEX_CAP: usize = 50_000;

/// Something that can be double-clicked: a tab, or a row of the file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clicked {
    Tab(GroupId, usize),
    Row(PathBuf),
}

#[derive(Debug, Clone)]
pub enum EditorMsg {
    /// Typing, clicks and scrolling in a group's code editor.
    Edit(GroupId, EditAction),
    // Keys the focused code editor hands to the app (see `ui::editor::binding`).
    Type(char),
    Backspace,
    Newline,
    Indent,
    Unindent,
    Escape,
    SelectTab(GroupId, usize),
    CloseTab(GroupId, usize),
    SplitGroup(GroupId),
    CloseGroup(GroupId),
    GroupClicked(pane_grid::Pane),
    GroupDragged(pane_grid::DragEvent),
    GroupResized(pane_grid::ResizeEvent),
    RegionResized(pane_grid::ResizeEvent),
    HideTerminal,
    FindQuery(String),
    FindReplacement(String),
    FindCase(bool),
    FindShowReplace(bool),
    FindNext,
    FindPrev,
    ReplaceOne,
    ReplaceAll,
    CloseFind,
    /// Resolves a conflict by saving the edits.
    KeepMine,
    /// Resolves a conflict by taking the file on disk.
    TakeDisk,
    DismissNotice(GroupId),
    OpenExternally(PathBuf),
    QuickQuery(String),
    QuickPick(usize),
    QuickSubmit,
    QuickClose,
    /// The project's files, for quick open.
    Indexed(WorkspaceId, Arc<Vec<String>>),
    /// A batch of disk changes, read in the background.
    DiskRead(WorkspaceId, disk::Scan),
}

impl App {
    // ---- access --------------------------------------------------------------------------------

    /// The active workspace's editor, when it's shown in Editor mode.
    pub(crate) fn editor(&self) -> Option<&EditorView> {
        self.active_view().filter(|ws| ws.model.mode == Mode::Editor)?.editor.as_ref()
    }

    /// Runs `f` with the active workspace's root and editor, when it's shown in Editor mode.
    pub(crate) fn with_editor<T>(&mut self, f: impl FnOnce(&Path, &mut EditorView) -> T) -> Option<T> {
        let ws = self.active_mut().filter(|ws| ws.model.mode == Mode::Editor)?;
        let root = ws.model.root.clone();
        Some(f(&root, ws.editor.as_mut()?))
    }

    /// Records a click; whether it's the second of a double click on the same thing.
    pub(crate) fn double_click(&mut self, target: Clicked) -> bool {
        let now = Instant::now();
        let double = self.last_click.take().is_some_and(|(last, at)| last == target && now - at < DOUBLE_CLICK);
        self.last_click = (!double).then_some((target, now));
        double
    }

    // ---- modes ---------------------------------------------------------------------------------

    pub(crate) fn set_mode(&mut self, mode: Mode) -> Task<Message> {
        let Some(ws) = self.active_view() else { return Task::none() };
        if ws.model.mode == mode {
            return Task::none();
        }
        let panes = ws.all_pane_ids();
        self.refocus(|app| app.active_mut().expect("checked").model.mode = mode);
        self.touch();
        match mode {
            Mode::Editor => self.start_editor(),
            Mode::Agents => {
                // The grid didn't draw while hidden.
                for pane in panes {
                    if let Some(rt) = self.sessions.get_mut(pane) {
                        rt.cache.clear();
                    }
                }
                view::unfocus()
            }
        }
    }

    /// Shows the active workspace's editor, building it the first time, and gives it the keys.
    pub(crate) fn start_editor(&mut self) -> Task<Message> {
        static WARM: std::sync::Once = std::sync::Once::new();
        WARM.call_once(|| {
            let _ = std::thread::Builder::new().name("syntax-warm".into()).spawn(pw_code::syntax::warm);
        });
        let Some(ws) = self.active_mut() else { return Task::none() };
        if ws.editor.is_none() {
            ws.editor = Some(EditorView::start(&ws.model.root, &ws.model.editor));
        }
        let id = ws.id();
        if self.editor().is_some_and(|e| e.show_terminal) {
            self.ensure_editor_terminal();
        }
        if let Some(rt) = self.editor().and_then(|e| self.sessions.get(e.terminal.id)) {
            rt.cache.clear();
        }
        self.sync_watches(id);
        let focus = self.editor().map_or(Focus::Editor, |e| e.focus);
        self.set_editor_focus(focus)
    }

    /// Starts the editor terminal's shell if it isn't running.
    fn ensure_editor_terminal(&mut self) {
        let size = self.default_grid();
        let Some(ws) = self.active_view() else { return };
        let Some(editor) = ws.editor.as_ref() else { return };
        let (id, root) = (editor.terminal.id, ws.model.root.clone());
        if self.sessions.contains(id) {
            return;
        }
        let cwd = Some(editor.terminal.cwd.clone()).filter(|c| !c.as_os_str().is_empty()).unwrap_or(root.clone());
        self.sessions.spawn(id, &cwd, &root, size);
        if let Some(session) = self.sessions.get(id).and_then(|rt| rt.session.as_ref()) {
            session.set_focused(self.has_focus(id));
        }
    }

    /// Watches what the workspace's editor shows.
    pub(crate) fn sync_watches(&mut self, id: WorkspaceId) {
        let Some(ws) = self.workspaces.iter().find(|w| w.id() == id) else { return };
        if let Some(editor) = &ws.editor {
            self.fs.watch(id, &ws.model.root, editor.watched_dirs(&ws.model.root));
        }
    }

    // ---- focus ---------------------------------------------------------------------------------

    /// Gives the keys to a part of Editor mode. The terminal learns it lost or gained them; the code
    /// editor widget is focused or unfocused to match.
    pub(crate) fn set_editor_focus(&mut self, focus: Focus) -> Task<Message> {
        self.refocus(|app| {
            app.key_window = app.main_window;
            if let Some(editor) = app.active_mut().and_then(|ws| ws.editor.as_mut()) {
                editor.focus = focus;
            }
        });
        match (focus, self.editor()) {
            (Focus::Editor, Some(editor)) => view::focus_group(editor.focused),
            _ => view::unfocus(),
        }
    }

    /// Focuses a group's code editor.
    fn focus_group(&mut self, group: GroupId) -> Task<Message> {
        self.with_editor(|_, e| e.focused = group);
        self.set_editor_focus(Focus::Editor)
    }

    /// Moves the keys between the file tree, the groups and the terminal.
    pub(crate) fn editor_focus_move(&mut self, direction: Direction) -> Task<Message> {
        let Some(editor) = self.editor() else { return Task::none() };
        let region = match editor.focus {
            Focus::Explorer => Region::Explorer,
            Focus::Editor => {
                if let Some(group) = editor.adjacent_group(direction) {
                    return self.focus_group(group);
                }
                Region::Editors
            }
            Focus::Terminal => Region::Terminal,
        };
        match editor.adjacent_region(region, direction) {
            Some(Region::Explorer) => self.set_editor_focus(Focus::Explorer),
            Some(Region::Editors) => self.set_editor_focus(Focus::Editor),
            Some(Region::Terminal) => self.set_editor_focus(Focus::Terminal),
            None => Task::none(),
        }
    }

    // ---- actions -------------------------------------------------------------------------------

    /// Shortcuts in Editor mode. Returns `None` for actions it doesn't handle (they keep their
    /// Agent-mode meaning).
    pub(crate) fn perform_in_editor(&mut self, action: Action) -> Option<Task<Message>> {
        let editor = self.editor()?;
        let focus = editor.focus;
        let group = editor.focused;
        let in_text = focus == Focus::Editor;
        Some(match action {
            Action::Split(axis) => self.split_group(group, axis),
            Action::ClosePane => match focus {
                Focus::Editor => match self.editor().and_then(|e| e.focused_group().active) {
                    Some(index) => self.close_tab(group, index),
                    None => self.close_group(group),
                },
                Focus::Terminal => self.update(Message::Editor(EditorMsg::HideTerminal)),
                Focus::Explorer => Task::none(),
            },
            Action::Focus(direction) => self.editor_focus_move(direction),
            Action::ToggleMaximize => {
                self.with_editor(|_, e| {
                    let handle = crate::split::handle(&e.grid, &e.focused);
                    match (e.grid.maximized(), handle) {
                        (Some(_), _) => e.grid.restore(),
                        (None, Some(h)) if e.grid.len() > 1 => e.grid.maximize(h),
                        _ => {}
                    }
                });
                Task::none()
            }
            Action::ToggleDetach => Task::none(),
            Action::ToggleTerminal => self.toggle_terminal(),
            Action::ToggleExplorer => {
                let show = !self.editor().is_some_and(|e| e.show_explorer);
                self.with_editor(|_, e| e.set_show_explorer(show));
                self.touch();
                if show { self.set_editor_focus(Focus::Explorer) } else { self.set_editor_focus(Focus::Editor) }
            }
            Action::QuickOpen => self.open_quick_open(),
            Action::Save => {
                self.save_active();
                Task::none()
            }
            Action::FontBigger | Action::FontSmaller | Action::FontReset if focus != Focus::Terminal => {
                let size = match action {
                    Action::FontBigger => self.prefs.editor_font_size + 1.0,
                    Action::FontSmaller => self.prefs.editor_font_size - 1.0,
                    _ => pw_model::UiPrefs::default().editor_font_size,
                };
                self.prefs.editor_font_size = size.clamp(pw_model::UiPrefs::MIN_FONT, pw_model::UiPrefs::MAX_FONT);
                self.touch();
                Task::none()
            }
            Action::Undo if in_text => self.edit(|b| b.undo()),
            Action::Redo if in_text => self.edit(|b| b.redo()),
            Action::ToggleComment if in_text => self.edit(|b| b.toggle_comment()),
            Action::MoveLines { up } if in_text => self.edit(|b| b.move_lines(up)),
            Action::DuplicateLines { down } if in_text => self.edit(|b| b.duplicate_lines(down)),
            Action::Find | Action::Replace if focus != Focus::Terminal => self.open_find(action == Action::Replace),
            Action::FindNext | Action::FindPrev if focus != Focus::Terminal => {
                self.find_step(action == Action::FindNext);
                Task::none()
            }
            Action::NextTab | Action::PrevTab if focus != Focus::Terminal => {
                self.with_editor(|_, e| {
                    if let Some(g) = e.group_mut(group) {
                        g.cycle(action == Action::NextTab);
                    }
                });
                self.set_editor_focus(Focus::Editor)
            }
            // Editing keys in the file tree or the terminal do nothing.
            Action::Undo
            | Action::Redo
            | Action::ToggleComment
            | Action::MoveLines { .. }
            | Action::DuplicateLines { .. }
            | Action::Find
            | Action::Replace
            | Action::FindNext
            | Action::FindPrev
            | Action::NextTab
            | Action::PrevTab => Task::none(),
            _ => return None,
        })
    }

    /// Runs an editing command on the focused group's active tab.
    fn edit(&mut self, f: impl FnOnce(&mut crate::editor::buffer::Buffer)) -> Task<Message> {
        self.with_editor(|_, e| e.change(e.focused, f));
        Task::none()
    }

    /// Show and focus the terminal; focus it if shown; hide it if it has the keys.
    fn toggle_terminal(&mut self) -> Task<Message> {
        let Some(editor) = self.editor() else { return Task::none() };
        match (editor.show_terminal, editor.focus == Focus::Terminal) {
            (true, true) => self.update(Message::Editor(EditorMsg::HideTerminal)),
            (true, false) => self.set_editor_focus(Focus::Terminal),
            (false, _) => {
                self.show_editor_terminal();
                self.set_editor_focus(Focus::Terminal)
            }
        }
    }

    /// Opens the terminal panel, starting its shell if needed.
    pub(crate) fn show_editor_terminal(&mut self) {
        if self.editor().is_none_or(|e| e.show_terminal) {
            return;
        }
        self.with_editor(|_, e| e.set_show_terminal(true));
        self.ensure_editor_terminal();
        self.touch();
    }

    // ---- files ---------------------------------------------------------------------------------

    /// Opens a file in the focused group. `focus` gives the editor the keys.
    pub(crate) fn open_file(&mut self, path: &Path, preview: bool, focus: bool) -> Task<Message> {
        let opened = self
            .with_editor(|root, e| {
                let group = e.focused;
                e.explorer.selected = Some(path.to_path_buf());
                e.open(root, group, path, preview).is_ok()
            })
            .unwrap_or(false);
        self.touch();
        if let Some(id) = self.active {
            self.sync_watches(id);
        }
        if focus && opened { self.set_editor_focus(Focus::Editor) } else { Task::none() }
    }

    /// Saves the focused group's file.
    fn save_active(&mut self) {
        self.with_editor(|root, e| {
            let group = e.focused;
            let Some(path) = e.focused_group().active_path().map(Path::to_path_buf) else { return };
            if let Err(err) = e.save(root, &path) {
                let message = format!("Couldn't save {}: {err}", path.display());
                if let Some(g) = e.group_mut(group) {
                    g.notice = Some(crate::editor::group::Notice { path, message, external: false });
                }
            }
        });
    }

    /// Closes a tab, asking first if it holds the document's only unsaved view.
    fn close_tab(&mut self, group: GroupId, index: usize) -> Task<Message> {
        let Some(editor) = self.editor() else { return Task::none() };
        let Some(path) = editor.group(group).and_then(|g| g.tabs.get(index)).map(|t| t.path.clone()) else {
            return Task::none();
        };
        let dirty = editor.doc(&path).is_some_and(|d| d.dirty) && editor.views(&path) == 1;
        let workspace = self.active.expect("an editor is shown");
        if dirty {
            self.modal = Some(Modal::Unsaved {
                files: vec![(workspace, path.clone())],
                then: Pending::CloseTab { workspace, group, path },
                error: None,
            });
            return Task::none();
        }
        self.with_editor(|_, e| e.close_tab(group, index));
        self.touch();
        self.sync_watches(workspace);
        self.set_editor_focus(Focus::Editor)
    }

    /// Closes a group, asking first about unsaved files only it shows.
    fn close_group(&mut self, group: GroupId) -> Task<Message> {
        let Some(editor) = self.editor() else { return Task::none() };
        let workspace = self.active.expect("an editor is shown");
        let Some(g) = editor.group(group) else { return Task::none() };
        let unsaved: Vec<(WorkspaceId, PathBuf)> = g
            .tabs
            .iter()
            .filter(|t| editor.doc(&t.path).is_some_and(|d| d.dirty))
            .filter(|t| editor.views(&t.path) == g.tabs.iter().filter(|o| o.path == t.path).count())
            .map(|t| (workspace, t.path.clone()))
            .collect();
        if !unsaved.is_empty() {
            self.modal =
                Some(Modal::Unsaved { files: unsaved, then: Pending::CloseGroup { workspace, group }, error: None });
            return Task::none();
        }
        self.with_editor(|_, e| e.close_group(group));
        self.touch();
        self.sync_watches(workspace);
        self.set_editor_focus(Focus::Editor)
    }

    /// Finishes a close that was waiting for the unsaved-changes sheet.
    pub(crate) fn finish_pending(&mut self, pending: Pending) -> Task<Message> {
        match pending {
            Pending::Quit => self.quit(),
            Pending::CloseTab { workspace, group, path } => {
                if self.active != Some(workspace) {
                    return Task::none();
                }
                self.with_editor(|_, e| {
                    if let Some(index) = e.group(group).and_then(|g| g.position(&path)) {
                        e.close_tab(group, index);
                    }
                });
                self.touch();
                self.sync_watches(workspace);
                self.set_editor_focus(Focus::Editor)
            }
            Pending::CloseGroup { workspace, group } => {
                if self.active != Some(workspace) {
                    return Task::none();
                }
                self.with_editor(|_, e| e.close_group(group));
                self.touch();
                self.sync_watches(workspace);
                self.set_editor_focus(Focus::Editor)
            }
        }
    }

    /// Every unsaved file, in every workspace.
    pub(crate) fn unsaved(&self) -> Vec<(WorkspaceId, PathBuf)> {
        self.workspaces
            .iter()
            .flat_map(|ws| {
                let paths = ws.editor.as_ref().map(EditorView::dirty_paths).unwrap_or_default();
                paths.into_iter().map(move |p| (ws.id(), p))
            })
            .collect()
    }

    /// Saves the given files. Returns what failed, with why.
    pub(crate) fn save_files(&mut self, files: &[(WorkspaceId, PathBuf)]) -> Vec<String> {
        let mut failed = Vec::new();
        for (id, path) in files {
            let Some(ws) = self.workspaces.iter_mut().find(|w| w.id() == *id) else { continue };
            let root = ws.model.root.clone();
            if let Some(editor) = ws.editor.as_mut()
                && let Err(err) = editor.save(&root, path)
            {
                failed.push(format!("{}: {err}", path.display()));
            }
        }
        failed
    }

    fn split_group(&mut self, group: GroupId, axis: Axis) -> Task<Message> {
        if self.with_editor(|root, e| e.split(root, group, axis)).flatten().is_none() {
            return Task::none();
        }
        self.touch();
        self.set_editor_focus(Focus::Editor)
    }

    // ---- find ----------------------------------------------------------------------------------

    fn open_find(&mut self, replace: bool) -> Task<Message> {
        let group = self
            .with_editor(|_, e| {
                let group = e.focused;
                // A selection within one line becomes the query.
                let selected = e
                    .focused_group()
                    .active_tab()
                    .and_then(|t| t.view.selection())
                    .filter(|s| !s.is_empty() && !s.contains('\n'));
                let g = e.group_mut(group)?;
                let find = g.find.get_or_insert_with(Find::default);
                if let Some(selected) = selected {
                    find.query = selected;
                }
                find.show_replace |= replace;
                Some(group)
            })
            .flatten();
        match group {
            Some(group) if replace => view::focus_replace(group),
            Some(group) => view::focus_find(group),
            None => Task::none(),
        }
    }

    fn find_step(&mut self, forward: bool) {
        self.with_editor(|_, e| {
            e.refresh_finds();
            let group = e.focused;
            let g = e.group(group)?;
            let (find, tab) = (g.find.as_ref()?, g.active_tab()?);
            let (line, range) = find.step(&tab.view, forward)?;
            e.change(group, |b| b.select(line, range))
        });
    }

    fn with_find(&mut self, f: impl FnOnce(&mut Find)) {
        self.with_editor(|_, e| {
            let group = e.focused;
            if let Some(find) = e.group_mut(group).and_then(|g| g.find.as_mut()) {
                f(find);
            }
        });
    }

    // ---- quick open ----------------------------------------------------------------------------

    fn open_quick_open(&mut self) -> Task<Message> {
        let Some(id) = self.active else { return Task::none() };
        let Some(root) = self.with_editor(|root, e| {
            e.quick_open = Some(QuickOpen::new(e.index.clone()));
            root.to_path_buf()
        }) else {
            return Task::none();
        };
        let index = Task::perform(
            background::blocking(move || {
                let (files, _) = pw_code::fs::index(&root, INDEX_CAP);
                Arc::new(files.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>())
            }),
            move |files| Message::Editor(EditorMsg::Indexed(id, files)),
        );
        Task::batch([index, view::focus_quick_open()])
    }

    fn close_quick_open(&mut self) -> Task<Message> {
        self.with_editor(|_, e| e.quick_open = None);
        let focus = self.editor().map_or(Focus::Editor, |e| e.focus);
        self.set_editor_focus(focus)
    }

    // ---- disk ----------------------------------------------------------------------------------

    /// Paths changed under a workspace (relative to its root, from the watcher). They're read in the
    /// background (see `editor::disk`), one batch at a time.
    pub(crate) fn on_fs(&mut self, id: WorkspaceId, paths: Vec<PathBuf>) -> Task<Message> {
        let Some(editor) = self.workspaces.iter_mut().find(|w| w.id() == id).and_then(|w| w.editor.as_mut()) else {
            return Task::none();
        };
        editor.disk.push(paths);
        self.read_disk(id)
    }

    /// Starts reading the workspace's waiting disk changes, unless a batch is already running.
    fn read_disk(&mut self, id: WorkspaceId) -> Task<Message> {
        let Some(ws) = self.workspaces.iter_mut().find(|w| w.id() == id) else { return Task::none() };
        let Some(editor) = ws.editor.as_mut() else { return Task::none() };
        let Some(paths) = editor.disk.start() else { return Task::none() };
        let plan = editor.plan_disk(&ws.model.root, &paths);
        Task::perform(background::blocking(move || plan.read()), move |scan| {
            Message::Editor(EditorMsg::DiskRead(id, scan))
        })
    }

    /// A batch came back: applies it, then starts the next one if changes waited meanwhile.
    fn on_disk_read(&mut self, id: WorkspaceId, scan: disk::Scan) -> Task<Message> {
        let Some(editor) = self.workspaces.iter_mut().find(|w| w.id() == id).and_then(|w| w.editor.as_mut()) else {
            return Task::none();
        };
        editor.disk.finished();
        let again = editor.apply_disk(scan);
        editor.disk.push(again);
        editor.refresh_finds();
        self.sync_watches(id);
        self.read_disk(id)
    }

    // ---- messages ------------------------------------------------------------------------------

    pub(crate) fn on_editor(&mut self, msg: EditorMsg) -> Task<Message> {
        let task = self.editor_update(msg);
        self.with_editor(|_, e| e.refresh_finds());
        task
    }

    fn editor_update(&mut self, msg: EditorMsg) -> Task<Message> {
        if let EditorMsg::Indexed(id, files) = msg {
            if let Some(editor) = self.workspaces.iter_mut().find(|w| w.id() == id).and_then(|w| w.editor.as_mut()) {
                if let Some(quick) = editor.quick_open.as_mut() {
                    quick.set_files(files.clone());
                }
                editor.index = Some(files);
            }
            return Task::none();
        }
        if let EditorMsg::DiskRead(id, scan) = msg {
            return self.on_disk_read(id, scan);
        }
        let Some(editor) = self.editor() else { return Task::none() };
        match msg {
            EditorMsg::Edit(group, action) => {
                let pointer = matches!(action, EditAction::Click(_) | EditAction::SelectWord | EditAction::SelectLine);
                if pointer && (group != editor.focused || editor.focus != Focus::Editor) {
                    self.with_editor(|_, e| e.focused = group);
                    // The widget focused itself on the click.
                    let _ = self.set_editor_focus(Focus::Editor);
                }
                self.with_editor(|_, e| e.change(group, |b| b.perform(action)));
            }
            EditorMsg::Type(c) => return self.edit(|b| b.type_char(c)),
            EditorMsg::Backspace => return self.edit(|b| b.backspace()),
            EditorMsg::Newline => return self.edit(|b| b.newline()),
            EditorMsg::Indent => return self.edit(|b| b.indent()),
            EditorMsg::Unindent => return self.edit(|b| b.unindent()),
            EditorMsg::Escape => {
                if editor.quick_open.is_some() {
                    return self.close_quick_open();
                }
                if editor.focused_group().find.is_some() {
                    return self.update(Message::Editor(EditorMsg::CloseFind));
                }
                if editor.focus == Focus::Terminal || editor.focus == Focus::Explorer {
                    return self.set_editor_focus(Focus::Editor);
                }
            }
            EditorMsg::SelectTab(group, index) => {
                let double = self.double_click(Clicked::Tab(group, index));
                self.with_editor(|_, e| {
                    let g = e.group_mut(group)?;
                    let tab = g.tabs.get_mut(index)?;
                    // Double-clicking a preview keeps it.
                    tab.preview &= !double;
                    g.active = Some(index);
                    e.explorer.selected = Some(e.group(group)?.active_path()?.to_path_buf());
                    Some(())
                });
                self.touch();
                return self.focus_group(group);
            }
            EditorMsg::CloseTab(group, index) => return self.close_tab(group, index),
            EditorMsg::SplitGroup(group) => return self.split_group(group, Axis::Vertical),
            EditorMsg::CloseGroup(group) => return self.close_group(group),
            EditorMsg::GroupClicked(pane) => {
                if let Some(group) = editor.grid.get(pane).copied() {
                    return self.focus_group(group);
                }
            }
            EditorMsg::GroupDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                self.with_editor(|_, e| e.grid.drop(pane, target));
                self.touch();
            }
            EditorMsg::GroupDragged(_) => {}
            EditorMsg::GroupResized(pane_grid::ResizeEvent { split, ratio }) => {
                self.with_editor(|_, e| e.grid.resize(split, ratio));
                self.touch();
            }
            EditorMsg::RegionResized(pane_grid::ResizeEvent { split, ratio }) => {
                self.with_editor(|_, e| e.resize_region(split, ratio));
                self.sessions.clear_all_caches();
                self.touch();
            }
            EditorMsg::HideTerminal => {
                self.with_editor(|_, e| e.set_show_terminal(false));
                self.touch();
                return self.set_editor_focus(Focus::Editor);
            }
            EditorMsg::FindQuery(query) => self.with_find(|f| f.query = query),
            EditorMsg::FindReplacement(text) => self.with_find(|f| f.replacement = text),
            EditorMsg::FindCase(on) => self.with_find(|f| f.case_sensitive = on),
            EditorMsg::FindShowReplace(on) => self.with_find(|f| f.show_replace = on),
            EditorMsg::FindNext => self.find_step(true),
            EditorMsg::FindPrev => self.find_step(false),
            EditorMsg::ReplaceOne => {
                self.with_editor(|_, e| {
                    e.refresh_finds();
                    let group = e.focused;
                    let find = e.group(group)?.find.as_ref()?;
                    find.current?;
                    let replacement = Arc::new(find.replacement.clone());
                    e.change(group, |b| b.perform(EditAction::Edit(Edit::Paste(replacement))))
                });
                self.find_step(true);
            }
            EditorMsg::ReplaceAll => {
                self.with_editor(|_, e| {
                    e.refresh_finds();
                    let group = e.focused;
                    let g = e.group(group)?;
                    let text = g.find.as_ref()?.replace_all(&g.active_tab()?.view);
                    e.change(group, |b| b.set_text(&text))
                });
            }
            EditorMsg::CloseFind => {
                self.with_editor(|_, e| {
                    let group = e.focused;
                    if let Some(g) = e.group_mut(group) {
                        g.find = None;
                    }
                });
                return self.set_editor_focus(Focus::Editor);
            }
            EditorMsg::KeepMine => self.save_active(),
            EditorMsg::TakeDisk => {
                self.with_editor(|root, e| {
                    let path = e.focused_group().active_path()?.to_path_buf();
                    e.take_disk(root, &path);
                    Some(())
                });
            }
            EditorMsg::DismissNotice(group) => {
                self.with_editor(|_, e| e.group_mut(group).map(|g| g.notice = None));
            }
            EditorMsg::OpenExternally(path) => {
                if let Some(root) = self.active_view().map(|ws| ws.model.root.clone()) {
                    open_externally(&root.join(path));
                }
            }
            EditorMsg::QuickQuery(query) => {
                self.with_editor(|_, e| e.quick_open.as_mut().map(|q| q.set_query(query)));
            }
            EditorMsg::QuickPick(_) | EditorMsg::QuickSubmit if editor.quick_open.is_some() => {
                let quick = editor.quick_open.as_ref().expect("checked");
                let index = if let EditorMsg::QuickPick(i) = msg { i } else { quick.selected };
                let Some(path) = quick.path(index).map(PathBuf::from) else { return Task::none() };
                self.with_editor(|_, e| e.quick_open = None);
                let task = self.open_file(&path, false, true);
                self.with_editor(|root, e| e.explorer.reveal(root, &path));
                return task;
            }
            EditorMsg::QuickPick(_) | EditorMsg::QuickSubmit => {}
            EditorMsg::QuickClose => return self.close_quick_open(),
            EditorMsg::Indexed(..) | EditorMsg::DiskRead(..) => unreachable!("handled above"),
        }
        Task::none()
    }

    /// Keys in Editor mode that no shortcut claimed, when the code editor widget didn't take them:
    /// quick open's list, Escape.
    pub(crate) fn editor_key(&mut self, key: &iced::keyboard::Key) -> Option<Task<Message>> {
        use iced::keyboard::Key;
        use iced::keyboard::key::Named;
        let editor = self.editor()?;
        if editor.quick_open.is_some() {
            return Some(match key {
                Key::Named(Named::ArrowDown) => {
                    self.with_editor(|_, e| e.quick_open.as_mut().map(|q| q.step(1)));
                    Task::none()
                }
                Key::Named(Named::ArrowUp) => {
                    self.with_editor(|_, e| e.quick_open.as_mut().map(|q| q.step(-1)));
                    Task::none()
                }
                Key::Named(Named::Escape) => self.close_quick_open(),
                _ => Task::none(),
            });
        }
        match key {
            Key::Named(Named::Escape) if editor.focus == Focus::Editor => {
                Some(self.update(Message::Editor(EditorMsg::Escape)))
            }
            _ => None,
        }
    }
}

/// Hands a file to the system's default app.
fn open_externally(path: &Path) {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let spawned = std::process::Command::new(program)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Reap it, so it doesn't linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!(%err, path = %path.display(), "couldn't open file externally"),
    }
}
