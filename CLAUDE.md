# Working on Portal Workspaces (instructions for coding agents)

Portal Workspaces is a native Rust desktop app (Linux + macOS): a drawer of project workspaces, each holding 1–8
terminal panes in a split layout. Any terminal can be detached into its own window (see ADR 0006). Each workspace can
also switch to Editor mode: a light code editor with a file tree, tabs, split groups and one terminal (see ADR 0007).
Terminals keep running across workspace switches; layouts, detached windows, cwds and editor tabs persist across
restarts (sessions and unsaved edits don't). The drawer also shows the 5-hour and weekly limits of the AI accounts you
track (Claude, Codex, Antigravity), and a system profile (CPU, GPU, RAM and network, attributed per workspace; see
ADR 0010). When a terminal you aren't watching finishes or needs input, its workspace says so and you can jump
to it (see ADR 0012). Read `README.md` for the user-facing picture.

## Hard rules
- **Performance is a feature.**
  - No polling for redraws, and no timers beyond the save debounce (which exists only while state is dirty),
    the 10 s cwd tick, the usage monitor's 2-minute poll, the system monitor's 5 s sample (each on its own
    thread; they only wake the UI when a reading arrives) and the activity watcher (`pw-term`'s one thread,
    asleep with no timeout unless a terminal is busy; it wakes the UI only when one goes quiet). The system monitor is paused, with no timeout at
    all, while its rings aren't on screen. File watching is event-driven (ADR 0009); the caret blink,
    scrollbar fade and highlighting catching up on a long file (ADR 0008) are redraw requests, not timers.
  - A terminal's `canvas::Cache` is cleared only when that terminal changed. Other canvases keep their drawing
    in `ui::Drawn`, redrawn only when what they draw from changes.
  - Views do no I/O: they're rebuilt after every message. Look things up once (like `sessions::home_dir`).
  - Hidden workspaces never draw, and neither does a workspace's hidden mode (Agents or Editor). Detached
    terminals' windows always draw: they're on screen.
- **The dependency direction is fixed:** `pw-app → pw-term`, `pw-app → pw-usage → pw-model`, `pw-app → pw-model`,
  `pw-app → pw-code`, `pw-app → pw-system`.
  - `pw-model` is pure data and logic (no iced, no PTY, no threads).
  - `pw-code` is editing knowledge (languages, highlighting, smart editing, file listing): no GUI types, no
    threads.
  - `pw-term` has no GUI types, and no alacritty types leave it.
  - `pw-usage` has no GUI types either; the app gets plain `Update`s through a callback.
  - `pw-system` has no GUI types and doesn't know about panes or workspaces: it totals process trees under
    the root pids it's given, and the app maps them to workspaces.
- **Checking usage never spends tokens.** Network calls go only to `pw_usage::http::Endpoint` variants and
  subprocesses only to `pw_usage::vendor::VendorCmd` variants, and every one of them must be a read-only,
  no-model call. Never add `claude -p`, `agy -p`, `codex exec` or a free-form URL or command. Credentials are
  read, never written; an expired sign-in is renewed by the vendor's own CLI (see ADR 0005).
- **`MAX_PANES` (8) is enforced in `pw-model`.** Detached panes count toward it; the editor's terminal doesn't.
  UI code asks `WorkspaceView::can_split` and `fits` (and `EditorView::can_split` for `MAX_GROUPS`); it never
  re-implements the rule.
- **Sessions are owned by `Sessions` (keyed by `PaneId`), never by views.** Only closing a pane or deleting a
  workspace drops a session.
- **State file changes bump `SCHEMA_VERSION` and add a `store::migrate` arm.** Never make old files unreadable
  without a migration.
- **Shortcuts use the app modifier.** Ctrl+Shift on Linux, Cmd on macOS. Plain Ctrl-keys belong to the shell: they
  are bound only while a code editor or the file tree has focus (`keymap::Context`), never in a terminal. The one
  exception is Ctrl+C on Linux (`keymap::copies_selection`): it copies a selection that's on screen and drops it;
  with nothing selected it reaches the shell (ADR 0011). Don't add another.
- **Text changes go through `EditorView::change`**, which mirrors them into the file's other open views. Never edit
  a tab's `Content` directly.
- **Disk edits are never silently overwritten.** A clean buffer reloads when its file changes on disk; a dirty one
  shows the conflict banner and waits for the user. Saving is explicit (no autosave), and quitting or closing with
  unsaved edits asks first.

## Commands
```bash
cargo test --workspace                             # unit + PTY integration tests (spawns /bin/sh), ~1 s
cargo clippy --workspace --all-targets             # must be warning-free
cargo fmt --all                                    # rustfmt.toml: 120 columns
cargo run -p pw-app                                # dev build (dependencies are optimized, see Cargo.toml)
PORTAL_WORKSPACES_STATE=/tmp/pw.json cargo run -p pw-app   # never touch the real state file while testing
PORTAL_WORKSPACES_DEV_SCRIPT="wait 2; shot /tmp/pw/a.png; quit" ...   # scripted run with screenshots (see below)
cargo build --release -p pw-app && ./scripts/install.sh    # install for the current user
```

## Map
- `crates/pw-model/`: the domain.
  - `layout.rs`: generic `SplitTree<Id>` (`LayoutNode` = `SplitTree<PaneId>`), `Preset`s, `MAX_PANES`.
  - `workspace.rs`: `Workspace` (grid layout + `detached` panes, `mode`, `editor`), `PaneSpec`, `repaired()`.
  - `editor.rs`: `Mode`, `EditorSession` (groups, tabs, cursors, expanded folders, terminal), `MAX_GROUPS`.
  - `state.rs`: `PersistedState`, `UiPrefs`.
  - `store.rs`: atomic JSON load/save, recovery, migration.
  - `usage.rs`: `UsageProfile`, `Provider`, `PollEnv::parse` (alias/env/folder → data dir + env).
- `crates/pw-term/`: the terminal engine.
  - `session.rs`: `Session` wraps an `alacritty_terminal` `Term` plus its PTY IO thread (spawn, write, resize,
    selection, scroll).
  - `events.rs`: `TermEvent`, and wakeup coalescing (one `Wakeup` until the next `snapshot`).
  - `tap.rs`: the PTY wrapper the event loop reads through. `notify.rs`: OSC 9/777/99 notifications.
    `activity.rs`: busy/idle (`ActivityConfig`) and the activity watcher thread.
  - `snapshot.rs`: the visible grid, copied into styled runs for drawing.
  - `input.rs`: key → bytes (xterm + kitty disambiguate).
  - `mouse.rs`: mouse reporting.
  - `cwd.rs`: shell cwd (`/proc` on Linux, libproc on macOS).
  - `color.rs`: `Palette`.
  - `tests/session.rs`: real-PTY integration tests.
- `crates/pw-code/`: editing knowledge, tested without a window.
  - `language.rs`: the `Language` table (comment tokens, quotes, indent, icon tint, syntax), detection.
  - `syntax.rs`: line-incremental highlighting (syntect grammars from two-face) into `Class`es.
  - `editing.rs`: bracket matching, auto-pairs, smart Enter, comment toggle, indent detection.
  - `text.rs`: `Pos`, `TextEdit::between`/`shift` (mirroring edits). `find.rs`, `fuzzy.rs` (nucleo).
  - `fs.rs`: one-folder listings with gitignore dimming, and the quick-open index.
- `crates/pw-usage/`: the usage monitor.
  - `monitor.rs`: the `Monitor` daemon thread (2-minute schedule, backoff).
  - `report.rs`: `Report`, `Window`, `Span`, `Failure`.
  - `http.rs`: the closed `Endpoint` allowlist. `vendor.rs`: the closed `VendorCmd` list.
  - `claude.rs`, `codex.rs`, `antigravity.rs`: one poll per provider; their parsers are tested against
    `fixtures/`. Codex falls back to its session logs without a ChatGPT sign-in.
  - `discover.rs`: finds installed accounts. `secrets.rs`: read-only keyring access.
  - `examples/poll.rs`: `cargo run -p pw-usage --example poll -- claude "CLAUDE_CONFIG_DIR=~/.claude-work"`.
- `crates/pw-system/`: the system profile (see ADR 0010).
  - `sample.rs`: `Sample` and its parts (plain data). `monitor.rs`: the `Monitor` thread (5 s while active,
    paused otherwise), driven by a `Source`.
  - `sampler.rs`: the real `Source` (sysinfo, refreshing only what's shown). `gpu.rs`: NVML, amdgpu sysfs,
    `ioreg`. `processes.rs`: tree attribution, grouping by name, readable names.
  - `examples/sample.rs`: `cargo run -p pw-system --release --example sample -- <pid…>` prints readings and
    their cost.
- `crates/pw-app/`: the Iced binary `portal-workspaces`.
  - `app.rs`: `App`, `Message`, `update`, `subscription` (Elm architecture). Keys go to `App::key_pane`.
  - `app/windows.rs`: the app is an iced daemon; the main window and detached terminals' windows (detach, dock).
  - `app/editor.rs`: `EditorMsg`, modes, tabs, groups, find, quick open, saving, the terminal panel, disk changes.
    `app/explorer.rs`: `ExplorerMsg`, the file tree's open/new/rename/trash and its keys.
  - `editor/`: Editor mode's state, no widgets. `mod.rs`: `EditorView` and `change`. `document.rs` (disk sync,
    conflicts), `buffer.rs` (editing commands), `history.rs` (undo, 32 MB per file), `group.rs` (tabs),
    `explorer.rs` (listings, virtualization), `quick_open.rs`, `find.rs`, `watch.rs` (`FsHub`).
  - `workspace.rs`: `WorkspaceView` (live `pane_grid::State` ⇄ `LayoutNode`, plus detached panes, mode, editor).
  - `split.rs`: `SplitTree` ⇄ `pane_grid` for agent panes and editor groups. `background.rs`: work off the UI
    thread.
  - `devtools.rs`: the `PORTAL_WORKSPACES_DEV_SCRIPT` driver.
  - `sessions.rs`: `Sessions` registry. `inbox.rs`: background thread → subscription channel.
  - `attention.rs`: Finished / Needs input, their rank and a workspace's `Status`. `app/attention.rs`: raising,
    acknowledging, jumping (Ctrl+Shift+I). `notifier.rs`: the one desktop notification.
  - `usage.rs`: `Usage` registry (profiles, latest readings, the monitor).
  - `system.rs`: `SystemProfile` (latest sample, 5 minutes of history, pausing, tracked shells).
  - `persist.rs`: state path + background `Saver`.
  - `keymap.rs`: shortcuts + iced key → terminal key.
  - `theme.rs`: design tokens (ported from Science Portal's dark theme) + widget styles.
  - `fonts.rs`: bundled Inter + JetBrains Mono NL; `CellMetrics`.
  - `icons.rs`: inline SVG icons.
  - `ui/`: `sidebar.rs`, `workspace_view.rs` (header + pane grid), `terminal.rs` (canvas renderer + mouse),
    `modal.rs` (workspace editor, confirmations, shortcuts), `usage.rs` (drawer meters + rail), `system.rs`
    (rings, hover cards, sparklines), `status.rs` (attention dots and chips), `profile_editor.rs` (the "Track an
    account" sheet), `pane_window.rs` (a detached terminal's window), `term_menu.rs` (a terminal's right-click
    menu).
  - `ui/editor/`: Editor mode's views. `code_editor.rs` (the forked widget, ADR 0008), `highlight.rs`, `group.rs`
    (tab strip, banners, find card, welcome), `explorer.rs`, `terminal_panel.rs`, `quick_open.rs`.
- `docs/decisions/`: ADRs. Add one for any decision a future reader would otherwise undo.
- `assets/`: fonts (OFL) and app icon · `packaging/linux/` .desktop · `scripts/install.sh`.

## Verifying UI changes
There's no UI test harness, but the app can drive itself and screenshot its own window. Use a scratch state file
(copy the real one there to test a migration) and a scratch project for anything that writes files:

```bash
PORTAL_WORKSPACES_STATE=/tmp/pw/state.json \
PORTAL_WORKSPACES_DEV_SCRIPT="wait 2; mode editor; click src; open src/main.rs; shot /tmp/pw/1.png; split; find fn; shot /tmp/pw/2.png; quit; discard" \
  cargo run -p pw-app
```

- Steps are listed in `devtools.rs`. A `shot` waits for the next drawn frame, so it shows the result of the steps
  before it. The window stays on top while a script plays. An unknown step quits at once (see the log).
- `;` separates steps, so `term` text can't contain one. `type` and `term` turn `\n` into Enter.
- Scripted keys and text skip iced's widgets, so keyboard focus between the code editor, inputs and terminals
  still needs a human.
- Remove `usage_profiles` from a copied state file, or every run polls the real accounts.
- Prefer script steps to real input. Real clicks and keys (XTest) go to whatever window is under the pointer or
  has focus, so before sending any, wait until the window is mapped and placed (it reports 0,0 until then) and
  check it has focus. Otherwise they land in the user's own windows, e.g. a Ctrl+C or Escape into a running agent.

Check at least:
- the first run (the "New workspace" sheet),
- a 4×2 layout,
- the collapsed drawer (Ctrl+Shift+B), including the usage rail,
- the usage section with a tracked account (and the "Track an account" sheet),
- the system rings and each hover card, in the drawer and the rail, with something busy in a workspace
  terminal (e.g. `timeout 30 sh -c 'yes >/dev/null'`) so it shows under "By workspace". Hover needs a real
  pointer: move it yourself, or warp it during a scripted run (`XWarpPointer` via ctypes works without
  xdotool).
- a TUI (`top`, `less`) and a CLI agent,
- copying from a terminal: drag then Ctrl+C (copies, doesn't interrupt), Ctrl+C with nothing selected
  (interrupts), the right-click menu (`menu <x> <y>` in a script), and Shift+drag in a program that uses the
  mouse,
- detaching a terminal (title bar button, Ctrl+Shift+O), typing in it, switching workspaces while it's out, and
  docking it back (its button, Ctrl+Shift+O, closing its window),
- Editor mode: the tree (ignored folders dimmed), preview and pinned tabs, a split with the same file in both
  groups, find, quick open, the terminal panel, and switching back to Agents (terminals redrawn),
- a file changed on disk while open: clean (reloads) and dirty (banner), and quitting with unsaved edits (sheet),
- typing in the code editor, then Ctrl+C in the editor terminal (reaches the shell) and Ctrl+S in the editor
  (saves),
- attention: a hidden workspace's terminal running `sleep 2 && printf '\033]9\073Needs your input\007'` (in a
  script, `\073` stands in for `;`) shows amber in the drawer, the rail and its pane's title bar; a few seconds of
  output then silence shows green; Ctrl+Shift+I (`attend`) jumps there and clears it; with the window
  unfocused, a desktop notification appears and clicking it jumps,
- that restarting restores layout, detached windows, cwds, the mode, tabs, cursors, groups and the expanded tree.
