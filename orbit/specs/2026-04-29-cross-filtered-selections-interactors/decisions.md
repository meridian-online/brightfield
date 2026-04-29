# Decision Pack: Cross-Filtered Selections — Interactor Surface & Lifecycle (v3)

**Card:** `orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml`
**Date:** 2026-04-29
**Slice:** v3 — interactor surface coverage and lifecycle correctness
**Rally:** `orbit/specs/2026-04-29-live-reactivity-rally/rally.yaml` (paired with card 0005 v3)
**Prior art:**
- v1 static analysis shipped at commit 4dd422e (`orbit/specs/2026-04-21-cross-filtered-selections-across-linked-views/`)
- v2 runtime coordinator shipped at commit 8ca4283 / approved in `review-pr-2026-04-28.md` (`orbit/specs/2026-04-28-cross-filtered-selections-runtime/`)

---

## Context summary — what is already built

The v2 runtime coordinator is fully shipped. The relevant code surface, audited fresh:

- `Session::propagate_selection(name: &str, contributor: ComponentPath, predicate: Predicate) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>` lives at `crates/brightfield-engine/src/lib.rs:262-335`. It (a) inserts/replaces the `(contributor, predicate)` entry in `selection_state[name]`; (b) looks up subscribers from `analysis.selection_subscribers`; (c) filters to mark components via `mark_index_map`; (d) calls `emit_query` per subscriber, which internally invokes `compile_selection` with `self_source = parent_plot(mark_path)`; (e) returns a per-subscriber result vec (`continue` on error).
- `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>` at `lib.rs:152`. Initialised empty in `Engine::load_spec` at `lib.rs:104`. Accessor: `Session::current_selections()` at `lib.rs:226`.
- `selection_predicates_for_emit()` at `lib.rs:235-246` stringifies `ComponentPath` into the shape `emit_query` consumes: `Vec<(String, Vec<(String, Predicate)>)>`.
- `compile_selection(selection: &SelectionNode, self_source: &str, predicates: &[(String, Predicate)]) -> Predicate` at `crates/brightfield-sql/src/lower.rs:341-374`. Already implements all four resolution strategies (Crossfilter / Intersect / Union / Single) and crossfilter self-exclusion via `source != self_source` byte equality.
- `emit_query(spec, mark_index, param_values, selection_predicates) -> Result<EmittedQuery, EmitError>` at `crates/brightfield-sql/src/emit.rs` — both `param_values` and `selection_predicates` are now actually consumed (LOW finding from card 0005 v2 closed in v2 of this card). The selection slice carries `Option<&[(String, Vec<(String, Predicate)>)]>`.
- `parent_plot(path: &str) -> &str` at `crates/brightfield-spec/src/analysis.rs` — byte-scan for the longest prefix ending in `/plot[<digits>]`, or the input unchanged.
- `propagate_param` at `lib.rs:433-489` — explicitly threads `self.selection_predicates_for_emit()` through `emit_query` (`lib.rs:467-475`). The doc comment at `lib.rs:464-466` reads: "Selection predicates are threaded from the live selection_state so a propagate_param call after a brush release continues to honour the active selection (correctness over micro-optimisation)."
- `analysis.selection_subscribers: SelectionSubscriberGraph` (alias for `HashMap<String, Vec<ComponentPath>>`) at `analysis.rs:690`. Built by `build_selection_subscriber_graph` (`analysis.rs:695-717`) — seeds entries from declared `params: { name: { select: ... } }` AND from `as: $name` interactor/input bindings (creating implicit selections), then walks the component tree collecting filterBy refs.
- `analysis.interactor_bindings: Vec<InteractorBinding>` at `analysis.rs:842`. Each binding records `path: ComponentPath` (the full interactor path) and `selection: String`. **Importantly**: today the binding does NOT carry `kind: InteractorKind`, channel columns, or any metadata about *what kind of contribution* the interactor produces.
- UI side: `BrushBinding { selection_name, contributor: ComponentPath, kind: BrushKind, channels: ChannelColumns }` at `crates/brightfield-ui/src/chart_view.rs:165-174`, `brush_rect_to_predicate` at `crates/brightfield-ui/src/brush.rs:81-115`, `SelectionDispatcher` trait at `brush.rs:120-129`. ChartView's `on_mouse_up_with_dispatch` at `chart_view.rs:128-157` dispatches one `propagate_selection` call on Brushing→Idle. Today **the app shell does not yet construct or pass a `BrushBinding`** — it lives in tests only (`grep "BrushBinding"` in `brightfield-app/` returns zero matches).

### What this card extends

Card 0006 v3 closes three gaps:

