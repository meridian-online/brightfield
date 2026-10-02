//! The chart legend as a native egui **margin panel** — outside the data,
//! always.
//!
//! # The rule this module mechanises
//!
//! A legend never sits inside the plot rect, never obstructs a mark, and never
//! exists twice. The in-scene legend the compose pipeline used to bake into
//! the top-right corner of the plot broke all three at once: it was drawn *on*
//! the data (over whatever marks happened to live in that corner), and the one
//! attempt to supplement it natively — a hardcoded "Series A/B/C" swatch block
//! in the controls rail — was a second legend, fixed at three series, that
//! mislabelled every chart that was not the fixture it was written against.
//!
//! So the pipeline now composes with its inline legend **off**
//! ([`crate::pipeline`] passes `draw_inline_legend = false`), and this module
//! is the only legend there is: one block per chart, derived from the
//! [`ScaleSet`] that chart was *actually drawn against* — the same value, not
//! a re-inference — placed in a reserved band beside the raster. Accuracy is
//! structural: the swatch colours are the palette the marks were painted with,
//! byte for byte, and the labels are the scale's own categories.
//!
//! The band is outside the presented raster by construction, which is what
//! makes "no legend overlaps data" a property a headless test can hold over
//! every example spec rather than a hope about pixel placement.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{ramp_at, Scale, ScaleSet};
use brightfield_spec::edit::colour_legend_covers;
use brightfield_spec::layout::collect_legend_nodes;
use brightfield_spec::vocab::LegendChannel;
use brightfield_spec::Spec;
use meridian_design::{control, semantic, spacing, typography};
use meridian_egui::Mode;

use crate::pipeline::{Composed, PlotHandle};

/// What one chart's legend says: the derivation from its displayed scales,
/// with no egui type in it, so accuracy can be asserted in a unit test.
#[derive(Clone, Debug, PartialEq)]
pub enum LegendSpec {
    /// A categorical colour scale: one swatch per category, in scale order.
    Categorical {
        /// The entries, in the scale's own order.
        entries: Vec<LegendEntry>,
    },
    /// A continuous colour ramp: a gradient bar with its domain ends.
    Sequential {
        /// Domain minimum.
        min: f64,
        /// Domain maximum.
        max: f64,
        /// The ramp's control points, low → high, straight-alpha RGBA.
        stops: Vec<[f32; 4]>,
    },
    /// A colour ramp about a pivot: a gradient bar from one pole through the
    /// midpoint colour to the other, with the pivot named at the middle.
    Diverging {
        /// Domain minimum — the same distance below the pivot as `max` is above.
        min: f64,
        /// Domain maximum.
        max: f64,
        /// The value the middle of the bar stands for.
        pivot: f64,
        /// The ramp's control points, low pole → midpoint → high pole,
        /// straight-alpha RGBA.
        stops: Vec<[f32; 4]>,
    },
}

/// One categorical legend entry: the category and the ink its marks wear.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    /// The category label, exactly as the scale spells it.
    pub label: String,
    /// The mark colour, straight-alpha RGBA — the palette value itself, so
    /// the swatch cannot drift from the raster.
    pub colour: [f32; 4],
}

