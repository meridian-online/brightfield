# Brightfield

> **Early Release** — Brightfield is in early development. The project is in the discovery and design phase. Expect breaking changes to spec semantics, CLI arguments, library APIs, and rendering output between releases. Pin to a specific version if stability matters for your use case.

GPU-native desktop application for interactive data visualisation at any scale. Brightfield combines [Mosaic](https://idl.uw.edu/mosaic/)'s declarative specification grammar and coordinator architecture with a [Vello](https://github.com/linebender/vello) GPU 2D scene renderer hosted in an [egui](https://github.com/emilk/egui)/[eframe](https://github.com/emilk/egui/tree/main/crates/eframe) application shell (sharing one wgpu device) and [DuckDB](https://duckdb.org)'s analytical query engine — all implemented in Rust for a single-process, zero-serialisation-overhead experience.

The goal is a tool that can interactively visualise and explore datasets from thousands to billions of records with fluid, GPU-rendered interactions, without the performance ceiling of browser-based rendering or the overhead of a webview shell.

## Quick Start

Brightfield builds with a recent Rust toolchain (1.95+, matching the CI pin). From a clone of the repo, render the self-contained example spec:

```sh
# Opens the native, GPU-rendered window
cargo run -p brightfield-shell -- examples/scatter.yaml

# Headless / CI — render the composed chart straight to a PNG
cargo run -p brightfield-shell --bin brightfield-shot -- \
  --spec examples/scatter.yaml --out scatter.png --vello-only
```

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
- **Query optimisation** — automatic pre-aggregation at pixel resolution, M4 downsampling for line/area marks, result caching, priority queuing
- **Direct data loading** — Parquet, CSV, JSON, inline data, and DuckDB database files
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

## Platform Support

- **macOS** — primary target (Metal via wgpu). Full support; the whole suite, pixel tier included, runs here.
- **Linux** — secondary target (Vulkan via wgpu). The stack supports it; the system-dependency matrix is not yet CI-validated.
- **Windows** — the egui/wgpu stack supports it in principle; unvalidated.

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

- A Mosaic YAML spec defining a two-view cross-filtered dashboard over a large Parquet file stays fluid as the table grows: interaction latency roughly independent of row count. That is a *property to demonstrate, not a figure to inherit* — Mosaic's published numbers were measured on Mosaic's coordinator, and this project quotes only numbers measured here. The measured record, with its machine, dataset and methodology, lives in [`benchmarks/`](benchmarks/); re-measure with `./scripts/bench-baseline.sh`. A CI gate (`scripts/check-borrowed-benchmarks.sh`) keeps upstream figures from being restated as ours.
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

### The arcform dependency (`arc`)

Brightfield **loads, validates and edits** `arcform.yaml` specs with the
[`arc` crate](https://github.com/meridian-online/arcform) — the same loader, the same
validators and the same format-preserving write path the `arc` binary itself uses. There is
deliberately **no brightfield-side copy of the spec schema**: two schemas drift, one cannot.
A spec this app accepts and a spec `arc run` accepts are the same thing by construction, and
edits made here preserve every byte they do not target (asserted by the hand-authored corpus
under `crates/brightfield-protocol/tests/corpus/`).

**What is pinned, and why.** `arc` is pre-1.0, so `brightfield-protocol/Cargo.toml` pins it
by git **rev** (`arc = { git = …/arcform, rev = <sha>, default-features = false }` — the
default `cli` feature is a binary entry point that parses the caller's argv, so it is
switched off). Treat every bump as potentially breaking and move deliberately.

**The sqlparser patch mirror — do not skip this.** arc parses SQL with a **vendored fork of
sqlparser 0.55** (`vendor/sqlparser-0.55.0` in the arcform repo), wired up there via
`[patch.crates-io]`. Cargo patch tables **do not propagate through dependencies**, so this
workspace's root `Cargo.toml` carries the same patch itself — in **git form**, pointing at
the same repo and rev as the arc pin, so a bare CI checkout (no sibling `../arcform`)
resolves it. A path-form patch into a sibling checkout works locally and breaks CI. The
patch's package version is 0.55.0, so it applies only to arc's requirement; the workspace's
own newer `sqlparser` keeps coming from crates.io.

**To bump the pin**, move three things in lockstep, in one commit:

1. the `rev` on `arc` in `crates/brightfield-protocol/Cargo.toml`;
2. the `rev` on `sqlparser` in the root `[patch.crates-io]` (same sha);
3. `Cargo.lock` — run a build, and if the resolver pulls a transitive dep above the CI
   toolchain pin (`tree-sitter-iter` has done this: arc pins `yamlpath = "=1.27.0"`, but a
   fresh resolve can still float its deps), pin it back with
   `cargo update <crate>@<ver> --precise <ok-ver>`.

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
