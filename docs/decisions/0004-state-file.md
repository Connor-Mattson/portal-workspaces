# 0004: One JSON state file, saved atomically in the background

- **Status:** accepted
- **Date:** 2026-09-30
- **Code:** `crates/pw-model/src/store.rs`, `crates/pw-app/src/persist.rs`

## Context

The app remembers workspaces, layouts (with split ratios), each pane's last cwd and a few UI preferences. It
does not remember running processes. Losing this file, or a crash corrupting it, would be a bad morning.

## Decision

- **Format:** one human-readable `state.json` with a `schema_version`.
- **Writes:** atomic (write a temp file, fsync, rename), on a dedicated thread. Only the newest pending state is
  written. On quit the app waits for the final write.
- **When:** changes are debounced by 1 s. Live cwds are polled every 10 s (`/proc/<pid>/cwd` on Linux, libproc
  on macOS) and saved when they change.
- **Recovery:** an unreadable or newer-version file is renamed to `state.json.bak-<unix time>` and the app
  starts empty. `repaired()` fixes a readable but inconsistent file (orphan panes, bad ratios, more than 8
  panes, dangling focus).

## Consequences

- If the app is killed, at most about 1 s of layout changes and 10 s of cwd changes are lost.
- Any change to the file's shape bumps `SCHEMA_VERSION` and adds a `store::migrate` arm.
