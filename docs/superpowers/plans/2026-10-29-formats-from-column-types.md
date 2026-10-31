# Numbers and dates formatted from a column's type — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A file opens with its money, rates, years and sizes shown from each column's FineType label and profile, on tile axes, in the grid and in the tooltip, and the generated chart file says the same thing in Mosaic's own terms.

**Architecture:** The source's `datapackage.json` is the column record (label, accepted marker, how the file wrote the column, the analyst's preference), read back on every open. The flat route casts in its in-memory copy, following FineType's own projection. One representation rule combines FineType's display block for the label with what the profile measured and the record's preference; the tile generator writes its result into the YAML, and the grid and tooltip call it when they draw.

**Tech Stack:** Rust; DuckDB through `brightfield-engine`; Mosaic YAML parsed by `brightfield-spec`; d3-format and d3-time-format ports in `brightfield-spec`; egui grid and tooltip in `brightfield-shell`; FineType's vendored exports in `brightfield-protocol/vendor/finetype`.

**Spec:** `docs/superpowers/specs/2026-10-29-formats-from-column-types-design.md`

## Global Constraints

- Nothing written into a chart file may draw differently in another Mosaic tool: no private attribute, no `G`→`B` substitution, only d3-format / d3-time-format specifiers and Mosaic spec forms.
- Only `$` reaches a tick format; any other currency symbol goes in the axis title, the grid header and the tooltip.
- Brightfield keeps no table of how a label is shown; that comes from FineType's vendored display export (Task 7).
- Brightfield's additions to a `datapackage.json` field live in one object, `x-brightfield`, with `written` and `prefer` members, exactly as the spec's example. View state never goes in the package.
- The tooltip's number–unit separator is U+00A0; a percent sign is attached (`45%`).
- A column is cast only when its label is trusted, FineType's projection casts it to a type a format needs (an integer, float or decimal type, DATE, TIME, a TIMESTAMP form or BOOLEAN), and every non-null value casts. A label whose cast targets anything else (JSON, UUID, INTERVAL, the coordinates struct) is read as it is: such a cast may not even bind, and one cast that fails to bind would leave the whole copy uncast.
- The public repo rule: no private planning identifiers in code, comments, commits, branch names or PR text.
- On this Mac: one cargo build at a time. Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the task names. After each commit: `cargo xtask ci --step comment-citations`. Never `cargo fmt --all`.

## Review Focus

1. **A file whose money column mixes `£` and `$`.** Expected: the column is still cast and drawn; no symbol is recorded, the title names none, and nothing prints the wrong symbol. Test in Task 6.
2. **A column with a single unparseable value among thousands** (`n/a` in an amount column). Expected: it is copied as read, not cast, and its tile is what it was before this work. Test in Task 6.
3. **A file that is edited or re-exported after its record was written.** Expected: its inferred labels and `written` are derived again, and the analyst's `nominated` labels and `prefer` formats are kept. Test in Task 4.
4. **A year column with a run longer than the ceiling, or with one year only.** Expected: a long run leaves the domain to the data; a single year is refused as today (one value). Test in Task 9.
5. **Negative and zero values on a scaled money axis** (−£0.3bn to £2.1bn). Expected: the scale is chosen from the larger magnitude, ticks print `−0.3` with d3's minus sign, and zero prints `0`. Test in Task 8.

---

## Order and dependencies

| Task | Needs | Waits on FineType |
|---|---|---|
| 1. Bin over an expression | — | no |
| 2. Grouped default numbers | — | no |
| 3. Whole-number bands take a format | — | no |
| 4. The column record | — | no |
| 5. Given labels skip the classifier | 4 | no |
| 6. Cast the flat copy | 4, 5 | no |
| 6b. Protocol columns record how they were written; stale records are pruned | 4, 6 | no |
| 7. Vendor FineType's display export | — | **yes**: display blocks, and the percentage cast storing a fraction |
| 8. The representation rule | 4, 6, 6b, 7 | via 7 |
| 9. Tiles write their representation | 1, 3, 6, 8 (and a band domain read from a list, Step 0) | via 7 |
| 10. The grid formats labelled columns | 8 | via 7 |
| 11. The tooltip formats compactly | 8 | via 7 |
| 12. The inspector's "auto" | 8, 9 | via 7 |
| 13. Fold into the architecture docs | all | — |

Tasks 1–6 and 6b can land and ship before FineType's release; each leaves the app working. Tasks 8–12 start once Task 7's vendored release is on the branch.

---

### Task 1: A bin can take an expression

