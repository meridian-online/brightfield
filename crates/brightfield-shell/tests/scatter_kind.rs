//! **The scatter kind** — two measures related as a cloud of dots, brushable in
//! both directions.
//!
//! `chart_kinds::SCATTER` declares two required quantitative slots and builds a
//! block of two `dot` layers over one table: the first reads the shell's table
//! straight and never narrows, the second reads it through `filterBy:` the
//! shared crossfilter selection and lands on top in the default mark ink. An
//! `intervalXY` producer makes the tile a contributor to that selection and not
//! only a subscriber to it.
//!
//! # Four tiers, because each one passes on a picture the tier below cannot see
//!
//! **The declaration.** Two required slots, both quantitative, and `accepts`
//! answering no to a table with one measure. This is what decides whether the
//! kind is offered at all.
//!
//! **The block.** The two layers, the columns they bind and the selection they
//! share, read off the *parsed* document rather than off the emitted text — a
//! count of `dot` lines in a string would pass on a block whose second layer
//! plotted a different pair of columns, or bound `filterBy:` to a name no
//! `params:` entry declares, which draws the picture under a *"had no effect"*
//! banner.
//!
//! **The picture.** Dots on the page, read off the raster `capture_vello_only`
//! writes — the same bytes an export produces. Geometry that never reached a
//! pixel satisfies every structural check there is, which is the argument
//! `tests/bar_orientation.rs` makes at length. The reading is a fingerprint of
//! the *pairing* rather than a liveness check: the fixture's y is a V in x, so
//! the ink dips in the middle of the frame and rises at both ends.
//! `a_cloud_drawn_over_one_column_twice_is_a_line` is the same reading over a
//! block that plots one column against itself, and it fails — which is what
//! makes the fingerprint a measurement.
//!
//! **The gesture.** A real diagonal pointer sweep on a scatter composed beside a
//! histogram tile, driven through `MeridianApp` on the presented raster. The
//! three tiers above take the selection as given, so none of them can tell a
//! working brush from a predicate handed to the document.

use brightfield_engine::{ColumnProfile, SemanticType, SqlPredicate};
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::dashboard::{self, SELECTION};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_str, live_spec, Composed};
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

use image::RgbaImage;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// The measure the scatter puts on x — the first of the fixture's measures, in
/// the table's own order.
const X: &str = "depth";
/// The measure the scatter puts on y — the second.
const Y: &str = "reading";
/// A third measure, drawn by the **sibling** tile in the gesture tier, so the
/// narrowing that tile shows cannot be a redraw of one of the scatter's own
/// columns.
const OTHER: &str = "weight";

/// How many rows the fixture carries.
const ROWS: i64 = 24;

/// The x value at the bottom of the V.
const TROUGH: i64 = 12;

/// The fixture's rows, under the name every kind in this registry reads.
///
/// `reading` is a **V** in `depth`: it falls to zero at [`TROUGH`] and rises
/// again. That shape is the whole reason the pixel tier is a measurement — a
/// monotone fixture would draw the same diagonal whichever column reached which
/// axis, and a constant one would draw a line the reading could not tell from a
/// mark that ignored its data.
///
/// `weight` rises with `depth`, so a rectangle swept over a middle band of the
/// scatter selects a middle band of `weight` and the sibling histogram's bars
/// drop away at both ends rather than thinning everywhere.
fn rows() -> String {
    let mut out = format!("data:\n  {}:\n", data_file::SOURCE);
    for depth in 0..ROWS {
        let reading = (depth - TROUGH).abs() * 4;
        let weight = depth * 10;
        let _ = writeln!(
            out,
            "    - {{ {X}: {depth}, {Y}: {reading}, {OTHER}: {weight} }}"
        );
    }
    out
}

/// A profiled column, for the field-order and generated-dashboard readings.
fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
    ColumnProfile {
        name: name.to_string(),
        type_name: type_name.to_string(),
        non_null: 100,
        nulls: 0,
        distinct,
        min: None,
        max: None,
        semantic: SemanticType::NotAsked,
    }
}

/// The shipped scatter kind.
fn scatter() -> &'static ChartKind<String> {
    chart_kinds::find(chart_kinds::SCATTER).expect("this build ships a scatter kind")
}

