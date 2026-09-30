//! Mouse reporting for programs that ask for it (vim, htop, TUIs).

use alacritty_terminal::term::TermMode;

use crate::input::Mods;
use crate::session::GridPoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    Press(MouseButton),
    Release(MouseButton),
    /// Motion with a button held.
    Drag(MouseButton),
    /// Motion with no button held.
    Move,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub point: GridPoint,
    pub mods: Mods,
}

/// Whether the program wants mouse events instead of the app handling them (selection, scroll).
pub(crate) fn reporting(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// The report for `event`, or `None` if the current mode doesn't ask for this kind of event.
pub(crate) fn encode(event: MouseEvent, mode: TermMode) -> Option<Vec<u8>> {
    use MouseEventKind::*;
    let wanted = match event.kind {
        Press(_) | Release(_) | WheelUp | WheelDown => mode.intersects(TermMode::MOUSE_MODE),
        Drag(_) => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
        Move => mode.contains(TermMode::MOUSE_MOTION),
    };
    if !wanted {
        return None;
    }

    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let mut code: u32 = match event.kind {
        Press(b) | Release(b) => button(b),
        Drag(b) => button(b) + 32,
        Move => 3 + 32,
        WheelUp => 64,
        WheelDown => 65,
    };
    let m = event.mods;
    code += 4 * m.shift as u32 + 8 * m.alt as u32 + 16 * m.ctrl as u32;
    let (col, row) = (event.point.col as u32 + 1, event.point.row as u32 + 1);

    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if matches!(event.kind, Release(_)) { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{col};{row}{end}").into_bytes());
    }

    // Legacy X10/normal encoding: releases don't say which button.
    if matches!(event.kind, Release(_)) {
        code = 3 + (code & !3);
    }
    let mut out = b"\x1b[M".to_vec();
    out.push(32 + code as u8);
    for v in [col, row] {
        if mode.contains(TermMode::UTF8_MOUSE) {
            let c = char::from_u32(32 + v)?;
            let mut buf = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            // Positions past 223 can't be expressed in this encoding.
            out.push(u8::try_from(32 + v).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: MouseEventKind, col: usize, row: usize) -> MouseEvent {
        MouseEvent { kind, point: GridPoint { row, col }, mods: Mods::NONE }
    }

    #[test]
    fn nothing_without_mouse_mode() {
        assert_eq!(encode(ev(MouseEventKind::Press(MouseButton::Left), 0, 0), TermMode::default()), None);
    }

    #[test]
    fn sgr_press_release_and_wheel() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        let press = encode(ev(MouseEventKind::Press(MouseButton::Left), 4, 9), mode).unwrap();
        assert_eq!(press, b"\x1b[<0;5;10M");
        let release = encode(ev(MouseEventKind::Release(MouseButton::Right), 0, 0), mode).unwrap();
        assert_eq!(release, b"\x1b[<2;1;1m");
        assert_eq!(encode(ev(MouseEventKind::WheelDown, 0, 0), mode).unwrap(), b"\x1b[<65;1;1M");
        // Drag isn't reported in click-only mode.
        assert_eq!(encode(ev(MouseEventKind::Drag(MouseButton::Left), 0, 0), mode), None);
    }

    #[test]
    fn legacy_encoding() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(encode(ev(MouseEventKind::Press(MouseButton::Left), 0, 0), mode).unwrap(), b"\x1b[M !!");
        assert_eq!(encode(ev(MouseEventKind::Release(MouseButton::Left), 0, 0), mode).unwrap(), b"\x1b[M#!!");
        assert_eq!(encode(ev(MouseEventKind::Press(MouseButton::Left), 300, 0), mode), None);
    }
}
