//! Editor mode: the file tree, editor groups and the terminal panel, with a status bar below and
//! quick open over them.

pub mod code_editor;
pub mod explorer;
pub mod group;
pub mod highlight;
pub mod quick_open;
pub mod terminal_panel;

use iced::advanced::widget::operation::focusable;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::scrollable::AbsoluteOffset;
use iced::widget::text_editor::{Binding, KeyPress, Status};
use iced::widget::{Space, column, container, row, stack, text};
use iced::{Alignment, Element, Fill, Length, Padding, Task};
use pw_model::GroupId;

use crate::app::{App, EditorMsg, Message};
use crate::editor::{EditorView, Focus, Region};
use crate::fonts;
use crate::keymap::{self, Action, Context};
use crate::theme;

pub const QUICK_INPUT: &str = "quick-open";
pub const DRAFT_INPUT: &str = "tree-draft";
pub const TREE: &str = "file-tree";

pub fn code_id(group: GroupId) -> iced::widget::Id {
    iced::widget::Id::from(format!("code-{group:?}"))
}

pub fn find_id(group: GroupId) -> iced::widget::Id {
    iced::widget::Id::from(format!("find-{group:?}"))
}

pub fn replace_id(group: GroupId) -> iced::widget::Id {
    iced::widget::Id::from(format!("replace-{group:?}"))
}

pub fn focus_group<T: Send + 'static>(group: GroupId) -> Task<T> {
    iced::widget::operation::focus(code_id(group))
}

/// Takes the keys away from whichever widget has them (a code editor, an input).
pub fn unfocus<T: Send + 'static>() -> Task<T> {
    iced::advanced::widget::operate(focusable::unfocus())
}

pub fn focus_find<T: Send + 'static>(group: GroupId) -> Task<T> {
    let id = find_id(group);
    Task::batch([iced::widget::operation::focus(id.clone()), iced::widget::operation::select_all(id)])
}

pub fn focus_replace<T: Send + 'static>(group: GroupId) -> Task<T> {
    iced::widget::operation::focus(replace_id(group))
}

pub fn focus_quick_open<T: Send + 'static>() -> Task<T> {
    iced::widget::operation::focus(QUICK_INPUT)
}

pub fn focus_draft<T: Send + 'static>() -> Task<T> {
    Task::batch([iced::widget::operation::focus(DRAFT_INPUT), iced::widget::operation::select_all(DRAFT_INPUT)])
}

pub fn scroll_tree_to<T: Send + 'static>(offset: f32) -> Task<T> {
    iced::widget::operation::scroll_to(TREE, AbsoluteOffset { x: 0.0, y: offset })
}

/// Code editor keys: shortcuts first, then the keys that need the language's rules (pairs, Enter,
/// indenting), then iced's defaults.
pub fn binding(press: KeyPress) -> Option<Binding<Message>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    if let Some(action) = keymap::action(&press.key, press.modifiers, Context::Editor) {
        return Some(match action {
            Action::Copy => Binding::Copy,
            Action::Paste => Binding::Paste,
            action => Binding::Custom(Message::Action(action)),
        });
    }
    let m = press.modifiers;
    let editor = |msg| Some(Binding::Custom(Message::Editor(msg)));
    let plain = !m.control() && !m.logo() && !m.alt();
    match press.key.as_ref() {
        Key::Named(Named::Tab) if plain && m.shift() => editor(EditorMsg::Unindent),
        Key::Named(Named::Tab) if plain => editor(EditorMsg::Indent),
        Key::Named(Named::Enter) if plain => editor(EditorMsg::Newline),
        Key::Named(Named::Backspace) if m.is_empty() => editor(EditorMsg::Backspace),
        Key::Named(Named::Escape) => editor(EditorMsg::Escape),
        _ => match Binding::from_key_press(press)? {
            Binding::Insert(c @ ('{' | '}' | '[' | ']' | '(' | ')' | '"' | '\'' | '`')) => editor(EditorMsg::Type(c)),
            other => Some(other),
        },
    }
}

