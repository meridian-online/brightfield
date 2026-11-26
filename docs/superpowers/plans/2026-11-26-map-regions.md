# Map regions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Any plot with a `dot` or `hexbin` layer over two quantitative columns outlines and names its dense regions from its own rows, recomputes them on each settled zoom, lets a click select a region's rows, and pins regions and names in committed baselines.

**Architecture:** Pure algorithms (weighted bandwidth, region finding, outlines, label anchors) live in `brightfield-render` beside `kde.rs`. SQL builders for the view-extent grid, the per-region label summary and the new `Predicate::Cells` live in `brightfield-sql`. The engine adds a rows query without cross-plot selections and a detached reader that runs on another thread. The shell runs one worker per chart document (the typed-SQL preview's pattern), turns summaries into names, and draws regions as an overlay after composition, the way the committed-selection band is drawn. No mark is injected into the spec, so mark indices do not move.

**Tech Stack:** Rust (pinned toolchain), DuckDB via duckdb-rs, egui/eframe + wgpu, vello scene, egui_kittest snapshots.

**Spec:** `docs/superpowers/specs/2026-11-26-map-regions-design.md`. Read it whole before any task; this plan argues from it and does not restate its reasons.

## Global constraints

- Labels come from the rows only: no model call, no network.
- Build from arXiv 2504.07285. Apple's `density_clustering` crate may be read and not copied: no code, comments or test data taken from it.
- Grid: 100 × 100 cells over the plot's current x and y domains (the displayed scales), not the whole table.
- Truncate each region to cells ≥ 0.1 × its own peak density.
- Hill-climb ties break by lower row-major cell index; regions are ordered by peak density descending, then peak cell index. Two runs over the same rows and extent give identical output.
- Regions describe the qualifying layer's rows without other plots' selections; they follow only the plot's own view.
- Region work never runs on the UI thread; a settled zoom takes as long as it does today.
- `regions: off` is the only spec form; it is written only when off.
- A plot without an `intervalXY` selection draws regions and offers no click.
- Public repo: no planning identifiers in code, comments, commits or PR text. Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the task's tests. After each commit: `cargo xtask ci --step comment-citations`.
- Pixel baselines are redrawn only on the Mac Studio, never from a hosted run.

## Review focus

The inputs the spec implies and no feature test would otherwise meet, most likely to bite first. Each has its test in the named task.

1. **A zoom so narrow that few or no rows are in view.** Expect no regions and no error. (Task 2 `empty_and_flat_grids_have_no_regions`, Task 7 `a_zoom_onto_empty_space_draws_no_regions`.)
2. **A newer zoom settling while an older region run is in flight.** The older result must never draw over the newer view. (Task 7 `a_result_for_a_stale_extent_is_dropped`.)
3. **Column names and text values with quotes, and nulls in the label column.** SQL must stay valid and nulls must not become a name. (Task 4 `quoted_column_names_round_trip`, Task 5 `nulls_and_blank_text_never_name_a_region`.)
4. **A constant axis, or one with all values equal in view.** Grid extent `lo == hi` gives no regions, not a division by zero. (Task 4 `a_degenerate_extent_yields_no_grid_sql`.)
5. **The toggle thrown, or the file reopened, while a region selection is held.** The selection clears with the session rebuild, as the scale switch's does, and no stale outline is drawn. (Task 9 `turning_regions_off_drops_a_held_region_selection`.)

---

### Task 1: Weighted 2D bandwidth

**Files:**
- Modify: `crates/brightfield-render/src/kde.rs` (beside `silverman_2d_per_axis` at :244)
- Modify: `crates/brightfield-render/src/mark.rs` (`build_kde_grid` at :2710)

**Interfaces:**
- Produces: `pub fn silverman_2d_per_axis_weighted(xs: &[(f64, f64)], ys: &[(f64, f64)]) -> (f64, f64)`, where each pair is `(value, weight)`.

- [ ] **Step 1: Write the failing tests** in `kde.rs`'s test module: `silverman_2d_weighted_matches_expanded` builds 50 `(x, y, count)` bins with counts 1–9, expands them into samples, and asserts the weighted result equals `silverman_2d_per_axis` on the expansion to within `1e-12` relative on each axis. `silverman_2d_weighted_degenerate_returns_zero` asserts a single bin gives `(0.0, 0.0)`.
- [ ] **Step 2: Run** `cargo test -p brightfield-render --lib kde::` and expect FAIL: function not defined.
- [ ] **Step 3: Implement** it by applying `silverman_1d_weighted`'s formula per axis. Switch `build_kde_grid`'s no-bandwidth path from expanding counts to the weighted form.
- [ ] **Step 4: Run** `cargo test -p brightfield-render` (about 3 min), then `cargo test -p brightfield-shell --test density_examples` or whichever test pins the byte-identical density PNGs (`grep -rln "density" crates/*/tests`). Expect PASS with PNGs unchanged. If a density PNG changes, stop and report the pixel difference; do not redraw it.
- [ ] **Step 5: Commit** `Compute the 2D density bandwidth from bin counts rather than expanded samples`.

### Task 2: Find regions in a density grid

**Files:**
- Create: `crates/brightfield-render/src/regions.rs`, and export it from `lib.rs`

**Interfaces:**
- Produces:
  - `pub struct DensityGrid { pub cols: usize, pub rows: usize, pub x: (f64, f64), pub y: (f64, f64), pub density: Vec<f64> }`. Row-major, row = y. `x` and `y` are the grid's outer edges in data units.
  - `pub struct Region { pub peak: usize, pub peak_density: f64, pub cells: Vec<usize>, pub share: f64 }`. `cells` are sorted row-major indices. `share` is the region's density mass over the grid's total.
  - `pub fn find_regions(grid: &DensityGrid) -> Vec<Region>`
  - `pub const MIN_REGION_CELLS: usize`, the builder's value with its reason in a doc comment (start at 6).
  - `pub fn cell_of(grid: &DensityGrid, x: f64, y: f64) -> Option<usize>` and `pub fn cell_bounds(grid: &DensityGrid, cell: usize) -> ((f64, f64), (f64, f64))`.

- [ ] **Step 1: Write the failing tests** in the module, on synthetic grids built by a test helper that sums Gaussians:
  - `two_separated_peaks_are_two_regions`: peaks at cells (20, 20) and (80, 80) give 2 regions, ordered by peak density.
  - `a_bump_on_a_mountain_merges_into_it`: a small peak whose saddle with a large one is within the merge criterion gives 1 region.
  - `regions_are_truncated_at_a_tenth_of_their_peak`: every cell in every region has `density >= 0.1 * peak_density`.
  - `a_speck_is_not_a_region`: an isolated peak covering fewer than `MIN_REGION_CELLS` cells above truncation is dropped.
  - `empty_and_flat_grids_have_no_regions`: all-zero and constant grids give `vec![]`.
  - `a_single_cloud_is_one_region`.
  - `equal_neighbours_break_ties_by_cell_index`: a plateau grid gives identical output over 20 runs, and the plateau joins the lowest-index cell.
  - `cell_of_and_cell_bounds_agree`: the centre of `cell_bounds(c)` maps back to `c` for every cell; points on the top edge map to the last row or column.
- [ ] **Step 2: Run** `cargo test -p brightfield-render --lib regions::` and expect FAIL.
- [ ] **Step 3: Implement** the paper's steps 1–4 (hill-climb by union-find over the 8-neighbourhood, a neighbour graph with saddle densities, greedy merge closest first by the paper's criterion, truncation), then the `MIN_REGION_CELLS` filter. Cite the paper's section for the merge criterion in a comment, with the constant it uses.
- [ ] **Step 4: Run** the tests and expect PASS.
- [ ] **Step 5: Commit** `Find a density grid's regions by hill-climbing and merging, after the published algorithm`.

