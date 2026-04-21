# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** /Users/hugh/github/meridian-online/brightfield/orbit/specs/2026-04-20-mosaic-web-spec-portability/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 4 |
| 2 — Assumption & failure | Pass 1 found a constraint conflict plus content signals (cross-crate boundary with card 0001) | 3 |
| 3 — Adversarial | not triggered (only a single localised structural issue; no cascading failure mode) | — |

All three gate ACs (ac-14, ac-15, ac-16) pass the deterministic verification check — each `verification` field is non-empty, non-placeholder, and >=20 chars.

## Findings

### [HIGH] `LayerCheck::run` signature conflicts between constraint #6 and AC-06
**Category:** constraint-conflict
**Pass:** 1
**Description:** The trait signature is specified twice with different arities.
**Evidence:**
- Constraint #6 (spec.yaml line 20): `trait LayerCheck { fn layer(&self) -> ConformanceLayer; fn run(&self, spec: &Spec, fixture: &CorpusEntry) -> LayerOutcome; }` — two args after `&self`.
- AC-06 description (lines 115-117): `trait LayerCheck { fn layer(&self) -> ConformanceLayer; fn run(&self, spec: &Spec, fixture: &CorpusEntry, registry: &DeviationRegistry) -> LayerOutcome; }` — three args, adds `registry`.
- AC-12's `run_conformance` takes `registry: &DeviationRegistry` at the top level (line 235) and dispatches to `LayerCheck` impls; either the dispatcher holds the registry and consults it itself, or it threads it through. Both designs are defensible, but the spec must commit.
**Recommendation:** Pick one signature and update the other site. Threading `&DeviationRegistry` into `run` is consistent with the `LayerOutcome::Suppressed { deviation_id }` outcome (a layer check that needs to know "is this spec suppressed at my layer?" needs the registry) and with the dispatcher in AC-12. If that's preferred, update constraint #6 to the three-arg form. Otherwise, reshape AC-06 and AC-12 so the dispatcher consults the registry before/after calling a two-arg `run`.

