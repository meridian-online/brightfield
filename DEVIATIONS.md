# Deviations

Deliberate divergences from Mosaic-web rendering. Generated from
`deviations.yaml` by `cargo run --bin generate-deviations`.

## DEV-0001 — rendering

**Mosaic behaviour.** Mosaic web renders plot/concat layouts, marks (dot, line, rectY, rule,
frame, etc.), legends, and interactors (intervalX, intervalXY,
highlight, toggle, nearest) against a live DuckDB session.


**Brightfield behaviour.** The renderer and the interaction seam both ship. Marks, axes, scales,
legends and interactors are drawn as a Vello 2D scene by the
framework-free `brightfield-render` crate and presented both live
(egui/wgpu shell) and headless (PNG); an interaction resolves to a
pushed predicate and a re-query through the Interaction/Coordinator
seam, which headless tests script today. The layered conformance runner
GATES layers 1 (AST round-trip) and 2 (data-source SQL/DDL equivalence):
layer 1 passes across all 10 curated specs, layer 2 across the 9 that
declare data sources. Layers 3 (visual-encoding equivalence) and 4
(interaction equivalence) are accounted for HERE — this record names all
10 curated specs at both layers, so the runner reports those 20 cells as
`LayerOutcome::Suppressed` against this id rather than leaving them to a
pending string nobody has to keep true.


**Rationale.** Conformance is only as strong as its oracle, and what is missing at
layers 3 and 4 is the ORACLE, not the capability. Nothing yet diffs a
rendered brightfield scene's mark/scale/channel structure against Mosaic
web's, and nothing yet holds a scripted interaction's resulting
selection state against Mosaic web's for the same events. Until
something does, calling those cells green would be a lie and leaving
them merely pending would make them invisible — so they are suppressed
against this written record, which is the accounting surface. Retiring
it on purpose is per-spec and MANUAL: wire the oracle, drop that
filename from `affected_specs`, flip the layer in the spec's
`.expected.yaml`, and the run judges the cell for real. Retiring it
involuntarily is the runner's job — the check runs even for a
suppressed pair, and a check that PASSES where this record claims a
divergence fails the cell as a stale deviation. So a spec whose layer
starts passing while still listed here does redden the run; what this
record cannot tell you is the difference between a layer that is still
broken and one nobody has built an oracle for, because both come back
not-passing.


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

## DEV-0005 — scales — positional domain pinning (`Domain: Fixed`)

**Mosaic behaviour.** Mosaic accepts `Fixed` at `xDomain`, `yDomain`, the `xyDomain`
both-axes shorthand, and the facet axes `fxDomain` / `fyDomain`, at a
plot and under `plotDefaults`. The domain is fixed after the first
render, on whatever data the marks then hold, and later filtering
leaves it where it is.


**Brightfield behaviour.** `Fixed` is read from a plot's own `xDomain` and `yDomain`, and the
capture moment is Mosaic's: the pin is taken from the scales the
plot's FIRST composition drew against, so a plot whose first render is
already filtered pins the filtered domain. The pin holds the domain
through a filter, a selection and a re-query, and a band scale keeps
its category ORDER, so a filtered-away category keeps its slot rather
than closing the gap.

Four positions are NOT read, and a spec using one gets a domain
inferred from the rows it is currently drawing: the `xyDomain`
shorthand, the facet axes `fxDomain` / `fyDomain`, `Fixed` written
under `plotDefaults`, and any spelling other than the exact string
`Fixed`. An explicit two-element domain (`xDomain: [0, 100]`) is a
different instruction and is likewise not read here.

One brightfield-local rule sits on top: pan and zoom are offered on
any plot with a continuous positional scale, and an axis the reader
has navigated is drawn at the navigated extent rather than the pin,
until the navigation is reset.


**Rationale.** The pin is a portability instruction, so the capture moment is copied
rather than improved on: resolving an unfiltered extent instead would
make a spec render differently here than in Mosaic, which is the one
thing `Fixed` exists to prevent.

The unread positions are scoped by what the renderer can act on rather
than by what the parser accepts. Facets are not rendered, so `fxDomain`
/ `fyDomain` name axes that do not exist here; `plotDefaults` is parsed
and round-tripped but applied to no plot, so honouring one key from it
would make that block half-live and hide the rest.

Navigation takes precedence because the two instructions come from
different people about different events. `Fixed` is the author saying
the frame must not move when the DASHBOARD moves; a pan is the reader
moving the frame on purpose. A pin that outranked the gesture would
make a plot silently refuse to pan.


**Affected specs:** facet-interval.yaml

**Conformance layers suppressed:** 3

