## Product Positioning

Brightfield targets analytics **authors** — the people who build dashboards and visualisations — much like Canva/Illustrator targets graphic designers. The primary user is an analyst who declares what they want to see in a Mosaic spec and gets an interactive, GPU-accelerated rendering.

Analytics **consumers** (people who view and interact with published dashboards) are a future audience. Good architectural choices now — portable specs via Mosaic, clean separation between authoring and rendering — ensure consumer-facing delivery can be enabled later without rearchitecting.

**Platform targets:** macOS, Linux, and Windows — the egui/eframe + wgpu window stack supports all three (Metal on macOS, Vulkan on Linux/Windows); macOS is the validated target today.

**Reference projects for data expression:** RillData (real-time analytics dashboards, DuckDB-native), MotherDuck (cloud DuckDB, query visualisation UX), apple/embedding-atlas (GPU-accelerated interactive data visualisation at scale).

## Planning

Feature cards, specs, and decisions for Brightfield are tracked centrally, outside this repository — this repo holds the code, not the planning substrate.

**This repo is public, and the planning substrate is not.** Identifiers from it — decision-record refs, task ids, milestone ids, acceptance-criterion shorthand, document ids, card ids, spec AC ids — resolve nowhere for a reader here and leak the shape of private work. Don't put them in code comments, doc comments, prose, commit messages, or PR text. If a pointer carried real meaning, write the actual rationale in plain English instead.

## The gate

Run `cargo xtask ci` before every commit; it is the one gate. It runs, in order, the steps a pull request's checks run, stops at the first that fails, and ends with a summary naming each step, its wall time, and the steps the failure skipped. CI's jobs call the same steps by name, so a green run here is a claim about the set CI runs. `cargo xtask ci --list` names the steps, and `cargo xtask ci --step <name>` re-runs one of them.

A full run needs the pinned toolchain, `cargo-deny`, `python3`, a clone of `open-analytics` beside this checkout, and the pinned DuckDB CLI in `target/duckdb-cli`. A step missing one of them stops and says what to install or where to put it.

The command scans the tracked files and the commit messages your branch adds for planning identifiers. It cannot read pull request titles or descriptions (CI scans those, as the summary says), review comments or branch names, and those are where leaks have happened: keep identifiers out of them yourself. If the identifier scan flags something genuinely legitimate, tighten the pattern and add the innocent string to `scripts/public-hygiene-innocent-strings.txt` (a tracked fixture the scan must stay silent on) rather than reaching for `scripts/public-hygiene-allowlist.txt`; an allowlist entry that does not parse, or that no longer suppresses anything, is a hard failure.
