//! **A plot whose file fixes an axis's ends counts the rows past each end, at
//! that end, for a dot.**
//!
//! `xDomain: [0, 100]` draws the x axis from 0 to 100 whatever the rows span,
//! and a dot at 150 is clipped at the frame and drawn nowhere. The count is what
//! tells a reader it is there: a triangle in the warning ink at the end the row
//! fell past, and the number of rows past it, outside the data area.
//!
//! Assertions read two things. [`PlotHandle::rows_past`] is the number the
//! count draws, and the raster of the composed scene, through the renderer the
//! window uses, says where the warning ink landed: at which end, and whether
//! any landed inside the data area. Each arm that fixes an end is paired with one that
//! does not, or with rows that stay inside, so a count drawn whatever the file
//! said would fail.
//!
//! [`PlotHandle::rows_past`]: brightfield_shell::pipeline::PlotHandle::rows_past

use std::path::PathBuf;

use brightfield_conformance::deviations::load_deviations;
use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::ink::ChartInk;
use brightfield_render::past_ends::{mark_counts_rows_past, EndCounts, PastEnds};
use brightfield_render::scale::ViewExtent;
use brightfield_render::VelloRenderer;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_in_mode, Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// Four dots, three inside 0 to 100 on x and the fourth, at x = 150, past it; on
/// y they run 3 to 97. `ATTRS` marks where the plot attributes go.
const DOTS: &str = r"
data:
  pts:
    - { a: 4,   b: 3 }
    - { a: 50,  b: 60 }
    - { a: 96,  b: 97 }
    - { a: 150, b: 60 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 400
height: 300
ATTRS
";

/// Rows past each of the four ends of 0 to 100, a different number at each:
/// one below x, two above it, one below y and three above it, and three rows
/// inside. The baselines are drawn from this.
const FOUR_ENDS: &str = r"
data:
  pts:
    - { a: -20, b: 50 }
    - { a: 150, b: 50 }
    - { a: 130, b: 40 }
    - { a: 50,  b: -10 }
    - { a: 40,  b: 140 }
    - { a: 60,  b: 150 }
    - { a: 70,  b: 160 }
    - { a: 20,  b: 30 }
    - { a: 50,  b: 60 }
    - { a: 80,  b: 70 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 400
height: 300
xDomain: [0, 100]
yDomain: [0, 100]
ATTRS
";

/// A scatter to brush on, and beside it the same table filtered by that brush,
/// the plot whose x ends are fixed. One row lies below 0 and one above 100.
const BRUSHED: &str = r"
params:
  brush: { select: crossfilter }
data:
  pts:
    - { a: -20, b: 3 }
    - { a: 20,  b: 22 }
    - { a: 50,  b: 60 }
    - { a: 80,  b: 71 }
    - { a: 150, b: 97 }
hconcat:
  - plot:
      - mark: dot
        data: { from: pts }
        x: a
        y: b
      - select: intervalX
        as: $brush
    width: 300
    height: 240
  - plot:
      - mark: dot
        data: { from: pts, filterBy: $brush }
        x: a
        y: b
    width: 300
    height: 240
    xDomain: [0, 100]
";

/// `layer` drawn over the rows of [`DOTS`], with the x ends fixed at 0 to 100.
fn layered(layer: &str) -> String {
    format!(
        "data:\n  pts:\n    - {{ a: 4, b: 3 }}\n    - {{ a: 50, b: 60 }}\n    \
         - {{ a: 96, b: 97 }}\n    - {{ a: 150, b: 60 }}\n\
         plot:\n{layer}width: 400\nheight: 300\nxDomain: [0, 100]\n"
    )
}

fn compose_str(source: &str) -> Composed {
    LiveDashboard::load_str(source, None)
        .unwrap_or_else(|e| panic!("the spec loads: {e}\n{source}"))
        .present()
        .unwrap_or_else(|e| panic!("the spec composes: {e}\n{source}"))
}

fn dots(attrs: &str) -> Composed {
    compose_str(&DOTS.replace("ATTRS", attrs))
}

/// Compose `source` in `mode`, through the one mode-aware entry point, which
/// reads a file.
fn compose_in(source: &str, mode: Mode, name: &str) -> Composed {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rows-past");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{name}-{mode:?}.yaml"));
    std::fs::write(&path, source).expect("write fixture");
    compose_spec_in_mode(path.to_str().expect("utf-8 path"), mode)
        .unwrap_or_else(|e| panic!("the fixture composes: {e}\n{source}"))
}

