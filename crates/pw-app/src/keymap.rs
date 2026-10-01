//! Keyboard shortcuts, and translating other key presses for the terminal.
//!
//! The app modifier is Cmd on macOS and Ctrl+Shift elsewhere, so shortcuts never take a
//! Ctrl-key away from the shell. Editing shortcuts (save, undo, find) use the editor modifier,
//! Cmd or Ctrl, and only while the code editor or the file tree has the keys: in a terminal,
//! plain Ctrl-keys belong to the shell. The one exception is Ctrl+C on Linux, which copies a
//! selection you can see before it interrupts (see [`copies_selection`] and ADR 0011).

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

/// What has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Terminal,
    Editor,
    Explorer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewWorkspace,
    EditWorkspace,
    SelectWorkspace(usize),
    NextWorkspace,
    PrevWorkspace,
    /// Goes to the terminal that has needed you longest (or finished first).
    NextAttention,
    ToggleSidebar,
    /// Agents ⇄ Editor for the active workspace.
    ToggleMode,
    /// Splits the focused terminal, or the focused editor group.
    Split(Axis),
    /// Closes the focused terminal, or the focused editor tab.
    ClosePane,
    Focus(Direction),
    ToggleMaximize,
    /// Moves the focused terminal into its own window, or docks the detached one back.
    ToggleDetach,
    Copy,
    Paste,
    /// Selects a terminal's whole history and screen.
    SelectAll,
    FontBigger,
    FontSmaller,
    FontReset,
    ScrollPageUp,
    ScrollPageDown,
    ShowShortcuts,
    Quit,
    // Editor mode.
    ToggleTerminal,
    ToggleExplorer,
    QuickOpen,
    Save,
    Undo,
    Redo,
    Find,
    Replace,
    FindNext,
    FindPrev,
    ToggleComment,
    MoveLines {
        up: bool,
    },
    DuplicateLines {
        down: bool,
    },
    NextTab,
    PrevTab,
}

/// Whether the platform's app modifier is held.
fn app_mod(m: Modifiers) -> bool {
    if cfg!(target_os = "macos") { m.logo() && !m.control() } else { m.control() && m.shift() && !m.logo() }
}

/// Whether the platform's editing modifier (Cmd, or Ctrl) is held.
fn editor_mod(m: Modifiers) -> bool {
    if cfg!(target_os = "macos") { m.logo() && !m.control() } else { m.control() && !m.logo() && !m.alt() }
}

/// The shortcut bound to a key press in a context, if any.
pub fn action(key: &Key, m: Modifiers, context: Context) -> Option<Action> {
    if context != Context::Terminal
        && let Some(action) = editing(key, m)
    {
        return Some(action);
    }
    app(key, m)
}

/// Shortcuts of the code editor and the file tree.
fn editing(key: &Key, m: Modifiers) -> Option<Action> {
    use Action::*;
    let alt_only = m.alt() && !m.control() && !m.logo();
    let plain = !m.control() && !m.logo() && !m.alt();
    match key {
        Key::Named(Named::ArrowUp) if alt_only => {
            return Some(if m.shift() { DuplicateLines { down: false } } else { MoveLines { up: true } });
        }
        Key::Named(Named::ArrowDown) if alt_only => {
            return Some(if m.shift() { DuplicateLines { down: true } } else { MoveLines { up: false } });
        }
        Key::Named(Named::F3) if plain => return Some(if m.shift() { FindPrev } else { FindNext }),
        Key::Named(Named::Tab) if m.control() && !m.logo() => return Some(if m.shift() { PrevTab } else { NextTab }),
        Key::Named(Named::PageDown) if m.control() && !m.shift() => return Some(NextTab),
        Key::Named(Named::PageUp) if m.control() && !m.shift() => return Some(PrevTab),
        _ => {}
    }
    if !editor_mod(m) {
        return None;
    }
    let Key::Character(c) = key else { return None };
    let shift = m.shift();
    match c.to_lowercase().as_str() {
        "s" if !shift => Some(Save),
        "z" if shift => Some(Redo),
        "z" => Some(Undo),
        "y" if !shift => Some(Redo),
        // ⌘H hides the app on macOS, so replace is ⌘⌥F there.
        "f" if cfg!(target_os = "macos") && m.alt() => Some(Replace),
        "f" if !shift => Some(Find),
        "h" if !shift && !cfg!(target_os = "macos") => Some(Replace),
        "/" if !shift => Some(ToggleComment),
        "p" if !shift => Some(QuickOpen),
        "w" if !shift => Some(ClosePane),
        "\\" if !shift => Some(Split(Axis::Vertical)),
        _ => None,
    }
}

