# Decision Pack: Multi-View Dashboard Composition

**Card:** 0009-multi-view-dashboard-composition
**Date:** 2026-04-22

---

## Decision 1: Where does composition layout logic live?

### Context

The card requires `hconcat`, `vconcat`, `hspace`, and `vspace` to arrange plots, inputs, and legends into dashboards. The AST already fully represents these (`Component::HConcat`, `VConcat`, `HSpace`, `VSpace` in `ast.rs` lines 239-245), and the parser already walks them (`walk_concat`, `walk_component` in `parse.rs` lines 647-661). The serialiser already round-trips them (`emit_component_into` in `parse.rs` lines 1344-1358). The engine already traverses them for mark indexing (`collect_marks_with_path` in `engine/lib.rs` lines 302-332). The question is where the *layout computation* (sizing, positioning, gap insertion) should live.

### Options

**A. New `brightfield-layout` crate -- layout is a separate concern from spec parsing and SQL emission**
- Gains: Clean separation. Layout computation is independent of DuckDB. Can be tested without an engine session. Follows the existing crate boundaries (`brightfield-spec` for parsing, `brightfield-sql` for queries, `brightfield-engine` for execution).
- Loses: Introduces a new crate for logic that may be small initially (hconcat/vconcat are just flex-row/flex-column). Overhead of Cargo.toml, lib.rs, integration.

**B. Layout logic inside `brightfield-engine` as a post-execution pass**
- Gains: Engine already owns the `Session` and walks the component tree. Layout computation naturally follows query execution (you need to know mark data to size plots). No new crate.
- Loses: Mixes concerns -- engine becomes responsible for both SQL execution and visual layout. Layout is not inherently tied to DuckDB.

**C. Layout logic inside `brightfield-spec` as a pure function of the AST**
- Gains: Layout is a structural property of the spec, not a runtime property. The `analysis.rs` module already does pure-function analysis of the spec. No new crate. Layout tree can be computed at parse time without executing queries.
- Loses: If layout needs runtime data (e.g. actual data extent to auto-size), it cannot stay pure. However, the card scenarios only require declared dimensions, not data-driven sizing.

### Recommendation

**Option C.** The card's scenarios describe layout from declared spec structure -- `hconcat`, `vconcat`, `hspace`, `vspace` with declared sizes. No scenario requires data-driven auto-sizing. The `analysis.rs` module in `brightfield-spec` already demonstrates the pattern of pure-function spec analysis (subscriber graph, dependency DAG). A `layout_tree()` function that walks `Spec.root` and produces a positioned tree fits naturally alongside `analyse_spec()`. If data-driven sizing emerges in a future card, the function can take optional size hints without migrating to a new crate.

---

## Decision 2: What layout model should composition use?

### Context

The card's scenarios require horizontal stacking (`hconcat`), vertical stacking (`vconcat`), nesting (grid layouts via nested concat), and explicit gaps (`hspace`, `vspace`). The corpus spec `legends.yaml` demonstrates deeply nested composition: `vconcat` of `hconcat` rows, each containing plots, `hspace` gaps, and standalone legends. The `crossfilter.yaml` spec uses `vconcat` of two plots. The model must handle all three component types (plots, inputs, legends) as first-class children.

### Options

**A. CSS Flexbox-style model -- each concat is a flex container, children are flex items with intrinsic sizes**
- Gains: Well-understood model. `hconcat` maps to `flex-direction: row`, `vconcat` to `flex-direction: column`. `hspace`/`vspace` map to explicit gap sizes. Nesting composes naturally. Future CSS-based rendering (WebAssembly target) maps 1:1.
- Loses: Flexbox has complex sizing rules (flex-grow, flex-shrink, basis) that may be overkill for v1. The spec does not declare flex properties.

**B. Simple box model -- each child has a fixed or intrinsic size; concat stacks them sequentially with no stretching**
- Gains: Minimal complexity. Each child occupies its declared `width`/`height` (from plot attributes) or a default size. `hspace`/`vspace` insert fixed gaps. No flex negotiation. The `PlotNode.attributes` bag already carries `width` and `height` values (seen in `crossfilter.yaml` lines 20, 35: `height: 200`).
- Loses: No automatic fill or stretch. If a child has no declared size, it gets a default. Cannot express "fill remaining space" without future extension.

