# The Protocol's spine beside the model — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Protocol of two or more sources shows its model-first spine on the navigator rail beside the model pane, and keeps showing it while one of its tables is in the grid.

**Architecture:** One spine-list drawing function is extracted from `OutlinePane::ui` and called by both the dock's navigator and a new navigator rail drawn in the model branch of the central draw. `ModelDoc` holds a `ModelSpine` (the `Spine` and its caption) built from the Protocol's manifest plus the pane's in-memory model joins, rebuilt when the pane's generation or its last run changes. While a table is in the grid, the window hands that spine to the Protocol document for the frame, and clicks come back as a request the window drains.

**Tech Stack:** Rust, egui (headless `egui::Context` in tests), crate `brightfield-shell`.

**Spec:** `docs/superpowers/specs/2026-11-13-spine-beside-the-model-design.md`

## Global Constraints

- Toolchain is pinned by `rust-toolchain.toml`; never `cargo fmt --all`. Format with `cargo xtask ci --step fmt`, lint with `cargo xtask ci --step clippy`.
- One cargo build at a time; no parallel implementers.
- No private planning identifiers in code, comments, commits or PR text. This repo is public.
- Code comments state facts the code holds, in plain English; `cargo xtask ci --step comment-citations` runs after each commit.
- The spine's ordering rule (`model_first`), row drawing (`spine_row`) and curve routing (`curve_lanes`, `paint_join_curve`) are not changed in behaviour.

## Review Focus

- A Protocol whose model makes no join: beside the model, the spine keeps the outline's order with no model step row; nothing breaks and no click is lost. Task 3 pins it.
- The navigator rail collapsed beside the model pane: the stub draws, no spine rows are recorded, and reopening restores it. Task 3 pins it.
- Back to the model refused because the grid's dashboard has unsaved edits: a click on the model row or another table's row leaves the grid where it is and the banner says why. Task 4 pins it.
- A manifest that fails to load (deleted mid-session): the spine is empty and the model pane still draws. Task 2 pins it.
- The named-controls record (`name_controls`) lists the spine rows drawn beside the model, so each control there still has a name. Task 3 pins it.

---

### Task 1: One spine-list drawing function

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`OutlinePane::ui`, around 3983–4100; `spine_row`, around 4400)

**Interfaces:**
- Produces, in `protocol.rs`:
  ```rust
  pub struct SpineListDrawn {
      pub rows: Vec<SpineRowDrawn>,      // head row first, then each spine row, as OutlinePane records them now
      pub curves: Vec<SpineCurveDrawn>,
      pub clicked: Option<usize>,        // index into the `Spine::rows` passed in
      pub graph_picked: bool,            // the head row's chip was clicked
  }
  pub struct SpineListStyle<'a> {
      pub caption: &'a str,
      pub holds_graph: bool,             // head row's on-canvas bar
      pub chip_live: bool,
      pub shows: &'a dyn Fn(&SpineRow) -> bool,          // the on-canvas bar per row
      pub senses_click: &'a dyn Fn(&SpineRow) -> bool,   // which rows are controls
  }
  pub(crate) fn spine_list_ui(ui: &mut egui::Ui, spine: &Spine, style: &SpineListStyle<'_>, mode: Mode) -> SpineListDrawn
  pub(crate) fn default_senses_click(row: &SpineRow) -> bool // Asset | View, today's rule
  ```
- `spine_row` gains a `senses_click: bool` parameter in place of its own role match.

- [ ] **Step 1:** Extract the head row, the spine-row loop (including the `Unjoined` caption rows) and the curve painting from `OutlinePane::ui` into `spine_list_ui`. `OutlinePane::ui` calls it inside its scroll area and interprets `clicked` exactly as it does today (view row → canvas pick, dashboard row → dashboard pick, asset row → `select_id`).
- [ ] **Step 2:** Run the spine's existing tests unchanged:
  `cargo test -p brightfield-shell --lib protocol -- --test-threads=1` and `cargo test -p brightfield-shell --test grid_spot --test protocol_panels_window --test square_look --test text_ink_frame -- --test-threads=1`. Expected: all pass, no snapshot changes.
- [ ] **Step 3:** fmt, clippy, commit ("Draw the spine's rows through one function").

### Task 2: The model pane keeps the Protocol's spine