/// Shortcuts that work everywhere.
fn app(key: &Key, m: Modifiers) -> Option<Action> {
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
            // ⌘M minimizes on macOS, so the mode switch is ⌘⇧M there.
            "m" if shifted || !cfg!(target_os = "macos") => Some(ToggleMode),
            "j" => Some(ToggleTerminal),
            "t" => Some(ToggleExplorer),
            "p" => Some(QuickOpen),
            "s" => Some(Save),
            "d" if shifted => Some(Split(Axis::Horizontal)),
            "d" => Some(Split(Axis::Vertical)),
            "e" => Some(Split(Axis::Horizontal)),
            "w" => Some(ClosePane),
            "o" => Some(ToggleDetach),
            "c" => Some(Copy),
            "v" => Some(Paste),
            "a" => Some(SelectAll),
            "=" | "+" => Some(FontBigger),
            "-" | "_" => Some(FontSmaller),
            "0" => Some(FontReset),
            "[" | "{" => Some(PrevWorkspace),
            "]" | "}" => Some(NextWorkspace),
            "i" => Some(NextAttention),
            "/" | "?" => Some(ShowShortcuts),
            "q" => Some(Quit),
            digit => digit.parse::<usize>().ok().filter(|d| (1..=9).contains(d)).map(|d| SelectWorkspace(d - 1)),
        },
        _ => None,
    }
}

/// Whether a key press in a terminal copies its selection, when one is on screen, instead of
/// reaching the program: Ctrl+C on Linux, as in Windows Terminal or VS Code. With nothing selected
/// it's an interrupt as always, and copying drops the selection, so the next Ctrl+C interrupts.
/// macOS copies with ⌘C and keeps Ctrl+C for the shell.
pub fn copies_selection(key: &Key, m: Modifiers) -> bool {
    !cfg!(target_os = "macos")
        && m == Modifiers::CTRL
        && matches!(key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
}

/// A shortcut's label: the app modifier, then the key.
pub fn shortcut_label(key: &str) -> String {
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+Shift+{key}") }
}

/// The help sheet: sections of (what, keys), in display order.
pub fn cheatsheet() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let mac = cfg!(target_os = "macos");
    let m = if mac { "⌘" } else { "Ctrl+Shift+" };
    let e = if mac { "⌘" } else { "Ctrl+" };
    let split_down = if mac { format!("{m}⇧D") } else { format!("{m}E") };
    vec![
        (
            "Workspaces",
            vec![
                ("New workspace", format!("{m}N")),
                ("Edit workspace", format!("{m},")),
                ("Switch to workspace 1–9", format!("{m}1…9")),
                ("Previous / next workspace", format!("{m}[  {m}]")),
                ("Go to the terminal that needs you", format!("{m}I")),
                ("Agents / Editor", if mac { "⌘⇧M".to_owned() } else { format!("{m}M") }),
                ("Toggle drawer", format!("{m}B")),
                ("Shortcuts", format!("{m}/")),
                ("Quit", format!("{m}Q")),
            ],
        ),
        (
            "Agents",
            vec![
                ("Split right", format!("{m}D")),
                ("Split down", split_down),
                ("Close terminal", format!("{m}W")),
                ("Move focus", format!("{m}Arrows")),
                ("Maximize terminal", format!("{m}Enter")),
                ("Terminal in its own window / dock it", format!("{m}O")),
                ("Copy / paste", format!("{m}C  {m}V")),
                ("Select all", format!("{m}A")),
                ("Font size", format!("{m}=  {m}-  {m}0")),
                ("Scroll history", "Shift+PgUp  Shift+PgDn".to_owned()),
            ],
        ),
        ("Terminal text", {
            let mut keys = vec![
                ("Copy, paste, select all", "Right-click".to_owned()),
                ("Select in apps that use the mouse", "Shift+drag".to_owned()),
                ("Paste the last selected text", "Middle-click".to_owned()),
            ];
            if !mac {
                keys.insert(0, ("Copy the selection, or interrupt", "Ctrl+C".to_owned()));
            }
            keys
        }),
        (
            "Editor",
            vec![
                ("Quick open", format!("{e}P")),
                ("Save", format!("{e}S")),
                ("Toggle terminal", format!("{m}J")),
                ("Toggle file tree", format!("{m}T")),
                ("Split editor", format!("{e}\\")),
                ("Close tab", format!("{e}W")),
                ("Next / previous tab", "Ctrl+Tab  Ctrl+Shift+Tab".to_owned()),
                ("Find / replace", if mac { "⌘F  ⌘⌥F".to_owned() } else { "Ctrl+F  Ctrl+H".to_owned() }),
                ("Next / previous match", "F3  Shift+F3".to_owned()),
                ("Undo / redo", if mac { "⌘Z  ⌘⇧Z".to_owned() } else { "Ctrl+Z  Ctrl+Y".to_owned() }),
                ("Comment lines", format!("{e}/")),
                (
                    "Move / duplicate lines",
                    if mac { "⌥↑↓  ⌥⇧↑↓".to_owned() } else { "Alt+↑↓  Alt+Shift+↑↓".to_owned() },
                ),
                ("File tree: rename / delete", "F2  Delete".to_owned()),
            ],
        ),
    ]
}

