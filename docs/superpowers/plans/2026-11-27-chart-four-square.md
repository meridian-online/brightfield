# A new chart starts from the suggestion four-square, and a chart is a node of its own: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chart works over any table or view. Over a node nothing reads, it opens the chart editor on an unsaved draft whose empty chart area shows the suggestion four-square: three suggested charts and the generated dashboard, drawn live from the node's rows. A saved chart is a node of its own in `charts/`, and a saved dashboard is a node in `dashboards/`.

**Architecture:** The graph gains `AssetKind::Chart`. The shell reads `charts/` and `dashboards/` when it opens a Protocol and adds a node for each file, with edges from the tables each file reads by name. The window keeps one held table and one chart document (approach A). Going to another node snapshots that node's rows from the Protocol's database to Parquet and makes it the held node, without a generated dashboard. A node no run has reached gets a preview run first. A draft is a chart document with no file, held as `CanvasHolds::Draft`. When it has no column on any channel, the chart area under the shelf draws the four-square. Picking a cell, or putting a column, replaces the four-square with the draft's chart. Cmd+S writes the draft to its folder, and the re-read adds its node.

**Tech Stack:**
- Rust 1.95 (`cargo +1.95.0` in a fresh worktree).
- egui/eframe 0.35 and egui_kittest.
- DuckDB, through `brightfield-engine`, and its `httpfs` reads for the sample.
- `serde_yaml` 0.9 and `brightfield-spec`'s parser.
- arc's library (`arc::spec`) for the manifest, the database path and `check_panels`.

**Spec:** `docs/superpowers/specs/2026-11-26-chart-four-square-design.md`. Read it first. Its goal numbers (G1–G8) are cited below.

## Global Constraints

- The repo is public. Commit messages, PR text, branch names, code comments and docs name no planning ids, no private repo and no local path. Each commit message says what the change does in plain words and names no other product as the one it follows. The spec's Tableau reference stays in the spec, which is deleted at the fold.
- Before each commit run `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy` and the tests the task names. After each commit run `cargo xtask ci --step comment-citations`.
- One cargo build at a time in the session. Background a build that may wait on a slot.
- Pixel baselines are drawn on the Mac Studio's M5 Max only (`UPDATE_SNAPSHOTS=1`). Never loosen `kittest.toml`, and never redraw a baseline from a hosted run.
- `crates/brightfield-shell/tests/token_discipline.rs` stays green with its allowlists unchanged. The shell constructs no `egui::Area`, `egui::Window` or `egui::Modal`; overlays go through `ModalLayer` as `WhichProtocolSheet` does.
- Every new verb is a `VerbEntry` in `crates/brightfield-keys/src/registry.rs` with a sentence-case unique `label`, declared `needs`, and a score row in `crates/brightfield-keys/tests/data/keymap-research.md`. The registry gates stay green: `no_two_verbs_share_a_chord_in_overlapping_contexts`, `no_two_verbs_in_one_context_share_a_keystroke_sequence`, `longname_snapshot_is_stable` (extend it) and `every_verb_has_a_unique_label_in_sentence_case`.
- "Greyed means a missing input, never a missing feature": every greyed verb says why.
- Fixed names, from the spec: the **suggestion four-square** (never "suggestion grid" or "suggestion tiles"); the folders `charts/` and `dashboards/`. Save never writes to `panels/` (anti-goal).
- The shelf is usable before any cell is picked, and no suggestion needs typing before it shows (anti-goals).
- A data file still opens on its generated dashboard (non-goal "What a file opens on").
- Agent-written `panels/` files keep the panel-tile window and are never moved (G8).

## Rulings this plan makes on the points the spec left to the builder

Each is recorded on the issue at the end of planning. Each can be undone, and none changes what the analyst gets.

1. **How a dashboard names a chart.** A layout entry `chart: <name>` may stand wherever a component may, inside `hconcat:` or `vconcat:`. Before parsing, `dashboard_file::resolve` replaces each entry with the named chart's root component. It merges that chart's `data:` and `params:` entries into the dashboard's, and the dashboard's own entries win on a clash. A missing chart becomes a one-mark `text` plot that reads *No chart named <name> in charts/.* The plots that come from named charts draw and cross-filter, and take no shelf edits. The shelf reads an inline hero plot only.
2. **Node ids.** A saved chart is `chart.<protocol>.<slug>` and a saved dashboard is `dashboard.<protocol>.<slug>`, where the slug is `panel_capture::filename_slug`. The second matches `generated_dashboard_id` for a dashboard saved under its table's name, so the dashboard Save writes for the table the file opened as is the same node on reopen. It is read back by the existing `read_saved_chart` path, as today, and the folder reader skips it.
3. **The preview run.** arc runs a whole Protocol (`arc run` has no target or row cap), so the steps up to a node run as one whole preview run. That run starts through `Run::begin` and is marked by `run::mark_preview`. While it runs, the pane says *Running the Protocol to show <label>…*.
4. **The 1,000-row sample.** When a `http_fetch` step whose outputs are not on disk stands upstream of the node, Brightfield does not start arc on the real folder. Instead it does five things:
   - It copies the Protocol to a scratch folder.
   - Through DuckDB it writes the first 1,000 rows of each such fetch's URL to the path the step would write: `read_csv_auto`, `read_parquet` or `read_json_auto` by the URL's extension, each with `LIMIT 1000`.
   - It drops those fetch steps from the scratch manifest.
   - It runs arc there.
   - It snapshots the node's rows from the scratch database.

   The pane says the rows are a sample, with a *Run in full* control that starts the analyst's whole run (`run_protocol`). A fetch whose URL is not one of those three formats, a zip for instance, cannot be sampled. That pane offers *Run in full* with the sentence *<source> is not rows Brightfield can sample; a full run fetches it.*
