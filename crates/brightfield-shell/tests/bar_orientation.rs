//! Gate: both bar orientations put ink on the page.
//!
//! `barX` was marked `Implemented` in the vocabulary and drew **no bars at
//! all** — axes, gridlines and category labels rendered, the process exited 0,
//! and a valid PNG was written. Every structural check passed the whole time:
//! the mark parsed, the kind was registered in the lowerer and in the renderer
//! registry, the SQL emitted, the scene encoded. Only the picture was missing.
//!
//! So this test counts PIXELS OF MARK INK. A test asserting "exit 0 and a PNG
//! exists" passes on the broken behaviour, and one asking the scene how many
//! ops it encoded would be satisfied by geometry that never reached a pixel or
//! landed outside the plot clip — see [`mark_ink_px`]. The measure has to be
//! the picture.
//!
//! Both orientations are asserted together and against each other. `barY`
//! always worked, so it is the control: if the two ever both go to zero the
//! cause is the harness, not the renderer, and the test says so instead of
//! quietly passing.

use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::compose_spec;
use std::path::PathBuf;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The mark colour a bar takes when nothing binds a colour channel — Harbour
/// slot 1, read from the token layer so a palette bump moves the expectation
/// with the picture rather than breaking this test.
fn mark_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance: the fill's core and the first ring of its
/// anti-aliasing. Matches `navigation_extent.rs`, which measures the same ink.
const MARK_INK_TOL: i32 = 20;

/// How many pixels of mark ink an exported chart holds.
///
/// Counted as pixels rather than the horizontal RUNS that `navigation_extent`
/// uses, because bars are area: a run measure over six solid bars spanning the
/// plot would report 1 whether they were bars or a single smear.
///
/// Nothing else in the picture is admitted at this tolerance — gridlines are
/// grey, axis type is near-black, the surface is near-white.
fn mark_ink_px(png: &std::path::Path) -> u64 {
    let img = image::open(png).expect("open png").to_rgba8();
    let want = mark_ink();
    img.pixels()
        .filter(|p| (0..3).all(|c| (i32::from(p.0[c]) - want[c]).abs() <= MARK_INK_TOL))
        .count() as u64
}

fn export(spec: &str, out: &str) -> u64 {
    let dir = std::env::temp_dir().join("bf-bar-orientation");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(out);
    let composed =
        compose_spec(example(spec).to_str().expect("utf-8 path")).expect("the example composes");
    capture_vello_only(composed, 1.0, &png).expect("export");
    mark_ink_px(&png)
}

/// Neither orientation may draw an empty frame.
///
/// The numbers are floors, not baselines — six bars over a 640x480 plot cover
/// tens of thousands of pixels, so 1_000 is far below anything a real bar chart
/// produces and far above the zero a broken one does. A loose floor here is
/// deliberate: this gate is about ink versus no ink, and pinning an exact count
/// would make it a raster baseline that every layout tweak has to re-bless.
#[test]
fn both_bar_orientations_put_ink_on_the_page() {
    let bary = export("bars.yaml", "bary.png");
    let barx = export("bars-x.yaml", "barx.png");

    // The control. barY has always drawn; if it is zero the harness is broken
    // — wrong colour token, wrong export path — and the barX assertion below
    // would be measuring that instead of the renderer.
    assert!(
        bary > 1_000,
        "barY drew {bary} px of mark ink. barY has always worked, so this is \
         the harness failing, not the renderer — check mark_ink() against the \
         current palette before believing anything else in this file."
    );

    assert!(
        barx > 1_000,
        "barX drew {barx} px of mark ink (barY drew {bary} over the same data). \
         barX renders its axes, gridlines and category labels and exits 0 while \
         drawing no bars whenever BarRenderer loses its orientation: it reads \
         the band width off the x scale, gets None from a Linear scale, and \
         returns before a single fill."
    );

    // Same six values, transposed, so the two pictures should carry comparable
    // ink. An order-of-magnitude gap means one orientation is drawing something
    // degenerate — zero-width bars, or bars collapsed onto the baseline —
    // which the ink-versus-no-ink assertions above would both pass.
    let (lo, hi) = if barx < bary {
        (barx, bary)
    } else {
        (bary, barx)
    };
    assert!(
        hi < lo * 4,
        "barX drew {barx} px and barY drew {bary} px over identical data. One \
         of them is drawing something degenerate."
    );
}

