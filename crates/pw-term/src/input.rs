//! Turns key presses into the bytes a terminal program expects.
//!
//! Covers the xterm legacy encoding plus the kitty keyboard protocol's "disambiguate" level,
//! which is what lets agents like Claude Code tell Shift+Enter from Enter.

use alacritty_terminal::term::TermMode;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Cmd on macOS, Super elsewhere. Combos with it are app shortcuts, never sent to the shell.
    pub logo: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, ctrl: false, alt: false, logo: false };

    pub fn any(self) -> bool {
        self.shift || self.ctrl || self.alt || self.logo
    }

    /// The xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl + 8·super.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8 + 8 * self.logo as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Named(NamedKey),
    /// The key's character without modifiers applied (`'a'` for Shift+A, `' '` for space).
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInput {
    pub key: Key,
    pub mods: Mods,
    /// Text the key produces with modifiers and keyboard layout applied, if any.
    pub text: Option<String>,
}

/// Bytes for `input` under the terminal's current `mode`, or `None` if the key sends nothing.
pub(crate) fn encode(input: &KeyInput, mode: TermMode) -> Option<Vec<u8>> {
    let kitty = mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL);
    let mods = input.mods;
    match &input.key {
        Key::Named(named) => encode_named(*named, mods, mode, kitty),
        Key::Char(c) => encode_char(*c, input.text.as_deref(), mods, kitty),
    }
}

fn encode_named(key: NamedKey, mods: Mods, mode: TermMode, kitty: bool) -> Option<Vec<u8>> {
    use NamedKey::*;
    let bytes = match key {
        Enter if kitty && mods.any() => csi_u(13, mods),
        Enter if mods.alt => b"\x1b\r".to_vec(),
        Enter => b"\r".to_vec(),
        Tab if kitty && mods.any() => csi_u(9, mods),
        Tab if mods.shift => b"\x1b[Z".to_vec(),
        Tab if mods.alt => b"\x1b\t".to_vec(),
        Tab => b"\t".to_vec(),
        Backspace if kitty && mods.any() => csi_u(127, mods),
        Backspace if mods.ctrl => vec![0x08],
        Backspace if mods.alt => b"\x1b\x7f".to_vec(),
        Backspace => vec![0x7f],
        Escape if kitty => csi_u(27, mods),
        Escape if mods.alt => b"\x1b\x1b".to_vec(),
        Escape => b"\x1b".to_vec(),
        Up => cursor_key(b'A', mods, mode),
        Down => cursor_key(b'B', mods, mode),
        Right => cursor_key(b'C', mods, mode),
        Left => cursor_key(b'D', mods, mode),
        Home => cursor_key(b'H', mods, mode),
        End => cursor_key(b'F', mods, mode),
        Insert => tilde(2, mods),
        Delete => tilde(3, mods),
        PageUp => tilde(5, mods),
        PageDown => tilde(6, mods),
        F(n @ 1..=4) => {
            let letter = b'P' + (n - 1);
            if mods.any() {
                format!("\x1b[1;{}{}", mods.param(), letter as char).into_bytes()
            } else {
                vec![0x1b, b'O', letter]
            }
        }
        F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize], mods),
        F(_) => return None,
    };
    Some(bytes)
}

fn encode_char(c: char, text: Option<&str>, mods: Mods, kitty: bool) -> Option<Vec<u8>> {
    if mods.logo {
        return None;
    }
    if kitty && (mods.ctrl || mods.alt) {
        return Some(csi_u(c.to_ascii_lowercase() as u32, mods));
    }
    if mods.ctrl {
        let mut out = Vec::with_capacity(2);
        if mods.alt {
            out.push(0x1b);
        }
        match ctrl_code(c) {
            Some(code) => out.push(code),
            // Ctrl with a key that has no control code: send the plain text, as xterm does.
            None => out.extend_from_slice(text.unwrap_or_default().as_bytes()),
        }
        return (out.len() > mods.alt as usize).then_some(out);
    }
    let text = match text {
        Some(t) if !t.is_empty() => t.to_owned(),
        _ => c.to_string(),
    };
    let mut out = Vec::with_capacity(text.len() + 1);
    if mods.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    Some(out)
}

