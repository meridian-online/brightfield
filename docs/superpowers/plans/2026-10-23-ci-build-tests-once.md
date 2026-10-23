# Build the tests once, run them in shards: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One CI job compiles every test executable, and the five test shards run those executables prebuilt, so a cold or DuckDB-recompiling run keeps every job under 36 minutes.

**Architecture:** A new ci-only xtask step, `build-tests`, runs `cargo test --no-run --message-format=json`, writes a manifest of the test executables, runs the doctests, and packs the executables and workspace binaries into a tar that the workflow uploads. When `TEST_ARCHIVE` names that manifest, the existing `test-one-thread`, `test-default`, `sibling-manifests` and `conformance` steps run the prebuilt executables, with the working directory and environment `cargo test` would give them. Without it they call cargo exactly as today.

**Tech Stack:** Rust 1.95.0 (xtask, `serde_json` only), GitHub Actions on `macos-15`, Swatinem/rust-cache@v2, actions/upload-artifact@v4 and actions/download-artifact@v4.

**Spec:** `docs/superpowers/specs/2026-10-23-ci-build-tests-once-design.md`

## Global Constraints

- No local cargo build or test today, because the Mac's build slot belongs to another build. Every compile, test and timing check is a CI run. The only allowed local checks are reading and `rustfmt +1.95.0 --edition 2021 --check <files>`, which compiles nothing.
- The job limit stays `timeout: 36` for every shard. The build job gets `timeout-minutes: 36` too.
- The suite stays as it is: the same items in each group, the same `--test-threads=1` for the one-thread group, one process per target, and a stop at the first failing target.
- A local `cargo xtask ci` behaves as today. `build-tests` is `only_in_ci`, and with `TEST_ARCHIVE` unset every test step calls cargo.
- This is a public repo. No planning identifiers go in code, comments, commits, the branch or PR text.
- Comments match the density and voice of `test.yml` and `xtask/src/steps.rs`: say why, with measured numbers and run ids.

## Review Focus

1. **A test that finds its fixtures relative to the working directory** (`tile_scale_switch`, `status_rail`, `one_step_protocol`, `saved_protocol_working_directory`): prebuilt, it must still run from its package root. Task 1's `runs_from_the_package_root_with_cargo_variables` pins the command shape, and Task 4's count comparison catches a miss.
2. **An integration test that spawns a workspace binary through `CARGO_BIN_EXE_*`** (`brightfield-shell` 27 times, `brightfield-shot` once): the binary must be in the tar at its `target/debug` path. Task 1's `bins_are_collected_beside_the_test_executables` covers this.
3. **A selected item with no prebuilt executable**, for example a test target added after the archive was built, or a mistyped name: the step must fail and name the item, not pass with nothing run. Task 1 has `an_item_with_no_executable_is_refused`.
4. **`sibling_manifests` run from the workspace build's executable** instead of its own narrow build: the executed-count floor of 2 must still hold. Task 2 keeps `executed_count` on the prebuilt run's output.
5. **A shard that runs with `TEST_ARCHIVE` set but no tar unpacked**: the step must fail and say the archive is missing. Task 2's `archive_mode_refuses_a_missing_manifest` covers this.

---

### Task 1: The archive manifest (`xtask/src/archive.rs`)

**Files:**
- Create: `xtask/src/archive.rs`
- Modify: `xtask/src/main.rs` (add `mod archive;`)

