# Operation From the Cursor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An analyst filters and sorts a data file's grid from the keyboard in a row at the foot of the tile, sees the operation explained in the Inspector rail, and records it through arc on Save. The folder then reopens as a Protocol whose generated steps show as not run, with a labelled first-page preview.

**Architecture:** The engine gains reads that take a view (a condition and an order) as arguments, plus a separate locked-down connection for previews. The shell holds the view on `ChartDoc`, draws the foot band inside `DataGridItem::ui`, and reaches arc's `operation describe` and `operation record` through the child runner, given an argument list. Generated steps are recognised by parsing arc's model template, which is what lets a load-plus-generated folder open without the offline variable.

**Tech Stack:** Rust 1.95.0 (`cargo +1.95.0` in a worktree), egui/eframe, egui_kittest, DuckDB through `brightfield-engine`, arcform pinned at `f0561cd`.

**Spec:** `docs/superpowers/specs/2026-10-15-operation-from-the-cursor-design.md`. Read it before any task. Its "What is already built" table and its arc facts are not repeated here.

## Global Constraints

- The repo is public. No planning identifiers in code, comments, commits or PR text. Describe the work in plain words.
- arcform stays at `f0561cd`. No pin move.
- One cargo build at a time. Never run two implementers in parallel. The Mac has 16 GB.
- Never `cargo fmt --all`. Use `cargo xtask ci --step fmt`.
- Before each commit, after `git add`: `cargo xtask ci --step fmt`, `--step clippy`, `--step public-hygiene`, and the tests the task touches. After each commit: `--step comment-citations` and `--step commit-messages`.
- A moved baseline is redrawn with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test <file>`, read, and named in the PR. `kittest.toml` stays unchanged.
- Housing numbers (16,640 rows): `house_age = 28` gives 375, `> 28` gives 8,831, `> 40` gives 3,402, `!= 28` gives 16,265. These are over `crates/brightfield-shell/assets/starts/california_housing.parquet`. The shell's `tests/data/california_housing_sample.csv` is a sample, so window tests compute their expected counts with the engine's `_where` count on the same file and do not hard-code them.
- Step names: `<root table>_<word>_by_<column>`. Words: `filter-rows` gives `filtered`, `sort-rows` gives `sorted`. Fallback without a single column: `<root table>_<word>`. A name already taken gets `_2`, `_3`, and so on.
- Preview size limit: `PREVIEW_FILE_LIMIT_BYTES = 256 * 1024 * 1024`, measured on the data file's size on disk.
- Foot band height: `FOOT_BAND_HEIGHT = 26.0`. No rung of the design ladder is 26, so state the frame as the source in the constant's doc comment.
- Provisional keys, Grid context: `w` is `filter-rows-at-cursor` and `o` is `sort-rows-at-cursor`. They stay provisional until the registry's scores decide.

## Review Focus

1. **A cell holding NULL, a quoted string, a float, or a date.** The key offers a condition that keeps the cursor's own row. Task 1 tests the literal for each kind. Task 3 tests that the NULL case offers `is null`.
2. **A condition naming a column that doesn't exist, or holding a second statement.** The rows are kept, DuckDB's words are shown, and nothing is recorded. Task 3 tests both.
3. **A generated model whose header is kept but whose body was edited by hand.** The folder is refused on open with a message naming the step, and is never previewed. Task 7 tests this.
4. **A preview condition that reads another file** (`house_age > (SELECT count(*) FROM read_csv('/etc/hosts'))`). The preview fails with DuckDB's message and the step stays not run. Task 9 tests this.
5. **Save with a sort and a filter applied, run twice in a row.** The second Save records nothing new and erases nothing. Task 8 tests this.

---

### Task 1: Engine reads under a view, with column references and literals

**Files:**
- Modify: `crates/brightfield-engine/src/lib.rs` (next to `step_rows_count_where` :1715 and `execute_step_rows_window_where` :1866)
- Test: `crates/brightfield-engine/tests/rows_under_view.rs` (new). Copy `session()` and its helpers from `tests/rows_under_condition.rs`.

**Interfaces:**
- Produces:
  - `pub struct RowsView { pub condition: Option<String>, pub order: Option<String> }`, which derives `Clone, Debug, Default, PartialEq, Eq, Hash`.
  - `Session::step_rows_count_view(&self, index: usize, audience: RowsAudience, view: &RowsView) -> Result<u64, EngineError>`
  - `Session::execute_step_rows_window_view(&self, index: usize, offset: u64, limit: u64, audience: RowsAudience, view: &RowsView) -> Result<Vec<RecordBatch>, EngineError>`
  - `Session::condition_columns(&self, condition: &str) -> Result<Vec<String>, EngineError>`, which returns distinct column names in order of first appearance.
  - `Session::order_columns(&self, order: &str) -> Result<Vec<String>, EngineError>`, with the same ordering.
  - `Session::cell_literal(&self, index: usize, audience: RowsAudience, view: &RowsView, row: u64, column: &str) -> Result<CellLiteral, EngineError>`, with `pub enum CellLiteral { Null, Value(String) }`. `Value` holds SQL literal text.
  - `EngineError::OrderRefused { order: String, refusal: ConditionRefusal }`

- [ ] **Step 1: Write the failing tests.**
  - `a_view_with_only_a_condition_reads_what_the_where_read_reads`: for `"house_age > 40"`, the count is 3,402, and the paged rows equal `execute_step_rows_window_where` page for page.
  - `an_order_sorts_every_page_and_ties_do_not_tear`: with `order = "house_age DESC"`, page through with `limit = 1000`. The concatenated `house_age` is non-increasing, the total is 16,640, and no row appears twice. Compare `(longitude, latitude, house_age, median_income)` tuples as a set.
  - `a_condition_and_an_order_compose`: `house_age = 28` with `median_income ASC` gives 375 rows with `median_income` non-decreasing.
  - `an_order_with_a_second_statement_is_refused`: `"house_age; DROP TABLE x"` gives `OrderRefused { refusal: ConditionRefusal::SecondStatement { .. }, .. }`.
  - `the_columns_a_condition_names_are_listed`: `"house_age > 28"` gives `["house_age"]`, `"house_age > 28 AND median_income < 3"` gives both in order, `"1 = 1"` gives `[]`.
  - `the_columns_an_order_names_are_listed`: `"median_income DESC, house_age"` gives `["median_income", "house_age"]`.
  - `a_cells_literal_keeps_its_own_row`: for row 0 of each column, `col = <literal>` counted under the view is at least 1. Cover the integer, the float and any string column in the file.
  - `literals_quote_strings_and_type_dates`: in a session over an inline `VALUES` table with a string holding `'`, a DATE, a TIMESTAMP and a NULL, the literals are `'O''Brien'`, `DATE '2024-01-31'`, `TIMESTAMP '2024-01-31 12:00:00'`, and `CellLiteral::Null`.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-engine --test rows_under_view`. Expected: compile errors for the missing items.
