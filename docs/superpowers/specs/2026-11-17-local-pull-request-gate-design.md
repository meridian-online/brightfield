# A local pull-request gate on the Mac Studio

## Intent

Brightfield is the only desktop app in the estate, so it has the heaviest build, and its pull-request checks spend most of their time waiting for GitHub's macOS runners rather than running. From 2026-10-17 to 2026-11-16 the test workflow ran 148 times; a median run took 63 minutes end to end against a 37-minute critical path, the rest queued for the five macOS runners GitHub's free plan allows across the organisation (job wait p90 29 minutes, max 93). The full `cargo xtask ci` on the Mac Studio took about 38 minutes with no queue.

Brightfield is in early access, and anything that shortens the time from a pushed commit to a merge comes first. The pull-request gate therefore moves to the Mac Studio. Hosted CI keeps running on each merge to `main` and nightly, as the backstop for what only GitHub's runner sees.

A self-hosted GitHub runner was set aside: it would run on any push to a public repo, a fork's included; `test.yml`'s disk-reclaim steps delete Xcode versions and simulator runtimes, which suits a throwaway VM and not a workstation; and its jobs would compete with sessions for cores outside the build-slot model. A Linux VM (colima) was set aside too: macOS gives a Linux guest no Metal GPU, so the pixel tier would draw through a software rasteriser on a window stack the repo has never validated, and the product's validated target is macOS.

## Goals

- `cargo xtask gate` runs the full gate for a pull request on the Mac Studio and posts the verdict on the pull request as a commit status, which Forge shows beside GitHub's checks.
- `test.yml` and `lint.yml` stop running on pull requests. They run on each push to `main`, nightly, and on demand.
- The Mac Studio is the reference machine for pixel baselines: every baseline is redrawn on it, and the hosted runner is the machine allowed a few pixels of rounding jitter.
- A red hosted run on `main` is noticed and becomes a Kata issue; the fix goes forward, with no revert.
- The local gate's wall time drops below 38 minutes by running its two test steps side by side.

## Non-goals

- Other repos keep their pull-request CI. Their Linux runners barely queue.
- No workflow is deleted. Each runs the same steps it runs today; only the triggers change.
- No change to methods' guards. They refuse every agent merge and never read CI.
- Sharding work to shorten hosted runs on `main` (the 45-minute job limit) is separate work that follows this.

## Anti-goals

- A gate that reports green without having run. A status is posted only after the steps ran, and it names the step count and wall time, so an empty run cannot pass for a full one.
- Loosening the pixel tier on the reference machine. The Mac Studio keeps the floor in `kittest.toml` with a budget of zero.
- A gate that tests the branch alone. CI tests the branch merged with `main`, and so does this.

## Rulings Hugh made

- **The pull-request gate runs locally, not on a self-hosted runner** (2026-11-16): CI was slow and unreliable across the board, and the queue for macOS runners was most of it.
- **Which workflows leave pull requests** (2026-11-16): `test` (warm-cache and its shards) and `lint` (fmt, clippy, check-release, doc, comment-citations) become on demand on pull requests. `brew-install-branch` and `signing-rehearsal` stay on pull requests, filtered to the packaging paths. `license` and `public-hygiene` stay (Linux, under a minute). `test` and `lint` keep running on each merge to `main`; `nightly-network` and `release` are unchanged.
- **The Mac Studio is the reference machine for pixel baselines** (2026-11-17): anything that reduces wall clock comes first during early access. Keeping hosted tests on pull requests that touch baselines would have put a quarter of merges (10 of 40 in the last month) back in the queue.
- **A red hosted run on `main` is flagged on Kata and fixed forward, with no revert** (2026-11-17).
- **The gate is a brightfield `cargo xtask` command**, not a script shared through methods (2026-11-17): only brightfield needs it, and xtask already owns the step list the gate runs.

## Approaches considered

