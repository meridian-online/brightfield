//! Gate: a binned column on a **log** axis is binned in log space, and the
//! axis is ticked in decades.
//!
//! The failure this exists to catch looks exactly like success. `xScale: log`
//! is a plot attribute; a build that parses it, carries it through the AST and
//! reads it nowhere emits the same SQL, draws the same 20 equal-width bins and
//! writes a perfectly good PNG — which is what this tree did before this
//! branch, for the four vendored specs that ask for a log axis. Ink-versus-no-
//! ink proves nothing here.
//!
//! So what is asserted is the two places the spaces actually differ:
//!
//! - **Where the bins were cut.** `examples/rect-bin-count-log.yaml` holds the
//!   same 37 observations as `examples/rect-bin-count.yaml`. Cut on the raw
//!   column they snap outward to `[0, 100]` in steps of 5; cut on `log10(v)`
//!   they snap to `[0, 2]` in steps of 0.1, which is `[1, 100]` once the edges
//!   are mapped back. **0 against 1 is the whole difference**, and it is the
//!   one number a build that ignored the attribute cannot produce — zero has
//!   no logarithm and is not a place on this axis.
//! - **Where the ticks are.** Decades, from the scale itself, not `nice_step`'s
//!   1/2/5 decimal ladder.
//!
//! Both are read off the composition's own record — `PlotHandle::scales` is
//! the scale set the plot was DRAWN against, not a second derivation — and the
//! raster is read through `capture_vello_only` beside them, so a scale that is
//! right in the record and wrong on the page cannot pass.
//!
//! The second half is about **rows at or below zero**, which is where `log`
//! and `symlog` part company and the reason a spec is offered both. Eight rows
//! over one column holding a negative and a zero: under `log` Mosaic drops
//! those two and six rows stand in bars; under `symlog` the transform is
//! defined through the origin and all eight do. The count is read off the
//! picture as bar heights, because the count channel IS the height.

use std::path::{Path, PathBuf};

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{compose_spec, Composed};

/// `fill: steelblue` — the colour both example documents ask for, and the one
/// `binned_histogram.rs` measures the linear companion in.
const STEELBLUE: [i32; 3] = [0x46, 0x82, 0xb4];

/// Per-channel tolerance: the fill's core and the first ring of its
/// anti-aliasing. Matches `binned_histogram.rs`, which measures the same ink.
const MARK_INK_TOL: i32 = 20;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("bf-log-binned-histogram");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Compose a spec file and rasterise it, returning both halves of the record:
/// the composition (scales included) and the PNG.
fn compose_and_capture(path: &Path, out: &str) -> (Composed, PathBuf) {
    let composed = compose_spec(path.to_str().expect("utf-8 path")).expect("the spec composes");
    let png = scratch().join(out);
    // `capture_vello_only` consumes the composition, so the record is read
    // from a second one built from the same document — the same bytes, and
    // the same query, run twice.
    let recorded = compose_spec(path.to_str().expect("utf-8 path")).expect("the spec composes");
    capture_vello_only(composed, 1.0, &png).expect("export");
    (recorded, png)
}

/// The x scale the first plot was drawn against.
fn x_scale(composed: &Composed) -> Scale {
    composed
        .plots
        .first()
        .expect("one plot")
        .scales
        .get(Channel::X)
        .expect("an x scale")
        .clone()
}

