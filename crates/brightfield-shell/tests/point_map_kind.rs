//! **The point-map kind** — a coordinate pair plotted as points, brushable,
//! drawn through a named map projection with a graticule behind it.
//!
//! `chart_kinds::POINT_MAP` declares two required quantitative slots and
//! builds a block of two `dot` layers over one table — the same device
//! `tests/scatter_kind.rs` documents at length for the scatter, with two
//! differences: the plot writes `projectionType`, and the columns are a
//! longitude and a latitude rather than the table's first two measures.
//!
//! # Three tiers, over the same declaration/block/gesture split
//! `tests/scatter_kind.rs` uses
//!
//! **The declaration** — two required quantitative slots, so `accepts`
//! answers no to a table with one measure, same as the scatter's.
//!
//! **The block** — the two layers, the columns they bind, the projection the
//! PLOT declares, and the selection they share, read off the *parsed* document.
//! What the projection then does to the drawing — a dot at its projected
//! position, a graticule from the projection and the visible extent — is
//! `crates/brightfield-render/tests/projected_point_map.rs`'s, the render crate
//! being where the scales live; this file holds the spec-level half, that the
//! kind's builder asks for a projection at all and asks for it where Mosaic
//! puts it.
//!
//! **The gesture** — a real diagonal pointer sweep on a point map composed
//! beside a histogram tile, driven through `MeridianApp` on the presented
//! raster, the same harness `tests/scatter_kind.rs`'s gesture tier uses.

use brightfield_engine::{ColumnProfile, RowsAudience, SqlPredicate};
use brightfield_shell::dashboard::{self, SELECTION};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_shell::{chart_kinds, data_file};
use brightfield_spec::ast::MarkData;
use brightfield_spec::vocab::SelectionResolution;
use brightfield_spec::{
    parse_spec, Component, Format, InteractorKind, Mark, MarkKind, ParamNode, PlotNode, Spec,
    SpecValue, ValueOrParamRef,
};
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::registry::{ChartKind, Field, FieldSlot, FieldType};

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// The longitude the map puts on x.
const LON: &str = "longitude";
/// The latitude the map puts on y.
const LAT: &str = "latitude";
/// A third measure, drawn by the **sibling** tile in the gesture tier, so the
/// narrowing that tile shows cannot be a redraw of one of the map's own
/// columns.
const OTHER: &str = "weight";

/// How many rows the fixture carries.
const ROWS: i64 = 24;

/// The x value at the bottom of latitude's V.
const TROUGH: i64 = 12;

/// The fixture's rows, under the name every kind in this registry reads.
///
/// `latitude` is a **V** in `longitude` — `tests/scatter_kind.rs`'s own
/// device, over columns renamed for this kind — so a rectangle over the
/// middle of the cloud commits a mid-range interval on both, and `weight`
/// rises with the row so a swept middle band narrows the sibling histogram at
/// both ends rather than thinning everywhere.
///
/// The magnitudes (not literal degrees) are `tests/scatter_kind.rs`'s own
/// fixture's, carried over unchanged: this tier exercises the brush
/// mechanics the map shares with the scatter, not the realism of the
/// numbers, and `tests/point_map_baseline.rs` is where a coordinate-shaped
/// span is exercised instead.
fn rows() -> String {
    let mut out = format!("data:\n  {}:\n", data_file::SOURCE);
    for depth in 0..ROWS {
        let lon = depth as f64 * 10.0;
        let lat = ((depth - TROUGH).abs() * 4) as f64;
        let weight = depth * 10;
        let _ = writeln!(
            out,
            "    - {{ {LON}: {lon}, {LAT}: {lat}, {OTHER}: {weight} }}"
        );
    }
    out
}

/// A profiled column, for the field-order reading.
fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
    ColumnProfile {
        name: name.to_string(),
        type_name: type_name.to_string(),
        non_null: 100,
        nulls: 0,
        distinct,
        min: None,
        max: None,
        semantic: brightfield_engine::SemanticType::NotAsked,
        moments: None,
    }
}

