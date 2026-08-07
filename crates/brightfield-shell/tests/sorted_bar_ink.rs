//! Gate: `sort:` and `limit:` change the PICTURE.
//!
//! Every layer beneath this one can be right while the chart is wrong. The
//! emitted SQL carries an `ORDER BY` and a `LIMIT`
//! (`brightfield-sql`'s `sorted_bar_sql`), and executing it returns the bands
//! in the ranked order (`brightfield-engine`'s `sort_limit_oracle`) — but the
//! ordering only reaches a reader if the renderer draws in row order, and
//! nothing in either of those files looks at a pixel. That is the step this
//! card's defect lived on: `sort` was parsed, held, re-serialised and then
//! dropped, and the frame was the only place it showed.
//!
//! So this file measures bars. It reads the sequence of bar LENGTHS down the
//! frame and asserts the ranking is in it — not "is there ink", which every
//! wrong answer here also produces.
//!
//! It lives in `brightfield-shell` rather than beside
//! `brightfield-render/tests/frame_ink.rs`, and the reason is the dependency
//! graph: `brightfield-render` depends on `brightfield-spec` and not on
//! `brightfield-sql`, so a raster test there cannot see the lowerer that
//! builds the `ORDER BY` at all and would stay green with it reverted.
//! `brightfield-shell` is the only crate with parse, lower, execute and render
//! in one graph. `banded_bar_ink.rs` is here for the same reason.
//!
//! Needs a wgpu adapter, as the shell's snapshot and canvas suites already do
//! on the same runner. Not `#[ignore]`d and behind no feature, for the reason
//! `crates/brightfield-shell/tests/snapshot.rs` gives.

use brightfield_render::VelloRenderer;
use brightfield_shell::pipeline::compose_spec_str;
use image::RgbaImage;

/// Ten rows over four categories, each row worth one unit.
///
/// | cat | rows | units |
/// |-----|------|-------|
/// | a   | 1    |     1 |
/// | b   | 4    |     4 |
/// | c   | 2    |     2 |
/// | q   | 3    |     3 |
///
/// The query's row order is `a b c q` — lengths `1 4 2 3`, which rises and
/// falls. Ranked descending it is `b q c a` — lengths `4 3 2 1`, strictly
/// monotone. **A picture cannot be both**, and monotonicity is the property
/// asserted, so the test does not depend on which pixel a bar starts at.
///
/// **A band scale's first row is at the BOTTOM of the frame**, so the
/// sequences below read reversed against the query. `Layout::y_range` returns
/// `(bottom, top)` — right for a continuous `y`, where a larger value belongs
/// higher up, and inherited by the band scale built over the same range. The
/// control proves it is not this card's doing: it carries no `sort:` at all,
/// its query orders by the band ascending, and its bars still come out
/// `q c b a` down the page.
///
/// Interleaved rather than blocked by category, so a lowerer that merely
/// collapsed adjacent rows is not sitting on sorted input.
const ROWS: &str = "    - { cat: c, v: 1 }
    - { cat: b, v: 1 }
    - { cat: q, v: 1 }
    - { cat: a, v: 1 }
    - { cat: b, v: 1 }
    - { cat: q, v: 1 }
    - { cat: b, v: 1 }
    - { cat: c, v: 1 }
    - { cat: q, v: 1 }
    - { cat: b, v: 1 }";

/// The chart under test, with `sort:` written as given (or omitted).
fn bar_spec(sort: Option<&str>) -> String {
    let line = sort.map_or(String::new(), |s| format!("  sort: {s}\n"));
    format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barX\n  data: {{ from: t }}\n  \
         x: {{ sum: v }}\n  y: cat\n{line}width: 640\nheight: 480\n"
    )
}

/// The ink the bars are measured in: the spec binds no colour channel, so they
/// take the default mark colour. Read from the token layer rather than
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
/// anti-aliasing. Matches `banded_bar_ink.rs`, which measures the same ink.
const MARK_INK_TOL: i32 = 20;

/// Compose `source` and rasterise it through the same Vello renderer the
/// window and the PNG export use.
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

/// Inked pixels per image ROW, top to bottom. A `barX` runs its bars along `x`
/// from the zero baseline, so a row's inked count is that bar's LENGTH.
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

