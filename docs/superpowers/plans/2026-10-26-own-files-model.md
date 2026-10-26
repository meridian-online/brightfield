# An analyst's own files open as a first viable model — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opening a folder, an Excel workbook or a DuckDB/SQLite file profiles every table by its FineType types, writes an Apache Ossie model of how the tables relate, materialises cleaned (silver) tables through arc, and lets the analyst inspect and reshape the model on a role-laid-out diagram, a list or the Ossie file itself.

**Architecture:** A new framework-free crate, `brightfield-ossie`, owns the model: the pinned Ossie document types with Meridian's vendor extension, the analyst's edits, the merge that keeps the analyst's decisions, and the role-banded layout. The shell writes the analyst's Protocol folder (load steps, per-source data packages, the Ossie file, silver steps) and runs it with the arc child process it already uses. `brightfield-render` draws the model scene in Vello, and a new model pane hosts Diagram, List and Ossie-file views beside an inspector. dovetail joins later as a child process of the same binary, behind a discovery seam whose only implementation until then reads hand-written stand-in models.

**Tech Stack:** Rust 1.95 workspace, DuckDB 2.0 (`duckdb = "=1.20000.0"`, bundled), arcform (pinned rev, child process), FineType bundle (DuckDB extension) plus `finetype-core` (taxonomy), `calamine` (Excel), `rusqlite` with `bundled` (SQLite), `serde_yaml`, `jsonschema`, egui/eframe, Vello.

**Spec:** `docs/superpowers/specs/2026-10-26-own-files-model-design.md`

## Global Constraints

- One model per open, stored as `model.ossie.yaml`, Ossie version string `0.2.0.dev0`, validated against the Ossie JSON Schema vendored at one recorded `apache/ossie` commit.
- Meridian's extension: `custom_extensions` entries with `vendor_name: MERIDIAN`, `data` a JSON string; shapes are fixed in Task 1.
- Roles are exactly `entity`, `event`, `reference`, `bridge`, `snapshot`, `unclear`. Grades are exactly `likely`, `possible`. `decided_by` is exactly `dovetail` or `analyst`.
- An element whose `decided_by` is `analyst` is never changed by discovery, verification or rerun.
- Lineage columns on every loaded and silver table: `__source_file` (VARCHAR, path relative to the Protocol folder or absolute for files left in place), `__source_part` (VARCHAR, sheet or table name, NULL for a flat file), `__source_row` (BIGINT, 1-based data row, header excluded).
- The silver cast is FineType's transformation contract: `finetype-core` `Taxonomy::ddl_info(label)` → its `transform` with `{col}` replaced by the quoted column, wrapped in `TRY(...)`; no transform and non-VARCHAR → `TRY_CAST(col AS duckdb_type)`; VARCHAR or unlabelled → passthrough. This mirrors `build_transform_projection_one` in finetype's `crates/finetype-cli/src/transform_projection.rs` with `try_wrap = true`.
- `finetype-core` is pinned to the same tag as `FINETYPE_TAG` in `packaging/finetype-pin.env`; the `finetype-pin` gate step refuses a mismatch.
- No network at open time: Excel sheets and SQLite tables are read by `calamine` and `rusqlite` and staged as Parquet; DuckDB's `excel` and `sqlite` extensions are not used on this path.
- Every analyst action is a verb in `brightfield-keys`' registry. `v` cycles Diagram → List → Ossie file. `h`, `j`, `k`, `l` are not rebound.
- A single CSV, TSV or Parquet file keeps today's route (`data_file::open`) unchanged.
- No private planning identifiers in code, comments, commits, branch names or pull request text.
- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the change touches. After each commit: `cargo xtask ci --step comment-citations`. One cargo build at a time on the 16 GB Mac.

## Review Focus

1. **A folder holding files Brightfield can't read** (`.DS_Store`, `notes.txt`, a corrupt CSV): the open still succeeds with the readable tables, and the skipped files are named in the model pane's header. Pinned in Task 4.
2. **Two sources that would get the same table name** (`sales.csv` and `sales.parquet`, or a sheet named like a file): each table gets a distinct name, deterministically. Pinned in Task 4.
3. **Column names with spaces, quotes, dots or SQL keywords** (`"Order ID"`, `select`, `a"b`): load, silver and preview SQL all quote them, and the silver table keeps the original name. Pinned in Task 6.
4. **A workbook with an empty sheet or a sheet whose first row is blank**: the empty sheet is skipped and named; a blank first row is not taken as the header. Pinned in Task 4.
5. **A hand edit that breaks the YAML or the schema**: the last good model stays on screen, the file view shows the error with its line, nothing on disk is lost, and the next good save takes effect. Pinned in Task 13.

## Verification cost

- `cargo test -p brightfield-ossie`: the new crate has no DuckDB dependency, so after the first build it runs in seconds.
- `cargo test -p brightfield-engine`, `-p brightfield-protocol`, `-p brightfield-render`, `-p brightfield-shell`: these build DuckDB 2.0 from source; a cold first build is the long part, later runs are incremental. Run a single test by name where a step says so.
- `cargo xtask ci --step <name>`: one gate step. The full `cargo xtask ci` is for reproducing a CI failure only.

---

### Task 1: The `brightfield-ossie` crate — model types, the Meridian extension, read, write, validate