/// The shipped point-map kind.
fn point_map() -> &'static ChartKind<String> {
    chart_kinds::find(chart_kinds::POINT_MAP).expect("this build ships a point map kind")
}

/// The block the kind builds over `lon` and `lat`, asked of the registry
/// rather than written out here — so what reddens is the product's emitter
/// changing its mind, not a fixture drifting from it.
fn block(lon: &str, lat: &str) -> String {
    let kind = point_map();
    let fields = vec![
        Field::new(lon, FieldType::Quantitative),
        Field::new(lat, FieldType::Quantitative),
    ];
    let binding = kind
        .bind(&fields)
        .expect("two measures fill the two required slots");
    kind.spec(&binding, &kind.options())
        .expect("the kind builds its spec")
}

/// [`rows`] with the point-map kind's block under it.
fn document(lon: &str, lat: &str) -> String {
    format!("{}{}", rows(), block(lon, lat))
}

fn parsed(source: &str) -> Spec {
    parse_spec(source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the block must parse: {e}\n{source}"))
        .spec
}

/// The one plot the block declares, inside the `hconcat:` it is written as.
fn the_plot(spec: &Spec) -> &PlotNode {
    let root = spec
        .root
        .as_ref()
        .expect("the block declares a root component");
    let items = match root {
        Component::HConcat(concat) => &concat.items,
        other => panic!("the block is a one-entry hconcat, and this is not one: {other:?}"),
    };
    match items.as_slice() {
        [Component::Plot(plot)] => plot,
        other => panic!("the block declares one plot and this is {other:?}"),
    }
}

/// The plot's marks, in declaration order — which is draw order, so the ghost
/// is the first and the subset the second.
fn layers(plot: &PlotNode) -> Vec<&Mark> {
    plot.items
        .iter()
        .filter_map(|item| match item {
            Component::Mark(mark) => Some(mark),
            _ => None,
        })
        .collect()
}

/// The column a layer binds on `channel`, as the emitter spelled it.
fn bound_column(layer: &Mark, channel: &str) -> String {
    match layer.options.get(channel) {
        Some(ValueOrParamRef::Value(SpecValue::String(name))) => name.clone(),
        other => panic!("the layer binds {channel} as {other:?} rather than a column name"),
    }
}

/// The selection name the block's brush writes, read off the block rather
/// than restated here.
fn emitted_selection(plot: &PlotNode) -> String {
    let brush = plot
        .items
        .iter()
        .find_map(|item| match item {
            Component::Interactor(i) if i.kind == InteractorKind::IntervalXY => Some(i),
            _ => None,
        })
        .expect("the block declares an intervalXY brush, so the tile can contribute a selection");
    match brush.options.get("as") {
        Some(ValueOrParamRef::Param(param)) => param.0.clone(),
        other => panic!("the brush writes no `as: $selection`: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AC1 — the declaration
// ---------------------------------------------------------------------------

/// **Two required quantitative slots, and nothing optional beside them.**
#[test]
fn the_point_map_declares_two_required_quantitative_slots() {
    let slots: &[FieldSlot] = point_map().slots;
    let required: Vec<&FieldSlot> = slots.iter().filter(|s| s.required).collect();
    assert_eq!(
        required.len(),
        2,
        "a point map relates a longitude and a latitude; this kind requires \
         {} slot(s): {slots:?}",
        required.len()
    );
    for slot in &required {
        assert_eq!(
            slot.accepts,
            &[FieldType::Quantitative],
            "slot {:?} takes {:?}, so a column that is not a measure could \
             reach an axis a pixel range has to invert through",
            slot.role,
            slot.accepts
        );
    }
    assert_eq!(
        slots.len(),
        required.len(),
        "an optional slot would let the kind be chosen and then draw a \
         different picture from the one its declaration describes: {slots:?}"
    );
}

/// **A table with one measure admits no point map**, whatever else it
/// carries — the same refusal the scatter's declaration earns, and for the
/// same reason: one axis would have nothing to invert a pixel range through.
#[test]
fn a_table_with_one_measure_admits_no_point_map() {
    let kind = point_map();
    let one = chart_kinds::fields_of(&[column(LON, "DOUBLE", 900)]);
    assert!(!kind.accepts(&one), "{one:?}");
    let two = chart_kinds::fields_of(&[column(LON, "DOUBLE", 900), column(LAT, "DOUBLE", 900)]);
    assert!(kind.accepts(&two), "{two:?}");
}

// ---------------------------------------------------------------------------
// AC3 — the block: two equal-aspect layers, ghosted, brushable
// ---------------------------------------------------------------------------

/// **The kind emits two `dot` layers over the bound columns, both asking for
/// an equal-aspect frame, and only the second is narrowed.**
#[test]
fn the_point_map_declares_a_ghost_cloud_behind_a_filtered_subset() {
    let source = document(LON, LAT);
    let spec = parsed(&source);
    let plot = the_plot(&spec);
    let selection = emitted_selection(plot);
    assert_eq!(
        selection, SELECTION,
        "the block's brush writes a selection no other tile in a generated \
         document subscribes to, so a dashboard's tiles would each filter \
         nothing but themselves"
    );

    match spec.params.get(&selection) {
        Some(ParamNode::Selection(node)) => assert_eq!(
            node.select,
            SelectionResolution::Crossfilter,
            "the tile's selection resolves as {:?}",
            node.select
        ),
        other => panic!("`{selection}` is bound by the brush but declared as {other:?}"),
    }

    let layers = layers(plot);
    assert_eq!(
        layers.len(),
        2,
        "the tile is a ghost cloud plus a subset, and this block carries {} \
         layer(s)\n{source}",
        layers.len()
    );
    for (n, layer) in layers.iter().enumerate() {
        assert_eq!(
            layer.kind,
            MarkKind::Dot,
            "layer {n} is not a dot; a point map draws the same mark the \
             scatter does"
        );
        assert_eq!(bound_column(layer, "x"), LON, "layer {n} on x");
        assert_eq!(bound_column(layer, "y"), LAT, "layer {n} on y");
        // `aspectRatio` is refused beside a projection
        // (`ParseWarning::AspectRatioWithProjection`), so a layer that still
        // wrote it would be asking for something the parser drops.
        assert!(
            layer.options.get("aspectRatio").is_none(),
            "layer {n} still asks for an equal-aspect frame, which a projected \
             plot refuses — the projection has already answered that question"
        );
    }

    // The projection, at PLOT level: Mosaic has no mark-level projection key,
    // and this is what puts both layers in one coordinate system by
    // construction rather than by two lookups agreeing.
    match plot.attributes.get("projectionType") {
        Some(SpecValue::String(name)) => assert_eq!(
            name,
            chart_kinds::POINT_MAP_PROJECTION,
            "the tile's projection moved"
        ),
        other => panic!(
            "the plot declares no `projectionType`, so the tile is a scatter \
             shaped like a map rather than a map: {other:?}\n{source}"
        ),
    }

    let source_of = |layer: &Mark| match layer.data.as_ref() {
        Some(MarkData::From {
            source, filter_by, ..
        }) => (source.clone(), filter_by.clone()),
        other => panic!("a layer reads {other:?} rather than the shell's one table"),
    };
    let (ghost_source, ghost_filter) = source_of(layers[0]);
    let (subset_source, subset_filter) = source_of(layers[1]);
    assert_eq!(
        (ghost_source.as_str(), subset_source.as_str()),
        (data_file::SOURCE, data_file::SOURCE),
        "both layers read the one table the shell synthesises a document for"
    );
    assert_eq!(
        ghost_filter, None,
        "the ghost layer is narrowed, so there is no whole cloud behind the \
         subset and both domains re-derive under a sibling's brush"
    );
    assert_eq!(
        subset_filter.map(|p| p.0),
        Some(selection),
        "the subset layer does not read the selection the brush writes"
    );
}

/// **Which column fills which slot is not the table's own order.**
///
/// Unlike the scatter, a point map is only ever built from
/// `dashboard::coordinate_pair`, which names the longitude and the latitude
/// explicitly — this kind's `bind` just has to keep whichever order it is
/// handed, and this is the test that it does, over both orders.
#[test]
fn a_point_maps_axes_are_whichever_columns_bind_names_lon_and_lat() {
    let kind = point_map();
    let fields = vec![
        Field::new(LAT, FieldType::Quantitative),
        Field::new(LON, FieldType::Quantitative),
    ];
    let binding = kind.bind(&fields).expect("two measures bind");
    assert_eq!(
        binding.name("lon"),
        Some(LAT),
        "bind is first-fit in slot order, so the FIRST field handed in fills \
         the FIRST slot regardless of its name — the caller decides which is \
         which, not this kind"
    );
    assert_eq!(binding.name("lat"), Some(LON));
}

// ---------------------------------------------------------------------------
// AC3 — the gesture: a two-dimensional brush filters the sibling tile
// ---------------------------------------------------------------------------

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-point-map-kind-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("the fixture writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The point map as the kind builds it, with a **second tile beside it**: the
/// generated dashboard's own histogram over a third measure.
fn two_tile_document() -> String {
    let mut out = rows();
    out.push_str(&block(LON, LAT));
    out.push_str(&dashboard::histogram_tile(OTHER, 2));
    out
}

fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

fn gesture_frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// A window holding the document at `path`, with two frames drawn so the
/// raster is presented and the pointer has something to land on.
fn window_over(path: &Path, ctx: &egui::Context) -> MeridianApp {
    let (live, composed) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the fixture loads");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path.to_path_buf());
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    gesture_frame(&mut app, ctx, Vec::new());
    gesture_frame(&mut app, ctx, Vec::new());
    app
}

/// A pointer position at `(fx, fy)` of plot `plot`'s **data area**.
fn at(app: &MeridianApp, plot: usize, fx: f64, fy: f64) -> egui::Pos2 {
    let doc = app.chart_doc();
    let raster = doc
        .raster_rect
        .expect("a settled frame presented the raster");
    let p = &doc.composed.plots[plot];
    let l = &p.layout;
    let x = p.rect.x + l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx;
    let y = p.rect.y + l.plot_y_start() + (l.plot_y_end() - l.plot_y_start()) * fy;
    egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32)
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Press at one corner, move to the opposite one, release.
fn sweep(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    plot: usize,
    from: (f64, f64),
    to: (f64, f64),
) {
    let start = at(app, plot, from.0, from.1);
    gesture_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let end = at(app, plot, to.0, to.1);
    gesture_frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    gesture_frame(app, ctx, vec![button(end, false)]);
    gesture_frame(app, ctx, Vec::new());
}

/// The `Interval` clauses in a predicate tree, as `(column, lo, hi)`.
fn intervals(predicate: &SqlPredicate) -> Vec<(String, f64, f64)> {
    match predicate {
        SqlPredicate::Interval { column, lo, hi, .. } => match (lo, hi) {
            (ScalarValue::Float(lo), ScalarValue::Float(hi)) => {
                vec![(column.trim_matches('"').to_string(), *lo, *hi)]
            }
            _ => Vec::new(),
        },
        SqlPredicate::And(parts) | SqlPredicate::Or(parts) => {
            parts.iter().flat_map(intervals).collect()
        }
        _ => Vec::new(),
    }
}

/// The interval clauses the document's selections are holding, by column.
fn held(app: &MeridianApp) -> Vec<(String, f64, f64)> {
    app.chart_doc()
        .live_dashboard()
        .expect("the opened document has a live session")
        .selection_clauses()
        .iter()
        .flat_map(|(_, p)| intervals(p))
        .collect()
}

/// How many rows the step behind mark `mark` returns under the session's
/// current selection state.
fn step_rows(app: &mut MeridianApp, mark: usize) -> u64 {
    app.chart_doc_mut()
        .live_coordinator()
        .expect("the opened document has a live session")
        .session()
        .step_rows_count(mark, RowsAudience::Plot)
        .expect("the step counts")
}

/// **A rectangle swept over the point map narrows the tile beside it.**
///
/// Four marks are composed, in this order: the map's ghost (0) and subset
/// (1), then the sibling histogram's ghost (2) and subset (3). The five
/// things asserted are `tests/scatter_kind.rs`'s own list, read over a
/// longitude/latitude pair instead of two arbitrary measures.
#[test]
fn a_rectangle_swept_over_the_point_map_narrows_the_tile_beside_it() {
    let dir = TempDir::new("two-tile");
    let path = dir.write("two-tile.yaml", &two_tile_document());
    let ctx = egui::Context::default();
    let mut app = window_over(&path, &ctx);

    assert_eq!(
        app.chart_doc().composed.plots.len(),
        2,
        "the fixture is a point map beside a histogram, so a one-plot \
         document would make the rest of this reading meaningless"
    );
    assert!(
        held(&app).is_empty(),
        "a document nobody has brushed is holding {:?}",
        held(&app)
    );

    let (map_rest, ghost_rest, subset_rest) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 2),
        step_rows(&mut app, 3),
    );
    assert_eq!(
        map_rest, ROWS as u64,
        "the map's subset layer draws every row before anything is brushed"
    );
    assert!(
        subset_rest > 2,
        "the sibling histogram fills {subset_rest} bin(s) at rest, so a drop \
         would not be readable"
    );

    // A rectangle over the middle of the cloud: middling fractions on both
    // axes, so neither bound can be right by clamping to an end of a domain.
    sweep(&mut app, &ctx, 0, (0.30, 0.25), (0.60, 0.75));

    let clauses = held(&app);
    let mut columns: Vec<&str> = clauses.iter().map(|(c, _, _)| c.as_str()).collect();
    columns.sort_unstable();
    assert_eq!(
        columns,
        vec![LAT, LON],
        "the sweep committed {clauses:?}; a two-dimensional brush constrains \
         both of the columns its plot binds"
    );
    // `longitude` spans 230 units over the plot's ~288 pixels and `latitude`
    // spans only 48 over its ~434 — the narrower fit — so the equal-aspect
    // frame widens LATITUDE's interactive domain past its own column range to
    // match longitude's px-per-unit, exactly as it widens the rendered axis
    // (`augment_scales_equal_aspect_widens_the_narrower_axis`, in
    // `brightfield-render`). So longitude is checked against its own values
    // and latitude only against being a genuine, ordered interval — a
    // narrower check would fail on the very widening this kind exists for.
    let (_, lon_lo, lon_hi) = clauses
        .iter()
        .find(|(c, _, _)| *c == LON)
        .expect("the clause for longitude");
    assert!(
        *lon_lo >= 0.0 && *lon_hi <= 10.0 * (ROWS - 1) as f64 && lon_lo < lon_hi,
        "the sweep committed [{lon_lo}, {lon_hi}] on {LON}, which is not an \
         interval of that column's own values (0..={}) — a bound derived \
         from pixels or row ordinals lands here",
        10.0 * (ROWS - 1) as f64
    );
    let (_, lat_lo, lat_hi) = clauses
        .iter()
        .find(|(c, _, _)| *c == LAT)
        .expect("the clause for latitude");
    assert!(
        lat_lo.is_finite() && lat_hi.is_finite() && lat_lo < lat_hi,
        "the sweep committed [{lat_lo}, {lat_hi}] on {LAT}, which is not a \
         genuine ordered interval"
    );

    let (map_after, ghost_after, subset_after) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 2),
        step_rows(&mut app, 3),
    );
    assert!(
        subset_after < subset_rest,
        "the tile beside the map still fills {subset_after} bin(s), where it \
         filled {subset_rest} before the sweep — the brush is drawn and \
         nothing downstream reads it"
    );
    assert_eq!(
        ghost_after, ghost_rest,
        "the sibling's unfiltered layer narrowed too, so its axis moves under \
         the pointer and the two frames are not comparable"
    );
    assert_eq!(
        map_after, map_rest,
        "the map narrowed on its own contribution, so the selection is not \
         resolving as crossfilter and the tile is filtering itself"
    );
}