5. **Fewer than three applicable kinds.** An unfilled chart cell takes the next kind in registry order that does not apply. It shows one line, *<description>: <bind error>*, where the error is the one `ChartKind::bind` returns. It is never blank.
6. **New chart.** The verb is `new-chart`, labelled *New chart*, bound to `cmd-alt-n` globally and scored 3/4/3 (frequency/mnemonic/convention).
7. **Cell drawing.** Each cell draws on the live canvas through `EguiCanvasHost::present_keyed`. It is composed with `pipeline::compose_spec_str_in` at the cell's body size, recomposed once its size has settled (as `PanelTiles::compose_for` does), and cached per node and size.
8. **The draft name.** A chart draft is `<node label>_<kind id>` and a dashboard draft is `<node label>_dashboard`, both slugged. `_2`, `_3` and so on are added until no file of that name is in the folder.
9. **An untouched draft is not unsaved.** A draft with no cell picked and no column put leaves without asking. Once a cell is picked or a column lands, the pane reads Edited until Cmd+S.
10. **Rename.** The spec says a draft is "renamed through the existing rename path", but that path renames only the Protocol folder. Task 13 adds `rename-chart`, which renames a draft or a saved chart's file and rewrites each `chart: <old>` entry in `dashboards/`.

## Review Focus

