//! Saying, in the plot's own ink, which range the reader has selected.
//!
//! # What this is for
//!
//! A cross-filter has two halves and brightfield drew one of them. The
//! *receiving* plot narrows, visibly; the plot the gesture happened on shows
//! the range it is imposing for as long as the pointer is down and nothing
//! afterwards. So the filter is in force, the page has changed, and the surface
//! does not say what the filter is — a reader cannot check it or trust it
//! without redoing the drag.
//!
//! This is the drawn representation of the *committed* selection: the band the
//! plot's own gesture is holding, mapped back through the same displayed scales
//! the gesture inverted through.
//!
//! # Why it is in the Vello scene rather than in the shell overlay
//!
//! Two reasons, and the second is the load-bearing one.
//!
//! Anything the shell draws is absent from `capture_vello_only` by
//! construction: that export rasterises the composed scene and never builds an
//! egui context. Drawing here means the band travels with the chart through the
//! ordinary export path — the argument [`crate::sample_notice`] already makes.
//!
//! And it is what separates this from the transient brush rectangle. That
//! rectangle is an egui quad over the raster, painted from the drag state and
//! gone the frame the button comes up; this is chart ink, laid down by the
//! composition the gesture produced. They are told apart at a glance by hue:
//! the overlay's wash is the design system's neutral `brush_fill`, and this is
//! the chart's own focus ink.
//!
//! # The treatment
//!
//! A low-alpha wash over the selected region, plus a bound rule down each
//! constrained edge in the same ink at full strength. The wash says *this
//! region*; the rules say *these two values*, which is the part a reader checks
//! against the axis.
//!
//! Only the constrained edges are ruled. An `intervalX` selection spans the
//! plot's full height, so ruling its top and bottom would draw two lines that
//! are a property of the plot rather than of the selection.

use kurbo::{Affine, Line, Rect, Stroke};
use peniko::Fill;
use vello::Scene;

use crate::channel::Channel;
use crate::layout::ChartLayout;
use crate::scale::{Scale, ScaleSet};

/// The wash over the selected region.
///
/// Low enough that marks inside the band keep their own colour — the band
/// reports a filter and must not restate the data's palette.
///
/// The alpha stays here, with its reasoning; the ink it is applied to is the
/// mode's focus token and lives on [`crate::ink::ChartInk::selection_wash`].
pub(crate) const WASH_ALPHA: f32 = 0.14;

/// The bound rules' width in pixels.
///
/// Two, not one. A rule this wide covers at least one whole pixel column
/// wherever its centre falls between pixels, so the bound reaches the raster at
/// full strength rather than as a pair of half-covered columns — which is what
/// lets a test find it in the picture at all.
const BOUND_WIDTH: f64 = 2.0;

/// What a plot's own gesture is holding, per positional channel, in the data
/// units the channel's scale reads.
///
/// Data units rather than pixels on purpose: the clause is what the engine
/// holds, and mapping it forward through the *displayed* scales here is the
/// same round trip the gesture made in reverse. A band carried in pixels would
/// go stale the moment the plot was panned, zoomed or re-laid-out, and would
/// say so nowhere.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommittedSelection {
    /// What the x channel is constrained to, if anything.
    pub x: Option<Selected>,
    /// What the y channel is constrained to, if anything.
    pub y: Option<Selected>,
}

impl CommittedSelection {
    /// Whether this holds any constraint at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.y.is_none()
    }
}

/// One channel's constraint.
#[derive(Clone, Debug, PartialEq)]
pub enum Selected {
    /// Inclusive numeric bounds in the channel's own data units — microseconds
    /// for a time scale, matching [`Scale::inverse_f64`]'s return.
    Interval(f64, f64),
    /// Categories picked off a band scale.
    Categories(Vec<String>),
}

/// One channel's constraint as pixel spans on that axis, resolved through the
/// plot's displayed scale. Empty when the constraint cannot be placed — a
/// numeric interval against a band scale, a category the scale does not carry.
fn spans(scale: &Scale, selected: &Selected) -> Vec<(f64, f64)> {
    match selected {
        Selected::Interval(lo, hi) => match scale {
            Scale::Linear { .. } | Scale::Time { .. } => {
                let (a, b) = (scale.map_f64(*lo), scale.map_f64(*hi));
                vec![(a.min(b), a.max(b))]
            }
            Scale::Band { .. } | Scale::Colour { .. } | Scale::Sequential { .. } => Vec::new(),
        },
        Selected::Categories(names) => {
            let Some(width) = scale.band_width() else {
                return Vec::new();
            };
            names
                .iter()
                .filter_map(|name| {
                    let centre = scale.map_category(name)?;
                    Some((centre - width / 2.0, centre + width / 2.0))
                })
                .collect()
        }
    }
}

