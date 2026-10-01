# 0009: The file tree and file watching

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-code/src/fs.rs`, `crates/pw-app/src/editor/explorer.rs`, `crates/pw-app/src/editor/watch.rs`,
  `crates/pw-app/src/ui/editor/explorer.rs`, `crates/pw-app/src/editor/quick_open.rs`

## Context

Workspaces are real projects, with `target/`, `node_modules/` and `.venv/` holding hundreds of thousands of files.
Agents, builds and `git` change files all the time, and the editor must notice without polling ("Performance is a
feature").

## Decision

- **The tree lists one folder at a time.** A folder is read when it's expanded (`fs::list_dir`). Folders come
  first, sorted case-insensitively. `.git` and OS litter (`.DS_Store`) are hidden. Other dotfiles show.
- **Gitignored entries are dimmed, not hidden.** People still open files in `target/` or a generated folder. The
  `ignore` crate decides what's ignored. It can't tell from rules alone that a folder's children are ignored, so
  the caller passes `parent_ignored` down. Quick open leaves ignored files out. It indexes the project in the
  background each time it opens, up to 50,000 files, and ranks matches with `nucleo-matcher`.
- **Rows are virtualized.** Every row is 24 px high, so the visible slice (plus 600 px of overscan) follows from the
  scroll offset alone. Expanding `node_modules` costs the rows on screen, not the folder's size. Before the first
  scroll event, the viewport is assumed to be 1,600 px.
- **Watches are targeted and non-recursive.** One `notify` watcher per workspace watches the root, every expanded
  folder, and the folders of open files. `FsHub::watch` reconciles that set after every tree or tab change. A
  recursive watch would walk `node_modules` and `target`, and on Linux can run out of inotify watches. Changes deep
  in a collapsed folder go unseen, which is fine: nothing on screen shows them.
- **Events are coalesced.** Raw events go to one thread, which waits for 150 ms of quiet and then delivers one
  batch of paths per workspace through an `Inbox`. An agent rewriting ten files, or a `git checkout`, is one update.
  The batch relists the affected folders and syncs the affected documents (ADR 0007).
- **Deleting moves to the Trash** (`trash` crate), after a confirmation sheet. Renames and new files are inline
  rows in the tree, and names are checked with `is_safe_relative` (no `..`, no absolute paths).

## Consequences

- No timers: the watcher is event-driven, and an idle editor uses no CPU.
- Files changed in a folder that isn't watched show up when the folder is expanded or refreshed (the refresh
  button relists everything shown).
- A tree row's height can't vary (no wrapped names, no inline details), because virtualization depends on it.
