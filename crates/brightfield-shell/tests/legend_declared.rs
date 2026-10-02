//! **A plot draws a legend when its file puts one on it, and none when it does
//! not.** The shell used to draw a legend beside every plot whose fill had a
//! scale, whether or not the file held a legend node, so a chart file could not
//! say no legend. A plot now draws its legend when its own items hold a
//! `legend: color`, when a standalone colour legend names it by `for:`, or when
//! a standalone colour legend with no `for:` stands in a file whose one plot
//! with a colour scale it is; and the band the legend sits in is reserved for
//! such a plot and no other.
//!
//! What a legend shows is still read from the plot's scales, so an item over a
//! plot with no colour scale draws nothing. The tests here hold both halves
//! through the real pipeline — spec text, DuckDB, per-plot scales, the band and
//! the rects a headless layout records — and the look of the legend is
//! `dot_number_fill_legend.rs`'s and the baselines'.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, LegendSpec};
use brightfield_shell::pipeline::{compose_spec_str, Composed, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The window every layout here is settled in: wide and tall enough that no
/// spec below is offered less room than it declared.
const WINDOW: (f32, f32) = (1400.0, 900.0);

/// Five rows: a number column `v` to colour by, a string column `g` of two
/// categories, and a position.
const VALUES: [f64; 5] = [0.825, 2.291, 1.5, 4.9, 3.2];

fn data() -> String {
    let rows: String = VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v}, g: g{} }}\n", i * 3, i % 2))
        .collect();
    format!("data:\n  t:\n{rows}")
}

/// A dot over `t` filled by `fill`, as a plot item at `indent` spaces.
fn dot(fill: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    format!(
        "{pad}- mark: dot\n{pad}  data: {{ from: t }}\n{pad}  x: x\n{pad}  y: y\n{pad}  fill: {fill}\n"
    )
}

/// One plot whose items are `items` (written at the plot's indent).
fn one_plot(items: &str) -> String {
    format!("{}plot:\n{items}width: 420\nheight: 300\n", data())
}

/// A plot as a row's child: its `fill`, an optional `name:`, and `extra` items.
fn row_plot(fill: &str, name: Option<&str>, extra: &str) -> String {
    let name = name.map_or(String::new(), |n| format!("    name: {n}\n"));
    format!(
        "  - plot:\n{}{extra}{name}    width: 300\n    height: 260\n",
        dot(fill, 6)
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None).unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// What each plot of the page draws, in plot order: `None` for no legend.
fn drawn(composed: &Composed) -> Vec<Option<LegendSpec>> {
    composed.plots.iter().map(LegendSpec::of_plot).collect()
}

/// `source` settled in [`WINDOW`] over a live document, so the pane re-lays the
/// chart into the room it has left, and the rects that layout recorded.
fn laid_out(source: &str) -> ChartDoc {
    let mut live = LiveDashboard::load_str(source, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let boot = Boot {
        live: Some(live),
        ..Boot::charts(composed)
    };
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(WINDOW.0, WINDOW.1),
        )),
        ..Default::default()
    };
    for _ in 0..4 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    let doc = app.chart_doc();
    let mut out = ChartDoc::headless(Composed::empty());
    out.raster_rect = doc.raster_rect;
    out.legend_rect = doc.legend_rect;
    out
}

/// A plot with `fill`, composed and laid out with the legend item and without it.
struct Pair {
    with: Composed,
    without: Composed,
    with_doc: ChartDoc,
    without_doc: ChartDoc,
}

fn pair(fill: &str) -> Pair {
    let with_source = one_plot(&format!("{}  - legend: color\n", dot(fill, 2)));
    let without_source = one_plot(&dot(fill, 2));
    Pair {
        with: compose(&with_source),
        without: compose(&without_source),
        with_doc: laid_out(&with_source),
        without_doc: laid_out(&without_source),
    }
}

