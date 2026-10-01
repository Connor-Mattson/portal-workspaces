//! Design tokens and widget styles.
//!
//! The colors are Science Portal's dark tokens (`../science-portal/web/src/styles/tokens.css`),
//! so the two apps read as one family. One accent is used for focus, selection and the primary
//! action. Status colors are for status only.

use iced::border::Radius;
use iced::widget::{button, container, pane_grid, scrollable, svg, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, color};
use pw_code::Tint;
use pw_code::syntax::Class;

pub const BG: Color = color!(0x0a0c10);
pub const SURFACE: Color = color!(0x111419);
pub const SURFACE_2: Color = color!(0x171b22);
pub const SURFACE_3: Color = color!(0x1e232c);
pub const LINE: Color = color!(0x232934);
pub const LINE_STRONG: Color = color!(0x313947);
pub const FG: Color = color!(0xe8ebf1);
pub const FG_2: Color = color!(0xb3bbc8);
pub const FG_3: Color = color!(0x8a93a3);
pub const ACCENT: Color = color!(0x7d95ff);
pub const ACCENT_HOVER: Color = color!(0x93a7ff);
pub const ACCENT_SOFT: Color = color!(0x1a2246);
pub const ACCENT_LINE: Color = color!(0x2e3c78);
/// A terminal at work: the accent, quieter than news.
pub const WORKING: Color = Color { a: 0.55, ..ACCENT };
pub const ON_ACCENT: Color = color!(0x0a0c10);
pub const WARN: Color = color!(0xeeb152);
pub const WARN_SOFT: Color = color!(0x2a200f);
pub const OK: Color = color!(0x4cc987);
pub const OK_SOFT: Color = color!(0x0f2a1d);
pub const BAD: Color = color!(0xff6f7b);
pub const BAD_SOFT: Color = color!(0x321519);
pub const TERM_BG: Color = color!(0x07090d);
/// The code editor's paper: a touch lighter than terminals, darker than surfaces.
pub const EDITOR_BG: Color = color!(0x0d1015);
pub const SCRIM: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 };

// 4px spacing grid and radii.
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S6: f32 = 24.0;
pub const R_SM: f32 = 6.0;
pub const R_MD: f32 = 8.0;
pub const R_LG: f32 = 12.0;

pub const T_XS: f32 = 11.0;
pub const T_SM: f32 = 12.0;
pub const T_MD: f32 = 13.0;
pub const T_LG: f32 = 16.0;

pub const SIDEBAR_WIDTH: f32 = 248.0;
/// Editor groups' tab strips.
pub const TAB_HEIGHT: f32 = 34.0;
pub const RAIL_WIDTH: f32 = 60.0;

pub fn theme() -> Theme {
    Theme::custom(
        "Portal".to_owned(),
        iced::theme::Palette {
            background: BG,
            text: FG,
            primary: ACCENT,
            success: color!(0x4cc987),
            warning: WARN,
            danger: BAD,
        },
    )
}

pub fn app_style(_: &Theme) -> iced::theme::Style {
    iced::theme::Style { background_color: BG, text_color: FG }
}

fn border(color: Color, width: f32, radius: f32) -> Border {
    Border { color, width, radius: Radius::from(radius) }
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: Border { color: LINE, width: 0.0, radius: 0.0.into() },
        ..Default::default()
    }
}

pub fn divider(_: &Theme) -> container::Style {
    container::Style { background: Some(LINE.into()), ..Default::default() }
}

pub fn workspace_area(_: &Theme) -> container::Style {
    container::Style { background: Some(BG.into()), ..Default::default() }
}

/// A terminal pane: rounded card, accent outline when focused.
pub fn pane(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(TERM_BG.into()),
        border: border(if focused { ACCENT_LINE } else { LINE }, 1.0, R_MD),
        ..Default::default()
    }
}

pub fn pane_title(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { text_color: Some(if focused { FG_2 } else { FG_3 }), ..Default::default() }
}

pub fn panes(_: &Theme) -> pane_grid::Style {
    pane_grid::Style {
        hovered_region: pane_grid::Highlight {
            background: Background::Color(Color { a: 0.12, ..ACCENT }),
            border: border(ACCENT, 1.0, R_MD),
        },
        picked_split: pane_grid::Line { color: ACCENT, width: 2.0 },
        hovered_split: pane_grid::Line { color: ACCENT_LINE, width: 2.0 },
    }
}

