//! Gate: under `stackOffset: normalize` a grouped histogram's every occupied
//! bin reaches the SAME height, because each segment is divided by its own
//! bin's total — on the ghost layer and on the filtered layer alike.
//!
//! The divisor is the whole feature. Divided by the LAYER's total each bin
//! keeps its relative height, the tall bins stay tall and the long tail stays
//! invisible, which is the reading the control exists to escape. That wrong
//! picture is a picture: it has bars, in the right colours, in the right
//! proportions within each bin. What it does not have is one common top, and
//! that is what this file reads.
//!
//! It reads it off the RASTER of `examples/rect-bin-count-grouped-shares.yaml`,
//! which is `rect-bin-count-ghost.yaml`'s two-layer device over grouped rows:
//! an unfiltered layer that never narrows and a `filterBy: $brush` layer over
//! it. A brush applied through the live session makes them differ, and both
//! must still reach the same top — the filtered layer because it normalised
//! over the rows it kept, the ghost because it normalised over all of them.
//!
//! **Ink is read by SATURATION**, not by matching a swatch. The three category
//! colours are strongly saturated and the ghost is the same hue at a fifth of
//! the alpha, while the plot's surface, its gridlines and its axis text are all
//! neutral warm greys — so one threshold finds every bar of both layers without
//! this file having to know which token the ghost takes.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{live_spec, Composed};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;
use image::RgbaImage;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-grouped-shares.yaml")
}

/// The histogram plot's frame in image pixels — the region every reading here
/// is taken over. The scatter beside it is plot 0 and its dots are saturated
/// too, so the frame is what keeps them out.
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

fn raster(composed: Composed, name: &str) -> RgbaImage {
    let dir = std::env::temp_dir().join("bf-grouped-shares");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let png = dir.join(name);
    capture_vello_only(composed, 1.0, &png).expect("export");
    image::open(&png).expect("open png").to_rgba8()
}

/// How far apart a pixel's channels must be to count as a bar rather than as
/// chrome. The categorical swatches clear it by a wide margin at full strength
/// and by roughly a third of it as ghosts; the surface, the grid and the text
/// are within three of neutral.
const SATURATION: i32 = 15;

fn is_ink(p: [u8; 4]) -> bool {
    let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
    r.max(g).max(b) - r.min(g).min(b) >= SATURATION
}

/// The topmost inked row in each frame column that has one, as
/// `(column, row)` — the tops of the stacks, left to right.
fn ink_tops(img: &RgbaImage, frame: &Frame) -> Vec<(u32, u32)> {
    (frame.x0..frame.x1)
        .filter_map(|x| {
            (frame.y0..frame.y1)
                .find(|&y| is_ink(img.get_pixel(x, y).0))
                .map(|y| (x, y))
        })
        .collect()
}

/// The three category swatches at FULL strength — the filtered layer's ink,
/// read from the token layer so a palette bump moves with the picture.
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

/// Frame columns carrying a full-strength category swatch anywhere.
fn columns_at_full_strength(img: &RgbaImage, frame: &Frame) -> Vec<u32> {
    let inks = category_inks();
    (frame.x0..frame.x1)
        .filter(|&x| {
            (frame.y0..frame.y1).any(|y| {
                let p = img.get_pixel(x, y).0;
                inks.iter()
                    .any(|want| (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= 20))
            })
        })
        .collect()
}

/// The per-column height of each category's full-strength ink — the filtered
/// layer's composition, column by column.
fn compositions(img: &RgbaImage, frame: &Frame) -> Vec<[u32; 3]> {
    let inks = category_inks();
    (frame.x0..frame.x1)
        .map(|x| {
            let mut out = [0u32; 3];
            for y in frame.y0..frame.y1 {
                let p = img.get_pixel(x, y).0;
                for (i, want) in inks.iter().enumerate() {
                    if (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= 20) {
                        out[i] += 1;
                    }
                }
            }
            out
        })
        .collect()
}

/// The rows the tops sit on, deduplicated within a two-pixel band — a stack
/// whose top row is 50 and one whose top row is 51 are the same height, and a
/// bar edge landing either side of the pixel grid is not a finding.
fn distinct_tops(tops: &[(u32, u32)]) -> Vec<u32> {
    let mut rows: Vec<u32> = tops.iter().map(|(_, y)| *y).collect();
    rows.sort_unstable();
    rows.dedup_by(|a, b| a.abs_diff(*b) <= 2);
    rows
}

