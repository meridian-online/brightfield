# Decision Pack: Card 0001 v2 -- Mosaic Spec-Driven Visualisation (End-to-End)

Card: `orbit/cards/0001-mosaic-spec-driven-visualisation.yaml`
Prior spec: `orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/spec.yaml` (v1, complete)
Date: 2026-04-24

## Summary

v1 shipped the spec parser (typed AST, vocabulary registry, conformance layer 1). Since then, four crates have shipped independently: `brightfield-sql` (SQL emission with IR, lowering, rendering), `brightfield-engine` (DuckDB execution, Arrow RecordBatch output, session lifecycle), `brightfield-render` (Vello scenes, mark renderers for dot/bar/line, axes, legends, scales, channel maps), and `brightfield-ui` (stubbed ChartElement, InteractionState with brush/hover/navigation). The v2 goal is to connect these into a working pipeline: spec file on disk produces a rendered chart in a GPUI window. Six decisions follow.

---

## Decision 1: Application Binary Location and Crate Structure

### Context
No `main.rs` or binary crate exists anywhere in the workspace. The workspace is six library crates. v2 requires a runnable application that opens a GPUI window and renders a chart from a spec file argument. The binary needs to depend on `brightfield-spec`, `brightfield-sql`, `brightfield-engine`, `brightfield-render`, and `brightfield-ui`, plus `gpui`.

### Options

**A. New `brightfield-app` binary crate in `crates/brightfield-app/`**
Add a new workspace member `crates/brightfield-app` with `src/main.rs`. This crate owns the GPUI `Application::new()` call, window creation, and the spec-load-execute-render orchestration loop.

**B. Add a `[[bin]]` target to `brightfield-ui`**
Put `main.rs` inside `brightfield-ui/src/bin/brightfield.rs`. The UI crate already depends on `brightfield-render` and `brightfield-spec`.

**C. Root-level `src/main.rs` with workspace `[[bin]]`**
Place the binary at the workspace root, avoiding a new crate.

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | Clean separation: library crates stay library-only; dependency graph is explicit; future CLI args, config, logging live here without touching libs | One more crate to maintain; slightly more Cargo.toml boilerplate |
| B | No new crate; UI crate is already the "top of the stack" | Mixes binary concerns (arg parsing, logging) into a library crate; `brightfield-ui` would need to depend on `brightfield-engine` and `brightfield-sql`, collapsing the current clean layering (`render -> ui` only) |
| C | Minimal files | Workspace root binary is non-standard for multi-crate workspaces; no natural home for app-specific modules |

### Recommendation
**Option A.** The current crate graph has a clear layering (`spec -> sql -> engine`, `spec -> render -> ui`). The binary crate sits at the top and pulls both stacks together without coupling them. This matches Zed's architecture (separate `zed` binary crate atop library crates). Option B would force `brightfield-ui` to depend on `brightfield-engine`, which today it does not.

---

## Decision 2: Spec-to-Render Pipeline Orchestrator

### Context
The v2 pipeline has four stages: (1) parse spec, (2) emit SQL + load data via engine, (3) extract channel maps from mark options, (4) call `build_chart_scene` per mark with real Arrow data. Today these stages live in separate crates with no orchestrator. Something must own the sequence: parse -> analyse -> load -> execute_all -> build scenes. The question is where this orchestration logic lives.

### Options

**A. Orchestration in `main.rs` (app crate)**
The app binary's `main` function (or a thin `App` struct) calls `parse_spec_path`, `analyse_spec`, `Engine::load_spec`, `session.execute_all`, then `ChannelMap::from_mark` + `build_chart_scene` for each mark result. Pure sequential code, no new abstraction.

**B. New `Pipeline` struct in `brightfield-ui`**
Create a `Pipeline` that encapsulates the parse-analyse-load-execute-render sequence, returning a `Vec<(Scene, ScaleSet)>`. The app crate calls `Pipeline::run(path)`.

**C. New `brightfield-pipeline` crate**
A dedicated crate that depends on spec, sql, engine, render and exposes a `Pipeline::run` API.

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | Simplest; no new types; the orchestration is ~30 lines of imperative code; easy to debug; matches "ship the loop before optimising it" | If a second entry point (tests, CLI, WASM) needs the same pipeline, code must be duplicated or extracted later |
| B | Reusable from tests; UI crate gains engine dependency | Couples `brightfield-ui` to `brightfield-engine` and `brightfield-sql`, breaking the current layering |
| C | Clean reuse boundary | Over-engineering for v2; a crate with one public function |

### Recommendation
**Option A.** The orchestration is straightforward sequential code. v2's only entry point is the binary. If a second entry point emerges, extraction is trivial -- the code is already a linear sequence of function calls. Matches engineering principle #2 ("ship the loop before optimising it").

