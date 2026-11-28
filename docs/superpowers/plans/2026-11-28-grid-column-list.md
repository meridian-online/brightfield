# The grid's column list, chart rows and sortable fields: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The grid shows every table, the opening one and any a later step makes, the same way: rows, a brushable one-line column list, or fixed chart rows, with a compact numbers receipt, a pane-wide scale switch, click-sortable fields, FineType glyphs, and the list beside the rows when the pane is wide.

**Architecture:** A ran step's table is read from the Protocol's last run as a Parquet snapshot, profiled with the opening file's type source, and given its own in-memory dashboard through the existing `Dashboard::of`, which already takes a file path; the grid reads that live session for rows and tiles, so every table goes through one path. Each column's one tile is drawn at two looks (a bare sparkline, a full chart row), with the value strip and quartile ladder drawn by the grid from the whole-table profile under the tile. Layout, sort and scale are per-document view state on the workspace's `Recent` row.

**Tech Stack:** Rust, egui/eframe, DuckDB, Brightfield's Mosaic-style spec and `LiveDashboard`, `meridian-egui` Tabler icons.

**Spec:** `docs/superpowers/specs/2026-11-28-grid-column-list-design.md` (same branch). Read it first; this plan argues from it.

## Global Constraints

- Public repository: no planning identifiers (issue refs, milestone ids) in code, comments, commits, branch names or PR text.
- Before each commit: `cargo xtask ci --step fmt` and `cargo xtask ci --step clippy`, and the tests the change touches. After each commit: `cargo xtask ci --step comment-citations`.
- After each commit, `roborev wait <sha>`; fold its findings into the task's fix round.
- One cargo build at a time per session; background a build that may wait on a slot.
- Pixel baselines are redrawn only on the Mac Studio, never from a hosted run.
- Statistics are whole-table; nothing is computed per brush except what the tiles already query.
- Nothing writes a chart, spec or layout into the Protocol folder.
- Copy fixed by the spec: switch words `rows`, `list`, `charts`; scale words `auto`, `linear`, `log`, `symlog`; the log note `log x`; the unrun reason `Run the step to brush its columns`.
- Numbers fixed by the spec: list line about 30 pt; side list from a pane width of about 900 pt, about 340 pt wide; auto picks log when skew > 3 and min > 0.

## Review Focus

1. A table with hundreds of columns: a brush must query only the tiles on screen (Task 8 pins it).
2. A column of all nulls, or of one value: its line and row draw with an empty chart place and the generator's reason, no panic (Task 8).
3. `log` or `auto` on a column with zeros or negatives: linear, with the fallback said on the row (Task 5).
4. A ran step whose run predates an edit upstream: its table is brushed and the pane names its state; a step never run keeps its 128-row preview with the brush off (Tasks 3 and 11).
5. A layout file written before this change (`grid_layout: columns`) opens on `charts`, and one with no sort reads source order (Task 4).

---

### Task 1: The profile carries the receipt's statistics

**Files:**
- Modify: `crates/brightfield-engine/src/profile.rs` (`ColumnMoments`, `ColumnProfile`)
- Modify: `crates/brightfield-engine/src/lib.rs:4228-4360` (`aggregate_source` and its reader)
- Test: unit tests beside `the_profile_carries_the_upper_quartile` in `crates/brightfield-engine/src/lib.rs`

**Interfaces:**
- Produces on `ColumnMoments`: `p5: Option<f64>`, `lower_quartile: Option<f64>`, `p95: Option<f64>`, `skew: Option<f64>` (DuckDB `skewness`), beside the existing `upper_quartile`.
- Produces on `ColumnProfile`: `temporal: Option<TemporalQuantiles>` with `pub struct TemporalQuantiles { pub p5: String, pub p25: String, pub median: String, pub p75: String, pub p95: String }` (values as DuckDB renders them, via `quantile_disc`), `Some` for DATE/TIMESTAMP columns with a non-null row; and `top: Option<(String, u64)>`, the most frequent value and its count (`mode` and a count), `Some` for VARCHAR and other non-numeric, non-temporal columns.
- All added to the one aggregate SELECT, so no second scan.

