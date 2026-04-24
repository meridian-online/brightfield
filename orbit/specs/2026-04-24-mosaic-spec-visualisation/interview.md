# Design Interview Record — Card 0001 v2: Mosaic Spec-Driven Visualisation

Card: `orbit/cards/0001-mosaic-spec-driven-visualisation.yaml`
Rally: "first end-to-end render"
Date: 2026-04-24

## Goal

A Mosaic YAML spec opened in the brightfield app renders as an interactive
chart in a GPUI window — engine executes queries, renderer paints marks
from real Arrow data, interaction events close the loop.

## Prior Art

v1 shipped (spec parser, all ACs complete):
- Mosaic 0.24.x YAML/JSON parser → typed AST
- Vocabulary registry with ImplStatus per mark/interactor/input/component
- Conformance layer 1 (AST round-trip)

Since v1, other cards shipped:
- brightfield-sql: SQL emission with QueryPlan IR, mark lowerers (Phase 1),
  NavigationFilterPass
- brightfield-engine: DuckDB execution, Engine/Session, Arrow RecordBatch output,
  update_param, update_extent, prepared statement cache
- brightfield-render: Vello scenes, dot/bar/line mark renderers, ScaleSet, axes,
  grid, legend, build_chart_scene, ChannelMap
- brightfield-ui: ChartElement (stubbed), InteractionState, render_overlay

## Decisions

### Q1: Where does the application binary live?

**Decision: New `brightfield-app` binary crate in `crates/brightfield-app/`.**

A new workspace member with src/main.rs. Depends on brightfield-spec,
brightfield-sql, brightfield-engine, brightfield-render, brightfield-ui, and gpui.
Owns the GPUI Application::new() call, window creation, and the
spec-load-execute-render orchestration.

Rationale: Library crates stay library-only. The dependency graph is explicit.
Future CLI args, config, and logging live here without touching libs. Matches
Zed's binary-crate-atop-libraries architecture.

### Q2: Where does the parse→execute→render orchestration live?

**Decision: Orchestration in main.rs (app crate).**

The app's main function (or a thin App struct) calls the pipeline sequentially:
1. parse_spec_path(path) → ParseOutput
2. analyse_spec(&spec) → SpecAnalysis
3. Engine::load_spec(spec, sql_output) → Session
4. session.execute_all() → Vec<Result<Vec<RecordBatch>>>
5. For each mark: ChannelMap::from_mark() + build_chart_scene()
6. Hand scenes to ChartView via Model<ChartState>

This is ~30 lines of imperative code.

Rationale: Simplest. One entry point for v2. "Ship the loop before optimising
it." If a second entry point emerges, extraction is trivial.

### Q3: How do marks produce executable SQL?

**Decision: Generic SimpleLowerer for all { from: table } marks.**

A single SimpleLowerer registered for MarkKind::Dot, Line, Bar (and BarX,
BarY) in default_lowerers(). It reads the mark's data.from field, emits
SELECT columns FROM view, and optionally appends a Filter from filterBy.

The MarkLower trait remains the extension point — when a mark needs
specialisation (e.g. hexbin with spatial binning), it gets its own impl.

Rationale: All three v2 marks share the same data pattern. No engine tests
exercise mark-specific transforms. Avoids duplicating identical logic.

### Q4: How do multiple marks share scales and axes in one plot?

**Decision: Add infer_scales_multi + build_multi_mark_scene alongside existing API.**

New functions in brightfield-render:
- infer_scales_multi(batches: &[(&RecordBatch, &ChannelMap)]) → ScaleSet
  Unions domains across batches for shared scale inference.
- build_multi_mark_scene(data: &[ChartData], layout: &ChartLayout) → Scene
  Renders grid once, then each mark renderer, then axes and legend.

The existing single-mark build_chart_scene is unchanged. All existing tests
continue to pass.

Rationale: Preserves existing tests. The shared-scale inference is the key
correctness requirement and deserves its own testable function.

### Q5: How does ChartElement bridge Vello into GPUI?

**Decision: CPU readback via canvas() / img() element.**

Use GPUI's canvas() or img() to present the Vello-rendered pixel buffer.
Vello renders to wgpu texture → read back pixels → submit as GPUI image.
Near-free on Apple Silicon unified memory.

Rationale: canvas() is a stable GPUI primitive. v2's goal is "see a chart"
— zero-copy GPU composition is a future optimisation.

Note: This complements card 0013's Decision 1 (wrapper pattern). ChartView
is the component structure; canvas()/img() is the paint mechanism inside.

### Q6: How are channel mappings extracted from the spec AST?

**Decision: Extract literal strings only, skip ParamRef with diagnostic.**

ChannelMap::from_mark() continues to extract ValueOrParamRef::Value(String)
entries. ParamRef channels are skipped with a ParseWarning or log line so
the gap is visible rather than silent.

Rationale: The vast majority of specs use literal column names for channel
encodings. The vendored corpus of 54 specs confirms this. Param-driven
channel bindings are a future card's concern (card 0005 v2 runtime
coordinator).

## Constraints

- The app binary must accept a spec file path as a CLI argument
- brightfield-ui must NOT gain dependencies on brightfield-engine or brightfield-sql
  (the app crate bridges them)
- All existing tests across all crates must continue to pass
- The pipeline must handle marks that fail to lower gracefully (log warning,
  skip mark, render the rest)