**Files:**
- Modify: `crates/brightfield-shell/src/model_pane.rs` (add `pub fn generation(&self) -> u64`)
- Modify: `crates/brightfield-shell/src/window.rs` (`ModelDoc` around 3135; `open_model_pane` around 13670; `open_model_in_grid` around 13780; `back_to_model`)
- Test: `crates/brightfield-shell/src/window.rs` unit tests module, or `crates/brightfield-shell/tests/spine_beside_the_model.rs` (new; harness copied from `tests/model_pane.rs`: scratch dirs, `runner()`, the engine's `own_files` fixture, 1440×900 frames)

**Interfaces:**
- Produces, in `window.rs`:
  ```rust
  struct ModelSpine { spine: crate::protocol::Spine, caption: String, key: (u64, Option<String>) }
  impl ModelDoc {
      fn spine(&mut self) -> &ModelSpine   // rebuilds when (doc.generation(), last run's id) differs from key
      grid_table: Option<String>           // new field: the model table whose rows the grid holds, None for a join or the model
  }
  pub fn model_spine_rows(&self) -> Option<Vec<crate::protocol::SpineRow>> // test hook on MeridianApp: the held spine's rows, not what was drawn
  ```
- Built from `agent_protocol_inputs(&doc.protocol.dir)` with `inputs.model_joins = Some(ModelJoins::of(&doc.model))`, then `ProtocolModel::new(inputs, ..)` (use the constructor `Boot::protocol` uses) and `spine_with_joins()` / `spine_caption()`. The run identity is whatever `RunView` exposes as its id or timestamp.

- [ ] **Step 1: Failing test** `the_model_pane_holds_its_protocols_spine_model_first`: open the fixture folder as `tests/model_pane.rs` does; `model_spine_rows()` is `Some`, contains a row with `role == SpineRole::Model` and label `MODEL_STEP` when the fixture's model makes a join, and every table row (`role == Asset`, label in `pane.model.datasets`) is present. If the fixture's discovery makes no join, the test first accepts one through `perform_model_action` (see `tests/model_inspector.rs` for the action that keeps a proposed join) — record which in a comment.
- [ ] **Step 2: Failing test** `the_spine_follows_the_model_when_a_join_changes`: drop (or keep) a join through `perform_model_action`; `model_spine_rows()` on the next frame differs in order and the `Model` row's kind counts the joins.
- [ ] **Step 3: Failing test** `a_missing_manifest_leaves_an_empty_spine_and_the_pane`: delete `arcform.yaml` from the opened Protocol's folder, bump the generation by a model action, draw a frame; `model_spine_rows()` is `Some(empty)` or has no table rows, and `model_pane()` is still `Some`.
- [ ] **Step 4:** Implement; `open_model_in_grid` sets `grid_table` for a `Selection::Dataset`, `back_to_model` clears it.
- [ ] **Step 5:** `cargo test -p brightfield-shell --test spine_beside_the_model --test model_pane -- --test-threads=1`. Expected: pass.
- [ ] **Step 6:** fmt, clippy, commit.

### Task 3: The navigator rail beside the model pane

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (model branch of the central draw, around 6387)
- Test: `crates/brightfield-shell/tests/spine_beside_the_model.rs`

**Interfaces:**
- Consumes: `spine_list_ui`, `SpineListStyle` (task 1); `ModelDoc::spine`, `grid_table` (task 2); `select_source`.
- The rail is a `Panel::left` drawn before the model pane's `CentralPanel`, under the navigator region's own ids and sizes exactly as the door branch's navigator (around 6306–6360): collapsed stub, `rail_split`, `rail_selector` with the label `"Protocol"` (the `OutlinePane` subject's title). Its rect goes in `self.regions` under `navigator.id`, its strip in `self.strips`, and its rows into `self.protocol.doc.spine_drawn` / `spine_curves` / `spine_body`, so `spine_rows()`, `spine_curves()`, `region_rect` and `name_controls` read it.
- Rows are marked before drawing: the `Model` row `selected = true`; nothing else selected. `senses_click`: a row whose role is `Asset` and whose label is one of the model's tables. `shows`: the `Model` row. `chip_live: false`, `holds_graph: false`.
- A click on a table row calls `self.select_source(ctx, &label)` after the panels have drawn (collect it, act after, as `model_actions` are).