- [ ] **Step 1: Write the failing tests**
  - `the_profile_carries_p5_p25_p95_and_skew`: over `range(1, 101)`, `p5 == Some(5.95)`, `lower_quartile == Some(25.75)`, `p95 == Some(95.05)`, `skew` within 1e-9 of 0.0.
  - `a_skewed_column_reports_positive_skew`: values `[1,1,1,1,100]`, `skew > 1.0`.
  - `a_date_column_carries_its_quartiles_as_dates`: dates 2024-01-01..2024-12-31 daily, `temporal.median == "2024-07-01"` (or the exact `quantile_disc` answer, asserted literally once measured), `moments.is_none()`.
  - `a_text_column_carries_its_most_frequent_value`: `['a','b','b']` → `top == Some(("b".into(), 2))`.
- [ ] **Step 2:** `cargo test -p brightfield-engine the_profile_carries a_skewed_column a_date_column_carries a_text_column_carries` — expect FAIL (no such fields).
- [ ] **Step 3:** Add the fields and the SELECT terms (`quantile_cont(q, [0.05, 0.25, 0.95])` or three terms, `skewness(q)`, `quantile_disc(q, [...])::VARCHAR[]` for temporal, `mode(q)` and `count(*) FILTER (WHERE q = mode)` via a subquery-free form). Update every struct literal the compiler names.
- [ ] **Step 4:** Same command — expect PASS. Then `cargo test -p brightfield-engine` — expect PASS.
- [ ] **Step 5: Commit** "The column profile carries p5, the quartiles, p95, skew, temporal quartiles and the most frequent value".

### Task 2: One column order, sortable by any field

**Files:**
- Create: `crates/brightfield-shell/src/column_order.rs` (pure, no egui)
- Modify: `crates/brightfield-workbench/src/persist.rs` (`GridSort`, `SortField`)
- Test: unit tests in `column_order.rs`

**Interfaces:**
- Produces in `brightfield_workbench::persist`: `#[serde(rename_all = "lowercase")] pub enum SortField { Index, Name, Valid, Median, Range }` and `pub struct GridSort { pub field: SortField, pub descending: bool }` with `Default` = `{ Index, false }`.
- Produces in `column_order.rs`: `pub fn ordered(columns: &[ColumnProfile], sort: GridSort) -> Vec<usize>` (indices into `columns`) and `pub fn clicked(current: GridSort, field: SortField) -> GridSort` (same field flips `descending`; a new field starts ascending).
- Rule (spec, "Calls made"): `Median` and `Range` group by kind (numeric, then temporal, then text), sort within a kind by median value / range minimum (text by `top` value), and put columns with no value last in either direction. `Valid` sorts by `non_null / (non_null + nulls)`. `Name` is case-insensitive. Ties keep source order. `Index` ascending is source order.

