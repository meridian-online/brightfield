# Local Pull-Request Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cargo xtask gate <pr>` runs brightfield's full gate on the Mac Studio against the pull request merged with `main` and posts a `local-gate` commit status; `test` and `lint` leave pull requests; the Mac Studio becomes the reference machine for pixel baselines.

**Architecture:** A new `gate` subcommand in `xtask` reuses `STEPS` from `xtask/src/steps.rs`, with its pure logic (argument parsing, status text, backstop reading, lock) in `xtask/src/gate.rs` under unit tests and its side effects (git, gh, kata, child processes) in thin functions around them. The pixel tier's per-GPU budget moves from the M5 Max to the hosted runner's adapter through one helper every snapshot call uses. Workflow triggers change last, after a hosted run against the redrawn baselines has measured that budget.

**Tech Stack:** Rust (xtask, std only plus the existing `serde_json`), `gh` CLI, `kata` CLI, git worktrees, egui_kittest snapshots, GitHub Actions YAML.

**Spec:** `docs/superpowers/specs/2026-11-17-local-pull-request-gate-design.md`

## Global Constraints

- One cargo build at a time in this session; never run two cargo commands at once except inside the gate's own side-by-side test pair. At most three sessions build on the Mac Studio.
- Toolchain is the pinned one (`rust-toolchain.toml`); never `cargo fmt --all` outside `cargo xtask ci --step fmt`.
- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the change touches. After each commit: `cargo xtask ci --step comment-citations`.
- brightfield is public: no private planning identifiers, Kata refs, private repo names or local absolute paths in code, comments, commit messages, branch names or PR text.
- Status context is exactly `local-gate`; description at most 140 characters.
- The gate runs the merged tree's own `cargo xtask ci`, never the steps compiled into the checkout that started it. It refuses a pull request whose head is outside `meridian-online/brightfield`.
- Redraw stop rule: an image is jitter when no pixel moves more than 16 of 255 in any channel and at most 50 pixels move. Hosted budget per test at most 50 failed pixels.
- A red `main` run opens an issue only when a job failed; cancelled or timed-out-only runs are reported, not filed. One open backstop issue (label `main-red`) is reused while `main` stays red.
- The reference machine (Apple M5 Max) keeps budget zero.
- No GitHub ruleset or branch protection change.

## Review Focus