1. **Clearing** (scenario 4) — there is no path today by which a contributor can *retract* its predicate. `propagate_selection` only inserts/replaces. Click-outside-the-brush-region is unhandled (`on_mouse_up` only commits a brush; an idle click does nothing).
2. **Multi-selection-per-plot** (scenario 5) — `BrushBinding` is *singular* (one selection name, one BrushKind, one channel set). The corpus has `protein-design.yaml` (an `intervalXY` interactor on `$query` AND a `filterBy: $query` table input writing `as: $point`) and `athletes.yaml` (intervalXY on `$query` plus a table feeding `$hover`) where a single plot participates in multiple selections via different *components*. The card scenario explicitly says "a plot is bound as contributor to two distinct selections (e.g. a point selection on one channel and an interval selection on another)" — the plot, not just the spec, is the unit. This is per-component within one plot today; the card stretches it to potentially per-channel within one interactor.
3. **Persistence across param changes** (scenario 6) — `propagate_param` already preserves `selection_state` (verified at `lib.rs:464-466`). What is *not* yet decided is what happens when the active selection's *bound domain* drifts because a param changed the data. The current shape is "predicate persists verbatim"; there is no auto-clear, no warn, no domain check.

### Disjointness with card 0005 v3

Per the rally `rally.yaml`, this card is paired with `0005-reactive-parameters-with-input-widgets` v3 (`orbit/specs/2026-04-29-reactive-parameters-runtime/`). The shared symbols this card touches **must be enumerated** for disjointness review:

```
| Symbol                              | Crate                    | This card's intent                            | Card 0005 v3's intent (per rally context)      |
|-------------------------------------|--------------------------|-----------------------------------------------|------------------------------------------------|
| Session::propagate_param            | brightfield-engine       | READ behaviour only (already preserves        | Likely extends with input-widget driven        |
|                                     |                          | selection_state); add test asserting it       | dispatch; field is shared, body is not — no    |
|                                     |                          | does not clear.                               | conflict if 0005 does not clear selection_state|
| Session::selection_state            | brightfield-engine       | Read AND write (clearing path)                | Read-only (forwards through emit_query)        |
| Session::selection_predicates_for_emit | brightfield-engine    | Read; possibly extend to skip empty-after-    | Read-only                                      |
|                                     |                          | clear entries                                 |                                                |
| analysis.selection_subscribers      | brightfield-spec         | Read-only (no schema change)                  | Read-only                                      |
| analysis.interactor_bindings        | brightfield-spec         | Likely EXTEND with `kind` (and possibly      | Touched only if input widgets need binding-    |
|                                     |                          | channel info) — see Decision 4                | metadata propagated; risk surface              |
| chart_view.on_mouse_up*             | brightfield-ui           | Add click-outside-clear path                  | Untouched (input widgets render in their own  |
|                                     |                          |                                               | views, not chart_view)                         |
| BrushBinding                        | brightfield-ui           | Either add a list of bindings or generalise   | Untouched                                      |
|                                     |                          | (Decision 3)                                  |                                                |
| ParamValues / param_state           | brightfield-engine       | Untouched (selection state lives on its own  | Read AND write (full focus of 0005 v3)         |
|                                     |                          | field)                                        |                                                |
| emit_query signature                 | brightfield-sql          | UNCHANGED — already takes both param_values  | UNCHANGED — already takes both                |
|                                     |                          | and selection_predicates                      |                                                |
```

Both cards converge on `selection_predicates_for_emit()` and `param_state` ↔ `selection_state` independence. The contract this card commits is: **`propagate_selection` does not touch `param_state`; the inverse (no `propagate_param` mutation of `selection_state`) is asserted in v2's existing implementation and re-verified by a regression test in this slice (Decision 6).** That is the rally seam.

Six decisions follow.

---

## Decision 1: Clearing — interactor surface and coordinator API

### Context

Scenario 4 says "I dismiss the selection — clicking outside the brushed region or otherwise clearing it." The runtime coordinator today only inserts; there is no retract path. The decision has two halves: (a) what the coordinator API for clearing looks like (a dedicated method? a sentinel predicate? remove-by-contributor?); (b) how the UI exposes the clear gesture (click-outside-brush, Escape key, dedicated clear button, or coupling to input widget reset).

Evidence:
- `propagate_selection` at `lib.rs:271-275` does linear-scan replacement of `(contributor, predicate)`. It cannot remove — only overwrite.
- The IR has `Predicate::True` (no-op filter) and `Predicate::False` (filter-everything-out) at `crates/brightfield-sql/src/ir.rs:50-52`. Neither is the "absent" signal — both still produce a stored entry.
- `compile_selection` at `lower.rs:357-362`: when the per-subscriber filtered list is empty the result is `Predicate::True` (no filter). So **removing a contributor entry is mechanically equivalent to "this contributor's brush is gone"** — exactly the scenario-4 outcome.
- `chart_view.on_mouse_up` at `chart_view.rs:107-114` already handles "no brush in flight" — it stays Idle and does not dispatch. Today a click that does not become a drag never reaches the coordinator.
- The card text mentions specifically "clicking outside the brushed region" — an idle click on the chart, when a brush is currently displayed (whose state lives in `selection_state`, not `InteractionState`), should retract the contribution.
- Mosaic's `empty: true` in `params: { hover: { select: intersect, empty: true } }` (athletes.yaml:11, protein-design.yaml:36) is a static option saying "when no contributors, filter to nothing rather than everything." It is not a runtime clear signal.

### Options

