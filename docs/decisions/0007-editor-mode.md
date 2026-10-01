# 0007: Editor mode

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-model/src/editor.rs`, `crates/pw-app/src/editor/`, `crates/pw-app/src/app/editor.rs`,
  `crates/pw-app/src/app/explorer.rs`, `crates/pw-app/src/ui/editor/`, `crates/pw-app/src/keymap.rs`

## Context

Agents change code in a workspace's terminals, and people want to read and touch up that code without leaving the
app: a light editor with a file tree, tabs, split groups and a terminal, like VS Code. It should sit next to the
agents, not replace them.

## Decision

- **Each workspace has a mode, Agents or Editor**, switched from the header or with App+M (⌘⇧M on macOS, where ⌘M
  minimizes). The mode is persisted (`Workspace::mode`, schema 4). The hidden mode never draws: in Editor mode the
  agent grid's caches go stale, and switching back clears them, as switching workspaces does. Agents keep running.
  While you edit, the Agents segment shows their activity and bell dot.
- **The editor starts lazily**, the first time a workspace is shown in Editor mode (`EditorView::start`): tabs
  reopen, the tree lists the root and its expanded folders, and watches start. Its layout, tabs, cursors, expanded
  folders and panel sizes persist in `Workspace::editor` (`EditorSession`). A workspace whose editor never started
  saves its stored session untouched.
- **Documents and views are separate.** `EditorView::docs` holds one `Document` per open file: the disk text, the
  undo history, and the dirty, conflict and deleted flags. Each tab holds only its view (`Content`: text layout,
  cursor, scroll). The same file can be open in two groups, each with its own cursor.
- **`EditorView::change` is the only way text changes.** It runs an edit on the active tab's `Buffer { doc, view }`,
  then mirrors the edit into the document's other views. It diffs the old and new text (`TextEdit::between`),
  applies only that span, and moves the mirrors' cursors with `pw_code::text::shift`. Typing, undo, reload,
  replace and line commands all go through it, so views can't drift apart. The diff costs O(n), and only when a
  file is open twice.
- **Saving is explicit.** There's no autosave. Closing a dirty tab or group, deleting a workspace or quitting opens
  one sheet (`Modal::Unsaved`): *Save*, *Don't save*, *Cancel*. Unsaved edits survive switching workspaces and
  modes, but not quitting. A save replaces the file atomically, keeping its permissions, and keeps CRLF files CRLF
  (ADR 0013).
- **Disk edits are never silently overwritten.** When a watched file changes on disk, a clean document reloads in
  place, as one undoable step, keeping its cursor and scroll. A dirty one gets a banner: *Keep mine* (save over
  the disk) or *Use disk version*. Saving while the banner is up counts as *Keep mine*. A deleted file gets a
  banner too, and saving recreates it.
- **The editor's terminal is its own shell, outside `MAX_PANES`.** `EditorSession::terminal` has its own `PaneId`,
  and its session lives in `Sessions` like any other. `WorkspaceView::owns` includes it, so events find their
  workspace. The 8-pane cap is about how many agents fit in a grid. One shell under the editor doesn't change that,
  and counting it would make a full grid lose its editor terminal. App+J cycles it: show and focus, then focus,
  then hide.
- **Keys depend on what has focus** (`keymap::Context`: `Terminal`, `Editor`, `Explorer`). App-modifier shortcuts
  work everywhere and go to the active mode's surface: split, close and move focus act on editor groups in Editor
  mode. Editing shortcuts (Ctrl on Linux, ⌘ on macOS: save, undo, find, comment, close tab, quick open…) apply
  **only while a code editor or the file tree has focus**. In a terminal, plain Ctrl still belongs to the shell,
  so Ctrl+C interrupts and Ctrl+W deletes a word. Moving focus away from a code editor by keyboard runs iced's
  `focusable::unfocus()`, so the widget stops taking keys.

## Consequences

- `App::key_pane`, `is_visible` and `context` are mode-aware. A terminal shortcut in Editor mode means the editor
  terminal.
- The editor terminal's shell is spawned when the panel is first shown. A hidden panel keeps its shell running.
- Unsaved edits live only in memory. Quitting asks first, and a crash loses them.
- The model caps editor groups at `MAX_GROUPS` (4). The UI asks `EditorView::can_split` instead of counting.
