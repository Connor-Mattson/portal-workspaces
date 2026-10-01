//! The file tree: the project's name with its actions, then the rows in view.
//!
//! Rows have a fixed height, so only those near the visible part of the scroll are built; the rest
//! of the list is two spacers. A huge expanded folder costs nothing until you scroll to it.

use std::path::Path;

use iced::widget::{Space, button, column, container, hover, mouse_area, row, scrollable, svg, text, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};
use pw_code::Language;

use super::{DRAFT_INPUT, TREE};
use crate::app::{ExplorerMsg, Message};
use crate::editor::EditorView;
use crate::editor::explorer::{Draft, ROW_HEIGHT, Row};
use crate::fonts;
use crate::icons;
use crate::theme;
use crate::ui::icon_button;

/// Indentation per level.
const INDENT: f32 = 12.0;
/// Rows built beyond the visible part, above and below, and the height assumed before the tree
/// reports its size.
const OVERSCAN: f32 = 600.0;
const MIN_VIEWPORT: f32 = 1600.0;

fn msg(m: ExplorerMsg) -> Message {
    Message::Explorer(m)
}

enum Item<'a> {
    Row(Row),
    /// The input for a new file or folder's name, at a depth.
    Draft {
        depth: usize,
        folder: bool,
        value: &'a str,
    },
}

pub fn view<'a>(name: &'a str, editor: &'a EditorView, focused: bool) -> Element<'a, Message> {
    let explorer = &editor.explorer;
    let header = row![
        text(name.to_uppercase())
            .size(theme::T_XS)
            .font(fonts::UI_SEMIBOLD)
            .color(theme::FG_3)
            .wrapping(text::Wrapping::None),
        Space::new().width(Fill),
        icon_button(&icons::FILE_PLUS, 14.0, "New file", Some(msg(ExplorerMsg::NewFile))),
        icon_button(&icons::FOLDER_PLUS, 14.0, "New folder", Some(msg(ExplorerMsg::NewFolder))),
        icon_button(&icons::REFRESH, 13.0, "Refresh", Some(msg(ExplorerMsg::Refresh))),
        icon_button(&icons::COLLAPSE, 14.0, "Collapse folders", Some(msg(ExplorerMsg::CollapseAll))),
    ]
    .spacing(0)
    .align_y(Alignment::Center);

    // The rows, with the draft row in its place.
    let mut items: Vec<Item<'_>> = Vec::new();
    let draft = explorer.draft.as_ref();
    let rows = explorer.rows();
    if let Some((Draft::New { dir, folder }, value)) = draft
        && dir.as_os_str().is_empty()
    {
        items.push(Item::Draft { depth: 0, folder: *folder, value });
    }
    for row in rows {
        let renaming = matches!(draft, Some((Draft::Rename(p), _)) if *p == row.path);
        let new_here = match draft {
            Some((Draft::New { dir, folder }, value)) if *dir == row.path => Some((*folder, value.as_str())),
            _ => None,
        };
        let depth = row.depth;
        if renaming {
            let value = draft.map_or("", |(_, v)| v.as_str());
            items.push(Item::Draft { depth, folder: row.is_dir, value });
        } else {
            items.push(Item::Row(row));
        }
        if let Some((folder, value)) = new_here {
            items.push(Item::Draft { depth: depth + 1, folder, value });
        }
    }

    // Only the rows near the view are built.
    let total = items.len();
    let viewport = explorer.viewport.max(MIN_VIEWPORT);
    let first = ((explorer.scroll - OVERSCAN) / ROW_HEIGHT).floor().max(0.0) as usize;
    let last = (((explorer.scroll + viewport + OVERSCAN) / ROW_HEIGHT).ceil() as usize).min(total);
    let first = first.min(last);
    let active = editor.focused_group().active_path();
    let mut list = column![Space::new().height(first as f32 * ROW_HEIGHT)];
    for item in items.drain(first..last) {
        list = list.push(match item {
            Item::Row(row) => row_view(editor, row, active, focused),
            Item::Draft { depth, folder, value } => draft_row(depth, folder, value),
        });
    }
    list = list.push(Space::new().height((total - last) as f32 * ROW_HEIGHT));

    let tree = scrollable(container(list).padding(Padding::from([0.0, 6.0])))
        .id(TREE)
        .on_scroll(|v| msg(ExplorerMsg::Scrolled(v.absolute_offset().y, v.bounds().height)))
        .height(Fill)
        .style(theme::scroller);
    let mut content = column![
        container(header).padding(Padding::from([6.0, 6.0]).left(theme::S3)),
        mouse_area(tree).on_press(msg(ExplorerMsg::Focus)),
    ];
    let error = explorer
        .error
        .clone()
        .or_else(|| explorer.listing_error(Path::new("")).map(|e| format!("Can't read this folder: {e}")));
    if let Some(error) = error {
        content = content.push(
            container(
                container(text(error).size(theme::T_XS).font(fonts::UI))
                    .padding([5, 8])
                    .width(Fill)
                    .style(theme::error_note),
            )
            .padding(6),
        );
    }
    content.height(Fill).into()
}