- [ ] **Step 1: Write the failing tests**: `index_ascending_is_source_order`, `a_second_click_reverses`, `a_new_field_starts_ascending`, `median_groups_numbers_then_dates_then_text`, `columns_with_no_median_sort_last_both_ways`, `valid_sorts_by_non_null_share`, `ties_keep_source_order`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --lib column_order` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same command — expect PASS.
- [ ] **Step 5: Commit** "The grid's columns can be ordered by any of their fields".

### Task 3: A ran step's table opens in the grid as a table of its own

Depends on the separate piece of work that reads any node's rows from the Protocol's last run (`node_rows::rows_from_run`, producing `NodeRows`), which must be on `main` first; rebase onto it before starting.

**Files:**
- Create: `crates/brightfield-shell/src/table_session.rs`
- Modify: `crates/brightfield-shell/src/window.rs:8238-8270` (the grid's choice of source), `:18345-18393` (`preview_request`, which now serves only steps with no run table)
- Test: `crates/brightfield-shell/tests/grid_any_table.rs`

**Interfaces:**
- Consumes: `node_rows::rows_from_run(dir, tables, into) -> Result<NodeRows, String>`; `Dashboard::of(path, columns)`; `LiveDashboard::load_str`; `LoadOptions::type_source` (the opening file's type source, so a step's table is FineType-labelled the same way; this replaces the spec's label-inheritance call, which was a fallback for a table with no type source).
- Produces: `pub struct TableSessions` (kept on the window, at most 8 entries, each keyed by table name + run content hash + steps, as `ColumnProfiles` keys them) with `pub fn session_for(&mut self, key: &ProfileKey, snapshot: &Path) -> Result<&mut LiveDashboard, String>` and `pub fn drop_all(&mut self)`. A ran step's grid reads rows and tiles from its entry; `PreviewRequest` remains only for a step whose table the run did not make (`NodeRows::NotReached`).
- Stale state: when the run predates an edit upstream, the pane header carries the spine's state words for the step beside the table name.

- [ ] **Step 1: Write the failing tests** (fixture: the one-step Protocol fixture with a `filter-rows` step, run once):
  - `a_ran_steps_grid_scrolls_its_whole_table`: the grid's row count equals the step table's `count(*)`, above `PREVIEW_ROWS`.
  - `a_ran_steps_table_has_a_tile_per_column`: the grid's columns layout draws one row per column of the step's table.
  - `a_brush_on_a_step_tables_tile_narrows_its_rows`.
  - `a_step_the_run_did_not_make_keeps_its_preview`: `PREVIEW_LABEL` shown, no count claimed.
  - `a_step_tables_columns_carry_finetype_labels`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_any_table` — expect FAIL. (Several minutes: an end-to-end target.)
- [ ] **Step 3:** Implement; profile the snapshot through `Session` load with the type source, build `Dashboard::of(&snapshot_path, &profiles)`, load it, cache it.
- [ ] **Step 4:** Same command — expect PASS; then `cargo test -p brightfield-shell --test grid_transposed_layout --test canvas_pane_group --test one_step_protocol` — expect PASS.
- [ ] **Step 5: Commit** "A ran step's table opens in the grid with its own rows and charts".

### Task 4: The switch reads rows · list · charts, and the sort and scale are saved per document

**Files:**
- Modify: `crates/brightfield-workbench/src/persist.rs:313-345` (`GridLayout`), `:418-433` (`Recent`)
- Modify: `crates/brightfield-shell/src/window.rs` (`record_layout_switch` ~`:23031`, `draw_layout_switch`, the layout getter/setter ~`:6129-6171`, restore on open)
- Test: persist unit tests; `crates/brightfield-shell/tests/grid_transposed_layout.rs`

**Interfaces:**
- Produces: `enum GridLayout { Rows, List, Charts }`, `#[serde(alias = "columns")]` on `Charts`; `word()` gives `rows`, `list`, `charts`; `other()` is replaced by `pub const ALL: [GridLayout; 3]` and `pub fn is_columns(self) -> bool` (List or Charts). `Recent` gains `#[serde(default)] grid_sort: GridSort` and `#[serde(default)] grid_scale: GridScale` (`enum GridScale { Auto, Linear, Log, Symlog }`, default `Auto`, lowercase).
- Throwing the switch from `rows` to the columns view on a document with no saved columns mode lands on `list` (the three-word switch makes this the plain click on `list`; the default on reopen is what was saved).

- [ ] **Step 1: Write the failing tests**: `a_saved_columns_layout_opens_on_charts`, `a_layout_file_with_no_sort_reads_source_order`, `the_switch_offers_rows_list_charts` (reads the three words off the band), `each_document_keeps_its_own_sort_and_scale`.
- [ ] **Step 2:** `cargo test -p brightfield-workbench persist && cargo test -p brightfield-shell --test grid_transposed_layout` — expect FAIL.
- [ ] **Step 3:** Implement. Until Tasks 8 and 9 land, `List` and `Charts` both draw today's transposed rows.
- [ ] **Step 4:** Same commands — expect PASS.
- [ ] **Step 5: Commit** "The grid's switch reads rows, list and charts, and each document keeps its sort and scale".

### Task 5: One scale switch for the pane

