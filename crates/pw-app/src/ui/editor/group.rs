//! An editor group: its tab strip, notes about the file, the code editor and the find card, or a
//! welcome when no file is open.

use iced::font::Style as FontStyle;
use iced::widget::pane_grid;
use iced::widget::text::{LineHeight, Wrapping};
use iced::widget::{
    Space, button, center, column, container, hover, opaque, row, scrollable, stack, svg, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Font, Length, Padding};
use pw_code::{Language, syntax};
use pw_model::GroupId;

use super::code_editor::CodeEditor;
use super::highlight::{self, CodeHighlighter};
use super::{binding, find_id, replace_id};
use crate::app::{App, EditorMsg, Message};
use crate::editor::EditorView;
use crate::editor::document::{Document, file_name};
use crate::editor::find::Find;
use crate::editor::group::{Group, Tab};
use crate::fonts;
use crate::icons;
use crate::theme;
use crate::ui::{icon_button, tip_for};

fn msg(m: EditorMsg) -> Message {
    Message::Editor(m)
}

// ---- tab strip ---------------------------------------------------------------------------------

pub fn tab_strip<'a>(editor: &'a EditorView, id: GroupId, focused: bool) -> pane_grid::TitleBar<'a, Message> {
    let group = editor.group(id).expect("group in grid");
    let tabs = row(group.tabs.iter().enumerate().map(|(i, tab)| tab_button(editor, group, id, i, tab, focused)))
        .spacing(1)
        .align_y(Alignment::End);

    let mut controls = row![].spacing(0).align_y(Alignment::Center);
    controls = controls.push(icon_button(
        &icons::SPLIT_RIGHT,
        13.0,
        "Split editor",
        editor.can_split().then_some(msg(EditorMsg::SplitGroup(id))),
    ));
    if editor.group_ids().len() > 1 {
        controls = controls.push(icon_button(&icons::CLOSE, 13.0, "Close group", Some(msg(EditorMsg::CloseGroup(id)))));
    }

    let tabs = scrollable(container(tabs).height(Length::Fixed(theme::TAB_HEIGHT)).padding(Padding::default().left(4)))
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(0).scroller_width(0)))
        .width(Fill);
    // The controls sit in the title itself, not in pane_grid's `controls`: a title bar hides a title that doesn't
    // fit beside its controls, and a strip that fills the bar never does, so the tabs vanished on hover.
    pane_grid::TitleBar::new(
        row![tabs, container(controls).height(theme::TAB_HEIGHT).center_y(theme::TAB_HEIGHT)]
            .align_y(Alignment::Center),
    )
    .padding(Padding::default().right(4))
    .style(theme::tab_strip)
}

fn tab_button<'a>(
    editor: &'a EditorView,
    group: &'a Group,
    id: GroupId,
    index: usize,
    tab: &'a Tab,
    group_focused: bool,
) -> Element<'a, Message> {
    let active = group.active == Some(index);
    let doc = editor.doc(&tab.path);
    let language = doc.map_or_else(|| Language::detect(&tab.path, None), |d| d.language);
    let name = file_name(&tab.path);
    let mut label = row![
        svg(icons::FILE.clone()).width(14).height(14).style(theme::icon(theme::tint(language.tint))),
        // A preview is italic where the font has an italic face. Inter's isn't bundled, so it's also quieter: an
        // active preview reads like a hovered tab, not a kept one.
        text(name.clone())
            .size(theme::T_SM)
            .font(Font {
                style: if tab.preview { FontStyle::Italic } else { FontStyle::Normal },
                ..if active && !tab.preview { fonts::UI_MEDIUM } else { fonts::UI }
            })
            .color_maybe((tab.preview && active).then_some(theme::FG_2))
            .wrapping(Wrapping::None),
    ]
    .spacing(7)
    .align_y(Alignment::Center);
    // Two tabs with the same name: say which folder each is in.
    if let Some(hint) = group.folder_hint(index) {
        label = label.push(text(hint).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).wrapping(Wrapping::None));
    }

    let marker: Element<'_, Message> = match doc {
        Some(d) if d.conflict => dot(theme::BAD),
        Some(d) if d.dirty => dot(theme::FG_2),
        _ if active => close_for(id, index),
        _ => Space::new().width(18).height(18).into(),
    };
    // A dirty mark turns into the close button on hover, as does the empty slot.
    let slot = hover(
        container(marker).center(Length::Fixed(18.0)),
        container(close_for(id, index)).center(Length::Fixed(18.0)),
    );

    let body = button(row![label, slot].spacing(6).align_y(Alignment::Center))
        .padding(Padding::from([0.0, 8.0]).left(12.0))
        .height(Length::Fixed(theme::TAB_HEIGHT - 2.0))
        .style(theme::tab(active))
        .on_press(msg(EditorMsg::SelectTab(id, index)));
    let mark: Element<'_, Message> = if active && group_focused {
        container(Space::new().width(Fill).height(2)).style(theme::tab_mark(theme::ACCENT)).into()
    } else {
        Space::new().height(2).into()
    };
    column![mark, body].width(Length::Shrink).into()
}