### Task 3: Region outlines and label anchors

**Files:**
- Modify: `crates/brightfield-render/src/regions.rs`

**Interfaces:**
- Consumes: `DensityGrid`, `Region`, `cell_bounds` (Task 2).
- Produces:
  - `pub fn region_outline(grid: &DensityGrid, region: &Region) -> Vec<Vec<(f64, f64)>>`: closed rings in data units, outer rings first.
  - `pub fn label_anchor(grid: &DensityGrid, region: &Region) -> (f64, f64)`: the centre, in data units, of the largest square of region cells nearest the peak.
  - `pub fn region_at(grid: &DensityGrid, regions: &[Region], x: f64, y: f64) -> Option<usize>`: the hit-test, an index into `regions`.

- [ ] **Step 1: Write the failing tests:**
  - `an_l_shaped_region_has_one_ring_of_eight_corners`.
  - `a_region_with_a_hole_has_two_rings`.
  - `the_anchor_sits_inside_its_region`: on a crescent region, the anchor's cell is a region cell.
  - `the_anchor_is_near_the_peak`: on a large square region, the anchor is within 3 cells of the peak.
  - `region_at_finds_the_region_under_a_point_and_none_in_the_gap`.
- [ ] **Step 2: Run** `cargo test -p brightfield-render --lib regions::` and expect FAIL.
- [ ] **Step 3: Implement** with a cell-edge tracer: walk the boundary edges between region and non-region cells. This gives axis-aligned rings, which read cleanly at 100 × 100. Use marching squares instead only if a test shows the tracer fails. For the anchor, compute the largest-square distance transform over the region mask and take its maximum, breaking ties by distance to the peak.
- [ ] **Step 4: Run** and expect PASS.
- [ ] **Step 5: Commit** `Trace a region's outline and place its label inside it, near its peak`.

