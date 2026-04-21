# Spec Review

**Date:** 2026-04-20
**Reviewer:** Context-separated agent (fresh session)
**Spec:** /Users/hugh/github/meridian-online/brightfield/orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | content signals (cross-card boundary to card 0002; stable contract consumed by query engine, coordinator, renderer; vendored upstream artefact) | 2 |
| 3 — Adversarial | not triggered (Pass 2 findings are isolated notes with no cascading structural impact) | — |

Gate-AC deterministic check (non-empty, non-placeholder, minimum 20 chars): AC-12, AC-13, AC-14 all PASS.

## Cycle-1 Follow-Up Summary

This is the v2 review following the v1 `REQUEST_CHANGES` verdict. Each v1 finding has been checked against the current spec:

```
| v1 finding                                                    | v1 sev | Status in v2                                                                                                                                         |
|---------------------------------------------------------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------|
| AC-02 contradicts ontology (Mark/Interactor/Input enum vs struct) | HIGH   | RESOLVED. AC-02 now splits structs (`Mark`, `Interactor`, `Input`) from sealed enums (`Component`, `ValueOrParamRef<T>`); grep verification aligned; ontology_schema updated. |
| Strict context for `$param` undefined; AC-14(e) untestable        | MEDIUM | RESOLVED. New constraint (spec.yaml:25) enumerates strict contexts (meta.* Strings, config.* scalars, plotDefaults.* scalars) and names the detection mechanism (post-deserialise visitor on leading `$`). AC-14(e) now asserts `field_path == "meta.title"`. |
| SQL tokeniser grammar under-specified (literal awareness)         | MEDIUM | RESOLVED. New constraint (spec.yaml:24) pins grammar to upstream `ExpressionNode.js` with explicit rules for single-quoted literals, double-quoted identifiers, line comments, block comments. AC-06 now has five cases (a)–(e) covering each edge. |
| AC-05 lift surface open-ended ("etc.")                            | MEDIUM | RESOLVED. Constraint (spec.yaml:26) enumerates every channel; pinned to `LIFT_SURFACE_FIELDS` module constant. AC-05 now parametrises over every entry of `LIFT_SURFACE_FIELDS`, so omissions surface as test fails. |
| AC-11 round-trip canonical form unspecified                       | MEDIUM | RESOLVED. AC-11 (and constraint at spec.yaml:30) now specify `ParamRef` serialises as string form (`"$name"`) unconditionally; textual idempotence is declared not a goal. |
| AC-03 grep missing `-R`                                           | LOW    | RESOLVED. AC-03 verification now uses `grep -Rq`. |
| AC-10 version comparison ambiguity                                | LOW    | RESOLVED. AC-10 now specifies `(major, minor)` comparison via `SUPPORTED_MOSAIC_MAJOR_MINOR: (u16, u16) = (0, 24)`; five tests cover match/mismatch/patch variance/unparseable cases. |
```

All seven v1 findings are addressed. The spec has also gained structural improvements: an explicit `vendor/mosaic-specs/README.md` requirement recording the upstream commit SHA (AC-12, exit condition), and the AC-02 unit test `ast::component_enum_is_sealed` verifying the absence of an `Other` variant.

## Findings

### [LOW] AC-02 verification does not assert `Mark`/`Interactor`/`Input` have a `kind` field
**Category:** test-gap
**Pass:** 1
**Description:** AC-02 (spec.yaml:44-61) describes the struct shape `Mark { kind: MarkKind, data: Option<PlotFrom>, options: IndexMap<...> }` and parallel shapes for `Interactor` and `Input`. Its verification greps only for the presence of `pub struct Mark`, `pub struct Interactor`, `pub struct Input` and for the `Component` enum sealing test. Nothing in the verification checks that the `kind` field exists or has the expected type. An implementer who writes `pub struct Mark { name: String, options: IndexMap<...> }` (no `kind` field of type `MarkKind`) would satisfy AC-02's grep while breaking the contract AC-03 depends on (the Kind enum as sole name authority).
**Evidence:** spec.yaml:44-61 (AC-02 description + verification).
**Recommendation:** Either (a) add a compile-time assertion test such as `ast::mark_has_kind_field` that constructs a `Mark { kind: MarkKind::Line, .. }` literal (failing to compile if the field is missing or mistyped), or (b) extend the grep to include `grep -Rq 'kind: MarkKind' crates/brightfield-spec/src/ast/`. Option (a) is stronger — literal construction catches field renames AC-03 silently tolerates.