/// The few keys worth showing in an empty editor group: (what, keys, the action they run).
pub fn editor_hints() -> Vec<(&'static str, String, Action)> {
    let mac = cfg!(target_os = "macos");
    let m = if mac { "⌘" } else { "Ctrl+Shift+" };
    let e = if mac { "⌘" } else { "Ctrl+" };
    vec![
        ("Quick open", format!("{e}P"), Action::QuickOpen),
        ("Toggle terminal", format!("{m}J"), Action::ToggleTerminal),
        ("Toggle file tree", format!("{m}T"), Action::ToggleExplorer),
        ("Split editor", format!("{e}\\"), Action::Split(Axis::Vertical)),
        ("Back to agents", if mac { "⌘⇧M".to_owned() } else { format!("{m}M") }, Action::ToggleMode),
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

    fn editor() -> Modifiers {
        if cfg!(target_os = "macos") { Modifiers::LOGO } else { Modifiers::CTRL }
    }

    #[test]
    fn shortcuts_need_the_app_modifier() {
        let t = Context::Terminal;
        assert_eq!(action(&ch("n"), app(), t), Some(Action::NewWorkspace));
        assert_eq!(action(&ch("n"), Modifiers::CTRL, t), None);
        assert_eq!(action(&ch("3"), app(), t), Some(Action::SelectWorkspace(2)));
        assert_eq!(action(&ch("d"), app(), t), Some(Action::Split(Axis::Vertical)));
        assert_eq!(action(&ch("o"), app(), t), Some(Action::ToggleDetach));
        assert_eq!(action(&ch("o"), Modifiers::CTRL, t), None);
        assert_eq!(action(&ch("j"), app(), t), Some(Action::ToggleTerminal));
    }

    #[test]
    fn editing_keys_only_outside_terminals() {
        assert_eq!(action(&ch("s"), editor(), Context::Editor), Some(Action::Save));
        assert_eq!(action(&ch("z"), editor(), Context::Explorer), Some(Action::Undo));
        assert_eq!(action(&ch("f"), editor(), Context::Editor), Some(Action::Find));
        assert_eq!(action(&ch("w"), editor(), Context::Editor), Some(Action::ClosePane));
        assert_eq!(
            action(&Key::Named(Named::ArrowUp), Modifiers::ALT, Context::Editor),
            Some(Action::MoveLines { up: true })
        );
        // In a terminal these reach the shell.
        if !cfg!(target_os = "macos") {
            assert_eq!(action(&ch("s"), Modifiers::CTRL, Context::Terminal), None);
            assert_eq!(action(&ch("z"), Modifiers::CTRL, Context::Terminal), None);
            assert_eq!(action(&ch("z"), app(), Context::Editor), Some(Action::Redo));
        }
        assert_eq!(action(&Key::Named(Named::ArrowUp), Modifiers::ALT, Context::Terminal), None);
        // App shortcuts still work in the editor.
        assert_eq!(action(&ch("n"), app(), Context::Editor), Some(Action::NewWorkspace));
    }

    #[test]
    fn plain_ctrl_keys_go_to_the_terminal() {
        let input = to_terminal(&ch("c"), Modifiers::CTRL, Some("\u{3}")).unwrap();
        assert_eq!(input.key, pw_term::Key::Char('c'));
        assert!(input.mods.ctrl);
        assert_eq!(input.text, None);
    }

    #[test]
    fn ctrl_c_copies_only_on_linux() {
        let linux = !cfg!(target_os = "macos");
        assert_eq!(copies_selection(&ch("c"), Modifiers::CTRL), linux);
        // Caps Lock reports an upper-case key.
        assert_eq!(copies_selection(&ch("C"), Modifiers::CTRL), linux);
        assert!(!copies_selection(&ch("c"), Modifiers::CTRL | Modifiers::SHIFT));
        assert!(!copies_selection(&ch("c"), Modifiers::CTRL | Modifiers::ALT));
        assert!(!copies_selection(&ch("d"), Modifiers::CTRL));
        assert!(!copies_selection(&ch("c"), Modifiers::empty()));
        assert_eq!(action(&ch("a"), app(), Context::Terminal), Some(Action::SelectAll));
        assert_eq!(action(&ch("c"), app(), Context::Terminal), Some(Action::Copy));
    }

    #[test]
    fn shift_page_scrolls_history() {
        assert_eq!(action(&Key::Named(Named::PageUp), Modifiers::SHIFT, Context::Terminal), Some(Action::ScrollPageUp));
    }
}
