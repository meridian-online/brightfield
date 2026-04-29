# Decision Pack: Reactive Parameters — Live Reactivity (v3)

**Card:** orbit/cards/0005-reactive-parameters-with-input-widgets.yaml
**Date:** 2026-04-29
**Slice:** v3 — close the live-reactivity gap (chained DAG walk, widget→coordinator wiring, partial-failure parity with `propagate_selection`)
**Prior art:**
- v1 (static analysis): `orbit/specs/2026-04-21-reactive-parameters-with-input-widgets/` — typed Input fields, subscriber graph, dependency DAG, topo order, type-mismatch warnings. Shipped.
- v2 (runtime coordinator, direct propagation only): `orbit/specs/2026-04-24-reactive-parameters-with-input-widgets/` — `Session::propagate_param`, `param_state`, partial failure. Shipped, with chained-DAG explicitly deferred.
- Sibling (cross-filtered selections runtime): `orbit/specs/2026-04-28-cross-filtered-selections-runtime/` — `Session::propagate_selection`, `selection_state`, brush-rect adapter, `SelectionDispatcher` trait, `on_mouse_up_with_dispatch`. Shipped.

---

## Context summary

Card 0005 v2 closed scenarios 1, 2, 3, 5, 6, 7 of the card *for direct, single-hop propagation*. What remains:

1. **Scenario 4 (chained params)** is unimplemented. v2's interview explicitly deferred the chained-DAG walk and said "Chained extraction has unresolved design questions (which column, which row, multi-row results). Ship the direct path first, learn from real usage" (interview.md Q2). The v1 decision pack D4 already chose topological propagation over flat or fan-out and committed the DAG and topo order to `SpecAnalysis` — but `propagate_param` ignores `analysis.topological_order` today (engine/lib.rs:441-447 looks up `subscriber_graph[name]` only).
2. **Widgets do not emit anything.** `InputKind::{Slider, Menu, Search, Table}` exist in `crates/brightfield-spec/src/vocab.rs:218-225` and are flagged `Unimplemented`. There is no `slider.rs`/`menu.rs` etc. in `crates/brightfield-ui/src/`. Brush release wires `propagate_selection` through a `SelectionDispatcher` trait (brush.rs:120-140) and `on_mouse_up_with_dispatch` (chart_view.rs:128-157). No analogue exists for the param coordinator — nothing in the UI layer calls `propagate_param`.
3. **`propagate_param`'s partial-failure shape predates `propagate_selection`'s.** Card 0005 v2's review (review-pr-2026-04-24-v2.md MEDIUM) flagged that v2's ac-04 cannot exercise mixed Ok/Err because no lowerer was registered. Card 0006 v2 ac-08 (cfs2_ac08) now does — with dot (supported) + rect (unsupported) — and is green. The same strengthening applies to params now that lowerers exist.
4. **The v2 review's "emit_query ignores param_values" LOW** has been *partially* closed: card 0006 v2 (cfs2_ac09) widened `emit_query` to consume both `param_values` and `selection_predicates` and updated all call sites (engine/lib.rs:354, 406, 475, 526). The plumbing is in place; the chained-DAG slice can rely on it.

Sprint goal: "Live reactivity — param widgets re-execute downstream queries and cross-filtered selections propagate across views, turning the static first-render into an interactive dashboard." Card 0006 v2 closed the selection half. This v3 slice closes the param half: chained DAG walk + at least one widget wired end-to-end.

Six decisions follow.

---

## Decision 1: Coordinator entry point — extend `propagate_param` or add `propagate_param_chain`?

### Context

`Session::propagate_param(name: &str, value: SpecValue)` is the existing entry point (engine/lib.rs:433). Its body does direct propagation only: state-update → look up `subscriber_graph[name]` → mark filter → per-subscriber emit + execute → results vec. To honour scenario 4, the coordinator must walk `analysis.topological_order` from the changed param, dispatch each level, and (per v1 D4) pass *epoch-consistent* param values to every emitted query within a single propagation. The decision is whether the chained walk extends the existing method's semantics or lives behind a new entry point.

### Evidence

- `analysis.topological_order: Vec<String>` is already populated by v1 (`crates/brightfield-spec/src/analysis.rs:838, 865, 885`).
- `analysis.subscriber_graph` is the same map keyed by *param* name and used by `propagate_param` today.
- v1 D4 chose topological propagation (decisions.md L92-114) and v1 ac-05 enforces a chain ordering: `[A, B]` for the two-level case.
- v2 interview Q2 deferred chained propagation but did not foreclose it — it explicitly said "v3" handles the DAG walk.
- `propagate_selection` (engine/lib.rs:262) added a *new* method rather than overloading anything — the codebase already accepts coordinator-per-axis surface growth.
- Scenario 4: "the coordinator walks the topological order: re-query for A's subscribers, then update B from the result, then re-query B's subscribers — no out-of-order execution." This describes a *single* propagation event, not a separate operation.