/// The rect of the legend band must sit to the right of the raster.
fn at_the_right(doc: &ChartDoc, what: &str) {
    let raster = doc.raster_rect.expect("a raster");
    let legend = doc.legend_rect.expect("a legend band");
    assert!(
        legend.min.x >= raster.max.x,
        "{what}: the legend band {legend:?} is not at the right of the raster {raster:?}"
    );
}

/// **AC1.** A number column on `fill` with the item draws its sequential legend
/// in the margin at the right; the same plot with the item taken out draws none,
/// reserves no band, and the plot takes the width the band held.
#[test]
fn a_number_fill_draws_its_legend_with_the_item_and_none_without_it() {
    let p = pair("v");

    assert!(
        matches!(drawn(&p.with)[..], [Some(LegendSpec::Sequential { .. })]),
        "the plot holding the item drew {:?}",
        drawn(&p.with)
    );
    let band = band_width(&p.with);
    assert!(band > 0.0, "the plot holding the item reserved no band");
    at_the_right(&p.with_doc, "with the item");

    assert_eq!(
        drawn(&p.without),
        vec![None],
        "the plot without the item drew a legend"
    );
    assert_eq!(
        band_width(&p.without),
        0.0,
        "the plot without the item reserved a band"
    );
    assert_eq!(
        p.without_doc.legend_rect, None,
        "the plot without the item recorded a legend band"
    );
    // The colour is still on the marks: the file said no legend, not no colour.
    assert!(
        LegendSpec::from_scales(&p.without.plots[0].scales).is_some(),
        "taking the item out took the colour scale with it"
    );

    let (r_with, r_without) = (
        p.with_doc.raster_rect.expect("a raster"),
        p.without_doc.raster_rect.expect("a raster"),
    );
    assert!(
        r_without.width() >= r_with.width() + band - 1.0,
        "the plot did not take the width the band held: {} with the item, {} without, band {band}",
        r_with.width(),
        r_without.width()
    );
}

/// **AC2.** A string column on `fill` does the same: swatches with the item, no
/// legend and no band without it.
#[test]
fn a_string_fill_draws_swatches_with_the_item_and_none_without_it() {
    let p = pair("g");

    match &drawn(&p.with)[..] {
        [Some(LegendSpec::Categorical { entries })] => {
            let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["g0", "g1"], "the string fill's own categories");
        }
        other => panic!("the plot holding the item drew {other:?}"),
    }
    assert!(
        band_width(&p.with) > 0.0,
        "the plot holding the item reserved no band"
    );
    at_the_right(&p.with_doc, "with the item");

    assert_eq!(drawn(&p.without), vec![None], "swatches drawn with no item");
    assert_eq!(band_width(&p.without), 0.0, "a band reserved with no item");
    assert_eq!(p.without_doc.legend_rect, None);
}

/// **AC3.** An item over a plot with no colour scale draws no legend and
/// reserves no band: a literal fill, no fill at all, and an item for another
/// channel over a plot that does have a colour scale.
#[test]
fn an_item_over_a_plot_with_no_colour_scale_draws_nothing() {
    let literal = compose(&one_plot(&format!(
        "{}  - legend: color\n",
        dot("\"#aaaaaa\"", 2)
    )));
    assert_eq!(drawn(&literal), vec![None], "a literal fill drew a legend");
    assert_eq!(band_width(&literal), 0.0, "a literal fill reserved a band");

    let none = compose(&one_plot(
        "  - mark: dot\n    data: { from: t }\n    x: x\n    y: y\n  - legend: color\n",
    ));
    assert_eq!(drawn(&none), vec![None], "a plot with no fill drew a legend");
    assert_eq!(band_width(&none), 0.0, "a plot with no fill reserved a band");

    let opacity = compose(&one_plot(&format!(
        "{}  - legend: opacity\n",
        dot("v", 2)
    )));
    assert_eq!(
        drawn(&opacity),
        vec![None],
        "an opacity legend put a colour legend on the plot"
    );
    assert_eq!(band_width(&opacity), 0.0, "an opacity legend reserved a band");
}