/// Transparent icon button that lights up on hover.
pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Hovered => (Some(SURFACE_3.into()), FG),
        button::Status::Pressed => (Some(SURFACE_2.into()), FG),
        button::Status::Disabled => (None, Color { a: 0.35, ..FG_3 }),
        button::Status::Active => (None, FG_3),
    };
    button::Style {
        background: bg,
        text_color: fg,
        border: border(Color::TRANSPARENT, 0.0, R_SM),
        ..Default::default()
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => ACCENT_HOVER,
        button::Status::Disabled => Color { a: 0.4, ..ACCENT },
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: ON_ACCENT,
        border: border(bg, 0.0, R_SM),
        ..Default::default()
    }
}

pub fn secondary_button(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => SURFACE_3,
        _ => SURFACE_2,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: FG,
        border: border(LINE_STRONG, 1.0, R_SM),
        ..Default::default()
    }
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0xff8f98),
        _ => BAD,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: ON_ACCENT,
        border: border(bg, 0.0, R_SM),
        ..Default::default()
    }
}

/// A row in the workspace drawer.
pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match (active, status) {
            (true, _) => Some(ACCENT_SOFT.into()),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(SURFACE_2.into()),
            _ => None,
        };
        button::Style {
            background: bg,
            text_color: if active { FG } else { FG_2 },
            border: border(if active { ACCENT_LINE } else { Color::TRANSPARENT }, 1.0, R_MD),
            ..Default::default()
        }
    }
}

/// Pill used for layout presets; `active` marks the current arrangement.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = matches!(status, button::Status::Disabled);
        button::Style {
            background: Some(
                if active {
                    ACCENT_SOFT
                } else if hovered {
                    SURFACE_3
                } else {
                    SURFACE_2
                }
                .into(),
            ),
            text_color: if disabled {
                Color { a: 0.35, ..FG_3 }
            } else if active {
                ACCENT
            } else {
                FG_2
            },
            border: border(if active { ACCENT_LINE } else { LINE }, 1.0, R_SM),
            ..Default::default()
        }
    }
}

pub fn icon(color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |_, _| svg::Style { color: Some(color) }
}

/// Icon that follows its button's text color on hover.
pub fn icon_hover(idle: Color, hovered: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |_, status| svg::Style { color: Some(if status == svg::Status::Hovered { hovered } else { idle }) }
}

pub fn badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border(color, 0.0, 999.0),
        ..Default::default()
    }
}

pub fn count_badge(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(if active { ACCENT } else { FG_3 }),
        background: Some(if active { ACCENT_LINE } else { SURFACE_3 }.into()),
        border: border(Color::TRANSPARENT, 0.0, 999.0),
        ..Default::default()
    }
}

pub fn monogram(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(if active { ON_ACCENT } else { FG_2 }),
        background: Some(if active { ACCENT } else { SURFACE_3 }.into()),
        border: border(Color::TRANSPARENT, 0.0, R_MD),
        ..Default::default()
    }
}

pub fn scrim(_: &Theme) -> container::Style {
    container::Style { background: Some(SCRIM.into()), ..Default::default() }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border(LINE_STRONG, 1.0, R_LG),
        shadow: Shadow { color: Color { a: 0.7, ..Color::BLACK }, offset: Vector::new(0.0, 24.0), blur_radius: 60.0 },
        ..Default::default()
    }
}

pub fn keycap(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(FG_2),
        background: Some(SURFACE_2.into()),
        border: border(LINE_STRONG, 1.0, 4.0),
        ..Default::default()
    }
}

pub fn error_note(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(BAD),
        background: Some(BAD_SOFT.into()),
        border: border(Color { a: 0.4, ..BAD }, 1.0, R_SM),
        ..Default::default()
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let line = match status {
        text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Hovered => LINE_STRONG,
        _ => LINE,
    };
    text_input::Style {
        background: SURFACE_2.into(),
        border: border(line, 1.0, R_SM),
        icon: FG_3,
        placeholder: FG_3,
        value: FG,
        selection: ACCENT_LINE,
    }
}

pub fn scroller(_: &Theme, _status: scrollable::Status) -> scrollable::Style {
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { background: LINE_STRONG.into(), border: border(LINE_STRONG, 0.0, 999.0) },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: SURFACE.into(),
            border: border(LINE, 1.0, 999.0),
            shadow: Shadow::default(),
            icon: FG_2,
        },
    }
}

// ---- usage meters ----------------------------------------------------------------------

