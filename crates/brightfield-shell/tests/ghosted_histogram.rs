//! **The ghosted cross-filtered histogram** — the unfiltered total kept behind
//! the filtered subset, so the denominator never leaves the page.
//!
//! Two `rectY` layers over one table and one `x: {bin: power}` + `y: {count:}`
//! transform: the first reads the table straight and never narrows, the second
//! reads it through `filterBy: $brush`. They share the plot's scales, so the
//! count axis and the pixel mapping are fixed by the total, and a subset reads
//! as a fraction of the bars behind it rather than as a chart that redrew
//! itself at a new scale.
//!
//! The failure this exists to catch is the one that looks like success. A plot
//! carrying only the filtered layer draws a perfectly good histogram after a
//! brush — correct bars, correct counts, re-scaled axis — and the reader has
//! no way to see what fraction of the data it is. So ink-versus-no-ink proves
//! nothing here, and what is asserted instead is the arithmetic the device
//! exists for: **column by column, the top of the two layers together is where
//! the unfiltered bar stood before the brush.** A ghost that re-queried, or a
//! scale that re-anchored to the subset, moves that top and this says so.
//!
//! Read off the raster through `capture_vello_only`, which is the composed
//! Vello scene and nothing else — the same bytes an export writes.
//!
//! # Three documents, one device
//!
//! The first part of this file holds `examples/rect-bin-count-ghost.yaml`, the
//! device authored by hand. The second holds the same device as the **product**
//! emits it: the block `binned-histogram` builds in
//! `brightfield_shell::chart_kinds`, which is the picture a table with a measure
//! opens as. They are asserted separately on purpose — the example says the
//! engine can draw this, and only the registry half says the shell asks for it.
//!
//! The third is `a_single_filtered_layer_redraws_the_whole_chart`: one filtered
//! layer and nothing behind it, written out as a fixture and read the same way.
//! It is what makes the registry reading a measurement — the same assertion over
//! the same rows and the same predicate passes on the two-layer form and fails
//! on the one-layer one.
//!
//! The fourth is `a_drag_on_the_numeric_tile_commits_the_interval_the_reader_swept`,
//! and it is a different tier: no pixels, a real CSV opened through
//! `MeridianApp::open_data_file`, and a real pointer sweep on the raster. The
//! three readings above take the predicate as given, so none of them can tell a
//! working brush from a selection handed to the document — which is what let
//! this tile ship an `intervalX` interactor that could not produce one.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_shell::app::{chart_registry, CHART};
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{live_spec, Composed, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_shell::{chart_kinds, data_file};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{AggregateFunc, MarkData};
use brightfield_spec::vocab::SelectionResolution;
use brightfield_spec::{
    parse_spec, Component, Format, InteractorKind, Mark, MarkKind, ParamNode, PlotNode, Spec,
    SpecValue, ValueOrParamRef,
};
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::registry::{Field, FieldType};
use brightfield_workbench::subject::ToolbarLocation;

use image::RgbaImage;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The ghost layer's ink — the pale constant `examples/rect-bin-count-ghost.yaml`
/// binds to its unfiltered layer.
const GHOST: [i32; 3] = [0xc4, 0xbc, 0xb0];

/// The filtered layer's ink: it binds no colour channel, so it takes the
/// default mark colour, read from the token layer so a palette bump moves the
/// expectation with the picture.
fn subset_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance.
///
/// Narrower than the mark-ink tolerance the other pixel gates use, because the
/// ghost is a warm grey and so is the chart's own chrome. A tolerance that
/// admitted the axis baseline would report a ghost bar in every column that
/// rule crosses, which is all of them — the first ghost ink tried did exactly
/// that, and `the_ghost_ink_is_not_the_charts_own_chrome` is what now catches
/// it.
const INK_TOL: i32 = 12;

fn matches(p: [u8; 4], want: [i32; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= INK_TOL)
}

/// Every ink the plot frame itself is drawn in, as opaque bytes — the colours a
/// ghost reading must not be able to mistake for a bar.
fn chrome_inks() -> Vec<(&'static str, [u8; 4])> {
    let ink = meridian_design::chrome::INK_LIGHT;
    [
        ("surface", ink.surface),
        ("gridline", ink.gridline),
        ("baseline", ink.baseline),
        ("ink_muted", ink.ink_muted),
        ("ink_secondary", ink.ink_secondary),
        ("ink_primary", ink.ink_primary),
    ]
    .into_iter()
    .map(|(name, token)| {
        (
            name,
            [
                (token.r * 255.0).round() as u8,
                (token.g * 255.0).round() as u8,
                (token.b * 255.0).round() as u8,
                255,
            ],
        )
    })
    .collect()
}

/// `ghost` is far enough from every chrome ink for [`columns_with`] to be a
/// reading of bars rather than of the plot frame.
fn assert_chrome_separated(ghost: [i32; 3], whose: &str) {
    for (name, rgba) in chrome_inks() {
        assert!(
            !matches(rgba, ghost),
            "the plot frame's {name} is inside the measurement tolerance of \
             {whose} ghost ink — the ghost readings would be reading chrome"
        );
    }
}

/// **The ghost is measurable only while its ink is nobody else's.**
///
/// The example picks a literal, the chrome comes from tokens, and neither
/// knows about the other. A palette bump that walked one of them into the
/// ghost's range would redden this rather than silently turn
/// `columns_with(GHOST)` into a reading of the gridlines.
#[test]
fn the_ghost_ink_is_not_the_charts_own_chrome() {
    assert_chrome_separated(GHOST, "the example's");
}

/// Render a composition and read its pixels back.
fn raster(composed: Composed, name: &str) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-ghosted-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(name);
    capture_vello_only(composed, 1.0, &png).expect("export");
    image::open(&png).expect("open png").to_rgba8()
}

