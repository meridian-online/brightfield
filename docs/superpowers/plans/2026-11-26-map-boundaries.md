# Map boundaries implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Point maps draw over grey country shapes, and a country column becomes a choropleth: offered in the Chart editor, joined to bundled Natural Earth countries by a Protocol step, coloured by an aggregate grouped by country code, clickable, filtered by the page, with unmatched rows counted beside it.

**Architecture:** The countries ship as two Parquet files published from open-analytics and copied into a Protocol's own `data/` folder when a map needs them, so the Protocol stays portable and draws offline. The join is an arc SQL step; the Protocol's model records its relationship to `countries`, and the shell passes that relationship to the `geo` lowerer, which groups by the key and attaches shapes from the shape side. The spec stays plain Mosaic throughout: a `geo` layer over a table.

**Tech Stack:** Rust (pinned toolchain), DuckDB, arc (arcform) for Protocol steps, egui/eframe + wgpu, vello, egui_kittest snapshots. Task 1 is in open-analytics (Python scripts and an arcform Protocol).

**Spec:** `docs/superpowers/specs/2026-11-26-map-boundaries-design.md`. Read it whole before any task.

## Global constraints

- Natural Earth 1:110m `admin_0_countries` only. No GADM, no OpenStreetMap boundary data.
- Geometry is GeoJSON text (`VARCHAR`) everywhere, so nothing needs the spatial extension or the network.
- The two bundled Parquet files together stay under 400 KB; over that, stop and report.
- `iso_a3` / `iso_a2` take Natural Earth's `ISO_A3_EH` / `ISO_A2_EH` where `ISO_A3` / `ISO_A2` are `-99`.
- The spec a chart writes is plain Mosaic. Brightfield-only behaviour (the fast grouping, the click on the analyst's column, the unmatched note) comes from the Protocol's model, never from a new spec attribute.
- Rows are never dropped silently: the join is a left join, and unmatched rows are counted on the page.
- Public repo: no planning identifiers in code, comments, commits, branch names or PR text. Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the task's tests. After each commit: `cargo xtask ci --step comment-citations`.
- Pixel baselines are redrawn only on the Mac Studio, never from a hosted run.

## Review focus

1. **A country column with nulls, blanks and mixed case ("france", " France ").** Nulls count as unmatched and never match a country; case and surrounding space never stop a match. (Task 6 `nulls_blanks_and_case_in_the_country_column`.)
2. **A column of names in which two spellings map to one country ("UK", "United Kingdom").** One country, one fill, the sum of both; a click selects both spellings. (Task 6 `two_spellings_fill_one_country`, Task 8 `a_click_selects_every_spelling_of_the_country`.)
3. **Quotes and odd characters in the table or column name, or in a value ("Côte d'Ivoire").** SQL stays valid and accents fold to a match. (Task 6 `quoted_names_and_accented_values`.)
4. **Choosing a choropleth twice over the same table, or a point map after a choropleth.** The countries sources are added once and the join step is reused, not duplicated. (Task 7 `a_second_choropleth_reuses_the_sources_and_the_join`.)
5. **A value that is zero or negative under `colorScale: log`.** It draws in the domain's low colour, never NaN or no fill. (Task 9 `log_colour_takes_the_low_end_for_zero_and_negatives`.)

---

### Task 1 (open-analytics): Publish the countries and their names to the lake

**Files (in `/Volumes/Dev/meridian-online/open-analytics`):**
- Create: `datasets/naturalearth/` holding the Protocol (`arcform.yaml`, `models/*.sql`, `datapackage.json`) and the alias list `datasets/naturalearth/aliases.csv` (columns `alias,iso_a3,reason`).
- Modify: `datasets/registry.json`, `datasets/licence-notices.json`, `README.md` (the address list).

**Interfaces:**
- Produces: `https://openlake.meridian.online/naturalearth/countries_110m.parquet` with columns `iso_a3 VARCHAR NOT NULL UNIQUE, iso_a2 VARCHAR, name VARCHAR, continent VARCHAR, geom VARCHAR` (GeoJSON text); and `naturalearth/country_names.parquet` with `name_key VARCHAR NOT NULL UNIQUE, iso_a3 VARCHAR NOT NULL`.

- [ ] **Step 1:** Read `docs/publishing.md` and one existing dataset folder (e.g. `datasets/fips`) and copy its layout.
- [ ] **Step 2:** Write the Protocol: fetch `https://naciscdn.org/naturalearth/110m/cultural/ne_110m_admin_0_countries.zip`, read it with spatial's `ST_Read`, write `countries_110m` (geometry through `ST_AsGeoJSON`) and `country_names`. `name_key` is the spelling lower-cased, trimmed and accent-folded with DuckDB's `strip_accents(lower(trim(x)))`. It is drawn from `NAME`, `NAME_LONG`, `FORMAL_EN`, `ADMIN`, `SOVEREIGNT`, `NAME_ALT`, every `NAME_<lang>` field, `iso_a2`, `iso_a3` and `aliases.csv`. A key that maps to two countries is dropped, and the step logs it. Start `aliases.csv` with `usa,USA`, `us,USA`, `uk,GBR`, `rsa,ZAF` and `eire,IRL`, each with its reason.
- [ ] **Step 3:** Run it and check: `countries_110m` has 177 rows and no null `iso_a3`, `country_names` has no duplicate `name_key`, both files have the descriptor stamped (`check_descriptors.py`), and their total size is under 400 KB.
- [ ] **Step 4:** Commit, push the branch, open a pull request in open-analytics, and say it is ready. Hugh merges, and publishing follows the path `docs/publishing.md` gives.

### Task 2: The bundled countries and copying them into a Protocol

**Files:**
- Create: `scripts/fetch-boundaries.sh` (modelled on `scripts/cut-usgs-earthquakes.sh`: download both Parquet files from the lake, check row counts and the size budget).
- Create: `crates/brightfield-shell/assets/boundaries/countries_110m.parquet` and `country_names.parquet`. Until Task 1 is published, these are a hermetic fixture with the same schema, built by `crates/brightfield-shell/tests/support/boundary_fixture.sql`: eight rectangles with real codes (GBR, IRL, FRA, DEU, USA, ZAF, CIV, AUS), names including "Côte d'Ivoire" and "Deutschland", and the five aliases.
- Create: `crates/brightfield-shell/src/boundaries.rs`
- Test: `crates/brightfield-shell/tests/boundaries.rs`

**Interfaces:**
- Produces: `pub const COUNTRIES: &str = "countries"; pub const COUNTRY_NAMES: &str = "country_names";` and `pub fn add_countries(runner: &Runner, dir: &Path) -> Result<Added, String>`. It copies both files into `dir/data/boundaries/` and records two SQL steps named `countries` and `country_names` (`SELECT * FROM read_parquet('data/boundaries/<file>')`) with `operation::record_sql`. It returns `Added::AlreadyThere` when both steps exist and `Added::Recorded` otherwise.

- [ ] **Step 1: Failing tests:** `add_countries_records_two_steps_that_read_offline` makes a temp Protocol, calls `add_countries`, runs it with networking disabled, and asserts `countries` has 8 rows and every `geom` parses as GeoJSON. `add_countries_twice_records_once` asserts the second call returns `AlreadyThere` and the step count does not change.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test boundaries`. Expect FAIL: module not found.
- [ ] **Step 3: Implement** `boundaries.rs` with the files embedded through `include_bytes!` and written out on copy.
- [ ] **Step 4: Run** the same command. Expect PASS.
- [ ] **Step 5: Commit** `Bundle Natural Earth's countries and copy them into a Protocol that draws a map`.

### Task 3: A `geo` layer in an unprojected plot draws in the plot's scales

**Files:**
- Modify: `crates/brightfield-render/src/mark.rs` (`GeoRenderer::augment_scales` and `render`, around :5519)
- Test: `crates/brightfield-render/src/mark.rs` test module, and `crates/brightfield-ui/tests/execution_conformance.rs`

**Interfaces:**
- Produces: a `geo` mark on a plot with no `projectionType` maps longitude through the plot's x scale and latitude through its y scale, and contributes its bbox to those domains only when the plot's domain is not `Fixed`.

- [ ] **Step 1: Failing test:** `geo_under_a_cartesian_plot_shares_its_scales`. A plot with a `dot` at (−0.1, 51.5) and a `geo` rectangle around it, `xDomain: Fixed`: the dot's pixel lies inside the rectangle's pixel path, and the x domain equals the dot layer's alone.
- [ ] **Step 2: Run** `cargo test -p brightfield-render geo_under_a_cartesian`. Expect FAIL.
- [ ] **Step 3: Implement** a branch in `GeoRenderer` for `MarkProjection` absent, reading the plot's linear scales instead of synthesising its own.
- [ ] **Step 4: Run** `cargo test -p brightfield-render` and `cargo test -p brightfield-ui --test execution_conformance`. Expect PASS, with existing geo PNGs (`examples/geo.yaml`) unchanged. Those plots are projected, so they don't take this branch.
- [ ] **Step 5: Commit** `A geo layer on an unprojected plot draws in the plot's own scales`.

### Task 4: Grey countries behind the generated point map

**Files:**
- Modify: `crates/brightfield-shell/src/chart_kinds.rs` (`point_map` :466 and its tile body), `crates/brightfield-shell/src/dashboard.rs` (`point_map_tile_for` :1228), `crates/brightfield-shell/src/own_files.rs` (call `boundaries::add_countries` when the generated dashboard has a coordinate pair)
- Test: `crates/brightfield-shell/tests/point_map_kind.rs`, `crates/brightfield-shell/tests/point_map_baseline.rs`

**Interfaces:**
- Consumes: `boundaries::add_countries`, `boundaries::COUNTRIES`.
- Produces: `pub(crate) fn backdrop_layer(indent: usize, below_60s: bool) -> String`, a `geo` layer over `countries` with `fill` from the design system's quietest surface tint and `stroke` the page background at `strokeWidth: 0.5`. When `below_60s` is false it carries `filter: "iso_a3 <> 'ATA'"`.

- [ ] **Step 1: Failing tests:** `the_point_map_draws_countries_first` asserts the kind's block has a `geo` layer over `countries` before its `dot` layers. `an_own_file_with_coordinates_gets_the_countries` opens a CSV with `longitude,latitude` as an own file and asserts the Protocol holds the `countries` step. `a_shared_protocol_is_not_written` opens a start's Protocol and asserts no step was added.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test point_map_kind`. Expect FAIL.
- [ ] **Step 3: Implement.** `below_60s` is true when the latitude column's minimum is below −60. Pick the tint and stroke tokens from `crates/brightfield-render`'s ink tokens, light and dark.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell --test point_map_kind --test point_map_baseline`. The baseline fails on pixels. Redraw it on the Mac Studio (`UPDATE_SNAPSHOTS=1`), look at both themes, and note the change for the PR.
- [ ] **Step 5: Commit** `The generated point map draws the world's countries in grey behind its points`.

### Task 5: `geo` aggregates, grouping by a declared key

**Files:**
- Modify: `crates/brightfield-sql/src/lower.rs` (`LowerCtx` :24, `GeoLowerer` :120)
- Test: `crates/brightfield-sql/src/lower.rs` test module; `crates/brightfield-engine/tests/geo_key_timing.rs` (new)

**Interfaces:**
- Produces: in `brightfield-sql`, `pub struct GeometryKey { pub key: String, pub shapes: String, pub shapes_key: String }` and a `LowerCtx` field `pub geometry_keys: &'a IndexMap<String, GeometryKey>`, keyed by the table a mark reads. Empty everywhere it is not filled.
- Lowering rules for a `geo` mark whose `fill` (or `r`) is an aggregate:
  - With no `GeometryKey` for the table: `SELECT geom, <agg> FROM t [WHERE …] GROUP BY geom`, the portable shape.
  - With one: `SELECT s.geom, a.<agg> FROM <shapes> s LEFT JOIN (SELECT <key>, <agg> FROM t [WHERE …] GROUP BY <key>) a ON s.<shapes_key> = a.<key>`. Shapes come first, so a shape with no rows has a null fill. `filterBy:` clauses go in the inner `WHERE`.
- A `geo` mark with no aggregate lowers as today, byte for byte.

- [ ] **Step 1: Failing tests:** `geo_count_without_a_key_groups_by_geometry` and `geo_count_with_a_key_aggregates_first_then_attaches_shapes` assert the two SQL shapes. `geo_without_aggregate_is_unchanged` asserts the existing SQL is unchanged.
- [ ] **Step 2: Run** `cargo test -p brightfield-sql geo_`. Expect FAIL.
- [ ] **Step 3: Implement.** Thread `geometry_keys` through every `LowerCtx` construction, filled with an empty map until Task 6.
- [ ] **Step 4: Timing test** `geo_key_lowering_is_within_three_times_a_plain_group_by` in `geo_key_timing.rs`: 1,000,000 rows and 177 shapes of 4.5 KB, the keyed SQL's median of 5 runs is no more than 3 × a plain `GROUP BY key` on the same rows. Run `cargo test -p brightfield-engine --test geo_key_timing --release` and expect PASS.
- [ ] **Step 5: Commit** `A geo layer aggregates, grouping by its table's key to the shapes when one is declared`.

### Task 6: The country join step and its relationship

**Files:**
- Modify: `crates/brightfield-shell/src/boundaries.rs`
- Modify: the shell's `LowerCtx` construction site (`grep -rn "LowerCtx {" crates/brightfield-shell crates/brightfield-engine`), to fill `geometry_keys` from the Protocol's model
- Test: `crates/brightfield-shell/tests/boundaries.rs`

**Interfaces:**
- Consumes: Task 2's `add_countries`; Task 5's `GeometryKey`.
- Produces:
  - `pub enum CountryForm { Alpha2, Alpha3, Name }`
  - `pub fn country_join_sql(table: &str, column: &str, form: CountryForm) -> String`. It keeps every column of `table`, adds `iso_a3` and `geom`, and is a left join. Codes compare `upper(trim(col))`; names compare `strip_accents(lower(trim(col)))` with `country_names.name_key`.
  - `pub fn add_country_join(runner: &Runner, dir: &Path, table: &str, column: &str, form: CountryForm) -> Result<String, String>`. It records the step `<table>_countries` and writes a model relationship `from: <table>_countries, to: countries, from_columns: [iso_a3], to_columns: [iso_a3]` with `meta.extras["geometry"] = "geom"` and `meta.extras["source_column"] = <column>` through `model_file`. It returns the step name and reuses an existing step of that name.
  - `pub fn geometry_keys(model: &brightfield_ossie::Model) -> IndexMap<String, GeometryKey>`, read from the relationships that carry that extra.

- [ ] **Step 1: Failing tests** over the Task 2 fixture plus a table of `country` values:
  - `codes_and_names_both_match`
  - `nulls_blanks_and_case_in_the_country_column`: `NULL`, `''` and `' france '`. The first two get a null `iso_a3`; the third matches FRA.
  - `two_spellings_fill_one_country`: "UK" and "United Kingdom" both match GBR.
  - `quoted_names_and_accented_values`: table `o"rders`, column `pays d'origine`, value "Cote d'Ivoire" matches CIV.
  - `every_row_survives_the_join`: the joined row count equals the source's.
  - `the_relationship_reaches_the_lowerer`: `geometry_keys` of the written model yields `<table>_countries → (iso_a3, countries, iso_a3)`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test boundaries`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** it again. Expect PASS.
- [ ] **Step 5: Commit** `Join a country column to the bundled countries, and record the join in the model`.

### Task 7: The choropleth kind in the Chart editor

**Files:**
- Modify: `crates/brightfield-shell/src/chart_kinds.rs` (register `choropleth()` in `kinds()` :225 after `point_map()`), and the place a chosen kind is applied (`grep -rn "\.build)(" crates/brightfield-shell/src`), so a kind may first write to the Protocol through the forms' write path with their undo
- Modify: `crates/brightfield-shell/src/dashboard.rs`: `pub(crate) fn country_column(columns: &[ColumnProfile]) -> Option<(usize, CountryForm, &'static str)>` beside `coordinate_pair` :1151
- Test: `crates/brightfield-shell/tests/choropleth_kind.rs` (new)

**Interfaces:**
- Consumes: `country_column`, `add_countries`, `add_country_join`.
- Produces: kind id `CHOROPLETH` (`"choropleth"`), icon `globe`, description "Fills each country by its rows, joined to the world's countries". The spec it builds is one plot:
  - a `geo` layer `from: <table>_countries, filterBy: $sel`, with `fill: { count: }` or `{ sum: <measure> }` from the kind's measure slot, and `tip: true`;
  - `select: toggle` as `$sel`;
  - `colorScheme: blues`, `colorScale: linear`, `projectionType: equirectangular`, and the backdrop as its first layer (Task 4's `backdrop_layer`).

  `country_column` passes in two rounds: first by FineType label (`geography.location.country_code` gives `Alpha2`, `geography.location.country` gives `Name`); then by column name, case-insensitive: `iso2`, `iso_a2`, `country_code` → `Alpha2`; `iso3`, `iso_a3`, `country_iso3` → `Alpha3`; `country`, `country_name`, `nation` → `Name`. A `country_code` column whose values are three letters is `Alpha3`.

- [ ] **Step 1: Failing tests:**
  - `offered_for_a_country_name_column`
  - `offered_for_an_iso3_column_by_name`
  - `not_offered_without_a_country_column`
  - `choosing_it_writes_the_sources_the_join_and_a_plain_mosaic_plot`: the plot parses with no `ParseWarning` and names no attribute outside Mosaic's vocabulary.
  - `a_second_choropleth_reuses_the_sources_and_the_join`
  - `undo_removes_what_it_wrote`
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test choropleth_kind`. Expect FAIL.
- [ ] **Step 3: Implement.** A kind offered over a Protocol a form can't write to is greyed, with the forms' own reason.
- [ ] **Step 4: Run** it again. Expect PASS. Also run `cargo test -p brightfield-shell --lib chart_kinds` and update `the_registry_ships_these_kinds_in_this_order`.
- [ ] **Step 5: Commit** `The Chart editor offers a choropleth for a column of countries`.

### Task 8: A choropleth follows the page and selects on click

**Files:**
- Modify: `crates/brightfield-spec/src/vocab.rs` (`Toggle` :323 → `Implemented` for `geo` layers only; other marks keep refusing it with the existing warning)
- Modify: `crates/brightfield-ui/src/interaction.rs`, `crates/brightfield-ui/src/crossfilter.rs`; `crates/brightfield-render/src/mark.rs` (a hit-test over `GeoRenderer`'s projected paths, `pub fn geo_feature_at(&self, px: Point) -> Option<usize>`)
- Test: `crates/brightfield-shell/tests/choropleth_interaction.rs` (new)

**Interfaces:**
- Consumes: Task 6's relationship (`from_columns` names the joined step's key; the analyst's column is the join step's source column, recorded in the relationship's `meta.extras["source_column"]`, which Task 6 also writes).
- Produces: a click on a country publishes `"<source_column>" IN (<every distinct value whose iso_a3 is that country>)` to the layer's `toggle` selection. Clicking it again clears it, and Escape clears it. The `geo` layer's `filterBy:` re-queries under other clauses with crossfilter self-exclusion. The tip shows `name` and the aggregate.

- [ ] **Step 1: Failing tests,** driven through the shell as `start_interaction.rs` does:
  - `a_click_selects_every_spelling_of_the_country`: rows "UK" and "United Kingdom"; the clause holds both.
  - `a_bar_toggle_recolours_the_map`
  - `a_second_click_and_escape_clear_it`
  - `the_tip_names_the_country_and_its_value`
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test choropleth_interaction`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** it again, plus `cargo test -p brightfield-ui`. Expect PASS.
- [ ] **Step 5: Commit** `A choropleth narrows with the page and a click selects a country's rows`.

### Task 9: Draw `log` and `quantile` colour scales

**Files:**
- Modify: `crates/brightfield-spec/src/layout.rs` (`DRAWN_COLOUR_SCALES` :1396 adds `"log"` and `"quantile"`); `crates/brightfield-render/src/scale.rs` (colour `Scale` variants `LogSequential { domain_min, domain_max, stops }` and `Quantile { thresholds: Vec<f64>, colours: Vec<[f32; 4]> }`); `crates/brightfield-render/src/legend.rs`
- Test: `scale.rs` and `legend.rs` test modules; a new example `examples/geo-quantile.yaml` in the PNG gate

**Interfaces:**
- Produces: `colorScale: quantile` with `colorN` (default 9, as Mosaic's unemployment example has it) cuts at the data's quantiles. `colorScale: log` maps `ln(v)` linearly, and a value ≤ 0 takes the low end, as `SYMLOG_CONSTANT`'s neighbour already does for axes. Both draw a legend.

- [ ] **Step 1: Failing tests:**
  - `quantile_colour_puts_equal_counts_in_each_step`: values 1…90 with `colorN: 9` give 10 values in each step.
  - `log_colour_is_linear_in_the_logarithm`: 1, 10 and 100 map to t = 0, 0.5 and 1.
  - `log_colour_takes_the_low_end_for_zero_and_negatives`
  - `the_parser_no_longer_warns_for_log_or_quantile`
- [ ] **Step 2: Run** `cargo test -p brightfield-render scale::` and `cargo test -p brightfield-spec colour_scale`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** them again. Expect PASS. Add `examples/geo-quantile.yaml` (the inline-GeoJSON regions of `examples/geo.yaml`, eight of them, `colorScale: quantile`, `colorN: 4`) and draw its PNG on the Mac Studio.
- [ ] **Step 5: Commit** `Draw log and quantile colour scales, with their legends`.

### Task 10: The unmatched note under a choropleth

**Files:**
- Modify: `crates/brightfield-shell/src/chart_item.rs` (beside the tile switches) and the session query path that `nearest_row` uses (`grep -rn "fn nearest_row" crates/brightfield-engine/src`)
- Test: `crates/brightfield-shell/tests/choropleth_kind.rs`

**Interfaces:**
- Consumes: `geometry_keys` and the relationship's `source_column`.
- Produces: `pub fn unmatched(&self, table: &str, key: &str, source_column: &str) -> Result<Unmatched, EngineError>` on the engine session, where `pub struct Unmatched { pub rows: u64, pub top: Vec<(String, u64)> }` holds up to five values, most rows first, ties broken by value. The note reads `"{rows} rows not drawn: {v1} {n1}, {v2} {n2}, …"`, below the plot in caption type. It counts the whole table, ignores the page's selection, and is hidden when `rows == 0`.

- [ ] **Step 1: Failing tests:**
  - `the_note_counts_and_names_unmatched_values`: fixture rows "EIRE"×3 (an alias, so it matches), "Unspecified"×2, "European Community"×1 give `"3 rows not drawn: Unspecified 2, European Community 1"`.
  - `no_note_when_every_row_matched`
  - `the_note_ignores_a_brush`
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test choropleth_kind unmatched`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** it again. Expect PASS.
- [ ] **Step 5: Commit** `Say beside a choropleth how many rows matched no country, and which`.

### Task 11: The starts: the earthquake backdrop and the retail choropleth

**Files:**
- Modify: `examples/starts/usgs-earthquakes-2025/{arcform.yaml, panels/dashboard.yaml}` (add the `countries` steps and Task 4's backdrop layer first in the map plot); `examples/starts/online-retail-ii/{arcform.yaml, models/, panels/dashboard.yaml}` (countries, `retail_orders_countries`, the model relationship, and a choropleth of orders by country beside "Orders, by country (top eight)", both on `$sel`); `crates/brightfield-shell/src/starts.rs` (raise both starts' `revision`); the bundled data under each start's `data/boundaries/`
- Test: `crates/brightfield-shell/tests/shipped_protocol_start.rs`, `start_interaction.rs`, `dashboard_baseline.rs` and the starts' thumbnails

- [ ] **Step 1: Failing tests:** `the_earthquake_map_draws_over_countries` and `the_retail_start_has_a_choropleth_beside_its_country_bar`. The latter asserts the unmatched note lists "Channel Islands", "European Community" and "Unspecified", and not "EIRE", "RSA" or "USA". The retail Protocol fetches from UCI, so mark that test `#[ignore]` with the network tests' reason, and also give it a hermetic counterpart over a 20-row cut of `retail_orders`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test shipped_protocol_start --test start_interaction`. Expect FAIL.
- [ ] **Step 3: Implement** the start edits by hand, through the same SQL Tasks 2 and 6 write.
- [ ] **Step 4: Redraw** every baseline these change on the Mac Studio (`UPDATE_SNAPSHOTS=1`), re-shoot the two starts' thumbnails, look at each in both themes, and list them for the PR. Then run `cargo test -p brightfield-shell --test shipped_protocol_start --test start_interaction --test dashboard_baseline` and expect PASS.
- [ ] **Step 5: Commit** `The earthquake map draws over the world's countries, and the retail start maps its orders by country`.

### Task 12: Fold the design into the docs, and the gate

**Files:**
- Create: `docs/maps.md`: where boundaries come from (bundled Natural Earth countries; finer sets hosted later), the backdrop, the choropleth and its join, the relationship that speeds it, the unmatched note, and Hugh's rulings from the spec with their reasons. Add one line in `CLAUDE.md` saying when to read it.
- Modify: `docs/interaction-speed.md` (the measured 47 ms against 3 ms, and that another Mosaic reader takes the first)
- Delete: `docs/superpowers/specs/2026-11-26-map-boundaries-design.md` and this plan

- [ ] **Step 1:** Write the docs and delete the spec and plan.
- [ ] **Step 2: Run** `cargo xtask ci --step fmt`, `--step clippy`, then commit `Fold the map-boundaries design into docs/maps.md`, then `cargo xtask ci --step comment-citations`.
- [ ] **Step 3:** Push and run `cargo xtask gate <pr>` on the Mac Studio. Expect a green `local-gate`. Because the PR touches the shell's end-to-end tests, also run `cargo test -p brightfield-shell --test choropleth_kind --test choropleth_interaction --test boundaries --test point_map_kind` before pushing.
- [ ] **Step 4:** Run `roborev review --branch --wait`, then fix or answer each finding at Medium and above, and close it.

## Dependencies and cost

| Task | Needs | Verification and its cost |
|---|---|---|
| 1 | — | arc run in open-analytics, about 1 min |
| 2 | — (fixture); Task 1 for the shipped files | `--test boundaries`, a few minutes with the shell build |
| 3 | — | `brightfield-render` tests, about 3 min |
| 4 | 2, 3 | two shell tests, Mac Studio redraw |
| 5 | — | `brightfield-sql` tests, under a minute; release timing test, about 2 min |
| 6 | 2, 5 | `--test boundaries` |
| 7 | 4, 6 | `--test choropleth_kind` |
| 8 | 7 | `--test choropleth_interaction`, `brightfield-ui` tests |
| 9 | — | render and spec tests, about 3 min; one PNG |
| 10 | 6 | `--test choropleth_kind` |
| 11 | 1, 4, 7, 8, 10 | three shell tests, baselines and thumbnails on the Mac Studio |
| 12 | all | the full gate, `cargo xtask gate`, about 30–40 min on the Mac Studio |

Tasks 1, 2, 3, 5 and 9 can start in parallel.
