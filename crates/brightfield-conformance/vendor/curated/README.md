# Curated conformance corpus

A hand-picked subset of the Mosaic upstream spec corpus, chosen to gate
all four conformance layers on a stable, reviewable set. Each entry has
a sibling `<name>.expected.yaml` declaring per-layer expectations.

The broader unfiltered corpus lives at
`crates/brightfield-spec/vendor/mosaic-specs/yaml/` and is used only for
the layer-1 observed-corpus gate (`tests/observed_layer1.rs`).

## Upstream

- **Repo:** https://github.com/uwdata/mosaic
- **Commit SHA:** `d4d41a3275dbd6bc7995e1d1a82b0be18769bbca`
- **Date:** 2026-04-12
- **Path in upstream:** `docs/public/specs/yaml/`

(Provenance mirrors the parent vendored-corpus README at
`crates/brightfield-spec/vendor/mosaic-specs/README.md`.)

## Specs & selection rationale

- **line.yaml** — The simplest single-mark spec; baseline layer-1 check.
- **crossfilter.yaml** — Multi-chart cross-filter with linked selections; the canonical interaction equivalence fixture.
- **mark-types.yaml** — Deliberate breadth over mark vocabulary; surfaces unimplemented marks in preflight.
- **legends.yaml** — Exercises the legend component (discrete and continuous scales).
- **flights-200k.yaml** — Mid-size dataset with binned aggregates; stresses SQL equivalence.
- **overview-detail.yaml** — Concat layout with linked brush; layer-4 interaction case.
- **seattle-temp.yaml** — Time-axis encoding; layer-3 encoding equivalence focus.
- **facet-interval.yaml** — Faceted plot with interval interactor; covers multi-axis scales.
- **table.yaml** — Tabular output component — layer-3 encoding equivalence in a non-visual surface.
- **sorted-bars.yaml** — Bar mark with sort transform; confirms mark-data round-trip and a non-default sort order.

## Refresh procedure

1. Update the broader vendored corpus first (see
   `crates/brightfield-spec/vendor/mosaic-specs/README.md`).
2. `cp <updated spec>.yaml crates/brightfield-conformance/vendor/curated/yaml/`
3. Update **Commit SHA** and **Date** above to match the broader corpus.
4. Re-run `cargo test -p brightfield-conformance`.

Any regression is a real signal — either a vocabulary change in upstream
Mosaic (update `brightfield-spec`'s `src/vocab.rs`), or an unacknowledged
divergence (add a record to `deviations.yaml`).

## Licensing

Upstream Mosaic is BSD-3-Clause. The vendored YAML files are specifications
(data, not code) reproduced under the same licence terms for reference-corpus
testing purposes.