fn row_view<'a>(editor: &'a EditorView, row: Row, active: Option<&Path>, focused: bool) -> Element<'a, Message> {
    let selected = editor.explorer.selected.as_deref() == Some(row.path.as_path());
    let is_active = active == Some(row.path.as_path());
    let dirty = !row.is_dir && editor.doc(&row.path).is_some_and(|d| d.dirty);
    let chevron: Element<'_, Message> = if row.is_dir {
        svg(if row.expanded { icons::CHEVRON_DOWN.clone() } else { icons::CHEVRON_RIGHT.clone() })
            .width(12)
            .height(12)
            .style(theme::icon(theme::FG_3))
            .into()
    } else {
        Space::new().width(12).into()
    };
    let (icon, tint) = if row.is_dir {
        (if row.expanded { icons::FOLDER_OPEN.clone() } else { icons::FOLDER.clone() }, theme::FG_3)
    } else {
        (icons::FILE.clone(), theme::tint(Language::detect(&row.path, None).tint))
    };
    let dim = |c: iced::Color| if row.ignored { theme::alpha(c, 0.45) } else { c };
    let name_color = if is_active { theme::FG } else { theme::FG_2 };
    let mut line = row![
        Space::new().width(row.depth as f32 * INDENT),
        chevron,
        svg(icon).width(14).height(14).style(theme::icon(dim(tint))),
        text(row.name.clone())
            .size(theme::T_SM)
            .font(if is_active { fonts::UI_MEDIUM } else { fonts::UI })
            .color(dim(name_color))
            .wrapping(text::Wrapping::None),
        Space::new().width(Fill),
    ]
    .spacing(5)
    .align_y(Alignment::Center);
    if dirty {
        line = line.push(container(Space::new().width(7).height(7)).style(theme::badge(theme::FG_2)));
    }
    let base = button(
        container(line).height(Length::Fixed(ROW_HEIGHT - 2.0)).center_y(Length::Fixed(ROW_HEIGHT - 2.0)).clip(true),
    )
    .padding(Padding::from([0.0, 6.0]))
    .width(Fill)
    .style(theme::tree_row(selected, focused))
    .on_press(msg(ExplorerMsg::Clicked(row.path.clone())));
    let actions = container(
        row![
            icon_button(&icons::PENCIL, 12.0, "Rename", Some(msg(ExplorerMsg::Rename(row.path.clone())))),
            icon_button(&icons::TRASH, 12.0, "Move to Trash", Some(msg(ExplorerMsg::Delete(row.path.clone())))),
        ]
        .spacing(0),
    )
    .align_right(Fill)
    .center_y(Fill)
    .padding(Padding::default().right(2.0));
    container(hover(base, actions)).height(Length::Fixed(ROW_HEIGHT)).center_y(Length::Fixed(ROW_HEIGHT)).into()
}

fn draft_row<'a>(depth: usize, folder: bool, value: &'a str) -> Element<'a, Message> {
    let icon = if folder { icons::FOLDER.clone() } else { icons::FILE.clone() };
    container(
        row![
            Space::new().width(depth as f32 * INDENT + 12.0),
            svg(icon).width(14).height(14).style(theme::icon(theme::ACCENT)),
            text_input(if folder { "folder name" } else { "file name" }, value)
                .id(DRAFT_INPUT)
                .on_input(|v| msg(ExplorerMsg::Draft(v)))
                .on_submit(msg(ExplorerMsg::CommitDraft))
                .size(theme::T_SM)
                .font(fonts::UI)
                .padding([2, 6])
                .style(theme::input),
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([0.0, 6.0]))
    .height(Length::Fixed(ROW_HEIGHT))
    .center_y(Length::Fixed(ROW_HEIGHT))
    .into()
}
