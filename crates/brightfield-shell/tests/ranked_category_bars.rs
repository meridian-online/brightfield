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
//! * **Labelled** — the label is READ BACK off the bar and matched against the
//!   string it should be. Presence alone is not enough and the gap was real:
//!   with only a presence reading, `drawn_total := 2 * sum` (every percentage
//!   halved) and `selected := value` (every bar reading `N / N`) both passed
//!   the whole workspace. The formatter's unit tests pin what `bar_label`
//!   returns *in isolation*; nothing pinned the renderer's wiring of the two
//!   numbers INTO it, which is the half a reader actually sees.
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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::LabelForm;
use brightfield_render::mark::BAR_LABEL_SIZE;
use brightfield_render::text::{draw_text, TextAnchor};
use brightfield_render::VelloRenderer;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{live_spec, Composed, PlotHandle};
use brightfield_shell::ranked_bars::{Dashboard, RankedCategoryBars, DEFAULT_LIMIT};
use brightfield_spec::analysis::BrushKind;
use brightfield_sql::ir::ScalarValue;
use image::RgbaImage;
use vello::peniko::Color;
use vello::Scene;

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

/// Whether a frame pixel carries a bar — either ink, either part.
///
/// A POSITIVE test against the two inks a bar can be, rather than a negative
/// one against the furniture. The negative form is what the sibling
/// part-of-whole gate uses and it is right there, but it cannot be used here:
/// the shell draws a committed point selection as a band across the plot that
/// produced the gesture, so "not the furniture" would report that band as a bar
/// running the full width of the frame. Naming the two inks measures the marks
/// and nothing else.
///
/// A knocked-out in-bar label is therefore NOT a bar to this predicate — it is
/// drawn in `meridian_design`'s `text.on_solid`, which that crate defines as
/// the light surface colour in both modes. That is right for a reading that
/// wants a bar's EXTENT, because the label is inset from the tip and the
/// outermost pixel is still fill; the label's own reading counts what is inside
/// a bar and not the fill, which is the same fact read the other way.
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
        let side = if i.is_multiple_of(2) { "a" } else { "b" };
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
    raster_at(composed, name, 1)
}

/// How much larger than logical the label readings are taken at.
///
/// A label is ten logical pixels tall, which is four or five pixels of actual
/// stroke — too few for a comparison to tell a `9` from an `8` by more than a
/// coin toss, and measured as such before this existed: the narrowest genuine
/// margin at 1× was eight ten-thousandths. Two makes every glyph carry four
/// times the evidence. It is the same `capture_vello_only` scale a Retina
/// export uses, so nothing here is a private path.
const LABEL_SCALE: u32 = 2;