**A. Add `Session::clear_selection(name: &str, contributor: ComponentPath) -> Vec<(usize, Result<…>)>` — symmetric to `propagate_selection`, removes the `(contributor, predicate)` entry from `selection_state[name]` (linear-scan find + remove), then dispatches re-emit/re-execute to all subscribers (the now-shorter contributor list flows through `compile_selection` naturally).**
- Gains: Mirrors `propagate_selection`'s shape exactly — same return type, same dispatch loop, same partial-failure pattern. Reuses every line of the existing dispatch machinery. The "absence" semantics live where they belong (the contributor list shrinks). Test surface mirrors `cfs2_ac02-08`. Trivial UI integration: `on_mouse_up_with_dispatch` already knows the `BrushBinding`'s contributor and selection name; a sibling method `commit_brush_clear` calls `dispatcher.clear` instead of `dispatcher.dispatch`.
- Loses: One more method on Session (now three: `propagate_param`, `propagate_selection`, `clear_selection`). The dispatcher trait grows a `clear` method or a new method (or `dispatch` takes an `Option<Predicate>`). Trait churn is small but real.

**B. Overload `propagate_selection` — pass `Predicate::True` to mean "clear my contribution." The coordinator detects `Predicate::True` and removes instead of inserting.**
- Gains: One method, one API. No trait change.
- Loses: Conflates "no-filter" with "no-contribution." A spec might legitimately set `Predicate::True` as a contributor's predicate (e.g. an `interval` brush over the full domain). Today that's never produced, but coupling the wire to the API would later forbid it. Stringly-typed sentinel — exactly the smell the v2 decisions document warned against (Decision 1 of v2: "Stringly-typed payload — every consumer parses an Object back into a Predicate").

**C. Add `Selection::Drop { contributor }` as a new payload variant on a `SelectionUpdate` enum — coordinator takes `propagate_selection(name, SelectionUpdate)` instead of a positional predicate.**
- Gains: Future-proofed for richer update kinds (e.g. point-add-to-set, point-remove-from-set, interval-extend).
- Loses: API churn for v2's existing call sites and tests (`cfs2_ac02..ac08`, `cfs2_ac11`, the `RecordingDispatcher` test double, and the `SelectionDispatcher` trait). v2 just shipped; reshaping the entry-point three weeks later is gratuitous given there is no second update kind on the horizon.

### Recommendation

**Option A.** A dedicated `clear_selection(name, contributor)` is the minimal-surface change with the cleanest semantics. The trait `SelectionDispatcher` grows one method (`clear`); existing tests and call sites are untouched. The UI wires a click-outside-the-brush handler in chart_view that calls `dispatcher.clear(...)` when (i) `InteractionState::Idle` is current, (ii) a click lands inside the plot area, and (iii) the chart's binding has a previously-dispatched brush still active for that selection (tracked by a small `last_dispatched` cell on ChartView, or by reading `Session::current_selections`). Two new ACs cover this:
- `cfs3_ac01_clear_selection_removes_contributor`: insert, then clear, assert `selection_state[name]` no longer contains that contributor; subscribers re-execute and their RecordBatches reflect the absence (e.g. row count returns to baseline).
- `cfs3_ac02_clear_selection_unsubscribed_silent`: clearing a selection with no contributors entry is a no-op (mirrors v2 ac-07).

A second, equally-important AC is the chart_view wiring — `cfs3_ac03_click_outside_active_brush_clears`: simulate mouse-down + mouse-up at the same point (no drag), with a previously-active brush on the same plot, and assert the dispatcher records exactly one `clear` call.

