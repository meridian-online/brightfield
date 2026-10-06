//! **A legend over a fill the transform produced names it by the transform.** A
//! hexbin's `fill: { count: }` and a raster with no fill colour by a count of
//! the points in each bin, and a heatmap by that count smoothed into a density;
//! no column of the author's holds either number, so the legend has no column to
//! name. It carries the word vgplot gives the same quantity instead: `count` for
//! a counting aggregate (`exprLabel` in `plot-renderer.js` strips `count(*)` to
//! `count`), and `density` for the grid a raster or a heatmap bins into
//! (`DENSITY` in `Grid2DMark.js`).
//!
//! A fill that is a column still names the column, which the average below holds
//! in place: an aggregate over a named column is that column's name, so a rule
//! that named every hexbin `count` goes red here.
//!
//! The legend is drawn through `legend::draw_band` (at the plot's right) and
//! `legend::draw_below` (under it) over a composed page, into a headless egui
//! context, and read back as the text it painted. The look of it, in both themes,
//! is `legend_transform_baseline.rs`.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, draw_band, draw_below, LegendSpec};
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// The band's left edge in the headless context, and the raster's top.
const BAND_LEFT: f32 = 400.0;
const RASTER_TOP: f32 = 20.0;

/// Sixty rows in seven clusters, so some bins hold several points and the counts
/// differ from bin to bin, with a `z` column an aggregate can average.
fn data() -> String {
    let rows: String = (0..60)
        .map(|i| {
            let cluster = (i % 7) as f64;
            let x = cluster * 1.5 + ((i * 13) % 10) as f64 / 40.0;
            let y = (cluster * 2.0) % 9.0 + ((i * 7) % 10) as f64 / 40.0;
            format!("    - {{ x: {x}, y: {y}, z: {i} }}\n")
        })
        .collect();
    format!("data:\n  t:\n{rows}")
}

/// The mark `block` draws over the table `t`, indented `indent` spaces.
fn mark(block: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    format!(
        "{pad}- mark: {block}\n{pad}  data: {{ from: t }}\n{pad}  x: x\n{pad}  y: y\n",
        block = block.replace('\n', &format!("\n{pad}  "))
    )
}

/// A page whose one plot draws `block`, with its colour legend at the right.
fn at_right(block: &str) -> String {
    format!(
        "{}plot:\n{}  - legend: color\nwidth: 420\nheight: 300\n",
        data(),
        mark(block, 2)
    )
}

/// A `vconcat` of a plot named `chart` drawing `block` and a colour legend `for`
/// it, which the layout puts under it.
fn below(block: &str) -> String {
    format!(
        "{}vconcat:\n  - plot:\n{}    name: chart\n    width: 420\n    height: 300\n  - legend: color\n    for: chart\n",
        data(),
        mark(block, 6)
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// The text `paint` draws for `composed`, in paint order.
fn painted(composed: &Composed, paint: impl Fn(&mut egui::Ui, &Composed)) -> Vec<String> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| paint(ui, composed));
    out.shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            // The glyphs laid out, not `Galley::text`, which is the whole text
            // before any of it is cut short.
            egui::Shape::Text(t) => Some(t.galley.rows.iter().map(|r| r.row.text()).collect()),
            _ => None,
        })
        .collect()
}

/// The text of the legend at the plot's right, in paint order.
fn band_text(composed: &Composed) -> Vec<String> {
    painted(composed, |ui, composed| {
        let band = egui::Rect::from_min_size(
            egui::pos2(BAND_LEFT, RASTER_TOP),
            egui::vec2(band_width(composed), 600.0),
        );
        draw_band(ui, band, RASTER_TOP, composed, Mode::Light);
    })
}

/// The text of the legend under the plot, in paint order.
fn below_text(composed: &Composed) -> Vec<String> {
    painted(composed, |ui, composed| {
        draw_below(ui, egui::pos2(20.0, 20.0), composed, Mode::Light);
    })
}

/// The name a legend carries: the first text it paints, which is drawn over the
/// ramp. A legend that names nothing paints a number first.
fn name_of(text: &[String]) -> &str {
    text.first().map(String::as_str).unwrap_or_default()
}

/// A sequential legend is the one a count or a density is drawn with.
fn assert_ramp(composed: &Composed) {
    assert!(
        matches!(
            LegendSpec::of_plot(&composed.plots[0]),
            Some(LegendSpec::Sequential { .. })
        ),
        "the plot draws a sequential legend: {:?}",
        LegendSpec::of_plot(&composed.plots[0])
    );
}

const HEXBIN: &str = "hexbin\nfill: { count: }";
const RASTER: &str = "raster\nbins: 20";
const HEATMAP: &str = "heatmap\nbins: 20";

/// **AC1.** A hexbin coloured by its count names its legend `count`.
#[test]
fn a_hexbin_filled_by_its_count_names_its_legend_count() {
    let composed = compose(&at_right(HEXBIN));
    assert_ramp(&composed);
    assert_eq!(name_of(&band_text(&composed)), "count");
}

/// **AC1.** A raster with no fill colours by the count in each bin, and says so.
#[test]
fn a_raster_with_no_fill_names_its_legend_count() {
    let composed = compose(&at_right(RASTER));
    assert_ramp(&composed);
    assert_eq!(name_of(&band_text(&composed)), "count");
}

/// **AC1.** A heatmap's legend carries the word for a density, not a count.
#[test]
fn a_heatmap_names_its_legend_density() {
    let composed = compose(&at_right(HEATMAP));
    assert_ramp(&composed);
    assert_eq!(name_of(&band_text(&composed)), "density");
}

/// **AC1, under the plot.** The legend that sits under its plot reads the same
/// name through its own drawing function, so it is held on its own.
#[test]
fn a_legend_under_the_plot_names_the_transform_too() {
    for (block, word) in [(HEXBIN, "count"), (RASTER, "count"), (HEATMAP, "density")] {
        let composed = compose(&below(block));
        assert_ramp(&composed);
        assert_eq!(
            name_of(&below_text(&composed)),
            word,
            "the legend under a {block:?} plot"
        );
    }
}

/// **AC2.** A fill that is a column keeps the column's name: a hexbin filled by
/// the average of `z` is named `z`, not `count` and not nothing.
#[test]
fn a_hexbin_filled_by_a_column_aggregate_still_names_the_column() {
    let composed = compose(&at_right("hexbin\nfill: { avg: z }"));
    assert_eq!(composed.plots[0].fill_column.as_deref(), Some("z"));
    assert_ramp(&composed);
    assert_eq!(name_of(&band_text(&composed)), "z");
}