**Files:**
- Create: `crates/brightfield-ossie/Cargo.toml`, `crates/brightfield-ossie/src/lib.rs`, `crates/brightfield-ossie/src/model.rs`, `crates/brightfield-ossie/src/meridian.rs`, `crates/brightfield-ossie/src/validate.rs`
- Create: `crates/brightfield-ossie/vendor/ossie/osi-schema.json`, `crates/brightfield-ossie/vendor/ossie/README.md` (source URL, commit, date)
- Create: `crates/brightfield-ossie/fixtures/retail_extract/` — seven small CSVs (`customers`, `stores`, `products`, `categories`, `transactions`, `loyalty_visits`, `customer_segments`) plus `promo_codes.csv`, and `model.ossie.yaml`, the hand-written stand-in model
- Modify: `Cargo.toml` (workspace member; `jsonschema` in `[workspace.dependencies]`)
- Test: `crates/brightfield-ossie/tests/model_io.rs`

**Interfaces:**
- Produces:

```rust
pub const OSSIE_VERSION: &str = "0.2.0.dev0";
pub const VENDOR: &str = "MERIDIAN";

pub struct Model { pub name: String, pub datasets: Vec<Dataset>, pub relationships: Vec<Relationship>, pub meta: ModelMeta }
pub struct Dataset { pub name: String, pub source: String, pub primary_key: Vec<String>, pub fields: Vec<Field>, pub meta: DatasetMeta }
pub struct Field { pub name: String, pub expression_sql: String, pub datatype: Option<Datatype>, pub meta: FieldMeta }
pub struct Relationship { pub name: String, pub from: String, pub to: String, pub from_columns: Vec<String>, pub to_columns: Vec<String>, pub meta: RelationshipMeta }

pub enum Role { Entity, Event, Reference, Bridge, Snapshot, Unclear }
pub enum Grade { Likely, Possible }
pub enum DecidedBy { Dovetail, Analyst }
pub enum Verified { Sample, Full }

pub struct DatasetMeta { pub role: Role, pub role_evidence: Vec<String>, pub folded_into: Option<String>, pub excluded: bool, pub decided_by: DecidedBy, pub source_file: String, pub source_part: Option<String>, pub rows: Option<u64> }
pub struct FieldMeta { pub finetype: Option<String>, pub decided_by: DecidedBy }
pub struct Evidence { pub value_overlap: f64, pub orphans: u64, pub child_rows: u64, pub child_type: Option<String>, pub parent_type: Option<String>, pub types_agree: bool, pub name_similarity: f64 }
pub struct RelationshipMeta { pub grade: Grade, pub evidence: Evidence, pub cardinality: String, pub verified: Verified, pub decided_by: DecidedBy, pub warning: Option<String> }
pub struct ModelMeta { pub generator: String, pub rejected: Vec<Relationship>, pub skipped_sources: Vec<SkippedSource> }
pub struct SkippedSource { pub path: String, pub reason: String }

pub fn read(yaml: &str) -> Result<Model, ReadError>;   // ReadError carries line and column
pub fn write(model: &Model) -> String;                  // stable key order, deterministic
pub fn validate(yaml: &str) -> Vec<SchemaIssue>;        // against the vendored schema
```

A dropped relationship lives in `ModelMeta::rejected`, not in `relationships`, so the core document only asserts joins the model holds.

- [ ] **Step 1: Write the failing tests** in `tests/model_io.rs`:
  - `stand_in_reads_and_validates`: `read(fixtures/retail_extract/model.ossie.yaml)` succeeds, has 8 datasets, `transactions` has `Role::Event`, `categories` has `folded_into == Some("products")`, `promo_codes` has `Role::Unclear`, and `validate(...)` is empty.
  - `write_round_trips`: `read(write(&m)) == m` for the stand-in, and `write` is byte-identical across two calls.
  - `meridian_extension_survives_a_foreign_reader`: parsing the written YAML as plain `serde_yaml::Value` shows each `custom_extensions[].vendor_name == "MERIDIAN"` and `data` is a JSON string.
  - `unknown_vendor_extensions_are_kept`: a relationship carrying a `DBT` extension round-trips with it unchanged.
  - `bad_yaml_names_the_line`: `read("datasets: [")` errors with `line == 1`.
- [ ] **Step 2: Run** `cargo test -p brightfield-ossie --test model_io`. Expected: FAIL, crate missing.
- [ ] **Step 3: Implement** the types with serde (`model.rs`), the extension encode/decode (`meridian.rs`, unknown vendors carried as opaque pairs), `validate` with `jsonschema` against the vendored schema. Vendor `core-spec/osi-schema.json` from `apache/ossie` at the commit current on the day; record URL, commit and date in `vendor/ossie/README.md`.
- [ ] **Step 4: Run** `cargo test -p brightfield-ossie --test model_io`. Expected: PASS. Run `cargo xtask ci --step deny` for the new dependency's licence.
- [ ] **Step 5: Commit** — "Add brightfield-ossie: the pinned Ossie model with Meridian's extension".

### Task 2: Analyst edits and the merge that keeps them

