//! How a terminal's or workspace's attention looks: a dot, its color and a chip (see `attention`).

use iced::widget::{Space, container, text};
use iced::{Color, Element, Length};

use crate::app::Message;
use crate::attention::{Kind, Status};
use crate::fonts;
use crate::theme;

pub fn color(kind: Kind) -> Color {
    match kind {
        Kind::NeedsInput => theme::WARN,
        Kind::Finished => theme::OK,
    }
}

fn soft(kind: Kind) -> Color {
    match kind {
        Kind::NeedsInput => theme::WARN_SOFT,
        Kind::Finished => theme::OK_SOFT,
    }
}

/// The color of a status's dot, if it gets one: what it wants, else a dimmer accent while it works.
pub fn tint(status: &Status) -> Option<Color> {
    match status.kind {
        Some(kind) => Some(color(kind)),
        None => status.working.then_some(theme::WORKING),
    }
}

/// A workspace's dot, `size` across in a slightly larger box: haloed when it needs you, plain when
/// it finished, smaller and dim while it works, and empty space (so rows line up) otherwise.
pub fn dot<'a>(status: &Status, size: f32) -> Element<'a, Message> {
    let boxed = |d: f32, c: Color| {
        container(container(Space::new().width(d).height(d)).style(theme::badge(c))).center(Length::Fixed(size + 4.0))
    };
    match status.kind {
        Some(Kind::NeedsInput) => boxed(size, theme::WARN).style(theme::badge(theme::WARN_SOFT)).into(),
        Some(Kind::Finished) => boxed(size, theme::OK).into(),
        None if status.working => boxed(size - 2.0, theme::WORKING).into(),
        None => Space::new().width(size + 4.0).height(size + 4.0).into(),
    }
}

/// "Needs input" / "Finished", as a small pill.
pub fn chip<'a>(kind: Kind) -> Element<'a, Message> {
    container(text(kind.label()).size(theme::T_XS).font(fonts::UI_SEMIBOLD).color(color(kind)))
        .padding([1, 7])
        .style(theme::badge(soft(kind)))
        .into()
}