/// The block the scatter kind builds over `x` and `y`, asked of the registry
/// rather than written out here — so what reddens is the product's emitter
/// changing its mind, not a fixture drifting from it.
fn block(x: &str, y: &str) -> String {
    let kind = scatter();
    let fields = vec![
        Field::new(x, FieldType::Quantitative),
        Field::new(y, FieldType::Quantitative),
    ];
    let binding = kind
        .bind(&fields)
        .expect("two measures fill the two required slots");
    kind.spec(&binding, &kind.options())
        .expect("the kind builds its spec")
}

/// [`rows`] with the scatter kind's block under it.
fn document(x: &str, y: &str) -> String {
    format!("{}{}", rows(), block(x, y))
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

/// The plot's marks, in declaration order — which is draw order, so the ghost is
/// the first and the subset the second.
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

/// The selection name the block's brush writes, read off the block rather than
/// restated here.
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
///
/// Read off the shipped kind rather than off the private constant that declares
/// it, because the slots are what `accepts` answers from and what a picker
/// offers columns for.
#[test]
fn the_scatter_declares_two_required_quantitative_slots() {
    let slots: &[FieldSlot] = scatter().slots;
    let required: Vec<&FieldSlot> = slots.iter().filter(|s| s.required).collect();
    assert_eq!(
        required.len(),
        2,
        "a scatter relates two measures; this kind requires {} slot(s): {slots:?}",
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

/// **A table with one measure admits no scatter**, whatever else it carries.
///
/// The refusal is the half of the declaration that a kind with one required
/// slot would pass while drawing a picture over a column it never bound. Asked
/// through `chart_kinds::fields_of`, which is the function that turns a
/// profiled table into the field list a kind is offered.
#[test]
fn a_table_with_one_measure_admits_no_scatter() {
    let kind = scatter();
    let refused = [
        vec![column("amount", "DOUBLE", 900)],
        vec![
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
        ],
        vec![
            column("amount", "DOUBLE", 900),
            column("day", "DATE", 30),
            column("region", "VARCHAR", 4),
        ],
        vec![column("region", "VARCHAR", 4), column("city", "VARCHAR", 9)],
    ];
    for profiles in refused {
        let fields = chart_kinds::fields_of(&profiles);
        assert!(
            !kind.accepts(&fields),
            "the scatter accepted {fields:?}, which carries fewer than two \
             measures — one of its axes would have nothing to invert a pixel \
             range through"
        );
    }
    let two = chart_kinds::fields_of(&[
        column("amount", "DOUBLE", 900),
        column("region", "VARCHAR", 4),
        column("weight", "BIGINT", 400),
    ]);
    assert!(
        kind.accepts(&two),
        "a second measure is what the kind waits for, and this table has one: \
         {two:?}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — which two columns fill the slots
// ---------------------------------------------------------------------------

/// **The axes are the table's first two measures, in the table's own order.**
///
/// The rule is `chart_kinds::fields_of`'s ordering — measures first, in the
/// table's order — met by `ChartKind::bind`'s first fit over the slots as they
/// are declared. Neither is new here, which is the point: a reader who knows
/// how a histogram picks its column knows how a scatter picks its pair.
///
/// The reading reorders the columns and watches the axes swap, so what it holds
/// is the ordering rather than the names. A rule that sorted by name, by width
/// or by anything else would put `alpha` on x in both halves.
#[test]
fn a_scatters_axes_are_the_tables_first_two_measures() {
    let kind = scatter();
    let axes = |profiles: &[ColumnProfile]| -> (String, String) {
        let fields = chart_kinds::fields_of(profiles);
        let binding = kind.bind(&fields).expect("two measures bind");
        (
            binding.name("x").expect("an x column").to_string(),
            binding.name("y").expect("a y column").to_string(),
        )
    };

    // A category and a date sit between the measures and after them; neither is
    // offered a quantitative slot, so neither can displace one.
    let declared = [
        column("zulu", "DOUBLE", 900),
        column("region", "VARCHAR", 4),
        column("alpha", "BIGINT", 400),
        column("day", "DATE", 30),
        column("mike", "DOUBLE", 700),
    ];
    assert_eq!(
        axes(&declared),
        ("zulu".to_string(), "alpha".to_string()),
        "x is the first measure the table declares and y is the second"
    );

    // The same five columns, the two leading measures swapped.
    let reordered = [
        column("alpha", "BIGINT", 400),
        column("region", "VARCHAR", 4),
        column("zulu", "DOUBLE", 900),
        column("day", "DATE", 30),
        column("mike", "DOUBLE", 700),
    ];
    assert_eq!(
        axes(&reordered),
        ("alpha".to_string(), "zulu".to_string()),
        "reordering the table did not swap the axes, so the pair is being \
         chosen by something other than the table's order"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the block, and the picture it draws
// ---------------------------------------------------------------------------

/// **The kind emits two `dot` layers over the two bound columns, and only the
/// second is narrowed.**
///
/// Asserted on the parsed document. The name the brush writes and the subset
/// reads has to be declared at crossfilter resolution, because that resolution
/// is what drops a plot's own clause from its own query — without it the tile
/// would filter itself and the cloud behind the subset could never separate
/// from it.
#[test]
fn the_scatter_declares_a_ghost_cloud_behind_a_filtered_subset() {
    let source = document(X, Y);
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
            "layer {n} is not a dot; `DotX` and `DotY` are unimplemented in the \
             vocabulary and plain `Dot` is the mark this device draws"
        );
        assert_eq!(bound_column(layer, "x"), X, "layer {n} on x");
        assert_eq!(bound_column(layer, "y"), Y, "layer {n} on y");
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

/// **The ghost ink is the design system's token, not a colour invented here.**
///
/// Two claims and it takes both: the fill the emitter writes has to be a step of
/// the generated warm-gray scale, which a hand-picked grey fails; and
/// `crates/brightfield-shell/src/chart_kinds.rs` must not contain that hex,
/// which a literal frozen against the token's current value fails.
///
/// The source read is deliberate. `tests/token_discipline.rs` scans this crate's
/// `src/` for hand-typed colour, but its hex needle is anchored on `0x` — the
/// Rust spelling of a channel triple — and a `"#rrggbb"` string bound for a YAML
/// `fill:` slips past it.
#[test]
fn the_scatters_ghost_ink_comes_from_the_design_tokens() {
    let spec = parsed(&document(X, Y));
    let fill = match layers(the_plot(&spec))[0].options.get("fill") {
        Some(ValueOrParamRef::Value(SpecValue::String(hex))) => hex.clone(),
        other => panic!("the ghost layer binds no colour on `fill:`: {other:?}"),
    };
    let steps: Vec<String> = meridian_design::scales::GRAY_LIGHT
        .iter()
        .map(meridian_design::colour::Rgba::hex)
        .collect();
    assert!(
        steps.contains(&fill),
        "the ghost is drawn in {fill}, which is no step of the design system's \
         gray scale: {steps:?}"
    );
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/chart_kinds.rs"),
    )
    .expect("the emitter's source");
    assert!(
        !src.contains(&fill),
        "the emitter spells {fill} out, so a palette regeneration would move \
         the rest of the chart's ink and leave the ghost behind"
    );
}

/// The frame of a composed plot in image pixels — where marks are allowed to be.
///
/// The margins carry the axes and their labels, and a low-coverage pixel of
/// anti-aliased label text is a warm grey like every other warm grey. Reading
/// inside the frame excludes them geometrically rather than by hoping a
/// tolerance separates them.
struct Frame {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Frame {
    fn of(composed: &Composed, index: usize) -> Self {
        let plot = &composed.plots[index];
        let (x, y) = (plot.rect.x, plot.rect.y);
        let l = &plot.layout;
        Self {
            x0: (x + l.plot_x_start()).ceil() as u32,
            y0: (y + l.plot_y_start()).ceil() as u32,
            x1: (x + l.plot_x_end()).floor() as u32,
            y1: (y + l.plot_y_end()).floor() as u32,
        }
    }
}

/// The default mark ink, read from the token layer so a palette bump moves the
/// expectation with the picture.
fn subset_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance for a mark-ink reading.
const INK_TOL: i32 = 20;

fn is_ink(p: [u8; 4], want: [i32; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= INK_TOL)
}

/// Render a composition and read its pixels back — the same bytes an export
/// writes.
fn raster(composed: Composed, name: &str) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-scatter-kind");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(name);
    capture_vello_only(composed, 1.0, &png).expect("export");
    image::open(&png).expect("open png").to_rgba8()
}

/// The topmost row carrying subset ink in each frame column, `None` where that
/// column carries none.
fn ink_tops(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    let want = subset_ink();
    (frame.x0..frame.x1)
        .map(|x| (frame.y0..frame.y1).find(|&y| is_ink(img.get_pixel(x, y).0, want)))
        .collect()
}

/// The highest dot in the band of frame columns around `fraction` of the frame's
/// width — a small band rather than one column, so a gap between two dots is not
/// read as an absence.
fn top_near(tops: &[Option<u32>], fraction: f64) -> Option<u32> {
    let width = tops.len() as f64;
    let centre = (width * fraction).round() as usize;
    let half = (width / 12.0).round() as usize;
    let lo = centre.saturating_sub(half);
    let hi = (centre + half).min(tops.len());
    tops[lo..hi].iter().flatten().copied().min()
}

/// **The dots reach the page, and they are where the two columns put them.**
///
/// The reading is a fingerprint of the pairing. `reading` is a V in `depth`, so
/// the cloud's highest dot is near the top of the frame at both ends and near
/// the bottom in the middle. Three things have to hold at once and the middle
/// one is what makes this a measurement rather than a liveness check:
///
/// - **there is ink at all**, over most of the frame's width, so a device that
///   composed and drew nothing says so here;
/// - **the profile dips**, by a clear fraction of the frame's height, which a
///   cloud plotted over one column against itself cannot do — the contrast is
///   `a_cloud_drawn_over_one_column_twice_is_a_line`;
/// - **the two ends are comparable**, because the V is very nearly symmetric
///   about [`TROUGH`], so a picture that dropped half its rows would fail even
///   while dipping.
#[test]
fn the_scatters_dots_reach_the_page_where_the_two_columns_put_them() {
    let composed = compose_spec_str(&document(X, Y), None).expect("the scatter block composes");
    let frame = Frame::of(&composed, 0);
    let img = raster(composed, "scatter.png");
    let tops = ink_tops(&img, &frame);

    let drawn = tops.iter().filter(|t| t.is_some()).count();
    assert!(
        drawn * 4 > tops.len(),
        "{drawn} of {} frame columns carry mark ink, so the cloud is a smudge \
         or nothing at all",
        tops.len()
    );

    let height = f64::from(frame.y1 - frame.y0);
    let left = top_near(&tops, 0.02).expect("a dot at the left edge of the cloud");
    let middle = top_near(&tops, 0.5).expect("a dot in the middle of the cloud");
    let right = top_near(&tops, 0.98).expect("a dot at the right edge of the cloud");
    let dip = |end: u32| f64::from(middle.saturating_sub(end)) / height;
    assert!(
        dip(left) > 0.5,
        "the cloud's highest dot falls only {:.2} of the frame between its left \
         edge (row {left}) and its middle (row {middle}) — `{Y}` is a V in \
         `{X}`, so a shallower profile is not this pair of columns",
        dip(left)
    );
    assert!(
        dip(right) > 0.5,
        "the cloud's highest dot rises only {:.2} of the frame between its \
         middle (row {middle}) and its right edge (row {right})",
        dip(right)
    );
    assert!(
        left.abs_diff(right) * 8 < frame.y1 - frame.y0,
        "the two ends of the V sit {} rows apart in a frame {} tall, and the \
         fixture is symmetric about its trough",
        left.abs_diff(right),
        frame.y1 - frame.y0
    );
}

/// **The same reading over one column plotted against itself, which fails it.**
///
/// A diagonal, not a V: the highest dot climbs steadily from one end of the
/// frame to the other and never dips. So the assertion above passes on the pair
/// of columns the kind binds and fails on a cloud drawn over one of them twice,
/// over the same rows and the same measurement — which is the answer to *could
/// that reading pass on anything?*
///
/// The block is asked of the same emitter, so what separates the two documents
/// is the pair of columns and nothing else.
#[test]
fn a_cloud_drawn_over_one_column_twice_is_a_line() {
    let composed = compose_spec_str(&document(X, X), None).expect("the block composes");
    let frame = Frame::of(&composed, 0);
    let img = raster(composed, "scatter-degenerate.png");
    let tops = ink_tops(&img, &frame);

    let drawn = tops.iter().filter(|t| t.is_some()).count();
    assert!(
        drawn * 4 > tops.len(),
        "{drawn} of {} frame columns carry ink, so this contrast is measuring \
         a broken harness rather than a line",
        tops.len()
    );

    let height = f64::from(frame.y1 - frame.y0);
    let left = top_near(&tops, 0.02).expect("a dot at the left end");
    let middle = top_near(&tops, 0.5).expect("a dot in the middle");
    let dip = f64::from(middle.saturating_sub(left)) / height;
    assert!(
        dip < 0.5,
        "one column against itself dipped {dip:.2} of the frame, so the V \
         reading above is not reading the pairing"
    );
}

// ---------------------------------------------------------------------------
// AC4 — a two-dimensional brush filters the other tiles
// ---------------------------------------------------------------------------

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-scatter-kind-{name}-{}-{nanos}",
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

/// The scatter as the kind builds it, with a **second tile beside it**: the
/// generated dashboard's own histogram over a third measure.
///
/// The scatter half is asked of the registry and the sibling of
/// `dashboard::histogram_tile`, so neither picture is written out here. They
/// compose into one document because the kind's block is an `hconcat:` whose
/// entries sit at the indent that function writes at — which is what makes a
/// two-column device something a caller can put beside a tile.
fn two_tile_document() -> String {
    let mut out = rows();
    out.push_str(&block(X, Y));
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

/// A window holding the document at `path`, with two frames drawn so the raster
/// is presented and the pointer has something to land on.
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
///
/// The data area rather than the whole plot rect: the margins carry the axes,
/// and a pixel out there inverts through a scale to a value off the end of the
/// domain — so a sweep measured over the rect would commit bounds the columns
/// never reach.
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

/// Press at one corner, move to the opposite one, release — the frames a real
/// rectangular sweep occupies.
///
/// Three frames and not one: the gesture machine is edge-triggered on the
/// button, so a press and a release in the same frame is a click, and the move
/// between them is what makes this a sweep.
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

/// Every `Interval` clause in a predicate tree, as `(column, lo, hi)`.
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

/// How many rows the step behind mark `mark` returns under the session's current
/// selection state — the executed answer, not the SQL that would produce it.
fn step_rows(app: &mut MeridianApp, mark: usize) -> u64 {
    app.chart_doc_mut()
        .live_coordinator()
        .expect("the opened document has a live session")
        .session()
        .step_rows_count(mark)
        .expect("the step counts")
}

/// **A rectangle swept over the scatter narrows the tile beside it.**
///
/// The gesture tier, and the one reading here that drives the product's own
/// pointer path. Four marks are composed, in this order: the scatter's ghost
/// (0) and subset (1), then the sibling histogram's ghost (2) and subset (3).
///
/// What is asserted, and why each part is needed:
///
/// - **at rest nothing is held**, so a harness that found a selection
///   everywhere would say so here;
/// - **the sweep commits two intervals, one per axis**, naming both bound
///   columns with bounds inside each column's own range. One clause is what an
///   `intervalX` producer would commit, and this device is the two-dimensional
///   one;
/// - **the sibling's subset step returns fewer rows**, which is the tile being
///   filtered rather than merely redrawn;
/// - **the sibling's ghost step returns as many as before**, which is the
///   unfiltered layer that keeps the axis fixed under the brush;
/// - **the scatter's own subset step is unchanged**, because crossfilter
///   self-exclusion drops a plot's own clause from its own query. A picture
///   that narrowed itself would mean the selection resolved as something other
///   than `crossfilter`, and the ghost could never separate from the subset.
#[test]
fn a_rectangle_swept_over_the_scatter_narrows_the_tile_beside_it() {
    let dir = TempDir::new("two-tile");
    let path = dir.write("two-tile.yaml", &two_tile_document());
    let ctx = egui::Context::default();
    let mut app = window_over(&path, &ctx);

    assert_eq!(
        app.chart_doc().composed.plots.len(),
        2,
        "the fixture is a scatter beside a histogram, so a one-plot document \
         would make the rest of this reading meaningless"
    );
    assert!(
        held(&app).is_empty(),
        "a document nobody has brushed is holding {:?}",
        held(&app)
    );

    let (scatter_rest, ghost_rest, subset_rest) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 2),
        step_rows(&mut app, 3),
    );
    assert_eq!(
        scatter_rest, ROWS as u64,
        "the scatter's subset layer draws every row before anything is brushed"
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
        vec![X, Y],
        "the sweep committed {clauses:?}; a two-dimensional brush constrains \
         both of the columns its plot binds"
    );
    let ranges = [(X, 0.0, f64::from((ROWS - 1) as u32)), (Y, 0.0, 48.0)];
    for (column, min, max) in ranges {
        let (_, lo, hi) = clauses
            .iter()
            .find(|(c, _, _)| c == column)
            .expect("the clause for a bound column");
        assert!(
            *lo >= min && *hi <= max && lo < hi,
            "the sweep committed [{lo}, {hi}] on {column}, which is not an \
             interval of that column's own values ({min}..={max}) — a bound \
             derived from pixels or row ordinals lands here"
        );
    }

    let (scatter_after, ghost_after, subset_after) = (
        step_rows(&mut app, 1),
        step_rows(&mut app, 2),
        step_rows(&mut app, 3),
    );
    assert!(
        subset_after < subset_rest,
        "the tile beside the scatter still fills {subset_after} bin(s), where \
         it filled {subset_rest} before the sweep — the brush is drawn and \
         nothing downstream reads it"
    );
    assert_eq!(
        ghost_after, ghost_rest,
        "the sibling's unfiltered layer narrowed too, so its axis moves under \
         the pointer and the two frames are not comparable"
    );
    assert_eq!(
        scatter_after, scatter_rest,
        "the scatter narrowed on its own contribution, so the selection is not \
         resolving as crossfilter and the tile is filtering itself"
    );
}

// ---------------------------------------------------------------------------
// AC5 — nothing already shipped opens differently
// ---------------------------------------------------------------------------

/// **A file with several measures opens on the same tiles it opened on.**
///
/// The generated dashboard is the one route the registry's contents can change
/// what a reader is shown, and it is closed by the slot count rather than by a
/// name: `dashboard::single_column_kinds` admits a kind with exactly one
/// required slot, and this one has two. So a scatter is chosen for no column,
/// and each measure keeps the distribution it had.
///
/// The registry-level half of AC5 is `adding_the_scatter_moved_no_first_look` in
/// `crates/brightfield-shell/src/chart_kinds.rs`, which compares the shipped
/// registry against the same list with this kind removed over every field list
/// of up to three columns. The shipped starts are a third route and the shortest
/// one: a start is authored YAML compiled into the binary, so no kind is
/// consulted when one opens, and the pictures they produce are held against
/// committed thumbnails by `crates/brightfield-shell/tests/gallery_gate.rs`.
#[test]
fn a_multi_measure_file_still_opens_on_a_distribution_per_column() {
    let dir = TempDir::new("multi-measure");
    let mut csv = format!("{X},{Y},{OTHER},region,day\n");
    for depth in 0..ROWS {
        let reading = (depth - TROUGH).abs() * 4;
        let _ = writeln!(
            csv,
            "{depth},{reading},{},r{},2020-01-{:02}",
            depth * 10,
            depth % 4,
            depth % 28 + 1
        );
    }
    let path = dir.write("readings.csv", &csv);

    let opened = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");
    let drawn: Vec<(&str, &str)> = opened
        .dashboard
        .tiles()
        .iter()
        .map(|t| (t.column(), t.kind().as_str()))
        .collect();
    assert_eq!(
        drawn,
        vec![
            (X, chart_kinds::BINNED_HISTOGRAM.as_str()),
            (Y, chart_kinds::BINNED_HISTOGRAM.as_str()),
            (OTHER, chart_kinds::BINNED_HISTOGRAM.as_str()),
            ("region", brightfield_shell::ranked_bars::KIND_ID.as_str()),
            ("day", chart_kinds::COUNTS_OVER_TIME.as_str()),
        ],
        "a tile per column in the file's own column order, each drawn by the \
         kind it was drawn by before the scatter was registered"
    );
    assert!(
        !dashboard::single_column_kinds()
            .iter()
            .any(|k| k.id == chart_kinds::SCATTER),
        "the scatter is offered to the per-column generator, which has one \
         column to give it and two slots to fill"
    );
}
