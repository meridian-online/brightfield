# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-interactive-feedback/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 3 |
| 2 — Assumption & failure | Pass 1 gaps (missing ACs, signature inconsistency) | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] Missing AC for TooltipElement GPUI rendering
**Category:** missing-requirement
**Pass:** 1
**Description:** The interview (Q2) designs a `TooltipElement` as a GPUI element that renders `TooltipContent` as a styled card, positioned via a coordinate bridge from chart-local Vello coordinates to GPUI screen coordinates. The spec's ac-08 only covers `TooltipContent` struct extraction from a `RecordBatch` row. No AC verifies that a tooltip can be rendered as a GPUI element or that the coordinate bridge works.
**Evidence:** Interview Q2 lists `crates/brightfield-ui/src/tooltip.rs` as a new module with `TooltipElement`. ac-08 description: "TooltipContent struct with field name/value pairs, extracted from RecordBatch at a given row index" -- stops at data extraction.
**Recommendation:** Either add an AC for `TooltipElement` construction and coordinate positioning (even if the test only verifies the struct builds without a live GPUI context), or explicitly declare tooltip rendering out of scope in this card and note it as a follow-up. The current state is ambiguous -- the interview promises it, but the spec does not require it.

### [MEDIUM] Missing AC for hover-to-nearest integration
**Category:** missing-requirement
**Pass:** 1
**Description:** No AC covers the integration point where the hover handler in `interaction.rs` calls `find_nearest()` and populates `InteractionState::Hovering` with a `NearestHit`. The interview (Q1, Q2) describes this as the key integration between `brightfield-render` and `brightfield-ui`, and lists `interaction.rs` as a modified file. ac-01 and ac-02 test `find_nearest` in isolation; no AC tests that the hover path actually invokes it.
**Evidence:** Interview Q1 files affected: "crates/brightfield-ui/src/interaction.rs -- call find_nearest() from hover handler." Interview Q2: "InteractionState::Hovering gains an optional NearestHit field." Neither is covered by any AC.
**Recommendation:** Add an AC for `InteractionState::Hovering` gaining a `NearestHit` field and a unit test that constructs the enriched state. The actual GPUI event wiring can remain untested at unit level, but the state model change should be verified.

### [LOW] render_interpolated default impl drops highlight parameter
**Category:** constraint-conflict
**Pass:** 1
**Description:** The interview Q4 shows `render_interpolated()` with a `highlight: Option<&HighlightState>` parameter, but its default impl calls `self.render(scene, batch, channel_map, scales)` -- the current `render()` signature which does not accept highlight. ac-04 changes `render()` to accept `Option<&HighlightState>`, but ac-07's default fallback does not account for this. Renderers that don't override `render_interpolated` will lose highlight state during transitions.
**Evidence:** Interview Q4 default impl: `self.render(scene, batch, channel_map, scales);` -- 4 args. ac-04 changes `render()` to gain `Option<&HighlightState>` -- 5 args. The default impl needs to forward the highlight parameter.
**Recommendation:** Note in ac-07 that the default impl must forward the highlight parameter to `render()`. This is a one-line fix but should be explicit in the spec to avoid a subtle regression during implementation.

### [MEDIUM] HighlightState with Box<dyn Fn> lacks trait bounds for composability
**Category:** assumption
**Pass:** 2
**Description:** `HighlightState` uses `Box<dyn Fn(usize) -> bool>` for its predicate. In Rust, bare `dyn Fn` is not `Send`, `Sync`, `Debug`, or `Clone`. The existing codebase passes `ChartData` (which would gain `Option<HighlightState>`) by reference, so `Send`/`Sync` may not be immediately required. However, `Debug` is needed for any debug logging, and the inability to clone or debug-print `HighlightState` will friction testing and future composition with GPUI elements (which typically require `Send + 'static`).
**Evidence:** ac-03 defines `HighlightState` with `predicate (Fn(usize) -> bool)`. `ChartData` in `scene.rs:16-29` is a plain struct passed by reference. GPUI elements in `chart_element.rs` hold owned state. If `ChartElement` needs to own a `HighlightState`, `Send + 'static` bounds become necessary.
**Recommendation:** Specify the predicate as `Box<dyn Fn(usize) -> bool + Send + Sync>` in ac-03. This costs nothing at the call site (closures that capture `Send` data are automatically `Send`) and avoids a breaking signature change later.

### [MEDIUM] prev_positions Vec<(f64, f64)> insufficient for bar interpolation
**Category:** failure-mode
**Pass:** 2
**Description:** ac-06 and ac-07 define `prev_positions: Vec<(f64, f64)>` for transition interpolation. For dots, (x, y) is sufficient. For bars, the visual position involves (x_center, y_top, y_bottom) -- a bar growing from 10 to 20 needs to interpolate the top edge while keeping the baseline fixed. A bare `(f64, f64)` loses the baseline, producing incorrect intermediate frames where bars appear to slide vertically rather than grow.
**Evidence:** Interview Q4: "bars grow/shrink" as a desired visual. `BarRenderer::render()` in `mark.rs:226-257` computes `(cx, y_top, y_bottom)` per bar. `prev_positions: &[(f64, f64)]` in the `render_interpolated` signature cannot express this.
**Recommendation:** Either (a) make `prev_positions` a `Vec<MarkPosition>` enum with per-renderer variants (Dot(x,y), Bar(x, y_top, y_bottom), Line(x,y)), or (b) scope ac-07 to DotRenderer only (which the spec nearly does -- "DotRenderer overrides") and explicitly defer bar/line interpolation to a follow-up card. Option (b) is simpler and matches the spec's current test coverage.

### [LOW] No AC for highlight/dim opacity fade animation
**Category:** missing-requirement
**Pass:** 2
**Description:** Interview Q5 specifies highlight/dim opacity changes should animate with 100-150ms duration, not snap. The spec's ac-04 tests that dimmed alpha is applied but does not verify animated transition between full and dimmed states. ac-06 covers data transitions but not highlight transitions. The `TransitionKind::Highlight` from the interview is not mentioned in any AC.
**Evidence:** Interview Q5 table: "Highlight/dim (selection change) | Animate (fade) | 100-150ms". ac-06 description: "Transition struct... with prev_positions, start time, duration, and easing function" -- this is data transition only.
**Recommendation:** Either add an AC for `TransitionKind::Highlight` with opacity fade, or explicitly note that highlight animation is deferred and the initial implementation snaps. Both are valid; the current silence is the problem.

---

## Honest Assessment

This spec is well-structured for its core render-crate work -- the nearest-point, highlight, and transition primitives are clearly specified with testable ACs. The main risk is at the integration boundary: the spec thoroughly covers `brightfield-render` internals but leaves the `brightfield-ui` integration points (hover handler calling `find_nearest`, tooltip GPUI element, highlight fade animation) either missing or implicit. The `prev_positions` shape for bar interpolation is a design gap that will surface during implementation. I recommend scoping bar/line interpolation out of this card (keeping it dot-only) and adding one AC for the enriched `Hovering` state. With those two changes, this is ready to implement.
