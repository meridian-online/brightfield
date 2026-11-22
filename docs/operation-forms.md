# Operations in the window

arcform holds a catalogue of 13 operations (filter, sort, hide, rename, derive, cast, fill, dedupe, limit, summarise, join, append, read a file). Brightfield makes each one reachable in the window through the interaction that suits it, previews its result as it is made, and records it through arc on Save. This document covers the parts every operation shares, the decisions behind them, and how to add an operation.

## The ruling this follows

The owner ruled on 2026-11-22 that each operation gets the interaction that suits it. Some operations want a small form, some are typed in place on a column's header, and some ask nothing and are a direct action on what is on screen. The foot row of the grid stays what it was: a convenience for a simple `where` and a simple `order by`, and not the place every operation is asked.

His worry about "a sea of dials and dropdowns" is the settings pages of cloud consoles, not forms as such. The aim is a window that recognises and responds to what the analyst does: operations that are transparent, with sensible defaults, previewed at each keystroke as DuckDB's Instant SQL previews a query.

Two rules follow from arc holding the catalogue. Brightfield writes no operation's SQL: a preview runs the model arc writes, and the step a Save records is the one `arc operation record` writes, byte for byte what any other way of recording the same operation writes. And an operation's values, which of them are required, and its words come from `arc operation describe`. The words Brightfield adds are its own labels, menus and sentences.

## The shared parts

