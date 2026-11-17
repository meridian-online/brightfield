# The pull-request gate

A pull request's checks run on the Mac Studio, not on GitHub's runners. `cargo xtask gate <pr>` runs the full gate there and posts the verdict on the pull request as the `local-gate` commit status. Hosted CI's `test` and `lint` workflows run on each merge to `main`, nightly and on demand, as the backstop for what only GitHub's runner sees.

## Why

Brightfield is the only desktop app in the estate, so it has the heaviest build, and its pull-request checks spent most of their time waiting for GitHub's macOS runners rather than running. From 2026-10-17 to 2026-11-16 the `test` workflow ran 148 times. A median run took 63 minutes end to end against a 37-minute critical path; the rest was queueing for the five macOS runners GitHub's free plan allows across the organisation (job wait p90 29 minutes, max 93). 49 runs were cancelled by a newer push, and 8 of 9 reruns went green. The same steps on the Mac Studio took about 38 minutes with no queue, run one after another.

Brightfield is in early access, and anything that shortens the time from a pushed commit to a merge comes first.

## Rulings

Hugh made these, with the reasons given:

- **The pull-request gate runs locally, not on a self-hosted runner** (2026-11-16). CI was slow and unreliable across the board, and the queue for macOS runners was most of it. A self-hosted runner was set aside because it would run on any push to a public repo, a fork's included; `test.yml`'s disk-reclaim steps delete Xcode versions and simulator runtimes, which suits a throwaway VM and not a workstation; and its jobs would compete with sessions for cores outside the machine's build-slot rule. A Linux VM was set aside too: macOS gives a Linux guest no Metal GPU, so the pixel tier would draw through a software rasteriser on a window stack this repo has never validated.
- **Which workflows leave pull requests** (2026-11-16). `test` (warm-cache and its shards) and `lint` (fmt, clippy, check-release, doc, comment-citations) leave pull requests and run on each merge to `main`, nightly and on demand. `brew-install-branch` and `signing-rehearsal` stay on pull requests, filtered to the packaging paths. `license` and `public-hygiene` stay: they run on Linux in under a minute. `nightly-network` and `release` are unchanged. No workflow was deleted; each runs the same steps it ran before, and only the triggers changed.
- **The Mac Studio is the reference machine for pixel baselines** (2026-11-17). Keeping hosted tests on the pull requests that touch baselines would have put a quarter of merges (10 of 40 in the month measured) back in the queue.
- **A red hosted run on `main` is flagged and fixed forward, with no revert** (2026-11-17).
- **The gate is a brightfield `cargo xtask` command**, not a script shared across repos (2026-11-17). Only brightfield needs it, and xtask already owns the step list the gate runs.

Other repos keep their pull-request CI: their Linux runners barely queue.

## What `cargo xtask gate <pr>` does