/// The histogram plot's frame in image pixels — where bars are allowed to be,
/// and the region every reading below is taken over.
///
/// Text is what forces this. An axis label is `ink_secondary` anti-aliased
/// against the surface, and a low-coverage pixel of that blend is a warm grey
/// like every other warm grey; a reading over the whole image finds tick labels
/// and reports them as ghost bars. The labels live in the margins by
/// construction, so reading inside the frame excludes them geometrically
/// instead of by hoping a tolerance separates them.
struct Frame {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Frame {
    /// The frame of `composed`'s plot at `index`, in the composed dashboard's
    /// own coordinates (each plot scene is placed at its rect's origin).
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

/// The topmost row carrying either layer's ink, per frame column — the top of
/// the bar standing in that column, whichever layer drew it.
///
/// `None` for a column with no bar. Both layers baseline on the same axis, so
/// the higher of the two tops is the taller bar's, which is the total's.
///
/// `ghost` is the pale ink of whichever document is being read: the example
/// binds a literal, the registry's emitter resolves a token, and the reading is
/// the same either way.
fn bar_tops(img: &RgbaImage, frame: &Frame, ghost: [i32; 3]) -> Vec<Option<u32>> {
    let subset = subset_ink();
    (frame.x0..frame.x1)
        .map(|x| {
            (frame.y0..frame.y1).find(|&y| {
                let p = img.get_pixel(x, y).0;
                matches(p, ghost) || matches(p, subset)
            })
        })
        .collect()
}

/// Frame columns carrying a given ink at all.
fn columns_with(img: &RgbaImage, frame: &Frame, want: [i32; 3]) -> Vec<u32> {
    (frame.x0..frame.x1)
        .filter(|&x| (frame.y0..frame.y1).any(|y| matches(img.get_pixel(x, y).0, want)))
        .collect()
}

/// **The device, verified by raster.**
#[test]
fn a_two_layer_spec_draws_the_filtered_subset_over_the_unfiltered_total() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/rect-bin-count-ghost.yaml");
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the example loads live");
    let brushed_plot = ComponentPath(resting.plots[0].path.clone());
    // The histogram is the second plot; the first is the scatter the brush
    // is drawn on.
    let frame = Frame::of(&resting, 1);

    // At rest the two layers coincide and the solid one covers the ghost. So
    // the resting picture is the unfiltered total, and it is what every
    // assertion below is measured against.
    let before = raster(resting, "resting.png");
    let ghost_at_rest = columns_with(&before, &frame, GHOST);
    assert!(
        ghost_at_rest.is_empty(),
        "with nothing filtered the subset IS the total, so the ghost is \
         covered — {} columns show it instead",
        ghost_at_rest.len()
    );
    let total_tops = bar_tops(&before, &frame, GHOST);
    assert!(
        total_tops.iter().filter(|t| t.is_some()).count() > 40,
        "fixture check: the resting histogram has bars to measure"
    );

