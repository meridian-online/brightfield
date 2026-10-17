# Agent-written Protocols, headless exports and shared versions

## Why

arcform's next release ships agent skills that take an analyst from a dataset and a question to an answer, an arcform Protocol, charts and an optional report. The agent writes each chart as a Mosaic spec at `panels/<name>.yaml` in the Protocol folder and writes `report.md` there with `arc report write`. arc draws no chart: the skills send the analyst to Brightfield to see the charts and how the answer was derived, and offer a PDF for anyone who will not open Brightfield. That arcform release waits on a Brightfield release that does what this document describes.

Brightfield is the analyst's primary tool for understanding the data. The agent and the analyst share one record of the work in open formats: DuckDB SQL for the steps, Mosaic specs for the charts, the Protocol folder for the whole. Neither needs anything proprietary to read what the other made. This release makes that sharing work in both directions:

- **Agent to analyst.** A Protocol an agent wrote opens in Brightfield with every chart drawn, and the analyst can see which versions of each chart came through arc rather than Brightfield (and, once arc records an author, which the agent made).
- **Analyst to agent.** The agent can see what the analyst has saved: which charts changed and when (through the Protocol's version log), and what they look like and hold (through headless PNG and data exports).

The agent runs on the analyst's machine with the analyst's permission, and reads the same raw data, Protocol and outputs on disk. It sees what is saved. Unsaved exploration in an open window is not part of the record and is not exposed.

## Goals

1. **A Protocol an agent wrote opens with its charts.** Opening a Protocol folder arc made (several steps, tables made by SQL and operator steps, more than one file under `panels/`, none written by Brightfield) draws every panel, each reading the table its `data: { from: … }` names from the Protocol's own database. Nothing is regenerated from a data file first.
2. **A panel Brightfield cannot draw says why, in its own tile.** A panel naming a table or column the Protocol does not make, a mark Brightfield does not draw, or a spec that does not parse shows a message in its tile naming the problem. The other panels draw. Today a missing table fails the whole open.
3. **The report exports to PDF, headless.** A command an agent can run with no window renders `report.md` to PDF with each panel the report lists drawn in place, as Brightfield draws it, locally and with no network. It exits non-zero with a message when there is no `report.md` or a panel cannot be drawn.
4. **The agent can see a panel as a picture and as data, headless.** From the same command line, a panel exports to PNG and the table behind it to a data file (CSV, and Parquet), with no window.
5. **Brightfield's saves join the Protocol's version log, and its version list says how each version was made.** Each chart the analyst saves adds a line to the Protocol's `arcform-log.txt` marked as Brightfield's, and a panel's Versions list in the app shows, for each version, whether it came through Brightfield, arc's command line or arc's agent tools (MCP). Until arc records an author, that is all the log can say: an agent driving arc from a terminal reads the same as a person doing so. Once arc records an author, the list names who made each version, agent or analyst.
6. **A released Brightfield does all of this** (not a pre-release), installable the way the README says, so arcform's release eval can open each eval task's Protocol and export its PDF.
7. **The chart-kind registry and the record of departures from Mosaic stay current**, because arcform's presenting skill teaches agents to write against them.

## Non-goals

- **A Protocol-wide "what changed since you last opened it" view.** The analyst seeing the agent's changes across the whole Protocol, read from the log, is the next release's feature. This release shows who made each version of each chart, in the existing Versions list.
- **Exposing unsaved state.** No live channel from the running app to the agent.
- **A full report-design system.** The PDF starts simple: one clean, readable layout. Themes, templates, cover pages, page furniture beyond page numbers, and analyst-authored layout are later work.
- **Editing `report.md` in Brightfield.** It is read and rendered, not edited.
- **Recording who made a version.** The `author` field in the log is arcform's work, already decided there (an agent declares its name; the analyst's name comes from one setting arc and Brightfield share). Brightfield reads it when present and passes the analyst's name when it records a save; it does not define the field.
- **Changing what arc's panel check or report contain.** Brightfield reuses them.
- **Windows and Linux headless verification.** The headless commands must not assume macOS, but this release verifies them on macOS only, which is the validated platform.

## Anti-goals

Things that would make this a failure even if the commands technically work:

