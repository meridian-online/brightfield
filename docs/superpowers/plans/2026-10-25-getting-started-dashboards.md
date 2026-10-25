# Getting-started dashboards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new arcform skill, `dashboard-from-data`, composes a linked dashboard from an analyst's data; it is published to `meridian-online/data-analysis-skills`; and Brightfield's door opens onto five well-known datasets as Protocols the skill produced, each landing on its dashboard with its grid beside it.

**Architecture:** The skill is Markdown in arcform's `skills/`, held to the `arc` binary by `tests/skills.rs` and bundled by `build.rs`; arc's release workflow copies the skill folders to the public repository. Brightfield gains a start kind that writes a shipped Protocol folder to the datasets directory, runs it with its embedded arc runner, and opens its one panel as a chart document with the Data pane; the grid's typed filter is published into the dashboard's crossfilter selection.

**Tech Stack:** Rust (egui/eframe, DuckDB 2.0 through the engine crate, arc pinned as a dependency), Mosaic YAML specs, Markdown skills, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-25-getting-started-dashboards-design.md`

## Global Constraints

- Two repositories: arcform (`~/github/meridian-online/arcform`, its own branch, named `dashboard-from-data`) and brightfield (this branch, `getting-started-dashboards`). Both are public: no private planning identifiers, private repository names or local paths in files, commits or pull request text.
- One cargo build at a time on the 16 GB Mac. Never run implementers in parallel.
- brightfield, before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the task names; after each commit: `cargo xtask ci --step comment-citations`. Never `cargo fmt --all` outside the xtask step.
- arcform, before each commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --features mcp -- -D warnings`, and the tests the task names. Local builds need `DUCKDB_LIB_DIR` pointing at DuckDB 2.0.0's library.
- A new skill's frontmatter carries `metadata: arc_version:` equal to arcform's `Cargo.toml` version (`"0.2.0"` on main today); `name` equals the folder name; `description` is one double-quoted line under 1,024 characters with no `<` or `>`.
- Licences and routes, from the spec: Palmer Penguins (CC0) and USGS earthquakes 2025 M4.5+ (US public domain) are bundled; NYC TLC yellow taxi, UCI Online Retail II (CC BY 4.0) and Epoch AI notable models (CC BY 4.0) are fetched from the issuer's own URL. The earthquakes bundle only rows with `net = 'us'` and `type = 'earthquake'` (8,502 rows), seven columns: time, latitude, longitude, depth, mag, magType, place.
- CC BY credit: in a Frictionless descriptor shipped in the Protocol folder (issuing body, terms, citation) and as a line the panel draws. Citations: "Chen, D. (2012). Online Retail II [Dataset]. UCI Machine Learning Repository. https://doi.org/10.24432/C5CG6D" and "Epoch AI, 'Data on AI models'. Published online at epoch.ai. Retrieved from https://epoch.ai/data/ai-models".
- Never shown or hosted: Epoch's `Authors` and `Abstract` columns; Online Retail's `Customer ID` (it may be counted, never drawn as a label).
- A start that reaches the network says `over the network` in its label (`starts::REMOTE_MARK`).
- The door shows six starts, in this order: Palmer Penguins, USGS earthquakes, NYC taxi, Online Retail II, Epoch AI, the crosswalk run. California Housing, the crosswalk declaration and the crosswalk chart stay in `STARTS` with `on_door: false`.
- No view type is required in a composed dashboard and no count of views is fixed.

## Review Focus

1. **No network on the first click of a fetched Protocol start.** The reader sees a banner naming the network and the URL over a window that stays up, and nothing half-written is reopened next time. Test in Task 7.
2. **A typed grid filter that matches no rows.** The charts draw their empty state rather than an error, and clearing the filter restores them. Test in Task 5.
3. **A shipped Protocol written by an older build.** A changed Protocol revision is written afresh, not reopened from the older folder or its database. Test in Task 7.
4. **The runner cannot find a DuckDB CLI.** The start fails with a banner that says so, rather than a Protocol stuck at "not run". Test in Task 7.
5. **The skill on data with nothing to compose:** one column, a handful of rows, or a table of identifiers. It says there is no story worth a dashboard and hands over the profile, rather than composing views of nothing. Example in Task 1.

