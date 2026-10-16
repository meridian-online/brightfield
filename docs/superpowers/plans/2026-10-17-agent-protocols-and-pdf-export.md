# Agent-written Protocols and headless exports: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Brightfield opens a Protocol an agent wrote with every panel drawn from the Protocol's database, says in a tile why a panel cannot draw, exports the report to PDF and a panel to PNG or data with no window, and records the analyst's saves in the Protocol's version log beside the agent's.

**Architecture:** arc exports the rules Brightfield must not copy (where the database is, the panel check, a run's table hashes, the report's run, the version log), and its command line learns to record under a caller's interface word. Brightfield loads each `panels/*.yaml` as its own composed chart, with each table it reads injected as a `data:` entry over the Protocol's database attached read-only, so one panel's failure is one tile's message. The headless exports reuse the Vello path `brightfield-shot --vello-only` already uses, and a new `brightfield-report` crate turns `report.md` into a PDF with Typst, with the charts placed as images.

**Tech stack:** Rust 1.95.0, egui/eframe 0.35, Vello 0.9 on wgpu, DuckDB (bundled crate, moving to arc's `=1.10506.0`), arc as a git dependency, Typst as a library (`typst`, `typst-pdf`, and `typst-render` for test previews), `pulldown-cmark`.

**Spec:** `docs/superpowers/specs/2026-10-17-agent-protocols-and-pdf-export-design.md`. Read it before starting any task; this plan argues from it.

## Global constraints

- **No planning identifiers.** Nothing tracked in this public repository, no commit message and no PR text names a private planning identifier (decision refs, task ids, card or issue ids).
- **One build at a time.** On Hugh's Mac (16 GB), run one cargo build at a time and never two implementers in parallel.
- **Pre-commit checks.** Before each commit, after `git add`, run `cargo xtask ci --step fmt`, `--step clippy` and `--step public-hygiene`, plus the tests the change touches. After each commit, run `--step comment-citations` and `--step commit-messages`. Run the full `cargo xtask ci` only to reproduce a CI failure.
- **No `cargo fmt --all`.** Format through `cargo xtask ci --step fmt`.
- **Offline.** The exports and the tests need no network. A test that would fetch is a failing test.
- **One renderer.** A chart in a PNG or PDF is drawn by the same Vello path the window uses. No second renderer.
- **No second copy of arc's rules.** Database location, panel findings, run hashes, the report's run and the log format all come from `arc::spec`.
- **DuckDB stays at arc's 1.5 version.** DuckDB 2.0 is not in this release.
- **The arcform export lands first.** The arcform change in Task 1 merges to arc's main before arc's DuckDB 2.0 move starts on 2026-10-21. If it has not merged by then, stop and raise it with Hugh. Do not pin to a 2.0 revision.
- **Where the build ends.** The build ends at a reviewed pull request whose checks pass. Hugh merges and tags it. Agents do not merge in meridian-online.
- **Keep the docs current.** Update `deviations.yaml`/`DEVIATIONS.md` (via `cargo run --bin generate-deviations`) and the chart-kind registry in the same task as any change to what Brightfield draws.
- **The CLI as it stands.** Subcommand names are `export-pdf`, `export-png` and `export-data`. The existing `brightfield-shell [FILE]` form and its flags keep working.

## Review focus

These are inputs the spec implies but doesn't spell out, most likely to bite first. Each has its test in the task named.

1. **The agent is still running `arc run` when the analyst opens the Protocol**, so the database is locked. Expect a message saying the Protocol is being run and to try again, not a crash or empty tiles. (Task 4: `a_locked_database_says_the_protocol_is_running`)
2. **The Protocol folder's path has a space or a quote in it.** Expect it to open and export as any other. (Task 4: `a_folder_with_a_space_and_a_quote_opens`)
3. **The report's prose holds characters Typst reads as markup**: `#`, `$`, `*`, `_`, `@`, `<`, `[`, `]`, `\`, backticks. Expect them printed literally. (Task 7: `typst_markup_in_prose_is_printed_literally`)
4. **`panels/` holds a `.yml` file, a hidden file, or a subfolder.** Expect exactly what arc's panel check reads: `*.yaml` files directly in `panels/`, in name order. (Task 4: `only_yaml_files_in_panels_are_tiles`)
5. **The Protocol names an explicit `db:`.** Expect its tables to be read from that file, relative to the folder. (Task 4: `an_explicit_db_is_read`)

## Tasks and their order

| Task | What | Repo | Depends on | Main verification and its cost |
| --- | --- | --- | --- | --- |
| 1 | arc exports and the interface override | arcform | — | `cargo test --features mcp` for the new tests, about 3 min warm |
| 2 | Move the arc pin and DuckDB, and tag arc children as Brightfield | brightfield | 1 merged | `cargo test -p brightfield-shell --test chart_save_reopens_chart`. The first build after the DuckDB change recompiles bundled DuckDB, about 20–30 min |
| 3 | Draw `lineY`/`lineX`, and add a line-over-time kind | brightfield | — | `cargo test -p brightfield-render -p brightfield-sql` plus the new kind test, about 5 min |
| 4 | Load a Protocol's panels headless, one outcome per tile | brightfield | 2, 3 | `cargo test -p brightfield-shell --test protocol_panels`, about 3 min |
| 5 | Open an agent-written Protocol in the window, with tile messages | brightfield | 4 | `cargo test -p brightfield-shell --test protocol_panels_window`, about 3 min, plus Hugh's look in the app |
| 6 | `export-png` and `export-data` | brightfield | 4 | `cargo test -p brightfield-shell --test export_cli`, about 3 min |
| 7 | `export-pdf` through a new `brightfield-report` crate | brightfield | 6 | `cargo test -p brightfield-report` and `--test export_pdf`, about 6 min; first Typst build about 5 min |
| 8 | Versions: every panel recorded, the list read from the log, who made each version | brightfield | 2, 5 | `cargo test -p brightfield-protocol --test chart_history` and `-p brightfield-shell --test versions_log`, about 4 min |
| 9 | The eval Protocols draw, and the release path proves export offline | brightfield | 5–8 | `scripts/check-agent-protocols.sh <runs-dir>`, about 2 min; `scripts/verify-airgapped.sh` runs in the release job |
| 10 | Fold the docs, review the branch, open the PR | brightfield | all | `roborev review --branch --wait`; `gh pr checks <pr> --watch` (CI takes about 25 min) |

**Execution order.**

1. Task 3 first, because it needs nothing from arc.
2. Then Task 1, whose arcform PR must be merged by Hugh before Task 2 can start.
3. Then 2, 4, 5, 6, 7, 8, 9 and 10 in that order.

Task 6 and Task 8 do not depend on each other, but builds are sequential, so they run in that order anyway.

**Branch.** The Brightfield work goes on one branch cut from this one, `agent-protocols-and-pdf-export`, so the spec, the plan and the fixture are on it.

---

### Task 1: arc exports and the interface override (arcform)

Work in a worktree of arcform's `main` on a branch `export-protocol-reading-for-apps`. Open a PR there and stop for Hugh to merge it. Do not commit to `main`.

**Files:**
- Modify: `src/spec.rs` (add the re-exports listed below, and their doc examples)
- Modify: `src/panel_check.rs` (make `PanelFinding` and `CheckedPanels` public)
- Modify: `src/manifest.rs` (expose the database path)
- Create: `src/protocol_read.rs` (`run_tables`, `report_run`, `read_log`)
- Modify: `src/history.rs` (expose a log line's parse, used by `read_log`)
- Modify: `src/cli.rs` (honour `ARCFORM_HISTORY_WAY`)
- Test: `tests/protocol_read.rs`, `tests/history_way_env.rs`

**Interfaces.** These are produced in `arc::spec`, and Tasks 2, 4, 7 and 8 rely on them:

```rust
pub fn protocol_db_path(dir: &Path) -> Result<PathBuf>;            // Manifest::load(dir)?.db_path(dir)
pub fn check_panels(dir: &Path) -> Result<CheckedPanels>;           // today's check_panels_from
pub fn check_panel(dir: &Path, panel: &Path) -> Result<Vec<PanelFinding>>;
pub type CheckedPanels = (bool, Vec<(PathBuf, Vec<PanelFinding>)>); // (tables read from the database, panels)
pub enum PanelFinding { MissingTable { from: String }, MissingColumn { from: String, column: String }, Unreadable { reason: String } } // + Display, unchanged
pub struct RunTables { pub run_id: String, pub tables: BTreeMap<String, Option<String>> } // table name -> content_hash
pub fn run_tables(dir: &Path, run_id: Option<&str>) -> Result<RunTables>; // None = newest run
pub fn report_run(dir: &Path) -> Result<Option<String>>;           // the run id report.md's Run section names; None when no report.md
pub struct LogLine { pub at: String, pub file: String, pub kind: String, pub interface: Option<String>, pub author: Option<String>, pub version: String, pub steps: Vec<(String, String)> }
pub fn read_log(dir: &Path) -> Result<Vec<LogLine>>;               // lines in file order; a line that does not parse is skipped
pub const HISTORY_WAY_ENV: &str = "ARCFORM_HISTORY_WAY";
```

`LogLine.steps` folds the log's `step`/`change` pair and its `steps` list into one list. `author` reads a key the log does not write yet, so a line without it gives `None`.

- [ ] **Step 1: Write the failing tests** in `tests/protocol_read.rs`:
  - `db_path_is_arcs_data_folder_without_db`: with `ARCFORM_DB_DIR` set to a temp dir, `protocol_db_path` of a Protocol named `p` ends in `/p.duckdb` under that dir.
  - `db_path_follows_an_explicit_db`: `db: data/x.duckdb` gives `<dir>/data/x.duckdb`.
  - `run_tables_reads_the_newest_run`: after `arc run` of the skills fixture with the aggregate step from `presenting-findings`, `run_tables(dir, None)` has `spend_by_category` with `Some(hash)`, and its `run_id` matches the newest file in `build/.arcform/runs/`.
  - `report_run_names_the_reports_run`: after `arc report write`, `report_run(dir) == Some(run_id)`.
  - `read_log_parses_every_line_kind`: a log holding a `step`/`change` line, a `steps` line, a line with no `interface`, a line with an `author`, and one line of garbage gives four `LogLine`s with the right fields.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test --features mcp --test protocol_read`. Expected: does not compile, with the names unresolved.
- [ ] **Step 3: Implement the exports.** `run_tables` reads the contract through `contract::runs_dir` and `report::newest_contract` (move that helper where both can use it), keeping assets whose kind is a table. `report_run` reads the `- Run: \`<id>\`` line arc's own `report.rs` writes. Keep the format in one place by having `report.rs` and `report_run` share a `const` for that line's prefix.
- [ ] **Step 4: Write the failing test** `tests/history_way_env.rs::the_env_names_the_cli_interface`. With `ARCFORM_HISTORY_WAY=brightfield`, `arc sql record … --name t` appends a log line with `"interface":"brightfield"`. With `ARCFORM_HISTORY_WAY=mcp` the command fails, and its message names the variable, because `HistoryWay::new` refuses arc's own words.
- [ ] **Step 5: Implement it.** Where the CLI opens its history handle with `HistoryWay::TERMINAL`, use `HistoryWay::new(v)` when the variable is set and non-empty.
- [ ] **Step 6: Run the tests.** Run `cargo test --features mcp --test protocol_read --test history_way_env`, then arcform's quick checks (fmt, clippy, the skills test `cargo test --features mcp --test skills`). Expected: all pass.
- [ ] **Step 7: Commit, push, and wait.** Commit, push, open the PR, wait for `roborev wait <sha>` and fold its findings in, then say the PR is ready with its link. Flag the Kata issue `needs-human` with "arcform export PR waits on Hugh's merge, needed before 2026-10-21".

### Task 2: Move the arc pin and DuckDB, and tag arc children as Brightfield

**Files:**
- Modify: `Cargo.toml:66` (the `arc` rev, set to Task 1's merge commit on arc's main)
- Modify: `Cargo.toml:71` (`duckdb = { version = "=1.10506.0", features = ["bundled"] }`, matching arc's pin)
- Modify: `Cargo.lock`
- Modify: `crates/brightfield-shell/src/run.rs:308` (`arc_command` sets `ARCFORM_HISTORY_WAY=brightfield`)
- Test: `crates/brightfield-shell/tests/chart_save_reopens_chart.rs` (existing), and a new `crates/brightfield-shell/tests/arc_child_way.rs`

**Interfaces:**
- Consumes: `arc::spec::HISTORY_WAY_ENV`.
- Produces: an arc pin with the Task 1 exports. Every arc child Brightfield starts records under `brightfield`.

- [ ] **Step 1: Write the failing test** `arc_child_way.rs::a_recorded_step_is_logged_as_brightfield`. Copy `examples/protocol/edgar_gleif` (or the smallest existing Protocol fixture) to a temp dir, with `ARCFORM_HISTORY_DIR` and `ARCFORM_DB_DIR` set to temp dirs. Call `operation::record_sql(&Runner::this_binary(), dir, "SELECT 1 AS one", "one")`. Assert that the last line of `<dir>/arcform-log.txt` holds `"interface":"brightfield"` and `"step":"one"`.
- [ ] **Step 2: Run it and see it fail.** Run `cargo test -p brightfield-shell --test arc_child_way`. Expected: FAIL, with no `arcform-log.txt` under the old pin.
- [ ] **Step 3: Move the pins and set the variable.** Update both pins, then `cargo update -p arc -p duckdb`. Set `ARCFORM_HISTORY_WAY` in `arc_command`. Fix any compile breaks the arc move causes, keeping behaviour the same.
- [ ] **Step 4: Run the tests.** Run `cargo test -p brightfield-shell --test arc_child_way --test chart_save_reopens_chart --test generated_reopen`, then `cargo test -p brightfield-protocol`. Expected: PASS.
- [ ] **Step 5: Check licences and advisories.** Run `cargo xtask ci --step deny-licenses` and `--step deny-advisories`. A new advisory not caused by this change is recorded as a ruling and fixed in a separate PR.
- [ ] **Step 6: Commit.**

### Task 3: Draw `lineY`/`lineX`, and add a line-over-time kind

**Files:**
- Modify: `crates/brightfield-spec/src/vocab.rs:127-128` (`LineX`, `LineY` → `Implemented`)
- Modify: `crates/brightfield-sql/src/lower.rs:1567` (add `(MarkKind::LineY, Box::new(SimpleLowerer))` and the same for `LineX`)
- Modify: `crates/brightfield-render/src/mark.rs:1499, 5489-5498` (make `LineRenderer` oriented the way `AreaRenderer { axis }` is, so `lineY` joins points in x order and `lineX` in y order, matching Mosaic)
- Modify: `crates/brightfield-spec/src/edit.rs:1177`, `crates/brightfield-shell/src/chart_item.rs:177, 2750`, and `crates/brightfield-render/src/inset.rs` (wherever `AreaY`/`AreaX` are listed, list `LineY`/`LineX` with `Line`)
- Modify: `crates/brightfield-shell/src/chart_kinds.rs:59-70, 207` (a `LINE_OVER_TIME` kind, `"line-over-time"`)
- Create: `examples/line-y.yaml`
- Test: `crates/brightfield-render/tests/` (the existing mark snapshot pattern), and `crates/brightfield-shell/tests/line_over_time_kind.rs`

**Interfaces:**
- Produces: `chart_kinds::LINE_OVER_TIME: ChartKindId = ChartKindId::new("line-over-time")`. It has two slots: a `DATE` or numeric x column, and a numeric y column, with an optional category column for `stroke`. It builds a `lineY` reading `data_file::SOURCE`, with `x`, `y`, an optional `stroke`, and `legend: color` when there is a stroke. In `kinds()` it goes after `counts_over_time()`. Its controls follow `scatter`'s.

- [ ] **Step 1: Write the failing tests.**
  - `lineY` with `x: year, y: v, stroke: country` over three countries and five years renders three polylines, each joining its points in increasing x. Use the render crate's existing snapshot helper. Add `examples/line-y.yaml` with inline rows and use it as the snapshot.
  - `lineX` joins in increasing y.
  - `line_over_time_kind.rs::builds_a_line_per_category`: the kind bound to (`year`, `value`, `country`) builds a spec whose single mark is `lineY` with `stroke: country`, and it is offered for a table with a `DATE` column and a numeric column.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-render` and `cargo test -p brightfield-shell --test line_over_time_kind`. Expected: FAIL. `lineY` is refused as unimplemented and the kind is unknown.
- [ ] **Step 3: Implement it.** Follow the `AreaRenderer` axis pattern for orientation. Build the kind like `counts_over_time` but with a measure on y.
- [ ] **Step 4: Run the tests.** Run them again, plus `cargo test -p brightfield-shell --test chart_module --test binned_histogram --test scatter_kind`. Expected: PASS. The existing kinds' order and snapshots are unchanged apart from the new kind.
- [ ] **Step 5: Update the docs.** Add a `deviations.yaml` entry only if `lineY`'s drawing departs from Mosaic's, then run `cargo run --bin generate-deviations`. Add the kind to whatever README section lists the registry's kinds.
- [ ] **Step 6: Commit.**

### Task 4: Load a Protocol's panels headless, one outcome per tile

**Files:**
- Create: `crates/brightfield-shell/src/protocol_panels.rs` (registered in `lib.rs`)
- Create: `crates/brightfield-shell/tests/support/agent_protocol.rs` (the fixture runner shared by Tasks 4–9)
- Test: `crates/brightfield-shell/tests/protocol_panels.rs`

**Interfaces:**
- Consumes: `arc::spec::{protocol_db_path, check_panels, PanelFinding}`; `pipeline::compose_spec_str_at(source, base_dir, viewport) -> Result<Composed, String>`; `brightfield_conformance::preflight(&Spec) -> SupportReport`.
- Produces:

```rust
pub struct ProtocolPanels { pub dir: PathBuf, pub name: String, pub db: PathBuf, pub tiles: Vec<PanelTile> }
pub struct PanelTile { pub file: PathBuf /* "panels/x.yaml" */, pub title: Option<String>, pub source: String /* the file's text as written */, pub outcome: Result<Composed, PanelProblem> }
pub enum PanelProblem { Unreadable(String), MissingTable { from: String }, MissingColumn { from: String, column: String }, Unsupported(Vec<String>), Failed(String) }
impl PanelProblem { pub fn message(&self) -> String }  // MissingTable/MissingColumn/Unreadable reuse PanelFinding's Display text
pub enum OpenProblem { NotAProtocol(PathBuf), NotRun { dir: PathBuf }, NoPanels { dir: PathBuf }, Locked { db: PathBuf }, Arc(String) }
impl OpenProblem { pub fn message(&self) -> String }
pub fn agent_panels_in(dir: &Path) -> bool;  // panels/ holds a *.yaml other than panel_file(dir, name)
pub fn load(dir: &Path, viewport: Rect) -> Result<ProtocolPanels, OpenProblem>;
pub fn with_protocol_tables(panel: &str, db: &Path) -> Result<String, String>;
pub const PROTOCOL_DB_KEY: &str = "__protocol";
```

- `with_protocol_tables` adds `__protocol: { file: <db> }` to the panel's top-level `data:`, which the existing `duckdb` source attaches `READ_ONLY`. For each `from: T` a mark reads that the panel's own `data:` does not define, it adds `T: { query: 'SELECT * FROM "__protocol"."main"."T"' }`, quoting `T` as an identifier. It never writes the result to disk.
- A panel whose findings are non-empty gets `MissingTable`/`MissingColumn`/`Unreadable` and is not composed. A panel whose preflight has `Blocking` entries gets `Unsupported(names)`. Any other compose error is `Failed`.
- **The runner.** `tests/support/agent_protocol.rs` exports `pub struct RanFixture { pub dir: PathBuf, _tmp: TempDir }` and `pub fn ran_fixture(name: &str) -> RanFixture`. It copies `tests/fixtures/agent-protocols/<name>` into a temp dir. It points `ARCFORM_DB_DIR` and `ARCFORM_HISTORY_DIR` at one temp root per test binary, set once through a `OnceLock` before any test reads them. It runs `arc run` through `env!("CARGO_BIN_EXE_brightfield-shell")` with `BRIGHTFIELD_RUN_AS_ARC=1`, and asserts the copied `data/owid-co2-data.csv` is byte-identical after the run, which proves no fetch replaced it. If arc's `http_fetch` downloads although `out` exists, the runner removes the fetch step from the copy's `arcform.yaml` before running (never from the fixture). Record that finding in a comment on the runner.

- [ ] **Step 1: Write the failing tests** in `tests/protocol_panels.rs`:
  - `the_co2_protocol_draws_both_panels`: `load(ran_fixture("co2-per-capita").dir, …)` gives two tiles, in the order `co2_per_capita_change_pct` then `co2_per_capita_since_1990`, both `Ok`. Their titles are the files' `title:`. The line chart's `Composed.plots.len() == 1`.
  - `a_missing_column_is_its_tiles_message`: in a copy, replace `x: change_pct` with `x: change_percent`. That tile's outcome is `MissingColumn { from: "co2_per_capita_change", column: "change_percent" }`, and the other tile is `Ok`.
  - `an_unsupported_mark_is_its_tiles_message`: `mark: waffleY` gives `Unsupported(vec!["waffleY"])`, with the other tile `Ok`.
  - `a_panel_that_is_not_yaml_is_its_tiles_message`: a tile with `Unreadable`.
  - `a_protocol_that_has_not_run_says_so`: an unrun copy gives `Err(NotRun { .. })`.
  - `a_locked_database_says_the_protocol_is_running`: hold a read-write `duckdb::Connection` on the database, then call `load`. Expect `Err(Locked { .. })`, whose message names the Protocol as being run.
  - `a_folder_with_a_space_and_a_quote_opens`: the fixture copied into `tmp/it's a test/` draws both tiles.
  - `only_yaml_files_in_panels_are_tiles`: adding `panels/x.yml`, `panels/.hidden.yaml` and `panels/sub/y.yaml` leaves two tiles, the same set `check_panels` names.
  - `an_explicit_db_is_read`: set `db: local.duckdb` in a copy's `arcform.yaml`, run, and both tiles draw.
  - `with_protocol_tables_leaves_inline_data_alone`: a panel whose `data:` defines `t` inline keeps its `t` and gains only `__protocol` and the tables it reads from the database.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-shell --test protocol_panels`. Expected: does not compile, with `protocol_panels` unresolved.
- [ ] **Step 3: Implement `protocol_panels.rs`** to the interfaces above. Map DuckDB's lock error on attach to `Locked`. Use the window's default chart viewport for `viewport` in tests.
- [ ] **Step 4: Run the tests.** Run `cargo test -p brightfield-shell --test protocol_panels`. Expected: PASS, with no network.
- [ ] **Step 5: Commit.**

### Task 5: Open an agent-written Protocol in the window, with tile messages

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs:1477` (`Boot::open_sampled`: a new route ahead of the one-step and generated shapes when `protocol_panels::agent_panels_in(dir)`)
- Modify: `crates/brightfield-shell/src/window.rs:1065` (`Boot` gains `panels: Option<ProtocolPanels>`)
- Create: `crates/brightfield-shell/src/panel_tiles.rs` (the tiles item: one tile per panel, its chart or its message, and a not-run state with a Run action through the existing `Run::begin`)
- Modify: `crates/brightfield-shell/src/app.rs` (selecting a tile opens that panel as the chart document. `save_chart_beside` writes the panel's own file, with the injected `__protocol` entries and injected tables stripped)
- Test: `crates/brightfield-shell/tests/protocol_panels_window.rs`

**Interfaces:**
- Consumes: Task 4's `load`, `agent_panels_in`, `ProtocolPanels`, `PanelProblem::message`, `OpenProblem::message`.
- Produces: `pub fn strip_protocol_tables(text: &str, injected: &[String]) -> String` in `protocol_panels.rs`, the inverse of `with_protocol_tables`. Also `Boot::agent_protocol(panels: ProtocolPanels, flow: Flow) -> Boot`.

**The route rule.** A Protocol whose `panels/` holds any `*.yaml` other than Brightfield's own `panel_file(dir, name)` opens by the new route, whatever its steps' shape. Otherwise the old routes apply unchanged. The tile layout is the builder's choice, held to the spec's anti-goals: no overlap, titles and legends as each panel declares them, and readable at the default window size.

- [ ] **Step 1: Write the failing tests** in `protocol_panels_window.rs`:
  - `the_co2_protocol_opens_as_two_tiles`: `Boot::open(ran.dir/arcform.yaml, …)` gives `boot.panels` with two `Ok` tiles. A full-window capture through `capture_png` (headless) has two non-blank tile regions.
  - `a_broken_tile_shows_its_message`: the missing-column copy captures with the tile's message text present in the egui output, found through the test harness's text query as `arrangement.rs` does.
  - `a_one_step_protocol_with_agent_panels_opens_by_the_new_route`: a one-step CSV Protocol with two `panels/*.yaml` opens with two tiles, not a generated dashboard.
  - `a_brightfield_protocol_opens_as_before`: `generated_reopen`'s fixture still opens its generated dashboard.
  - `saving_a_tile_writes_the_panel_without_injected_tables`: edit a tile's title and save. The panel file holds the new title and no `__protocol` key, and is otherwise byte-identical to the original.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-shell --test protocol_panels_window`. Expected: FAIL.
- [ ] **Step 3: Implement** the route, `panel_tiles.rs`, tile selection and save.
- [ ] **Step 4: Run the tests.** Run them plus `--test arrangement --test generated_reopen --test chart_save_reopens_chart`. Expected: PASS.
- [ ] **Step 5: Hugh's look.** Build the app (`cargo build -p brightfield-shell --release`) and ask Hugh to open `tests/fixtures/agent-protocols/co2-per-capita` (after one run) in it. Flag the Kata issue `needs-human` first. His findings are fixed in this task.
- [ ] **Step 6: Commit.**

### Task 6: `export-png` and `export-data`

**Files:**
- Modify: `crates/brightfield-shell/src/capture.rs:856` (split out `pub fn render_composed(composed: Composed, scale: f32) -> Result<image::RgbaImage, String>`, with `capture_vello_only` calling it)
- Create: `crates/brightfield-shell/src/export.rs`
- Modify: `crates/brightfield-shell/src/main.rs:96, 110, 160, 972` (`Invocation::Export(Export)`, the `HELP` text, parsing next to `--check-type-source`, and dispatch before `boot_layout`)
- Test: `crates/brightfield-shell/tests/export_cli.rs`

**Interfaces:**
- Consumes: Task 4's `load`.
- Produces:

```rust
pub enum Export {
    Pdf { dir: PathBuf, out: Option<PathBuf> },
    Png { dir: PathBuf, panel: String, out: Option<PathBuf>, scale: f32 },
    Data { dir: PathBuf, panel: String, format: DataFormat, out: Option<PathBuf> },
}
pub enum DataFormat { Csv, Parquet }
pub fn run_export(export: &Export) -> Result<Vec<PathBuf>, Vec<String>>; // written paths, or one stderr line per problem
pub fn resolve_panel(dir: &Path, panel: &str) -> Result<PathBuf, String>; // "x", "x.yaml" or "panels/x.yaml"
```

The defaults are:

- `<dir>` may be the folder or its `arcform.yaml`.
- `export-png` writes `<dir>/exports/<panel>.png` at scale 2.
- `export-data` writes `<dir>/exports/<table>.<csv|parquet>`, one file per table the panel's marks read from the database, with every row, through DuckDB `COPY`. When a panel reads several tables, `--out` must be a directory.
- `Pdf` is parsed here and runs in Task 7. Until then it returns `Err(vec!["export-pdf is not built yet"])`.

The exit codes are:

- 0 when the export succeeds. Each written path is printed on stdout.
- 1 when the export fails. Each problem goes on stderr, as `<panel file>: <message>` for panel problems.
- 2 for a usage error.
- A machine with no GPU adapter fails with `no GPU is available; headless export draws with the GPU and needs one`.

- [ ] **Step 1: Write the failing tests** in `export_cli.rs`. Run the built binary through `CARGO_BIN_EXE_brightfield-shell` against `ran_fixture`.
  - `png_of_a_panel`: `export-png <dir> co2_per_capita_since_1990` exits 0. It writes a PNG whose size is the panel's `width`×`height` (760×440) times 2, and whose pixels equal `render_composed` of the same tile.
  - `data_of_a_panel`: `export-data <dir> co2_per_capita_change_pct` writes `co2_per_capita_change.csv` with 10 lines (a header and 9 rows).
  - `parquet_data`: `--format parquet` writes a file DuckDB reads back with 9 rows.
  - `unknown_panel_fails`: exit 1, stderr names the panel and lists the panels that exist.
  - `broken_panel_fails`: the missing-column copy exits 1, with `panels/co2_per_capita_change_pct.yaml: a mark reads the column change_percent …`.
  - `unrun_protocol_fails`: exit 1, with the not-run message.
  - `help_lists_the_exports`: `--help` names all three subcommands.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-shell --test export_cli`. Expected: FAIL.
- [ ] **Step 3: Implement** to the interfaces above.
- [ ] **Step 4: Run the tests.** Run them plus `cargo test -p brightfield-shell --test shot_cli`, or whichever existing test covers `brightfield-shot --vello-only`. Expected: PASS, and `brightfield-shot` output is unchanged.
- [ ] **Step 5: Commit.**

### Task 7: `export-pdf` through a new `brightfield-report` crate

**Files:**
- Create: `crates/brightfield-report/` (`Cargo.toml`, `src/lib.rs`, `src/markdown.rs`, `src/world.rs`), added to the workspace
- Modify: `crates/brightfield-shell/src/export.rs` (the `Pdf` arm, and the stale-table notice)
- Test: `crates/brightfield-report/tests/report.rs`, `crates/brightfield-shell/tests/export_pdf.rs`

**Interfaces:**
- Consumes: Task 6's `render_composed` and `resolve_panel`; `arc::spec::{run_tables, report_run}`; the font bytes in `meridian_design::fonts` (`INTER_REGULAR`, `INTER_MEDIUM`, `INTER_SEMIBOLD`, `INTER_BOLD`, `JBM_REGULAR`).
- Produces:

```rust
// brightfield-report
pub struct ChartImage { pub path: String /* "panels/x.yaml" */, pub title: String, pub png: Vec<u8>, pub width_px: u32, pub height_px: u32 }
pub struct ReportInput<'a> { pub markdown: &'a str, pub charts: &'a [ChartImage], pub notice: Option<&'a str> }
pub fn to_typst(input: &ReportInput) -> String;               // the Typst source, for tests and for debugging
pub fn compile_pdf(input: &ReportInput) -> Result<Vec<u8>, String>;
pub fn preview_pages(input: &ReportInput, ppi: f32) -> Result<Vec<Vec<u8>>, String>; // PNG per page, through typst-render
// brightfield-shell
pub fn stale_tables(dir: &Path) -> Result<Vec<String>, String>; // tables the panels read whose newest-run hash differs from the report's run's
```

**What's decided:**

- **Page and type.** A4 portrait. 11pt Inter body text, with Inter SemiBold headings. Markdown emphasis is set in Inter Medium, since no italic Inter is bundled, and strong emphasis in Inter Bold. Code is JetBrains Mono. Pages are numbered at the foot. There is no cover.
- **Charts.** Each `` - `panels/<file>`: <title> `` item in the report's `## Charts` section is replaced by the title, set as a caption, above the chart image. The chart is rendered at scale 3, set to the text width, in a block that is not breakable. A `Check:` line in that section means the export fails.
- **Tables.** A Markdown table is set at 9pt, with numeric cells right-aligned in a box that does not break. A table of 7 or more columns is set at 8pt. A table of 10 or more columns goes on a landscape (flipped) page of its own.
- **The stale notice.** When `stale_tables` is non-empty, a shaded block above the Charts heading reads: "These charts read the Protocol's tables as they are now. `<t1>`, `<t2>` changed since the run this report describes (`<run id>`)."
- **Offline.** The Typst world has the bundled fonts and the images only. Package imports are refused, and nothing is fetched.
- **Versions.** Pin `typst`, `typst-pdf`, `typst-render` and `pulldown-cmark` to releases at least two weeks old, at one Typst version, and check them with `deny-licenses`.

The export:

- `export-pdf <dir> [--out <file>]` writes `<dir>/report.pdf` by default.
- It fails with exit 1 when there is no `report.md`, when a listed panel cannot draw (naming the panel and its message), or when the Protocol has not run.

- [ ] **Step 1: Write the failing tests** in `crates/brightfield-report/tests/report.rs`:
  - `typst_markup_in_prose_is_printed_literally`: prose holding `# $5 *a* _b_ @c <d> [e] \f` and backticks round-trips. Check `to_typst` escapes each character, and that the extracted text of `compile_pdf`'s first page contains the literal string. Extract the text with `pdf-extract` as a dev-dependency.
  - `charts_replace_their_list_items`: `to_typst` with two `ChartImage`s holds two image calls and neither list item's text.
  - `a_wide_table_goes_landscape`: a 10-column table puts a flipped page in the source. A 6-column one doesn't.
  - `no_chart_is_split`: a report with five tall charts has every image call inside a non-breakable block, and `compile_pdf` succeeds.
  - `package_imports_are_refused`: `#import "@preview/x:0.1.0"` in prose is printed as text, not compiled as an import.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-report`. Expected: does not compile.
- [ ] **Step 3: Implement** `brightfield-report` to the decisions above.
- [ ] **Step 4: Write the failing tests** in `crates/brightfield-shell/tests/export_pdf.rs`:
  - `the_co2_report_exports`: exit 0. The file starts with `%PDF`. Its text contains the report's first sentence and both chart titles. It has no stale notice. It writes page previews to `target/export-pdf-preview/co2-per-capita-<n>.png` for Hugh's look.
  - `no_report_fails`: delete `report.md`, then expect exit 1 with `no report.md in <dir>; write one with arc report write`.
  - `a_broken_panel_fails`: the missing-column copy gives exit 1, naming the panel.
  - `a_changed_table_raises_the_notice`: change `models/03_co2_per_capita_change.sql`'s `round(…, 1)` to `round(…, 0)`, run again, and `stale_tables` returns `["co2_per_capita_change"]`. The PDF's text contains "changed since the run".
  - `an_unchanged_rerun_raises_no_notice`: run again with no change, and `stale_tables` is empty.
- [ ] **Step 5: Implement** the `Pdf` arm and `stale_tables`, then run `cargo test -p brightfield-shell --test export_pdf`. Expected: PASS.
- [ ] **Step 6: Hugh's look.** Flag the Kata issue `needs-human`. Ask Hugh to read `target/export-pdf-preview/` against his five points: layout, titles and axes, overlaps, legends, and table wrapping. Fix what he finds in this task.
- [ ] **Step 7: Commit.**

### Task 8: Versions: every panel recorded, the list read from the log, who made each version

**Files:**
- Modify: `crates/brightfield-protocol/src/chart_history.rs:166-271, 438-482` (`ChartVersion` gains `who` and `on_this_machine`. `versions` reads the folder's log through `arc::spec::read_log` and joins it to the store)
- Modify: `crates/brightfield-shell/src/app.rs:1836` (`save_chart_beside` takes the panel file's path, not only `name`, so any panel is recorded)
- Modify: `crates/brightfield-shell/src/versions.rs:499, 892-1021` (a WHO column. A row not on this machine is dimmed, and its show and restore actions are disabled with the reason)
- Test: `crates/brightfield-protocol/tests/chart_history.rs` (extend), `crates/brightfield-shell/tests/versions_log.rs`

**Interfaces:**
- Consumes: `arc::spec::{read_log, LogLine}`.
- Produces:

```rust
pub enum Who { Brightfield, ArcCommandLine, ArcMcp, Other(String), Unknown }
// ChartVersion { id, kind, way, at, bytes, change, who: Who, author: Option<String>, on_this_machine: bool }
pub fn save_chart_at(&mut self, panel: &Path, history: Option<&HistoryStore>) -> Result<Option<NotRecorded>, ChartSaveError>;
// save_chart_beside(dir, name, h) becomes save_chart_at(&panel_file(dir, name), h)
```

How the list is built:

- `who` comes from the log line's `interface`: `brightfield`, `terminal`, `mcp`, any other word, or nothing.
- A version the log names but the store lacks is `on_this_machine: false`, with `change: NoPredecessor`.
- A store entry the log lacks, which is everything an older arc recorded, is still listed, with `who` from its `way`.
- When the log holds an `author`, the WHO column shows it after the interface word. The spec says to drop the author from this release if arc hasn't landed it.

- [ ] **Step 1: Write the failing tests:**
  - `versions_log.rs::the_agents_versions_show_as_arc`: list `ran_fixture`'s `panels/co2_per_capita_since_1990.yaml` versions with an empty store. That gives one version, `who: ArcCommandLine` and `on_this_machine: false`, read from the committed log.
  - `versions_log.rs::a_save_in_brightfield_joins_the_log`: save that panel through `save_chart_at`. The log gains a line with `"interface":"brightfield"` and `"file":"panels/co2_per_capita_since_1990.yaml"`. The list's newest version is `who: Brightfield` and `on_this_machine: true`.
  - `chart_history.rs::an_author_is_read_when_present`: a log line with `"author":"hugh"` gives `author: Some("hugh")`.
  - `chart_history.rs::a_store_entry_missing_from_the_log_is_listed`.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p brightfield-protocol --test chart_history` and `cargo test -p brightfield-shell --test versions_log`. Expected: FAIL.
- [ ] **Step 3: Implement** to the interfaces above, and update the Versions panel's words. "Brightfield", "arc (command line)" and "arc (agent tool)" are the words for the three known interfaces.
- [ ] **Step 4: Run the tests.** Run them plus `--test chart_save_reopens_chart` and the existing versions tests. Expected: PASS.
- [ ] **Step 5: Commit.**

### Task 9: The eval Protocols draw, and the release path proves export offline

**Files:**
- Create: `scripts/check-agent-protocols.sh`
- Modify: `scripts/verify-airgapped.sh` (after its window check, run `export-pdf` on a copy of the fixture with the network off, and expect exit 0 and a PDF)
- Modify: `README.md:9-24` (Quick Start: `brew install meridian-online/tap/brightfield`, then the three export commands) and `README.md:104` (Distribution)
- Test: the scripts themselves, run as below

**Interfaces:**
- Consumes: the three export subcommands.

**The eval Protocols** were written on 2026-10-17 by headless Claude Code runs of arcform's ten eval tasks. They are kept outside this repository at `/Volumes/Dev/meridian-online/arcform-eval-runs/2026-10-17/<task>/work/<protocol>/`; the session that wrote this plan moves them there from its scratchpad. A static inventory of the 25 panels from the first seven runs found one mark Brightfield did not draw, `lineY`, which Task 3 handles.

- [ ] **Step 1: Write `scripts/check-agent-protocols.sh <runs-dir>`.** For every folder under `<runs-dir>` holding `arcform.yaml` and `panels/`, it runs `arc run` when there is no run record, then `export-png` for each panel and `export-pdf` when there is a `report.md`. It prints one `ok`/`FAIL <panel>: <message>` line each and exits 1 on any failure.
- [ ] **Step 2: Run it** with `scripts/check-agent-protocols.sh /Volumes/Dev/meridian-online/arcform-eval-runs/2026-10-17`. Expected: every line `ok`. Draw each failure, or record it as a deviation with its tile message, in this task. Put the final counts in the PR description.
- [ ] **Step 3: Run the release checks.** Run `scripts/package.sh` for the host target, then `scripts/verify-airgapped.sh` on its output. Expected: the export step passes with the network off.
- [ ] **Step 4: Commit.**

### Task 10: Fold the docs, review the branch, open the PR

**Files:**
- Modify: `README.md` (Architecture section: how a Protocol's panels are loaded, the export subcommands, the `brightfield-report` crate, and how versions are read from the log)
- Delete: `docs/superpowers/specs/2026-10-17-agent-protocols-and-pdf-export-design.md` and this plan

- [ ] **Step 1: Fold the docs.** Move the outcome into the README's Architecture and Quick Start sections, then delete the spec and the plan.
- [ ] **Step 2: Review the branch.** Run `roborev review --branch --wait`. Fix or answer every finding at Medium and above, then `roborev close <job>`.
- [ ] **Step 3: Open the PR and wait for its checks.** Push, open the PR, then `gh pr checks <pr> --watch`.
- [ ] **Step 4: Report readiness.** When both are clean, say the PR is ready with its link. Put CI's test totals, the rulings made during the build, and Task 9's eval counts in its description. Then stop.