### [MEDIUM] `Expectation` enum admits states that constraint #9 forbids per-layer
**Category:** constraint-conflict
**Pass:** 1
**Description:** Constraint #9 declares `<name>.expected.yaml` schema as `layer_1: pass | suppressed(DEV-ID)` — no `pending` state for layer 1. But constraint #10 defines `pub enum Expectation = Pass | Pending | Suppressed(String)` with no layer-awareness, so the type system permits `LayerExpectations { layer_1: Pending, ... }` which the YAML schema forbids. Whether the parser rejects `layer_1: pending` is unspecified.
**Evidence:** Lines 23 (constraint #9, curated layer_1 values) vs 25 (constraint #10, `Expectation` enum). The observed-corpus default in constraint #10 — `layer_1: Pass, layer_{2,3,4}: Pending` — keeps the happy path consistent, but a hand-written curated `.expected.yaml` with `layer_1: pending` has no declared failure mode.
**Recommendation:** Add a constraint (or amend #10) stating explicitly: "A curated `<name>.expected.yaml` with `layer_1: pending` is a `RegistryError::InvalidLayerExpectation { layer: 1 }` (or equivalent `ExpectationError`) at corpus-discovery time." Alternatively, split the type: `Layer1Expectation = Pass | Suppressed(String)` vs `LayerNExpectation = Pass | Pending | Suppressed(String)`. Either resolves the ambiguity and is verifiable.

### [MEDIUM] Ontology schema's `RegistryError` lists `UnknownAffectedSpec` but AC-10 excludes it
**Category:** constraint-conflict
**Pass:** 1
**Description:** The ontology schema (line 375) declares `RegistryError: Io | YamlSyntax | DuplicateId | InvalidLayer | InvalidIdFormat | MissingField | UnknownAffectedSpec`. AC-10 (lines 195-204) and constraint #11 (line 27) both explicitly state `UnknownAffectedSpec` is NOT a loader error — cross-artefact integrity is owned by AC-14.
**Evidence:** Ontology schema field (line 375) contradicts AC-10's explicit "NOT a loader error" clause.
**Recommendation:** Drop `UnknownAffectedSpec` from the ontology schema entry for `RegistryError` to match AC-10 and constraint #11. Minor doc fix, but the ontology schema is meant to be authoritative — mismatch here will confuse the implementer or cause a spurious grep/test assertion.

### [LOW] Observed-corpus path resolution relies on implicit `CARGO_MANIFEST_DIR` convention
**Category:** failure-mode
**Pass:** 2
**Description:** AC-08 says `OBSERVED_CORPUS` is a `&str` relative path from the crate root; the test "resolves relative to the crate manifest". The resolution mechanism is not specified. If the test uses `std::env::current_dir()` instead of `env!("CARGO_MANIFEST_DIR")`, invocations from the workspace root (e.g. `cargo test --workspace`) will fail to find the path. Cargo normally runs tests with CWD = crate dir, but this varies with workspace configurations and some IDE test runners.
**Evidence:** AC-08 description (lines 155-159) and verification (lines 161-165) don't pin the resolution primitive.
**Recommendation:** Add to AC-08: "the path is resolved relative to `env!(\"CARGO_MANIFEST_DIR\")` at compile time (or `CARGO_MANIFEST_DIR` env var at runtime), not process CWD". One line.

### [LOW] AC-07 file-count bound is tested by shell globbing; fragile to stray files
**Category:** test-gap
**Pass:** 2
**Description:** AC-07 verification (lines 145-150) relies on `ls crates/.../vendor/curated/yaml/*.yaml` listing 10-12 files. A stray `.yaml.bak`, `.orig`, editor swap file, or `.expected.yaml` sibling matched by a wider glob could throw the count off. The current glob `*.yaml` would also match `foo.expected.yaml` if someone's expected-fixture loader changes filename convention — creating a confusing failure where the curated count swells.
**Evidence:** AC-07 verification uses a raw glob on `*.yaml` in a directory that also contains `<name>.expected.yaml` siblings per constraint #9.
**Recommendation:** Change the constraint/AC to require curated specs live under `vendor/curated/yaml/` and expected fixtures under `vendor/curated/expected/` (sibling dirs, not sibling files), OR tighten the AC-07 verification to exclude `*.expected.yaml` via explicit glob (`ls vendor/curated/yaml/*.yaml | grep -v '\.expected\.yaml$'`). Belt-and-braces: keep the integrity check in AC-14 as-is.

### [LOW] AC-11 "committed DEVIATIONS.md is current" test assumes clean determinism across hosts
**Category:** failure-mode
**Pass:** 2
**Description:** AC-11's `dfconf_committed_deviations_md_is_current` diffs a freshly-generated `DEVIATIONS.md` against the committed one for byte-equality. This is sound on Unix; on Windows, line endings (CRLF vs LF) or path-separator echoes could break byte-equality. The spec says nothing about target platforms. If brightfield is Unix-only this is a non-issue.
**Evidence:** AC-11 verification (lines 221-230) asserts byte-equality after a subprocess invocation.
**Recommendation:** Either declare platform scope ("Unix-only for v1") explicitly in constraints, or add to AC-11: "the binary writes output with LF line endings on all platforms and normalises any host-sourced paths". Low priority if platform scope is already understood. Either way, the integration test should set an explicit output path via `--output` to avoid CWD-relative differences.

### [LOW] `Deviation` struct shape is defined only in the YAML-schema constraint, not as a Rust type
**Category:** missing-requirement
**Pass:** 2
**Description:** Constraint #10 specifies the on-disk YAML schema for deviation records. The ontology schema (line 370-372) mentions `Deviation { id, surface, mosaic_behaviour, ... }` as a struct. No AC requires the Rust struct exists with those fields — AC-09 exercises `DeviationRegistry { entries: IndexMap<String, Deviation> }` and the loader, but never asserts the `Deviation` type's public shape. A perverse implementer could hide fields behind accessors or use a sub-struct-of-structs split and pass every AC test as written.
**Evidence:** AC-09 verification covers only load errors; AC-14's integrity test reads `affected_specs` and `id` through the registry but doesn't assert `Deviation` is `pub struct Deviation { pub id: String, pub surface: String, ... }`.
**Recommendation:** Add one line to AC-09 (or a short AC-09a): "`pub struct Deviation { pub id: String, pub surface: String, pub mosaic_behaviour: String, pub brightfield_behaviour: String, pub rationale: String, pub affected_specs: Vec<String>, pub conformance_layers_suppressed: Vec<ConformanceLayer> }`" and add a `grep -Rq 'pub struct Deviation'` verification. Closes the "struct shape is only implicit" gap at near-zero cost.

---

## Honest Assessment

The spec is carefully constructed and unusually coherent — the layered-equivalence model is clearly thought through, the honest-scaffolding principle (layers 2-4 return `Pending`, not fake-green) is encoded in the type system, and the registry-integrity gate plus the deterministic `generate-deviations` binary together close the "docs vs tests drift" risk cleanly. The cross-card reach into card 0001 is bounded to four well-named constraints and matches what card 0001's shipped spec exposes. The biggest concrete risk is the `LayerCheck::run` arity conflict (HIGH finding above): two places in the spec declare the trait differently, and whichever one the implementer follows first will force rework in the other. That's a cheap fix but has to happen before implementation. The other findings are tightening work — per-layer expectation validity, ontology-schema alignment with AC-10, path-resolution primitive for the observed corpus, and an explicit `Deviation` struct AC — none of which block the plan but all of which will save a review cycle at PR time. Fix the HIGH, nudge the MEDIUMs, and this is ready to implement.
