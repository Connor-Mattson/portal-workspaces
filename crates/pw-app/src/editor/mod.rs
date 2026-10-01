//! Editor mode while the app runs: one workspace's open documents, editor groups, file tree and
//! terminal panel.
//!
//! This layer holds state and rules, not widgets: views read it, `app::editor` changes it.
//!
//! One document can show in several groups, each tab with its own view (cursor, scroll). Every
//! change to text goes through [`EditorView::change`], which edits one view and replays the change
//! into the document's other views, so they never drift apart.

pub mod buffer;
pub mod document;
pub mod explorer;
pub mod find;
pub mod group;
pub mod history;
pub mod quick_open;
pub mod watch;

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::pane_grid::{self, Axis as IcedAxis, Configuration};
use pw_code::TextEdit;
use pw_model::{Axis, EditorSession, GroupId, GroupSpec, MAX_GROUPS, OpenFile, TerminalSpec};

use self::buffer::Buffer;
use self::document::{Document, OpenError, Reload};
use self::explorer::Explorer;
use self::group::{Group, Notice, Tab};
use self::quick_open::QuickOpen;
use crate::keymap::Direction;
use crate::split;

/// The parts of Editor mode, as laid out by [`EditorView::regions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Explorer,
    Editors,
    Terminal,
}

/// What has the keyboard in Editor mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Explorer,
    /// The focused group's code editor.
    Editor,
    Terminal,
}

pub struct EditorView {
    docs: HashMap<PathBuf, Document>,
    groups: HashMap<GroupId, Group>,
    /// The editor groups' live layout.
    pub grid: pane_grid::State<GroupId>,
    /// Explorer | (groups / terminal).
    pub regions: pane_grid::State<Region>,
    /// The group that edits and opens go to.
    pub focused: GroupId,
    pub focus: Focus,
    pub explorer: Explorer,
    pub terminal: TerminalSpec,
    pub show_explorer: bool,
    pub show_terminal: bool,
    explorer_ratio: f32,
    editor_ratio: f32,
    pub quick_open: Option<QuickOpen>,
    /// The last file index, shown while a fresh one is built.
    pub index: Option<Arc<Vec<String>>>,
}

impl EditorView {
    /// Builds the editor from its saved session: reopens tabs (dropping files that are gone) and lists
    /// the file tree.
    pub fn start(root: &Path, session: &EditorSession) -> Self {
        let mut view = Self {
            docs: HashMap::new(),
            groups: HashMap::new(),
            grid: split::grid(&session.layout),
            regions: pane_grid::State::new(Region::Editors).0,
            focused: session.focused,
            focus: Focus::Editor,
            explorer: Explorer::default(),
            terminal: session.terminal.clone(),
            show_explorer: session.show_explorer,
            show_terminal: session.show_terminal,
            explorer_ratio: session.explorer_ratio,
            editor_ratio: session.editor_ratio,
            quick_open: None,
            index: None,
        };
        for id in session.layout.panes() {
            let mut group = Group::default();
            let spec = session.groups.get(&id).cloned().unwrap_or_default();
            for file in &spec.tabs {
                let Some(text) = view.text_for(root, &file.path) else { continue };
                let mut tab = Tab::new(file.path.clone(), &text, file.preview);
                buffer::go_to(&mut tab.view, file.line, file.column);
                group.tabs.push(tab);
            }
            group.active = spec.active.and_then(|a| group.position(&a)).or((!group.tabs.is_empty()).then_some(0));
            view.groups.insert(id, group);
        }
        view.explorer.start(root, &session.expanded);
        view.rebuild_regions();
        view
    }

    /// The text of `path`, from an open document or from disk (opening the document).
    fn text_for(&mut self, root: &Path, path: &Path) -> Option<String> {
        self.load(root, path).ok()
    }

    fn load(&mut self, root: &Path, path: &Path) -> Result<String, OpenError> {
        if let Some(text) = self.view_of(path).map(|v| v.text()) {
            return Ok(text);
        }
        let (doc, text) = Document::load(root, path)?;
        self.docs.insert(path.to_path_buf(), doc);
        Ok(text)
    }

