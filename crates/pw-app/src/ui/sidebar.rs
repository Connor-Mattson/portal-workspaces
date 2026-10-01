//! The workspace drawer on the left: full list, or a slim rail when collapsed.

use iced::widget::{Space, button, column, container, hover, row, scrollable, svg, text, tooltip};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::keymap::Action;
use crate::theme;
use crate::ui::{icon_button, status, system, tildify, usage};
use crate::workspace::WorkspaceView;

fn monogram(name: &str, active: bool) -> Element<'_, Message> {
    let letters: String = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let letters = if letters.is_empty() { "·".to_owned() } else { letters };
    container(text(letters).size(theme::T_SM).font(fonts::UI_SEMIBOLD))
        .center(Length::Fixed(30.0))
        .style(theme::monogram(active))
        .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    if app.prefs.sidebar_collapsed { rail(app) } else { drawer(app) }
}

fn drawer(app: &App) -> Element<'_, Message> {
    let brand = row![
        container(svg(icons::TERMINAL.clone()).width(16).height(16).style(theme::icon(theme::ON_ACCENT)))
            .center(Length::Fixed(26.0))
            .style(theme::monogram(true)),
        column![
            text("Portal").size(theme::T_MD).font(fonts::UI_SEMIBOLD).color(theme::FG),
            text("Workspaces").size(theme::T_XS).font(fonts::UI).color(theme::FG_3),
        ]
        .spacing(0),
        Space::new().width(Fill),
        icon_button(&icons::SIDEBAR, 16.0, "Collapse drawer", Some(Message::Action(Action::ToggleSidebar))),
    ]
    .spacing(theme::S2 + 2.0)
    .align_y(Alignment::Center);

    let header = row![
        text("WORKSPACES").size(theme::T_XS).font(fonts::UI_SEMIBOLD).color(theme::FG_3),
        Space::new().width(Fill),
        icon_button(&icons::PLUS, 15.0, "New workspace", Some(Message::Action(Action::NewWorkspace))),
    ]
    .align_y(Alignment::Center)
    .padding(Padding::default().left(theme::S2));

    let items = column(app.workspaces.iter().enumerate().map(|(i, ws)| item(app, ws, i))).spacing(2);

    let footer = row![
        button(
            row![
                svg(icons::KEYBOARD.clone()).width(15).height(15).style(theme::icon_hover(theme::FG_3, theme::FG)),
                text("Shortcuts").size(theme::T_SM).font(fonts::UI),
            ]
            .spacing(theme::S2)
            .align_y(Alignment::Center)
        )
        .padding([6, 8])
        .style(theme::ghost_button)
        .on_press(Message::Action(Action::ShowShortcuts)),
        Space::new().width(Fill),
        icon_button(
            if app.prefs.notifications { &icons::BELL } else { &icons::BELL_OFF },
            15.0,
            if app.prefs.notifications { "Desktop notifications: on" } else { "Desktop notifications: off" },
            Some(Message::ToggleNotifications),
        ),
        text(concat!("v", env!("CARGO_PKG_VERSION"))).size(theme::T_XS).font(fonts::UI).color(theme::LINE_STRONG),
    ]
    .spacing(theme::S1)
    .align_y(Alignment::Center);

    container(
        column![
            container(brand).padding(Padding::from([theme::S4, theme::S3]).bottom(theme::S3)),
            container(header).padding([0.0, theme::S3]),
            scrollable(container(items).padding([theme::S1, theme::S2])).height(Fill).style(theme::scroller),
            container(usage::section(app)).padding([0.0, theme::S2]),
            container(system::section(app)).padding([0.0, theme::S2]),
            container(footer).padding([theme::S2, theme::S2]),
        ]
        .spacing(theme::S1),
    )
    .width(theme::SIDEBAR_WIDTH)
    .height(Fill)
    .style(theme::sidebar)
    .into()
}

