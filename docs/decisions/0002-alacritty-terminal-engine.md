# 0002: alacritty_terminal is the terminal engine

- **Status:** accepted
- **Date:** 2026-09-30
- **Code:** `crates/pw-term`

## Context

We need a correct VT emulator: truecolor, alternate screen, scrollback, selection, bracketed paste, mouse modes,
synchronized updates (DEC 2026), and the kitty keyboard protocol (Claude Code uses it for Shift+Enter). We also
need a PTY on Linux and macOS.

## Decision

Use `alacritty_terminal`, the same crate Zed uses. It provides:
- the parser and grid,
- a unix PTY with the right session and controlling-tty setup (and the `login` wrapper on macOS),
- an IO thread (`event_loop`) that parses output off the UI thread.

`pw-term` wraps it behind plain types (`Session`, `TermEvent`, `Snapshot`, `KeyInput`), so nothing else depends
on it.

## Consequences

- **We write:** key encoding (`input.rs`), mouse reports (`mouse.rs`) and replies to color/size queries
  (`events.rs`). These are tested in-crate.
- **Kitty protocol:** only the "disambiguate" level is implemented. Higher flags degrade to it.
- **OSC 52:** programs reading the clipboard (paste) is deliberately unsupported. Copying to it is supported.
