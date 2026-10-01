//! The file tree in the app: opening, expanding, creating, renaming and deleting files.

use std::path::{Path, PathBuf};

use iced::Task;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use pw_model::editor::is_safe_relative;

use crate::app::editor::Clicked;
use crate::app::{App, Message};
use crate::editor::Focus;
use crate::editor::explorer::Draft;
use crate::ui::editor as view;
use crate::ui::modal::Modal;

#[derive(Debug, Clone)]
pub enum ExplorerMsg {
    /// A row was clicked: folders open or close, files open as a preview.
    Clicked(PathBuf),
    NewFile,
    NewFolder,
    Rename(PathBuf),
    Delete(PathBuf),
    /// The name typed into a new or renamed row.
    Draft(String),
    CommitDraft,
    Refresh,
    CollapseAll,
    /// The tree scrolled: offset and height of what's in view.
    Scrolled(f32, f32),
    /// A click on the tree's empty space.
    Focus,
}

impl App {
    pub(crate) fn on_explorer(&mut self, msg: ExplorerMsg) -> Task<Message> {
        let Some(id) = self.active else { return Task::none() };
        let task = self.explorer_update(msg);
        self.sync_watches(id);
        task
    }

    fn explorer_update(&mut self, msg: ExplorerMsg) -> Task<Message> {
        match msg {
            ExplorerMsg::Clicked(path) => {
                let double = self.double_click(Clicked::Row(path.clone()));
                let is_dir = self
                    .with_editor(|root, e| {
                        e.explorer.selected = Some(path.clone());
                        e.explorer.error = None;
                        let dir = root.join(&path).is_dir();
                        if dir {
                            e.explorer.toggle(root, &path);
                        }
                        dir
                    })
                    .unwrap_or(false);
                self.touch();
                let focus = self.set_editor_focus(Focus::Explorer);
                if is_dir {
                    return focus;
                }
                // One click previews the file and keeps the tree's keys; two keep it and start editing.
                return Task::batch([focus, self.open_file(&path, !double, double)]);
            }
            ExplorerMsg::NewFile | ExplorerMsg::NewFolder => {
                let folder = matches!(msg, ExplorerMsg::NewFolder);
                self.with_editor(|root, e| {
                    if !e.show_explorer {
                        e.set_show_explorer(true);
                    }
                    let dir = e.explorer.target_dir();
                    if !dir.as_os_str().is_empty() {
                        e.explorer.expand(root, &dir);
                    }
                    e.explorer.draft = Some((Draft::New { dir, folder }, String::new()));
                    e.explorer.error = None;
                });
                let focus = self.set_editor_focus(Focus::Explorer);
                return Task::batch([focus, view::focus_draft()]);
            }
            ExplorerMsg::Rename(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.with_editor(|_, e| {
                    e.explorer.selected = Some(path.clone());
                    e.explorer.draft = Some((Draft::Rename(path), name));
                    e.explorer.error = None;
                });
                let focus = self.set_editor_focus(Focus::Explorer);
                return Task::batch([focus, view::focus_draft()]);
            }
            ExplorerMsg::Delete(path) => {
                if let Some(workspace) = self.active {
                    self.modal = Some(Modal::ConfirmTrash { workspace, path });
                }
            }
            ExplorerMsg::Draft(text) => {
                self.with_editor(|_, e| {
                    if let Some((_, draft)) = e.explorer.draft.as_mut() {
                        *draft = text;
                    }
                });
            }
            ExplorerMsg::CommitDraft => return self.commit_draft(),
            ExplorerMsg::Refresh => {
                self.with_editor(|root, e| e.explorer.reload(root));
            }
            ExplorerMsg::CollapseAll => {
                self.with_editor(|_, e| e.explorer.collapse_all());
                self.touch();
            }
            ExplorerMsg::Scrolled(offset, height) => {
                self.with_editor(|_, e| {
                    e.explorer.scroll = offset;
                    e.explorer.viewport = height;
                });
            }
            ExplorerMsg::Focus => return self.set_editor_focus(Focus::Explorer),
        }
        Task::none()
    }

    /// Creates or renames what the draft row names.
    fn commit_draft(&mut self) -> Task<Message> {
        let Some(result) = self.with_editor(|root, e| {
            let (draft, name) = e.explorer.draft.take()?;
            let name = name.trim().to_owned();
            if name.is_empty() {
                return None;
            }
            let result = match &draft {
                Draft::New { dir, folder } => create(root, dir, &name, *folder),
                Draft::Rename(from) => rename(root, from, &name).inspect(|to| e.renamed(from, to)),
            };
            match result {
                Ok(path) => {
                    e.explorer.reload(root);
                    e.explorer.reveal(root, &path);
                    let open = matches!(draft, Draft::New { folder: false, .. });
                    Some((path, open))
                }
                Err(err) => {
                    e.explorer.error = Some(err);
                    // Keep the row so the name can be fixed.
                    e.explorer.draft = Some((draft, name));
                    None
                }
            }
        }) else {
            return Task::none();
        };
        self.touch();
        match result {
            Some((path, true)) => self.open_file(&path, false, true),
            Some(_) => self.set_editor_focus(Focus::Explorer),
            None => Task::none(),
        }
    }