/// How full a limit is: the accent while there's room, amber from 70%, red from 90%.
pub fn level_color(used: f32) -> Color {
    if used >= 90.0 {
        BAD
    } else if used >= 70.0 {
        WARN
    } else {
        ACCENT
    }
}

/// `color` faded, for numbers that may be out of date.
pub fn faded(color: Color, stale: bool) -> Color {
    if stale { Color { a: color.a * 0.4, ..color } } else { color }
}

/// The empty part of a meter.
pub fn meter_track(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE_3.into()),
        border: border(Color::TRANSPARENT, 0.0, 999.0),
        ..Default::default()
    }
}

pub fn meter_fill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border(color, 0.0, 999.0),
        ..Default::default()
    }
}

/// The mark showing how much of a window's time has passed.
pub fn pace_tick(_: &Theme) -> container::Style {
    container::Style { background: Some(Color { a: 0.75, ..FG_2 }.into()), ..Default::default() }
}

/// One tracked account in the drawer.
pub fn usage_card(_: &Theme) -> container::Style {
    container::Style { background: Some(SURFACE_2.into()), border: border(LINE, 1.0, R_MD), ..Default::default() }
}

/// The unfilled ring of a system gauge.
pub const RING_TRACK: Color = LINE;

/// A system gauge's hover card: a tooltip with room for a chart and lists, lifted off the page.
pub fn hover_card(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(FG_2),
        background: Some(SURFACE_2.into()),
        border: border(LINE_STRONG, 1.0, R_MD),
        shadow: Shadow { color: Color { a: 0.45, ..Color::BLACK }, offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

// ---- editor ------------------------------------------------------------------------------

pub fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

/// A region of Editor mode (file tree, editor group, terminal): a card like a terminal pane, on its
/// own background, outlined in the accent while it has the keys.
pub fn region(background: Color, focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(background.into()),
        border: border(if focused { ACCENT_LINE } else { LINE }, 1.0, R_MD),
        ..Default::default()
    }
}

/// The strip of tabs atop an editor group.
pub fn tab_strip(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: Border { radius: Radius::default().top(R_MD), ..Default::default() },
        ..Default::default()
    }
}

/// An editor tab. The active one takes the editor's background, so it reads as the page's top.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(
                if active {
                    EDITOR_BG
                } else if hovered {
                    SURFACE_2
                } else {
                    Color::TRANSPARENT
                }
                .into(),
            ),
            text_color: if active {
                FG
            } else if hovered {
                FG_2
            } else {
                FG_3
            },
            border: Border { radius: Radius::default().top(R_SM), ..Default::default() },
            ..Default::default()
        }
    }
}

/// The accent line on top of the focused group's active tab.
pub fn tab_mark(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(color.into()), border: border(color, 0.0, 1.0), ..Default::default() }
}

/// A row of the file tree.
pub fn tree_row(selected: bool, focused: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, line) = match (selected, focused) {
            (true, true) => (ACCENT_SOFT, ACCENT_LINE),
            (true, false) => (SURFACE_3, Color::TRANSPARENT),
            (false, _) if hovered => (SURFACE_2, Color::TRANSPARENT),
            _ => (Color::TRANSPARENT, Color::TRANSPARENT),
        };
        button::Style {
            background: Some(bg.into()),
            text_color: FG_2,
            border: border(line, 1.0, R_SM),
            ..Default::default()
        }
    }
}

/// The Agents / Editor switch: a recessed track…
pub fn segmented(_: &Theme) -> container::Style {
    container::Style { background: Some(SURFACE.into()), border: border(LINE, 1.0, R_MD), ..Default::default() }
}

/// …and its segments; the active one is raised.
pub fn segment(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if active {
                Some(SURFACE_3.into())
            } else if hovered {
                Some(SURFACE_2.into())
            } else {
                None
            },
            text_color: if active {
                FG
            } else if hovered {
                FG_2
            } else {
                FG_3
            },
            border: border(if active { LINE_STRONG } else { Color::TRANSPARENT }, 1.0, R_SM),
            shadow: if active {
                Shadow { color: alpha(Color::BLACK, 0.35), offset: Vector::new(0.0, 1.0), blur_radius: 3.0 }
            } else {
                Shadow::default()
            },
            ..Default::default()
        }
    }
}