/// The length of each bar, **top to bottom**.
///
/// One entry per maximal run of inked rows, taking the run's longest row: a
/// bar's top and bottom rows are anti-aliased and read short, while its
/// interior is flat. Unlike `banded_bar_ink.rs`'s helper this neither sorts
/// nor de-duplicates — the SEQUENCE is the whole measurement here.
fn bar_lengths_top_to_bottom(img: &RgbaImage, want: [i32; 3]) -> Vec<u32> {
    const MIN_ROWS: usize = 3;
    let mut out = Vec::new();
    let mut run: Vec<u32> = Vec::new();
    for n in row_lengths(img, want) {
        if n > 0 {
            run.push(n);
        } else {
            if run.len() >= MIN_ROWS {
                out.push(run.iter().copied().max().unwrap_or(0));
            }
            run.clear();
        }
    }
    if run.len() >= MIN_ROWS {
        out.push(run.iter().copied().max().unwrap_or(0));
    }
    out
}

/// The same lengths as FRACTIONS of the longest bar in the same picture, so
/// the assertion is about the data and not about the plot's pixel width.
///
/// Against the longest rather than the shortest: a `limit:` removes bars, and
/// the shortest SURVIVOR is a different number of units in each picture, which
/// would make the same chart read differently depending on how many bars it
/// kept. The longest bar spans the value axis in every case here, because a
/// bar's domain starts at zero.
fn bar_ratios(img: &RgbaImage, want: [i32; 3]) -> Vec<f64> {
    let lengths = bar_lengths_top_to_bottom(img, want);
    let longest = f64::from(*lengths.iter().max().expect("at least one bar"));
    lengths.iter().map(|&n| f64::from(n) / longest).collect()
}

/// Per-bar tolerance on those fractions: the bars here are hundreds of pixels
/// long, so a quarter of the axis is ~150 px and this is a handful of them.
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

/// **AC1, in pixels.** The control draws the bands in band order; the ranked
/// chart draws them longest-first.
///
/// The control is asserted FIRST and by its exact sequence. It is the same
/// chart with one line removed, so if it ever reads wrong the cause is the
/// harness — a palette bump, a render path change, a scale change — and this
/// file says so instead of quietly passing.
#[test]
fn a_sorted_bar_chart_draws_its_bars_in_value_order() {
    let want = default_mark_ink();

    assert_ratios(
        &bar_ratios(&raster(&bar_spec(None)), want),
        &[0.75, 0.5, 1.0, 0.25],
        "the control — bands a, b, c, q worth 1, 4, 2 and 3 rows, drawn \
         bottom-up, so the page reads q c b a and the sequence is not \
         monotone. A wrong reading here is the harness, not the sort",
    );

    assert_ratios(
        &bar_ratios(&raster(&bar_spec(Some("{ y: -x }"))), want),
        &[0.25, 0.5, 0.75, 1.0],
        "`sort: { y: -x }` — every bar longer than the one above it, which is \
         the ranking; the control shares no position with this",
    );

    assert_ratios(
        &bar_ratios(&raster(&bar_spec(Some("{ y: x }"))), want),
        &[1.0, 0.75, 0.5, 0.25],
        "and without the `-` the same four bars come out the other way up",
    );
}

/// **AC2, in pixels.** `limit: n` leaves n bars in the frame, and they are the
/// n longest.
///
/// Both halves are needed. A limit that kept the FIRST two bands would also
/// draw two bars, so the test asserts which two: the descending pair is 4 and
/// 3 units (a ratio of 0.75), and the ascending pair is 1 and 2 (a ratio of
/// 0.5) — the two the descending case dropped.
#[test]
fn a_limit_leaves_only_the_top_bars_on_the_page() {
    let want = default_mark_ink();

    assert_ratios(
        &bar_ratios(&raster(&bar_spec(Some("{ y: -x, limit: 2 }"))), want),
        &[0.75, 1.0],
        "`limit: 2` descending — the 4-unit and 3-unit bars and nothing else",
    );

    assert_ratios(
        &bar_ratios(&raster(&bar_spec(Some("{ y: x, limit: 2 }"))), want),
        &[1.0, 0.5],
        "ascending keeps the 1-unit and 2-unit pair instead, so the ordering \
         is what chooses which two survive",
    );

    let one = bar_ratios(&raster(&bar_spec(Some("{ y: -x, limit: 1 }"))), want);
    assert_eq!(one.len(), 1, "`limit: 1` draws one bar: {one:?}");
}
