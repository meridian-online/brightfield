//! Gate: `x: {sum: col}` over a categorical `y` puts BARS on the page.
//!
//! The idiom parsed clean, emitted `SELECT * FROM …` and rasterised to a frame
//! with no bars in it. Every structural check passed on the way: the mark was
//! registered in both the lowerer and the renderer registry, a query emitted, a
//! scene encoded, the export exited 0. Only the picture was missing — which is
//! why this test counts PIXELS, for the reason `binned_histogram.rs` and
//! `bar_orientation.rs` do.
//!
//! It does not stop at ink-versus-no-ink. A lowerer that grouped by the wrong
//! column, or one that passed the raw rows through, would clear an ink floor
//! easily: six rows of `v: 1` draw six bars, and so does a picture that
//! collapsed them into six identical ones. What is asserted instead is the
//! SHAPE — the bar LENGTHS, normalised by the shortest bar, must be the
//! per-category totals — and the fixture is built so the grouped and ungrouped
//! answers differ in bar COUNT and in every bar's length at once.
//!
//! A **pre-aggregated** bar carrying the same three totals written out by hand
//! is the control, and it is asserted FIRST. It drew before the lift and must
//! draw after it, so if it ever reads zero the cause is the harness (wrong
//! colour token, wrong render path, a scale change) and this file says so
//! instead of quietly passing. It is also the tighter claim the pair makes
//! together: the computed chart and the hand-written one are the SAME picture.
//!
//! Needs a wgpu adapter, as the shell's snapshot and canvas suites already do
//! on the same runner. Not `#[ignore]`d and behind no feature, for the reason
//! `crates/brightfield-shell/tests/snapshot.rs` gives.

use brightfield_render::VelloRenderer;
use brightfield_shell::pipeline::compose_spec_str;
use image::RgbaImage;

/// Six rows over three categories whose totals are 1, 2 and 3 units.
///
/// Deliberately interleaved rather than blocked by category, and deliberately
/// all-equal in `v`: every row is worth exactly one unit, so a picture built
/// from the RAW rows is six bars of one unit each. The grouped picture is three
/// bars of one, two and three. The two disagree on the number of bars and on
/// the length of all but one of them, so no ink floor and no bar count can be
/// cleared by the wrong answer.
const ROWS: &str = "    - { region: c, v: 1 }
    - { region: b, v: 1 }
    - { region: c, v: 1 }
    - { region: a, v: 1 }
    - { region: b, v: 1 }
    - { region: c, v: 1 }";

/// The chart under test: one bar per region, its length that region's total.
fn aggregating_spec() -> String {
    format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barX\n  data: {{ from: t }}\n  \
         x: {{ sum: v }}\n  y: region\nwidth: 640\nheight: 480\n"
    )
}

/// The control: the same three bars, with the answer written out by hand.
///
/// A pre-aggregated bar — a plain column on the value axis — takes the
/// pass-through path, which is the half of `BarLowerer` that must keep
/// behaving exactly as `SimpleLowerer` did.
const PREAGGREGATED_SPEC: &str = "data:
  t:
    - { region: a, v: 1 }
    - { region: b, v: 2 }
    - { region: c, v: 3 }
plot:
- mark: barX
  data: { from: t }
  x: v
  y: region
width: 640
height: 480
";

/// The ink the bars are measured in: neither spec binds a colour channel, so
/// both take the default mark colour. Read from the token layer rather than
/// transcribed, so a palette bump moves the expectation with the picture.
fn default_mark_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance: the fill's core and the first ring of its
/// anti-aliasing. Matches `binned_histogram.rs`, which measures the same ink.
const MARK_INK_TOL: i32 = 20;

/// Compose `source` and rasterise it through the same Vello renderer the window
/// and the PNG export use.
fn raster(source: &str) -> RgbaImage {
    let composed = compose_spec_str(source, None).expect("the spec composes");
    let (w, h) = (composed.width, composed.height);
    let renderer = VelloRenderer::new();
    let pixels = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&composed.scene, w, h);
    RgbaImage::from_raw(w, h, pixels).expect("vello pixel buffer size mismatch")
}

/// Inked pixels per image ROW, top to bottom.
///
/// A `barX` runs its bars along `x` from the zero baseline, so a row's inked
/// count is that bar's LENGTH in pixels — which is what makes the aggregate
/// measurable from the picture rather than from the scene.
fn row_lengths(img: &RgbaImage, want: [i32; 3]) -> Vec<u32> {
    let (w, h) = img.dimensions();
    (0..h)
        .map(|y| {
            (0..w)
                .filter(|&x| {
                    let p = img.get_pixel(x, y).0;
                    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= MARK_INK_TOL)
                })
                .count() as u32
        })
        .collect()
}