### Options

**A. Extend `propagate_param` in place — same signature, body becomes a topological walk.**
- Gains: One entry point. No widget-side branching ("did the analyst change a leaf or a chained param?"). The widget always calls `propagate_param(name, value)` and gets the right behaviour. Matches scenario 4's "the coordinator walks the topological order" framing.
- Loses: v2 callers of `propagate_param` (rpw2 tests, gomb_ac12 cache test, app shell) implicitly depend on the direct-only semantics. Strengthening the body to walk the DAG must preserve their behaviour for non-chained specs (a single param with no downstream params produces an identical results shape — decision 2 nails this down).

**B. Keep `propagate_param` direct-only; add `propagate_param_chain(name, value)` for chained walks.**
- Gains: Zero risk to v2 callers — direct propagation stays byte-for-byte the same. The chain entry point is opt-in.
- Loses: Two entry points the UI must choose between, with the choice depending on static analysis the UI shouldn't have to consult. Scenario 4 frames the chained behaviour as the *coordinator's* responsibility, not the caller's. Diverges from `propagate_selection`'s single-method shape.

**C. Internal split: keep `propagate_param` public; factor a private `propagate_chain_from(level_names, values)` helper.**
- Gains: Keeps the public surface tight while exposing internal seam for testing. The public method always walks; the helper is reusable for future chained-selection work if it materialises.
- Loses: One public method (same as A), one private helper (negligible cost).

### Trade-offs

A and C are functionally equivalent at the API surface — both honour scenario 4 with one call. C exposes a private helper that aids unit-testing the walk in isolation; A inlines the walk. B fragments the surface in a way the codebase explicitly rejected for selections (which never split direct vs chained). The v2 deferred work is *finishing the same method*, not adding a new one — the v2 spec goal sentence says "Direct propagation only … chained DAG walking is deferred to a future spec," not "chained propagation is a new method."

### Recommendation

**Option A (or C if a private helper aids testability).** Extend `propagate_param` in place to walk `analysis.topological_order` from the changed param. v2 callers see no behavioural change for direct-only specs (decision 2 enforces this). The widget always calls `propagate_param(name, value)`. Keep the `update_param` legacy method — it has its own callers (dex_ac06) and a stricter semantic (single-param ParamValues, not full param_state) that the cleanup memo can address later.

Cite: scenario 4; v1 decisions.md D4; v2 spec.yaml goal.

---

## Decision 2: DAG walk semantics — what does "topological propagation" actually compute at each level?

### Context

