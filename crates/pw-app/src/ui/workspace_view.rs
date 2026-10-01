//! The main area: a header for the active workspace (with the Agents / Editor switch) above its grid of
//! terminals or its editor.

use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{Space, button, center, column, container, row, stack, svg, text};
use iced::{Alignment, Element, Fill, Length, Padding};
use pw_model::{Axis, Mode, PaneId, Preset};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::keymap::Action;
use crate::theme;
use crate::ui::terminal::TerminalCanvas;
use crate::ui::{icon_button, pane_label, preset_glyph, status, tildify, tip_for};
use crate::workspace::WorkspaceView;

const TITLE_HEIGHT: f32 = 28.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let content: Element<'_, Message> = match app.active_view() {
        Some(ws) => {
            let body = match (ws.model.mode, &ws.editor) {
                (Mode::Editor, Some(editor)) => crate::ui::editor::view(app, &ws.model.name, editor),
                (Mode::Editor, None) => center(text("Opening…").size(theme::T_SM).color(theme::FG_3)).into(),
                (Mode::Agents, _) => grid(app, ws),
            };
            column![header(app, ws), body].into()
        }
        None => empty(
            "No workspace yet",
            "A workspace is one project: a folder and the terminals you keep open for it.",
            "New workspace",
            Message::Action(Action::NewWorkspace),
        ),
    };
    container(content).width(Fill).height(Fill).style(theme::workspace_area).into()
}

fn header<'a>(app: &'a App, ws: &'a WorkspaceView) -> Element<'a, Message> {
    let title = row![
        text(&ws.model.name).size(theme::T_LG).font(fonts::UI_SEMIBOLD).color(theme::FG).wrapping(text::Wrapping::None),
        row![
            svg(icons::FOLDER.clone()).width(13).height(13).style(theme::icon(theme::FG_3)),
            text(tildify(&ws.model.root))
                .size(theme::T_SM)
                .font(fonts::UI)
                .color(theme::FG_3)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(theme::S3)
    .align_y(Alignment::Center);

    let tools = match ws.model.mode {
        Mode::Agents => agent_tools(ws),
        Mode::Editor => editor_tools(ws),
    };
    // The switch sits in the middle, between two halves of equal width.
    container(
        row![
            container(title).width(Fill).clip(true),
            mode_switch(app, ws),
            container(tools).width(Fill).align_right(Fill),
        ]
        .spacing(theme::S4)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([10.0, theme::S4]).left(theme::S6))
    .width(Fill)
    .into()
}

/// Agents / Editor. Each side shows a dot for what's happening on the other: agents that finished,
/// need you or are working while editing, unsaved files while watching agents.
fn mode_switch<'a>(app: &'a App, ws: &'a WorkspaceView) -> Element<'a, Message> {
    let mode = ws.model.mode;
    let activity = (mode == Mode::Editor).then(|| status::tint(&app.status_of(ws.all_pane_ids()))).flatten();
    let unsaved = mode == Mode::Agents && ws.editor.as_ref().is_some_and(|e| !e.dirty_paths().is_empty());
    let keys = if cfg!(target_os = "macos") { "⌘⇧M" } else { "Ctrl+Shift+M" };
    let segment = |icon: &'static svg::Handle, label: &'static str, target: Mode, dot: Option<iced::Color>| {
        let active = mode == target;
        let mut content = row![
            svg(icon.clone()).width(14).height(14).style(theme::icon(if active { theme::ACCENT } else { theme::FG_3 })),
            text(label).size(theme::T_SM).font(if active { fonts::UI_SEMIBOLD } else { fonts::UI_MEDIUM }),
        ]
        .spacing(7)
        .align_y(Alignment::Center);
        if let Some(color) = dot {
            content = content.push(container(Space::new().width(6).height(6)).style(theme::badge(color)));
        }
        button(content)
            .padding([5, 12])
            .style(theme::segment(active))
            .on_press_maybe((!active).then_some(Message::SetMode(target)))
    };
    let switch = container(
        row![
            segment(&icons::LAYOUT_GRID, "Agents", Mode::Agents, activity),
            segment(&icons::CODE, "Editor", Mode::Editor, unsaved.then_some(theme::FG_2)),
        ]
        .spacing(2),
    )
    .padding(2)
    .style(theme::segmented);
    iced::widget::tooltip(
        switch,
        container(text(format!("Switch modes  {keys}")).size(theme::T_SM).font(fonts::UI).color(theme::FG_2))
            .padding([4, 8])
            .style(theme::keycap),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(6)
    .into()
}

fn editor_tools<'a>(ws: &'a WorkspaceView) -> Element<'a, Message> {
    let Some(editor) = &ws.editor else { return Space::new().into() };
    let action = |a| Some(Message::Action(a));
    row![
        icon_button(&icons::SEARCH, 15.0, "Quick open", action(Action::QuickOpen)),
        icon_button(
            &icons::FILES,
            15.0,
            if editor.show_explorer { "Hide file tree" } else { "Show file tree" },
            action(Action::ToggleExplorer)
        ),
        icon_button(
            &icons::SPLIT_RIGHT,
            16.0,
            "Split editor",
            editor.can_split().then_some(Message::Action(Action::Split(Axis::Vertical)))
        ),
        icon_button(
            &icons::PANEL_BOTTOM,
            16.0,
            if editor.show_terminal { "Hide terminal" } else { "Show terminal" },
            action(Action::ToggleTerminal)
        ),
        container(Space::new().width(1).height(20)).style(theme::divider),
        icon_button(&icons::PENCIL, 15.0, "Edit workspace", Some(Message::EditWorkspace(ws.id()))),
    ]
    .spacing(2)
    .align_y(Alignment::Center)
    .into()
}

fn agent_tools<'a>(ws: &'a WorkspaceView) -> Element<'a, Message> {
    let current = ws.matching_preset();
    let presets = row(Preset::ALL.into_iter().map(|preset| {
        let active = current == Some(preset);
        // With terminals detached, the bigger layouts would go over the cap.
        let fits = ws.fits(preset);
        let color = if active {
            theme::ACCENT
        } else if fits {
            theme::FG_3
        } else {
            theme::LINE_STRONG
        };
        let chip = button(preset_glyph(preset, color))
            .padding([5, 7])
            .style(theme::chip(active))
            .on_press_maybe(fits.then_some(Message::ApplyPreset(preset)));
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
            focused.filter(|_| ws.grid_len() > 1).map(Message::MaximizePane),
        ),
        icon_button(&icons::PENCIL, 15.0, "Edit workspace", Some(Message::EditWorkspace(ws.id()))),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let count = text(format!("{} / {}", ws.pane_count(), pw_model::MAX_PANES))
        .size(theme::T_XS)
        .font(fonts::UI_MEDIUM)
        .color(theme::FG_3);

    row![count, presets, container(Space::new().width(1).height(20)).style(theme::divider), actions]
        .spacing(theme::S3)
        .align_y(Alignment::Center)
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
        let focused = app.has_focus(pane);
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
            (ws.grid_len() > 1).then_some(Message::MaximizePane(pane)),
        ),
        icon_button(&icons::POP_OUT, 13.0, "Open in its own window", Some(Message::DetachPane(pane))),
        icon_button(&icons::CLOSE, 13.0, "Close terminal", Some(Message::ClosePane(pane))),
    ]
    .spacing(0)
    .align_y(Alignment::Center);

    pane_grid::TitleBar::new(
        container(pane_title(app, pane, focused)).height(TITLE_HEIGHT - 6.0).align_y(Alignment::Center).clip(true),
    )
    .controls(pane_grid::Controls::new(controls))
    .padding(Padding::from([3.0, theme::S2]).left(theme::S3))
    .style(theme::pane_title(focused))
}

