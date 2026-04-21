# Decision Pack: Mosaic Web Spec Portability (Card 0002)

Scope: defines the *portability contract* between Mosaic web and brightfield. Assumes card 0001 produces an AST faithful to Mosaic's `parseSpec` output (see `packages/vgplot/spec/src/parse-spec.js`, `packages/vgplot/spec/src/ast/*`). Where portability places additional constraints on 0001, they are called out in decision D6.

These are the five decisions forced by the card's scenarios. Each is phrased so it can be resolved in a consolidated design gate.

---

## D1. Equivalence criterion — what does "works unchanged" mean?

**Context.** The brief §8 success criterion states: "the same spec, unmodified, produces *equivalent output* to Mosaic's web rendering." The card's scenarios 1, 2, and 5 all depend on a testable definition of equivalence. Pixel-level parity is not achievable: brightfield uses GPUI/Metal/Vulkan (brief §3.4), Mosaic web uses Observable Plot/SVG, and the brief §6 explicitly anticipates Observable Plot quirks and CSS layout leakage that *should* render differently.

**Options.**

- **A. Pixel-diff conformance.** Render both sides to bitmaps, compare with a perceptual-diff budget.
- **B. Layered semantic equivalence.** Define equivalence on four layers: (1) AST round-trip — parsed AST equals Mosaic's `parseSpec` output for the same spec; (2) SQL equivalence — the query engine emits SQL whose result sets match the ones Mosaic's `mosaic-sql` would produce; (3) visual-encoding equivalence — for each mark, a structured description (mark type, data binding, scale mapping, channel values after scale application) matches; (4) interaction equivalence — given a scripted event sequence, coordinator selection state evolves identically.
- **C. Behavioural golden outputs only.** No pixel comparison, no structural AST comparison. Drive specs through a scripted harness, snapshot the resulting DuckDB query log and serialised selection-state timeline, diff those.

**Trade-offs.**

- **A.** Gains a visceral "it looks the same" signal. Loses almost everything else: Observable Plot tick placement, SVG antialiasing, CSS font metrics, and legend flow (`packages/vgplot/inputs/src/Menu.js` and `Table.js` both mount raw HTML `<select>`/`<table>` — no GPUI equivalent will be pixel-identical) will all bust the budget. Brief §6 flags this directly. High maintenance cost, low signal-to-noise.
- **B.** Gains a precise, layered definition: portability can be asserted at whichever layers are meaningful for a given feature. A tooltip is never going to pass layer 4 trivially, but layers 1–3 can still pass. Each layer is independently diagnostic when a regression occurs (AST diverged? SQL diverged? encoding diverged? interaction diverged?). Loses simplicity — four harnesses, not one. Requires brightfield to expose structured introspection surfaces (an "encoding snapshot" API).
- **C.** Gains implementation simplicity. Loses the ability to catch regressions that only manifest visually (wrong scale, wrong domain inference, wrong facet wiring). SQL-log equivalence alone does not prove the renderer is wiring the right column to the right channel.

**Recommendation: B, with layers stated as independent pass/fail gates per spec.**

Evidence: the brief's §6 "spec compatibility" risk explicitly splits the world into things that *must* match (the declarative semantics) and things that *will* deviate (Observable Plot and CSS quirks). Layer 1 (AST) + Layer 2 (SQL) + Layer 3 (encoding) is exactly the semantic core; Layer 4 (interaction) covers the coordinator reactivity the brief §3.3 describes. Pixel diff is kept *out* of the conformance contract; it can still be used as a debugging aid but is not authoritative. This also lets the gate be *graduated* — a spec may conform at layers 1–3 while being explicitly known-deviating at layer 4.

---

## D2. Conformance corpus — which reference specs are in scope?

**Context.** `docs/public/specs/yaml/` contains 54 canonical specs (verified: `aeromagnetic-survey.yaml` through `wnba-shots.yaml`). They span marks the brief defers (geo, contour, hexbin, raster, density) and interactors the brief defers (pan/zoom, nearest/tooltip). The brief §3.4 lists initial mark priorities: `lineY, barY, dot, areaY, rect, text, rule`, and §7 stages the work v1→v8. Running all 54 through a conformance gate pre-v4 is guaranteed red.

