//! A detached terminal's own window: the pane's title bar above the terminal, framed like a grid pane.

use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Fill, Padding};
use pw_model::PaneId;

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::theme;
use crate::ui::icon_button;
use crate::ui::workspace_view::{body, pane_title};

pub fn view(app: &App, pane: PaneId) -> Element<'_, Message> {
    let focused = app.has_focus(pane);
    let workspace = app.workspace_of(pane).map(|ws| ws.model.name.as_str()).unwrap_or_default();

    let header = container(
        row![
            container(pane_title(app, pane, focused)).clip(true),
            Space::new().width(Fill),
            text(workspace).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_3).wrapping(text::Wrapping::None),
            row![
                icon_button(&icons::DOCK, 13.0, "Return to the workspace", Some(Message::DockPane(pane))),
                icon_button(&icons::CLOSE, 13.0, "Close terminal", Some(Message::ClosePane(pane))),
            ]
            .spacing(0)
            .align_y(Alignment::Center),
        ]
        .spacing(theme::S3)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([3.0, theme::S2]).left(theme::S3))
    .style(theme::pane_title(focused));

    let card =
        container(column![header, body(app, pane, focused)]).width(Fill).height(Fill).style(theme::pane(focused));
    container(card).padding(theme::S2).width(Fill).height(Fill).style(theme::workspace_area).into()
}
