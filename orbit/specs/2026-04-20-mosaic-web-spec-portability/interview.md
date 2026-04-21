# Design: Mosaic web spec portability

**Date:** 2026-04-20
**Interviewer:** Nightingale (rally design sub-agent + lead)
**Card:** orbit/cards/0002-mosaic-web-spec-portability.yaml
**Rally:** orbit/specs/rally.yaml (spec foundation; paired with card 0001)

---

## Context

Card: *Mosaic web spec portability* — 5 scenarios; goal: "A reference set of Mosaic web example specs renders equivalently in brightfield."
Prior specs: 0 (greenfield project)
Gap: Turning the "works unchanged" promise of brief §2 and §8 into a testable contract. Defines the portability contract between Mosaic web and brightfield; constrains what "equivalent output" means, how unsupported-but-valid features are detected and reported, how deviations are documented, and what the conformance harness actually verifies.

Reference material consulted:
- `/Users/hugh/github/meridian-online/brightfield-brief.md` (§2 principles, §6 risks, §8 success criteria)
- `/Users/hugh/github/uwdata/mosaic/packages/vgplot/plot/` (web rendering reference)
- `/Users/hugh/github/uwdata/mosaic/packages/vgplot/inputs/` (input widgets — CSS-dependent surfaces)
- `/Users/hugh/github/uwdata/mosaic/packages/vgplot/spec/src/config/` (plots/inputs/transforms whitelists)
- `/Users/hugh/github/uwdata/mosaic/docs/public/specs/yaml/` (54 canonical specs as candidates for conformance corpus)

Decision pack: `orbit/specs/2026-04-20-mosaic-web-spec-portability/decisions.md`.

## Q&A

### Q1: Equivalence criterion — what does "works unchanged" mean?
**Q:** Pixel-diff conformance, layered semantic equivalence (AST + SQL + encoding + interaction), or behavioural golden outputs (query log + selection-state timeline)?
**A:** Layered semantic equivalence, with each layer as an independent pass/fail gate per spec.
- **Layer 1 — AST round-trip:** `parse(spec) → AST → serialise → parse` is idempotent; AST equals Mosaic's `parseSpec` output structure.
- **Layer 2 — SQL equivalence:** the query engine's emitted SQL produces the same result sets as Mosaic's `mosaic-sql` output for the same spec.
- **Layer 3 — Visual-encoding equivalence:** for each mark, a structured snapshot (mark type, data binding, scale mapping, channel values after scale application) matches Mosaic's.
- **Layer 4 — Interaction equivalence:** given a scripted event sequence, coordinator selection state evolves identically.

Pixel parity is deliberately out of scope — GPUI vs Observable Plot/SVG/CSS guarantees pixel differences the brief §6 already anticipates. Each layer is independently diagnostic. A spec may pass layers 1–3 while being explicitly known-deviating at layer 4.

### Q2: Conformance corpus — which reference specs are in scope?
**Q:** All 54 specs tagged `pass | expected-fail | skip`, capability-gated subset (intersection of specs × implemented marks), or curated core of ~10–15 specs covering the v1–v6 feature ladder?
**A:** Curated core, sized ~10–12 for v1. Starter set: `line.yaml`, `crossfilter.yaml`, `mark-types.yaml`, `legends.yaml`, `flights-200k.yaml`, `overview-detail.yaml`, `seattle-temp.yaml`, `facet-interval.yaml`, `table.yaml`, `sorted-bars.yaml`. Each spec exercises distinct v1–v6 features; every entry is interesting.

**Growth rule:** a spec enters the corpus when (a) every mark/interactor/input it uses is implemented and (b) adding it doesn't duplicate coverage already in the corpus.

A parallel `observed/` directory holds the full canon at the layer-1 (parse/round-trip) gate only — early evidence that the parser round-trips the Mosaic corpus without committing to rendering parity. This gives card 0001's parser totality claim (imposed via Option Z — see card 0001 Q6) a visible scoreboard from day one.

### Q3: Unsupported-feature detection — parse-time reject or render-time report?
**Q:** Parse-time whitelist rejection, full-vocabulary AST + preflight `SupportReport` pass, or lenient render with warning banner?
**A:** Full-vocabulary AST + preflight `SupportReport`. Parsing is total (per Option Z, settled with card 0001 Q6). A preflight pass walks the AST before render and emits a `SupportReport` enumerating unsupported nodes, their locations, and their Mosaic-identities. Render fails with a located error if any unsupported node is reached; the SupportReport is available to tools (linters, IDE plugins, conformance runner) without needing to parse separately.

**Cross-card resolution (Option Z):** Card 0001's parser stubs unimplemented marks/interactors/inputs into the AST with implementation-status metadata (`Implemented | Planned | Unimplemented`). The preflight pass in this card consumes that metadata directly — no second-pass introspection needed. Unknown names (not in the vocabulary registry at all) still fail at parse time with a `ParseError`.

### Q4: Deviation documentation — inline, manifest, or registry?
**Q:** Inline source comments, a `DEVIATIONS.md` narrative doc, or a structured deviation registry parsed by tests and docs?
**A:** Structured deviation registry. A typed file (YAML or TOML) at a known repo location holds one record per deviation:

```yaml
deviations:
  - id: DEV-0001
    surface: "legend"
    mosaic-behaviour: "Observable Plot SVG legend with implicit flow"
    brightfield-behaviour: "GPUI native legend, fixed horizontal layout"
    rationale: "no equivalent for Observable Plot's SVG flow layout"
    affected-specs: ["legends.yaml"]
    conformance-layers-suppressed: [3, 4]
```

