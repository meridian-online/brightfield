# Brightfield

> **Early Release** — Brightfield is in early development. The project is in the discovery and design phase. Expect breaking changes to spec semantics, CLI arguments, library APIs, and rendering output between releases. Pin to a specific version if stability matters for your use case.

GPU-native desktop application for interactive data visualisation over large tables. How large depends on the mark: an aggregating mark — density, binned density, heatmap, bar — is served from a small pre-aggregated summary, so a gesture costs what the summary costs rather than what the table costs; a row-level mark such as a raw scatter draws one mark per row, has no summary to stand in for it, and its cost tracks the rows. [What makes an interaction fast](docs/interaction-speed.md) says which marks get which, and the measured gap between them is in [`benchmarks/results/`](benchmarks/results/). Brightfield combines [Mosaic](https://idl.uw.edu/mosaic/)'s declarative specification grammar and coordinator architecture with a [Vello](https://github.com/linebender/vello) GPU 2D scene renderer hosted in an [egui](https://github.com/emilk/egui)/[eframe](https://github.com/emilk/egui/tree/main/crates/eframe) application shell (sharing one wgpu device) and [DuckDB](https://duckdb.org)'s analytical query engine — all implemented in Rust for a single-process, zero-serialisation-overhead experience.

The goal is a tool that can interactively visualise and explore datasets from thousands to billions of records with fluid, GPU-rendered interactions, without the performance ceiling of browser-based rendering or the overhead of a webview shell — reached today for aggregating marks and not for row-level ones, which is the split [what makes an interaction fast](docs/interaction-speed.md) sets out and [`benchmarks/results/`](benchmarks/results/) measures.

## Quick Start

Install the released app on macOS with Homebrew:

```sh
brew install meridian-online/tap/brightfield

# Open a chart spec, a data file, or a Protocol an agent wrote with arc
brightfield path/to/protocol/arcform.yaml
```

The same binary exports a Protocol with no window, for an agent or a script. Each command needs the Protocol to have run, prints the path it wrote, and exits non-zero naming the problem when it cannot:

```sh
# The Protocol's report.md as a PDF, each panel the report lists drawn in place
brightfield export-pdf path/to/protocol

# One panel as a PNG, drawn as the window draws it
brightfield export-png path/to/protocol <panel>

# The rows a panel reads from the Protocol's database, as CSV or Parquet
brightfield export-data path/to/protocol <panel> --format parquet
```

`brightfield --help` lists their options. `--out` names the file to write; for `export-data`, a path that is a folder, or ends in `/`, is a folder to write one file per table into. `export-png` and `export-pdf` print what a panel's load has to say (an option nothing draws, colours that repeat, a mark that drew nothing) on stderr, one line per note naming the panel file, and still write their file. None of them needs the network; the PDF and PNG exports draw on the machine's GPU.

To build from source instead, Brightfield needs a recent Rust toolchain (1.95+, matching the CI pin). From a clone of the repo, render the self-contained example spec:

```sh
# Opens the native, GPU-rendered window
cargo run -p brightfield-shell -- examples/scatter.yaml

# Headless / CI — render the composed chart straight to a PNG
cargo run -p brightfield-shell --bin brightfield-shot -- \
  --spec examples/scatter.yaml --out scatter.png --vello-only
```

A build run from source has no FineType bundle beside it, so its column headers name no types. `scripts/stage-finetype-bundle-for-source-run.sh` downloads the pinned bundle into `target/debug/finetype` (pass `release` for a `--release` build), where the next `cargo run` finds it. It lives under `target/`, so git ignores it.

`examples/scatter.yaml` is inline data — nothing external to download. Over 50 further Mosaic specs ship under [`crates/brightfield-spec/vendor/mosaic-specs/yaml/`](crates/brightfield-spec/vendor/mosaic-specs/yaml/); note that many read from Parquet/CSV files that are not bundled, so not all of them render out of the box yet.

The live window and the headless shot both run on the egui/wgpu stack. macOS (Metal) is the validated target today; the Linux/Windows window stack is wgpu-supported but not yet CI-validated (see [Platform Support](#platform-support)).

## Design Principles

- **Spec-driven.** The Mosaic declarative specification (YAML/JSON) is the stable contract. Users author visualisations and dashboards as portable spec files. The rendering backend is an implementation detail.
- **Database-first.** All data-intensive computation — filtering, aggregation, binning, joins, window functions — is pushed to DuckDB. The renderer receives only the minimal data needed for display.
- **Single-process, in-memory.** DuckDB runs in-process via `duckdb-rs`. No HTTP server, no WebSocket layer, no serialisation boundary between the query engine and the coordinator. Data flows as Arrow record batches in shared memory.
- **GPU-native rendering.** Charts are drawn as [Vello](https://github.com/linebender/vello) 2D scenes rasterised on the GPU via wgpu (Metal on macOS, Vulkan on Linux). The egui/eframe shell hosts the window and widget chrome on the same wgpu device, so a Vello scene lands on a texture egui samples directly — no second device, no CPU readback.

## Features

- **Mosaic spec compatibility** — author dashboards as portable YAML/JSON spec files; specs authored for Mosaic's web environment should work unchanged
- **Reactive params** — single-value variables shared across components; slider, menu, search, and table widgets drive live re-queries
- **Cross-filtered selections** — first-class query predicates combine filters from multiple interactors across multiple views, with `intersect`, `union`, `single`, and cross-filter resolution
- **Grammar-of-graphics marks** — dot, bar, rect, cell, text, tick, rule, line, area, density (KDE), regression, geo (GeoJSON), hexbin, contour, raster/heatmap
- **Interactive navigation** — pan, zoom, brush (`intervalX/Y/XY`), toggle, nearest-point hover, highlight
- **Multi-view composition** — `hconcat`, `vconcat`, `hspace`, `vspace` arrange plots, inputs, and legends into dashboards
- **Query optimisation** — automatic pre-aggregation at pixel resolution, M4 downsampling for line/area marks, result caching, priority queuing. Interaction cost stops tracking row count *where a summary can be pre-computed*: 0.5–3.6 ms per gesture at ten million rows on an aggregating mark, against 65–80 ms without one. [What makes an interaction fast](docs/interaction-speed.md) sets out when you get that and when you do not
- **Direct data loading** — Parquet, CSV, JSON, inline data, and DuckDB database files
- **An analyst's own files as a model** — a folder of CSV, TSV or Parquet files, an Excel workbook, or a DuckDB or SQLite database opens with each table typed by FineType and cleaned through FineType's casts, and the tables drawn as an [Apache Ossie](https://github.com/apache/ossie) model the analyst reshapes
- **Single native binary** — no webview, no HTTP server, no runtime dependencies beyond system graphics drivers

## Architecture

The system comprises four layers:

```
┌──────────────────────────────────────────────┐
│              Mosaic Spec (YAML/JSON)         │
│         Declarative application definition    │
└──────────────────┬───────────────────────────┘
                   │ parse
                   ▼
┌──────────────────────────────────────────────┐
│              Spec Parser → AST               │
│       Abstract syntax tree representation     │
└──────────────────┬───────────────────────────┘
                   │ drives
                   ▼
┌──────────────────────────────────────────────┐
│            Coordinator (Rust)                 │
│  Params, Selections, query management,       │
│  caching, pre-aggregation optimisation        │
│                                              │
│  ┌─────────────┐    ┌─────────────────────┐  │
│  │ Query Engine │    │ Selection Manager   │  │
│  │ (SQL gen)    │    │ (predicates, cross- │  │
│  │              │    │  filter resolution) │  │
│  └──────┬───────┘    └─────────────────────┘  │
│         │                                     │
│         ▼                                     │
│  ┌─────────────────────────┐                  │
│  │  DuckDB (in-process)    │                  │
│  │  via duckdb-rs          │                  │
│  │  Arrow record batches   │                  │
│  └─────────────────────────┘                  │
└──────────────────┬───────────────────────────┘
                   │ minimal data
                   ▼
┌──────────────────────────────────────────────┐
│        Renderer (Vello scene + egui)          │
│  Marks, axes, scales, legends, interactors    │
│  Vello 2D scene, GPU-drawn via wgpu           │
│  (Metal/Vulkan); presented by the egui shell  │
└──────────────────────────────────────────────┘
```

**Spec parser.** Mosaic-compatible YAML or JSON specifications are parsed into a Rust AST mirroring Mosaic's `parseSpec()` output structure. Scope covers data definitions, param/selection declarations, plot/mark/encoding definitions, input widgets, layout, metadata, and config.

**Query engine.** A Rust equivalent of Mosaic's `mosaic-sql` package. Translates AST marks and encodings into DuckDB-dialect SQL, handling aggregates, window functions, bin calculations, and dynamic parameter substitution via Mosaic's `$param` expression syntax.

**Coordinator.** A Rust equivalent of Mosaic's `mosaic-core` package. Manages reactive params, first-class selections with resolution strategies, SQL-keyed result caching, automatic pre-aggregation, M4 downsampling, and priority queuing for interactive vs background queries. Data flows as Arrow record batches via `duckdb-rs`, staying in Rust memory throughout.

**Renderer.** A framework-free `brightfield-render` crate turns marks, scales (linear/log/sqrt/band/ordinal/time/colour), axes, legends, interactors, and layout primitives into a [Vello](https://github.com/linebender/vello) `Scene` — no UI-framework dependency, so the same scene builder feeds both the live window and the headless PNG path. The egui/eframe shell (`brightfield-shell`) rasterises the Vello scene on the shared wgpu device and presents it, wiring pointer events into the interaction transitions (the framework-free interaction logic lives in `brightfield-ui`). Text is shaped and rendered with [skrifa](https://github.com/googlefonts/fontations).

### Agent-written Protocols

An [arcform](https://github.com/meridian-online/arcform) Protocol that has run and holds a `panels/` folder opens as one tile per `panels/*.yaml`, in file-name order. Each panel is a Mosaic spec whose marks read the tables the Protocol's run made, by name: a mark's `data: { from: T }` reads table `T` from arc's database whenever the panel's own top-level `data:` does not define `T`, the rule arc's own panel check applies.

Brightfield never holds that database open, since DuckDB refuses a writer while any reader holds the file and an open handle would make the next `arc run` fail. After arc's panel check, it attaches the database read-only to an in-memory DuckDB, copies each table a panel reads, detaches, and writes the copies to a Parquet snapshot in a temporary folder. Each panel is composed against the snapshot, and the snapshot is taken again when a new run record appears. A panel that cannot draw shows its problem in its own tile, naming the file (a missing table or column from arc's check, a mark Brightfield does not draw, a parse error), and the others draw. A panel's notes are shown above its chart.

The headless exports read the same snapshot. `export-png` draws one panel as the window draws it, legend included. `export-data` copies the tables the panel reads out of the snapshot as CSV or Parquet. `export-pdf` reads the Protocol's `report.md`, draws each panel its `## Charts` section lists, and hands the Markdown and the pictures to the `brightfield-report` crate. When a table a chart reads has changed since the run the report describes, judged by the content hashes arc records for each table, the PDF says so above the charts.

**`brightfield-report`** typesets that Markdown to PDF through Typst, offline, with the fonts bundled with the design system: A4 portrait, numbered pages, tables with numbers kept whole, and each listed chart drawn in its item's place under its title and never split across pages. It knows Markdown, Typst and PNG bytes and nothing of panels, databases or the GPU; the shell renders the charts and passes the pictures in.

### The front door's starts

With nothing open, the window shows the front door. Its Datasets row offers six starts that ship inside the binary, each card drawn from the screen a click lands on:

- **Palmer Penguins** and **the USGS earthquakes of 2025** carry their data in their folders, so their first run reads no network.
- **NYC yellow taxi trips** (January 2026, from the TLC), **Online Retail II** (from the UCI Machine Learning Repository) and **Epoch AI's notable models** fetch their data from the publisher's own URL on their first run, and their cards say "over the network". With no network, a banner names the URL the run could not reach, and the window stays up.
- **A run of the EDGAR↔GLEIF crosswalk** opens a Protocol with a recorded run, and reads no network.

The first five are shipped Protocols: the arc Protocol folders under [`examples/starts/`](examples/starts/), each holding a manifest, a load step, a saved schema, one panel, and a `datapackage.json` naming the source and its terms, compiled into the binary. Taking one writes its folder to `datasets/protocols/v<revision>/<id>` beside Brightfield's layout file, runs it with Brightfield's own arc runner when it has no successful run there yet, and opens its panel beside its grid, where an analyst's own run of the same Protocol would land. While the run is going the canvas says so, naming what it fetches and from where. A later click, or a relaunch that remembers the start, opens the run already there with no network. A failed run leaves no database, so the next click runs the Protocol again. A build that changes a folder raises its revision and writes the new folder beside the old one, which stays, since the reader may have saved edits there.

**A filter typed in the grid reaches the charts.** A dashboard's grid takes part in its crossfilter both ways: a brush narrows the grid, and a condition typed in the grid narrows every chart that reads the selection, while a grey layer that shows the whole table stays whole.

### An analyst's own files

The front door's Open… takes a single file or a folder. A single CSV, TSV or Parquet file opens on its grid and dashboard as it always has. A folder of them, an Excel workbook or a DuckDB or SQLite database file is written as an arc Protocol that loads every table, and opens on the model pane. One sheet of a workbook or one table of a database opens on its grid like a single file, through the same Protocol.

**What is read.** A folder is read at its top level only, with no recursion. It takes `csv`, `tsv`, `parquet`, `xlsx`, `duckdb`, `db` and `sqlite` files. A file it cannot read (a `notes.txt`, a corrupt CSV, an empty sheet) is passed over with its reason, which the model pane's header names, and the open goes on with the rest. Each table is named after its file, its file and sheet, or its database table, and two that would share a name get `_2`, `_3` in path order. What DuckDB can read, DuckDB reads: rows come from `read_csv`, `read_parquet`, the `excel` extension's `read_xlsx` and the `sqlite` extension, in SQL a Protocol step records, so the transformation can be rerun and traced. Those two extensions download the first time they are needed, and the canvas names the one it is fetching. DuckDB's `excel` extension cannot list a workbook's sheets, so only the sheet names are read with the `calamine` crate.

**Lineage to the row.** Every loaded and cleaned table starts with three columns: `__source_file`, `__source_part` (the sheet or database table, NULL for a flat file) and `__source_row`, the 1-based data row. A bad value in the model points at the line of the extract it came from.

**The Protocol folder.** It is made in Brightfield's data directory under `datasets/protocols/own/<name>-<n>/`, beside the starts' Protocols, and reads the analyst's files where they are. It holds:

- `arcform.yaml`, with a `load_<table>` step per table (`models/01_load_<table>.sql`, making `raw_<table>`) and a silver and a failed-values step per table kept in the model (`models/02_silver_<table>.sql`, `models/02_failed_<table>.sql`);
- `packages/<source>/datapackage.json`, a Frictionless 2.0 data package per source file, each field carrying its FineType label;
- `model.ossie.yaml`, the model;
- `.sources.json`, the tables the open found and the files it passed over, so the folder reopens without finding them again.

**Silver tables.** Each table kept in the model is cast column by column through its FineType label's transformation contract, the `transform` expression `finetype validate` applies, wrapped in `TRY`; a label with no transform falls back to `TRY_CAST` to its DuckDB type, and a VARCHAR or unlabelled column passes through. A value the cast turns to NULL is kept, with its lineage and raw text, in `<table>__failed`. A reference table folded into another is joined into it, its columns prefixed with its name. The transforms are vendored from FineType's label definitions at the pinned tag (`scripts/vendor-finetype-transforms.sh`), and the `finetype-pin` gate step refuses the table when its tag is not the bundle's pin.

**The model.** `brightfield-ossie`, a crate with no UI or DuckDB dependency, reads, writes and validates the Ossie document (version `0.2.0.dev0`, checked against the Ossie JSON Schema vendored at a recorded commit). Ossie's core has datasets, fields and relationships. Meridian's additions ride in `custom_extensions` entries under the `MERIDIAN` vendor name: each table's role (entity, event, reference, bridge, snapshot or unclear), each join's grade (likely or possible) and evidence, folds, tables left out, and who decided each element, the discoverer or the analyst. An element the analyst decided is never changed by discovery, verification or a rerun.

**The model pane.** One model, shown three ways, with one inspector. `v` steps through them, and `1`, `2` and `3` pick one:

- the diagram, with each table a card banded by its role (events on the left, entities on the right, the rest below) and each join an edge labelled with its keys and cardinality, possible joins dashed;
- the list, each table a row grouped by role with the joins routed in a gutter on its left;
- the Ossie file, editable as text. A save redraws the picture; a save that breaks the YAML or the schema leaves the last good model on screen and shows the error at its line.

Selecting a card or an edge dims what is not connected to it. The inspector shows a join's keys, grade and evidence with a preview of its joined rows on a sample, and a table's role, columns with their FineType types, what was folded into it and its failed values.

**Reshaping.** Each change is a verb in the keyboard registry, reached by key or the Space palette, and rewrites the Ossie file, the silver steps, and reruns the Protocol: keep a possible join (`y`), drop a join (`shift-d`), add one (`shift-a`, or drag one card onto another, which opens the picker with keys guessed from a shared column name), change a join's keys (`shift-c`), set a table's role (`r`), fold a reference into the table it labels (`z`) or unfold it (`shift-z`), leave a table out (`shift-x`) or bring it back (`i`), and retype a column (`shift-t`).

**Rerun and Rediscover.** After each run, and when the pane opens over a Protocol already run, each join's evidence is measured again on the silver tables, and a join the data no longer bears out carries a warning on its pill and in the inspector; the model is not restructured. Rediscover (`shift-r`) proposes afresh around the analyst's decisions.

**Discovery.** Proposing roles and joins runs in two passes: a quick one over a sample, whose model the pane opens on, and a full one on a worker, folded into the model as it stands when it lands, edits made in the meantime included. A pass that a newer one supersedes is stopped. A discoverer run as a child process writes its proposal to `.discovery/<pass>.ossie.yaml`, and Brightfield alone writes `model.ossie.yaml`. Until [dovetail](https://github.com/meridian-online/dovetail) is built in as that child, the discoverer returns the model already in the folder, or lists each table as unclear with no joins: an opened folder or workbook shows its tables with no proposed joins, and the analyst adds the joins by hand.

### Chart versions

Each chart file Brightfield saves, including each panel file, is recorded in arcform's local history (`~/.arcform/history`, or `$ARCFORM_HISTORY_DIR`): the text a Save replaces, the text it writes, and the text a window closes without saving. A panel's Versions list joins that store with `arcform-log.txt`, the log arc keeps in the Protocol's folder, which names each version, when it was made and by which route. The WHO column shows that route: Brightfield, arc (command line), or arc (agent tool) for arc's MCP server. The log travels with the folder and the store's text does not, and the store keeps only a file's newest versions, so a version the log names whose text the store does not hold is listed with its text marked as not kept on this machine, dimmed, and cannot be shown or stepped back to.

## Platform Support

- **macOS** — primary target (Metal via wgpu). Full support; the whole suite, pixel tier included, runs here.
- **Linux** — secondary target (Vulkan via wgpu). The stack supports it; the system-dependency matrix is not yet CI-validated.
- **Windows** — the egui/wgpu stack supports it in principle; unvalidated.

## Distribution

One native binary, no runtime dependencies beyond the OS graphics stack — no
server, no webview, no language runtime, and no network required to start,
render, or open a local protocol. A release artifact carries one more
executable: the official DuckDB v2.0.0 command-line shell (`engine/duckdb` in
the tarball, `Contents/Helpers/duckdb` in the app), which the Run control hands
a Protocol's SQL steps to so a run needs nothing installed.
`scripts/fetch-duckdb-cli.sh` pins it by sha256; `scripts/package.sh` stages it
when `BRIGHTFIELD_DUCKDB_CLI` names the directory that script wrote. A local
build may leave it out; a release may not. The release reads it back out of
both packaged artifacts with `scripts/check-artifact-engine.sh`, and fails the
job when it is missing, is not the pinned bytes, or is not the DuckDB the
binary links. On the native leg the packaged binary's `--check-engine` runs it.

Each tagged release is installed with `brew install meridian-online/tap/brightfield`, and the installed binary carries the headless exports (`export-pdf`, `export-png`, `export-data`) as well as the window.

```bash
# Package: release build → linked-library audit (OS allowlist) →
# brightfield-<version>-<target>.tar.gz + .sha256 under dist/
scripts/package.sh

# Prove the air-gapped claim against the artifact itself: a negative control
# (curl must FAIL inside the jail), then the *packaged* binary renders a chart
# and a Protocol manifest, and runs an agent-written Protocol and exports its
# report to PDF, with the network denied (sandbox-exec / unshare -rn)
scripts/verify-airgapped.sh dist/brightfield-*.tar.gz
```

The tarball contains the binary, LICENSE, a README and the self-contained
examples — everything its own verification opens. Tag pushes run the same
`package.sh` per target in CI (`.github/workflows/release.yml`) and lift the
results onto a GitHub release; the `<tool>-<version>-<target>.tar.gz` +
`.sha256` asset naming is the contract the `install.meridian.online`
convention consumes.

A release tag is `v` and three numbers, and it must match `version` in `crates/brightfield-shell/Cargo.toml` and have a section in [`CHANGELOG.md`](CHANGELOG.md). The release workflow's first job, `tag-guard`, refuses any other tag before anything is built, and publishes that section at the head of the release notes. `CHANGELOG.md` says how to cut a release.

## Technology Stack

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | Rust | Performance, memory safety, single-language stack |
| 2D scene renderer | Vello (via wgpu) | GPU-accelerated vector 2D; marks, axes, scales, legends drawn as a retained `Scene`; renders live and headless |
| Application shell | egui + eframe (+ egui_tiles, egui_table) | Window, docked panes, spec editor, and widget chrome; hosts and presents the Vello scene on a shared wgpu device |
| Query engine | DuckDB via duckdb-rs | Best-in-class analytical database, in-process, Arrow-native |
| Data transfer | Apache Arrow (arrow-rs) | Zero-copy columnar format, shared between DuckDB and renderer |
| Spec format | Mosaic spec (YAML/JSON) | Best available declarative grammar, portable, well-documented |
| YAML / JSON | serde + serde_yaml / serde_json | Standard Rust deserialisation |
| Geo support | DuckDB spatial extension | GeoJSON handling within the database |

## Success Criteria

- A Mosaic YAML spec defining a two-view cross-filtered dashboard over a large Parquet file stays fluid as the table grows: interaction latency independent of row count for the aggregating marks, where a pre-aggregated summary can stand in for the source. A row-level mark such as a raw scatter has no such summary, so its cost tracks the rows and this criterion does not claim it — [what makes an interaction fast](docs/interaction-speed.md) says which marks fall on which side of that line. That is a *property to demonstrate, not a figure to inherit* — Mosaic's published numbers were measured on Mosaic's coordinator, and this project quotes only numbers measured here. The measured record, with its machine, dataset and methodology, lives in [`benchmarks/`](benchmarks/); re-measure with `./scripts/bench-baseline.sh`. A CI gate (`scripts/check-borrowed-benchmarks.sh`) keeps upstream figures from being restated as ours.
- The same spec, unmodified, produces equivalent output to Mosaic's web rendering.
- The application is a single native binary with no runtime dependencies beyond system graphics drivers.

## Status

Early development. The project is in the discovery and design phase. The architecture favours vertical slices — a working end-to-end pipeline for a single chart type — over horizontal layers completed in isolation.

Feature cards and specifications are tracked centrally, outside this repository.

## Relationship to the Meridian Ecosystem

Brightfield is part of the [Meridian](https://meridian.online) project family, alongside [FineType](https://github.com/meridian-online/finetype) and [Arcform](https://github.com/meridian-online/arcform).

- **FineType** classifies and validates text data types, providing a transformation contract from raw text to typed DuckDB expressions.
- **Arcform** orchestrates local-first analytical pipelines — ingestion, validation, modelling, and export.
- **Brightfield** renders the resulting data interactively as native GPU-accelerated dashboards.

The three libraries are designed to compose: Arcform prepares data, FineType types it, and Brightfield visualises it.

### Working on the design system alongside brightfield

Brightfield's look — every colour, gap, radius, and the egui `Style`/`Visuals`/font
bridge — comes from the [Meridian design system](https://github.com/meridian-online/design),
consumed as two git dependencies: `meridian-design` (the framework-neutral tokens) and
`meridian-egui` (the egui emitter). Both are pinned in the crates that use them.

To iterate on a token or the theme bridge without a publish-and-bump round trip, clone the
design repo **beside** this one so the two sit side by side:

```
meridian-online/
  brightfield/      ← this repo
  design/           ← git clone of meridian-online/design
```

The workspace root `Cargo.toml` carries a `[patch."https://github.com/meridian-online/design"]`
section that redirects both crates to `../design/meridian-design` and `../design/meridian-egui`.
With the sibling clone present, a change there is picked up on the next `cargo build` here.
(`meridian-egui` is a newer crate; until it is published, the patch is also what makes it
resolve at all, so the sibling checkout is required to build a branch that depends on it.)

The fastest way to *see* a design-system change in context is the in-app gallery: run the
shell with `BRIGHTFIELD_GALLERY=1` and a **Gallery** tab joins the chart view, rendering
the shared component vocabulary — rows, pills, chips, the query line, the focus ring, the
icon set — inside the real window, under the live theme, each entry with a status pill so
unfinished coverage is visible rather than absent. Headlessly,

```sh
cargo run -p brightfield-shell --bin brightfield-shot -- \
  --gallery list-row --out list-row.png --theme dark
```

captures one component solo through the same deterministic pipeline the pixel tests use
(`--size WxH` is honoured on this path). The per-component conformance gate lives in
`crates/brightfield-shell/tests/gallery_gate.rs`.

### Regenerating the protocol frame published outside this repo

A published picture of Brightfield rendering the `edgar_gleif` Protocol — outline rail and
DAG, without the operator rail or the steps band — is a build output, not a screenshot taken
once by hand:

```sh
BRIGHTFIELD_PROTOCOL_OFFLINE=1 cargo run -p brightfield-shell --bin brightfield-shot -- \
  --spec examples/protocol/edgar_gleif/arcform.yaml --scale 1.0 --theme light \
  --crop 1285x815+0+0 --out brightfield-protocol-light.png
```

Swap `--theme dark` for the dark variant. `--crop WxH+X+Y` trims the capture after it is
rendered (see the flag's own doc comment in `src/bin/brightfield-shot.rs`); the pixels are
the same deterministic real-UI capture `--spec`/`--gallery` always produce, only the file on
disk is the named rectangle instead of the whole window, written at the smallest of the 18
PNG compression/filter combinations this crate's `image` dependency exposes (measured over
this picture; see [`write_png_smallest`](crates/brightfield-shell/src/capture.rs)'s own doc
comment for the numbers and why `NoFilter` wins here specifically). Running the command twice
on the same commit writes byte-identical files, down to the SHA-256 — the capture path has no
wall-clock or randomness in it (see `capture.rs`'s module doc), and the encoding is pinned to
one setting rather than left to a default that could move. That determinism is a property of
running *this* command, not of the PNG format: a file cut by a different tool (ImageMagick,
Pillow, `oxipng`) will not match these bytes even when its pixels are identical, because
different encoders produce different compressed streams for the same input. Diff decoded
pixels, not files, when comparing across tools.

### The arcform dependency (`arc`)

Brightfield **loads, validates and edits** `arcform.yaml` specs with the
[`arc` crate](https://github.com/meridian-online/arcform) — the same loader, the same
validators and the same format-preserving write path the `arc` binary itself uses. There is
deliberately **no brightfield-side copy of the spec schema**: two schemas drift, one cannot.
A spec this app accepts and a spec `arc run` accepts are the same thing by construction, and
edits made here preserve every byte they do not target (asserted by the hand-authored corpus
under `crates/brightfield-protocol/tests/corpus/`).

**What is pinned, and why.** `arc` is pre-1.0, so the root `Cargo.toml`'s
`[workspace.dependencies]` pins it by git **rev** (`arc = { git = …/arcform, rev = <sha>,
default-features = false }` — the default `cli` feature is a binary entry point that parses
the caller's argv, so it is switched off). `brightfield-protocol` and `brightfield-shell`
both inherit that one entry; the shell adds the `cli` feature to run a Protocol as a child
process. Treat every bump as potentially breaking and move deliberately.

**The sqlparser fork.** arc parses SQL with its own fork of sqlparser 0.55, which arcform publishes as `arcform-sqlparser`. arcform declares it as a path dependency inside its own repo, so Cargo.lock resolves it from the arc git rev and this workspace carries no `[patch.crates-io]` for it. The workspace's own newer `sqlparser` keeps coming from crates.io too. The upstream package is named `arcform`; the root entry says `package = "arcform"` so the crate stays `arc` in this workspace's source.

**To bump the pin**, move two things in lockstep, in one commit:

1. the `rev` on `arc` in the root `[workspace.dependencies]`;
2. `Cargo.lock` — run a build, and if the resolver pulls a transitive dep above the CI
   toolchain pin (`tree-sitter-iter` has done this: arc pins `yamlpath = "=1.27.0"`, but a
   fresh resolve can still float its deps), pin it back with
   `cargo update <crate>@<ver> --precise <ok-ver>`.

arc pins `duckdb` exactly, so a bump that moves arc's DuckDB moves the workspace `duckdb` pin and the CLI pin in `scripts/fetch-duckdb-cli.sh` with it, and the version named in the Distribution section above. While `.cargo/config.toml` defines `DUCKDB_VERSION` for the bundled build (libduckdb-sys 1.20000.0 otherwise names DuckDB 2.0.0 a development build), that value moves with them, and the entry goes once a libduckdb-sys release names its DuckDB correctly; `the_linked_duckdb_is_the_release_the_cli_is_pinned_to` fails while the two disagree. The packaging scripts read the CLI version from that pin, and `scripts/check-artifact-engine.sh` refuses a release whose pin and `libduckdb-sys` in `Cargo.lock` name different DuckDBs.

**What tells you the pin has gone stale.** Not a person. The pin drifted three weeks behind
arcform's operator catalog once, and the first signal was somebody opening a shipped Protocol
in the shell and reading `unknown operator 'text_embed' (not in the operator catalog)`.
`crates/brightfield-protocol/tests/sibling_manifests.rs` is the check that replaces that
discovery: it walks every `arcform.yaml` the sibling data repo ships and loads each through
the pinned `arc`, naming the manifest and quoting arc's own diagnostic when one is refused.
Its two tests are `#[ignore]`d because their input is not in this checkout — `test.yml`
checks the sibling repo out, sets `OPEN_ANALYTICS_DIR`, runs them with `--ignored`, and holds
libtest's executed count to a floor so a filter matching nothing cannot pass for coverage.
To run it locally against a sibling clone:

```
OPEN_ANALYTICS_DIR=../open-analytics \
  cargo test -p brightfield-protocol --test sibling_manifests -- --ignored --nocapture
```

Then run the whole suite: `crates/brightfield-protocol/tests/roundtrip.rs` is the canary for
write-path behaviour changes, and the protocol/render/shell fixtures are the canary for
validator tightening (arc's gate rejects what the old in-tree parser tolerated — an armless
step, an unknown operator, an incomplete `with:` block — so a stricter arc surfaces here as
fixture failures, which is the point).

**DuckDB note:** arc's engine links DuckDB even when only the spec library is consumed.
`brightfield-protocol` therefore enables `duckdb/bundled` (compiled from source) so the
workspace builds hermetically — no system `libduckdb`, no `DUCKDB_LIB_DIR`, on any machine
or CI runner.

## License

MIT — see [`LICENSE`](LICENSE)

## Contributing

Contributions welcome! Please open an issue or PR.

## Credits

Part of the [Meridian](https://meridian.online) project.

Built with [Vello](https://github.com/linebender/vello) (Linebender's GPU 2D scene renderer) on [wgpu](https://wgpu.rs/), hosted in [egui](https://github.com/emilk/egui)/[eframe](https://github.com/emilk/egui/tree/main/crates/eframe), with text shaped by [skrifa](https://github.com/googlefonts/fontations), [DuckDB](https://duckdb.org) via [duckdb-rs](https://crates.io/crates/duckdb), [Apache Arrow](https://arrow.apache.org/) via [arrow-rs](https://github.com/apache/arrow-rs), and [Serde](https://serde.rs).

Chart labels are rendered with [Inter](https://rsms.me/inter/), bundled (with its SIL Open Font License 1.1) via the Meridian design crate.

Spec format, grammar-of-graphics semantics, param/selection model, and query optimisation strategies derived from the [Mosaic](https://idl.uw.edu/mosaic/) project (UW IDL + CMU DIG — see [Mosaic TVCG'24](https://idl.uw.edu/papers/mosaic)).

Interaction and dashboard design informed by [Embedding Atlas](https://github.com/apple/embedding-atlas) (Apple, MIT licence) — its schema-inferred default dashboard, ghosted total-behind-filtered histograms, saved SQL predicates, and point-map-as-chart treatment. Design study only; no Embedding Atlas code is vendored here. See Ren, Hohman, Lin & Moritz, *Embedding Atlas: Low-Friction, Interactive Embedding Visualization*, [arXiv:2505.06386](https://arxiv.org/abs/2505.06386).
