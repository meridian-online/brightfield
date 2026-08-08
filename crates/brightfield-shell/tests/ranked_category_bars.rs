//! **The ranked category bars module, in pixels** — ranked, limited, labelled
//! and ghosted, each measured off the raster the export writes.
//!
//! Every layer under this one can be right while the chart is wrong, and this
//! module is assembled entirely out of those layers: the `ORDER BY` and the
//! `LIMIT` come from `brightfield-sql`, the conditional `SUM` beside them comes
//! from the highlight projection, and none of the three looks at a pixel. The
//! defect this shape exists to prevent is a chart that emits all of it and
//! draws none of it.
//!
//! Four readings, and the reason each is what it is:
//!
//! * **Ranked** — the sequence of bar LENGTHS down the frame, not "is there
//!   ink". Every wrong answer here also produces ink.
//! * **Limited** — the NUMBER of bars, and which ones survive. A cap that kept
//!   the first `n` categories also draws `n` bars, so the lengths say which
//!   `n`.
//! * **Labelled** — knocked-out pixels INSIDE the bar rect. What the label says
//!   is pinned by the unit tests on the formatter; what this adds is that it is
//!   drawn, and drawn on the bar rather than beside it.
//! * **Ghosted** — the one that carries the device. A selection must leave every
//!   bar exactly where and as long as it was, and change only the ink inside
//!   it. Ink-versus-no-ink proves nothing: a chart that re-queried under the
//!   filter and redrew itself at a new scale also has ink everywhere, and looks
//!   entirely reasonable until you ask what fraction of the data it is.
//!
//! It lives in `brightfield-shell` for the reason `sorted_bar_ink.rs` gives:
//! this is the only crate with parse, lower, execute and render in one
//! dependency graph, so a raster test beside the renderer could not see the
//! `ORDER BY` at all and would stay green with the lowerer reverted.
//!
//! Needs a wgpu adapter, as the shell's snapshot and canvas suites already do
//! on the same runner. Not `#[ignore]`d and behind no feature, for the reason
//! `crates/brightfield-shell/tests/snapshot.rs` gives.

use std::path::PathBuf;

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::LabelForm;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{live_spec, Composed};
use brightfield_shell::ranked_bars::{Dashboard, RankedCategoryBars, DEFAULT_LIMIT};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;
use image::RgbaImage;

// ---------------------------------------------------------------------------
// Ink
// ---------------------------------------------------------------------------

/// Per-channel tolerance for an ink match. The same figure the sibling
/// part-of-whole and ghost gates use, and for the same reason: the chart's own
/// chrome is a family of warm greys, so a wide tolerance turns a reading of the
/// bars into a reading of the gridlines.
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

/// The bar ink at full strength: the module binds no colour channel, so it
/// takes the default mark colour. Read from the token layer so a palette bump
/// moves the expectation with the picture rather than reddening this.
fn mark_ink() -> [i32; 3] {
    rgb(meridian_design::viz::MARK_DEFAULT_LIGHT)
}

/// The deemphasis multiplier a `highlight` applies to what it did not select —
/// Mosaic's default, and what `brightfield-render` falls back to when the
/// interactor carries no `otherwise:` style.
///
/// Duplicated here deliberately. The renderer's own constant is private, and
/// what this file needs is not the constant but the COLOUR it puts on the page;
/// naming it means a change to the deemphasis strength reddens this file rather
/// than quietly widening what counts as a bar.
const DEEMPHASIS_ALPHA: f64 = 0.2;

