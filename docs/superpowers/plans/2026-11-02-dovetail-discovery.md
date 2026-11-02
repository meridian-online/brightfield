# Dovetail discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run dovetail as a child process of the Brightfield binary to propose the own-files model, and make Brightfield read, keep and build what it proposes.

**Architecture:** dovetail is a git dependency linked into `brightfield-shell`; `main` hands the process to `dovetail::cli_main` under `BRIGHTFIELD_RUN_AS_DOVETAIL`, as it does for arc. `discovery::Dovetail` copies the Protocol database under a process-wide lock, runs `dovetail model` on the copy, and reads the proposal file. `brightfield-ossie` types dovetail's new Meridian keys and keeps the rest; silver builds combined datasets from their query.

**Tech Stack:** Rust 1.95.0, DuckDB 2.0 (`duckdb = "=1.20000.0"` bundled), serde/serde_json/serde_yaml, dovetail at commit `c5c1a4f229d9186feeaa7dd13b7e2d841e9b28b7`.

**Spec:** `docs/superpowers/specs/2026-11-02-dovetail-discovery-design.md`

## Global Constraints

- Work only in `/Volumes/Dev/meridian-online/brightfield-wt-dovetail`, branch `dovetail-discovery`. One cargo build at a time; never run two cargo commands in parallel.
- Before each commit: `cargo xtask ci --step fmt` and `cargo xtask ci --step clippy`, plus the tests the task names. Never `cargo fmt --all`. After each commit: `cargo xtask ci --step comment-citations`.
- Public repo: no planning identifiers (issue refs, card ids) in code, comments, commits.
- Comments in this repo are full sentences explaining why; follow the surrounding style. A test name a comment cites must exist (the comment-citations step checks).
- dovetail dependency: `git = "https://github.com/meridian-online/dovetail"`, `rev = "c5c1a4f229d9186feeaa7dd13b7e2d841e9b28b7"`, `default-features = false`.
- The child never writes `model.ossie.yaml`. Proposal files: `.discovery/sample.ossie.yaml`, `.discovery/full.ossie.yaml` (`discovery::proposal_file`).
- A stand-in model must write byte-identical YAML before and after these changes (existing snapshot tests stay green).

## Review Focus

1. A save while a full pass runs: the save's arc run must succeed (lock + copy). Test in Task 6.
2. A Rediscover while a full pass's child is still running: the old child is killed, its files removed, and its answer never merged; the new pass's files are its own. Test in Task 5.
3. A model with a combination: the parts get no silver table, the combined one does, and its joins measure. Test in Task 3.
4. A hand edit typed against stale text after a full pass changed a dovetail join: the join keeps the pass's value. Test in Task 1.
5. dovetail failing (bad database, missing packages): the pane says why, the held model stays. Test in Task 5 (`a_failing_child_answers_its_stderr`).

---

### Task 1: Hand-edit marking compares with the editor's base

**Files:**
- Modify: `crates/brightfield-shell/src/model_file.rs` (`mark_changed`, its two calls in `read_hand_edit`)
- Test: `crates/brightfield-shell/tests/model_file_edit.rs`

**Interfaces:** Produces `fn mark_changed(current: &Model, base: &Model, edited: &mut Model)` (private).

- [ ] **Step 1: Write the failing tests** in `model_file_edit.rs`, using the file's existing helpers for a retail model and its text:
  - `an_edit_against_stale_text_keeps_what_a_pass_changed`: `base` = a model with dovetail join J graded `possible`; `current` = the same with J graded `likely` (as a full pass would leave it); the text is `base`'s text with an unrelated dataset's description changed. `read_hand_edit(&current, &base, &sources, &text)` gives a model where J's grade is `Likely` and `J.meta.decided_by == DecidedBy::Dovetail`; the edited dataset is `DecidedBy::Analyst`.
  - `an_edit_of_the_join_itself_is_the_analysts`: same `base`/`current`, but the text changes J's `cardinality` to `1:1`; J comes back with `cardinality == "1:1"` and `DecidedBy::Analyst`.
  - `a_field_the_text_left_as_it_was_takes_the_current_field`: a field's finetype differs between `base` and `current`; text unchanged there; result holds `current`'s finetype, `DecidedBy::Dovetail`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test model_file_edit -- --test-threads=1`. Expected: the first and third FAIL.
- [ ] **Step 3: Implement.** For each dataset (bare, as `same_dataset` compares), field, and relationship of `edited`: when `base` holds one of the same name and it compares equal to `edited`'s, replace `edited`'s with `current`'s when `current` holds it (for a dataset, the bare part only; its fields go through the field rule), and drop it when `current` does not; otherwise mark it `DecidedBy::Analyst` (today's rule). Keep `keep_measurements` as is. Update the doc comment.
- [ ] **Step 4: Run** the same command. Expected: PASS, and the whole `model_file_edit` suite stays green.
- [ ] **Step 5: Commit** `Mark a hand edit the analyst's only where it differs from the text it started from`.

