## Product Positioning

Brightfield targets analytics **authors** — the people who build dashboards and visualisations — much like Canva/Illustrator targets graphic designers. The primary user is an analyst who declares what they want to see in a Mosaic spec and gets an interactive, GPU-accelerated rendering.

Analytics **consumers** (people who view and interact with published dashboards) are a future audience. Good architectural choices now — portable specs via Mosaic, clean separation between authoring and rendering — ensure consumer-facing delivery can be enabled later without rearchitecting.

**Platform targets:** macOS, Linux, and Windows — the egui/eframe + wgpu window stack supports all three (Metal on macOS, Vulkan on Linux/Windows); macOS is the validated target today.

**Reference projects for data expression:** RillData (real-time analytics dashboards, DuckDB-native), MotherDuck (cloud DuckDB, query visualisation UX), apple/embedding-atlas (GPU-accelerated interactive data visualisation at scale).

## Planning

Feature cards, specs, and decisions for Brightfield are tracked centrally, outside this repository — this repo holds the code, not the planning substrate.

**This repo is public, and the planning substrate is not.** Identifiers from it — decision-record refs, task ids, milestone ids, acceptance-criterion shorthand, document ids, card ids, spec AC ids — resolve nowhere for a reader here and leak the shape of private work. Don't put them in code comments, doc comments, prose, commit messages, or PR text. If a pointer carried real meaning, write the actual rationale in plain English instead.

## The gate

`cargo xtask ci` is the one gate, and CI runs it on each push to a pull request; a merge waits on it being green. It runs, in order, the steps a pull request's checks run, stops at the first that fails, and ends with a summary naming each step, its wall time, and the steps the failure skipped. CI's jobs call the same steps by name, so a green run here is a claim about the set CI runs. `cargo xtask ci --list` names the steps, and `cargo xtask ci --step <name>` re-runs one of them.

Before each commit, run `cargo xtask ci --step fmt` and `--step clippy`, and the tests the change touches. After each commit, run `--step comment-citations`. It reads the branch's commits rather than the staged work and checks the claims in changed comments at `HEAD`, so a bad comment is caught before the branch is pushed.

Run the full `cargo xtask ci` locally to reproduce a CI failure, not before each push. In October 2026 a full local run took between 4 and 55 minutes on a 16 GB Mac and a single-threaded run of the whole test suite over an hour, while CI's sharded run of the same steps took about 25 minutes. Quote test totals in a pull request from its CI run.

A full run needs the pinned toolchain, `cargo-deny`, `python3`, a clone of `open-analytics` beside this checkout, and the pinned DuckDB CLI in `target/duckdb-cli`. A step missing one of them stops and says what to install or where to put it.

Keep private planning identifiers out of code, commit messages, pull request text and branch names; no CI step scans for them.