**Files:**
- Modify: `crates/brightfield-spec/src/parse.rs` (`bin_transform`, ~2895)
- Modify: the query builder that turns a bin into SQL (follow `bin_transform`'s callers in `brightfield-spec` / `brightfield-engine`)
- Test: in-module tests beside `bin_transform`; `crates/brightfield-engine/tests/` for the emitted SQL and the brush predicate

**Interfaces:**
- Produces: a bin whose operand is a SQL expression, written in the YAML in the spelling Mosaic's spec uses for an expression in `bin` (Task 9 writes it).

- [ ] **Step 1: Pin the spelling.** Read `TransformField` in Mosaic's `packages/vgplot/spec/src/spec/Transform.ts` (main) and record in the parser's doc comment the exact YAML form of an expression operand (expected: an object with an `sql` key holding the expression text). If Mosaic has no expression form for `bin`, stop and flag the task: the spec's scaling depends on it.
- [ ] **Step 2: Write the failing tests.** `bin_takes_an_expression`: parsing `x: { bin: { sql: "\"revenue\" / 1e9" } }` yields a bin over that expression. `binned_expression_query_divides`: the emitted SQL bins `("revenue" / 1e9)`. `brush_over_a_binned_expression_filters_in_its_units`: an interval brush of `[1.0, 2.0]` on that tile filters another tile's rows to `revenue / 1e9 BETWEEN 1.0 AND 2.0`.
- [ ] **Step 3: Run them to see them fail.** `cargo test -p brightfield-spec bin_takes_an_expression` → FAIL (parse refuses a mapping).
- [ ] **Step 4: Implement.** `bin_transform` returns the operand as an enum `BinOperand { Column(String), Expression(String) }`, not a `String`; every caller matches on it. A bare column stays byte-identical in emitted SQL.
- [ ] **Step 5: Run the three tests and the crate's existing bin tests.** `cargo test -p brightfield-spec bin` and `cargo test -p brightfield-engine bin` → PASS.
- [ ] **Step 6: Commit** — "A chart's bin can take an expression, as Mosaic's spec allows".

### Task 2: Unformatted numbers group thousands

**Files:**
- Modify: `crates/brightfield-render/src/axis.rs:695` (`format_number`)
- Modify: `examples/line-y.yaml`, `examples/starts/epoch-ai-models/panels/dashboard.yaml` (both plots, including the binned one) and `tests/fixtures/agent-protocols/co2-per-capita/panels/co2_per_capita_since_1990.yaml` (add `xTickFormat: d` where a year sits on a number axis without one)
- Modify: committed PNG snapshots that move (re-record only)

- [ ] **Step 1: Write the failing test** in `axis.rs` tests: `default_numbers_group_thousands` — `format_number(2024.0) == "2,024"`, `format_number(12000.0) == "12,000"`, `format_number(1234.5) == "1,234.5"`, `format_number(-1500.0) == "\u{2212}1,500"`, `format_number(999.0) == "999"`.
- [ ] **Step 2: Run it to see it fail.** `cargo test -p brightfield-render default_numbers_group_thousands` → FAIL.
- [ ] **Step 3: Implement** by formatting through `brightfield_spec::number_format::NumberFormat::parse(",")` for whole numbers and `",.1f"` otherwise, so the minus sign and rounding match d3.
- [ ] **Step 4: Keep years ungrouped in committed charts.** Add `xTickFormat: d` to each committed chart that puts a year on a number axis without a format (`git grep -nE "(x|y): *(\{ *bin: *)?[a-z_]*year"` over `examples/` and `tests/fixtures/` finds them, bare and binned: `examples/line-y.yaml`, `examples/starts/epoch-ai-models/panels/dashboard.yaml`, and the co2 fixture). Add `committed_year_axes_stay_ungrouped`: `examples/line-y.yaml` renders ticks `2018` … `2022`.
- [ ] **Step 5: Run the render and shell snapshot tests**; re-record the PNGs that moved, using the mechanism `crates/brightfield-shell/tests/dashboard_baseline.rs` documents. Open three re-recorded PNGs and check only tick text changed.
- [ ] **Step 6: Commit** — "Unformatted axis numbers group thousands, as Plot's default does". The commit holds `axis.rs`, the year charts' `xTickFormat: d` and the PNGs only.

### Task 3: A band of whole numbers takes a number format

**Files:**
- Modify: `crates/brightfield-render/src/axis.rs` (`axis_kind`, ~211–286)
- Test: in-module tests

- [ ] **Step 1: Write the failing tests.** `whole_number_band_is_a_number_axis`: a band with categories `["2019","2020","2024"]` has `axis_kind == Some(AxisKind::Number)` and `xTickFormat: d` prints `2024`. `mixed_band_is_still_names`: `["2019","n/a"]` takes no format. `unsorted_integer_band_is_in_value_order`: counts per year over `999`, `1000`, `2024` draw in that order, because the renderer lays a numeric band out in value order.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** A band whose every category parses as an `i64` is a number axis for formatting; its category order is untouched. A numeric column on a band is laid out in value order by the renderer whatever `sort:` says, which is separate from this work. The ISO-day rule is unchanged.
- [ ] **Step 4: Run** `cargo test -p brightfield-render axis` → PASS.
- [ ] **Step 5: Commit** — "A band of whole numbers takes a number format".

### Task 4: The column record

**Files:**
- Create: `crates/brightfield-shell/src/column_record.rs`
- Modify: `crates/brightfield-shell/src/own_files.rs` (`field`, ~412: write `x-brightfield` when the record has one)
- Modify: `crates/brightfield-shell/src/lib.rs` (module)
- Test: in-module tests; `crates/brightfield-shell/tests/own_files_protocol.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct FieldRecord {
      pub label: Option<String>,          // x-finetype-label
      pub nominated: bool,                // x-finetype-nominated
      pub written: Written,               // x-brightfield.written
      pub prefer: Prefer,                 // x-brightfield.prefer
  }
  pub struct Written { pub currency_symbol: Option<String>, pub negatives: Option<Negatives> }
  pub enum Negatives { Minus, Parentheses }
  pub struct Prefer { pub number_format: Option<String>, pub date_format: Option<String> }
  pub struct SourceRecord { pub fields: BTreeMap<String, FieldRecord> }

  pub fn read_record(package: &Path, resource: &str) -> Result<Option<SourceRecord>, String>;
  pub fn write_record(package: &Path, resource: &str, record: &SourceRecord) -> Result<(), String>;
  pub fn flat_record_path(file: &Path) -> Result<PathBuf, String>;
  ```
  `flat_record_path` answers `starts::datasets_dir()/records/<key>/datapackage.json`, where `<key>` hashes the file's canonical path only. The record stores the file's byte size and modification time as `x-brightfield-source: { "size": …, "modified": … }` on the resource.
  ```rust
  pub fn refresh_if_changed(record: &mut SourceRecord, file: &Path) -> Result<bool, String>; // true when size or mtime differed: clears inferred labels and `written`, keeps `nominated` and `prefer`
  ```

- [ ] **Step 1: Write the failing tests.** `record_round_trips_the_spec_example`: writing then reading the spec's `revenue` field gives back `£`, `Parentheses`, `",.0f"`, label `finance.currency.amount`, `nominated == false`. `record_keeps_other_properties`: a field with `x-finetype-confidence` keeps it after a write. `edited_file_keeps_choices` (Review Focus 3): after the file's modification time changes, `flat_record_path` answers the same path, `refresh_if_changed` returns `true`, the inferred label and `written` are cleared, and a nominated label and a `prefer.number_format` survive. `protocol_package_carries_x_brightfield` in `own_files_protocol.rs`: a Protocol written for a workbook keeps an `x-brightfield` object placed on a field across `write_packages`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement** with `serde_json::Value` editing of the existing package, so unknown properties survive; an absent `x-brightfield` reads as empty `Written` / `Prefer`.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell column_record` and `cargo test -p brightfield-shell --test own_files_protocol` → PASS.
- [ ] **Step 5: Commit** — "Keep what a source taught Brightfield on its datapackage fields".

### Task 5: A given label skips the classifier, and a cast copy keeps its labels

**Files:**
- Modify: `crates/brightfield-engine/src/semantic.rs` (~1051, where `ft_profile` is asked per column) and `LoadOptions` in `crates/brightfield-engine/src/lib.rs`
- Modify: `crates/brightfield-shell/src/data_file.rs` (`columns_of`, ~901; `open_any`, ~970)
- Test: `crates/brightfield-shell/tests/own_files_protocol.rs`, `crates/brightfield-shell/tests/front_door_open.rs`

**Interfaces:**
- Consumes: `read_record`, `flat_record_path` (Task 4).
- Produces: `LoadOptions::given_labels: BTreeMap<String, String>` (column → label), keyed by bare column name, so a label applies to a column of that name in every source the session loads (a flat open loads one source); a given column's `ColumnProfile::semantic` is `Labelled` with that label and no `ft_profile` term is issued for it. `OpenOptions::already_cast: bool`.

- [ ] **Step 1: Write the failing tests.** `nominated_label_is_not_asked`: with a record nominating `amount` as `finance.currency.amount`, the profile's SQL has no `ft_profile` term for `amount` and its semantic label is the nominated one. `single_sheet_cast_copy_keeps_its_currency_label`: opening a one-sheet workbook whose sheet has a `$1,234.50` column gives the grid's source a column labelled `finance.currency.amount`, read from the Protocol's package, not re-profiled. `cast_copy_is_not_cast_again`: a one-sheet workbook whose `finance.currency.amount` column holds `(£12.00)` reopens with `-12.00` in its cast copy, and the open succeeds (casting the DECIMAL again would fail or change it). `copy_inside_a_protocol_folder_is_recognised`: opening the grid copy's Parquet path directly, as a recent file, also takes the Protocol's labels and opens as already cast.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** `columns_of` reads the record for the file and passes nominated labels as `given_labels`. A file under `<protocol>/.grid/`, where `<protocol>` holds an own-files `arcform.yaml` (check for that file alone, not `own_protocol_dir`, which needs a model file a one-table Protocol never writes), whose stem names a resource in the Protocol's `packages/`, is that Protocol's copy: its labels come from that record as given, and it opens with `already_cast`. Any other file under that `.grid/` (join previews, model dataset snapshots, built from silver tables) also opens `already_cast`, with no given labels. A `.profile/` snapshot holds the raw, uncast table, so it takes the normal flat route and is cast. `open_any`'s one-part branch passes the same directly.
- [ ] **Step 4: Run** the three tests plus `cargo test -p brightfield-engine semantic` → PASS.
- [ ] **Step 5: Commit** — "An accepted label is taken as given, and a cast copy keeps its labels".

### Task 6: The flat route casts in its copy

**Files:**
- Modify: `crates/brightfield-protocol/src/silver.rs` (make `cast_expr` `pub`, unchanged behaviour)
- Modify: `crates/brightfield-engine/src/lib.rs` (`materialise_source`, 3322)
- Modify: `crates/brightfield-shell/src/data_file.rs` (`open_traced`, 701: order of profile, copy, aggregate, dashboard)
- Modify: `crates/brightfield-shell/src/data_grid.rs` (hide raw columns)
- Test: `crates/brightfield-shell/tests/open_materialise.rs`; new fixture `crates/brightfield-shell/tests/data/typed_formats.csv`

**Interfaces:**
- Consumes: `cast_expr(column, finetype, transforms) -> Option<String>` (now `pub`), `given_labels`, `already_cast`, `write_record`.
- Produces:
  ```rust
  // engine
  pub fn materialise_source_with(&mut self, name: &str, budget_bytes: u64, projection: &[String]) -> Result<(), EngineError>;
  // shell, data_file.rs
  pub const RAW_PREFIX: &str = "__raw__";
  struct CastPlan { casts: Vec<(String /*column*/, String /*expr*/)> }
  ```
  A candidate's label must cast to a type a format needs (see Global Constraints); `CastPlan::of` reads any other label's column as it is, and `a_label_no_format_needs_is_read_as_it_is` holds a JSON and a UUID column beside a cast `revenue` to that. The file is read once. The copy projects, for each candidate column, `TRY(<transform over CAST("c" AS VARCHAR)>) AS "c"` and `"c" AS "__raw__c"`, as `silver::source` reads cast columns as text first. One aggregate over the in-memory copy then decides each candidate (`count("__raw__c") = count("c")`), measures the cast columns' `min`, `max` and `moments`, and reads `written` from the raw values. A candidate that failed is put back: its cast column is dropped and its raw column renamed to `c`. `Dashboard::of` runs on the updated profiles; `written` is saved with `write_record`.

- [ ] **Step 1: Write the fixture.** `typed_formats.csv`, 200 rows: `revenue` (sterling amounts written `£1,234.50`, some `(£12.00)`, spanning roughly £0.2bn to £3.1bn), `price_usd` (`$12.50`…`$980.00`), `share` (`12%`…`78%`), `yield` (`4.1%`…`4.6%`), `year` (2001–2024 with 2009 absent), `downloads` (integers up to 4,000,000), `day` (ISO dates), `notes` (free text). Keep it generated by a short committed script beside it, so the values are reproducible.
- [ ] **Step 2: Write the failing tests.** `flat_open_casts_a_currency_column`: `revenue` is DECIMAL in the copy and its tile is a histogram. `raw_text_survives_the_cast`: `__raw__revenue` holds `(£12.00)` and the grid lists no `__raw__` column. `one_bad_value_keeps_the_column_text` (Review Focus 2): with one `n/a` added to `price_usd`, it is not cast and its tile matches the pre-change kind. `mixed_symbols_record_none` (Review Focus 1): with one `$` row in `revenue`, it is cast and its record's `currency_symbol` is `None`. `parentheses_are_recorded`: `revenue`'s record has `Negatives::Parentheses`. `over_budget_file_is_not_cast`: with a budget of 1 byte, nothing is cast. `cast_open_reads_the_file_once`: the copy and the deciding aggregate record their statements in a tally `open_traced` exposes (for example `OpenTrace::cast: ScanTally`, since `OpenTrace::composition` covers only the first composition); the aggregate's `file_reads` is 0 and the copy reads the file exactly once. `typed_dates_survive`: the ISO `day` column, which DuckDB loads as DATE, opens and stays DATE.
- [ ] **Step 3: Run them to see them fail.**
- [ ] **Step 4: Implement** as the Interfaces block says. Expose from `silver.rs` one helper that gives the cast over `CAST(col AS VARCHAR)`, so the flat route and `silver::source` share it. A column DuckDB already stored as the label's target type is not a candidate. Columns FineType's projection passes through (including the twelve labels FineType types as VARCHAR) stay text: follow FineType, do not second-guess it.
- [ ] **Step 5: Run** `cargo test -p brightfield-shell --test open_materialise` and `cargo test -p brightfield-engine materialise` → PASS.
- [ ] **Step 6: Commit** — "Cast a lone CSV's columns in the copy it is opened from".

### Task 6b: Protocol columns record how they were written; stale records are pruned

Does not wait on FineType. It lands before Task 8, whose rule reads `written` for every route.

A Protocol (a folder, workbook or database) casts in its silver table, and the spec reads `written` "when the column is cast", but nothing fills it there: `carry_over` keeps an `x-brightfield` object across `write_packages`, the one-part open arrives `already_cast`, and `keep_record` runs on the lone-file route only. So a workbook's sterling column would reach the representation rule with no symbol and no negatives. Separately, a lone file's record under `records/` is never removed when its file goes, and the spec says such records must not accumulate indefinitely.

**Files:**
- Modify: `crates/brightfield-shell/src/own_files.rs` (`write_packages`, ~327: read `written` from each cast column's raw values as the package is written)
- Modify: `crates/brightfield-shell/src/flat_cast.rs` (move the per-column `written` terms of `decide_casts` into a helper both routes call)
- Modify: `crates/brightfield-shell/src/column_record.rs` (`carry_over`, ~332: merge `x-brightfield` member by member instead of replacing it; stamp the file's canonical path on a lone file's record; `prune_records`)
- Modify: the first flat open of a process, or startup, to call `prune_records` once
- Test: `crates/brightfield-shell/tests/own_files_protocol.rs`; in-module tests in `column_record.rs`

**Interfaces:**
- Consumes: `Written`, `Negatives`, `write_record` (Task 4); the cast list `decide_casts` builds (Task 6).
- Produces:
  ```rust
  // flat_cast.rs: the aggregate terms that read a currency column's symbol and negatives from its text
  pub(crate) fn written_terms(raw: &str) -> [String; 2];
  pub(crate) fn written_from(symbol: Option<String>, negatives: Option<String>) -> Written;
  // column_record.rs
  pub fn prune_records(datasets: &Path) -> Result<usize, String>; // records removed
  ```
  `SourceStamp` gains `path: Option<PathBuf>`, the canonical path the record's key was hashed from.

- [ ] **Step 1: Write the failing tests.** `protocol_package_records_how_a_column_was_written` in `own_files_protocol.rs`: a one-sheet workbook whose `amount` column holds `£1,234.50` and `(£12.00)` writes a package whose `amount` field carries `x-brightfield.written` with `currency_symbol: "£"` and `negatives: "parentheses"`; a column the silver table does not cast carries no `written`. The test then writes the packages a second time, after the sheet's `amount` values change to `$1,234.50` and `-$12.00` and a `prefer.number_format` is put on the field, and asserts the second package's `written` is `currency_symbol: "$"`, `negatives: "minus"`, with the `prefer` kept. One write alone would pass with `carry_over` as it stands, which replaces the fresh field's whole `x-brightfield` with the old one. `protocol_written_is_read_from_the_raw_table`: the values are read from the raw table the silver cast reads, not from the cast column, whose text no longer holds a symbol. `a_record_whose_file_is_gone_is_pruned`: a record stamped with a path that no longer exists, inside a folder that does, is removed by `prune_records`; a record whose file exists is kept; a record whose file's folder is missing (an unmounted drive) is kept.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** `write_packages` runs one aggregate per source table over its raw table, with `written_terms` for each column the silver cast casts with a `finance.currency.` label, and writes the answers through `written_from` into each field's `x-brightfield.written`. `carry_over` runs after the fresh package is built (`own_files.rs`, ~374) and today copies the old field's whole `x-brightfield` over the fresh one, so a second write would keep the stale `written`. Change it to merge: from the old field it carries `prefer` (and `nominated` with its label, as now), and it takes `written` from the fresh field, so the values just read win. A lone file's record is stamped with its canonical path when `keep_record` writes it. `prune_records` walks `records/`, reads each record's stamped path, and removes the record's folder when that path is missing and its parent folder exists; a record with no stamped path is kept (records written before the stamp carried a path refresh it on their next open).
- [ ] **Step 4: Run** `cargo test -p brightfield-shell column_record`, `cargo test -p brightfield-shell --test own_files_protocol` and `cargo test -p brightfield-shell --test open_materialise` → PASS.
- [ ] **Step 5: Commit** — "A workbook's cast columns record how they were written, and a gone file's record is pruned".

### Task 7: Vendor FineType's display export

**Waits on** a FineType release whose label definitions carry display blocks exported beside `transforms.json`, and whose percentage cast stores a fraction.

**Files:**
- Modify: `crates/brightfield-protocol/vendor/finetype/` (the new export and the updated `transforms.json`), `packaging/finetype-pin.env`, via `scripts/finetype-pin.sh` as the last FineType staging did
- Create: `crates/brightfield-protocol/src/display.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct FinetypeDisplays { /* label -> Display */ }
  impl FinetypeDisplays {
      pub fn vendored() -> Self;
      pub fn get(&self, label: &str) -> Option<&Display>;
  }
  pub struct Display {
      pub number_format: Option<String>,
      pub date_format: Option<String>,
      pub money: bool,
      pub currency: Option<String>,
      pub fraction: bool,
      pub bytes: bool,
  }
  ```
  Field names follow FineType's export; map them here if FineType names them differently.

- [ ] **Step 1: Write the failing tests.** `vendored_displays_read`: `get("datetime.component.year").number_format == Some("d")`; `get("finance.currency.amount").money`; `get("representation.numeric.percentage").fraction`; `get("representation.file.file_size").bytes`. `every_number_format_parses`: each vendored `number_format` parses with `NumberFormat::parse`, each `date_format` with the date-format parser.
- [ ] **Step 2: Stage the release** with `scripts/finetype-pin.sh`; run `cargo xtask ci --step finetype-pin` → PASS.
- [ ] **Step 3: Implement `display.rs`** on the pattern of `FinetypeTransforms::vendored`.
- [ ] **Step 4: Run** `cargo test -p brightfield-protocol display` and the existing silver tests (the percentage cast changed) → PASS.
- [ ] **Step 5: Commit** — "Stage FineType vX, whose labels say how their values are shown".

### Task 8: The representation rule

**Files:**
- Create: `crates/brightfield-shell/src/representation.rs`
- Test: in-module tests

**Interfaces:**
- Consumes: `ColumnProfile`, `Display` (Task 7), `FieldRecord` (Task 4), and whether the label's cast ran on this column: from the session's cast list (`Dashboard::casts` on the flat route), or, for a Protocol's copy, which holds the silver table, whether `cast_expr` gives a cast for the column's label. The record does not say which columns were cast, and a column DuckDB stored as a number is never cast (Task 6), so a DOUBLE `45.0` labelled percentage holds 45, not the fraction 0.45 the percentage cast stores.
- Produces:
  ```rust
  pub struct Representation {
      pub tick_format: Option<String>,   // written as x/yTickFormat
      pub scale: Option<Scale>,          // divide in the bin
      pub title_unit: Option<String>,    // "£bn", "%", "MiB"
      pub cell_format: Option<String>,   // grid: full value
      pub compact_format: Option<String>,// tooltip
      pub symbol: Option<String>,        // printed by grid and tooltip only
  }
  pub struct Scale { pub divisor: f64, pub unit: &'static str } // "k","m","bn" or "KiB","MiB","GiB"
  pub fn representation_of(column: &ColumnProfile, display: Option<&Display>, record: Option<&FieldRecord>, cast: bool) -> Representation;
  pub fn format_cell(rep: &Representation, value: f64) -> String;
  pub fn format_compact(rep: &Representation, value: f64) -> String;
  ```

- [ ] **Step 1: Write the failing tests**, each with the spec's values:
  - `sterling_billions`: min 2e8, max 3.1e9, money, symbol `£` → `scale == Scale { divisor: 1e9, unit: "bn" }`, `title_unit == "£bn"`, tick format has no `$`, `format_compact(2.412e9) == "£2.4\u{a0}bn"`, `format_cell(2_412_345_000.0) == "£2,412,345,000.00"`.
  - `dollars_keep_their_symbol_on_ticks`: symbol `$`, max 980 → tick format starts with `$`, no scale.
  - `accounting_negatives`: `Negatives::Parentheses` → `format_cell(-12.0) == "(£12.00)"`.
  - `negative_scaled_axis` (Review Focus 5): min −3e8, max 2.1e9 → scale `bn`; tick of −0.3 prints `\u{2212}0.3`; 0 prints `0`.
  - `percentage_precision_from_spread`: fraction, 0.12–0.78 → `.0%`; 0.041–0.046 → `.1%`; `format_cell(0.45) == "45%"`.
  - `year_has_no_grouping`: `datetime.component.year` → tick format `d`.
  - `bytes_scale_binary`: bytes, max 5e9 → `Scale { divisor: 1073741824.0, unit: "GiB" }`.
  - `preference_wins`: `prefer.number_format == ",.0f"` → `tick_format == ",.0f"` and `cell_format == ",.0f"`, whatever FineType's display says.
  - `unlabelled_column_has_no_representation`: no display, no record → all `None`.
  - `money_with_no_recorded_symbol_never_prints_dollars`: money, no recorded symbol (a column whose values mixed `£` and `$`, or a Protocol column before Task 6b) → `symbol == None`, no tick, cell or compact format contains `$`, and `title_unit` names no symbol.
  - `uncast_percentage_is_not_multiplied`: fraction, `cast == false`, a DOUBLE column with min 12.0 and max 78.0 → no `%` specifier (which multiplies by 100), `format_cell(45.0) == "45%"` with the `%` attached, never `"4500%"`. With `cast == true` and 0.45 the same cell prints `45%` through `.0%`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Scale: the largest of 10³, 10⁶, 10⁹ (or 2¹⁰, 2²⁰, 2³⁰) not above the larger of `|min|`, `|max|`, applied when that magnitude is at least 10⁴ (bytes: 2²⁰). Precision for a fraction: the fewest decimals (0–3) at which five evenly spaced ticks across the spread print distinctly.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell representation` → PASS.
- [ ] **Step 5: Commit** — "One rule for how a column's values are shown".

### Task 9: Tiles write their representation; years count in time order

**Files:**
- Modify: `crates/brightfield-shell/src/dashboard.rs` (`tile_for` ~830, `histogram_tile_sized` 1237, `time_bars_tile_sized` 1365, `tile_comment` 1398, the year route)
- Modify: `crates/brightfield-shell/tests/dashboard_baseline.rs`
- Modify: `crates/brightfield-spec/src/layout.rs` (`read_domains_in`, ~943: read a list longer than two ends, and a two-item list on a band, as a categorical domain)
- Modify: `crates/brightfield-render/src/scale.rs` and `crates/brightfield-render/src/scene.rs` (apply a categorical domain as a band's order; `axis_ends_apply`, 846, stays linear-only)
- Create: `crates/brightfield-shell/tests/typed_formats_baseline.rs`, its committed YAML snapshot and light and dark PNGs

**Interfaces:**
- Consumes: `representation_of` (Task 8), `BinOperand::Expression` spelling (Task 1).
- Produces: generated YAML per tile with `xTickFormat`, `xLabel: "<column> (<unit>)"`, a bin over `"<column>" / <divisor>` when scaled, `xGrid: true` / `yGrid: true`, and a tile comment naming the label that chose the format.

- [ ] **Step 0: A band reads its domain from a list.** The year tile's `xDomain` cannot be written in a form the renderer reads today: `read_domains_in` accepts only two numeric ends and refuses any other list, and `axis_ends_apply` is for a linear axis, while the counts-per-date tile draws a band. Add a categorical positional domain: `read_domains_in` reads a list into `DomainReading::Categories { key, values }` (a two-item list stays `Ends` on a continuous axis and becomes categories on a band); the renderer turns each number into the band's category text (`2009` → `"2009"`, the text the band's categories carry) and applies the list as the band's order, keeping a listed category with no rows as an empty slot. Test in `layout.rs` and the render crate: `a_listed_domain_orders_the_band`: `xDomain: [2001, …, 2024]` over a band of years with 2009 absent parses with no warning, and the band draws 2001–2024 in that order with an empty slot at 2009. `a_listed_domain_on_a_linear_axis_is_refused`: a three-item list on a linear axis is still refused with the existing warning. Run `cargo test -p brightfield-spec domain` and `cargo test -p brightfield-render domain` → PASS; commit — "A band takes its order from a listed domain".
- [ ] **Step 1: Write the failing tests** in `typed_formats_baseline.rs` over `typed_formats.csv`: `generated_yaml_matches_snapshot` (the committed YAML; this is the baseline that reddens when a label's format changes); `revenue_tile_is_scaled`: its block contains `xLabel: "revenue (£bn)"` and the expression bin; `year_tile_counts_in_time_order`: `year` gets the counts-per-date kind, `xTickFormat: d`, and an `xDomain` of 2001–2024 including 2009; `long_year_run_leaves_domain_to_data` (Review Focus 4): a run of 400 years writes no domain; `every_tile_writes_its_grid`; `light_and_dark_pngs_match`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** The `datetime.component.year` route goes to the counts-per-date kind whatever the category ceiling says; the year-run ceiling is 120. The count axis writes no format.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell --test typed_formats_baseline --test dashboard_baseline` → PASS; re-record `dashboard_baseline` only if its tiles now write grid and format lines, and check its choice table is unchanged.
- [ ] **Step 5: Commit** — "Generated tiles write each column's format, scale and unit".

### Task 10: The grid formats labelled columns

**Files:**
- Modify: `crates/brightfield-shell/src/data_grid.rs` (`tabulate` 1248, `number_cell`, `GridColumn` 169)
- Test: `crates/brightfield-shell/tests/data_grid.rs`; grid PNGs for `typed_formats.csv`

**Interfaces:**
- Consumes: `representation_of`, `format_cell` (Task 8).
- Produces: `GridColumn::unit: Option<String>` drawn in the header as `<name> · <unit>`.

- [ ] **Step 1: Write the failing tests.** `labelled_cells_use_the_file_convention`: `revenue` shows `(£12.00)` and `£1,234.50`; `share` shows `45%`; `CellText::exact` holds the stored value where it differs. `unlabelled_columns_are_unchanged`: an unlabelled BIGINT `1234567` shows `1234567`. `header_carries_the_unit`: `revenue · £`, `share · %`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement** by passing each column's representation into `tabulate`.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell --test data_grid` → PASS; record the grid PNGs.
- [ ] **Step 5: Commit** — "The grid shows each labelled column the way its file wrote it".

### Task 11: The tooltip formats compactly

**Files:**
- Modify: `crates/brightfield-shell/src/chart_item.rs` (`hover_read`, 842)
- Test: the existing hover tests beside it

- [ ] **Step 1: Write the failing tests.** The line names the column, not the bin's SQL: an expression bin's axis is named for its expression's text, and the tooltip must never print that text (`"revenue" / 1e9:`). `tooltip_names_the_column_not_the_expression`: on the generated `revenue` tile, the hover line starts `revenue:` and contains no `1e9` and no `"`. `tooltip_reads_compact`: resting on the generated `revenue` histogram's bar for £2.0–2.5bn gives a line `revenue: £2.0\u{a0}bn–£2.5\u{a0}bn`; on an unlabelled column the line is unchanged. `tooltip_follows_a_hand_written_axis`: on a chart the analyst wrote over `revenue` with no format and no scale, the line prints the grouped default (`£2,412,345,000`), not the rule's scaled form.
- [ ] **Step 2: Run it to see it fail.**
- [ ] **Step 3: Implement.** Name the line from the axis label's column (or the bin's source column), not from the expression text the low edge is aliased to. Format with the hovered axis's tick format and the scale its bin applies, plus the record's symbol and unit word; on a generated tile that is `format_compact`'s output.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell hover` → PASS.
- [ ] **Step 5: Commit** — "The tooltip reads a value in its column's unit".

### Task 12: The inspector's "auto" shows the chosen format

**Files:**
- Modify: `crates/brightfield-shell/src/shelf.rs` (`AUTO`, 756; the format row, ~1220–1317)
- Test: in-module tests; re-record `shelf_settings_format_*` PNGs if they move

- [ ] **Step 1: Write the failing tests.** `auto_shows_the_rule_format`: for the `revenue` tile, the format row reads `auto` with the rule's specifier beside it. `typed_format_is_not_auto`: after typing `,d`, the row shows `,d` and no `auto`. `saved_chart_keeps_its_format`: a saved chart whose label later changes shows its saved specifier, not `auto`, and the file is not rewritten. `empty_specifier_suggests_the_rule`: a hand-written chart with no format shows `default` with the rule's specifier as a suggestion. `auto_preset_writes_the_rule`: picking the `auto` preset writes the rule's specifier and the row then reads `auto`; picking `default` takes the key out. `unlabelled_column_steps_past_auto`: on an unlabelled column, stepping from `default` through every preset reaches `number` and never shows `auto`. `rule_matching_a_preset_does_not_trap_the_stepper`: with a preference of `~s`, stepping `l` from `default` reaches `currency` and `custom`, and stepping back returns to `default`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement**: three cases. A specifier equal to `representation_of(..).tick_format` (when that is `Some`) reads `auto`; no specifier reads `default` with the rule's specifier as a suggestion; any other specifier is shown as typed. In `FORMAT_PRESETS`, a new `default` preset writes none and comes first; `auto` follows it and its specifier is resolved per row from that row's `representation_of(..).tick_format`, not from the const table (which keeps `None` for it). Where the rule gives `None` (an unlabelled column), the stepper skips `auto`, so stepping from `default` goes straight to `number`. Where the rule's specifier equals a later preset's (`,f`, `~s`, `%`, `$,f`, say from a preference of `~s`), the stepper skips that preset too, since it is the same step as `auto`, so every remaining preset stays reachable.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell shelf` → PASS.
- [ ] **Step 5: Commit** — "The inspector says when an axis format was chosen from the column's type".

### Task 13: Fold the design into the architecture docs

- [ ] **Step 1:** Write the outcome into the repo's architecture docs on this branch: the column record and its `x-brightfield` object, the flat cast, the representation rule and what FineType owns versus Brightfield. Include two notes:
  - A lone file's casts are declared in its spec as `TRY` casts over the file's view, so a rebuild (a tile switch, the written spec opened again) re-runs them on each query over the file. A file that gains a bad value after it was opened reads that value as NULL until the next open decides again and puts the column back as text.
  - A band of whole numbers with no format prints its category text ungrouped (`2024`), while Plot's band prints `toLocaleString` (`2,024`). Generated tiles avoid the difference by writing `d`; a hand-written spec over a numeric band without a format draws differently in the two.
- [ ] **Step 2:** Delete this plan and the spec from the branch.
- [ ] **Step 3: Commit** — "Fold the column-format design into the architecture docs".

## Verification commands and their cost

- Per task: the `cargo test -p <crate> ...` lines above, plus `cargo xtask ci --step fmt` and `--step clippy` before the commit and `--step comment-citations` after. The first build on a cleaned tree is a full rebuild; after that a shell-crate test run is a few minutes.
- Snapshot tasks (2, 9, 10, 12) render PNGs and are the slowest per task.
- The full `cargo xtask ci` runs in CI on the pull request (about 25 minutes there); run it locally only to reproduce a CI failure.