### Task 2: The model crate reads and keeps dovetail's keys

**Files:**
- Modify: `crates/brightfield-ossie/src/model.rs`, `src/lib.rs` (exports), `src/validate.rs` (schema path), `src/wire.rs` only if struct literals need the new fields
- Move: `crates/brightfield-ossie/vendor/ossie/osi-schema.json` → `vendor/ossie/ossie-schema.json`; add `vendor/ossie/LICENSE` copied from `/Volumes/Dev/meridian-online/dovetail/vendor/ossie/LICENSE`
- Create: `crates/brightfield-ossie/fixtures/dovetail/retail.sample.ossie.yaml`, `retail.full.ossie.yaml` copied from dovetail's `tests/ossie-fixtures/` at the pinned commit
- Test: `crates/brightfield-ossie/tests/dovetail_output.rs`
- Fix every `DatasetMeta { .. }` / `ModelMeta { .. }` struct literal across the workspace (`rg -n "DatasetMeta \{|ModelMeta \{" crates`) with `..Default`-style additions or explicit `None`/empty.

**Interfaces:** Produces, in `brightfield_ossie`:
- `pub struct RoleScores { pub entity: f64, pub event: f64, pub reference: f64, pub bridge: f64, pub snapshot: f64, pub unclear: f64 }` (serde, field order as listed)
- `pub enum CombinationKind { Partition, Snapshot, Duplicate }` (serde lowercase)
- `pub struct Combination { pub kind: CombinationKind, pub parts: Vec<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub token_column: Option<String> }`
- `DatasetMeta` gains `role_scores: Option<RoleScores>`, `fold_column: Option<String>`, `combined_into: Option<String>`, `combination: Option<Combination>`, and `#[serde(flatten)] pub other: serde_json::Map<String, serde_json::Value>` (keys it does not type).
- `ModelMeta` gains `pass: Option<String>`, `typing: Option<serde_json::Value>`, `sample_rows: Option<u64>`, `other` as above.
- `FieldMeta` and `RelationshipMeta` gain `other` as above.
- All new optional keys `skip_serializing_if` empty, so a model without them writes as before. Place the typed keys in the order dovetail writes them (dataset: `role, role_evidence, role_scores, folded_into, fold_column, combined_into, combination, excluded, decided_by, source_file, source_part, rows`).

- [ ] **Step 1: Write the failing tests** in `tests/dovetail_output.rs`:
  - `dovetails_goldens_read_and_validate`: for both goldens, `validate(&text).is_empty()` and `read(&text)` is `Ok`.
  - `dovetails_new_keys_are_typed`: in the sample golden, dataset `members` has `combination == Some(Combination { kind: Snapshot, parts: ["members_2024","members_2025"], token_column: Some("year") })`; `members_2024` has `combined_into == Some("members")`; some dataset has `fold_column == Some("category")`; every dovetail dataset has `role_scores.is_some()`; model `meta.pass == Some("sample")`, `meta.sample_rows == Some(50000)`.
  - `every_meridian_key_survives_a_round_trip`: for both goldens, for each element, the Meridian `data` JSON parsed as `serde_json::Value` before and after `write(&read(text))` is equal (compare per element by name; values, not bytes).
  - `an_unknown_meridian_key_survives`: a golden with `"zz_new":1` inserted into one dataset's data; after read/write the key is still there.
- [ ] **Step 2: Run** `cargo test -p brightfield-ossie --test dovetail_output`. Expected: FAIL (types missing).
- [ ] **Step 3: Implement** the types above; rename the schema file and update `include_str!`.
- [ ] **Step 4: Run** `cargo test -p brightfield-ossie` and `cargo test -p brightfield-shell --test model_file_edit -- --test-threads=1`. Expected: PASS; existing tests green (stand-in bytes unchanged).
- [ ] **Step 5: Commit** `Read and keep the keys dovetail writes on the model`.

### Task 3: Silver builds a combined dataset from its query

**Files:**
- Modify: `crates/brightfield-protocol/src/silver.rs` (`has_silver`, `source`), `crates/brightfield-shell/src/own_files.rs` (`write_silver`, `drop_unmade_silver`)
- Test: `crates/brightfield-protocol/tests/silver_combined.rs`; `crates/brightfield-shell/tests/model_reshape.rs` (or the file holding `left_out_table_is_not_left_materialised`)

**Interfaces:** Consumes Task 2's `DatasetMeta::combination`, `combined_into`. Produces `pub fn silver_datasets(model: &Model, sources: &[&str]) -> Vec<String>` in `own_files` (private is fine): the source table names, then each dataset with a `combination`, in model order, without repeats.

