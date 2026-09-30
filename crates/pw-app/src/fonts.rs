//! Bundled fonts, so cell metrics are identical on every machine.

use iced::Font;
use iced::font::{Family, Style, Weight};

pub const DATA: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMonoNL-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMonoNL-Bold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMonoNL-Italic.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMonoNL-BoldItalic.ttf"),
];

const INTER: Family = Family::Name("Inter");
const MONO_FAMILY: Family = Family::Name("JetBrains Mono NL");

pub const UI: Font = Font { family: INTER, ..Font::DEFAULT };
pub const UI_MEDIUM: Font = Font { family: INTER, weight: Weight::Medium, ..Font::DEFAULT };
pub const UI_SEMIBOLD: Font = Font { family: INTER, weight: Weight::Semibold, ..Font::DEFAULT };
pub const MONO: Font = Font { family: MONO_FAMILY, ..Font::DEFAULT };

pub fn mono(bold: bool, italic: bool) -> Font {
    Font {
        family: MONO_FAMILY,
        weight: if bold { Weight::Bold } else { Weight::Normal },
        style: if italic { Style::Italic } else { Style::Normal },
        ..Font::DEFAULT
    }
}

/// Terminal cell size in logical pixels for a font size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    pub font_size: f32,
    pub width: f32,
    pub height: f32,
}

impl CellMetrics {
    /// JetBrains Mono's advance is 600/1000 em; its ascent + descent is 1320/1000 em.
    /// Heights are rounded to whole pixels so rows never blur.
    pub fn for_size(font_size: f32) -> Self {
        Self { font_size, width: font_size * 0.6, height: (font_size * 1.32).round() }
    }
}
