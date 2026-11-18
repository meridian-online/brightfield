# Hosted CI leaves GitHub's Macs, and the Mac Studio gates main each night

## Intent

GitHub's free plan runs at most five macOS jobs at once across the whole meridian-online organisation, and every run queues behind them. On 2026-11-18 main's `test` run for the merge of #448 queued 75 minutes for a macOS runner, and another repo's distribution run on main sat queued behind Brightfield's. Since 2026-11-17 a pull request's checks already run on the Mac Studio (`cargo xtask gate <pr>`, see `docs/pull-request-gate.md`), so the hosted macOS runs on main are only a backstop, and the backstop is what now holds the queue.

Hugh's rulings, 2026-11-18:

- "let's try to remove any macos runners."
- The hosted `test` and `lint` runs on main become a nightly full gate on the Mac Studio, reported the way the backstop reports now: a red main is still flagged on the `main-red` Kata issue and fixed forward, and the pull-request gate's read of main's backstop reads the nightly instead of the hosted runs.
- Check jobs that need no Mac may run on Linux in hosted CI. The rest leave hosted CI.
- The release workflow and its release-time clean-Mac install check keep GitHub's macOS runners, for release tags only.

## Goals

1. No push, pull request or schedule starts a GitHub-hosted macOS job, except a pushed release tag (`release.yml`, and `brew-install.yml` which it calls).
2. The Mac Studio runs the full `cargo xtask ci` on main's head every night, plus what the hosted nightly-only and packaging-path workflows ran that can safely run on a workstation, and posts the verdict on main's head commit.
3. A failed nightly opens or updates the `main-red` issue and flags it for Hugh; a green nightly closes it. This works whether or not any pull-request gate runs that day.
4. The pull-request gate reads the nightly's verdict instead of `test.yml` and `lint.yml` runs.
5. `workflow-coverage` refuses a workflow that would bring a hosted macOS job back on a push, pull request or schedule.

## Non-goals

