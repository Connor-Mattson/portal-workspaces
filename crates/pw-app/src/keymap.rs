//! Keyboard shortcuts, and translating other key presses for the terminal.
//!
//! The app modifier is Cmd on macOS and Ctrl+Shift elsewhere, so shortcuts never take a
//! Ctrl-key away from the shell.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use pw_model::Axis;
use pw_term::{KeyInput, Mods, NamedKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewWorkspace,
    EditWorkspace,
    SelectWorkspace(usize),
    NextWorkspace,
    PrevWorkspace,
    ToggleSidebar,
    Split(Axis),
    ClosePane,
    Focus(Direction),
    ToggleMaximize,
    Copy,
    Paste,
    FontBigger,
    FontSmaller,
    FontReset,
    ScrollPageUp,
    ScrollPageDown,
    ShowShortcuts,
    Quit,
}

/// Whether the platform's app modifier is held.
fn app_mod(m: Modifiers) -> bool {
    if cfg!(target_os = "macos") { m.logo() && !m.control() } else { m.control() && m.shift() && !m.logo() }
}

/// The shortcut bound to a key press, if any.
pub fn action(key: &Key, m: Modifiers) -> Option<Action> {
    use Action::*;
    if let Key::Named(named) = key {
        match named {
            Named::PageUp if m == Modifiers::SHIFT => return Some(ScrollPageUp),
            Named::PageDown if m == Modifiers::SHIFT => return Some(ScrollPageDown),
            _ => {}
        }
    }
    if !app_mod(m) {
        return None;
    }
    // On macOS Shift is free to mean "the other one"; on Linux it's part of the app modifier.
    let shifted = cfg!(target_os = "macos") && m.shift();
    match key {
        Key::Named(Named::Enter) => Some(ToggleMaximize),
        Key::Named(Named::ArrowLeft) => Some(Focus(Direction::Left)),
        Key::Named(Named::ArrowRight) => Some(Focus(Direction::Right)),
        Key::Named(Named::ArrowUp) => Some(Focus(Direction::Up)),
        Key::Named(Named::ArrowDown) => Some(Focus(Direction::Down)),
        Key::Character(c) => match c.to_lowercase().as_str() {
            "n" => Some(NewWorkspace),
            "," => Some(EditWorkspace),
            "b" => Some(ToggleSidebar),
            "d" if shifted => Some(Split(Axis::Horizontal)),
            "d" => Some(Split(Axis::Vertical)),
            "e" => Some(Split(Axis::Horizontal)),
            "w" => Some(ClosePane),
            "c" => Some(Copy),
            "v" => Some(Paste),
            "=" | "+" => Some(FontBigger),
            "-" | "_" => Some(FontSmaller),
            "0" => Some(FontReset),
            "[" | "{" => Some(PrevWorkspace),
            "]" | "}" => Some(NextWorkspace),
            "/" | "?" => Some(ShowShortcuts),
            "q" => Some(Quit),
            digit => digit.parse::<usize>().ok().filter(|d| (1..=9).contains(d)).map(|d| SelectWorkspace(d - 1)),
        },
        _ => None,
    }
}

/// Human-readable shortcut list for the help sheet, in display order.
pub fn cheatsheet() -> Vec<(&'static str, String)> {
    let m = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+Shift+" };
    let split_down = if cfg!(target_os = "macos") { format!("{m}⇧D") } else { format!("{m}E") };
    vec![
        ("New workspace", format!("{m}N")),
        ("Edit workspace", format!("{m},")),
        ("Switch to workspace 1–9", format!("{m}1…9")),
        ("Previous / next workspace", format!("{m}[  {m}]")),
        ("Toggle drawer", format!("{m}B")),
        ("Split right", format!("{m}D")),
        ("Split down", split_down),
        ("Close terminal", format!("{m}W")),
        ("Move focus", format!("{m}Arrows")),
        ("Maximize terminal", format!("{m}Enter")),
        ("Copy / paste", format!("{m}C  {m}V")),
        ("Font size", format!("{m}=  {m}-  {m}0")),
        ("Scroll history", "Shift+PgUp  Shift+PgDn".to_owned()),
        ("Shortcuts", format!("{m}/")),
        ("Quit", format!("{m}Q")),
    ]
}