/// The committed selection's own rectangle on this plot, **plot-local** — the
/// same frame [`ChartLayout::plot_x_start`] answers in, and the box
/// [`render_committed_selection`] washes.
///
/// `None` when the selection is empty, when a constrained channel has no
/// scale, or when a constraint cannot be placed on the scale it names. And
/// `None` for a channel constrained by [`Selected::Categories`]: a set of
/// discontiguous bands is not a rectangle to hit-test or drag, and the caller
/// this exists for — the shell's move gesture — asks about a plot whose own
/// binding is an interval, which is what keeps that variant off this path.
#[must_use]
pub fn committed_selection_rect(
    layout: &ChartLayout,
    scales: &ScaleSet,
    selection: &CommittedSelection,
) -> Option<Rect> {
    if selection.is_empty() {
        return None;
    }
    let (px0, px1) = (layout.plot_x_start(), layout.plot_x_end());
    let (py0, py1) = (layout.plot_y_start(), layout.plot_y_end());
    if px1 <= px0 || py1 <= py0 {
        return None;
    }
    let axis_span =
        |channel: Channel, selected: &Option<Selected>, whole: (f64, f64)| match selected {
            None => Some(whole),
            Some(Selected::Interval(lo, hi)) => {
                let scale = scales.get(channel)?;
                let (a, b) = (scale.map_f64(*lo), scale.map_f64(*hi));
                Some((a.min(b), a.max(b)))
            }
            Some(Selected::Categories(_)) => None,
        };
    let (x0, x1) = axis_span(Channel::X, &selection.x, (px0, px1))?;
    let (y0, y1) = axis_span(Channel::Y, &selection.y, (py0, py1))?;
    Some(Rect::new(x0, y0, x1, y1))
}

