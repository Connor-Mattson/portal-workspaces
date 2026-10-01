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
- **Editor mode.** Flip any workspace from its agents to a light code editor (file tree, tabs, split groups and a
  terminal) to read and touch up what the agents wrote, then flip back. The agents keep running.
- **Nothing stops when you switch, and you hear when it's your turn.** Terminals in other workspaces keep
  running. When an agent finishes or needs your input, its workspace says so in the drawer, and Ctrl+Shift+I
  takes you to it. A desktop notification tells you if you're in another app (see
  [Knowing when an agent needs you](#knowing-when-an-agent-needs-you)).
- **Restores your setup.** On relaunch every workspace comes back with the same layout, split sizes, detached
  windows, each terminal's last working directory and the editor's open files, with fresh shells. Running
  processes and unsaved edits are not restored.

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
| Go to the terminal that needs you | Ctrl+Shift+I | ⌘I |
| Agents ⇄ Editor | Ctrl+Shift+M | ⌘⇧M |
| Toggle drawer | Ctrl+Shift+B | ⌘B |
| Split right / down | Ctrl+Shift+D / Ctrl+Shift+E | ⌘D / ⌘⇧D |
| Close terminal | Ctrl+Shift+W | ⌘W |
| Move focus | Ctrl+Shift+Arrows | ⌘Arrows |
| Maximize terminal | Ctrl+Shift+Enter | ⌘Enter |
| Detach terminal into its own window / dock it back | Ctrl+Shift+O | ⌘O |
| Copy / paste | Ctrl+Shift+C / V | ⌘C / V |
| Copy the selection (with nothing selected: interrupt, as always) | Ctrl+C | — |
| Select all | Ctrl+Shift+A | ⌘A |
| Font size | Ctrl+Shift+= / - / 0 | ⌘= / - / 0 |
| Scroll history | Shift+PgUp / PgDn | Shift+PgUp / PgDn |
| All shortcuts | Ctrl+Shift+/ | ⌘/ |

On Linux, shortcuts use Ctrl+Shift so that plain Ctrl-keys (Ctrl+D, Ctrl+W, …) reach the shell. Plain Ctrl
shortcuts exist only in Editor mode, and only while the code or the file tree has the keys (see below). The one
exception is Ctrl+C: while text is selected on screen it copies it and clears the selection, so the next Ctrl+C
interrupts as usual. With nothing selected it always reaches the program, so it can't keep you from stopping an
agent. Mouse:
- Drag selects text (and copies it to the primary selection on Linux). Double-click selects a word, triple-click
  a line.
- Right-click opens a menu: Copy, Paste, Select all.
- Middle-click pastes the primary selection.
- The wheel scrolls history.
- Programs that use the mouse (vim, htop) get mouse events. Hold Shift to select text or open the menu anyway.

Pastes into programs that ask for bracketed paste (shells, Claude Code, Codex) arrive as one paste. Escape
characters and Ctrl+C are removed from them, so pasted text can't end the paste early and run as typed keys.

New terminals open in the folder of the terminal they were split from. Shells get
`TERM=xterm-256color`, `COLORTERM=truecolor` and `TERM_PROGRAM=PortalWorkspaces`. The kitty keyboard
protocol is supported, so Shift+Enter works in Claude Code.

## Editor mode

The **Agents / Editor** switch at the top of a workspace (Ctrl+Shift+M) flips it to a code editor on the same
folder. Each workspace remembers its mode.

- **File tree:** folders load as you open them. Files your `.gitignore` excludes (`target/`, `node_modules/`) are
  dimmed, not hidden. Hover a row to rename or delete it; deleting moves it to the Trash. Arrows, Enter, F2 and
  Delete work when the tree has the keys.
- **Tabs:** one click opens a file as a *preview* (dimmed name), which the next click replaces. Double-click or
  start typing to keep it. Tabs with the same name show the folders that tell them apart.
- **Split groups:** up to 4, side by side or stacked. The same file can be open in two groups, each with its own
  cursor, and edits show in both.
- **Editing:** syntax highlighting for most languages (files over 50,000 lines show as plain text), bracket
  matching, auto-closing pairs, comment toggling, moving and duplicating lines, and the file's own indentation
  (tabs, or 2 or 4 spaces). Find and replace, and quick open (fuzzy file search, honouring `.gitignore`).
- **Terminal:** one shell of its own under the editor (Ctrl+Shift+J). It doesn't count toward the 8 terminals.
- **Saving is explicit.** Nothing is saved until you press Ctrl+S. Closing a tab, deleting a workspace or quitting
  with unsaved edits asks first. A pencil in the drawer marks workspaces with unsaved edits.
- **Agents editing your files:** a file you haven't touched reloads in place when it changes on disk. If you have
  unsaved edits, a banner lets you keep yours or take the disk version; nothing is overwritten without asking.

| | Linux | macOS |
|---|---|---|
| Quick open | Ctrl+P (or Ctrl+Shift+P) | ⌘P |
| Save | Ctrl+S | ⌘S |
| Toggle terminal / file tree | Ctrl+Shift+J / T | ⌘J / T |
| Split editor | Ctrl+\ | ⌘\ |
| Close tab | Ctrl+W | ⌘W |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Find / replace | Ctrl+F / Ctrl+H | ⌘F / ⌘⌥F |
| Next / previous match | F3 / Shift+F3 | F3 / Shift+F3 |
| Undo / redo | Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z) | ⌘Z / ⌘⇧Z |
| Comment lines | Ctrl+/ | ⌘/ |
| Move / duplicate lines | Alt+↑↓ / Alt+Shift+↑↓ | ⌥↑↓ / ⌥⇧↑↓ |

