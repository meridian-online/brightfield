# Design Interview — Card 0009: Multi-View Dashboard Composition

**Card:** `orbit/cards/0009-multi-view-dashboard-composition.yaml`
**Date:** 2026-04-22
**Mode:** Rally design (agent self-answers from card + codebase evidence; author approves at consolidated gate)

---

## Q1: Where does composition layout logic live?

**Decision:** In `brightfield-spec` as a pure function of the AST, alongside `analyse_spec()` in the analysis module.

**Rationale:** Layout is structural — it depends on the spec's component tree, not on runtime data. The `analysis.rs` module already demonstrates pure-function spec analysis (subscriber graph, dependency DAG). A `compute_layout()` function walks `Spec.root` and produces a positioned `LayoutTree`. The card's scenarios only require declared dimensions, not data-driven sizing. If data-driven sizing emerges later, the function can take optional size hints.

**Files affected:** `crates/brightfield-spec/src/layout.rs` (new module), `crates/brightfield-spec/src/lib.rs` (re-export).

---

## Q2: What layout model should composition use?

**Decision:** Simple box model — each child has a fixed or intrinsic size; concat stacks children sequentially with no stretching.

**Rationale:** Corpus specs declare explicit `height` on plots and explicit pixel values on spacers (`hspace: 35`). No scenario requires flex negotiation or grid alignment. Each child occupies its declared `width`/`height` (from plot attributes) or a default size. `hspace`/`vspace` insert fixed gaps. Sequential stacking with fixed sizes covers all five card scenarios.

**Files affected:** `crates/brightfield-spec/src/layout.rs` — `LayoutNode`, `compute_layout()`.

---

## Q3: How do legends participate as interactive selection interactors?

**Decision:** Legends use the same subscriber graph pathway as interactors. A legend with `as: $toggle` is just another selection producer.

**Rationale:** The `build_subscriber_graph` function in `analysis.rs` already walks the full component tree. The `legends.yaml` corpus spec confirms this pattern: legends use the same `as:` binding as interactors. No separate `LegendInteraction` pathway is needed — the predicate translation (legend channel value → SQL predicate) can be handled at the engine level when processing interaction events.

**Files affected:** `crates/brightfield-engine/src/lib.rs` — legend interaction event handling (future, not this card's scope). `crates/brightfield-spec/src/analysis.rs` — verify subscriber graph already captures legend `as:` bindings.

---

## Q4: How should the layout tree handle mixed component types?

**Decision:** Unified layout node — every `Component` variant (Plot, Input, Legend, HConcat, VConcat, HSpace, VSpace) gets a `LayoutNode` with position and size.

**Rationale:** The card explicitly requires plots, inputs, and legends as layout participants. The `Component` enum already models them as peers. Default sizes for inputs (reasonable widget width, e.g. 200px) and legends (channel-dependent height/width) can be hardcoded initially and overridden by spec attributes.

**Files affected:** `crates/brightfield-spec/src/layout.rs` — `LayoutNode` enum with `Rect { x, y, width, height }` per node.

---

## Q5: What representation for the layout tree?

**Decision:** Recursive tree mirroring the AST structure. A `LayoutNode` enum with children as `Vec<LayoutNode>`.

**Rationale:** Matches codebase style — `Component`, `QueryPlan` are recursive enums. Dashboard trees are small (5-30 nodes). Recursive algorithms are natural for layout (parent sizes depend on children). Arena-based approaches are premature optimisation at this scale.

**Files affected:** `crates/brightfield-spec/src/layout.rs` — `LayoutNode` enum definition.

---

## Q6: How should hspace/vspace values be parsed?

**Decision:** Support numeric values (interpreted as pixels) and `em` unit strings (multiplied by base font size, default 16px). Reject other units.

**Rationale:** The corpus uses both numeric (`hspace: 35`) and unit-bearing (`vspace: 1em`) spacer values. `em` conversion is a single multiplication. Restricting to pixels and `em` is a reasonable v1 boundary — no corpus spec uses other CSS units for spacers. The `SpecValue::Integer`, `SpecValue::Float`, and `SpecValue::String` variants in `SpaceNode.value` already discriminate these cases.

**Files affected:** `crates/brightfield-spec/src/layout.rs` — `resolve_space_value()` helper.

---

## Summary of key files and symbols

### New module: `crates/brightfield-spec/src/layout.rs`
- `LayoutNode` enum — mirrors `Component` variants with position/size
- `Rect` struct — `{ x: f64, y: f64, width: f64, height: f64 }`
- `compute_layout(spec: &Spec, viewport: Rect) -> LayoutTree` — walks `spec.root`, computes positions
- `resolve_space_value(value: &SpecValue, base_font_size: f64) -> f64` — pixel/em parsing
- Default sizes: plot (640x400, matching Observable Plot), input (200x32), legend (120x varies)

### Modified files
- `crates/brightfield-spec/src/lib.rs` — add `pub mod layout;` and re-exports
- `crates/brightfield-spec/src/analysis.rs` — verify legend `as:` bindings appear in subscriber graph (may already work)
- `Cargo.toml` — no new dependencies (layout is pure computation on existing AST types)
