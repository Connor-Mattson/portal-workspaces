//! The workspace drawer on the left: full list, or a slim rail when collapsed.

use iced::widget::{Space, button, column, container, hover, row, scrollable, svg, text, tooltip};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::keymap::Action;
use crate::theme;
use crate::ui::{icon_button, tildify, usage};
use crate::workspace::WorkspaceView;

#[derive(Clone, Copy)]
enum Activity {
    None,
    Output,
    Bell,
}

fn activity(app: &App, ws: &WorkspaceView) -> Activity {
    let panes = ws.pane_ids();
    let rts = panes.iter().filter_map(|p| app.sessions.get(*p));
    let (mut output, mut bell) = (false, false);
    for rt in rts {
        output |= rt.unseen_output;
        bell |= rt.bell;
    }
    if bell {
        Activity::Bell
    } else if output {
        Activity::Output
    } else {
        Activity::None
    }
}

fn dot<'a>(activity: Activity) -> Element<'a, Message> {
    let color = match activity {
        Activity::None => return Space::new().width(8).height(8).into(),
        Activity::Output => theme::ACCENT,
        Activity::Bell => theme::WARN,
    };
    container(Space::new().width(8).height(8)).style(theme::badge(color)).into()
}

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
        text(concat!("v", env!("CARGO_PKG_VERSION"))).size(theme::T_XS).font(fonts::UI).color(theme::LINE_STRONG),
    ]
    .align_y(Alignment::Center);

    container(
        column![
            container(brand).padding(Padding::from([theme::S4, theme::S3]).bottom(theme::S3)),
            container(header).padding([0.0, theme::S3]),
            scrollable(container(items).padding([theme::S1, theme::S2])).height(Fill).style(theme::scroller),
            container(usage::section(app)).padding([0.0, theme::S2]),
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
    let label = column![
        text(&ws.model.name)
            .size(theme::T_MD)
            .font(if active { fonts::UI_SEMIBOLD } else { fonts::UI_MEDIUM })
            .wrapping(text::Wrapping::None),
        text(tildify(&ws.model.root))
            .size(theme::T_XS)
            .font(fonts::UI)
            .color(theme::FG_3)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(1);
    let label = container(label).width(Fill).clip(true);

    let badge = container(text(count.to_string()).size(theme::T_XS).font(fonts::UI_SEMIBOLD))
        .padding([1, 7])
        .style(theme::count_badge(active));

    let base = button(
        row![monogram(&ws.model.name, active), label, dot(activity(app, ws)), badge]
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
        let mark = iced::widget::stack![
            monogram(&ws.model.name, active),
            container(dot(activity(app, ws))).align_right(Length::Fixed(30.0)).align_top(Length::Fixed(30.0)),
        ];
        let btn = button(mark).padding(4).style(theme::nav_item(false)).on_press(Message::SelectWorkspace(ws.id()));
        tooltip(
            btn,
            container(text(&ws.model.name).size(theme::T_SM).font(fonts::UI_MEDIUM).color(theme::FG))
                .padding([4, 8])
                .style(theme::keycap),
            tooltip::Position::Right,
        )
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