/// Inked pixels per image COLUMN, left to right — a rectY bar sits on the
/// baseline, so a column's inked count is that bar's height in pixels.
fn column_heights(png: &Path, want: [i32; 3]) -> Vec<u32> {
    let img = image::open(png).expect("open png").to_rgba8();
    let (w, h) = img.dimensions();
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

/// The solid bars in the picture as `(left_x, right_x, height)`, in image
/// columns.
///
/// A run of columns of the same height is one bar; anti-aliased edges sit
/// between two heights and are excluded by requiring `RUN` neighbours to
/// agree, the same rule `binned_histogram.rs` uses.
fn bars(png: &Path, want: [i32; 3]) -> Vec<(u32, u32, u32)> {
    const RUN: usize = 4;
    const TOL: u32 = 2;
    let cols = column_heights(png, want);
    let mut out: Vec<(u32, u32, u32)> = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut run_height = 0_u32;
    for (x, &h) in cols.iter().enumerate() {
        match run_start {
            Some(start) if h > 0 && h.abs_diff(run_height) <= TOL => {
                let _ = start;
            }
            _ => {
                if let Some(start) = run_start {
                    if x - start >= RUN {
                        out.push((start as u32, (x - 1) as u32, run_height));
                    }
                }
                run_start = if h > 0 { Some(x) } else { None };
                run_height = h;
            }
        }
    }
    if let Some(start) = run_start {
        if cols.len() - start >= RUN {
            out.push((start as u32, (cols.len() - 1) as u32, run_height));
        }
    }
    out
}

/// The rows standing in bars, counted off the picture: every bar's height in
/// units of the shortest bar, summed.
fn rows_in_bars(png: &Path, want: [i32; 3]) -> u32 {
    let bars = bars(png, want);
    let unit = f64::from(bars.iter().map(|(_, _, h)| *h).min().expect("a bar"));
    bars.iter()
        .map(|(_, _, h)| (f64::from(*h) / unit).round() as u32)
        .sum()
}

// ---------------------------------------------------------------------------
// Where the bins were cut
// ---------------------------------------------------------------------------

/// The log document's bins are cut in log space, and the linear one's are not.
///
/// The two documents hold the same 37 observations, so every number below that
/// differs between them is the attribute doing something.
#[test]
fn a_log_axis_bins_the_column_in_log_space() {
    let (linear, _) = compose_and_capture(&example("rect-bin-count.yaml"), "linear.png");
    let (log, png) = compose_and_capture(&example("rect-bin-count-log.yaml"), "log.png");

    // The control. Cut on the raw column, Mosaic's extent snaps outward to a
    // nice step, and over 1..97 that is [0, 100] — the low edge is zero.
    let linear_x = x_scale(&linear);
    assert!(
        matches!(linear_x, Scale::Linear { .. }),
        "the companion document names no scale and must stay linear, got {linear_x:?}"
    );
    assert_eq!(
        (linear_x.domain_min(), linear_x.domain_max()),
        (Some(0.0), Some(100.0)),
        "the linear document's 37 observations bin across [0, 100] in steps of 5"
    );

    // The log document. Same rows; the extent is snapped in log10 space to
    // [0, 2] in steps of 0.1, which is [1, 100] in the column's own units.
    let log_x = x_scale(&log);
    assert!(
        matches!(log_x, Scale::Log { .. }),
        "`xScale: log` must reach the drawn scale set, got {log_x:?}"
    );
    let (lo, hi) = (
        log_x.domain_min().expect("lo"),
        log_x.domain_max().expect("hi"),
    );
    assert!(
        (lo - 1.0).abs() < 1e-9,
        "the first bin's low edge is 10^0 = 1, not {lo} — a low edge of 0 is \
         the raw column's extent, which is what a build that binned on the \
         column and then drew a log axis would produce"
    );
    assert!(
        (hi - 100.0).abs() < 1e-6,
        "the last bin's high edge is 10^2 = 100, got {hi}"
    );

    // …and the picture stands on those edges. The smallest observation is 1,
    // which is the first bin's low edge, and the largest is 97, which is in
    // the last bin — so the ink runs the full width between them.
    let cols = column_heights(&png, STEELBLUE);
    let first = cols.iter().position(|&h| h > 0).expect("some ink");
    let last = cols.iter().rposition(|&h| h > 0).expect("some ink");
    let (want_first, want_last) = (log_x.map_f64(1.0), log_x.map_f64(100.0));
    assert!(
        (first as f64 - want_first).abs() <= 2.0,
        "the leftmost bar stands on the first bin edge: ink starts at column \
         {first}, the scale puts 1 at {want_first}"
    );
    assert!(
        (last as f64 - want_last).abs() <= 2.0,
        "the rightmost bar reaches the last bin edge: ink ends at column \
         {last}, the scale puts 100 at {want_last}"
    );
}

/// A log axis is ticked in DECADES, and `nice_step`'s decimal ladder is not
/// reachable from one.
#[test]
fn a_log_axis_is_ticked_in_decades() {
    let (log, _) = compose_and_capture(&example("rect-bin-count-log.yaml"), "log-ticks.png");
    let scale = x_scale(&log);
    let ticks = brightfield_render::axis::compute_ticks(&scale, 10);
    let labels: Vec<String> = ticks.iter().map(|t| t.label.clone()).collect();
    assert_eq!(
        labels,
        vec!["1".to_string(), "10".to_string(), "100".to_string()],
        "a log axis over [1, 100] is ticked 1, 10, 100 — a linear nice step \
         over that span would be 10, 20, 30, … or 20, 40, 60, …"
    );
    // Equal ratios are equal distances: the gap 1→10 and the gap 10→100 are
    // the same number of pixels. A decimal ladder cannot have this property
    // and a tick placed by one would not.
    let gap_low = ticks[1].position - ticks[0].position;
    let gap_high = ticks[2].position - ticks[1].position;
    assert!(
        (gap_low - gap_high).abs() < 1e-6,
        "a decade is a decade: 1→10 spans {gap_low} px and 10→100 spans {gap_high}"
    );
}

// ---------------------------------------------------------------------------
// Rows at or below zero
// ---------------------------------------------------------------------------

/// Eight rows over one column, two of them at or below zero, with the occupied
/// bins far enough apart in both spaces that every bar is its own run.
const ZERO_ROWS: &str = r#"
data:
  observations:
    - { v: -10 }
    - { v: 0 }
    - { v: 1 }
    - { v: 1 }
    - { v: 10 }
    - { v: 10 }
    - { v: 10 }
    - { v: 97 }
plot:
  - mark: rectY
    data: { from: observations }
    x: { bin: v }
    y: { count: }
    fill: steelblue
xScale: SCALE
width: 640
height: 400
"#;

fn zero_row_document(scale: &str) -> PathBuf {
    let path = scratch().join(format!("zero-rows-{scale}.yaml"));
    std::fs::write(&path, ZERO_ROWS.replace("SCALE", scale)).expect("write fixture");
    path
}

/// `log` drops the rows it has no logarithm for; `symlog` keeps them.
///
/// The same eight rows, the same mark, the same everything but the attribute.
/// Six rows stand in bars under `log` and eight under `symlog`, and the two
/// missing ones are exactly the negative and the zero.
#[test]
fn log_drops_the_rows_at_or_below_zero_and_symlog_keeps_them() {
    let (log, log_png) = compose_and_capture(&zero_row_document("log"), "zero-log.png");
    let (symlog, symlog_png) = compose_and_capture(&zero_row_document("symlog"), "zero-symlog.png");

    assert!(matches!(x_scale(&log), Scale::Log { .. }));
    assert!(matches!(x_scale(&symlog), Scale::Symlog { .. }));

    let under_log = rows_in_bars(&log_png, STEELBLUE);
    assert_eq!(
        under_log, 6,
        "log has no value at or below zero and Mosaic drops those rows: six of \
         the eight stand in bars"
    );

    let under_symlog = rows_in_bars(&symlog_png, STEELBLUE);
    assert_eq!(
        under_symlog, 8,
        "symlog is defined through the origin, so the -10 row and the 0 row \
         keep their bars and all eight rows stand"
    );

    // The zero row is not merely counted — it has a bar, and it is on the
    // positive side of nothing. Symlog's domain reaches below zero because a
    // row does; log's cannot.
    let symlog_x = x_scale(&symlog);
    assert!(
        symlog_x.domain_min().expect("lo") < 0.0,
        "the -10 row pulls the symlog domain below zero, got {:?}",
        symlog_x.domain_min()
    );
    assert!(
        x_scale(&log).domain_min().expect("lo") > 0.0,
        "a log domain cannot start at or below zero"
    );
}
