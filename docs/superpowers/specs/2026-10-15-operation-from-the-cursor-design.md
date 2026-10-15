# An operation applied from the keyboard to what the cursor is on

Design, 2026-10-15. The builder plans from this document. When the branch is ready, its outcome is folded into the module docs it changes, and this document is deleted.

## Intent

An analyst with a table's grid open puts the cursor on a cell and presses a key. A field in a row at the foot of the grid's tile asks for a condition, already filled in from that cell. The grid shows the rows the condition keeps, and a panel in the Inspector rail explains the operation. When the analyst saves, the condition is recorded through arc as a generated `filter-rows` step that `arc run` runs. An order typed in the `order by` field sorts the grid and is recorded as `sort-rows` in the same way.

Why it is built this way:

- **Keyboard first, one value at a time.** What the cursor doesn't supply is asked for one value per question. A Protocol should not become a sequence of clicks.
- **See before you keep.** A view shows its rows immediately, and recording happens at Save. The analyst sees the result before anything is written.
- **The same operation writes the same file however it is recorded.** brightfield records through arc and writes no filter step of its own, and the Protocol's versions show who recorded what.
- **The row asks, and the panel shows.** The row keeps entry on the keyboard. The panel hides no setting and teaches what an operation takes.

## Goals

1. The row at the foot of a grid tile, with `where` and `order by` fields, and a key that opens `where` from the cursor's cell.
2. A panel in the Inspector rail, drawn from arc's description of the operation.
3. Save records each applied view through arc, as a step whose name says what it did.
4. `order by`: an ordered read in the grid, recorded as `sort-rows`.
5. A folder that holds the load plus generated steps reopens as a Protocol with those steps laid out. A never-run step shows as not run, with a labelled first-page preview when it is safe to compute one.
6. A test drives the whole sequence: cursor, key, condition, view, panel, Save, reopen.

## Non-goals

These are settled for later work, not dropped:

- **Showing a run's results**, read from arc's working database, and **filtering or sorting a saved step**. These are the next issue, which is blocked on this one. Hugh's rule for that issue: filtering or sorting a step that hasn't run starts a run up to that step, and the view works on the run's real table, never on a preview. Here, the row at the foot works on the loaded table only.
- **Opening hand-written Protocols.** A folder with more than one step, where any step after the load is not a generated operation, is still refused on open, as it is today.
- The key itself. It is provisional until the key registry's scores decide it.
- Stepping back through versions. Until that exists, a filter applied by mistake is undone by clearing the field.
- Changing a value inside the panel.
- `hide-column`, operators in the app, and typed SQL recognised as an operation. Typed SQL will later put a recognised filter into this row's `where` field, so the asking state must accept a condition from somewhere other than the key.

## Anti-goals

The work fails, even if it technically works, in any of these cases:

- **Opening a folder starts a run.** Opening never invokes `arc run` and never executes a step brightfield can't account for in full.
- **brightfield writes a step arc would write differently.** Every recorded step comes from `arc operation record`.
- **A preview is shown as if it were a run, or as if it were the data's range.** It must read as not run. It is a first page only, shown to prompt the next question, and it claims no total row count.
- **The Protocol drifts away from a standard, parseable spec.** Every step brightfield opens must be one whose operator it can name and whose place in the graph it can draw.
- **Save asks for a name.** Asking every time would get frustrating.

## Constraints

- arcform stays at the current pin (`f0561cd`), and this work needs no pin move. At that pin, `arc operation describe` and `arc operation record` are reachable only through the CLI. In brightfield that means the child runner.
- One cargo build at a time on the Mac, which has 16 GB of memory.
- The repo is public. Spec, plan, commits and pull request descriptions use plain words.
- Baselines follow the repo's rule. A moved baseline is redrawn with `UPDATE_SNAPSHOTS=1`, read, and named in the pull request, and `kittest.toml`'s thresholds stay as they are.

## What is already built