/// Render at `scale` device pixels per logical pixel.
fn raster_at(composed: Composed, name: &str, scale: u32) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-ranked-category-bars");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(format!("{name}.png"));
    capture_vello_only(composed, scale as f32, &png).expect("export");
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
    /// The same frame in device pixels, for a capture taken at `scale`.
    fn scaled(self, scale: u32) -> Self {
        Self {
            x0: self.x0 * scale,
            y0: self.y0 * scale,
            x1: self.x1 * scale,
            y1: self.y1 * scale,
        }
    }

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
/// A band scale's first row is at the BOTTOM of the frame —
/// `ChartLayout::y_range` returns `(bottom, top)` — so a descending ranking
/// reads as bars getting LONGER down the page. Fourteen categories worth 1…14
/// rows, capped at the default ten, leaves the ten longest: 5…14 rows, drawn 5
/// at the top and 14 at the bottom.
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
///
/// This is the PRESENCE half only, and on its own it is not enough: a label
/// carrying the wrong number is present. What it says is read back and matched
/// below, in `the_number_in_each_bar_is_the_number_that_bar_stands_for` and
/// `a_percentage_is_a_percentage_of_what_the_mark_drew`.
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
// Reading a label back off the picture
// ---------------------------------------------------------------------------
//
// The presence reading above cannot tell `0 / 1` from `1 / 1`, and two
// mutations lived in that gap: `drawn_total := 2 * sum` halves every
// percentage, and `selected := value` makes every bar read `N / N`. Both left
// the whole workspace green.
//
// So a label is read back and matched against the string it should be. The
// reference is not a transcription and not a golden — it is the SAME
// `draw_text` at the SAME `BAR_LABEL_SIZE` in the SAME knockout token over the
// SAME fill, rendered here through the same `render_to_pixels` the capture path
// uses. What the comparison decides is which of the strings a bar could
// honestly have carried it actually did, and the assertion is that the right
// one wins. That form needs no tuned similarity threshold to mean anything: a
// wrong number loses to the right one by construction.
//
// WHAT IT COMPARES IS COVERAGE, NOT A THRESHOLDED SHAPE. The chart anchors a
// label against its bar's tip at a fractional pixel and centres it on a band at
// another; the reference is drawn at an integer origin. A seven-pixel glyph
// half a pixel out has every stroke smeared across two rows, so thresholding
// both at half coverage returns a clean glyph on one side and salt-and-pepper
// on the other — measured, on this fixture, before this comparison replaced it.
// Anti-aliasing conserves area, so coverage does not care about the phase.

/// A glyph run as a coverage field, cropped to its own extent, smoothed and
/// normalised to unit total.
///
/// Smoothed because that is what makes two renderings half a pixel apart
/// comparable; normalised because the comparison is of shape and not of how
/// much of the bar the string happened to cover.
#[derive(Debug, Clone)]
struct Ink {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

/// Coverage below which a pixel is the fill rather than any part of a glyph.
///
/// Low on purpose: this decides the CROP, and cropping away a glyph's faint
/// outer ramp would make the extent depend on the sub-pixel phase, which is the
/// dependence this whole comparison exists to remove.
const INK_FLOOR: f32 = 0.12;

impl Ink {
    /// Crop a coverage grid to the extent of its inked pixels, then smooth and
    /// normalise it.
    fn build(w: usize, h: usize, v: &[f32]) -> Self {
        let set: Vec<(usize, usize)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|(x, y)| v[y * w + x] >= INK_FLOOR)
            .collect();
        let Some(&(fx, fy)) = set.first() else {
            return Self {
                w: 0,
                h: 0,
                v: Vec::new(),
            };
        };
        let (mut x0, mut x1, mut y0, mut y1) = (fx, fx, fy, fy);
        for (x, y) in &set {
            x0 = x0.min(*x);
            x1 = x1.max(*x);
            y0 = y0.min(*y);
            y1 = y1.max(*y);
        }
        // Two pixels of margin all round, so a stroke on the crop edge keeps the
        // faint ramp a resample needs to land on.
        const MARGIN: usize = 2;
        let (cw, ch) = (x1 - x0 + 1 + 2 * MARGIN, y1 - y0 + 1 + 2 * MARGIN);
        let mut out = vec![0.0_f32; cw * ch];
        for y in y0..=y1 {
            for x in x0..=x1 {
                out[(y - y0 + MARGIN) * cw + (x - x0 + MARGIN)] = v[y * w + x];
            }
        }
        let total: f32 = out.iter().sum();
        if total > 0.0 {
            for p in &mut out {
                *p /= total;
            }
        }
        Self {
            w: cw,
            h: ch,
            v: out,
        }
    }