**Interfaces:**
- Produces:
  - `pub struct Executable { pub package: String, pub version: String, pub kind: Kind, pub name: String, pub path: String, pub package_dir: String }`. Both `path` and `package_dir` are relative to the repository root.
  - `pub enum Kind { Lib, Bin, Test }`, with each lib flavour (`lib`, `rlib`, `proc-macro`, `cdylib`) mapped to `Lib`.
  - `pub struct Manifest { pub executables: Vec<Executable>, pub bins: Vec<String> }`. `bins` holds root-relative paths of non-test workspace binaries.
  - `pub fn from_cargo_messages(json_lines: &str, root: &Path) -> Result<Manifest, String>` reads `compiler-artifact` messages. A message with `profile.test == true` and a non-null `executable` becomes an `Executable`. A non-test `bin` artifact under `root` with an `executable` goes into `bins`. Package name and version come from `package_id` (both the `name@version` and the `path+file://…#name@version` / `#version` forms). `package_dir` is the parent of `manifest_path`.
  - `pub fn to_json(&self) -> String` / `pub fn from_json(text: &str) -> Result<Manifest, String>`.
  - `pub fn for_items(&self, items: &[String]) -> Result<Vec<&Executable>, String>` maps `@lib` to every `Lib`, `@bins` to every `Bin`, and a name to the `Test` executables with that name. It skips `@doc` and `@protocols`. Any other item with no match is an `Err` naming it. The result is sorted by `(package, kind, name)`.
  - `pub fn command(&self, exe: &Executable, root: &Path) -> Command` builds `root.join(path)`, with `current_dir(root.join(package_dir))` and `CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, `CARGO_PKG_NAME`, `CARGO_PKG_VERSION`, `CARGO_PKG_VERSION_MAJOR`/`MINOR`/`PATCH`/`PRE` and `CARGO_CRATE_NAME` (the name with `-` replaced by `_`) set.

- [ ] **Step 1: Write the tests** in `archive.rs`'s `#[cfg(test)] mod tests`, from a fixture of four or five real-shaped cargo JSON lines (one lib unit test, one bin unit test, two integration tests, one non-test bin, one `build-script-executed` line to ignore):
  - `test_artifacts_become_executables_with_their_package` asserts the count, `Kind`, `name`, the root-relative `path` and the `package_dir`.
  - `bins_are_collected_beside_the_test_executables` asserts `bins == ["target/debug/brightfield-shell"]`.
  - `package_ids_are_read_in_both_forms` covers `brightfield-shell@0.1.7` and `path+file:///r/crates/brightfield-shell#0.1.7`.
  - `items_select_lib_bins_and_named_targets` checks that `["@lib","front_door","@doc"]` selects every lib executable plus `front_door`, and nothing for `@doc`.
  - `an_item_with_no_executable_is_refused`: `for_items(&["no_such".into()])` returns an `Err` containing `no_such`.
  - `the_manifest_survives_json`: `from_json(&m.to_json()) == m`.
  - `runs_from_the_package_root_with_cargo_variables` asserts `get_current_dir()` and the `CARGO_MANIFEST_DIR` and `CARGO_PKG_NAME` values from `Command::get_envs`.
- [ ] **Step 2: Implement** the interfaces above with `serde_json::Value` (no new dependency). Derive `Debug, Clone, PartialEq`.
- [ ] **Step 3: Format check:** `rustfmt +1.95.0 --edition 2021 --check xtask/src/archive.rs xtask/src/main.rs` exits 0.
- [ ] **Step 4: Commit:** "Read cargo's test-build messages into a manifest of test executables". The tests run in CI with Task 3's push.

### Task 2: The steps use the archive (`xtask/src/steps.rs`)

**Files:**
- Modify: `xtask/src/steps.rs` (the `STEPS` table, `test_one_thread`, `test_default`, `sibling_manifests`, `conformance`, a new `build_tests`)