### Task 4: The view-extent grid query and the cells predicate

**Files:**
- Create: `crates/brightfield-sql/src/regions.rs`, and export it from `lib.rs`
- Modify: `crates/brightfield-sql/src/ir.rs` (the `Predicate` enum at :180 and its `Display`)
- Modify: every exhaustive `match` on `Predicate` that the compiler names
- Test: `crates/brightfield-engine/tests/region_sql.rs` (DuckDB round trips)

**Interfaces:**
- Produces:
  - `pub struct GridExtent { pub x: (f64, f64), pub y: (f64, f64), pub bins: u32 }`
  - `pub fn region_grid_sql(rows_sql: &str, x_col: &str, y_col: &str, extent: &GridExtent) -> Option<String>`. Returns columns `bx INTEGER, by INTEGER, n DOUBLE`, occupied cells only, ordered by `by, bx`. `None` when either extent has `lo >= hi` or is not finite. Rows outside the extent and nulls are excluded. The top edge folds into the last bin, as `equiwidth_bin_centre` does.
  - `pub fn cell_index_sql(x_col: &str, y_col: &str, extent: &GridExtent) -> String`: the `by * bins + bx` expression the other queries share.
  - `Predicate::Cells { x: String, y: String, extent: GridExtent, cells: Vec<u32> }`. It renders as `(<x> BETWEEN … AND <y> BETWEEN … AND <cell_index_sql> IN (…))`, so the range test bounds the scan and the membership test selects the cells. `cells` are sorted and deduplicated at construction (`Predicate::cells(..)`), so equal regions give equal predicates.
  - `GridExtent` derives `Clone, Debug, PartialEq` and implements `Eq` and `Hash` over the bit patterns of its `f64`s, because `Predicate` derives `Eq` and `Hash`.

- [ ] **Step 1: Write the failing tests.** In the sql crate:
  - `a_degenerate_extent_yields_no_grid_sql`.
  - `quoted_column_names_round_trip`: a column named `a "b"` is quoted correctly in both builders.
  - `cells_predicate_is_canonical`: unsorted, duplicated input renders the same as sorted.
  In `crates/brightfield-engine/tests/region_sql.rs`, run against an in-memory DuckDB:
  - `the_grid_counts_only_rows_in_view`: 1,000 rows, an extent over half of x; the sum of `n` equals the rows in that half.
  - `the_top_edge_falls_in_the_last_bin`.
  - `a_cells_predicate_selects_exactly_the_rows_in_those_cells`: compared with a hand-computed count.
