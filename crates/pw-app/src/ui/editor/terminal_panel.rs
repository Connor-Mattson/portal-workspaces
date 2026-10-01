//! The editor's terminal, below the groups: one shell of its own, framed like an agent pane.

use iced::widget::pane_grid;
use iced::widget::{container, row};
use iced::{Alignment, Element, Padding};

use crate::app::{App, EditorMsg, Message};
use crate::editor::EditorView;
use crate::icons;
use crate::theme;
use crate::ui::icon_button;
use crate::ui::workspace_view::{body as terminal_body, pane_title};

pub fn title_bar<'a>(app: &'a App, editor: &'a EditorView, focused: bool) -> pane_grid::TitleBar<'a, Message> {
    let pane = editor.terminal.id;
    let controls =
        row![icon_button(&icons::CLOSE, 13.0, "Hide terminal", Some(Message::Editor(EditorMsg::HideTerminal)))]
            .align_y(Alignment::Center);
    pane_grid::TitleBar::new(
        container(pane_title(app, pane, focused)).height(22.0).align_y(Alignment::Center).clip(true),
    )
    .controls(pane_grid::Controls::new(controls))
    .padding(Padding::from([3.0, theme::S2]).left(theme::S3))
    .style(theme::pane_title(focused))
}

pub fn body<'a>(app: &'a App, editor: &'a EditorView) -> Element<'a, Message> {
    let pane = editor.terminal.id;
    terminal_body(app, pane, app.has_focus(pane))
}
