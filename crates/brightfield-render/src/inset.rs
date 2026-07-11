//! Axis inset resolution (card 0008, axis-inset round).
//!
//! Pulls positional scale ranges inward by a small pixel inset so an edge mark
//! (e.g. a dot sitting exactly at a domain bound) renders its whole disc inside
//! the frame clip instead of clipping to a sliver. Insets are RANGE-side only —
//! domains and the frame clip are untouched.
//!
//! Resolution composes two layers, most-specific-wins:
//! 1. **Explicit** per-side attributes already resolved by
//!    [`brightfield_spec::layout::resolve_plot_insets`] (`inset`, `xInset`,
//!    `yInset`, `xInsetLeft`/`xInsetRight`/`yInsetTop`/`yInsetBottom`).
//! 2. **Default** [`DEFAULT_SCALE_INSET`] applied to each continuous (linear /
//!    time) positional scale end, EXCEPT the end pinned to zero by a renderer
//!    that declares a [`zero_baseline_channel`](crate::mark::MarkRenderer::zero_baseline_channel)
//!    — bars and areas stay flush on their baseline. Band axes get no default;
//!    an explicit `Some(0.0)` is the Mosaic-exact opt-out.
//!
//! The final four pixels are stamped into both [`ChartLayout`](crate::layout::ChartLayout)
//! models (render + ui) so scale ranges, brush inversion, and hit-testing all
//! agree.

use arrow::record_batch::RecordBatch;

use brightfield_spec::layout::SideInsets;

use crate::channel::{Channel, ChannelMap};
use crate::layout::Insets;
use crate::mark::MarkRenderer;
use crate::scale::{infer_scales_multi, positional_axis_class, AxisClass};

/// Default pixel inset on each continuous positional scale end. `DOT_RADIUS`
/// (4) + 1px breathing. Mosaic/Plot default is 0 (Plot does not clip marks);
/// Brightfield clips to the frame, so a nonzero default keeps edge data whole.
/// Explicit spec insets (including an explicit `0`) override — see
/// [`brightfield_spec::layout::resolve_plot_insets`].
pub const DEFAULT_SCALE_INSET: f64 = 5.0;

/// Which pixel end of an axis range a domain value maps to. For x, the range is
/// `(left = Start, right = End)`; for the inverted y range it is
/// `(bottom = Start, top = End)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeEnd {
    Start,
    End,
}

/// One mark's inputs for inset resolution: its data, channel bindings, and
/// renderer (queried for [`zero_baseline_channel`](MarkRenderer::zero_baseline_channel)).
pub type MarkInsetEntry<'a> = (&'a RecordBatch, &'a ChannelMap, &'a dyn MarkRenderer);