**Options.**

- **A. All 54, each tagged `pass | expected-fail | skip`.** The corpus is frozen; progress is measured as the `pass` count climbing.
- **B. Capability-gated subset.** The corpus is the intersection of (specs in `docs/public/specs/yaml/`) × (marks/interactors/inputs implemented in brightfield). As implementation advances, the corpus grows.
- **C. Curated core of ~10–15 specs.** Hand-pick a representative subset covering the v1–v4 feature ladder: one per mark type, one cross-filter, one facet, one concat layout, one legend-selection spec. Grow it deliberately, not by coverage reflex.

**Trade-offs.**

- **A.** Gains a single visible scoreboard across the whole Mosaic canon. Loses signal: most of the board is red for months of v1–v4 work, and the `expected-fail` tag decays into "we forgot this". Tag drift is a real maintenance tax.
- **B.** Gains tight coupling between implementation scope and conformance scope — impossible to be "green but missing everything". Loses the ability to use conformance as a *forcing function*: you never see `crossfilter.yaml` fail until you've already declared you implement cross-filtering, so the corpus gives no feedback ahead of the work.
- **C.** Gains curation: `crossfilter.yaml`, `mark-types.yaml`, `legends.yaml`, `flights-200k.yaml`, `overview-detail.yaml`, `seattle-temp.yaml`, `line.yaml`, `facet-interval.yaml`, `table.yaml`, `sorted-bars.yaml` as a starter set directly exercises v1–v6 of the brief §7 ladder. Every spec in the corpus is *interesting*. Loses exhaustive coverage — a quirk present only in `observable-latency.yaml` won't surface until someone adds it.

**Recommendation: C (curated core) for v1, with a stated growth rule.**

Rule: a spec enters the corpus when (a) every mark/interactor/input it uses is implemented, and (b) adding it doesn't duplicate coverage already in the corpus. Starter set above, sized ~10–12. A separate `observed/` directory holds specs that are parsed-only (AST layer 1 gate) — this gives early-release evidence that the parser round-trips the full canon without committing to rendering parity.

Evidence: brief §6 ("prioritise a minimal viable set of marks ... first") and §7's staged ladder argue against a frozen full-corpus target. `packages/vgplot/spec/src/config/plots.js` and `packages/vgplot/spec/src/config/inputs.js` already demonstrate that Mosaic treats supported component names as *sets* that can be narrowed — a capability-gated corpus maps onto this structure naturally.

---

## D3. Unsupported-feature detection — parse-time reject or render-time report?

**Context.** Scenario 3 requires that unsupported features be *surfaced explicitly*, "rather than silently omitting or approximating". Mosaic's own parser is already whitelist-driven: `ParseContext` takes `components`, `transforms`, `inputs`, and `plot` (attributes/interactors/legends/marks) as `Set<string>` (see `packages/vgplot/spec/src/parse-spec.js` lines 33–68). Narrowing those sets rejects unsupported directives at parse time. The alternative is to accept the full Mosaic vocabulary into the AST and have the renderer raise when it encounters a node type it can't draw.

**Options.**