- Running anything on a self-hosted GitHub runner. That was set aside on 2026-11-16 for a public repo (fork pushes reach it, the workflows' disk-reclaim steps suit only a throwaway VM, and its jobs sit outside the build-slot rule), and nothing here reopens it.
- Validating the Linux window stack. `clippy`, `check-release` and `doc` build the whole dependency graph, and on Linux that needs the Vulkan, wayland/X11, xkbcommon and fontconfig system packages this repo has never validated. They stay on the Mac Studio's nightly and gate, not on Linux.
- Deleting the hosted macOS workflows. As in the 2026-11-16 change, each keeps the steps it runs and loses only its automatic triggers, so anyone can still run one by hand with `gh workflow run <file> --ref <branch>`.
- Changing `release.yml` or `brew-install.yml`.
- Removing the hosted-runner pixel jitter allowance in `gpu_jitter.rs`. A by-hand `test.yml` run still draws on that runner.

## Anti-goals

- A nightly that silently stops running, so main goes red with nobody told. The design has to make a missing nightly visible.
- A nightly that competes with daytime sessions for the machine's three Rust build slots.
- A nightly that touches Hugh's real Homebrew, keychain or login items. That is why the clean-Mac install check and the signing rehearsal do not move to the Mac Studio.
- A nightly whose own code comes from whatever branch the main checkout happens to be on.

## What runs where today, and after

| Workflow, job | Today | After |
| --- | --- | --- |
| `test.yml` warm-cache, five shards, artifact-type-source (disk image), airgap-jail, packaged-artifact-staging | macOS, on push to main, 03:17 UTC nightly, by hand | By hand only. The nightly runs every one of these steps. |
| `test.yml` `test` aggregate | Linux | By hand only, with the rest of the workflow |
| `lint.yml` fmt, clippy, check-release, doc, comment-citations and its self-test | macOS, on push to main, nightly, by hand | `fmt`, `comment-citations` and `comment-citations-selftest` move to a Linux job on pull requests and pushes to main. The macOS job runs by hand only. The nightly runs all of them. |
| `nightly-network` | macOS, 06:37 UTC daily, by hand | By hand only. Its shell moves into a script, and the nightly runs that script. |
| `managed-duckdb` | macOS, on pull requests touching its paths, by hand | By hand only. The nightly runs its two steps, rebuilding DuckDB only when its inputs change. |
| `brew-install-branch` | macOS, on pull requests touching packaging paths, by hand | By hand only. Not run by the nightly (see calls). |
| `signing-rehearsal` | macOS, on pull requests touching signing paths, by hand | By hand only. Not run by the nightly (see calls). |
| `license`, `public-hygiene` | Linux, on pull requests and pushes to main | Unchanged |
| `release`, `brew-install` | macOS and Linux, on a release tag | Unchanged |

## Approaches

1. **A launchd job runs a new `cargo xtask nightly` on the Mac Studio, which posts a `nightly-gate` commit status on main's head and keeps the `main-red` issue in step itself. The pull-request gate reads that status.** (Chosen.) It reuses the gate's lock, worktree and isolated environment, so the nightly is one more gate run rather than a new mechanism. The status is public and durable, readable from any machine with `gh`, and shows in Forge and on GitHub beside the commit.
2. **The nightly writes its verdict to a local file, and the gate reads the file.** Simpler to read, but the verdict exists only on the Mac Studio and nowhere a reader of the repo can see it. Set aside.
3. **A self-hosted runner picks up the hosted nightly's schedule.** Set aside by the 2026-11-16 ruling (see non-goals).

`docs/pull-request-gate.md` lists a launchd poller for red runs under "Not built", because it would be one more unwatched process. Hugh's ruling of 2026-11-18 makes the Mac Studio the backstop, so a scheduled job is now the backstop itself, not a poller beside it. The missing-nightly check below answers the "nobody watches it" objection.

## The design

### The launcher

`scripts/nightly/install.sh install|uninstall|status|run-now`, after finetype's `scripts/launchd/install.sh`. `install` writes a LaunchAgent plist to `~/Library/LaunchAgents/online.meridian.brightfield-nightly.plist`, filling in the checkout's absolute path and the user's home at install time, so no machine path is tracked in this public repo. It fires once a day at 02:30 local time (`StartCalendarInterval`, which launchd also fires on wake if the machine slept through it). Logs go to `~/Library/Logs/brightfield-nightly/`.

The plist cannot run `scripts/nightly/run.sh` from the checkout, since that file is whatever branch the checkout has. `install` copies it out of the repo to `~/.cache/meridian/gate/nightly/run.sh`, and the job runs that copy, which:

1. fetches `origin/main` into the repo the plist names;
2. makes a fresh detached worktree at that commit under `~/.cache/meridian/gate/trees/`, the same place the gate puts its trees;
3. hands over to that tree's own `scripts/nightly/run.sh`, which runs the tree's `cargo xtask nightly --sha <sha>` with the pinned toolchain, so every line of code the nightly runs is main's;
4. removes the worktree, whatever the outcome.

The checkout the plist names is only a git object store. Its branch and working files are never read.

### `cargo xtask nightly`

A new subcommand beside `gate`, in `xtask/src/nightly.rs`, sharing `gate.rs`'s helpers (moved to a common module where both need them).

1. **Takes the gate lock**, as a pull-request gate does, so the nightly never runs beside a gate and counts as one build slot. It waits for the lock until 05:30 local time at the latest, so a full run can still end before the working day. A nightly that never gets it posts no status (see step 6). The plist sets the same caps a Claude Code session has, 6 cargo jobs and 6 test threads, since a LaunchAgent inherits none.
2. **Isolates the environment** exactly as the gate does (step 4 of `docs/pull-request-gate.md`): fresh `HOME` and `TMPDIR`, the real cargo and rustup homes, the gate's persistent target directory, the pinned DuckDB CLI, the real `open-analytics` checkout, compiler and build-flag variables removed. A missing `open-analytics` checkout or DuckDB CLI stops the run before the steps, naming which.
3. **Posts nothing yet.** GitHub keeps only the latest status per context on a commit, so a `pending` posted at the start, or an `error` after an early stop, would overwrite an earlier night's verdict on the same unchanged head.
4. **Runs `cargo xtask ci --nightly --summary-json`** in the tree. `--nightly` is a new flag, used only by the nightly: it adds the nightly-only steps below, and the runner carries on past a failed step instead of stopping, and the summary records every step's outcome, so a clippy slip on main does not hide that night's test results, as the separate hosted `lint` and `test` runs never did. Steps that depend on an earlier one (the two test steps on `warm-cache`) are still skipped when it fails. The output is teed to `~/.cache/meridian/gate/logs/nightly-<sha7>-<YYYYMMDD>.log`.
5. **Runs the nightly-only steps**, each after the full gate whether or not it passed, so one bad night reports everything that is red:
   - `nightly-network`: the network-gated `--ignored` tests and the real FineType bundle checks that `nightly-network.yml` runs today, moved from that workflow's inline shell into `scripts/nightly-network.sh`. The workflow, run by hand, calls the same script.
   - `duckdb-managed` then `managed-duckdb-test`, with `BRIGHTFIELD_DUCKDB_MANAGED_DIR` under `~/.cache/meridian/gate/duckdb-managed`. The DuckDB build is reused while the hash of its inputs (the files `managed-duckdb.yml` keys its cache on) is unchanged, and rebuilt when it changes.
   A red `nightly-network` says the published data moved or a host was down far more often than that a commit broke something, so it does not make main red. It posts its own `nightly-network` status on the commit, and a failure comments on the open Kata issue labelled `nightly-network` (opening one if needed) and flags it `needs-human`; a pass closes it.
   These become `cargo xtask ci` steps marked nightly-only, in the way `duckdb-managed` is marked CI-only today, so `cargo xtask ci --list` names them, `--step` runs them by hand, and a pull request's gate does not run them.
6. **Posts the verdict** on the commit as `nightly-gate`: `success` when every step but `nightly-network` passed, `failure` when one failed, with a description under 140 characters in the gate's form, for example `failed at test-default after 33 steps, 52m 10s`. A run that left no summary is `failure`. A run that never reached the steps (lock timeout, fetch, worktree, missing checkout or CLI) posts no status, so the commit keeps its last real verdict; it reports through the `nightly-missing` issue instead (step 7).
7. **Keeps the `main-red` issue in step**: `failure` opens or finds the issue, flags it `needs-human` and comments the commit, the failing step and the local log's path once per commit; `success` closes it with the commit as evidence. A run that stopped before the steps comments on, or opens, an issue labelled `nightly-missing` and flags it `needs-human`, since a nightly that errors every night is as blind as one that never runs; only the nightly's next `success` or `failure` closes it; a pull-request gate raises it but never closes it. The `main-red` update is the same code the gate's backstop check calls.

A nightly whose commit already has a `nightly-gate` `success` still runs: the network tests and the advisory-reading steps can go red with no new commit.

### The gate's backstop check

`check_backstop` stops reading `test.yml` and `lint.yml`. It reads the newest commit among main's last 50 that carries a `nightly-gate` status of `success` or `failure` (one GraphQL call through `gh api graphql`), and:

- `failure`: main is red. It runs the same `main-red` update the nightly runs, which is idempotent.
- `success`: main is green at that commit, and it closes an open `main-red` issue.
- **The nightly is missing** when that verdict is more than 36 hours old or there is none, so a run of early stops or a run killed mid-way counts as missing: the gate prints that main's nightly has not given a verdict since that date and raises the `nightly-missing` issue. It never closes that issue; only a nightly verdict does.

A red or missing nightly still does not stop a pull request's gate, since the fix goes forward.

### Hosted CI after the change

- `test.yml`, `lint.yml`'s macOS job, `nightly-network.yml`, `managed-duckdb.yml`, `brew-install-branch.yml` and `signing-rehearsal.yml` keep `workflow_dispatch` and lose every other trigger. Each file's header comment says why, and that the nightly or the release covers it.
- `lint.yml` becomes a Linux workflow, on pull requests, pushes to main and by hand, running `fmt`, `comment-citations` and `comment-citations-selftest`, with the pinned toolchain. Its `fmt-clippy` macOS job moves to a new by-hand-only `lint-macos.yml`, since a workflow's triggers cover all its jobs and the coverage rule reads them per workflow.
- `workflow-coverage` gains one rule and changes another:
  - New: a job whose `runs-on` names a macOS image may sit only in a workflow whose triggers are `workflow_dispatch`, `workflow_call`, or a push of tags alone. The rule also follows `uses: ./.github/workflows/<file>` calls: a workflow with any other trigger may not call a workflow that has a macOS job. Its tests prove it refuses a `pull_request`, branch `push` or `schedule` trigger on such a workflow, and such a call.
  - Changed: every `cargo xtask ci` step must still be invoked by some workflow (a by-hand one counts), so a by-hand hosted run of any step stays possible. The rule that a push to main must run what the local gate runs goes, since the nightly now runs it.

### Docs

`docs/pull-request-gate.md` gains a section on the nightly and loses the hosted backstop's description, with Hugh's 2026-11-18 rulings and their reasons in its Rulings list. `CLAUDE.md`'s gate section says the backstop is the Mac Studio's nightly, how to run it now (`scripts/nightly/install.sh run-now`) and where its log goes.

## Calls made on Hugh's behalf

These are for Hugh to check when he reads the plan:

1. **The packaging checks lose their pull-request runs.** `brew-install-branch` installs the app with Homebrew and `signing-rehearsal` puts a keychain on the user's search list; on the Mac Studio both would touch Hugh's real Homebrew and keychain, and his ruling keeps the clean-Mac install check on GitHub's Macs. Both become by-hand only, and a pull request touching packaging or signing paths is checked by them only when a session runs `gh workflow run` for it, or at the next release tag. The pull-request gate prints a line naming the workflow to dispatch when a pull request touches their paths. The alternative, keeping them on their path-filtered pull-request triggers, is a few macOS jobs a month and would miss goal 1.
2. **`managed-duckdb` moves from pull requests touching the DuckDB build to the nightly.** A broken DuckDB build is found the night after the merge, not before it, and fixed forward. The Mac Studio has `cmake` and `ninja`.
3. **The nightly runs at 02:30 local time, waits for the gate lock until 05:30, runs with a session's 6-job, 6-thread caps, and is reported missing after 36 hours without a verdict.** The gate lock does not stop a session that is building Rust overnight from making a fourth build; the three-slot rule stays a convention, as `docs/pull-request-gate.md` already says.
4. **A nightly that cannot run, or errors, flags a `nightly-missing` issue itself, and the gate flags it when 36 hours pass without a verdict.** A nightly that cannot even start (plist unloaded, machine off) is caught only by the next pull-request gate; no second watcher is built, for the reason the gate doc gives against a poller.
5. **A red `nightly-network` does not make main red.** It has its own status and its own Kata issue, as it had its own workflow, which the gate never read.
6. **The nightly keeps going past a failed step**, through a new `--nightly` flag on `cargo xtask ci` that the pull-request gate does not use, so each night reports every red step.
7. **The verdict is a `nightly-gate` commit status**, not a local file and not a workflow run.
8. **Only `fmt` and `comment-citations` move to Linux.** `clippy`, `check-release` and `doc` would need the unvalidated Linux window-stack packages; making them pass on Linux is separate work for whoever validates Linux.
9. **The Linux lint job runs on pull requests as well as main.** It takes a minute, Linux runners barely queue, and it tells a pull request about a formatting slip before the gate does.

## Left to the builder

- How `gate.rs` and `nightly.rs` share code: which helpers move to a common module.
- The GraphQL query's exact shape, and how the gate's existing tests of `read_backstop` and `combine` become tests of reading `nightly-gate` statuses from fixed JSON.
- The plist's exact keys, beyond the schedule, the log paths and that it runs `run.sh` through a login shell so Homebrew and cargo are on `PATH`.
- How the DuckDB build's input hash is recorded between nights.

## Done when

- `workflow-coverage` passes on the branch, and its self-test shows it refusing a macOS job on a pull-request, branch-push or schedule trigger.
- `scripts/nightly/install.sh run-now` on the Mac Studio, run once against main's head, posts a `nightly-gate` status on that commit and keeps the `main-red` issue in step with it.
- A pull-request gate run after it reads that status, not the hosted workflows.
- After merge, a push to main starts no macOS job (read from `gh run list` and the jobs of the runs that push started).
- A release tag still builds, signs and checks the app on GitHub's Macs; nothing in `release.yml` or `brew-install.yml` changed.