/// Resolve the four per-side pixel insets for a plot from its explicit
/// attributes and its marks. See the module docs for the composition rule.
#[must_use]
pub fn resolve_insets_for_marks(
    explicit: SideInsets,
    entries: &[MarkInsetEntry<'_>],
    default: f64,
) -> Insets {
    let pairs: Vec<(&RecordBatch, &ChannelMap)> =
        entries.iter().map(|(b, c, _)| (*b, *c)).collect();
    let x_class = positional_axis_class(&pairs, Channel::X);
    let y_class = positional_axis_class(&pairs, Channel::Y);
    let x_zero = zero_pinned_end(entries, Channel::X);
    let y_zero = zero_pinned_end(entries, Channel::Y);
    overlay(explicit, x_class, y_class, x_zero, y_zero, default)
}

/// Pure composition of the explicit insets over the per-axis defaults. Split
/// out so the default matrix (continuous/band × default/explicit/zero-anchored
/// × single/multi-mark) is exhaustively unit-testable without Arrow batches or
/// renderers.
fn overlay(
    explicit: SideInsets,
    x_class: Option<AxisClass>,
    y_class: Option<AxisClass>,
    x_zero: Option<RangeEnd>,
    y_zero: Option<RangeEnd>,
    default: f64,
) -> Insets {
    Insets {
        // x range: left = Start, right = End.
        left: explicit
            .left
            .unwrap_or_else(|| axis_default(x_class, x_zero, RangeEnd::Start, default)),
        right: explicit
            .right
            .unwrap_or_else(|| axis_default(x_class, x_zero, RangeEnd::End, default)),
        // y range (inverted): bottom = Start, top = End.
        top: explicit
            .top
            .unwrap_or_else(|| axis_default(y_class, y_zero, RangeEnd::End, default)),
        bottom: explicit
            .bottom
            .unwrap_or_else(|| axis_default(y_class, y_zero, RangeEnd::Start, default)),
    }
}

/// The default inset for one end of one axis: [`DEFAULT_SCALE_INSET`] iff the
/// axis is continuous and this end is not the zero-baseline-pinned end;
/// otherwise 0 (band ends, the flush baseline end, and augment-only/absent
/// axes get no default).
fn axis_default(
    class: Option<AxisClass>,
    zero_end: Option<RangeEnd>,
    end: RangeEnd,
    default: f64,
) -> f64 {
    if class == Some(AxisClass::Continuous) && zero_end != Some(end) {
        default
    } else {
        0.0
    }
}

/// If any mark declares `axis` as its zero-baseline channel, work out which
/// pixel end the zero baseline lands on (mirroring `extend_domain_to_zero`), so
/// the default inset can exempt it and the baseline stays flush. Returns `None`
/// when no mark baselines this axis, or when the composed domain straddles zero
/// (0 is interior — both ends still inset).
///
/// The pinned end is decided from the axis domain **composed exactly as the
/// scene composes it** — `infer_scales_multi` (base columns + literals) plus each
/// mark's `augment_scales` (a ranged mark's x1/x2 · y1/y2 extent, etc.). A plain
/// re-scan of the base `axis` column would miss a co-mark's literal or ranged
/// contribution and could wrongly exempt an end whose composed domain actually
/// crosses zero, re-clipping the co-mark. Range is irrelevant to the domain, so
/// a benign one is passed; the guard keeps this off the hot path (only bar /
/// area / rect plots declare a zero baseline, and their data is small).
fn zero_pinned_end(entries: &[MarkInsetEntry<'_>], axis: Channel) -> Option<RangeEnd> {
    let baselined = entries
        .iter()
        .any(|(_, _, r)| r.zero_baseline_channel() == Some(axis));
    if !baselined {
        return None;
    }
    let pairs: Vec<(&RecordBatch, &ChannelMap)> =
        entries.iter().map(|(b, c, _)| (*b, *c)).collect();
    let mut scales = infer_scales_multi(&pairs, (0.0, 1.0), (0.0, 1.0));
    for (batch, cm, renderer) in entries {
        renderer.augment_scales(&mut scales, batch, cm, (0.0, 1.0), (0.0, 1.0));
    }
    let scale = scales.get(axis)?;
    let (min, max) = (scale.domain_min()?, scale.domain_max()?);
    // extend_domain_to_zero: domain_min = min(min, 0), domain_max = max(max, 0).
    // 0 lands on the range Start when the composed domain is all-nonnegative
    // (domain_min becomes 0), on the range End when all-nonpositive, and is
    // interior (no exemption) when the domain straddles zero.
    if min >= 0.0 {
        Some(RangeEnd::Start)
    } else if max <= 0.0 {
        Some(RangeEnd::End)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: f64 = DEFAULT_SCALE_INSET;

    fn none() -> SideInsets {
        SideInsets::default()
    }

    // --- axi_ac02: default overlay matrix (pure `overlay`) ---

    #[test]
    fn axi_ac02_continuous_both_axes_inset_all_four_ends() {
        // A scatter: both axes continuous, no zero baseline → 5px all round.
        let got = overlay(
            none(),
            Some(AxisClass::Continuous),
            Some(AxisClass::Continuous),
            None,
            None,
            D,
        );
        assert_eq!(
            got,
            Insets {
                left: D,
                right: D,
                top: D,
                bottom: D
            }
        );
    }

    #[test]
    fn axi_ac02_band_axis_gets_no_default() {
        // Bars: x band (no default), y continuous with a zero baseline at the
        // range start (bottom) → only the top (non-zero continuous end) insets.
        let got = overlay(
            none(),
            Some(AxisClass::Band),
            Some(AxisClass::Continuous),
            None,
            Some(RangeEnd::Start),
            D,
        );
        assert_eq!(
            got,
            Insets {
                left: 0.0,
                right: 0.0,
                top: D,
                bottom: 0.0
            }
        );
    }

    #[test]
    fn axi_ac02_zero_baseline_exempts_only_its_end() {
        // areaY: y continuous, baseline pinned at range start (bottom). Bottom
        // stays flush; top insets. x continuous → both x ends inset.
        let got = overlay(
            none(),
            Some(AxisClass::Continuous),
            Some(AxisClass::Continuous),
            None,
            Some(RangeEnd::Start),
            D,
        );
        assert_eq!(
            got,
            Insets {
                left: D,
                right: D,
                top: D,
                bottom: 0.0
            }
        );
    }

    #[test]
    fn axi_ac02_zero_baseline_at_end_exempts_top() {
        // All-nonpositive value axis: 0 pins to the range End (top) → top flush.
        let got = overlay(
            none(),
            None,
            Some(AxisClass::Continuous),
            None,
            Some(RangeEnd::End),
            D,
        );
        assert_eq!(got.top, 0.0);
        assert_eq!(got.bottom, D);
    }

    #[test]
    fn axi_ac02_absent_axis_no_default() {
        // Augment-only / absent axis (None class) → no default either end.
        let got = overlay(none(), None, None, None, None, D);
        assert_eq!(got, Insets::default());
    }

    #[test]
    fn axi_ac02_explicit_overrides_default_including_zero() {
        // Explicit values win over the default; an explicit 0 forces flush even
        // on a continuous non-baseline end (Mosaic-exact opt-out).
        let explicit = SideInsets {
            left: Some(12.0),
            right: Some(0.0),
            top: None,
            bottom: Some(3.0),
        };
        let got = overlay(
            explicit,
            Some(AxisClass::Continuous),
            Some(AxisClass::Continuous),
            None,
            None,
            D,
        );
        assert_eq!(
            got,
            Insets {
                left: 12.0,  // explicit
                right: 0.0,  // explicit 0 opt-out
                top: D,      // absent → default
                bottom: 3.0, // explicit
            }
        );
    }

    #[test]
    fn axi_ac02_explicit_applies_to_band_axis() {
        // Band gets no DEFAULT, but an explicit inset still applies.
        let explicit = SideInsets {
            left: Some(4.0),
            right: None,
            top: None,
            bottom: None,
        };
        let got = overlay(explicit, Some(AxisClass::Band), None, None, None, D);
        assert_eq!(got.left, 4.0);
        assert_eq!(got.right, 0.0); // band, no explicit → no default
    }

    // --- axi_ac02 wiring + axi_ac05: end-to-end over real fixtures ---

    mod fixtures {
        use super::*;
        use crate::channel::{Channel, ChannelMap};
        use crate::layout::ChartLayout;
        use crate::mark::{BarRenderer, DotRenderer};
        use crate::scale::Scale;
        use arrow::array::{Float64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc;

        /// A continuous scatter fixture: numeric x and y, dots ride the domain
        /// bounds at x = 0 and x = 10.
        fn dot_batch() -> RecordBatch {
            let schema = Arc::new(Schema::new(vec![
                Field::new("x", DataType::Float64, false),
                Field::new("y", DataType::Float64, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])),
                    Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                ],
            )
            .unwrap()
        }

        fn dot_channels() -> ChannelMap {
            let mut cm = ChannelMap::new();
            cm.insert(Channel::X, "x".into());
            cm.insert(Channel::Y, "y".into());
            cm
        }

        /// A bar fixture: categorical (band) x, numeric y (zero baseline). The
        /// caller supplies the values so the sign (all-positive / all-negative /
        /// straddling) drives which end the baseline pins to.
        fn bar_batch_vals(vals: Vec<f64>) -> RecordBatch {
            let cats: Vec<String> = (0..vals.len()).map(|i| format!("c{i}")).collect();
            let schema = Arc::new(Schema::new(vec![
                Field::new("cat", DataType::Utf8, false),
                Field::new("val", DataType::Float64, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(StringArray::from(cats)),
                    Arc::new(Float64Array::from(vals)),
                ],
            )
            .unwrap()
        }

        fn bar_batch() -> RecordBatch {
            bar_batch_vals(vec![3.0, 5.0, 2.0])
        }

        fn bar_channels() -> ChannelMap {
            let mut cm = ChannelMap::new();
            cm.insert(Channel::X, "cat".into());
            cm.insert(Channel::Y, "val".into());
            cm
        }

        #[test]
        fn axi_ac02_scatter_fixture_insets_all_four_ends() {
            let batch = dot_batch();
            let cm = dot_channels();
            let dot = DotRenderer;
            let entries: Vec<MarkInsetEntry> = vec![(&batch, &cm, &dot)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);
            assert_eq!(
                insets,
                Insets {
                    left: D,
                    right: D,
                    top: D,
                    bottom: D
                }
            );
        }

        #[test]
        fn axi_ac02_bar_fixture_stays_flush_at_baseline() {
            // Band x → no inset; positive value y → baseline flush at the bottom
            // (range start), only the top continuous end insets.
            let batch = bar_batch();
            let cm = bar_channels();
            let bar = BarRenderer;
            let entries: Vec<MarkInsetEntry> = vec![(&batch, &cm, &bar)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);
            assert_eq!(
                insets,
                Insets {
                    left: 0.0,
                    right: 0.0,
                    top: D,
                    bottom: 0.0
                }
            );
        }

        #[test]
        fn axi_ac02_multi_mark_exempts_baseline_if_any_mark_declares_it() {
            // A bar (declares a Y zero baseline) overlaid with a dot (does not).
            // The baseline end must stay flush because SOME mark declares it —
            // guards zero_pinned_end's `.any()` (an `.all()` would float the bar).
            let bar = bar_batch_vals(vec![3.0, 5.0, 2.0]);
            let bcm = bar_channels();
            let dotb = dot_batch();
            let dcm = dot_channels();
            let barr = BarRenderer;
            let dot = DotRenderer;
            let entries: Vec<MarkInsetEntry> =
                vec![(&bar, &bcm, &barr), (&dotb, &dcm, &dot)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);
            assert_eq!(insets.bottom, 0.0, "Y baseline stays flush across a multi-mark plot");
            assert_eq!(insets.top, D, "the non-baseline Y end still insets");
        }

        #[test]
        fn axi_ac02_all_negative_value_axis_exempts_the_top_end() {
            // extend_domain_to_zero pins 0 to the range End (top) when the value
            // axis is all-nonpositive → top stays flush, bottom insets.
            let bar = bar_batch_vals(vec![-5.0, -3.0, -2.0]);
            let bcm = bar_channels();
            let barr = BarRenderer;
            let entries: Vec<MarkInsetEntry> = vec![(&bar, &bcm, &barr)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);
            assert_eq!(insets.top, 0.0, "0 pins to the top (range end) → flush there");
            assert_eq!(insets.bottom, D);
        }

        #[test]
        fn axi_ac02_straddling_value_axis_insets_both_ends() {
            // The composed domain crosses zero → 0 is interior, no end is pinned,
            // both continuous ends inset.
            let bar = bar_batch_vals(vec![-3.0, 2.0, 4.0]);
            let bcm = bar_channels();
            let barr = BarRenderer;
            let entries: Vec<MarkInsetEntry> = vec![(&bar, &bcm, &barr)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);
            assert_eq!(insets.top, D);
            assert_eq!(insets.bottom, D, "straddle-zero → baseline is interior, both ends inset");
        }

        #[test]
        fn axi_ac05_edge_dot_renders_whole_inside_frame() {
            // The finding closes, as a real before/after geometry contrast: a dot
            // at domain max, mapped through the resolved (inset) range vs the
            // un-inset range, and its outer disc edge measured against the frame.
            // The pixel-level frame-clip proof is the PNG sweep (axi-ac06) + the
            // gallery (axi-ac07); this pins the scale geometry the clip acts on.
            const DOT_RADIUS: f64 = 4.0; // mark.rs
            let batch = dot_batch();
            let cm = dot_channels();
            let dot = DotRenderer;
            let entries: Vec<MarkInsetEntry> = vec![(&batch, &cm, &dot)];
            let insets = resolve_insets_for_marks(SideInsets::default(), &entries, D);

            let edge = |layout: &ChartLayout| {
                let (rx0, rx1) = layout.x_range();
                Scale::Linear {
                    domain_min: 0.0,
                    domain_max: 10.0,
                    range_start: rx0,
                    range_end: rx1,
                }
                .map_f64(10.0)
            };

            // AFTER (inset): the max-domain dot's whole disc fits inside the
            // frame — outer edge ≤ plot_x_end, inner edge ≥ plot_x_start.
            let inset_layout = ChartLayout::with_insets(640.0, 480.0, insets);
            let c_after = edge(&inset_layout);
            assert!(
                c_after + DOT_RADIUS <= inset_layout.plot_x_end(),
                "inset: outer edge {} inside frame {}",
                c_after + DOT_RADIUS,
                inset_layout.plot_x_end()
            );
            assert!(c_after - DOT_RADIUS >= inset_layout.plot_x_start());

            // BEFORE (no inset): the same dot's centre lands ON the frame, so its
            // outer edge crosses it — the finding, reproduced.
            let base_layout = ChartLayout::new(640.0, 480.0);
            let c_before = edge(&base_layout);
            assert!((c_before - base_layout.plot_x_end()).abs() < f64::EPSILON);
            assert!(
                c_before + DOT_RADIUS > base_layout.plot_x_end(),
                "un-inset: outer edge clips past the frame"
            );

            // The contrast is real only because the inset moved the edge inward.
            assert!(c_after < c_before, "inset pulled the edge dot inside the frame");
        }
    }
}