    /// Any view of a document.
    fn view_of(&self, path: &Path) -> Option<&crate::ui::editor::code_editor::Content> {
        self.groups.values().flat_map(|g| &g.tabs).find(|t| t.path == path).map(|t| &t.view)
    }

    /// The persisted form. `terminal_cwd` is the terminal's live directory, if it's running.
    pub fn to_session(&self, terminal_cwd: Option<PathBuf>) -> EditorSession {
        let (mut explorer_ratio, mut editor_ratio) = (self.explorer_ratio, self.editor_ratio);
        read_ratios(self.regions.layout(), &mut explorer_ratio, &mut editor_ratio);
        let layout = split::tree(&self.grid);
        let groups = layout
            .panes()
            .into_iter()
            .map(|id| {
                let group = &self.groups[&id];
                let tabs = group
                    .tabs
                    .iter()
                    .map(|t| {
                        let (line, column) = buffer::line_col(&t.view);
                        OpenFile { path: t.path.clone(), line, column, preview: t.preview }
                    })
                    .collect();
                (id, GroupSpec { tabs, active: group.active_path().map(Path::to_path_buf) })
            })
            .collect();
        let mut terminal = self.terminal.clone();
        if let Some(cwd) = terminal_cwd {
            terminal.cwd = cwd;
        }
        EditorSession {
            layout,
            groups,
            focused: self.focused,
            expanded: self.explorer.expanded.iter().cloned().collect(),
            show_explorer: self.show_explorer,
            explorer_ratio,
            terminal,
            show_terminal: self.show_terminal,
            editor_ratio,
        }
    }