    // Brush the scatter down to its low temperatures. The subset is a real
    // subset: some bins empty entirely, others thin.
    let filtered = live
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: brushed_plot,
            predicate: SqlPredicate::Interval {
                column: "temp".to_string(),
                lo: ScalarValue::Float(1.0),
                hi: ScalarValue::Float(14.0),
                meta: None,
            },
        })
        .expect("the brush re-composites");
    let after = raster(filtered, "filtered.png");

    // The ghost is now visible, in its own ink.
    let ghost_columns = columns_with(&after, &frame, GHOST);
    assert!(
        !ghost_columns.is_empty(),
        "the unfiltered total is drawn behind the subset once they differ"
    );

    // And the subset is genuinely smaller: bins the brush excluded lost their
    // solid ink entirely.
    let subset_columns = columns_with(&after, &frame, subset_ink());
    assert!(
        !subset_columns.is_empty(),
        "the filtered layer still draws the rows the brush kept"
    );
    assert!(
        ghost_columns.iter().any(|c| !subset_columns.contains(c)),
        "some bin the brush excluded stands as ghost alone, with no subset ink \
         over it — otherwise the subset is not a subset"
    );

    // **The denominator held.** Column by column, the top of the two layers
    // together is where the unfiltered bar stood before the brush. A ghost
    // layer that re-queried under the filter, or a count axis that re-anchored
    // to the subset, moves this.
    //
    // Two readings, in the direction that matters. A column that LOST its bar
    // is the failure — that is the denominator going missing. A column that
    // gained one is a bar edge landing on the pixel grid differently between
    // two renders, which the inter-bar gaps do and which says nothing about
    // either layer's height.
    let after_tops = bar_tops(&after, &frame, GHOST);
    assert_eq!(
        after_tops.len(),
        total_tops.len(),
        "the two renders are the same size"
    );
    let pairs = || total_tops.iter().zip(after_tops.iter()).enumerate();
    let vanished: Vec<usize> = pairs()
        .filter(|(_, (b, a))| b.is_some() && a.is_none())
        .map(|(x, _)| x)
        .collect();
    assert!(
        vanished.is_empty(),
        "the filtered picture lost the bar entirely in these frame columns, so \
         the total is not behind the subset: {vanished:?}"
    );
    let moved: Vec<(usize, u32, u32)> = pairs()
        .filter_map(|(x, (b, a))| match (b, a) {
            (Some(b), Some(a)) if b.abs_diff(*a) > 1 => Some((x, *b, *a)),
            _ => None,
        })
        .collect();
    assert!(
        moved.is_empty(),
        "the filtered picture must keep the unfiltered total's bar tops — \
         these columns moved: {moved:?}"
    );
    let compared = pairs()
        .filter(|(_, (b, a))| b.is_some() && a.is_some())
        .count();
    assert!(
        compared * 2 > total_tops.len(),
        "fixture check: most of the frame must carry a bar in both renders or \
         the two assertions above are comparing almost nothing ({compared} of \
         {} columns)",
        total_tops.len()
    );
}

// ---------------------------------------------------------------------------
// The registry's numeric tile — the same device, as the product emits it
// ---------------------------------------------------------------------------

/// The measure the tile below is drawn over.
const MEASURE: &str = "v";

/// A `data:` block of a hundred distinct measure values with row counts that
/// vary, declared under the name every kind in the registry reads.
///
/// The varying counts are load bearing: bins of equal height would let a bar top
/// that moved land back on a neighbour's, and the per-column comparisons below
/// would pass on a picture that had changed.
fn fixture_rows() -> String {
    let mut out = format!("data:\n  {}:\n", data_file::SOURCE);
    for v in 0..100u32 {
        for _ in 0..=(v % 7) {
            let _ = writeln!(out, "    - {{ {MEASURE}: {v} }}");
        }
    }
    out
}

/// The `binned-histogram` block the registry builds, over [`fixture_rows`].
///
/// The block is **asked of the registry** rather than written out here, and that
/// is the whole point of this half of the file: what has to redden is the
/// product's emitter ceasing to ask for the device, not a fixture drifting from
/// it.
fn registry_document() -> String {
    let mut out = fixture_rows();
    let kind = chart_kinds::find(chart_kinds::BINNED_HISTOGRAM)
        .expect("this build ships a binned-histogram kind");
    let fields = vec![Field::new(MEASURE, FieldType::Quantitative)];
    let binding = kind.bind(&fields).expect("one measure fills its one slot");
    out.push_str(
        &kind
            .spec(&binding, &kind.options())
            .expect("the kind builds its spec"),
    );
    out
}

/// [`registry_document`] parsed.
fn registry_spec() -> Spec {
    let source = registry_document();
    parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the registry's block must parse: {e}\n{source}"))
        .spec
}