- [ ] **Step 3: Implement.**
  - `step_rows_sql_view` wraps `step_rows_sql` with the condition, using `step_rows_sql_where`'s gate.
  - The window read orders by `ORDER BY (<order>), ALL` when an order is set, and by `ORDER BY ALL` otherwise. Check that DuckDB accepts `ALL` after an expression list. If it doesn't, order by the order followed by every column by position.
  - The order probe is `SELECT 1 ORDER BY <order>` with `condition_statement_count`'s method.
  - Column references come from `json_serialize_sql('SELECT 1 FROM t WHERE <condition>')`: walk the JSON for `COLUMN_REF` nodes and take the last element of `column_names`. Do the same for orders.
  - Literals: re-read the one row under the view with `OFFSET row LIMIT 1` and match the Arrow type. Floats use Rust's shortest round-trip `{}` formatting. Dates and timestamps are cast to VARCHAR and wrapped.
- [ ] **Step 4: Run the tests to see them pass.** Same command. Also run `cargo +1.95.0 test -p brightfield-engine --test rows_under_condition`. Expected: all pass.
- [ ] **Step 5: Commit.** `git add crates/brightfield-engine/src/lib.rs crates/brightfield-engine/tests/rows_under_view.rs`, run the pre-commit steps, then commit with the message "Read a step's rows under a condition and an order together".

### Task 2: The grid reads through the document's view

**Files:**
- Modify: `crates/brightfield-shell/src/app.rs` (`ChartDoc`, near `grid_cursor` :890)
- Modify: `crates/brightfield-shell/src/data_grid.rs` (`GridCache` :1247, `EngineRows::new` :1303, `fetch_page` :1179)
- Modify: `crates/brightfield-shell/src/window.rs` (`carries_unsaved_mark` :3078)
- Test: `crates/brightfield-shell/tests/data_grid.rs`

**Interfaces:**
- Consumes: Task 1's `RowsView`, `*_view` reads.
- Produces:
  - `ChartDoc.grid_view: RowsView`, which is the applied view.
  - `ChartDoc::apply_grid_view(&mut self, view: RowsView)`
  - `ChartDoc::grid_view_unrecorded(&self) -> bool`, true when the applied view is not empty and not yet recorded.
  - `ChartDoc::mark_grid_view_recorded(&mut self)`
  - `fetch_page(session, mark_index, window, audience, view: &RowsView)`
  - `GridCache` keyed on `(generation, mark, view)`

