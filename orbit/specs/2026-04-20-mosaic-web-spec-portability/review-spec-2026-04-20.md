# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** /Users/hugh/github/meridian-online/brightfield/orbit/specs/2026-04-20-mosaic-web-spec-portability/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by                                                         | Findings |
|------|----------------------------------------------------------------------|----------|
| 1    | always                                                               | 2        |
| 2    | content signals (shared repo-root config, cross-crate path coupling, | 4        |
|      |   CI gate plumbing) + 1 MEDIUM Pass-1 finding                        |          |
| 3    | not triggered (no structural / cascade concerns in Pass 2)           | —        |
```

## Findings

### [MEDIUM] `load_deviations` signature cannot enforce `UnknownAffectedSpec`
**Category:** constraint-conflict
**Pass:** 1
**Description:** Constraint #10 and AC-09 state that `load_deviations(path: &Path) -> Result<DeviationRegistry, RegistryError>` enforces "affected-spec existence in curated corpus" and returns `RegistryError::UnknownAffectedSpec { id, spec }` when an entry references a curated filename that does not exist. The signature takes only a path to `deviations.yaml` — it has no input describing the curated corpus's file list, so it cannot evaluate the "spec must exist in curated" predicate deterministically from its arguments alone.

Either (a) the loader needs an additional parameter (e.g. `curated_root: &Path` or `curated_files: &HashSet<String>`), (b) the loader resolves the curated path via a module constant and performs filesystem I/O at load time (which couples library correctness to filesystem layout and complicates testing), or (c) `UnknownAffectedSpec` is promoted out of `load_deviations` into the AC-14 registry-integrity gate only (and removed from the loader's error taxonomy in AC-09 / constraint #11).

**Evidence:**
- spec.yaml line 24 (constraint #10): "Loader: `fn load_deviations(path: &Path) -> Result<DeviationRegistry, RegistryError>`."
- spec.yaml line 25 (constraint #11): `UnknownAffectedSpec { id: String, spec: String }` listed as a `RegistryError` variant with the comment "affected spec must exist in curated corpus".
- spec.yaml AC-09 verification line 184: `dfconf_load_rejects_unknown_affected_spec` "feeds a malformed fixture and assert the matching `RegistryError` variant." The fixture-based test implies the loader surfaces this error, but the loader has no way to know what "malformed" means for this class without corpus knowledge.
- spec.yaml AC-14 (registry-integrity gate) already asserts the same bidirectional property at a point where both the registry and the corpus are in scope — suggesting option (c) is the cleanest fix.

**Recommendation:** Resolve the signature/semantics mismatch. Pick one:
1. Extend the signature: `fn load_deviations(path: &Path, curated: &CuratedCorpus) -> Result<DeviationRegistry, RegistryError>` and update AC-09's verification to match.
2. Drop `UnknownAffectedSpec` from `load_deviations`; rely on AC-14's integrity gate to catch it. Update constraint #11 and AC-09 accordingly.

Option 2 is preferable: it keeps `load_deviations` a pure file-level validator (syntax, id format, id uniqueness, layer range, field completeness) and makes the cross-artefact check live exactly once, in the integrity test that already owns it.

---

### [MEDIUM] `CorpusEntry` type is referenced but never defined
**Category:** missing-requirement
**Pass:** 1
**Description:** Constraint #6 and AC-06 define the seam trait as `trait LayerCheck { fn layer(&self) -> ConformanceLayer; fn run(&self, spec: &Spec, fixture: &CorpusEntry, registry: &DeviationRegistry) -> LayerOutcome; }`. The type `CorpusEntry` is used in two signatures but never structurally defined — no constraint lists its fields, no AC requires it to exist as a named type, and the ontology schema doesn't list it. AC-07 describes the curated corpus layout on disk (yaml + expected.yaml siblings, README) but never defines the in-memory representation that `LayerCheck::run` receives.

This is a missing-requirement finding rather than a pure naming gap because the expected-fixture plumbing (layer expectations per curated spec, suppression IDs) has to live somewhere, and `CorpusEntry` is the obvious home. Without defining it, downstream implementers will invent the shape, and the registry-integrity test in AC-14 depends on the `expected.yaml` schema being structurally accessible.

**Evidence:**
- spec.yaml constraint #6 (line 20): `fn run(&self, spec: &Spec, fixture: &CorpusEntry) -> LayerOutcome;` — `CorpusEntry` used, not defined.
- spec.yaml AC-06 (line 113): same trait signature with the same undefined `CorpusEntry`.
- spec.yaml AC-07 specifies the on-disk layout only.
- spec.yaml ontology_schema lists `Corpus = Curated | Observed` but not `CorpusEntry`.
- AC-14's integrity test needs to read the per-spec `expected.yaml` contents; that data clearly belongs on `CorpusEntry`, but the spec never says so.

**Recommendation:** Add either a constraint or a new AC defining `CorpusEntry`. Minimum fields implied by current constraints:
```
CorpusEntry {
  name: String,                 // filename stem, e.g. "crossfilter"
  source_path: PathBuf,         // path to the .yaml
  expectations: LayerExpectations, // the parsed <name>.expected.yaml
}
LayerExpectations {
  layer_1: Expectation,
  layer_2: Expectation,
  layer_3: Expectation,
  layer_4: Expectation,
}
Expectation = Pass | Pending | Suppressed(String) // deviation id
```
And add the type to the ontology schema. Bonus: add a loader constraint for `<name>.expected.yaml` ("parsed at corpus-discovery time; missing or malformed expectations file is a hard error, not Pending") so AC-14's test has a structural footing.

---

### [LOW] Observed corpus path coupling is fragile but unacknowledged
**Category:** assumption
**Pass:** 2
**Description:** AC-08 defines `OBSERVED_CORPUS` as a relative path const into `brightfield-spec`'s vendored corpus (`../brightfield-spec/vendor/mosaic-specs/yaml/`). This works in the current workspace layout but couples `brightfield-conformance` to both the physical directory layout of the monorepo and the invariant that `brightfield-spec` will always be a sibling crate that vendors this corpus at this exact sub-path. Neither is guaranteed by a constraint or an AC: card 0001's spec.yaml constraint line 32 pins the vendor location but doesn't commit to permanence.

The verification ("asserts the directory exists and contains ≥54 `.yaml` files") will silently succeed the moment the corpus size dips below its current count without any upstream revendor — the `≥54` floor is correct today but brittle if card 0001 ever re-pins to a smaller upstream slice. The threshold is also a hand-wavy number: it matches the current count of canonical Mosaic specs but isn't sourced from a single authority.

**Evidence:**
- spec.yaml constraint #8 (line 22): "observed/ — a symbolic reference to brightfield-spec's own vendored corpus".
- spec.yaml AC-08 verification (line 160): "contains ≥54 `.yaml` files (the minimum corpus size card 0001 vendored)".
- card 0001 spec.yaml constraint at line 32: vendor path declared but no cross-crate stability guarantee.

**Recommendation:** Either (a) downgrade the floor to a non-numeric check ("directory exists and contains ≥1 `.yaml` file") so it's resilient to upstream revendors, (b) derive the floor from a shared constant both crates import, or (c) add an explicit constraint to card 0001 committing that the vendor path is stable and accept the coupling as documented. Option (a) is lowest-friction.

---

### [LOW] `DEVIATIONS.md` drift is possible — not caught by any gate
**Category:** test-gap
**Pass:** 2
**Description:** The spec declares (non-goal, constraint #14) that automatic CI regeneration of `DEVIATIONS.md` is out of scope for v1; `generate-deviations` is run manually. AC-17 requires the file to exist at repo root but does not require it to match the current `deviations.yaml`. Combined with AC-11's determinism guarantee, this means a human workflow is the only thing keeping the generated doc in sync with the registry. Drift is plausible in practice (someone adds a deviation record, forgets to run the binary).

This is a deliberate v1 trade-off (non-goal stated explicitly) and the loss is only user-facing doc accuracy, not engine correctness — so low severity. But the spec doesn't acknowledge the drift risk or suggest a mitigation path (e.g. a commit hook, a nightly CI task, or a post-v1 follow-up card).

**Evidence:**
- spec.yaml constraint #14 (line 32): "generating `DEVIATIONS.md` automatically in CI (manual `cargo run --bin generate-deviations` is sufficient for v1)".
- spec.yaml AC-17 (line 318): `test -f DEVIATIONS.md` — existence-only check, no up-to-date-ness check.
- spec.yaml AC-11 determinism test guarantees the binary is idempotent — so a comparison-based drift check would be trivial to add later.

**Recommendation:** Either (a) add a lightweight drift-check integration test — `tests/generated_md_is_current.rs` runs `generate-deviations` to a tempfile and diffs against committed `DEVIATIONS.md`; fails if they differ. This costs ~10 lines and closes the loop for zero CI infra work. Or (b) explicitly note the drift risk in the README (AC-17.e non-goals section) so future maintainers know the commitment. Option (a) is the stronger move and fits the spec's overall "deterministic + machine-checked" ethos; option (b) is the minimum acceptable.

---

### [LOW] `Pending::reason` type disagreement between constraint and AC
**Category:** constraint-conflict
**Pass:** 2
**Description:** Constraint #7 defines `LayerOutcome::Pending { reason: String }`; AC-06 defines the same variant as `Pending { reason: &'static str }`. These are different types — `String` is heap-allocated and owned, `&'static str` is a string slice with program-lifetime scope. The distinction matters for the API surface: `&'static str` forces the reasons to be compile-time literals (which is fine for layers 2/3/4 since their reasons are fixed strings per AC-06) but forbids dynamically constructed reasons; `String` permits both.

AC-06's verification asserts the pending strings verbatim, which is consistent with either type. The practical choice is `&'static str` (the reasons are a closed, compile-time set) but the spec should say so in one place.

**Evidence:**
- spec.yaml constraint #7 (line 21): `Pending { reason: String }`.
- spec.yaml AC-06 (line 116): `Pending { reason: &'static str }`.
- spec.yaml AC-12 verification (line 234): `Pending { reason: "SQL emitter not yet available" }` — literal, consistent with either.

**Recommendation:** Pick one representation and fix the other site. `&'static str` is the better choice here (the reason set is closed: one reason per pending layer, known at compile time). Update constraint #7 to read `Pending { reason: &'static str }`.

---

### [INFO] Gate-AC verification determinism check: passes
**Category:** structural
**Pass:** 1
**Description:** Pass 1's deterministic gate-AC verification rule was applied to each `ac_type: gate` AC (ac-14, ac-15, ac-16). All three have non-empty verification fields, none contain placeholder tokens (TBD/TODO/FIXME/PLACEHOLDER/XXX/???), and all three far exceed the 20-char minimum length. No finding raised — recorded here for traceability.

**Evidence:** ac-14 verification (lines 267–272), ac-15 verification (lines 283–287), ac-16 verification (lines 298–302) — each is multi-line, names a specific integration-test file, and specifies the assertion in full.

---

## Honest Assessment

This is a well-structured, well-constrained spec — 17 ACs, every one with a concrete verification path, a clean layered equivalence model, and honest scaffolding around layers 2–4 that cleanly defers fake-green risk to the specific downstream cards that own it. The kind-registry cross-reference with card 0001 (verified present in the implemented `brightfield-spec` crate) is the single most valuable structural decision: it prevents the deviation registry from drifting into string-matching.

The two MEDIUM findings both concern type/signature gaps that an implementer would hit within an hour of starting — they will surface, but cheaply. The cleanest fix is: (a) drop `UnknownAffectedSpec` from `load_deviations` and keep cross-artefact integrity in AC-14's gate test only, and (b) define `CorpusEntry` and `LayerExpectations` as an explicit AC. Both are small textual edits, not redesigns. The LOW findings are best-addressed opportunistically; they do not block implementation.

Biggest residual risk: the single-source-of-truth contract is strong, but the doc-drift gap (registry vs committed `DEVIATIONS.md`) is the one place the spec lets a human-in-the-loop maintain synchronisation. In practice this will drift at least once. The LOW-severity mitigation (a 10-line drift-check test) closes it permanently for almost zero cost and would upgrade this from "good spec with one soft edge" to "airtight".