| Piece | Where |
|---|---|
| The cell cursor: `GridCursor`, held on the document, moved by a click, `h j k l` or the arrows, in a Grid key context whose bindings a test pins | `crates/brightfield-shell/src/data_grid.rs:441`, `app.rs:890`, `crates/brightfield-keys/src/registry.rs:1653` |
| Rows under a condition: `step_rows_count_where` and `execute_step_rows_window_where`, which take the condition as an argument to the read rather than as state, gated by `step_rows_sql_where`. A condition is refused unless DuckDB parses `SELECT 1 WHERE <condition>` as one statement, which is arc's own probe. Nothing in the shell calls them yet. | `crates/brightfield-engine/src/lib.rs:1715`, `:1763`, `:1866` |
| The grid's page reads, ordered by `ORDER BY ALL` | `lib.rs:1878-1895` (`window_rows`) |
| `GridCache`, keyed on the interaction generation and the mark only | `data_grid.rs:1247` |
| The child runner: `BRIGHTFIELD_RUN_AS_ARC` makes this binary act as arc. `run_to_completion(program, dir)` always passes `run`, and merges stderr into its log. | `run.rs:71`, `:105`, `:311` |
| Save and Run: both call `save_to`, which rewrites `arcform.yaml` and `models/load.sql` in full | `window.rs:7585`, `:7691`, `one_step.rs:305` |
| The unsaved mark, which reads chart edits only | `window.rs:3078`, `UNSAVED_MARK` at `:183` |
| A one-step Protocol opens as its data file. Any other manifest is refused unless the offline variable is set. | `one_step.rs:343`, `window.rs:1474-1497`, `protocol.rs:303-371` |
| The Inspector rail's panes: `protocol-inspector` and `chart-controls`. The rail opens shut for a one-step Protocol. | `crates/brightfield-workbench/src/arrangement.rs:546`, `window.rs:3264` |
| `record_grid_filter` writes a filter step of brightfield's own. It has no caller outside tests. | `crates/brightfield-protocol/src/record.rs:121` |

What arc provides at the pin:

- `arc operation describe <long name>` prints pretty JSON to stdout: `long_name`, `summary`, `reads` (roles, each with `name`, `kind`, `required`, `description`, and sometimes `of`), `writes`, and `parameters` (a JSON Schema). For `filter-rows`, `reads` is `table` and `column`, and `parameters.properties.where` carries a description sentence and `x-comparisons`: is `=`, is not `!=`, over `>`, under `<`, between, is null. For `sort-rows`, the parameter is `order_by` with `x-directions` ascending `asc` and descending `desc`. An unknown name prints `error: …` to stderr and exits 1.
- `arc operation record <long name> --on <table> --name <step> --arg <key>=<value> --dir <dir>` appends `models/NN_<slug>.sql`, headed `-- generated: <long name> on <table>`, and a step entry at the end of `arcform.yaml`, with no `depends_on`. It writes a local-history version and runs nothing. On success it prints one line to stdout and exits 0. On refusal it prints `error: <message>` to stderr, exits 1, and leaves the folder untouched. It refuses a step name that is already taken, and an `--on` that no step makes as a table.
- The generated SQL is written over several lines: the header, `CREATE OR REPLACE TABLE "<name>" AS`, `SELECT *`, `FROM <table>` (bare or double-quoted, as arc's `from_ident` writes it), and `WHERE <where>;`, or `ORDER BY <order_by>;` for a sort. The condition and the order are written exactly as given.
- `arc::spec::sql_is_generated(sql)` checks the header. Nothing public parses the header into an operation and a table, so brightfield splits `<long name> on <table>` itself.

## The design

### 1. The row at the foot, and the key that asks

- Each grid tile in its rows layout gets a band 26 high at its foot (`DataGridItem::ui`, `data_grid.rs:1742`). The band holds a `where` field and an `order by` field, each starting with its word. The count of rows and columns sits at the band's trailing end. The band shows no key hints.
- The field reads as the condition that will be recorded, with no prompt glyph. It is brightfield's own field and not the design system's `query_line`, because `query_line` draws a `›` glyph.
- **The row works on the loaded table's grid only**, that is, the data file's own step. On a generated step's grid the band is shown read-only, and the key puts a message on the status band. On a step that has not been run, the message offers Run. On one that has been run, it says that filtering a saved step comes in a later version. Filtering or sorting a step that has not been run must start from a run, never from a preview, and showing a run's table is the next issue.
- **The key**, pressed on the cursor's cell, focuses `where` with `where house_age = 28` in it and the value selected.
- **The literal comes from the stored value, never from the text shown.**
  - Strings are single-quoted with `'` doubled.
  - Floats are written in DuckDB's exact round-trip form, so `=` on a float keeps the row the cursor is on.
  - Dates and timestamps are written as typed literals (`DATE '…'`, `TIMESTAMP '…'`).
  - A NULL cell offers `where <column> is null`, because `= NULL` keeps nothing.
  - Tests cover each of these alongside the integer `house_age` case.
  - Typing `=`, `!`, `>` or `<` replaces the comparison with `=`, `!=`, `>` or `<` and keeps the value.
  - Any other key replaces the value.
  - The status band names the comparison in words: is, is not, over, under.
- **While the analyst types**, the grid and the count follow each condition that parses. A condition that doesn't parse leaves the rows as they were and shows DuckDB's message.
- **Enter applies a view.**
  - No file is written.
  - The field carries a not-recorded dot.
  - The window's unsaved mark covers the view, and so does the close question. `carries_unsaved_mark` reads applied views as well as chart edits.
- **Esc** restores the field to its value before editing. **An emptied field plus Enter** shows the whole table again.
- **The view refines the node in front.** It is a read of the same step with a condition, not a new node. The read uses the existing `_where` engine calls, and `GridCache` is keyed on the condition and order as well, so a filtered page is never served from an unfiltered cache.
- The asking state accepts a condition from any source, not only from the key, so typed SQL can fill it later.
- **A view changes the grid only.** Charts on the same step keep their rows, which is why the engine takes the condition as an argument to the read. After Save and on reopen, the dashboard and the saved chart file still draw from the load. A test pins both.
- Builder's calls, which the builder may change: there is no row cursor while the field is focused, and after Enter the cursor sits on the first row shown, in the same column.

Reference counts, from DuckDB over the start file at `0abd430` (16,640 rows): `house_age = 28` keeps 375, `> 28` keeps 8,831, `> 40` keeps 3,402, and `!= 28` keeps 16,265. Tests should use these numbers.

### 2. The panel in the Inspector rail

- A new pane in `INSPECTOR_PANES`, next to `protocol-inspector` and `chart-controls`. It is an `ItemSpec` plus an `Item` impl, the way `chart-controls` is built.
- **The key opens the rail** on this pane. Esc shuts the rail again if the key was what opened it.
- **What the panel draws** comes from `operation describe <long name>`, run through the child runner:
  - *target*: each role in `reads` (table and column), with arc's description sentence and where the value came from: the node in view, or the cursor.
  - *values*: each entry in `parameters.properties`. `where` shows the value from the cell, or *asked* while it is still being asked, and lists the six `x-comparisons` by word and sign. Each row carries arc's description sentence.
  - arc's words are the panel's words. Where a value came from, the value itself, and the keys are brightfield's.
- **The runner needs an argument list.** It needs a variant of `run_to_completion` that takes arguments and keeps stdout and stderr apart. JSON is read from stdout, and stderr is the refusal.
- **The describe output is cached** per operation for the session, because it does not change while the pin is fixed.
- **`sort-rows`** is drawn the same way, from `order_by` and `x-directions`.
- **With no description** (the runner is missing or the describe call fails), the row at the foot still asks, and the panel says why it has nothing to show.
- **The panel changes no value.** Editing values in the panel is later work.
- A test pins that the runner lists both operations and that describing `filter-rows` yields `where`.

### 3. Save records through arc

- **For each applied view, in the order filter then sort**, Save runs `arc operation record filter-rows --on <table> --name <name> --arg where=<condition> --dir <dir>`. For a sort, it runs `sort-rows` with `--arg order_by=<order>`.
  - `--on` is the *table* the node in front makes. For a data file that is `california_housing`, not the step `load`, which arc refuses.
  - A sort recorded in the same Save goes `--on` the filter's new table.
- **Step names, as Hugh ruled on 2026-10-15.** Save never asks for a name.
  - The name is `<table>_<effect>_by_<column>`, where the effect is `filtered` for `filter-rows` and `sorted` for `sort-rows`. Examples: `california_housing_filtered_by_house_age`, `california_housing_sorted_by_median_income`.
  - `<table>` is the table the data came from (the root table), not the full chain. A step's name says only what that step did, and the step list shows what it reads from. A filter followed by a sort is therefore `california_housing_filtered_by_house_age`, then `california_housing_sorted_by_median_income`.
  - The column is the one the condition actually names. It is not the cell where the cursor started, because the analyst may have edited the condition to name another column. brightfield finds the column references by having DuckDB parse the condition, for example with `json_serialize_sql` over `SELECT 1 WHERE <condition>`. If there is exactly one, it goes in the name. If there are none or several, the name falls back to `<table>_filtered`. A sort uses the first column of its order.
  - The word for each operation comes from a small table in brightfield. An operation with no entry falls back to its long name.
  - The column and the root table are turned into a slug before they go in the name: characters outside `[A-Za-z0-9_]` become `_`, runs collapse, and leading and trailing `_` are trimmed. arc refuses a step name holding `#`, a control character, or surrounding whitespace, and a CSV header can hold any of those. A column with nothing left after slugging falls back to `<table>_<effect>`.
  - A name already taken by a step in the manifest gets `_2`, `_3`, and so on, chosen before calling arc, so a recording is never refused on name and never overwrites.
  - This replaces the earlier design, in which the analyst named the step at Save.
- **Save and Run stop erasing steps they did not write.** Once the folder has generated steps, `save_to` must not rewrite `arcform.yaml` from brightfield's one-step template. The builder decides how. The requirement is that arc's appended steps and their models survive Save, Run, and a second Save.
- **A refusal** is shown in arc's words, from stderr. The view stays applied and the unsaved mark stays on. A Save that records several views stops at the first refusal. Views already recorded stay recorded, since each recording is its own arc version. The grid keeps showing what the analyst saw: a recorded part reads as recorded (no dot), and the rest stays applied and unrecorded, remembered with the table it must go on (the filter's new table), so the next Save records only the rest, on that table. The window adopts the Protocol once nothing is left unrecorded. Save reports failure on a refusal, so closing after it does not lose the view.
- **After a successful Save**, the window shows what reopening the folder would show: the Protocol with the new steps laid out, the ledger rail open, and the newest step selected. Its grid shows the labelled preview described in part 5.
- **After a Run in the session**, each step's status comes from the run record, which Run's reload already reads (`inputs_with_last_run`, `one_step.rs:260`). A generated step that ran therefore reads as run, not "not run". Its grid still shows brightfield's first-page preview, labelled as read from the file and not as the run's table, until the next issue shows the run's table. The load step keeps showing what it shows today.
- `record_grid_filter` writes a step arc would write differently, so it is deleted along with its tests. Hugh's ruling that brightfield writes no filter step of its own is the reason.