fn item<'a>(app: &'a App, ws: &'a WorkspaceView, index: usize) -> Element<'a, Message> {
    let active = app.active == Some(ws.id());
    let count = ws.pane_count();
    let unsaved = ws.editor.as_ref().is_some_and(|e| !e.dirty_paths().is_empty());
    let mut name = row![
        text(&ws.model.name)
            .size(theme::T_MD)
            .font(if active { fonts::UI_SEMIBOLD } else { fonts::UI_MEDIUM })
            .wrapping(text::Wrapping::None),
    ]
    .spacing(5)
    .align_y(Alignment::Center);
    if unsaved {
        name = name.push(tooltip(
            svg(icons::PENCIL.clone()).width(10).height(10).style(theme::icon(theme::FG_3)),
            container(text("Unsaved edits").size(theme::T_SM).font(fonts::UI).color(theme::FG_2))
                .padding([4, 8])
                .style(theme::keycap),
            tooltip::Position::Bottom,
        ));
    }
    // What it wants replaces the path, with what the agent said: the reason to look.
    let wants = app.workspace_status(ws);
    let subtitle = match (wants.kind, wants.summary()) {
        (Some(kind), Some(summary)) => {
            let line = match &wants.message {
                Some(message) if wants.count == 1 => format!("{summary} · {message}"),
                _ => summary,
            };
            text(line).color(status::color(kind)).font(fonts::UI_MEDIUM)
        }
        _ => text(tildify(&ws.model.root)).color(theme::FG_3).font(fonts::UI),
    };
    let label = column![name, subtitle.size(theme::T_XS).wrapping(text::Wrapping::None)].spacing(1);
    let label: Element<'a, Message> = container(label).width(Fill).clip(true).into();
    // The row is narrow; hovering shows everything the agent said.
    let label = match wants.message.clone().filter(|_| wants.count == 1) {
        Some(message) => tooltip(
            label,
            container(text(message).size(theme::T_SM).font(fonts::UI).color(theme::FG_2))
                .max_width(360)
                .padding([4, 8])
                .style(theme::keycap),
            tooltip::Position::Bottom,
        )
        .gap(4)
        .into(),
        None => label,
    };

    let badge = container(text(count.to_string()).size(theme::T_XS).font(fonts::UI_SEMIBOLD))
        .padding([1, 7])
        .style(theme::count_badge(active));

    let base = button(
        row![monogram(&ws.model.name, active), label, status::dot(&wants, 8.0), badge]
            .spacing(theme::S2 + 2.0)
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding([7, 8])
    .style(theme::nav_item(active))
    .on_press(Message::SelectWorkspace(ws.id()));

    // Hover reveals reorder and edit controls over the right edge of the row.
    let first = index == 0;
    let last = index + 1 == app.workspaces.len();
    let controls = container(
        container(
            row![
                icon_button(
                    &icons::CHEVRON_UP,
                    14.0,
                    "Move up",
                    (!first).then_some(Message::MoveWorkspace(ws.id(), -1))
                ),
                icon_button(
                    &icons::CHEVRON_DOWN,
                    14.0,
                    "Move down",
                    (!last).then_some(Message::MoveWorkspace(ws.id(), 1))
                ),
                icon_button(&icons::PENCIL, 14.0, "Edit workspace", Some(Message::EditWorkspace(ws.id()))),
            ]
            .spacing(0),
        )
        .padding(2)
        .style(theme::keycap),
    )
    .align_right(Fill)
    .center_y(Fill)
    .padding(Padding::default().right(6.0));

    hover(base, controls)
}

fn rail(app: &App) -> Element<'_, Message> {
    let items = column(app.workspaces.iter().map(|ws| {
        let active = app.active == Some(ws.id());
        let wants = app.workspace_status(ws);
        let mark = iced::widget::stack![
            monogram(&ws.model.name, active),
            container(status::dot(&wants, 8.0)).align_right(Length::Fixed(33.0)).align_top(Length::Fixed(33.0)),
        ];
        let btn = button(mark).padding(4).style(theme::nav_item(false)).on_press(Message::SelectWorkspace(ws.id()));
        let mut tip = column![text(&ws.model.name).size(theme::T_SM).font(fonts::UI_MEDIUM).color(theme::FG)];
        if let (Some(kind), Some(summary)) = (wants.kind, wants.summary()) {
            tip = tip.push(text(summary).size(theme::T_XS).font(fonts::UI_MEDIUM).color(status::color(kind)));
        }
        tooltip(btn, container(tip.spacing(1)).padding([4, 8]).style(theme::keycap), tooltip::Position::Right)
            .gap(8)
            .into()
    }))
    .spacing(theme::S1)
    .align_x(Alignment::Center);

    container(
        column![
            icon_button(&icons::SIDEBAR, 16.0, "Expand drawer", Some(Message::Action(Action::ToggleSidebar))),
            scrollable(items).height(Fill).style(theme::scroller),
            usage::rail(app),
            system::rail(app),
            icon_button(&icons::PLUS, 16.0, "New workspace", Some(Message::Action(Action::NewWorkspace))),
        ]
        .spacing(theme::S3)
        .align_x(Alignment::Center),
    )
    .width(theme::RAIL_WIDTH)
    .height(Fill)
    .padding([theme::S4, theme::S1])
    .style(theme::sidebar)
    .into()
}