- **A `cargo xtask gate` command in brightfield (chosen).** Reuses `cargo xtask ci`'s steps, its `--list`, and the workflow-coverage check that keeps CI's jobs and the local run in step.
- **A generic gate script in methods.** Would have to learn each repo's checkout, build slots and test layout for a need only brightfield has, and would put the red-`main` check where brightfield's tests cannot reach it.
- **A background poller for red runs on `main`** (a launchd job). Notices sooner, but is one more process on the Mac Studio that nobody watches. Not built.

## Design

### `cargo xtask gate <pr>`

1. **Check `main`'s backstop first.** Read the latest completed `test` and `lint` runs on `main` (`gh run list --branch main --workflow <file> --limit 1`) and decide once across both. If either failed, find the one open Kata issue labelled `main-red` in brightfield's project, or open it (titled "main is red", with no single commit in the title) and set it `work.attention needs-human`; comment each red run's workflow, commit and link on it once, skipping a run already noted. When every watched workflow's latest run is green and the issue is open, comment that `main` is green at that commit and close it. The gate then carries on: a red `main` does not stop a pull request's gate, since the fix goes forward.
2. **Take the gate lock and a build slot.** One gate runs at a time on the machine (a lock file under a shared directory such as `~/.cache/meridian/`), and the gate counts as one of the machine's build slots while it holds the lock. If either is taken it waits, saying what it waits for.
3. **Build the tree to test.** Refuse a pull request whose head is not in `meridian-online/brightfield` (a fork's): the gate builds and runs the branch's code on the Mac Studio, and the isolated environment in step 4 is test hygiene, not containment. Fetch the pull request's head and `origin/main`, then make a clean worktree at the head merged with `main`. A merge conflict fails the gate with "merge main into the branch first", posted as a failure status.
4. **Isolate the environment.** Run every step with `HOME` and `TMPDIR` pointing at fresh temporary directories, `CARGO_HOME` and `RUSTUP_HOME` pointing at the real ones, a persistent target directory reserved for the gate, so a run reuses compiled dependencies and a rerun costs test time rather than a rebuild, `ARC_DUCKDB_BIN` pointing at the invoking checkout's pinned DuckDB CLI (checked with `scripts/fetch-duckdb-cli.sh --check`), and `OPEN_ANALYTICS_DIR` pointing at the real `open-analytics` checkout, since a fresh worktree has neither beside it.
5. **Run the steps.** Run the merged tree's own `cargo xtask ci`, so the steps are the pull request's, as CI's are, and not those compiled into the checkout that started the gate. A full `cargo xtask ci` runs `warm-cache` as a local step and then `test-one-thread` and `test-default` side by side, stops at the first failure and ends with the same step summary, which it also writes as JSON for the gate to read.
6. **Post the verdict.** A commit status with context `local-gate` on the pull request's head commit: `success` or `failure`, and a description, within GitHub's 140 characters, naming the steps passed, the failing step if any, the wall time and `main`'s short SHA. Posting uses the real `HOME`'s `gh` credentials, outside the isolated environment. The full log stays on the Mac Studio, and the gate prints its path.
7. **Clean up** the worktree and the temporary directories, and release the lock.

The status attaches to the head commit, as GitHub's checks do, so a later push leaves the old status on the old commit and Forge shows the new head as unchecked until the gate runs again. A move of `main` after the gate ran does not invalidate the status, which matches how GitHub's own pull-request checks behave.

### Pixel baselines

- Redraw every committed baseline on the Mac Studio (169 images today) with the tests' snapshot-update switch, in one commit whose message says the images moved only by rasteriser rounding.
- Spot-check the redraw: every changed image differs from its old one only by small per-channel differences at edges, the same pattern PR 430 measured.
- Move the jitter allowance in `crates/brightfield-shell/tests/support/gpu_jitter.rs` from the Apple M5 Max to the hosted runner's paravirtual adapter, with each test's budget set to the count a hosted run against the redrawn baselines measured for that test, and zero for every test that ran clean there. The M5 Max's budget becomes zero, and the comments and `kittest.toml`'s policy text say which machine is the reference and why.
- `test.yml`'s comment about regenerating baselines from the failing run's uploaded artifact changes: a baseline is drawn on the reference machine, and a hosted failure within the allowance is jitter, not a regression.

Sequence the branch so the hosted budget is measured before the triggers change. While `test.yml` still runs on pull requests, a push of the redraw commit gets a hosted run that shows the paravirtual adapter's differences; the trigger change comes after.

### Workflows

- `test.yml` and `lint.yml`: `on:` becomes `push` to `main`, `workflow_dispatch`, and a nightly `schedule`. No pull-request trigger.
- The workflow-coverage check keeps passing: it requires every step to be invoked by some workflow, which still holds, and it only polices direct gate calls in workflows a pull request triggers. Its wording and tests change wherever they assume `test.yml` runs on pull requests.

### Instructions

- brightfield's `CLAUDE.md`, "The gate": the merge waits on a green `local-gate` status, run by `cargo xtask gate <pr>` once the branch is pushed and before it is called ready, rerun after each fix round. Hosted CI runs on `main` and nightly, and a red run there is flagged and fixed forward. The paragraph that runs the full gate only to reproduce a CI failure, and the October timings, are replaced with the gate's measured wall time.
- A separate pull request in methods changes `practice/loop.md`'s line "CI is the full gate, and a merge waits on it": a repo may name a local gate that stands in for CI on pull requests, and brightfield's is `cargo xtask gate`. The session starts the gate as soon as the branch is pushed, without waiting for roborev's review.

## Constraints

- One cargo build at a time per session, and at most three sessions building on the Mac Studio at once. The gate holds one of those slots for its whole run.
- brightfield is public: the gate's output, its status text and the docs carry no private planning identifiers. The Kata issue the backstop opens is in Kata, which is private.
- The pinned toolchain, `cargo-deny`, `python3`, the `open-analytics` clone beside the checkout and the pinned DuckDB CLI are prerequisites; the gate reports a missing one the way `cargo xtask ci` does.

## Calls made on Hugh's behalf

These settle what the design left open; each can be reversed later without rework.

- **Build slots.** Nothing counts slots today; the three-session rule is a convention. The gate lock is the only counter, and a gate's run is the build of the session that started it: that session does not run another cargo build until its gate finishes. No change to methods' hooks.
- **Cores for the two test steps.** The gate builds every test binary once (as `warm-cache` does in CI), then runs `test-one-thread` (one thread) and `test-default` (the session's 6 test threads) against the built binaries, so neither waits on cargo's build lock and the pair uses about seven threads, one over a slot's cap.
- **When a redraw stops being rounding.** A redrawn image counts as jitter when no pixel moves more than 16 of 255 in any channel and at most 50 pixels move. An image past either limit keeps its old baseline, its test is listed in the pull request, and the issue is flagged for Hugh; the rest of the redraw goes ahead. The hosted runner's budgets follow the same limits: a test that needs more than 50 failed pixels on the hosted runner is flagged rather than budgeted.
- **Which red runs on `main` open an issue.** A run with a failed job counts. A run whose only non-success jobs were cancelled or timed out is reported by the gate but opens no issue, since the timeout is known and tracked separately. While `main` stays red, the gate reuses the one open issue labelled for the backstop and comments the new run on it, instead of opening one per run; the next gate run after `main` goes green says so on that issue.
- **No GitHub ruleset.** `local-gate` is not made a required status. Only Hugh merges, he sees the status in Forge, and a required status would make every pull request, docs-only ones included, wait for a gate run.

## Left to the builder

- The lock file's location and format, and how the gate waits on it.
- Whether `ci` gains a flag to run the two test steps side by side, or `gate` drives the steps itself.
- The exact status description format, within 140 characters.
- The gate's persistent target directory's location.
- If running the two test steps side by side contends for the GPU or memory enough to fail or slow down, fall back to running them in sequence and record the measurement.
