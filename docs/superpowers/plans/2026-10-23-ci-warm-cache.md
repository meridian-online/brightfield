# Warm the build cache before the test shards: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `warm-cache` job compiles the dependency graph once per lockfile ahead of the test shards, so that every shard starts from an exact cache match.

**Architecture:** There is one new ci-only xtask step, `warm-cache`, which runs the three compiles the shards would otherwise run. A new job in `test.yml` runs that step only when rust-cache's `lookup-only` check misses the exact key. The shards `need` that job, and the `test` summary job needs both.

**Tech Stack:** Rust 1.95.0 (xtask), GitHub Actions on `macos-15`, Swatinem/rust-cache@v2.

**Spec:** `docs/superpowers/specs/2026-10-23-ci-warm-cache-design.md`

## Global Constraints

- No local cargo build today. The xtask tests run on the Beelink (`cargo +1.95.0 test --locked -p xtask` in a clone there), and everything else runs in CI.
- Every shard keeps `timeout: 36`.
- This is a public repo, so no planning identifiers go into anything that lands.
- Comments match `test.yml`'s voice: they say why, with measured numbers and run ids, and nothing false is left standing.

## Review Focus

1. **The warm job and the shards computing different cache keys** (a different `CARGO*`/`RUST*` environment or a different `shared-key`). The shards would then miss and compile cold, behind a green warm job. Task 3 checks for "full match: true" in every shard log.
2. **The lookup step saving a cache** from a half-built `target/` on a miss. `save-if: false` on that step prevents it, and Task 2 checks the warm job's log for a single save.
3. **A warm job that fails** has to block the shards and turn `test` red, not let the shards run cold. The shards get `needs: warm-cache`, and Task 2 updates the summary check.
4. **The workflow-coverage test** (`this_repositorys_workflows_and_steps_agree`) requires every step to be invoked by a workflow, and it flags direct cargo calls. The compiles therefore go through `cargo xtask ci --step warm-cache`.
5. **A run on `main`** behaves the same as a pull request. Its warm job saves to the `main` scope that pull requests restore from.

---

### Task 1: The `warm-cache` xtask step

**Files:** Modify `xtask/src/steps.rs`.

- [ ] **Step 1: Write the failing test** `warm_cache_is_ci_only_and_runs_before_the_suite` in `steps.rs`'s test module. It asserts that the `warm-cache` step's `only_in_ci` is `Some` and that its index in `STEPS` is below `test-one-thread`'s.
- [ ] **Step 2: Run it on the Beelink.** Expected: FAIL, because no step is named `warm-cache`.
- [ ] **Step 3: Add the step**, placed before `test-one-thread`. Its `runs` line reads `cargo test --locked --workspace --no-run; the sibling-manifest and conformance builds`. Its `only_in_ci` reason reads "a local run's test steps build what they run". Its `fn warm_cache(ctx: &Ctx) -> Result<(), String>` calls `ctx.run_cargo` three times, in order:
  - `["test","--locked","--workspace","--no-run"]`
  - `["test","--locked","-p","brightfield-protocol","--test","sibling_manifests","--no-run"]`
  - `["build","--locked","--bin","conformance"]`

  A doc comment says why there are three: each later step resolves its own feature set.
- [ ] **Step 4: Run all xtask tests on the Beelink.** Expected: the new test passes, and only `this_repositorys_workflows_and_steps_agree` fails, because no workflow invokes `warm-cache` yet.
- [ ] **Step 5:** Run `rustfmt +1.95.0 --edition 2021 --check xtask/src/steps.rs` locally, then commit.

### Task 2: The workflow

**Files:** Modify `.github/workflows/test.yml`.

- [ ] **Step 1: Add the `warm-cache` job** before `shard`, on `macos-15` with `timeout-minutes: 36`. Its steps, in order:
  - checkout;
  - `dtolnay/rust-toolchain@1.95.0`;
  - rust-cache with `id: lookup`, `shared-key: test`, `lookup-only: true` and `save-if: false`;
  - rust-cache with `shared-key: test`, under `if: steps.lookup.outputs.cache-hit != 'true'`;
  - the disk reclaim, under the same `if` (a copy of the shard's step);
  - `cargo xtask ci --step warm-cache`, under the same `if`.

  The job's comment carries the spec's "Why", with the 470-crate rebuild measured on runs 39777977727 and 39843349418. It also explains the first-finisher-wins save and why the sibling-manifest variant is built here.
- [ ] **Step 2: Add `needs: warm-cache` to `shard`.** Correct the shard comments this makes false: the ceiling paragraph's lockfile measure, the cache paragraph, and the sibling-manifest step's paragraph about the second `libduckdb-sys` compile on a cache-key miss. The reclaim paragraph can stay, because the shards still compile the workspace.
- [ ] **Step 3: The `test` summary job** gets `needs: [warm-cache, shard]` and fails unless `needs.warm-cache.result` and `needs.shard.result` are both `success`. Its comment names both.
- [ ] **Step 4:** Run the xtask tests on the Beelink. Expected: all pass, `this_repositorys_workflows_and_steps_agree` included. Then commit and push, and open a draft pull request.
- [ ] **Step 5: Verification 1 (warm).** Run `gh pr checks <pr> --watch`. Expected: all checks green. The warm job's log shows a cache hit and skipped steps, and lasts about a minute. Record each shard's time.

### Task 3: Prove the cold claim, then rebalance

- [ ] **Step 1: Verification 2 (cold).** Commit `prefix-key: v1-rust` on all three rust-cache steps in `test.yml`, push, and watch. Expected:
  - the warm job shows "No cache found", builds, and saves once;
  - each shard log shows "full match: true" and fewer than 30 `Compiling` lines;
  - every shard finishes under 36 minutes.

  Record the warm job's time. Then revert the commit and push. If the warm job goes over 36 minutes, record its time and rule on its ceiling as the spec says.
- [ ] **Step 2: Verification 3 (totals).** Sum the `test result:` lines across the shards, with a scratchpad script, and compare them with `main` run 39858758378. Expected: equal passed and ignored totals.
- [ ] **Step 3: Refresh `target-seconds:`** from the green warm run, by the rule the comment states. Update its run ids, and set `@protocols` from that run's two steps on the last default shard. Commit, push, and confirm green with `gh pr checks <pr> --watch`.

### Task 4: Fold, review, ready

- [ ] **Step 1:** Check `test.yml`'s comments against the final timings. Delete this plan and the spec, then commit.
- [ ] **Step 2:** Run `roborev review --branch --wait`, fix or answer each finding at Medium and above, and close the review.
- [ ] **Step 3:** Mark the pull request ready. Its description gives the CI test totals, the warm and cold timings and the rulings. Watch it go green with `gh pr checks <pr> --watch`. Sync Forge and link the issue, then report it ready with the link.
