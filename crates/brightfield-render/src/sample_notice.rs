//! Saying, in the plot's own ink, that the picture is a sample.
//!
//! # Why this is not a banner
//!
//! A sampled scatter is still a picture, and people screenshot pictures
//! without the caption. Anything the shell draws — a status strip, a toast,
//! the margin legend — is absent from `capture_vello_only` **by construction**:
//! that export rasterises the composed Vello scene and never builds an egui
//! context at all. A notice that vanishes when the chart is exported or cropped
//! is not a notice. So this draws into the scene, next to the marks.
//!
//! # Why a hatched band, and not the alternatives
//!
//! The requirement is a signal that reads **without reading text**, per plot
//! (a dashboard can hold one sampled and one complete plot, and a page-level
//! device would lie about the complete one).
//!
//! - *Not a mark treatment.* Dimming or shrinking the marks was the obvious
//!   move and it is the wrong one: mark alpha is owned by crossfilter
//!   highlighting, so a view that was both sampled and brushed would carry two
//!   independent dims and neither would read as either.
//! - *Not status colour.* The design system's hard rule is that status colours
//!   are never colour-alone — always icon plus label — and the criterion here
//!   asks for exactly a sole non-text signal. So this uses the neutral
//!   chart-furniture ink (the same token the axes and baselines wear) and makes
//!   the difference a matter of FORM.
//! - *A hatch, because nothing else here is hatched.* This renderer contains no
//!   gradient and no image brush; every fill is flat. A band of repeating
//!   diagonal rule is therefore unmistakably not data and unlike anything else
//!   on the canvas, at any zoom, in either theme, and in greyscale.
//!
//! It is drawn as a reserved band below the plot, exactly the way the axis
//! titles reserve theirs — [`sample_band_margins`] grows the bottom margin by
//! [`NOTICE_BAND`] when a plot is sampled. Taking the device away later
//! restores the previous geometry and re-baselines only sampled fixtures.

use kurbo::{Affine, BezPath, Line, Rect, Stroke};
use peniko::{Color, Fill};
use vello::Scene;

use crate::ink::{ink, ink_with_alpha};
use crate::layout::{ChartLayout, Margins};
use crate::text::{draw_text, measure_width, TextAnchor, LABEL_SIZE};

/// What a sampled plot knows about its own sample.
///
/// `Copy` on purpose: it rides on [`ChartData`](crate::scene::ChartData) and on
/// the shell's plot handle, both of which are cheap value types, and it must
/// survive a crossfilter rebuild without anyone remembering to clone it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleFact {
    /// Rows the plot actually drew.
    pub drawn: u64,
    /// Rows the same query would have returned unsampled. Counted by an
    /// unsampled query, not inferred by multiplying `drawn` by the modulus —
    /// a hash sample is not a perfectly uniform partition and the inferred
    /// figure would be a guess presented as a count.
    pub of: u64,
}

/// The band reserved below the plot for the notice, in logical pixels.
pub const NOTICE_BAND: f64 = 22.0;

/// Spacing between hatch rules, in logical pixels.
const HATCH_PITCH: f64 = 6.0;

/// Hatch rule width.
const HATCH_WIDTH: f64 = 1.4;

/// The neutral ink the hatch and the label wear.
///
/// `ink_muted` rather than the `baseline` the axes use: the axis tokens are
/// deliberately quiet because an axis is scaffolding you look past, and a
/// notice you look past is not a notice. It is still chart furniture — no
/// status hue, nothing from the series palette — so the difference the eye
/// picks up is the FORM, per the design system's rule that a state is never
/// signalled by colour alone.
fn furniture_ink() -> Color {
    ink(meridian_design::chrome::INK_LIGHT.ink_muted)
}

/// The label ink — the same weight the axis titles carry, so the words are
/// read rather than deciphered.
fn label_ink() -> Color {
    ink(meridian_design::chrome::INK_LIGHT.ink_secondary)
}

/// A surface plate behind the label so it stays legible over the hatch, at the
/// same alpha the inline legend panel uses.
fn plate_ink() -> Color {
    ink_with_alpha(meridian_design::chrome::INK_LIGHT.surface, 0.92)
}

