//! Gate: a binned rect whose `fill:` names a COLUMN draws one STACKED segment
//! per category per bin.
//!
//! `examples/rect-bin-count.yaml` and `examples/rect-bin-count-grouped.yaml`
//! differ by one word — `fill: steelblue` against `fill: site` — and that word
//! decides whether the mark carries groups at all. Before this landed the
//! grouped form did not draw: the parser refused to lift a grouped bin pair, so
//! the spec kept its uncomputed-transform warning and the plot came back empty.
//!
//! So this file counts PIXELS, for the reason `binned_histogram.rs` does, and
//! it does not stop at ink-versus-no-ink. Three wrong pictures each clear a
//! weaker check and are excluded here by a different assertion:
//!
//! - **the group dropped from the GROUP BY** — one bar per bin at the bin's
//!   total, in one colour. Excluded by the composition table, which reads a
//!   different triple out of every one of the eleven bins.
//! - **segments drawn side by side** rather than stacked. Excluded by the same
//!   table: a dodged bar is a fraction of a bin wide, so the runs this finds
//!   would be narrow and three times as many.
//! - **the stack offsets lost**, each segment drawn from the baseline. The
//!   composition table would still pass — every segment keeps its own height —
//!   so `each_bin_s_segments_stack_from_the_baseline_in_category_order` reads
//!   the vertical ORDER instead, and the tiling: the slabs must touch, in
//!   category order, from the baseline to the bin's total.
//!
//! `examples/rect-bin-count.yaml` is the control. Its constant fill must NOT
//! become a group, and `a_constant_fill_stays_one_bar_per_bin` holds that from
//! the other side: no category ink anywhere in its raster.

use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::compose_spec;
use std::path::{Path, PathBuf};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The three categorical swatches the grouped fixture's `site` column takes,
/// in the order the colour scale hands them out — read from the token layer, so
/// a palette bump moves the expectation with the picture rather than reddening
/// this file.
fn category_inks() -> [[i32; 3]; 3] {
    let slot = |i: usize| {
        let c = meridian_design::viz::CATEGORICAL_LIGHT[i];
        [
            (c.r * 255.0).round() as i32,
            (c.g * 255.0).round() as i32,
            (c.b * 255.0).round() as i32,
        ]
    };
    [slot(0), slot(1), slot(2)]
}

/// The names those three slots stand for, for assertion messages.
const CATEGORIES: [&str; 3] = ["harbour", "quarry", "ridge"];

/// Per-channel tolerance: the fill's core and the first ring of its
/// anti-aliasing. Matches `binned_histogram.rs`, which measures the same inks.
const MARK_INK_TOL: i32 = 20;

/// The composition of each occupied bin, as counts per category in
/// [`CATEGORIES`] order — the table `examples/rect-bin-count-grouped.yaml`
/// documents, and the one the fixture's rows were chosen to produce.
const EXPECTED: [[u32; 3]; 11] = [
    [2, 1, 0],
    [2, 1, 1],
    [1, 2, 1],
    [0, 3, 1],
    [2, 0, 2],
    [0, 1, 1],
    [1, 1, 0],
    [0, 0, 1],
    [2, 0, 0],
    [1, 0, 0],
    [0, 0, 2],
];

/// Render an example and return the PNG path.
fn export(spec: &str, out: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("bf-grouped-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(out);
    let composed =
        compose_spec(example(spec).to_str().expect("utf-8 path")).expect("the example composes");
    capture_vello_only(composed, 1.0, &png).expect("export");
    png
}

/// One category's ink in one image column: how many rows carry it, and the
/// topmost and bottommost of those rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Slab {
    pixels: u32,
    top: u32,
    bottom: u32,
}

/// One bin's bar, read off the raster: the columns it spans and, per category,
/// the slab of ink found in the column down its middle.
#[derive(Debug)]
struct Bar {
    left: u32,
    right: u32,
    slabs: [Option<Slab>; 3],
}

impl Bar {
    fn width(&self) -> u32 {
        self.right - self.left + 1
    }

    /// The bar's total ink height in pixels — the bin's count, once divided by
    /// the unit.
    fn ink(&self) -> u32 {
        self.slabs.iter().flatten().map(|s| s.pixels).sum()
    }
}

fn near(a: i32, b: i32) -> bool {
    (a - b).abs() <= MARK_INK_TOL
}

