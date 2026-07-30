//! Gate: `x: {bin: col}` + `y: {count:}` puts a HISTOGRAM on the page.
//!
//! Before this landed, the idiom parsed clean, emitted `SELECT * FROM …`,
//! exited 0 and wrote a valid PNG with no bars in it. Every structural check
//! passed: the mark was registered in both the lowerer and the renderer
//! registry, the SQL emitted, the scene encoded. Only the picture was missing.
//! So this test counts PIXELS, for the reason `bar_orientation.rs` does.
//!
//! And it does not stop at ink-versus-no-ink. A renderer that lost the count
//! channel and drew one bar per bin at a constant height would clear an ink
//! floor easily, and so would a bin expression that put every row in the same
//! bucket. What is asserted instead is the SHAPE: the bar heights, normalised
//! by the shortest bar, must be the bin counts. `examples/rect-bin-count.yaml`
//! is built for that — 37 raw observations whose 15 occupied bins run
//! 3, 3, 6, 8, 4, 3, 2 and then a long tail of ones, asymmetric and unimodal,
//! so no uniform block and no monotone ramp can impersonate it.
//!
//! `examples/rect-histogram.yaml` — the same chart with its bins pre-aggregated
//! into `data:` — is the control, and it is asserted FIRST. It drew before this
//! change and must draw after it. If it ever reads zero the cause is the
//! harness (wrong colour token, wrong export path) and this file says so
//! instead of quietly passing.

