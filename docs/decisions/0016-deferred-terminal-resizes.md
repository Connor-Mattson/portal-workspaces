# 0016: Terminals resize once a divider is let go

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-term/src/session.rs` (`defer_resize`, `settle`), `crates/pw-app/src/app.rs`
  (`dragging_divider`, `settle_terminals`), `crates/pw-app/src/ui/terminal.rs`

## Context

A terminal notices its size when it's drawn, and resized at once: alacritty reflowed the grid (up to 10,000
lines of history) and the program got a `SIGWINCH`. Dragging a divider crosses a column every few pixels, so
each terminal next to it was resized dozens of times. Full-screen agents redraw their whole screen on every
`SIGWINCH`, so a drag set off a storm of output, and shells left a trail of prompts reflowed at every width.

## Decision

- **While a divider is dragged, terminals defer their resizes.** A `pane_grid` resize event (the agents grid,
  or the editor's regions, which hold its terminal) marks a drag. A size noticed meanwhile is kept by the
  `Session` (`defer_resize`), not applied. The terminal keeps drawing its old grid, clipped or with a margin,
  until the drag ends.
- **The drag ends when the left button is released.** That is an event, so no timer is needed. The listener
  only exists during a drag. Then every terminal takes its deferred size (`settle`) and redraws. Losing focus
  also ends a drag, in case the release went elsewhere.
- **Everything else still resizes at once:** window resizes, maximizing a pane, detaching, docking, switching
  layouts. These are single steps, except dragging a window's border. A window manager gives us no "drag ended"
  event for that, and waiting for a quiet period would need a timer, which the rules in `CLAUDE.md` don't allow
  for this. Revisit it if window-border drags turn out to matter as much.

## Consequences

- Dragging a divider in 15 steps over a shell that counts `SIGWINCH`: **15 before, 1 after**, ending at the
  same size (`stty size`). The neighbouring shell no longer fills with prompts redrawn at each width.
- During a drag a terminal shows its old grid, so its text doesn't follow the divider until it's let go.
- A resize that arrives after a deferred one replaces it, so a terminal never ends at a stale size.
