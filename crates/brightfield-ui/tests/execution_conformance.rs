//! Execution-conformance: every renderer must produce geometry end-to-end.
//!
//! The bugs the first-render follow-ups catalogued (Int32 columns silently
//! dropped, a literal treated as a column, a missing DuckDB function) all shared
//! a shape: tests checked the emitted SQL *string* but never ran it through
//! DuckDB and the renderer. This layer closes that gap — for each implemented
//! mark it drives the REAL pipeline:
//!
//!   parse → analyse → Engine::load_spec → execute_all → ChannelMap::from_mark
//!     → find_renderer → build_multi_mark_scene → assert the scene has geometry
//!
//! "Geometry" = a path (fill/stroke) or a glyph; a mark that emits valid SQL but
//! renders nothing fails here. See MADR 0002.

use brightfield_engine::{concat_batches, Engine, RecordBatch};
use brightfield_render::channel::ChannelMap;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{
    count_scene_glyphs, count_scene_paths, default_renderers, find_renderer,
};
use brightfield_render::scale::infer_scales_multi;
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::collect_marks;
use vello::Scene;

/// Drive the full pipeline for a plot spec and return the rendered MARK
/// geometry's (path count, glyph count). Renders each mark's renderer directly
/// onto a fresh scene — NOT via `build_multi_mark_scene`, which always paints a
/// white background + grid that would satisfy `paths > 0` even when the mark
/// itself draws nothing (the axes-only baseline the follow-up memo warned about).
fn render_spec(yaml: &str) -> (usize, usize) {
    let parsed = parse_spec(yaml, Format::Yaml).expect("spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("spec analyses");
    let engine = Engine::new();
    let mut session = engine
        .load_spec(parsed.spec.clone(), analysis, None)
        .expect("spec loads")
        .session;

    let results = session.execute_all();
    let marks = collect_marks(&parsed.spec);
    let registry = default_renderers();
    let layout = ChartLayout::new(640.0, 480.0);

    // Per-mark (ChannelMap, MarkKind, batch), keeping only marks that executed.
    let mut metas: Vec<(ChannelMap, MarkKind, RecordBatch)> = Vec::new();
    for (i, res) in results.into_iter().enumerate() {
        if let Ok(batches) = res {
            if let Some(batch) = concat_batches(batches) {
                metas.push((ChannelMap::from_mark(marks[i]), marks[i].kind, batch));
            }
        }
    }

    // Scales inferred across all marks, so e.g. a rule's perpendicular axis has
    // a scale even when a sibling mark provides it.
    let entries: Vec<(&RecordBatch, &ChannelMap)> =
        metas.iter().map(|(cm, _, b)| (b, cm)).collect();
    let mut scales = infer_scales_multi(&entries, layout.x_range(), layout.y_range());

    // Mirror the real scene builders: let each mark contribute positional scales
    // that generic column inference can't supply (regression's x/y extents,
    // 1D-density's perpendicular axis).
    for (cm, kind, batch) in &metas {
        if let Some(renderer) = find_renderer(&registry, *kind) {
            renderer.augment_scales(&mut scales, batch, cm, layout.x_range(), layout.y_range());
        }
    }

    let mut scene = Scene::new();
    for (cm, kind, batch) in &metas {
        if let Some(renderer) = find_renderer(&registry, *kind) {
            renderer.render(&mut scene, batch, cm, &scales, None);
        }
    }
    (count_scene_paths(&scene), count_scene_glyphs(&scene))
}

/// Assert a spec renders some geometry (a path or a glyph).
fn assert_renders(name: &str, yaml: &str) {
    let (paths, glyphs) = render_spec(yaml);
    assert!(
        paths > 0 || glyphs > 0,
        "mark `{name}` emitted SQL but rendered NO geometry end-to-end \
         (paths={paths}, glyphs={glyphs}) — a renderer/lowering/data-type gap"
    );
}

// Integer inline data is deliberate: DuckDB types YAML integers as Int32, which
// is exactly the column type that once silently dropped (follow-up #1).

#[test]
fn dot_renders_geometry() {
    assert_renders(
        "dot",
        r#"
data:
  t: [{ a: 1, b: 4 }, { a: 2, b: 7 }, { a: 3, b: 5 }]
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn line_renders_geometry() {
    assert_renders(
        "line",
        r#"
data:
  t: [{ a: 1, b: 4 }, { a: 2, b: 7 }, { a: 3, b: 5 }, { a: 4, b: 9 }]
plot:
  - mark: line
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn bary_renders_geometry() {
    assert_renders(
        "barY",
        r#"
data:
  t: [{ name: A, n: 3 }, { name: B, n: 7 }, { name: C, n: 5 }]
plot:
  - mark: barY
    data: { from: t }
    x: name
    y: n
"#,
    );
}

#[test]
fn areay_renders_geometry() {
    assert_renders(
        "areaY",
        r#"
data:
  t: [{ a: 1, b: 4 }, { a: 2, b: 7 }, { a: 3, b: 5 }, { a: 4, b: 9 }]
plot:
  - mark: areaY
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn areax_renders_geometry() {
    assert_renders(
        "areaX",
        r#"
data:
  t: [{ a: 4, b: 1 }, { a: 7, b: 2 }, { a: 5, b: 3 }, { a: 9, b: 4 }]
plot:
  - mark: areaX
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn recty_renders_geometry() {
    // rectY: numeric-edged bars — x-bins [lo, hi], y value from zero. Exercises
    // RectRenderer::augment_scales synthesizing the shared Channel::X scale from
    // the x1/x2 columns (no bare x column exists), then the per-bin fills.
    assert_renders(
        "rectY",
        r#"
data:
  bins: [{ lo: 0, hi: 1, n: 3 }, { lo: 1, hi: 2, n: 7 }, { lo: 2, hi: 3, n: 5 }]
plot:
  - mark: rectY
    data: { from: bins }
    x1: lo
    x2: hi
    y: n
"#,
    );
}

#[test]
fn rectx_renders_geometry() {
    // rectX: horizontal numeric-edged bars — y-bins [lo, hi], x value from zero.
    assert_renders(
        "rectX",
        r#"
data:
  bins: [{ lo: 0, hi: 1, n: 3 }, { lo: 1, hi: 2, n: 7 }, { lo: 2, hi: 3, n: 5 }]
plot:
  - mark: rectX
    data: { from: bins }
    y1: lo
    y2: hi
    x: n
"#,
    );
}

#[test]
fn rect_renders_geometry() {
    // Bare rect: both axes ranged (x-extent [x1,x2] × y-extent [y1,y2]).
    // augment_scales synthesizes BOTH shared Channel::X and Channel::Y scales
    // from the interval columns — neither has a bare x/y column to infer from.
    assert_renders(
        "rect",
        r#"
data:
  cells: [{ x1: 0, x2: 1, y1: 0, y2: 2 }, { x1: 1, x2: 3, y1: 1, y2: 4 }, { x1: 2, x2: 4, y1: 2, y2: 5 }]
plot:
  - mark: rect
    data: { from: cells }
    x1: x1
    x2: x2
    y1: y1
    y2: y2
"#,
    );
}

#[test]
fn ruley_renders_geometry() {
    // ruleY needs a perpendicular (x) scale; the rule's own x column provides
    // it. Its y is a literal channel value (a baseline at 6).
    assert_renders(
        "ruleY",
        r#"
data:
  t: [{ a: 1 }, { a: 2 }, { a: 3 }]
plot:
  - mark: ruleY
    data: { from: t }
    x: a
    y: 6
"#,
    );
}

#[test]
fn text_renders_geometry() {
    assert_renders(
        "text",
        r#"
data:
  t: [{ a: 1, b: 4, label: one }, { a: 2, b: 7, label: two }]
plot:
  - mark: text
    data: { from: t }
    x: a
    y: b
    text: label
"#,
    );
}

// The three statistical marks below were the conformance layer's first catch:
// marked `Implemented` but rendering NOTHING end-to-end. Their lowerers emitted
// aggregate/binned columns while the renderers and scale inference looked for the
// raw x/y channel columns, so the executed batch had no column they could find.
// Fixed by the statistical-mark contract work: density now emits bucket *centres
// in data units, aliased to the channel column* (so the bin axis flows through
// generic scale inference); regression emits x/y data extents; and each renderer
// contributes the positional scales generic inference can't supply — the
// perpendicular density axis, regression's x/y domains — via `augment_scales`.
#[test]
fn regressiony_renders_geometry() {
    assert_renders(
        "regressionY",
        r#"
data:
  t: [{ a: 1, b: 2 }, { a: 2, b: 5 }, { a: 3, b: 4 }, { a: 4, b: 8 }, { a: 5, b: 7 }]
plot:
  - mark: regressionY
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn density_2d_renders_geometry() {
    assert_renders(
        "density",
        r#"
data:
  t: [{ a: 1, b: 2 }, { a: 2, b: 3 }, { a: 3, b: 2 }, { a: 2, b: 4 }, { a: 1, b: 3 }, { a: 3, b: 4 }]
plot:
  - mark: density
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn density_x_renders_geometry() {
    assert_renders(
        "densityX",
        r#"
data:
  t: [{ a: 1 }, { a: 2 }, { a: 2 }, { a: 3 }, { a: 3 }, { a: 3 }, { a: 4 }, { a: 5 }]
plot:
  - mark: densityX
    data: { from: t }
    x: a
"#,
    );
}

#[test]
fn density_y_renders_geometry() {
    // densityY exercises the DensityAxis::Y branch of `augment_scales` — the
    // perpendicular (x) density axis is synthesised over x_range, not inferred.
    assert_renders(
        "densityY",
        r#"
data:
  t: [{ a: 1 }, { a: 2 }, { a: 2 }, { a: 3 }, { a: 3 }, { a: 3 }, { a: 4 }, { a: 5 }]
plot:
  - mark: densityY
    data: { from: t }
    y: a
"#,
    );
}

// The four ramp-fill / grid marks below share density's lowering substrate
// (raster/heatmap/contour ride DensityLowerer{TwoD}; cell is pass-through) but
// each renders through its own renderer — so each gets its own end-to-end
// probe: real spec → DuckDB → renderer, no mocked batches.

#[test]
fn raster_renders_geometry() {
    // Same two-axis binned substrate as density, drawn as one filled cell per
    // occupied bin through the count ramp.
    assert_renders(
        "raster",
        r#"
data:
  t: [{ a: 1, b: 2 }, { a: 2, b: 3 }, { a: 3, b: 2 }, { a: 2, b: 4 }, { a: 1, b: 3 }, { a: 3, b: 4 }]
plot:
  - mark: raster
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn heatmap_renders_geometry() {
    // The KDE-smoothed sibling of raster: every grid cell filled through the
    // density ramp (HeatmapRenderer::augment_scales builds the Fill Sequential).
    assert_renders(
        "heatmap",
        r#"
data:
  t: [{ a: 1, b: 2 }, { a: 2, b: 3 }, { a: 3, b: 2 }, { a: 2, b: 4 }, { a: 1, b: 3 }, { a: 3, b: 4 }]
plot:
  - mark: heatmap
    data: { from: t }
    x: a
    y: b
"#,
    );
}

#[test]
fn contour_renders_geometry() {
    // Marching squares over the same reconstructed KDE grid. `thresholds` on a
    // contour mark is the ISO-LEVEL count — the lowerer-registration shield must
    // keep it out of the density SQL's bin count, so this also fails if the
    // shield regresses into emitting a degenerate 5-bucket grid.
    assert_renders(
        "contour",
        r#"
data:
  t: [{ a: 1, b: 2 }, { a: 2, b: 3 }, { a: 3, b: 2 }, { a: 2, b: 4 }, { a: 1, b: 3 }, { a: 3, b: 4 }, { a: 2, b: 3 }, { a: 2, b: 3 }]
plot:
  - mark: contour
    data: { from: t }
    x: a
    y: b
    thresholds: 5
"#,
    );
}

#[test]
fn cell_renders_geometry() {
    // Pre-aggregated categorical × categorical grid with a numeric fill: Band
    // scales on both axes, one rect per pair, fill ramped through the Sequential
    // that CellRenderer::augment_scales builds (a numeric fill would otherwise
    // infer Linear — the trap the density-marks recon called out).
    assert_renders(
        "cell",
        r#"
data:
  t: [{ day: Mon, slot: am, v: 1 }, { day: Mon, slot: pm, v: 4 }, { day: Tue, slot: am, v: 2 }, { day: Tue, slot: pm, v: 8 }]
plot:
  - mark: cell
    data: { from: t }
    x: slot
    y: day
    fill: v
"#,
    );
}

#[test]
fn geo_renders_geometry() {
    // Full geo pipeline end-to-end: the GeoLowerer passes the inline
    // VARCHAR `geom` column through UNWRAPPED (no ST_AsGeoJSON, so no spatial
    // extension needed), execution returns the GeoJSON text, and the GeoRenderer
    // parses each Polygon, projects it (equirectangular), synthesizes its own
    // x/y scales in `augment_scales` (there is no positional column) and fills a
    // choropleth through the ramp — so the scene carries geometry. This is
    // hard-coded per-mark, so promoting geo to Implemented won't auto-trip it.
    assert_renders(
        "geo",
        r#"
data:
  regions:
    - { id: 1, rate: 2, geom: '{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}' }
    - { id: 2, rate: 8, geom: '{"type":"MultiPolygon","coordinates":[[[[10,0],[20,0],[20,10],[10,10],[10,0]]]]}' }
plot:
  - mark: geo
    data: { from: regions }
    fill: rate
"#,
    );
}

#[test]
fn geo_renders_geometry_with_custom_geometry_column() {
    // A non-default `geometry:` column must still render END-TO-END: the
    // GeoLowerer canonicalises the author's `shape` column to `geom`, so the
    // GeoRenderer (which reads the fixed canonical column) finds it and draws.
    // Without the canonicalisation this produced a silent blank plot — the
    // renderer looked for `geom` while the batch column was `shape`. Every other
    // geo fixture uses the default column, so this is the only guard for it.
    assert_renders(
        "geo (custom geometry column)",
        r#"
data:
  regions:
    - { id: 1, rate: 2, shape: '{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}' }
    - { id: 2, rate: 8, shape: '{"type":"Polygon","coordinates":[[[10,0],[20,0],[20,10],[10,10],[10,0]]]}' }
plot:
  - mark: geo
    data: { from: regions }
    geometry: shape
    fill: rate
"#,
    );
}
