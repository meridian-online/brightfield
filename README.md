# Brightfield

> **Early Release** — Brightfield is in early development. The project is in the discovery and design phase. Expect breaking changes to spec semantics, CLI arguments, library APIs, and rendering output between releases. Pin to a specific version if stability matters for your use case.

GPU-native desktop application for interactive data visualisation at any scale. Brightfield combines [Mosaic](https://idl.uw.edu/mosaic/)'s declarative specification grammar and coordinator architecture with [GPUI](https://www.gpui.rs/)'s GPU-accelerated rendering and [DuckDB](https://duckdb.org)'s analytical query engine — all implemented in Rust for a single-process, zero-serialisation-overhead experience.

The goal is a tool that can interactively visualise and explore datasets from thousands to billions of records with fluid, GPU-rendered interactions, without the performance ceiling of browser-based rendering or the overhead of a webview shell.

## Quick Start

Brightfield builds with a standard Rust toolchain (1.80+). From a clone of the repo, render the self-contained example spec:

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
- **GPU-native rendering.** GPUI provides GPU-accelerated rendering at up to 120 FPS via Metal (macOS) and Vulkan (Linux), with a hybrid immediate/retained mode model suited to interactive, frequently-updating visualisations.

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
│           Renderer (GPUI)                     │
│  Marks, axes, scales, legends, interactors    │
│  GPU-accelerated via Metal/Vulkan             │
│  Built on GPUI + gpui-plot foundations        │
└──────────────────────────────────────────────┘
```

**Spec parser.** Mosaic-compatible YAML or JSON specifications are parsed into a Rust AST mirroring Mosaic's `parseSpec()` output structure. Scope covers data definitions, param/selection declarations, plot/mark/encoding definitions, input widgets, layout, metadata, and config.

**Query engine.** A Rust equivalent of Mosaic's `mosaic-sql` package. Translates AST marks and encodings into DuckDB-dialect SQL, handling aggregates, window functions, bin calculations, and dynamic parameter substitution via Mosaic's `$param` expression syntax.

**Coordinator.** A Rust equivalent of Mosaic's `mosaic-core` package. Manages reactive params, first-class selections with resolution strategies, SQL-keyed result caching, automatic pre-aggregation, M4 downsampling, and priority queuing for interactive vs background queries. Data flows as Arrow record batches via `duckdb-rs`, staying in Rust memory throughout.

**Renderer.** Native rendering via GPUI, building on `gpui-plot` as a foundation. Marks, scales (linear/log/sqrt/band/ordinal/time/colour), axes, legends, interactors, layout primitives, and GPUI's built-in animation system for smooth transitions on data and selection updates.

## Platform Support

- **macOS** — primary target. GPUI uses Metal for rendering. Full support.
- **Linux** — secondary target. GPUI supports Vulkan. Full support expected.
- **Windows** — not currently supported by GPUI. Monitor upstream progress.

## Technology Stack

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | Rust | Performance, memory safety, single-language stack |
| UI framework | GPUI (from Zed) | GPU-accelerated, hybrid immediate/retained mode, 120 FPS target, Rust-native |
| Charting foundation | gpui-plot | Existing GPUI-native plotting with axes, zooming, panning |
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

Feature cards and specifications live under [`orbit/`](orbit/).

## Relationship to the Meridian Ecosystem

Brightfield is part of the [Meridian](https://meridian.online) project family, alongside [FineType](https://github.com/meridian-online/finetype) and [Arcform](https://github.com/meridian-online/arcform).

- **FineType** classifies and validates text data types, providing a transformation contract from raw text to typed DuckDB expressions.
- **Arcform** orchestrates local-first analytical pipelines — ingestion, validation, modelling, and export.
- **Brightfield** renders the resulting data interactively as native GPU-accelerated dashboards.

The three libraries are designed to compose: Arcform prepares data, FineType types it, and Brightfield visualises it.

## License

MIT — see [`LICENSE`](LICENSE)

## Contributing

Contributions welcome! Please open an issue or PR.

## Credits

Part of the [Meridian](https://meridian.online) project.

Built with [GPUI](https://www.gpui.rs/) (Zed's GPU-accelerated UI framework), [gpui-plot](https://crates.io/crates/gpui-plot) (native plotting foundation), [DuckDB](https://duckdb.org) via [duckdb-rs](https://crates.io/crates/duckdb), [Apache Arrow](https://arrow.apache.org/) via [arrow-rs](https://github.com/apache/arrow-rs), and [Serde](https://serde.rs).

Chart labels are rendered with [Inter](https://rsms.me/inter/), bundled (with its SIL Open Font License 1.1) via the Meridian design crate.

Spec format, grammar-of-graphics semantics, param/selection model, and query optimisation strategies derived from the [Mosaic](https://idl.uw.edu/mosaic/) project (UW IDL + CMU DIG — see [Mosaic TVCG'24](https://idl.uw.edu/papers/mosaic)).