| Part | Where | What it does |
|---|---|---|
| arc's description, read with its shapes | `crates/brightfield-shell/src/operation.rs` | Each value's shape (a kind of SQL, a column's name, a list of names, a whole number, one word of a few, a path, text), whether it is required, which values go together, and whether a table role takes one table or a list. `record_step` records an operation naming each table it reads by its role. |
| A slot per operation | `crates/brightfield-shell/src/operation_ui/` | One file per operation holding its `OperationUi`: where it starts, what the window must hold for it to start, the draft it begins as, whether it is kept at once or on Enter, how it asks, and how its step is named. `OPERATIONS` lists the 13 slots; a slot answers `None` until its operation is built. |
| A key per operation | `crates/brightfield-keys/src/operations.rs` | One key-registry entry per operation, each behind a `*_BUILT` constant of its own, with every key chosen up front so two operations built side by side cannot pick the same one. |
| arc's model for a draft | `operation_ui/draft.rs` | `draft_model` records the draft into a Protocol of stub tables in a temp folder, reads back the model arc wrote, and removes the folder. arc's history snapshots for that folder are sent inside it, so the analyst's history holds no entry for it. |
| The warm preview | `crates/brightfield-engine/src/preview.rs` | `WarmPreview` loads the data file and makes the steps the draft reads once, on a connection shut to anything outside it, then runs each edit's model alone. It holds the table whole, so a preview of an aggregate is exact. |
| The reads of a draft | `operation_ui/reads.rs` | One worker per draft. An edit is read at once with no pause; the worker takes the newest request waiting, so one arc child runs at a time and the last edit of a burst is always read; an answer for text since changed is dropped. |
| Keeping a draft | `crates/brightfield-shell/src/typed_sql.rs`, `window.rs` | A kept draft is an unsaved step carrying the request arc will be asked to record. It is laid out at the spine's foot and previewed by running arc's model. Save records it through arc; `remove-step` on the spine takes it back. |
| The window's side | `crates/brightfield-shell/src/window/operation_host.rs` | One entry point for a key, a palette row, a header gesture and a menu item; the keys while an operation is being made; the status band's line. |
| A header as a place to act | `operation_ui/header.rs` | The parts of a column's header an operation starts from (a double-click on the name, a click on the type or the missing count), the menu a secondary click opens, the new-column cell, and the field an operation asks with in place, with what it suggests listed under it. |
| A form in the Operation section | `crates/brightfield-shell/src/operation_panel.rs` | An operation with a form of its own draws it in the inspector's Operation section, over its own draft. The section adds the operation's name and no control of its own. |
| Availability | `operation_ui/open.rs` | What the window must hold for an operation to start, checked in the order the operation lists its needs, with one sentence per need. |
| Reading a generated step back | `crates/brightfield-shell/src/generated.rs` | A folder holding a step arc generated for any operation opens, lays out and previews. A filter or a sort is read back into its clause; any other step is read by its header and the table it makes, and its preview runs the model as arc wrote it. |

## From a start to a Save

1. **A start.** A key in the grid, a palette row, a gesture on a part of a column's header, or a menu item comes to `MeridianApp::start_operation`. It says the first thing in its way on the status band: the window has no arc, arc's description has not come, a need is unmet, or the operation's own `begin` refuses. The sentence stays until the next key or pointer press.
2. **The draft.** `begin` makes the draft with its defaults filled from what Brightfield knows of the column. A column given to a value is spelled for that value: bare for a value arc takes as a name, and as SQL spells it (quoted when it needs quotes) for a value arc takes as SQL.
3. **The preview.** The draft is read at each edit. The grid shows the answer's page under the preview label with the missing-value counts of the preview's own columns. While arc or DuckDB refuses the draft as it stands, the last good rows stay on screen, faded, and the reason is on the status band.
4. **The Editor.** The Editor tab shows arc's model for the draft, read-only, following each edit. For a kept or recorded step in front it shows that step's model.
5. **Keeping.** Enter keeps the draft once each value arc requires is given and the reads hold arc's model for the draft as it stands. An operation that asks nothing is kept on its first answer. Esc drops the draft. A kept draft is an unsaved step; no file is written.
6. **Saving.** ⌘S keeps an open draft first when Enter would, then records the kept steps through arc in the order they were made. A draft that cannot be kept stays open, saying why, and the rest is saved. A refusal from arc at Save leaves the step kept, with arc's words on a banner.

Starting an operation over the grid of a kept step of the same operation corrects that step: the draft begins from what was kept, and Enter replaces it in place under its name.

## Decisions made on the owner's behalf

Each of these is reversible, and each is the owner's to overturn.

1. **A kept operation, other than filter and sort, is a new node**: an unsaved step at the spine's foot with the not-recorded mark and its table in front, as a statement kept from the palette is. Filter and sort stay views of the node in front.
2. **The preview runs arc's own model on each edit.** arc's round trip, about 150 ms a call when this was built, is the floor until arc can render a model in-process.
3. **Fade, don't clear.** While an edit fails, the grid keeps the last good rows faded. The rows are not faded while a newer edit is being read and has not been refused: the read takes about a tenth of a second, and a fade at each keystroke would flicker.
4. **A preview states its own missing values**, as a line above its rows naming each column that has some with its count. The header band's counts are the loaded table's, which the operation may have changed. A coloured validity band in the preview's own header would need the rest of the column profile for rows that are not profiled, so the count is given as a number.
5. **Direct actions keep at once.** An operation that asks nothing keeps its step as soon as arc admits it and DuckDB has run it. A dedupe that keeps every row keeps no step and says the table has no duplicate rows.
6. **Names.** `<table>_<word>_by_<column>` for row operations and `<table>_<word>_<column>` for column operations, slugged, with `_2`, `_3` for a name taken. A name says only what that one step did, and is fixed when the draft is kept.
7. **Keys** take VisiData's where it has one: `-` hide, `=` derive, `f` fill, `shift-f` summarise, `&` join. The rest are `shift-r` rename, `shift-t` cast, `shift-u` dedupe, `shift-l` limit and `shift-a` append.
8. **An operation does not start over a `where` or `order by` that is applied and not recorded**, and says why: it would read the table, not the rows on screen.
9. **The Editor shows a step's SQL read-only.** Editing it there into a SQL step is later work.
10. **`remove-step` on a kept step takes it back at once**, a kept statement included, since no file holds it. A step kept after it that reads its table refuses that, named in a banner.
11. **A statement typed over a draft's grid is a step of its own.** The draft's text is arc's model, which is corrected from the operation and not typed over.
12. **A `read-file` step previews when it reads a local CSV or Parquet file**, loaded beside the data file before the preview's connection is shut. A URL, a glob, another format, or a path from the home folder is left to a run, and the preview says so.
13. **The foot row's fields open without waiting on arc, and without a Protocol.** They write their own text and apply it as a view, so `w` and `o` open their fields in a window with no arc, and in one with no Protocol of its own, where a click on the field opens it too. The palette lists the two only where a Protocol is saved.
14. **A keystroke typed into a field in place is that field's alone.** While an operation asks on a header, a letter goes into its value and reaches no other handler, so `o` typed into a name does not sort the rows. A key held with Cmd, Ctrl or Alt is still the window's.
15. **The new-column cell stays in reach.** It stands after the table's last column where that leaves it inside the pane, and at the pane's trailing edge otherwise.
16. **A header's type is one place, not a region.** The type part is the leaf and storage row at the full density, the title beside the name at the compact one, and the glyph where neither is drawn.

## What arc lacks

These are arcform's to close. Until then the workarounds above stand.

1. **A render that writes nothing, in-process.** Each edit records into a throwaway Protocol through a child process. A library call that returns the model for a request, with arc's refusals and no folder, would close most of the gap to Instant SQL and retire the stub tables.
2. **A record of a step's operation that a program can read.** A generated step carries its model and a header written for a person. Brightfield reads the tables a step reads from that header, which is ambiguous for a table whose name holds a comma and a space. A header of two names that are one table is settled by the model's own quoted spelling; three or more names with two of them one table is not. Correcting a recorded step from its form is out of reach for the same reason.
3. **Which column values must already exist.** A column-shaped value does not say whether it must be a column of the table or is a new name, so each operation's `begin` says it.
4. **A list item holding a comma.** The command line splits a list on `,`.
5. **Rules the schema does not state.** `join-tables` takes exactly one of `using` or `condition`, which only arc's check enforces, so a form learns it from the refusal.
6. **No stage, and no other tool's word, in a description.** Brightfield holds the stages.

## Adding an operation

An operation is three files, so operations can be built side by side.

1. **Fill its slot** in `crates/brightfield-shell/src/operation_ui/<op>.rs`: a `static` `OperationUi` and a `ui()` that returns it. Say where it starts (`starts`), what it needs (`needs`, in the order the sentences should be said), the draft it begins as (`begin`), whether it keeps at once or on Enter, its in-place field or its form, and its name rule.
2. **Flip its `*_BUILT`** in `crates/brightfield-keys/src/operations.rs`. Its key then joins the registry, the palette and the help sheet. `hide-column` and `derive-column` use `-` and `=`, which a dispatch test in `crates/brightfield-keys` holds to resolving to no verb in the grid's context; that test changes with them. A shifted key is taken before its bare letter in the grid, so `shift-l` limits rows and `l` still moves the cursor.
3. **Write its test file** from the harness in `crates/brightfield-shell/tests/support/operations.rs`: an `OpWindow` on a copy of a fixture, started from the operation's key, its header gesture and the palette.

The tests in `tests/operation_registry.rs` then bite: the slot holds its own operation, its verb is in the key registry, its draft names only what arc describes, its needs agree with what arc says it reads, and it is filed under its stage.

Three things the shared code does not do yet. The palette lists an unavailable operation without greying it, and choosing it says its sentence; a header's menu does grey it, with the sentence under it. The spine does not pick a table for an operation, so `Begin::taken` is empty. And `Begin::model`, the Protocol's model of its tables' relationships, is `None`.