Decision 1 commits to a topological walk inside `propagate_param`. The semantically loaded question is: when the walk reaches param B (a downstream param whose *value* depends on the result of A's subscribing query), where does B's new value come from? v2 interview Q2 named this as the unresolved design question that justified deferral: "which column, which row, multi-row results."

Three structural cases exist in the corpus and the card scenarios:

**Case (i)** — Simple multi-subscriber (scenarios 1, 3): one param, multiple marks subscribe directly. No chaining. Already shipped.

**Case (ii)** — *Filtered widget chain*: an input widget has both `filter_by: $A` and `as_param: $B`. The widget's *visible options* depend on A; once the user picks an option, B receives a new value. **B's new value comes from the user's next interaction with the widget, not from A's query result.** This is `athletes.yaml` (search filtered by category). v1 D4 decisions.md L114 cites this exact corpus path.

**Case (iii)** — *Computed param chain*: param A drives a query whose *result* (a scalar derived from a RecordBatch) sets param B. No corpus example exists; no input widget kind in vocab.rs supports this; the AST has no `param: { from_query: ... }` form. v2 interview Q2 named this as the deferred case ("which column, which row, multi-row results").

### Evidence

- `analysis.dependency_dag` edges only fire when a single component both *consumes* a param (`filter_by` or `from`) and *writes* one (`as_param`). The edge-collection code is `analysis.rs:385-427` — only `Component::Input` produces edges, and only when both `as_param` and (`filter_by` OR an option containing a `ParamRef`) are present.
- No spec in `crates/brightfield-spec/vendor/mosaic-specs/yaml/` declares a param whose value is computed from a query result. The chain in athletes.yaml is case (ii): `filter_by: $category, as: $query` on the search widget.
- `ParamNode` has variants `Value(SpecValue)` and `Selection(SelectionNode)` only (ast.rs). No `ParamNode::FromQuery` exists.
- Scenario 4 says: "param A drives a query whose result sets param B" — phrased ambiguously, fits both case (ii) and case (iii).

### Options

**A. Topological re-execution only — do *not* derive new values from query results.** The walk processes params in topological order: at each level, every subscribing mark of that level's params is re-executed against the current `param_state`. Downstream params already in `param_state` (from prior `propagate_param` calls or initial defaults) ride along. *Case (ii)* is supported because the widget's `from`-source query re-fires (giving the user fresh options); when the user picks one, *that* triggers a separate `propagate_param(B, value)` call which walks B's subgraph. *Case (iii)* is unsupported by design — there is no AST surface for it.

**B. Topological re-execution + computed-param extraction.** Same as A, plus: when the walk crosses a node whose `ParamNode` (a hypothetical new `ParamNode::FromQuery` variant) declares a derivation expression, the coordinator extracts a value from the query's RecordBatch (e.g. `result[0].column("max_speed")[0]`) and writes it into `param_state` before continuing. Case (iii) becomes first-class.

**C. Bound-depth flat fan-out — single-level propagation only, even for chained params.** The walk runs one level deep; downstream params are not visited. Effectively v2's behaviour with no change. Scenario 4 fails for any case beyond direct.

### Trade-offs

- **A.** Matches every corpus spec. Honours scenario 4's "no out-of-order execution" by topological ordering of mark re-execution. The "param B updates from A's result" phrasing is satisfied by *re-querying* B's downstream subscribers with current state when A changes — which is what Mosaic's own coordinator does. Cost: scenario 4's literal phrasing ("update B from the result") could be read as case (iii); A interprets it as case (ii) and case-(iii) deferral. The interpretation is defensible because every corpus chain is case (ii). v2 interview Q2 already deferred (iii) for unresolved design.
- **B.** Closes the literal reading of scenario 4 but adds substantial AST and runtime surface for a case with zero corpus evidence. v2 interview Q2 itemised the open questions: which column? which row? multi-row results? scalar coercion rules? None of these have answers from the corpus, so a design committed now is speculative. Risks shipping a `ParamNode::FromQuery` we have to deprecate.
- **C.** Punts scenario 4 entirely. Fails the card.

### Recommendation

**Option A.** Walk `analysis.topological_order` filtered to params reachable from the changed param. At each level, dispatch each subscribing mark's emit + execute with the full `param_state`. *Case (ii)* (widget chains) works because the widget's `from`-source query re-fires; the next user interaction with that widget triggers a fresh `propagate_param(B, …)` which inherits the prior epoch's `param_state` for A. *Case (iii)* (computed params from query results) stays out of scope — defer until a corpus spec or an explicit user request motivates it. Document the deferral in the spec's implementation notes so the next sprint's review knows where the boundary lives.

The walk shape:

```
fn propagate_param(&mut self, name, value):
    self.param_state.insert(name, value)
    let downstream = topological_descendants(&analysis, name)  // [name, ...]
    let mut results = vec![];
    for level_param in downstream:
        for mark_idx in subscriber_graph[level_param] filtered to marks:
            emitted = emit_query(spec, mark_idx, Some(&param_state), selections_ref)?
            results.push((mark_idx, execute_emitted(...)))
    dedup mark_idx-already-dispatched-at-earlier-level
    return results
```

The dedup is necessary because a mark with `data.from: q` referencing both `$A` and `$B` is in *both* levels' subscriber lists; we want to re-execute it once at the deepest level (when both upstream params have settled — but they're settled at level 0 already since `param_state` is the source of truth).

Cite: scenario 4; v2 interview Q2; v1 decisions.md D4; absence of `ParamNode::FromQuery`.

---

## Decision 3: Per-walk dedup — first-level wins or last-level wins?

### Context

Decision 2's walk visits a mark once per level whose param it subscribes to. A mark with two upstream params (e.g. `WHERE country = $A AND year = $B` where B depends on A) appears twice. Re-executing twice is wasteful and breaks the "one fresh RecordBatch per affected mark" reading of scenario 4. Decision 3 fixes the dedup policy.

### Evidence

