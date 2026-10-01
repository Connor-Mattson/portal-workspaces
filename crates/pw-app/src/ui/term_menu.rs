//! A terminal's right-click menu: copy, paste, select all.
//!
//! It floats over the window the terminal is in, on a full-window layer that closes it on a click,
//! right-click or scroll anywhere else, the way desktop menus do. The click that closes it does
//! nothing else, so dismissing the menu over a terminal can't start a selection or reach the program.

use iced::widget::{Space, button, column, container, mouse_area, opaque, pin, row, text};
use iced::{Alignment, Element, Fill, Point, Rectangle, mouse, window};
use pw_model::PaneId;

use crate::app::Message;
use crate::fonts;
use crate::keymap;
use crate::theme;

const WIDTH: f32 = 216.0;
const ITEM_HEIGHT: f32 = 28.0;
const PAD: f32 = 4.0;
const SEPARATOR: f32 = 9.0;
const HEIGHT: f32 = 3.0 * ITEM_HEIGHT + SEPARATOR + 2.0 * PAD;

/// An open terminal menu.
#[derive(Debug, Clone)]
pub struct TermMenu {
    pub window: window::Id,
    pub pane: PaneId,
    /// Its top-left corner, in window coordinates.
    pub at: Point,
    /// Whether there was a selection to copy when it opened.
    pub can_copy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuMsg {
    Copy,
    Paste,
    SelectAll,
    Close,
}

/// Where a menu opened by a click at `click` goes: below and right of the pointer, flipped up or
/// left where that would leave `area` (the terminal's bounds, which are inside the window).
pub fn place(click: Point, area: Rectangle) -> Point {
    let x = if click.x + WIDTH <= area.x + area.width { click.x } else { (click.x - WIDTH).max(area.x) };
    let y = if click.y + HEIGHT <= area.y + area.height { click.y } else { (click.y - HEIGHT).max(area.y) };
    Point::new(x, y)
}

pub fn view(menu: &TermMenu) -> Element<'_, Message> {
    let item = |label: &'static str, keys: String, msg: Option<MenuMsg>| {
        let content = row![
            text(label).size(theme::T_SM).font(fonts::UI),
            Space::new().width(Fill),
            text(keys).size(theme::T_XS).font(fonts::UI).color(theme::FG_3),
        ]
        .align_y(Alignment::Center);
        button(container(content).center_y(Fill))
            .width(Fill)
            .height(ITEM_HEIGHT)
            .padding([0.0, theme::S3 - PAD])
            .style(theme::menu_item)
            .on_press_maybe(msg.map(Message::TermMenu))
    };
    let separator = container(container(Space::new().width(Fill).height(1)).style(theme::divider))
        .height(SEPARATOR)
        .center_y(SEPARATOR);
    let card = container(
        column![
            item("Copy", keymap::shortcut_label("C"), menu.can_copy.then_some(MenuMsg::Copy)),
            item("Paste", keymap::shortcut_label("V"), Some(MenuMsg::Paste)),
            separator,
            item("Select all", keymap::shortcut_label("A"), Some(MenuMsg::SelectAll)),
        ]
        .spacing(0),
    )
    .padding(PAD)
    .width(WIDTH)
    .style(theme::floating_panel);

    let close = Message::TermMenu(MenuMsg::Close);
    mouse_area(pin(opaque(card)).x(menu.at.x).y(menu.at.y))
        .on_press(close.clone())
        .on_right_press(close.clone())
        .on_middle_press(close.clone())
        .on_scroll(move |_| close.clone())
        // Not `None`, so the terminal under the layer stops seeing the pointer.
        .interaction(mouse::Interaction::Idle)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rectangle = Rectangle { x: 100.0, y: 50.0, width: 800.0, height: 600.0 };

    #[test]
    fn opens_at_the_pointer_when_it_fits() {
        assert_eq!(place(Point::new(200.0, 100.0), AREA), Point::new(200.0, 100.0));
    }

    #[test]
    fn flips_away_from_the_edges() {
        let corner = Point::new(890.0, 640.0);
        assert_eq!(place(corner, AREA), Point::new(890.0 - WIDTH, 640.0 - HEIGHT));
    }

    #[test]
    fn stays_inside_a_small_terminal() {
        let small = Rectangle { x: 100.0, y: 50.0, width: 150.0, height: 80.0 };
        assert_eq!(place(Point::new(240.0, 120.0), small), Point::new(100.0, 50.0));
    }
}