/// **AC4, by `for:`.** A standalone colour legend that names the plot keeps the
/// plot's legend at its right.
#[test]
fn a_standalone_legend_that_names_its_plot_by_for_keeps_the_legend_at_its_right() {
    let source = format!(
        "{}hconcat:\n{}  - legend: color\n    for: scatter\n",
        data(),
        row_plot("g", Some("scatter"), "")
    );
    let composed = compose(&source);
    match &drawn(&composed)[..] {
        [Some(LegendSpec::Categorical { entries })] => assert_eq!(entries.len(), 2),
        other => panic!("the named plot drew {other:?}"),
    }
    assert!(band_width(&composed) > 0.0, "the named plot reserved no band");
    at_the_right(&laid_out(&source), "named by for:");
}

/// **AC4, with no `for:`.** A standalone colour legend with no `for:` belongs to
/// the file's one plot whose scales call for a legend — whether it stands alone
/// in the file or beside plots that have none.
#[test]
fn a_standalone_legend_with_no_for_belongs_to_the_files_one_colour_plot() {
    let alone_source = format!(
        "{}hconcat:\n{}  - legend: color\n",
        data(),
        row_plot("g", None, "")
    );
    let alone = compose(&alone_source);
    assert!(
        matches!(drawn(&alone)[..], [Some(LegendSpec::Categorical { .. })]),
        "the one colour plot drew {:?}",
        drawn(&alone)
    );
    assert!(band_width(&alone) > 0.0, "no band for the one colour plot");
    at_the_right(&laid_out(&alone_source), "one colour plot, no for:");

    let beside = compose(&format!(
        "{}hconcat:\n{}{}  - legend: color\n",
        data(),
        row_plot("\"#aaaaaa\"", None, ""),
        row_plot("g", None, "")
    ));
    let d = drawn(&beside);
    assert_eq!(d.len(), 2, "two plots placed");
    assert_eq!(d[0], None, "the plot with no colour scale drew a legend");
    assert!(
        matches!(d[1], Some(LegendSpec::Categorical { .. })),
        "the file's one colour plot drew {:?}",
        d[1]
    );
}

/// With no `for:` and two plots whose scales call for a legend, the legend
/// cannot mean either, and neither draws one. And a legend that is not a colour
/// legend names no plot at all.
#[test]
fn a_standalone_legend_with_no_for_among_two_colour_plots_belongs_to_neither() {
    let two = compose(&format!(
        "{}hconcat:\n{}{}  - legend: color\n",
        data(),
        row_plot("g", None, ""),
        row_plot("v", None, "")
    ));
    assert_eq!(
        drawn(&two),
        vec![None, None],
        "an unnamed legend picked a plot out of two"
    );
    assert_eq!(band_width(&two), 0.0, "an unnamed legend reserved a band");

    let opacity = compose(&format!(
        "{}hconcat:\n{}  - legend: opacity\n",
        data(),
        row_plot("g", None, "")
    ));
    assert_eq!(
        drawn(&opacity),
        vec![None],
        "a standalone opacity legend put a colour legend on the plot"
    );
}

/// A standalone legend that names one plot gives that plot its legend and the
/// plot beside it none, though both have a colour scale.
#[test]
fn a_standalone_legend_that_names_one_plot_leaves_the_other_without() {
    let composed = compose(&format!(
        "{}hconcat:\n{}{}  - legend: color\n    for: second\n",
        data(),
        row_plot("g", Some("first"), ""),
        row_plot("v", Some("second"), "")
    ));
    let d = drawn(&composed);
    assert_eq!(d.len(), 2, "two plots placed");
    assert_eq!(d[0], None, "the plot the legend does not name drew one");
    assert!(
        matches!(d[1], Some(LegendSpec::Sequential { .. })),
        "the plot the legend names drew {:?}",
        d[1]
    );
    assert!(band_width(&composed) > 0.0, "no band for the named plot");
}
