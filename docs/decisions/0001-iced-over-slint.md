# 0001: Iced, not Slint, for the GUI

- **Status:** accepted
- **Date:** 2026-09-30
- **Code:** `crates/pw-app`

## Context

The brief allowed Slint or Iced. Most of the pixels are terminal grids: up to 8 panes of about 200×60 styled
cells each, redrawn at frame rate while agents stream output. Everything else (the drawer, header and sheets) is
ordinary UI.

## Decision

Use Iced 0.14 with the wgpu renderer (tiny-skia is the fallback):
- **Terminal drawing.** `canvas::Program` with a per-pane `canvas::Cache` gives direct control over drawing, and
  caching at pane granularity. Slint's declarative element tree is a poor fit for thousands of cells.
- **Layouts.** The built-in `pane_grid` already does splits, drag-resize, drag-to-rearrange and maximize.
- **Code.** The Elm architecture keeps every state change in `App::update`, and all UI is plain Rust (no second
  language or build step).

## Consequences

- Iced's API changes between minor versions. Upgrades are deliberate and pinned in `Cargo.toml`.
- There's no live-preview designer; UI changes are checked by running the app (see AGENTS.md).
- If canvas text ever becomes the bottleneck, a glyph-atlas `shader` widget can replace `ui/terminal.rs::paint`
  without touching `pw-term` (it only consumes `Snapshot`).
