//! **The part-of-whole cross-filtered bar chart** — one bar per bin, standing
//! at its unfiltered height, with the part the selection accounts for drawn
//! inside it.
//!
//! One `rectY` layer over `observations`, unfiltered, under a Mosaic
//! `highlight` interactor bound to the brush on the sibling scatter. The bars
//! never narrow; what changes is the ink inside them.
//!
//! The failure this exists to catch is the one the card was written for: a bar
//! chart cross-filtered by a selection made elsewhere that shows NOTHING about
//! it. Ink-versus-no-ink is therefore not enough — a chart drawing every bar in
//! one colour passes that. What is asserted instead is that the picture carries
//! the two quantities a part-of-whole reading needs, and that they are
//! genuinely different:
//!
//!   * the bar tops do not move when the brush lands, so the denominator is
//!     still on the page;
//!   * the selected ink is a PROPER part — it starts at the baseline and stops
//!     short of the top in a bin the brush only partly covers;
//!   * a bin the brush excludes entirely keeps its bar and carries no selected
//!     ink at all.
//!
//! Read off the raster through `capture_vello_only`, which is the composed
//! Vello scene and nothing else — the same bytes an export writes.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{live_spec, Composed};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;

use image::RgbaImage;
use std::path::PathBuf;

/// The unselected part's ink — the pale constant
/// `examples/rect-bin-count-part-of-whole.yaml` binds to its `highlight`.
const UNSELECTED: [i32; 3] = [0xc4, 0xbc, 0xb0];

/// The selected part's ink: the mark binds no colour channel, so it takes the
/// default mark colour, read from the token layer so a palette bump moves the
/// expectation with the picture.
fn selected_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance. Narrower than the mark-ink tolerance the other pixel
/// gates use, because the unselected ink is a warm grey and so is the chart's
/// own chrome — `the_unselected_ink_is_not_the_charts_own_chrome` is what holds
/// the two apart.
const INK_TOL: i32 = 12;

fn matches(p: [u8; 4], want: [i32; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= INK_TOL)
}

/// **The unselected part is measurable only while its ink is nobody else's.**
///
/// The example picks a literal, the chrome comes from tokens, and neither knows
/// about the other. A palette bump that walked one of them into range would
/// redden this rather than silently turn the readings below into a reading of
/// the gridlines.
#[test]
fn the_unselected_ink_is_not_the_charts_own_chrome() {
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
            !matches(rgba, UNSELECTED),
            "the plot frame's {name} is inside the unselected part's measurement \
             tolerance — the readings below would be reading chrome"
        );
    }
}

/// Render a composition and read its pixels back.
fn raster(composed: Composed, name: &str) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-part-of-whole-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(name);
    capture_vello_only(composed, 1.0, &png).expect("export");
    image::open(&png).expect("open png").to_rgba8()
}

/// The histogram plot's frame in image pixels — where bars are allowed to be,
/// and the region every reading below is taken over.
///
/// Text is what forces this: an axis label is anti-aliased against the surface,
/// and a low-coverage pixel of that blend is a warm grey like every other warm
/// grey. The labels live in the margins by construction, so reading inside the
/// frame excludes them geometrically rather than by hoping a tolerance
/// separates them.
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

/// The topmost frame row carrying `want`, per frame column. `None` for a column
/// with no such ink.
fn tops_of(img: &RgbaImage, frame: &Frame, want: [i32; 3]) -> Vec<Option<u32>> {
    (frame.x0..frame.x1)
        .map(|x| (frame.y0..frame.y1).find(|&y| matches(img.get_pixel(x, y).0, want)))
        .collect()
}

/// The topmost frame row carrying EITHER ink, per frame column — the top of the
/// whole bar standing in that column, whichever part drew it.
fn bar_tops(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    let selected = selected_ink();
    (frame.x0..frame.x1)
        .map(|x| {
            (frame.y0..frame.y1).find(|&y| {
                let p = img.get_pixel(x, y).0;
                matches(p, UNSELECTED) || matches(p, selected)
            })
        })
        .collect()
}

