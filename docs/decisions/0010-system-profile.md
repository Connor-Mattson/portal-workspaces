# 0010: A light system profile, sampled only while it's shown, and attributed per workspace

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-system/`, `crates/pw-app/src/system.rs`, `crates/pw-app/src/ui/system.rs`

## Context

Agents, builds and dev servers started from the app's terminals compete for the same machine. When things slow
down, the first questions are "what's busy?" and "is it something I started, and in which workspace?". A full
system monitor is a different app. This needs to fit in the drawer, cost close to nothing, and answer those two
questions on hover.

## Decision

- **A `pw-system` crate** with no GUI types (like `pw-usage`):
  - A `Monitor` thread samples every **5 s** (`INTERVAL`). The first reading comes 1 s after it starts, because
    CPU and network rates need two reads.
  - It **starts paused**, and while paused it blocks on its channel with no timeout. The app keeps it running
    only while the rings are on screen: the drawer's *System* section is open, or the drawer is collapsed and
    the rail shows its 2×2 tile.
  - Resuming primes the counters first, so the first rates cover the last second, not the whole pause.
- **sysinfo, refreshing only what's shown:**
  - global CPU, memory and swap;
  - each process's CPU time, memory and parent;
  - each process's command line, once, when the process is first seen;
  - interface counters.
  - **`ProcessRefreshKind::nothing()` still lists every thread as a process on Linux.** We add `without_tasks()`.
    Without it, threads are counted on top of their process, and a sample took 46 ms instead of 9 ms.
- **Cost, measured** (i9-12900KF, about 600 processes, release build): the NVML probe takes about 60 ms, once,
  on the monitor thread. The first sample takes about 26 ms (it reads every command line). After that a sample
  takes about 9 ms, which is about 0.2% of one core.
- **GPU without root:**
  - NVIDIA: NVML, loaded at run time (`libnvidia-ml.so.1`). It also gives per-process GPU memory.
  - AMD: amdgpu's sysfs files.
  - macOS: the `PerformanceStatistics` in the I/O Registry, read with a fixed `ioreg` command.
  - Intel on Linux needs perf counters (root), so it isn't supported. The card says why.
- **Network counts physical interfaces only.**
  - Linux: ones with `/sys/class/net/<if>/device`. macOS: `en*`.
  - Loopback, bridges, veths and VPN tunnels carry traffic that already crosses a real interface.
  - The ring shows link utilization (the busier direction ÷ link speed) when the driver reports a speed.
    Otherwise it shows traffic relative to the recent peak, in the accent, never as a warning.
- **Attribution per workspace:**
  - The app hands the monitor its terminals' shell pids (`set_roots`).
  - The monitor totals each one's descendants (`Tree`), and tags listed programs with their root.
  - The app maps roots to workspaces at draw time. `pw-system` knows nothing about panes.
  - New terminals are counted from the next reading, because the pid list goes over after each sample.
- **Programs, not processes:**
  - Processes with the same name under the same root are summed into one entry (`chrome ×73`).
  - Runtimes show their script: `node …/claude-code/cli.js` shows as `claude-code`, `python -m http.server`
    as `http.server`.
  - A reading carries only the top 5 by CPU, the top 5 by memory, and the GPU users, so it stays small.
- **Readings aren't persisted.** The 5-minute sparkline history lives in memory and is cleared on restart.
  Pausing drops the latest reading, so reopening never shows old numbers as current. History kept across a pause
  is drawn with a gap.

## Consequences

- Process CPU is shown as a share of the whole machine, so the lists add up toward the ring. A process using one
  core of 24 reads 4.2%, not 100%.
- Summed RSS counts shared pages more than once, so `chrome ×73` overstates a little. Every task manager that
  groups processes has the same trade-off.
- Per-process network use needs root (eBPF or packet capture), so the network card lists interfaces, not
  programs.
- iced builds tooltip content on every `view()`, shown or not. The cards format only what they display, and
  the workspace map is built once per view, not once per card.
- In the rail, the rings' initials are drawn inside the ring's canvas. A text layer stacked over an 18 px
  canvas didn't render.