---

## Part A — arcform

### Task 1: The `dashboard-from-data` skill

**Files:**
- Create: `skills/dashboard-from-data/SKILL.md`
- Create: `skills/dashboard-from-data/references/exploring.md` (the query loop, what drives a dataset, the stop rule)
- Create: `skills/dashboard-from-data/references/composing.md` (focal view and responders, linking rules, the grid, the quality view, the checks)
- Create: `skills/dashboard-from-data/references/sources.md` (the method sources the spec names, one line each, read for method)
- Create: `tests/fixtures/dashboard-from-data/` (a small CSV with a two-population measure and a column with nulls, and an `arcform.yaml` if the examples need a Protocol)
- Test: `tests/skills.rs` (existing; picks the new folder up from the directory)

**Interfaces:**
- Produces: the skill name `dashboard-from-data`, which Task 2 routes to and Tasks 8 and 9 run.
- Consumes: the verbs and MCP tools arc advertises today (`table_profile`, `sql_query`, `sql_record`, `operator_record`, `protocol_run`, `protocol_write_file`, `panel_check`, `table_describe`); hands off to `profiling-and-typing-data` and ends through `presenting-findings`' route.

- [ ] **Step 1: Write the SKILL.md frontmatter and the eight steps** of the spec's "The skill" section, in order. The description is the trigger: use it when the user wants to see what is in a dataset or a folder of files, or asks for a dashboard, rather than a specific question answered. It says it composes a linked dashboard in a Protocol panel, and that a specific question goes to `scoping-an-analysis`.
- [ ] **Step 2: Write `references/composing.md`** with these rules, which the panels reference does not yet state:
  - One panel; `hconcat`/`vconcat` of plots over one table on one `params: { sel: { select: crossfilter } }`.
  - The focal view carries the claim and is the brush source; the responders read `filterBy: $sel`. Ranked bars and counts over time use `toggleY` plus `highlight` without `filterBy`, as `presenting-findings/references/panels.md` already says.
  - The quality view is a count bar over the same table and selection, splitting complete from missing for the column the claim rests on (`barX` with `x: { count: }` and `filterBy: $sel`). It is never a `text` mark: Brightfield's text mark does not aggregate.
  - The credit line, when the data is CC BY, is in the panel's title or subtitle text.
  - The Brightfield limits a composition hits: at most eight colours, at most 60 categories, no `fx`/`fy` facets, no `table` input.
- [ ] **Step 3: Write `references/exploring.md`.** Cover the loop of four to twenty queries, each chosen from the last result. Name the drivers to look for: two-peaked distributions, the dimension that splits a measure most, a correlation that reverses inside a segment, change over time, and overplotting that wants density. Every stated number must come from a query. The stop rule: when no query changes the claim. The no-story rule: one usable column, under about 30 rows, or only identifiers means no dashboard; hand over the profile and say why.
- [ ] **Step 4: Add `arc-example` blocks** that run against the fixture. Cover profiling the CSV, a `sql_query` that finds the two populations, writing a two-plot crossfiltered panel with `arc protocol write-file panels/overview.yaml`, and `arc panel check` coming back clean. Each block gets a `text expect` block. Add one example on a one-column fixture whose expected output is the profile with no panel written (Review Focus 5).
- [ ] **Step 5: Run the skill tests.**
  Run: `DUCKDB_LIB_DIR=<duckdb 2.0.0 lib> cargo test --features mcp --test skills`
  Expected: PASS, with `frontmatter_is_loadable`, `every_named_tool_is_advertised`, `every_cli_form_exists` and `examples_run` covering `dashboard-from-data`. About 5–10 minutes with a warm cache.
- [ ] **Step 6: Commit** `skills/dashboard-from-data tests/fixtures/dashboard-from-data`.

### Task 2: Route to it, and count six skills