- **A PDF an analyst would wince at.** Hugh named what that means: bad layout, misleading titles or axes, overlapping elements or annotations, badly placed legends, and tables with too much wrapping. The PDF must avoid all five on the fixture below and on arcform's eval Protocols.
- **A chart in the PDF or PNG that differs from what Brightfield draws in the window.** One renderer, one look.
- **An empty tile.** A panel that cannot draw always says why.
- **Silently drawing stale or different data.** If the database the panels read from is from a different run than the one `report.md` cites, the PDF says so rather than presenting the charts as that run's.
- **A second, Brightfield-only copy of arc's rules** (where the database lives, what the panel check reports, the log's format). Brightfield calls arc for them.

## Current state

What the code does today, read on 2026-10-17:

- **Opening.** A Protocol opens only as one of a few shapes: a data file; a one-step Protocol whose single SQL step reads one CSV, TSV or Parquet file; a load followed by Brightfield-generated filter, sort or SQL steps; or, behind an opt-in, a DAG view. The manifest is read to find the data file, which is profiled into a generated dashboard; a single chart file `panels/<protocol-name>.yaml`, when present, replaces that dashboard. Brightfield never reads the tables a Protocol's run made. A mark's `from:` names a `data:` entry in the same spec, which the chart's in-process DuckDB loads.
- **Unsupported marks** raise a banner and the rest of the chart draws. **A missing table or column** fails the load with an error string; in a session it raises a chart-fault banner. There is no per-tile message.
- **Headless rendering** exists in `brightfield-shot` (full window through an offscreen wgpu texture, or chart-only through Vello), which needs a real GPU adapter and is not in the release package. The shipped binary has no subcommands; `--shot-after N --shot-out` opens a real window and captures it.
- **Versions.** Brightfield records each chart Save in arc's local history (`~/.arcform/history`) under the way `brightfield`, for the one chart file only. It is pinned to an arc revision from before arc began writing `arcform-log.txt` (5 October), so its saves do not appear in any Protocol's log.
- **No Markdown or PDF** dependency anywhere in the workspace.
- **Release.** A `v*` tag builds macOS tarballs and DMGs, publishes a GitHub release and updates the Homebrew tap (`brew install meridian-online/tap/brightfield`). The README's Quick Start still describes running from a clone, and does not mention brew.
- **DuckDB.** Brightfield bundles the `duckdb` crate at 1.5.2; arc's main pins 1.5.6. arc's runs write the database with the DuckDB CLI.

arc's side, on its main branch:

- **The database** of a Protocol that names no `db:` lives at `$ARCFORM_DB_DIR` or `~/.arcform/db`, under a directory keyed by a hash of the Protocol folder's canonical path, as `<name>.duckdb`. An explicit `db:` is relative to the folder. The lookup is `working_db::locate`, crate-private.
- **The panel check** (`panel_check`) reports a mark that reads a table or column the Protocol does not make, from the database when it exists. Crate-private.
- **The report** (`report.md`) is the agent's prose followed by three sections arc writes: "How this was derived" (each step, its description, status and row count), "Charts" (one list item per panel: `` `panels/<file>`: <title> ``, plus any finding of the panel check) and "Run" (the run id, time, outcome and asset hashes). The Charts section notes that findings are checked against the current database, not the run's.
- **The version log** `arcform-log.txt` is newline-delimited JSON in the Protocol folder, one line per recorded version of `arcform.yaml` or any file beside it: `at` (UTC, milliseconds), `file`, `kind` (`save`, `checkpoint`, `unsaved`), `interface` (optional: `terminal`, `mcp`, or a library caller's own word), `version` (the snapshot id `arc history show/restore` take), and for `arcform.yaml` the steps touched (`step` + `change`, or `steps`). No file contents. The snapshots stay in the local store. `.gitattributes` merges the log by union. A caller's own interface word comes from `HistoryWay::new`.

## The fixture

An agent-written Protocol made on 2026-10-17 by running arcform's `analysis-co2-per-capita` eval task headless in Claude Code with arc built from arcform's main and its skills installed, the prompt extended to answer the plan checkpoint in advance ("show the G7 as its own countries … write me a report at the end"). It is the Protocol this release's first goal opens. It has:

- five steps: an `http_fetch` of Our World in Data's CO2 CSV (13.7 MB), a raw load, a cleaned table of nine countries from 1990 (`clean_co2_per_capita`, 315 rows), a per-country change table (`co2_per_capita_change`, 9 rows), and the year China overtook each G7 country (`china_overtakes_g7`, 7 rows);
- two panels: `co2_per_capita_change_pct.yaml` (a `barX` of percent change, filled by group, sorted, with a colour legend) and `co2_per_capita_since_1990.yaml` (a `lineY` per country over time, with a colour legend, `xTickFormat: d`, fixed width and height);
- `report.md` with an answer, caveats, next steps and arc's three sections; `schemas/`, `README.md`, `.gitattributes` and `arcform-log.txt` (13 lines, every one `"interface":"terminal"`).

It already exercises the gaps: `lineY` is a Mosaic mark Brightfield does not yet draw (it draws `line`), and the registry offers no line-over-time kind, yet a time series per group is the obvious chart for a question about change over time.

The fixture is committed at `tests/fixtures/agent-protocols/co2-per-capita/` with every file the agent wrote kept as written, except the data file, which is cut to the 315 rows the cleaned step keeps (the nine countries, 1990 on) so it is small (252 KB in all) and the tests run offline, as the crosswalk chart's thumbnails already do from a reduced copy. Run against the cut-down file, the steps produce `clean_co2_per_capita` and `co2_per_capita_change` with the same rows and values as against the full file (checked on 2026-10-17 by hashing both). The fetch step must not run in tests. How (a `db:` the test builds, running arc with the fetch skipped, or building the database directly from the reduced CSV) is the builder's call. A test that the fixture still matches what the skills write is not required; arcform's eval is where that is checked.

### The eval Protocols

Hugh chose to run arcform's other nine eval tasks before the build, so the release covers the marks agents actually write rather than the one fixture's. They run headless in Claude Code the same way the fixture was made, each prompt extended to say the analyst is away and the agent should go ahead, chart what helps and write a report. Interactive checkpoints are answered in advance by that line, so these runs are not eval scores; they are a sample of what agents write. The plan's first task collects every panel they produce, lists each mark, channel and plot attribute Brightfield does not handle, and decides for each whether it is drawn in this release or recorded as a deviation with its per-tile message. Their Protocols are kept outside this repository; any that a test needs is added as a fixture the same way as the CO2 one.

## Decided

### Opening an arc Protocol

- Brightfield opens a Protocol folder by asking arc where its database is and opening it read-only, then drawing each `panels/*.yaml` against it. A mark's `data: { from: T }` resolves to the table `T` in that database when the panel's own top-level `data:` does not define `T`, the same rule arc's panel check uses.
- The existing shapes (a data file, a one-step Protocol, a Brightfield-generated Protocol, its single chart file) keep working as they do; this is a new route for any Protocol that has run and has a `panels/` folder, not a rewrite of the old ones. Where a Protocol fits both, the builder decides which route wins and records why.
- **Brightfield never holds the Protocol's database open.** DuckDB lets several processes read a file but refuses a writer while any reader holds it, so a held handle would make the agent's `arc run`, and Brightfield's own Run, fail while the Protocol is open. Brightfield attaches the database read-only, copies the tables the panels read into its own in-memory database, and detaches before drawing. It reloads when the Protocol's newest run changes. A headless export does the same, and one that meets the database locked by a run in progress fails with a message saying the Protocol is being run, rather than waiting.
- **A database written by a DuckDB that Brightfield's cannot read** (an agent's arc using a newer DuckDB command line, after arc's 2.0 move) is a plain message in the tiles and on stderr naming the version mismatch, not an opaque open failure.
- **A Protocol that has not run** has no database. Brightfield says so and offers to run it (the existing run path), rather than drawing empty tiles.
- **Several panels** are laid out as tiles, one per panel, in file-name order unless the builder finds a better rule. The layout's rules are the builder's, held to the anti-goals above.

### Per-tile messages

- Each tile that cannot draw shows a message in its own tile naming the panel file and the problem in plain words: the missing table or column (from arc's panel check), the mark Brightfield does not draw (from Brightfield's own preflight), or the parse error. The rest draw.
- The headless commands use the same messages, on stderr, one line per problem, naming the panel file.

### Headless commands

- They are subcommands of the shipped `brightfield` binary, so an agent finds them with `brightfield --help`: `brightfield export-pdf <protocol-dir> [--out <file>]`, `brightfield export-png <protocol-dir> <panel> [--out <file>]`, and `brightfield export-data <protocol-dir> <panel> [--format csv|parquet] [--out <file>]`. Names and flags can be adjusted by the builder if Brightfield's existing CLI conventions call for it; the PDF one is what arcform's skill will name, so it is settled before the release.
- No window opens. They use Brightfield's own renderer, so a chart is drawn the way the window draws it. They need no network.
- Each exits 0 on success and prints the written path, and exits non-zero with a message naming the problem otherwise: no `report.md`, no such panel, a panel that cannot be drawn, a Protocol that has not run.
- `export-data` writes the rows the panel's marks read from the database, not a re-run of anything.

### The PDF

> **Under redesign (2026-10-18).** Built as written below, the PDF is a snapshot. The agent's prose carries the numbers it saw, so a re-run on revised data redraws the charts but leaves the sentences behind; the stale-table notice only warns that they disagree. Hugh expected a report in the spirit of Evidence (evidence.dev): Markdown whose numbers and charts are bound to queries over the Protocol's own tables and filled at render time, so a re-run cannot leave the prose behind. What the report is, who writes it and where it renders is being brainstormed in its own session and will amend this section. The rendering pieces built here (the Typst world, charts with legends drawn as the window draws them, tables, escaping, offline fonts) are expected to carry over.

- It is `report.md` rendered as a document, top to bottom, with each item of the report's Charts section replaced by that panel drawn in place, under its title. Any panel the prose links to by its path is drawn there too, if the builder finds that straightforward; otherwise the Charts section is the only place charts appear.
- **Start simple.** One page size (A4, portrait), one typeface family (Brightfield's bundled fonts), page numbers, and no cover page. The bar is the anti-goals list: clean layout, titles and axes exactly as the panel declares them, nothing overlapping, legends placed as Brightfield places them, and tables that do not wrap their numbers. A chart is never split across pages and is sized to the text width. A table too wide for the page is set smaller, then rotated or split, before any cell wraps a number.
- The "Run" section is shown as written. When a table a chart reads holds different content from what the report's run recorded, the PDF says so above the Charts. "Different" is judged by the content hashes arc records for each table in its run record, compared with the newest run's, not by run id: a later run that changed nothing raises no notice, and the fixture, whose derived tables are unchanged by the cut-down data, raises none.
- It embeds its fonts and needs no network.

### Versions and the log

- Brightfield moves its arc pin to a revision that writes `arcform-log.txt`, so every chart Save adds a line with `"interface":"brightfield"`. When Brightfield runs arc to record a step (a grid view saved as a step), that line also says `brightfield`, not `terminal`.
- Brightfield records a version for each panel the analyst saves, not only `panels/<protocol-name>.yaml`.
- Each panel's Versions list shows who made each version: Brightfield, arc's command line or arc over MCP from the entry's interface, and the author's name when arc records one.
- The list is read from the Protocol's `arcform-log.txt` (who, when, which version), and each version's text from the local store. The log travels with the folder and the snapshots do not, so a version whose snapshot is not on this machine (a fresh clone, a copy from another machine) is listed and marked as not on this machine, and cannot be shown or restored. This keeps the list true on any machine and lets a test show the agent's versions from the committed fixture.
- When arc's author field lands, Brightfield passes the analyst's name from the shared setting with each Save. If it has not landed when this release is cut, the release ships without it and the Versions list shows the interface alone.

### Registry and deviations

- `lineY` and `lineX` are drawn, and the registry gains a line-over-time kind, since the fixture's agent reached for one unprompted. Any other Mosaic mark the eval Protocols use that Brightfield does not draw is listed in the plan and either drawn or recorded as a deviation with its per-tile message.
- `DEVIATIONS.md` (and `deviations.yaml`) and the chart-kind registry are updated in the same branch as any change to what Brightfield draws.

## Calls I made on Hugh's behalf

- **The PDF engine is Typst, as a library** (the `typst` and `typst-pdf` crates), with charts rendered by Brightfield's Vello renderer to images at print resolution. It runs offline, embeds fonts, handles tables and page breaks well, and avoids shipping a browser. The builder may pick another engine if Typst fails a hard requirement (offline, fonts, no chart split across pages); the reason goes in the plan.
- **arc exports what Brightfield needs** rather than Brightfield copying it: the database lookup, the panel check, the newest run record's table hashes and a reader for `arcform-log.txt` become part of `arc::spec`. That is a small arcform pull request ahead of Brightfield's pin move, and it keeps each rule in one implementation.
- **The PNG and data exports are in this release.** Hugh confirmed the agent needs them to see what the analyst sees, and they share the PDF's headless rendering, so they cost little extra.
- **Headless means no window, not no GPU.** The commands use the machine's GPU through wgpu without a surface, as `brightfield-shot` does. A software fallback for machines with no GPU is not in this release; on such a machine the command fails with a message saying so.
- **The DuckDB crate moves to arc's version** (1.5.6) with the arc pin, since both link one DuckDB, and the DuckDB command line `scripts/fetch-duckdb-cli.sh` pins for runs and the release package moves with it (version, both assets and their hashes), so the engine that writes a run and the one that reads it stay one version.
- **arcform's skill changes are arcform's.** This release gives the skill what to name (the PDF command, the line kind, the per-tile messages, reading the log). The edits to `presenting-findings` and its `panels.md` go in an arcform pull request, not here.
- **Agent detection for the author field** (DuckDB 2.0's list of environment variables agents set, such as `CLAUDECODE`, `CODEX_THREAD_ID`, `CURSOR_AGENT`, `GEMINI_CLI`) is a suggestion for arcform's author work, so an agent that declares nothing is recorded as that agent rather than as the account. It is not built in Brightfield.

## Left to the builder

- The tile layout for several panels, and the order.
- Which route wins when a Protocol fits both an old shape and the new one.
- How the fixture's database is built for tests without the fetch.
- How report Markdown maps to Typst (or the chosen engine), including how wide tables are handled within the rule above.
- Exact subcommand flags beyond those named, and the data export's default format.
- Whether to draw panels the prose links to as well as those in the Charts section.

## Constraints

- **The arcform export lands before arc's DuckDB 2.0 move** (Hugh's decision). The arcform pull request merges to arc's main before that move starts on 2026-10-21, and Brightfield pins to that revision, on DuckDB 1.5.6. Brightfield takes DuckDB 2.0 later, with arc's. If the arcform change is not merged in time, this is raised with Hugh rather than taking 2.0 here.
- One cargo build at a time on Hugh's Mac (16 GB). Brightfield's build shares that slot with arc's DuckDB 2.0 move from 2026-10-21.
- **Where the build ends.** The build ends at a reviewed pull request whose checks pass on the fixture and on whichever eval Protocols became fixtures. Hugh merges and tags the release; arcform's eval then runs against it, and what it finds comes back as follow-up fixes.
- `cargo xtask ci` is the gate. Nothing in this public repository names private planning identifiers.
- The release must be a tagged release, installable the way the README says; the README's install section is brought up to date (Homebrew) in the same release.

## Done when

1. The fixture opens in Brightfield with both panels drawn from its database, and each panel's Versions list shows the agent's versions marked as arc's.
2. A copy of the fixture with a panel naming a missing column, and another with a mark Brightfield does not draw, opens with that tile showing the problem and the other tile drawn.
3. `brightfield export-pdf <fixture>` writes a PDF with the report's prose and both charts drawn in place, offline, and Hugh has looked at it and found none of the five things he named; `export-png` and `export-data` write a panel's picture and rows. Each exits non-zero with a message on a Protocol with no `report.md`, an unknown panel, or a panel that cannot draw.
4. A Save in Brightfield adds a line to the fixture's `arcform-log.txt` marked `brightfield`.
5. A tagged Brightfield release does all of the above, installed by `brew install meridian-online/tap/brightfield`, and arcform's eval opens each eval task's Protocol and exports its PDF with it.