/// Translates a key press into terminal input.
pub fn to_terminal(key: &Key, m: Modifiers, text: Option<&str>) -> Option<KeyInput> {
    let mods = Mods { shift: m.shift(), ctrl: m.control(), alt: m.alt(), logo: m.logo() };
    let key = match key {
        Key::Named(named) => match named {
            Named::Enter => pw_term::Key::Named(NamedKey::Enter),
            Named::Tab => pw_term::Key::Named(NamedKey::Tab),
            Named::Backspace => pw_term::Key::Named(NamedKey::Backspace),
            Named::Escape => pw_term::Key::Named(NamedKey::Escape),
            Named::ArrowUp => pw_term::Key::Named(NamedKey::Up),
            Named::ArrowDown => pw_term::Key::Named(NamedKey::Down),
            Named::ArrowLeft => pw_term::Key::Named(NamedKey::Left),
            Named::ArrowRight => pw_term::Key::Named(NamedKey::Right),
            Named::Home => pw_term::Key::Named(NamedKey::Home),
            Named::End => pw_term::Key::Named(NamedKey::End),
            Named::PageUp => pw_term::Key::Named(NamedKey::PageUp),
            Named::PageDown => pw_term::Key::Named(NamedKey::PageDown),
            Named::Insert => pw_term::Key::Named(NamedKey::Insert),
            Named::Delete => pw_term::Key::Named(NamedKey::Delete),
            Named::Space => pw_term::Key::Char(' '),
            Named::F1 => pw_term::Key::Named(NamedKey::F(1)),
            Named::F2 => pw_term::Key::Named(NamedKey::F(2)),
            Named::F3 => pw_term::Key::Named(NamedKey::F(3)),
            Named::F4 => pw_term::Key::Named(NamedKey::F(4)),
            Named::F5 => pw_term::Key::Named(NamedKey::F(5)),
            Named::F6 => pw_term::Key::Named(NamedKey::F(6)),
            Named::F7 => pw_term::Key::Named(NamedKey::F(7)),
            Named::F8 => pw_term::Key::Named(NamedKey::F(8)),
            Named::F9 => pw_term::Key::Named(NamedKey::F(9)),
            Named::F10 => pw_term::Key::Named(NamedKey::F(10)),
            Named::F11 => pw_term::Key::Named(NamedKey::F(11)),
            Named::F12 => pw_term::Key::Named(NamedKey::F(12)),
            // Modifier keys alone, media keys and the like send nothing.
            _ => return None,
        },
        Key::Character(c) => pw_term::Key::Char(c.chars().next()?),
        Key::Unidentified => pw_term::Key::Char(text?.chars().next()?),
    };
    // Control characters in `text` (e.g. from Ctrl combos) are re-derived by the encoder.
    let text = text.filter(|t| !t.chars().any(char::is_control)).map(str::to_owned);
    Some(KeyInput { key, mods, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Modifiers {
        if cfg!(target_os = "macos") { Modifiers::LOGO } else { Modifiers::CTRL | Modifiers::SHIFT }
    }

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    #[test]
    fn shortcuts_need_the_app_modifier() {
        assert_eq!(action(&ch("n"), app()), Some(Action::NewWorkspace));
        assert_eq!(action(&ch("n"), Modifiers::CTRL), None);
        assert_eq!(action(&ch("3"), app()), Some(Action::SelectWorkspace(2)));
        assert_eq!(action(&ch("d"), app()), Some(Action::Split(Axis::Vertical)));
    }

    #[test]
    fn plain_ctrl_keys_go_to_the_terminal() {
        let input = to_terminal(&ch("c"), Modifiers::CTRL, Some("\u{3}")).unwrap();
        assert_eq!(input.key, pw_term::Key::Char('c'));
        assert!(input.mods.ctrl);
        assert_eq!(input.text, None);
    }

    #[test]
    fn shift_page_scrolls_history() {
        assert_eq!(action(&Key::Named(Named::PageUp), Modifiers::SHIFT), Some(Action::ScrollPageUp));
    }
}