/// **AC2.** Every occupied bin's stack reaches the same height, before a brush
/// and after one, and the two layers reach it independently.
///
/// Replacing the bin's total with the layer's — `PARTITION BY <bin low edge>`
/// dropped from the divisor's window — leaves every assertion here reading a
/// different number of tops, because the fixture's bins carry 3, 4, 4, 4, 4, 2,
/// 2, 1, 2, 1 and 2 rows and a share of the layer is a different fraction for
/// each.
#[test]
fn an_occupied_bin_reaches_the_full_height_under_normalise() {
    let path = fixture();
    let (mut live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the fixture loads live");
    let brushed_plot = ComponentPath(resting.plots[0].path.clone());
    // The histogram is the second plot; the first is the scatter the brush is
    // drawn on.
    let frame = Frame::of(&resting, 1);

    let before = raster(resting, "resting.png");
    let tops_before = ink_tops(&before, &frame);
    assert!(
        tops_before.len() > 200,
        "fixture check: the resting histogram has bars to measure, not {}",
        tops_before.len()
    );
    assert_eq!(
        distinct_tops(&tops_before),
        vec![distinct_tops(&tops_before)[0]],
        "at rest every occupied bin's stack reaches one height; it reached {:?}",
        distinct_tops(&tops_before)
    );

    // Brush the scatter down to its first half. `hour` runs 1..29 in row order,
    // so this keeps a contiguous prefix: the shallow bins thin unevenly and the
    // deep ones empty entirely.
    let filtered = live
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: brushed_plot,
            predicate: SqlPredicate::Interval {
                column: "hour".to_string(),
                lo: ScalarValue::Float(1.0),
                hi: ScalarValue::Float(14.0),
                meta: None,
            },
        })
        .expect("the brush re-composites");
    let after = raster(filtered, "filtered.png");

    // **The filtered layer redrew.** Some bin's full-strength composition is
    // not what it was, so the assertion below is over a picture the brush
    // actually changed rather than over the resting one photographed twice.
    assert_ne!(
        compositions(&before, &frame),
        compositions(&after, &frame),
        "the brush changed no bin's composition, so nothing was re-normalised"
    );

    // **The ghost layer is alone somewhere.** A bin the brush emptied carries
    // no full-strength ink at all, and it is still inked — which is the ghost,
    // normalised over its own rows.
    let tops_after = ink_tops(&after, &frame);
    let full = columns_at_full_strength(&after, &frame);
    let ghost_only: Vec<u32> = tops_after
        .iter()
        .map(|(x, _)| *x)
        .filter(|x| !full.contains(x))
        .collect();
    assert!(
        !ghost_only.is_empty(),
        "the brush emptied no bin, so the ghost is never measured alone"
    );

    // **And both layers reach the same top.** One height over the whole frame,
    // whichever layer drew the column.
    assert_eq!(
        distinct_tops(&tops_after),
        vec![distinct_tops(&tops_after)[0]],
        "after the brush the bins reached {:?}",
        distinct_tops(&tops_after)
    );
    assert_eq!(
        distinct_tops(&tops_after)[0],
        distinct_tops(&tops_before)[0],
        "a share of a bin is a share whatever the brush kept, so the top did \
         not move"
    );
}

/// **The control, from the other side.** The same rows WITHOUT the offset read
/// as counts, so their bins reach different heights.
///
/// Without this the assertion above would pass over a plot that had lost its
/// bars entirely, or over one drawn at a single height for a reason that has
/// nothing to do with the divisor — a y domain collapsed to the tallest bin
/// would do it. `examples/rect-bin-count-grouped.yaml` is the same three sites
/// over the same bins with the offset absent.
#[test]
fn without_the_offset_the_same_bins_reach_different_heights() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-grouped.yaml");
    let (_live, resting) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the fixture loads live");
    let frame = Frame::of(&resting, 0);
    let img = raster(resting, "counts.png");
    let tops = ink_tops(&img, &frame);
    assert!(
        tops.len() > 200,
        "fixture check: the counted histogram has bars to measure"
    );
    assert!(
        distinct_tops(&tops).len() >= 3,
        "the fixture's bins carry 1, 2, 3 and 4 rows, so a counted reading \
         stands at several heights; it stood at {:?}",
        distinct_tops(&tops)
    );
}
