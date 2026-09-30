//! Colors and the palette used to resolve terminal colors to RGB.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn hex(hex: u32) -> Self {
        Self { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
    }

    /// Linear blend towards `other` by `t` in `[0, 1]`.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb { r: lerp(self.r, other.r), g: lerp(self.g, other.g), b: lerp(self.b, other.b) }
    }
}

impl From<alacritty_terminal::vte::ansi::Rgb> for Rgb {
    fn from(c: alacritty_terminal::vte::ansi::Rgb) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

impl From<Rgb> for alacritty_terminal::vte::ansi::Rgb {
    fn from(c: Rgb) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

/// The terminal's color scheme.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// The 16 ANSI colors: 8 normal then 8 bright.
    pub ansi: [Rgb; 16],
}

impl Default for Palette {
    /// Tuned to Science Portal's dark tokens (`--term-bg`, `--term-fg`, `--accent`).
    fn default() -> Self {
        Self {
            foreground: Rgb::hex(0xd6dbe4),
            background: Rgb::hex(0x07090d),
            cursor: Rgb::hex(0x9fb0ff),
            selection: Rgb::hex(0x2e3c78),
            ansi: [
                Rgb::hex(0x1b1f27), // black
                Rgb::hex(0xff6f7b), // red
                Rgb::hex(0x4cc987), // green
                Rgb::hex(0xeeb152), // yellow
                Rgb::hex(0x6ea3ff), // blue
                Rgb::hex(0xb597ff), // magenta
                Rgb::hex(0x5fd0d6), // cyan
                Rgb::hex(0xc4cad6), // white
                Rgb::hex(0x5c6577), // bright black
                Rgb::hex(0xff8f98), // bright red
                Rgb::hex(0x74dca4), // bright green
                Rgb::hex(0xf5c87a), // bright yellow
                Rgb::hex(0x93bbff), // bright blue
                Rgb::hex(0xcab5ff), // bright magenta
                Rgb::hex(0x87e0e4), // bright cyan
                Rgb::hex(0xf2f4f8), // bright white
            ],
        }
    }
}

impl Palette {
    /// Resolves a cell color. `overrides` are colors the program set with OSC 4/10/11/12.
    pub(crate) fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb.into(),
            Color::Indexed(i) => overrides[i as usize].map_or_else(|| self.indexed(i), Rgb::from),
            Color::Named(named) => overrides[named].map_or_else(|| self.named(named), Rgb::from),
        }
    }

    pub(crate) fn named(&self, named: NamedColor) -> Rgb {
        use NamedColor::*;
        match named {
            Foreground | BrightForeground => self.foreground,
            Background => self.background,
            Cursor => self.cursor,
            DimForeground => self.foreground.mix(self.background, 0.35),
            DimBlack | DimRed | DimGreen | DimYellow | DimBlue | DimMagenta | DimCyan | DimWhite => {
                let base = named as usize - DimBlack as usize;
                self.ansi[base].mix(self.background, 0.35)
            }
            other => self.ansi[(other as usize).min(15)],
        }
    }

    /// The xterm 256-color table: 16 palette colors, a 6×6×6 cube, then 24 grays.
    pub(crate) fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb { r: level(i / 36), g: level((i / 6) % 6), b: level(i % 6) }
            }
            232..=255 => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays_match_xterm() {
        let p = Palette::default();
        assert_eq!(p.indexed(16), Rgb::hex(0x000000));
        assert_eq!(p.indexed(196), Rgb::hex(0xff0000));
        assert_eq!(p.indexed(231), Rgb::hex(0xffffff));
        assert_eq!(p.indexed(232), Rgb::hex(0x080808));
        assert_eq!(p.indexed(255), Rgb::hex(0xeeeeee));
    }

    #[test]
    fn named_and_indexed_agree_on_ansi() {
        let p = Palette::default();
        assert_eq!(p.named(NamedColor::BrightBlue), p.indexed(12));
    }
}
