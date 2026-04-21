# Deviations

Deliberate divergences from Mosaic-web rendering. Generated from
`deviations.yaml` by `cargo run --bin generate-deviations`.

## DEV-0001 — rendering

**Mosaic behaviour.** Mosaic web renders plot/concat layouts, marks (dot, line, rectY, rule,
frame, etc.), legends, and interactors (intervalX, intervalXY,
highlight, toggle, nearest) against a live DuckDB session.


**Brightfield behaviour.** v1 ships the spec-portability contract only — the AST parser, preflight
SupportReport, deviation registry, and layer-1 AST round-trip gate. No
layout, mark, legend, interactor, or input is rendered yet; every
`ImplStatus` in `brightfield-spec`'s vocabulary registry is `Unimplemented`
pending the renderer card.


**Rationale.** Honest scaffolding: layers 2 (SQL equivalence), 3 (encoding
equivalence), and 4 (interaction equivalence) return
`LayerOutcome::Pending` rather than fake-green. Flipping an individual
`LayerCheck` to Pass/Fail is what unblocks each curated spec; this
deviation is the accounting surface that keeps the preflight gate
honest until then.


**Affected specs:** crossfilter.yaml, facet-interval.yaml, flights-200k.yaml, legends.yaml, line.yaml, mark-types.yaml, overview-detail.yaml, seattle-temp.yaml, sorted-bars.yaml, table.yaml

**Conformance layers suppressed:** 2, 3, 4