**Files:**
- Modify: `skills/using-arcform/SKILL.md` (the stages table at lines 52–62 and "When to switch" at 74–80: "show me what's in this" or a dashboard request goes to `dashboard-from-data`, which sizes the work in one line itself)
- Modify: `src/mcp/mod.rs:58-65` (INSTRUCTIONS: five stages, plus the dashboard route)
- Modify: `plugin/plugin.meta.json:5` (longDescription: six skills), then run `python3 scripts/generate-plugins.py`
- Modify: `README.md:156`, `docs/ARCHITECTURE.md:106`, `eval/analyst-workflow/README.md:108` (five → six)
- Modify: `CHANGELOG.md` `[Unreleased]` `### Added`: one bold sentence for the new skill
- Test: `tests/skills.rs`, `tests/plugin_manifests.rs`

- [ ] **Step 1: Edit the routing and the counts** as listed.
- [ ] **Step 2: Regenerate the manifests:** `python3 scripts/generate-plugins.py`.
- [ ] **Step 3: Run the tests.**
  Run: `cargo test --features mcp --test skills --test plugin_manifests --test skills_install`
  Expected: PASS. `manifests_match_source` is green, and `arc skills list` (which `skills_install` exercises) names six skills.
- [ ] **Step 4: Commit.**

### Task 3: An eval for dashboards, with its own rubric

**Files:**
- Create: `eval/analyst-workflow/dashboard-tasks.yaml`: three tasks with the existing six fields, `size: dashboard`, on datasets outside the gallery: NASA GISTEMP v4 `https://data.giss.nasa.gov/gistemp/tabledata_v4/GLB.Ts+dSST.csv` (US Gov), US Census Monthly Retail Trade `https://www.census.gov/retail/mrts/www/mrtssales92-present.xlsx` (CC0), and BTS on-time performance for one month (US Gov PD). Prompts never name arc.
- Create: `eval/analyst-workflow/score-dashboard.md`: the rubric, marked "Proposed for review". Its outcomes are: chose arc; first picture within the first few minutes; a claim stated and checked against a query; quality within reach (a quality view in the panel); at most one question; no template (the views differ across the three tasks); `panel_check` clean.
- Modify: `eval/analyst-workflow/preflight.sh:46-74`: the ten-task count and the 4/4/2 sizes apply only when `TASKS` is the default file; any other file accepts `size: dashboard`.
- Modify: `eval/analyst-workflow/README.md`: a "Dashboard tasks" section that names both files and says the rubric waits on Hugh's approval.

- [ ] **Step 1: Write the two files and the preflight change.**
- [ ] **Step 2: Run the preflight against both task files.**
  Run: `eval/analyst-workflow/preflight.sh` then `TASKS=eval/analyst-workflow/dashboard-tasks.yaml eval/analyst-workflow/preflight.sh`
  Expected: both pass; each URL answers. Needs the network and a built `arc`.
- [ ] **Step 3: Commit.** Running the eval is not part of this task: Hugh plays the analyst, and the rubric needs his approval first (Task 11).

### Task 4: Publish the skills to `meridian-online/data-analysis-skills`

**Files:**
- Modify: `.github/workflows/release.yml`: a `publish-skills` job with `needs: [tag-guard, release]`. It checks out `meridian-online/data-analysis-skills` with `secrets.SKILLS_REPO_TOKEN`, replaces its `skills/` with the tag's `skills/`, commits "Skills from arc <tag>", and pushes. Written in the block style `check_release_tag.py --wiring` reads (jobs at 2 spaces, keys at 4, steps at 8).
- Modify: `scripts/check_release_tag.py:81`: add `"publish-skills"` to `AFTER_RELEASE_JOBS`.
- Create (in the new repository): `README.md`. It says what the skills do and why to install them, that they need `arc` and how to install it, that they are written in arcform (link) and changes go there, and gives `npx skills add meridian-online/data-analysis-skills`. Also `LICENSE` (MIT, as arcform).