fn counts(x: (u64, u64), y: (u64, u64)) -> PastEnds {
    PastEnds {
        x: EndCounts {
            low: x.0,
            high: x.1,
        },
        y: EndCounts {
            low: y.0,
            high: y.1,
        },
    }
}

// ---------------------------------------------------------------------------
// Reading the warning ink off the raster
// ---------------------------------------------------------------------------

/// The composed scene as pixels, through the renderer the window uses.
fn raster(c: &Composed) -> image::RgbaImage {
    let renderer = VelloRenderer::new();
    let px = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&c.scene, c.width, c.height);
    image::RgbaImage::from_raw(c.width, c.height, px).expect("vello pixel buffer size mismatch")
}

/// A pixel within a few 8-bit steps of `ink` on every channel: the inside of a
/// glyph's stroke or of the triangle, not the anti-aliased rim.
fn is_ink(p: [u8; 4], ink: peniko::Color) -> bool {
    let [r, g, b, _] = ink.components;
    let want = [r, g, b].map(|c| (c * 255.0).round() as i32);
    (0..3).all(|i| (i32::from(p[i]) - want[i]).abs() <= 24)
}

/// A region of plot `plot`, in its own tile's coordinates.
#[derive(Clone, Copy, Debug)]
struct Region {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

/// How many pixels of `img` inside `region` of plot `plot` wear `ink`.
fn inked(
    img: &image::RgbaImage,
    c: &Composed,
    plot: usize,
    region: Region,
    ink: peniko::Color,
) -> usize {
    let at = c.plots[plot].rect;
    let mut n = 0;
    let (x0, x1) = (at.x + region.x0, at.x + region.x1);
    let (y0, y1) = (at.y + region.y0, at.y + region.y1);
    for y in (y0.max(0.0).floor() as u32)..(y1.ceil() as u32).min(img.height()) {
        for x in (x0.max(0.0).floor() as u32)..(x1.ceil() as u32).min(img.width()) {
            if is_ink(img.get_pixel(x, y).0, ink) {
                n += 1;
            }
        }
    }
    n
}

/// The plot's data area, a pixel in from its edges.
fn data_area(c: &Composed, plot: usize) -> Region {
    let l = c.plots[plot].layout;
    Region {
        x0: l.plot_x_start() + 1.0,
        x1: l.plot_x_end() - 1.0,
        y0: l.plot_y_start() + 1.0,
        y1: l.plot_y_end() - 1.0,
    }
}

/// The four places a count is drawn, outside the data area: the x title's row
/// at the left and right edges, and the y title's column at the top and bottom.
/// In order: x low, x high, y low, y high, for axes that run the default way.
fn end_regions(c: &Composed, plot: usize) -> [Region; 4] {
    let l = c.plots[plot].layout;
    let (left, right) = (l.plot_x_start(), l.plot_x_end());
    let (top, bottom) = (l.plot_y_start(), l.plot_y_end());
    let row = |x0: f64, x1: f64| Region {
        x0,
        x1,
        y0: bottom + 18.0,
        y1: bottom + 36.0,
    };
    let column = |y0: f64, y1: f64| Region {
        x0: 0.0,
        x1: 16.0,
        y0,
        y1,
    };
    [
        row(left - 2.0, left + 40.0),
        row(right - 40.0, right + 2.0),
        column(bottom - 40.0, bottom + 2.0),
        column(top - 2.0, top + 40.0),
    ]
}

/// Which of the four ends of plot `plot` carry the warning ink, in
/// [`end_regions`]' order, and how many warning pixels the data area holds.
fn ends_inked(c: &Composed, plot: usize, mode: Mode) -> ([bool; 4], usize) {
    let ink = ChartInk::for_mode(matches!(mode, Mode::Dark)).warning;
    let img = raster(c);
    let ends = end_regions(c, plot).map(|r| inked(&img, c, plot, r, ink) >= 12);
    (ends, inked(&img, c, plot, data_area(c, plot), ink))
}

/// Every warning pixel in the whole picture.
fn warning_anywhere(c: &Composed, mode: Mode) -> usize {
    let ink = ChartInk::for_mode(matches!(mode, Mode::Dark)).warning;
    let img = raster(c);
    img.pixels().filter(|p| is_ink(p.0, ink)).count()
}

// ---------------------------------------------------------------------------
// AC1 — the count at the end the rows fell past
// ---------------------------------------------------------------------------

/// **With `xDomain: [0, 100]`, the row at 150 is counted at the x axis's high
/// end, in the warning ink, and nothing is drawn at the low end or inside the
/// data area.** The same plot without the key counts nothing and draws no
/// warning ink.
#[test]
fn a_row_past_the_high_end_of_x_is_counted_at_that_end() {
    let unset = dots("");
    assert_eq!(
        unset.plots[0].rows_past,
        PastEnds::default(),
        "no key, no count"
    );
    assert_eq!(
        warning_anywhere(&unset, Mode::Light),
        0,
        "no key, no warning ink"
    );

    let asked = dots("xDomain: [0, 100]");
    assert_eq!(asked.plots[0].rows_past, counts((0, 1), (0, 0)));
    let (ends, inside) = ends_inked(&asked, 0, Mode::Light);
    assert_eq!(
        ends,
        [false, true, false, false],
        "the count is drawn at x's high end and at no other"
    );
    assert_eq!(inside, 0, "the count draws no ink inside the data area");
}

/// **A row past y's low end is counted at the bottom of the y title's column,
/// and one past its high end at the top**, and x, whose ends the file left
/// alone, counts nothing. Each arm puts a row past one end only, so a count
/// drawn at the other end fails it.
#[test]
fn a_row_past_each_end_of_y_is_counted_at_that_end() {
    let low = dots("yDomain: [10, 100]");
    assert_eq!(
        low.plots[0].rows_past,
        counts((0, 0), (1, 0)),
        "the row at 3"
    );
    let (ends, inside) = ends_inked(&low, 0, Mode::Light);
    assert_eq!(ends, [false, false, true, false], "at the bottom only");
    assert_eq!(inside, 0, "the count draws no ink inside the data area");

    let high = dots("yDomain: [0, 90]");
    assert_eq!(
        high.plots[0].rows_past,
        counts((0, 0), (0, 1)),
        "the row at 97"
    );
    let (ends, inside) = ends_inked(&high, 0, Mode::Light);
    assert_eq!(ends, [false, false, false, true], "at the top only");
    assert_eq!(inside, 0);
}

/// **With no row past an end, nothing is drawn there**: the same fixed ends
/// over rows that all fall inside them draw no warning ink at all.
#[test]
fn rows_inside_the_ends_draw_no_count() {
    let inside = dots("xDomain: [0, 200]\nyDomain: [0, 100]");
    assert_eq!(inside.plots[0].rows_past, PastEnds::default());
    assert_eq!(warning_anywhere(&inside, Mode::Light), 0);
}

/// **An axis that runs from high to low draws its low end's count where its low
/// end is**: with `xReverse: true` the row at 150 lies past the high end, which
/// is now the left edge.
#[test]
fn a_reversed_axis_draws_the_count_at_the_end_it_moved_to() {
    let reversed = dots("xDomain: [0, 100]\nxReverse: true");
    assert_eq!(reversed.plots[0].rows_past, counts((0, 1), (0, 0)));
    let (ends, inside) = ends_inked(&reversed, 0, Mode::Light);
    assert_eq!(
        ends,
        [true, false, false, false],
        "the high end is at the left"
    );
    assert_eq!(inside, 0);
}

// ---------------------------------------------------------------------------
// AC2 — a binned, raster or aggregated mark draws no count
// ---------------------------------------------------------------------------

/// **A binned, a raster and an aggregating mark draw no count over the same
/// rows and the same fixed ends a dot counts one at.** Their batches hold
/// bins, cells or groups, not rows. The dot arm is the control: a fixture whose
/// rows lay inside the ends would pass the others without the judgement.
#[test]
fn a_binned_raster_or_aggregated_mark_draws_no_count() {
    let dot = compose_str(&layered(
        "  - mark: dot\n    data: { from: pts }\n    x: a\n    y: b\n",
    ));
    assert_eq!(
        dot.plots[0].rows_past,
        counts((0, 1), (0, 0)),
        "control: the dot counts"
    );

    for (what, layer) in [
        (
            "a binned rectY",
            "  - mark: rectY\n    data: { from: pts }\n    x: { bin: a }\n    y: { count: null }\n",
        ),
        (
            "a hexbin",
            "  - mark: hexbin\n    data: { from: pts }\n    x: a\n    y: b\n",
        ),
        (
            "a raster",
            "  - mark: raster\n    data: { from: pts }\n    x: a\n    y: b\n",
        ),
        (
            "a densityX",
            "  - mark: densityX\n    data: { from: pts }\n    x: a\n",
        ),
    ] {
        let composed = compose_str(&layered(layer));
        assert_eq!(
            composed.plots[0].rows_past,
            PastEnds::default(),
            "{what} draws no count"
        );
        assert_eq!(
            warning_anywhere(&composed, Mode::Light),
            0,
            "{what} draws no warning ink"
        );
    }
}

/// **A dot beside a binned mark counts its own rows**, so the count a plot draws
/// is the dot's whatever else the plot holds.
#[test]
fn a_dot_beside_a_binned_mark_counts_its_own_rows() {
    let mixed = compose_str(&layered(
        "  - mark: rectY\n    data: { from: pts }\n    x: { bin: a }\n    y: { count: null }\n\
         \x20 - mark: dot\n    data: { from: pts }\n    x: a\n    y: b\n",
    ));
    assert_eq!(mixed.plots[0].rows_past.x, EndCounts { low: 0, high: 1 });
}

/// **`deviations.yaml` says which marks count and which do not, and its two
/// lists are the code's**: each mark kind the vocabulary declares is named
/// once, under the marks that count exactly when [`mark_counts_rows_past`]
/// says it counts, and `DEVIATIONS.md` carries the entry.
#[test]
fn the_register_names_which_marks_count_as_the_code_does() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = load_deviations(&root.join("deviations.yaml")).expect("the register loads");
    let found: Vec<_> = registry
        .iter()
        .filter(|d| d.surface.contains("a count of the rows past an end"))
        .collect();
    assert_eq!(found.len(), 1, "one entry names the count: {found:?}");
    let entry = found[0];

