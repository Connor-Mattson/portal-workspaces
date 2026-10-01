# Portal Workspaces

**Every project, its terminals, one keystroke away.**

Portal Workspaces is a native desktop app for Linux and macOS for running many projects at once, each with
its own CLI agents (Claude Code, Codex, …). It's a companion to [Science Portal](../science-portal).

- **Workspaces** live in a drawer on the left. Each one is a project folder with its own layout of
  **1–8 terminals**.
- **Layouts** are free splits: split any terminal right or down, drag dividers to resize, and drag title bars
  to rearrange. One-click presets cover 1, 2, 3, 4, 6 and 8 terminals.
- **Detach any terminal** into its own window, e.g. to keep an agent on another screen. It stays open while you
  switch workspaces. Closing its window docks it back into the grid.
- **Nothing stops when you switch.** Terminals in other workspaces keep running. The drawer shows a dot when
  one of them printed something, and an amber dot when it rang the bell (e.g. an agent finished).
- **Restores your setup.** On relaunch every workspace comes back with the same layout, split sizes, detached
  windows and each terminal's last working directory, with fresh shells. Running processes are not restored.

Written in Rust with [Iced](https://iced.rs) (GPU rendering via wgpu). `alacritty_terminal` handles terminal
emulation. There's no web runtime and no garbage collector.

## Install

```bash
# Rust toolchain, once: https://rustup.rs
./scripts/install.sh
```

- **Linux:** installs `portal-workspaces` to `~/.local/bin`, plus an app-menu entry and icon.
- **macOS:** builds `Portal Workspaces.app` into `~/Applications` (installs `cargo-bundle` if needed).

Run from source with `cargo run --release -p pw-app`.

Ubuntu build dependencies (usually present on a desktop install):
`sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libfontconfig1-dev libdbus-1-dev`.

## Using it

The first launch asks for a project folder. After that:

| | Linux | macOS |
|---|---|---|
| New workspace | Ctrl+Shift+N | ⌘N |
| Edit workspace (rename, change folder, delete) | Ctrl+Shift+, | ⌘, |
| Switch to workspace 1–9 | Ctrl+Shift+1…9 | ⌘1…9 |
| Previous / next workspace | Ctrl+Shift+[ / ] | ⌘[ / ] |
| Toggle drawer | Ctrl+Shift+B | ⌘B |
| Split right / down | Ctrl+Shift+D / Ctrl+Shift+E | ⌘D / ⌘⇧D |
| Close terminal | Ctrl+Shift+W | ⌘W |
| Move focus | Ctrl+Shift+Arrows | ⌘Arrows |
| Maximize terminal | Ctrl+Shift+Enter | ⌘Enter |
| Detach terminal into its own window / dock it back | Ctrl+Shift+O | ⌘O |
| Copy / paste | Ctrl+Shift+C / V | ⌘C / V |
| Font size | Ctrl+Shift+= / - / 0 | ⌘= / - / 0 |
| Scroll history | Shift+PgUp / PgDn | Shift+PgUp / PgDn |
| All shortcuts | Ctrl+Shift+/ | ⌘/ |

On Linux, shortcuts use Ctrl+Shift so that plain Ctrl-keys (Ctrl+C, Ctrl+D, Ctrl+W, …) always reach the
shell. Mouse:
- Drag selects text (and copies it to the primary selection on Linux). Double-click selects a word, triple-click
  a line.
- Middle-click pastes the primary selection.
- The wheel scrolls history.
- Programs that use the mouse (vim, htop) get mouse events. Hold Shift to select text anyway.

New terminals open in the folder of the terminal they were split from. Shells get
`TERM=xterm-256color`, `COLORTERM=truecolor` and `TERM_PROGRAM=PortalWorkspaces`. The kitty keyboard
protocol is supported, so Shift+Enter works in Claude Code.

## Usage limits

The bottom of the drawer shows the **5-hour** and **weekly** limits of the AI accounts you track, refreshed
every 2 minutes. Click **+** next to *Usage* and pick an account. Claude config dirs (`~/.claude*`), Codex and
Antigravity are detected automatically. For another setup, paste the alias you start it with, e.g.
`alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`.

- Each bar fills amber from 70% and red from 90%. The tick marks how much of the window's time has passed, so
  fill past the tick means you're ahead of pace. Hover a card for exact reset times.
- **Checking never uses tokens.** Each provider is asked for its usage numbers with the account's own sign-in,
  so usage from your other machines counts too. No model is ever run. Codex signed in with an API key falls
  back to the limits the CLI recorded after its last turn here.
- Sign-ins are only read. When one has expired, the app asks the CLI itself to renew it without running a
  model (`claude doctor`, `agy models`, or Codex's app server). If that doesn't renew it, the card says
  **idle** and has a **Renew** button that tries again right away. If that doesn't work either, run the CLI once
  yourself.

## Where state lives

`~/.config/portal-workspaces/state.json` on Linux, `~/Library/Application Support/Portal Workspaces/state.json`
on macOS. Set `PORTAL_WORKSPACES_STATE=/path/to/file.json` to use another file (handy for trying things out).
If the file can't be read, it is moved aside as `state.json.bak-<time>` and the app starts empty.

## Performance

- **Idle:** no timers except a 10-second cwd check and the 2-minute usage check (on its own thread). An idle
  app uses about 0% CPU.
- **Output:** terminals redraw only when their output changes, and at most once per frame. Hidden workspaces
  never draw.
- **Measured** on Ubuntu 22.04 with an RTX 3090 (release build):
  - The window appears about 70 ms after launch.
  - With four live terminals (one running `top`), the app uses about 0.4% CPU.
  - Memory is about 50 MB of the app's own, plus about 150 MB of shared, file-backed GPU-driver mappings that
    show up in RSS.
- **Less memory:** `ICED_BACKEND=tiny-skia portal-workspaces` renders on the CPU instead (about 30 MB in
  total). It's fine for light use, but slower when output floods.

## Development

See [AGENTS.md](AGENTS.md) for the layout of the code and the commands, and
[docs/decisions](docs/decisions) for why things are the way they are.
