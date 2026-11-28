# An operator step's values in the inspector: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Selecting a node an operator step made shows that step's values in the inspector's Step section, each number on its range and each choice among its options, worked out by arc.

**Architecture:** arc gains `arc step describe <step>` (and an MCP tool of the same meaning) that answers with the operator's description, the values the step sets, the values in force and each data bound worked out on the input the run reads, through one reading of `x-at-most` that the run's own refusal also uses. Brightfield moves its arc pin to that work, asks arc for the step on a worker when the Step section's node was made by an operator, and draws the answer with a bar, chips and plain rows written once from the schema.

**Tech Stack:** Rust; arcform (clap CLI, `serde_json`, DuckDB for the Parquet row count); Brightfield (egui, kittest headless windows, the window's `Runner`, which runs Brightfield as its pinned arc).

**Spec:** `docs/superpowers/specs/2026-11-28-operator-step-values-design.md` (Brightfield, branch `design/operator-step-values`).

## Global Constraints

- Arc works out every value, default in force and bound; Brightfield only draws arc's answer.
- Every parameter in the operator's description is drawn, and no other.
- A bar only where both limits are known; **logarithmic when the lower limit is above 0 and the upper limit is more than 100 times it**, otherwise straight; `exclusiveMaximum` counts as the upper limit.
- The panel is read-only: no handle on a bar, no hover state that suggests a drag.
- A non-operator step's Step section is unchanged.
- Brightfield and arcform are public: no private planning identifiers, Kata refs or local paths in code, comments, commits, branch names or pull request text.
- arcform: branch `step-describe` in `~/github/meridian-online/arcform`, its own pull request. Brightfield: branch `feat/operator-step-values` from `design/operator-step-values`, its own pull request. Brightfield's arc `rev` may point at the arc branch's head while both are open; before Brightfield's pull request is called ready it points at the merged commit on arcform's `main`.
- Brightfield: before each commit `cargo xtask ci --step fmt` and `--step clippy` and the touched tests; after each, `--step comment-citations`. Fresh worktrees use `cargo +1.95.0`. arcform: its own `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and touched tests.

## Review Focus

1. **A step whose input path is a remote URL or a glob.** Arc should answer with no bound and a sentence (it cannot count), not error. Pinned in Task 3.
2. **A Protocol edited while an answer is in flight.** An answer for the old manifest must not be drawn over the new one. Pinned in Task 6.
3. **An operator Brightfield's pinned arc does not know** (a manifest written for a newer arc). The Step section shows arc's refusal sentence and the node's facts, and does not panic. Pinned in Task 8.
4. **A numeric value the step sets as text or out of type** (`neighbors: "15"`). Arc's loader refuses it at manifest load, so the window shows the load error as it does today; Task 3 pins that `step describe` refuses with the loader's sentence rather than guessing.
5. **A very narrow inspector.** The bar's end labels must not overlap the numbers; at the rail's minimum width the end words drop before the numbers do. Pinned in Task 9's screenshot at the rail's minimum width.

---

## The answer's shape (fixed in Task 3, read in Task 6)

`arc step describe <step> [--dir DIR]` prints one JSON object:

```json
{
  "step": "project",
  "operator": "umap_project",
  "version": 1,
  "description": { "long_name": "...", "summary": "...", "reads": [...], "writes": [...], "parameters": {...} },
  "roles": { "input": "build/in.parquet", "columns": ["x", "y"], "out": "maps/out.parquet", "fit": null },
  "values": {
    "neighbors": {
      "set": null,
      "in_force": 15,
      "bound": { "at_most": 47, "role": "input", "path": "build/in.parquet", "rows": 48, "writer": "write_input", "writer_stale": false, "reason": null },
      "past_bound": null
    },
    "min_dist": { "set": null, "in_force": 0.1, "bound": null, "past_bound": null },
    "metric": { "set": "cosine", "in_force": "cosine", "bound": null, "past_bound": null }
  }
}
```

- `values` has one key per key of `description.parameters.properties`. arcform builds `serde_json` without `preserve_order`, so both are in key order, and Brightfield draws them in that order.
- `bound` is present for a parameter with `x-at-most`, else `null`. When the input cannot be counted, `at_most` and `rows` are `null` and `reason` is a sentence: the file does not exist yet, or the path is not a local file.
- `writer` names the step whose `produces` (or operator output role) is the counted path, else `null`; `writer_stale` is whether arc's staleness says it would run.
- `past_bound` is the run's refusal sentence when `set` is above `bound.at_most`, else `null`.
- A step that is not an operator's, or does not exist: exit 1, stderr `error: <sentence>`.

---

### Task 1 (arcform): one reading of a data bound

**Files:**
- Create: `src/operator/bound.rs`
- Modify: `src/operator.rs` (`UmapProject::refuse_neighbors_past_the_input` around line 6771, `parquet_row_count`; `mod bound;`)
- Test: unit tests in `src/operator/bound.rs`; `tests/umap_project_neighbors_bound.rs` stays green unchanged

**Interfaces:**
- Produces: `pub(crate) struct WorkedBound { pub at_most: Option<u64>, pub role: String, pub path: String, pub rows: Option<u64>, pub reason: Option<String> }` and `pub(crate) fn work_out(rule: &serde_json::Value, with: &serde_json::Value, dir: &Path) -> Option<WorkedBound>` — `None` when `rule` is not an `x-at-most` object; `rule` is the property's `x-at-most` value. Also `pub(crate) fn past_bound_sentence(operator: &str, key: &str, asked: u64, bound: &WorkedBound) -> String` producing the exact text the run's refusal uses today.

- [ ] **Step 1: Write failing tests** in `bound.rs`: `forty_eight_rows_bound_neighbors_at_47` (writes a 48-row Parquet in a temp dir with DuckDB, `work_out` on umap's `neighbors` rule gives `at_most: Some(47)`, `rows: Some(48)`, `role: "input"`); `a_missing_input_has_no_bound_and_says_why` (`at_most: None`, `reason` contains `does not exist yet`); `a_remote_input_is_not_counted` (`input: "https://example.org/a.parquet"`, `at_most: None`, `reason` contains `not a local file`); `the_sentence_is_the_runs` (`past_bound_sentence("umap_project","neighbors",48,&b)` equals the string `refuse_neighbors_past_the_input` formats today for 48 rows of `build/in.parquet`).
- [ ] **Step 2: Run** `cargo test --lib operator::bound` — expect FAIL (module missing).
- [ ] **Step 3: Implement** `work_out` (reads `rows_of`, `minus`; counts with the existing `parquet_row_count`, moved into `bound.rs`) and `past_bound_sentence`; make `refuse_neighbors_past_the_input` call both, reading the rule from umap's own schema so the schema and the refusal cannot part.
- [ ] **Step 4: Run** `cargo test --lib operator::bound && cargo test --test umap_project_neighbors_bound` — expect PASS.
- [ ] **Step 5: Commit** `Read a data bound in one place, for the run's refusal and for a description`.

### Task 2 (arcform): words for a range's two ends

**Files:**
- Modify: `src/operator.rs` (umap `neighbors` and `min_dist` properties near line 408; `IGNORED_KEYWORDS` near line 580)
- Test: existing schema tests module in `src/operator.rs`

**Interfaces:**
- Produces: schema annotation `"x-ends": [low, high]` (two strings). `neighbors`: `["local", "global"]`; `min_dist`: `["packed", "spread"]`.

- [ ] **Step 1: Failing test** `umap_projects_numbers_say_what_their_ends_mean`: `describe("umap_project")["parameters"]["properties"]["neighbors"]["x-ends"] == json!(["local","global"])`, `min_dist` `== json!(["packed","spread"])`; and `x_ends_is_an_annotation_the_checker_ignores`: a `with:` checked against a schema carrying `x-ends` reports no fault.
- [ ] **Step 2: Run** `cargo test --lib x_ends umap_projects_numbers` — FAIL.
- [ ] **Step 3: Implement**: add the two annotations; add `"x-ends"` to `IGNORED_KEYWORDS`.
- [ ] **Step 4: Run** the same — PASS.
- [ ] **Step 5: Commit** `Say what the two ends of umap_project's numbers mean`.

### Task 3 (arcform): `arc step describe`

**Files:**
- Create: `src/step_describe.rs`; `tests/step_describe.rs`
- Modify: `src/cli.rs` (new `Commands::Step { cmd: StepCmd }`, `StepCmd::Describe { step: String, #[arg(long, default_value = ".")] dir: PathBuf }`, dispatch); `src/lib.rs` (`pub mod step_describe;` or `pub(crate)` per the crate's pattern); `src/runner.rs` (expose staleness)

**Interfaces:**
- Consumes: Task 1's `work_out`, `past_bound_sentence`; `record::describe`; `Manifest::load(dir)`.
- Produces: `pub fn step_describe(dir: &Path, step: &str) -> Result<serde_json::Value>` returning the shape above; `pub(crate) fn stale_steps(manifest: &Manifest, dir: &Path) -> Result<HashSet<String>>` in `runner.rs`, wrapping `compute_staleness` with the Protocol's state backend and environment and `force: false`.

- [ ] **Step 1: Failing tests** in `tests/step_describe.rs`, each on a temp Protocol built like `tests/umap_project_neighbors_bound.rs` (a `write_input` SQL step `COPY`ing N rows to `build/in.parquet`, then a `project` umap step):
  - `an_unset_neighbors_takes_15_under_a_bound_of_47` — run `write_input` first (48 rows); `values.neighbors` is `{set: null, in_force: 15, bound.at_most: 47, bound.rows: 48, bound.writer: "write_input", past_bound: null}`; `values.metric.set == "cosine"` when the step sets it.
  - `on_ten_rows_an_unset_neighbors_is_in_force_at_9`.
  - `a_neighbors_past_the_bound_is_answered_with_the_runs_sentence` — `neighbors: 48` on 48 rows: exit 0, `past_bound` equals the stderr `arc run` gives for the same Protocol.
  - `before_the_input_is_written_the_bound_names_its_writer` — `write_input` not run: `bound.at_most: null`, `writer: "write_input"`, `writer_stale: true`, `reason` contains `does not exist yet`.
  - `a_stale_writer_is_said` — run, then change `models/write_input.sql`: `at_most` is the count on disk and `writer_stale: true`.
  - `roles_carry_what_the_step_names` — `roles.columns == ["x","y"]`, `roles.fit` null when unset.
  - `values_are_exactly_the_described_parameters` — key set of `values` equals key set of `description.parameters.properties`.
  - `a_sql_step_is_refused` and `an_unknown_step_is_refused` — exit 1, stderr starts `error: ` and names the step.
  - `a_remote_input_has_no_bound` — `input: https://…`: exit 0, `bound.reason` contains `not a local file`.
- [ ] **Step 2: Run** `cargo test --test step_describe` — FAIL.
- [ ] **Step 3: Implement** `step_describe` and the CLI wiring. `in_force`: the set value, else the default; when the property has `x-at-most` with `bounds_default: true` and `at_most` is known, the smaller of the default and `at_most`.
- [ ] **Step 4: Run** `cargo test --test step_describe` — PASS; `cargo clippy --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `Describe a step: its operator's values as set, in force and bounded by its input`.

### Task 4 (arcform): the MCP tool

**Files:**
- Modify: `src/mcp/hero.rs` (beside `operator_describe`, line ~298; the tool list and its test of the tool set)
- Test: `tests/mcp_step_describe.rs`

**Interfaces:**
- Consumes: Task 3's `step_describe`.
- Produces: MCP tool `step_describe` with arguments `{ "step": string, "dir": string (optional, default ".") }`, answering the same JSON.

- [ ] **Step 1: Failing test** `the_tool_answers_what_the_command_prints`: on Task 3's 48-row fixture, the tool's JSON equals `arc step describe project`'s stdout parsed; and the server's tool list includes `step_describe` (update the existing tool-count assertion).
- [ ] **Step 2: Run** `cargo test --test mcp_step_describe` — FAIL.
- [ ] **Step 3: Implement** the tool, calling `step_describe`.
- [ ] **Step 4: Run** `cargo test --test mcp_step_describe` and the existing MCP tool-list test — PASS.
- [ ] **Step 5: Commit** `Offer step_describe to agents`, push `step-describe`, open arcform's pull request, run `roborev review --branch --wait`, fix findings at Medium and above, close the review.

### Task 5 (Brightfield): the pin and the fixtures

**Files:**
- Modify: `Cargo.toml` (`arc` `rev`), `Cargo.lock`
- Create: `crates/brightfield-shell/tests/data/umap-step/arcform.yaml`, `.../umap-step/build/in.parquet` (48 rows, columns `id`, `x DOUBLE`, `y DOUBLE`), `.../umap-step/models/write_input.sql`; `crates/brightfield-shell/tests/data/umap-step-unbuilt/` (the same without `build/in.parquet`)

The manifest: step `write_input` (`sql: models/write_input.sql`, the `COPY` of Task 3's fixture), then `project` (`op: umap_project@1`, `with: {input: build/in.parquet, columns: [x, y], out: build/out.parquet, metric: cosine}`). In `umap-step` the Parquet is committed, so `write_input` reads as not run but the file exists.

- [ ] **Step 1:** Point `arc`'s `rev` at the head of arcform's `step-describe`; `cargo +1.95.0 update -p arcform`.
- [ ] **Step 2:** Write the Parquet with the pinned DuckDB CLI: `target/duckdb-cli/duckdb -c "COPY (SELECT range AS id, range*1.0 AS x, range*2.0 AS y FROM range(48)) TO 'crates/brightfield-shell/tests/data/umap-step/build/in.parquet' (FORMAT parquet)"`.
- [ ] **Step 3: Run** `cargo xtask ci --step clippy` and `cargo test -p brightfield-shell --test operation_panel --test grid_foot` — PASS (the pin moves nothing they read). Run `cargo xtask ci --step sibling-manifests` if the clone is present and note any change.
- [ ] **Step 4: Commit** `Pin arc at step describe, and add a Protocol with a UMAP step`.

### Task 6 (Brightfield): asking arc for a step

**Files:**
- Modify: `crates/brightfield-shell/src/operation.rs`
- Test: unit tests in `operation.rs` beside `describe`'s, and `crates/brightfield-shell/tests/step_values_read.rs`

**Interfaces:**
- Consumes: the answer's shape above.
- Produces, in `operation.rs`:
  - `pub struct StepValues { pub step: String, pub operator: String, pub summary: String, pub values: Vec<StepValue>, pub reads: Vec<RoleGiven>, pub writes: Vec<RoleGiven> }`
  - `pub struct StepValue { pub key: String, pub sentence: String, pub more: Option<String>, pub set: Option<serde_json::Value>, pub in_force: Option<serde_json::Value>, pub range: Option<Range>, pub choices: Vec<String>, pub ends: Option<(String, String)>, pub bound: Option<Bound>, pub past_bound: Option<String> }` — `sentence` is the description's first sentence, `more` the rest.
  - `pub struct Range { pub lower: f64, pub upper: Option<f64>, pub upper_exclusive: bool }`
  - `pub struct Bound { pub at_most: Option<u64>, pub rows: Option<u64>, pub path: String, pub writer: Option<String>, pub writer_stale: bool, pub reason: Option<String> }`
  - `pub struct RoleGiven { pub name: String, pub kind: String, pub required: bool, pub given: Option<serde_json::Value> }`
  - `pub fn describe_step(runner: &Runner, dir: &Path, step: &str) -> Result<StepValues, String>`
  - `pub struct StepDescriptions` with `with_wake(..)`, `get(&mut self, runner: &Runner, dir: &Path, step: &str, revision: u64) -> DescriptionState<'_, StepValues>` and `forget(&mut self)`. `revision` is a number the window bumps when the manifest changes or a run finishes; an answer asked under an older revision is dropped and asked again.

  `DescriptionState` becomes generic over what it holds (`DescriptionState<'a, T = OperationDescription>`), so its existing users read the same.

- [ ] **Step 1: Failing tests**: `a_umap_steps_values_read_from_arcs_answer` (parse a literal of the answer above: three values in order, `neighbors.range == Range{lower:2.0, upper:None, ..}` with `bound.at_most == Some(47)`, `min_dist.range.upper == Some(1.0)` and `upper_exclusive`, `metric.choices == ["euclidean","cosine"]`, `neighbors.ends == Some(("local","global"))`); `fit_is_read_and_written_and_not_given`; in `tests/step_values_read.rs`, `the_runner_answers_for_the_fixture` (`describe_step` on a copy of `umap-step` via the test runner gives `neighbors.in_force == 15`, `metric.set == "cosine"`); `an_answer_for_an_older_revision_is_not_given` (`get` at revision 1, then at 2 before it lands: the first answer is never `Ready`).
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --lib operation:: && cargo test -p brightfield-shell --test step_values_read` — FAIL.
- [ ] **Step 3: Implement** with `arc_with_args(runner, dir, &["step", "describe", step])`, following `describe`'s refusal handling.
- [ ] **Step 4: Run** the same — PASS.
- [ ] **Step 5: Commit** `Ask arc for an operator step's values`.

### Task 7 (Brightfield): the bar, the choices and the plain row

**Files:**
- Create: `crates/brightfield-shell/src/step_values.rs` (`pub mod step_values;` in `lib.rs`)
- Test: unit tests in `step_values.rs`

**Interfaces:**
- Consumes: Task 6's `StepValues`, `StepValue`, `Range`.
- Produces: `pub enum Scale { Straight, Log }`; `pub fn bar_scale(lower: f64, upper: f64) -> Scale`; `pub fn mark_at(value: f64, lower: f64, upper: f64, scale: Scale) -> f32` (0.0 to 1.0, clamped, so a value past the bound sits at 1.0); `pub enum Drawn { Bar { lower: f64, upper: f64, scale: Scale }, Choices, Row }`; `pub fn drawn_as(value: &StepValue) -> Drawn` (a bar's upper limit is `bound.at_most` when known, else `range.upper`); `pub fn draw_step_values(ui: &mut egui::Ui, values: &StepValues, mode: Mode)`; `pub const SET: &str = "set"`, `DEFAULT: &str = "default"`, `NOT_SET: &str = "not set"`; `pub fn known_once(writer: &str) -> String` = `format!("known once {writer} runs")`, `pub fn may_change(writer: &str) -> String` = `format!("may change once {writer} runs")`.

- [ ] **Step 1: Failing tests**: `bar_scale(2.0, 47.0) == Straight`; `bar_scale(2.0, 26_528.0) == Log`; `bar_scale(0.0, 1.0) == Straight`; `mark_at(48.0, 2.0, 47.0, Straight) == 1.0`; `drawn_as(neighbors with at_most 47) == Bar{lower:2, upper:47, Straight}`; `drawn_as(neighbors with at_most None) == Row`; `drawn_as(metric) == Choices`; `drawn_as(a list value) == Row`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --lib step_values` — FAIL.
- [ ] **Step 3: Implement**. Drawing uses the design system's tokens (`semantic(mode)`, `ui_font()`, the warning ink for a past-bound mark); the bar senses no drag and no hover; the end words sit above the numbers and are dropped first when the rail is too narrow for both; "More" is a disclosure under the sentence.
- [ ] **Step 4: Run** the same — PASS.
- [ ] **Step 5: Commit** `Draw a step's values on their ranges`.

### Task 8 (Brightfield): the Step section draws them

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`InspectorPane::ui` for the Protocol document, `inspector_body` around line 6112), `crates/brightfield-shell/src/window.rs` (hold a `StepDescriptions` and the revision; bump it where the manifest is re-read and where a run's end is handled)
- Test: `crates/brightfield-shell/tests/step_values.rs`

**Interfaces:**
- Consumes: Tasks 6 and 7.
- Produces: after the node's facts, when the producing step is an operator step, the Step section draws `Values`, then `Reads` and `Writes`; while the answer is pending, one muted line `Asking arc for this step's values`; on a refusal, arc's sentence.

- [ ] **Step 1: Failing tests** in `tests/step_values.rs`, each with `OpWindow::on_protocol_folder(..)` then `select_made_by("project")`:
  - `a_umap_steps_values_show_under_its_node` (`umap-step`): `texts` contain `metric`, `min_dist`, `neighbors` in that order, `default` twice, `set` once, `2` and `47` as the bar's ends, `local`, `global`, `cosine`, `euclidean`.
  - `fit_is_not_set_under_reads_and_writes`: `not set` appears beside `fit` in both lists.
  - `only_described_values_show`: the value keys drawn equal the description's parameter keys.
  - `before_its_input_is_written_the_bound_names_its_writer` (`umap-step-unbuilt`): `known once write_input runs`.
  - `a_value_past_its_bound_says_the_run_will_refuse_it`: copy of `umap-step` with `neighbors: 48`; arc's sentence is drawn.
  - `a_sql_steps_section_is_unchanged`: `select_made_by("write_input")` draws no `Values` heading.
  - `an_operator_this_arc_does_not_know_shows_arcs_sentence`: a manifest with `op: not_an_operator@1` that the window still opens (or, if the loader refuses it, assert the window's existing load banner and skip); no panic.
  - `an_edit_redraws_from_the_new_manifest`: change `metric: cosine` to `metric: euclidean` on disk, let the window re-read it; `metric` shows `default`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test step_values` — FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell --test step_values --test inspector_contract --test navigator_spine --test operation_panel` — PASS.
- [ ] **Step 5: Commit** `The Step section shows an operator step's values`.

### Task 9 (Brightfield): screenshots

**Files:**
- Modify: `crates/brightfield-shell/tests/snapshot.rs`; Create: `crates/brightfield-shell/tests/snapshots/step_values_light.png`, `step_values_dark.png`, `step_values_narrow.png`

- [ ] **Step 1:** Add `step_values_light`, `step_values_dark` (the `umap-step` window, node selected, Step section open) and `step_values_narrow` (rail at its minimum width).
- [ ] **Step 2:** Draw them on the Mac Studio (`UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test snapshot step_values`), look at each, then run without the variable — PASS. Check every other snapshot still passes unchanged.
- [ ] **Step 3: Commit** `Screenshots of an operator step's values`.

### Task 10 (Brightfield): fold and ready

**Files:**
- Modify: `docs/inspector.md` (a section "An operator step's values": what is drawn and why, the scale rule, arc as the one source and the owner's reason, the rulings from the spec's calls), `CLAUDE.md` only if a new doc is added (one line under Docs)
- Delete: `docs/superpowers/specs/2026-11-28-operator-step-values-design.md`, this plan

- [ ] **Step 1:** Once the owner has merged arcform's pull request, point `arc`'s `rev` at the merged commit on arcform's `main`; `cargo +1.95.0 update -p arcform`.
- [ ] **Step 2:** Write the fold into `docs/inspector.md`; delete the spec and the plan.
- [ ] **Step 3: Run** `cargo xtask gate <pr>` (about 25 minutes on the Mac Studio) and the end-to-end targets the change touches: `cargo test -p brightfield-shell --test step_values --test step_values_read --test inspector_contract --test navigator_spine --test operation_panel --test snapshot` — PASS.
- [ ] **Step 4:** `roborev review --branch --wait`, fix findings at Medium and above, close it; link the pull request to its Kata issue in Forge; say it is ready with its link and stop.
