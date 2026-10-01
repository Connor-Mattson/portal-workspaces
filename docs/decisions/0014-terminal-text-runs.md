# 0014: Terminal text runs

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-term/src/snapshot.rs` (`capture`, `fits_a_cell`), `crates/pw-app/src/ui/terminal.rs` (`paint`)

## Context

A terminal redraw turns the screen into text primitives, one per run of same-styled cells. Runs used to hold
ASCII only, broken at every blank, and every other character got a run of its own, shaped with
`Shaping::Advanced` so a fallback font could supply it. The rule exists because a fallback glyph can be wider than
a cell and would push the rest of its run off the grid.

Agent TUIs (Claude Code, Codex) draw borders, rules, tables and meters with box-drawing and block characters. On a
typical 120×42 agent screen (`crates/pw-term/fixtures/agent-screen.ansi`) that made 660 text primitives a frame,
530 of them single glyphs.

## Decision

- **Characters that fit a cell merge into runs** (`snapshot::fits_a_cell`): printable ASCII, Latin-1 (except the
  soft hyphen, which some shapers hide), box drawing (U+2500–257F) and block elements (U+2580–259F). The bundled
  JetBrains Mono NL has every one of them in all four styles at exactly one cell (600/1000 em), so they can never
  push text off the grid. A test in `ui/terminal.rs` reads the font files and checks this; widening the list
  means the test must still pass.
- **Runs reach across blanks.** A blank draws nothing, and the font's space is one cell wide, so `a  b` in one
  style is one run with its spaces kept. A run takes a blank only if its style isn't underlined or struck out,
  since the decoration would then cover the blank too.
- **Runs of such characters are drawn with `Shaping::Basic`**: no shaping, no font fallback. Anything else
  (symbols like `✻` and `⎿`, wide characters, characters with combining marks) still gets its own run with
  `Shaping::Advanced`.
- `paint` takes the snapshot by value and moves each run's text into its primitive instead of copying it.

## Consequences

- The fixture screen draws 62 text primitives instead of 660. Rendered headless at 1× (a full redraw, as after
  the pane's cache is cleared), the terminal's share of a frame went from 17.6 ms to about 1.9 ms with
  tiny-skia (iced's CPU renderer). With wgpu it was about 0.3 ms either way, within the noise of the readback.
- A character outside the list that the bundled font does have (some arrows, geometric shapes) still gets its own
  run. Add it to `fits_a_cell` if it shows up often; the font test says whether it may.
- Snapshot buffers aren't reused between captures. Measure before adding that.
