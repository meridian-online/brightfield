# Spec Review

**Date:** 2026-04-20
**Reviewer:** Context-separated agent (fresh session)
**Spec:** /Users/hugh/github/meridian-online/brightfield/orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | content signals (cross-card boundary to card 0002; stable contract for query engine/coordinator/renderer) + 1 structural contradiction found in Pass 1 carried forward | 5 |
| 3 — Adversarial | structural contradiction in AC-02 vs ontology; under-defined tokeniser grammar; unspecified "strict context" rule cascades across AC-14(e), AC-11, and downstream coordinator contract | 1 |

Gate-AC deterministic check (non-empty, non-placeholder, >=20 chars): AC-12, AC-13, AC-14 all pass.

## Findings

### [HIGH] AC-02 contradicts the ontology: is `Mark` an enum or a struct?
**Category:** constraint-conflict
**Pass:** 1
**Description:** AC-02's description states `Component`, `Mark`, `Interactor`, `Input` are "sealed enums (no `#[non_exhaustive]`, no trailing `Other` catch-all)" and its verification greps for `^pub enum \(Component\|Mark\|Interactor\|Input\)`. However the `ontology_schema` declares `Mark` as `type: "struct"` with shape `{ kind: MarkKind, data: Option<PlotFrom>, options: IndexMap<...> }`, and the Q1 interview answer explicitly says `Mark { kind: MarkKind, data: Option<PlotFrom>, options: MarkOptions }`. The enum is `MarkKind`, not `Mark`. Same pattern applies to `Input` — the ontology lists `Input` as a Component variant while AC-02 treats `Input` as a sealed enum in its own right. An implementer following AC-02 literally will produce a `pub enum Mark` that fights the ontology and the interview; one following the ontology will have AC-02's grep verification fail.
**Evidence:** spec.yaml:44-50 (AC-02 description + verification); spec.yaml:200-211 (ontology_schema Mark/MarkKind/Component); interview.md:27 ("Mark { kind: MarkKind, data: Option<PlotFrom>, options: MarkOptions }").
**Recommendation:** Split AC-02 into two explicit clauses: (a) the enum-typed kind registries are `MarkKind`, `InteractorKind`, `InputKind`, `ComponentKind` (already covered by AC-03); (b) the AST node types `Mark`, `Interactor`, `Input` are structs keyed by their `*Kind` enum. `Component` is the sole composition-level sealed enum. Update the AC-02 grep accordingly (e.g. grep for `pub struct Mark` and `pub enum Component`, and drop `Mark|Interactor|Input` from the enum regex).

### [MEDIUM] "Strict context" for `$param` resolution is undefined, yet AC-14(e) tests it
**Category:** missing-requirement
**Pass:** 2
**Description:** Constraint list and AC-09 both mention `StrictContextUnresolvedRef` / `ParseError::StrictContextUnresolvedRef`. AC-14(e) makes this testable with the example `meta.title: $missing`. But nowhere does the spec enumerate which field positions are "strict contexts" for `$param` resolution, nor the mechanism by which the parser detects a `$foo` string inside a strict field (given `meta` uses `deny_unknown_fields` and `meta.title` is a typed `String`, a `$missing` value would deserialise as the literal string `"$missing"` with no resolution attempted unless a post-deserialise semantic pass exists). Without a defined rule, the implementer invents the boundary; the test in AC-14(e) is then satisfied by whatever rule the implementer chose, defeating the point of the gate.
**Evidence:** spec.yaml:24 ("unresolved strict-context reference"), spec.yaml:117-125 (AC-09 `StrictContextUnresolvedRef`), spec.yaml:177-181 (AC-14(e) test case), spec.yaml:22 (lift surface listed as `filterBy`, `as`, mark channel slots — no dual list of strict positions).
**Recommendation:** Add an explicit constraint enumerating strict contexts (candidates: `meta.title`, `meta.description`, `meta.version`, `config.*` scalar fields, `plotDefaults.*` scalar fields — i.e. exactly the top-level closed-vocabulary heads). State the detection mechanism: either (a) a post-deserialise visitor scans String-typed fields under strict heads for a leading `$` and raises, or (b) those fields use a newtype that refuses `$`-prefixed values at deserialisation. Then AC-14(e)'s test sits on a defined contract.

### [MEDIUM] SQL tokeniser grammar is under-specified — literal-awareness unaddressed
**Category:** test-gap
**Pass:** 2
**Description:** Constraint (SQL expressions "tokenised at parse time into `ExpressionNode { spans, params }`") and AC-06 say the tokeniser "splits on `$ident` occurrences" but do not define (a) what character class terminates `$ident`, (b) whether `$` occurrences inside SQL string literals (`'$100'`, `"$col"`) or comments (`-- $note`) are treated as params or preserved as literal text. The sole verification example is `"x > $lo AND x < $hi"`, which exercises neither edge case. A literal-unaware tokeniser silently synthesises false-positive `ParamRef` entries, corrupting the coordinator's subscription graph (Q5: "the coordinator walks the AST once to build its subscription graph") without any AC failing.
**Evidence:** spec.yaml:23 (tokenisation constraint), spec.yaml:86-93 (AC-06), interview.md:42-43 (Q5 — coordinator consumes `ExpressionNode.params`).
**Recommendation:** Either (i) port Mosaic's `ExpressionNode.js` tokeniser behaviour verbatim and add a constraint "Tokeniser behaviour mirrors upstream Mosaic 0.24.x `ExpressionNode.js` — in particular, `$` inside single-quoted, double-quoted, and line-comment spans is preserved as literal text"; or (ii) explicitly declare string-literal-unawareness a v1 limitation with a `ParseWarning::AmbiguousDollarInLiteral` and an AC test. Extend AC-06 with one positive case containing a string literal (`"x = '$foo' AND y = $bar"` → `spans=[..., "'$foo' AND y = ", ""]`, `params=[ParamRef("bar")]`).