---

## Decision 3: Mark Lowerer Registration for v2

### Context
`brightfield-sql`'s `default_lowerers()` currently returns an empty vec -- every mark kind returns `EmitError::UnsupportedMark`. The engine's `execute_mark` and `execute_all` methods propagate this error. For v2 to render real data, at least dot, line, and bar marks need working lowerers that produce `QueryPlan::Source` (select from the data view). Without lowerers, `execute_all` fails for every mark.

### Options

**A. Implement concrete `MarkLower` for dot, line, bar in `brightfield-sql`**
Each lowerer reads the mark's `data.from` field, resolves it to a `QueryPlan::Source { table }`, and optionally appends a `QueryPlan::Filter` from `filterBy`. Register them in `default_lowerers()`.

**B. Generic pass-through lowerer for all "simple" marks**
A single `SimpleLowerer` that handles any mark whose data declaration is `{ from: table_name }` -- just emits `SELECT * FROM table_name`. Per-mark specialisation deferred.

**C. Skip lowerers; hand-craft SQL in the app**
The app crate bypasses `emit_query` and calls `session.execute_raw_sql("SELECT * FROM {table}")` directly, using the mark's data source name.

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | Correct architecture; lowerers handle filterBy, column selection, future aggregation; tests prove the emitter works end-to-end | More code in brightfield-sql; must handle the `from` field extraction for each mark type |
| B | Less code; covers all three marks (dot, line, bar all use `{ from: t }` today); single registration | Loses the per-mark extension point that the `MarkLower` trait was designed for; harder to add mark-specific transforms (e.g. barY needs implicit count aggregation) |
| C | Zero changes to brightfield-sql | Bypasses the entire emission pipeline; `execute_raw_sql` is `#[cfg(test)]` only; breaks the architectural contract; no filterBy support; no plan_hash caching |

### Recommendation
**Option B.** The three v2 marks (dot, line, bar) all share the same data pattern: `SELECT columns FROM view [WHERE filterBy]`. A single `SimpleLowerer` registered for `MarkKind::Dot`, `MarkKind::Line`, `MarkKind::Bar` (and potentially `MarkKind::BarX`, `MarkKind::BarY`) avoids duplicating identical logic. The `MarkLower` trait remains the extension point -- when a mark needs specialisation (e.g. `hexbin` with spatial binning), it gets its own impl and overrides the registration. Evidence: all existing engine tests use marks with the `{ from: t }` pattern; no test exercises mark-specific transforms.

---

## Decision 4: Multi-Mark Rendering in a Shared Plot

### Context
Card scenario: "Multiple marks render in the same view sharing scales and axes." Today `build_chart_scene` accepts a single `ChartData` with one `MarkRenderer` and one `RecordBatch`. A spec like `plot: [mark: dot, mark: line]` has two marks that should share x/y scales and render into the same Vello scene. The API must evolve to handle N marks per plot.

### Options

**A. Extend `build_chart_scene` to accept `Vec<ChartData>`**
Change the signature to take a slice of `(RecordBatch, ChannelMap, MarkRenderer)` tuples. Infer scales from the union of all batches, render axes once, then iterate marks.

**B. Multi-pass: infer shared scales first, then render each mark**
Add a `infer_scales_multi(batches: &[(&RecordBatch, &ChannelMap)]) -> ScaleSet` function that unions domains across batches. Then call each `MarkRenderer::render` against the shared scales. The scene builder composes: grid + mark[0] + mark[1] + ... + axes + legend.

**C. Composite at the caller level**
The app calls `build_chart_scene` once per mark, then manually merges the Vello scenes. Scales are inferred independently per mark.

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | Single call site; scales naturally unified; axes/grid rendered once | Changes the existing `build_chart_scene` signature (breaking existing tests); the `ChartData` struct becomes more complex |
| B | Existing `build_chart_scene` signature untouched for single-mark case; new `build_multi_mark_scene` added alongside; scale inference is explicit and testable in isolation | Two code paths (single vs multi); slightly more surface area |
| C | No changes to brightfield-render | Scale domains diverge between marks (dot sees [1,5], line sees [1,3] -- axes are wrong); axes rendered N times; manual scene merging is fragile |

### Recommendation
**Option B.** Add `infer_scales_multi` and `build_multi_mark_scene` alongside the existing single-mark API. This preserves all existing tests while adding the multi-mark path. The shared-scale inference is the key correctness requirement (scenario: "sharing scales and axes") and deserves its own testable function. Evidence: `build_chart_scene` is used in 5+ existing tests; changing its signature would require updating them all with no functional benefit for single-mark cases.

---