/// Total mark ink in the picture, in the given colour.
fn ink(img: &RgbaImage, want: [i32; 3]) -> u64 {
    row_lengths(img, want).iter().map(|&n| u64::from(n)).sum()
}

/// The rows that sit solidly inside a bar rather than on its edge.
///
/// A bar's top and bottom rows are anti-aliased and read short, so a row counts
/// only when `RUN` of its neighbours agree with it — a bar in this fixture is
/// tens of rows tall and an edge is one or two.
fn solid_rows(img: &RgbaImage, want: [i32; 3]) -> Vec<u32> {
    const RUN: usize = 5;
    const TOL: u32 = 2;
    row_lengths(img, want)
        .windows(RUN)
        .filter(|w| w[0] > 0 && w.iter().all(|n| n.abs_diff(w[0]) <= TOL))
        .map(|w| w[0])
        .collect()
}

/// The distinct bar lengths present, as MULTIPLES of the shortest bar, in
/// ascending order.
fn bar_lengths_in_units(img: &RgbaImage, want: [i32; 3]) -> Vec<u32> {
    let solid = solid_rows(img, want);
    let unit = f64::from(*solid.iter().min().expect("at least one bar"));
    let mut units: Vec<u32> = solid
        .iter()
        .map(|&n| (f64::from(n) / unit).round() as u32)
        .collect();
    units.sort_unstable();
    units.dedup();
    units
}

/// How many bars are in the picture — maximal runs of inked rows, separated by
/// the band scale's own padding.
///
/// Counted separately from the lengths because the two failures are different:
/// a picture with the right lengths and six bars is the ungrouped rows drawn
/// twice over, and a picture with three bars of equal length is a grouping that
/// lost the aggregate.
fn bar_count(img: &RgbaImage, want: [i32; 3]) -> usize {
    const MIN_ROWS: usize = 3;
    let mut bars = 0;
    let mut run = 0usize;
    for n in row_lengths(img, want) {
        if n > 0 {
            run += 1;
        } else {
            if run >= MIN_ROWS {
                bars += 1;
            }
            run = 0;
        }
    }
    if run >= MIN_ROWS {
        bars += 1;
    }
    bars
}

/// **AC4.** The whole gate: the hand-written control inks, the computed chart
/// inks, and the computed chart's bars ARE the per-category totals.
#[test]
fn a_banded_aggregate_bar_draws_the_totals_its_aggregate_describes() {
    let want = default_mark_ink();

    // The control, first. Three bars whose lengths are written into the data;
    // it drew before the lift and has to draw after it.
    let control = raster(PREAGGREGATED_SPEC);
    let control_ink = ink(&control, want);
    assert!(
        control_ink > 1_000,
        "the PRE-AGGREGATED control drew {control_ink} px of mark ink. That \
         path is untouched by the lift, so this is the HARNESS failing — a \
         palette bump, a render path change or a scale change — not the \
         aggregate."
    );
    assert_eq!(bar_count(&control, want), 3, "fixture check: three bars");
    assert_eq!(
        bar_lengths_in_units(&control, want),
        vec![1, 2, 3],
        "fixture check: the control's bars are 1, 2 and 3 units"
    );

    // The chart under test. Same three bars, computed.
    let computed = raster(&aggregating_spec());
    let computed_ink = ink(&computed, want);
    assert!(
        computed_ink > 1_000,
        "`x: {{sum: v}}` over `y: region` drew {computed_ink} px of mark ink. \
         The spec parsed, a query emitted and a scene encoded; there are no \
         bars in the frame."
    );
    assert_eq!(
        bar_count(&computed, want),
        3,
        "six rows over three regions must collapse to three bars — {} is the \
         raw row count, not the category count",
        bar_count(&computed, want)
    );
    assert_eq!(
        bar_lengths_in_units(&computed, want),
        vec![1, 2, 3],
        "the bars are not the per-category totals: every row is worth one \
         unit, so a, b and c must come out 1, 2 and 3 units long"
    );

    // And the computed picture is the hand-written one. Both specs describe the
    // same three bars on the same scales, so the ink they put down agrees to
    // within anti-aliasing rather than merely sharing a shape.
    let drift = control_ink.abs_diff(computed_ink);
    assert!(
        drift * 100 < control_ink,
        "the computed chart and the chart that states the same answer put down \
         different amounts of ink: control {control_ink}, computed \
         {computed_ink}"
    );
}