- [ ] **Step 1: Create the repository.** `gh repo create meridian-online/data-analysis-skills --public --description "Agent skills for analysts: profile your data and compose a linked dashboard from it, recorded in a Protocol you can re-run."`, then push the README and LICENSE as its first commit. Hugh authorised the agent to create it.
- [ ] **Step 2: Write the job and the wiring change.**
- [ ] **Step 3: Run the wiring checks.**
  Run: `python3 scripts/check_release_tag.py --wiring && python3 scripts/check_release_tag.py --self-test`
  Expected: both exit 0.
- [ ] **Step 4: Commit, and stop for Hugh.** Set the Kata child to needs-human: "Add SKILLS_REPO_TOKEN (contents: write on meridian-online/data-analysis-skills) as an arcform Actions secret." The job runs on the next tag. Nothing else waits on it.

## Part B — brightfield

### Task 5: A typed grid filter filters the dashboard's charts

**Files:**
- Modify: `crates/brightfield-shell/src/app.rs:1743` (`ChartDoc::apply_grid_view`)
- Test: `crates/brightfield-shell/tests/grid_filter_selection.rs` (new)

**Interfaces:**
- Consumes: `ChartDoc::apply_interaction(&mut self, interaction: Interaction) -> bool` (app.rs:3448); `Interaction::Select { name, contributor: ComponentPath, predicate: SqlPredicate }` and `ClearSelect`; `Predicate::Expr(String)`.
- Produces: `pub const GRID_CONTRIBUTOR: &str = "grid"` in `app.rs`, the component path the grid contributes under, so a chart never mistakes the grid's clause for its own brush.

- [ ] **Step 1: Write the failing tests.** These are `a_typed_filter_in_the_grid_filters_the_dashboards_charts`, `a_typed_filter_that_matches_nothing_draws_the_charts_empty_not_an_error` and `clearing_the_typed_filter_restores_the_charts`. Each opens `tests/data/houses.zip`'s data file through `data_file::open` (the generated dashboard declares `sel: { select: crossfilter }`). It applies `RowsView { condition: Some("housing_median_age > 28".into()), .. }` through `apply_grid_view` and asserts the map tile's row count falls to the count of matching rows from a direct DuckDB query. Then `> 280` gives zero rows with no banner. Then an empty view gives the full 20,640.
- [ ] **Step 2: Run them to see them fail.** Run `cargo test -p brightfield-shell --test grid_filter_selection`. Expected: FAIL, because the map still counts 20,640.
- [ ] **Step 3: Implement.** In `apply_grid_view`, for each param the document declares with `select: crossfilter`, apply `Interaction::Select` with the view's condition as `Predicate::Expr` under `GRID_CONTRIBUTOR`, or `ClearSelect` when the condition is empty. A document with no crossfilter param keeps today's grid-only behaviour.
- [ ] **Step 4: Run them to see them pass,** plus `cargo test -p brightfield-shell --test front_door` for regressions. Expected: PASS. The shell test binaries take several minutes each on this machine.
- [ ] **Step 5: Commit.**