impl LegendSpec {
    /// The legend `scales` calls for, or `None` when nothing on the plot maps
    /// colour. Reads the **fill** channel — the same trigger the retired
    /// in-scene legend keyed on, so suppressing that one and drawing this one
    /// changes where the legend is, never whether there is one.
    #[must_use]
    pub fn from_scales(scales: &ScaleSet) -> Option<Self> {
        match scales.get(Channel::Fill)? {
            Scale::Colour {
                categories,
                palette,
            } => Some(Self::Categorical {
                entries: categories
                    .iter()
                    .zip(palette.iter())
                    .map(|(label, colour)| LegendEntry {
                        label: label.clone(),
                        colour: *colour,
                    })
                    .collect(),
            }),
            Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            } => Some(Self::Sequential {
                min: *domain_min,
                max: *domain_max,
                stops: stops.clone(),
            }),
            Scale::Diverging {
                domain_min,
                domain_max,
                pivot,
                stops,
            } => Some(Self::Diverging {
                min: *domain_min,
                max: *domain_max,
                pivot: *pivot,
                stops: stops.clone(),
            }),
            _ => None,
        }
    }

    /// The legend `plot` draws: the one its scales call for, when its file
    /// puts a legend on it ([`PlotHandle::legend_declared`]), and `None`
    /// otherwise. [`Self::from_scales`] says what a legend would show; this
    /// says whether the page draws one, and is what the band and its blocks
    /// read.
    #[must_use]
    pub fn of_plot(plot: &PlotHandle) -> Option<Self> {
        if plot.legend_declared {
            Self::from_scales(&plot.scales)
        } else {
            None
        }
    }

    /// The labels this legend shows, in order — the test hook behind
    /// "accurate to the series actually shown".
    #[must_use]
    pub fn labels(&self) -> Vec<&str> {
        match self {
            Self::Categorical { entries } => entries.iter().map(|e| e.label.as_str()).collect(),
            Self::Sequential { .. } | Self::Diverging { .. } => Vec::new(),
        }
    }
}

/// The label column a legend row reserves beside its swatch, in logical
/// points. Declared rather than measured because the *band* has to be a fact
/// before any frame exists — the window is sized around it — and a band sized
/// to its longest label would resize the window every time a category renamed.
/// A label longer than the column truncates; the swatch and the scale order
/// still identify the entry.
pub const LABEL_COLUMN: f32 = 96.0;

/// The width of one legend block: swatch, gap, label column.
#[must_use]
pub fn block_width() -> f32 {
    control::ICON_XS + spacing::ICON_LABEL_GAP + LABEL_COLUMN
}

/// Mark each plot of `plots` that the file puts a colour legend on.
///
/// A plot has one when its own items hold a `legend: color`, or a standalone
/// colour legend names it by `for:` (both read by
/// [`colour_legend_covers`]), or a standalone colour legend with no `for:`
/// stands in a file with exactly one plot whose scales call for a legend — the
/// plot such a legend can only mean. A `for:` that is a `$param` names no plot,
/// as it does for the shelf's writer. The test for "calls for a legend" is the
/// scales' own ([`LegendSpec::from_scales`]), so a legend item over a plot with
/// no colour scale marks nothing and reserves no band.
///
/// Called once by the composition, after every plot is placed: the no-`for:`
/// case counts the plots beside the one it marks.
pub(crate) fn declare_legends(spec: &Spec, plots: &mut [PlotHandle]) {
    let unnamed_standalone = collect_legend_nodes(spec).iter().any(|(_, legend)| {
        legend.channel == LegendChannel::Color && !legend.options.contains_key("for")
    });
    let mut coloured = plots
        .iter()
        .enumerate()
        .filter(|(_, p)| LegendSpec::from_scales(&p.scales).is_some())
        .map(|(i, _)| i);
    let sole = match (coloured.next(), coloured.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    };
    for (i, plot) in plots.iter_mut().enumerate() {
        plot.legend_declared = colour_legend_covers(spec, &plot.path)
            || (unnamed_standalone && sole == Some(i));
    }
}

/// The width the chart pane's legend band consumes, in logical points — `0.0`
/// when the file puts no legend on any plot of `composed` whose scales call
/// for one, which is what keeps a legendless dashboard's window byte-identical
/// to what it was before the band existed, and gives a plot with no legend item
/// the width the band would have held. Includes the gap between the raster and
/// the band.
///
/// Read by [`crate::window::chart_window_size`], so the band is a term of the
/// window arithmetic rather than a bite out of the raster's budget.
#[must_use]
pub fn band_width(composed: &Composed) -> f32 {
    if composed
        .plots
        .iter()
        .any(|p| LegendSpec::of_plot(p).is_some())
    {
        spacing::CONTROL_GAP + block_width()
    } else {
        0.0
    }
}

