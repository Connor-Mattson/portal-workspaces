# 0012: Attention: terminals say when they finished or need you

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-term/src/{notify,tap,activity}.rs`, `crates/pw-app/src/attention.rs`,
  `crates/pw-app/src/app/attention.rs`, `crates/pw-app/src/notifier.rs`, `crates/pw-app/src/ui/status.rs`

## Context

With agents running in several workspaces, you couldn't tell from workspace A that the agent in B had finished
or was waiting on a permission prompt. The drawer had a dot for "printed something while hidden", but an
agent's spinner prints all the time, so the dot was always on and meant nothing.

Agents do announce these moments, but not to us:

- **Claude Code** sends notifications only to terminals it recognizes by `TERM_PROGRAM` (iTerm2, kitty,
  Ghostty, Apple Terminal). For ours it sends nothing unless `preferredNotifChannel` is set: `iterm2` sends OSC 9
  with a message, and `terminal_bell` sends BEL.
- **Codex** sends OSC 9 to terminals it knows and BEL to the others. It does this only while its terminal reports
  being unfocused (mode 1004), which our hidden panes already do.
- **`alacritty_terminal` drops OSC 9, 777 and 99** as unhandled, so we never saw them.

## Decision

**Three states per terminal**, ranked:

1. **Needs input**: BEL, or a notification whose text asks for something (permission, approval, input, a
   question, confirmation; see `attention::classify`).
2. **Finished**: any other notification, or the terminal went quiet after working.
3. **Working**: the terminal has been printing steadily.

A higher state replaces a lower one. A repeat of the same state keeps its place in line and takes the newer
message.

**When it's raised.** A terminal raises Finished or Needs input only while you aren't watching it, i.e. it isn't
the terminal with the keys in a focused app window. A visible but unfocused pane in a 4×2 grid counts as not
watched: its title bar shows a chip.

**Seen means it had the keys.** A terminal's state clears when it has the keys while the app is focused:
`App::acknowledge` runs from `refocus`, and so from every focus change, including the window regaining focus.
Merely being on screen doesn't clear it; in a big grid you may not have looked at that pane. Going back to work
clears a Finished that came from going quiet, but never one the program announced.

**Explicit signals come from the PTY bytes.** `TappedPty` wraps alacritty's `Pty`, and the event loop reads
through it. `OscScanner` picks out OSC 9 (but not ConEmu's numbered `9;n;…` subcommands such as progress), OSC
777 `notify` and OSC 99 (kitty, including chunks). It's a small state machine that skips to the next ESC, keeps
at most 1 KB per sequence, and works across reads. The trait's `register` is `unsafe fn`, so its one-line
delegation carries a local `#[allow(unsafe_code)]`.

**The quiet heuristic makes it work without setup.** A terminal is busy once its output has had no gap of 4 s
or more for 3 s. It is idle again after 4 s without output.

- Becoming busy is checked as output is read, with no timer.
- Becoming idle needs a deadline. One `pw-term-activity` thread, shared by all sessions, blocks with no timeout
  while nothing is busy. Otherwise it sleeps until the earliest moment a busy terminal could have gone quiet.
- It holds `Weak` references, so closed sessions simply drop out. The UI wakes only on a real transition.
- This is the one timer added to the hard rules' list, and it exists only while something is busy.
- Known false positives: dev servers and log tails that print in bursts. Their Finished is green and calm, and
  goes away as soon as you look.

**Notifications go outside the app only when it isn't focused.** Otherwise the drawer is enough.

- `window::request_user_attention(Informational)` flags the terminal's window: an X11/Wayland urgency hint or a
  dock bounce, which the system clears on focus.
- Unless turned off (`UiPrefs.notifications`, the drawer's bell button, added in schema 6), `notify-rust` also
  posts a desktop notification saying what the agent said. Only a rise in level notifies, so repeated bells
  don't.
- **One notification, replaced in place**, not one per terminal: on Linux a click-waiting thread blocks until
  its notification closes, so a pile of notifications would mean a pile of threads. At most one thread waits.
  Clicking the notification jumps to the terminal it was about.
- On macOS a click just brings the app forward, and ⌘I goes to the terminal from there.
- Scripted runs (`devtools`) never notify.

**Going there.** Ctrl+Shift+I / ⌘I (`Action::NextAttention`) goes to the oldest Needs input, otherwise the oldest
Finished. It switches workspace and mode, restores a maximized neighbor, raises a detached window or opens the
editor's terminal panel, and gives the terminal the keys. Clicking a workspace in the drawer does the same for
that workspace, unless the terminal that already has its keys wants something too.

## Consequences

- Nothing is persisted but the pref. Attention belongs to running sessions, as sessions do (ADR 0003).
- An agent that stops printing while it is still working (e.g. a long silent tool call, or a TUI that pauses its
  spinner when unfocused) shows as Finished early. Its next output clears that.
- Other agents get the explicit path for free if they speak OSC 9/777/99 or ring the bell.