1. A gate started while another holds the lock, or after a gate crashed and left its lock: the second waits and says so; a lock whose owning process is dead is reclaimed. Test in Task 2.
2. The status post failing (gh not authenticated, network): the gate must exit non-zero and say the status was not posted, never print a green verdict that GitHub never received. Test in Task 3 (the post function's error path).
3. One of the two side-by-side test steps failing: the other still finishes, both appear in the summary, the verdict is failure and the description names the failing step. Test in Task 1.
4. A merge conflict between the branch and `main`: failure status "merge main into the branch first", no steps run. Test in Task 3.
5. A description that would exceed 140 characters (long step name): truncated within the limit, never rejected by GitHub. Test in Task 2.

---

### Task 1: Run the two test steps side by side

**Files:**
- Modify: `xtask/src/runner.rs` (add a paired-run function)
- Modify: `xtask/src/steps.rs` (expose `warm_cache` for local use via a `pub fn build_tests(ctx: &Ctx) -> Result<(), String>`)
- Test: unit tests in `xtask/src/runner.rs`

**Interfaces:**
- Produces: `pub fn run_with_pair<F>(steps: &[Step], pair: (&str, &str), run: F) -> Vec<Row> where F: Fn(&Step) -> Result<(), String> + Sync` — same contract as `run_in_order`, except the two named steps start together when the run reaches the first of them, both run to completion, and the run stops after the pair if either failed. Rows stay in table order.
- Produces: `pub fn build_tests(ctx: &Ctx) -> Result<(), String>` — `cargo test --locked --workspace --no-run`.

- [ ] **Step 1: Find out whether two `cargo test` runs on one target directory block each other once the binaries are built.** Run `cargo test --locked --workspace --no-run`, then start `cargo xtask ci --step test-one-thread` and, a few seconds later, `cargo xtask ci --step test-default` in another shell. Expected: either both run tests at once (no "Blocking waiting for file lock" after the first starts testing), or the second blocks. Record the result as a comment on the gate's Kata issue. If it blocks, the pair uses an APFS clone of the target directory for the one-thread step (`cp -c -R <target> <target>-pair`, then `CARGO_TARGET_DIR` set for that step), and `build_tests` refreshes the clone after building; write that into Step 3's implementation.
- [ ] **Step 2: Write failing tests** in `runner.rs`:
  - `a_pair_starts_together_and_both_finish_when_one_fails`: steps `a, b, c, d` with pair `(b, c)`, `b` fails; assert rows are `a Passed, b Failed, c Passed, d Skipped` and `c` was called.
  - `a_pair_runs_concurrently`: `b` and `c` each wait on a shared `std::sync::Barrier(2)` with a timeout; assert both `Passed` (sequential running would time out).
  - `without_the_pair_named_the_run_is_run_in_order`: pair names absent from `steps` gives the same rows as `run_in_order`.
- [ ] **Step 3: Run `cargo test -p xtask runner`** — expected FAIL (function missing).
- [ ] **Step 4: Implement `run_with_pair`** with `std::thread::scope`; implement `build_tests`.
- [ ] **Step 5: Run `cargo test -p xtask`** — expected PASS. Cost: under a minute.
- [ ] **Step 6: Commit** `xtask: run a pair of steps side by side`.
- [ ] **Step 7 (review round): a full `cargo xtask ci` uses the pair.** `Invocation::All` calls `run_with_pair(STEPS, ("test-one-thread", "test-default"), …)` instead of `run_in_order`, and `warm-cache` becomes a local step (`only_in_ci: None`; its test becomes `warm_cache_runs_locally_before_the_suite`), so the test binaries are built as a named row just before the pair. Remove the `expect(dead_code)` on `run_with_pair`, and remove `build_tests` if nothing calls it. Extend `without_the_pair_named_the_run_is_run_in_order` to also call `run_with_pair(&steps, ("b", "c"), …)` with `b` CI-only and assert the rows equal `run_in_order`'s. Run `cargo test -p xtask` — PASS. Commit `xtask: a full ci run builds the tests, then runs both test steps at once`.

### Task 2: The gate's pure logic

**Files:**
- Create: `xtask/src/gate.rs`
- Modify: `xtask/src/main.rs` (`mod gate;`, parse `gate <pr>`)
- Test: unit tests in `xtask/src/gate.rs` and `main.rs`

**Interfaces:**
- Produces: `Invocation::Gate(u64)` from `["gate", n]`; `USAGE` gains `cargo xtask gate <pr>   run the full gate on a pull request merged with main, and post its verdict as the local-gate status`.
- Produces: `pub struct Verdict { pub passed: usize, pub failed: Option<String> }` and `pub fn read_summary(json: &str) -> Result<Verdict, String>`, reading the summary file `ci --summary-json` writes (Task 3): `{"steps":[{"name":"fmt","outcome":"passed"}, …]}`, outcomes `passed|failed|skipped|ci-only`.
- Produces: `pub fn description(verdict: &Verdict, took: Duration, main_sha: &str) -> String` — e.g. `passed 41 steps in 24m 10s on main 09c2efb` or `failed at test-default after 31 steps, 18m 2s, on main 09c2efb`; always `<= 140` chars (truncate the step name with `…` if needed).
- Produces: `pub enum Backstop { Green { sha: String }, Red { run_id: u64, url: String, sha: String, workflow: String }, CancelledOnly { run_id: u64, url: String } }` and `pub fn read_backstop(workflow: &str, run_json: &str, jobs_json: &str) -> Result<Backstop, String>` reading `gh run list --json databaseId,conclusion,headSha,url` and `gh run view <id> --json jobs` output.
- Produces: `pub enum MainState { Red(Vec<Backstop>), Green { sha: String }, Unsettled }` and `pub fn combine(each: &[Backstop]) -> MainState`: any `Red` → `Red` with the red ones; every one `Green` → `Green` (the latest sha); otherwise `Unsettled` (say nothing on the issue).
- Produces: `pub struct Lock { path: PathBuf }` with `pub fn acquire(dir: &Path, say: impl Fn(&str)) -> Result<Lock, String>` (creates `gate.lock` with the owner pid; waits polling every 10 s while the owner pid is alive, saying who holds it; reclaims when the pid is dead) and `Drop` removing the file.

- [ ] **Step 1: Write failing tests:**
  - `the_command_line_reads_a_gate_run`: `gate 431` → `Gate(431)`; `gate`, `gate x` → usage error.
  - `the_summary_file_is_read_into_a_verdict`: three `passed` and one `ci-only` → `Verdict { passed: 3, failed: None }`; a `failed` row → `failed: Some(name)`.
  - `a_passing_description_names_steps_time_and_main`: assert exact string `passed 3 steps in 1m 5s on main 09c2efb` for `Verdict { passed: 3, failed: None }`, 65 s, sha `09c2efb0…`.
  - `a_failing_description_names_the_step`: assert it starts `failed at test-default`.
  - `a_description_never_exceeds_140_characters`: a 200-char step name → `len() <= 140`.
  - `one_red_workflow_makes_main_red_whatever_the_other_says`: `combine(&[Red{..}, Green{..}])` is `Red`; `combine(&[Green, Green])` is `Green`; `combine(&[Green, CancelledOnly])` is `Unsettled`.
  - `a_failed_job_is_red_and_a_cancelled_one_is_not`: fixture JSON with one `failure` job → `Red`; only `cancelled`/`timed_out` jobs → `CancelledOnly`; `success` → `Green`.
  - `a_second_gate_waits_and_a_dead_owner_is_reclaimed`: write a lock with the current pid, acquire from a thread, assert it is still waiting after 1 s (use an injectable poll interval); write one with pid `999999` (dead), assert acquire succeeds at once.
- [ ] **Step 2: Run `cargo test -p xtask gate`** — expected FAIL.
- [ ] **Step 3: Implement** the functions above in `gate.rs` and the parse arm in `main.rs`. Use `runner::wall` for time formatting.
- [ ] **Step 4: Run `cargo test -p xtask`** — expected PASS.
- [ ] **Step 5: Commit** `xtask: the gate's status text, backstop reading and lock`.

### Task 3: `cargo xtask gate <pr>`, end to end

**Files:**
- Modify: `xtask/src/gate.rs` (orchestration), `xtask/src/main.rs` (dispatch, and `ci --summary-json <path>`), `xtask/src/runner.rs` (`pub fn summary_json(rows: &[Row], steps: &[Step]) -> String`)

**Interfaces:**
- Consumes: a full `cargo xtask ci` that pairs the tests (Task 1); `read_summary`, `description`, `read_backstop`, `combine`, `Lock` (Task 2).
- Produces: `pub fn run(pr: u64) -> ExitCode`.

Order inside `run`, each failure posting a `failure` status where a head SHA is known:
1. Backstop: for `test.yml` and `lint.yml`, `gh run list --repo meridian-online/brightfield --branch main --workflow <file> --status completed --limit 1 --json …`, then `read_backstop` each and `combine`. On `Red`: `kata list --label main-red --json` in brightfield's project (`--project brightfield`); with none open, `kata create "main is red" --body "<what the backstop is>" --label main-red --agent` and `kata meta set <ref> work.attention needs-human`; then for each red run whose URL is not already in the issue's comments (`kata show <ref> --json`), `kata comment` its workflow, short SHA and URL. On `Green` with an open `main-red` issue: comment `main is green at <sha7>` and `kata close <ref> --done --message … --agent`. `Unsettled`: print it. Continue either way. A failed kata call is printed and does not stop the gate.
2. `Lock::acquire` in `~/.cache/meridian/gate/`.
3. `gh pr view <pr> --json headRefOid,headRepository,headRepositoryOwner`; refuse a head from another repository. `git fetch origin main pull/<pr>/head`, `git worktree add --detach <tmp>/tree <head>`, `git -C <tree> merge --no-edit origin/main`; on conflict post failure `merge main into the branch first` and stop.
4. Environment for the child process: fresh `HOME` and `TMPDIR` under the run's temp dir; `CARGO_HOME`/`RUSTUP_HOME` set to the real ones (from the environment, else `~/.cargo`/`~/.rustup`); `CARGO_TARGET_DIR` = `~/.cache/meridian/gate/target`; `ARC_DUCKDB_BIN` = the invoking checkout's `target/duckdb-cli/duckdb` after `scripts/fetch-duckdb-cli.sh --check` passes (else fail naming the script); `OPEN_ANALYTICS_DIR` = the invoking checkout's resolved open-analytics sibling. Remove the `CARGO_*` package variables as `Ctx::command` does.
5. Post `pending` with `running on main <sha7>`. Run the tree's own gate: `cargo xtask ci --summary-json <tmp>/summary.json` with the tree as working directory and that environment, its output teed to `~/.cache/meridian/gate/logs/pr-<pr>-<sha7>.log`. Read the verdict with `read_summary`; a missing or unreadable summary file is a failure (`the gate's run left no summary`), never a pass.
6. Post `success`/`failure` with `description(...)` via `gh api repos/meridian-online/brightfield/statuses/<head> -f state=… -f context=local-gate -f description=…`, run with the real `HOME`. A failed post prints `the verdict was not posted: <error>` and exits non-zero.
7. Print the summary and the log path; remove the worktree and temp dirs; the lock drops.

- [ ] **Step 0: Write a failing test** `the_summary_json_names_every_step_and_its_outcome` in `runner.rs` for `summary_json`, then add `ci --summary-json <path>` (written after the run, whatever its verdict) and its parse test.
- [ ] **Step 1: Write a failing test** `the_merge_conflict_is_reported_before_any_step`: in a temp git repo with two conflicting branches, `merge_with_main(tree, "origin/main")` (the extracted function) returns `Err("merge main into the branch first")`.
- [ ] **Step 1b: Write a failing test** `a_fork_is_refused`: `check_head_repository("someone/brightfield")` is `Err` naming the fork; `"meridian-online/brightfield"` is `Ok`.
- [ ] **Step 2: Write a failing test** `a_status_that_cannot_be_posted_is_an_error`: `post_status` with a `gh` replaced by a script that exits 1 (pass the program path in) returns `Err` containing `not posted`.
- [ ] **Step 3: Run `cargo test -p xtask gate`** — expected FAIL; implement; expected PASS.
- [ ] **Step 4: Commit** `xtask: cargo xtask gate runs the gate on a pull request and posts local-gate`.
- [ ] **Step 5: Verify for real** once this branch has a pull request: `cargo xtask gate <this pr>`. Expected: a `local-gate` status on the head commit (`gh api repos/meridian-online/brightfield/commits/<sha>/status` lists it) and Forge's pull request shows it after `curl -s -X POST http://127.0.0.1:8091/api/v1/pulls/github/meridian-online/brightfield/<pr>/sync`. Record the wall time and whether the pair ran side by side. Cost: about 25 to 40 minutes, one build slot.

### Task 4: The Mac Studio draws the baselines

**Files:**
- Modify: `crates/brightfield-shell/tests/support/gpu_jitter.rs`
- Modify: every snapshot call site in `crates/brightfield-shell/tests/*.rs` (about 22: `snapshot_options`, `image_snapshot_options`, `try_image_snapshot_options`, and `SnapshotOptions::default()` uses)
- Modify: `kittest.toml` (policy text), all committed baseline PNGs under `crates/brightfield-shell/tests/snapshots/`

**Interfaces:**
- Produces: `pub const HOSTED_RUNNER: &str` (the hosted macOS runner's adapter name, read from the hosted run) and `pub fn options(base: SnapshotOptions, hosted_pixels: usize) -> SnapshotOptions` that sets `failed_pixel_count_threshold(budget_on(HOSTED_RUNNER, hosted_pixels))`. Each call site passes its own measured count, zero where the hosted run was clean; a non-zero count carries a comment naming the jitter and the hosted runner, per `kittest.toml`'s policy. `APPLE_M5_MAX` and its call-site budgets are removed.
- Produces: a test `the_adapter_is_named_in_the_log` that prints `adapter_name()` so the hosted log shows it.

- [ ] **Step 1:** Add `options`, route every call site through it with `hosted_pixels = 0` and `HOSTED_RUNNER = "Apple Paravirtual device"` (provisional), remove the M5 Max budgets. Run `cargo test -p brightfield-shell` — expected: only the three tests PR 430 budgeted fail, by the known pixels.
- [ ] **Step 2:** Redraw: `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell -- --test-threads=1`. Then for each changed PNG, `git show HEAD:<png> > old.png` and compare with `cargo run -q -p brightfield-shell --bin bf-imgdiff -- old.png <png>`. Expected: every image within the stop rule. An image past it: restore it with `git checkout HEAD -- <png>`, list it, and flag the gate's Kata issue for Hugh.
- [ ] **Step 3:** Run `cargo test -p brightfield-shell -- --test-threads=1` — expected PASS with budget zero on the M5 Max. Update `kittest.toml`'s policy text and the module comment: the Apple M5 Max is the reference machine; the hosted runner is allowed the per-test counts its runs measured.
- [ ] **Step 4: Commit** `Draw the pixel baselines on the reference Mac`, the message saying the images moved only by rasteriser rounding, within 16 of 255 per channel and 50 pixels.
- [ ] **Step 5:** Push and open the draft pull request (`test.yml` still runs on pull requests). Read the hosted run: the adapter name from `the_adapter_is_named_in_the_log` and every failing snapshot's failed-pixel count from the log or `gh run download -n kittest-snapshot-output`.
- [ ] **Step 6:** Set `HOSTED_RUNNER` to the logged name and each failing test's `hosted_pixels` to its own measured count. If any test needs more than 50, keep its old hosted-drawn baseline instead and flag it on the Kata issue. Commit `Allow the hosted runner its measured rounding jitter`; push; expected: hosted `test` green.

### Task 5: `test` and `lint` leave pull requests

**Files:**
- Modify: `.github/workflows/test.yml`, `.github/workflows/lint.yml` (`on:` and comments)
- Modify: `xtask/src/workflows.rs` (tests or wording that assume `test.yml` runs on pull requests), `xtask/src/main.rs` doc comment

- [ ] **Step 1:** Set both workflows' `on:` to `push: branches: [main]`, `workflow_dispatch:`, and `schedule: - cron: '17 3 * * *'`. Rewrite `test.yml`'s baseline comment (regenerate from artifact) to: baselines are drawn on the reference Mac; a hosted failure within the runner's budget is jitter.
- [ ] **Step 2:** Run `cargo xtask ci --step workflow-coverage` and `cargo test -p xtask` — expected PASS; adjust any wording that says the coverage check guards what a pull request runs.
- [ ] **Step 3: Commit** `Run test and lint on main, nightly and on demand, not on pull requests`.

### Task 6: Instructions and docs

**Files:**
- Modify: `CLAUDE.md` ("The gate")
- Create: `docs/pull-request-gate.md` (the folded design: what the gate does, the reference machine, the backstop, and each of Hugh's rulings with its reason)
- Delete: `docs/superpowers/specs/2026-11-17-local-pull-request-gate-design.md`, this plan

- [ ] **Step 1:** Rewrite "The gate": a merge waits on a green `local-gate` status from `cargo xtask gate <pr>`, run once the branch is pushed and after each fix round, alongside roborev; hosted `test` and `lint` run on `main`, nightly and on demand, and a red run there is flagged on Kata and fixed forward. Replace the October timings with Task 3's measured wall time.
- [ ] **Step 2:** Write `docs/pull-request-gate.md`; delete the spec and plan.
- [ ] **Step 3: Commit** `Say how the local gate works and why`.

### Task 7 (methods, its own pull request): the loop's gate line

**Files:** `practice/loop.md` in a methods worktree.

- [ ] **Step 1:** Change "CI is the full gate, and a merge waits on it" to say a repo's CLAUDE.md may name a local gate that stands in for CI on pull requests; brightfield's is `cargo xtask gate`, started as soon as the branch is pushed without waiting for roborev. Run methods' own checks (`tools/check`).
- [ ] **Step 2:** Commit, push, open the pull request, link it to the Kata issue in Forge.