/// Grow `margins` to reserve the notice band when the plot is sampled.
///
/// The one place the geometry and the drawing agree. Callers apply this
/// wherever they apply [`grow_margins`](crate::title::grow_margins) — if a
/// caller forgets, the band draws over the x-axis title rather than beside it,
/// which is why both live next to each other here rather than being derived
/// twice.
#[must_use]
pub fn sample_band_margins(margins: Margins, sampled: bool) -> Margins {
    if sampled {
        Margins {
            bottom: margins.bottom + NOTICE_BAND,
            ..margins
        }
    } else {
        margins
    }
}

/// Draw the notice into `scene` for a plot whose layout already reserves the
/// band (see [`sample_band_margins`]).
///
/// The hatch spans the full plot width, because the thing being marked is the
/// whole picture, not a corner of it.
pub fn render_sample_notice(scene: &mut Scene, layout: &ChartLayout, fact: SampleFact) {
    let top = layout.height - NOTICE_BAND;
    let band = Rect::new(
        layout.margins.left,
        top,
        layout.width - layout.margins.right,
        layout.height,
    );
    if band.width() <= 0.0 || band.height() <= 0.0 {
        return;
    }

    // Diagonal rules, clipped to the band. Stepping x from far enough left that
    // every 45° line crossing the band is emitted.
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &band);
    let colour = furniture_ink();
    let stroke = Stroke::new(HATCH_WIDTH);
    let mut x = band.x0 - band.height();
    while x < band.x1 {
        let line = Line::new((x, band.y1), (x + band.height(), band.y0));
        scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
        x += HATCH_PITCH;
    }
    // A hairline along the top of the band, so the hatch reads as a bounded
    // strip belonging to this plot rather than as bleed from the page.
    let mut edge = BezPath::new();
    edge.move_to((band.x0, band.y0));
    edge.line_to((band.x1, band.y0));
    scene.stroke(&stroke, Affine::IDENTITY, colour, None, &edge);
    scene.pop_layer();

    // The words, on a plate so they survive the hatch behind them. The hatch is
    // the signal; this is the detail someone reads once they have noticed it.
    let label = notice_label(fact);
    let plate_w = measure_width(&label, LABEL_SIZE) + 10.0;
    let plate = Rect::new(
        band.x0 + 2.0,
        band.y0 + 3.0,
        (band.x0 + 2.0 + plate_w).min(band.x1),
        band.y1 - 3.0,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, plate_ink(), None, &plate);
    draw_text(
        scene,
        &label,
        band.x0 + 7.0,
        band.y1 - 7.0,
        LABEL_SIZE,
        label_ink(),
        TextAnchor::Start,
    );
}

/// The words in the band. Plain, and never a percentage — "0.4 % of the data"
/// invites reading the picture as if the missing 99.6 % were merely small.
#[must_use]
pub fn notice_label(fact: SampleFact) -> String {
    format!(
        "SAMPLED — {} of {} rows drawn",
        thousands(fact.drawn),
        thousands(fact.of)
    )
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mark::count_scene_paths;

    #[test]
    fn band_is_reserved_only_when_sampled() {
        let base = Margins::default();
        assert_eq!(sample_band_margins(base, false).bottom, base.bottom);
        assert_eq!(
            sample_band_margins(base, true).bottom,
            base.bottom + NOTICE_BAND
        );
        // Only the bottom moves — the device is removable without disturbing
        // any other side's geometry.
        let grown = sample_band_margins(base, true);
        assert_eq!(
            (grown.top, grown.left, grown.right),
            (base.top, base.left, base.right)
        );
    }

    /// The device is many strokes, not one — which is what makes it read as a
    /// texture rather than as another rule on the chart.
    #[test]
    fn notice_draws_a_hatch_of_many_rules() {
        let layout =
            ChartLayout::with_margins(400.0, 300.0, sample_band_margins(Margins::default(), true));
        let mut scene = Scene::new();
        render_sample_notice(&mut scene, &layout, SampleFact { drawn: 10, of: 100 });
        let paths = count_scene_paths(&scene);
        assert!(
            paths > 20,
            "expected a hatch of many rules plus the plate, got {paths} paths"
        );
    }

    #[test]
    fn label_counts_rows_and_never_a_percentage() {
        let s = notice_label(SampleFact {
            drawn: 4_688,
            of: 75_000,
        });
        assert_eq!(s, "SAMPLED — 4,688 of 75,000 rows drawn");
        assert!(!s.contains('%'));
    }
}