- [ ] **Step 1: Write the failing tests:**
  - `a_combined_dataset_reads_its_query_with_lineage` (silver crate): model with parts `a`, `b` (`combined_into: Some("ab")`) and `ab` with `combination` partition and `source = SELECT "id", 'x' AS "region" FROM "raw_a" UNION ALL BY NAME SELECT "id", 'y' AS "region" FROM "raw_b"`. `silver_sql(&m, "a", ..)` and `"b"` are empty; `silver_sql(&m, "ab", ..)` run in an in-memory DuckDB holding `raw_a`/`raw_b` (each with the three lineage columns, typed VARCHAR, VARCHAR, BIGINT, and `id`) makes table `ab` with row count = rows(a)+rows(b) and columns starting `__source_file, __source_part, __source_row`.
  - `failed_sql` for `ab` runs too.
  - In the shell suite, `a_combination_gets_one_silver_table_and_its_parts_none`: `write_silver` + `run_loads` over a Protocol of two CSVs with a model combining them leaves `ab` in the database and no `a`/`b` silver tables.
- [ ] **Step 2: Run** `cargo test -p brightfield-protocol --test silver_combined`. Expected: FAIL.
- [ ] **Step 3: Implement.** `has_silver` also requires `combined_into.is_none()`. `source(d)` reads `FROM (<lineage row> UNION ALL BY NAME SELECT * FROM (<d.source>))` when `d.meta.combination.is_some()`, the lineage row being exactly `SELECT NULL::VARCHAR AS "__source_file", NULL::VARCHAR AS "__source_part", NULL::BIGINT AS "__source_row" WHERE false`; else as today. `write_silver` iterates `silver_datasets`; `drop_unmade_silver` iterates the source tables as today (a part has no silver, so it is dropped).
- [ ] **Step 4: Run** the step 2 command, then `cargo test -p brightfield-shell --test model_reshape -- --test-threads=1`. Expected: PASS.
- [ ] **Step 5: Commit** `Build a combined dataset's silver table from its query`.

### Task 4: Link dovetail and hand the process to it

**Files:**
- Modify: root `Cargo.toml` (`[workspace.dependencies]`), `crates/brightfield-shell/Cargo.toml`, `Cargo.lock`, `crates/brightfield-shell/src/discovery.rs` (const + hook), `crates/brightfield-shell/src/main.rs` (call the hook beside `serve_as_runner_if_asked`), `deny.toml` only if `cargo xtask ci --step deny` refuses a new licence or duplicate.
- Test: `crates/brightfield-shell/tests/dovetail_child.rs`

**Interfaces:** Produces `pub const DOVETAIL_ENV: &str = "BRIGHTFIELD_RUN_AS_DOVETAIL";` and `pub fn serve_as_dovetail_if_asked()` in `brightfield_shell::discovery`. The hook runs `dovetail::cli_main()` and calls `std::process::exit(0)` when the code equals `ExitCode::SUCCESS`, else `exit(1)`.

- [ ] **Step 1: Write the failing test** `the_binary_serves_as_dovetail_under_its_variable`: run `env!("CARGO_BIN_EXE_brightfield-shell")` with `--help` and `DOVETAIL_ENV=1`; status success and stdout contains `dovetail — the modelling layer of meridian`.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test dovetail_child`. Expected: FAIL (const missing).
- [ ] **Step 3: Implement.** Add the dependency with a comment in the style of the `arc` entry (why a child, why the rev, why no default features). Call the hook first in `main`, before `serve_as_runner_if_asked`. Run `cargo xtask ci --step deny`; resolve what it says.
- [ ] **Step 4: Run** the step 2 command. Expected: PASS.
- [ ] **Step 5: Commit** `Link dovetail and let the binary serve as it`.

### Task 5: `Dovetail` runs the pass as a child

**Files:**
- Modify: `crates/brightfield-shell/src/discovery.rs` (`Dovetail`), `crates/brightfield-shell/src/own_files.rs` (lock), `crates/brightfield-shell/tests/discovery.rs` (unignore, construct with a runner)
- Test: `crates/brightfield-shell/tests/discovery.rs`, `tests/dovetail_child.rs`

**Interfaces:**
- Consumes Task 4's `DOVETAIL_ENV`; `run::Runner` (`program()`); `arc::spec::protocol_db_path`.
- Produces `pub fn own_files::database_lock() -> std::sync::MutexGuard<'static, ()>` (poison-tolerant), taken by `run_loads` and `drop_unmade_silver` for their whole duration.
- Produces `pub struct Dovetail { program: PathBuf }`, `impl Dovetail { pub fn new(runner: &Runner) -> Self }`, `impl Discovery for Dovetail` overriding `discover_until`; `discover` calls it with `Cancel::default()`.