    fn at(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return 0.0;
        }
        self.v[y as usize * self.w + x as usize]
    }

    /// The field sampled between pixels, so a reference can be moved by a
    /// fraction of one.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let p = |dx: isize, dy: isize| self.at(x0 + dx, y0 + dy);
        let top = p(0, 0) * (1.0 - fx) + p(1, 0) * fx;
        let bottom = p(0, 1) * (1.0 - fx) + p(1, 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// The field's centre of mass. Sub-pixel, which is the whole point: it is
    /// what lets two renderings of one string that landed on different parts of
    /// the pixel grid be brought into register without a search fine enough to
    /// cost anything.
    fn centroid(&self) -> (f32, f32) {
        let (mut sx, mut sy, mut s) = (0.0_f32, 0.0_f32, 0.0_f32);
        for y in 0..self.h {
            for x in 0..self.w {
                let v = self.v[y * self.w + x];
                sx += v * x as f32;
                sy += v * y as f32;
                s += v;
            }
        }
        if s <= 0.0 {
            return (0.0, 0.0);
        }
        (sx / s, sy / s)
    }

    fn is_empty(&self) -> bool {
        self.v.iter().all(|p| *p <= 0.0)
    }
}

/// How far apart two coverage fields are: half the L1 distance between them,
/// with `b` moved onto `a` first.
///
/// Both fields sum to one, so the L1 distance runs 0…2 and halving it puts the
/// answer on 0…1 — nought for identical, one for disjoint.
///
/// The registration is by CENTRE OF MASS, refined by a quarter-pixel search
/// around it. That is what makes the comparison independent of where on the
/// pixel grid each rendering landed without also making it blind: an earlier
/// version smoothed both fields until the phase stopped mattering, and smoothed
/// away enough of the difference between `10%` and `15%` that the runner-up
/// came within four thousandths of the winner.
fn distance(a: &Ink, b: &Ink) -> f64 {
    let (acx, acy) = a.centroid();
    let (bcx, bcy) = b.centroid();
    let (base_x, base_y) = (bcx - acx, bcy - acy);
    const STEPS: i32 = 4;
    const STEP: f32 = 0.25;
    let mut best = f64::INFINITY;
    for sy in -STEPS..=STEPS {
        for sx in -STEPS..=STEPS {
            let (ox, oy) = (base_x + sx as f32 * STEP, base_y + sy as f32 * STEP);
            let mut sum = 0.0_f64;
            for y in 0..a.h {
                for x in 0..a.w {
                    let pa = a.v[y * a.w + x];
                    let pb = b.sample(x as f32 + ox, y as f32 + oy);
                    sum += f64::from((pa - pb).abs());
                }
            }
            // Whatever of `b` fell outside `a`'s box is missing from the sum
            // above; add it, or a reference wider than the reading would score
            // better the more of it hung off the edge.
            let covered: f32 = (0..a.h)
                .flat_map(|y| (0..a.w).map(move |x| (x, y)))
                .map(|(x, y)| b.sample(x as f32 + ox, y as f32 + oy))
                .sum();
            sum += f64::from((1.0 - covered).max(0.0));
            best = best.min(sum / 2.0);
        }
    }
    best
}

/// One Vello renderer for every reference glyph run in this file. Standing one
/// up asks the OS for a GPU adapter, which costs far more than the tiny draws
/// it is then used for.
fn reference_renderer() -> &'static Arc<Mutex<VelloRenderer>> {
    static R: OnceLock<Arc<Mutex<VelloRenderer>>> = OnceLock::new();
    R.get_or_init(VelloRenderer::new)
}

/// A design-system token as a Vello colour.
fn ink(c: meridian_design::colour::Rgba) -> Color {
    Color::new([c.r, c.g, c.b, c.a])
}

/// How much of a pixel is label rather than the fill behind it.
///
/// The pixel projected onto the segment from the fill to the knockout ink and
/// clamped — nought where the bar shows through untouched, one at the middle of
/// a stroke. Asked against the ink ACTUALLY behind the label, which is the
/// solid mark colour on a bar the selection accounted for and the deemphasised
/// one on a bar it did not: the two sit at different distances from the
/// knockout, so reading one against the other's fill rescales every coverage in
/// the run.
fn label_coverage(p: [u8; 4], fill: [i32; 3]) -> f32 {
    let knockout = rgb(meridian_design::chrome::INK_LIGHT.surface);
    let axis: Vec<f64> = (0..3).map(|c| f64::from(knockout[c] - fill[c])).collect();
    let len2: f64 = axis.iter().map(|a| a * a).sum();
    if len2 <= 0.0 {
        return 0.0;
    }
    let dot: f64 = (0..3)
        .map(|c| f64::from(i32::from(p[c]) - fill[c]) * axis[c])
        .sum();
    (dot / len2).clamp(0.0, 1.0) as f32
}