- [ ] **Step 1: Failing test** `a_protocol_of_several_sources_draws_its_spine_beside_the_model`: after open, `spine_rows()` holds the joined tables' rows, then a `Model` row with `washed == true`, then an `Unjoined` caption when a source is unjoined; every spine row's `rect.max.x <= ` the model pane's drawn rect's `min.x` (read the canvas region via `region_rect` for the canvas id); `spine_curves().len()` equals the model's non-proposal relationships between listed tables.
- [ ] **Step 2: Failing test** `a_click_on_a_tables_row_opens_its_grid_and_keeps_the_spine`: click the centre of a table row's rect (press and release frames, as `tests/new_protocol.rs::click` does); `model_behind_grid()` (use the existing public probe, or `model_pane()` plus `chart_doc().is_live()`) is true and the chart doc's title names the table.
- [ ] **Step 3: Failing test** `a_collapsed_navigator_beside_the_model_records_no_rows`: toggle the navigator's strip collapse; next frame `spine_rows()` is empty and the model pane still draws; toggle again and the rows return.
- [ ] **Step 4: Failing test** `the_spine_rows_beside_the_model_are_named_controls`: each table row's rect appears in the named-controls list (find the existing public accessor `name_controls` feeds, e.g. `named_controls()`), labelled with the table's name.
- [ ] **Step 5:** Implement. Run the new file and `--test model_pane --test model_inspector --test model_reshape --test model_file_edit --test new_protocol --test region_gate -- --test-threads=1`. A region test that asserts the model pane spans the whole canvas region is now wrong: update it to the new arrangement and name it in the commit message.
- [ ] **Step 6:** fmt, clippy, commit.

### Task 4: The spine stays the Protocol's while a table is in the grid

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`ProtocolDoc`, `OutlinePane::ui`)
- Modify: `crates/brightfield-shell/src/window.rs` (dock branch, before the navigator draws; the frame's drain of model asks)
- Test: `crates/brightfield-shell/tests/spine_beside_the_model.rs`

**Interfaces:**
- Produces, in `protocol.rs`:
  ```rust
  pub struct SpineOverride { pub spine: Spine, pub caption: String, pub selected_table: String }
  pub enum SpineAsk { Model, Table(String) }
  ProtocolDoc { pub spine_override: Option<SpineOverride>, spine_ask: Option<SpineAsk> }
  impl ProtocolDoc { pub fn take_spine_ask(&mut self) -> Option<SpineAsk> }
  ```
- The window sets `protocol.doc.spine_override` each frame while `model_behind_grid()` and `grid_table` is `Some` (cleared otherwise), from `ModelDoc::spine()`. `OutlinePane::ui` with an override: draws the override's spine through `spine_list_ui` with `selected_table`'s row selected and on-canvas, the `Model` row and other model tables as controls, `chip_live: false`; a click records `SpineAsk` instead of a canvas pick. The column outline under it is drawn from `doc.model` as now.
- Drained after the frame's panels: `SpineAsk::Model` → `back_to_model()`; `SpineAsk::Table(t)` → `back_to_model()` and, only if that returned true, `select_source(ctx, &t)`.

- [ ] **Step 1: Failing test** `the_spine_stays_the_protocols_while_a_table_is_in_the_grid`: from the model, click a table row; on the next frame `spine_rows()` lists the same labels in the same order as before the click (head row aside), with that table's row `washed`, and the `Model` row not washed.
- [ ] **Step 2: Failing test** `the_model_row_goes_back_to_the_model`: from the grid, click the `Model` row; `model_pane().is_some()` and the model branch draws (the spine's `Model` row is washed again).
- [ ] **Step 3: Failing test** `another_tables_row_opens_that_table_instead`: from table A's grid, click table B's row; the chart doc's title names B and `spine_rows()` washes B's row.
- [ ] **Step 4: Failing test** `unsaved_grid_edits_hold_the_spine_click`: make the grid's dashboard dirty (the edit `tests/model_inspector.rs` or the back-to-model test in the repo uses; search for `model-grid-unsaved`), click the `Model` row; the grid stays and the `model-grid-unsaved` notification is raised.
- [ ] **Step 5:** Implement. Run the new file plus `--test grid_spot --test new_protocol --test model_inspector --lib protocol -- --test-threads=1`.
- [ ] **Step 6:** fmt, clippy, commit.

### Task 5: Fold the design into the docs and remove the spec and plan

**Files:**
- Modify: `README.md` (the section that describes the spine and the model pane), `CHANGELOG.md` (Unreleased)
- Delete: `docs/superpowers/specs/2026-11-13-spine-beside-the-model-design.md`, this plan

- [ ] **Step 1:** In README's description of the window, say that a Protocol of several sources shows its spine on the navigator rail beside the model, the model step selected, and that a table's row opens its rows with the spine kept; and the calls made (step and file rows are readouts beside the model, the graph chip is a readout while a table is in the grid, the column outline stays the table's), each with its reason.
- [ ] **Step 2:** Add a CHANGELOG line under Unreleased in the file's existing voice.
- [ ] **Step 3:** Delete the spec and plan; commit.