/// A coloured note inside a region (conflicts, files that can't be opened).
pub fn note(color: Color, soft: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(color),
        background: Some(soft.into()),
        border: border(alpha(color, 0.4), 1.0, R_SM),
        ..Default::default()
    }
}

/// A panel floating over content (find, quick open).
pub fn floating_panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE_2.into()),
        border: border(LINE_STRONG, 1.0, R_MD),
        shadow: Shadow { color: alpha(Color::BLACK, 0.5), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

/// A small on/off button (find's "Aa").
pub fn toggle_button(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, fg, line) = match (on, hovered) {
            (true, _) => (ACCENT_SOFT, ACCENT_HOVER, ACCENT_LINE),
            (false, true) => (SURFACE_3, FG, Color::TRANSPARENT),
            (false, false) => (Color::TRANSPARENT, FG_3, Color::TRANSPARENT),
        };
        button::Style {
            background: Some(bg.into()),
            text_color: fg,
            border: border(line, 1.0, R_SM),
            ..Default::default()
        }
    }
}

/// A row of a context menu. Disabled rows are dimmed.
pub fn menu_item(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered | button::Status::Pressed => (Some(SURFACE_3.into()), FG),
        button::Status::Active => (None, FG),
        button::Status::Disabled => (None, FG_3),
    };
    button::Style { background, text_color, border: border(Color::TRANSPARENT, 0.0, R_SM), ..Default::default() }
}

/// A row in a list of results (quick open); `selected` is the keyboard's.
pub fn result_row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if selected {
                Some(ACCENT_SOFT.into())
            } else if hovered {
                Some(SURFACE_3.into())
            } else {
                None
            },
            text_color: FG,
            border: border(Color::TRANSPARENT, 0.0, R_SM),
            ..Default::default()
        }
    }
}

/// The code editor: quiet line numbers, a faint current-line band, accent caret and brackets.
pub fn code_editor() -> crate::ui::editor::code_editor::Style {
    crate::ui::editor::code_editor::Style {
        background: EDITOR_BG,
        text: color!(0xd4d9e3),
        selection: alpha(ACCENT, 0.28),
        caret: ACCENT_HOVER,
        current_line: alpha(Color::WHITE, 0.035),
        line_number: color!(0x434b5a),
        line_number_active: FG_2,
        bracket: color!(0x9aa8c7),
        found: alpha(WARN, 0.22),
        found_current: WARN,
        thumb: alpha(FG, 0.2),
        thumb_active: alpha(FG, 0.36),
    }
}

/// Colour, bold and italic for a kind of code. One palette for every language, tuned to sit
/// beside the accent: cool hues for structure, warm ones for values.
pub fn syntax(class: Class) -> (Color, bool, bool) {
    match class {
        Class::Keyword => (color!(0xc4a7ff), false, false),
        Class::Operator => (color!(0x9aa8c7), false, false),
        Class::String => (color!(0xa6d189), false, false),
        Class::Escape => (color!(0xf4b8e4), false, false),
        Class::Number | Class::Constant => (color!(0xf5a97f), false, false),
        Class::Function => (color!(0x8fa6ff), false, false),
        Class::Macro => (color!(0x6fd3b8), false, false),
        Class::Type => (color!(0x7dd3e8), false, false),
        Class::Attribute => (color!(0xe9b872), false, false),
        Class::Parameter => (color!(0xeed49f), false, true),
        Class::Property => (color!(0xb4c3f0), false, false),
        Class::Tag => (color!(0xff8fa3), false, false),
        Class::Comment => (color!(0x6b7484), false, true),
        Class::Punctuation => (color!(0x7f889a), false, false),
        Class::Heading => (color!(0x8fa6ff), true, false),
        Class::Emphasis => (FG, false, true),
        Class::Strong => (FG, true, false),
        Class::Link => (color!(0x7dd3e8), false, false),
        Class::Added => (OK, false, false),
        Class::Removed | Class::Invalid => (BAD, false, false),
    }
}

/// A file icon's colour.
pub fn tint(tint: Tint) -> Color {
    match tint {
        Tint::Orange => color!(0xf5a97f),
        Tint::Blue => color!(0x8fa6ff),
        Tint::Yellow => color!(0xe9c46a),
        Tint::Green => color!(0x7fd18b),
        Tint::Purple => color!(0xc4a7ff),
        Tint::Red => color!(0xff8fa3),
        Tint::Teal => color!(0x6fd3b8),
        Tint::Gray => FG_3,
    }
}

pub fn to_color(rgb: pw_term::Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}
