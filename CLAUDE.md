## Product Positioning

Brightfield targets analytics **authors** — the people who build dashboards and visualisations — much like Canva/Illustrator targets graphic designers. The primary user is an analyst who declares what they want to see in a Mosaic spec and gets an interactive, GPU-accelerated rendering.

Analytics **consumers** (people who view and interact with published dashboards) are a future audience. Good architectural choices now — portable specs via Mosaic, clean separation between authoring and rendering — ensure consumer-facing delivery can be enabled later without rearchitecting.

**Platform targets:** macOS, Linux, and Windows — the egui/eframe + wgpu window stack supports all three (Metal on macOS, Vulkan on Linux/Windows); macOS is the validated target today.

**Reference projects for data expression:** RillData (real-time analytics dashboards, DuckDB-native), MotherDuck (cloud DuckDB, query visualisation UX), apple/embedding-atlas (GPU-accelerated interactive data visualisation at scale).

## Planning

Feature cards, specs, and decisions for Brightfield are tracked centrally, outside this repository — this repo holds the code, not the planning substrate.

**This repo is public, and the planning substrate is not.** Identifiers from it — decision-record refs, task ids, milestone ids, acceptance-criterion shorthand, document ids, card ids, spec AC ids — resolve nowhere for a reader here and leak the shape of private work. Don't put them in code comments, doc comments, prose, commit messages, or PR text. If a pointer carried real meaning, write the actual rationale in plain English instead.

## The gate

`cargo xtask ci` is the one list of checks. It runs the steps in order, test-one-thread and test-default side by side, stops at the first that fails, and ends with a summary naming each step, its wall time, and the steps the failure skipped. `cargo xtask ci --list` names the steps, and `cargo xtask ci --step <name>` re-runs one of them. The workflow files call the same steps by name, and the `workflow-coverage` step keeps the two lists one list.

A pull request's gate is `cargo xtask gate <pr>`, on the Mac Studio. It merges the pull request's head with `main` in a clean worktree, runs that tree's own `cargo xtask ci` in an isolated environment, and posts the verdict as the `local-gate` commit status on the head, which Forge shows beside GitHub's checks. A merge waits on a green `local-gate` and a closed roborev branch review. Start the gate as soon as the branch is pushed, without waiting for roborev, and run it again after each fix round, since a new head has no status until the gate runs on it. One gate runs on the machine at a time, and it is its session's one cargo build while it runs. A full run took 1 hour 4 minutes in November 2026, with two other sessions building Rust on the machine; `docs/pull-request-gate.md` says how it works and why.

Hosted CI's `test` and `lint` no longer run on pull requests. They run on each merge to `main`, nightly and on demand (`gh workflow run test.yml --ref <branch>`), as the backstop for what only GitHub's runner sees. Each gate reads their latest runs on `main` first, and a red one is flagged on the `main-red` Kata issue and fixed forward, with no revert. `license`, `public-hygiene`, and the packaging workflows filtered to their paths still run on pull requests.

Before each commit, run `cargo xtask ci --step fmt` and `--step clippy`, and the tests the change touches. After each commit, run `--step comment-citations`. It reads the branch's commits rather than the staged work and checks the claims in changed comments at `HEAD`, so a bad comment is caught before the branch is pushed.

Pixel baselines are drawn on the Mac Studio's Apple M5 Max, which holds every snapshot to `kittest.toml`'s floor. The hosted runner's paravirtual GPU is allowed the per-test failed-pixel counts its runs measured (`crates/brightfield-shell/tests/support/gpu_jitter.rs`). Never redraw a baseline from a hosted run's output.

A full run needs the pinned toolchain, `cargo-deny`, `python3`, a clone of `open-analytics` beside this checkout, and the pinned DuckDB CLI in `target/duckdb-cli`. A step missing one of them stops and says what to install or where to put it.

Keep private planning identifiers out of code, commit messages, pull request text and branch names; no CI step scans for them.