The Ctrl shortcuts in this table work while the code or the file tree has the keys. In the editor's terminal,
Ctrl-keys go to the shell as usual.

## Knowing when an agent needs you

Each terminal you aren't looking at can be in one of three states:

- **Working** (a dim dot): it has been printing steadily for a few seconds, e.g. an agent's spinner or a build.
- **Finished** (green): it was working and went quiet, or the agent said its turn is complete.
- **Needs input** (amber, haloed): the agent asked for permission, approval or an answer, or the program rang
  the bell.

They show up in several places:

- **The drawer:** the dot on the workspace's row. "Needs input" and "Finished" also replace the folder path,
  followed by what the agent said; hover to read all of it.
- **The collapsed drawer:** the dot on the workspace's letters.
- **The terminal's title bar:** a chip, so you can spot the right pane in a full grid.
- **The Agents switch:** a dot while you're in Editor mode.
- **The window title:** a count, e.g. "(2)".

To get there:

- **Ctrl+Shift+I** goes to the terminal that has needed you longest, or else the one that finished first. It
  switches workspace and mode, and un-maximizes or raises a window if needed.
- **Clicking the workspace** goes straight to that terminal.
- A terminal's state clears as soon as it has the keys.

When the app isn't focused, the first change also posts a desktop notification and flags the app in the taskbar
or dock. On Linux, clicking the notification takes you to the terminal. The bell at the bottom of the drawer
turns notifications off.

Agents tell terminals about this in different ways:

- **Claude Code** only sends notifications to terminals it recognizes. Without them, "Finished" comes from the
  output going quiet, which also covers permission prompts. For the exact message, set Claude Code's
  notification channel to iTerm2: `/config` → Notifications → `iterm2`. That stores
  `"preferredNotifChannel": "iterm2"` in its global config.
- **Codex** rings the bell when it needs you. For its message, set `tui.notification_method = "osc9"` in
  `~/.codex/config.toml`.
- **Any program** can use OSC 9, OSC 777 or OSC 99 notifications, or the bell, e.g.
  `printf '\033]9;Deploy finished\007'`.

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

## System profile

Below the usage limits, four rings show **CPU**, **GPU**, **RAM** and **network**, sampled every 5 seconds. A ring
fills amber from 70% and red from 90%. Hover one for its card:

- **The last 5 minutes** as a sparkline, with the peak.
- **By workspace:** how much CPU, memory or GPU memory each workspace's terminals are using, counting every
  process they started (agents, builds, dev servers).
- **Top programs:** the busiest programs on the machine. Processes of the same name are counted together
  (`chrome ×73`). A program running in one of your workspaces says which one.
- **Details:** load average, swap, GPU temperature and power, and each network link's speed and traffic.

GPU numbers come from NVIDIA's driver (NVML) or AMD's on Linux, and from the GPU driver's own statistics on
macOS. Intel GPUs on Linux don't report their load without root, so their ring stays empty. Network traffic
counts only real interfaces (Ethernet, Wi-Fi), not loopback, containers or VPN tunnels. The ring shows how full
the link is when its speed is known, otherwise traffic relative to the recent peak.

Sampling stops while the section is closed. In the collapsed drawer the rings shrink to a 2×2 tile.

## Where state lives

`~/.config/portalworkspaces/state.json` on Linux, `~/Library/Application Support/Portal Workspaces/state.json`
on macOS. Set `PORTAL_WORKSPACES_STATE=/path/to/file.json` to use another file (handy for trying things out).
If the file can't be read, it is moved aside as `state.json.bak-<time>` and the app starts empty.

Logs (and the backtrace of any crash) go to `logs/portal-workspaces.log` in the same folder, as well as to stderr.
The file is capped at 2 MB, and the one before it is kept as `portal-workspaces.log.1`. `RUST_LOG` sets the level.

## Performance

- **Idle:** no timers except a 10-second cwd check, the 2-minute usage check and the 5-second system sample
  (each on its own thread). While a terminal is working, one thread waits until it could have gone quiet. It
  wakes the app only when that happens. An idle app uses about 0% CPU. A system sample reads about 600 processes in about
  9 ms (about 0.2% of one core), and none are taken while the section is closed.
- **Output:** terminals redraw only when their output changes, and at most once per frame. Hidden workspaces
  never draw, and neither does the mode a workspace isn't showing.
- **Files:** the editor watches only the folders it shows (no recursive watches), and changes arrive as events,
  not by polling.
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
