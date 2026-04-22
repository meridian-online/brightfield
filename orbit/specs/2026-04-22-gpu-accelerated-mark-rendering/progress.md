# Implementation Progress

Spec path: orbit/specs/2026-04-22-gpu-accelerated-mark-rendering/spec.yaml
Spec hash: sha256:faded6d53298c5698ff90b4ce04ce547e238b0943d887fab7927f18d372e4515
Started: 2026-04-22
Current AC: none

## Hard Constraints
- [x] Vello is the 2D rendering backend; all chart content renders into a single vello::Scene
- [x] CPU readback to GPUI image element for v1 texture handoff
- [x] Two new crates: brightfield-render (headless) and brightfield-ui (GPUI shell)
- [x] brightfield-render must NOT depend on gpui
- [x] brightfield-ui depends on brightfield-render, brightfield-engine, gpui, wgpu
- [x] Fixed margin model with Observable Plot defaults (top: 20, right: 20, bottom: 30, left: 40)
- [x] Interaction overlay renders immediately during drag; DuckDB re-query fires only on brush release
- [x] MarkRenderer trait returns vello scene fragments, not GpuiElement
- [x] Must work on Apple Silicon

## Detours

## Acceptance Criteria
- [x] ac-01: brightfield-render crate exists at crates/brightfield-render/ with Cargo.toml depending on brightfield-spec, arrow, vello, kurbo, peniko — but NOT gpui
- [x] ac-02: ScaleSet type infers linear, band, and time scales from RecordBatch column types and ChannelMap — 6 tests covering linear, band, time, colour scale inference + mapping
- [x] ac-03: DotRenderer implements MarkRenderer and produces a vello::Scene with positioned circles — 2 tests (basic + with colour)
- [x] ac-04: BarRenderer implements MarkRenderer and produces rectangles with correct band/value proportions — 2 tests (rendering + band width)
- [x] ac-05: LineRenderer implements MarkRenderer and produces a connected path following data points — 1 test with 4-point time series
- [x] ac-06: Axis renderer draws tick marks, labels, and grid lines for linear, band, and time scales — 7 tests covering tick computation + rendering
- [x] ac-07: Colour legend renderer draws colour-to-value mapping swatches with labels — 2 tests (4-category + non-colour skip)
- [x] ac-08: build_chart_scene orchestrator composes marks + axes + legend into a single vello::Scene — 3 integration tests (dot, bar, line)
- [x] ac-09: brightfield-ui crate exists with ChartElement wrapping Vello-rendered texture as GPUI image element — 2 tests
- [x] ac-10: Interaction state tracks brush rect and hovered point; overlay renders without DuckDB re-query — 4 tests (brush, hover, idle, overlay)
- [x] ac-11: Workspace Cargo.toml includes brightfield-render and brightfield-ui as workspace members — cargo check --workspace passes
