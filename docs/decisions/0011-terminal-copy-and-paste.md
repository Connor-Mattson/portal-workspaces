# 0011: Copying from terminals: Ctrl+C copies a visible selection, and right-click opens a menu

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `keymap::copies_selection`, `App::on_key`, `Session::take_visible_selection`,
  `crates/pw-app/src/ui/term_menu.rs`, `input::encode_paste`

## Context

Copying out of an agent's terminal is one of the most common things people do here. Before this decision it
only worked through Ctrl+Shift+C, which few people find. Ctrl+C, the key everyone presses, interrupted the agent,
and right-click did nothing. The hard rule "plain Ctrl-keys belong to the shell" made this deliberate. That rule is
right for every other key, but here it cost too much for one key.

## Decision

- **Ctrl+C copies when a selection is on screen (Linux only).** It copies the text, clears the selection, and
  sends nothing to the program. With nothing selected, Ctrl+C reaches the program exactly as before. This is
  Windows Terminal's and VS Code's behavior, and kitty's `copy_or_interrupt`.
  - **Copying clears the selection**, so the next Ctrl+C interrupts. Pressing Ctrl+C twice always stops the
    agent.
  - **Only a selection you can see counts.** A selection that scrolled off screen would make Ctrl+C silently do
    nothing visible. Typing and pasting also clear the selection, as in other terminals.
  - **macOS is unchanged.** There ⌘C copies, and Ctrl+C belongs to the shell, as in Terminal and iTerm.
  - Ctrl+Shift+C still copies. It keeps the selection.
- **Ctrl+V is not bound.** Agents use it themselves: Claude Code pastes images with Ctrl+V. Paste with
  Ctrl+Shift+V, the menu, or middle-click (the primary selection).
- **Right-click opens a menu with Copy, Paste and Select all.**
  - Copy is disabled with no selection. Each item shows its shortcut.
  - The menu belongs to the window the terminal is in, including a detached terminal's window. It opens at the
    pointer, flipped to stay inside the terminal.
  - It sits on a full-window layer. A click, right-click or scroll anywhere else closes it, and that click does
    nothing else. Keys, layout changes and focus changes close it too.
  - Escape only closes the menu. It isn't sent on, because an agent would read Escape as "stop".
  - The window's view is always a `stack` with the menu layer last (empty when closed). Opening the menu then
    doesn't rebuild the widget tree, which would drop focus and drags.
- **Programs that use the mouse get the right-click**, as they get other clicks. Shift+right-click opens the menu
  anyway, as Shift+drag selects anyway.
- **Select all** (Ctrl+Shift+A, ⌘A, or the menu) selects the history and the screen, down to the last line with
  text. Like a mouse selection, it sets the primary selection.
- **Bracketed pastes drop ESC and ^C**, as Alacritty does. Removing only `ESC[201~` is not enough:
  `ESC[20ESC[201~1~` leaves a new end marker behind. A paste that ends the bracket runs the rest as typed keys.

## Consequences

- The plain-Ctrl rule now has exactly one exception, written down in CLAUDE.md. Any other plain Ctrl binding in a
  terminal still needs a decision of its own.
- A selection left on screen turns the next Ctrl+C into a copy. Because the selection is visible and copying
  clears it, the second Ctrl+C always interrupts.
- When a program redraws (erases) the lines under a selection, alacritty drops the selection. Text in a busy
  agent's live area can be hard to keep selected. History above it is stable.
