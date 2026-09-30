# Working on Portal Workspaces (instructions for coding agents)

Portal Workspaces is a native Rust desktop app (Linux + macOS): a drawer of project workspaces, each holding 1–8
terminal panes in a split layout. Terminals keep running across workspace switches; layouts and cwds persist
across restarts (sessions don't). Read `README.md` for the user-facing picture.

## Hard rules
- **Performance is a feature.**
  - No polling for redraws, and no timers beyond the save debounce (which exists only while state is dirty)
    and the 10 s cwd tick.
  - A terminal's `canvas::Cache` is cleared only when that terminal changed.
  - Hidden workspaces never draw.
- **The dependency direction is fixed:** `pw-app → pw-term`, `pw-app → pw-model`.
  - `pw-model` is pure data and logic (no iced, no PTY, no threads).
  - `pw-term` has no GUI types, and no alacritty types leave it.
- **`MAX_PANES` (8) is enforced in `pw-model`.** UI code asks `WorkspaceView::can_split`; it never re-implements
  the rule.
- **Sessions are owned by `Sessions` (keyed by `PaneId`), never by views.** Only closing a pane or deleting a
  workspace drops a session.
- **State file changes bump `SCHEMA_VERSION` and add a `store::migrate` arm.** Never make old files unreadable
  without a migration.
- **Shortcuts use the app modifier.** Ctrl+Shift on Linux, Cmd on macOS. Never bind a plain Ctrl-key (it
  belongs to the shell).

## Commands
```bash
cargo test --workspace                             # unit + PTY integration tests (spawns /bin/sh), ~1 s
cargo clippy --workspace --all-targets             # must be warning-free
cargo fmt --all                                    # rustfmt.toml: 120 columns
cargo run -p pw-app                                # dev build (dependencies are optimized, see Cargo.toml)
PORTAL_WORKSPACES_STATE=/tmp/pw.json cargo run -p pw-app   # never touch the real state file while testing
cargo build --release -p pw-app && ./scripts/install.sh    # install for the current user
```

## Map
- `crates/pw-model/`: the domain.
  - `layout.rs`: `LayoutNode` split tree, `Preset`s, `MAX_PANES`.
  - `workspace.rs`: `Workspace`, `PaneSpec`, `repaired()`.
  - `state.rs`: `PersistedState`, `UiPrefs`.
  - `store.rs`: atomic JSON load/save, recovery, migration.
- `crates/pw-term/`: the terminal engine.
  - `session.rs`: `Session` wraps an `alacritty_terminal` `Term` plus its PTY IO thread (spawn, write, resize,
    selection, scroll).
  - `events.rs`: `TermEvent`, and wakeup coalescing (one `Wakeup` until the next `snapshot`).
  - `snapshot.rs`: the visible grid, copied into styled runs for drawing.
  - `input.rs`: key → bytes (xterm + kitty disambiguate).
  - `mouse.rs`: mouse reporting.
  - `cwd.rs`: shell cwd (`/proc` on Linux, libproc on macOS).
  - `color.rs`: `Palette`.
  - `tests/session.rs`: real-PTY integration tests.
- `crates/pw-app/`: the Iced binary `portal-workspaces`.
  - `app.rs`: `App`, `Message`, `update`, `subscription` (Elm architecture).
  - `workspace.rs`: `WorkspaceView` (live `pane_grid::State` ⇄ `LayoutNode`).
  - `sessions.rs`: `Sessions` registry + the single event inbox subscription.
  - `persist.rs`: state path + background `Saver`.
  - `keymap.rs`: shortcuts + iced key → terminal key.
  - `theme.rs`: design tokens (ported from Science Portal's dark theme) + widget styles.
  - `fonts.rs`: bundled Inter + JetBrains Mono NL; `CellMetrics`.
  - `icons.rs`: inline SVG icons.
  - `ui/`: `sidebar.rs`, `workspace_view.rs` (header + pane grid), `terminal.rs` (canvas renderer + mouse),
    `modal.rs` (workspace editor, confirmations, shortcuts).
- `docs/decisions/`: ADRs. Add one for any decision a future reader would otherwise undo.
- `assets/`: fonts (OFL) and app icon · `packaging/linux/` .desktop · `scripts/install.sh`.

## Verifying UI changes
There's no UI test harness. Run the app against a scratch state file
(`PORTAL_WORKSPACES_STATE=/tmp/pw.json`) and look at it. Check at least:
- the first run (the "New workspace" sheet),
- a 4×2 layout,
- the collapsed drawer (Ctrl+Shift+B),
- a TUI (`top`, `less`) and a CLI agent,
- that restarting restores layout and cwds.