### [MEDIUM] AC-05 lifting surface is open-ended ("etc."), making the AC untestable for full coverage
**Category:** test-gap
**Pass:** 2
**Description:** AC-05 names `filterBy`, `as`, and "mark channel slots (`x`, `y`, `fill`, `stroke`, etc.)" with an explicit "at least" hedge. The hedge admits partial compliance. Downstream cards (query engine, coordinator) will depend on the exhaustive lifting surface; if `r`, `opacity`, `dx`, `dy`, `symbol`, `size`, `stroke`, `strokeWidth`, `strokeOpacity`, `fillOpacity`, etc. are not lifted, those parts of the AST still contain raw strings and the coordinator's subscription graph has silent holes — not caught by AC-12 corpus totality (which only asserts `Ok(_)`).
**Evidence:** spec.yaml:77-83 (AC-05 description + verification).
**Recommendation:** Pin the lift surface to "every field position Mosaic's `parse-spec.js` routes through `maybeParam` in 0.24.x" (already referenced in interview.md:42). Either enumerate the full list in a constraint or add a generative test that walks the vendored `parse-spec.js` channel registry and asserts each channel is covered. At minimum, replace "etc." with the complete enumerated list for 0.24.x marks.

### [MEDIUM] AC-11 round-trip "canonical form per field position" rule not pinned
**Category:** assumption
**Pass:** 2
**Description:** AC-11 requires round-trip to "preserve `ParamRef` lifting (re-serialises as the canonical form the parser chose — string or object — consistently per field position)". This implies a deterministic, position-indexed canonical form choice, but the spec nowhere defines it. Two valid implementations (always emit string form; emit the form that matches the source) produce different textual outputs; only the first satisfies idempotence when the source used object form for one field and string form for another in the same position across different specs in the corpus. Risk: flaky round-trip test, or a test that passes under one canonicalisation choice and fails under another.
**Evidence:** spec.yaml:137-146 (AC-11 description + verification).
**Recommendation:** Add a constraint: `ParamRef` re-serialises as the string form `"$name"` unconditionally (simplest canonicalisation, matches the majority of Mosaic corpus usage), OR state explicitly that round-trip idempotence is on AST only and textual round-trip is not a goal. Drop the "string or object — consistently per field position" clause unless the per-position table is supplied.

### [LOW] AC-03 grep uses directory path without `-r`/`-R`
**Category:** test-gap
**Pass:** 1
**Description:** `grep -q 'enum ImplStatus' crates/brightfield-spec/src/vocab/` treats `vocab/` as a file; standard grep returns an error when given a directory without a recursive flag. Verification command will fail even when the code is correct.
**Evidence:** spec.yaml:61 (AC-03 verification).
**Recommendation:** Use `grep -Rq 'enum ImplStatus' crates/brightfield-spec/src/vocab/` (the same `-R` flag AC-02 already uses).

### [LOW] AC-10 `VersionMismatch` warning fires on `0.24.0` vs `0.24.x` pin — string equality ambiguity
**Category:** assumption
**Pass:** 3
**Description:** `SUPPORTED_MOSAIC_VERSION` is the literal string `"0.24.x"`. The mismatch rule in AC-10 is undefined — is it exact string equality, semver-range containment, or major/minor comparison? The test uses `"0.20.0"` (definitely mismatched) so passes under any rule, but a real spec with `meta.version: "0.24.0"` could be flagged as mismatched under exact-string equality. This is a minor UX footgun that will surface the first time a corpus spec declares a concrete version.
**Evidence:** spec.yaml:127-135 (AC-10).
**Recommendation:** Specify the comparison rule: "mismatch = the spec's major.minor differs from the pinned major.minor (0.24)". Add a negative-case test (`meta.version: "0.24.0"` → no warning).

## Honest Assessment

The spec is substantively well-scoped and the design decisions (Option Z vocabulary, staged typing, miette-free library boundary, structural lift at parse time) are coherent and well-justified in the interview. Acceptance criteria are largely testable with concrete commands and named tests, and the gate ACs clear the deterministic-verification bar. The biggest risk is the Pass 1 contradiction in AC-02 vs the ontology on whether `Mark`/`Interactor`/`Input` are enums or structs — an implementer will have to choose, and choosing wrong against the grep verification makes AC-02 mechanically fail even when the code matches the interview intent. Second-tier risk is the under-defined semantics around strict-context `$param` detection, SQL tokeniser grammar, and the open-ended lifting surface ("etc.") — these look minor but together constitute the parser's stable contract to downstream cards (query engine, coordinator, renderer), so any ambiguity here will be expensive to unwind later. Fix AC-02's wording, pin the tokeniser grammar to upstream Mosaic's behaviour, enumerate strict contexts, and close the "etc." in AC-05; after that the spec is implementation-ready.