- [ ] **Step 2: Run** `cargo test -p brightfield-sql regions` and `cargo test -p brightfield-engine --test region_sql`. Expect FAIL.
- [ ] **Step 3: Implement.** For every existing consumer of `Predicate`:
  - the cube (`crates/brightfield-sql/src/cube.rs:53–65`) declines `Cells`, so the query runs directly;
  - `compile_selection` treats it like any leaf;
  - the navigation pass leaves it alone.
- [ ] **Step 4: Run** both test commands, then `cargo test -p brightfield-sql` (about 4 min). Expect PASS.
- [ ] **Step 5: Commit** `Count rows in a grid over a plot's view, and select rows by grid cell`.

### Task 5: Region names from row summaries

**Files:**
- Modify: `crates/brightfield-sql/src/regions.rs` (the label query)
- Create: `crates/brightfield-shell/src/region_names.rs` (column roles and naming rules)
- Test: `crates/brightfield-engine/tests/region_sql.rs` (query), plus unit tests in `region_names.rs`

**Interfaces:**
- Consumes: `GridExtent`, `cell_index_sql` (Task 4).
- Produces:
  - `pub enum LabelRole { Text, Category, Number }`.
  - `pub struct LabelColumn { pub name: String, pub role: LabelRole }`.
  - `pub fn region_label_sql(rows_sql: &str, x_col: &str, y_col: &str, extent: &GridExtent, cell_regions: &[(u32, u32)], columns: &[LabelColumn]) -> Option<String>`. `cell_regions` holds `(cell index, region index)` pairs. The query joins them as a `VALUES` list on `cell_index_sql`; rows in no region carry region `-1`. It returns one long table `(column, region, key, value)` holding:
    - per text token and two-token phrase: its count of rows per region;
    - per category value: its count per region;
    - per number: `q1` and `q3` per region and for the rest of the view.
    It also returns row totals per region and for the view. The builder picks the exact shape; the contract is that `name_regions` reads it.
  - In `region_names.rs`:
    - `pub fn label_columns(columns: &[ColumnProfile], x: &str, y: &str) -> Vec<LabelColumn>`, which assigns roles by the spec's rule: storage type and distinct count first, FineType's type refining when present. Skip the axis columns, identifiers, dates and timestamps.
    - `pub fn name_regions(columns: &[LabelColumn], summary: &[RecordBatch], regions: usize) -> Vec<Option<String>>`, one entry per region.
  - Thresholds as named constants with their reasons:
    - `TEXT_MIN_SHARE = 0.5`
    - `CATEGORY_MIN_SHARE = 0.6`
    - `CATEGORY_MIN_LIFT = 2.0`
    - number: interquartile ranges must not overlap.
  - Tokenising: lower-case, split on non-letters, drop tokens of fewer than 3 letters and a short English stop-word list, keep two-token phrases from adjacent kept tokens. Print a name in the capitalisation most common in the rows.