fn close_for<'a>(id: GroupId, index: usize) -> Element<'a, Message> {
    button(svg(icons::CLOSE.clone()).width(12).height(12).style(theme::icon_hover(theme::FG_2, theme::FG)))
        .padding(3)
        .style(theme::ghost_button)
        .on_press(msg(EditorMsg::CloseTab(id, index)))
        .into()
}

fn dot<'a>(color: Color) -> Element<'a, Message> {
    container(Space::new().width(8).height(8)).style(theme::badge(color)).into()
}

// ---- body --------------------------------------------------------------------------------------

pub fn body<'a>(app: &'a App, editor: &'a EditorView, id: GroupId) -> Element<'a, Message> {
    let group = editor.group(id).expect("group in grid");
    let mut content = column![];
    if let Some(notice) = &group.notice {
        let mut actions = row![].spacing(theme::S1).align_y(Alignment::Center);
        if notice.external {
            actions =
                actions.push(small_button("Open in default app", msg(EditorMsg::OpenExternally(notice.path.clone()))));
        }
        actions = actions.push(icon_button(&icons::CLOSE, 12.0, "Dismiss", Some(msg(EditorMsg::DismissNotice(id)))));
        content = content.push(banner(&notice.message, theme::FG_2, theme::SURFACE_2, Some(actions.into())));
    }
    let Some((tab, doc)) = group.active_tab().and_then(|t| Some((t, editor.doc(&t.path)?))) else {
        return content.push(welcome()).height(Fill).into();
    };
    if doc.conflict {
        let actions = row![
            small_button("Keep mine", msg(EditorMsg::KeepMine)),
            small_button("Use disk version", msg(EditorMsg::TakeDisk))
        ]
        .spacing(theme::S1);
        content = content.push(banner(
            &format!("{} changed on disk while you were editing it.", doc.name()),
            theme::WARN,
            theme::WARN_SOFT,
            Some(actions.into()),
        ));
    } else if doc.deleted {
        content = content.push(banner(
            &format!("{} was deleted on disk. Saving recreates it.", doc.name()),
            theme::WARN,
            theme::WARN_SOFT,
            None,
        ));
    }
    let code = source(app, id, tab, doc, group.find.as_ref());
    let code: Element<'_, Message> = match &group.find {
        Some(find) => stack![
            code,
            container(opaque(find_card(id, find))).align_right(Fill).padding(Padding::from([8.0, 22.0]).left(0.0)),
        ]
        .into(),
        None => code,
    };
    content.push(code).height(Fill).into()
}

fn source<'a>(
    app: &'a App,
    id: GroupId,
    tab: &'a Tab,
    doc: &'a Document,
    find: Option<&'a Find>,
) -> Element<'a, Message> {
    let (found, current): (&[_], _) = match find {
        Some(find) => (&find.matches, find.current),
        None => (&[], None),
    };
    container(
        CodeEditor::<CodeHighlighter, Message>::new(
            &tab.view,
            move |action| Message::Editor(EditorMsg::Edit(id, action)),
            highlight::Settings {
                syntax: syntax::token_for(doc.language.syntax, tab.view.line_count()),
                tab: tab.id,
                parse: tab.parse.clone(),
            },
            highlight::format,
            theme::code_editor(),
        )
        .id(super::code_id(id))
        .font(fonts::MONO)
        .size(app.prefs.editor_font_size)
        .line_height(LineHeight::Relative(1.55))
        .wrapping(Wrapping::WordOrGlyph)
        .language(doc.language)
        .found(found, current)
        .key_binding(binding),
    )
    .padding(Padding::default().bottom(4))
    .into()
}

fn small_button<'a>(label: &'a str, on_press: Message) -> Element<'a, Message> {
    button(text(label).size(theme::T_XS).font(fonts::UI_SEMIBOLD))
        .padding([3, 8])
        .style(theme::secondary_button)
        .on_press(on_press)
        .into()
}

fn banner<'a>(message: &str, color: Color, soft: Color, actions: Option<Element<'a, Message>>) -> Element<'a, Message> {
    let mut content = row![
        svg(icons::WARNING.clone()).width(14).height(14).style(theme::icon(color)),
        container(text(message.to_owned()).size(theme::T_SM).font(fonts::UI).color(theme::FG_2)).width(Fill),
    ]
    .spacing(theme::S2)
    .align_y(Alignment::Center);
    if let Some(actions) = actions {
        content = content.push(actions);
    }
    container(container(content).padding([6, 10]).style(theme::note(color, soft)))
        .padding(Padding::from([8.0, 10.0]).bottom(0.0))
        .into()
}