/// The slab each category occupies in one image column.
fn column_slabs(img: &image::RgbaImage, x: u32, inks: &[[i32; 3]; 3]) -> [Option<Slab>; 3] {
    let mut out = [None; 3];
    for (i, want) in inks.iter().enumerate() {
        let mut pixels = 0;
        let mut top = u32::MAX;
        let mut bottom = 0;
        for y in 0..img.height() {
            let p = img.get_pixel(x, y).0;
            let hit = near(i32::from(p[0]), want[0])
                && near(i32::from(p[1]), want[1])
                && near(i32::from(p[2]), want[2]);
            if hit {
                pixels += 1;
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        if pixels > 0 {
            out[i] = Some(Slab { pixels, top, bottom });
        }
    }
    out
}

/// The narrowest run of columns this reads as a bar. A bin is ~100 device
/// columns wide in the fixture; the runs below this are the anti-aliased seam
/// between two bins, one or two columns of blended ink that match no swatch.
const MIN_BAR_WIDTH: u32 = 15;

/// The bars in a raster, left to right.
///
/// A bar is a maximal run of image columns whose per-category ink HEIGHTS are
/// the same, which is what one bin's bar is and what two adjacent bins are not:
/// the fixture's eleven bins carry eleven different compositions, so no two
/// neighbours merge into one run.
fn bars(png: &Path, inks: &[[i32; 3]; 3]) -> Vec<Bar> {
    let img = image::open(png).expect("open png").to_rgba8();
    let heights = |s: &[Option<Slab>; 3]| s.map(|c| c.map_or(0, |c| c.pixels));
    let mut out: Vec<Bar> = Vec::new();
    let mut run: Option<(u32, u32, [u32; 3])> = None;
    for x in 0..img.width() {
        let slabs = column_slabs(&img, x, inks);
        let h = heights(&slabs);
        match run {
            Some((start, _, prev)) if prev == h => run = Some((start, x, prev)),
            _ => {
                if let Some((start, end, prev)) = run {
                    if end - start + 1 >= MIN_BAR_WIDTH && prev.iter().any(|p| *p > 0) {
                        let mid = (start + end) / 2;
                        out.push(Bar {
                            left: start,
                            right: end,
                            slabs: column_slabs(&img, mid, inks),
                        });
                    }
                }
                run = Some((x, x, h));
            }
        }
    }
    if let Some((start, end, prev)) = run {
        if end - start + 1 >= MIN_BAR_WIDTH && prev.iter().any(|p| *p > 0) {
            let mid = (start + end) / 2;
            out.push(Bar {
                left: start,
                right: end,
                slabs: column_slabs(&img, mid, inks),
            });
        }
    }
    out
}

/// Pixels per unit count: the shortest bar in the raster is one count tall, so
/// its ink IS the unit. The same normalisation `binned_histogram.rs` uses, and
/// for the same reason — it reads the shape out of the picture without the test
/// having to know the plot's inner height or its device scale.
fn unit(bars: &[Bar]) -> f64 {
    f64::from(bars.iter().map(Bar::ink).min().expect("at least one bar"))
}

/// A slab's height in counts, rounded — the segment's own value, recovered from
/// the picture.
fn counts(pixels: u32, unit: f64) -> u32 {
    (f64::from(pixels) / unit).round() as u32
}

/// **AC1.** Each bin draws one segment per category present in it, in the
/// categorical colour scale, at that category's own count.
///
/// Removing the category from `RectLowerer`'s `group_by` collapses the rows
/// back to one per bin and reddens this: the fixture's eleven bins carry eleven
/// different compositions, so there is no reading of the picture in which the
/// collapsed form passes for the split one.
#[test]
fn a_categorical_fill_splits_each_bin_into_one_stacked_segment_per_category() {
    let inks = category_inks();
    let png = export("rect-bin-count-grouped.yaml", "grouped.png");
    let bars = bars(&png, &inks);

    assert_eq!(
        bars.len(),
        EXPECTED.len(),
        "one bar per occupied bin; found {:?}",
        bars.iter().map(|b| (b.left, b.right)).collect::<Vec<_>>()
    );

    // A dodged (side-by-side) layout would put three narrow runs where one bin
    // stands, so the bars would be a third of the width and three times as many.
    // The count above catches the second half of that; this catches the first.
    let widest = bars.iter().map(Bar::width).max().expect("a bar");
    let narrowest = bars.iter().map(Bar::width).min().expect("a bar");
    assert!(
        widest - narrowest <= 4,
        "the bins are one width: {narrowest}..{widest}"
    );

    let unit = unit(&bars);
    let read: Vec<[u32; 3]> = bars
        .iter()
        .map(|b| b.slabs.map(|s| s.map_or(0, |s| counts(s.pixels, unit))))
        .collect();
    assert_eq!(
        read.as_slice(),
        EXPECTED.as_slice(),
        "the composition of each bin, as {CATEGORIES:?}, in units of {unit} px"
    );
}

/// **AC1, the stacking half.** The segments of a bin touch, in category order,
/// from the baseline up to the bin's total.
///
/// This is the assertion that separates a stack from three overlapping bars
/// each grown from the baseline. That wrong picture keeps every segment's own
/// height, so
/// `a_categorical_fill_splits_each_bin_into_one_stacked_segment_per_category`
/// passes over it unchanged; what it cannot keep is the ORDER and the seams.
#[test]
fn each_bin_s_segments_stack_from_the_baseline_in_category_order() {
    let inks = category_inks();
    let png = export("rect-bin-count-grouped.yaml", "grouped-stack.png");
    let bars = bars(&png, &inks);
    assert_eq!(bars.len(), EXPECTED.len(), "one bar per occupied bin");

    // One baseline for the whole plot: the lowest inked row in any bar. A
    // segment drawn from the baseline when it should have been lifted is
    // exactly what the seam walk below refuses.
    let baseline = bars
        .iter()
        .filter_map(|b| b.slabs.iter().flatten().map(|s| s.bottom).max())
        .max()
        .expect("inked rows");

    for (i, bar) in bars.iter().enumerate() {
        let mut edge = baseline;
        for (slot, slab) in bar.slabs.iter().enumerate() {
            let Some(slab) = slab else { continue };
            assert!(
                slab.bottom.abs_diff(edge) <= 2,
                "bin {i}: {} starts at row {} but the slab beneath it ended at {edge}",
                CATEGORIES[slot],
                slab.bottom
            );
            assert!(
                (slab.bottom - slab.top + 1).abs_diff(slab.pixels) <= 2,
                "bin {i}: {}'s ink is not one contiguous slab ({} rows over {}..{})",
                CATEGORIES[slot],
                slab.pixels,
                slab.top,
                slab.bottom
            );
            edge = slab.top;
        }
        // The stack reaches the bin's total, so the topmost ink sits its whole
        // count above the baseline rather than its tallest segment's count.
        assert!(
            (baseline - edge + 1).abs_diff(bar.ink()) <= 4,
            "bin {i}: the stack spans {} rows but its segments sum to {}",
            baseline - edge + 1,
            bar.ink()
        );
    }
}

/// **The control, and the other side of the claim.** A `fill:` naming a COLOUR
/// is not a group: `examples/rect-bin-count.yaml` still draws one steelblue bar
/// per bin and puts no categorical swatch on the page.
///
/// It drew before the grouped form landed and must draw after it. If this file
/// ever reads no steelblue the cause is the harness — wrong ink, wrong export
/// path — and this test says so instead of the two above passing vacuously.
#[test]
fn a_constant_fill_stays_one_bar_per_bin() {
    const STEELBLUE: [i32; 3] = [0x46, 0x82, 0xb4];
    let png = export("rect-bin-count.yaml", "constant.png");

    let steelblue = bars(&png, &[STEELBLUE, STEELBLUE, STEELBLUE]);
    assert!(
        steelblue.len() > 4,
        "the constant-fill histogram still draws its bars; found {}",
        steelblue.len()
    );

    let inks = category_inks();
    let img = image::open(&png).expect("open png").to_rgba8();
    let mut found = [0u32; 3];
    for x in 0..img.width() {
        for (slot, slab) in column_slabs(&img, x, &inks).iter().enumerate() {
            found[slot] += slab.map_or(0, |s| s.pixels);
        }
    }
    // Slot 0 IS the default mark ink, which a plot's chrome does not use but a
    // sibling mark would, so it is excluded from the swatch count rather than
    // asserted at zero. Slots 1 and 2 belong to a colour scale this spec has
    // no reason to build.
    assert_eq!(
        (found[1], found[2]),
        (0, 0),
        "a constant fill builds no categorical colour scale"
    );
}