/// The distinct bar LENGTHS in a `barX` export, longest first.
///
/// Every barX bar starts at the zero baseline, so a bar's length is the
/// rightmost inked pixel on its rows minus the leftmost. Rows inside one band
/// all share a length, so collecting per-row extents and deduplicating gives
/// one entry per bar.
///
/// Tolerance of 2 px when deduplicating: anti-aliasing moves an edge by a
/// fraction of a pixel between rows, and without it a single bar reports as
/// two or three near-identical lengths.
fn bar_lengths(png: &std::path::Path) -> Vec<u32> {
    let img = image::open(png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();
    let want = mark_ink();
    let inked = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= MARK_INK_TOL)
    };
    let mut lengths: Vec<u32> = Vec::new();
    for y in 0..h {
        let first = (0..w).find(|&x| inked(x, y));
        let last = (0..w).rev().find(|&x| inked(x, y));
        if let (Some(a), Some(b)) = (first, last) {
            let len = b - a;
            if !lengths.iter().any(|l| l.abs_diff(len) <= 2) {
                lengths.push(len);
            }
        }
    }
    lengths.sort_unstable_by(|a, b| b.cmp(a));
    lengths
}

/// A `barX`'s bar lengths are PROPORTIONAL to its values.
///
/// The ink gate above only asks whether anything was drawn. A renderer that
/// mapped every bar through the wrong scale, or dropped the value channel and
/// drew six identical bars, would sail through it — and so would a scene-op
/// count. This is the assertion that says the picture means what the data says.
///
/// `examples/bars-x.yaml` holds 30/18/45/22/12/38. Sorted descending that is
/// 45/38/30/22/18/12, so every length divided by its value must give the same
/// pixels-per-unit.
#[test]
fn barx_bar_lengths_are_proportional_to_their_values() {
    let dir = std::env::temp_dir().join("bf-bar-orientation");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join("barx-lengths.png");
    let composed = compose_spec(example("bars-x.yaml").to_str().expect("utf-8 path"))
        .expect("the example composes");
    capture_vello_only(composed, 1.0, &png).expect("export");

    let lengths = bar_lengths(&png);
    let mut values = [30.0_f64, 18.0, 45.0, 22.0, 12.0, 38.0];
    values.sort_by(|a, b| b.partial_cmp(a).expect("no NaN"));

    assert_eq!(
        lengths.len(),
        values.len(),
        "expected one distinct length per bar, got {lengths:?} for {values:?}"
    );

    let ppu: Vec<f64> = lengths
        .iter()
        .zip(values.iter())
        .map(|(l, v)| f64::from(*l) / v)
        .collect();
    let lo = ppu.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ppu.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // 3% covers anti-aliased edges and the half-open pixel grid; a genuine
    // mapping error is off by tens of percent or more.
    assert!(
        hi / lo < 1.03,
        "bar lengths are not proportional to their values: pixels-per-unit \
         ranged {lo:.2}..{hi:.2} across {lengths:?} for {values:?}"
    );
}

/// The value axis of a `barX` starts at zero, and the bars sit flush against
/// it.
///
/// Two more symptoms of the same missing discriminator, both invisible to an
/// ink count. `zero_baseline_channel()` answered `Y` for both orientations, so
/// for `barX` it named the *band* scale: `extend_domain_to_zero` is a no-op on
/// a Band, leaving the x domain starting at the data minimum (12 for this
/// fixture, not 0), and `zero_pinned_end` got `None` from `domain_min()` on a
/// Band and inset both ends of the value axis, floating the bars off their own
/// baseline.
///
/// Asserted through the composed scales rather than the raster because that is
/// where the fact lives; the picture only shows it indirectly.
#[test]
fn barx_value_axis_starts_at_zero() {
    use brightfield_render::channel::Channel;

    let composed = compose_spec(example("bars-x.yaml").to_str().expect("utf-8 path"))
        .expect("the example composes");
    let scales = composed
        .plots
        .first()
        .map(|p| &p.scales)
        .expect("bars-x.yaml composes at least one plot");
    let x = scales
        .get(Channel::X)
        .expect("barX binds a value scale on x");
    let min = x.domain_min().expect("a Linear value scale has a min");
    assert!(
        min.abs() < f64::EPSILON,
        "barX's value axis should start at 0, got {min}. The counts in \
         bars-x.yaml start at 12, so a domain beginning there means the zero \
         baseline was applied to the band axis instead of the value axis."
    );
}