/// The floating find (and replace) card at the editor's top right.
fn find_card(id: GroupId, find: &Find) -> Element<'_, Message> {
    let count = match (find.matches.len(), find.current) {
        _ if find.query.is_empty() => String::new(),
        (0, _) => "No results".to_owned(),
        (n, Some(i)) => format!("{} of {n}", i + 1),
        (n, None) => format!("{n} found"),
    };
    let count_color = if !find.query.is_empty() && find.matches.is_empty() { theme::BAD } else { theme::FG_3 };
    let toggle = button(
        svg(if find.show_replace { icons::CHEVRON_DOWN.clone() } else { icons::CHEVRON_RIGHT.clone() })
            .width(13)
            .height(13)
            .style(theme::icon_hover(theme::FG_3, theme::FG)),
    )
    .padding([6, 3])
    .style(theme::ghost_button)
    .on_press(msg(EditorMsg::FindShowReplace(!find.show_replace)));
    let case = tip_for(
        button(text("Aa").size(theme::T_XS).font(fonts::UI_SEMIBOLD))
            .padding([3, 6])
            .style(theme::toggle_button(find.case_sensitive))
            .on_press(msg(EditorMsg::FindCase(!find.case_sensitive)))
            .into(),
        "Match case",
    );
    let has_matches = !find.matches.is_empty();
    let query = row![
        toggle,
        text_input("Find", &find.query)
            .id(find_id(id))
            .on_input(|q| msg(EditorMsg::FindQuery(q)))
            .on_submit(msg(EditorMsg::FindNext))
            .size(theme::T_SM)
            .font(fonts::UI)
            .padding([5, 8])
            .style(theme::input)
            .width(Fill),
        case,
        container(text(count).size(theme::T_XS).font(fonts::UI).color(count_color)).width(64).align_right(64),
        icon_button(&icons::ARROW_UP, 14.0, "Previous match", has_matches.then(|| msg(EditorMsg::FindPrev))),
        icon_button(&icons::ARROW_DOWN, 14.0, "Next match", has_matches.then(|| msg(EditorMsg::FindNext))),
        icon_button(&icons::CLOSE, 13.0, "Close", Some(msg(EditorMsg::CloseFind))),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let mut card = column![query].spacing(6);
    if find.show_replace {
        let action = |label, m: Option<EditorMsg>| {
            button(text(label).size(theme::T_XS).font(fonts::UI_SEMIBOLD))
                .padding([5, 9])
                .style(theme::secondary_button)
                .on_press_maybe(m.map(msg))
        };
        card = card.push(
            row![
                Space::new().width(19),
                text_input("Replace", &find.replacement)
                    .id(replace_id(id))
                    .on_input(|r| msg(EditorMsg::FindReplacement(r)))
                    .on_submit(msg(EditorMsg::ReplaceOne))
                    .size(theme::T_SM)
                    .font(fonts::UI)
                    .padding([5, 8])
                    .style(theme::input)
                    .width(Fill),
                action("Replace", find.current.map(|_| EditorMsg::ReplaceOne)),
                action("All", has_matches.then_some(EditorMsg::ReplaceAll)),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    container(card).padding(6).width(430).style(theme::floating_panel).into()
}

/// An empty group: what to do next, with the keys for it.
fn welcome<'a>() -> Element<'a, Message> {
    let hints = column(crate::keymap::editor_hints().into_iter().map(|(label, keys, action)| {
        button(
            row![
                text(label).size(theme::T_SM).font(fonts::UI).color(theme::FG_2),
                Space::new().width(Fill),
                container(text(keys).size(theme::T_XS).font(fonts::MONO)).padding([2, 7]).style(theme::keycap),
            ]
            .align_y(Alignment::Center),
        )
        .width(280)
        .padding([5, 8])
        .style(theme::ghost_button)
        .on_press(Message::Action(action))
        .into()
    }))
    .spacing(2);
    center(
        column![
            container(svg(icons::CODE.clone()).width(24).height(24).style(theme::icon(theme::ACCENT)))
                .center(Length::Fixed(48.0))
                .style(theme::count_badge(true)),
            text("No file open").size(theme::T_LG).font(fonts::UI_SEMIBOLD).color(theme::FG),
            text("Pick a file in the tree, or find one by name.").size(theme::T_MD).font(fonts::UI).color(theme::FG_3),
            Space::new().height(theme::S2),
            hints,
        ]
        .spacing(theme::S2)
        .align_x(Alignment::Center),
    )
    .into()
}
