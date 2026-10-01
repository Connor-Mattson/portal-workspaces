# 0013: Saving files atomically

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-app/src/editor/document.rs` (`write_file`)

## Context

Saving used `std::fs::write`, which truncates the file and then writes it. A crash, a kill or a power cut in
between left the user's source file empty or cut short. That is worse than losing the unsaved edits, which ADR
0007 already accepts: it loses the saved file too.

## Decision

- **A save writes a temporary file next to the original, syncs it, and renames it over the original**, then
  syncs the folder so the rename itself is durable (the same pattern as `pw_model::store::save`). The temporary
  file is a hidden sibling (`.name.<pid>-<n>.pw-save`) created with `create_new`, so two saves never share one,
  and it is removed if anything fails. After a crash you have the old file or the new one, never a mix.
- **The file keeps what it is.** The temporary file gets the original's owner, then its permissions (in that
  order: changing the owner can clear setuid bits). A file we may not write fails to save, as it did before,
  even when its folder would let us replace it.
- **Symlinks are written through.** Saving `link.txt → real.txt` replaces `real.txt` and leaves the link alone.
- **Some files are written in place instead, not atomically.** Renaming would change what they are:
  - a file with more than one hard link (a rename would split this name from the others),
  - a file whose owner or group we can't give the temporary file (it isn't ours, or its group isn't one of ours),
  - a dangling symlink (writing through it creates the target, as before).
  These are rare in projects, and keeping them working beats atomicity. Vim's `backupcopy=auto` makes the same
  call.
- **Our own save still looks unchanged to the watcher.** The rename is a change event for the file, and the
  temporary file is one for its folder. Both land in the same 150 ms batch; the document reads its file back,
  finds the text it just wrote, and does nothing (ADR 0009). The tree relists the folder, by then without the
  temporary file.

## Consequences

- A save syncs to disk, so it costs more than it did: on an NVMe ext4 disk, about 1.3 ms for a small file instead
  of 0.07 ms, and 19 ms for an 8 MB one instead of 14 ms. Saving is explicit and rare, so it stays on the UI
  thread.
- Extended attributes, ACLs and macOS file flags aren't copied to the new file. A file that needs them can be
  hard-linked, or saved by another tool.
- A file replaced by rename gets a new inode. Programs holding the old one open keep reading the old text, as
  they do after any editor's atomic save.