    // ---- queries -------------------------------------------------------------------------------

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(&id)
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.get_mut(&id)
    }

    /// Groups in reading order.
    pub fn group_ids(&self) -> Vec<GroupId> {
        split::tree(&self.grid).panes()
    }

    pub fn focused_group(&self) -> &Group {
        &self.groups[&self.focused]
    }

    pub fn doc(&self, path: &Path) -> Option<&Document> {
        self.docs.get(path)
    }

    /// The focused group's active tab and its document.
    pub fn active(&self) -> Option<(&Tab, &Document)> {
        let tab = self.focused_group().active_tab()?;
        Some((tab, self.docs.get(&tab.path)?))
    }

    /// Documents with unsaved edits.
    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.docs.values().filter(|d| d.dirty).map(|d| d.path.clone()).collect();
        paths.sort();
        paths
    }

    /// How many tabs show `path`.
    pub fn views(&self, path: &Path) -> usize {
        self.groups.values().flat_map(|g| &g.tabs).filter(|t| t.path == path).count()
    }

    pub fn can_split(&self) -> bool {
        self.groups.len() < MAX_GROUPS
    }

    /// Folders worth watching, absolute: the file tree's open folders and those of open files.
    pub fn watched_dirs(&self, root: &Path) -> BTreeSet<PathBuf> {
        let parents = self.docs.keys().map(|p| p.parent().unwrap_or(Path::new("")));
        self.explorer.watched().chain(parents).map(|d| root.join(d)).collect()
    }

    // ---- opening and closing -------------------------------------------------------------------

    /// Shows a file in `group`, opening a tab if needed. A preview tab replaces the group's previous
    /// preview; opening an already open file as non-preview pins it.
    pub fn open(&mut self, root: &Path, group: GroupId, path: &Path, preview: bool) -> Result<(), OpenError> {
        let Some(g) = self.groups.get_mut(&group) else { return Ok(()) };
        g.notice = None;
        if let Some(i) = g.position(path) {
            g.active = Some(i);
            if !preview {
                g.tabs[i].preview = false;
            }
            return Ok(());
        }
        let text = self.load(root, path).inspect_err(|err| {
            self.groups.get_mut(&group).expect("checked").notice = Some(Notice {
                path: path.to_path_buf(),
                message: format!("{} can't be opened here: {err}.", document::file_name(path)),
                external: err.openable_elsewhere(),
            });
        })?;
        let replaced =
            self.groups.get_mut(&group).expect("checked").insert(Tab::new(path.to_path_buf(), &text, preview));
        if let Some(replaced) = replaced {
            self.forget_unused(&replaced);
        }
        Ok(())
    }

    /// Closes a tab (the caller has dealt with unsaved edits).
    pub fn close_tab(&mut self, group: GroupId, index: usize) {
        let Some(tab) = self.groups.get_mut(&group).and_then(|g| g.remove(index)) else { return };
        self.forget_unused(&tab.path);
    }

    /// Drops a document no tab shows any more.
    fn forget_unused(&mut self, path: &Path) {
        if self.views(path) == 0 {
            self.docs.remove(path);
        }
    }

    /// Closes the tabs of files under `path` that have no unsaved edits (after it was deleted).
    pub fn close_clean_under(&mut self, path: &Path) {
        for id in self.group_ids() {
            loop {
                let docs = &self.docs;
                let Some(index) = self.groups[&id]
                    .tabs
                    .iter()
                    .position(|t| t.path.starts_with(path) && !docs.get(&t.path).is_some_and(|d| d.dirty))
                else {
                    break;
                };
                self.close_tab(id, index);
            }
        }
    }

    /// Follows a rename: documents, tabs and the tree move to the new path.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        let moved = |p: &Path| match p.strip_prefix(from) {
            Ok(rest) if rest.as_os_str().is_empty() => Some(to.to_path_buf()),
            Ok(rest) => Some(to.join(rest)),
            Err(_) => None,
        };
        let docs = std::mem::take(&mut self.docs);
        for (path, mut doc) in docs {
            let path = moved(&path).unwrap_or(path);
            doc.moved_to(path.clone());
            self.docs.insert(path, doc);
        }
        for tab in self.groups.values_mut().flat_map(|g| g.tabs.iter_mut()) {
            if let Some(path) = moved(&tab.path) {
                tab.path = path;
            }
        }
        self.explorer.renamed(from, to);
    }

    // ---- editing -------------------------------------------------------------------------------

    /// Runs `f` on the active tab of `group`, then replays the change in the document's other views.
    pub fn change<T>(&mut self, group: GroupId, f: impl FnOnce(&mut Buffer) -> T) -> Option<T> {
        let index = self.groups.get(&group)?.active?;
        self.change_at(group, index, f)
    }

    /// [`change`](Self::change) for a given tab.
    pub fn change_at<T>(&mut self, group: GroupId, index: usize, f: impl FnOnce(&mut Buffer) -> T) -> Option<T> {
        let path = self.groups.get(&group)?.tabs.get(index)?.path.clone();
        let mirrored = self.views(&path) > 1;
        let tab = &mut self.groups.get_mut(&group)?.tabs[index];
        let doc = self.docs.get_mut(&path)?;
        let revision = doc.revision();
        // Scrolls, clicks and cursor moves go through here too: only an edit costs a copy of the text.
        if mirrored {
            tab.view.record_edits();
        }
        let out = f(&mut Buffer { doc, view: &mut tab.view });
        // Editing a preview keeps it.
        if doc.revision() != revision {
            tab.preview = false;
        }
        if let Some(before) = tab.view.recorded()
            && let Some(edit) = TextEdit::between(&before, &tab.view.text())
        {
            for (id, g) in &mut self.groups {
                for (i, other) in g.tabs.iter_mut().enumerate() {
                    if other.path == path && !(*id == group && i == index) {
                        buffer::apply_edit(&mut other.view, &edit);
                    }
                }
            }
        }
        Some(out)
    }

    /// Where a document is shown, for edits that aren't from a particular view (disk reloads).
    fn first_view(&self, path: &Path) -> Option<(GroupId, usize)> {
        self.group_ids().into_iter().find_map(|id| Some((id, self.groups[&id].position(path)?)))
    }

    pub fn save(&mut self, root: &Path, path: &Path) -> io::Result<()> {
        let text = self.view_of(path).map(|v| v.text()).ok_or_else(|| io::Error::other("not open"))?;
        let doc = self.docs.get_mut(path).ok_or_else(|| io::Error::other("not open"))?;
        doc.save(root, &text)
    }

    /// Resolves a conflict for the disk version, as one undoable step.
    pub fn take_disk(&mut self, root: &Path, path: &Path) {
        let Some(text) = self.docs.get(path).and_then(|d| d.disk_text(root)) else { return };
        self.reload(path, text);
    }

    fn reload(&mut self, path: &Path, text: String) {
        if let Some((group, index)) = self.first_view(path) {
            self.change_at(group, index, |b| b.set_text(&text));
        }
        if let Some(doc) = self.docs.get_mut(path) {
            doc.synced(text);
        }
    }

    /// Reacts to changed paths (relative to the root): re-lists their folders and reloads, flags
    /// or marks deleted the open documents among them.
    pub fn on_disk_changed(&mut self, root: &Path, paths: &[PathBuf]) {
        let mut dirs = BTreeSet::new();
        for path in paths {
            dirs.insert(path.parent().map(Path::to_path_buf).unwrap_or_default());
            dirs.insert(path.clone());
        }
        self.explorer.refresh(root, &dirs);
        for path in paths {
            let Some(current) = self.docs.contains_key(path).then(|| self.view_of(path).map(|v| v.text())).flatten()
            else {
                continue;
            };
            let doc = self.docs.get_mut(path).expect("checked");
            if let Reload::Changed(text) = doc.on_disk_change(root, &current) {
                self.reload(path, text);
            }
        }
    }

    // ---- groups --------------------------------------------------------------------------------

    /// Splits `group`, showing its active file in the new group too, and focuses it.
    pub fn split(&mut self, root: &Path, group: GroupId, axis: Axis) -> Option<GroupId> {
        if !self.can_split() {
            return None;
        }
        let handle = split::handle(&self.grid, &group)?;
        let new = GroupId::new();
        self.grid.restore();
        self.grid.split(split::to_iced_axis(axis), handle, new)?;
        self.groups.insert(new, Group::default());
        let active = self.groups[&group].active_tab().map(|t| (t.path.clone(), t.view.cursor()));
        if let Some((path, cursor)) = active {
            let _ = self.open(root, new, &path, false);
            if let Some(tab) = self.groups.get_mut(&new).and_then(Group::active_tab_mut) {
                tab.view.move_to(cursor);
            }
        }
        self.focused = new;
        Some(new)
    }

    /// Closes a group and its tabs (the caller has dealt with unsaved edits). The last group stays,
    /// emptied.
    pub fn close_group(&mut self, group: GroupId) {
        while self.groups.get(&group).is_some_and(|g| !g.tabs.is_empty()) {
            self.close_tab(group, 0);
        }
        if self.groups.len() == 1 {
            return;
        }
        let Some(handle) = split::handle(&self.grid, &group) else { return };
        self.grid.restore();
        if let Some((_, sibling)) = self.grid.close(handle) {
            self.groups.remove(&group);
            if self.focused == group {
                self.focused = *self.grid.get(sibling).expect("sibling exists");
            }
        }
    }

    /// The group next to the focused one.
    pub fn adjacent_group(&self, direction: Direction) -> Option<GroupId> {
        split::adjacent(&self.grid, &self.focused, direction)
    }

    /// The region next to `from`, for moving focus across the explorer, groups and terminal.
    pub fn adjacent_region(&self, from: Region, direction: Direction) -> Option<Region> {
        split::adjacent(&self.regions, &from, direction)
    }

    // ---- layout --------------------------------------------------------------------------------

    pub fn set_show_explorer(&mut self, show: bool) {
        self.capture_ratios();
        self.show_explorer = show;
        self.rebuild_regions();
        if !show && self.focus == Focus::Explorer {
            self.focus = Focus::Editor;
        }
    }

    pub fn set_show_terminal(&mut self, show: bool) {
        self.capture_ratios();
        self.show_terminal = show;
        self.rebuild_regions();
        if !show && self.focus == Focus::Terminal {
            self.focus = Focus::Editor;
        }
    }

    fn capture_ratios(&mut self) {
        read_ratios(self.regions.layout(), &mut self.explorer_ratio, &mut self.editor_ratio);
    }

    fn rebuild_regions(&mut self) {
        let pane = |r| Box::new(Configuration::Pane(r));
        let column = if self.show_terminal {
            Configuration::Split {
                axis: IcedAxis::Horizontal,
                ratio: self.editor_ratio,
                a: pane(Region::Editors),
                b: pane(Region::Terminal),
            }
        } else {
            Configuration::Pane(Region::Editors)
        };
        let config = if self.show_explorer {
            Configuration::Split {
                axis: IcedAxis::Vertical,
                ratio: self.explorer_ratio,
                a: pane(Region::Explorer),
                b: Box::new(column),
            }
        } else {
            column
        };
        self.regions = pane_grid::State::with_configuration(config);
    }

    /// Resizes a region, keeping the file tree and the terminal within sensible bounds.
    pub fn resize_region(&mut self, split: pane_grid::Split, ratio: f32) {
        let (min, max) = match split_axis(self.regions.layout(), split) {
            Some(IcedAxis::Vertical) => EditorSession::EXPLORER_RANGE,
            _ => EditorSession::EDITOR_RANGE,
        };
        self.regions.resize(split, ratio.clamp(min, max));
    }

    // ---- find ----------------------------------------------------------------------------------

    /// Brings every open find bar's matches up to date.
    pub fn refresh_finds(&mut self) {
        let docs = &self.docs;
        for group in self.groups.values_mut() {
            let Some(find) = group.find.as_mut() else { continue };
            let tab = group.active.and_then(|i| group.tabs.get(i));
            find.refresh(tab.and_then(|t| Some((t.id, docs.get(&t.path)?.revision(), &t.view))));
        }
    }
}

