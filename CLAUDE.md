# Working on Portal Workspaces (instructions for coding agents)

Portal Workspaces is a native Rust desktop app (Linux + macOS): a drawer of project workspaces, each holding 1–8
terminal panes in a split layout. Any terminal can be detached into its own window (see ADR 0006). Terminals keep
running across workspace switches; layouts, detached windows and cwds persist across restarts (sessions don't). The drawer also shows the 5-hour and weekly limits of the AI accounts you track
(Claude, Codex, Antigravity). Read `README.md` for the user-facing picture.

## Hard rules
- **Performance is a feature.**
  - No polling for redraws, and no timers beyond the save debounce (which exists only while state is dirty),
    the 10 s cwd tick and the usage monitor's 2-minute poll (on its own thread; it only wakes the UI when a
    reading arrives).
  - A terminal's `canvas::Cache` is cleared only when that terminal changed.
  - Hidden workspaces never draw. Detached terminals' windows always draw: they're on screen.
- **The dependency direction is fixed:** `pw-app → pw-term`, `pw-app → pw-usage → pw-model`, `pw-app → pw-model`.
  - `pw-model` is pure data and logic (no iced, no PTY, no threads).
  - `pw-term` has no GUI types, and no alacritty types leave it.
  - `pw-usage` has no GUI types either; the app gets plain `Update`s through a callback.
- **Checking usage never spends tokens.** Network calls go only to `pw_usage::http::Endpoint` variants and
  subprocesses only to `pw_usage::vendor::VendorCmd` variants, and every one of them must be a read-only,
  no-model call. Never add `claude -p`, `agy -p`, `codex exec` or a free-form URL or command. Credentials are
  read, never written; an expired sign-in is renewed by the vendor's own CLI (see ADR 0005).
- **`MAX_PANES` (8) is enforced in `pw-model`.** Detached panes count toward it. UI code asks
  `WorkspaceView::can_split` and `fits`; it never re-implements the rule.
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
  - `workspace.rs`: `Workspace` (grid layout + `detached` panes), `PaneSpec`, `repaired()`.
  - `state.rs`: `PersistedState`, `UiPrefs`.
  - `store.rs`: atomic JSON load/save, recovery, migration.
  - `usage.rs`: `UsageProfile`, `Provider`, `PollEnv::parse` (alias/env/folder → data dir + env).
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
- `crates/pw-usage/`: the usage monitor.
  - `monitor.rs`: the `Monitor` daemon thread (2-minute schedule, backoff).
  - `report.rs`: `Report`, `Window`, `Span`, `Failure`.
  - `http.rs`: the closed `Endpoint` allowlist. `vendor.rs`: the closed `VendorCmd` list.
  - `claude.rs`, `codex.rs`, `antigravity.rs`: one poll per provider; their parsers are tested against
    `fixtures/`. Codex falls back to its session logs without a ChatGPT sign-in.
  - `discover.rs`: finds installed accounts. `secrets.rs`: read-only keyring access.
  - `examples/poll.rs`: `cargo run -p pw-usage --example poll -- claude "CLAUDE_CONFIG_DIR=~/.claude-work"`.
- `crates/pw-app/`: the Iced binary `portal-workspaces`.
  - `app.rs`: `App`, `Message`, `update`, `subscription` (Elm architecture). Keys go to `App::key_pane`.
  - `app/windows.rs`: the app is an iced daemon; the main window and detached terminals' windows (detach, dock).
  - `workspace.rs`: `WorkspaceView` (live `pane_grid::State` ⇄ `LayoutNode`, plus detached panes).
  - `sessions.rs`: `Sessions` registry. `inbox.rs`: background thread → subscription channel.
  - `usage.rs`: `Usage` registry (profiles, latest readings, the monitor).
  - `persist.rs`: state path + background `Saver`.
  - `keymap.rs`: shortcuts + iced key → terminal key.
  - `theme.rs`: design tokens (ported from Science Portal's dark theme) + widget styles.
  - `fonts.rs`: bundled Inter + JetBrains Mono NL; `CellMetrics`.
  - `icons.rs`: inline SVG icons.
  - `ui/`: `sidebar.rs`, `workspace_view.rs` (header + pane grid), `terminal.rs` (canvas renderer + mouse),
    `modal.rs` (workspace editor, confirmations, shortcuts), `usage.rs` (drawer meters + rail),
    `profile_editor.rs` (the "Track an account" sheet), `pane_window.rs` (a detached terminal's window).
- `docs/decisions/`: ADRs. Add one for any decision a future reader would otherwise undo.
- `assets/`: fonts (OFL) and app icon · `packaging/linux/` .desktop · `scripts/install.sh`.

## Verifying UI changes
There's no UI test harness. Run the app against a scratch state file
(`PORTAL_WORKSPACES_STATE=/tmp/pw.json`) and look at it. Check at least:
- the first run (the "New workspace" sheet),
- a 4×2 layout,
- the collapsed drawer (Ctrl+Shift+B), including the usage rail,
- the usage section with a tracked account (and the "Track an account" sheet),
- a TUI (`top`, `less`) and a CLI agent,
- detaching a terminal (title bar button, Ctrl+Shift+O), typing in it, switching workspaces while it's out, and
  docking it back (its button, Ctrl+Shift+O, closing its window),
- that restarting restores layout, detached windows and cwds.