### 4. `order by`

- The grid's page reads gain an ordered form: the order followed by every column by position (`ORDER BY <order>, 1, 2, …, n`; DuckDB refuses `ALL` after another key), a total order over the remaining columns, so that pages over ties do not tear. An order is refused unless DuckDB parses `SELECT 1 ORDER BY <order>` as one statement, which is arc's probe.
- The engine gains the ordered reads alongside the `_where` ones, taking condition and order together as arguments.
- A key on the cursor's column opens `order by <column>` with the caret after it. This is the builder's call.
- The field behaves like `where`: it follows the order as it is typed, applies on Enter, carries the not-recorded dot, Esc restores it, and emptied it shows the natural order.
- Save records `sort-rows` after any filter, on the filter's table. See part 3.

### 5. Reopening a folder with generated steps

- **What opens.** A folder opens as a Protocol with its steps laid out when:
  - its first step has the data-file load shape `data_file_named_by` recognises today, and
  - every other step's model starts with arc's header (`sql_is_generated`), and the header names an operation brightfield knows (`filter-rows`, `sort-rows`) and a table an earlier step makes, and
  - the model matches arc's template for that operation, with the step's own name quoted in `CREATE OR REPLACE TABLE`.
  The match is checked by parsing, not by trusting the header. A model that keeps the header but was edited by hand into something else does not match, and the folder is refused as a hand-written Protocol is today, with a message naming the step. This holds to Hugh's constraint that a Protocol stays a standard spec, with each step's operator clear and its graph drawable.
