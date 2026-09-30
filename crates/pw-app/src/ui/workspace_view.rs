//! The main area: a header for the active workspace above its grid of terminals.

use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{Space, button, center, column, container, row, stack, svg, text};
use iced::{Alignment, Element, Fill, Length, Padding};
use pw_model::{Axis, PaneId, Preset};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::keymap::Action;
use crate::theme;
use crate::ui::terminal::TerminalCanvas;
use crate::ui::{icon_button, preset_glyph, tildify, tip_for};
use crate::workspace::WorkspaceView;

const TITLE_HEIGHT: f32 = 28.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let content: Element<'_, Message> = match app.active_view() {
        Some(ws) => column![header(ws), grid(app, ws)].into(),
        None => empty(
            "No workspace yet",
            "A workspace is one project: a folder and the terminals you keep open for it.",
            "New workspace",
            Message::Action(Action::NewWorkspace),
        ),
    };
    container(content).width(Fill).height(Fill).style(theme::workspace_area).into()
}

fn header<'a>(ws: &'a WorkspaceView) -> Element<'a, Message> {
    let current = ws.matching_preset();
    let presets = row(Preset::ALL.into_iter().map(|preset| {
        let active = current == Some(preset);
        let chip = button(preset_glyph(preset, if active { theme::ACCENT } else { theme::FG_3 }))
            .padding([5, 7])
            .style(theme::chip(active))
            .on_press(Message::ApplyPreset(preset));
        tip_for(chip.into(), preset_tip(preset))
    }))
    .spacing(theme::S1);

    let can_split = ws.can_split();
    let focused = ws.focused;
    let actions = row![
        icon_button(
            &icons::SPLIT_RIGHT,
            16.0,
            "Split right",
            can_split.then_some(Message::Action(Action::Split(Axis::Vertical)))
        ),
        icon_button(
            &icons::SPLIT_DOWN,
            16.0,
            "Split down",
            can_split.then_some(Message::Action(Action::Split(Axis::Horizontal)))
        ),
        icon_button(
            if ws.maximized().is_some() { &icons::MINIMIZE } else { &icons::MAXIMIZE },
            16.0,
            if ws.maximized().is_some() { "Restore layout" } else { "Maximize terminal" },
            focused.filter(|_| ws.pane_count() > 1).map(Message::MaximizePane),
        ),
        icon_button(&icons::PENCIL, 15.0, "Edit workspace", Some(Message::EditWorkspace(ws.id()))),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let title = row![
        text(&ws.model.name).size(theme::T_LG).font(fonts::UI_SEMIBOLD).color(theme::FG),
        row![
            svg(icons::FOLDER.clone()).width(13).height(13).style(theme::icon(theme::FG_3)),
            text(tildify(&ws.model.root)).size(theme::T_SM).font(fonts::UI).color(theme::FG_3),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(theme::S3)
    .align_y(Alignment::Center);

    let count = text(format!("{} / {}", ws.pane_count(), pw_model::MAX_PANES))
        .size(theme::T_XS)
        .font(fonts::UI_MEDIUM)
        .color(theme::FG_3);

    container(
        row![
            title,
            Space::new().width(Fill),
            count,
            presets,
            container(Space::new().width(1).height(20)).style(theme::divider),
            actions
        ]
        .spacing(theme::S3)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([10.0, theme::S4]).left(theme::S6))
    .width(Fill)
    .into()
}

fn preset_tip(preset: Preset) -> &'static str {
    match preset {
        Preset::Single => "Single terminal",
        Preset::Columns(2) => "2 columns",
        Preset::Rows(2) => "2 rows",
        Preset::Columns(3) => "3 columns",
        Preset::Grid { cols: 2, rows: 2 } => "2 × 2 grid",
        Preset::Grid { cols: 3, rows: 2 } => "3 × 2 grid",
        Preset::Columns(4) => "4 columns",
        Preset::Grid { cols: 4, rows: 2 } => "4 × 2 grid",
        _ => "Layout",
    }
}

fn grid<'a>(app: &'a App, ws: &'a WorkspaceView) -> Element<'a, Message> {
    let Some(state) = &ws.grid else {
        return empty(
            "No terminals open",
            "Open a terminal to get going, or pick a layout above.",
            "New terminal",
            Message::NewPane,
        );
    };
    let panes = PaneGrid::new(state, |_, &pane, maximized| {
        let focused = ws.focused == Some(pane);
        pane_grid::Content::new(body(app, pane, focused))
            .title_bar(title_bar(app, ws, pane, focused, maximized))
            .style(theme::pane(focused))
    })
    .spacing(theme::S2)
    .on_click(Message::PaneClicked)
    .on_drag(Message::PaneDragged)
    .on_resize(8, Message::PaneResized)
    .style(theme::panes);

    container(panes).padding(Padding::from([0.0, theme::S3]).bottom(theme::S3)).into()
}

fn title_bar<'a>(
    app: &'a App,
    ws: &'a WorkspaceView,
    pane: PaneId,
    focused: bool,
    maximized: bool,
) -> pane_grid::TitleBar<'a, Message> {
    let rt = app.sessions.get(pane);
    let cwd = rt.map(|rt| tildify(&rt.cwd)).unwrap_or_default();
    let exited = rt.is_some_and(|rt| rt.exited.is_some() || rt.session.is_none());
    let label = rt.and_then(|rt| rt.title.clone()).unwrap_or_else(|| cwd.clone());

    let status = container(Space::new().width(6).height(6)).style(theme::badge(if exited {
        theme::WARN
    } else if focused {
        theme::ACCENT
    } else {
        theme::LINE_STRONG
    }));

    let mut title = row![status, text(label).size(theme::T_SM).font(fonts::UI_MEDIUM).wrapping(text::Wrapping::None)]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
    // Shells usually put the cwd in the title already; only add it when they don't.
    if rt.is_some_and(|rt| rt.title.as_ref().is_some_and(|t| !t.contains(cwd.as_str()))) {
        title =
            title.push(text(cwd).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).wrapping(text::Wrapping::None));
    }

    let can_split = ws.can_split();
    let controls = row![
        icon_button(
            &icons::SPLIT_RIGHT,
            13.0,
            "Split right",
            can_split.then_some(Message::SplitPane(pane, Axis::Vertical))
        ),
        icon_button(
            &icons::SPLIT_DOWN,
            13.0,
            "Split down",
            can_split.then_some(Message::SplitPane(pane, Axis::Horizontal))
        ),
        icon_button(
            if maximized { &icons::MINIMIZE } else { &icons::MAXIMIZE },
            13.0,
            if maximized { "Restore" } else { "Maximize" },
            (ws.pane_count() > 1).then_some(Message::MaximizePane(pane)),
        ),
        icon_button(&icons::CLOSE, 13.0, "Close terminal", Some(Message::ClosePane(pane))),
    ]
    .spacing(0)
    .align_y(Alignment::Center);

    pane_grid::TitleBar::new(container(title).height(TITLE_HEIGHT - 6.0).align_y(Alignment::Center).clip(true))
        .controls(pane_grid::Controls::new(controls))
        .padding(Padding::from([3.0, theme::S2]).left(theme::S3))
        .style(theme::pane_title(focused))
}

