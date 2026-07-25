# brightfield-conformance

The portability contract between Mosaic-web specs and brightfield's rendering.

This crate ships the machinery that makes conformance to Mosaic-web
provable: a preflight support report, `LoadDiagnostics`, two vendored
corpora, a deviation registry, a doc generator, and a layered conformance
runner.

Layer 1 (AST round-trip) gates both corpora. Layer 2 (data-source DDL
equivalence) gates the curated corpus. Layers 3 and 4 are **suppressed
against a written deviation record** rather than merely pending, because
what is missing there is an *oracle* — nothing yet diffs a rendered
brightfield scene, or a scripted interaction's resulting selection state,
against Mosaic web's for the same spec. Both the renderer and the
scriptable Interaction/Coordinator seam shipped long ago.

Two things settle a cell, in this order:

1. **Registry coverage.** A deviation naming this spec's filename in
   `affected_specs` and this layer in `conformance_layers_suppressed`
   makes the cell `Suppressed`, and the check does not run. A pair the
   registry does not name can never come back `Suppressed`, so every
   suppression traces to a reviewed record.
2. **The declared expectation.** A cell whose settled outcome differs from
   its `<name>.expected.yaml` becomes a `Fail` naming both sides. That is
   what makes the expectation an assertion rather than a note: a layer
   regressing from pass to pending reddens the run, and so does a
   suppressed layer quietly starting to pass while its deviation record
   still claims otherwise.

## `LoadDiagnostics` — what a spec load has to say

`LoadDiagnostics::collect` bundles the preflight walk's blocking entries
with every warning the parse and the analysis produced, worded per line.
It is what the shell puts in front of a person: the preflight mechanism
and the `ParseOutput.warnings` were both fully built and both reached
nobody — this crate was a dependency of no application crate, and all four
spec-load entry points dropped their warnings.

## `SupportReport` — preflight

`preflight(&Spec) -> SupportReport` walks a parsed spec and records every
component whose `ImplStatus` is `Planned` or `Unimplemented`. Walk order
is document order; repeated runs on the same spec produce a bytewise-
identical report.

```rust
use brightfield_conformance::{preflight, SupportReport};
use brightfield_spec::{parse_spec, Format};

let parsed = parse_spec(source, Format::Yaml)?;
let report: SupportReport = preflight(&parsed.spec);

if report.is_renderable() {
    // render path
} else {
    for entry in report.blocking() {
        // explain which components are unimplemented
    }
}
```

Two accessors drive the decision gate:

- `SupportReport::blocking()` — the subset with `status == Unimplemented`.
- `SupportReport::is_renderable()` — `true` iff `blocking()` is empty.

`Planned` components are advisory, not blocking.

## Corpora

Two corpora are addressed:

- **Curated** — 10 hand-picked specs under
  `crates/brightfield-conformance/vendor/curated/yaml/`. Each has a sibling
  `<name>.expected.yaml` declaring per-layer expectations. The curated set
  is the cross-layer gate; it's where layers 2–4 will activate first when
  their downstream infra lands.
- **Observed** — a path reference to `brightfield-spec`'s own vendored
  corpus (`crates/brightfield-spec/vendor/mosaic-specs/yaml/`). No sibling
  expectation files; layer-1 only. Observed is the totality check —
  "can we parse everything the upstream ships?"

### Growth rule

When Mosaic upstream ships new specs, bump the broader vendored corpus
in `brightfield-spec` first; the observed gate picks them up automatically.
Add a spec to the curated set only when cross-layer gating adds signal —
e.g. the spec is a deviation case, or it exercises a mark/interactor
class the existing curated set doesn't cover.

Curated READMEs record provenance: upstream commit SHA and a per-spec
rationale line. See `vendor/curated/README.md`.

## Deviation registry

`deviations.yaml` at the repo root is the single source of truth for
deliberate divergences from Mosaic-web rendering.