/// The one plot the block declares.
fn the_plot(spec: &Spec) -> &PlotNode {
    match spec
        .root
        .as_ref()
        .expect("the block declares a root component")
    {
        Component::Plot(plot) => plot,
        other => panic!("the block is a single plot, and this is not one: {other:?}"),
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

/// The selection name the block's brush writes, read off the block instead of
/// restated here — so renaming it in the emitter does not need an edit in two
/// places to stay honest.
fn emitted_selection(plot: &PlotNode) -> String {
    let brush = plot
        .items
        .iter()
        .filter_map(|item| match item {
            Component::Interactor(i) if i.kind == InteractorKind::IntervalX => Some(i),
            _ => None,
        })
        .next()
        .expect("the block declares an intervalX brush, so the tile can contribute a selection");
    match brush.options.get("as") {
        Some(ValueOrParamRef::Param(param)) => param.0.clone(),
        other => panic!("the brush writes no `as: $selection`: {other:?}"),
    }
}

/// The colour the ghost layer's `fill:` asks for, exactly as the emitter wrote
/// it.
fn emitted_ghost_fill(ghost: &Mark) -> String {
    match ghost.options.get("fill") {
        Some(ValueOrParamRef::Value(SpecValue::String(hex))) => hex.clone(),
        other => panic!("the ghost layer binds no colour on `fill:`: {other:?}"),
    }
}

/// A `#rrggbb` scalar as channels.
fn rgb_of(hex: &str) -> [i32; 3] {
    let body = hex
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("the ghost fill is not a hex colour: {hex}"));
    assert_eq!(body.len(), 6, "the ghost fill is not `#rrggbb`: {hex}");
    [0, 2, 4].map(|i| {
        i32::from_str_radix(&body[i..i + 2], 16)
            .unwrap_or_else(|e| panic!("{hex} channel at {i}: {e}"))
    })
}

/// The ghost ink the emitted block asks for, ready to measure pixels against.
fn registry_ghost_ink() -> [i32; 3] {
    let spec = registry_spec();
    rgb_of(&emitted_ghost_fill(layers(the_plot(&spec))[0]))
}

/// **The kind emits two `rectY` layers over one bin transform, and only the
/// second is narrowed.**
///
/// Asserted on the parsed document, not on the emitted text. A count of `rectY`
/// lines in the string would pass on a block whose second layer binned a
/// different column, counted something else, or bound `filterBy:` to a name no
/// `params:` entry declares — and the last of those draws the picture under a
/// *"had no effect"* banner.
#[test]
fn the_registrys_numeric_tile_declares_a_ghost_behind_a_filtered_subset() {
    let spec = registry_spec();
    let plot = the_plot(&spec);
    let selection = emitted_selection(plot);

    // The name the brush writes and the subset reads has to be declared, at
    // crossfilter resolution: that resolution is what drops a plot's own clause
    // from its own query.
    match spec.params.get(&selection) {
        Some(ParamNode::Selection(node)) => assert_eq!(
            node.select,
            SelectionResolution::Crossfilter,
            "the tile's selection resolves as {:?}, so a tile would filter \
             itself and the ghost could never separate from the subset",
            node.select
        ),
        other => panic!("`{selection}` is bound by the brush but declared as {other:?}"),
    }

    let layers = layers(plot);
    assert_eq!(
        layers.len(),
        2,
        "the tile is a ghost plus a subset, and this block carries {} layer(s)",
        layers.len()
    );
    for (n, layer) in layers.iter().enumerate() {
        assert_eq!(layer.kind, MarkKind::RectY, "layer {n} is not a rectY");
        assert_eq!(
            layer.options.get("x"),
            Some(&ValueOrParamRef::Value(SpecValue::Bin {
                column: MEASURE.to_string(),
                steps: None,
            })),
            "layer {n} does not bin {MEASURE} — the two layers must share one \
             transform or they are two different pictures stacked up"
        );
        assert_eq!(
            layer.options.get("y"),
            Some(&ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            })),
            "layer {n} does not count its rows"
        );
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
        "the ghost layer is narrowed, so there is no total behind the subset"
    );
    assert_eq!(
        subset_filter.map(|p| p.0),
        Some(selection),
        "the subset layer does not read the selection the brush writes"
    );
}

/// **AC4's other half — the emitter names a token, it does not spell a colour.**
///
/// Two claims, and it takes both. The emitted fill has to be one of the design
/// system's warm-gray steps, which is what a colour invented for the occasion
/// fails; and `src/chart_kinds.rs` must not contain that hex, which is what a
/// literal frozen against the token's current value fails.
///
/// The source read is deliberate rather than lazy. `tests/token_discipline.rs`
/// scans this crate's `src/` for hand-typed colour, but its hex needle is
/// anchored on `0x` — the Rust spelling of a `Color32` channel triple — and a
/// `"#rrggbb"` string bound for a YAML `fill:` slips past it. So the emitter's
/// half of the rule is checked here, beside the reading that depends on it.
#[test]
fn the_registrys_ghost_ink_comes_from_the_design_tokens() {
    let spec = registry_spec();
    let fill = emitted_ghost_fill(layers(the_plot(&spec))[0]);

    let steps: Vec<String> = meridian_design::scales::GRAY_LIGHT
        .iter()
        .map(meridian_design::colour::Rgba::hex)
        .collect();
    assert!(
        steps.contains(&fill),
        "the ghost fill {fill} is not a step of the design system's warm-gray \
         scale, so it is a colour the emitter invented — steps are {steps:?}"
    );

    let emitter = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/chart_kinds.rs"),
    )
    .expect("the emitter's source is readable");
    let written: Vec<&str> = emitter
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter(|line| line.contains(&fill))
        .collect();
    assert!(
        written.is_empty(),
        "the ghost's ink is written into the emitter as the literal {fill}, so a \
         palette regeneration would move the tokens and leave the tile behind: \
         {written:?}"
    );
}

/// **The ghost ink the emitter chose stays measurable against the chart's own
/// chrome.**
///
/// Its own test rather than a line inside the reading below, because it is the
/// reason a different step of the same scale would be the wrong choice: the plot
/// frame's baseline is drawn from that scale too, and a ghost inside the
/// tolerance of it would turn every reading here into a reading of gridlines
/// while still passing.
#[test]
fn the_registrys_ghost_ink_is_not_the_charts_own_chrome() {
    assert_chrome_separated(registry_ghost_ink(), "the registry's");
}