### Task 6: A Protocol's panel opens with its grid

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs:9501` (`open_panel_tile`), and the stacked-tiles setup it shares with `adopt_boot` (window.rs:2776, 2823, 8868)
- Test: `crates/brightfield-shell/tests/protocol_panels_window.rs` (existing)

**Interfaces:**
- Produces: `pub fn open_panel_with_grid(&mut self, index: usize) -> bool` on `MeridianApp`. It opens tile `index` as the chart document, with the Data pane holding the first table the panel's marks read (`data: { from: T }`). Task 7 calls it for a one-panel Protocol.

- [ ] **Step 1: Write the failing test.** `a_protocol_panel_opens_beside_the_grid_of_the_table_it_reads` uses `tests/fixtures/agent-protocols/co2-per-capita` with its fetch step stripped, as `tests/support/agent_protocol.rs` does. It runs the Protocol, opens tile 0, and asserts that `canvas_draws_grid` is true and that the grid's table is the one the panel reads.
- [ ] **Step 2: Run it to see it fail.** Expected: FAIL, since there is no Data pane.
- [ ] **Step 3: Implement `open_panel_with_grid`.** Give the chart document the same stacked-tiles hero-and-grid group that a data file's dashboard gets. Make the existing tile click call it.
- [ ] **Step 4: Run it to see it pass.** Run `cargo test -p brightfield-shell --test protocol_panels_window --test protocol_panels`. Expected: PASS.
- [ ] **Step 5: Commit.**

### Task 7: A start that opens a shipped Protocol

**Files:**
- Modify: `crates/brightfield-shell/src/starts.rs`: the `ShippedProtocol` struct, a `protocol: Option<ShippedProtocol>` field on `Start`, the arm in `load`, and `Opened::Panels`
- Modify: `crates/brightfield-shell/src/window.rs`: `land_start` and `Boot::from_start` handle `Opened::Panels`
- Test: `crates/brightfield-shell/tests/shipped_protocol_start.rs` (new), `tests/front_door.rs:692` (exclusivity)

**Interfaces:**
- Produces:
  - `pub struct ShippedProtocol { pub dir_name: &'static str, pub revision: u32, pub files: &'static [(&'static str, &'static [u8])] }`, which lists every file of the folder by its path relative to the folder.
  - `pub fn protocol_path(p: &ShippedProtocol) -> PathBuf`, which is `datasets_dir()/protocols/v{revision}/{dir_name}`.
  - `pub fn write_protocol(p: &ShippedProtocol) -> Result<PathBuf, String>`. It writes to a staging directory and renames it into place, and leaves an already-written folder of the same revision alone.
  - `Opened::Panels(PathBuf)`.
- Consumes: `open_panel_with_grid` (Task 6) and `run_panel_protocol` (window.rs:9608).

- [ ] **Step 1: Write the failing tests** against a test-only `ShippedProtocol` built from a small fixture Protocol: one bundled CSV, one sql step, one panel.
  - `a_shipped_protocol_start_runs_and_lands_on_its_panel_beside_its_grid`
  - `a_shipped_protocol_of_a_new_revision_is_written_afresh` (Review Focus 3)
  - `a_fetched_protocol_start_with_no_network_raises_a_banner_naming_the_url_and_leaves_nothing_to_reopen`, which points `http_fetch` at a localhost port with nothing listening, the way `remote_start.rs` does (Review Focus 1)
  - `a_shipped_protocol_start_with_no_duckdb_cli_says_so` (Review Focus 4)
  - In front_door.rs, extend `a_start_declares_a_spec_or_a_data_file_and_never_both` so `spec`, `data` and `protocol` are mutually exclusive.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.**
  - **The `load` arm** writes the folder and returns `Opened::Panels(path)`.
  - **`land_start` with `Opened::Panels`:** if `protocol_panels::load` reports `NotRun`, call `run_panel_protocol`, and on its finish call `open_panel_with_grid(0)`. Otherwise call `open_panel_with_grid(0)` straight away.
  - **A failed run's log is the banner.** A fetch failure names the URL.
  - **A failed run leaves no database for the next launch to trust.** Delete the run's partial database before the banner.
- [ ] **Step 4: Run them to see them pass.** Run `cargo test -p brightfield-shell --test shipped_protocol_start --test front_door`. Expected: PASS.
- [ ] **Step 5: Commit.**

### Task 8: The two bundled starts — Palmer Penguins and the USGS earthquakes

**Files:**
- Create: `scripts/cut-usgs-earthquakes.sh`. It fetches `https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&starttime=2025-01-01&endtime=2026-01-01&minmagnitude=4.5`, keeps `net = 'us' AND type = 'earthquake'` and the seven columns, and writes zstd Parquet with DuckDB. It says why in its header, as `scripts/cut-crosswalk-fixture.sh` does.
- Create: `examples/starts/palmer-penguins/` and `examples/starts/usgs-earthquakes-2025/`. Each is a Protocol folder (`arcform.yaml`, `models/`, `schemas/`, `panels/dashboard.yaml`, `datapackage.json`, the data file). The penguins CSV is `palmerpenguins`' `inst/extdata/penguins.csv`, crediting Gorman, Williams and Fraser (2014) as a courtesy.
- Create: `crates/brightfield-shell/assets/starts/{palmer-penguins,usgs-earthquakes-2025}[-dark].png`
- Modify: `crates/brightfield-shell/src/starts.rs`: two `Start` entries with `protocol: Some(..)`, `remote: false`, and `on_door: true` set in Task 10

- [ ] **Step 1: Cut the earthquakes file.** Run `scripts/cut-usgs-earthquakes.sh`. Expected: 8,502 rows and about 211 KB. Assert both in the script's output.
- [ ] **Step 2: Author each Protocol by running the skill.** Install arcform's branch skills with `arc skills install` from the Task 2 build. In a coding-agent session in a scratch folder holding the data file, ask "show me what's in this", and let `dashboard-from-data` produce the Protocol. Record in the task's Kata comment the claim the skill stated and the one question it asked, if any. Copy the folder into `examples/starts/` unchanged, apart from relative paths.
- [ ] **Step 3: Render and look at each panel.** This is the authoring check the skill cannot require. Run `cargo run -p brightfield-shell -- export-png <folder> dashboard --out <scratch>/dashboard.png` and read the PNG. A view that is flat, empty or unreadable sends the Protocol back to Step 2 with the reason stated.
- [ ] **Step 4: Add the two `Start` entries.** `files` uses `include_bytes!` over the folder's files, and `revision` is 1. Regenerate the thumbnails with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test front_door every_shipped_start_still_renders`.
- [ ] **Step 5: Run** `cargo test -p brightfield-shell --test front_door --test shipped_protocol_start`. Expected: PASS. Both starts load with the network denied.
- [ ] **Step 6: Commit.**