- [ ] **Step 1: Write the failing tests:**
  - Unignore `dovetail_child_writes_valid_ossie`, constructing `Dovetail::new(&runner())`.
  - `a_cancelled_pass_kills_its_child_and_leaves_no_files`: start `discover_until` on a thread over the retail Protocol with a `Cancel`; request it after the child has started (poll until a `.discovery/run-*.duckdb` exists); the call answers `Err`, within 5 s; afterwards `.discovery/` holds no `run-*` file and no `full.ossie.yaml`. `Cancel::request` is private today: make it `pub` (doc: what a pass's owner calls to stop it).
  - `a_failing_child_answers_its_stderr`: a Protocol whose `packages/` is removed before the pass; `discover` answers `Err` containing `dovetail model:`.
  - `a_failed_sample_pass_leaves_no_proposal_to_verify`: after the previous failure, `proposal_file(dir, Pass::Sample)` does not exist.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test discovery -- --test-threads=1`. Expected: FAIL.
- [ ] **Step 3: Implement** per the spec's "How it gets built": remove `proposal_file(pass)` (and, for the sample pass, the full proposal); take `database_lock()`, copy the database and its `.wal` (when present) to `.discovery/run-<n>.duckdb` (`n` from a process-wide `AtomicU64`), release; spawn `program` with `DOVETAIL_ENV=1`, `NO_COLOR=1`, stdin null, stdout null, stderr piped and read on a thread, args `model --db <copy> --packages <dir>/packages --pass <sample|full> --name <dir name> --out <dir>/.discovery/run-<n>.<pass>.ossie.yaml`, plus `--base <dir>/model.ossie.yaml` when it is a file and, for the full pass, `--verify <sample proposal>` when it is a file; `try_wait` every 50 ms, `kill` + `wait` on cancel; on exit 0 and not cancelled rename the out file to `proposal_file(pass)` and `read` it; always remove the copy (and its `.wal`), the out file and `<out>.writing`. Errors: cancelled → `"the discovery pass was superseded"`; nonzero → the child's stderr, trimmed. Rewrite the `Dovetail` doc comment to say what it does now, and keep the note on warnings as the spec rules it (the silver measurement overwrites dovetail's warning after each run).
- [ ] **Step 4: Run** the step 2 command. Expected: PASS.
- [ ] **Step 5: Commit** `Run dovetail as the discovery child, on a copy of the database`.

### Task 6: The open and the pane run dovetail

**Files:**
- Modify: `crates/brightfield-shell/src/data_file.rs` (`open_any`), `src/window.rs` (`open_model_pane`'s full pass), `src/model_pane.rs` (`perform_reshape`'s Rediscover; the pane holds `runner`), `README.md` (Discovery paragraph)
- Test: `crates/brightfield-shell/tests/discovery.rs`

**Interfaces:** Consumes Task 5's `Dovetail::new(&Runner)`.

- [ ] **Step 1: Write the failing tests:**
  - `opening_related_tables_proposes_dovetails_joins`: `data_file::open_any(&retail_copy("joins"), &runner())` answers `Opened::Model(p)`; the model file it wrote holds at least one relationship with `decided_by == Dovetail`, and its `meta.generator` starts with `dovetail `.
  - `a_save_during_a_full_pass_runs_the_protocol`: open the retail pane, `start_full_pass(Dovetail::new(&runner()))`, immediately dispatch a reshape that saves (exclude a table); it answers `Dispatched::Done`; then wait for `poll_discovery` to land and assert the excluded table is still excluded.
  - Existing tests that assert the stand-in's open (`unclear` roles, no joins) are updated to the dovetail answer or pinned to `StandIn` explicitly, each by what it is testing.
- [ ] **Step 2: Run** `cargo test -p brightfield-shell --test discovery -- --test-threads=1`. Expected: FAIL.
- [ ] **Step 3: Implement.** `open_any` runs `Dovetail::new(runner)` when `one_part.is_none()`, `StandIn` otherwise. The window's full pass and the pane's Rediscover run `Dovetail::new(&self.runner)`. Update the README Discovery paragraph to say dovetail proposes the model now, in its two passes, reading a copy of the database.
- [ ] **Step 4: Run** `cargo test -p brightfield-shell -- --test-threads=1` filtered to the touched suites: `discovery`, `model_rerun`, `model_file_edit`, `model_reshape`, `own_files*`, `scripted_open`. Expected: PASS.
- [ ] **Step 5: Commit** `Open own files with dovetail's proposal`.

### Task 7 (lead, not a subagent): Fold and ready

- Put the outcome and rulings in the architecture docs (the README's Discovery section and the `discovery.rs` module doc), delete `docs/superpowers/`, push, open the PR, roborev branch review, CI, Forge link.