- [ ] **Step 1: Write the failing tests** in `tests/data_grid.rs`, using the kittest harness at :551 and `live_doc`.
  - `an_applied_condition_reads_only_the_rows_it_keeps`: apply `house_age = 28`. `grid_drawn().rows` equals the engine's `step_rows_count_where` for that condition on the same doc's session.
  - `changing_the_view_is_never_served_from_the_unfiltered_cache`: draw, apply the view, draw. The first visible row's `house_age` cell is `"28"`.
  - `clearing_the_view_restores_the_whole_table`.
  - `an_applied_view_raises_the_unsaved_mark`: this is a window test in `tests/chart_save_unsaved_mark.rs`. Apply a view and assert `carries_unsaved_mark()`.
- [ ] **Step 2: Run the tests to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test data_grid --test chart_save_unsaved_mark`.
- [ ] **Step 3: Implement.** Add the field, the cache key, and the view threaded through `EngineRows::new` and `fetch_page`. Extend `carries_unsaved_mark` to `(has_unsaved_edit() || grid_view_unrecorded()) && !front_door_is_live()`. Every other reader of `has_unsaved_edit()` that stands for "unsaved work" reads the widened predicate too: the OS window title push and `save_for_close`'s success test. The close question's body names an unrecorded view alongside the shelf's edits. Confirm with `tests/close_question_keeps_unsaved.rs`.
- [ ] **Step 4: Run the tests to see them pass.** Same command, plus `--test grid_cursor --test close_question_keeps_unsaved`.
- [ ] **Step 5: Commit** with the message "Hold a grid view on the document and read the grid through it".

### Task 3: The row at the foot and the `where` key

**Files:**
- Create: `crates/brightfield-shell/src/grid_foot.rs`, which holds the band's state and drawing. The state is `FootState`. The drawing is `fn foot_band(ui, doc: &mut ChartDoc, state: &mut FootState, mode) -> FootOutcome`.
- Modify: `crates/brightfield-shell/src/data_grid.rs` (`DataGridItem::ui` :1742). Carve `FOOT_BAND_HEIGHT` off the bottom of `ui.max_rect()` before `scope_builder`, following `carve_shelf_band` (window.rs:10078) for the shape.
- Modify: `crates/brightfield-keys/src/registry.rs`. Add the verbs `filter-rows-at-cursor` (`grid("w")`) and `sort-rows-at-cursor` (`grid("o")`), with tier, drives and scores in the shape at :909. Add both to `longname_snapshot_is_stable` (:1424) and to the Grid binding test (:1659).
- Modify: `crates/brightfield-shell/src/window.rs`. In `grid_cursor_keys` (:6025), dispatch the two verbs to `ChartDoc::ask_where_at_cursor()` and `ChartDoc::ask_order_at_cursor()`. Add a status entry beside `grid_cursor_status_entry` (:8533) with the id `"grid-foot"`.
- Test: `crates/brightfield-shell/tests/grid_foot.rs` (new), modelled on `tests/grid_cursor.rs`'s `Window`.

**Interfaces:**
- Consumes: Task 1's `cell_literal` and `CellLiteral`. Task 2's `apply_grid_view` and `grid_view`.
- Produces:
  - `pub enum FootField { Where, OrderBy }`
  - `ChartDoc::ask_where_at_cursor(&mut self) -> bool`
  - `ChartDoc::ask_order_at_cursor(&mut self) -> bool`
  - `ChartDoc::foot_asking(&self) -> Option<(FootField, &str)>`
  - `ChartDoc::foot_offer(&mut self, field: FootField, text: String)`, the entry point typed SQL will use later.
  - `pub const FOOT_BAND_HEIGHT: f32 = 26.0`
  - `pub const GRID_FOOT_STATUS_ID: &str = "grid-foot"`

- [ ] **Step 1: Write the failing tests** in `tests/grid_foot.rs`.
  - `w_on_a_cell_asks_where_from_that_cell`: `press_cell(r, house_age col)`, then `key(W)`. `foot_asking()` is `(Where, "house_age = <that cell>")`, and the band shows `where house_age = …`.
  - `a_comparison_key_replaces_the_sign_and_keeps_the_value`: after `w`, the keys `>`, `!`, `<`, `=` give `>`, `!=`, `<`, `=`, and the status entry reads `over`, `is not`, `under`, `is`.
  - `another_key_replaces_the_value`: after `w`, typing `4` `0` gives `house_age = 40`.
  - `the_grid_follows_each_condition_that_parses`: after `w` and `>`, `drawn().rows` equals the engine count for `house_age > <v>`.
  - `enter_applies_and_marks_the_view_unrecorded`: the count holds, `carries_unsaved_mark()` is true, and the band's not-recorded dot is drawn. Assert through a label or accessor; don't diff pixels.
  - `escape_restores_the_field_before_editing`.
  - `an_emptied_field_and_enter_shows_the_whole_table`.
  - `a_refused_condition_keeps_the_rows_and_says_why`: covers `nosuch > 1` and `house_age > 1; select 1`. The rows are unchanged and the status entry carries DuckDB's message.
  - `a_null_cell_asks_is_null`: use a CSV fixture with an empty cell. Add `tests/data/with_nulls.csv`, three rows.
  - `no_row_cursor_is_drawn_while_asking`: `drawn().cursor.ring` is None while the field has focus.
  - `the_cursor_lands_on_the_first_row_after_enter`, in the same column.
  - `the_band_is_26_high_and_holds_the_count`: the band rect's height is 26, and its trailing label is `"{rows} rows · {cols} columns"`.
- [ ] **Step 2: Run the tests to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test grid_foot` and `cargo +1.95.0 test -p brightfield-keys`.
- [ ] **Step 3: Implement.**
  - The field is brightfield's own `TextEdit`, with no prompt glyph, starting with its word.
  - The prefilled column is written bare when it matches `^[a-z_][a-z0-9_]*$` and is not a DuckDB keyword (from `duckdb_keywords()` through the engine), and with `sql_ident::quote` otherwise, so the offered condition always parses and `house_age` still reads bare. Test it with a column named `order` or `median income` in a CSV fixture.
  - While the field has focus, `grid_cursor_keys` must not consume keys. It is already gated on `egui_wants_keyboard_input`; confirm this in a test.
  - Use `foot_offer` so the key is not the only way in.