/// The bar ink of a group the selection did NOT account for: the mark colour at
/// [`DEEMPHASIS_ALPHA`] over the plot surface.
///
/// Computed rather than transcribed, so it moves with the palette. **This is
/// the absolute-ink half of every reading below.** A test that asked only
/// "is this pixel different from that one" could not tell a faded bar from a
/// solid one drawn in a palette that had shifted under both, and both are drawn
/// through one routine.
fn faded_ink() -> [i32; 3] {
    let mark = meridian_design::viz::MARK_DEFAULT_LIGHT;
    let surface = meridian_design::chrome::INK_LIGHT.surface;
    let mix = |m: f32, s: f32| {
        (255.0 * (DEEMPHASIS_ALPHA * f64::from(m) + (1.0 - DEEMPHASIS_ALPHA) * f64::from(s)))
            .round() as i32
    };
    [
        mix(mark.r, surface.r),
        mix(mark.g, surface.g),
        mix(mark.b, surface.b),
    ]
}

/// The chart's own furniture — the tokens a frame pixel takes when no bar
/// stands on it.
///
/// `surface` is in this list twice over: it is what a blank frame pixel is, and
/// it is the ink an in-bar label is knocked out in (`meridian_design`'s
/// `text.on_solid`, which that crate defines as the light surface colour in
/// both modes).
fn chrome() -> Vec<[i32; 3]> {
    let ink = meridian_design::chrome::INK_LIGHT;
    [ink.surface, ink.page, ink.gridline, ink.baseline]
        .into_iter()
        .map(rgb)
        .collect()
}

/// Whether a frame pixel carries a bar — either ink, either part.
///
/// A POSITIVE test against the two inks a bar can be, rather than a negative
/// one against the furniture. The negative form is what the sibling
/// part-of-whole gate uses and it is right there, but it cannot be used here:
/// the shell draws a committed point selection as a band across the plot that
/// produced the gesture, so "not the furniture" would report that band as a bar
/// running the full width of the frame. Naming the two inks measures the marks
/// and nothing else.
fn is_bar(p: [u8; 4]) -> bool {
    matches(p, mark_ink()) || matches(p, faded_ink())
}

/// **The readings must not be able to see each other's ink.**
///
/// Three separations, and every assertion in this file rests on all three:
/// the solid ink is not the furniture (or a bar stops being a bar), the faded
/// ink is not the furniture (or the ghost is invisible and every
/// "nothing vanished" assertion passes vacuously), and the two bar inks are not
/// each other (or a part-of-whole reading is unmeasurable).
///
/// Each side is derived independently — the marks from the palette, the
/// furniture from the chrome tokens, the faded ink from both plus a
/// multiplier — so a palette bump that collapsed any pair reddens here rather
/// than passing everywhere.
#[test]
fn the_two_bar_inks_are_not_each_other_and_neither_is_the_furniture() {
    let ink = meridian_design::chrome::INK_LIGHT;
    for (name, token) in [
        ("surface", ink.surface),
        ("page", ink.page),
        ("gridline", ink.gridline),
        ("baseline", ink.baseline),
    ] {
        let c = rgb(token);
        let px = [c[0] as u8, c[1] as u8, c[2] as u8, 255];
        assert!(
            !matches(px, mark_ink()),
            "the plot frame's {name} is inside the solid bar ink's tolerance — \
             the readings below would be reading chrome"
        );
        assert!(
            !matches(px, faded_ink()),
            "the plot frame's {name} is inside the FADED bar ink's tolerance — \
             a ghost bar would be indistinguishable from empty frame, and every \
             assertion that nothing vanished would pass vacuously"
        );
    }
    let solid = mark_ink();
    let faded = faded_ink();
    let px = [solid[0] as u8, solid[1] as u8, solid[2] as u8, 255];
    assert!(
        !matches(px, faded),
        "the deemphasised ink is inside the solid ink's tolerance ({solid:?} \
         against {faded:?}) — there is no part-of-whole reading to measure"
    );
}

// ---------------------------------------------------------------------------
// The fixture, and the specs built over it
// ---------------------------------------------------------------------------