1. **A dashboard naming a chart that reads another table.** Both tables must have rows before it draws (spec, "A chart may read several tables"). Task 5 snapshots every table a chart or dashboard reads, not only the node in focus.
2. **Moving back to the table the file opened as.** It must come back with its generated dashboard and its saved chart, not as a bare node. Task 6 tests the round trip.
3. **A column name the four-square binds that holds a quote or a space.** The rebound spec and the Parquet injection must both quote it. Task 8 tests a column named `price ($)`.
4. **Save of a draft while the Protocol has never been saved** (a data file opened, not yet a folder). Cmd+S must save the Protocol first and then the chart, in one press. Task 11 tests it.
5. **A sample run that fails**: the URL refuses or arc errors in the scratch folder. The pane must say why and still offer *Run in full*, not spin. Task 7 tests a refused URL.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/brightfield-protocol/src/graph.rs` | `AssetKind::Chart`; `chart_node_id`, `saved_dashboard_id`; `add_chart_files` |
| `crates/brightfield-protocol/src/chart_file.rs` (new) | `ChartFile`, `ChartFileKind`: what a file in `charts/` or `dashboards/` reads and names |
| `crates/brightfield-model/src/panel_capture.rs` | `CHARTS_DIR`, `DASHBOARDS_DIR`, `chart_file`, `dashboard_file`, the tables and joins dashboard paths |
| `crates/brightfield-shell/src/chart_files.rs` (new) | Reads `charts/` and `dashboards/` into `ChartFile`s |
| `crates/brightfield-shell/src/dashboard_file.rs` (new) | `resolve`: a dashboard's `chart:` entries made into components |
| `crates/brightfield-shell/src/node_rows.rs` (new) | The rows of a node: a snapshot from the run, a preview run, or a sample run |
| `crates/brightfield-shell/src/four_square.rs` (new) | The four suggestions for a node's columns, and the four-square's drawing |
| `crates/brightfield-shell/src/draft.rs` (new) | `ChartDraft`: the unsaved chart or dashboard, its name and its spec text |
| `crates/brightfield-shell/src/chart_rename.rs` (new) | Renaming a chart file and the dashboard entries naming it |
| `crates/brightfield-shell/src/chart_kinds.rs` | `rebind_source` |
| `crates/brightfield-shell/src/protocol.rs` | `ProtocolInputs::add_chart_files`, `hold_node`; widened `charts_reading`; `tables_read_by` |
| `crates/brightfield-shell/src/window.rs` | `CanvasHolds::Draft`, moving to a node, the chooser, Save of a draft, `Leaving::Focus` |
| `crates/brightfield-keys/src/{registry,activity,menus}.rs` | `new-chart`, `rename-chart`; the Chart activity's needs |
| `docs/charts-and-dashboards.md` (new) | The folded record |

---

### Task 1: A chart is a kind of node, and chart files become nodes of the graph

**Files:**
- Create: `crates/brightfield-protocol/src/chart_file.rs` (export from `lib.rs`)
- Modify: `crates/brightfield-protocol/src/graph.rs:61-92` (enum and `is_tabular` unchanged), near `:708` (ids, `add_chart_files`)
- Modify: `crates/brightfield-protocol/src/panel.rs:146` (`kind_label` → `"chart"`), `layout.rs:219` (height 36.0 with Table and Dashboard), `crates/brightfield-render/src/asset_scene.rs:577` (draw Chart with the Dashboard silhouette and no header rule), `crates/brightfield-shell/src/protocol.rs:6318` (inspector line `"chart · one plot with a mosaic spec of its own"`)
- Test: `crates/brightfield-protocol/tests/chart_files.rs` (new)

**Interfaces:**
- Produces:
  - `pub enum AssetKind { …, Chart }`
  - `pub enum ChartFileKind { Chart, Dashboard }`
  - `pub struct ChartFile { pub kind: ChartFileKind, pub name: String, pub reads: Vec<String>, pub names: Vec<String> }`. `reads` holds the table names the file's marks read and its own `data:` does not define. `names` holds the chart names a dashboard's `chart:` entries give, and is empty for a chart.
  - `pub fn chart_node_id(proto: &str, name: &str) -> AssetId` (`chart.<proto>.<name>`)
  - `pub fn saved_dashboard_id(proto: &str, name: &str) -> AssetId` (`dashboard.<proto>.<name>`)
  - `pub fn add_chart_files(graph: &mut AssetGraph, proto: &str, files: &[ChartFile]) -> Vec<AssetId>`, which returns the ids added, in file order

- [ ] **Step 1: Write the failing tests** in `tests/chart_files.rs`, over `build_graph` of the fixture manifest these tests already use (see `tests/fixture_graph.rs` for the builder):
  - `a_chart_file_is_a_chart_node_under_each_table_it_reads`: a chart reading `widgets` and `orders` gives one `AssetKind::Chart` node labelled with its name and edges `asset.<p>.widgets → chart.<p>.x` and `asset.<p>.orders → chart.<p>.x`, in `reads` order.
  - `a_dashboard_naming_a_chart_hangs_off_the_chart`: the edge `chart.<p>.x → dashboard.<p>.d`, with no table edge unless `reads` names one.
  - `a_table_no_node_matches_gives_no_edge_and_keeps_the_node`: the node is added and has no in-edge.
  - `a_dashboard_named_for_its_table_is_the_generated_dashboards_node`: `saved_dashboard_id(p, "widgets") == generated_dashboard_id(&"asset.<p>.widgets")`.
  - `add_chart_files_twice_adds_nothing_the_second_time`.
  - Match a table by its node label, ignoring case, among `is_tabular` nodes, because DuckDB names ignore case.
- [ ] **Step 2: Run them to see them fail.** Run `cargo test -p brightfield-protocol --test chart_files`. Expect a compile failure: `ChartFile` is not defined.
- [ ] **Step 3: Implement** the variant, the two id functions and `add_chart_files`. Edges carry `via: None, shield: false`, as `add_generated_dashboard` writes them. Add the `Chart` arm to every exhaustive match the compiler names.
- [ ] **Step 4: Run** `cargo test -p brightfield-protocol` and `cargo test -p brightfield-render`. Expect PASS, with `contract_view.rs` unchanged.
- [ ] **Step 5: Commit**: "A chart is a node of the graph, and chart and dashboard files become nodes".

### Task 2: Brightfield reads `charts/` and `dashboards/` when it opens a Protocol, and "reads the node" includes them

**Files:**
- Create: `crates/brightfield-shell/src/chart_files.rs`
- Modify: `crates/brightfield-shell/src/protocol.rs:278` (beside `hold_table`), `:2706` (`charts_reading`), `:2727` (`table_read_by`)
- Modify: `crates/brightfield-shell/src/one_step.rs:392-430` (`folder_inputs`), `crates/brightfield-shell/src/protocol.rs:469` (`load_protocol_offline`), `window.rs:868` (`agent_protocol_inputs`)
- Modify: `crates/brightfield-shell/src/window.rs:9192-9260` (`node_in_focus`, `chart_reading`, `chart_for`)
- Test: `crates/brightfield-shell/tests/chart_nodes.rs` (new)

**Interfaces:**
- Consumes: Task 1's `ChartFile`, `add_chart_files`, `chart_node_id`.
- Produces:
  - `pub fn chart_files_in(dir: &Path, skip: Option<&str>) -> Vec<ChartFile>`. It lists `charts/*.yaml` then `dashboards/*.yaml`, top level only and sorted by name, and passes over the dashboard named `skip` (the held table's own; ruling 2). `reads` comes from `protocol_panels::protocol_tables` and `names` from `dashboard_file::chart_names`. Until Task 3 lands, that is a stub returning an empty list in `dashboard_file.rs`.
  - `ProtocolInputs::add_chart_files(&mut self, files: &[ChartFile])`, applied to all four graphs as `hold_table` does
  - `ProtocolModel::charts_reading(&self, table: &AssetId) -> Vec<AssetId>`. It lists the Chart and Dashboard nodes an edge out of `table` reaches, then the Dashboard nodes reached from those charts, in edge order and without duplicates.
  - `ProtocolModel::tables_read_by(&self, chart: &AssetId) -> Vec<&AssetId>`. It lists the tabular nodes feeding a chart or dashboard, directly or through a chart. `table_read_by` stays as its first element.
  - `FocusedNode::Chart { node, table }` now holds a Chart node or a Dashboard node.

- [ ] **Step 1: Write the failing tests**, using `scenes::own_files()` with files written into its folder before `Window::open`:
  - `a_saved_chart_comes_back_on_reopen_as_a_spine_row_under_its_table`: write `charts/price_hist.yaml` reading the scene's table by name. Reopen. The spine has a row `price_hist` with kind `chart` at depth 0 after the table's rows.
  - `charts_reading_lists_charts_then_the_dashboards_that_arrange_them`: a chart `a`, a dashboard `d` naming `a`, and a dashboard `e` reading the table directly. Expect the order `chart.a`, `dashboard.e`, `dashboard.d` (edge order: the table's edges first, then the charts').
  - `the_dashboard_saved_under_the_tables_name_is_not_read_twice`: `dashboards/<table>.yaml` adds no second node.
  - `an_agent_panel_protocol_still_opens_as_tiles` (G8): the `protocol_panels_window.rs` fixture still gives tiles and has no chart nodes.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test chart_nodes`. Expect FAIL.
- [ ] **Step 3: Implement.** In `folder_inputs`, call `add_chart_files(&chart_files_in(dir, Some(&self.name)))` after `hold_table`. The two other load routes take `skip: None`. `chart_for` accepts any `FocusedNode::Chart` whose node is in `charts_reading(table)`; the single-table restriction goes in Task 6.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell --test chart_nodes --test activity_switcher --test navigator_spine --test protocol_panels_window`. Expect PASS.
- [ ] **Step 5: Commit**: "Saved charts and dashboards come back on reopen as nodes under the tables they read".

### Task 3: A dashboard arranges charts by name

**Files:**
- Create: `crates/brightfield-shell/src/dashboard_file.rs`
- Modify: wherever a `dashboards/` file is loaded for drawing. This is `Boot::read_saved_chart` (`window.rs:1776`) → `load_saved_chart` (`:20282`) and the open-chart path Task 10 adds. Resolve the text before `parse_spec`.
- Test: unit tests in `dashboard_file.rs`; `crates/brightfield-shell/tests/dashboard_names_charts.rs` (new)

**Interfaces:**
- Produces:
  - `pub fn chart_names(text: &str) -> Vec<String>`, the `chart:` entries in layout order
  - `pub struct Resolved { pub text: String, pub missing: Vec<String> }`
  - `pub fn resolve(text: &str, charts_dir: &Path) -> Result<Resolved, String>`. It errors only when `text` is not a YAML mapping.
  - `pub const MISSING_CHART: &str = "No chart named {name} in charts/."`

- [ ] **Step 1: Write the failing tests**:
  - `a_named_chart_is_put_in_its_place`: a dashboard with `hconcat: [{chart: a}, {chart: b}]` resolves to the same `Spec` as the hand-written inline form. Compare `parse_spec` outputs.
  - `the_dashboards_own_data_entry_wins_a_clash`.
  - `a_missing_chart_draws_a_sentence_in_its_place`: `missing == ["gone"]`, and the resolved spec holds a `text` mark whose inline row reads `No chart named gone in charts/.`
  - `a_dashboard_with_no_chart_entries_is_handed_back_byte_for_byte`.
  - Window test `a_dashboard_naming_two_charts_draws_both_and_they_cross_filter`: brush one, and the other's `filterBy` count changes. Use the existing cross-filter assertion helper in `tests/support/scenes.rs`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell dashboard_file` and `cargo test -p brightfield-shell --test dashboard_names_charts`. Expect FAIL.
- [ ] **Step 3: Implement** over `serde_yaml::Value`. A chart's root component is every top-level key that is not `meta|data|params|config|plotDefaults`, the split `parse.rs:1365` makes.
- [ ] **Step 4: Run both again.** Expect PASS.
- [ ] **Step 5: Commit**: "A dashboard can arrange saved charts by name, and says which one is missing".

### Task 4: Save writes `dashboards/`, and a Protocol saved before reopens as it did

**Files:**
- Modify: `crates/brightfield-model/src/panel_capture.rs:40-130`, `crates/brightfield-shell/src/one_step.rs:247` (`chart_file`)
- Modify: `crates/brightfield-shell/src/window.rs:1776` (`read_saved_chart`: fall back to the old path), `crates/brightfield-shell/src/protocol_panels.rs:338` (`agent_panels_in`), `crates/brightfield-shell/src/rename.rs:448` (`repair_panels` also walks `charts/` and `dashboards/`)
- Test: update `crates/brightfield-shell/tests/cmd_s_saves_the_protocol.rs`, `chart_save_writes_chart.rs`, `chart_save_reopens_chart.rs`; add `crates/brightfield-shell/tests/older_panels_reopen.rs`

**Interfaces:**
- Produces:
  - `pub const CHARTS_DIR: &str = "charts"` and `pub const DASHBOARDS_DIR: &str = "dashboards"`
  - `pub fn chart_file(dir: &Path, name: &str) -> PathBuf`
  - `pub fn dashboard_file(dir: &Path, name: &str) -> PathBuf`
  - `pub fn table_dashboard_file`, which gives `dashboards/tables/<slug>.yaml`
  - `pub fn join_dashboard_file`, which gives `dashboards/joins/<slug>.yaml`
  - `OneStepProtocol::chart_file` returns these
  - `pub fn older_chart_file(&self) -> PathBuf`, the `panels/` path the same match gave before

- [ ] **Step 1: Update and write the tests**:
  - The existing Save tests now expect `dashboards/…` paths.
  - `nothing_is_written_under_panels_by_any_save` sweeps every Save test's folder.
  - `a_protocol_saved_into_panels_reopens_on_its_saved_dashboard`.
  - `a_protocol_saved_into_panels_is_not_opened_as_tiles`.
  - `saving_a_protocol_that_has_an_older_panel_writes_dashboards_and_leaves_the_panel`: the old file is byte-identical, and the reopen prefers `dashboards/`.
  - `rename_repairs_file_paths_in_charts_and_dashboards`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test cmd_s_saves_the_protocol --test older_panels_reopen --test rename_protocol`. Expect FAIL.
- [ ] **Step 3: Implement.** `read_saved_chart` tries `chart_file()` and then `older_chart_file()`.
- [ ] **Step 4: Run** the same tests plus `cargo test -p brightfield-model` and `--test chart_save_reopens_chart --test chart_save_reopens_relative --test protocol_panels`. Expect PASS.
- [ ] **Step 5: Commit**: "Save writes a dashboard to dashboards/, and one saved to panels/ still reopens".

### Task 5: The rows of any node, from the Protocol's last run

**Files:**
- Create: `crates/brightfield-shell/src/node_rows.rs`
- Modify: `crates/brightfield-shell/src/protocol_panels.rs:870` (`snapshot_tables` → `pub(crate)`)
- Test: unit tests in `node_rows.rs` over a run fixture already used by `tests/protocol_panels.rs`

**Interfaces:**
- Produces:
  - `pub enum NodeRows { Ready { tables: Vec<(String, PathBuf)>, sample: bool }, NotReached { tables: Vec<String> } }`
  - `pub fn rows_from_run(dir: &Path, tables: &[String], into: &Path) -> Result<NodeRows, String>`. Each table that holds rows in the Protocol's database is snapshotted as `into/<lower>.parquet`, under the name `protocol_panels::with_protocol_tables` expects. The result is `NotReached` listing the tables the database lacks, or all of them when there is no database. It errors only on a failed copy.
  - `pub fn inject(spec: &str, rows: &NodeRows) -> Result<String, String>`, a thin call to `with_protocol_tables`

- [ ] **Step 1: Write the failing tests**:
  - `a_run_table_is_snapshotted_under_its_lower_case_name`
  - `a_protocol_with_no_database_has_every_table_not_reached`
  - `a_table_the_run_did_not_make_is_named_as_not_reached_and_the_rest_are_ready`: two tables, one made. Expect `NotReached{["b"]}`. Partial readiness reports not reached, so the caller runs first.
  - `the_database_is_left_unlocked_after_a_snapshot`: `arc` can open it read-write after the call, as `arc_can_run_while_the_protocol_is_open` pins.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell node_rows`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run it again.** Expect PASS.
- [ ] **Step 5: Commit**: "Any node's rows can be read from the Protocol's last run".

### Task 6: The window moves to the node in focus

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs:278` (add `hold_node`)
- Modify: `crates/brightfield-shell/src/window.rs`:
  - `perform_activity` `:9271`
  - the grid branch at `:7830-7868`, where `grid_unheld` goes
  - `draw_unheld_grid_pane` `:21484`, kept only for `NodeRows::NotReached` until Task 7
  - a new `MeridianApp::move_to_node`
- Test: `crates/brightfield-shell/tests/move_to_node.rs` (new); update `tests/navigator_spine.rs:2748`

**Interfaces:**
- Consumes: Task 5's `rows_from_run`, `NodeRows`.
- Produces:
  - `ProtocolInputs::hold_node(&mut self, table: AssetId)`. It sets `table` and adds no generated dashboard (spec: "goes through a path that does not call `add_generated_dashboard`").
  - `pub fn move_to_node(&mut self, ctx: &egui::Context, node: &AssetId, then: NodeActivity) -> bool`. It goes through `leave(Leaving::Focus { node, then })` (Task 12 adds the question; until then `go` runs it straight away). It then snapshots the node's rows into `<dir>/.grid/`, opens the snapshot as the held table through the `open_model_in_grid` route with `hold_node` in place of `hold_table`, keeps the Protocol's spine, and performs `then`.
  - `MeridianApp::opened_as(&self) -> Option<&AssetId>`, the table the file opened as. Moving back to it reopens through the original `Boot::open` route, with its generated dashboard and saved chart.

- [ ] **Step 1: Write the failing tests** on `scenes::own_files()` after a run (the scene's run fixture):
  - `grid_over_a_run_view_lists_its_rows`: the second table's grid shows its row count, not `No rows for`.
  - `moving_to_another_node_adds_no_dashboard_to_it`: `charts_reading(view)` is empty.
  - `moving_back_to_the_opened_table_brings_back_its_generated_dashboard`: Review Focus 2.
  - `the_spine_and_the_selection_survive_a_move`.
  - Update `each_table_carries_a_grid_row_and_its_click_lands_on_that_nodes_grid` to expect rows where the run made them.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test move_to_node`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `--test move_to_node --test navigator_spine --test activity_switcher --test model_pane --test spine_beside_the_model`. Expect PASS.
- [ ] **Step 5: Commit**: "Grid and Chart over another node move the window to that node's rows".

### Task 7: A node no run has reached runs on its own as a preview, and a network source is sampled

**Files:**
- Modify: `crates/brightfield-shell/src/node_rows.rs` (add the run and sample paths)
- Modify: `crates/brightfield-shell/src/window.rs` (the pane while a preview runs; the sample band; *Run in full* → `run_protocol` `:17573`)
- Modify: `crates/brightfield-shell/src/starts.rs:1594` (reuse `is_fetch_op`, `fetch_steps`)
- Test: `crates/brightfield-shell/tests/node_preview_run.rs` (new), with the local HTTP fixture `tests/remote_start.rs` serves from

**Interfaces:**
- Consumes: Task 6's `move_to_node`.
- Produces:
  - `pub enum PreviewPlan { Run, Sample { fetches: Vec<String> }, CannotSample { source: String } }`
  - `pub fn preview_plan(dir: &Path, manifest: &arc::spec::Manifest) -> PreviewPlan`. It decides from fetch steps whose outputs are not on disk and the URL's extension. Rows are `.csv`, `.tsv`, `.parquet`, `.json` and `.ndjson`, with any query string ignored.
  - `pub const SAMPLE_ROWS: u64 = 1_000`
  - `pub fn sample_run(dir: &Path, runner: &Runner, scratch: &Path) -> Result<PathBuf, String>`, which returns the scratch folder whose database holds the sampled run
  - `NodeRows::Ready { sample: true, .. }` when the rows came from it
  - The copy: `pub const RUNNING_PREVIEW: &str = "Running the Protocol to show {label}…"`, `pub const SAMPLE_NOTE: &str = "A sample: the first 1,000 rows of {source}."`, `pub const RUN_IN_FULL: &str = "Run in full"`, `pub const NOT_SAMPLEABLE: &str = "{source} is not rows Brightfield can sample; a full run fetches it."`

- [ ] **Step 1: Write the failing tests**:
  - `a_node_no_run_reached_starts_a_preview_run_and_then_shows_its_rows`: the run record is listed in `.previews`, and the ledger reads `last preview`.
  - `the_pane_says_what_it_is_waiting_for_while_the_preview_runs`.
  - `a_fetch_upstream_reads_only_its_first_thousand_rows_and_says_so`: the fixture serves 5,000 CSV rows. Exactly 1,000 reach the grid, the sample note shows, and the real folder has no fetched file and no new run record.
  - `run_in_full_starts_the_analysts_run`.
  - `a_zip_source_offers_a_full_run_and_fetches_nothing`.
  - `a_refused_url_says_why_and_still_offers_run_in_full`: Review Focus 5.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test node_preview_run`. Expect FAIL.
- [ ] **Step 3: Implement.** The run goes on a worker that wakes the frame, as `run_protocol` does. One preview runs at a time per window, and a second move while one runs waits for it. The scratch copy goes under `tempfile` and skips `build/`. Fetch steps are dropped with arc's format-preserving manifest write.
- [ ] **Step 4: Run it again.** Expect PASS.
- [ ] **Step 5: Commit**: "A node no run has reached runs as a preview, and a network source is read as a 1,000-row sample".

### Task 8: The four suggestions for a node's columns

**Files:**
- Create: `crates/brightfield-shell/src/four_square.rs` (the suggestion half; drawing is Task 9)
- Modify: `crates/brightfield-shell/src/chart_kinds.rs` (`rebind_source`)
- Test: unit tests in `four_square.rs` and `chart_kinds.rs`

**Interfaces:**
- Consumes: `chart_kinds::registry()`, `fields_of`, `ChartKind::bind`, `ChartKind::spec`, `Dashboard::of`, `Dashboard::to_spec`.
- Produces:
  - `pub fn rebind_source(spec: &str, table: &str) -> Result<String, String>`. It parses the spec, rewrites every `data.from` equal to `data_file::SOURCE` to `table`, drops a top-level `data:` entry named `SOURCE`, and serialises.
  - `pub enum Cell { Chart { kind: ChartKindId, title: String, spec: String }, Dashboard { spec: String }, Reason { line: String } }`
  - `pub fn suggestions(columns: &[ColumnProfile], table: &str, path: &Path) -> [Cell; 4]`:
    - Cells 0 to 2 are the first three of `registry().applicable(&fields_of(columns))`, in order, each bound and built with the kind's default options, then rebound.
    - The cells left unfilled take ruling 5.
    - Cell 3 is `Dashboard::of(path, columns).to_spec()`, rebound.
  - `title` is the kind's `description`.

- [ ] **Step 1: Write the failing tests**:
  - `the_first_three_applicable_kinds_fill_the_chart_cells_in_registry_order`, over two measures, a date and a category. Expect `binned_histogram`, `scatter`, `counts_over_time`.
  - `one_category_fills_one_cell_and_says_why_for_the_other_two`: each `Reason` line starts with the description of the next inapplicable kind and holds its bind error.
  - `the_fourth_cell_is_the_generated_dashboard_over_the_node_by_name`.
  - `no_suggested_spec_reads_opened`: every cell's spec, parsed, has no `from: opened`.
  - `a_column_named_with_a_quote_and_a_space_binds_and_parses`: Review Focus 3, `price ($)`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell four_square` and `cargo test -p brightfield-shell rebind_source`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run both again.** Expect PASS.
- [ ] **Step 5: Commit**: "The suggestion four-square's cells come from the chart kinds that fit a node's columns".

### Task 9: A draft chart opens on the four-square, and picking a cell fills the shelf

**Files:**
- Create: `crates/brightfield-shell/src/draft.rs`
- Modify: `crates/brightfield-shell/src/four_square.rs` (drawing), `crates/brightfield-shell/src/window.rs`:
  - add `CanvasHolds::Draft` at `:1196`
  - the draw path beside `carve_shelf_band` at `:21239`
  - widen `hero_shelf_channels` at `:21290`
- Modify: `crates/brightfield-shell/src/app.rs:1921` (`has_unsaved_work` counts an edited draft)
- Test: `crates/brightfield-shell/tests/four_square.rs` (new); snapshot baselines `tests/snapshots/four_square_{light,dark}.png` via `tests/snapshot.rs`

**Interfaces:**
- Consumes: Task 8's `suggestions`, `Cell`; Task 6's held node.
- Produces:
  - `pub enum DraftKind { Chart, Dashboard }`
  - `pub struct ChartDraft { pub kind: DraftKind, pub name: String, pub table: AssetId, pub picked: bool }`
  - `pub fn draft_name(dir: &Path, label: &str, suffix: &str, kind: DraftKind) -> String` (ruling 8)
  - `pub const EMPTY_DRAFT: &str`, a spec with `params: sel: {select: crossfilter}`, one plot and one `dot` mark reading `{table}` with no channel. The shelf reads three *add a column* cells.
  - `CanvasHolds::Draft { table: AssetId }`, with the draft itself held as `ChartView.draft: Option<ChartDraft>`
  - `MeridianApp::open_draft(&mut self, ctx, table: &AssetId) -> bool`
  - `MeridianApp::pick_suggestion(&mut self, ctx, cell: usize) -> bool`
  - `MeridianApp::four_square_cells(&self) -> Option<[String; 4]>`, a test hook giving each cell's title or reason line
  - `four_square::draw(ui, area, cells, host, mode) -> Option<usize>`, which returns the cell picked by click or Enter. Arrow keys move between cells while the chart pane has focus.

- [ ] **Step 1: Write the failing tests**:
  - `a_draft_shows_the_shelf_and_the_four_square_with_no_typing`: the shelf band is drawn and reads `add a column` three times, and `four_square_cells()` is `Some`.
  - `putting_a_column_on_the_shelf_replaces_the_four_square_with_the_chart`.
  - `picking_a_chart_cell_puts_its_plot_on_the_shelf_and_marks_the_draft_edited`: the shelf channels equal the kind's binding, the title carries the unsaved mark, and `four_square_cells()` is `None`.
  - `picking_the_dashboard_cell_makes_a_draft_dashboard_drawn_as_the_page`.
  - `an_untouched_draft_is_not_unsaved` (ruling 9).
  - `enter_picks_the_cell_the_arrows_moved_to`.
  - Snapshot `four_square` in light and dark at 1280×800.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test four_square`. Expect FAIL.
- [ ] **Step 3: Implement** (ruling 7 for drawing). The cell frames use `chrome::pane_frame` and the same cell gap as `panel_tiles`, without its minimum size.
- [ ] **Step 4: Run** `--test four_square --test shelf_band --test shelf_band_window`. Expect PASS. Then draw the baselines with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test snapshot four_square`, and look at both PNGs before committing them.
- [ ] **Step 5: Commit**: "A new chart opens on the suggestion four-square under a live shelf".

### Task 10: Chart over any table or view, the chooser, and New chart

**Files:**
- Modify: `crates/brightfield-keys/src/registry.rs:341` (`NEEDS_CHART_ACTIVITY` is `Selected::NodeRows` only), the `vec![]` at `:361` (add `new-chart`), `activity.rs`, `menus.rs` (*New chart* in the Chart menu after *Show chart*), `tests/data/keymap-research.md`
- Modify: `crates/brightfield-shell/src/window.rs`:
  - `perform_activity` `:9271`
  - a new `WhichChartSheet` beside `WhichProtocolSheet` `:2383`
  - its `Overlay` arm beside `:10744`
  - the name map `:12268`
- Test: update `crates/brightfield-shell/tests/activity_switcher.rs`; add `crates/brightfield-shell/tests/which_chart.rs`

**Interfaces:**
- Consumes: Task 2's `charts_reading`; Task 6's `move_to_node`; Task 9's `open_draft`; Task 3's `resolve` (to open a dashboard naming charts); Task 5 for each table it reads.
- Produces:
  - `pub const NEW_CHART: &str = "new-chart"` in `activity.rs`, not in `ACTIVITY_VERBS`
  - `struct WhichChartSheet { table: AssetId, choices: Vec<AssetId>, selected: usize, rows: Vec<egui::Rect> }`. Each row reads `<label> · chart` or `<label> · dashboard`, and the last row is `pub const WHICH_CHART_NEW: &str = "New chart"`.
  - `MeridianApp::which_chart_choices(&self) -> Option<Vec<String>>` (test hook)
  - SHOW_CHART over a table: none reading it → `open_draft`; one → open it (moving first); several → the sheet. NEW_CHART → `open_draft` whatever reads the node.

- [ ] **Step 1: Write the failing tests**:
  - Replace `chart_over_a_node_no_chart_reads_is_greyed_on_the_menu_and_by_its_offer` with `chart_over_a_node_nothing_reads_opens_a_draft_on_the_four_square`.
  - Keep `chart_opens_the_one_chart_that_reads_the_node`.
  - `several_readers_ask_which_with_new_chart_last`: the order is `charts_reading`'s.
  - `new_chart_opens_the_four_square_over_a_node_a_chart_reads`.
  - `cmd_alt_n_is_new_chart_and_answers` through `keys_answered.rs`.
  - `opening_a_chart_that_reads_two_tables_draws_both`: Review Focus 1.
  - In `crates/brightfield-keys`, keep the registry gates green and extend the longname snapshot.
- [ ] **Step 2: Run** `cargo test -p brightfield-keys` and `cargo test -p brightfield-shell --test activity_switcher --test which_chart --test keys_answered --test menu_commands`. Expect FAIL.
- [ ] **Step 3: Implement.** Remove `Condition::ChartReadsFocus` if nothing else sets it, together with its `default_sentence` arm.
- [ ] **Step 4: Run them again**, plus `--test availability`. Expect PASS.
- [ ] **Step 5: Commit**: "Chart works over any table or view, asks which when several read it, and New chart opens the four-square".

### Task 11: Cmd+S saves a draft to `charts/` or `dashboards/` and adds its node

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs`: `save_everything` `:9058`, `anything_to_save` `:9078`, `save_protocol` `:16651`
- Modify: `crates/brightfield-shell/src/app.rs:2450` (`save_chart_at`, used with the draft's path; the injected tables come out through `strip_protocol_tables`)
- Test: `crates/brightfield-shell/tests/draft_save.rs` (new)

**Interfaces:**
- Consumes: Task 4's `chart_file` and `dashboard_file`; Task 9's `ChartDraft`; Task 2's re-read.
- Produces: `MeridianApp::save_draft(&mut self, ctx) -> Option<Result<PathBuf, String>>`. It runs after the Protocol's own save in `save_everything`. It writes the draft, re-reads the folder through `adopt_saved_protocol`, and sets `canvas_holds` to `Dashboard { node: chart_node_id(..) | saved_dashboard_id(..), table }`.

- [ ] **Step 1: Write the failing tests**:
  - `cmd_s_writes_the_draft_chart_to_charts_reading_its_table_by_name`: the file parses, holds no `file:` entry for the table, and its marks read `from: <table>`.
  - `the_saved_chart_is_a_spine_row_and_holds_the_canvas`.
  - `cmd_s_on_a_draft_dashboard_writes_dashboards`.
  - `a_draft_saved_over_a_data_file_saves_the_protocol_first`: Review Focus 4; one press writes the manifest and `charts/…`.
  - `a_second_draft_of_the_same_kind_takes_the_next_name`.
  - `nothing_is_written_under_panels`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test draft_save`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `--test draft_save --test cmd_s_saves_the_protocol --test chart_save_writes_chart`. Expect PASS.
- [ ] **Step 5: Commit**: "Cmd+S saves a new chart to charts/ and a new dashboard to dashboards/".

### Task 12: Leaving an edited draft asks first

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs:2556` (`Leaving::Focus { node: AssetId, then: NodeActivity }`), `leave` `:11603`, `go`; and `:5164` (`has_unsaved_work` includes an edited draft)
- Modify: `crates/brightfield-shell/src/overlays.rs:~892` (`AskedBefore::MovingNode`)
- Test: extend `crates/brightfield-shell/tests/close_question_every_way_out.rs`

**Interfaces:**
- Consumes: Task 6's `move_to_node`; Task 9's draft; Task 11's `save_draft` (Save and move).

- [ ] **Step 1: Write the failing tests**:
  - `moving_to_another_node_with_an_edited_draft_asks`
  - `save_and_move_writes_the_draft_then_moves`
  - `move_without_saving_discards_the_draft_and_writes_nothing`
  - `keep_editing_stays_on_the_draft`
  - `opening_another_chart_with_an_edited_draft_asks`
  - `closing_the_window_with_an_edited_draft_asks`
  - `an_untouched_draft_moves_without_asking`
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test close_question_every_way_out`. Expect FAIL.
- [ ] **Step 3: Implement.** The question's wording reads `CloseQuestion` as it is. Its `chart_edits` count is 1 for a picked draft.
- [ ] **Step 4: Run** it plus `--test close_question_keeps_unsaved --test close_question_baseline --test chart_save_close_asks`. Expect PASS.
- [ ] **Step 5: Commit**: "Leaving an unsaved new chart asks first".

### Task 13: Rename a chart

**Files:**
- Create: `crates/brightfield-shell/src/chart_rename.rs`
- Modify: `crates/brightfield-keys/src/registry.rs` (`rename-chart`, label *Rename chart*, no key, needs `Selected::Chart`), `menus.rs`; `crates/brightfield-shell/src/window.rs` (the name field, as `rename_protocol` `:15435` takes one)
- Test: unit tests in `chart_rename.rs`; `crates/brightfield-shell/tests/rename_chart.rs`

**Interfaces:**
- Produces: `pub fn rename_chart(dir: &Path, kind: DraftKind, from: &str, to: &str) -> Result<PathBuf, String>`. It refuses a name already in the folder, or one with nothing left after slugging. It renames the file, and for a chart rewrites each `chart: <from>` in `dashboards/*.yaml`. A draft's rename sets `ChartDraft.name` and writes nothing.

- [ ] **Step 1: Write the failing tests**:
  - `renaming_a_chart_rewrites_the_dashboards_naming_it_and_nothing_else`
  - `a_name_taken_in_the_folder_is_refused_with_a_sentence`
  - `renaming_a_draft_writes_nothing_and_the_save_uses_the_new_name`
  - `the_renamed_charts_spine_row_reads_the_new_name`
- [ ] **Step 2: Run** `cargo test -p brightfield-shell chart_rename` and `--test rename_chart`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again**, plus `cargo test -p brightfield-keys`. Expect PASS.
- [ ] **Step 5: Commit**: "A chart can be renamed, and the dashboards that name it follow".

### Task 14: Fold the record and run the gate

**Files:**
- Create: `docs/charts-and-dashboards.md`. It says what a chart and a dashboard are, the folders, how a dashboard names a chart, moving to a node, preview and sample runs, the four-square, and drafts. It gives each of Hugh's rulings from the spec with his reason, and the plan's rulings above.
- Modify: `CLAUDE.md` `## Docs`: add one line saying when to read the new doc.
- Delete: `docs/superpowers/specs/2026-11-26-chart-four-square-design.md` and this plan.

- [ ] **Step 1:** Write the doc and the CLAUDE.md line. Delete the spec and the plan.
- [ ] **Step 2:** Run `cargo xtask ci --step public-hygiene` and `--step comment-citations`. Expect PASS.
- [ ] **Step 3:** Push the branch and run `cargo xtask gate <pr>` on the Mac Studio. Expect `local-gate` to be green.
- [ ] **Step 4: Commit**: "Record how charts, dashboards and the four-square work, and retire the spec and plan".

## Task order

- Tasks 1 → 2 → 3 come first.
- Task 4 can run in parallel with Task 3.
- Task 5 depends on nothing, then 6 → 7.
- Task 8 depends on nothing.
- Task 9 needs 6 and 8.
- Task 10 needs 2, 3, 6 and 9.
- Task 11 needs 4 and 9.
- Task 12 needs 6, 9 and 11.
- Task 13 needs 11.
- Task 14 comes last.

## Verification cost

- A `brightfield-protocol` or `brightfield-keys` test run takes seconds after the first build.
- A first build of a new shell test target takes a few minutes, and each later run under a minute.
- Snapshot redraws take about a minute and need the Mac Studio's GPU.
- `cargo xtask gate <pr>` takes the machine's cores for roughly 20 to 40 minutes, and only one gate runs at a time.
