//! Axis + plot titles — resolve the spec's Override/Suppress/Derive decisions
//! against the plot's channel maps into concrete title text, and grow the plot
//! margins to reserve a fixed band for each present title.
//!
//! The DERIVE decision (from [`brightfield_spec::layout::resolve_axis_titles`])
//! is turned into a field name HERE — where the channel map lives — because the
//! spec crate can't see the render `ChannelMap`. The resolved titles then ride a
//! render arg + a `LivePlot` field (not `ChartLayout`, which stays `Copy`), and
//! are captured in the `ChromeSnapshot` gate.

use brightfield_spec::ast::PlotNode;
use brightfield_spec::layout::{resolve_axis_titles, AxisTitle};

use crate::channel::{Channel, ChannelMap};
use crate::layout::Margins;
use crate::text::TITLE_SIZE;

/// A plot's fully-resolved titles: the concrete text (or `None`) for each
/// positional axis and the plot, ready to render and to size margins. A
/// `Suppress`, an underivable (interval-only / augment-only), or an absent axis
/// is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTitles {
    /// The x-axis title, if any (drawn below the tick labels).
    pub x: Option<String>,
    /// The y-axis title, if any (drawn rotated up the left margin).
    pub y: Option<String>,
    /// The per-plot title, if any (drawn above the frame).
    pub plot: Option<String>,
}

impl ResolvedTitles {
    /// True when the plot carries no title at all (no margin growth, no ink).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.y.is_none() && self.plot.is_none()
    }
}

/// Resolve one axis's decision against the mark channel maps. A `Derive` axis
/// takes the FIRST map (in mark order) that binds the positional channel to a
/// COLUMN — subsuming the `entries[0]` default and covering a literal-first mark
/// (e.g. a `ruleY` at a constant) whose bound sibling names the axis. An
/// interval-only axis (`x1`/`x2`, `y1`/`y2` — no plain `x`/`y` column) or an
/// augment-only axis binds no column, so `Derive` yields `None`.
fn resolve_axis(decision: &AxisTitle, channel: Channel, maps: &[&ChannelMap]) -> Option<String> {
    match decision {
        AxisTitle::Override(s) => Some(s.clone()),
        AxisTitle::Suppress => None,
        AxisTitle::Derive => maps.iter().find_map(|m| m.get(channel).map(str::to_string)),
    }
}

/// Resolve a plot's [`ResolvedTitles`] from its attributes + the mark channel
/// maps. The Override/Suppress/Derive decision comes from the pure spec resolver
/// ([`resolve_axis_titles`]); the Derive field name and the plot title text are
/// filled in here.
#[must_use]
pub fn resolve_titles(plot: &PlotNode, channel_maps: &[&ChannelMap]) -> ResolvedTitles {
    let decided = resolve_axis_titles(plot);
    ResolvedTitles {
        x: resolve_axis(&decided.x, Channel::X, channel_maps),
        y: resolve_axis(&decided.y, Channel::Y, channel_maps),
        plot: decided.plot,
    }
}

/// Fixed title-band width added to a margin for one present title: the font
/// height plus padding. A title runs ALONG its axis, so the extent it consumes
/// IN the margin is this constant cross-axis band, independent of text length.
pub const TITLE_BAND: f64 = TITLE_SIZE as f64 + 8.0;

/// Grow the base margins to reserve a fixed [`TITLE_BAND`] for each present
/// title: left for a y-title, bottom for an x-title, top for a plot title.
/// Absent titles leave that side at its base (Observable-default) value; the
/// right margin never grows. Both layout models feed this the same base +
/// titles, so their grown margins agree per-side.
#[must_use]
pub fn grow_margins(base: Margins, titles: &ResolvedTitles) -> Margins {
    Margins {
        left: base.left + if titles.y.is_some() { TITLE_BAND } else { 0.0 },
        bottom: base.bottom + if titles.x.is_some() { TITLE_BAND } else { 0.0 },
        top: base.top + if titles.plot.is_some() { TITLE_BAND } else { 0.0 },
        right: base.right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::{PlotNode, SpecValue};
    use indexmap::IndexMap;

    fn plot_with(attrs: &[(&str, SpecValue)]) -> PlotNode {
        let mut attributes = IndexMap::new();
        for (k, v) in attrs {
            attributes.insert((*k).to_string(), v.clone());
        }
        PlotNode { items: vec![], attributes }
    }

    fn map_cols(cols: &[(Channel, &str)]) -> ChannelMap {
        let mut m = ChannelMap::new();
        for (c, col) in cols {
            m.insert(*c, (*col).to_string());
        }
        m
    }

    #[test]
    fn derive_override_suppress_and_interval() {
        // Bare plot, column-bound x/y → derived field names.
        let cm = map_cols(&[(Channel::X, "temp"), (Channel::Y, "power")]);
        let t = resolve_titles(&plot_with(&[]), &[&cm]);
        assert_eq!(t.x.as_deref(), Some("temp"));
        assert_eq!(t.y.as_deref(), Some("power"));

        // Override wins; explicit null suppresses (→ None, no band).
        let t = resolve_titles(
            &plot_with(&[
                ("xLabel", SpecValue::String("Temperature (°C)".into())),
                ("yLabel", SpecValue::Null),
            ]),
            &[&cm],
        );
        assert_eq!(t.x.as_deref(), Some("Temperature (°C)"));
        assert_eq!(t.y, None);

        // Interval-only axis (x1/x2, no plain x) → no derived title.
        let interval = map_cols(&[(Channel::X1, "lo"), (Channel::X2, "hi"), (Channel::Y, "count")]);
        let t = resolve_titles(&plot_with(&[]), &[&interval]);
        assert_eq!(t.x, None, "interval-only x names no single field → no title");
        assert_eq!(t.y.as_deref(), Some("count"));
    }

    #[test]
    fn multimark_first_column_bound_entry_names_the_axis() {
        // First mark binds y to a literal only (no y column); the second binds
        // y to `level`. Derive must pick the bound sibling, not drop the title.
        let mut literal_first = ChannelMap::new();
        literal_first.insert(Channel::X, "date".into());
        literal_first.insert_literal(Channel::Y, 30.0);
        let bound = map_cols(&[(Channel::X, "date"), (Channel::Y, "level")]);
        let t = resolve_titles(&plot_with(&[]), &[&literal_first, &bound]);
        assert_eq!(t.y.as_deref(), Some("level"), "first column-bound entry names y");
        assert_eq!(t.x.as_deref(), Some("date"));
    }

    #[test]
    fn grow_margins_reserves_a_band_per_present_title() {
        let base = Margins::default();
        // All three present → left/bottom/top each grow by the band; right holds.
        let all = ResolvedTitles {
            x: Some("x".into()),
            y: Some("y".into()),
            plot: Some("p".into()),
        };
        let g = grow_margins(base, &all);
        assert!((g.left - (base.left + TITLE_BAND)).abs() < f64::EPSILON);
        assert!((g.bottom - (base.bottom + TITLE_BAND)).abs() < f64::EPSILON);
        assert!((g.top - (base.top + TITLE_BAND)).abs() < f64::EPSILON);
        assert!((g.right - base.right).abs() < f64::EPSILON, "right never grows");

        // No titles → margins unchanged (an untitled plot is byte-identical).
        let none = ResolvedTitles::default();
        let g0 = grow_margins(base, &none);
        assert!((g0.left - base.left).abs() < f64::EPSILON);
        assert!((g0.bottom - base.bottom).abs() < f64::EPSILON);
        assert!((g0.top - base.top).abs() < f64::EPSILON);
        assert!(none.is_empty());
    }
}
