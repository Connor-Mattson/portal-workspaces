# 0003: Sessions belong to panes, not to what's on screen

- **Status:** accepted
- **Date:** 2026-09-30
- **Code:** `crates/pw-app/src/sessions.rs`, `App::activate`, `App::on_term_event`

## Context

Switching workspaces must not interrupt anything. An agent in a hidden workspace keeps working and you come back
to it later. At the same time, 8 panes × N workspaces must not cost redraws.

## Decision

- `Sessions` owns every running terminal, keyed by `PaneId`. Only closing a pane or deleting a workspace drops
  one. Switching workspaces changes `App::active` and nothing else.
- Output from hidden panes is still parsed (on the session's IO thread), so their screens are current when
  shown. The UI only marks them `unseen_output`, and the drawer shows a dot. Wakeups are coalesced per session
  until the next snapshot, so a hidden, busy pane sends one message, not thousands.
- **Lazy spawn:** a workspace's shells start the first time it's shown, not at launch.

## Consequences

- Launch cost doesn't grow with the number of workspaces.
- Showing a workspace clears its panes' caches, because they missed redraws while hidden.
- Memory grows with the number of terminals ever opened, bounded by 10k lines of scrollback each.