### [LOW] AC-12 "at minimum the 54 canon specs" introduces an unpinned corpus-size floor
**Category:** assumption
**Pass:** 1
**Description:** AC-12 (spec.yaml:199-205) says "at minimum the 54 canon specs" parse. This is descriptive prose inside the description; the verification itself iterates whatever is vendored. The floor is not machine-checked — if someone vendors only 30 specs the test still passes (vacuously: 30 of 30 are `Ok(_)`). This is only a real problem if the vendoring step happens separately from the test run and silently under-populates. The exit condition "Vendored corpus README records the upstream commit SHA" helps but does not count files.
**Evidence:** spec.yaml:199-205 (AC-12), spec.yaml:300 (exit condition).
**Recommendation:** Add a one-line assertion in `tests/corpus_totality.rs`: `assert!(corpus.len() >= 54, "vendored corpus under-populated: got {}", corpus.len());`. Cheap; makes the floor machine-enforced. Alternatively, accept the risk and drop the "54" number from the prose so the AC and its verification agree that the corpus size is whatever is present.

### [LOW] `LIFT_SURFACE_FIELDS` is referenced as a module constant but its type/shape is not pinned
**Category:** missing-requirement
**Pass:** 2
**Description:** The constraint at spec.yaml:26 and AC-05's verification both reference a module constant `crates/brightfield-spec/src/parse/lift_surface.rs::LIFT_SURFACE_FIELDS`. The spec does not state its Rust type (e.g. `&[&str]`, `&[(SurfaceKind, &str)]`), nor how the AC-05 parametrised test iterates it (macro expansion, `#[test_case]`, a runtime loop inside a single `#[test]`). Interpretation matters because "test compiles to one assertion per field, so omissions surface as fails, not as silent under-coverage" (AC-05 verification prose) is only true if the harness is compile-time enumerating (e.g. `proptest`, `test_case`, or a build.rs-generated test module). A plain runtime `for field in LIFT_SURFACE_FIELDS { assert!(...) }` technically meets the letter but a single missing field still surfaces as a single fail — acceptable but weaker than implied.
**Evidence:** spec.yaml:26 (constraint), spec.yaml:86-96 (AC-05).
**Recommendation:** Either (i) accept the weaker runtime-loop reading and rephrase AC-05's verification as "a single test iterates `LIFT_SURFACE_FIELDS` and asserts lifting at every entry; the first missing field fails the test"; or (ii) mandate a compile-time enumeration (e.g. `test_case` crate, or a macro) so each field is an independent `#[test]`. Minor — the correctness bar is the same; only test-failure granularity differs.

### [LOW] Strict-context detection rule does not specify `$$` column-reference handling
**Category:** assumption
**Pass:** 2
**Description:** The strict-context detection rule (spec.yaml:25) triggers on "any value whose leading non-whitespace character is `$`". Mosaic's tokeniser uses `$$ident` as a *column* reference (distinct from `$ident` param reference) — see the decisions pack Decision 5 (`$param` vs `$$col`). A `meta.title: "$$col"` value, were such a thing ever authored, would trip the strict-context rule even though semantically it is not a param reference. This is an edge case (users do not typically write column-refs in metadata fields) but the rule as written also fires on any SQL-snippet-shaped string that happens to begin with `$`, including literal currency tokens (`"$5 discount"`). The spec pins strict contexts to `meta` / `config` / `plotDefaults` top-level heads where such values are unlikely, so the blast radius is small.
**Evidence:** spec.yaml:25 (strict-context detection rule); decisions.md:116 (Decision 5 `$$col` variant).
**Recommendation:** Either (a) tighten the rule to "leading `$` followed by an identifier character (`[A-Za-z_]`)" (matching the tokeniser's param-ref grammar, which excludes `$$` and `$5`); or (b) document explicitly that any leading `$` is treated as an unresolved-ref in strict contexts by design, with the rationale that these heads are metadata and should never contain `$`-prefixed values. Rule (a) is lower-surprise; rule (b) is simpler.

---

## Honest Assessment

All seven v1 findings are substantively addressed, including the HIGH structural contradiction between AC-02 and the ontology on `Mark`/`Interactor`/`Input` typing. The v2 spec is notably crisper on the three previously under-defined surfaces — strict contexts, SQL tokeniser grammar, and lift surface — each now pinned to an upstream artefact at the pinned Mosaic 0.24.x commit, with tests that parametrise over the constants rather than sampling. The four new findings in this v2 review are all LOW severity: two test-hardening suggestions (AC-02 kind-field assertion, AC-12 corpus-size floor), one clarification about `LIFT_SURFACE_FIELDS`'s shape, and one edge-case note on `$$` column-ref handling in strict contexts. None block implementation; any or all could be folded in during drafting or left for a post-merge polish pass. The spec is implementation-ready.