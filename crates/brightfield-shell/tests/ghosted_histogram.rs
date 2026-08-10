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
//! # Two documents, one device
//!
//! The first half of this file holds `examples/rect-bin-count-ghost.yaml`, the
//! device authored by hand. The second half holds the same device as the
//! **product** emits it: the block `binned-histogram` builds in
//! `brightfield_shell::chart_kinds`, which is the picture every numeric column
//! of an opened file becomes. They are asserted separately on purpose — the
//! example says the engine can draw this, and only the registry half says the
//! shell asks it to.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::chart_kinds;
use brightfield_shell::pipeline::{live_spec, Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::registry::{Field, FieldType};

use image::RgbaImage;
use std::path::PathBuf;

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

/// **The ghost is measurable only while its ink is nobody else's.**
///
/// The example picks a literal, the chrome comes from tokens, and neither
/// knows about the other. A palette bump that walked one of them into the
/// ghost's range would redden this rather than silently turn
/// `columns_with(GHOST)` into a reading of the gridlines.
#[test]
fn the_ghost_ink_is_not_the_charts_own_chrome() {
    let ink = meridian_design::chrome::INK_LIGHT;
    for (name, token) in [
        ("surface", ink.surface),
        ("gridline", ink.gridline),
        ("baseline", ink.baseline),
        ("ink_muted", ink.ink_muted),
        ("ink_secondary", ink.ink_secondary),
        ("ink_primary", ink.ink_primary),
    ] {
        let rgba = [
            (token.r * 255.0).round() as u8,
            (token.g * 255.0).round() as u8,
            (token.b * 255.0).round() as u8,
            255,
        ];
        assert!(
            !matches(rgba, GHOST),
            "the plot frame's {name} is inside the ghost's measurement \
             tolerance — the ghost reading below would be reading chrome"
        );
    }
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