**Files:**
- Create: `crates/brightfield-ossie/src/edit.rs`, `crates/brightfield-ossie/src/merge.rs`
- Test: `crates/brightfield-ossie/tests/edits.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:

```rust
pub enum Edit {
    KeepJoin { relationship: String },
    DropJoin { relationship: String },
    AddJoin { from: String, from_columns: Vec<String>, to: String, to_columns: Vec<String> },
    ChangeKeys { relationship: String, from_columns: Vec<String>, to_columns: Vec<String> },
    SetRole { dataset: String, role: Role },
    Fold { dataset: String, into: String },
    Unfold { dataset: String },
    Exclude { dataset: String },
    Include { dataset: String },
    Retype { dataset: String, field: String, finetype: String },
}
pub fn apply(model: &Model, edit: &Edit) -> Result<Model, EditError>;
pub fn merge_discovery(current: &Model, proposed: &Model) -> Model;
```

`apply` marks every element it touches `DecidedBy::Analyst`. `KeepJoin` turns a possible join into a kept one (grade stays as evidence says, `decided_by` analyst). `DropJoin` moves the relationship to `meta.rejected`. `AddJoin` names the relationship `{from}_{to}` with a numeric suffix on collision, and its grade is `Possible` with zeroed evidence until verified (Task 14). `Fold` requires a relationship between the two datasets and refuses otherwise. `merge_discovery` takes `proposed` for every element `current` holds as `Dovetail`, keeps every `Analyst` element of `current` unchanged, never re-adds a relationship that `current.meta.rejected` holds as `Analyst`, and keeps `current`'s excluded datasets excluded.

- [ ] **Step 1: Write the failing tests**: `drop_moves_to_rejected`, `keep_marks_analyst`, `add_join_names_and_dedupes`, `fold_without_join_is_refused`, `retype_marks_field_analyst`, `merge_keeps_analyst_role` (current sets `stores` to `Snapshot` as analyst, proposed says `Entity` → result `Snapshot`), `merge_does_not_resurrect_dropped`, `merge_takes_new_dovetail_grades`, `merge_is_idempotent` (`merge(m, m) == m`).
- [ ] **Step 2: Run** `cargo test -p brightfield-ossie --test edits`. Expected: FAIL.
- [ ] **Step 3: Implement** `apply` and `merge_discovery`; elements are matched by name (datasets) and by `(from, from_columns, to, to_columns)` (relationships).
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** — "Analyst edits to the Ossie model, and a discovery merge that never overrides them".

### Task 3: Role-banded layout, list order and edge labels

**Files:**
- Create: `crates/brightfield-ossie/src/layout.rs`
- Test: `crates/brightfield-ossie/tests/layout.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:

```rust
pub struct Card { pub dataset: String, pub x: f64, pub y: f64, pub width: f64, pub height: f64, pub band: Band }
pub enum Band { Events, Entities, Below }
pub struct EdgeLabel { pub relationship: String, pub text: String, pub grade: Grade, pub points: Vec<(f64, f64)> }
pub struct ModelLayout { pub width: f64, pub height: f64, pub cards: Vec<Card>, pub edges: Vec<EdgeLabel> }
pub fn layout(model: &Model) -> ModelLayout;
pub fn list_rows(model: &Model) -> Vec<ListRow>;   // grouped by role, possible-join review rows first
pub fn edge_text(r: &Relationship) -> String;      // "customer_id = customer_id  N:1"
pub fn summary(model: &Model) -> String;           // "7 tables · 6 joins · 1 possible · 1 unclear"
```

Events in the left band, entities in the right band, snapshots, bridges and unclear tables in a row below; excluded datasets are not placed; a folded dataset is not placed and is named on its host's card. Within a band, order by number of joins descending, then name. Coordinates are whole pixels; spacing comes from `meridian_design::spacing`, as `brightfield-protocol`'s layout does. Edge text joins columns with `, ` for composite keys.

- [ ] **Step 1: Write the failing tests**: `retail_bands` (transactions and loyalty_visits in `Events`, customers/stores/products in `Entities`, customer_segments and promo_codes in `Below`, categories absent), `layout_is_deterministic` (two calls equal; shuffling `datasets` order gives the same result), `edge_text_for_composite_keys`, `summary_counts_retail` equals `"7 tables · 6 joins · 1 possible · 1 unclear"` for the stand-in, `list_puts_possible_first`.
- [ ] **Step 2: Run** `cargo test -p brightfield-ossie --test layout`. Expected: FAIL.
- [ ] **Step 3: Implement** `layout`, `list_rows`, `edge_text`, `summary`; edges route from the event card's right edge to the entity card's left edge as a cubic through two control points; edges between cards in the same band route around the band's outer side.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** — "Lay out an Ossie model by table role".

### Task 4: Finding the tables in what was opened, and staging them with lineage

**Files:**
- Create: `crates/brightfield-engine/src/sources.rs`
- Modify: `crates/brightfield-engine/src/lib.rs` (export), `crates/brightfield-engine/Cargo.toml` (`calamine`, `rusqlite` with `bundled`), `Cargo.toml` (workspace deps)
- Create: `crates/brightfield-engine/tests/data/own_files/` — a folder fixture (two CSVs, one Parquet named like a CSV, a `.DS_Store`, a `notes.txt`, a corrupt CSV), `book.xlsx` (two sheets, one empty, one with a blank first row), `shop.sqlite` (two tables)
- Test: `crates/brightfield-engine/tests/sources.rs`