    /// Moves a file or folder to the trash (after the sheet confirmed it).
    pub(crate) fn trash(&mut self, path: &Path) {
        self.with_editor(|root, e| match trash::delete(root.join(path)) {
            Ok(()) => {
                e.close_clean_under(path);
                e.explorer.selected = None;
                e.explorer.reload(root);
            }
            Err(err) => e.explorer.error = Some(format!("Couldn't move {} to the trash: {err}", path.display())),
        });
        self.touch();
    }

    /// Keys for the file tree when it has them.
    pub(crate) fn explorer_key(&mut self, key: &Key) -> Task<Message> {
        let Some(editor) = self.editor() else { return Task::none() };
        if editor.explorer.draft.is_some() {
            if *key == Key::Named(Named::Escape) {
                self.with_editor(|_, e| {
                    e.explorer.draft = None;
                    e.explorer.error = None;
                });
            }
            return Task::none();
        }
        let selected = editor.explorer.selected.clone();
        let row = selected.as_ref().and_then(|s| editor.explorer.rows().into_iter().find(|r| &r.path == s));
        match key {
            Key::Named(Named::ArrowDown) => return self.explorer_step(1),
            Key::Named(Named::ArrowUp) => return self.explorer_step(-1),
            Key::Named(Named::PageDown) => return self.explorer_step(20),
            Key::Named(Named::PageUp) => return self.explorer_step(-20),
            Key::Named(Named::ArrowRight) => {
                if let Some(row) = row.filter(|r| r.is_dir && !r.expanded) {
                    self.with_editor(|root, e| e.explorer.toggle(root, &row.path));
                    self.touch();
                }
            }
            Key::Named(Named::ArrowLeft) => match row {
                Some(row) if row.is_dir && row.expanded => {
                    self.with_editor(|root, e| e.explorer.toggle(root, &row.path));
                    self.touch();
                }
                Some(row) => {
                    let parent = row.path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
                    if parent.is_some() {
                        self.with_editor(|_, e| e.explorer.selected = parent);
                    }
                }
                None => {}
            },
            Key::Named(Named::Enter) => match row {
                Some(row) if row.is_dir => {
                    self.with_editor(|root, e| e.explorer.toggle(root, &row.path));
                    self.touch();
                }
                Some(row) => return self.open_file(&row.path, false, true),
                None => {}
            },
            Key::Named(Named::Space) => {
                if let Some(row) = row.filter(|r| !r.is_dir) {
                    return self.open_file(&row.path, true, false);
                }
            }
            Key::Named(Named::F2) => {
                if let Some(path) = selected {
                    return self.on_explorer(ExplorerMsg::Rename(path));
                }
            }
            Key::Named(Named::Delete) => {
                if let Some(path) = selected {
                    return self.on_explorer(ExplorerMsg::Delete(path));
                }
            }
            Key::Named(Named::Escape) => return self.set_editor_focus(Focus::Editor),
            _ => {}
        }
        Task::none()
    }

    fn explorer_step(&mut self, delta: isize) -> Task<Message> {
        let scroll = self.with_editor(|_, e| {
            e.explorer.step(delta);
            e.explorer.scroll
        });
        scroll.map_or_else(Task::none, view::scroll_tree_to)
    }
}

/// Creates `name` (which may contain folders) in `dir`.
fn create(root: &Path, dir: &Path, name: &str, folder: bool) -> Result<PathBuf, String> {
    let path = dir.join(name.trim_matches('/'));
    if !is_safe_relative(&path) {
        return Err(format!("“{name}” isn't a valid name."));
    }
    let full = root.join(&path);
    if full.exists() {
        return Err(format!("{} already exists.", path.display()));
    }
    let result = if folder {
        std::fs::create_dir_all(&full)
    } else {
        full.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::OpenOptions::new().write(true).create_new(true).open(&full).map(drop))
    };
    match result {
        Ok(()) => Ok(path),
        Err(e) => Err(format!("Couldn't create {}: {e}", path.display())),
    }
}

/// Renames `from` to `name` in the same folder.
fn rename(root: &Path, from: &Path, name: &str) -> Result<PathBuf, String> {
    let to = from.parent().unwrap_or(Path::new("")).join(name);
    if !is_safe_relative(&to) || name.contains('/') {
        return Err(format!("“{name}” isn't a valid name."));
    }
    if to == from {
        return Ok(to);
    }
    if root.join(&to).exists() {
        return Err(format!("{} already exists.", to.display()));
    }
    std::fs::rename(root.join(from), root.join(&to))
        .map(|()| to)
        .map_err(|e| format!("Couldn't rename {}: {e}", from.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_files_and_folders_safely() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(create(root, Path::new(""), "src/a.rs", false), Ok(PathBuf::from("src/a.rs")));
        assert!(root.join("src/a.rs").is_file());
        assert!(create(root, Path::new("src"), "a.rs", false).unwrap_err().contains("already exists"));
        assert!(create(root, Path::new(""), "../escape", false).unwrap_err().contains("valid"));
        assert_eq!(create(root, Path::new("src"), "ui", true), Ok(PathBuf::from("src/ui")));
        assert!(root.join("src/ui").is_dir());
        assert_eq!(rename(root, Path::new("src/a.rs"), "b.rs"), Ok(PathBuf::from("src/b.rs")));
        assert!(rename(root, Path::new("src/b.rs"), "x/y.rs").is_err());
        assert!(rename(root, Path::new("src/b.rs"), "ui").unwrap_err().contains("already exists"));
    }
}
