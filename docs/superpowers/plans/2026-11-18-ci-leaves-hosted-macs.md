# Hosted CI leaves GitHub's Macs: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** No push, pull request or schedule starts a GitHub-hosted macOS job; the Mac Studio runs the full gate on main each night, posts a `nightly-gate` status, and keeps the `main-red` issue in step; the pull-request gate reads that status.

**Architecture:** A LaunchAgent runs `scripts/nightly/run.sh`, which makes a worktree at `origin/main` and runs that tree's `cargo xtask nightly --sha <sha>`. The new subcommand reuses the gate's lock and isolated environment, runs `cargo xtask ci --nightly` (keep going past failures, include the nightly-only steps), posts commit statuses, and raises or clears Kata flags through a module shared with the gate. Hosted macOS workflows keep only `workflow_dispatch`; `workflow-coverage` enforces that.

**Tech Stack:** Rust (xtask crate, `serde_json` only), bash, launchd, GitHub Actions YAML, `gh`, `kata`.

**Spec:** `docs/superpowers/specs/2026-11-18-ci-leaves-hosted-macs-design.md`

## Global Constraints

- No new crate dependencies in `xtask` (it has `serde_json` only).
- Nothing tracked names a machine path, a Kata ref or a private repo; the plist is rendered at install time.
- Commit status contexts: `local-gate` (unchanged), `nightly-gate`, `nightly-network`. Descriptions at most 140 characters, with no machine paths. A run that stops before the steps posts no status; its fixed phrase, never the underlying error, goes into the `nightly-missing` comment.
- Kata labels: `main-red` (unchanged), `nightly-missing`, `nightly-network`.
- Schedule 02:30 local; lock wait ends 05:30 local; missing after 36 hours without a `success` or `failure`; GraphQL reads main's last 50 commits.
- The nightly's environment: `CARGO_BUILD_JOBS=6`, `RUST_TEST_THREADS=6`, plus the gate's isolation.
- `BRIGHTFIELD_DUCKDB_MANAGED_DIR=~/.cache/meridian/gate/duckdb-managed` for the nightly.
- `release.yml` and `brew-install.yml` are not edited.
- No date crate: local time comes from `date +%H%M` / `date +%Y%m%d` through `Command`; GitHub's `createdAt` (`YYYY-MM-DDTHH:MM:SSZ`) is parsed by hand to unix seconds with a days-from-civil function, tested.
- The nightly posts only `success` or `failure` statuses, never `pending` or `error`: GitHub keeps one status per context per commit, and those would overwrite a real verdict.
- One cargo build at a time per session; no `cargo fmt --all` beyond `cargo xtask ci --step fmt`.

## Review Focus

1. A nightly killed mid-run (reboot) posts nothing, and a `pending` or `error` some other tool left must still not count as a verdict; after 36 hours without one the gate reports the nightly missing. Pinned in Task 3's `a_pending_or_error_status_is_not_a_verdict`.
2. Two commits on main where the newer has only `error` and the older a `failure`: the gate reads the older `failure` as main's verdict, and also reports the newer `error` as not a verdict. Pinned in Task 3's `the_newest_verdict_wins_over_a_newer_error`.
3. `warm-cache` fails under `--nightly`: the two test steps are skipped, not run against a stale target, while later independent steps still run. Pinned in Task 1's `keep_going_skips_only_what_needs_the_failed_step`.
4. The lock is still held at 05:30: the nightly posts no status, raises `nightly-missing` with the fixed phrase, and does not start a build. Pinned in Task 4's `a_lock_wait_past_the_deadline_gives_up`.
5. A workflow with `on: push: tags: [...]` plus a branch push, on macOS: `workflow-coverage` refuses it (only a tag-only push is allowed). Pinned in Task 6's `a_macos_job_on_a_branch_push_beside_tags_is_refused`.

## Costs of the verification commands

