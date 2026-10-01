# 0017: A log file, and a record of panics

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-app/src/logging.rs`, `crates/pw-app/src/persist.rs` (`Flush`), `Cargo.toml` (`strip`)

## Context

Logs went only to stderr, which is lost when the app starts from a desktop entry or an app bundle. A panic on
the UI thread exits the app and takes every terminal with it, and nothing recorded why. The default log filter
also named `pw_app`, but the binary's crate is `portal_workspaces`, so the app's own info logs never showed.

## Decision

- **Logs go to stderr and to `logs/portal-workspaces.log` next to the state file.** Next to the state file, so
  `PORTAL_WORKSPACES_STATE` moves them too and a test run never writes to the real logs. When the file passes
  2 MB it becomes `portal-workspaces.log.1` (replacing the older one), so logs never take more than about 4 MB.
  A small writer does this. `tracing-appender` only rotates by time, so it doesn't bound the size, and it isn't
  worth another dependency.
- **The default filter is `warn,<crate>=info`**, with the crate name from `CARGO_CRATE_NAME`, so it stays
  right if the binary is renamed.
- **A panic hook** runs after the default one (which prints to stderr). It writes the message, its location
  and a backtrace to the log file. If the UI thread panicked, the app is going down, so it also waits up to
  2 s for the saver to write the state already handed to it.
  - It can't reach the app's state, which is mid-update and may be inconsistent anyway. At most one save
    debounce (1 s) of layout changes is lost.
  - It must not deadlock if the panic struck while holding a lock. It only tries the log file's lock for
    200 ms (the panic may have happened mid-write on the same thread), and talks to the saver over a channel
    with a timeout.
- **Release builds keep their symbol table** (`strip = "debuginfo"` instead of `strip = true`), so a backtrace
  names functions. Line numbers would need debug info, which is much bigger.

## Consequences

- The release binary grows from 28.7 MB to 34.5 MB.
- A forced panic (`panic` in a dev script) leaves its report and backtrace in the log file, in dev and release
  builds.
- Logging is synchronous: each event is one `write` to the file. At the default level there are only a few
  events per session.