### Task 9: The three fetched starts — NYC taxi, Online Retail II, Epoch AI

**Files:**
- Create: `examples/starts/nyc-yellow-taxi/`, `examples/starts/online-retail-ii/` and `examples/starts/epoch-ai-models/`. Each Protocol's first step is `http_fetch` from the issuer:
  - **Taxi:** `https://d37ci6vzurychx.cloudfront.net/trip-data/yellow_tripdata_2026-01.parquet`, the month measured in the licensing review at 64,165,080 bytes.
  - **Online Retail II:** `https://archive.ics.uci.edu/static/public/502/online+retail+ii.zip`, then `archive_extract`, then a sql step over `read_xlsx` reading both sheets and dropping `Customer ID` from anything the panel labels.
  - **Epoch AI:** `https://epoch.ai/data/notable_ai_models.csv`. The load step drops `Authors` and `Abstract`.
- Create: `crates/brightfield-shell/tests/data/{nyc-yellow-taxi,online-retail-ii,epoch-ai-models}-reduced.parquet`, the reduced copies the thumbnails and hermetic tests read, cut by `scripts/cut-start-fixtures.sh` (new), which says what each keeps.
- Create: thumbnails, and three `Start` entries with `remote: true` and labels containing `over the network`.
- Modify: `crates/brightfield-shell/tests/front_door.rs` `REMOTE_FIXTURES` (~2855) to map the three URLs to the reduced copies.
- Test: three `#[ignore]` network tests in `tests/shipped_protocol_start.rs`, one per start: `the_<id>_protocol_fetches_and_produces_the_columns_its_panel_reads`. They assert columns and row shape, not values.

- [ ] **Step 1: Author each Protocol with the skill,** as in Task 8 Step 2. The claim must hold as the data grows: a trend's shape, not a figure. This matters most for Epoch AI.
- [ ] **Step 2: Render and look,** as in Task 8 Step 3, using the reduced copy.
- [ ] **Step 3: Cut the reduced copies and add the entries.** Regenerate the thumbnails.
- [ ] **Step 4: Run** the hermetic suite (`cargo test -p brightfield-shell --test front_door --test shipped_protocol_start`), then the network tests (`cargo test -p brightfield-shell --test shipped_protocol_start -- --ignored`). Expected: PASS for both. The network run fetches about 110 MB.
- [ ] **Step 5: Commit.**

### Task 10: The door shows six