/// `n` categories, the `i`th carrying `i + 1` rows, each row also carrying a
/// second categorical column `side` that splits the categories evenly by parity.
///
/// Distinct counts throughout, so a ranking has exactly one right answer and
/// the length of a bar names which category it is. The categories are emitted
/// in a rotated order rather than in count order, so a lowerer that merely
/// preserved its input is not sitting on sorted rows: `cat-00` (one row) is
/// first and `cat-01` (two rows) is last.
///
/// `side` is what makes the ghost readable. Selecting one `side` selects EVERY
/// row of half the categories and NOT ONE ROW of the other half — so under a
/// filter the second half would produce no group at all and leave the axis,
/// which is the failure AC3's second clause names, while the device keeps them
/// standing at full length with no solid ink in them.
fn fixture_rows(n: usize) -> String {
    let mut rows = Vec::new();
    for step in 0..n {
        let i = (n - 1 + step) % n;
        let side = if i % 2 == 0 { "a" } else { "b" };
        for _ in 0..=i {
            rows.push(format!("    - {{ cat: cat-{i:02}, side: {side} }}"));
        }
    }
    format!("  t:\n{}\n", rows.join("\n"))
}

/// A dashboard of one module over `cat`, built through the module's own
/// builder — so what is rasterised is what a generated dashboard would get,
/// not a spec hand-written to match.
fn module_spec(categories: usize, module: RankedCategoryBars) -> String {
    let mut dash = Dashboard::of_columns(fixture_rows(categories), "t", &["cat"]);
    dash.modules = vec![module];
    dash.to_spec()
}

/// The generated dashboard proper: a module over `cat` beside a module over
/// `side`, both driving and reading the one selection.
fn two_module_dashboard(categories: usize) -> String {
    Dashboard::of_columns(fixture_rows(categories), "t", &["cat", "side"]).to_spec()
}

/// A module beside a chart that consumes the same selection the ordinary way —
/// `filterBy:`, the narrowing form. Appended by hand because a dashboard of
/// modules is all the generator emits.
fn module_beside_a_filtered_chart(categories: usize) -> String {
    let mut source = module_spec(categories, RankedCategoryBars::new("cat"));
    source.push_str(
        "  - plot:\n    - mark: dot\n      data: { from: t, filterBy: $sel }\n      \
         x: side\n      y: cat\n    width: 360\n    height: 300\n",
    );
    source
}

/// Write a generated spec somewhere `live_spec` can open it.
fn spec_path(source: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("bf-ranked-category-bars");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{name}.yaml"));
    std::fs::write(&path, source).expect("write spec");
    path
}

/// Render a composition and read its pixels back — the composed Vello scene
/// and nothing else, the same bytes an export writes.
fn raster(composed: Composed, name: &str) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-ranked-category-bars");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(format!("{name}.png"));
    capture_vello_only(composed, 1.0, &png).expect("export");
    image::open(&png).expect("open png").to_rgba8()
}