**Files:**
- Create: `crates/brightfield-shell/src/grid_scale.rs`
- Modify: `crates/brightfield-shell/src/chart_item.rs:1587-1720`, `:2227` (no per-tile scale switch on a tile drawn in the grid's columns view); `window.rs` (the switch on the pane header in List and Charts)
- Test: unit tests in `grid_scale.rs`; `crates/brightfield-shell/tests/tile_scale_switch.rs` updated

**Interfaces:**
- Produces: `pub struct Resolved { pub scale: ScaleType, pub note: Option<&'static str> }` and `pub fn resolve(chosen: GridScale, moments: Option<&ColumnMoments>) -> Resolved`. `Auto`: `Log` when `skew > 3.0 && min > 0.0`, note `"log x"`; else `Linear`. `Log`: `Log` when `min > 0.0`, else `Linear` with note `"linear: has zeros or negatives"`. `Symlog`, `Linear` as chosen. No moments → `Linear`, no note.
- The grid applies the resolved scale to each column's tile as its x scale when composing it for the grid.

- [ ] **Step 1: Write the failing tests**: `auto_picks_log_past_a_skew_of_three`, `auto_keeps_a_skewed_column_with_a_zero_linear`, `log_falls_back_on_a_negative_and_says_so`, `the_pane_header_carries_one_scale_switch_in_charts` (e2e, reads `auto`, `linear`, `log`, `symlog` once each), `no_transposed_tile_draws_its_own_scale_switch`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --lib grid_scale && cargo test -p brightfield-shell --test tile_scale_switch` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "The grid's columns view has one scale switch, and auto puts a skewed column on a log axis".

### Task 6: The numbers block is a compact receipt

**Files:**
- Create: `crates/brightfield-shell/src/receipt.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (`draw_row_summaries` ~`:22820` calls the receipt instead of `draw_column_band` at `GridDensity::Row`)
- Test: unit tests in `receipt.rs`

**Interfaces:**
- Produces: `pub fn receipt_lines(profile: &ColumnProfile) -> Vec<(&'static str, String)>` in the spec's order (min, max, p5, p95, p25, p75, median, mean, sd, distinct, valid) for a numeric column; (min, max, p5, p95, p25, p75, median, distinct, valid) for a temporal one; (distinct, top, valid) for text. `pub fn range_text(profile: &ColumnProfile) -> (String, String)` printing a temporal range at the resolution its span fits (span over 2 days: dates; over 2 hours: date and minutes; else full). `pub fn draw_receipt(painter: &egui::Painter, rect: egui::Rect, profile: &ColumnProfile, glyph: Glyph, mode: Mode)`, two columns, labels left of values, Inter with tabular figures. `pub const RECEIPT_WIDTH: f32` (builder sets it; starting value 200.0).

- [ ] **Step 1: Write the failing tests**: `a_numeric_receipt_lists_the_spec_fields_in_order`, `one_valid_share_replaces_missing_and_nulls` (no line labelled `missing` or `nulls`), `a_year_long_timestamp_range_prints_dates`, `a_text_receipt_shows_distinct_and_top`, `the_two_range_ends_never_overlap_at_receipt_width` (lays out both with egui fonts at the widest housing values and at a full timestamp, asserts a gap).
- [ ] **Step 2:** `cargo test -p brightfield-shell --lib receipt` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "A column's numbers are set as a compact receipt, with ranges that fit their span".

### Task 7: Charts mode draws fixed chart rows under a sortable heading strip

**Files:**
- Create: `crates/brightfield-shell/src/column_rows.rs` (the chart row, the value strip, the quartile ladder, the heading strip)
- Modify: `crates/brightfield-shell/src/window.rs` (`draw_transposed_pane_group` ~`:22692` draws `column_rows` in `Charts`)
- Test: `crates/brightfield-shell/tests/grid_transposed_layout.rs`

**Interfaces:**
- Consumes: Task 2 `ordered`, `clicked`; Task 5 `resolve`; Task 6 `draw_receipt`, `RECEIPT_WIDTH`.
- Produces: `pub fn draw_heading_strip(ui, rect, fields: &[SortField], sort: GridSort, mode) -> Option<SortField>` (the clicked field), shared with Task 8; `pub fn draw_chart_row(ui, rect, column: &ColumnProfile, tile_rect_out: &mut egui::Rect, resolved: &Resolved, mode)` which lays out receipt | tile area, and under the tile area the value strip (from `ColumnMoments::rug`, shaded by count) and the quartile ladder (p5–p95 line, p25–p75 bar, median notch), both mapped through the resolved scale. A row draws no x-axis title. `pub const LADDER_HEIGHT: f32 = 11.0; pub const STRIP_HEIGHT: f32 = 9.0;`

- [ ] **Step 1: Write the failing tests**: `charts_mode_draws_a_row_per_column_in_sort_order`, `a_heading_click_sorts_and_a_second_reverses`, `sorting_on_index_restores_source_order`, `a_row_carries_a_quartile_ladder_at_the_profiles_quartiles` (reads the ladder's recorded x positions against the tile's scale for median_income), `a_log_row_names_its_scale`, `a_transposed_brush_still_narrows_the_other_rows` (kept from today).
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_transposed_layout` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "The chart rows carry the receipt, a value strip, a quartile ladder and a sortable heading strip".

### Task 8: List mode: one line per column, a brushable sparkline, and a click that opens the row

**Files:**
- Create: `crates/brightfield-shell/src/column_list.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (List branch of the transposed draw); the composer's re-homing path (`PaneViews`, `PaneSplit::Rehomed`) to draw a tile at a sparkline look and to skip composing tiles off screen
- Test: `crates/brightfield-shell/tests/grid_column_list.rs`

**Interfaces:**
- Consumes: Tasks 2, 5, 6, 7 (`draw_heading_strip`, `draw_chart_row`).
- Produces: `pub const LINE_HEIGHT: f32 = 30.0;` `pub fn draw_column_list(ui, rect, columns: &[ColumnProfile], tiles: &TileIndex, order: &[usize], open: &mut BTreeSet<String>, sort: GridSort, scale: GridScale, brush_enabled: bool, mode) -> ListOutcome` where `ListOutcome { pub clicked_sort: Option<SortField>, pub tiles_drawn: Vec<String> }`. A line: index, glyph, name, sparkline (the column's tile at sparkline look, bare, no axes), valid share, median (text: top value), range (text: distinct count). A column with no tile draws an empty sparkline place with the generator's omission reason on hover. Open lines are per pane, not saved.
- The sparkline and the opened row are the same tile; a column's bins, scale and filter cannot differ between them.

- [ ] **Step 1: Write the failing tests**:
  - `the_list_draws_a_30_point_line_per_column` (housing: nine lines, each 30 pt).
  - `a_click_opens_the_columns_chart_row_and_a_second_closes_it`.
  - `a_brush_on_a_sparkline_narrows_the_other_sparklines`.
  - `the_sparkline_and_the_open_row_share_bins_and_scale`.
  - `a_wide_table_queries_only_the_lines_on_screen` (a 300-column fixture; `tiles_drawn.len()` equals lines on screen; the session's query tally per brush is no larger).
  - `a_column_with_no_tile_draws_its_line_with_the_reason_on_hover` (an all-null and a one-value column).
  - `a_text_column_shows_its_top_value_and_distinct_count`.
  - `a_fresh_documents_columns_view_opens_on_the_list`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_column_list` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS; then `--test grid_transposed_layout --test grid_any_table` — expect PASS.
- [ ] **Step 5: Commit** "The grid's columns view lists one line per column with a sparkline that brushes".

### Task 9: The list beside the rows when the pane is wide

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (the `Rows` branch of the grid pane draw, both on the first screen and in `draw_canvas_grid_pane` ~`:23100`)
- Test: `crates/brightfield-shell/tests/grid_column_list.rs`

**Interfaces:**
- Consumes: Task 8 `draw_column_list`.
- Produces: `pub const SIDE_LIST_FROM: f32 = 900.0; pub const SIDE_LIST_WIDTH: f32 = 340.0;` The rows take `width - SIDE_LIST_WIDTH`; the side list shares the document's sort and scale; a click opens the row inline with the receipt stacked above the tile.

- [ ] **Step 1: Write the failing tests**: `a_wide_rows_pane_draws_the_list_beside_the_rows`, `a_narrow_rows_pane_draws_rows_alone` (the first screen at its default split), `a_brush_on_the_side_list_narrows_the_rows_beside_it`, `the_side_list_follows_the_documents_sort`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_column_list` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "A wide grid draws the column list beside its rows".

### Task 10: The spine's grid view carries the same header switches

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (`draw_canvas_grid_pane` ~`:23100` calls `record_layout_switch`, the scale switch, and `record_spot_switch`; its doc says why each is drawn)
- Test: `crates/brightfield-shell/tests/grid_spot.rs` or a new `grid_view_switches.rs`

- [ ] **Step 1: Write the failing test** `the_grid_view_header_offers_the_same_switches_as_the_dashboards_grid`: reads `rows`, `list`, `charts`, `canvas`, `ledger` off the spine grid view's header by text, for the opening table and for a ran step's table.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_view_switches` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "The spine's grid view offers the grid's own switches".

### Task 11: A step nobody has run lists its columns without the brush

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (the preview branch draws `draw_column_list` / chart rows from the preview profile with `brush_enabled: false`)
- Test: `crates/brightfield-shell/tests/grid_any_table.rs`

**Interfaces:**
- Consumes: `brightfield_engine::preview_profile_stoppable` via `ColumnProfiles`; Task 8 with `brush_enabled: false`.
- Copy: the pane says `Run the step to brush its columns`.

- [ ] **Step 1: Write the failing tests**: `an_unrun_steps_list_draws_from_its_preview`, `an_unrun_steps_sparklines_do_not_brush`, `an_unrun_step_says_to_run_it_to_brush`.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test grid_any_table` — expect FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same — expect PASS.
- [ ] **Step 5: Commit** "A step nobody has run lists its columns, and says to run it to brush them".

### Task 12: The design system carries the FineType family icons

Work in a worktree of `meridian-online/design`, as its own pull request there.

**Files:**
- Modify: `meridian-egui/src/icons/tabler.manifest` (regenerate with `cargo run -p meridian-egui --bin gen-tabler-icons -- <svg-dir> 3.45.0`), `meridian-egui/src/icons/data.rs` (generated), `meridian-egui/INVENTORY.md` if it lists icons

**Interfaces:**
- Produces: `Icon` consts for every icon in the family table below that is not already in the manifest. The table (FineType family → Tabler name), the builder's to confirm against Tabler 3.45.0's names:
  - `datetime.date` calendar · `.timestamp` clock · `.time` clock-hour-4 · `.component` calendar-event · `.offset` world-longitude · `.epoch` hash · `.period` calendar-stats · `.duration` hourglass
  - `geography.coordinate` current-location · `.location` map-pin · `.address` mailbox · `.transportation` plane · `.index` grid-dots · `.format` map · `.contact` phone
  - `finance.currency` coin · `.securities` chart-candle · `.banking` building-bank · `.rate` percentage · `.crypto` currency-bitcoin · `.payment` credit-card
  - `identity.person` user · `.medical` stethoscope · `.government` id · `.commerce` barcode · `.industry` building-factory · `.academic` school
  - `representation.numeric` hash · `.scientific` flask · `.text` letter-case · `.identifier` key · `.file` file · `.format` code-dots · `.boolean` toggle-left · `.discrete` list-numbers
  - `technology.internet` world-www · `.code` code · `.identifier` fingerprint · `.development` git-branch · `.cryptographic` lock · `.filesystem` folder · `.cloud` cloud
  - `container.object` braces · `.array` brackets · `.key_value` list-details
  - Storage fallbacks: numeric `hash`, temporal `clock`, text `letter-case`, boolean `toggle-left`, other `question-mark`.

- [ ] **Step 1:** Regenerate the manifest with the new names; `cargo test -p meridian-egui` — expect PASS (its inventory and manifest tests).
- [ ] **Step 2: Commit and open the pull request** "Tabler icons for FineType's type families". Hugh merges it.

### Task 13: Every place the grid names a column draws its FineType glyph

Starts once Task 12's pull request is merged.

**Files:**
- Create: `crates/brightfield-shell/src/type_glyph.rs`
- Modify: `Cargo.toml:90-91` (bump `meridian-design` and `meridian-egui` `rev` to the merge commit), `crates/brightfield-shell/src/column_header.rs:736` (`glyph_for` returns a `Glyph`), the list, the chart rows, the side list
- Test: unit tests in `type_glyph.rs`; the e2e targets above

**Interfaces:**
- Produces: `pub enum Glyph { Icon(meridian_egui::icons::Icon) }` and `pub fn glyph_for(semantic: &SemanticType, storage: &str, catalogue: Option<&SchemaCatalogue>) -> Glyph`: the catalogue's `icon(label)` when it names one that the manifest holds; else the family table by the label's first two segments; else the storage fallback. `SemanticType::Unusable` takes the storage fallback, as the generator distrusts it.

- [ ] **Step 1: Write the failing tests**: `a_date_label_draws_a_calendar`, `a_coordinate_label_draws_the_coordinate_glyph`, `a_catalogue_icon_wins_over_the_family`, `an_unlabelled_varchar_draws_the_text_fallback`, `an_unusable_label_takes_the_storage_fallback`, `every_finetype_family_has_a_glyph` (walks the families in the vendored catalogue; none falls through to `question-mark`).
- [ ] **Step 2:** `cargo test -p brightfield-shell --lib type_glyph` — expect FAIL.
- [ ] **Step 3:** Implement, replacing every caller of the old `glyph_for`.
- [ ] **Step 4:** Same — expect PASS; then `--test grid_column_list --test grid_transposed_layout` — expect PASS.
- [ ] **Step 5: Commit** "The grid draws each column's FineType glyph".

### Task 14: Baselines, the fold and the branch made ready

**Files:**
- Modify: `crates/brightfield-shell/tests/dashboard_baseline.rs` (add list-mode and side-list baselines in both themes); redraw `dashboard_transposed_{light,dark}.png`, `grid_in_ledger_columns_{light,dark}.png`, `legend_ring_right_columns_{light,dark}.png`; delete the stray `grid_in_ledger_columns_dark.diff.png` and `.new.png`
- Create: `docs/grid-columns.md` (the fold: what each layout draws, the scale rule, the sort rule, the glyph rule, the any-table path, and each of the owner's rulings from the spec with its reason)
- Modify: `CLAUDE.md` (one line under Docs: when to read `docs/grid-columns.md`)
- Delete: the spec and this plan

- [ ] **Step 1:** On the Mac Studio, redraw the baselines; open each new PNG and check it against the spec's goals.
- [ ] **Step 2:** `cargo test -p brightfield-shell --test dashboard_baseline --test legend_ring_baseline` — expect PASS.
- [ ] **Step 3:** Write the fold; delete the spec and plan; commit "The grid's column views are recorded in docs, and their baselines redrawn".
- [ ] **Step 4:** `cargo xtask gate <pr>` — expect a green `local-gate`; `roborev review --branch --wait`, fix Medium and above, close it; link the PR in Forge.

## Order and dependencies

| Task | Needs |
| --- | --- |
| 1 profile stats | — |
| 2 column order | — |
| 3 any table | the run-rows work on `main` |
| 4 switch and saved state | 2 |
| 5 scale | 1, 4 |
| 6 receipt | 1 |
| 7 chart rows | 2, 5, 6, 3 |
| 8 list | 7 |
| 9 side list | 8 |
| 10 grid view switches | 4, 3 |
| 11 unrun step | 8, 3 |
| 12 design icons | — (other repository) |
| 13 glyphs | 12 merged, 8 |
| 14 baselines and fold | all |

Any table first: Task 3 lands before the views are built on it, so every view task is tested over a step's table as well as the opening one.

## Verification cost

- Engine and pure unit tests (`cargo test -p brightfield-engine`, `cargo test -p brightfield-shell --lib <module>`): under a minute once built.
- Each `brightfield-shell` end-to-end target (`--test grid_any_table`, `grid_column_list`, `grid_transposed_layout`): a few minutes each; the gate leaves them to the nightly, so run the ones a task touches before pushing.
- `cargo xtask gate <pr>`: the full gate on the Mac Studio, one at a time, tens of minutes.
- Baseline redraws: Mac Studio only.
