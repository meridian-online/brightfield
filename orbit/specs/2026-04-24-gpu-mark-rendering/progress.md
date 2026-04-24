# Implementation Progress

Spec path: orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml
Spec hash: sha256:dfb7524a973f9fb7c6c77c2e02c09b764774e7c49bc886e71481da278a739670
Started: 2026-04-24
Current AC: ac-09

## Hard Constraints
- [x] brightfield-render must NOT gain a gpui dependency
- [x] ChartView and all GPUI wiring live in brightfield-ui
- [x] All existing brightfield-render and brightfield-ui tests must continue to pass
- [x] The render pipeline must work on Apple Silicon (macOS Metal via wgpu)
- [x] ChartElement is the inner element (IntoElement); ChartView is the outer component (Render + Model)
- [x] Vello rendering is synchronous in paint() — no background threading for v2
- [x] The wgpu device is dedicated (not shared with GPUI), created once, shared via Arc<Mutex>
- [x] ChartElement is a stateless rendering shell — borrows from ChartState
- [x] GPU-requiring tests use #[cfg(feature = "gpu-tests")]
- [x] VelloRenderer::new() panics with diagnostic if device creation fails

## Detours
- VelloRenderer uses `Arc<Mutex<VelloRenderer>>` instead of `Arc<VelloRenderer>` because Vello's
  internal Renderer contains RefCell (not Sync). Mutex is correct since GPUI paint is single-threaded.
- Uses `vello::wgpu` re-export instead of standalone wgpu crate to avoid version conflicts
  (Vello 0.4 uses wgpu 23; standalone was wgpu 24).

## Acceptance Criteria
- [x] ac-01: ChartState struct with reactive state (Scene, InteractionState, NavigationState, Transition, dimensions, VelloRenderer ref)
- [x] ac-02: VelloRenderer wraps wgpu device/queue/renderer, render_to_pixels() returns RGBA pixels
- [x] ac-03: ChartView implements gpui::Render, owns Entity<ChartState>, returns ChartElement
- [x] ac-04: ChartElement implements gpui::Element (request_layout, prepaint, paint)
- [x] ac-05: Mouse event handlers on ChartView with coordinate transform and InteractionState updates
- [x] ac-06: Repaint triggers via Model notifications (set_scene → cx.notify)
- [x] ac-07: Window resize updates layout dimensions and triggers repaint
- [x] ac-08: Coordinate mapping pipeline (ChartLayout, plot area bounds, inverse scale transform)
- [x] ac-09 (gate): All existing tests pass — cargo test --workspace
