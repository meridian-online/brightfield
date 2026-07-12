Design brief: a read-and-map study of the Altair/Vega-Lite *declarative interaction grammar* against Brightfield's current Mosaic interaction surface (research pass, 2026-07-12). Altair is the Python spec-frontend for Vega-Lite and the well-documented grandparent of Mosaic's selection/param grammar (shared UW-IDL lineage) — so it is the reference for interaction *vocabulary*, not rendering mechanics (those live in the Vega-Lite/Vega runtime, out of scope). Feeds interaction-fidelity card candidates; companion to `2026-06-30-crossfilter-foundation.md` / `2026-06-30-crossfilter-live-wiring.md`. Triggered by the persistent-brush fidelity fix (2026-07-11).

## What Brightfield already has (verified against source)

More than the framing assumed. The core Altair selection triad is live:

- **Variable params** — a slider drives a `WHERE` via `$param` interpolation (`ExpressionNode`, `expr.rs` / `ast.rs`).
- **Interval brushes** (`intervalX/Y/XY`) and **point selections** (`toggleX/Y`) → cross-filter across linked plots (card 0006/0009; `crossfilter.rs`, `brush.rs`).
- **`filterBy`** — `MarkData::From.filter_by` → `emit.rs`. This is how cross-filter is expressed today.
- **The full resolution algebra** — crossfilter / union / intersect / single is genuinely implemented at the SQL-emit layer (`compile_selection`, `lower.rs`; resolution mapped in `ir.rs`; `emit.rs`) and runtime-tested (`cfs2_ac06`). NOTE: the vocab registry still flagged all four `Unimplemented` (`vocab.rs`, `enum SelectionResolution`) — that flag only feeds the preflight SupportReport and was **stale vs the runtime** (corrected 2026-07-12).
- Brushes **persist** as a drawn rectangle after release (`InteractionState::Selected`) — Mosaic/Vega-Lite fidelity, landed 2026-07-11.

So the story is not "missing interaction" — it's a short list of well-scoped extensions plus one strategic bet.

## Ranked gaps (impact × clean-map-to-Mosaic)

1. **Conditional encoding — `when/then/otherwise` (VL `condition`)** — MISSING, L. Make any channel (color/opacity/size/fill) resolve differently for data inside vs outside a selection/predicate. The load-bearing primitive of Altair's whole interaction chapter. Brightfield has exactly two **hardcoded** instances (legend dim-non-active; `HighlightState` emphasis) and no general grammar — an author cannot declare "colour the selected points, grey the rest." Maps to a VL `condition` object on a channel plus the `empty: true|false` knob. **Net-new card** (the honest generalisation of the two hardcoded dims); gate on `/orb:discovery` to scope a v1 (likely colour/opacity only, `empty` included). Highest strategic value, biggest lift (spec parse + analysis + every renderer's channel resolution).

2. **Legend multi-select (shift-click union)** — PARTIAL, S/M. The engine already ORs multiple predicates (`compile_selection`); the legend emits a single category (`selected: Option<String>`, `legend_element.rs`). Almost entirely a UI-accumulation change. **Extends 0009.** Already a sprint candidate — confirmed cheap for the value.

3. **Input widgets: menu / radio / checkbox** — MISSING (slider only), M. `InputKind::{Menu,Search,Table}` are `Unimplemented` (`vocab.rs`), but the AST already models the whole shape (`Input { as_param, from_source, filter_by, options }`, `ast.rs`) and the subscriber graph handles `as:`→param. "Add the widget + repeat the slider wiring," not new architecture. Pairs naturally with #1 (checkbox→conditional) and `filterBy` (dropdown→filter). **Extends 0005/0017.**

4. **Draggable / resizable persisted brush (interval `translate`)** — PARTIAL (persist only), M. The persisted rect (`InteractionState::Selected`) has no hit-test to grab/move/resize — a new drag starts a fresh brush. Vega-Lite defaults `translate: true`. The explicitly-deferred follow-on to the persistent-brush fix. Mostly a UI gesture card; little/no new spec surface. **Extends 0006.**