    // The backticked names between the paragraph's colon and the full stop
    // that ends its first sentence.
    let named = |lead: &str| -> Vec<String> {
        let paragraph = entry
            .brightfield_behaviour
            .split("\n\n")
            .find(|p| p.trim_start().starts_with(lead))
            .unwrap_or_else(|| panic!("the entry has a paragraph that opens `{lead}`"));
        let after = &paragraph[paragraph.find(':').expect("a colon") + 1..];
        let list = &after[..after.find('.').expect("a full stop")];
        list.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    };
    let count = named("Marks that count");
    let none = named("Marks that draw no count");
    for kind in MarkKind::all() {
        let wire = kind.wire_name().to_string();
        let (counts, not) = (count.contains(&wire), none.contains(&wire));
        assert!(
            counts != not,
            "`{wire}` is named once: count {counts}, no count {not}"
        );
        assert_eq!(
            counts,
            mark_counts_rows_past(*kind),
            "`{wire}`: the register and the code disagree on whether it counts"
        );
    }
    assert_eq!(
        count.len() + none.len(),
        MarkKind::all().len(),
        "the register names a mark the vocabulary does not declare: {count:?} {none:?}"
    );

    let doc = std::fs::read_to_string(root.join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    assert!(
        doc.contains(&format!("## {} — ", entry.id)),
        "DEVIATIONS.md has no section for {}",
        entry.id
    );
}

// ---------------------------------------------------------------------------
// AC3 — a plot with no fixed ends draws as it does today
// ---------------------------------------------------------------------------

/// **A plot whose file fixes no end keeps the margins a plot had before the
/// count existed, titled or not; an untitled axis the file fixes reserves its
/// title's band for the count, whether or not a row lies past it.** The
/// reservation reads the file, so it is the same with a count drawn and
/// without one.
#[test]
fn only_a_fixed_untitled_axis_reserves_a_row_for_its_count() {
    let titled = dots("");
    let titled_fixed = dots("xDomain: [0, 200]\nyDomain: [0, 100]");
    assert_eq!(
        margins(&titled),
        margins(&titled_fixed),
        "a titled axis shares its title's row, and the frame does not move"
    );

    let bare = dots("xLabel: null\nyLabel: null");
    let bare_fixed = dots("xLabel: null\nyLabel: null\nxDomain: [0, 200]\nyDomain: [0, 100]");
    let bare_counting = dots("xLabel: null\nyLabel: null\nxDomain: [0, 100]\nyDomain: [10, 90]");
    let (b, f) = (margins(&bare), margins(&bare_fixed));
    assert_eq!(
        f.0,
        b.0 + 20.0,
        "x's title band is reserved under the fixed axis"
    );
    assert_eq!(
        f.1,
        b.1 + 20.0,
        "y's title band is reserved beside the fixed axis"
    );
    assert_eq!(
        margins(&bare_fixed),
        margins(&bare_counting),
        "the band is the same with a count drawn and without one"
    );
    let (ends, inside) = ends_inked(&bare_counting, 0, Mode::Light);
    assert_eq!(
        ends,
        [false, true, true, true],
        "the reserved row carries the count"
    );
    assert_eq!(inside, 0);
}

/// The bottom and left margins of the first plot.
fn margins(c: &Composed) -> (f64, f64) {
    let m = c.plots[0].layout.margins;
    (m.bottom, m.left)
}

// ---------------------------------------------------------------------------
// AC4 — the count follows a brush on another tile
// ---------------------------------------------------------------------------

/// **After a range is swept on the first tile, the second tile's count is of
/// the rows the brush left**, and its frame stays where it was.
#[test]
fn the_count_follows_the_rows_a_brush_on_another_tile_leaves() {
    let mut live = LiveDashboard::load_str(BRUSHED, None).expect("the spec loads live");
    let first = live.present().expect("first composite");
    assert_eq!(
        first.plots[1].rows_past,
        counts((1, 1), (0, 0)),
        "before the brush, one row below 0 and one above 100"
    );
    assert_eq!(
        first.plots[0].rows_past,
        PastEnds::default(),
        "the brushed tile fixes no end"
    );

    let path = first.plots[0].path.clone();
    let mut sweep = |lo: f64, hi: f64| {
        live.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath(path.clone()),
            predicate: SqlPredicate::Interval {
                column: "a".to_string(),
                lo: ScalarValue::Float(lo),
                hi: ScalarValue::Float(hi),
                meta: None,
            },
        })
        .expect("the brush re-composites")
    };

    let inside = sweep(15.0, 85.0);
    assert_eq!(
        inside.plots[1].rows_past,
        PastEnds::default(),
        "the brush left no row past an end"
    );
    assert_eq!(
        warning_anywhere(&inside, Mode::Light),
        0,
        "and nothing is drawn"
    );

    let low = sweep(-30.0, 60.0);
    assert_eq!(
        low.plots[1].rows_past,
        counts((1, 0), (0, 0)),
        "the brush left the row below 0 and none above 100"
    );
    let (ends, inside_ink) = ends_inked(&low, 1, Mode::Light);
    assert_eq!(ends, [true, false, false, false]);
    assert_eq!(inside_ink, 0);

    assert_eq!(
        low.plots[1].layout.margins.bottom, first.plots[1].layout.margins.bottom,
        "the brush moved the frame"
    );
}

