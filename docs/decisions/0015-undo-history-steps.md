# 0015: Undo steps store what they changed

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-app/src/editor/history.rs`

## Context

Every undo step stored the whole text from before it. Whole texts can't drift out of sync with the editor's
text the way a log of the widget's operations can, which is why it was built that way. But it made big files
expensive: with a 32 MB budget per document, a file at the 8 MB limit kept only 3 steps, and held 24 MB to
do it.

## Decision

- **Only the step being typed holds the whole text from before it.** When the next step starts (or on undo),
  it is closed: the text before it and the text now are compared, and the step keeps only the span between
  their common prefix and suffix, both sides of it (`before` and `after`), plus where it is and the text's
  length. So a closed step costs the bytes it changed, and a big file keeps as many steps as a small one.
- **Steps still come from whole texts, not from operations.** The span is found by comparing two real texts,
  the same as before, so there's no operation log to drift. Grouping (a word, a pause, a paste) is unchanged.
- **Applying a step checks the text first.** Undo needs the text's length to match and `after` to be where the
  step left it (redo, the same with `before`). If not, something changed the text without the history seeing
  it, and the history is dropped (with a warning) rather than applied to the wrong text. Every edit goes through
  `Buffer` (and so the history), so this shouldn't happen.
- The 32 MB budget now counts the open step's whole text plus every closed step's two spans.
- `pw_code::text::common_prefix`/`common_suffix` compare 64-byte chunks with `memcmp` before the last bytes
  one by one, since closing a step scans the whole text.

## Consequences

- Measured on an 8 MB file, 200 one-character steps (`history_cost` in `history.rs`, opt-level 2):

  |                      | before | after  |
  |----------------------|--------|--------|
  | held                 | 24 MB  | 8 MB   |
  | steps you can undo   | 3      | 200    |
  | time per new step    | 0.9 ms | 1.4 ms |
  | time per undo        | 1.3 ms | 1.5 ms |

  A new step costs a little more: the comparison is one more pass over the text. Steps start at most once per
  burst of typing.
- A whole-file change (a reload from disk, replacing everything) still costs both whole texts in its step.
- Undo needs the text to be exactly what the step left. A change that bypasses the history now loses the
  history, where before undo would have jumped back over it.