Parsed by:
- the conformance runner — suppresses expected differences at the specified layers for the specified specs
- a doc generator — renders a user-facing `DEVIATIONS.md` from the registry for inclusion in brightfield docs

Concrete surfaces expected to need registry entries from day one: legend flow, tooltip / `nearest` rendering, `Menu`/`Search`/`Slider`/`Table` widgets, facet layout (`fx`/`fy` implicit flow), tick density / axis label placement heuristics.

### Q5: Conformance runner architecture and cadence
**Q:** Integrated `cargo test` harness on every PR, separate binary run pre-release / nightly, or hybrid (layers 1–2 per-PR as `cargo test`, layers 3–4 as a separate gated binary)?
**A:** Hybrid. Layers 1–2 are table-tests over fixture files — fast, deterministic, no rendering — and live under `tests/conformance/` as `cargo test` targets run on every PR. Layers 3–4 need live DuckDB + a mock event pump + the renderer; they run as a separate `cargo run --bin conformance` target gated pre-release and nightly. This protects card 0001's invariants cheaply on every commit without letting conformance runtime dominate CI.

### Q6: Constraints on card 0001
**Q:** What does this card require of card 0001's parser and AST?
**A:** Four reach-ins, all acknowledged in card 0001's interview (see `orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/interview.md` Summary → Constraints):

1. **AST totality.** Every valid Mosaic spec produces an AST, even for marks/interactors/inputs brightfield does not yet render. (Settled via Option Z.)
2. **Round-trip fidelity.** `parse → serialise → parse` is idempotent on the AST.
3. **Structured introspection.** AST node types are enumerated (sealed); each node exposes source location where available; each component reference (mark name, interactor name, input name, transform name) is a typed value intersectable against a capability whitelist.
4. **Deviation-aware identity.** Each AST node carries a stable identity (mark kind, channel name, interactor type, input type) the deviation registry can reference without matching on renderer-internal labels.

These constraints are recorded in card 0001's interview as explicit design inputs.

---

## Summary

### Goal
Define and enforce the portability contract between Mosaic web specs and brightfield's rendering. The contract is a layered, independently-diagnostic equivalence: AST round-trip, SQL equivalence, visual-encoding equivalence, interaction equivalence. A curated corpus of canonical Mosaic specs acts as the release gate; a full-canon `observed/` corpus gates the parser's totality promise. Unsupported (but valid) spec features surface via a preflight `SupportReport` sourced from card 0001's vocabulary registry. Deliberate deviations from Mosaic web rendering are recorded in a structured registry that drives both tests and user-facing docs.

### Constraints
- Equivalence is layered (AST + SQL + encoding + interaction); pixel-diff is explicitly not part of the contract.
- Conformance corpus: curated core (~10–12 specs) for v1 gating layers 1–4; full-canon `observed/` for layer-1 only.
- Unsupported features are surfaced explicitly via preflight `SupportReport` — never silently omitted or approximated.
- Deviations are recorded in a structured registry file (`deviations.yaml` or equivalent) with stable IDs, affected-spec lists, and suppressed-conformance-layer annotations.
- The conformance runner is split: cheap layers (1, 2) run on every PR via `cargo test`; expensive layers (3, 4) run via a gated `cargo run --bin conformance` target.
- Card 0001 constraints: AST totality, round-trip fidelity, structured introspection, deviation-aware identity.
- Mosaic web rendering is the portability target; Observable Plot and CSS-layout-dependent behaviours are explicitly documented as deviations, not bugs.

### Success Criteria
- Every spec in the `observed/` corpus (full `docs/public/specs/yaml/` canon) passes layer 1 (AST round-trip) via `cargo test` on every PR.
- Every spec in the curated core (~10–12 specs) passes layers 1 and 2 on every PR.
- Every spec in the curated core passes layers 3 and 4 via the gated conformance binary pre-release, with exceptions limited to deviations registered in the deviation registry.
- The preflight `SupportReport` identifies every Mosaic mark/interactor/input not yet implemented in brightfield, located by spec position, without failing parsing.
- The deviation registry is the single source of truth: every conformance-runner suppression cites a registry ID, and `DEVIATIONS.md` is generated from the registry rather than hand-maintained.
- A Mosaic web example spec loaded into brightfield either (a) renders equivalently at all four layers within registered deviations, or (b) produces a diagnostic naming the unsupported feature and its location — never silently produces broken output.

### Decisions Surfaced
- **Equivalence criterion**: Layered (AST + SQL + encoding + interaction), per-layer pass/fail. Pixel parity out of scope.
- **Conformance corpus**: Curated core (~10–12 specs) for full gating; `observed/` holds the full canon at layer-1 only; growth rule is implementation-gated + non-duplicative.
- **Unsupported detection**: Full-vocabulary AST (via Option Z) + preflight `SupportReport`.
- **Deviation docs**: Structured registry file drives both conformance-runner suppressions and a generated `DEVIATIONS.md`.
- **Runner architecture**: Hybrid — layers 1–2 on every PR as `cargo test`; layers 3–4 as a gated `cargo run --bin conformance` target.
- **Cross-card (Option Z)**: Registry-backed vocabulary with `Implemented | Planned | Unimplemented` status. Resolves the totality-vs-strictness contradiction with card 0001 Q6.

### Open Questions
- Conformance runner output format — plain text for CI logs, JSON for downstream tooling, or both?
- Does the deviation registry live in the repo root or under `orbit/specs/`? (Probably root — it's user-facing material.)
- Initial surfaces for registry entries from day one: legend flow, tooltips, input widgets (`Menu`, `Search`, `Slider`, `Table`), facet layout, tick/axis heuristics. Prioritisation and coverage when each lands are future-card concerns.
