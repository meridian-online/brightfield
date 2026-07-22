//! The examples exercise: every chart spec in `examples/*.yaml`, composed and
//! laid out, held to the legend law — **no legend overlaps data, anywhere.**
//!
//! GPU-free on purpose. The law is geometric — the legend band is a rect the
//! chart pane reserves *beside* the raster, outside the plot rect — so it can
//! be held over rects a real layout pass produced, for every example at once,
//! in a test that runs without an adapter. (What the band's pixels look like
//! is the pixel tier's question; whether they can sit on the data is settled
//! here, structurally.)
//!
//! Each composable spec is booted headless at exactly the window
//! `chart_window_size` asks for, two frames run (font atlas + layout settle,
//! as the capture path does), and the recorded rects read back:
//!
//! - the raster rect and the legend rect are **disjoint** — the margin panel
//!   is outside the data by layout, not by hope;
//! - both sit inside the pane's content box — the window budgeted for the
//!   band rather than letting it bite the raster;
//! - a spec whose scales call for no legend reserves **no** band at all.
//!
//! Accuracy rides the same pass: each plot's legend derivation is compared
//! against the scale set that plot was composed with — entries exactly the
//! scale's categories, in the scale's order.

use std::path::PathBuf;

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, LegendSpec};
use brightfield_shell::pipeline::{compose_spec, Composed};
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};

/// Every top-level example spec, by path. `examples/protocol/` and
/// `examples/live/` are other views' fixtures; the chart pipeline's corpus is
/// the flat files.
fn example_specs() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut specs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("examples/ exists")
        .filter_map(|entry| {
            let path = entry.expect("readable dir entry").path();
            (path.extension().is_some_and(|e| e == "yaml")).then_some(path)
        })
        .collect();
    specs.sort();
    assert!(
        specs.len() >= 20,
        "the examples corpus shrank to {} specs — is the path right?",
        specs.len()
    );
    specs
}

/// Compose every example that composes, with its name.
fn composed_examples() -> Vec<(String, Composed)> {
    example_specs()
        .into_iter()
        .filter_map(|path| {
            let name = path
                .file_name()
                .expect("a file has a name")
                .to_string_lossy()
                .into_owned();
            match compose_spec(path.to_str().expect("utf-8 path")) {
                Ok(composed) => Some((name, composed)),
                Err(e) => {
                    // Not silently: a spec beyond the ported compose scope is
                    // stated, so a regression that breaks a previously-good
                    // spec at least changes this output. The count floor
                    // below is the hard gate.
                    eprintln!("note: {name} does not compose here: {e}");
                    None
                }
            }
        })
        .collect()
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
    // `MeridianApp` owns the document; carry the recorded rects out on a
    // fresh headless one, which is all the assertions read.
    let doc = app.chart_doc();
    let mut out = ChartDoc::headless(Composed::empty());
    out.viewport = doc.viewport;
    out.raster_rect = doc.raster_rect;
    out.legend_rect = doc.legend_rect;
    out
}

/// The law, over the whole corpus.
#[test]
fn no_legend_overlaps_data_in_any_example() {
    let composed = composed_examples();
    assert!(
        composed.len() >= 15,
        "only {} example specs composed — the corpus gate lost its teeth",
        composed.len()
    );

    let mut with_legend = 0usize;
    for (name, composed) in composed {
        let banded = band_width(&composed) > 0.0;
        let doc = laid_out(composed);
        let raster = doc.raster_rect.expect("recorded");
        let viewport = doc.viewport.expect("recorded");

        match doc.legend_rect {
            Some(legend) => {
                assert!(banded, "{name}: a band was drawn that no scale called for");
                with_legend += 1;
                assert!(
                    !legend.intersects(raster),
                    "{name}: the legend band {legend:?} overlaps the raster \
                     {raster:?} — a legend is sitting on the data"
                );
                assert!(
                    viewport.contains_rect(legend),
                    "{name}: the legend band {legend:?} leaves the pane's \
                     content box {viewport:?} — the window did not budget it"
                );
            }
            None => {
                assert!(
                    !banded,
                    "{name}: scales call for a legend but no band was reserved"
                );
            }
        }
        assert!(
            viewport.contains_rect(raster),
            "{name}: the raster {raster:?} leaves the content box {viewport:?}"
        );
    }
    assert!(
        with_legend >= 3,
        "only {with_legend} examples produced a margin legend — the law was \
         held over almost nothing"
    );
}

/// Accuracy, per plot: the margin legend is the displayed scale, verbatim —
/// same categories, same order — and a plot with no colour scale derives no
/// legend for a phantom band to draw.
#[test]
fn every_margin_legend_is_accurate_to_its_plots_displayed_scale() {
    let mut categorical = 0usize;
    for (name, composed) in composed_examples() {
        for (i, plot) in composed.plots.iter().enumerate() {
            match (
                LegendSpec::from_scales(&plot.scales),
                plot.scales.get(Channel::Fill),
            ) {
                (
                    Some(LegendSpec::Categorical { entries }),
                    Some(Scale::Colour {
                        categories,
                        palette,
                    }),
                ) => {
                    categorical += 1;
                    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
                    let expect: Vec<&str> = categories.iter().map(String::as_str).collect();
                    assert_eq!(
                        labels, expect,
                        "{name} plot {i}: the legend says other than its scale"
                    );
                    for (entry, colour) in entries.iter().zip(palette.iter()) {
                        assert_eq!(
                            entry.colour, *colour,
                            "{name} plot {i}: a swatch drifted from the palette"
                        );
                    }
                }
                (Some(LegendSpec::Sequential { .. }), Some(Scale::Sequential { .. })) => {}
                (None, None) => {}
                (None, Some(Scale::Colour { .. } | Scale::Sequential { .. })) => {
                    panic!("{name} plot {i}: a colour scale derived no legend")
                }
                (legend, scale) => {
                    panic!("{name} plot {i}: legend {legend:?} does not follow scale {scale:?}")
                }
            }
        }
    }
    assert!(
        categorical >= 2,
        "only {categorical} categorical legends were checked — the accuracy \
         law was held over almost nothing"
    );
}