- **Opening starts no run.** `arc run` is not invoked.
- **Each generated step shows as not run yet.** Showing a previous run's tables is the next issue.
- **A labelled preview** is drawn in a never-run step's grid when it is safe. It is the first page of the step's rows and no more. It prompts the next question and must not pass for the data's range, so it claims no total count. It doesn't scroll past the first page, and the count at the band's end reads as a preview, not as a row count.
  - *Free of malice.* The step is a known generated operation over the data file the load reads. The preview's read runs on its own DuckDB connection, separate from the session's. That connection loads the data file into a table first, then sets `enable_external_access = false` and locks its configuration, and only then runs the steps' conditions and orders. A condition that tries to read another file or the network therefore fails. The step stays not run and shows DuckDB's message.
  - *Memory safe.* The data file's size on disk is under a fixed limit, checked before anything is read. The builder's call is 256 MB. The preview connection also sets a memory limit and does not spill to disk, because a compressed Parquet file can take several times its size once loaded. Above the size limit, or when the preview runs out of memory, the grid says the file is too large to preview and offers Run. The file's path is quoted with `'` doubled.
  - The grid labels its rows as a preview computed by brightfield and not a run. The step's status still reads not run. No run record is written, and arc's working database is never touched.
  - A chain is previewed through its chain: a sort on a filter previews the filter, then the sort.
  - A `sort-rows` step's grid reads with that step's own `order_by`, taken from its generated model, followed by every column by position as the tiebreak. A table built with `ORDER BY` keeps no order DuckDB guarantees on read, so the grid's default `ORDER BY ALL` would show it unsorted. Parsing a generated model therefore yields the operation, the table it is on, and its argument, for use by both the open check and the grid.
- **A step that is not safe to preview** shows an empty grid that says why, with Run as the way forward.
- The exemption text in `protocol.rs:303-371` gains this third case and says why it is honest: brightfield draws only steps it can recompute from the file, and labels them as previews.

### 6. The driven test

A test with the window handed the cargo-built binary as its runner, the way `tests/run_control.rs:89` does it, walks the sequence end to end on the housing file:

1. Cursor to a `house_age` cell holding 28.
2. Press the key. `where house_age = 28` is shown, and the panel lists table, column and `where`.
3. Type `>`. 8,831 rows.
4. Enter. Not-recorded dot shown, unsaved mark on.
5. Save. `arcform.yaml` has the step `california_housing_filtered_by_house_age`, its model starts with `-- generated: filter-rows on california_housing`, and the window shows two steps.
6. Reopen the folder. Two steps, the second not run. Its preview's first page holds only rows with `house_age > 28`, and no total count is shown. The key on that grid offers Run.
7. Run. The run succeeds, and the second step reads as run.
8. Charts drew from the load throughout.

A second test does the same with `order by`.

## What is left to the builder

- How Save stops erasing steps it didn't write (part 3).
- The exact layout of the band and the panel, within the rulings above.
- Whether the panel is its own pane or a section of the existing inspector pane, so long as the key opens it and Esc shuts it.
- How the runner variant is shaped.
- The order of work. The suggested order is parts 1, 2, 3, 4, 5, since each uses what the one before builds. All of them touch `window.rs` and `app.rs`.

## Rulings Hugh made on 2026-10-15

- Save never asks for a name. A step's name says what that one step did, using the table and the column, and the step list shows what it reads from.
- Reopening shows the steps laid out as a Protocol, and opening never starts a run, as Jupyter never re-executes on open.
- A never-run step may show a labelled preview when the data is safe to read, and the preview never counts as a run.
- A preview is a first page, shown to prompt the next step. Filtering or sorting a step works from a run of the Protocol up to that step, over the full data, never from the preview. That work is in the next issue, and here the row works on the loaded table only.
- Showing run results and opening hand-written Protocols are split into their own issues. A Protocol must stay a standard spec that can be parsed, so each step's operator is clear and its graph can be drawn.

## Calls made in this design that Hugh did not rule

- `_by_` in the step name, and the fallback to `<table>_filtered` when the condition names zero or several columns.
- `_2` suffixes for a name already taken.
- The preview's safety rules: a separate connection with external access locked off, and a 256 MB file-size limit.
- A reopened folder whose generated model was edited by hand is refused, not opened.
- After Save, the window adopts the Protocol and selects the new step.
- After a Run in the session, a generated step reads as run, but its grid still shows the labelled preview from the file.
- The literal comes from the stored value, and a NULL cell offers `is null`.
- Views and recorded steps leave charts on the load.
- `record_grid_filter` is deleted.
- Carried over from the earlier cut: brightfield's own field, no row cursor while asking, the cursor on the first row after Enter, and a key on the column that opens `order by`.