fn ctrl_code(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

fn cursor_key(letter: u8, mods: Mods, mode: TermMode) -> Vec<u8> {
    if mods.any() {
        format!("\x1b[1;{}{}", mods.param(), letter as char).into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', letter]
    } else {
        vec![0x1b, b'[', letter]
    }
}

fn tilde(code: u8, mods: Mods) -> Vec<u8> {
    if mods.any() { format!("\x1b[{code};{}~", mods.param()) } else { format!("\x1b[{code}~") }.into_bytes()
}

fn csi_u(code: u32, mods: Mods) -> Vec<u8> {
    if mods.any() { format!("\x1b[{code};{}u", mods.param()) } else { format!("\x1b[{code}u") }.into_bytes()
}

/// Bytes for pasting `text`: bracketed when the program asked for it, so shells and agents
/// don't execute pasted newlines.
pub(crate) fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        // Strip anything that could end the bracket early.
        let clean = text.replace("\x1b[201~", "");
        let mut out = Vec::with_capacity(clean.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, mods: Mods, text: Option<&str>) -> KeyInput {
        KeyInput { key, mods, text: text.map(str::to_owned) }
    }

    fn enc(input: KeyInput, mode: TermMode) -> Vec<u8> {
        encode(&input, mode).unwrap_or_default()
    }

    const CTRL: Mods = Mods { ctrl: true, ..Mods::NONE };
    const ALT: Mods = Mods { alt: true, ..Mods::NONE };
    const SHIFT: Mods = Mods { shift: true, ..Mods::NONE };

    #[test]
    fn plain_text_uses_layout_text() {
        let m = TermMode::default();
        assert_eq!(enc(key(Key::Char('a'), SHIFT, Some("A")), m), b"A");
        assert_eq!(enc(key(Key::Char('é'), Mods::NONE, Some("é")), m), "é".as_bytes());
        assert_eq!(enc(key(Key::Char(' '), Mods::NONE, Some(" ")), m), b" ");
    }

    #[test]
    fn ctrl_and_alt_combos() {
        let m = TermMode::default();
        assert_eq!(enc(key(Key::Char('c'), CTRL, None), m), [0x03]);
        assert_eq!(enc(key(Key::Char(' '), CTRL, None), m), [0x00]);
        assert_eq!(enc(key(Key::Char('['), CTRL, None), m), [0x1b]);
        assert_eq!(enc(key(Key::Char('b'), ALT, Some("b")), m), b"\x1bb");
        assert_eq!(enc(key(Key::Char('d'), Mods { alt: true, ..CTRL }, None), m), [0x1b, 0x04]);
    }

    #[test]
    fn logo_combos_are_never_sent() {
        let logo = Mods { logo: true, ..Mods::NONE };
        assert_eq!(encode(&key(Key::Char('c'), logo, Some("c")), TermMode::default()), None);
    }

    #[test]
    fn cursor_keys_follow_app_cursor_mode() {
        let up = || key(Key::Named(NamedKey::Up), Mods::NONE, None);
        assert_eq!(enc(up(), TermMode::default()), b"\x1b[A");
        assert_eq!(enc(up(), TermMode::APP_CURSOR), b"\x1bOA");
        assert_eq!(enc(key(Key::Named(NamedKey::Left), CTRL, None), TermMode::APP_CURSOR), b"\x1b[1;5D");
    }

    #[test]
    fn special_keys() {
        let m = TermMode::default();
        assert_eq!(enc(key(Key::Named(NamedKey::Enter), Mods::NONE, None), m), b"\r");
        assert_eq!(enc(key(Key::Named(NamedKey::Tab), SHIFT, None), m), b"\x1b[Z");
        assert_eq!(enc(key(Key::Named(NamedKey::Backspace), Mods::NONE, None), m), [0x7f]);
        assert_eq!(enc(key(Key::Named(NamedKey::Delete), Mods::NONE, None), m), b"\x1b[3~");
        assert_eq!(enc(key(Key::Named(NamedKey::PageUp), SHIFT, None), m), b"\x1b[5;2~");
        assert_eq!(enc(key(Key::Named(NamedKey::F(1)), Mods::NONE, None), m), b"\x1bOP");
        assert_eq!(enc(key(Key::Named(NamedKey::F(5)), Mods::NONE, None), m), b"\x1b[15~");
        assert_eq!(enc(key(Key::Named(NamedKey::F(12)), CTRL, None), m), b"\x1b[24;5~");
    }

    #[test]
    fn kitty_disambiguates_modified_keys() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES;
        assert_eq!(enc(key(Key::Named(NamedKey::Enter), SHIFT, None), m), b"\x1b[13;2u");
        assert_eq!(enc(key(Key::Named(NamedKey::Enter), Mods::NONE, None), m), b"\r");
        assert_eq!(enc(key(Key::Named(NamedKey::Escape), Mods::NONE, None), m), b"\x1b[27u");
        assert_eq!(enc(key(Key::Char('c'), CTRL, None), m), b"\x1b[99;5u");
        assert_eq!(enc(key(Key::Char('a'), SHIFT, Some("A")), m), b"A");
    }

    #[test]
    fn paste_brackets_and_sanitizes() {
        assert_eq!(encode_paste("a\nb", TermMode::default()), b"a\rb");
        assert_eq!(encode_paste("x\x1b[201~y", TermMode::BRACKETED_PASTE), b"\x1b[200~xy\x1b[201~");
    }
}
