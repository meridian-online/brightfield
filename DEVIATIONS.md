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

**Conformance layers suppressed:** 3, 4

## DEV-0002 — data source — DuckDB attach

**Mosaic behaviour.** Mosaic web uses ATTACH without a read-only flag, allowing both
read and write access to the attached DuckDB database file.


**Brightfield behaviour.** brightfield emits ATTACH '<path>' AS "<alias>" (READ_ONLY),
enforcing read-only access to prevent accidental corruption of
the user's production database during exploration.


**Rationale.** Exploration safety: the card's scope is read-only analysis. A
user pointing brightfield at a production .duckdb file should not
risk inadvertent writes. The READ_ONLY flag is a deliberate
safety divergence from Mosaic-web's wire shape.


**Conformance layers suppressed:** 2

## DEV-0003 — colour scale — sequential scheme default

**Mosaic behaviour.** Mosaic/Observable Plot default an unspecified quantitative colour
scale's scheme to `turbo` (a rainbow map).


**Brightfield behaviour.** brightfield defaults the sequential colour scheme to `viridis`.
`turbo` remains available by name (`colorScheme: turbo`), so a spec
that names a scheme renders it; only the *unspecified* default differs.


**Rationale.** Perceptual uniformity and colourblind safety: viridis is perceptually
uniform and colourblind-safe, whereas turbo is a rainbow map with known
perceptual artefacts at the extremes. Viridis is the de-facto modern
default (matplotlib, ggplot). Spec portability is preserved — a
`colorScheme: turbo` spec still renders turbo; only the default diverges.


**Conformance layers suppressed:** 3

## DEV-0004 — colour — default categorical palette / default mark colour

**Mosaic behaviour.** Mosaic/Observable Plot default the categorical colour scheme to
observable10 (first slot #4269d0) and an unencoded mark's colour to
"currentColor" resolving in practice to the observable10/Tableau10
steel blue; there is no "meridian" scheme name.


**Brightfield behaviour.** brightfield's default categorical palette is the Meridian design
system's "Harbour" order (8 slots, first slot blue #0083c4) and the
default single-mark colour is Harbour slot 1. `colorScheme: meridian`
is Brightfield-local sugar for the Meridian sequential ramp (13
blue-240 stops); the sequential DEFAULT remains viridis (DEV-0003).
Explicit `colorDomain`/`colorRange` literals are honoured, and
`serialise_spec` expands `colorScheme: meridian` into explicit
`colorRange` stops on export, so exported specs stay
vanilla-Mosaic-portable.


**Rationale.** Design-system adoption (Meridian phase 4): Harbour's slot ORDER is a
colourblind-safety mechanism (chosen for maximum adjacent CVD
distance) and its colours are tuned to the warm Meridian chart
surface. Portability is preserved the same way DEV-0003 preserves it:
a spec that names a portable scheme renders it; only renderer DEFAULTS
and a Brightfield-local scheme name (expanded to explicit colours on
export) diverge.


**Conformance layers suppressed:** 3