**C. Grid model -- nested concat creates a 2D grid with row/column tracks**
- Gains: Natural for the "nested composition creates grid layouts" scenario. Each hconcat-inside-vconcat maps to a grid row.
- Loses: Over-engineers the spec's actual semantics. Mosaic's concat is not CSS Grid -- it is sequential stacking. Grid alignment (aligning columns across rows) is not in the card's scenarios.

### Recommendation

**Option B.** The card's scenarios are satisfied by sequential stacking with fixed sizes and explicit gaps. The corpus specs declare explicit `height` on plots and explicit pixel values on spacers (`hspace: 35`, `vspace: 1em`). No scenario requires flex negotiation or grid alignment. The simple box model is the minimal correct solution. If flex-like behaviour is needed later, Option B can be extended by adding optional `flex` attributes to the layout tree nodes.

---

## Decision 3: How should legends participate as interactive selection interactors?

### Context

Card scenario 2 states: "a legend bound to a selection... that selection updates and any views filtering on it respond accordingly." The AST already models legends with an `as` option that lifts to a `ParamRef` (seen in `legends.yaml`: `as: $toggle`, `as: $interval`). The `LegendNode.options` bag carries `as`, `for`, `label`, etc. as `ValueOrParamRef<SpecValue>`. The question is how the legend's interaction semantics (click to toggle, drag to select range) should be wired into the reactive param/selection system.

### Options

**A. Legends are treated identically to interactors -- their `as: $param` binding routes through the same subscriber graph and selection compilation**
- Gains: Uniform model. The `analysis.rs` subscriber graph builder already walks all components. Legends that write `as: $toggle` are functionally equivalent to an interactor writing `as: $toggle`. The `compile_selection` function does not care who contributed a predicate, only which selection it targets.
- Loses: Legends have a dual role (display + interact) that interactors do not. A legend's interaction semantics (what predicate it contributes) depend on its channel type (color toggle vs opacity range), which is not needed for pure interactors.

**B. Legends get a separate interaction pathway -- a `LegendInteraction` struct that translates legend clicks into predicates before feeding the selection system**
- Gains: Explicit separation of the legend-as-display and legend-as-interactor roles. The translation from "clicked legend entry X" to "predicate: color = X" lives in a dedicated module.
- Loses: More code paths to maintain. The end result is the same predicate feeding the same selection. The indirection adds complexity without changing the reactive flow.

### Recommendation

**Option A.** The subscriber graph already handles `as: $param` uniformly across all component types. The `build_subscriber_graph` function in `analysis.rs` walks the full component tree. A legend with `as: $toggle` is simply another producer for the `toggle` selection, and marks with `filterBy: $toggle` are its consumers. The predicate translation (legend channel value to SQL predicate) can be handled at the engine level when processing user interaction events, without a separate pathway. The `legends.yaml` corpus spec confirms this pattern: legends use the same `as:` binding as interactors.

---

## Decision 4: How should the layout tree handle mixed component types (plots, inputs, legends)?

### Context

Card scenario 5 requires plots, input widgets, and standalone legends to compose side by side in the same `hconcat`. The engine currently only indexes marks for query execution (`build_mark_index_map` in `engine/lib.rs`). Inputs and legends are skipped in the mark walk (the `_ => {}` catch-all at line 326). But layout needs to position *all* children, not just marks.

### Options

**A. Unified layout node -- every Component variant gets a layout-tree node with position and size**
- Gains: The layout tree is a complete representation of the visual dashboard. Downstream renderers can walk it to place every element. No component type is second-class.
- Loses: Inputs and legends have no query results, so their "size" must come from defaults or spec-declared attributes, not data. This is extra logic per component type.

**B. Layout tree for concat/space/plot only -- inputs and legends are attached as metadata on their parent concat node**
- Gains: Simpler tree -- only structural containers and data-bearing plots have layout nodes. Inputs and legends are metadata rather than first-class layout participants.
- Loses: Violates the card scenario. If an hconcat contains [plot, input, legend], the input and legend must occupy horizontal space between plot and legend. Treating them as metadata means they have no position in the layout flow.

### Recommendation