/// Compose a generated spec and rasterise it, with no interaction.
fn raster_of(source: &str, name: &str) -> (RgbaImage, Frame) {
    let path = spec_path(source, name);
    let (_, composed) = live_spec(path.to_str().expect("utf-8 path")).expect("the spec loads");
    let frame = Frame::of(&composed, 0);
    (raster(composed, name), frame)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A plot's frame in image pixels — where bars are allowed to be, and the
/// region every reading is taken over.
///
/// Text is what forces this. A category name on the y axis is `ink_secondary`
/// anti-aliased against the surface, and a partial-coverage pixel of that blend
/// is neither token; a reading over the whole image would report the axis
/// labels as bars. The labels live in the margins by construction, so reading
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

/// The rightmost frame column carrying bar ink, per frame ROW — the tip of
/// whatever bar stands in that row.
///
/// Rightmost rather than a count of inked pixels, because an in-bar label is
/// knocked out of the fill and punches holes a count would subtract. A `barX`
/// grows from the baseline at the frame's left edge, so the tip is the whole
/// measurement.
fn row_tips(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    (frame.y0..frame.y1)
        .map(|y| {
            (frame.x0..frame.x1)
                .rev()
                .find(|&x| is_bar(img.get_pixel(x, y).0))
        })
        .collect()
}

/// The rightmost frame column carrying the mark's FULL ink, per frame row —
/// the tip of the selected PART, when one is drawn.
fn row_solid_tips(img: &RgbaImage, frame: &Frame) -> Vec<Option<u32>> {
    let want = mark_ink();
    (frame.y0..frame.y1)
        .map(|y| {
            (frame.x0..frame.x1)
                .rev()
                .find(|&x| matches(img.get_pixel(x, y).0, want))
        })
        .collect()
}

/// One bar, as measured off the raster.
#[derive(Debug, Clone, Copy)]
struct Bar {
    /// Index into the frame's rows of the first row this bar occupies.
    first_row: usize,
    /// One past the last.
    end_row: usize,
    /// The bar's length in pixels, from the baseline at the frame's left edge.
    length: u32,
}

/// Every bar in the frame, TOP TO BOTTOM.
///
/// One entry per maximal run of rows carrying ink, taking the run's longest
/// tip: a bar's first and last rows are anti-aliased and read short while its
/// interior is flat. Neither sorted nor de-duplicated — the sequence is the
/// measurement.
fn bars(img: &RgbaImage, frame: &Frame) -> Vec<Bar> {
    /// Rows below which a run is an artefact rather than a bar.
    const MIN_ROWS: usize = 3;
    let tips = row_tips(img, frame);
    let mut out: Vec<Bar> = Vec::new();
    let mut run_start = None;
    let mut longest = 0_u32;
    for (i, tip) in tips.iter().enumerate() {
        match tip {
            Some(t) => {
                run_start.get_or_insert(i);
                longest = longest.max(t.saturating_sub(frame.x0));
            }
            None => {
                if let Some(start) = run_start.take() {
                    if i - start >= MIN_ROWS {
                        out.push(Bar {
                            first_row: start,
                            end_row: i,
                            length: longest,
                        });
                    }
                }
                longest = 0;
            }
        }
    }
    if let Some(start) = run_start {
        if tips.len() - start >= MIN_ROWS {
            out.push(Bar {
                first_row: start,
                end_row: tips.len(),
                length: longest,
            });
        }
    }
    out
}

/// Bar lengths as fractions of the longest bar in the same picture, so the
/// assertion is about the data and not about the plot's pixel width.
fn ratios(bars: &[Bar]) -> Vec<f64> {
    let longest = f64::from(
        bars.iter()
            .map(|b| b.length)
            .max()
            .expect("at least one bar"),
    );
    bars.iter().map(|b| f64::from(b.length) / longest).collect()
}

/// Per-bar tolerance on those fractions. The bars here are hundreds of pixels
/// long, so this is a handful of pixels.
const RATIO_TOL: f64 = 0.03;

fn assert_ratios(found: &[f64], expected: &[f64], case: &str) {
    assert_eq!(
        found.len(),
        expected.len(),
        "{case}: bar COUNT differs — found {found:?}, expected {expected:?}"
    );
    for (i, (f, e)) in found.iter().zip(expected).enumerate() {
        assert!(
            (f - e).abs() < RATIO_TOL,
            "{case}: bar {i} down the frame is {f:.3} of the longest, expected \
             {e:.3} — whole picture {found:?} against {expected:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC1 — ranked, with a default cap and a settable one
// ---------------------------------------------------------------------------

/// **The module ranks, in pixels.**
///
/// A band scale's first row is at the BOTTOM of the frame — `Layout::y_range`
/// returns `(bottom, top)` — so a descending ranking reads as bars getting
/// LONGER down the page. Fourteen categories worth 1…14 rows, capped at the
/// default ten, leaves the ten longest: 5…14 rows, drawn 5 at the top and 14 at
/// the bottom.
#[test]
fn a_module_draws_its_categories_longest_last_and_stops_at_the_default_cap() {
    let source = module_spec(14, RankedCategoryBars::new("cat"));
    let (img, frame) = raster_of(&source, "default-cap");
    let found = bars(&img, &frame);
    assert_eq!(
        found.len(),
        DEFAULT_LIMIT,
        "a module that says nothing about a cap draws {DEFAULT_LIMIT} bars, \
         not {}",
        found.len()
    );
    // The ten survivors are the ten longest: 5..=14 rows, against 14.
    let expected: Vec<f64> = (5..=14).map(|n| f64::from(n) / 14.0).collect();
    assert_ratios(
        &ratios(&found),
        &expected,
        "the default cap keeps the TOP ten and draws them in rank order — a cap \
         that kept the first ten categories instead would draw 1..=10 here",
    );
}

/// **The cap is the module's to set**, and what survives it is chosen by the
/// ranking rather than by the row order.
#[test]
fn a_configured_cap_leaves_exactly_its_own_bars_on_the_page() {
    for limit in [2_usize, 3, 5] {
        let source = module_spec(14, RankedCategoryBars::new("cat").with_limit(limit));
        let (img, frame) = raster_of(&source, &format!("cap-{limit}"));
        let found = bars(&img, &frame);
        let expected: Vec<f64> = (0..limit)
            .map(|i| f64::from(14 - (limit - 1 - i) as u32) / 14.0)
            .collect();
        assert_ratios(
            &ratios(&found),
            &expected,
            &format!("`limit: {limit}` keeps the {limit} longest and no others"),
        );
    }
}

// ---------------------------------------------------------------------------
// AC2 — each bar carries a readable number
// ---------------------------------------------------------------------------

/// Frame pixels strictly inside a bar that are NOT the bar's own ink — right of
/// the baseline, left of the tip, and inside the bar's rows.
///
/// That is the label and nothing else. A bar is a flat fill: the gridlines it
/// crosses are drawn under it and covered, so an unlabelled bar's interior is
/// one colour throughout and this counts zero. Counting "not the fill" rather
/// than "is the knockout token" is deliberate — a glyph is anti-aliased, and
/// most of a ten-pixel digit's pixels are a partial blend that no single token
/// matches.
fn unfilled_pixels_inside_bars(img: &RgbaImage, frame: &Frame) -> usize {
    let want = mark_ink();
    let tips = row_tips(img, frame);
    bars(img, frame)
        .iter()
        .map(|bar| {
            (bar.first_row + 2..bar.end_row.saturating_sub(2))
                .filter_map(|r| tips[r].map(|t| (r, t)))
                .map(|(r, tip)| {
                    let y = frame.y0 + r as u32;
                    (frame.x0 + 3..tip)
                        .filter(|&x| !matches(img.get_pixel(x, y).0, want))
                        .count()
                })
                .sum::<usize>()
        })
        .sum()
}

/// **AC2, in pixels.** A labelled module knocks its number out of the bar; the
/// same module with the label turned off does not.
///
/// The control is the whole assertion. Ink inside a bar is not evidence on its
/// own — a bar with a hole in it for any reason would produce some — so what is
/// asserted is a DIFFERENCE against the identical chart with one line removed.
/// What the label SAYS is pinned separately, by the formatter's unit tests;
/// what this adds is that it is drawn, and drawn on the bar.
#[test]
fn each_bar_carries_its_number_and_carries_none_when_asked_not_to() {
    let labelled = module_spec(6, RankedCategoryBars::new("cat"));
    let (img, frame) = raster_of(&labelled, "labelled");
    let with = unfilled_pixels_inside_bars(&img, &frame);

    let bare = module_spec(6, RankedCategoryBars::new("cat").with_label(None));
    let (img, frame) = raster_of(&bare, "unlabelled");
    let without = unfilled_pixels_inside_bars(&img, &frame);

    assert_eq!(
        without, 0,
        "with no `label:` a bar is a flat fill — {without} pixels inside one \
         are not the fill, so this reading is measuring something else"
    );
    assert!(
        with > 100,
        "a labelled module knocks its number out of every bar; found only \
         {with} such pixels across the whole frame"
    );

    // The percentage form draws over the same geometry, and draws MORE: a
    // percentage carries a `%` the count does not, so a run that produced the
    // same figure for both would be reading something that is not the text.
    let pct = module_spec(
        6,
        RankedCategoryBars::new("cat").with_label(Some(LabelForm::Percent)),
    );
    let (img, frame) = raster_of(&pct, "percent");
    let percent = unfilled_pixels_inside_bars(&img, &frame);
    assert!(
        percent > with,
        "`label: percent` prints a longer string than `label: count` — {percent} \
         knocked-out pixels against {with}"
    );
}

// ---------------------------------------------------------------------------
// AC3 / AC4 — the total behind the subset, and the reach across the spec
// ---------------------------------------------------------------------------

/// A point selection on one column.
fn pick(column: &str, value: &str) -> SqlPredicate {
    SqlPredicate::Point {
        column: column.to_string(),
        values: vec![ScalarValue::Text(value.to_string())],
        meta: None,
    }
}

/// **The device, verified by raster** — and the sharpest form of AC3's second
/// clause available.
///
/// The gesture is made on the `side` module and read off the `cat` module, for
/// two reasons. It puts a genuine cross-filter under the assertion — the
/// selected column is not one of the measured mark's grouping dimensions,
/// which is the case a `WHERE` could not express at all. And the shell draws a
/// committed point selection as a band across the plot that PRODUCED it, so
/// reading the other plot measures the marks rather than that chrome.
///
/// `side: b` selects every row of the odd-numbered categories and not one row
/// of the even ones. So under a `filterBy:` the even categories would produce
/// no group and **half the axis would disappear** — with the selected value
/// deciding which half. That is the failure; what is asserted is its absence,
/// alongside the two others:
///
/// * a bar that moved or changed length is a ranking or a scale recomputed
///   under the filter, so the denominator left the page;
/// * a bar drawn in one ink throughout carries no part-of-whole reading at all,
///   however correct its length.
#[test]
fn a_selection_changes_the_ink_inside_the_bars_and_nothing_else() {
    let source = two_module_dashboard(6);
    let path = spec_path(&source, "ghost");
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the spec loads live");
    assert_eq!(
        resting.plots.len(),
        2,
        "the dashboard is one module per categorical column"
    );
    // Plot 0 counts `cat` and is what is measured; plot 1 counts `side` and is
    // where the gesture happens.
    let gesturing = ComponentPath(resting.plots[1].path.clone());
    let frame = Frame::of(&resting, 0);

    // At rest nothing is selected, no per-group counts are projected, and every
    // bar is full mark ink to its tip. That resting picture is the total, and
    // it is what the selected one is measured against.
    let before = raster(resting, "ghost-resting");
    let total = bars(&before, &frame);
    assert_eq!(total.len(), 6, "fixture check: six categories, six bars");
    let rest_tips = row_tips(&before, &frame);
    let rest_solid = row_solid_tips(&before, &frame);
    let faded_at_rest: Vec<usize> = interior_rows(&total)
        .filter(|r| match (rest_tips[*r], rest_solid[*r]) {
            (Some(t), Some(s)) => t.abs_diff(s) > 1,
            (Some(_), None) => true,
            _ => false,
        })
        .collect();
    assert!(
        faded_at_rest.is_empty(),
        "with nothing selected a bar is not part of anything, so it is mark ink \
         to the tip — these frame rows are faded already: {faded_at_rest:?}"
    );

    let selected = live
        .apply(Interaction::Select {
            name: "sel".to_string(),
            contributor: gesturing,
            predicate: pick("side", "b"),
        })
        .expect("the selection re-composites");
    let after = raster(selected, "ghost-selected");
    let after_bars = bars(&after, &frame);

    // **No category left the axis, and no bar moved.**
    assert_eq!(
        after_bars.len(),
        total.len(),
        "a category left the axis when the selection excluded every one of its \
         rows: {} bars became {}",
        total.len(),
        after_bars.len()
    );
    for (i, (b, a)) in total.iter().zip(after_bars.iter()).enumerate() {
        assert!(
            b.length.abs_diff(a.length) <= 2,
            "bar {i} changed length under a gesture that changed no data: \
             {} px became {} px",
            b.length,
            a.length
        );
        assert!(
            b.first_row.abs_diff(a.first_row) <= 2,
            "bar {i} moved on the axis: row {} became row {}",
            b.first_row,
            a.first_row
        );
    }

    // **The whole faded, and the part that was selected stayed solid.** Three
    // categories had every row selected and are solid to their tips; the other
    // three had none and carry no full-strength ink at all. Both halves are
    // asserted: a chart that faded everything and a chart that faded nothing
    // each satisfy only one of them.
    let after_tips = row_tips(&after, &frame);
    let after_solid = row_solid_tips(&after, &frame);
    let mut solid = 0;
    let mut empty = 0;
    for bar in &after_bars {
        let rows: Vec<usize> = interior_rows(std::slice::from_ref(bar)).collect();
        let to_tip = rows
            .iter()
            .all(|r| match (after_tips[*r], after_solid[*r]) {
                (Some(t), Some(s)) => t.abs_diff(s) <= 1,
                _ => false,
            });
        let none = rows.iter().all(|r| after_solid[*r].is_none());
        if to_tip {
            solid += 1;
        }
        if none {
            empty += 1;
        }
    }
    assert_eq!(
        solid, 3,
        "three of the six categories had every row selected, so three bars are \
         solid to their tips — found {solid}"
    );
    assert_eq!(
        empty, 3,
        "the other three had none, so they stand at full length in the faded \
         ink with no selected part inside them — found {empty}"
    );
}

/// The rows strictly inside a bar — its anti-aliased first and last rows
/// dropped, because a partial-coverage row blends toward the surface and its
/// measured tip moves with the ink behind it rather than with the bar.
fn interior_rows(bars: &[Bar]) -> impl Iterator<Item = usize> + '_ {
    bars.iter()
        .flat_map(|b| (b.first_row + 2)..b.end_row.saturating_sub(2))
}

/// **AC4.** A selection made on the module reaches a chart that is not the
/// module, in the same spec.
///
/// Two consumers, because a selection reaches its readers two ways and the
/// module deliberately uses the second: `filterBy:` NARROWS a chart's rows,
/// and `highlight` leaves them alone and moves the ink inside. The dot plot
/// here takes the first form and loses points; the assertion is that the
/// selection crossed the plot boundary at all, which is the part that is
/// adoption rather than authorship.
#[test]
fn a_selection_on_the_module_reaches_the_other_charts_in_the_spec() {
    let source = module_beside_a_filtered_chart(6);
    let path = spec_path(&source, "crossfilter");
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the spec loads live");
    assert_eq!(resting.plots.len(), 2, "the module and its consumer");
    let module = ComponentPath(resting.plots[0].path.clone());
    let detail = Frame::of(&resting, 1);

    let before = raster(resting, "crossfilter-resting");
    let before_ink = inked(&before, &detail);
    assert!(
        before_ink > 0,
        "fixture check: the consumer chart draws something at rest"
    );

    let filtered = live
        .apply(Interaction::Select {
            name: "sel".to_string(),
            contributor: module,
            predicate: pick("cat", "cat-00"),
        })
        .expect("the selection re-composites");
    let after = raster(filtered, "crossfilter-filtered");
    let after_ink = inked(&after, &detail);

    assert!(
        after_ink < before_ink,
        "the consumer chart is filtered by a selection made on the module — \
         {before_ink} inked pixels became {after_ink}"
    );
    assert!(
        after_ink > 0,
        "…to the rows the selection kept, not to nothing: {after_ink}"
    );
}

/// Frame pixels carrying either mark ink.
fn inked(img: &RgbaImage, frame: &Frame) -> usize {
    (frame.y0..frame.y1)
        .flat_map(|y| (frame.x0..frame.x1).map(move |x| (x, y)))
        .filter(|(x, y)| is_bar(img.get_pixel(*x, *y).0))
        .count()
}