/// **An axis the reader has zoomed counts nothing**: the frame is theirs, and
/// rows off it are off because they moved it, not because the file fixed it.
/// The other axis keeps its count, and a zoom put back counts again.
#[test]
fn a_zoomed_axis_counts_nothing_and_the_other_keeps_its_count() {
    let source = DOTS.replace("ATTRS", "xDomain: [0, 100]\nyDomain: [10, 100]");
    let mut live = LiveDashboard::load_str(&source, None).expect("the spec loads live");
    let before = live.present().expect("first composite");
    assert_eq!(
        before.plots[0].rows_past,
        counts((0, 1), (1, 0)),
        "fixture check"
    );

    let path = before.plots[0].path.clone();
    live.set_view_extent(
        &path,
        ViewExtent {
            x: Some((20.0, 60.0)),
            y: None,
        },
    );
    let zoomed = live.present().expect("the zoom re-composites");
    assert_eq!(
        zoomed.plots[0].rows_past,
        counts((0, 0), (1, 0)),
        "x is the reader's now; y still counts the row at 3"
    );

    live.set_view_extent(&path, ViewExtent { x: None, y: None });
    let reset = live.present().expect("the reset re-composites");
    assert_eq!(
        reset.plots[0].rows_past, before.plots[0].rows_past,
        "the reset counts again"
    );
}

// ---------------------------------------------------------------------------
// AC5 — the baselines, in both themes
// ---------------------------------------------------------------------------

/// **The count at each of the four ends**, a different number at each, drawn in
/// the warning ink outside the data area, in light and in dark. The ink is read
/// off the raster before the image is held against its baseline, so a baseline
/// redrawn over a count in the wrong place fails here first.
#[test]
fn the_count_at_each_of_the_four_ends_light_and_dark_baselines() {
    for (mode, name) in [
        (Mode::Light, "rows_past_four_ends_light"),
        (Mode::Dark, "rows_past_four_ends_dark"),
    ] {
        let composed = compose_in(&FOUR_ENDS.replace("ATTRS", ""), mode, name);
        assert_eq!(composed.plots[0].rows_past, counts((1, 2), (1, 3)));
        let (ends, inside) = ends_inked(&composed, 0, mode);
        assert_eq!(ends, [true; 4], "{name}: a count at each end");
        assert_eq!(inside, 0, "{name}: no warning ink inside the data area");
        egui_kittest::image_snapshot(&raster(&composed), name);
    }
}