fn body<'a>(app: &'a App, pane: PaneId, focused: bool) -> Element<'a, Message> {
    let Some(rt) = app.sessions.get(pane) else {
        return center(text("Starting…").size(theme::T_SM).color(theme::FG_3)).into();
    };
    if let Some(error) = &rt.spawn_error {
        return notice(pane, "Couldn't start a shell", error.clone());
    }
    let terminal =
        iced::widget::canvas(TerminalCanvas { pane, rt, focused, metrics: app.metrics }).width(Fill).height(Fill);
    match rt.exited {
        None => container(terminal).padding(Padding::default().bottom(4)).into(),
        Some(code) => {
            let status = match code {
                Some(code) => format!("Process exited with code {code}"),
                None => "Process exited".to_owned(),
            };
            let bar = container(
                row![
                    text(status).size(theme::T_SM).font(fonts::UI_MEDIUM).color(theme::FG_2),
                    Space::new().width(Fill),
                    button(
                        row![
                            svg(icons::RESTART.clone()).width(13).height(13).style(theme::icon(theme::FG)),
                            text("Restart").size(theme::T_SM).font(fonts::UI_MEDIUM),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .padding([4, 10])
                    .style(theme::secondary_button)
                    .on_press(Message::RestartPane(pane)),
                    text("or press Enter").size(theme::T_XS).font(fonts::UI).color(theme::FG_3),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            )
            .padding([theme::S2, theme::S3])
            .style(theme::card);
            stack![terminal, container(bar).align_bottom(Fill).width(Fill).padding(theme::S3)].into()
        }
    }
}

fn notice<'a>(pane: PaneId, title: &'a str, detail: String) -> Element<'a, Message> {
    center(
        column![
            text(title).size(theme::T_MD).font(fonts::UI_SEMIBOLD).color(theme::FG),
            container(text(detail).size(theme::T_SM).font(fonts::MONO)).padding([6, 10]).style(theme::error_note),
            button(text("Try again").size(theme::T_SM).font(fonts::UI_MEDIUM))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press(Message::RestartPane(pane)),
        ]
        .spacing(theme::S3)
        .align_x(Alignment::Center),
    )
    .into()
}

fn empty<'a>(title: &'a str, detail: &'a str, cta: &'a str, on_press: Message) -> Element<'a, Message> {
    center(
        column![
            container(svg(icons::TERMINAL.clone()).width(26).height(26).style(theme::icon(theme::ACCENT)))
                .center(Length::Fixed(52.0))
                .style(theme::count_badge(true)),
            text(title).size(theme::T_LG).font(fonts::UI_SEMIBOLD).color(theme::FG),
            text(detail).size(theme::T_MD).font(fonts::UI).color(theme::FG_3),
            button(
                row![
                    svg(icons::PLUS.clone()).width(14).height(14).style(theme::icon(theme::ON_ACCENT)),
                    text(cta).size(theme::T_MD).font(fonts::UI_SEMIBOLD),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([8, 14])
            .style(theme::primary_button)
            .on_press(on_press),
        ]
        .spacing(theme::S3)
        .align_x(Alignment::Center),
    )
    .into()
}