- [ ] **Step 1: Write the failing tests:**
  - `a_shared_place_word_names_its_region`: fixture rows like the earthquake `place` strings name one region "Russia" and another "Tonga".
  - `a_dominant_category_names_its_region`: "Gentoo".
  - `a_category_common_everywhere_names_nothing`: below lift 2.
  - `a_clean_range_names_a_numeric_region`: it is formatted in the column's own format, as its axis takes it (`docs/column-formats.md`), for example `median_house_value $300k–$450k`.
  - `overlapping_ranges_name_nothing`.
  - `nulls_and_blank_text_never_name_a_region`.
  - `the_first_passing_column_in_table_order_wins`.
  - `identifiers_and_axis_columns_are_never_tried`.
  - In `region_sql.rs`: `the_label_query_runs_and_counts_rows_per_region`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --lib region_names` and `cargo test -p brightfield-engine --test region_sql`. Expect FAIL.
- [ ] **Step 3: Implement.** Score text with class-based TF-IDF, as the spec says. Text tokenising may use DuckDB's `regexp_split_to_array` and `unnest` in the query.
- [ ] **Step 4: Run** both and expect PASS.
- [ ] **Step 5: Commit** `Name a region from what its rows share and the rest of the view does not`.

### Task 6: Engine rows without cross-plot selections, and a detached reader

**Files:**
- Modify: `crates/brightfield-engine/src/lib.rs` (beside `step_rows_sql` at :1788 and `execute_uncached` at :2368)
- Test: `crates/brightfield-engine/tests/region_reader.rs`

**Interfaces:**
- Produces:
  - `pub fn region_rows_sql(&self, index: usize) -> Result<String, EngineError>`: `emit_rows_query(&self.spec, index, params, None, RowsAudience::Plot)`, the mark's rows with parameters and without any selection.
  - `pub fn region_reader(&self) -> Result<RegionReader, EngineError>`.
  - `pub struct RegionReader` (`Send`), with `pub fn query(&self, sql: &str) -> Result<Vec<RecordBatch>, duckdb::Error>` and `pub fn interrupt(&self)`. It wraps `duckdb::Connection::try_clone()` of the session's connection.

- [ ] **Step 1: Write the failing tests:**
  - `region_rows_ignore_other_plots_selections`: a two-plot spec with a held brush on plot B; plot A's `region_rows_sql` returns all rows.
  - `the_reader_sees_the_sessions_tables_and_views`: open the earthquake start's Protocol tables the way `crates/brightfield-shell/src/protocol_panels.rs` does, then `query(region_rows_sql(..))` from a spawned thread returns 8,502 rows.
  - `interrupt_stops_a_running_read`.
- [ ] **Step 2: Run** `cargo test -p brightfield-engine --test region_reader` and expect FAIL.
- [ ] **Step 3: Implement.** If `try_clone` cannot see objects the session created (connection-local temporary views), stop and report which objects and where they are created. Do not change how the session creates them in this task.
- [ ] **Step 4: Run** and expect PASS. Then run `cargo test -p brightfield-engine` (about 6 min).
- [ ] **Step 5: Commit** `Read a plot's rows without other plots' selections, from a second thread`.

### Task 7: The region worker and the overlay

**Files:**
- Create: `crates/brightfield-shell/src/regions.rs` (qualification, job, worker, state)
- Modify: `crates/brightfield-shell/src/pipeline.rs`: call `ink_regions` after composition, before `ink_committed_selections` (:2027)
- Modify: `crates/brightfield-shell/src/app.rs`: `ChartDoc` owns a `RegionsWorker`; start jobs after a compose and in `settle_navigation` (:4453); poll each frame
- Test: `crates/brightfield-shell/tests/regions_overlay.rs`

**Interfaces:**
- Consumes: Tasks 2–6.
- Produces:
  - `pub fn qualifying_layer(spec: &Spec, plot_path: &str) -> Option<(usize, String, String)>`: the mark index and x and y columns of the first `dot` or `hexbin` layer over plain quantitative columns, on a plot with no band scale and no `regions: off`.
  - `pub struct RegionJob { plot_path: String, mark: usize, extent: GridExtent, rows_sql: String, columns: Vec<LabelColumn>, x: String, y: String }`.
  - `pub struct PlotRegions { pub extent: GridExtent, pub grid: DensityGrid, pub regions: Vec<Region>, pub outlines: Vec<Vec<Vec<(f64, f64)>>>, pub names: Vec<Option<String>>, pub anchors: Vec<(f64, f64)> }`.
  - `pub struct RegionsWorker`:
    - `request(job)` stops any in-flight run for that plot and counts it as superseded;
    - `poll() -> bool` returns true when something changed;
    - `regions(plot_path) -> Option<&PlotRegions>`;
    - `started()` and `superseded()` counters.
  - `pub(crate) fn ink_regions(composed: &mut Composed, worker: &RegionsWorker)`: outlines, then names with knockout ink, decluttered by share order. A name whose box overlaps a placed one, or leaves the plot, is dropped and its outline kept.
  - A job runs grid query → `kde_2d` with the weighted bandwidth → `find_regions` → outlines and anchors → label query → `name_regions`. A result is kept only if its extent equals the plot's current displayed extent.

- [ ] **Step 1: Write the failing tests** in `regions_overlay.rs`, with headless composition as `point_map_baseline.rs` does:
  - `the_earthquake_panel_qualifies_on_its_dot_layer`: mark 0, `longitude`, `latitude`.
  - `a_bar_or_band_plot_does_not_qualify`.
  - `regions_arrive_after_the_first_compose_without_blocking_it`: compose returns before the worker finishes; after polling, regions exist.
  - `a_result_for_a_stale_extent_is_dropped`: request A, then B with a new extent before A finishes. Only B's result is kept, and `superseded() == 1`.
  - `a_settled_zoom_recomputes_over_the_view`: after `zoom_view(4.0)`, the extent equals the displayed domains and the regions differ from the wide view's.
  - `a_zoom_onto_empty_space_draws_no_regions`.
  - `two_names_never_overlap`: on the earthquake panel at default size, the placed label boxes are pairwise disjoint.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test regions_overlay` and expect FAIL.
