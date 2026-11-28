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

A pull request's gate is `cargo xtask gate <pr>`, on the Mac Studio. It merges the pull request's head with `main` in a clean worktree, runs that tree's own `cargo xtask ci` in an isolated environment, and posts the verdict as the `local-gate` commit status on the head, which Forge shows beside GitHub's checks. A merge waits on a green `local-gate` and a closed roborev branch review. Start the gate as soon as the branch is pushed, without waiting for roborev, and run it again after each fix round, since a new head has no status until the gate runs on it. One gate runs on the machine at a time, and it is its session's one cargo build while it runs. It takes every core when no other build holds a slot, and a session's 6 otherwise. The gate leaves `brightfield-shell`'s own test targets, its end-to-end and screenshot tests, and the managed edition's tests to the nightly, so before pushing run the end-to-end tests your change touches (`cargo test -p brightfield-shell --test <target>`); `cargo xtask ci --step <name> --nightly` runs a test step over the whole suite. `docs/pull-request-gate.md` says how it works and why.

No push, pull request or schedule starts a hosted macOS job. `main`'s backstop is the Mac Studio's nightly: a LaunchAgent runs `cargo xtask nightly` at 02:30 local time, which runs the full gate on `main`'s head plus the network tests and the managed-DuckDB build, and posts the `nightly-gate` commit status (and `nightly-network`, which does not make `main` red). Each gate reads the newest nightly verdict first. A red one is flagged on the `main-red` Kata issue and fixed forward, with no revert, and a nightly with no verdict in 36 hours is flagged on `nightly-missing`. Install the agent with `scripts/nightly/install.sh install`, run it now with `scripts/nightly/install.sh run-now`; its log is `~/.cache/meridian/gate/logs/nightly-<sha7>-<date>.log`. `lint.yml` runs `fmt` and `comment-citations` on Linux for each pull request; the other hosted workflows (`test`, `lint-macos`, `nightly-network`, `managed-duckdb`, `brew-install-branch`, `signing-rehearsal`) run by hand with `gh workflow run <file> --ref <branch>`, and the gate prints the command for the packaging checks when a pull request touches their paths. `release` and its clean-Mac install check keep GitHub's Macs, for release tags only, and `store`, which builds and signs the Mac App Store package, runs by hand and on `store-v*` tags. `license` and `public-hygiene` still run on pull requests.

Before each commit, run `cargo xtask ci --step fmt` and `--step clippy`, and the tests the change touches. After each commit, run `--step comment-citations`. It reads the branch's commits rather than the staged work and checks the claims in changed comments at `HEAD`, so a bad comment is caught before the branch is pushed.

Pixel baselines are drawn on the Mac Studio's Apple M5 Max, which holds every snapshot to `kittest.toml`'s floor. The hosted runner's paravirtual GPU is allowed the per-test failed-pixel counts its runs measured (`crates/brightfield-shell/tests/support/gpu_jitter.rs`). Never redraw a baseline from a hosted run's output.

A full run needs the pinned toolchain, `cargo-deny`, `cargo-nextest`, `python3` and the pinned DuckDB CLI in `target/duckdb-cli`. A step missing one of them stops and says what to install or where to put it. The gate tests Brightfield's own Protocols only. The nightly also reports which Protocols on open-analytics' `main` the pinned arc refuses (`cargo xtask ci --step sibling-manifests`, with a clone of `open-analytics` beside this checkout), on its own status and outside the gate's verdict, since a check against another repository's `main` never blocks a merge.

Keep private planning identifiers out of code, commit messages, pull request text and branch names; no CI step scans for them.

## Docs

`docs/operation-forms.md` says how arc's operations are started, made, previewed, kept and saved in the window, what each needs and why; read it before changing an operation, its slot or key, the shared preview and keep path, or adding one.

`docs/ribbon.md` says what the ribbon's tabs list, how a command reads against the window, and how a greyed command or its refused key says why; read it before changing the ribbon or a Global key handler's refusal.

`docs/map-regions.md` says how a plot's dense regions are found, named, drawn and selected, and why; read it before changing the region pipeline, its thresholds, its naming rules or the regions switch.