- [ ] **Step 4: Run the tests to see them pass.** Run the same commands plus `--test grid_cursor --test rail_entry_ids --test status_rail`. Redraw any moved baseline: `data_grid` and `dashboard_baseline` will move, because the band takes 26 from the table.
- [ ] **Step 5: Commit** with the message "Ask a condition in a row at the foot of the grid".

### Task 4: `order by` in the foot band

**Files:**
- Modify: `crates/brightfield-shell/src/grid_foot.rs`, `app.rs`
- Test: `crates/brightfield-shell/tests/grid_foot.rs`

**Interfaces:**
- Consumes: Task 3's `FootField::OrderBy`, `ask_order_at_cursor`. Task 1's ordered reads.
- Produces: `RowsView.order` is set by the band.

- [ ] **Step 1: Write the failing tests.**
  - `o_on_a_column_opens_order_by_that_column_with_the_caret_after_it`: after `o`, `foot_asking()` is `(OrderBy, "median_income")`.
  - `typing_desc_and_enter_sorts_the_grid`: the first visible row's value is the column's maximum, from the engine.
  - `a_condition_and_an_order_hold_together`: `where house_age = 28` plus `order by median_income` keeps the filtered count and the order.
  - `a_refused_order_keeps_the_rows`: `median_income; select 1`.
  - `an_emptied_order_shows_the_natural_order`.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test grid_foot`.
- [ ] **Step 3: Implement** on Task 3's field machinery. There are no comparison keys in this field.
- [ ] **Step 4: Run them to see them pass.** Same command.
- [ ] **Step 5: Commit** with the message "Order the grid from the foot band".

### Task 5: arc through the runner, with arguments

**Files:**
- Modify: `crates/brightfield-shell/src/run.rs` (next to `run_to_completion` :311)
- Create: `crates/brightfield-shell/src/operation.rs`, which holds the description types and the describe and record calls.
- Test: `crates/brightfield-shell/tests/operation_runner.rs` (new). It uses `Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))`.

**Interfaces:**
- Produces:
  - `pub struct ArcOutput { pub stdout: String, pub stderr: String, pub succeeded: bool }`
  - `pub fn arc_with_args(runner: &Runner, dir: &Path, args: &[&str]) -> ArcOutput`. It uses the same environment as `run_to_completion`, keeps stdout and stderr apart, and strips ANSI from stderr.
  - `pub struct OperationDescription { pub long_name: String, pub summary: String, pub reads: Vec<Role>, pub parameters: Vec<Parameter> }`
  - `pub struct Role { pub name: String, pub kind: String, pub required: bool, pub description: String }`
  - `pub struct Parameter { pub key: String, pub description: String, pub offered: Vec<Offered> }`, where `offered` comes from `x-comparisons` or `x-directions`.
  - `pub struct Offered { pub word: String, pub sign: String }`
  - `pub fn describe(runner: &Runner, long_name: &str) -> Result<OperationDescription, String>`. The `Err` is arc's stderr, without its `error: ` prefix.
  - `pub fn list_operations(runner: &Runner) -> Result<Vec<String>, String>`
  - `pub fn record(runner: &Runner, dir: &Path, operation: &str, on: &str, name: &str, args: &[(&str, &str)]) -> Result<String, String>`. `Ok` is arc's stdout line.
  - `pub struct Descriptions`, a session cache: `fn get(&mut self, runner: &Runner, long_name: &str) -> Result<&OperationDescription, String>`

- [ ] **Step 1: Write the failing tests.**
  - `the_runner_lists_both_operations`: the list contains `filter-rows` and `sort-rows`.
  - `describing_filter_rows_yields_where_and_its_six_comparisons`: the reads are `["table", "column"]`, the parameter is `where`, and the offered words are `is`, `is not`, `over`, `under`, `between`, `is null`.
  - `describing_sort_rows_yields_order_by_and_two_directions`.
  - `an_unknown_operation_is_refused_in_arcs_words`: the `Err` starts with `no operation or operator called`.
  - `record_appends_a_generated_step_and_runs_nothing`: in a temp dir holding the housing Parquet, write the one-step Protocol with `OneStepProtocol::save_to` (as `tests/run_control.rs` does), then record `filter-rows --on california_housing --name california_housing_filtered_by_house_age --arg where=house_age > 28`. `arcform.yaml` has two steps, the new model's first line is `-- generated: filter-rows on california_housing`, and `build/` doesn't exist.
  - `record_refuses_a_step_as_on`: `--on load` gives an `Err` containing `no step of the protocol makes a table called`, and the folder is unchanged byte for byte.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test operation_runner`.