/// Copies the live ratios out of the regions' layout.
fn read_ratios(node: &pane_grid::Node, explorer: &mut f32, editor: &mut f32) {
    if let pane_grid::Node::Split { axis, ratio, b, .. } = node {
        match axis {
            IcedAxis::Vertical => {
                *explorer = *ratio;
                read_ratios(b, explorer, editor);
            }
            IcedAxis::Horizontal => *editor = *ratio,
        }
    }
}

fn split_axis(node: &pane_grid::Node, split: pane_grid::Split) -> Option<IcedAxis> {
    match node {
        pane_grid::Node::Split { id, axis, a, b, .. } => {
            if *id == split {
                Some(*axis)
            } else {
                split_axis(a, split).or_else(|| split_axis(b, split))
            }
        }
        pane_grid::Node::Pane(_) => None,
    }
}

/// The groups' layout, for tests.
#[cfg(test)]
fn group_tree(view: &EditorView) -> pw_model::SplitTree<GroupId> {
    split::tree(&view.grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Hi\n").unwrap();
        std::fs::write(dir.path().join("logo.png"), [0u8, 159, 146]).unwrap();
        dir
    }

    fn started(root: &Path) -> EditorView {
        EditorView::start(root, &EditorSession::default())
    }

    fn tabs(view: &EditorView, group: GroupId) -> Vec<String> {
        view.group(group).unwrap().tabs.iter().map(|t| t.path.display().to_string()).collect()
    }

    #[test]
    fn previews_replace_each_other_until_pinned() {
        let dir = project();
        let mut v = started(dir.path());
        let g = v.focused;
        v.open(dir.path(), g, Path::new("src/main.rs"), true).unwrap();
        v.open(dir.path(), g, Path::new("src/lib.rs"), true).unwrap();
        assert_eq!(tabs(&v, g), vec!["src/lib.rs"]);
        assert!(v.doc(Path::new("src/main.rs")).is_none(), "replaced preview's document is dropped");
        // Editing pins it.
        v.change(g, |b| b.type_char('x'));
        v.open(dir.path(), g, Path::new("README.md"), true).unwrap();
        assert_eq!(tabs(&v, g), vec!["src/lib.rs", "README.md"]);
        assert!(!v.group(g).unwrap().tabs[0].preview);
    }

    #[test]
    fn unreadable_files_leave_a_notice() {
        let dir = project();
        let mut v = started(dir.path());
        let g = v.focused;
        assert_eq!(v.open(dir.path(), g, Path::new("logo.png"), true), Err(OpenError::Binary));
        let notice = v.group(g).unwrap().notice.clone().unwrap();
        assert!(notice.external);
        v.open(dir.path(), g, Path::new("README.md"), true).unwrap();
        assert!(v.group(g).unwrap().notice.is_none());
    }

    #[test]
    fn split_shows_the_same_document_in_two_views_kept_in_sync() {
        let dir = project();
        let mut v = started(dir.path());
        let left = v.focused;
        v.open(dir.path(), left, Path::new("src/main.rs"), false).unwrap();
        let right = v.split(dir.path(), left, Axis::Vertical).unwrap();
        assert_eq!(v.focused, right);
        assert_eq!(v.views(Path::new("src/main.rs")), 2);

        // Type in the right view at the end of line 0; the left view follows and keeps its cursor.
        v.change(right, |b| {
            buffer::go_to(b.view, 0, 12);
            b.newline();
            b.type_char('x');
        });
        let text = |v: &EditorView, g: GroupId| v.group(g).unwrap().active_tab().unwrap().view.text();
        assert_eq!(text(&v, right), "fn main() {}\nx\n");
        assert_eq!(text(&v, left), text(&v, right));
        assert!(v.doc(Path::new("src/main.rs")).unwrap().dirty);
        assert_eq!(v.dirty_paths(), vec![PathBuf::from("src/main.rs")]);

        // Undo from the left view is undo of the document (typing, then the new line), mirrored right.
        v.change(left, |b| b.undo());
        assert_eq!(text(&v, right), "fn main() {}\n\n");
        v.change(left, |b| b.undo());
        assert_eq!(text(&v, right), "fn main() {}\n");
        assert!(!v.doc(Path::new("src/main.rs")).unwrap().dirty);

        v.close_group(right);
        assert_eq!(v.group_ids(), vec![left]);
        assert_eq!(v.focused, left);
        assert_eq!(v.views(Path::new("src/main.rs")), 1);
    }

    #[test]
    fn groups_are_capped() {
        let dir = project();
        let mut v = started(dir.path());
        for _ in 1..MAX_GROUPS {
            assert!(v.split(dir.path(), v.focused, Axis::Vertical).is_some());
        }
        assert!(!v.can_split());
        assert!(v.split(dir.path(), v.focused, Axis::Vertical).is_none());
        assert_eq!(group_tree(&v).pane_count(), MAX_GROUPS);
    }

    #[test]
    fn disk_changes_reload_clean_documents_and_flag_dirty_ones() {
        let dir = project();
        let mut v = started(dir.path());
        let g = v.focused;
        v.open(dir.path(), g, Path::new("src/main.rs"), false).unwrap();
        v.open(dir.path(), g, Path::new("src/lib.rs"), false).unwrap();
        v.change(g, |b| b.type_char('!'));
        std::fs::write(dir.path().join("src/main.rs"), "fn main() { agent(); }\n").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "changed\n").unwrap();
        std::fs::write(dir.path().join("src/new.rs"), "").unwrap();
        v.explorer.expand(dir.path(), Path::new("src"));
        v.on_disk_changed(dir.path(), &["src/main.rs".into(), "src/lib.rs".into(), "src/new.rs".into()]);

        let main = &v.group(g).unwrap().tabs[0];
        assert_eq!(main.view.text(), "fn main() { agent(); }\n");
        assert!(!v.doc(Path::new("src/main.rs")).unwrap().dirty);
        assert!(v.doc(Path::new("src/lib.rs")).unwrap().conflict);
        assert!(v.explorer.rows().iter().any(|r| r.name == "new.rs"));

        v.take_disk(dir.path(), Path::new("src/lib.rs"));
        assert_eq!(v.group(g).unwrap().tabs[1].view.text(), "changed\n");
        let lib = v.doc(Path::new("src/lib.rs")).unwrap();
        assert!(!lib.dirty && !lib.conflict);
    }

    #[test]
    fn session_round_trips_tabs_cursors_and_layout() {
        let dir = project();
        let mut v = started(dir.path());
        let g = v.focused;
        v.open(dir.path(), g, Path::new("src/main.rs"), false).unwrap();
        v.change(g, |b| buffer::go_to(b.view, 0, 3));
        v.open(dir.path(), g, Path::new("README.md"), true).unwrap();
        v.split(dir.path(), g, Axis::Horizontal);
        v.explorer.expand(dir.path(), Path::new("src"));
        v.set_show_terminal(false);
        let session = v.to_session(Some("/tmp/x".into()));
        assert_eq!(session.clone().repaired(), session);
        assert_eq!(
            session.groups[&g].tabs[0],
            OpenFile { path: "src/main.rs".into(), line: 0, column: 3, preview: false }
        );
        assert!(session.groups[&g].tabs[1].preview);
        assert_eq!(session.expanded, vec![PathBuf::from("src")]);
        assert_eq!(session.terminal.cwd, PathBuf::from("/tmp/x"));
        assert!(!session.show_terminal);

        let restored = EditorView::start(dir.path(), &session);
        assert_eq!(restored.to_session(Some("/tmp/x".into())), session);
    }

    #[test]
    fn rename_moves_documents_and_tabs() {
        let dir = project();
        let mut v = started(dir.path());
        let g = v.focused;
        v.open(dir.path(), g, Path::new("src/main.rs"), false).unwrap();
        std::fs::rename(dir.path().join("src"), dir.path().join("app")).unwrap();
        v.renamed(Path::new("src"), Path::new("app"));
        assert_eq!(tabs(&v, g), vec!["app/main.rs"]);
        assert!(v.doc(Path::new("app/main.rs")).is_some());
        v.change(g, |b| b.type_char('x'));
        v.save(dir.path(), Path::new("app/main.rs")).unwrap();
        assert!(std::fs::read_to_string(dir.path().join("app/main.rs")).unwrap().starts_with('x'));
    }

    #[test]
    fn non_edits_in_a_mirrored_view_copy_nothing_and_leave_the_other_view_alone() {
        use iced::widget::text_editor::{Action, Motion};

        use crate::ui::editor::code_editor::TEXT_COPIES;

        let dir = project();
        let mut v = started(dir.path());
        let left = v.focused;
        v.open(dir.path(), left, Path::new("src/main.rs"), false).unwrap();
        v.change(left, |b| buffer::go_to(b.view, 0, 3));
        let right = v.split(dir.path(), left, Axis::Vertical).unwrap();
        let cursor = |v: &EditorView, g: GroupId| v.group(g).unwrap().active_tab().unwrap().view.cursor();
        let before = cursor(&v, left);

        TEXT_COPIES.with(|n| n.set(0));
        for action in [
            Action::Move(Motion::Right),
            Action::Select(Motion::End),
            Action::SelectAll,
            Action::Scroll { lines: 1 },
            Action::Move(Motion::DocumentEnd),
        ] {
            v.change(right, |b| b.perform(action));
        }
        assert_eq!(TEXT_COPIES.with(|n| n.get()), 0, "non-edits copy no text");
        assert_eq!(cursor(&v, left), before);
        assert_eq!(cursor(&v, right).position.line, 1);
        assert!(!v.doc(Path::new("src/main.rs")).unwrap().dirty);

        // An edit after them still mirrors.
        v.change(right, |b| b.type_char('x'));
        let text = |v: &EditorView, g: GroupId| v.group(g).unwrap().active_tab().unwrap().view.text();
        assert_eq!(text(&v, left), "fn main() {}\nx");
        assert_eq!(text(&v, left), text(&v, right));
        assert_eq!(cursor(&v, left), before);
    }
}