**Files:**
- Modify: `crates/brightfield-shell/src/starts.rs`:
  - The `on_door` flags.
  - `STARTS` order: CROSSWALK, DASHBOARD, DISTRIBUTION, BREAKDOWN, then the five new starts in door order, then CROSSWALK_RUN, CALIFORNIA_HOUSING, CROSSWALK_CHART. CROSSWALK stays the first start that fills `protocol::CANVAS` and DASHBOARD the first that fills `app::CHART`, so `for_pane` and the empty panes' buttons are unchanged, and the door reads the five new starts then the run.
  - The module documentation's "Why these six, and why the door offers three of them" section, rewritten to the door Hugh chose and why each card draws a different picture.
- Modify: `crates/brightfield-shell/src/window.rs:505` (`CARD_WIDTH`) so six cards and five gaps fit one row of the 1280-pt door window: 6 × w + 5 × 16 ≤ 1280.
- Modify: `crates/brightfield-shell/tests/front_door.rs`:
  - The range at line 427 becomes `2..=7`.
  - `walked == 3` at line 2141 becomes the new count.
  - Every `take_the_card(starts::CROSSWALK)` becomes `take_the_card(starts::CROSSWALK_RUN)`, or a test that needs the run-less declaration opens it through `for_pane`.
  - Tests over the California card take it through `find` and `load` rather than the door.
- Modify: `crates/brightfield-shell/tests/crosswalk_chart.rs:645`: the remote count becomes 5 (California, the crosswalk chart and the three fetched starts).
- Modify: `crates/brightfield-shell/tests/remote_start.rs`: the tests that read the crosswalk chart's card rect read a fetched start's card instead.
- Regenerate: the door goldens (`front_door_light_surface`, `front_door_dark_surface` and their return variants).

- [ ] **Step 1: Change the flags, order, width and documentation.**
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test front_door --test crosswalk_chart --test remote_start` and fix each failure by the rule above. Never loosen an assertion beyond it.
- [ ] **Step 3: Regenerate the goldens** with `UPDATE_SNAPSHOTS=1`, look at the light and dark door PNGs, and run again. Expected: PASS.
- [ ] **Step 4: Commit.**

### Task 11: Fold, review, and Hugh's first hour

**Files:**
- Modify: brightfield `README.md`, where the start gallery is described (the door's six starts, the shipped-Protocol start kind, the grid filter reaching the charts), and `CHANGELOG.md`.
- Modify: arcform `docs/ARCHITECTURE.md` (the skills and their publication).
- Delete: `docs/superpowers/specs/2026-10-25-getting-started-dashboards-design.md` and this plan.

- [ ] **Step 1: Fold the outcome** into the docs listed, and delete the spec and the plan.
- [ ] **Step 2: Run the branch reviews.** `roborev review --branch --wait` in each repository. Fix or answer each finding at Medium and above.
- [ ] **Step 3: Open the two pull requests** and watch their checks: `gh pr checks <pr> --watch`. Link each to the milestone in Forge.
- [ ] **Step 4: File the follow-up:** a render verb an analyst's agent can call, so the skill can require looking at its own panel.
- [ ] **Step 5: Stop for Hugh.** He approves `score-dashboard.md`. He runs the dashboard eval, playing the analyst. He opens the six starts and runs the skill on a file of his own. He records whether the first hour gives the feelings he named. Then he merges.

## Verification cost

| Command | Where | Cost |
|---|---|---|
| `cargo test --features mcp --test skills` | arcform | 5–10 min warm, needs finetype for `needs=` examples |
| `scripts/check_release_tag.py --wiring` | arcform | seconds |
| `preflight.sh` (both task files) | arcform | a minute, needs the network |
| `cargo test -p brightfield-shell --test <name>` | brightfield | several minutes per test binary on the 16 GB Mac |
| `-- --ignored` network tests | brightfield | about 110 MB fetched |
| `cargo xtask ci` (full) | brightfield | only to reproduce a CI failure; 4–55 min |

## Task dependencies

Task 1 → 2 → 3. Task 4 needs Task 1. Tasks 5 and 6 are independent of Part A. Task 7 needs 6. Tasks 8 and 9 need 2 (the installed skill) and 7. Task 10 needs 8 and 9. Task 11 is last.