/// `text` as this renderer draws an in-bar label over `fill`.
///
/// Drawn the way the chart draws it rather than approximated: same `draw_text`,
/// same [`BAR_LABEL_SIZE`], same knockout token, same fill behind it, and the
/// same `render_to_pixels` the capture path calls. The one thing that differs
/// is where on the pixel grid it lands, and that is what the smoothing and the
/// translation search are for.
fn reference_ink(text: &str, fill: [i32; 3]) -> Ink {
    const W: u32 = 320;
    const H: u32 = 48;
    let mut logical = Scene::new();
    logical.fill(
        vello::peniko::Fill::NonZero,
        vello::kurbo::Affine::IDENTITY,
        Color::from_rgb8(fill[0] as u8, fill[1] as u8, fill[2] as u8),
        None,
        &vello::kurbo::Rect::new(0.0, 0.0, f64::from(W), f64::from(H)),
    );
    draw_text(
        &mut logical,
        text,
        16.0,
        32.0,
        BAR_LABEL_SIZE,
        ink(meridian_design::chrome::INK_LIGHT.surface),
        TextAnchor::Start,
    );
    // Scaled by appending under an affine, which is exactly what
    // `capture_vello_only` does to the chart — so the reference glyph is
    // enlarged by the same arithmetic as the one it is compared with, rather
    // than by being asked for at a larger font size.
    let mut scene = Scene::new();
    scene.append(
        &logical,
        Some(vello::kurbo::Affine::scale(f64::from(LABEL_SCALE))),
    );
    let (dw, dh) = (W * LABEL_SCALE, H * LABEL_SCALE);
    let px = reference_renderer()
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&scene, dw, dh);
    let v: Vec<f32> = px
        .chunks_exact(4)
        .map(|p| label_coverage([p[0], p[1], p[2], p[3]], fill))
        .collect();
    let mut glyphs = segment_glyphs(dw as usize, dh as usize, &v);
    assert_eq!(
        glyphs.len(),
        1,
        "the reference for {text:?} segmented into {} glyphs — one character \
         must produce one, or the two sides of the comparison are not measuring \
         the same thing",
        glyphs.len()
    );
    glyphs.remove(0)
}

/// Every character an in-bar label can contain, each over each of the two inks
/// a bar can be — since a label sits on whichever one its own bar carries.
///
/// **One glyph at a time, not one string at a time.** A whole-string comparison
/// asks a five-character run whether it is `0 / 5` or `5 / 5`, a difference of
/// one glyph in five, and one degraded reading is enough to tip it: measured on
/// this fixture, that form read `5 / 5` for a bar standing for `0 / 5` and
/// `15%` for one standing for `10%`. Segmenting first puts each glyph's whole
/// mass on its own question.
///
/// Built once for the process. Twenty-four small renders, and standing up the
/// GPU adapter behind them costs more than all of them together.
fn glyph_references() -> &'static BTreeMap<char, Vec<Ink>> {
    static REFS: OnceLock<BTreeMap<char, Vec<Ink>>> = OnceLock::new();
    REFS.get_or_init(|| {
        "0123456789/%"
            .chars()
            .map(|c| {
                let text = c.to_string();
                (
                    c,
                    vec![
                        reference_ink(&text, mark_ink()),
                        reference_ink(&text, faded_ink()),
                    ],
                )
            })
            .collect()
    })
}