**Option A.** The card explicitly requires all three types as layout participants. The `Component` enum already models them as peers (`Plot`, `Input`, `Legend` are all `Component` variants). The layout tree should mirror this -- each variant gets a layout node. Default sizes for inputs (a reasonable widget width) and legends (channel-dependent height/width) can be hardcoded initially and overridden by spec attributes if present. This ensures the layout tree is complete and renderers can position everything.

---

## Decision 5: What representation should the layout tree use?

### Context

The layout computation takes a `Component` tree and produces a positioned tree. The positioned tree needs to carry: (1) the original component reference (for rendering), (2) computed position (x, y), (3) computed size (width, height). Downstream consumers include renderers (future cards) and the engine's mark dispatch. The representation choice affects memory layout, traversal ergonomics, and extensibility.

### Options

**A. Arena-based flat vector with parent indices**
- Gains: Cache-friendly. Single allocation. Index-based traversal avoids borrow-checker friction. Used in many Rust UI libraries (egui, slotmap).
- Loses: Requires index management. Slightly less ergonomic for recursive algorithms. Overkill for trees that are typically <100 nodes (dashboard specs have 5-20 components).

**B. Recursive tree mirroring the AST structure**
- Gains: Direct structural correspondence to the `Component` tree. Natural for recursive algorithms (layout is inherently recursive -- parent sizes depend on children). No index management. Idiomatic for the codebase -- the AST itself uses recursive enums (`Component` contains `Vec<Component>` via `ConcatNode`/`PlotNode`).
- Loses: Heap allocations per node (though `Vec` amortises). Cannot do random access by index without walking.

**C. Flat vector of `(ComponentPath, Rect)` pairs -- no tree structure, just a mapping from path to position**
- Gains: Simplest possible output. Consumers look up position by path. Easy to serialise.
- Loses: Loses structural information. Cannot traverse parent-child relationships. Layout algorithms that need child sizes to compute parent sizes cannot work bottom-up without the tree.

### Recommendation

**Option B.** The codebase consistently uses recursive tree structures (`Component`, `QueryPlan`). The layout tree is small (dashboard specs are 5-30 components). Recursive algorithms are natural for layout computation (each concat sizes its children, sums their sizes, then reports its own size). A `LayoutNode` enum mirroring `Component` with added `x, y, width, height` fields is the simplest correct representation. Arena-based approaches can be adopted later if profiling shows allocation pressure, but for dashboard-scale trees this is premature optimisation.

---

## Decision 6: How should `hspace`/`vspace` values be parsed and applied?

### Context

The `SpaceNode.value` is currently a `SpecValue` -- it could be an integer (`35`), a float (`2.5`), or a string with units (`"1em"`). The `legends.yaml` corpus uses both numeric (`hspace: 35`, `hspace: 30`) and unit-bearing (`vspace: 1em`) spacer values. The layout system needs to interpret these as pixel distances.

### Options

**A. Numeric values only -- treat the value as pixels; reject unit-bearing strings**
- Gains: Simple. No unit conversion logic. The majority of corpus spacers are numeric.
- Loses: Rejects `vspace: 1em` which appears in the corpus. Would require a deviation declaration for the `legends.yaml` spec.

**B. Support numeric (pixels) and `em` units with a configurable base font size**
- Gains: Handles all corpus spacer values. `em` conversion is a single multiplication (`value * base_font_size`). Base font size can default to 16px (standard browser default) or come from `config:` / `plotDefaults:`.
- Loses: Adds unit parsing logic. Opens the door to more CSS unit requests (`rem`, `%`, `vh`).

**C. Treat all values as opaque -- pass them through to the renderer without interpretation**
- Gains: No parsing logic in the layout system. The renderer (which knows the target medium) handles unit conversion.
- Loses: Layout computation cannot produce pixel-accurate positions. The layout tree's coordinates are meaningless if spacer sizes are unknown. Downstream renderers must re-compute layout.

### Recommendation

**Option B.** The corpus uses both numeric and `em` values, so supporting both is necessary for corpus conformance. The unit parsing is trivial (check for `em` suffix, multiply by base font size, default 16px). Restricting to pixels and `em` only is a reasonable v1 boundary -- no corpus spec uses other CSS units for spacers. The `SpecValue::Integer`, `SpecValue::Float`, and `SpecValue::String` variants in `SpaceNode.value` are already sufficient to discriminate these cases.