The card text also mentions "or otherwise clearing it." Keyboard Escape is a natural second gesture; the spec should include a placeholder AC `cfs3_ac04_escape_key_clears_active_brush` only if escape-key handling already routes through chart_view (verify during spec; if it doesn't, defer to a follow-up).

---

## Decision 2: Point selection — does click-to-select belong on chart_view, or on input widgets?

### Context

Scenario 5's example is "a point selection on one channel and an interval selection on another." The card glosses on what *produces* a point selection. In Mosaic vocabulary, point selections are typically driven by:
- `input: table` widgets — a row click in a table writes a row predicate to a selection (athletes.yaml:67-68: `filterBy: $query, as: $hover`; protein-design.yaml:147-148: `filterBy: $query, as: $point`).
- `Toggle`-family interactors (toggle / toggleX / toggleY) — declared in `vocab.rs:203-205` but `ImplStatus::Unimplemented`.
- `Nearest`-family interactors — already `Implemented` in vocab (`vocab.rs:200-202`) for highlight/hover use cases (card 0010), and they emit a per-row identity rather than a range predicate.

Today: there is no input-widget runtime (`grep Component::Input` in engine returns no matches in the dispatch path), no Toggle handling, no row-click-on-mark handling in chart_view. The corpus uses `input: table` for point selections almost universally; the card's "point selection on one channel" framing suggests a chart-side gesture (click a dot to write a row predicate).

### Options

**A. Defer point-via-interactor to a future card; treat `input: table` row clicks as the canonical point-selection driver in this slice. Implement only Toggle-family interactors in chart_view if explicitly required by the card text.**
- Gains: Stays in the card's literal scope. The corpus's actual point-selection pattern (`input: table` writing `as: $point`) already shapes the analysis layer (`InteractorBinding` records inputs the same way as interactors via `validate_interactor_bindings`, but inputs are walked in `collect_selection_subscribers` at `analysis.rs:751-759`). The runtime side is already a coordinator-shaped problem: an input row click → `propagate_selection(name, contributor, point_predicate)`. No chart_view changes needed for this AC. Tests can drive the input directly through a test double dispatcher exactly like the `RecordingDispatcher` already in place at `chart_view.rs:324-346`.
- Loses: No GPU-side point-click in this slice. The card scenario's example "(e.g. a point selection on one channel and an interval selection on another)" is satisfied by composition of input + interval brush, not by a single-plot two-gesture interactor.

**B. Implement chart-side click-to-point in chart_view. Mouse-down + mouse-up at same point + nearest-hit resolves to a row predicate (`row_id = N` or `(x = X AND y = Y)`). Adds a `Point` brush kind and a `point_to_predicate` adapter alongside `brush_rect_to_predicate`.**
- Gains: Self-contained on the chart; no input-widget runtime needed.
- Loses: Requires a row-identity convention. The render layer already exposes `NearestHit { row, point, distance }` (referenced in `interaction.rs:14`); the SQL side does not yet have a `row_id` predicate convention. Selecting "the row at coordinates (x, y)" via SQL needs either a unique key column (no spec declares one) or a stable `rowid`-style anchor (DuckDB specific). Either is a card-level commitment to "row identity in the SQL layer," which is bigger than scenario 5 needs.

**C. Implement both: `input: table` runtime AND chart-side click-to-point. Two new surfaces.**
- Gains: Maximum coverage.
- Loses: Two big slices in one card. Input-widget runtime is itself the centre of card 0005 v3 — duplicating it here violates focus-gate-correct discipline. Chart-side click-to-point inherits all of B's open questions.

### Recommendation

**Option A.** Point-selection-as-row-click is conceptually an input-widget feature and properly belongs in card 0005's runtime work (which is where input widgets actually become live emitters). For card 0006 v3, **scenario 5's "multi-selection per plot" is satisfied at the spec level** by accepting two component-level contributors on the same plot (e.g. an `intervalXY` interactor *and* an `input: table` filter-binding). The runtime change here is purely about ensuring the dispatch path supports it — see Decision 3.

The spec should record explicitly: "Point-as-click-on-mark is deferred to a future card; this slice exercises the multi-selection-per-plot path via composed components (interactor + input + multiple interactors) which is the corpus pattern." That's the focus gate at work.

A small forward-compat win: extend the `BrushKind` enum **today** with a `Point` variant carrying a `(column, value)` pair, and add a `point_predicate(column, value)` helper next to `brush_rect_to_predicate`. The variant is not wired to chart_view in this slice but unblocks card 0005 v3 (which can use it for `input: table` row clicks) and any future click-on-mark card. **Coordination point with card 0005 v3:** the rally should call out which card lands the `BrushKind::Point` variant — recommend this card lands the type, card 0005 lands the wiring.

---

## Decision 3: Multi-selection-per-plot — singular `BrushBinding` or list?

### Context

Today `BrushBinding` is exactly one (selection_name, contributor, kind, channels) tuple. A plot like protein-design.yaml's scatter plot has:
- An `intervalXY` interactor writing `as: $query`
- A `dot` mark with `data: { from: proteins, filterBy: $point }` (subscribing to $point)
- The `input: table` widget at the bottom of the spec writes `as: $point` and reads `filterBy: $query`

That plot is a *contributor to one selection (`$query`) via its interval interactor*, and a *subscriber to two selections (`$query` for the raster, `$point` for the dot)*. The contributor side is one thing per plot. The subscriber side is already handled by the coordinator (each mark is a separate subscriber entry in `selection_subscribers`). So the "multi-selection-per-plot" question reduces to: **can a single plot have multiple interactors, each writing to a different selection?**

Evidence:
- splom.yaml at lines 37, 45, 50, 55, 62: **every facet plot** has an `intervalXY` writing `as: $brush` — but each is a separate plot, so each plot has exactly one contributor. SPLOM is multi-plot, not multi-selection-per-plot.
- The corpus has no spec where one plot declares two interactors writing to two different `as: $name` targets. (`grep "as: \$" -A 5` per plot block confirms.)
- gaia.yaml's scatter plot at lines 87-105 has a single `intervalXY` writing `as: $brush`.
- The card text's framing — "a point selection on one channel and an interval selection on another" — does *not* require both to be chart-side. Resolving Decision 2 toward Option A means only the interval is chart-driven; the point is input-driven.

### Options

**A. Keep `BrushBinding` singular. Each interactor inside a plot becomes its own `BrushBinding`; ChartView holds `Vec<BrushBinding>` keyed by interactor (or by selection name); on_mouse_up dispatches one `propagate_selection` per binding that the brush satisfies.**
- Gains: No type churn on the existing struct. The shipped tests (`cfs2_ac11_*`) keep working unchanged. Multi-binding becomes a list at the call site — the same shape `propagate_param` already accepts (one param can have many subscribers, the coordinator iterates).
- Loses: The chart_view needs to know which binding(s) a given brush gesture maps to. If two `intervalXY` interactors live in the same plot writing to two different selections, the brush is ambiguous — one gesture, two selections. (No corpus spec has this today, but the card scenario opens the door.)

**B. Generalise `BrushBinding` to `Vec<SelectionContribution>` where each contribution names a selection and is matched against the brush kind/channels. Mouse-up iterates through contributions, dispatching one `propagate_selection` per match.**
- Gains: Explicit multi-binding support; the brush kind acts as a filter ("dispatch to every selection whose binding has kind ⊆ this brush's kind").
- Loses: New type design with no current consumer driving the design choices. Easy to over-fit.

**C. Defer multi-binding entirely; assert that today's corpus has at most one contributor-interactor per plot, and treat scenario 5 as "satisfied at the analysis layer" (the `selection_subscribers` graph already records multiple subscribers per plot).**
- Gains: Zero code change.
- Loses: The card's scenario 5 is explicit: "a plot is bound as contributor to two distinct selections." Defer means scenario 5 has no AC-level evidence that the multi-binding shape works.

### Recommendation

**Option A.** Make ChartView hold `Vec<BrushBinding>`; `on_mouse_up_with_dispatch` iterates and dispatches one `propagate_selection` per binding whose `kind` is compatible with the produced brush rect. The dispatch loop's outer shape is already symmetric to `propagate_param`'s subscriber loop — partial-failure on a per-binding basis is the natural fit. The result vec is `Vec<(selection_name, Vec<(usize, Result<…>)>)>` rather than the current single-selection vec; the test double can absorb either shape.

The corpus does not yet exercise per-plot multi-binding, so the first acceptance test will be a **synthetic spec** (similar to v2's `cfs2_ac06_resolution_strategies_runtime` mini-specs):

- `cfs3_ac05_plot_drives_multiple_selections`: a plot with two `intervalXY` interactors, one writing `as: $a`, one writing `as: $b`. Both are kind-compatible with an XY brush. Construct two `BrushBinding`s on the ChartView, simulate a brush release. Assert the dispatcher recorded **two** `dispatch` calls — one per binding, each with the same predicate but different selection names. Assert `current_selections()` shows both `$a` and `$b` populated.

A natural-but-deferred case: an `intervalX` interactor and an `intervalY` interactor on the same plot. The brush rect is XY but the bindings are 1D — should both fire? The cleanest semantics: each binding consumes only its kind's coordinates from the rect (`intervalX` ignores the y-range). Spell this out in the AC text.

**Coordination with Decision 2:** if we land `BrushKind::Point` as a forward-compat type (Decision 2 recommendation), ChartView's `Vec<BrushBinding>` happens to already accept point-bound entries — but those entries are not driven by mouse-up's brush rect. The driver is whatever input-widget plumbing card 0005 v3 introduces. ChartView itself does not dispatch `Point`-kind bindings on brush release in this slice.

---

## Decision 4: Should `InteractorBinding` carry kind and channel metadata?

### Context

Today `InteractorBinding { path: ComponentPath, selection: String }` (analysis.rs:622-628) is the only persisted record of an interactor's binding to a selection. It carries the interactor's path and the target selection name — nothing about the *kind* (intervalX/Y/XY/toggle/nearest), nothing about the channel columns it brushes over.

For the chart_view to construct `BrushBinding`s at app startup, it needs:
- The selection name (already in `InteractorBinding.selection`)
- The contributor path — *the parent plot path*, per Decision 4 of v2 — derivable from the interactor path via `parent_plot()`
- The interactor kind (to map to `BrushKind`) — **NOT in `InteractorBinding`**
- The bound channel columns (the parent plot's `x:` and `y:` channel definitions, which the brush rect coordinates compare against) — **NOT in `InteractorBinding`**

The app shell can re-walk the spec to collect kind and channels, but every consumer would do the same lookup. The decision is whether to put the metadata where it's known (in `analysis`, alongside the binding) or where it's needed (the app shell, computing on demand).

Evidence:
- The `Interactor` AST node at `crates/brightfield-spec/src/ast.rs:308-315` carries `kind: InteractorKind` and `options: IndexMap<String, ValueOrParamRef<SpecValue>>`. Channel columns for the *plot* live on the parent `PlotNode`'s options (`x:`, `y:`, `channels:` etc.) — not on the interactor itself.
- The corpus binding pattern is consistent: `select: intervalX, as: $brush` (a leaf object) inside a plot's items list. The interactor's options never carry `x:` or `y:` — it is the parent plot's channels that the brush spans.
- The card 0006 v2 spec's `ontology_schema.fields` for `InteractorBinding` (analysis.rs:570-575 in v1, finalised as `path: ComponentPath, selection: String`) is the structurally-stable shape. Extending it is an analysis-layer change.

### Options

**A. Extend `InteractorBinding` to carry `kind: InteractorKind` and `channels: ChannelColumns` (the parent plot's resolved x/y column names).**
- Gains: Single source of truth. `analysis.interactor_bindings` becomes "everything chart_view needs to construct `BrushBinding`s" — minus the brush rect itself. Channel resolution lives where the rest of analysis-time validation lives.
- Loses: Schema breaking change to `InteractorBinding` — affects v1 ac-08 test (`cfs_ac08`), the round-trip property (v1 ac-11), and any consumer that destructures the binding. Need a migration that keeps v1 tests green.

**B. Add a sibling helper `pub fn brush_binding_for(spec: &Spec, binding: &InteractorBinding) -> Option<BrushBinding>` (in brightfield-spec or brightfield-ui) that resolves kind + channels on demand.**
- Gains: No schema change. The composition is a pure function over existing AST.
- Loses: `BrushBinding` itself lives in brightfield-ui. A helper in brightfield-spec returning a brightfield-ui type is a layering inversion. Putting it in brightfield-ui adds a dependency on the full Spec walk — fine, but the function lives in one place and the binding info is reconstructed at construction time per chart_view, not memoised.

**C. Add a new `analysis.brushable_bindings: Vec<BrushableBinding>` field — a derived view that joins `InteractorBinding` with `kind` and resolved channels for every interactor whose kind has a brush rect (the IntervalX/Y/XY family).**
- Gains: Keeps `InteractorBinding` stable; introduces a new derived view that the UI consumes directly. Non-brush interactors (Toggle, Highlight, Nearest, Pan*) are excluded by construction.
- Loses: Two parallel collections (`interactor_bindings` and `brushable_bindings`). They must be kept consistent; the v1 ac-08 test counts and shape stay green by virtue of `interactor_bindings` being unchanged.

### Recommendation

**Option C.** Add a derived `brushable_bindings: Vec<BrushableBinding>` view alongside the existing `interactor_bindings`. `BrushableBinding` is the union of (a) the interactor's own path, (b) the parent plot path (the contributor identity), (c) the selection name, (d) the interactor kind, (e) the resolved channel columns. The derivation is: walk `interactor_bindings`, for each binding look up the interactor in the AST by path, look up the parent plot's `x:` and `y:` channel options, and assemble the result. Brush-incompatible kinds (Toggle, Highlight, Nearest, Pan*) are filtered out — those go through a different runtime path and do not need a `BrushBinding`.

Two ACs:
- `cfs3_ac06_brushable_bindings_built`: load a spec with one `intervalXY` and one `panZoom` interactor; assert `brushable_bindings.len() == 1` and the entry has the correct kind, channels, and parent plot path.
- `cfs3_ac07_brushable_bindings_join_with_chart_view`: app-shell-shaped test (or a small helper test) that constructs `BrushBinding` from a `BrushableBinding`. The conversion is `From<&BrushableBinding> for BrushBinding` (or equivalent) — a one-line idiom.

This keeps v1's `interactor_bindings` shape stable (zero breakage on `cfs_ac08`, the round-trip test, or any analysis consumer) and gives the UI a clean, pre-resolved entry point.

**Coordination with card 0005 v3:** input widgets ALSO declare `as: $name` bindings (athletes.yaml:25 — `input: search ... as: $query`). The current `InteractorBinding` walker at `analysis.rs:798-806` matches only `Component::Interactor`, not `Component::Input`. Card 0005 v3 may extend the binding walk to inputs (or add a sibling `input_bindings` collection). The disjointness check: this card touches `analysis.brushable_bindings` (new field, analysis crate), card 0005 may touch `analysis.input_bindings` (also new field). They do not collide. Recommend the spec call out both new fields explicitly so the rally review can confirm.

---

## Decision 5: Selection persistence across param changes — domain check, auto-clear, or trust the predicate?

### Context

Scenario 6: "a brush is active on a plot whose query references a reactive param... a slider changes that param and the plot's data refreshes... the brush predicate persists and continues to filter downstream views — the selection is independent of param-driven re-execution as long as the brushed domain is still meaningful."

Today's behaviour (verified at `lib.rs:464-466` and the `cfs2_ac02..ac08` suite): `propagate_param` does not touch `selection_state`; it threads the live `selection_state` predicates through every subscribing mark's emit. The brush predicate persists verbatim.

The card's "as long as the brushed domain is still meaningful" is the hairy clause. Imagine:
- A plot's data is `SELECT * FROM flights WHERE delay > $threshold`.
- User brushes `delay BETWEEN 50 AND 100`. The brush predicate is `delay >= 50 AND delay <= 100`.
- User moves the slider so `$threshold = 200`. New baseline data has zero rows in `[50, 100]` — the brush "still fires" but matches nothing.

Three interpretations of the card text:
1. **Brush persists, possibly empty result.** Mechanically simplest. SQL truthful. UI shows an empty plot — user's responsibility to clear or re-brush.
2. **Brush persists but the coordinator emits a warning when the brushed domain falls outside the new data range.** Diagnostic only.
3. **Brush auto-clears when its domain falls outside the new data range.** UI feels smart; UI feels surprising.

### Options

**A. Trust the predicate — brush persists verbatim across param changes. Empty results are correct. No domain check, no warning.**
- Gains: Mirrors v2's already-shipped behaviour (`propagate_param` already preserves `selection_state`). One AC adds the regression test. Deterministic. Aligns with the card text's literal reading: "the brush predicate persists and continues to filter downstream views."
- Loses: The "as long as the brushed domain is still meaningful" qualifier is left to the user's judgement. No automatic safety net.

**B. Persist verbatim, but surface a `ParseWarning::SelectionDomainOutOfRange { selection, contributor }` (or a new `SelectionWarning` channel since this is runtime, not parse-time) when the predicate's range, applied to the new data, returns zero rows.**
- Gains: User sees why the plot is empty. Diagnostic without prescriptive action.
- Loses: Requires (i) a runtime warning channel that today does not exist (parse-time `ParseWarning` is the wrong type), (ii) a domain-extraction step on the predicate (parsing `delay >= 50 AND delay <= 100` to extract `[50, 100]`) — fragile. The `Predicate` IR has no structural ranges, only `Expr(String)`. Would need either a richer IR or a regex.

**C. Auto-clear: after a `propagate_param` re-execute, if any subscriber's RecordBatch returns zero rows AND the empty result is attributable to an active selection on that subscriber's selections, clear the offending contributor.**
- Gains: "Smart" UI feel.
- Loses: Causes are ambiguous (zero rows could be the correct answer, not a stale brush). Auto-clearing user state is a UX footgun. Requires the same domain-attribution logic as B but with a destructive action.

### Recommendation

**Option A.** Brush persists verbatim. The card text's "as long as the brushed domain is still meaningful" is satisfied **at the user-experience level**: the user sees an empty plot, looks at their brush, decides to clear (Decision 1's `clear_selection` path) or re-brush. The runtime makes no inference about meaningfulness.

Two ACs:
- `cfs3_ac08_param_change_preserves_selection`: a spec with a slider param `$threshold` and a brush selection `$brush`. Set the brush to a non-trivial predicate, then `propagate_param("threshold", new_value)`. Assert: (i) `current_selections()` still contains the brush; (ii) the subscribing mark's emitted SQL still contains the brush predicate; (iii) the executed result is the new param-filtered data with the brush still applied.
- `cfs3_ac09_param_change_does_not_clobber_selection_state`: explicit regression for the rally seam. Construct a session with both `param_state` and `selection_state` populated; call `propagate_param`; assert `selection_state` is bit-identical (or, more pragmatically, that the contributor list and predicates are unchanged).

The second AC is the rally seam test — it's the contract this card commits to with card 0005 v3.

**The "domain still meaningful" hint is logged as a follow-up memo** (`orbit/cards/memos/2026-04-29-selection-domain-meaningfulness.md` or similar): if user feedback shows confusion, options B and C are revisitable in a future card. For now, simplicity wins.

---

## Decision 6: Test surface — extend `cfs2_` or create `cfs3_`?

### Context

V1 tests use prefix `cfs_`. V2 tests use `cfs2_`. V3 will introduce 7-9 new ACs (Decisions 1, 3, 4, 5 each yield 1-2 ACs). The test prefix question matters because:
- The `cfs ac-10` corpus regression gate tests carry the `cfs_` prefix (analysis layer).
- The v2 review used `rg -n '\bfn cfs2_'` to count tests as the v2 ac-15 gate.
- A v3 card with mixed `cfs2_` and `cfs3_` muddies the audit.

Evidence:
- `rpw` (card 0005) precedent: v1 tests `rpw_*`, v2 tests `rpw2_*` (per v2 spec note "Test prefix `cfs2_` matches the `rpw_`→`rpw2_` precedent").
- The v2 ac-15 gate is `rg -n '\bfn cfs2_' crates/ | wc -l >= 8` — tied to a literal prefix.
- All v2 tests already exist and pass. The v3 work adds new tests; it does not refactor existing ones.

### Options

**A. New prefix `cfs3_` for all v3 tests; keep `cfs2_` and `cfs_` untouched.**
- Gains: Mirrors `rpw_`→`rpw2_` precedent. Each version's tests are countable in isolation. The v3 ac-15-equivalent gate is `rg -n '\bfn cfs3_' crates/ | wc -l >= N`.
- Loses: Three prefixes for one card is unusual.

**B. Reuse `cfs2_` — v3 tests append to the existing prefix, gate becomes `>= existing_count + N`.**
- Gains: Two prefixes total.
- Loses: Breaks the "one card slice = one prefix" precedent. The v2 review's count gate becomes ambiguous.

**C. Promote `cfs_` to mean "all card 0006 tests", drop the version suffix going forward.**
- Gains: Long-term simplification.
- Loses: Re-prefixing v2 tests (16 of them) is destructive churn. The v2 PR review explicitly counted `cfs2_` tests against the gate.

### Recommendation

**Option A.** Use prefix `cfs3_` for this slice's new tests, mirroring `rpw_`→`rpw2_` and the v2 spec's own statement of the precedent. The v3 ac-count gate is `rg -n '\bfn cfs3_' crates/ | wc -l >= 7` (one per code-typed AC enumerated above). Test homes mirror v2's distribution:

- `brightfield-engine`: `cfs3_ac01` (clear), `cfs3_ac02` (clear unsubscribed), `cfs3_ac05` (multi-binding dispatch), `cfs3_ac08` (param-preserves-selection), `cfs3_ac09` (selection-state-bit-identical regression).
- `brightfield-spec`: `cfs3_ac06` (brushable_bindings build), `cfs3_ac07` (BrushableBinding shape).
- `brightfield-ui`: `cfs3_ac03` (click-outside-clears), `cfs3_ac04` (escape clears, if scope allows), and the `BrushKind::Point` constructor test.

A note for the spec: the corpus regression gate (`cfs ac-10`) remains the structural trip-wire and is not renamed. The same is true for v1's `cfs_*` tests — they are the v1 surface and stay green.

---

## Decision summary table

```
| # | Decision                                                | Recommendation                                                                                            |
|---|---------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| 1 | Clearing — coordinator API and UI gesture               | New `Session::clear_selection(name, contributor)`; chart_view dispatches on click-outside-active-brush    |
| 2 | Point selection — chart-side click vs. input widgets    | Defer chart-side click; add `BrushKind::Point` type for forward-compat; input-driven points land in 0005   |
| 3 | Multi-selection-per-plot — singular vs. list binding    | Make ChartView hold `Vec<BrushBinding>`; iterate-and-dispatch on mouse-up                                 |
| 4 | InteractorBinding kind/channel metadata                 | Add derived `analysis.brushable_bindings` (do not modify v1's `interactor_bindings`)                      |
| 5 | Persistence across param changes                        | Trust the predicate — brush persists verbatim, no domain check, no auto-clear; user clears explicitly     |
| 6 | Test prefix                                             | New prefix `cfs3_`; gate `rg cfs3_ | wc -l >= 7`                                                          |
```

---

## Cross-cutting consequences (not decisions, but follow-ons)

- **`SelectionDispatcher` trait grows a `clear` method.** Two methods now: `dispatch(name, contributor, predicate) -> …` and `clear(name, contributor) -> …`. The `Session: SelectionDispatcher` impl forwards to `propagate_selection` and `clear_selection` respectively. The `RecordingDispatcher` test double at `crates/brightfield-ui/src/chart_view.rs:324-346` grows a parallel `clears: Vec<(String, ComponentPath)>` field.

- **`ChartView` must learn which selection(s) it's a contributor to.** Today `BrushBinding` is passed in by the caller (the test). The app shell needs a way to construct one or more `BrushBinding`s per chart, which means it needs `analysis.brushable_bindings` (Decision 4) at the point where ChartView is constructed. The integration-point work in card 0010-style app shell will pick this up; the spec records the dependency.

- **Click vs. drag discrimination in chart_view.** Decision 1 introduces a click-outside-clear path that fires only when mouse-down + mouse-up land at the same point with no drag (a click). Today's `on_mouse_down` immediately starts a brush at the click point (`chart_view.rs:67-77`); a click that doesn't drag becomes a zero-area brush, which would dispatch a degenerate predicate. The fix is to gate the brush-start on a minimum drag distance (e.g. 4 pixels) or to defer brush-start until first mouse-move-while-down. Spell out the choice in the spec — this is small but visible behaviour change.

- **Disjointness commitment with card 0005 v3.** This card's `analysis.brushable_bindings` and card 0005 v3's potential `analysis.input_bindings` are non-overlapping new fields. Both cards may touch `Session::propagate_param` only in the read-direction (this card adds a regression test that it does not clobber `selection_state`; card 0005 may add tests that input widgets re-execute downstream marks). Neither modifies `selection_state` from `propagate_param`. The disjointness gate is: any v3 PR review for either card must show `git diff` is structurally non-overlapping in `engine/src/lib.rs` — coordinator bodies, but the bodies of `propagate_param` and `propagate_selection` are different methods.

- **Forward-compat for `BrushKind::Point`.** Adding the variant is a one-line enum extension and a one-test adapter (`point_predicate(column, value) -> Predicate::Expr(format!("{column} = {value}"))`). Card 0005 v3's input-widget runtime will use it for `input: table` row-click → predicate. **Recommend the spec calls this out as a coordination AC** (`cfs3_ac10_brush_kind_point_constructs`) so the rally review confirms the type is shipped.

- **The card 0006 v3 spec's `metadata.test_prefix` field is `cfs3`.** The v2 spec uses `cfs2`; matching the precedent.

- **Spec syntax for multi-selection / clearing — no new YAML grammar required.** The corpus's existing pattern (multiple components per plot, each with its own `as: $name`) covers multi-selection. Clearing is a runtime gesture, not a spec declaration. This card lands purely as runtime + analysis additions.