## Decision 5: GPUI Element Implementation Strategy

### Context
`ChartElement` is currently a plain struct holding a `Scene`, `InteractionState`, width, and height. It does not implement GPUI's `Element` or `IntoElement` traits. For v2, the chart must appear in a GPUI window. GPUI requires elements to implement `IntoElement` (or use built-in primitives like `img()`, `canvas()`). The question is how to bridge Vello's `Scene` into GPUI's rendering.

### Options

**A. CPU readback via `canvas()` element**
Use GPUI's `canvas()` to get a draw callback. Inside the callback, render the Vello `Scene` to a pixel buffer using `vello::util::RenderContext` (wgpu), read back pixels, and present as a GPUI `img()`. This is the "CPU readback" approach mentioned in the existing `ChartElement` doc comment.

**B. Custom `Element` trait implementation on `ChartElement`**
Implement `gpui::Element` directly, gaining access to GPUI's paint context. Use `cx.paint_image()` or `cx.insert_texture()` to blit the Vello-rendered texture.

**C. Shared wgpu device between Vello and GPUI**
Pass GPUI's underlying wgpu device to Vello's renderer, rendering to a shared texture that GPUI composites directly. Zero-copy on unified memory (Apple Silicon Metal).

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | Simplest; `canvas()` is a stable GPUI primitive; Vello rendering is decoupled from GPUI internals; works immediately | CPU readback has latency on discrete GPU systems (irrelevant on Apple Silicon unified memory, but matters on Linux/Windows Vulkan); extra pixel buffer allocation |
| B | Tighter integration; can respond to GPUI layout changes; more idiomatic | GPUI's `Element` trait is complex (prepaint, paint, measure); couples to GPUI internals that may change |
| C | Best performance; zero-copy on unified memory; GPU-native | Requires deep GPUI internals access (wgpu device extraction); fragile coupling to GPUI's rendering backend; GPUI does not publicly expose its wgpu device |

### Recommendation
**Option A.** The existing `ChartElement` doc comment already states "CPU readback for v1 -- on Apple Silicon unified memory, this is near-free." GPUI's `canvas()` or `img()` elements are stable and well-documented. v2's goal is "see a chart" -- not "achieve zero-copy GPU composition." Option C can be pursued later as a performance optimisation once the pipeline works end-to-end. Evidence: Zed uses `canvas()` for custom rendering in several places.

---

## Decision 6: ChannelMap Extraction from Spec AST

### Context
Today `ChannelMap::from_mark(mark)` extracts channels by scanning the mark's `options: IndexMap<String, ValueOrParamRef<SpecValue>>` for known channel wire names (x, y, fill, etc.) and taking `SpecValue::String` values as column names. This works for hand-constructed marks in tests but has a gap: in the real pipeline, the mark's options come from the parsed spec, and channel values may be `ParamRef` (e.g. `x: $col`) rather than plain strings. The v2 pipeline must handle this.

### Options

**A. Require resolved values only; skip ParamRef channels**
`ChannelMap::from_mark` continues to extract only `ValueOrParamRef::Value(SpecValue::String(_))` entries. ParamRef channels are silently skipped. If a mark has `x: $col`, it gets no X channel and rendering produces an incomplete chart.

**B. Add param resolution to ChannelMap extraction**
Extend `ChannelMap::from_mark` to accept a `&ParamValues` context. For `ValueOrParamRef::ParamRef(name)`, look up the current value in the param context and use it as the column name.

**C. Resolve all ParamRefs during a pre-render pass**
Before building the channel map, walk the spec and substitute all `ParamRef` values with their current resolved values, producing a "resolved mark" with no ParamRefs. Then `ChannelMap::from_mark` works unchanged.

### Trade-offs

| Option | Gains | Loses |
|--------|-------|-------|
| A | No code changes; simplest; handles the majority of real specs (most channel encodings are literal column names, not param refs) | Breaks on dynamic channel specs; silent failure mode |
| B | Correct for all cases; minimal change (add an optional `&ParamValues` parameter); ChannelMap stays in `brightfield-render` | Couples render crate to param resolution concepts |
| C | Clean separation; render crate never sees ParamRef | Requires a new "resolved spec" intermediate type; more indirection; over-engineering for v2 |

### Recommendation
**Option A for v2, with diagnostic.** In practice, the vast majority of Mosaic specs use literal column names for channel encodings. The vendored corpus of 54 specs confirms this -- channel options are string literals, not `$param` references. For v2 ("see a chart from a spec file"), Option A is sufficient. Add a `ParseWarning` or log line when a ParamRef channel is skipped, so the gap is visible rather than silent. Option B becomes necessary when reactive param updates need to change channel bindings, but that is a future card's concern.