**Interfaces:**
- Produces:

```rust
pub enum Reader { Csv, Tsv, Parquet, StagedParquet, DuckdbTable { database: PathBuf } }
pub struct SourceTable { pub name: String, pub file: PathBuf, pub part: Option<String>, pub reader: Reader }
pub struct SourceSet { pub tables: Vec<SourceTable>, pub skipped: Vec<(PathBuf, String)> }
pub fn discover(path: &Path) -> Result<SourceSet, String>;
pub fn stage(set: &SourceSet, staging_dir: &Path) -> Result<SourceSet, String>;  // xlsx sheets and sqlite tables → Parquet; others unchanged
pub fn load_sql(table: &SourceTable, protocol_dir: &Path) -> String;             // CREATE OR REPLACE TABLE "raw_<name>" AS ... with the three lineage columns
```

`discover` reads a folder's top level only (no recursion), takes `csv`, `tsv`, `parquet`, `xlsx`, `duckdb`, `db`, `sqlite`, and lists everything else in `skipped` with a reason; a file that fails to open is skipped, not fatal. Table names are the file stem (or `stem_sheet`, or the database table name), lower-cased, non-alphanumerics to `_`; a collision appends `_2`, `_3` in path order. `stage` reads each sheet with `calamine` (first non-blank row is the header; an all-empty sheet is skipped with reason "empty sheet") and each SQLite table with `rusqlite`, writing `<staging_dir>/<name>.parquet` with `__source_file`, `__source_part`, `__source_row` already present, all data columns VARCHAR. `load_sql` adds lineage for CSV/TSV with `row_number() OVER ()` under `preserve_insertion_order`, and for Parquet with `read_parquet(..., file_row_number = true)` plus one.

- [ ] **Step 1: Write the failing tests**: `folder_takes_readable_and_names_skipped` (3 tables; `.DS_Store`, `notes.txt` and the corrupt CSV in `skipped`), `name_collision_is_deterministic` (`sales` and `sales_2`), `workbook_sheets_and_empty_sheet` (one table, empty sheet skipped with "empty sheet"), `blank_first_row_is_not_header`, `sqlite_tables_stage_to_parquet`, `lineage_rows_are_one_based` (loading a 3-row CSV yields `__source_row` 1, 2, 3 in file order), `duckdb_file_lists_its_tables`.
- [ ] **Step 2: Run** `cargo test -p brightfield-engine --test sources`. Expected: FAIL.
- [ ] **Step 3: Implement** `discover`, `stage`, `load_sql`.
- [ ] **Step 4: Run** the same command. Expected: PASS. Run `cargo xtask ci --step deny`.
- [ ] **Step 5: Commit** — "Find and stage the tables in a folder, workbook or database file, with row lineage".

### Task 5: The analyst's Protocol folder: load steps, profile, data packages

**Files:**
- Create: `crates/brightfield-shell/src/own_files.rs`
- Modify: `crates/brightfield-shell/src/lib.rs`
- Test: `crates/brightfield-shell/tests/own_files_protocol.rs`

**Interfaces:**
- Consumes: `sources::{discover, stage, load_sql}` (Task 4); `starts::datasets_dir()`; `run::Run` (the arc child process); `Session::profile_sources` on Parquet snapshots of the raw tables, as `panel_tiles.rs` snapshots arc's tables.
- Produces:

```rust
pub const OWN_DIR: &str = "own";   // under starts::datasets_dir()/protocols/
pub struct OwnProtocol { pub dir: PathBuf, pub sources: SourceSet, pub profiles: Vec<SourceProfile> }
pub fn create(opened: &Path) -> Result<OwnProtocol, String>;              // folder, arcform.yaml with load steps, staged files under sources/
pub fn run_loads(p: &OwnProtocol) -> Result<(), String>;                   // arc run; raw tables exist
pub fn profile(p: &mut OwnProtocol) -> Result<(), String>;                 // FineType labels per column
pub fn write_packages(p: &OwnProtocol) -> Result<Vec<PathBuf>, String>;    // packages/<source>/datapackage.json
```

Folder: `<datasets_dir>/protocols/own/<slug>-<n>/` where `slug` is the opened path's stem and `n` the first unused integer from 1. Contents: `arcform.yaml` (name = folder name, `engine: duckdb`, one step per table `load_<name>` → `models/01_load_<name>.sql`), `sources/` (staged Parquet), `packages/<source-stem>/datapackage.json` (Frictionless 2.0, one resource per table of that source, each field with `type` from the FineType label's Frictionless mapping and `x-finetype-label`). Data packages are what finetype will write once it can; this task writes them in Brightfield so the boundary file exists now.

