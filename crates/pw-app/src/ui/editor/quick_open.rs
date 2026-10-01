//! Quick open: a search box over the editor, and the best-matching files, matched letters lit.

use iced::widget::text::Span;
use iced::widget::{
    Space, button, center, column, container, mouse_area, opaque, rich_text, row, svg, text, text_input,
};
use iced::{Alignment, Element, Fill, Length, Padding};
use pw_code::Language;

use super::QUICK_INPUT;
use crate::app::{EditorMsg, Message};
use crate::editor::quick_open::QuickOpen;
use crate::fonts;
use crate::icons;
use crate::theme;

fn msg(m: EditorMsg) -> Message {
    Message::Editor(m)
}

pub fn view(quick: &QuickOpen) -> Element<'_, Message> {
    let input = row![
        svg(icons::SEARCH.clone()).width(15).height(15).style(theme::icon(theme::FG_3)),
        text_input("Go to file…", &quick.query)
            .id(QUICK_INPUT)
            .on_input(|q| msg(EditorMsg::QuickQuery(q)))
            .on_submit(msg(EditorMsg::QuickSubmit))
            .size(theme::T_MD)
            .font(fonts::UI)
            .padding([6, 4])
            .style(input_style),
    ]
    .spacing(theme::S2)
    .align_y(Alignment::Center);

    let results: Element<'_, Message> = match &quick.files {
        None => hint("Indexing files…"),
        Some(_) if quick.matches.is_empty() => hint("No matching files"),
        Some(_) => column(
            quick
                .matches
                .iter()
                .enumerate()
                .filter_map(|(i, m)| Some(result(i, quick.path(i)?, &m.positions, i == quick.selected))),
        )
        .spacing(1)
        .into(),
    };

    let card = container(
        column![
            container(input).padding(Padding::from([4.0, theme::S3])),
            container(Space::new().height(1).width(Fill)).style(theme::divider),
            container(results).padding(6).max_height(420.0).clip(true),
        ]
        .spacing(0),
    )
    .width(580)
    .style(theme::floating_panel);

    mouse_area(container(opaque(card)).center_x(Fill).height(Fill).padding(Padding::default().top(56.0)))
        .on_press(msg(EditorMsg::QuickClose))
        .into()
}

fn hint(label: &str) -> Element<'_, Message> {
    center(text(label).size(theme::T_SM).font(fonts::UI).color(theme::FG_3)).height(Length::Fixed(64.0)).into()
}

/// One file: its name, matched letters in the accent, and its folder.
fn result<'a>(index: usize, path: &'a str, positions: &[u32], selected: bool) -> Element<'a, Message> {
    let split = path.rfind('/').map_or(0, |i| i + 1);
    let name_start = path[..split].chars().count() as u32;
    let (dir, name) = path.split_at(split);
    let spans = |part: &'a str, offset: u32, base: iced::Color, font: iced::Font| -> Vec<Span<'a, ()>> {
        let mut out = Vec::new();
        let mut run = String::new();
        let mut lit = false;
        for (i, c) in part.chars().enumerate() {
            let hit = positions.binary_search(&(offset + i as u32)).is_ok();
            if hit != lit && !run.is_empty() {
                out.push(span(std::mem::take(&mut run), lit, base, font));
            }
            lit = hit;
            run.push(c);
        }
        if !run.is_empty() {
            out.push(span(run, lit, base, font));
        }
        out
    };
    let language = Language::detect(std::path::Path::new(path), None);
    let line = row![
        svg(icons::FILE.clone()).width(14).height(14).style(theme::icon(theme::tint(language.tint))),
        rich_text(spans(name, name_start, theme::FG, fonts::UI_MEDIUM)).size(theme::T_MD),
        rich_text(spans(dir.trim_end_matches('/'), 0, theme::FG_3, fonts::UI)).size(theme::T_SM),
    ]
    .spacing(theme::S2)
    .align_y(Alignment::Center);
    button(line)
        .width(Fill)
        .padding([6, 10])
        .style(theme::result_row(selected))
        .on_press(msg(EditorMsg::QuickPick(index)))
        .into()
}

fn span<'a>(text: String, lit: bool, base: iced::Color, font: iced::Font) -> Span<'a, ()> {
    Span::new(text).color(if lit { theme::ACCENT_HOVER } else { base }).font(if lit {
        fonts::UI_SEMIBOLD
    } else {
        font
    })
}

/// The search box sits flush in the card.
fn input_style(theme: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let base = theme::input(theme, status);
    text_input::Style {
        background: iced::Color::TRANSPARENT.into(),
        border: iced::Border { width: 0.0, ..base.border },
        ..base
    }
}