- `propagate_param` already dedups within a single subscriber list via `mark_indices.sort(); mark_indices.dedup();` (engine/lib.rs:456-457). This handles same-level duplicates from `subscriber_graph[name]` having repeated entries.
- The `param_state` is the single source of truth: by the time the walk starts, it has the new value for `name`; downstream params still hold their prior values until the walk re-emits queries that re-set them (case ii).
- For case (ii), the widget's `from`-source query re-fires at level 0 (subscribing to A); the widget's *displayed options* update; the user's next selection writes B; that's a separate `propagate_param(B)` call.
- DuckDB plan-hash cache (`Session.cache`, engine/lib.rs:131) already dedups *byte-identical* re-executions — but only at the SQL level, not the mark level.

### Options

**A. First-level-wins: dispatch each mark at the *earliest* level it appears in the topological walk; skip re-dispatch at later levels.**
- Gains: Maximally lazy — runs each mark once. Matches the "no out-of-order" reading because by the time we reach level N, level N-1's marks have already been dispatched against the same `param_state`.
- Loses: A mark whose query references *both* A and B (where B's value is yet to be set in the same propagation) would dispatch with stale-B before B's level. But this case requires (iii) which is out of scope (decision 2). For case (i)/(ii), `param_state` for B is unchanged across the walk, so first-level dispatch is correct.

**B. Last-level-wins: defer each mark's dispatch to the latest level it appears in.**
- Gains: Maximally deferred — guarantees that *all* upstream params have been visited before the mark fires.
- Loses: Visit-order reasoning is more complex; for case (ii)/(i) the result is identical to A; the only case where "all upstream visited" matters is case (iii). Carrying complexity for an out-of-scope case.

**C. No dedup — re-execute at every level the mark subscribes to.**
- Gains: Simplest implementation. DuckDB's plan-hash cache makes the second execution effectively free (same SQL → same prepared statement → cache hit per gomb_ac12).
- Loses: The result vec carries duplicate `(mark_idx, Result)` entries. Callers (the renderer) must dedup. The shape diverges from `propagate_selection`'s "one Result per subscriber".

### Trade-offs

A and B converge for in-scope cases. C punts the dedup to the caller and silently double-renders. The renderer in `brightfield-render` is the consumer here; it would have to learn that "the same mark appears twice — use the second" or "use the first" — duplicating the dedup logic the coordinator already owns.

### Recommendation

**Option A (first-level-wins).** Maintain a `dispatched_marks: HashSet<usize>` across the walk; before dispatching a mark, check membership and skip if already present. This produces a result vec with the same shape as v2's `propagate_param` and as `propagate_selection`: one `(mark_idx, Result)` per affected mark, in topological order of first appearance. Aligns with `propagate_selection`'s single-pass shape.

Cite: gomb_ac12 (proves cache efficiency); engine/lib.rs:456-457 (existing in-level dedup pattern).

---

## Decision 4: Partial-failure isolation — match `propagate_selection`'s shape exactly?

### Context

`propagate_param` v2 isolates emit failures with the `continue` pattern (engine/lib.rs:475-481). The v2 review's MEDIUM finding noted that with no registered lowerers, the test couldn't exercise *mixed* Ok/Err — both subscribers were Err. Card 0006 v2 (cfs2_ac08) now does, using dot (supported) + rect (unsupported lowerer). The v3 slice should strengthen ac-04 and confirm parity.

### Evidence

- `propagate_param`'s `continue` on `EmitFailed` is at engine/lib.rs:475-481.
- `propagate_selection`'s identical pattern is at engine/lib.rs:319-324.
- `cfs2_ac08_partial_failure` exercises mixed Ok/Err with rect (unsupported) + dot (supported); rect produces `Err(EmitFailed { cause: UnsupportedMark })`, dot produces `Ok(batches)`.
- Lowerers are now registered (the "no lowerers registered" v2 limitation is gone — the conformance/render slice landed lowerers for dot, line, bar, density, regression).
- Scenario 7: "one mark's re-query fails (e.g. DuckDB error) and the other succeeds — the successful mark gets fresh data, the failed mark retains its previous state, and a warning surfaces the error."

### Options

**A. Match `propagate_selection` exactly: per-subscriber `Result`, `continue` on emit/execute error, `param_state` always updated regardless of subscriber outcomes. Across-level: an Err at level N does not abort levels N+1...**
- Gains: Symmetry with selections coordinator. Scenario 7 is a structural property of the loop, not a new mechanism. Single execution failure does not propagate up the DAG — downstream subscribers still see updated state.
- Loses: Nothing — this is the existing v2 shape, just exercised more deeply by the chained walk and the new mixed-result test.

**B. Abort the rest of the walk on any Err — bail out at the first level that fails.**
- Gains: Atomic-ish — either everything succeeds or the walk stops cleanly.
- Loses: Diverges from `propagate_selection`. Violates scenario 3 ("no stale view is left behind") for the unaffected branches of the DAG. Non-composable.

**C. Distinguish emit-error from execute-error — continue on the former, abort on the latter.**
- Gains: Treats genuine DuckDB errors as fatal while being permissive about author-error (unsupported mark).
- Loses: Two failure regimes at the same boundary; users have no way to map back to which is which without inspecting the EngineError discriminant. `propagate_selection` doesn't distinguish.

### Trade-offs

A is the established project pattern. The v2 review's MEDIUM is closed structurally now that lowerers are registered — the v3 slice can simply add a strengthened ac-04 test using dot + rect (mirroring cfs2_ac08).

### Recommendation

**Option A.** Per-subscriber `Result`; `continue` on emit or execute error; `param_state` always updated; walk continues across levels regardless of per-mark errors. Strengthened ac-04 test uses dot (supported) + rect (unsupported lowerer) and asserts `results.len() == 2` with one Ok + one Err. Optionally surface a warning channel — the v2 spec called for "a warning surfaces the error" but did not define a mechanism. Recommend deferring the warning surface to a separate slice (it's a UI concern; the result vec already carries the Err, and the renderer/app can log on observation). Document this as a known follow-up.

Cite: scenario 7; cfs2_ac08; v2 review-pr MEDIUM.

---

## Decision 5: Widget→coordinator wiring — slider only, all four widgets, or trait-only?

### Context

Card scenario 2 names slider, menu, search, and table as first-class param emitters. The selections runtime (cfs2_ac10/ac11) wired only one input source — the brush — through a `SelectionDispatcher` trait, `BrushBinding` struct, and `on_mouse_up_with_dispatch`. The param coordinator needs an analogous integration point. The decision is which input widget(s) actually land *in this card* and which deferred.

### Evidence

- No widget code exists in `crates/brightfield-ui/src/` — `ls` returns `brush.rs`, `chart_element.rs`, `chart_layout.rs`, `chart_state.rs`, `chart_view.rs`, `interaction.rs`, `lib.rs`, `vello_renderer.rs`. No `slider.rs`, `menu.rs`, `search.rs`, `table.rs`.
- `InputKind::{Menu, Search, Slider, Table}` are all `Unimplemented` (vocab.rs:218-225).
- The selections wiring used `SelectionDispatcher` trait + `RecordingDispatcher` test double + `on_mouse_up_with_dispatch` method (brush.rs:120-140, chart_view.rs:128-157). The `Session` impl forwards to `propagate_selection` (brush.rs:131-140).
- Sprint goal: "param widgets re-execute downstream queries" — singular "widgets" is plural but does not specify breadth.
- Card scenario 2 is a *behavioural* claim ("each widget emits the new param value to the coordinator"); it can be satisfied by one widget plus the trait that lets the others slot in.
- Slider is the canonical Mosaic example and the simplest UI shape (one float, monotonic dragging). Menu and search require a populated `from`-source query (more wiring). Table requires a sortable + load-on-scroll virtualised list (substantial UI work).
- The vocab status flag `Unimplemented` is conventionally promoted to `Implemented` only when the widget actually runs end-to-end.

### Options

**A. Slider-only: ship a working `Slider` GPUI widget that calls `propagate_param` via a `ParamDispatcher` trait. Promote `InputKind::Slider` to `Implemented`. Menu/Search/Table remain `Unimplemented` and out of scope.**
- Gains: One widget gives end-to-end evidence (matches the rally pattern: "first end-to-end render" shipped one chart, not all marks). Slider is the shortest path to a demoable interactive dashboard. The `ParamDispatcher` trait lets future widgets drop in.
- Loses: Card scenario 2 names four widgets; shipping one means the scenario is partially satisfied at the *runtime* layer (any widget *can* dispatch, slider *does*).

**B. All four widgets: ship Slider, Menu, Search, Table GPUI widgets in this slice.**
- Gains: Card scenario 2 fully satisfied at the UI layer.
- Loses: Massive UI scope. Table alone is multiple PRs (sorting, virtualised scrolling, load-on-scroll). Menu and search need data-binding (their option lists come from `from`-source queries). Sprint goal compromised. The selections rally shipped only the brush — not all interactor kinds — for the same reason.

**C. Trait-only: ship `ParamDispatcher` trait + `Session` impl + a recording test double, no real widget. Defer slider to next sprint.**
- Gains: Matches the structural shape of selections wiring without committing UI work. Test coverage proves the dispatch path.
- Loses: No actual interactive widget — the card's scenario 2 ("when I interact with any widget") cannot be exercised. Sprint goal explicitly says "param widgets re-execute downstream queries" — no widgets ⇒ no live reactivity demo.

### Trade-offs

A is the rally precedent: ship one path end-to-end, leave structurally analogous paths to follow. B over-extends. C ships infrastructure with no end-user evidence. The selections runtime (cfs2_ac11) shipped one input source (brush) and proved the dispatch shape; the param runtime should mirror that — one widget (slider), proven dispatch shape.

### Recommendation

**Option A.** Ship slider end-to-end:

1. New `crates/brightfield-ui/src/slider.rs` — GPUI widget rendering a track + thumb, mouse handlers (down/move/up), value bound to `(min, max, step)` declared from the `Input.options` bag.
2. `ParamDispatcher` trait in `slider.rs` (or a new `crates/brightfield-ui/src/param.rs`) mirroring `SelectionDispatcher`:
   ```rust
   pub trait ParamDispatcher {
       fn dispatch(&mut self, name: &str, value: SpecValue)
           -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>;
   }
   impl ParamDispatcher for brightfield_engine::Session {
       fn dispatch(&mut self, name: &str, value: SpecValue) -> ... {
           self.propagate_param(name, value)
       }
   }
   ```
3. `SliderBinding { param_name: String, min: f64, max: f64, step: Option<f64> }` analogous to `BrushBinding`.
4. `on_value_change` (or `on_mouse_up_with_dispatch` analogue) — slider commits its value to the dispatcher on drag release (debounced — see decision 6).
5. Promote `InputKind::Slider` to `ImplStatus::Implemented` in vocab.rs.

Menu, Search, Table remain `Unimplemented` and are deferred to a future sprint candidate. Document this in the spec's implementation_notes so the boundary is explicit. The `ParamDispatcher` trait is a contract the future widgets adopt without coordinator changes.

Cite: cfs2_ac10/ac11 precedent; sprint goal singular "live reactivity"; absence of widget code.

---

## Decision 6: Re-render integration — coordinator returns RecordBatches, or dispatches render directly?

### Context

`propagate_param` returns `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`. `propagate_selection` returns the same shape. The renderer (`brightfield-render`) consumes RecordBatches via `render_mark`-style entry points; the GPUI window (`brightfield-app`) drives a `ChartView` over a `ChartState`. None of the engine's `propagate_*` methods *render* — they return data and let the caller decide.

The card's scenario 1 says "re-queries and re-renders." Scenario 6 says "returns fresh RecordBatches for the affected marks." These are not contradictory — re-rendering is the caller's job; the coordinator's job is to deliver fresh batches.

### Evidence

- `propagate_param` returns batches; the v2 spec's ontology_schema lists `propagation_result` as the return shape (spec.yaml L132-134).
- `propagate_selection` returns batches; the cfs2 spec ac-12 verifies row-count reduction, not pixel-level rendering.
- `ChartView` does not currently consume `propagate_*` results — it reads `ChartState::scene()` (chart_view.rs:46-53). The scene is built from RecordBatches by the render layer.
- The "first end-to-end render" shipping memo (`orbit/cards/memos/2026-04-29-first-render-followups.md`) noted that the path "RecordBatch → Vello scene → GPU texture" is one direction; reversing it (mutating the scene from outside the GPUI ChartState entity) is a deferred concern.
- Slider drag will fire many `propagate_param` calls per second — debounce/throttle is a UI concern (the decision 5 of the selections pack chose UI-side debounce mirroring `NavigationState.check_settle`).

### Options

**A. Coordinator returns batches; caller (app shell or `ChartView` integration) re-renders.** Same shape as v2 and `propagate_selection`. The slider widget's dispatcher invocation returns the result vec; the app shell observes it (or the slider widget passes it back to a `ChartView::on_param_dispatch` method) and updates `ChartState`.
- Gains: Symmetry with all existing coordinators. No engine→render dependency. Pure data flow at the engine boundary.
- Loses: Caller must wire the "result vec → ChartState update" path. This is the same wiring problem the renderer faces for selections — sufficient evidence that this is a *one-time* integration, not a per-coordinator burden.

**B. Coordinator dispatches render directly via a callback / `RenderSink` trait stored on Session.** When `propagate_param` finishes a mark's execute, it calls `sink.render_mark(idx, batches)` synchronously.
- Gains: Single call drives the whole pipeline.
- Loses: Engine gains a render dependency (or a `RenderSink` trait it must define); brightfield-render's no-gpui invariant becomes harder to keep when render eventually wants per-mark scene caching. Same as B in cross-card decisions.

**C. Coordinator returns a `PropagationOutcome` struct that includes both batches and a "render hints" payload.**
- Gains: Future-proof for partial re-render (only update the marks that changed).
- Loses: Premature abstraction. v2 and cfs2 both ship the bare batches vec; no consumer is asking for hints yet.

### Trade-offs

A keeps the engine pure. B couples engine and renderer. C carries no current weight. The first-render memo's follow-ups list "literal channel values, vocab/runtime alignment, execution-conformance test layer" — none of which require coupling renderer into engine. The render layer's job is to consume `Vec<RecordBatch>` and produce a scene; the coordinator's job is to produce those batches per affected mark.

### Recommendation

**Option A.** `propagate_param` continues to return `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`. The slider widget invokes its `ParamDispatcher` (which forwards to `Session::propagate_param`); the dispatcher result is observed by the app shell or `ChartView` in a way that updates `ChartState` and triggers a re-render via `cx.notify()`. The exact wiring (whether `ChartView` exposes a `propagate_param_and_redraw(name, value)` helper, or whether the app shell composes the two calls itself) is an implementation detail — the test pattern from cfs2_ac11 (a `RecordingDispatcher` test double) covers the dispatch-was-invoked property; the render-was-triggered property is a pure UI test using `cx.notify()` observation.

UI-side debounce: slider drags fire `propagate_param` only on `mouse_up` (matching the brush's `on_mouse_up_with_dispatch` discipline). Mid-drag is overlay-only — the slider thumb tracks the cursor purely in UI state, no engine call. This mirrors decision 5 of the cfs2 pack ("Sync coordinator + UI debounce on brush release").

Cite: cfs2 decisions.md D5; first-render-followups memo; sprint goal.

---

## Cross-card disjointness check (with card 0006 v2)

The selections rally and the param rally both touch `Session`. To avoid stepping on shipped code, every shared symbol this slice touches:

### Session fields touched

- `param_state: ParamValues` — this slice **reads and writes** (already done by v2). No structural change.
- `selection_state: HashMap<...>` — this slice **reads only** (the chained walk passes the live `selection_state` to `emit_query` so re-executions honour the active brush; same as v2 already does at engine/lib.rs:467-472). **No write.**
- `analysis: SpecAnalysis` — **reads only** `subscriber_graph` and `topological_order`. Both already exist; no schema change.
- `mark_index_map: HashMap<...>` — **reads only**. No structural change.

### Session methods this slice modifies

- **`propagate_param`** — body changes from direct dispatch to topological walk (decision 1+2). Signature unchanged. Existing rpw2 tests must pass; gomb_ac12 must pass; dex_ac06 (which uses the older `update_param` not `propagate_param`) is unaffected.
- **`update_param`** — **untouched.** Legacy single-param dispatch; has its own callers (dex_ac06). A future cleanup memo can deprecate it.

### Session methods this slice adds

- None beyond optional private helpers (`topological_descendants`, `dispatched_marks` set). Public API is additive-only at the trait layer (decision 5: `ParamDispatcher`).

### Methods on `emit_query` / `emit_query_with_passes`

- **Untouched.** Card 0006 v2 widened the signature to consume both `param_values` and `selection_predicates`. This slice **uses** that surface (passes `Some(&self.param_state)` and `selections_ref`) but does not change it.

### Symbols added in this slice

- `crates/brightfield-ui/src/slider.rs` (new file) — `Slider` GPUI widget, `SliderBinding`, `ParamDispatcher` trait, `Session: ParamDispatcher` impl. Mirrors `brush.rs`'s `SelectionDispatcher` shape.
- `crates/brightfield-ui/src/lib.rs` — re-exports for the slider module.
- Optional helper `topological_descendants(analysis: &SpecAnalysis, root_param: &str) -> Vec<String>` — lives in `brightfield-spec::analysis` (next to `build_dependency_dag`) so both crates can use it. Pure function over the DAG.
- Vocab change: `InputKind::Slider` flips from `Unimplemented` to `Implemented` (vocab.rs:222). Affects nothing in `propagate_*`; affects spec status reporting only.

### Test prefix

- **`rpw3_`** for this slice's tests, mirroring the v1→v2 prefix evolution (`rpw_` → `rpw2_` → `rpw3_`). Locale: `crates/brightfield-engine/src/lib.rs` for engine tests, `crates/brightfield-ui/src/slider.rs` for slider tests, `crates/brightfield-spec/src/analysis.rs` for `topological_descendants`.

### Disjointness with cfs2 confirmed

No symbol added by cfs2 is mutated. No file added by cfs2 is rewritten. The `selections_ref` plumbing inside `propagate_param` remains identical. The `SelectionDispatcher` trait is not extended — `ParamDispatcher` is a sibling, not a refactor target.

---

## Decision summary table

```
| # | Decision                                      | Recommendation                                                                       |
|---|-----------------------------------------------|--------------------------------------------------------------------------------------|
| 1 | Coordinator entry point                       | Extend `propagate_param` in place; topological walk replaces direct-only dispatch    |
| 2 | DAG walk semantics                            | Topological re-execution against full param_state; computed-param case (iii) deferred|
| 3 | Per-walk dedup                                | First-level-wins via `dispatched_marks: HashSet<usize>` across the walk              |
| 4 | Partial-failure isolation                     | Match `propagate_selection` exactly; strengthen ac-04 with dot+rect mixed Ok/Err      |
| 5 | Widget→coordinator wiring                     | Slider only; `ParamDispatcher` trait; promote `InputKind::Slider` to `Implemented`   |
| 6 | Re-render integration                         | Coordinator returns batches; UI observes result vec and re-renders via `cx.notify()` |
```

---

## Cross-cutting implementation notes (consequences of the decisions, not decisions themselves)

- **`topological_descendants` helper** lives in `brightfield-spec::analysis` (pure DAG traversal), takes `&SpecAnalysis` and a root param name, returns the slice of `analysis.topological_order` reachable from the root via DAG edges, with the root included as the first element. Trivial — Kahn-style traversal restricted to descendants of the root node, ≤30 lines.
- **Slider widget lives in `crates/brightfield-ui/src/slider.rs`** alongside `brush.rs` and `chart_view.rs`. Implements GPUI mouse-down / move / up handlers; tracks a `value: f64` in widget state; on mouse-up calls `dispatcher.dispatch(param_name, SpecValue::Float(value))`. Renders a horizontal track + thumb; min/max/step from the spec's `Input.options` bag (e.g. `min: 0, max: 100, step: 1`).
- **Slider→ChartState wiring** is the equivalent of the brush→ChartView wiring. The app shell holds the `Session` (already true for the brush via the `SelectionDispatcher` impl) and routes the slider's dispatch result back to update `ChartState`. Concrete shape: a `paramatic_chart_view` integration test or app-shell helper that observes `propagate_param` results and updates the ChartState entity. This wiring is a UI-level test fixture (mirrors cfs2_ac11's `RecordingDispatcher` plus a real-Session integration variant).
- **Cycle detection at static-analysis time already exists** (v1 ac-06; analysis.rs:364-380). The runtime walk does not need to defend against cycles — `analysis.topological_order` is well-formed by construction.
- **The "warning surfaces the error" requirement of scenario 7** is partially satisfied by the existing Err entry in the result vec. A richer surface (e.g. a session-level warning channel returning text strings on each propagation) is a follow-up — propose it as a separate slice if author feedback emerges.
- **Vocab status flip for `InputKind::Slider`** (`Unimplemented` → `Implemented`) requires a conformance check: `crates/brightfield-spec/src/vocab.rs:218-225` enumerates all kinds with their statuses. The flip is a small change but should be paired with a conformance test that asserts `InputKind::Slider.status() == ImplStatus::Implemented` so future regressions are caught.
- **Test target: ≥10 new `rpw3_` tests** — matching v2's count (the v2 review counted 10 rpw2_ tests). Coverage:
  - Engine: `rpw3_ac01_topological_walk_chain`, `rpw3_ac02_walk_dedup_first_level_wins`, `rpw3_ac03_partial_failure_mixed_ok_err`, `rpw3_ac04_walk_against_unsubscribed_param_no_op`, `rpw3_ac05_walk_descendants_only_not_unrelated_params`, `rpw3_ac06_widget_dispatcher_trait_forwards_to_propagate_param`.
  - Spec: `rpw3_ac07_topological_descendants_simple`, `rpw3_ac08_topological_descendants_athletes_yaml_chain`.
  - UI: `rpw3_ac09_slider_on_mouse_up_dispatches_param`, `rpw3_ac10_slider_no_drag_no_dispatch`.
- **Corpus regression gate** must remain green. No AST changes; no parser changes; only vocab status flip (which is wire-compatible). The cfs ac-13 corpus iteration test continues to be the trip-wire.
