# Implementation Progress

Spec path: orbit/specs/2026-04-22-interactive-navigation/spec.yaml
Spec hash: sha256:25de03d97ddf414f1e23905b50df2cb972244ee340382646dc1f01a40569a050
Started: 2026-04-22
Current AC: complete

## Hard Constraints
- [x] Scale enum stays immutable — view extent is a separate struct, not mutable fields on Scale
- [x] ViewExtent lives in brightfield-render so both brightfield-ui and brightfield-engine can depend on it
- [x] NavigationFilterPass uses the existing Pass trait and QueryPlan::Filter IR node — no new IR nodes
- [x] Band and Colour scales are not navigable — navigation gestures on categorical axes are no-ops
- [x] Full scene rebuild every frame during active gesture — no affine-transform shortcut
- [x] Debounce timer for zoom-settle — not velocity-based, not platform gesture-phase events (future refinement)
- [x] Scale::inverse_f64 returns f64 for all scale types (including Time) for API consistency with map_f64

## Detours

## Acceptance Criteria
- [x] ac-01: ViewExtent struct in brightfield-render/src/scale.rs with x: Option<(f64, f64)> and y: Option<(f64, f64)> fields
- [x] ac-02: Scale::inverse_f64 method returns Option<f64> — Some for Linear and Time, None for Band and Colour
- [x] ac-03: build_chart_scene accepts Option<&ViewExtent> and overrides inferred scale domains when Some
- [x] ac-04: NavigationConfig struct with pan, zoom, x_navigable, y_navigable booleans; from_interactor_kind maps all six variants
- [x] ac-05: Pan gesture handler computes normalised delta, applies to ViewExtent respecting axis lock
- [x] ac-06: Zoom gesture handler scales ViewExtent around cursor position using normalised coordinates
- [x] ac-07: Double-click reset sets ViewExtent to None and rebuilds scene from original ScaleSet
- [x] ac-08: Debounce timer with configurable duration fires re-query after last zoom/pan event
- [x] ac-09: NavigationFilterPass implements Pass trait and inserts Filter node with BETWEEN predicate
- [x] ac-10: Session::update_extent activates NavigationFilterPass and re-executes subscribing marks
- [x] ac-11: InteractorKind Pan variants flipped from Unimplemented to Implemented in vocab.rs

## Test Summary

| Crate | Tests | Status |
|-------|-------|--------|
| brightfield-render | 38 | all pass |
| brightfield-ui | 21 | all pass |
| brightfield-sql | 51 + 19 integration | all pass |
| brightfield-engine | 19 | all pass |
| brightfield-spec | passes (pre-existing clippy warnings) | all pass |
