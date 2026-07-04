Design brief: VisiData's interaction model ("vim for spreadsheets", visidata.org) distilled for Brightfield's keyboard-first authoring UX. Hugh named it as the ergonomic inspiration for the planned in-app spec editor. Companion to `2026-07-04-framing-the-canvas.md`; feeds a future card when the editor/keyboard-grammar work enters a sprint.

## VisiData's principles, verified

1. **Everything is a sheet.** Data, column metadata, options, errors, status history, async threads, the command log — all are sheets manipulated with the *same* commands. One data structure, one vocabulary, total leverage.
2. **The sheet stack.** `Enter` dives (into a group, a row, an error traceback), `q` pops. Navigation is spatial and reversible — you can always answer "where am I and how did I get here?"
3. **Single-keystroke commands with a compositional scope grammar.** Prefixes modify *scope*, not meaning: `g` = global/all/selected, `z` = smaller/alternate scope, `gz` = both. Learn a verb once, its whole scope family comes free. Every command also has a longname reachable via `Space` (palette). Discoverability (menus, sidebar) was added late and additively — the expert path is never lengthened to serve the novice path.
4. **Columns are first-class, typed lenses; rows are opaque.** One-key type/rename/hide/key-toggle/derive-with-expression. Explicitly closer to an RDBMS than to a cell-grid spreadsheet.
5. **The command log makes every session a program.** Every modifying command (never navigation) is logged; the log is itself an editable sheet, filterable to "what steps produced this sheet?", replayable headless. Undo and macros sit on the same substrate.
6. **Async everywhere; the UI never blocks.** Per-sheet progress in the status line, `^C` cancels, threads are inspectable as a sheet.
7. **Thin chrome: status line + input line.** No panels or ribbons; errors open as sheets, never modal dialogs. Screen area belongs to data.
8. **Immediacy.** Every keystroke produces a visible result now, on real data.

What VisiData deliberately lacks: mouse-first affordances (every action must be nameable and loggable), WYSIWYG editing (authoring edits a structured artifact; rendering is a projection), persistent panels, blocking operations.

## Mapping to Brightfield

- **Everything is a surface.** The spec (structured outline), each view's *generated SQL*, its result preview, per-column stats (DuckDB `SUMMARIZE`), param/selection state, the error list, in-flight queries, the edit log — all keyboard-navigable surfaces sharing one key grammar. Focus a view → dive into its SQL; `q` pops back to the canvas (the root surface, never destroyed by diving).
- **Scope grammar is dashboard-native.** bare verb = focused view, `g` = whole dashboard, `z` = single mark/channel. E.g. `f` filter this view, `gf` cross-filter everywhere — card 0006's semantics as a keystroke. Every command gets a longname + `Space` palette. GPUI's action/keymap dispatch (Zed is keyboard-first) is the native substrate for exactly this — **the key grammar does not wait for the editor; it starts in the minimal shell (Option A)**.
- **The data sidebar is columns-as-lenses, not a file tree.** Per-column one-key ops (type, rename, hide, derive with a SQL expression) that *emit spec edits* — the sidebar is a view of the spec's data layer, YAML stays the single source of truth. This is MotherDuck's Column Explorer crossed with VisiData's Columns Sheet.
- **Command log = the stream of structured spec edits.** The strongest fit of all: authoring actions are spec edits, param/selection changes are already declarative in Mosaic — so replay, undo, per-view lineage ("how did I build this chart?"), authoring macros, and headless session replay for CI all fall out of one substrate. This is the spine; the other principles pay off through it.
- **Async is already half-built** (reactive sprint). The rule to adopt: no query ever blocks the render loop; stale views dim with progress; `^C` cancels the focused view's queries; a Queries surface lists in-flight DuckDB work.
- **Status line, not dialogs.** One-line bar: last edit, row counts, query ms, error count. Errors (bad YAML, failed SQL) open as surfaces, `Enter` dives from error → offending spec node.

**Resolving the keyboard/mouse tension:** the canvas stays mouse-interactive — but pointer gestures (brush, click-select, drag) are *data interactions* that flow through Mosaic params/selections, hence loggable and replayable like keystrokes. Authoring stays keyboard-first and command-named. Never let a style/layout edit exist only as a drag.

**Implication for the editor decision:** "in-app spec editor" is not primarily a text pane. It is the surface system + key grammar + command log, of which a YAML text surface (gpui-component's tree-sitter `input`, eventually) is one member. Most of the VisiData vision is buildable in the hand-rolled shell on the current gpui pin, before any dock library is adopted.

Priority order if this becomes a card: (1) command log as spec-edit stream, (2) surfaces for SQL-per-view + column stats, (3) the scope-prefix key grammar + palette, (4) columns-as-lenses sidebar, (5) never-block async with a Queries surface.

→ Card candidate: "keyboard grammar + surfaces" (could land incrementally alongside the framed-window milestone; the command-log substrate wants design attention first — it touches spec edits, params, and undo).
