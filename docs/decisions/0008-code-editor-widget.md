# 0008: The code editor widget and syntax highlighting

- **Status:** accepted
- **Date:** 2026-10-01
- **Code:** `crates/pw-app/src/ui/editor/code_editor.rs`, `crates/pw-app/src/ui/editor/highlight.rs`,
  `crates/pw-code/src/syntax.rs`, `crates/pw-code/src/language.rs`, `crates/pw-code/src/editing.rs`

## Context

The editor needs a gutter with line numbers, a current-line band, bracket matching, find highlights and an
overlay scrollbar. All of them need the editor's scroll position and line layout, which iced 0.14's `text_editor`
keeps private. Portal Papers hit the same wall and forked the widget (Papers ADR 0007). Its fork works with
LaTeX only.

Highlighting has to cover whatever is in a project (Rust, TypeScript, Python, TOML, Markdown, shell…) and fit
iced's `Highlighter`, which is fed one line at a time and told the first line that changed.

## Decision

- **Port Papers' fork of `text_editor` (MIT) and make it language-neutral.** Editing still goes through iced's
  `Action`s, `Binding`s and cosmic-text editor, so typing, selection, IME and the clipboard behave like iced's
  widget. The bracket matcher comes from `pw_code::editing`, using the file's `Language` to skip strings and
  comments. Each group's widget has its own `widget::Id`, so focus can be moved by id. Compile-issue marks from
  Papers are gone.
- **The widget hands keys to the app.** Its `key_binding` resolves `keymap::Context::Editor` first, so shortcuts
  work while it has focus. Copy and paste stay the widget's own bindings. Everything else that edits (typing,
  Enter, Tab, Backspace) becomes a message and goes through `EditorView::change` (ADR 0007), which applies the
  language's smart editing: auto-closing pairs, Enter between brackets, comment toggling, and the file's detected
  indentation.
- **Highlight with syntect grammars from two-face**, not tree-sitter. two-face is bat's syntax set: syntect's
  defaults plus TypeScript/TSX, TOML, Dockerfile, Kotlin, Swift, Zig and others. Sublime grammars parse line by
  line with carried state, which is the shape of iced's `Highlighter`. Tree-sitter parses whole files and would
  need its own incremental bridge, plus a grammar crate per language.
  - Parser state is snapshotted every 64 lines. An edit re-parses from the snapshot before it, not from the top.
  - Lines over 4,096 bytes (minified bundles) aren't highlighted, because regex grammars get slow on them.
  - Scopes map to a small `Class` enum (keyword, string, comment, function, type…), which the app colours with
    its own palette (`theme::syntax`). No stock colour theme is used.
  - The `SyntaxSet` loads once, on a background thread, the first time any workspace enters Editor mode.
  - **Each tab keeps its own parse state** (`highlight::Parse`), shared with the group widget's highlighter
    while the tab is shown. Coming back to a tab resumes where its highlighting stopped; its view still holds
    the colours of the lines already done. A fresh highlighter per switch re-parsed from line 0 every time
    (about 260 ms to come back to the end of a 10,000-line file). Don't make the highlighter per widget again.
  - Because the state outlives the widget, `Content` tracks the topmost line edited since the last layout
    and tells the highlighter. Iced keeps only the latest edit's line, which loses an earlier, higher one
    when several edits land before a layout (a mirrored view that isn't shown collects them all).
  - **A frame spends at most 8 ms parsing** (`HIGHLIGHT_BUDGET`). Parsing runs at about 40 lines/ms and
    iced colours everything from the highlighter's position down to the last visible line in one go, so
    jumping to the end of a long file used to block for seconds. Past the deadline the highlighter leaves
    lines plain and waits at the first of them; while visible lines are still waiting, the widget asks for
    another frame (a redraw request, like the caret blink, and only until they're done). The end of a
    50,000-line file is coloured over about 150 frames, none longer than ~16 ms. The cost: when a re-parse
    takes longer than the budget (an edit far above what another view shows), that view's lines show plain
    for those frames instead of the UI freezing.
  - **Files over 50,000 lines show as plain text** (`syntax::MAX_LINES`), like VS Code's large-file mode.
    Colouring their end takes several seconds of frames, and the snapshots kept grow with the file.
  - Classifying a scope stack is cached per scope (the innermost scope with a class wins, and each scope
    resolves on its own), which makes parsing about 20% faster.
- **Drawing an unchanged editor costs the lines in view, not the file.** In Editor mode a streaming terminal panel
  redraws the window every frame, so per-frame work on a long file adds up.
  - The bracket matcher reads lines through an accessor, only the `MAX_LINES` (1,000) around the caret, and
    the pair is kept in the `Content` until the text, the caret or the language changes. It used to collect
    every line into a `Vec<&str>` each frame: about 280 µs and 1.6 MB a frame on a 100,000-line file.
  - The scrollbar's row counts (where each line starts in wrapped rows) are kept in the `Content` too, and
    counted again only when the text, the width, the font size or the wrapping changes, or when a line in
    view was laid out to a different height than it was counted at. Checking that costs the lines in view.
    Counting walked every line on each frame and each scrollbar drag event: about 230 µs a frame on that file;
    kept, it's under 1 µs. `Content` counts its edits, which is how both caches know the text changed.
- **`pw-code` holds the editing knowledge.** It's a leaf crate with no GUI types and no threads: languages,
  highlighting, smart editing, text diffs, find, fuzzy matching and file listing. All of it is unit-tested
  without a window.

## Consequences

- Lines soft-wrap (`Wrapping::WordOrGlyph`). The fork's horizontal scrolling hasn't been made to work yet, so
  there's no "no wrap" option.
- Language support is a table row in `language.rs` (comment tokens, quotes, indent, icon tint, syntax name). A
  language two-face lacks shows as plain text.
- Opening a long file is still slow, and not because of highlighting: iced's `Editor::with_text` lays out
  every line before the editor has a size (about 20 µs a line, so ~1 s for 50,000 lines), and the first layout
  re-applies the font to every line. Fixing that needs a change in iced, not in the fork.
- Fixes to iced's `text_editor` don't reach the fork by themselves. Diff the fork against iced when upgrading.