use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::compose_spec;
use std::path::{Path, PathBuf};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The mark colour a rect takes when nothing binds a colour SCALE — Harbour
/// slot 1, read from the token layer so a palette bump moves the expectation
/// with the picture. (`fill: steelblue` is a colour constant, and brightfield
/// paints constants in the default mark ink; see `resolve_colour`.)
fn mark_ink() -> [i32; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance: the fill's core and the first ring of its
/// anti-aliasing. Matches `bar_orientation.rs`, which measures the same ink.
const MARK_INK_TOL: i32 = 20;

/// Render an example and return the PNG path.
fn export(spec: &str, out: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("bf-binned-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(out);
    let composed =
        compose_spec(example(spec).to_str().expect("utf-8 path")).expect("the example composes");
    capture_vello_only(composed, 1.0, &png).expect("export");
    png
}

/// Inked pixels per image COLUMN, left to right.
///
/// A rectY bar sits on the baseline, so a column's inked count is that bar's
/// height in pixels — which is what makes the count channel measurable from
/// the picture rather than from the scene.
fn column_heights(png: &Path) -> Vec<u32> {
    let img = image::open(png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();
    let want = mark_ink();
    (0..w)
        .map(|x| {
            (0..h)
                .filter(|&y| {
                    let p = img.get_pixel(x, y).0;
                    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= MARK_INK_TOL)
                })
                .count() as u32
        })
        .collect()
}

/// Total mark ink in the export.
fn ink(png: &Path) -> u64 {
    column_heights(png).iter().map(|&h| u64::from(h)).sum()
}

/// The distinct bar heights present, as MULTIPLES of the shortest bar, in
/// ascending order.
///
/// Anti-aliased edge columns sit between two heights and would each read as
/// their own bar, so a column is only counted when at least `RUN` of its
/// neighbours agree with it — a real bar is tens of pixels wide, an edge is
/// one or two.
fn bar_heights_in_units(png: &Path) -> Vec<u32> {
    const RUN: usize = 5;
    const TOL: u32 = 2;
    let cols = column_heights(png);
    let solid: Vec<u32> = cols
        .windows(RUN)
        .filter(|w| w[0] > 0 && w.iter().all(|h| h.abs_diff(w[0]) <= TOL))
        .map(|w| w[0])
        .collect();
    let unit = *solid.iter().min().expect("at least one bar") as f64;
    let mut units: Vec<u32> = solid
        .iter()
        .map(|&h| (f64::from(h) / unit).round() as u32)
        .collect();
    units.sort_unstable();
    units.dedup();
    units
}

/// The whole gate: the control inks, the computed histogram inks, and the
/// computed one's bar heights ARE its bin counts.
#[test]
fn a_binned_counted_rect_draws_the_histogram_its_counts_describe() {
    // The control, first. Pre-aggregated bins through the same renderer; it
    // drew before this change and has to draw after it.
    let control = ink(&export("rect-histogram.yaml", "pre-binned.png"));
    assert!(
        control > 1_000,
        "the PRE-BINNED control drew {control} px of mark ink. That path is \
         untouched by binning, so this is the harness failing — check \
         mark_ink() against the current palette before believing anything \
         else in this file."
    );

    let png = export("rect-bin-count.yaml", "computed.png");
    let computed = ink(&png);
    assert!(
        computed > 1_000,
        "the COMPUTED histogram drew {computed} px of mark ink against the \
         control's {control}. `x: {{bin: v}}` + `y: {{count:}}` parses clean, \
         emits SQL and exits 0 while drawing nothing whenever the lowerer \
         falls back to SELECT * — the scales then find no column to build \
         from and RectRenderer returns before its first fill."
    );

    // The shape. 37 observations over 15 occupied bins of width 5, counting
    // 3, 3, 6, 8, 4, 3, 2 and then ones — so six distinct heights, in exactly
    // these ratios. A constant-height bar per bin gives [1]; a lost count
    // channel gives [1]; a bin expression that collapses the column gives one
    // bar. Every one of those clears the ink floor above.
    let heights = bar_heights_in_units(&png);
    assert_eq!(
        heights,
        vec![1, 2, 3, 4, 6, 8],
        "the drawn bar heights, as multiples of the shortest bar, must BE the \
         bin counts of examples/rect-bin-count.yaml"
    );

    // Unimodal: the tallest bar is one bin wide, so the peak is a peak rather
    // than a plateau the arithmetic smeared.
    let cols = column_heights(&png);
    let tallest = *cols.iter().max().expect("inked");
    let at_peak = cols.iter().filter(|&&h| h + 2 >= tallest).count();
    let widest = cols.iter().filter(|&&h| h > 0).count();
    assert!(
        at_peak * 4 < widest,
        "the tallest bar spans {at_peak} of {widest} inked columns — a single \
         bin is one twentieth of the axis, so this is a smear, not a mode"
    );
}

/// The transpose draws too, and draws the same histogram lying down.
///
/// `rectX` bins on `y` and counts on `x`, which is the orientation
/// `flights-hexbin.yaml` needs and the one that lets that spec stop warning.
/// Shipping only `rectY` would leave the corpus half-served with nothing in
/// the picture to say which half.
#[test]
fn the_transposed_histogram_draws_the_same_shape_lying_down() {
    let dir = std::env::temp_dir().join("bf-binned-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let spec_path = dir.join("rect-bin-count-x.yaml");
    let source = std::fs::read_to_string(example("rect-bin-count.yaml")).expect("read example");
    let transposed = source
        .replace("mark: rectY", "mark: rectX")
        .replace("x: { bin: v }", "y: { bin: v }")
        .replace("y: { count: }", "x: { count: }");
    assert!(
        transposed.contains("mark: rectX") && transposed.contains("y: { bin: v }"),
        "the transposition rewrote nothing — the example's channel lines moved"
    );
    std::fs::write(&spec_path, transposed).expect("write transposed spec");

    let png = dir.join("computed-x.png");
    let composed =
        compose_spec(spec_path.to_str().expect("utf-8 path")).expect("the transpose composes");
    capture_vello_only(composed, 1.0, &png).expect("export");

    // Ink, and the same six bar LENGTHS — measured down the rows instead of
    // across the columns, which is what transposing the mark means.
    let img = image::open(&png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();
    let want = mark_ink();
    let rows: Vec<u32> = (0..h)
        .map(|y| {
            (0..w)
                .filter(|&x| {
                    let p = img.get_pixel(x, y).0;
                    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= MARK_INK_TOL)
                })
                .count() as u32
        })
        .collect();
    let total: u64 = rows.iter().map(|&r| u64::from(r)).sum();
    assert!(
        total > 1_000,
        "the transposed histogram drew {total} px of mark ink"
    );
    let solid: Vec<u32> = rows
        .windows(5)
        .filter(|s| s[0] > 0 && s.iter().all(|r| r.abs_diff(s[0]) <= 2))
        .map(|s| s[0])
        .collect();
    let unit = f64::from(*solid.iter().min().expect("at least one bar"));
    let mut units: Vec<u32> = solid
        .iter()
        .map(|&r| (f64::from(r) / unit).round() as u32)
        .collect();
    units.sort_unstable();
    units.dedup();
    assert_eq!(
        units,
        vec![1, 2, 3, 4, 6, 8],
        "rectX must draw the same counts as rectY, along the other axis"
    );
}