/// The label knocked out of `bar`, as a raw `(width, height, coverage)` grid —
/// uncropped, so the gaps between its glyphs are still in it.
fn label_field(
    img: &RgbaImage,
    frame: &Frame,
    bar: &Bar,
    tips: &[Option<u32>],
) -> (usize, usize, Vec<f32>) {
    let tip = (bar.first_row..bar.end_row)
        .filter_map(|r| tips[r])
        .max()
        .unwrap_or(frame.x0);
    let x_lo = frame.x0 + 4;
    let x_hi = tip.saturating_sub(2);
    // Only rows the bar covers FULLY. A bar's first and last rows are partial
    // coverage of the fill over the surface, so every pixel in them is already
    // part-way to the knockout and the whole row reads as glyph — a horizontal
    // rule through the field that moves its extent and makes every comparison
    // meaningless. Asked as "is the pixel just past the baseline one of the two
    // bar inks", which is true only at full coverage.
    let solid: Vec<usize> = (bar.first_row..bar.end_row)
        .filter(|r| is_bar(img.get_pixel(x_lo, frame.y0 + *r as u32).0))
        .collect();
    // The middle of the band, where the label is. A bar's own top and bottom
    // edges are partial coverage of the fill over the surface, so pixels there
    // are already part-way to the knockout colour and read as glyph — and the
    // artefact they leave is tall enough to move the extent of whatever is
    // cropped out of the region, which is how a `3` came back as a
    // fifty-row smear. Reading the middle excludes them by geometry rather
    // than by hoping a threshold separates them.
    let margin = solid.len() / 5;
    let rows: Vec<usize> = solid
        .get(margin..solid.len().saturating_sub(margin))
        .unwrap_or(&[])
        .to_vec();
    if x_hi <= x_lo || rows.is_empty() {
        return (0, 0, Vec::new());
    }
    // Which ink stands behind this bar's label — solid where the selection
    // reached the tip, deemphasised where it did not.
    //
    // Taken as the MEDIAN over the whole region rather than sampled at one
    // point. A glyph covers a small fraction of the area, so the median is the
    // fill; a single sample near the tip lands in the bar's own edge ramp
    // whenever the tip falls between pixels, and reading a solid bar against
    // the faded ink rescales every coverage in the run into noise. That is what
    // happened to two bars in six the first time this was measured at two
    // device pixels per logical one.
    let mut channels: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for r in &rows {
        let y = frame.y0 + *r as u32;
        for x in x_lo..x_hi {
            let p = img.get_pixel(x, y).0;
            for (c, ch) in channels.iter_mut().enumerate() {
                ch.push(p[c]);
            }
        }
    }
    let median: [i32; 3] = std::array::from_fn(|c| {
        channels[c].sort_unstable();
        i32::from(channels[c][channels[c].len() / 2])
    });
    let d2 = |want: [i32; 3]| -> i32 {
        (0..3)
            .map(|c| {
                let d = median[c] - want[c];
                d * d
            })
            .sum()
    };
    let fill = if d2(mark_ink()) <= d2(faded_ink()) {
        mark_ink()
    } else {
        faded_ink()
    };

    let w = (x_hi - x_lo) as usize;
    let h = rows.len();
    let mut v = vec![0.0_f32; w * h];
    for (ry, r) in rows.iter().enumerate() {
        let y = frame.y0 + *r as u32;
        for (rx, x) in (x_lo..x_hi).enumerate() {
            v[ry * w + rx] = label_coverage(img.get_pixel(x, y).0, fill);
        }
    }
    // A column inked down more than half the band is not a glyph — it is the
    // bar's own edge, caught by the region when the tip falls between pixels.
    // A label is a fraction of a band tall by construction, so this is a rule
    // about what a glyph IS rather than a margin chosen against a size.
    for x in 0..w {
        let inked = (0..h).filter(|y| v[y * w + x] >= INK_FLOOR).count();
        if inked * 2 > h {
            for y in 0..h {
                v[y * w + x] = 0.0;
            }
        }
    }
    (w, h, v)
}

