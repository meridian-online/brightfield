//! **A page drawn from a dot with a number-column `fill` draws a sequential
//! legend**, whose ends span the column's values and which sits outside the
//! plot's data area.
//!
//! The shell's legend reads the plot's *fill scale* (`LegendSpec::from_scales`),
//! and a dot's number fill used to leave that scale a `Linear` — which the
//! legend derives no legend from — so the page drew no legend and no reserved band. The
//! paint itself is held in `brightfield-render`'s `tests/dot_number_fill.rs`,
//! over the renderer; this file holds the other half through the real pipeline:
//! spec text → DuckDB → per-plot scales → the legend and the band it reserves.
//! The page draws that legend where the file puts a `legend: color` item on the
//! plot, so each spec here that expects a legend holds one; the file's own
//! tests for the item are `legend_declared.rs`.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, LegendSpec};
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};

/// The column the dot is filled by. The values are positive, so the ramp is
/// anchored at zero and ends at the maximum, which is 4.9.
const VALUES: [f64; 5] = [0.825, 2.291, 1.5, 4.9, 3.2];

fn rows() -> String {
    VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v}, g: g{} }}\n", i * 3, i % 2))
        .collect()
}

/// A one-plot spec: `layers` are the plot's marks, written by the caller.
fn spec(layers: &str) -> String {
    format!(
        "data:\n  t:\n{}plot:\n{layers}width: 420\nheight: 300\n",
        rows()
    )
}

fn dot(fill: &str) -> String {
    format!("  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: {fill}\n")
}

/// `layers` with the item that puts the colour legend on the plot.
fn with_legend(layers: &str) -> String {
    format!("{layers}  - legend: color\n")
}

fn compose(layers: &str) -> Composed {
    compose_spec_str(&spec(layers), None).unwrap_or_else(|e| panic!("the spec must compose: {e}"))
}

/// One headless layout pass at the window the shell would ask for, returning
/// the document with its recorded rects.
fn laid_out(composed: Composed) -> ChartDoc {
    let (w, h) = chart_window_size(&composed);
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    let doc = app.chart_doc();
    let mut out = ChartDoc::headless(Composed::empty());
    out.viewport = doc.viewport;
    out.raster_rect = doc.raster_rect;
    out.legend_rect = doc.legend_rect;
    out
}

/// **AC1.** A dot filled by a number column draws a sequential legend, and the
/// legend is the fill scale the marks were painted against — its ends span the
/// column's values — in a band beside the raster, not on it.
#[test]
fn a_number_fill_on_a_dot_draws_a_sequential_legend_beside_the_plot() {
    let composed = compose(&with_legend(&dot("v")));
    assert_eq!(composed.plots.len(), 1, "one plot");
    let scales = &composed.plots[0].scales;

    let Some(LegendSpec::Sequential { min, max, stops }) = LegendSpec::from_scales(scales) else {
        panic!(
            "a number fill drew no sequential legend; the fill scale is {:?}",
            scales.get(Channel::Fill)
        );
    };
    let lo = VALUES.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = VALUES.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        min <= lo && max >= hi,
        "the legend's ends [{min}, {max}] do not span the column's values [{lo}, {hi}]"
    );
    assert_eq!(max, hi, "the top of the ramp is the column's maximum");
    assert!(
        stops.len() >= 2,
        "a ramp needs two stops to be a gradient, got {}",
        stops.len()
    );
    assert!(
        matches!(scales.get(Channel::Fill), Some(Scale::Sequential { .. })),
        "the legend must read the fill scale the dots were painted with"
    );

    assert!(
        band_width(&composed) > 0.0,
        "a legend was derived but no band was reserved for it"
    );
    let doc = laid_out(composed);
    let raster = doc.raster_rect.expect("the raster's rect is recorded");
    let legend = doc.legend_rect.expect("the legend's band is recorded");
    let viewport = doc.viewport.expect("the pane's content box is recorded");
    assert!(
        !legend.intersects(raster),
        "the legend band {legend:?} overlaps the raster {raster:?} — a legend is on the data"
    );
    assert!(
        viewport.contains_rect(legend) && viewport.contains_rect(raster),
        "the legend {legend:?} or raster {raster:?} leaves the content box {viewport:?}"
    );
}

/// **AC2, through the pipeline.** The tile the point map is generated as: a
/// ghost dot with a literal fill, then a dot filled by a number column. The
/// ghost's literal contributes no fill scale and the second layer's column does,
/// so the page's legend is the ramp over the second layer's values.
#[test]
fn a_literal_ghost_layer_leaves_the_second_layers_ramp_and_its_legend() {
    let ghost = "  - mark: dot\n    data: { from: t }\n    x: x\n    y: y\n    fill: \"#aaaaaa\"\n";
    for (order, layers) in [
        ("ghost first", format!("{ghost}{}", dot("v"))),
        ("ghost last", format!("{}{ghost}", dot("v"))),
    ] {
        let composed = compose(&layers);
        let scales = &composed.plots[0].scales;
        let Some(LegendSpec::Sequential { min, max, .. }) = LegendSpec::from_scales(scales) else {
            panic!(
                "{order}: no sequential legend; the fill scale is {:?}",
                scales.get(Channel::Fill)
            );
        };
        assert_eq!(
            (min, max),
            (0.0, 4.9),
            "{order}: the ramp is over the number column's values"
        );
    }
}

/// **AC3, through the pipeline.** A literal fill draws no legend and reserves no
/// band; a string column draws the categorical legend it drew before.
#[test]
fn a_literal_fill_and_a_string_fill_draw_the_legends_they_drew_before() {
    let literal = compose(&dot("\"#aaaaaa\""));
    assert!(
        LegendSpec::from_scales(&literal.plots[0].scales).is_none(),
        "a literal fill drew a legend"
    );
    assert_eq!(band_width(&literal), 0.0, "a literal fill reserved a band");

    let by_string = compose(&dot("g"));
    match LegendSpec::from_scales(&by_string.plots[0].scales) {
        Some(LegendSpec::Categorical { entries }) => {
            let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["g0", "g1"], "the string fill's own categories");
        }
        other => panic!("a string fill must draw a categorical legend, got {other:?}"),
    }
}