- [ ] **Step 3: Implement.** Follow `crates/brightfield-shell/src/typed_sql.rs` (:455–480) for the thread, the stop handle and the wake. Use outline and label ink from the design system's tokens, as the selection band takes its ink. Draw text with `brightfield_render::text::draw_text` and `measure_width`.
- [ ] **Step 4: Run** and expect PASS. Then `cargo test -p brightfield-shell --test point_map_baseline` will fail on pixels, which is expected until Task 10. Note which snapshots fail and do not redraw them.
- [ ] **Step 5: Commit** `Draw each qualifying plot's regions and names, computed off the UI thread`.

### Task 8: The regions switch and `regions: off`

**Files:**
- Modify: `crates/brightfield-spec/src/parse.rs` (the plot attribute keys near :150), and wherever `yScale` and the other plot attributes are validated (find with `grep -rn '"yScale"' crates/brightfield-spec/src`)
- Modify: `crates/brightfield-shell/src/chart_item.rs`: `draw_regions_switch`, beside `draw_scale_switch` (:1646) and `draw_normalise_switch` (:1801), drawn at :1369–1389
- Modify: `crates/brightfield-shell/src/app.rs`: `set_plot_regions(plot, on: bool) -> bool` through `set_plot_attribute` (:2903)
- Test: `crates/brightfield-shell/tests/regions_switch.rs`

**Interfaces:**
- Consumes: `qualifying_layer` (Task 7), which reads `regions: off`.
- Produces: `ChartDoc::set_plot_regions(&mut self, plot: usize, on: bool) -> bool`.

- [ ] **Step 1: Write the failing tests:**
  - `regions_off_parses_without_a_warning`.
  - `an_unknown_regions_value_warns`, for example `regions: maybe`.
  - `the_switch_shows_only_on_qualifying_plots`.
  - `turning_regions_off_writes_the_attribute_and_hides_them`.
  - `turning_them_on_removes_the_attribute`: the saved spec is byte-identical to the original.
  - `a_spec_saved_with_regions_off_reopens_with_them_off`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test regions_switch` and `cargo test -p brightfield-spec`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** and expect PASS.
- [ ] **Step 5: Commit** `Switch a plot's regions off and on, saved as regions: off`.

### Task 9: Click a region to select its rows

**Files:**
- Modify: `crates/brightfield-shell/src/chart_item.rs`: in `resolve_gesture` (~:2141), a press-and-release without a drag inside a region on a plot whose gesture is `IntervalXY` resolves to `Interaction::Select` with `Predicate::Cells`; a repeat clears (~:2178)
- Modify: `crates/brightfield-shell/src/pipeline.rs`: `gather_selected` (:2084) and the committed band read `Cells`; `ink_regions` emphasises the held region's outline
- Modify: `docs/interaction-speed.md`: a line beside the brush-while-zoomed gap saying a region selection runs uncached
- Test: `crates/brightfield-shell/tests/regions_select.rs`

**Interfaces:**
- Consumes: `region_at` (Task 3), `Predicate::cells` (Task 4), `RegionsWorker::regions` (Task 7).