/// Draw every plot's legend into the reserved band beside the raster.
///
/// `band` is the rect the chart pane reserved — entirely outside the
/// presented raster — and `raster_top` is the raster rect's top in the same
/// (window-space) coordinates, so each block can sit level with the plot it
/// describes: one legend per chart, at its chart's height, scoped to what
/// that chart shows.
pub fn draw_band(
    ui: &egui::Ui,
    band: egui::Rect,
    raster_top: f32,
    composed: &Composed,
    mode: Mode,
) {
    let painter = ui.painter_at(band);
    for plot in &composed.plots {
        let Some(legend) = LegendSpec::of_plot(plot) else {
            continue;
        };
        let y = raster_top + plot.rect.y as f32;
        draw_block(&painter, egui::pos2(band.left(), y), &legend, mode);
    }
}

/// One legend block at `origin`: swatch + label rows for a categorical scale,
/// a gradient bar with its domain ends for a sequential one, and a bar from
/// pole to pole with the pivot at its middle for a diverging one.
fn draw_block(painter: &egui::Painter, origin: egui::Pos2, legend: &LegendSpec, mode: Mode) {
    let sem = semantic(mode.is_dark());
    let ink = crate::design::to_color32(sem.text.secondary);
    let font = egui::FontId::proportional(typography::UI_SIZE);
    let swatch = control::ICON_XS;
    let row = swatch + spacing::SPACE_2;
    match legend {
        LegendSpec::Categorical { entries } => {
            for (i, entry) in entries.iter().enumerate() {
                let top = origin.y + i as f32 * row;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, top),
                    egui::vec2(swatch, swatch),
                );
                painter.rect_filled(rect, 0.0, chart_ink(entry.colour));
                let galley = painter.layout(entry.label.clone(), font.clone(), ink, LABEL_COLUMN);
                painter.galley(
                    egui::pos2(
                        rect.right() + spacing::ICON_LABEL_GAP,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    ink,
                );
            }
        }
        LegendSpec::Sequential { min, max, stops } => {
            // The ramp as adjacent solid strips: visually continuous at strip
            // widths this small, and free of any gradient-mesh dependency.
            let bar = egui::Rect::from_min_size(origin, egui::vec2(block_width(), swatch));
            let n = stops.len().max(2);
            let strip = bar.width() / (n as f32 - 1.0).max(1.0);
            for (i, stop) in stops.iter().enumerate() {
                let left = bar.left() + i as f32 * strip;
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left, bar.top()),
                    egui::pos2((left + strip).min(bar.right()), bar.bottom()),
                );
                painter.rect_filled(rect, 0.0, chart_ink(*stop));
            }
            let label_y = bar.bottom() + spacing::SPACE_1;
            painter.text(
                egui::pos2(bar.left(), label_y),
                egui::Align2::LEFT_TOP,
                format_domain(*min),
                font.clone(),
                ink,
            );
            painter.text(
                egui::pos2(bar.right(), label_y),
                egui::Align2::RIGHT_TOP,
                format_domain(*max),
                font,
                ink,
            );
        }
        LegendSpec::Diverging {
            min,
            max,
            pivot,
            stops,
        } => {
            let bar = egui::Rect::from_min_size(origin, egui::vec2(block_width(), swatch));
            let colours = diverging_strip_colours(stops);
            let strip = bar.width() / colours.len() as f32;
            for (i, colour) in colours.iter().enumerate() {
                let left = bar.left() + i as f32 * strip;
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left, bar.top()),
                    egui::pos2((left + strip).min(bar.right()), bar.bottom()),
                );
                painter.rect_filled(rect, 0.0, chart_ink(*colour));
            }
            let label_y = bar.bottom() + spacing::SPACE_1;
            for (x, align, value) in [
                (bar.left(), egui::Align2::LEFT_TOP, *min),
                (bar.center().x, egui::Align2::CENTER_TOP, *pivot),
                (bar.right(), egui::Align2::RIGHT_TOP, *max),
            ] {
                painter.text(
                    egui::pos2(x, label_y),
                    align,
                    format_domain(value),
                    font.clone(),
                    ink,
                );
            }
        }
    }
}

/// How many strips a diverging legend's bar is drawn in. Odd, so the middle
/// strip is the ramp's midpoint colour and stands at the bar's centre, where the
/// pivot's label is.
const DIVERGING_STRIPS: usize = 61;