/// Draw the committed selection `selection` holds on this plot.
///
/// A no-op when nothing is held, when the constrained channel has no scale, or
/// when the constraint cannot be placed on the scale it names — a band that
/// cannot be resolved is not drawn at a guessed position.
pub fn render_committed_selection(
    scene: &mut Scene,
    layout: &ChartLayout,
    scales: &ScaleSet,
    selection: &CommittedSelection,
) {
    if selection.is_empty() {
        return;
    }
    let (px0, px1) = (layout.plot_x_start(), layout.plot_x_end());
    let (py0, py1) = (layout.plot_y_start(), layout.plot_y_end());
    if px1 <= px0 || py1 <= py0 {
        return;
    }

    let axis = |channel: Channel, selected: &Option<Selected>, whole: (f64, f64)| match selected {
        None => Some(vec![whole]),
        Some(sel) => {
            let scale = scales.get(channel)?;
            let placed = spans(scale, sel);
            (!placed.is_empty()).then_some(placed)
        }
    };
    let Some(xs) = axis(Channel::X, &selection.x, (px0, px1)) else {
        return;
    };
    let Some(ys) = axis(Channel::Y, &selection.y, (py0, py1)) else {
        return;
    };

    // Clipped to the plot area: a selection made before a pan can name a range
    // that is now partly off-frame, and the band must stop where the frame
    // does rather than run out over the axes.
    let clip = Rect::new(px0, py0, px1, py1);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    let stroke = Stroke::new(BOUND_WIDTH);
    // The plot's own focus ink, for the mode the scale set was resolved in —
    // the same question `chrome::tone_colour` asks for `Tone::Accent`.
    let ink = scales.ink();
    for &(x0, x1) in &xs {
        for &(y0, y1) in &ys {
            let band = Rect::new(x0, y0, x1, y1);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                ink.selection_wash,
                None,
                &band,
            );
            if selection.x.is_some() {
                for x in [x0, x1] {
                    let rule = Line::new((x, y0), (x, y1));
                    scene.stroke(&stroke, Affine::IDENTITY, ink.selection_bound, None, &rule);
                }
            }
            if selection.y.is_some() {
                for y in [y0, y1] {
                    let rule = Line::new((x0, y), (x1, y));
                    scene.stroke(&stroke, Affine::IDENTITY, ink.selection_bound, None, &rule);
                }
            }
        }
    }
    scene.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(domain: (f64, f64), range: (f64, f64)) -> Scale {
        Scale::Linear {
            domain_min: domain.0,
            domain_max: domain.1,
            range_start: range.0,
            range_end: range.1,
        }
    }

    fn band(categories: &[&str], range: (f64, f64)) -> Scale {
        Scale::Band {
            categories: categories.iter().map(|s| (*s).to_string()).collect(),
            range_start: range.0,
            range_end: range.1,
            padding: 0.1,
        }
    }

    /// The band is the gesture's own round trip: a clause inverted off pixels
    /// maps forward to the pixels it came from.
    #[test]
    fn an_interval_maps_back_to_the_pixels_that_produced_it() {
        let scale = linear((0.0, 10.0), (40.0, 340.0));
        let (a, b) = (100.0, 220.0);
        let (lo, hi) = (
            scale.inverse_f64(a).expect("continuous"),
            scale.inverse_f64(b).expect("continuous"),
        );
        let placed = spans(&scale, &Selected::Interval(lo, hi));
        assert_eq!(placed.len(), 1, "one interval, one span");
        assert!((placed[0].0 - a).abs() < 1e-9, "low bound returns");
        assert!((placed[0].1 - b).abs() < 1e-9, "high bound returns");
    }

    /// A numeric interval against a categorical scale places nowhere, and the
    /// answer is no span rather than a guessed one.
    #[test]
    fn an_interval_on_a_band_scale_places_nothing() {
        let scale = band(&["north", "south"], (40.0, 340.0));
        assert!(spans(&scale, &Selected::Interval(0.0, 1.0)).is_empty());
    }

    /// Each selected category takes its own slot, and a category the scale
    /// does not carry contributes none.
    #[test]
    fn categories_take_their_own_band_slots() {
        let scale = band(&["north", "south", "east"], (40.0, 340.0));
        let width = scale.band_width().expect("a band scale has a slot width");
        let placed = spans(
            &scale,
            &Selected::Categories(vec![
                "south".to_string(),
                "west".to_string(),
                "east".to_string(),
            ]),
        );
        assert_eq!(
            placed.len(),
            2,
            "two of the three name a slot on this scale"
        );
        for (i, name) in ["south", "east"].iter().enumerate() {
            let centre = scale.map_category(name).expect("carried category");
            assert!((placed[i].0 - (centre - width / 2.0)).abs() < 1e-9);
            assert!((placed[i].1 - (centre + width / 2.0)).abs() < 1e-9);
        }
    }

    /// An unconstrained channel spans the plot: an x-only selection is a
    /// full-height band, which is what an `intervalX` brush means.
    #[test]
    fn an_unconstrained_channel_spans_the_plot() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 10.0), (40.0, 340.0)));
        let mut scene = Scene::new();
        render_committed_selection(
            &mut scene,
            &ChartLayout::new(360.0, 300.0),
            &scales,
            &CommittedSelection {
                x: Some(Selected::Interval(2.0, 4.0)),
                y: None,
            },
        );
        assert!(
            !scene.encoding().is_empty(),
            "a placeable selection lays down geometry"
        );
    }

    /// Nothing held draws nothing — the rest state is no ink, not a band over
    /// the whole plot.
    #[test]
    fn an_empty_selection_draws_nothing() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 10.0), (40.0, 340.0)));
        let mut scene = Scene::new();
        render_committed_selection(
            &mut scene,
            &ChartLayout::new(360.0, 300.0),
            &scales,
            &CommittedSelection::default(),
        );
        assert!(scene.encoding().is_empty(), "no constraint, no geometry");
    }

    /// A constraint on a channel the plot has no scale for draws nothing —
    /// including the wash, which would otherwise mark a region no bound was
    /// resolved for.
    #[test]
    fn a_constraint_with_no_scale_draws_nothing() {
        let mut scene = Scene::new();
        render_committed_selection(
            &mut scene,
            &ChartLayout::new(360.0, 300.0),
            &ScaleSet::new(),
            &CommittedSelection {
                x: Some(Selected::Interval(2.0, 4.0)),
                y: None,
            },
        );
        assert!(scene.encoding().is_empty(), "no scale, no geometry");
    }
}
