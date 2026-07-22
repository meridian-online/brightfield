# Brightfield

> **Early Release** — Brightfield is in early development. The project is in the discovery and design phase. Expect breaking changes to spec semantics, CLI arguments, library APIs, and rendering output between releases. Pin to a specific version if stability matters for your use case.

GPU-native desktop application for interactive data visualisation at any scale. Brightfield combines [Mosaic](https://idl.uw.edu/mosaic/)'s declarative specification grammar and coordinator architecture with a [Vello](https://github.com/linebender/vello) GPU 2D scene renderer hosted in a [GPUI](https://www.gpui.rs/) application shell and [DuckDB](https://duckdb.org)'s analytical query engine — all implemented in Rust for a single-process, zero-serialisation-overhead experience.

The goal is a tool that can interactively visualise and explore datasets from thousands to billions of records with fluid, GPU-rendered interactions, without the performance ceiling of browser-based rendering or the overhead of a webview shell.

## Quick Start

Brightfield builds with a recent Rust toolchain (1.95+ — the floor set by the pinned GPUI and Vello 0.9). From a clone of the repo, render the self-contained example spec:

```sh
# macOS — opens a native, GPU-rendered window
cargo run -p brightfield-app -- examples/scatter.yaml

# Linux / headless / CI — render the chart straight to a PNG
BRIGHTFIELD_DUMP_PNG=scatter.png cargo run -p brightfield-app -- examples/scatter.yaml
```

`examples/scatter.yaml` is inline data — nothing external to download. Over 50 further Mosaic specs ship under [`crates/brightfield-spec/vendor/mosaic-specs/yaml/`](crates/brightfield-spec/vendor/mosaic-specs/yaml/); note that many read from Parquet/CSV files that are not bundled, so not all of them render out of the box yet.

The native window is currently macOS-only (it needs GPUI's Metal backend). On Linux and Windows, use `BRIGHTFIELD_DUMP_PNG=<path>` to render to an image — the headless render path is cross-platform. Live-window support on other platforms tracks GPUI's progress (see [Platform Support](#platform-support)).

## Design Principles

- **Spec-driven.** The Mosaic declarative specification (YAML/JSON) is the stable contract. Users author visualisations and dashboards as portable spec files. The rendering backend is an implementation detail.
- **Database-first.** All data-intensive computation — filtering, aggregation, binning, joins, window functions — is pushed to DuckDB. The renderer receives only the minimal data needed for display.
- **Single-process, in-memory.** DuckDB runs in-process via `duckdb-rs`. No HTTP server, no WebSocket layer, no serialisation boundary between the query engine and the coordinator. Data flows as Arrow record batches in shared memory.
- **GPU-native rendering.** Charts are drawn as [Vello](https://github.com/linebender/vello) 2D scenes rendered on the GPU via wgpu (Metal on macOS, Vulkan on Linux). GPUI hosts the application window and widget shell and presents the rendered scene, giving a hybrid immediate/retained model suited to interactive, frequently-updating visualisations.

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
│        Renderer (Vello scene + GPUI)          │
│  Marks, axes, scales, legends, interactors    │
│  Vello 2D scene, GPU-drawn via wgpu           │
│  (Metal/Vulkan); presented by the GPUI shell  │
└──────────────────────────────────────────────┘
```

**Spec parser.** Mosaic-compatible YAML or JSON specifications are parsed into a Rust AST mirroring Mosaic's `parseSpec()` output structure. Scope covers data definitions, param/selection declarations, plot/mark/encoding definitions, input widgets, layout, metadata, and config.

**Query engine.** A Rust equivalent of Mosaic's `mosaic-sql` package. Translates AST marks and encodings into DuckDB-dialect SQL, handling aggregates, window functions, bin calculations, and dynamic parameter substitution via Mosaic's `$param` expression syntax.

**Coordinator.** A Rust equivalent of Mosaic's `mosaic-core` package. Manages reactive params, first-class selections with resolution strategies, SQL-keyed result caching, automatic pre-aggregation, M4 downsampling, and priority queuing for interactive vs background queries. Data flows as Arrow record batches via `duckdb-rs`, staying in Rust memory throughout.

**Renderer.** A framework-free `brightfield-render` crate turns marks, scales (linear/log/sqrt/band/ordinal/time/colour), axes, legends, interactors, and layout primitives into a [Vello](https://github.com/linebender/vello) `Scene` — no GPUI dependency, so the same scene builder feeds both the live window and the headless PNG path. The GPUI shell (`brightfield-ui`) owns a dedicated wgpu device that rasterises the Vello scene and presents it, wiring pointer events into the interaction transitions and driving smooth transitions on data and selection updates. Text is shaped and rendered with [skrifa](https://github.com/googlefonts/fontations).

## Platform Support

- **macOS** — primary target. GPUI uses Metal for rendering. Full support.
- **Linux** — secondary target. GPUI supports Vulkan. Full support expected.
- **Windows** — not currently supported by GPUI. Monitor upstream progress.

## Technology Stack

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | Rust | Performance, memory safety, single-language stack |
| 2D scene renderer | Vello (via wgpu) | GPU-accelerated vector 2D; marks, axes, scales, legends drawn as a retained `Scene`; renders live and headless |
| Application shell | GPUI (from Zed) + gpui-component | Window, docked panels, spec editor, and widget shell; hosts and presents the Vello scene |
| Query engine | DuckDB via duckdb-rs | Best-in-class analytical database, in-process, Arrow-native |
| Data transfer | Apache Arrow (arrow-rs) | Zero-copy columnar format, shared between DuckDB and renderer |
| Spec format | Mosaic spec (YAML/JSON) | Best available declarative grammar, portable, well-documented |
| YAML / JSON | serde + serde_yaml / serde_json | Standard Rust deserialisation |
| Geo support | DuckDB spatial extension | GeoJSON handling within the database |

## Success Criteria

- A Mosaic YAML spec defining a two-view cross-filtered dashboard over a multi-million-row Parquet file renders interactively at 60+ FPS with sub-100ms filter response times.
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

Built with [Vello](https://github.com/linebender/vello) (Linebender's GPU 2D scene renderer) on [wgpu](https://wgpu.rs/), hosted in [GPUI](https://www.gpui.rs/) (Zed's GPU-accelerated UI framework), with text shaped by [skrifa](https://github.com/googlefonts/fontations), [DuckDB](https://duckdb.org) via [duckdb-rs](https://crates.io/crates/duckdb), [Apache Arrow](https://arrow.apache.org/) via [arrow-rs](https://github.com/apache/arrow-rs), and [Serde](https://serde.rs).

Chart labels are rendered with [Inter](https://rsms.me/inter/), bundled (with its SIL Open Font License 1.1) via the Meridian design crate.

Spec format, grammar-of-graphics semantics, param/selection model, and query optimisation strategies derived from the [Mosaic](https://idl.uw.edu/mosaic/) project (UW IDL + CMU DIG — see [Mosaic TVCG'24](https://idl.uw.edu/papers/mosaic)).