/// **The product's tile, verified by raster: at rest the solid layer covers the
/// ghost, and under a selection the total's bar tops hold.**
///
/// The two halves are one test because the second is measured *against* the
/// first — the resting picture is the unfiltered total, and it is the only thing
/// the filtered picture can be compared with.
///
/// The brush arrives from `root/plot[99]`, a path no plot in this document has,
/// and that is the situation being modelled rather than a shortcut. Crossfilter
/// self-exclusion drops a plot's own clause from its own query, so this tile
/// narrows only on a **sibling's** contribution; the fabricated path stands in
/// for the sibling a generated dashboard puts beside it.
#[test]
fn the_registrys_numeric_tile_holds_the_total_behind_a_brushed_subset() {
    let ghost_ink = registry_ghost_ink();
    let source = registry_document();
    let selection = emitted_selection(the_plot(&registry_spec()));

    let mut live = LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("the registry's block must load live: {e}\n{source}"));
    let resting = live
        .present()
        .unwrap_or_else(|e| panic!("the registry's block must compose: {e}\n{source}"));
    let frame = Frame::of(&resting, 0);
    let before = raster(resting, "registry-resting.png");

    // At rest nothing is filtered, so the subset IS the total and the solid
    // layer covers the ghost exactly. This is also what makes the resting
    // picture usable as the total below.
    let ghost_at_rest = columns_with(&before, &frame, ghost_ink);
    assert!(
        ghost_at_rest.is_empty(),
        "with nothing selected the subset is the total, so the ghost is \
         covered — {} frame columns show it instead",
        ghost_at_rest.len()
    );
    let total_tops = bar_tops(&before, &frame, ghost_ink);
    let drawn = total_tops.iter().filter(|top| top.is_some()).count();
    assert!(
        drawn * 2 > total_tops.len(),
        "fixture check: most of the frame must carry a bar or the readings \
         below compare almost nothing ({drawn} of {} columns)",
        total_tops.len()
    );

    // A sibling tile brushes the lower half of the measure's range. Some bins
    // lose every row; the rest keep some.
    let filtered = live
        .apply(Interaction::Select {
            name: selection,
            contributor: ComponentPath("root/plot[99]".to_string()),
            predicate: SqlPredicate::Interval {
                column: MEASURE.to_string(),
                lo: ScalarValue::Float(0.0),
                hi: ScalarValue::Float(40.0),
                meta: None,
            },
        })
        .expect("a sibling's contribution re-composites the tile");
    let after = raster(filtered, "registry-filtered.png");

    // The ghost is now on the page, in the ink the emitter asked for.
    let ghost_columns = columns_with(&after, &frame, ghost_ink);
    assert!(
        !ghost_columns.is_empty(),
        "the unfiltered total is not drawn behind the subset once the two differ"
    );
    let subset_columns = columns_with(&after, &frame, subset_ink());
    assert!(
        !subset_columns.is_empty(),
        "the filtered layer draws none of the rows the selection kept"
    );
    assert!(
        ghost_columns.iter().any(|c| !subset_columns.contains(c)),
        "no bin stands as ghost alone with no subset ink over it, so the \
         subset is not a subset"
    );

    // **The denominator held.** Column by column, the top of the two layers
    // together is where the unfiltered bar stood before the selection — which is
    // the count axis and the pixel mapping not having moved, because an axis
    // re-anchored to the subset moves every column at once.
    // `a_single_filtered_layer_redraws_the_whole_chart` is the same reading over
    // the shape this tile replaced, and it is what makes this one a measurement
    // rather than a formality.
    //
    // Read in the direction that matters, for the reason the example's half of
    // this file states: a column that LOST its bar is the denominator going
    // missing, while a column that gained one is a bar edge landing on the pixel
    // grid differently between two renders.
    let after_tops = bar_tops(&after, &frame, ghost_ink);
    assert_eq!(
        after_tops.len(),
        total_tops.len(),
        "the two renders are the same size"
    );
    let pairs = || total_tops.iter().zip(after_tops.iter()).enumerate();
    let vanished: Vec<usize> = pairs()
        .filter(|(_, (before, after))| before.is_some() && after.is_none())
        .map(|(x, _)| x)
        .collect();
    assert!(
        vanished.is_empty(),
        "the selected picture lost the bar entirely in these frame columns, so \
         the total is not behind the subset: {vanished:?}"
    );
    let moved: Vec<(usize, u32, u32)> = pairs()
        .filter_map(|(x, (before, after))| match (before, after) {
            (Some(b), Some(a)) if b.abs_diff(*a) > 1 => Some((x, *b, *a)),
            _ => None,
        })
        .collect();
    assert!(
        moved.is_empty(),
        "the selected picture must keep the unfiltered total's bar tops — \
         these columns moved: {moved:?}"
    );
}

