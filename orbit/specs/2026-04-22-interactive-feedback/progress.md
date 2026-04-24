# Implementation Progress

Spec path: orbit/specs/2026-04-22-interactive-feedback/spec.yaml
Started: 2026-04-22
Current AC: complete

## Hard Constraints
- [x] find_nearest is a pure function in brightfield-render — no UI dependency, no spatial index
- [x] GPUI tooltip element deferred — this card establishes TooltipContent data extraction only
- [x] HighlightState uses per-row opacity multiplication — not a second render pass
- [x] HighlightState predicate uses Send + Sync bounds
- [x] MarkRenderer::render() gains highlight parameter — all three renderers respect it
- [x] Mark-level interpolation scoped to DotRenderer only
- [x] render_interpolated() default impl forwards highlight to render()
- [x] Brush overlays, hover highlights, tooltips immediate — highlight/dim fade deferred
- [x] Nearest, NearestX, NearestY, Highlight vocab entries flip to Implemented

## Detours

## Acceptance Criteria
- [x] ac-01: NearestMode enum (X, Y, XY) and NearestHit struct in brightfield-render/src/nearest.rs
- [x] ac-02: find_nearest() scans RecordBatch rows via ScaleSet, returns Option<NearestHit>
- [x] ac-03: HighlightState struct with Send+Sync predicate and dimmed_alpha
- [x] ac-04: MarkRenderer::render() gains Option<&HighlightState> — all renderers apply dim
- [x] ac-05: build_chart_scene accepts optional HighlightState via ChartData
- [x] ac-06: Transition struct with prev_positions, duration, easing; TransitionState enum
- [x] ac-07: MarkRenderer::render_interpolated() default + DotRenderer override
- [x] ac-08: TooltipContent struct extracted from RecordBatch row
- [x] ac-09: Nearest, NearestX, NearestY, Highlight vocab flip to Implemented
- [x] ac-10: InteractionState::Hovering gains optional NearestHit field

## Test Summary

| Crate | Tests | Status |
|-------|-------|--------|
| brightfield-render | 61 | all pass |
| brightfield-ui | 23 | all pass |
| brightfield-spec | 84+ | all pass |