- [ ] **Step 3: Implement.**
  - `serve_as_runner_if_asked` already hands argv to `arc::cli_main`, so `arc_with_args` passes `args` in place of `["run"]`.
  - `run_to_completion` keeps its behaviour. If it is cheap, make it a call of a shared spawn helper.
- [ ] **Step 4: Run them to see them pass.** Same command, plus `--test run_control`.
- [ ] **Step 5: Commit** with the message "Describe and record operations through the runner".

### Task 6: The operation panel in the Inspector rail

**Files:**
- Modify: `crates/brightfield-workbench/src/arrangement.rs:546`. Add `ItemId::new("operation-inspector")` to `INSPECTOR_PANES`.
- Create: `crates/brightfield-shell/src/operation_panel.rs`, which holds `pub struct OperationPane` implementing `Item<ChartDoc>` (trait at `brightfield-workbench/src/item.rs:326`).
- Modify: `crates/brightfield-shell/src/app.rs` (`chart_registry_with` ~:3924). Register its `ItemSpec`, with no toggle verb.
- Modify: `crates/brightfield-shell/src/window.rs`. When `ask_where_at_cursor` or `ask_order_at_cursor` succeeds, open `INSPECTOR_RAIL` and set `inspector_panel` to the new pane's index, remembering in `opened_rail_for_ask: bool` whether it was collapsed. On Esc from the field, collapse it again if it was.
- Test: `crates/brightfield-shell/tests/operation_panel.rs` (new), using the `Window` pattern. Also update `tests/inspector_contract.rs` and `tests/arrangement.rs` for the third pane.

**Interfaces:**
- Consumes: Task 5's `Descriptions` and `OperationDescription`. Task 3's `foot_asking`.
- Produces: `ChartDoc::operation_in_hand(&self) -> Option<OperationInHand>`, with `pub struct OperationInHand { pub long_name: &'static str, pub table: String, pub table_from: ValueFrom, pub column: Option<String>, pub column_from: ValueFrom, pub value: Option<String> }` and `pub enum ValueFrom { NodeInView, Cursor, Asked }`.

- [ ] **Step 1: Write the failing tests.**
  - `w_opens_the_inspector_on_the_operation_panel`: on a data file the rail starts collapsed (`rail_is_collapsed(INSPECTOR_RAIL)`). After `w` it is open, on the operation pane.
  - `escape_shuts_the_rail_it_opened`.
  - `escape_leaves_an_already_open_rail_open`.
  - `the_panel_lists_table_column_and_where_with_arcs_sentences`: the labels include `california_housing` with "the node in view", `house_age` with "the cursor", the `where` value, the six comparison words, and arc's description sentence for `where`. Read the sentence from `describe`; don't hard-code it.
  - `with_no_runner_the_row_still_asks_and_the_panel_says_why`: `running_with(None)`. `w` still asks, and the panel shows a label naming the missing description.
  - `the_panel_shows_sort_rows_for_order_by`.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test operation_panel`.
- [ ] **Step 3: Implement.** The panel draws from `OperationDescription` and `OperationInHand`. The words for where a value came from are brightfield's: "the node in view", "the cursor", "asked". The panel changes no value.
- [ ] **Step 4: Run them to see them pass.** Same command, plus `--test inspector_contract --test arrangement --test rail_entry_ids`. Redraw any moved baseline.
- [ ] **Step 5: Commit** with the message "Explain the operation in hand in the Inspector rail".

### Task 7: Recognising generated steps, and naming new ones

**Files:**
- Create: `crates/brightfield-shell/src/generated.rs`
- Test: unit tests in `generated.rs`, plus `crates/brightfield-shell/tests/generated_steps.rs` for folder-level cases.

**Interfaces:**
- Produces:
  - `pub enum KnownOperation { FilterRows, SortRows }`, with `long_name(&self) -> &'static str`, `word(&self) -> &'static str` (`"filtered"`, `"sorted"`), `argument_key(&self) -> &'static str` (`"where"`, `"order_by"`), and `from_long_name(&str) -> Option<Self>`.
  - `pub struct GeneratedStep { pub name: String, pub operation: KnownOperation, pub on: String, pub argument: String }`
  - `pub fn parse_generated(step_name: &str, model_sql: &str) -> Result<GeneratedStep, String>`. The `Err` names the step and the first point of mismatch.
  - `pub struct GeneratedProtocol { pub data: PathBuf, pub dir: PathBuf, pub root_table: String, pub steps: Vec<GeneratedStep> }`. `data` is resolved as `data_file_named_by` resolves the load step's `depends_on`, and `root_table` is the table the load model makes.
  - `pub fn generated_protocol_in(manifest_text: &str, dir: &Path) -> Result<Option<GeneratedProtocol>, String>`. `Ok(None)` means the folder is not this shape, so the caller falls through as today. `Err` means the folder has generated headers that don't match, and the caller refuses with that message.
  - `pub fn step_name_for(operation: KnownOperation, root_table: &str, columns: &[String], taken: &BTreeSet<String>) -> String`