**Interfaces:**
- Consumes: Task 1's `Manifest`, `for_items` and `command`.
- Produces:
  - A step named `build-tests`, placed before `test-one-thread`, with `only_in_ci: Some("a local run builds as its test steps run, so there is nothing to build ahead of them")`. Its `runs` line reads `cargo test --locked --workspace --no-run, then --doc; packs the test executables`. It does four things:
    1. runs `cargo test --locked --workspace --no-run --message-format=json-render-diagnostics`, with stdout captured and stderr inherited;
    2. builds the `Manifest` and writes `manifest.json` under `target/test-archive/`;
    3. runs `cargo test --locked --workspace --doc` with `ARC_DUCKDB_BIN` from `pinned_duckdb`;
    4. runs `tar -cf <scratch>/test-archive.tar` over the manifest, every executable's `path` and every `bins` path, relative to the root.
    It prints the tar's size in MB.
  - `fn archive(ctx: &Ctx) -> Result<Option<Manifest>, String>`: `None` when `TEST_ARCHIVE` is unset or empty. Otherwise it reads the manifest at that root-relative path, and a missing file is an `Err` saying the build job's archive was not unpacked.
  - In `test_one_thread` and `test_default`, when `archive` is `Some`, run each `for_items` executable in turn through `command`, passing `--test-threads=1` for the one-thread group. Stop at the first non-zero exit and name that target in the `Err`. `@doc` prints "the doctests ran in the build-tests job" and runs nothing. When `archive` is `None`, behave exactly as today.
  - In `sibling_manifests` under the archive, run the `Test` executable named `sibling_manifests` with `--ignored --test-threads=1 --nocapture`, logging to the same file, with the same `executed_count` floor of 2 and the same failure messages. In `conformance` under the archive, run `target/debug/conformance` with `--layers 1,2,3,4 --corpus curated` from the root.

- [ ] **Step 1: Write the tests** in `steps.rs`'s test module:
  - `archive_mode_refuses_a_missing_manifest` sets up a `Ctx` with `root` at an empty temp directory under `target/xtask`, calls `archive_at(&ctx, Some("target/test-archive/manifest.json"))`, and asserts an `Err` containing `not unpacked`. Split out `fn archive_at(ctx: &Ctx, var: Option<&str>)` so the test doesn't touch the process environment.
  - `build_tests_is_ci_only_and_runs_before_the_suite` asserts that the step's `only_in_ci` is `Some` and its index in `STEPS` is below `test-one-thread`'s.
- [ ] **Step 2: Implement** the interfaces above. Add `conformance` to the bins the manifest needs: it is a workspace `bin`, so `cargo test --no-run` builds it as a test-harness unit, and its non-test form is what `conformance` runs. If the JSON shows that `cargo test --no-run` doesn't build the non-test `conformance` binary, add `cargo build --locked --workspace --bins` before the manifest is read, and collect `bins` from both commands' messages.
- [ ] **Step 3: Format check:** `rustfmt +1.95.0 --edition 2021 --check xtask/src/steps.rs` exits 0.
- [ ] **Step 4: Commit:** "Run the prebuilt test executables when the build job's archive is unpacked".

### Task 3: The workflow (`.github/workflows/test.yml`)

**Files:**
- Modify: `.github/workflows/test.yml`

**Interfaces:**
- Consumes: Task 2's step names and `TEST_ARCHIVE`.
- Produces: jobs `build-tests` → `shard` (`needs: build-tests`) → `test` (`needs: [build-tests, shard]`).

- [ ] **Step 1: Add the `build-tests` job** on `macos-15` with `timeout-minutes: 36`. It runs, in order: checkout, the sibling checkout, the toolchain, `Swatinem/rust-cache@v2` with `shared-key: test`, the disk reclaim (moved from `shard`), the DuckDB CLI fetch, then `cargo xtask ci --step build-tests`. Then:
  - a step named `keep the build artifacts past the cache's one-week pruning`, which runs `find target/debug/deps target/debug/build target/debug/.fingerprint -mindepth 1 -maxdepth 1 -exec touch -t "$(date +%Y%m%d%H%M.%S)" {} +`. Its comment carries the mechanism: rust-cache deletes artifacts older than a week on a partial key match, warm runs never rebuild dependencies so their artifacts age past that, the measured 470-crate rebuild on runs 39777977727 and 39843349418, and why one shared timestamp keeps cargo's freshness checks unchanged;
  - `actions/upload-artifact@v4`, with `name: test-archive`, `path: ${{ runner.temp }}/test-archive.tar`, `retention-days: 1` and `if-no-files-found: error`.