5. **Initial / seeded selection values (`selection_*(value=…)`)** — MISSING, S/M. `SelectionNode.options` preserves extras verbatim but nothing consumes an initial `value`; selections always start empty. Read at app assembly, seed `selection_state` + the persisted overlay. Directly relevant to the future **consumer** audience (a shared dashboard opening pre-filtered). **Extends 0006**; urgency rises with consumer delivery.

6. **Nearest-point hover selection (`nearest`, `on: pointerover`)** — PARTIAL, M. `highlight` is wired (hover emphasis via `HighlightState`) but `nearest/nearestX/Y` are `Unimplemented` (`vocab.rs`; `find_nearest` has no production caller) and there is no configurable `on:` (click vs pointerover) on point selections. `Hovering { nearest }` state exists (`interaction.rs`) but isn't consumed for selection. Overlaps what `highlight` already gives → mid-low. **Extends 0006.**

7. **Axis pan/zoom (`interactive()`, `bind: scales`)** — MISSING (parsed, unwired), M/L. `Pan/Zoom*` vocab `Unimplemented`; `apply_pan`/`apply_zoom`/`NavigationState` exist and are unit-tested but have **no production caller** (no wheel handler; navigation always `None`). **TENSION:** this fights the launch-anchored widen-only scale design — gestures deliberately hold axes still so they read as *filtering*. Zoom is a *navigation* gesture that moves axes: a philosophy call, not just wiring. **Net-new, design-decision-first.** Rank low until navigation-vs-filtering is resolved.

## Consciously ruled out / already covered

- **Resolution algebra** (crossfilter/union/intersect/single) — implemented + tested; only the stale vocab flag needed fixing (a one-liner, not a card). Done 2026-07-12.
- **`filterBy` / `transform_filter(selection)`** — already *has* (`MarkData::From.filter_by` → `emit.rs`).
- **Parameter composition across *distinct* selections** (`(a|b)&~(a&b)`) — advanced; single-selection resolution covers the common case. L effort, niche payoff. Not yet.
- **Full Vega expression language** (`datum` predicates, `transform_calculate`, regex `expr.test`) — rabbit hole. The `$param` SQL-interpolation tokeniser (`ast.rs`, `expr.rs`) already covers the pragmatic "slider drives a WHERE" case. Skip the general expr engine.
- **Encoding-channel binding** (dropdown swaps the x *column*) — niche; needs a calculate-transform in VL itself. Defer.
- **General HTML bindings** (colour picker, search box) — long tail; fold search into #3 if ever wanted.

## Priority if this becomes cards

Cheap fidelity cluster first — all reuse machinery Brightfield already has: **(2) legend multi-select → (3) input widgets → (4) draggable brush.** Then the strategic bet: **(1) conditional encoding** — the most "grammar of graphics"-true item on the list, `/orb:discovery`-gated. (5)/(6) opportunistic; (7) deferred behind a navigation-vs-filtering design call.

→ Card candidates: **"Conditional channel encoding"** (net-new, discovery-first); **"Legend multi-select"** + **"Input widget family"** + **"Draggable brush"** (extend 0009 / 0005 / 0006). This memo is `/orb:distill`-ready.

Key files for whoever picks these up: spec vocab `crates/brightfield-spec/src/vocab.rs`; selection AST `crates/brightfield-spec/src/ast.rs`; brush→predicate `crates/brightfield-ui/src/brush.rs`; live coordinator `crates/brightfield-ui/src/crossfilter.rs`; resolution algebra `crates/brightfield-sql/src/lower.rs` (`compile_selection`) + `emit.rs`; legend producer `crates/brightfield-ui/src/legend_element.rs`; interaction states `crates/brightfield-ui/src/interaction.rs`.