/// What has the keys in this window's Editor mode, if anything.
pub fn keys(app: &App, editor: &EditorView) -> Option<Focus> {
    (app.key_window == app.main_window && app.modal.is_none()).then_some(editor.focus)
}

pub fn view<'a>(app: &'a App, name: &'a str, editor: &'a EditorView) -> Element<'a, Message> {
    let keys = keys(app, editor);
    let regions = PaneGrid::new(&editor.regions, move |_, region, _| match region {
        Region::Explorer => pane_grid::Content::new(explorer::view(name, editor, keys == Some(Focus::Explorer)))
            .style(theme::region(theme::SURFACE, keys == Some(Focus::Explorer))),
        Region::Editors => pane_grid::Content::new(groups(app, editor, keys == Some(Focus::Editor))),
        Region::Terminal => {
            let focused = app.has_focus(editor.terminal.id);
            pane_grid::Content::new(terminal_panel::body(app, editor))
                .title_bar(terminal_panel::title_bar(app, editor, focused))
                .style(theme::pane(focused))
        }
    })
    .spacing(theme::S2)
    .on_resize(8, |e| Message::Editor(EditorMsg::RegionResized(e)))
    .style(theme::panes);

    let body =
        column![container(regions).padding(Padding::from([0.0, theme::S3])).height(Fill), status_bar(app, editor),];
    match &editor.quick_open {
        Some(quick) => stack![body, quick_open::view(quick)].into(),
        None => body.into(),
    }
}

/// The editor groups, side by side.
fn groups<'a>(app: &'a App, editor: &'a EditorView, has_keys: bool) -> Element<'a, Message> {
    PaneGrid::new(&editor.grid, move |_, &id, _| {
        let focused = has_keys && editor.focused == id;
        pane_grid::Content::new(group::body(app, editor, id))
            .title_bar(group::tab_strip(editor, id, focused))
            .style(theme::region(theme::EDITOR_BG, focused))
    })
    .spacing(theme::S2)
    .on_click(|p| Message::Editor(EditorMsg::GroupClicked(p)))
    .on_drag(|d| Message::Editor(EditorMsg::GroupDragged(d)))
    .on_resize(8, |e| Message::Editor(EditorMsg::GroupResized(e)))
    .style(theme::panes)
    .into()
}

/// The quiet line under the regions: where the cursor is and what the file is, and what's unsaved.
fn status_bar<'a>(app: &'a App, editor: &'a EditorView) -> Element<'a, Message> {
    let label = |s: String| text(s).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).wrapping(text::Wrapping::None);
    let mut left = row![].spacing(theme::S4).align_y(Alignment::Center);
    if let Some((tab, doc)) = editor.active() {
        let (line, col) = crate::editor::buffer::line_col(&tab.view);
        let selected = tab.view.selection().map_or(0, |s| s.chars().count());
        let position = match selected {
            0 => format!("Ln {}, Col {}", line + 1, col + 1),
            n => format!("Ln {}, Col {} ({n} selected)", line + 1, col + 1),
        };
        left = left
            .push(label(position))
            .push(label(doc.indent.label()))
            .push(label(if doc.crlf() { "CRLF" } else { "LF" }.to_owned()))
            .push(label("UTF-8".to_owned()))
            .push(label(doc.language.name.to_owned()));
    }
    let unsaved = app.unsaved().len();
    let right: Element<'_, Message> = if unsaved > 0 {
        row![
            container(Space::new().width(6).height(6)).style(theme::badge(theme::WARN)),
            text(format!("{unsaved} unsaved")).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_2),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        Space::new().into()
    };
    container(row![left, Space::new().width(Fill), right].align_y(Alignment::Center))
        .padding(Padding::from([0.0, theme::S4 + 2.0]))
        .height(Length::Fixed(26.0))
        .center_y(Length::Fixed(26.0))
        .into()
}