/// **The one-layer form, measured the same way — so the reading above is a
/// contrast and not a formality.**
///
/// One `rectY` narrowed by the selection and nothing behind it. Under the same
/// kind of selection, most of the frame's bar tops move. Both the bin edges and
/// the count axis are recomputed from the rows that survived, so the picture is
/// not the same chart with shorter bars — it is a different chart at a different
/// scale, and a reader has nothing left on the page to judge the fraction
/// against.
///
/// The layer is written out here rather than asked of the registry because no
/// kind in the registry builds it. A fixture is the only way to keep the
/// comparison alive.
///
/// This is also the answer to *"could that reading pass on anything?"* — it
/// passes on the two-layer form and fails on the one-layer form, over the same
/// rows, the same predicate and the same measurement.
#[test]
fn a_single_filtered_layer_redraws_the_whole_chart() {
    let source = format!(
        "{rows}params:\n  sel: {{ select: crossfilter }}\nplot:\n  \
         - mark: rectY\n    data: {{ from: {src}, filterBy: $sel }}\n    \
         x: {{ bin: '{MEASURE}' }}\n    y: {{ count: }}\n  \
         - select: intervalX\n    as: $sel\n",
        rows = fixture_rows(),
        src = data_file::SOURCE,
    );
    let mut live = LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("the one-layer fixture must load: {e}\n{source}"));
    let resting = live
        .present()
        .unwrap_or_else(|e| panic!("the one-layer fixture must compose: {e}\n{source}"));
    let frame = Frame::of(&resting, 0);
    let before = raster(resting, "one-layer-resting.png");
    // No ghost ink in this document, so the only bars are the mark's own.
    let before_tops = bar_tops(&before, &frame, subset_ink());

    let filtered = live
        .apply(Interaction::Select {
            name: "sel".to_string(),
            contributor: ComponentPath("root/plot[99]".to_string()),
            predicate: SqlPredicate::Interval {
                column: MEASURE.to_string(),
                lo: ScalarValue::Float(0.0),
                hi: ScalarValue::Float(40.0),
                meta: None,
            },
        })
        .expect("the same contribution re-composites");
    let after = raster(filtered, "one-layer-filtered.png");
    let after_tops = bar_tops(&after, &frame, subset_ink());

    let disturbed = before_tops
        .iter()
        .zip(after_tops.iter())
        .filter(|(before, after)| match (before, after) {
            (Some(b), Some(a)) => b.abs_diff(*a) > 1,
            (Some(_), None) => true,
            _ => false,
        })
        .count();
    assert!(
        disturbed * 2 > before_tops.len(),
        "the one-layer form was expected to redraw most of the frame under a \
         selection, and only {disturbed} of {} columns moved — if this shape has \
         stopped rescaling, the reading in \
         `the_registrys_numeric_tile_holds_the_total_behind_a_brushed_subset` no \
         longer distinguishes the two and needs a different measurement",
        before_tops.len()
    );
}

// ---------------------------------------------------------------------------
// The tile's own brush, driven as a pointer sweep
// ---------------------------------------------------------------------------

/// A directory of this test's own, removed when the test ends.
///
/// The same arrangement `tests/data_file.rs` uses, and for its reason: what is
/// under test is DuckDB reading a file the user chose off a real filesystem, and
/// a committed fixture path would resolve from the repo root and nowhere else.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-ghosted-histogram-{name}-{}-{nanos}",
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

/// The lowest and highest value in [`measure_csv`] — what a clause over
/// [`MEASURE`] can legitimately name, and the interval every bound below must
/// land inside.
const MEASURE_RANGE: (f64, f64) = (0.0, 99.0);

/// A CSV of one numeric column: the same hundred values [`fixture_rows`] holds,
/// as a file for the front door to open.
///
/// One column and no other, so `first_look` has one field to bind and
/// `binned-histogram` is the kind it reaches — the tile under test rather than a
/// neighbour of it, which the test asserts rather than assumes.
fn measure_csv() -> String {
    let mut out = format!("{MEASURE}\n");
    for v in 0..100u32 {
        for _ in 0..=(v % 7) {
            let _ = writeln!(out, "{v}");
        }
    }
    out
}

/// The window this file's gestures are made in.
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

/// A window that has opened `path` through the control the front door's
/// *Open a data file* reaches, with a frame drawn and the raster presented.
fn opened_window(path: &Path, ctx: &egui::Context) -> MeridianApp {
    let mut app = MeridianApp::headless_with_layout(Boot::empty(), default_layout(), Mode::Light);
    gesture_frame(&mut app, ctx, Vec::new());
    app.open_data_file(ctx, &path.to_string_lossy());
    gesture_frame(&mut app, ctx, Vec::new());
    gesture_frame(&mut app, ctx, Vec::new());
    app
}