```yaml
deviations:
  - id: DEV-0001
    surface: rendering
    mosaic_behaviour: "<what Mosaic web does>"
    brightfield_behaviour: "<what brightfield does>"
    rationale: "<why the divergence exists>"
    affected_specs: [line.yaml, crossfilter.yaml]
    conformance_layers_suppressed: [2, 3, 4]
```

The loader enforces id uniqueness, `DEV-NNNN` format, layer range
(1..=4), and field completeness:

```rust
use brightfield_conformance::load_deviations;
use std::path::Path;

let registry = load_deviations(Path::new("deviations.yaml"))?;
for dev in registry.iter() {
    println!("{}: {}", dev.id, dev.rationale);
}
```

Cross-artefact integrity (`affected_specs` pointing at real curated
files; curated `<name>.expected.yaml` suppression ids resolving to real
registry entries) is enforced by the registry-integrity test, not the
loader.

### `generate-deviations` — DEVIATIONS.md doc generator

```
cargo run --bin generate-deviations
```

Reads `deviations.yaml` (default `./deviations.yaml`) and writes
`DEVIATIONS.md` (default `./DEVIATIONS.md`). Output is stable-sorted by
id and deterministic — two runs on unchanged input produce byte-identical
output. When the registry is empty, the output is a `(no deviations
registered)` stub.

The `dfconf_committed_deviations_md_is_current` integration test gates
drift: if `DEVIATIONS.md` stops matching the registry, CI fails until
someone re-runs the binary and commits the update.

## Conformance runner

The runner is exposed three ways, all dispatching through the same
`LayerCheck` trait:

### 1. `cargo test`

```
cargo test -p brightfield-conformance
```

Runs the layer-1 gate over curated + observed corpora, plus
registry-integrity and curated-preflight gates, as `#[test]` functions.

### 2. `conformance` binary

```
cargo run --bin conformance -- --layers 1,2,3,4 --corpus curated
```

This is a named CI step with its own pass/fail. Exit code is non-zero iff
any `LayerOutcome::Fail` is present — including a cell whose outcome
differs from the expectation its `.expected.yaml` declares.

Per-layer cell counts come first, because the roll-up alone cannot say
*which* layer is carrying the greens, and that is the only question worth
asking of a layered contract. Then the machine-readable footer:

```
LAYER 1: cells=10 passed=10 failed=0 suppressed=0  pending=0   layer 1: AST round-trip
LAYER 2: cells=10 passed=9  failed=0 suppressed=0  pending=1   layer 2: SQL equivalence
LAYER 3: cells=10 passed=0  failed=0 suppressed=10 pending=0   layer 3: encoding equivalence
LAYER 4: cells=10 passed=0  failed=0 suppressed=10 pending=0   layer 4: interaction equivalence
SUMMARY: cells=40 passed=19 failed=0 suppressed=20 pending=1
```

The one layer-2 pending is `legends.yaml`, which declares no data sources:
there is no data-source DDL for it to be equivalent about, and a green
cell earned by having nothing to check is not a green cell.

### 3. Library API

```rust
use brightfield_conformance::{
    run_conformance, ConformanceLayer, Corpus, DeviationRegistry,
};

let registry = DeviationRegistry::default();
let report = run_conformance(
    Corpus::Observed,
    &[ConformanceLayer::AstRoundTrip],
    &registry,
);
assert_eq!(report.summary.failed, 0);
```

## Non-goals

- **No encoding or interaction oracle.** Layers 3 and 4 make no judgement
  of their own; their cells are suppressed against `DEV-0001`, which
  states in writing what is unproven and why. Retiring it is per-spec:
  wire the oracle, drop that filename from `affected_specs`, and the run
  then judges the cell for real. The expectation assertion makes that safe
  in both directions, so neither a regression nor a silent improvement can
  pass unremarked.
- **No automatic `DEVIATIONS.md` regeneration in CI.** Manual
  `cargo run --bin generate-deviations` is sufficient for v1; the drift
  test catches out-of-date commits.
- **No JSON output format for the runner** — v1 emits plain text with
  the machine-readable `SUMMARY:` footer.