1. **Takes the gate lock.** One gate runs on the machine at a time: an OS advisory lock on `~/.cache/meridian/gate/gate.lock`, so a crashed gate never blocks the next. A waiting gate says who holds the lock and polls every 30 seconds. While a session's gate runs, that is the session's one cargo build.
2. **Reads `main`'s backstop.** It reads the latest completed `test.yml` and `lint.yml` runs on `main`. If either failed, it opens the one Kata issue labelled `main-red` (or finds the open one), flags it for Hugh, and comments each red run on it once. When both workflows are green on the same commit, it says so on that issue and closes it. A run whose only non-success jobs were cancelled or timed out is reported but opens no issue, and the workflow's aggregate job, which fails whenever a job under it was cancelled, is left out of the reading. A red `main` does not stop the pull request's gate, since the fix goes forward. Each call it makes to GitHub or Kata times out after 120 seconds, so a hung call cannot hold the lock.
3. **Builds the tree to test.** It refuses a pull request whose head is not in `meridian-online/brightfield`: the gate runs the branch's code on the Mac Studio, and the isolated environment below is test hygiene, not containment. It fetches the head and `origin/main` and makes a clean worktree at the head merged with `main`, as GitHub's pull-request checks test it. The worktree goes under `~/.cache/meridian/gate/trees/`, beside the gate's target directory, as a checkout's tree sits beside its own `target` in CI: a test that spells a scratch file relative to its working directory then climbs a few levels, not to `/`. A merge conflict fails the gate with "merge main into the branch first".
4. **Isolates the environment.** Every step runs with `HOME` and `TMPDIR` pointing at fresh temporary directories, the real `CARGO_HOME` and `RUSTUP_HOME`, a persistent target directory under `~/.cache/meridian/gate/` so a run reuses compiled dependencies (the workspace's own crates rebuild, since each run's worktree is new), the DuckDB CLI the merged tree pins (the invoking checkout's copy when the tree's `fetch-duckdb-cli.sh --check` accepts it, else one the tree's script fetched into `~/.cache/meridian/gate/duckdb-cli`, so a branch that moves the pin still gates), and the real `open-analytics` checkout. The machine's cargo job cap stays in the environment, since it changes no build output; compiler and build-flag variables are removed, so the build is CI's.
5. **Runs the steps.** It posts a `pending` status, then runs the merged tree's own `cargo xtask ci`, so the steps are the pull request's and not those compiled into the checkout that started the gate. The output is teed to `~/.cache/meridian/gate/logs/pr-<pr>-<sha>.log`, and the gate prints its path.
6. **Posts the verdict.** A `local-gate` status, `success` or `failure`, on the head commit, with a description under GitHub's 140 characters naming the steps passed, the failing step if any, the wall time and `main`'s short SHA, for example `failed at test-default after 33 steps, 37m 01s, on main 09c2efb`. A run that leaves no step summary is a failure, never a pass, so an empty run cannot pass for a full one. Every failure after the head is known is posted.
7. **Cleans up** the worktree and temporary directories and releases the lock.

The status attaches to the head commit, as GitHub's checks do. A later push leaves the old status on the old commit, and Forge shows the new head as unchecked until the gate runs again. A move of `main` after the gate ran does not invalidate the status, which matches how GitHub's own checks behave. `local-gate` is not a required status in a GitHub ruleset: only Hugh merges, he sees the status in Forge, and a required status would make every pull request, docs-only ones included, wait for a gate run.

## The two test steps run side by side

A full `cargo xtask ci` runs `warm-cache` as a local step, which builds every test binary once, and then runs `test-one-thread` and `test-default` at the same time against the built binaries, so neither waits on cargo's build lock. Inside a Claude Code session, whose settings set `RUST_TEST_THREADS` to 6, the pair uses about seven threads, one over a session's cap; from a plain terminal `test-default` uses every core. `test-default`'s output goes to `target/xtask/test-default.log` while the pair runs, and its failure excerpt is printed in the summary.

Running them together surfaced two tests that shared files across processes: five shell test targets pointed brightfield's config directory at one `CARGO_TARGET_TMPDIR/config`, which every test target of the crate shares, and one target removed a data file there that another target, in the other group, could be opening. Each target now uses a config folder named after it. A test that writes outside a folder of its own will race under the gate.

The first passing gate run, in November 2026 with two other sessions building Rust on the machine, took 1 hour 4 minutes: 2 minutes 21 seconds for `warm-cache`, then 16 minutes 17 seconds for `test-one-thread` and 57 minutes 18 seconds for `test-default`, overlapping.

## Pixel baselines

The reference machine is the Mac Studio's Apple M5 Max. Every committed baseline is drawn on it, and on it every snapshot is held to `kittest.toml`'s floor with no pixel past the threshold. When the reference moved there, 22 of 191 images changed, all by rasteriser rounding: at most 14 of 255 in any channel and 23 pixels in any image.

The hosted macOS runner's paravirtual GPU rounds a few pixels of some baselines differently. `crates/brightfield-shell/tests/support/gpu_jitter.rs` allows each test the failed-pixel count a hosted run measured for it, on that runner's adapter alone; every other GPU keeps zero. Each binary that takes a budget writes the adapter's name to stderr, so a hosted log says which GPU a run drew on. A test that calls `image_snapshot` directly takes the floor everywhere until a hosted run measures jitter on it.

A redrawn image counts as rounding when no pixel moves more than 16 of 255 in any channel and at most 50 pixels move. An image past either limit keeps its old baseline and is raised with Hugh, and so is a test that would need more than 50 failed pixels on the hosted runner. A baseline is never drawn from a hosted run's output.

## The workflows

`test.yml` and `lint.yml` run on a push to `main`, at 03:17 UTC nightly, and by `workflow_dispatch`, which is how to run them on a branch by hand (`gh workflow run test.yml --ref <branch>`). `test.yml`'s concurrency group is per commit and cancels nothing, so no run on `main` ends without a verdict.

`workflow-coverage` reads the workflow files: every step `cargo xtask ci` carries must be invoked by a workflow, no workflow may invoke a step the command lacks, and a workflow that a pull request or a push to `main` triggers may not call a gate script or cargo directly. Pushes to `main` count because the backstop has to run what the local gate runs.

## Not built

- A background poller for red runs on `main` (a launchd job). It would notice sooner, but it is one more process on the Mac Studio that nobody watches. The gate's backstop check runs often enough while work is in flight.
- A count of build slots. The three-session rule is a convention, and the gate lock is the only counter.
- Shortening the hosted runs on `main` past the 45-minute job limit is separate work.