/// **The device, verified by raster.**
#[test]
fn a_highlighted_aggregating_mark_draws_the_selected_part_inside_each_bar() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-part-of-whole.yaml");
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the example loads live");
    let brushed_plot = ComponentPath(resting.plots[0].path.clone());
    // The histogram is the second plot; the first is the scatter the brush is
    // drawn on.
    let frame = Frame::of(&resting, 1);

    // At rest nothing is selected, so no membership column is projected and the
    // bars are drawn exactly as an uninteracted chart's: mark ink to the top,
    // no pale part anywhere. That resting picture is the total, and it is what
    // the brushed one is measured against.
    let before = raster(resting, "resting.png");
    let unselected_at_rest = tops_of(&before, &frame, UNSELECTED);
    assert!(
        unselected_at_rest.iter().all(Option::is_none),
        "with nothing selected the bar is not part-of-anything, so the pale ink \
         must be absent — {} frame columns carry it",
        unselected_at_rest.iter().filter(|t| t.is_some()).count()
    );
    let total_tops = bar_tops(&before, &frame);
    let bar_columns = total_tops.iter().filter(|t| t.is_some()).count();
    assert!(
        bar_columns * 2 > total_tops.len(),
        "fixture check: most of the frame must carry a bar for the readings \
         below to be measuring the chart ({bar_columns} of {} columns)",
        total_tops.len()
    );

    // Brush the scatter down to its low temperatures. The subset is a real
    // subset: some bins fall entirely outside it, others only partly.
    let brushed = live
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
    let after = raster(brushed, "brushed.png");

    // **The denominator held.** Column by column, the bar still tops out where
    // it did before the brush. A mark that filtered instead of highlighting, or
    // a count axis that re-anchored to the subset, moves this.
    //
    // Read in the direction that matters. A column that LOST its bar is the
    // failure — that is the denominator going missing. A column that gained one
    // is a bar edge landing on the pixel grid differently between two renders,
    // which the inter-bar gaps do and which says nothing about either height.
    let after_tops = bar_tops(&after, &frame);
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
        "the brushed picture lost the bar entirely in these frame columns, so \
         the total is no longer on the page: {vanished:?}"
    );
    let moved: Vec<(usize, u32, u32)> = pairs()
        .filter_map(|(x, (b, a))| match (b, a) {
            (Some(b), Some(a)) if b.abs_diff(*a) > 1 => Some((x, *b, *a)),
            _ => None,
        })
        .collect();
    assert!(
        moved.is_empty(),
        "the brushed picture must keep the unfiltered bar tops — these columns \
         moved: {moved:?}"
    );

    // **The selection is drawn, and it is drawn as a PART.** In a column the
    // brush covers only partly, the selected ink starts somewhere below the top
    // of the bar: its topmost row is strictly greater than the bar's. A chart
    // that redrew the whole bar in one ink — the failure this card names —
    // has no such column.
    let selected_tops = tops_of(&after, &frame, selected_ink());
    let partial: Vec<usize> = selected_tops
        .iter()
        .zip(after_tops.iter())
        .enumerate()
        .filter_map(|(x, (s, b))| match (s, b) {
            (Some(s), Some(b)) if *s > *b => Some(x),
            _ => None,
        })
        .collect();
    assert!(
        !partial.is_empty(),
        "no frame column shows the selection as a part of its bar — the picture \
         gives no account of what the selection accounts for"
    );

    // **And a bin the brush excludes keeps its bar with no selected ink in it.**
    // Without this the previous assertion is satisfiable by a chart that shades
    // every bar the same fraction.
    let untouched: Vec<usize> = selected_tops
        .iter()
        .zip(after_tops.iter())
        .enumerate()
        .filter(|(_, (s, b))| s.is_none() && b.is_some())
        .map(|(x, _)| x)
        .collect();
    assert!(
        !untouched.is_empty(),
        "every column that has a bar also has selected ink, so the treatment is \
         not discriminating between bins the brush reached and bins it did not"
    );
}