/// The colour of each strip of a diverging legend's bar, left (low pole) to
/// right (high pole): strip `i` samples the ramp at `i / (n - 1)`, so the first
/// strip is the low pole's colour, the last is the high pole's, and the middle
/// one is the midpoint colour.
///
/// The sequential bar draws one strip per stop and leaves the last stop's
/// without a width; sampling the ramp instead is what puts both poles on a
/// bar that has to show both.
#[must_use]
pub fn diverging_strip_colours(stops: &[[f32; 4]]) -> Vec<[f32; 4]> {
    (0..DIVERGING_STRIPS)
        .map(|i| ramp_at(stops, i as f64 / (DIVERGING_STRIPS - 1) as f64))
        .collect()
}

/// A domain end, spelled the short way: integers bare, fractions to two
/// places.
fn format_domain(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// A palette colour to egui ink — straight alpha, same quantisation as the
/// chrome's one colour boundary. The value is the **chart's** ink, used raw:
/// remapping it through a semantic token would let the swatch and the raster
/// disagree.
fn chart_ink(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_scale() -> ScaleSet {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["A".into(), "B".into(), "C".into()],
                palette: vec![
                    [0.1, 0.2, 0.3, 1.0],
                    [0.4, 0.5, 0.6, 1.0],
                    [0.7, 0.8, 0.9, 1.0],
                ],
            },
        );
        scales
    }

    /// Accuracy is structural: the legend is the scale's own categories in the
    /// scale's own order, wearing the scale's own palette — not a re-inference
    /// that could drift from what the marks were painted with.
    #[test]
    fn a_categorical_legend_is_the_scale_verbatim() {
        let legend = LegendSpec::from_scales(&colour_scale()).expect("a fill scale has a legend");
        assert_eq!(legend.labels(), vec!["A", "B", "C"]);
        let LegendSpec::Categorical { entries } = legend else {
            panic!("a colour scale derives a categorical legend");
        };
        assert_eq!(entries[1].colour, [0.4, 0.5, 0.6, 1.0]);
    }

    /// No fill scale, no legend — and therefore no band: a legendless
    /// dashboard gives up no width at all.
    #[test]
    fn no_fill_scale_means_no_legend_and_no_band() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 100.0,
            },
        );
        assert_eq!(LegendSpec::from_scales(&scales), None);
        assert_eq!(band_width(&Composed::empty()), 0.0);
    }

    /// A sequential ramp derives the gradient form with its domain ends.
    #[test]
    fn a_sequential_scale_derives_a_ramp_legend() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 2.0,
                domain_max: 9.0,
                stops: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            },
        );
        let legend = LegendSpec::from_scales(&scales).expect("a ramp has a legend");
        assert_eq!(
            legend,
            LegendSpec::Sequential {
                min: 2.0,
                max: 9.0,
                stops: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            }
        );
        assert!(legend.labels().is_empty(), "a ramp has ends, not entries");
    }
    /// **A categorical swatch is drawn square** — the corner is the token's
    /// zero and not a literal of the call site's.
    #[test]
    fn a_categorical_swatch_is_drawn_without_a_corner() {
        let spec = LegendSpec::from_scales(&colour_scale()).expect("a colour scale has a legend");
        let LegendSpec::Categorical { entries } = &spec else {
            panic!("a categorical scale drew {spec:?}");
        };
        let inks: Vec<egui::Color32> = entries.iter().map(|e| chart_ink(e.colour)).collect();
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 200.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let painter = ui.painter().clone();
            draw_block(&painter, egui::pos2(10.0, 10.0), &spec, Mode::Light);
        });
        let mut swatches = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Rect(r) = &clipped.shape {
                if inks.contains(&r.fill) {
                    swatches.push(r.clone());
                }
            }
        }
        assert_eq!(
            swatches.len(),
            inks.len(),
            "one swatch per entry: {swatches:?}"
        );
        for swatch in &swatches {
            assert_eq!(
                swatch.corner_radius,
                egui::CornerRadius::ZERO,
                "the swatch at {:?} carries a corner",
                swatch.rect
            );
        }
    }
}