| Command | What it builds | Cost on the Mac Studio |
| --- | --- | --- |
| `cargo test -p xtask` | xtask only (serde_json) | about 1 minute cold, seconds warm |
| `cargo xtask ci --step fmt` | nothing | seconds |
| `cargo xtask ci --step clippy` | the workspace's dependency graph | 5 to 20 minutes, one build slot |
| `cargo xtask ci --step workflow-coverage` | xtask only | seconds |
| `bash scripts/nightly/launcher-selftest.sh` | nothing | seconds |
| `scripts/nightly/install.sh run-now` (Task 8) | the full gate plus nightly steps | about 1 hour, plus 30 to 90 minutes the first time DuckDB builds; holds the gate lock and one build slot |

## Task dependencies

- Task 1 (runner) has no dependency.
- Task 2 (nightly-only steps) needs Task 1.
- Task 3 (flags and status reading, gate rewired) has no dependency.
- Task 4 (`cargo xtask nightly`) needs Tasks 1, 2 and 3.
- Task 5 (launcher) needs Task 4's command line.
- Task 6 (workflows and coverage) needs Task 2 (step names) and Task 5 (the launcher's self-test step).
- Task 7 (docs) needs Tasks 1 to 6.
- Task 8 (live run on the Mac Studio) needs everything, and is run once, in a session that holds a build slot.

Tasks 1 and 3 can run in parallel.

---

### Task 1: The runner keeps going and knows nightly-only steps

**Files:**
- Modify: `xtask/src/runner.rs` (`Step`, `Outcome`, `run_in_order`, `run_with_pair`, `summary`, `summary_json`)
- Modify: `xtask/src/main.rs` (`USAGE`, `Invocation`, `parse`, `main`)
- Modify: `xtask/src/steps.rs` (every `Step` literal gains the new field)

**Interfaces:**
- Produces: `Step.only_at_night: Option<&'static str>` (reason shown by `--list` and a non-nightly full run). `Outcome::OnlyAtNight(&'static str)`, rendered in `summary_json` as `"outcome":"nightly-only"`. `pub struct RunMode { pub keep_going: bool, pub nightly: bool }` passed to `run_in_order(steps, mode, run)` and `run_with_pair(steps, pair, mode, run)`. `pub const NEEDS: &[(&str, &str)] = &[("test-one-thread", "warm-cache"), ("test-default", "warm-cache")]`. `Invocation::All { summary: Option<PathBuf>, nightly: bool }`, parsed from `ci [--nightly] [--summary-json <path>]` in either order.
- `gate.rs::read_summary` must accept `"nightly-only"` as a non-counting outcome (as it does `"ci-only"`).

- [ ] **Step 1: Write the failing tests** in `runner.rs`'s test module:
  - `keep_going_runs_past_a_failure_and_records_every_outcome`: steps `a`, `b` (fails), `c`; with `keep_going: true` the rows are `Passed`, `Failed`, `Passed`, and `run` was called three times.
  - `keep_going_skips_only_what_needs_the_failed_step`: steps `warm-cache` (fails), `test-one-thread`, `test-default`, `doc`; rows are `Failed`, `Skipped`, `Skipped`, `Passed`.
  - `a_nightly_only_step_is_listed_not_run_outside_the_nightly`: `nightly: false` gives `OnlyAtNight(reason)` and `run` is not called; `nightly: true` runs it.
  - `the_summary_json_names_a_nightly_only_step`: `"outcome":"nightly-only"` appears.
  - The existing `a_failure_stops_the_run_and_every_later_step_is_skipped_unstarted` still passes with `RunMode { keep_going: false, nightly: false }`.
  - In `main.rs`: `the_command_line_reads_a_nightly_full_run` parses `ci --nightly --summary-json /x` and `ci --summary-json /x --nightly` to `All { summary: Some("/x"), nightly: true }`.
  - In `gate.rs`: `a_summary_with_a_nightly_only_step_reads` — `read_summary` on a summary with one `passed` and one `nightly-only` gives `Verdict { passed: 1, failed: None }`.
- [ ] **Step 2: Run** `cargo test -p xtask` — expected: the new tests fail to compile or fail.
- [ ] **Step 3: Implement** the interfaces above. Under `keep_going` a failure no longer marks later steps skipped; a step is skipped when its `NEEDS` entry names a step that failed or was skipped. `run_with_pair` keeps its concurrency and applies the same rule. Every existing `Step` literal gets `only_at_night: None`. `USAGE` gains the `--nightly` line: "run every step, carrying on past a failure, including the nightly-only steps".
- [ ] **Step 4: Run** `cargo test -p xtask` — expected: all pass. Then `cargo xtask ci --step fmt` and `cargo xtask ci --step clippy` — expected: pass.
- [ ] **Step 5: Commit** "Let a full run carry on past a failure and name nightly-only steps". Then `cargo xtask ci --step comment-citations` — expected: pass.

### Task 2: The nightly-only steps

**Files:**
- Create: `scripts/nightly-network.sh` (the body of `nightly-network.yml`'s inline shell, with `${RUNNER_TEMP}` replaced by `${TMPDIR}` and `::error` lines kept as plain `error:` lines when `GITHUB_ACTIONS` is unset)
- Modify: `.github/workflows/nightly-network.yml` (its steps call `cargo xtask ci --step nightly-network`)
- Modify: `xtask/src/steps.rs` (new step `nightly-network`; `duckdb-managed` and `managed-duckdb-test` move from `only_in_ci` to `only_at_night`, keeping their reasons reworded to "the nightly runs it")
- Create: `xtask/src/duckdb_stamp.rs`

**Interfaces:**
- Consumes: Task 1's `only_at_night`.
- Produces: step names `nightly-network`, `duckdb-managed`, `managed-duckdb-test`, placed after every other step in `STEPS`. `pub fn inputs_hash(root: &Path) -> Result<String, String>` (a stable hash, using `std::hash` over sorted file paths and bytes, of `scripts/build-duckdb.sh`, `scripts/duckdb-managed/**` and `packaging/finetype-pin.env`, the files `managed-duckdb.yml` keys on). `pub fn build_is_current(dir: &Path, hash: &str) -> bool` and `pub fn record_build(dir: &Path, hash: &str) -> Result<(), String>`, using `<dir>/inputs.hash`.

- [ ] **Step 1: Write the failing tests** in `duckdb_stamp.rs`:
  - `the_hash_changes_when_an_input_changes_and_not_otherwise`: in a temp tree, hash, hash again (equal), edit `scripts/duckdb-managed/x`, hash (different), edit an unrelated file (unchanged).
  - `a_recorded_build_is_current_until_the_hash_moves`: `build_is_current` false on an empty dir, true after `record_build(h)`, false for another hash.
  - `starting_a_rebuild_forgets_the_old_stamp`: after `record_build(h)` then `forget_build(dir)`, `build_is_current(dir, h)` is false. (`pub fn forget_build(dir: &Path) -> Result<(), String>`.)
- [ ] **Step 2: Run** `cargo test -p xtask duckdb_stamp` — expected: fail.
- [ ] **Step 3: Implement.** The `duckdb-managed` step skips `build-duckdb.sh` when `build_is_current` for `BRIGHTFIELD_DUCKDB_MANAGED_DIR` (default `target/duckdb-managed`), still runs `build-duckdb-selftest.sh`, removes `inputs.hash` before a rebuild starts (so a half-finished build is never taken for current), and calls `record_build` after a successful build. The `nightly-network` step runs `scripts/nightly-network.sh`.
- [ ] **Step 4: Run** `cargo test -p xtask` and `cargo xtask ci --list` — expected: tests pass; the list ends with the three steps tagged nightly-only. Run `bash -n scripts/nightly-network.sh` — expected: no output. Run `cargo xtask ci --step workflow-coverage` — expected: pass (nightly-network.yml invokes the new step).
- [ ] **Step 5: Commit** "Make the network tests and the managed DuckDB build nightly steps", then `--step comment-citations`.

### Task 3: Flags and the nightly's status, and the gate reads it

**Files:**
- Create: `xtask/src/flags.rs` (the `main-red` logic now in `gate.rs`'s `open_red_issue_with`, `note_red`, `note_green_with`, generalised)
- Create: `xtask/src/nightly_status.rs`
- Modify: `xtask/src/gate.rs` (`check_backstop` reads the nightly; `BACKSTOPS`, `AGGREGATE_JOBS`, `read_backstop`, `combine`, `Backstop`, `MainState` and their tests are removed; `RED_BODY` reworded to name the nightly)
- Modify: `xtask/src/workflows.rs` (remove the test `the_aggregate_jobs_the_backstop_leaves_out_are_still_in_their_workflows`)

**Interfaces:**
- Produces in `flags.rs`: `pub struct Flag { pub label: &'static str, pub title: &'static str, pub body: &'static str, pub attention: &'static str }`; consts `MAIN_RED`, `NIGHTLY_MISSING`, `NIGHTLY_NETWORK`. `pub fn raise_with(kata: &OsStr, flag: &Flag, comment: &str)` — finds or opens the one open issue with the label (creating with `--force-new` as now), sets `work.attention` `needs-human` and `work.attention_msg` to `flag.attention`, and adds `comment` unless an existing comment's body already contains it. `pub fn clear_with(kata: &OsStr, flag: &Flag, sha: &str, note: &str)` — closes the open issue `--done --commit <sha> --comment <note>`; nothing when none is open. `raise` and `clear` wrap them with `kata`. Every failure is printed and passed over, as now.
- Produces in `nightly_status.rs`: `pub const COMMITS_READ: u32 = 50`, `pub const MISSING_AFTER: Duration = 36 h`. `pub enum Seen { Verdict { sha: String, state: VerdictState, description: String, at: String }, None }` with `pub enum VerdictState { Success, Failure }`. `pub fn read_newest_verdict(graphql_json: &str, context: &str) -> Result<Seen, String>` reading `data.repository.ref.target.history.nodes[].{oid, status.contexts[]{context, state, description, createdAt}}`, newest first, skipping `PENDING` and `ERROR`. `pub fn is_missing(seen: &Seen, now_unix: u64) -> bool`. `pub fn unix_seconds(iso: &str) -> Result<u64, String>` for `YYYY-MM-DDTHH:MM:SSZ`. `pub fn query(context: &str) -> String` (the GraphQL text, for `gh api graphql -f query=...`). `pub fn post_status` moves here from `gate.rs`, taking a `context` argument.

- [ ] **Step 1: Write the failing tests.** In `nightly_status.rs`, from fixed JSON:
  - `a_failure_on_the_newest_commit_is_red`.
  - `a_pending_or_error_status_is_not_a_verdict`: a single commit with `PENDING` gives `Seen::None`, likewise `ERROR`.
  - `the_newest_verdict_wins_over_a_newer_error`: commit 1 `ERROR`, commit 2 `FAILURE` → `Verdict { sha: commit 2, state: Failure, .. }`.
  - `another_context_is_ignored`: only `local-gate` statuses → `Seen::None`.
  - `iso_times_read_to_unix_seconds`: `1970-01-01T00:00:00Z` → 0, `2026-11-18T02:30:00Z` → 1795055400, a leap day `2028-02-29T00:00:00Z` → 1835395200; a malformed string is `Err`.
  - `missing_after_thirty_six_hours`: a verdict 35 h old is not missing, 37 h is, `Seen::None` is.
  In `flags.rs`, with a shell-script shim for `kata` in a temp dir that logs its arguments (the pattern `note_green_with`'s tests use):
  - `raise_opens_flags_and_comments_once`: no open issue → `create` with the label, two `meta set`, one `comment`; a second `raise` with the same comment, the shim now listing it, adds no comment.
  - `clear_closes_with_the_commit_as_evidence`, and `clear_with_nothing_open_calls_only_list`.
- [ ] **Step 2: Run** `cargo test -p xtask` — expected: fail.
- [ ] **Step 3: Implement.** `check_backstop` runs `gh api graphql` with `query("nightly-gate")`, then: `Failure` → `raise(MAIN_RED, "the nightly gate failed on main at <sha7>: <description>")`; `Success` → `clear(MAIN_RED, sha, "main is green at <sha7>")`. Independently, `is_missing` → print "main's nightly has not given a verdict since <date or 'ever'>" and `raise(NIGHTLY_MISSING, "no nightly verdict on main since <date or 'ever'>, read <today>")`; the gate never clears `NIGHTLY_MISSING`; only the nightly's verdict does (Task 4). A failed `gh` call is printed and changes nothing. The existing `local-gate` posting calls `nightly_status::post_status(gh, sha, "local-gate", ...)`.
- [ ] **Step 4: Run** `cargo test -p xtask`, `--step fmt`, `--step clippy` — expected: pass.
- [ ] **Step 5: Commit** "Read main's nightly verdict instead of the hosted runs", then `--step comment-citations`.

### Task 4: `cargo xtask nightly`

**Files:**
- Create: `xtask/src/nightly.rs`
- Create: `xtask/src/isolate.rs` (moved from `gate.rs`: `Scratch`, `tree_beside_target`, `Paths`, `removed_from_child`, `is_compiler_variable`, `run_teed`, the `Lock`, and the environment build inside `gate_head`/`run_and_read` that both commands need; `gate.rs` calls them)
- Modify: `xtask/src/main.rs` (`Invocation::Nightly { sha: String, trial: bool }`, from `nightly --sha <40-hex> [--trial]`)

**Interfaces:**
- Consumes: Task 1's `--nightly`; Task 2's step names; Task 3's `flags::{raise, clear, MAIN_RED, NIGHTLY_MISSING, NIGHTLY_NETWORK}` and `nightly_status::post_status`.
- Produces: `pub fn run(sha: &str, trial: bool) -> ExitCode`; with `trial` it posts both statuses but raises and clears no flag, since the commit is not main's, run from the tree it gates (the repo root is that tree; no worktree of its own). `Lock::acquire_until(dir, poll, deadline: SystemTime, say) -> Result<Lock, String>`. `pub fn deadline_after(now_local: (u32, u32)) -> Duration` (hour, minute; read from `date +%H%M`) giving the wait until 05:30 local (zero when now is past 05:30 and before 12:00; the 02:30 run normally gets three hours). `pub enum NightStage { Lock, OpenAnalytics, DuckDb, Start, NoSummary }` with fixed phrases. `pub fn statuses_of(summary: &Verdicts) -> (Option<(&'static str, String)>, Option<(&'static str, String)>)` returning the `nightly-gate` and `nightly-network` (state, description) pair, where `Verdicts` is the summary read keeping every failed step's name (extend `gate::read_summary` to return `failed_all: Vec<String>` beside `failed`).
- Log path: `~/.cache/meridian/gate/logs/nightly-<sha7>-<YYYYMMDD>.log`.

- [ ] **Step 1: Write the failing tests** in `nightly.rs`:
  - `a_lock_wait_past_the_deadline_gives_up`: hold a `Lock` in a temp dir, `acquire_until` with a deadline 100 ms ahead and 10 ms poll → `Err`, within 1 s.
  - `the_wait_ends_at_half_past_five`: `deadline_after((2, 30))` = 3 h; `(5, 0)` = 30 min; `(6, 0)` = 0.
  - `network_failures_post_their_own_status_and_leave_the_gate_green`: summary with every step passed but `nightly-network` failed → `nightly-gate` `success`, `nightly-network` `failure`.
  - `a_failed_gate_step_names_the_first_and_counts_the_rest`: `clippy` and `test-default` failed → `nightly-gate` `failure`, description starts `failed at clippy` and contains `and 1 more`, at most 140 characters.
  - `a_run_without_a_summary_is_a_failure`.
  - In `main.rs`: `the_command_line_reads_a_nightly_run` (`nightly --sha <40 hex>` and `nightly --sha <40 hex> --trial`; a short sha is a usage error).
- [ ] **Step 2: Run** `cargo test -p xtask` — expected: fail.
- [ ] **Step 3: Implement** `run`: acquire the lock until the deadline; check the open-analytics checkout and pinned DuckDB CLI as the gate does; set `BRIGHTFIELD_DUCKDB_MANAGED_DIR`; run `cargo xtask ci --nightly --summary-json <scratch>/summary.json` teed to the log, under the isolated environment; post both statuses from `statuses_of`; then flags: gate `failure` → `raise(MAIN_RED, "the nightly gate failed on main at <sha7>: <description>; log <path>")`, gate `success` → `clear(MAIN_RED, ...)`; network `failure` → `raise(NIGHTLY_NETWORK, ...)`, `success` → `clear(NIGHTLY_NETWORK, ...)`; any verdict → `clear(NIGHTLY_MISSING, ...)`. A stop before the steps posts no status and calls `raise(NIGHTLY_MISSING, "the nightly stopped on <date>: <phrase>")`. Exit non-zero on anything but two successes.
- [ ] **Step 4: Run** `cargo test -p xtask`, `--step fmt`, `--step clippy` — expected: pass. The existing gate tests still pass after the move to `isolate.rs`.
- [ ] **Step 5: Commit** "Add cargo xtask nightly, the gate's run on main's head", then `--step comment-citations`.

### Task 5: The launcher

**Files:**
- Create: `scripts/nightly/install.sh` (`install`, `uninstall`, `status`, `run-now`), after finetype's `scripts/launchd/install.sh`
- Create: `scripts/nightly/online.meridian.brightfield-nightly.plist.in` (placeholders `@REPO@`, `@HOME@`)
- Create: `scripts/nightly/run.sh`
- Create: `scripts/nightly/launcher-selftest.sh`
- Modify: `xtask/src/steps.rs` (step `nightly-launcher-selftest`, a normal step, before the test steps)
- Modify: `.github/workflows/lint.yml` (its macOS job runs `cargo xtask ci --step nightly-launcher-selftest`, so `workflow-coverage` stays green; Task 6 moves that job)

**Interfaces:**
- Consumes: Task 4's `cargo xtask nightly --sha <sha>`.
- Produces: `install.sh install` copies `run.sh` to `@HOME@/.cache/meridian/gate/nightly/run.sh` and the plist runs that copy, never the checkout's file, so the code that runs is the fetched ref's. The plist label `online.meridian.brightfield-nightly`, `StartCalendarInterval` Hour 2 Minute 30, `ProgramArguments` `/bin/bash -lc "@HOME@/.cache/meridian/gate/nightly/run.sh"`, with `BRIGHTFIELD_NIGHTLY_REPO=@REPO@` in `EnvironmentVariables`, `EnvironmentVariables` `CARGO_BUILD_JOBS=6`, `RUST_TEST_THREADS=6`, logs in `@HOME@/Library/Logs/brightfield-nightly/`. `run.sh` has two stages. Run without `--in-tree` (the installed copy, or `run-now`), it reads `BRIGHTFIELD_NIGHTLY_REPO` (required for the installed copy; `run-now` sets it to the checkout) and `BRIGHTFIELD_NIGHTLY_REF` (default `main`), fetches that ref from `origin`, adds a detached worktree at it under `~/.cache/meridian/gate/trees/nightly-<sha7>`, and runs that worktree's own `scripts/nightly/run.sh --in-tree <sha> [--trial]`, which runs `cargo +$(toolchain from rust-toolchain.toml) xtask nightly --sha <sha>` inside it, adding `--trial` when the ref is not `main`, and removes the worktree in a `trap`. `install.sh run-now` runs `run.sh` in the foreground. `install.sh install` with `BRIGHTFIELD_NIGHTLY_DRY_RUN=1` renders to `$HOME/Library/LaunchAgents` without calling `launchctl`.

- [ ] **Step 1: Write the failing self-test** `launcher-selftest.sh`: with a temp `HOME` and `BRIGHTFIELD_NIGHTLY_DRY_RUN=1`, `install.sh install` writes the plist; `plutil -lint` accepts it; it contains no `@` placeholder; its `ProgramArguments` names the installed copy under the temp `HOME`, not this checkout's `run.sh`, and the copy exists; `git grep -nE '/Users/|/Volumes/|/home/'` over tracked files under `scripts/nightly/`, and a grep for the real `$HOME` captured before it is overridden, find nothing. Then a `run.sh` run against a temp clone with a stub `cargo` on `PATH` that records its arguments: the stub saw `xtask nightly --sha <origin/main's sha>` from inside a worktree at that sha, and the `run.sh` that ran it was the worktree's (make the temp clone's committed `run.sh` differ from the checkout's by a marker it echoes, and check the marker), and the worktree is gone afterwards, including when the stub exits 1; with `BRIGHTFIELD_NIGHTLY_REF` set to a temp branch the stub saw that branch's sha and `--trial`.
- [ ] **Step 2: Run** `bash scripts/nightly/launcher-selftest.sh` — expected: fails (scripts missing).
- [ ] **Step 3: Write** the three scripts and the plist template.
- [ ] **Step 4: Run** `bash scripts/nightly/launcher-selftest.sh`, `cargo xtask ci --step nightly-launcher-selftest` and `cargo xtask ci --step workflow-coverage` — expected: pass. `shellcheck scripts/nightly/*.sh` if installed — expected: clean.
- [ ] **Step 5: Commit** "Schedule the nightly gate with a LaunchAgent the checkout renders", then `--step comment-citations`.

### Task 6: Hosted workflows leave the Macs, and coverage holds them there

**Files:**
- Modify: `.github/workflows/test.yml`, `nightly-network.yml`, `managed-duckdb.yml`, `brew-install-branch.yml`, `signing-rehearsal.yml` (`on:` becomes `workflow_dispatch:` alone; header comment says the nightly or the release covers it, and how to run it by hand)
- Create: `.github/workflows/lint-macos.yml` (`workflow_dispatch` only; the `fmt-clippy` macOS job moved from `lint.yml`, running `clippy`, `check-release`, `doc` and `nightly-launcher-selftest`)
- Modify: `.github/workflows/lint.yml` (Linux only: a `fmt-citations` job on `ubuntu-latest` with `dtolnay/rust-toolchain@1.95.0`, running `--step fmt`, the unshallow fetch and `--step comment-citations`, and `--step comment-citations-selftest`; `on:` keeps `push: branches: [main]`, adds `pull_request`, keeps `workflow_dispatch`, drops `schedule`; `fmt-clippy` moves out to `lint-macos.yml`)
- Modify: `xtask/src/workflows.rs` (new rule; remove the push-to-main reasoning; update the module doc)
- Modify: `xtask/src/gate.rs` (packaging hint)

**Interfaces:**
- Produces: `pub fn macos_jobs(text: &str) -> Vec<(usize, String)>` (line and `runs-on` value naming `macos-*`). `pub fn automatic_triggers(text: &str) -> Vec<String>` (each trigger under `on:` other than `workflow_dispatch` and `workflow_call`, with `push` reported as `push(tags-only)` when its block has `tags:` and no `branches:`/`branches-ignore:`). `coverage` reports `<file>:<line>: a macOS job in a workflow that <trigger> starts; macOS runners are for release tags only` for every macOS job whose workflow has an automatic trigger other than `push(tags-only)`. A workflow with an automatic trigger other than `push(tags-only)` whose job has `uses: ./.github/workflows/<file>` naming a workflow with a macOS job is reported the same way. The direct-call rule now applies to workflows with any automatic trigger, without the push-to-main special case. `gate.rs` prints `this pull request touches <path>; run gh workflow run <file> --ref <branch> for its macOS check` for each of `brew-install-branch.yml` and `signing-rehearsal.yml` whose path list (kept as a const beside the hint, copied from the workflows' `paths:` blocks before Task 6 removes them, and held by a test that each listed path or glob matches at least one tracked file) matches a changed file from `git diff --name-only <main>...<head>` in the gate's tree.

- [ ] **Step 1: Write the failing tests** in `workflows.rs`:
  - `a_macos_job_on_a_pull_request_is_refused`, `..._on_a_schedule_is_refused`, `a_macos_job_on_a_branch_push_beside_tags_is_refused`.
  - `a_macos_job_on_a_tag_push_or_dispatch_or_call_is_allowed`.
  - `a_pull_request_workflow_calling_a_macos_workflow_is_refused`: a Linux workflow on `pull_request` with `uses: ./.github/workflows/brew-install.yml` is reported; `release.yml` (tag push) calling it is not.
  - `the_repository_s_workflows_start_no_macos_job_automatically`: `read_all(repo_root)` through `coverage` reports no macOS finding (this is goal 1, read from the real files).
  - In `gate.rs`: `the_packaging_hint_paths_name_tracked_files` (each const path or glob matches a file in `git ls-files`) and `a_change_to_package_sh_names_both_workflows`.
- [ ] **Step 2: Run** `cargo test -p xtask` — expected: the rule tests fail, and the real-files test fails on today's workflows.
- [ ] **Step 3: Implement** the rule, the hint, and the workflow edits.
- [ ] **Step 4: Run** `cargo test -p xtask`, `cargo xtask ci --step workflow-coverage`, `--step fmt`, `--step clippy` — expected: pass. Run `grep -lE 'runs-on: *macos' .github/workflows/*.yml` and check each file's `on:` by eye: only dispatch, `workflow_call` or tag pushes.
- [ ] **Step 5: Commit** "Run hosted macOS jobs only by hand or on a release tag", then `--step comment-citations`.

### Task 7: Docs

**Files:**
- Modify: `docs/pull-request-gate.md` (a "The nightly" section: what runs, when, where its log goes, the three statuses and three labels, how to install and run it now; the backstop paragraph in "What `cargo xtask gate <pr>` does" reads the nightly; the 2026-11-18 rulings and the calls in the spec, with reasons, in "Rulings"; the "Not built" poller entry says why a scheduled nightly is now the backstop)
- Modify: `CLAUDE.md` (the paragraph beginning "Hosted CI's `test` and `lint` no longer run on pull requests")

- [ ] **Step 1:** Write the changes. Plain words, no planning identifiers, no machine paths.
- [ ] **Step 2: Run** `cargo xtask ci --step measured-figures` and `--step scale-claims` — expected: pass (they read docs for unsourced figures).
- [ ] **Step 3: Commit** "Say in the gate's docs that main's backstop is the Mac Studio's nightly", then `--step comment-citations`.

### Task 8: First live run on the Mac Studio

Run once in a session that holds a build slot, after Tasks 1 to 7 are pushed. Until the branch merges, main's `xtask` has no `nightly` subcommand, so the trial runs the branch's code with `--trial`, which posts statuses on the branch head and leaves the Kata flags alone.

- [ ] **Step 1:** Run `BRIGHTFIELD_NIGHTLY_REF=<branch> scripts/nightly/install.sh run-now`. Expected: within about 1 hour (plus 30 to 90 minutes for the first DuckDB build), `gh api repos/meridian-online/brightfield/commits/<branch head>/statuses` lists `nightly-gate` and `nightly-network` with `success` or `failure`, and no Kata issue changed.
- [ ] **Step 2:** Run `cargo xtask gate <this branch's pr>`. Expected: its backstop line reads main's nightly (none yet, so it reports the nightly missing and raises `nightly-missing`) and makes no `gh run list` call (read in its log). The first real nightly after merge clears it.
- [ ] **Step 3:** After merge (Hugh merges), run `scripts/nightly/install.sh install` and `status`. Expected: `launchctl print gui/$(id -u)/online.meridian.brightfield-nightly` shows the job loaded. The next morning: a `nightly-gate` status on main's head dated after 02:30.
- [ ] **Step 4:** After the first push to main following the merge: `gh run list --commit <sha> --json workflowName,databaseId` and each run's jobs show no `macos` runner label.