- [ ] **Step 1: Write the failing tests.**
  - `parses_arcs_filter_template`: the exact text arc writes, `-- generated: filter-rows on california_housing\nCREATE OR REPLACE TABLE "x" AS\nSELECT *\nFROM california_housing\nWHERE house_age > 28;\n`, gives `on = california_housing` and `argument = house_age > 28`.
  - `parses_arcs_sort_template`.
  - `parses_a_model_arc_actually_wrote`: record through Task 5's `record`, then parse the file from disk. This guards against drift from arc's template.
  - `a_hand_edited_body_under_the_header_is_refused`: header kept, `SELECT *` replaced with `SELECT house_age`. The `Err` names the step.
  - `a_quoted_name_that_is_not_the_steps_own_is_refused`.
  - `an_unknown_operation_in_the_header_is_refused`.
  - `a_load_plus_generated_folder_is_recognised`: a chain of filter then sort, the sort `on` the filter's table.
  - `a_generated_step_on_a_table_no_earlier_step_makes_is_refused`.
  - `a_one_step_folder_is_not_this_shape`, which gives `Ok(None)`.
  - `names_carry_the_one_column_the_condition_names`, which gives `california_housing_filtered_by_house_age`.
  - `names_fall_back_without_a_single_column`: zero and two columns both give `california_housing_filtered`.
  - `a_column_arc_would_refuse_in_a_name_is_slugged`: the column is turned into a slug before it goes in the name (characters outside `[A-Za-z0-9_]` become `_`, runs of `_` collapse, leading and trailing `_` are trimmed), so `#rooms` gives `california_housing_filtered_by_rooms` and ` price ` gives `…_by_price`. A column with nothing left after slugging falls back to `<root table>_<word>`. The root table is slugged the same way.
  - `a_taken_name_gets_a_suffix`: with `{x, x_2}` taken, the result is `x_3`.
  - `a_sort_names_its_first_column`, which gives `california_housing_sorted_by_median_income`.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --lib generated` and `--test generated_steps`.
- [ ] **Step 3: Implement.** Parse by matching arc's template line by line: header, `CREATE OR REPLACE TABLE "<name>" AS`, `SELECT *`, `FROM <on>`, `WHERE <arg>;` or `ORDER BY <arg>;`. `<on>` may be bare or double-quoted, the way arc's `from_ident` writes it. The argument runs to the final `;` and may span lines.
- [ ] **Step 4: Run them to see them pass.**
- [ ] **Step 5: Commit** with the message "Recognise steps arc generated, and name new ones by their effect".

### Task 8: Save records applied views through arc

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (`save_protocol` :7585, `run_protocol` :~7700)
- Modify: `crates/brightfield-shell/src/one_step.rs` (`save_to` :305)
- Delete: `record_grid_filter`, `amend_recorded_filter`, `GridFilter`, `Promotion`, `AmendOutcome` in `crates/brightfield-protocol/src/record.rs`, the re-export at `lib.rs:79`, and `tests/record_from_exploration.rs`. First grep for any other users and stop if there are some.
- Test: `crates/brightfield-shell/tests/save_records_operation.rs` (new)

**Interfaces:**
- Consumes: Task 5's `record`. Task 7's `step_name_for`, `generated_protocol_in`. Task 1's `condition_columns`, `order_columns`. Task 2's `grid_view`, `mark_grid_view_recorded`.
- Produces:
  - `OneStepProtocol::save_to` writes the manifest only when the folder's manifest is absent or still one step. Otherwise it leaves `arcform.yaml` alone and still writes `models/load.sql`.
  - `MeridianApp::record_applied_views(&mut self) -> Result<Vec<String>, String>`, which returns the recorded step names.

- [ ] **Step 1: Write the failing tests.**
  - `save_records_the_filter_as_a_named_generated_step`: apply `house_age > 28`, then call `save_protocol`. `arcform.yaml` has the step `california_housing_filtered_by_house_age`, its model starts with `-- generated: filter-rows on california_housing`, and `carries_unsaved_mark()` is false.
  - `save_records_filter_then_sort_on_the_filters_table`: the second step is `california_housing_sorted_by_median_income`, its header says `on california_housing_filtered_by_house_age`.
  - `a_second_save_records_nothing_and_erases_nothing`: the files are byte-identical after the second Save.
  - `run_keeps_the_generated_steps`: Save, then `run_protocol` and poll to completion. The run succeeds and both generated models are still on disk.
  - `a_refusal_is_shown_in_arcs_words_and_keeps_the_mark`: make the folder read-only, apply, Save. The banner body comes from arc's stderr, and the view stays applied and unrecorded.
  - `after_save_the_window_shows_the_protocols_steps`: the ledger rail is open, the sheet has two rows, and the newest step is selected.
  - `a_view_leaves_charts_on_the_load`: the dashboard's composed data source and `save_chart_beside`'s chart file name the load's table before and after Save.
- [ ] **Step 2: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-shell --test save_records_operation`.
- [ ] **Step 3: Implement.**
  - `save_protocol` and `run_protocol` call `save_to`, then `record_applied_views`, filter first. Each gets `--on` from the table the node in front makes, and its name from `step_name_for` with the manifest's current step names as `taken`.
  - Stop at the first refusal. A partial Save (filter recorded, sort refused) keeps the grid as the analyst saw it: the condition stays shown but reads as recorded (no dot), the order stays applied and unrecorded, and the document remembers the table the order must go `--on` (the filter's new table). The next Save records only the unrecorded part, on that table. The window adopts the Protocol only once nothing is left unrecorded. While a recorded condition is held this way, the `where` field is read-only and the status band says the condition is already recorded, so the grid always equals the recorded filter plus the pending order; emptying the order field and pressing Enter leaves nothing unrecorded, and the window adopts the Protocol. Refine Task 2's `mark_grid_view_recorded` into per-part recording for this. Test it: `a_partial_save_keeps_the_grid_and_records_the_rest_on_the_filters_table` and `after_a_partial_save_the_recorded_condition_cannot_be_edited`.
  - `save_protocol` reports failure when a recording is refused, so `save_for_close` does not close the window over an unrecorded view. Test it: `closing_after_a_refused_recording_keeps_the_window`.
  - On success, adopt the result through Task 9's open path. Until Task 9 lands, adopt through `load_protocol_offline` and mark the test `#[ignore]` with the reason. Task 9 removes the `#[ignore]`.
- [ ] **Step 4: Run them to see them pass.** Same command, plus `--test one_step_protocol --test run_control --test chart_save_unsaved_mark`, and `cargo +1.95.0 test -p brightfield-protocol`.
- [ ] **Step 5: Commit** with the message "Record applied views through arc on Save".

### Task 9: Reopening, the labelled first-page preview, and the band on generated steps

**Files:**
- Modify: `crates/brightfield-engine/src/lib.rs`. Add the preview read on its own connection.
- Modify: `crates/brightfield-shell/src/window.rs`. In `Boot::open_sampled` near :1474, check `generated_protocol_in` before the offline gate. Show the preview in the grid of a never-run generated step. Make the band read-only there and have `w` offer Run.
- Modify: `crates/brightfield-shell/src/protocol.rs`. Build `ProtocolInputs` for the generated shape so that `adopt_run` (:274) matches its step set. Add the third exemption to the doc at :321.
- Modify: `crates/brightfield-shell/src/data_grid.rs`. Draw the preview label and leave the count out.
- Test: `crates/brightfield-shell/tests/generated_reopen.rs` (new), and `crates/brightfield-engine/tests/preview_read.rs` (new).

**Interfaces:**
- Consumes: Task 7's `GeneratedProtocol` and `GeneratedStep`.
- Produces:
  - `pub fn preview_first_page(data: &Path, reader: FileReader, root_table: &str, chain: &[(String, PreviewOp)], limit: u64) -> Result<Vec<RecordBatch>, PreviewRefusal>`, in `brightfield-engine`, with `pub enum PreviewOp { Where(String), OrderBy(String) }` and `pub enum PreviewRefusal { TooLarge { bytes: u64 }, Engine(String) }`.
  - `pub const PREVIEW_FILE_LIMIT_BYTES: u64 = 256 * 1024 * 1024`.

- [ ] **Step 1: Write the failing engine tests** in `preview_read.rs`.
  - `a_filter_previews_its_first_page`: every `house_age` on the page is > 28, and there are at most `limit` rows.
  - `a_sort_previews_in_its_own_order`: a `sort-rows` step's page is ordered by its `order_by`.
  - `a_chain_previews_through_its_chain`.
  - `a_condition_that_reads_another_file_fails`: `house_age > (SELECT count(*) FROM read_csv('/etc/hosts'))` gives `Engine(msg)`, and `msg` mentions external access.
  - `a_file_over_the_limit_is_not_read`: call a test-only `preview_first_page_with_limit` with a 1-byte limit. It gives `TooLarge`.
  - `a_path_holding_a_quote_still_previews`: the data file sits in a folder whose name holds `'`.
- [ ] **Step 2: Write the failing window tests** in `generated_reopen.rs`. Build each folder by Save through the window or by Task 5's `record`.
  - `a_load_plus_generated_folder_opens_as_a_protocol_without_the_offline_variable`: two steps, the ledger open, and the generated step `not run`.
  - `opening_starts_no_run`: no new file appears under `build/.arcform/runs`.
  - `a_never_run_steps_grid_shows_a_labelled_first_page`: the preview label is shown, the rows satisfy the condition, no total count is shown, and the grid doesn't scroll past the first page.
  - `a_hand_edited_generated_model_is_refused_naming_the_step`.
  - `w_on_a_generated_step_offers_run`: the status entry offers Run, and `foot_asking()` is None.
  - `after_a_run_the_step_reads_as_run_and_still_shows_the_labelled_preview`.
  - `a_step_over_the_limit_says_why_and_offers_run`: use the test-only limit hook.
- [ ] **Step 3: Run them to see them fail.** Run `cargo +1.95.0 test -p brightfield-engine --test preview_read` and `cargo +1.95.0 test -p brightfield-shell --test generated_reopen`.
- [ ] **Step 4: Implement.**
  - The preview opens `Connection::open_in_memory()` and checks the size limit before reading.
  - It sets `memory_limit` (the builder's call: 2 GB) and disables spilling to disk, then runs `CREATE TABLE <root> AS SELECT * FROM <reader>('<path>')` with `'` in the path doubled, then `SET enable_external_access = false`, then `SET lock_configuration = true`. An out-of-memory failure is `TooLarge`, which offers Run, not an engine error.
  - It then creates each chain step as a table and reads the last one's first page. A sort orders by `ORDER BY <order_by>, 1, 2, …, n` (the step's columns by position, because DuckDB refuses `, ALL` after another key), built the way the engine's `window_view_sql` builds it, with the order on lines of its own so a trailing `--` comment cannot hide the tie-break or the LIMIT; reuse that builder if it fits. Test it: `a_sort_whose_order_ends_in_a_comment_still_previews_one_page`; otherwise `ORDER BY ALL`.
- [ ] **Step 5: Run them to see them pass.** Same commands, plus `--test save_records_operation` with Task 8's `#[ignore]` removed, and `--test one_step_protocol --test reload_data_file --test protocol_contract`.
- [ ] **Step 6: Commit** with the message "Reopen a folder of generated steps with a labelled preview".

### Task 10: The driven test, and folding the design into the docs

**Files:**
- Create: `crates/brightfield-shell/tests/operation_from_the_cursor.rs`
- Modify: the module docs of `grid_foot.rs`, `operation.rs`, `operation_panel.rs`, `generated.rs`, the `protocol.rs` exemption section, and `one_step.rs` (Save no longer writes the whole manifest).
- Delete: `docs/superpowers/specs/2026-10-15-operation-from-the-cursor-design.md` and this plan.

- [ ] **Step 1: Write the driven tests.**
  - `filter_from_the_cursor_end_to_end`: the spec's eight steps in order, on the housing start written to a temp dir as `run_control.rs` does it, with `Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))`.
  - `order_from_the_cursor_end_to_end`: the same with `o`, `median_income DESC`.
- [ ] **Step 2: Run them.** Run `cargo +1.95.0 test -p brightfield-shell --test operation_from_the_cursor`. Expected: pass. Tasks 1 to 9 built every piece, so a failure here is a defect to fix in the owning task's code.
- [ ] **Step 3: Fold the spec.** Write the outcome into the module docs listed above, in plain words. Then delete the spec and this plan.
- [ ] **Step 4: Commit** with the message "Drive the operation from the cursor end to end, and fold its design into the module docs".
- [ ] **Step 5: Ready the branch.** Follow the build loop's "Before you call a branch ready": roborev's branch review, `gh pr create`, and `gh pr checks <pr> --watch`. Put the CI test totals and the run's rulings in the PR description.