/// A terminal's status dot, its label and (when the label doesn't show it) its cwd.
pub fn pane_title<'a>(app: &'a App, pane: PaneId, focused: bool) -> Element<'a, Message> {
    let rt = app.sessions.get(pane);
    let cwd = rt.map(|rt| tildify(&rt.cwd)).unwrap_or_default();
    let exited = rt.is_some_and(|rt| rt.exited.is_some() || rt.session.is_none());
    let label = rt.map(pane_label).unwrap_or_default();

    let wants = rt.and_then(|rt| rt.attention.as_ref()).map(|a| a.kind);
    let working = rt.is_some_and(|rt| rt.working);
    let dot = match (exited, wants) {
        (true, _) => theme::WARN,
        (false, Some(kind)) => status::color(kind),
        _ if focused => theme::ACCENT,
        _ if working => theme::WORKING,
        _ => theme::LINE_STRONG,
    };
    let dot = container(Space::new().width(6).height(6)).style(theme::badge(dot));

    let mut title = row![dot, text(label).size(theme::T_SM).font(fonts::UI_MEDIUM).wrapping(text::Wrapping::None)]
        .spacing(theme::S2)
        .align_y(Alignment::Center);
    // You haven't been to it since it finished or asked: say so where it's easy to spot in a full grid.
    if let Some(kind) = wants {
        title = title.push(status::chip(kind));
    }
    // Shells usually put the cwd in the title already; only add it when they don't.
    if rt.is_some_and(|rt| rt.title.as_ref().is_some_and(|t| !t.contains(cwd.as_str()))) {
        title =
            title.push(text(cwd).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).wrapping(text::Wrapping::None));
    }
    title.into()
}

pub fn body<'a>(app: &'a App, pane: PaneId, focused: bool) -> Element<'a, Message> {
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