/// Fraction of a label's tallest column below which a column is the gap
/// between two glyphs rather than part of one.
///
/// Judged on the COLUMN's total coverage and not on whether any single pixel in
/// it is inked. The gap between two adjacent digits carries the outer ramp of
/// both, so at ten pixels it is never empty: measured on this fixture, the gaps
/// run to 0.4 of a pixel's worth of coverage while the thinnest column of a
/// real glyph is 1.0. A per-pixel test merged `14%` into one blob and read it
/// as `%%`.
///
/// Relative to the run's own peak rather than absolute, so it does not need
/// re-tuning when the label size or the palette contrast moves.
const GLYPH_GAP_FRACTION: f32 = 0.15;

/// Split a label's coverage grid into one field per glyph, left to right.
///
/// Both sides of the comparison go through this, so a faint edge column trimmed
/// from a chart glyph is trimmed from its reference too.
fn segment_glyphs(w: usize, h: usize, v: &[f32]) -> Vec<Ink> {
    let column: Vec<f32> = (0..w).map(|x| (0..h).map(|y| v[y * w + x]).sum()).collect();
    let peak = column.iter().copied().fold(0.0_f32, f32::max);
    if peak <= 0.0 {
        return Vec::new();
    }
    let floor = peak * GLYPH_GAP_FRACTION;
    let mut out = Vec::new();
    let mut start = None;
    for x in 0..=w {
        let on = x < w && column[x] > floor;
        match (on, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                let gw = x - s;
                let mut cell = vec![0.0_f32; gw * h];
                for y in 0..h {
                    cell[y * gw..(y + 1) * gw].copy_from_slice(&v[y * w + s..y * w + x]);
                }
                let glyph = Ink::build(gw, h, &cell);
                if !glyph.is_empty() {
                    out.push(glyph);
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// The characters the label in `bar` is made of, and the two margins that say
/// how much the reading was worth: the worst distance any glyph matched at, and
/// the smallest gap between a glyph's winner and its runner-up.
fn read_label(
    img: &RgbaImage,
    frame: &Frame,
    bar: &Bar,
    tips: &[Option<u32>],
) -> (String, f64, f64) {
    let (w, h, v) = label_field(img, frame, bar, tips);
    let glyphs = segment_glyphs(w, h, &v);
    assert!(
        !glyphs.is_empty(),
        "no label was knocked out of the bar at rows {}..{} — either the module \
         drew none or it was placed outside the bar, and either way there is \
         nothing here to read",
        bar.first_row,
        bar.end_row
    );
    let refs = glyph_references();
    let (mut text, mut worst, mut narrowest) = (String::new(), 0.0_f64, f64::INFINITY);
    for glyph in &glyphs {
        let mut scored: Vec<(f64, char)> = refs
            .iter()
            .map(|(c, inks)| {
                let d = inks
                    .iter()
                    .map(|m| distance(glyph, m))
                    .fold(f64::INFINITY, f64::min);
                (d, *c)
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        text.push(scored[0].1);
        worst = worst.max(scored[0].0);
        narrowest = narrowest.min(scored[1].0 - scored[0].0);
    }
    (text, worst, narrowest)
}

/// The dashboard the label readings are taken over.
///
/// The same two modules, with the measured one widened so every label fits
/// INSIDE its bar. At the default width the shortest bar is about forty pixels
/// and `0 / 1` is about that wide, so the placement rule would put some labels
/// outside the tip — correct behaviour, and the wrong picture to read a
/// knockout off. Placement is the sibling reading's subject
/// (`a_label_that_does_not_fit_is_drawn_past_the_tip_instead_of_inside_it`, a
/// unit test); this one is about the number.
fn wide_dashboard(categories: usize, label: Option<LabelForm>) -> String {
    let mut dash = Dashboard::of_columns(fixture_rows(categories), "t", &["cat", "side"]);
    dash.modules[0] = RankedCategoryBars::new("cat")
        .with_label(label)
        .with_size(720, 320);
    dash.to_spec()
}

/// Read every bar's label off a picture, top to bottom.
fn labels_down_the_frame(img: &RgbaImage, frame: &Frame) -> Vec<(String, f64, f64)> {
    let tips = row_tips(img, frame);
    bars(img, frame)
        .iter()
        .map(|bar| read_label(img, frame, bar, &tips))
        .collect()
}

/// How far a glyph may be from the character it is read as.
///
/// Not a similarity threshold in the usual sense — what carries the test is
/// that the right character beats every other, which needs no constant. This is
/// the floor underneath it: it stops a reading that matched NOTHING well from
/// passing merely by being least bad, which is how a harness whose extraction
/// had broken would otherwise stay green.
///
/// The worst glyph over the eighteen labels these tests read matches at 0.244,
/// and a reading whose extraction had failed came back at 0.58 and 0.74 while
/// this was being built — so the two populations are far apart and this sits
/// between them.
const GLYPH_MATCH_FLOOR: f64 = 0.35;

/// How much the winning character must beat the runner-up by.
///
/// A reading that came first by a thousandth is a coin toss dressed as a
/// measurement, and at one device pixel per logical one the narrowest margin
/// over these labels was eight ten-thousandths — which is why they are read at
/// [`LABEL_SCALE`]. There the narrowest is 0.082, so this leaves a factor of
/// nearly three.
const GLYPH_MARGIN: f64 = 0.03;

/// Compare labels as CHARACTER SEQUENCES, spaces dropped.
///
/// The reading segments on blank columns, so it recovers `0/5` from a label
/// drawn `0 / 5` — the spacing is the renderer's and is not what these tests
/// are about. Dropping it on both sides keeps the expectation written the way
/// the label actually reads.
fn assert_labels(found: &[(String, f64, f64)], expected: &[&str], case: &str) {
    let read: Vec<&str> = found.iter().map(|(t, _, _)| t.as_str()).collect();
    let want: Vec<String> = expected
        .iter()
        .map(|e| e.chars().filter(|c| !c.is_whitespace()).collect())
        .collect();
    assert_eq!(
        read, want,
        "{case}: the numbers printed in the bars, top to bottom, are not the \
         numbers those bars stand for"
    );
    for ((text, worst, narrowest), expect) in found.iter().zip(expected) {
        assert!(
            *worst < GLYPH_MATCH_FLOOR,
            "{case}: {text:?} was read for the label drawn on the bar standing \
             for {expect:?}, but its worst glyph matched only at distance \
             {worst:.3} — nothing matched, so the extraction is broken rather \
             than the label being right"
        );
        assert!(
            *narrowest > GLYPH_MARGIN,
            "{case}: reading {text:?} for {expect:?}, some glyph beat its \
             runner-up by only {narrowest:.3} — the reading cannot tell two \
             characters apart and its answer is a coin toss"
        );
    }
}

/// **AC2's other half: the label says the right thing.**
///
/// Six categories worth one to six rows. `side: b` selects every row of the
/// odd-numbered ones and none of the even, so the six bars stand for
/// `0/1, 2/2, 0/3, 4/4, 0/5, 6/6` — three fully selected, three not at all, and
/// no two alike.
///
/// That spread is the point. A label wired to print the whole twice reads
/// `1/1, 2/2, 3/3, …` and differs on three of the six; a label wired to print
/// the part twice differs on the other three. A fixture where the selection
/// took a uniform fraction could be passed by either.
#[test]
fn the_number_in_each_bar_is_the_number_that_bar_stands_for() {
    let source = wide_dashboard(6, Some(LabelForm::Count));
    let path = spec_path(&source, "label-count");
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the spec loads live");
    let gesture = click_on_band(&resting.plots[1], "b");
    let frame = Frame::of(&resting, 0).scaled(LABEL_SCALE);

    // At rest there is no selection, so each bar prints its own total and
    // nothing else. A label that printed a part here would be inventing one.
    let before = raster_at(resting, "label-count-resting", LABEL_SCALE);
    assert_labels(
        &labels_down_the_frame(&before, &frame),
        &["1", "2", "3", "4", "5", "6"],
        "at rest",
    );

    let selected = live.apply(gesture).expect("the selection re-composites");
    let after = raster_at(selected, "label-count-selected", LABEL_SCALE);
    assert_labels(
        &labels_down_the_frame(&after, &frame),
        &["0 / 1", "2 / 2", "0 / 3", "4 / 4", "0 / 5", "6 / 6"],
        "under a selection",
    );
}

/// **A percentage is a percentage of what the mark drew**, and the denominator
/// is in the picture rather than only in the formatter.
///
/// One to six rows over six categories is twenty-one drawn rows, so the bars
/// read 5, 10, 14, 19, 24 and 29 per cent. Every one of those is a different
/// string from what a doubled denominator would give (2, 5, 7, 10, 12, 14), and
/// the sequences do not overlap position for position — a denominator wrong by
/// any constant factor lands on a different set of strings.
#[test]
fn a_percentage_is_a_percentage_of_what_the_mark_drew() {
    let source = wide_dashboard(6, Some(LabelForm::Percent));
    let path = spec_path(&source, "label-percent");
    let (_, composed) = live_spec(path.to_str().expect("utf-8 path")).expect("the spec loads");
    let frame = Frame::of(&composed, 0).scaled(LABEL_SCALE);
    let img = raster_at(composed, "label-percent", LABEL_SCALE);
    assert_labels(
        &labels_down_the_frame(&img, &frame),
        &["5%", "10%", "14%", "19%", "24%", "29%"],
        "at rest",
    );
}

// ---------------------------------------------------------------------------
// AC3 / AC4 — the total behind the subset, and the reach across the spec
// ---------------------------------------------------------------------------

/// The interaction a click on `plot`'s band axis produces, picking `value`.
///
/// **Built from the plot's own [`GestureBinding`], never from literals.** The
/// binding is what the composition resolved out of the module's declared
/// `select: toggleY / as: $sel`: the selection name, the contributor identity
/// the coordinator's self-exclusion compares, and the column the band axis
/// names. A test that spelled those three itself would keep passing with the
/// producer deleted from the spec — the coordinator takes a contribution to a
/// selection from whoever hands it one — and the module would then be a chart
/// no gesture could drive.
///
/// The gesture kind is asserted here rather than assumed: a `barX` puts its
/// categories on y, so the click that picks one is a `PointY`.
fn click_on_band(plot: &PlotHandle, value: &str) -> Interaction {
    let gesture = plot
        .gesture
        .as_ref()
        .unwrap_or_else(|| panic!("the module at {} declares no gesture", plot.path));
    assert_eq!(
        gesture.kind,
        BrushKind::PointY,
        "a module's categories are on its y axis, so the gesture that picks one \
         is a point selection on y"
    );
    let column = gesture
        .y_column
        .clone()
        .expect("the gesture names the band column it picks from");
    Interaction::Select {
        name: gesture.selection.clone(),
        contributor: gesture.contributor.clone(),
        predicate: SqlPredicate::Point {
            column,
            values: vec![ScalarValue::Text(value.to_string())],
            meta: None,
        },
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
    let gesture = click_on_band(&resting.plots[1], "b");
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

    let selected = live.apply(gesture).expect("the selection re-composites");
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
    let gesture = click_on_band(&resting.plots[0], "cat-00");
    let detail = Frame::of(&resting, 1);

    let before = raster(resting, "crossfilter-resting");
    let before_ink = inked(&before, &detail);
    assert!(
        before_ink > 0,
        "fixture check: the consumer chart draws something at rest"
    );

    let filtered = live.apply(gesture).expect("the selection re-composites");
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
