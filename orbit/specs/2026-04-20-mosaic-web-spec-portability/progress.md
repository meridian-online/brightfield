# Progress — Mosaic web spec portability (card 0002)

Tracking checklist for implementation against
`orbit/specs/2026-04-20-mosaic-web-spec-portability/spec.yaml`.

## Acceptance criteria

- [x] ac-01 — new crate `brightfield-conformance` at `crates/brightfield-conformance/` with correct deps (no miette); added to workspace members
- [x] ac-02 — `SupportReport` + `SupportEntry` + `ComponentIdentity` + `Surface` public types
- [x] ac-03 — `preflight(&Spec) -> SupportReport` in document order, deterministic
- [x] ac-04 — `SupportReport::blocking()` and `is_renderable()` decision gate
- [x] ac-05 — `ConformanceLayer` sealed enum with `#[repr(u8)]` discriminants 1..=4, `TryFrom<u8>`, `Display`
- [x] ac-06 — `LayerCheck` trait + `LayerOutcome` enum + four concrete impls (AstRoundTripCheck + three Pending impls)
- [x] ac-07 — 10 curated specs vendored under `vendor/curated/yaml/` with sibling `.expected.yaml` files + README with upstream SHA
- [x] ac-08 — `OBSERVED_CORPUS` const resolved via `CARGO_MANIFEST_DIR`; `observed_specs()` enumerator
- [x] ac-09 — `deviations.yaml` at repo root; `Deviation` struct with all fields `pub`; `DeviationRegistry`; `load_deviations`
- [x] ac-10 — `RegistryError` thiserror enum with all variants; `Display` names id
- [x] ac-11 — `generate-deviations` binary with deterministic LF output, empty-registry stub, drift test
- [x] ac-12 — `run_conformance(Corpus, &[ConformanceLayer], &DeviationRegistry) -> ConformanceReport`
- [x] ac-13 — `conformance` binary with `--layers` + `--corpus` flags, `SUMMARY:` footer, non-zero exit on failure
- [x] ac-14 — Registry-integrity gate (bidirectional check)
- [x] ac-15 — Observed corpus layer-1 gate passes for every vendored spec
- [x] ac-16 — Curated corpus preflight gate (every blocking entry is accounted for by a deviation)
- [x] ac-17 — Crate README documents all five sections; `DEVIATIONS.md` at repo root

## Exit conditions

- [x] `cargo test -p brightfield-conformance` passes (33 lib + 8 integration = 41 tests, all green)
- [x] `cargo run --bin conformance -- --layers 1 --corpus curated` exits 0 with `failed=0`
- [x] `cargo run --bin conformance -- --layers 1 --corpus observed` exits 0 with `failed=0`
- [x] `cargo run --bin generate-deviations` produces `DEVIATIONS.md` deterministically
- [x] `RUSTFLAGS="-D warnings" cargo check -p brightfield-conformance --all-targets` compiles without warnings

## Notes

- Card 0001's vocab marks every component `Unimplemented`, so the curated
  preflight gate (ac-16) is satisfied via a single bootstrap deviation
  (DEV-0001) whose `affected_specs` list covers all 10 curated filenames
  with layers 2–4 suppressed. This is expected honest scaffolding —
  individual deviations will replace the bootstrap as the renderer card
  flips specific `ImplStatus` values to `Implemented`.
- `Layer1Expectation` has no `Pending` variant — layer 1 machinery ships
  in v1, so `layer_1: pending` in a curated `<name>.expected.yaml` is
  caught at discovery time as `ExpectationError::InvalidLayer1Pending`.
- `LayerCheck::run` takes three args `(&Spec, &CorpusEntry, &DeviationRegistry)`.
  The registry is threaded through because a future concrete impl may
  need to resolve suppression against it during its own dispatch (e.g.
  the SQL-equivalence check consulting whether an expected divergence at
  its layer is suppressed).