- [ ] **Step 2: Change the `shard` job.** Add `needs: build-tests`. Remove rust-cache and the disk reclaim. Add `actions/download-artifact@v4` (`name: test-archive`, `path: ${{ runner.temp }}`) and a step that runs `tar -xf "$RUNNER_TEMP/test-archive.tar"`. Set `TEST_ARCHIVE: target/test-archive/manifest.json` in the job `env`. Keep the toolchain step, because xtask runs through `cargo xtask` and `cargo metadata`.
- [ ] **Step 3: Rewrite the comments that this change makes false.** The ceiling paragraph needs the new per-shard composition. The cache paragraph moves to the build job. The disk-reclaim paragraph moves to the build job. The sibling-manifest step's "compiles `libduckdb-sys` a second time" paragraph must say that the step now runs the workspace build's executable, which removes that second compile. The `test` job's comment must name both `needs`. Delete any sentence that is no longer true; don't leave history in it.
- [ ] **Step 4: Update the `test` job** so it fails unless both `needs.build-tests.result` and `needs.shard.result` are `success`, and echoes both.
- [ ] **Step 5: Commit and push:** "Build the test executables once and run them in the shards". Then `cargo xtask ci --step comment-citations` cannot run locally because it compiles xtask, so it is read off CI's lint run.
- [ ] **Step 6: Verify, warm (verification 1).** Open the pull request as a draft, then run `gh pr checks <pr> --watch`. Expected: every check green, including `lint` (clippy, comment citations) and xtask's own tests in a shard. Record each job's wall time and the tar size from the build log.

### Task 4: Prove the cold, partial and count claims in CI

**Files:**
- Modify temporarily, then revert: `.github/workflows/test.yml` (`prefix-key`) and `Cargo.lock`
- Modify: `.github/workflows/test.yml` (`target-seconds:` lines)
- Create (scratchpad only): a script that sums `test result:` lines per group from a run's logs

- [ ] **Step 1: Compare test counts.** Sum the `N passed; M failed; K ignored` figures from every shard plus the build job's doctests on the green run, and compare them with `main` run 39858758378. Expected: the same passed and ignored totals. Any difference is a target that ran nowhere or twice; fix it before going on.
- [ ] **Step 2: Force a cold run (verification 2).** Commit `prefix-key: v1-rust` on the build job's rust-cache, push, and watch. Expected: the build job's log shows "No cache found", and every job finishes under 36 minutes. Record the build job's time. Then revert the commit and push.
- [ ] **Step 3: Force a partial match (verification 3).** This step only proves anything once a run with the touch step has saved this pull request's cache, which Step 2's revert run does. Commit `cargo update -p <a leaf crate with no dependents in the shell, chosen from Cargo.lock> --precise <its previous patch>`, which is a lock-only change. Push and watch. Expected: the build log shows "full match: false", and the number of `Compiling` lines is far below 470 (record it). Then revert the commit and push.
- [ ] **Step 4: Refresh `target-seconds:`** from the green warm run's per-target `test result:` times, rounded up, using the same rule the comment states (targets of 10 seconds or more, plus the four stand-ins). Update the comment's run ids. Commit: "Weigh the shards by their test time alone".
- [ ] **Step 5: Verify:** `gh pr checks <pr> --watch` is all green, and the slowest shard is below 36 minutes with its time recorded.

### Task 5: Fold, review, ready

- [ ] **Step 1: Fold the outcome** into `test.yml`'s comments, which are this repo's record of how CI works (done in Task 3; check them against the final timings). Delete `docs/superpowers/specs/2026-10-23-ci-build-tests-once-design.md` and this plan. Commit: "Fold the build-once design into the workflow's comments".
- [ ] **Step 2: Run `roborev review --branch --wait`.** Fix or answer each finding at Medium and above, then `roborev close <job>`.
- [ ] **Step 3:** Mark the pull request ready. Its description gives the CI test totals, the job times for the warm, cold and partial runs, and the rulings. Watch `gh pr checks <pr> --watch` until it is green. Sync Forge and link the issue, as `practice/loop.md` says. Report it as ready, with the link.