- [ ] **Step 1: Write the failing tests** (they run arc as the shell's other arc tests do; FineType-typed assertions skip with a printed reason when no bundle is beside the test binary, as existing semantic tests do): `creates_numbered_folder` (two creates for the same input give `-1` and `-2`), `manifest_has_one_load_step_per_table`, `raw_tables_carry_lineage` (after `run_loads`, `raw_customers` has the three lineage columns), `packages_name_each_field`, `profile_labels_email_column` (bundle present: `customers.email`'s label ends in `.email`).
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test own_files_protocol`. Expected: FAIL.
- [ ] **Step 3: Implement** `create`, `run_loads`, `profile`, `write_packages`.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** — "Write an analyst's own files as a Protocol: load steps, profiles and data packages".

### Task 6: The silver cast, folds and failed values

**Files:**
- Create: `crates/brightfield-protocol/src/silver.rs`
- Modify: `crates/brightfield-protocol/src/lib.rs`, `crates/brightfield-protocol/Cargo.toml` (`finetype-core` with `embedded-taxonomy`, `brightfield-ossie`), `Cargo.toml` (`finetype-core` git dependency at the `FINETYPE_TAG` tag), `scripts/check-finetype-pin.sh` and `scripts/check-finetype-pin-selftest.sh` (refuse when the `finetype-core` tag differs from `FINETYPE_TAG`)
- Modify: `crates/brightfield-shell/src/own_files.rs` (`write_silver`)
- Test: `crates/brightfield-protocol/tests/silver.rs`, `crates/brightfield-shell/tests/own_files_protocol.rs`

**Interfaces:**
- Consumes: `brightfield_ossie::Model` (Task 1), `OwnProtocol` (Task 5).
- Produces:

```rust
pub fn cast_projection(column: &str, finetype: Option<&str>, taxonomy: &finetype_core::Taxonomy) -> String;
pub fn silver_sql(model: &Model, dataset: &str, taxonomy: &Taxonomy) -> String;   // CREATE OR REPLACE TABLE "<dataset>" ...
pub fn failed_sql(model: &Model, dataset: &str, taxonomy: &Taxonomy) -> String;   // CREATE OR REPLACE TABLE "<dataset>__failed" (__source_file, __source_part, __source_row, "column", "value")
// in own_files.rs:
pub fn write_silver(p: &OwnProtocol, model: &Model) -> Result<(), String>;        // models/02_silver_<name>.sql and arc steps; rewrites them on every call
```

`cast_projection` follows the global constraint's five branches exactly. `silver_sql` selects the three lineage columns, then each field's projection under its original name; a dataset with folded references `LEFT JOIN`s each folded dataset's silver-cast columns on the fold's relationship keys, prefixed `<folded>_`, excluding the join key. Excluded and folded datasets get no silver table. `failed_sql` unions, per typed column, the rows where the raw value is not NULL and the projection is NULL.

- [ ] **Step 1: Write the failing tests** in `tests/silver.rs`: `varchar_passes_through`, `transform_is_try_wrapped` (a date label whose taxonomy transform uses `strptime` → output starts `TRY(strptime(`), `no_transform_uses_try_cast`, `unlabelled_passes_through`, `awkward_names_are_quoted` (`Order ID`, `select`, `a"b` each appear as `"Order ID"`, `"select"`, `"a""b"` in the SQL and parse in DuckDB), `fold_left_joins_with_prefix` (products' silver has `categories_name`), `failed_table_shape`. In `own_files_protocol.rs`: `silver_runs_and_keeps_failures` (a CSV with one unparseable date: silver row has NULL, `__failed` has that row's `__source_row` and raw value).
- [ ] **Step 2: Run** `cargo test -p brightfield-protocol --test silver`. Expected: FAIL.
- [ ] **Step 3: Implement** `cast_projection`, `silver_sql`, `failed_sql`, `write_silver`, and the pin check.
- [ ] **Step 4: Run** `cargo test -p brightfield-protocol --test silver`, `cargo test -p brightfield-shell --test own_files_protocol`, `cargo xtask ci --step finetype-pin-selftest`, `cargo xtask ci --step finetype-pin`. Expected: PASS.
- [ ] **Step 5: Commit** — "Cast silver tables through FineType's transformation contracts, keeping failed values".

### Task 7: The front door opens folders, workbooks and database files

**Files:**
- Modify: `crates/brightfield-shell/src/data_file.rs` (`OPENABLE_EXTENSIONS`, `accept`, `pick`, `openable_prose`), `crates/brightfield-shell/src/window.rs` (the door's promise text at `OPEN_FILE_PROMISE`, `names_a_data_file`, the pick handler)
- Test: `crates/brightfield-shell/tests/front_door_open.rs`

**Interfaces:**
- Consumes: `own_files::{create, run_loads, profile, write_packages, write_silver}` (Tasks 5–6).
- Creates: `crates/brightfield-shell/src/discovery.rs` with the `Discovery` trait, `Pass`, `StandIn` and `discover_model` exactly as Task 15's Interfaces block gives them, and its first two tests (`stand_in_reads_fixture_model`, `stand_in_without_model_lists_unclear_tables`); Task 15 adds the dovetail child process.
- Produces: `pub enum Opened { OneTable(OpenedFile), Model(OwnProtocol) }` and `pub fn open_any(path: &Path) -> Result<Opened, String>`.

`OPENABLE_EXTENSIONS` becomes `["csv", "tsv", "parquet", "xlsx", "duckdb", "db", "sqlite"]`. The picker offers files and folders (`rfd::FileDialog::pick_folder` beside `pick_file`, behind one "Open…" button with a choice of file or folder). `open_any` routes a path whose `discover` yields exactly one flat-file table to `data_file::open` unchanged, and everything else to the own-files flow. The promise text names the new kinds: "A CSV, Parquet or Excel file, a folder of them, or a DuckDB or SQLite database on this machine".

- [ ] **Step 1: Write the failing tests**: `single_csv_keeps_todays_route` (returns `Opened::OneTable`), `folder_opens_a_model`, `workbook_opens_a_model`, `sqlite_opens_a_model`, `accept_still_refuses_urls`, `typed_path_to_folder_is_recognised` (`names_a_data_file` true for a directory).
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test front_door_open`. Expected: FAIL.
- [ ] **Step 3: Implement** `open_any` and the picker and acceptance changes.
- [ ] **Step 4: Run** the same command, then `cargo test -p brightfield-shell data_file`. Expected: PASS.
- [ ] **Step 5: Commit** — "The front door opens folders, workbooks and database files".

### Task 8: The compact header band names each column's FineType type

**Files:**
- Modify: `crates/brightfield-shell/src/column_header.rs` (compact density, `glyph_for`), `crates/brightfield-engine/src/semantic.rs` (`SchemaCatalogue` exposes `title(label) -> Option<&str>` and `icon(label) -> Option<&str>`)
- Create: `scripts/stage-finetype-bundle-for-source-run.sh`; document it in `README.md` beside the build instructions
- Test: `crates/brightfield-shell/tests/column_header_type.rs`; baselines in `crates/brightfield-shell/tests/snapshots/`

The compact band draws the label's title from the bundle's catalogue (and its icon when the catalogue carries one; the current catalogue carries none, so the icon slot is empty and the band does not reserve its width). The script stages the pinned bundle with `scripts/fetch-finetype-bundle.sh` where `bundle_beside` looks for a `cargo run` build.

- [ ] **Step 1: Write the failing tests**: `compact_band_reads_title_by_text` (a labelled email column's compact band contains the catalogue's title for that label), `no_icon_reserves_no_width`, plus light and dark baselines of the compact band.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test column_header_type`. Expected: FAIL.
- [ ] **Step 3: Implement** the catalogue accessors, the band, and the staging script.
- [ ] **Step 4: Run** the same command. Expected: PASS with baselines written once and reviewed.
- [ ] **Step 5: Commit** — "Name each column's FineType type in the compact header band".

### Task 9: The model scene

**Files:**
- Create: `crates/brightfield-render/src/model_scene.rs`
- Modify: `crates/brightfield-render/src/lib.rs`, `crates/brightfield-render/Cargo.toml` (`brightfield-ossie`)
- Test: `crates/brightfield-render/tests/model_scene.rs`; baselines `crates/brightfield-render/tests/snapshots/model_retail_{light,dark}.png`

**Interfaces:**
- Consumes: `brightfield_ossie::{Model, layout, ModelLayout}` (Tasks 1, 3).
- Produces: `pub struct ModelFocus { pub selected: Option<Selection> }`, `pub enum Selection { Dataset(String), Relationship(String) }`, `pub fn render_model(scene: &mut Scene, model: &Model, layout: &ModelLayout, focus: &ModelFocus, dark: bool)`, `pub fn hit_test(layout: &ModelLayout, x: f64, y: f64) -> Option<Selection>`.

Cards show role marker, name, rows, field count, and "+ <folded>" when references are folded in. Edges: likely solid, possible dashed in the design system's proposal colour, label pill with `edge_text` and the grade for possible joins, a warning marker when `meta.warning` is set. With a selection, everything not connected to it is drawn at the design system's dimmed ink. Colours, radii and type sizes come from `meridian_design`, as `asset_scene.rs` does.

- [ ] **Step 1: Write the failing tests**: `retail_baseline_light`, `retail_baseline_dark`, `selection_dims_unconnected` (pixel sample at an unconnected card's centre is the dimmed ink when `products.sku` join is selected), `hit_test_finds_card_and_edge_label`.
- [ ] **Step 2: Run** `cargo test -p brightfield-render --test model_scene`. Expected: FAIL.
- [ ] **Step 3: Implement** `render_model` and `hit_test`.
- [ ] **Step 4: Run** the same command. Expected: PASS; baselines committed after review.
- [ ] **Step 5: Commit** — "Draw an Ossie model as role-banded cards with labelled joins".

### Task 10: The model pane, its three views and `v`

**Files:**
- Create: `crates/brightfield-shell/src/model_pane.rs`
- Modify: the shell's pane registration (where `ItemSpec`s are built for the Protocol and Grid panes), `crates/brightfield-keys/src/registry.rs` (verb `cycle-model-view`, `ws("v")`, scope Model, `Drives::View`, scores filled), `crates/brightfield-keys/src/scope.rs` (Model scope)
- Test: `crates/brightfield-shell/tests/model_pane.rs`, `crates/brightfield-keys/src/registry.rs` unit tests

**Interfaces:**
- Consumes: Task 9's `render_model`, `hit_test`; Task 3's `list_rows`, `summary`.
- Produces: `pub enum ModelView { Diagram, List, File }`, `pub struct ModelPane { pub protocol: OwnProtocol, pub model: Model, pub view: ModelView, pub focus: ModelFocus }`, `pub fn save_model(&mut self, model: Model) -> Result<(), String>` (writes `model.ossie.yaml`, then `own_files::write_silver`, then re-runs the silver steps only).

The pane opens on Diagram with the summary line above it, and the skipped sources named after it ("2 files skipped: notes.txt (not a table), broken.csv (could not be read)"). List renders `list_rows` with a joins gutter. File shows the YAML (editing is Task 13). The palette offers "Show the model as a diagram", "…as a list", "…as the Ossie file".

- [ ] **Step 1: Write the failing tests**: `opens_on_diagram`, `v_cycles_three_views` (scripted keystrokes: `v` → List, `v` → File, `v` → Diagram), `l_still_moves_focus` (with the pane focused, `l` does not change the view), `skipped_sources_are_named`, `palette_lists_three_views`, and in the registry: `v_binding_is_unique_in_model_scope`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test model_pane` and `cargo test -p brightfield-keys registry`. Expected: FAIL.
- [ ] **Step 3: Implement** the pane, the verb and scope, and `save_model`.
- [ ] **Step 4: Run** both commands. Expected: PASS.
- [ ] **Step 5: Commit** — "A model pane with Diagram, List and Ossie-file views, cycled by v".

### Task 11: The inspector: evidence, live preview, table details

**Files:**
- Create: `crates/brightfield-shell/src/model_inspector.rs`
- Modify: `crates/brightfield-shell/src/model_pane.rs`
- Test: `crates/brightfield-shell/tests/model_inspector.rs`

**Interfaces:**
- Consumes: `ModelPane` (Task 10), the engine session over the Protocol's silver tables.
- Produces: `pub fn join_preview_sql(model: &Model, relationship: &str, limit: usize) -> String` (in `brightfield-ossie`, so it is testable without a window) and the inspector panel.

For a join: keys and cardinality, grade, each evidence item, "verified on the full data" or "sample only", and a preview of the joined rows (`limit` 20, sampled from the child side with `USING SAMPLE` before the join, so the parent side is complete for the sampled keys). Actions: Keep (possible only), Drop, Change keys…, Show in file. For a table: role (editable), its columns with their FineType titles as chips and their failed-value count, folded tables, its `__failed` rows (first 20, with `__source_file`, `__source_part`, `__source_row`). Actions: Change role, Fold into…, Unfold, Leave out, Retype column…, and **Open in grid**, which opens the table's silver rows in the Grid pane (and from there its dashboard and charts, as an opened file's table reaches them today). A join's inspector also has **Open in grid**, which opens the joined rows of `join_preview_sql` without the sample limit.

- [ ] **Step 1: Write the failing tests**: `preview_sql_samples_child_then_joins` (SQL contains `USING SAMPLE` in the child subquery and a `JOIN` to the parent), `preview_runs_on_retail` (returns ≤ 20 rows with columns from both tables), `table_inspector_lists_failed_rows_with_lineage`, `possible_join_offers_keep`, `likely_join_does_not_offer_keep`, `open_in_grid_shows_silver_rows`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test model_inspector`. Expected: FAIL.
- [ ] **Step 3: Implement** `join_preview_sql` and the inspector.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** — "Inspect a join's evidence and joined rows, and a table's types and failures".

### Task 12: Reshape verbs

**Files:**
- Modify: `crates/brightfield-keys/src/registry.rs` (verbs `keep-join`, `drop-join`, `add-join`, `change-join-keys`, `set-role`, `fold-table`, `unfold-table`, `leave-out-table`, `include-table`, `retype-column`; `Drives::SpecEdit`-class durable writes; keys chosen and scored in the registry, none of `h j k l v`), `crates/brightfield-shell/src/model_pane.rs` (dispatch → `brightfield_ossie::apply` → `save_model`; drag from a column on one card to a column on another starts `add-join`)
- Test: `crates/brightfield-shell/tests/model_reshape.rs`

**Interfaces:**
- Consumes: `Edit`, `apply` (Task 2); `save_model` (Task 10).

Each verb reaches `apply` with the selection as its subject, writes the file, regenerates silver, and refreshes the preview; the diagram redraws from the new model in the same frame the file is written. A refused edit (fold without a join) leaves the model unchanged and says why in the status line.

- [ ] **Step 1: Write the failing tests**: `drop_join_rewrites_file_and_diagram` (file's `relationships` loses it, `meta.rejected` gains it, layout no longer has the edge), `set_role_moves_card_band`, `fold_rewrites_silver` (products' silver SQL gains the folded columns), `refused_fold_explains`, `every_reshape_verb_is_registered_and_reachable_from_palette`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test model_reshape`. Expected: FAIL.
- [ ] **Step 3: Implement** the verbs, dispatch and the column-to-column drag.
- [ ] **Step 4: Run** the same command and `cargo test -p brightfield-keys`. Expected: PASS.
- [ ] **Step 5: Commit** — "Reshape the model from the diagram, the list and the keyboard".

### Task 13: Editing the Ossie file by hand

**Files:**
- Modify: `crates/brightfield-shell/src/model_pane.rs` (File view editor), reusing the shell's existing text editor widget from `editor.rs`
- Test: `crates/brightfield-shell/tests/model_file_edit.rs`

On each pause in typing (the editor's existing debounce), the text is `read` and `validate`d. Valid: diff against the current model; elements changed by the edit become `decided_by: analyst`; `save_model`. Invalid: keep the last good model on screen, show the error with its line, write nothing. A valid edit that names something Brightfield cannot act on (a relationship to a dataset that does not exist, a role outside the six, a metric) is listed under the editor as one warning per item, and the rest of the edit takes effect. Selecting a join or table in another view scrolls the file to it.

- [ ] **Step 1: Write the failing tests**: `broken_yaml_keeps_last_good` (diagram unchanged, file on disk unchanged, error names line 3), `valid_edit_redraws` (changing a role in the text moves the card), `hand_edit_marks_analyst`, `unknown_dataset_warns_and_rest_applies`, `selection_scrolls_file`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test model_file_edit`. Expected: FAIL.
- [ ] **Step 3: Implement** the editor wiring and warnings.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** — "Hand-edit the Ossie file, with the picture following and warnings for what can't apply".

### Task 14: Re-verify on rerun, and Rediscover

**Files:**
- Create: `crates/brightfield-ossie/src/verify.rs`
- Modify: `crates/brightfield-shell/src/model_pane.rs`, `crates/brightfield-keys/src/registry.rs` (verb `rediscover-model`)
- Test: `crates/brightfield-ossie/tests/verify.rs`, `crates/brightfield-shell/tests/model_rerun.rs`

**Interfaces:**
- Produces: `pub fn evidence_sql(model: &Model, relationship: &str) -> String` (overlap, orphans, child rows, parent uniqueness over silver tables) and `pub fn apply_evidence(model: &Model, relationship: &str, measured: Evidence) -> Model`.

After a Protocol rerun, each relationship's evidence is re-measured. When orphans rise above zero on a likely join, or parent uniqueness breaks, `meta.warning` is set ("12 orders now point at no customer") and the edge shows it; grade and `decided_by` do not change. An analyst-added join gets its first measured evidence here. Rediscover calls the discovery seam (Task 15) and `merge_discovery`.

- [ ] **Step 1: Write the failing tests**: `new_orphans_warn_without_regrading`, `added_join_gets_measured_evidence`, `rediscover_keeps_analyst_decisions` (with the stand-in seam).
- [ ] **Step 2: Run** `cargo test -p brightfield-ossie --test verify` and `cargo test -p brightfield-shell --test model_rerun`. Expected: FAIL.
- [ ] **Step 3: Implement** `evidence_sql`, `apply_evidence`, the rerun hook and the verb.
- [ ] **Step 4: Run** both commands. Expected: PASS.
- [ ] **Step 5: Commit** — "Re-verify joins on rerun and rediscover around the analyst's decisions".

### Task 15: The discovery seam, and dovetail as a child process

**Files:**
- Create: `crates/brightfield-shell/src/discovery.rs`
- Modify: `crates/brightfield-shell/src/main.rs` (a second runner environment variable beside `run::RUNNER_ENV`, handing the process to dovetail's CLI entry point), `Cargo.toml` (dovetail git dependency, `default-features = false`, added only when dovetail's DuckDB 2.0 release exists)
- Test: `crates/brightfield-shell/tests/discovery.rs`

**Interfaces:**
- Produces:

```rust
pub trait Discovery { fn discover(&self, protocol: &OwnProtocol, pass: Pass) -> Result<Model, String>; }
pub enum Pass { Sample, Full }
pub struct StandIn;      // returns the model at <protocol>/model.ossie.yaml if present, else every table as an `unclear` dataset with no joins
pub struct Dovetail;     // child process: reads packages/ and the raw tables, writes model.ossie.yaml
pub fn discover_model(protocol: &OwnProtocol) -> Result<Model, String>;  // Sample, then Full on a worker; merges each into the current model
```

`StandIn`, the trait and `discover_model` land in Task 7. This task adds the sample-then-full merge test and the `Dovetail` implementation; the `Dovetail` implementation and its dependency land when dovetail has shipped DuckDB 2.0, Ossie output, table roles and the sample-then-verify passes; until then this task's `Dovetail` tests are written and `#[ignore]`d with that reason.

- [ ] **Step 1: Write the failing tests**: `stand_in_reads_fixture_model`, `stand_in_without_model_lists_unclear_tables`, `sample_then_full_merges_without_flicker` (the model after Full keeps every analyst element set between the two passes), `dovetail_child_writes_valid_ossie` (ignored until dovetail ships).
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test discovery`. Expected: FAIL.
- [ ] **Step 3: Implement** the trait, `StandIn`, `discover_model`, and the runner hand-off.
- [ ] **Step 4: Run** the same command. Expected: PASS (the dovetail test ignored with its reason).
- [ ] **Step 5: Commit** — "A discovery seam with a stand-in, ready for dovetail as a child process".

### Task 16: Fold the design into the repo's docs

**Files:**
- Modify: `README.md` (opening folders, workbooks and database files; the model pane; the Protocol folder layout), `CHANGELOG.md` (Unreleased), `docs/` architecture notes for `brightfield-ossie`
- Delete: `docs/superpowers/specs/2026-10-26-own-files-model-design.md`, `docs/superpowers/plans/2026-10-26-own-files-model.md`

- [ ] **Step 1:** Write the README and CHANGELOG sections from what shipped, quoting test totals from the CI run, not a local run.
- [ ] **Step 2:** Delete the spec and this plan.
- [ ] **Step 3: Run** `cargo xtask ci --step comment-citations` and the doc-claim steps the gate lists (`cargo xtask ci --list`). Expected: PASS.
- [ ] **Step 4: Commit** — "Document opening an analyst's own files as a model, and retire the spec and plan".