- **A. Parse-time whitelist rejection.** brightfield configures the parser with the currently supported sets; an unsupported directive fails parsing with a located error (file, line, directive name).
- **B. Full-vocabulary AST + render-time reporting.** The parser accepts everything Mosaic's schema allows. A preflight pass walks the AST and emits a structured "support report" *before* rendering begins. Unsupported nodes fail the render with the same located error, but the AST itself is still inspectable and portable.
- **C. Lenient render with warning banner.** Unsupported features are skipped, a banner lists them. (Rejected immediately — violates scenario 3's "rather than silently omitting or approximating".)

**Trade-offs.**

- **A.** Gains alignment with Mosaic's own parser architecture — reuses the whitelist contract Mosaic already exposes. Simpler: one failure mode, one location. Loses a useful affordance: tools (linters, IDE plugins, conformance runners) can't reason over the full AST of a spec that uses features brightfield doesn't implement yet, because parsing halts.
- **B.** Gains a tool-friendly AST (card 0001's output is always the full Mosaic AST; support is a layered property on top). The preflight report is exactly the data structure the conformance runner needs anyway. Loses the single-failure-mode simplicity — two enforcement points (preflight, renderer) must agree.
- **C.** Already excluded.

**Recommendation: B.**

Evidence: scenario 3 demands explicit surfacing, and the conformance runner (D1, D2) needs to introspect unsupported features *without* failing the parse to report them. A full-vocabulary AST with a preflight `SupportReport` is strictly more expressive than parse-time rejection and costs one extra pass. Parsing should be *total* (any valid Mosaic spec parses to an AST); support is a separate judgement. This imposes a constraint on card 0001 — see D6.

---

## D4. Deviation documentation — inline, manifest, or registry?

**Context.** Scenario 4 requires: "brightfield applies its documented native equivalent and the deviation from web output is recorded in the brightfield docs rather than being an accident." Examples from Mosaic's renderer: `plot-renderer.js` delegates to `@observablehq/plot` (see line 1 import); legends are rendered by Observable Plot with SVG flow; `Menu.js` uses raw `<select>`; `Table.js` uses raw HTML `<table>` with CSS-driven layout. Each of these has no native GPUI equivalent that matches pixels — each is a deviation site.

**Options.**

- **A. Inline source comments.** `// DEVIATION: tick placement differs from Plot's implicit ...` near the relevant renderer code.
- **B. `DEVIATIONS.md` narrative doc.** One markdown file, human-curated, referenced from user-facing docs.
- **C. Structured deviation registry.** A typed file (`deviations.yaml` or `.toml`) with one record per deviation: `{ id, surface, mosaic-behaviour, brightfield-behaviour, rationale, affected-specs: [...] }`. Parsed by (i) the conformance runner to suppress expected differences at the relevant layer, (ii) a doc generator that renders `DEVIATIONS.md` from the registry.

**Trade-offs.**

- **A.** Gains proximity — the comment lives next to the code that causes the deviation. Loses discoverability and audit: no single list, no machine-readable link to test expectations. Drifts as code moves.
- **B.** Gains human readability. Loses coupling to the conformance suite: the doc drifts from actual behaviour, and "expected deviations" in the test runner are tracked in a separate, parallel structure.
- **C.** Gains a single source of truth that's both test-consumed and doc-consumed. Every expected conformance-suite variance *must* appear in the registry with a linked ID, which is exactly the audit trail scenario 4 asks for. Loses low-ceremony: a one-off typo fix in a tooltip is now a registry edit.

**Recommendation: C.**

Evidence: scenarios 3 and 4 together describe two states — "unsupported (error)" and "supported with documented deviation (warning / metadata)". The conformance runner in D1 must distinguish these states *at each layer*. A structured registry is the cleanest way to drive both gates from one source. Concrete surfaces that will need registry entries: (1) legend flow — Observable Plot's SVG legends vs a native GPUI legend (`packages/vgplot/plot/src/legend.js`); (2) tooltips/`nearest` — HTML DOM tooltips vs GPUI overlays; (3) `Menu`/`Search`/`Slider`/`Table` widgets — HTML form controls vs GPUI widgets; (4) facet layout — Observable Plot's implicit facet flow (`facet-interval.yaml` uses `fx`/`fy`) vs brightfield's native facet layout; (5) tick density and axis label placement — Plot's implicit heuristics vs the renderer's.

---

## D5. Conformance runner architecture and cadence

**Context.** The runner is the executable embodiment of D1–D4. It must (a) execute all layers per D1, (b) consult the registry per D4, (c) filter to the capability-gated corpus per D2/D3, (d) produce a report the project can use as a release gate per scenario 5.

**Options.**

- **A. Integrated `cargo test` harness, per-PR.** Conformance runs on every commit. Fixtures live under `tests/conformance/`.
- **B. Separate binary, gated.** `cargo run --bin conformance` produces a report. Run pre-release and on a nightly cron, not per-PR.
- **C. Hybrid: layer 1–2 (AST + SQL) per-PR as `cargo test`; layer 3–4 (encoding + interaction) as a separate gated binary.**

**Trade-offs.**

- **A.** Gains fast-feedback: no regression lasts longer than a PR. Loses runtime budget: layer 4 (interaction) requires driving the coordinator through scripted event sequences over real DuckDB fixtures for (at minimum) the starter 10–12 specs; this is slow, and making every contributor wait on it taxes velocity (brief §6: "single-developer velocity — favour vertical slices").
- **B.** Gains clean separation, cheap CI. Loses regression-detection speed: a layer-1 parser break shouldn't wait until the nightly.
- **C.** Gains tiered cost: parser and SQL gen are cheap to regress-test exhaustively on every PR (these are the layers card 0001 owns; cheap tests here protect its invariants too). Encoding and interaction tests are where the runtime lives and where the fixtures are heavy; these run pre-release. Loses a single-command mental model.

**Recommendation: C.**

Evidence: layer 1 (AST round-trip) and layer 2 (SQL equivalence) are table-tests over fixture files — fast, deterministic, no rendering needed. These fit `cargo test` naturally. Layers 3 and 4 need a live DuckDB and a mock-event-pump; gating them prevents CI from becoming the bottleneck the brief §6 warns against. This also gives card 0001 a cheap, tight feedback loop for its own work (AST + SQL) without being blocked on card 0002's heavier infrastructure.

---

## D6. Constraints this decision pack places on card 0001

Not a separate decision — a set of constraints that should be acknowledged in the consolidated design gate:

1. **AST totality (from D3 option B).** The parser must accept any valid Mosaic spec and produce an AST node, even for components brightfield does not yet render. Support is a layered judgement, not a parse gate. This means card 0001's node taxonomy must cover Mosaic's full component set (see `packages/vgplot/spec/src/ast/*` — 22 node types including `ContourMark`, `Grid2DMark`, `RegressionMark`, `WindowFrame`, etc., even though several are v7+ in the brief's ladder). Card 0001 is free to stub the payload of a node type but not its identity.

2. **AST round-trip fidelity (from D1 layer 1).** The AST must round-trip: `parse(spec) → AST → serialise → parse → AST'` with `AST == AST'`. This is weaker than requiring `serialise(AST) == original source` (whitespace/YAML-vs-JSON breaks that) but strong enough to be a cheap layer-1 conformance gate. Card 0001 should expose a serialisation path, even if it is only used by tests.

3. **Structured introspection (from D1 layer 3).** The AST must be walkable by the conformance runner and the preflight support-report pass. Concretely: node types are enumerated (sealed), each node exposes its source location, and each component reference (mark name, interactor name, input name, transform name) is a typed value that can be intersected against the capability whitelist. This is the shape `parseSpec` already takes via `Set<string>` whitelists; card 0001 should preserve it.

4. **Deviation-aware identity (from D4).** Each AST node should carry a stable identity the deviation registry can reference (e.g., mark type, channel name, interactor type). The registry shouldn't need to match on renderer-internal labels.

These four constraints are the only places card 0002 reaches into 0001's territory. Everything else (AST shape, node relationships, parser ergonomics) stays in 0001.

---

## Summary of recommendations

```
| # | Decision                        | Recommendation                                        |
|---|---------------------------------|-------------------------------------------------------|
| 1 | Equivalence criterion           | Layered: AST + SQL + encoding + interaction          |
| 2 | Conformance corpus              | Curated core (~10-12) growing by capability gate     |
| 3 | Unsupported-feature detection   | Full-vocabulary AST + preflight SupportReport        |
| 4 | Deviation documentation         | Structured registry drives both tests and docs       |
| 5 | Runner architecture             | Hybrid: layers 1-2 per-PR, layers 3-4 gated          |
| 6 | Constraints on card 0001        | AST totality, round-trip, introspection, identity    |
```