- [ ] **Step 1: Write the failing tests** on the earthquake panel:
  - `clicking_a_region_filters_the_other_plots_to_its_rows`: the weekly bars' total equals the region's row count, from a `Cells` predicate count against DuckDB.
  - `clicking_it_again_clears_the_selection`.
  - `escape_clears_a_region_selection`.
  - `a_drag_still_draws_the_rectangle_brush`.
  - `a_plot_without_an_xy_brush_offers_no_click`: a scatter spec with no `select:`.
  - `turning_regions_off_drops_a_held_region_selection`.
  - `a_held_region_keeps_its_cells_after_a_zoom`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test regions_select` and expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** and expect PASS.
- [ ] **Step 5: Commit** `Select a region's rows by clicking it`.

### Task 10: Region baselines and the redrawn snapshots

**Files:**
- Create: `crates/brightfield-shell/tests/region_baseline.rs` and `crates/brightfield-shell/tests/regions/*.txt` (one committed table per fixture and view)
- Create: an `#[ignore]`d test, `region_names_match_with_finetype`, in `crates/brightfield-shell/tests/region_baseline.rs`. It reads `BRIGHTFIELD_FINETYPE_BUNDLE` as `crates/brightfield-engine/tests/finetype_bundle.rs` does.
- Modify: snapshots under `crates/brightfield-shell/tests/snapshots/`, redrawn on the Mac Studio

**Interfaces:**
- Consumes: `PlotRegions` (Task 7).

- [ ] **Step 1: Write the tests.** `region_table(&PlotRegions) -> String` writes one line per region: index, peak cell and its data position, cell count, share to 4 decimal places, bounding box, name or `-`, anchor. Data positions are printed to 6 significant figures. Fixtures:
  - the earthquake start's panel (`examples/starts/usgs-earthquakes-2025`), at the default view and at x 140–170, y 35–60;
  - Palmer Penguins (`examples/starts/palmer-penguins/data/penguins.csv`) as a scatter of bill length against bill depth, at the default view;
  - `california_housing_sample.csv` as `dashboard_baseline.rs` reads it, as a point map at the default view.
  Each test compares with its `.txt` and fails before any image snapshot. Add the mutation tests:
  - `a_bandwidth_change_reddens_the_tables`: halve the bandwidth through a test-only override and assert the earthquake table differs;
  - `a_label_rule_change_reddens_the_tables`: raise `TEXT_MIN_SHARE` to 0.99 and assert it differs.
- [ ] **Step 2: Record the tables** by running once with `UPDATE_REGION_TABLES=1`, then read every line. Check that the earthquake wide view names the Kamchatka cluster from its rows ("Russia" is expected) and that each anchor lies within its region's bounding box. Commit the tables.
- [ ] **Step 3: Run** the whole shell suite, `cargo test -p brightfield-shell` (about 15 min). List every snapshot that fails. On the Mac Studio, redraw only those with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test <file>`. Open each redrawn PNG and confirm the only change is the regions. Render tests that count ink on a plot and are about something else get `regions: off` in their spec.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell` again and expect PASS. Run the ignored test with a bundle where one is available, and say in the PR whether it ran.
- [ ] **Step 5: Commit** `Pin each fixture's regions and names ahead of its picture, and redraw the snapshots regions change`.

### Task 11: Fold the design into the docs and run the gate

**Files:**
- Create: `docs/map-regions.md`: what regions are, the pipeline, the naming rules and their thresholds, and the rulings with their reasons in plain words, from the spec
- Modify: `CLAUDE.md`: one line saying when to read `docs/map-regions.md`
- Delete: `docs/superpowers/specs/2026-11-26-map-regions-design.md` and this plan

- [ ] **Step 1: Write** `docs/map-regions.md` from the spec's rulings and design, as it stands after the build. Record any threshold changed during Tasks 2–5, with its reason.
- [ ] **Step 2: Run** `cargo xtask gate <pr>` on the Mac Studio (about 51 minutes on an idle machine). Expect `local-gate` to be green on the head.
- [ ] **Step 3: Commit** `Record how map regions work, and retire the spec and plan`.

## Dependencies

- Tasks 1, 2 and 4 are independent.
- Task 3 needs 2. Task 5 needs 4. Task 6 needs 4 for its tests.
- Task 7 needs 1, 2, 3, 5 and 6. Tasks 8 and 9 need 7. Task 10 needs 7, 8 and 9. Task 11 is last.
