# brightfield-conformance

The portability contract between Mosaic-web specs and brightfield's rendering.

This crate ships the *machinery* that makes conformance to Mosaic-web
provable: a preflight support report, two vendored corpora, a deviation
registry, a doc generator, and a layered conformance runner. V1 wires the
trait seams and gates layer 1 (AST round-trip) over both corpora. Layers
2–4 are scaffolded behind the same `LayerCheck` trait so that adding the
SQL emitter, renderer, and event pump in later cards wires up without
rework.

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

Exit code is non-zero iff any `LayerOutcome::Fail` is present. The footer
line has a machine-readable summary:

```
SUMMARY: passed=N failed=N suppressed=N pending=N
```

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

## v1 non-goals

- **Layers 2–4 return `Pending`**, not fake-green. The concrete
  `SqlEquivalenceCheck`, `EncodingEquivalenceCheck`, and
  `InteractionEquivalenceCheck` impls each return
  `LayerOutcome::Pending { reason }` naming the missing downstream
  ("SQL emitter not yet available", "renderer not yet available",
  "event pump not yet available"). Flipping any of these from `Pending`
  to real `LayerCheck` logic requires a later card. This is deliberate:
  an honest `Pending` today becomes a failing test the moment its
  concrete impl goes in.
- **No automatic `DEVIATIONS.md` regeneration in CI.** Manual
  `cargo run --bin generate-deviations` is sufficient for v1; the drift
  test catches out-of-date commits.
- **No JSON output format for the runner** — v1 emits plain text with
  the machine-readable `SUMMARY:` footer.
