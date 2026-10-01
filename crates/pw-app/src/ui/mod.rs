//! Views. Each module renders one region of the window from the app state.

pub mod modal;
pub mod pane_window;
pub mod profile_editor;
pub mod sidebar;
pub mod terminal;
pub mod usage;
pub mod workspace_view;

use iced::mouse::Cursor;
use iced::widget::canvas::{self, Geometry, Path};
use iced::widget::{Space, button, container, opaque, row, stack, svg, text, tooltip};
use iced::{Color, Element, Fill, Length, Rectangle, Renderer, Theme};
use pw_model::{Axis, LayoutNode, PaneId, Preset};

use crate::app::{App, Message};
use crate::fonts;
use crate::sessions::PaneRuntime;
use crate::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let body = row![
        sidebar::view(app),
        container(Space::new().width(1).height(Fill)).style(theme::divider),
        workspace_view::view(app),
    ];
    match &app.modal {
        Some(modal) => stack![body, opaque(modal::view(app, modal))].into(),
        None => body.into(),
    }
}

/// A square icon button with a tooltip. `on_press` of `None` renders it disabled.
pub fn icon_button<'a>(
    icon: &'static svg::Handle,
    size: f32,
    tip: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let glyph = svg(icon.clone()).width(size).height(size).style(if on_press.is_some() {
        theme::icon_hover(theme::FG_3, theme::FG)
    } else {
        theme::icon_hover(theme::LINE_STRONG, theme::LINE_STRONG)
    });
    let btn = button(container(glyph).center(Length::Fixed(size + 10.0)))
        .padding(0)
        .style(theme::ghost_button)
        .on_press_maybe(on_press);
    tip_for(btn.into(), tip)
}

pub fn tip_for<'a>(content: Element<'a, Message>, tip: &'a str) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(tip).size(theme::T_SM).font(fonts::UI).color(theme::FG_2)).padding([4, 8]).style(theme::keycap),
        tooltip::Position::Bottom,
    )
    .gap(6)
    .into()
}

/// What a terminal is called: the title its program set, or its cwd.
pub fn pane_label(rt: &PaneRuntime) -> String {
    rt.title.clone().unwrap_or_else(|| tildify(&rt.cwd))
}

/// Shortens a path under the home directory to `~/…`.
pub fn tildify(path: &std::path::Path) -> String {
    let home = crate::sessions::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A 20×14 drawing of a preset's arrangement.
pub fn preset_glyph<'a>(preset: Preset, color: Color) -> Element<'a, Message> {
    canvas::Canvas::new(PresetGlyph { preset, color }).width(20).height(14).into()
}

/// A miniature drawing of a preset's arrangement, built from the same tree the preset produces.
struct PresetGlyph {
    preset: Preset,
    color: Color,
}

impl canvas::Program<Message> for PresetGlyph {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ids: Vec<PaneId> = (0..self.preset.pane_count()).map(|_| PaneId::new()).collect();
        let mut cells = Vec::new();
        regions(&self.preset.build(&ids), Rectangle::with_size(bounds.size()), &mut cells);
        for cell in cells {
            let shrunk =
                Rectangle { x: cell.x + 0.75, y: cell.y + 0.75, width: cell.width - 1.5, height: cell.height - 1.5 };
            frame.fill(&Path::rounded_rectangle(shrunk.position(), shrunk.size(), 1.5.into()), self.color);
        }
        vec![frame.into_geometry()]
    }
}

fn regions(node: &LayoutNode, area: Rectangle, out: &mut Vec<Rectangle>) {
    match node {
        LayoutNode::Pane { .. } => out.push(area),
        LayoutNode::Split { axis, ratio, a, b } => {
            let (first, second) = match axis {
                Axis::Vertical => {
                    let w = area.width * ratio;
                    (Rectangle { width: w, ..area }, Rectangle { x: area.x + w, width: area.width - w, ..area })
                }
                Axis::Horizontal => {
                    let h = area.height * ratio;
                    (Rectangle { height: h, ..area }, Rectangle { y: area.y + h, height: area.height - h, ..area })
                }
            };
            regions(a, first, out);
            regions(b, second, out);
        }
    }
}