/// A pointer position at `fraction` across the one plot's **data area**,
/// halfway down it.
///
/// The data area rather than the whole plot rect: the margins carry the axis and
/// its labels, and a pixel out there inverts through the x scale to a value off
/// the end of the domain. A sweep measured over the rect would therefore commit
/// bounds the column never reaches, and [`MEASURE_RANGE`] below would be
/// asserting against the margin width instead of against the data.
fn at(app: &MeridianApp, fraction: f64) -> egui::Pos2 {
    let doc = app.chart_doc();
    let raster = doc
        .raster_rect
        .expect("a settled frame presented the raster");
    let plot = &doc.composed.plots[0];
    let l = &plot.layout;
    let x = plot.rect.x + l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fraction;
    let y = plot.rect.y + (l.plot_y_start() + l.plot_y_end()) / 2.0;
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

/// Press at `from`, move to `to`, release — the frames a real drag occupies.
///
/// Three frames and not one. The gesture machine is edge-triggered on the
/// button, so a press and a release in the same frame is a click; the move
/// between them is what makes this a sweep.
fn drag(app: &mut MeridianApp, ctx: &egui::Context, from: f64, to: f64) {
    let start = at(app, from);
    gesture_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let end = at(app, to);
    gesture_frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    gesture_frame(app, ctx, vec![button(end, false)]);
    gesture_frame(app, ctx, Vec::new());
}

/// A press and release on one pixel — which an interval binding reads as
/// *retract this plot's contribution*.
fn click(app: &mut MeridianApp, ctx: &egui::Context, fraction: f64) {
    let pos = at(app, fraction);
    gesture_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    gesture_frame(app, ctx, vec![button(pos, false)]);
    gesture_frame(app, ctx, Vec::new());
}

/// Every `Interval` clause in a predicate tree, as `(column, lo, hi)`.
///
/// Walked rather than matched at the root because a selection's clause is
/// assembled by `compile_selection` — how many contributors it wraps, and in
/// what, is that function's business and not what this test is about.
fn intervals(predicate: &SqlPredicate) -> Vec<(String, f64, f64)> {
    match predicate {
        SqlPredicate::Interval { column, lo, hi, .. } => match (lo, hi) {
            (ScalarValue::Float(lo), ScalarValue::Float(hi)) => {
                vec![(column.clone(), *lo, *hi)]
            }
            _ => Vec::new(),
        },
        SqlPredicate::And(parts) | SqlPredicate::Or(parts) => {
            parts.iter().flat_map(intervals).collect()
        }
        _ => Vec::new(),
    }
}

/// The one interval the document's selections are holding, or a panic naming
/// what they held instead.
fn held_interval(app: &MeridianApp, whose: &str) -> (String, f64, f64) {
    let held = app
        .chart_doc()
        .live_dashboard()
        .expect("the opened document has a live session")
        .selection_clauses();
    let found: Vec<(String, f64, f64)> = held.iter().flat_map(|(_, p)| intervals(p)).collect();
    match found.as_slice() {
        [one] => one.clone(),
        other => panic!(
            "{whose} was expected to hold exactly one interval clause; the \
             document holds {other:?} (selections: {:?})",
            app.chart_doc().selection_sql()
        ),
    }
}

/// Whether the chart pane's `clear-selection` control is shown, and whether it
/// can act — read off the pane's declared [`Subject`], which is the same list
/// the chrome draws.
///
/// [`Subject`]: brightfield_workbench::Subject
fn clear_selection_control(app: &MeridianApp) -> (ToolbarLocation, bool) {
    let subject = chart_registry()
        .specs()
        .iter()
        .find(|spec| spec.id == CHART)
        .map(|spec| (spec.make)().subject(app.chart_doc()))
        .expect("the chart pane is in the registry");
    let entry = subject
        .toolbar
        .iter()
        .find(|e| e.verb.as_str() == "clear-selection")
        .expect("the chart pane declares a clear-selection control");
    (entry.location, entry.enabled)
}

/// The x scale's domain on the tile — what the axis is showing, asked of the
/// scale the plot was drawn against.
fn x_domain(app: &MeridianApp) -> (f64, f64) {
    let scale = app.chart_doc().composed.plots[0]
        .scales
        .get(brightfield_render::channel::Channel::X)
        .expect("the binned x axis has a scale");
    match scale {
        brightfield_render::scale::Scale::Linear {
            domain_min,
            domain_max,
            ..
        } => (*domain_min, *domain_max),
        other => panic!("a binned measure's x axis is not linear: {other:?}"),
    }
}

/// **A drag on the tile commits the interval the reader swept, over the column
/// the tile bins.**
///
/// The one test in this file that drives the product's own gesture path, and the
/// reason it exists: every other reading here — and every brush test in this
/// crate — hands `Interaction::Select` to the document with a predicate the test
/// wrote. That proves the picture responds to a selection. It cannot prove the
/// tile can *make* one, and this tile shipped an `intervalX` interactor that
/// could not: the interval binding read its column from string-valued channels
/// only, `x: { bin: v }` is not one, and `resolve_gesture` bailed before any
/// `Interaction` was dispatched. Nothing reddened. What a reader got was a brush
/// rectangle that painted and resolved to nothing, and a *Clear selection*
/// control that was shown and permanently disabled.
///
/// So the entry point is the one a reader has: a numeric CSV opened through
/// `MeridianApp::open_data_file`, and pointer events on the presented raster.
/// Four things are asserted, and the middle two are what make it a measurement
/// rather than a liveness check:
///
/// - **At rest nothing is held** and the control is shown-but-disabled, so a
///   harness that found a selection everywhere would say so here.
/// - **A sweep commits an interval naming the binned column**, with bounds
///   inside the column's own range. A derivation that handed back a pixel, a bin
///   ordinal or a count would land outside [`MEASURE_RANGE`].
/// - **The bounds are the sweep's.** A second, narrower sweep inside the first
///   commits bounds inside the first pair, and each bound lands where the x
///   scale's own domain says that fraction of the axis is. A constant, or a
///   whole-domain clause, fails both.
/// - **A click retracts it**, which is the crossfilter convention and the other
///   half of the same code path.
#[test]
fn a_drag_on_the_numeric_tile_commits_the_interval_the_reader_swept() {
    let dir = TempDir::new("numeric-tile-drag");
    let path = dir.write("measure.csv", &measure_csv());
    let ctx = egui::Context::default();
    let mut app = opened_window(&path, &ctx);

    // Fixture check: this really is the ghosted tile, chosen by the registry
    // for a numeric file rather than written out here.
    let authored = app
        .chart_doc()
        .authored()
        .expect("an opened data file records the kind that drew it");
    assert_eq!(
        authored.kind,
        chart_kinds::BINNED_HISTOGRAM,
        "a one-measure CSV opened as some other kind, so the rest of this test \
         is about the wrong picture"
    );

    // The floor, and the reported symptom. Nothing is held; the control is on
    // the toolbar because the tile declares a gesture, and it cannot act
    // because there is nothing to clear.
    assert!(
        app.chart_doc().selection_sql().is_none(),
        "a tile nobody has brushed is holding {:?}",
        app.chart_doc().selection_sql()
    );
    assert_eq!(
        clear_selection_control(&app),
        (ToolbarLocation::Leading, false),
        "at rest the tile's clear-selection control is shown and inert"
    );

    let (d0, d1) = x_domain(&app);
    assert!(
        d1 > d0,
        "the x axis has no width, so no sweep on it could mean anything: \
         ({d0}, {d1})"
    );
    // Where a fraction of the data area lands in the column's units, from the
    // domain the axis is showing. A tolerance of a fiftieth of the domain
    // absorbs the pixel the pointer is rounded to and nothing wider.
    let expect_at = |fraction: f64| d0 + (d1 - d0) * fraction;
    let slack = (d1 - d0) / 50.0;

    // The sweep. Middling fractions on purpose: far enough inside the axis that
    // a bound cannot be right by clamping to an end of the domain.
    drag(&mut app, &ctx, 0.25, 0.60);
    let (column, lo, hi) = held_interval(&app, "a released sweep across the tile");
    assert_eq!(
        column, MEASURE,
        "the committed clause constrains {column} rather than the column the \
         tile bins — the interval binding is not reading the bin transform"
    );
    assert!(
        lo >= MEASURE_RANGE.0 && hi <= MEASURE_RANGE.1 && lo < hi,
        "the sweep committed [{lo}, {hi}], which is not an interval of \
         {MEASURE}'s own values ({:?}) — a bound derived from pixels, bin \
         ordinals or counts lands here",
        MEASURE_RANGE
    );
    assert!(
        (lo - expect_at(0.25)).abs() < slack && (hi - expect_at(0.60)).abs() < slack,
        "a sweep from 0.25 to 0.60 of the axis committed [{lo}, {hi}]; the \
         domain on screen is ({d0}, {d1}), so those fractions are \
         [{}, {}]",
        expect_at(0.25),
        expect_at(0.60)
    );
    assert_eq!(
        clear_selection_control(&app),
        (ToolbarLocation::Leading, true),
        "the committed brush left the clear-selection control unable to act"
    );

    // The bounds are the sweep's, not a constant. A narrower sweep strictly
    // inside the first commits a strictly narrower interval.
    drag(&mut app, &ctx, 0.35, 0.50);
    let (_, inner_lo, inner_hi) = held_interval(&app, "a second, narrower sweep");
    assert!(
        inner_lo > lo && inner_hi < hi,
        "the narrower sweep committed [{inner_lo}, {inner_hi}], which is not \
         inside the first sweep's [{lo}, {hi}] — the clause is not coming from \
         the pixels"
    );
    assert!(
        (inner_lo - expect_at(0.35)).abs() < slack && (inner_hi - expect_at(0.50)).abs() < slack,
        "a sweep from 0.35 to 0.50 of the axis committed [{inner_lo}, \
         {inner_hi}] rather than [{}, {}]",
        expect_at(0.35),
        expect_at(0.50)
    );

    // A click with no sweep retracts this plot's contribution — the other half
    // of the same `resolve_gesture` branch.
    click(&mut app, &ctx, 0.5);
    assert!(
        app.chart_doc().selection_sql().is_none(),
        "a click on an interval binding did not retract the contribution: \
         {:?}",
        app.chart_doc().selection_sql()
    );
    assert_eq!(
        clear_selection_control(&app),
        (ToolbarLocation::Leading, false),
        "the retracted selection left the clear-selection control claiming it \
         can act"
    );
}
