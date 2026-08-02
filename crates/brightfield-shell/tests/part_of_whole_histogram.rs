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
//!     well short of the top in a bin the brush only partly covers;
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

/// Per-channel tolerance for an ink match.
const INK_TOL: i32 = 12;

fn rgb(c: meridian_design::colour::Rgba) -> [i32; 3] {
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

fn matches(p: [u8; 4], want: [i32; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= INK_TOL)
}

/// The selected part's ink: the mark binds no colour channel, so it takes the
/// default mark colour, read from the token layer so a palette bump moves the
/// expectation with the picture.
fn selected_ink() -> [i32; 3] {
    rgb(meridian_design::viz::MARK_DEFAULT_LIGHT)
}

/// The chart's own furniture — everything a frame pixel can be that is not a
/// bar. Named from tokens for the same reason the mark ink is.
fn chrome() -> Vec<[i32; 3]> {
    let ink = meridian_design::chrome::INK_LIGHT;
    [ink.surface, ink.page, ink.gridline, ink.baseline]
        .into_iter()
        .map(rgb)
        .collect()
}

/// Whether a frame pixel carries a bar — either part of one. Asked as "not the
/// furniture" rather than as a list of bar inks, because the unselected part is
/// the mark colour composited against whatever is behind it, so it is one ink
/// over blank surface and a slightly different one over a gridline.
fn is_bar(p: [u8; 4], chrome: &[[i32; 3]]) -> bool {
    !chrome.iter().any(|c| matches(p, *c))
}

/// **The two readings must not be able to see each other's ink.**
///
/// `is_bar` is a negative test against the chrome tokens and `selected_ink` a
/// positive one against the palette; neither knows about the other. A palette
/// bump that walked the mark colour into a chrome token's tolerance would make
/// the selected part invisible to `is_bar` and every assertion below vacuous,
/// so it reddens here instead.
#[test]
fn the_mark_ink_is_not_the_charts_own_furniture() {
    for (name, token) in [
        ("surface", meridian_design::chrome::INK_LIGHT.surface),
        ("page", meridian_design::chrome::INK_LIGHT.page),
        ("gridline", meridian_design::chrome::INK_LIGHT.gridline),
        ("baseline", meridian_design::chrome::INK_LIGHT.baseline),
    ] {
        let c = rgb(token);
        let px = [c[0] as u8, c[1] as u8, c[2] as u8, 255];
        assert!(
            !matches(px, selected_ink()),
            "the plot frame's {name} is inside the mark ink's tolerance — the \
             readings below would be reading chrome"
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
/// and a low-coverage pixel of that blend is neither the surface token nor the
/// mark ink, so a reading over the whole image would report tick labels as
/// bars. The labels live in the margins by construction, so reading inside the
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

/// The topmost frame row carrying a bar, per frame column — the top of the
/// whole bar standing there, whichever part drew it.
fn bar_tops(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    let chrome = chrome();
    (frame.x0..frame.x1)
        .map(|x| (frame.y0..frame.y1).find(|&y| is_bar(img.get_pixel(x, y).0, &chrome)))
        .collect()
}

/// The topmost frame row carrying the mark's FULL ink, per frame column — the
/// top of the selected part.
fn selected_tops(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    (frame.x0..frame.x1)
        .map(|x| (frame.y0..frame.y1).find(|&y| matches(img.get_pixel(x, y).0, selected_ink())))
        .collect()
}

/// Which frame columns are the SOLID interior of a bar, judged on the resting
/// render: a few rows under the bar's top the pixel is the mark's full ink.
///
/// Two kinds of column are excluded, and both would otherwise wreck a height
/// comparison. A bar's outermost column is a partial-coverage sliver, and what
/// that sliver blends to depends on how strong the ink behind it is — a faded
/// edge lands inside the surface's tolerance where a full-ink edge does not, so
/// its MEASURED top moves when the ink changes although the bar did not. And
/// where two bars abut, the shared column is a seam neither rect covers fully,
/// so it never reaches the mark's ink at all.
fn solid_columns(img: &RgbaImage, frame: &Frame, tops: &[Option<u32>]) -> Vec<bool> {
    /// How far under the top to sample: past the antialiased cap, still well
    /// inside the shortest bar the fixture draws.
    const UNDER_TOP: u32 = 3;
    tops.iter()
        .enumerate()
        .map(|(i, top)| match top {
            Some(t) => {
                let x = frame.x0 + i as u32;
                let y = t + UNDER_TOP;
                y < frame.y1 && matches(img.get_pixel(x, y).0, selected_ink())
            }
            None => false,
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

    // At rest nothing is selected, so no per-group counts are projected and the
    // bars are drawn exactly as an uninteracted chart's: full mark ink all the
    // way to the top, no faded part anywhere. That resting picture is the
    // total, and it is what the brushed one is measured against.
    let before = raster(resting, "resting.png");
    let total_tops = bar_tops(&before, &frame);
    let rest_selected = selected_tops(&before, &frame);
    let bar_columns = total_tops.iter().filter(|t| t.is_some()).count();
    assert!(
        bar_columns * 2 > total_tops.len(),
        "fixture check: most of the frame must carry a bar for the readings \
         below to be measuring the chart ({bar_columns} of {} columns)",
        total_tops.len()
    );
    let inside = solid_columns(&before, &frame, &total_tops);
    let measured = inside.iter().filter(|i| **i).count();
    assert!(
        measured * 3 > total_tops.len(),
        "fixture check: too little of the frame is solid bar interior for the \
         height comparisons below to be measuring the chart ({measured} of {} \
         columns)",
        total_tops.len()
    );
    let faded_at_rest: Vec<usize> = total_tops
        .iter()
        .zip(rest_selected.iter())
        .enumerate()
        .filter(|(x, _)| inside[*x])
        .filter(|(_, (b, s))| match (b, s) {
            (Some(b), Some(s)) => s.abs_diff(*b) > 1,
            (Some(_), None) => true,
            _ => false,
        })
        .map(|(x, _)| x)
        .collect();
    assert!(
        faded_at_rest.is_empty(),
        "with nothing selected the bar is not part of anything, so it is mark \
         ink to the top — these frame columns are faded already: {faded_at_rest:?}"
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
    let pairs = || {
        total_tops
            .iter()
            .zip(after_tops.iter())
            .enumerate()
            .filter(|(x, _)| inside[*x])
    };
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
    // brush covers only partly, the full-ink part starts well below the top of
    // the bar. A chart that redrew every bar in one ink — the failure this card
    // names — has no such column.
    let after_selected = selected_tops(&after, &frame);
    let partial: Vec<usize> = after_selected
        .iter()
        .zip(after_tops.iter())
        .enumerate()
        .filter_map(|(x, (s, b))| match (s, b) {
            (Some(s), Some(b)) if *s > b + 2 => Some(x),
            _ => None,
        })
        .collect();
    assert!(
        !partial.is_empty(),
        "no frame column shows the selection as a part of its bar — the picture \
         gives no account of what the selection accounts for"
    );

    // **And a bin the brush excludes keeps its bar with no selected ink in it.**
    // Without this the assertion above is satisfiable by a chart that shades
    // every bar the same fraction.
    let untouched: Vec<usize> = after_selected
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
