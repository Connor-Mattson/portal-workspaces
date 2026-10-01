# 0006: Terminals can be detached into their own windows

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-app/src/app/windows.rs`, `crates/pw-app/src/ui/pane_window.rs`, `WorkspaceView::detach`/`dock`,
  `crates/pw-app/src/main.rs`

## Context

People want an agent's terminal on another screen, or kept in view while they work in another workspace. iced's
`application` has exactly one window.

## Decision

- **The app is an `iced::daemon`.** It opens its own windows: the main one at boot, and one per detached terminal.
  `view` and `title` take a `window::Id`. A daemon doesn't exit when its windows close, so quitting always ends
  with `iced::exit()`.
- **A detached pane leaves the grid but stays in its workspace.** `Workspace::detached` lists it, its `PaneSpec`
  stays in `panes`, and it counts toward `MAX_PANES`, so docking it back always fits. Presets replace only the grid;
  a preset that would go over the cap with the detached panes is disabled.
- **Same session, no copying.** The window draws the pane's `PaneRuntime` with the same canvas. Sessions are still
  keyed by `PaneId` and owned by `Sessions`.
- **Windows outlive workspace switches.** A detached terminal is on screen, so it draws whichever workspace is
  active. "Hidden workspaces never draw" still holds for their grids.
- **Keys follow the focused window.** `App::key_window` is the app window that last had focus. Its terminal (the
  detached one, or the main window's focused pane) gets typed keys and terminal shortcuts, and is the only one
  drawn as focused. Grid shortcuts (split, move focus, maximize) do nothing in a detached window. A sheet in the
  main window doesn't block detached terminals.
- **Closing a detached window docks the terminal**, next to the grid's focused pane. Only "Close terminal" ends
  its shell.
- **Detachment persists** (schema 3). Detached windows reopen when their workspace's shells start: at launch for
  the active workspace, on first switch for the others. They all open at the last size a detached window was
  resized to (`UiPrefs::terminal_window`).

## Consequences

- Window events (focus, resize, close) carry the window id.
- Docking puts the pane to the right of the focused pane, not back in its old spot. Drag its title bar to move it.
- Window managers decide focus for new windows. On GNOME, a restored detached window may take focus at launch.
