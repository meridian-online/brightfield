//! Scene builder — orchestrates data -> scales -> marks + axes + legend
//! into a single vello::Scene.

use arrow::record_batch::RecordBatch;
use kurbo::{Affine, Circle, Rect, RoundedRect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::axis::{compute_ticks, render_x_axis, render_y_axis};
use crate::channel::{Channel, ChannelMap};
use crate::grid::{render_x_grid, render_y_grid};
use crate::layout::ChartLayout;
use crate::legend::render_colour_legend;
use crate::mark::{HighlightState, MarkRenderer};
use crate::scale::{infer_scales, infer_scales_multi, Scale, ScaleSet, ViewExtent};

/// Opaque white chart background. Drawn first so grid, marks, axes and legend
/// composite on top. Without it the scene renders onto transparency, which a
/// PNG export shows as a black/checkerboard backdrop and which makes a working
/// chart look broken.
const BACKGROUND_COLOUR: Color = Color::new([1.0, 1.0, 1.0, 1.0]);

/// Fill the full chart area with [`BACKGROUND_COLOUR`]. Must be the first
/// geometry added to the scene so everything else draws on top.
fn render_background(scene: &mut Scene, layout: &ChartLayout) {
    let rect = Rect::new(0.0, 0.0, layout.width, layout.height);
    scene.fill(Fill::NonZero, Affine::IDENTITY, BACKGROUND_COLOUR, None, &rect);
}

/// Input data for building a chart scene.
pub struct ChartData<'a> {
    /// The Arrow record batch containing the data.
    pub batch: &'a RecordBatch,
    /// The channel map (encoding channels -> column names).
    pub channel_map: &'a ChannelMap,
    /// The mark renderer to use.
    pub renderer: &'a dyn MarkRenderer,
    /// Chart layout (dimensions and margins).
    pub layout: ChartLayout,
    /// Optional view extent override for pan/zoom navigation.
    /// When `Some`, scale domains are overridden to the specified range.
    /// When `None`, the full data-inferred domain is used.
    pub view_extent: Option<&'a ViewExtent>,
    /// Optional highlight state for per-row dim/emphasis.
    /// When `Some`, matching rows render at full alpha; non-matching rows are dimmed.
    pub highlight: Option<&'a HighlightState>,
}

/// Build a complete chart scene from data and configuration.
///
/// Orchestrates: infer scales -> render grid -> render marks -> render axes -> render legend.
/// Returns the scene and the inferred scales (for interaction coordinate mapping).
/// Override a scale's domain with the given min/max values.
/// Only applies to continuous scales (Linear, Time). Band and Colour are returned unchanged.
fn override_scale_domain(scale: &Scale, new_min: f64, new_max: f64) -> Scale {
    match scale {
        Scale::Linear {
            range_start,
            range_end,
            ..
        } => Scale::Linear {
            domain_min: new_min,
            domain_max: new_max,
            range_start: *range_start,
            range_end: *range_end,
        },
        Scale::Time {
            range_start,
            range_end,
            ..
        } => Scale::Time {
            domain_min_us: new_min as i64,
            domain_max_us: new_max as i64,
            range_start: *range_start,
            range_end: *range_end,
        },
        // Band and Colour scales are not navigable — return unchanged.
        other => other.clone(),
    }
}

/// Plot-area rectangle in pixel coordinates, used to clip mark geometry so it
/// can't spill onto the axes or margins.
fn plot_area_rect(layout: &ChartLayout) -> Rect {
    Rect::new(
        layout.plot_x_start(),
        layout.plot_y_start(),
        layout.plot_x_end(),
        layout.plot_y_end(),
    )
}

/// Extend a linear scale's domain to include zero, so e.g. bars baseline on the
/// axis instead of extrapolating off the plot. Non-linear scales are unchanged.
fn extend_domain_to_zero(scale: &Scale) -> Scale {
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => Scale::Linear {
            domain_min: domain_min.min(0.0),
            domain_max: domain_max.max(0.0),
            range_start: *range_start,
            range_end: *range_end,
        },
        other => other.clone(),
    }
}

pub fn build_chart_scene(data: &ChartData<'_>) -> (Scene, ScaleSet) {
    let mut scene = Scene::new();
    render_background(&mut scene, &data.layout);

    let mut scales = infer_scales(
        data.batch,
        data.channel_map,
        data.layout.x_range(),
        data.layout.y_range(),
    );

    // Let the mark contribute positional scales generic column inference can't
    // supply (regression's x/y extents, 1D-density's perpendicular axis).
    data.renderer.augment_scales(
        &mut scales,
        data.batch,
        data.channel_map,
        data.layout.x_range(),
        data.layout.y_range(),
    );

    // Anchor the value axis at zero for marks that need it (e.g. bars), so the
    // baseline lands on the axis rather than extrapolating off the plot. Applied
    // before any view-extent override so an explicit pan/zoom still wins.
    if let Some(ch) = data.renderer.zero_baseline_channel() {
        if let Some(scale) = scales.get(ch) {
            let zeroed = extend_domain_to_zero(scale);
            scales.insert(ch, zeroed);
        }
    }

    // Apply view extent override for pan/zoom navigation.
    if let Some(extent) = data.view_extent {
        if let Some((x_min, x_max)) = extent.x {
            if let Some(x_scale) = scales.get(Channel::X) {
                let overridden = override_scale_domain(x_scale, x_min, x_max);
                scales.insert(Channel::X, overridden);
            }
        }
        if let Some((y_min, y_max)) = extent.y {
            if let Some(y_scale) = scales.get(Channel::Y) {
                let overridden = override_scale_domain(y_scale, y_min, y_max);
                scales.insert(Channel::Y, overridden);
            }
        }
    }

    // Grid lines (behind marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_grid(&mut scene, &data.layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_grid(&mut scene, &data.layout, &y_ticks);
    }

    // Marks, clipped to the plot area so geometry can't spill onto axes/margins.
    let plot_clip = plot_area_rect(&data.layout);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &plot_clip);
    data.renderer
        .render(&mut scene, data.batch, data.channel_map, &scales, data.highlight);
    scene.pop_layer();

    // Axes (on top of grid/marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_axis(&mut scene, &data.layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_axis(&mut scene, &data.layout, &y_ticks);
    }

    // Colour legend.
    if let Some(fill_scale) = scales.get(Channel::Fill) {
        render_colour_legend(&mut scene, &data.layout, fill_scale);
    }

    (scene, scales)
}

/// Build a scene from multiple marks sharing a single set of scales.
///
/// Calls `infer_scales_multi` to union domains across all entries, renders grid
/// once, then each mark renderer, then axes and legend. Returns `(Scene, ScaleSet)`.
/// The existing `build_chart_scene()` is unchanged.
///
/// `draw_inline_legend` controls the plot's own colour legend in its top-right
/// corner. Pass `false` when a standalone `legend:` node has relocated it to a
/// separate layout rect, so the same scale isn't drawn twice.
pub fn build_multi_mark_scene(
    entries: &[&ChartData<'_>],
    draw_inline_legend: bool,
) -> (Scene, ScaleSet) {
    if entries.is_empty() {
        return (Scene::new(), ScaleSet::new());
    }

    let layout = &entries[0].layout;

    // Collect (batch, channel_map) pairs for multi-scale inference.
    let pairs: Vec<(&RecordBatch, &ChannelMap)> = entries
        .iter()
        .map(|d| (d.batch, d.channel_map))
        .collect();

    let mut scales = infer_scales_multi(&pairs, layout.x_range(), layout.y_range());

    // Let each mark contribute positional scales generic column inference can't
    // supply (regression's x/y extents, 1D-density's perpendicular axis).
    for entry in entries {
        entry.renderer.augment_scales(
            &mut scales,
            entry.batch,
            entry.channel_map,
            layout.x_range(),
            layout.y_range(),
        );
    }

    // Anchor value axes at zero for any mark that needs it (e.g. bars).
    let mut zero_channels: Vec<Channel> = Vec::new();
    for entry in entries {
        if let Some(ch) = entry.renderer.zero_baseline_channel() {
            if !zero_channels.contains(&ch) {
                zero_channels.push(ch);
            }
        }
    }
    for ch in zero_channels {
        if let Some(scale) = scales.get(ch) {
            let zeroed = extend_domain_to_zero(scale);
            scales.insert(ch, zeroed);
        }
    }

    // Apply view extent from the first entry (shared navigation).
    if let Some(extent) = entries[0].view_extent {
        if let Some((x_min, x_max)) = extent.x {
            if let Some(x_scale) = scales.get(Channel::X) {
                let overridden = override_scale_domain(x_scale, x_min, x_max);
                scales.insert(Channel::X, overridden);
            }
        }
        if let Some((y_min, y_max)) = extent.y {
            if let Some(y_scale) = scales.get(Channel::Y) {
                let overridden = override_scale_domain(y_scale, y_min, y_max);
                scales.insert(Channel::Y, overridden);
            }
        }
    }

    let scene = draw_multi_mark_scene(entries, draw_inline_legend, &scales);
    (scene, scales)
}

/// Draw a multi-mark plot's background, grid, marks, axes, and inline legend
/// against an ALREADY-RESOLVED `scales`. The shared drawing half of
/// [`build_multi_mark_scene`] (which infers `scales` first) and
/// [`build_multi_mark_scene_pinned`] (which is handed a launch-pinned set), so
/// both render byte-identical geometry from the same scale set. Callers
/// guarantee `entries` is non-empty.
fn draw_multi_mark_scene(
    entries: &[&ChartData<'_>],
    draw_inline_legend: bool,
    scales: &ScaleSet,
) -> Scene {
    let layout = &entries[0].layout;
    let mut scene = Scene::new();
    render_background(&mut scene, layout);

    // Grid lines (behind marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_grid(&mut scene, layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_grid(&mut scene, layout, &y_ticks);
    }

    // Render each mark layer, clipped to the plot area.
    let plot_clip = plot_area_rect(layout);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &plot_clip);
    for entry in entries {
        entry.renderer.render(
            &mut scene,
            entry.batch,
            entry.channel_map,
            scales,
            entry.highlight,
        );
    }
    scene.pop_layer();

    // Axes (on top of marks).
    if let Some(x_scale) = scales.get(Channel::X) {
        let x_ticks = compute_ticks(x_scale, 5);
        render_x_axis(&mut scene, layout, &x_ticks);
    }
    if let Some(y_scale) = scales.get(Channel::Y) {
        let y_ticks = compute_ticks(y_scale, 5);
        render_y_axis(&mut scene, layout, &y_ticks);
    }

    // Colour legend — unless a standalone `legend:` node has relocated it.
    if draw_inline_legend {
        if let Some(fill_scale) = scales.get(Channel::Fill) {
            render_colour_legend(&mut scene, layout, fill_scale);
        }
    }

    scene
}

/// Launch-pinned sibling of [`build_multi_mark_scene`]: draws the grid, marks,
/// axes, and inline legend against the CALLER-SUPPLIED `scales` instead of
/// inferring them from `entries`. Runs NONE of inference / `augment_scales` /
/// zero-baseline / view-extent — the pinned set already went through all of
/// those when it was inferred at launch.
///
/// The cross-filter coordinator captures each plot's launch `ScaleSet` and
/// rebuilds every gesture (brush, point click, slider, legend click) through
/// this, so axes, colour assignments, and ramp anchoring hold still while only
/// the data moves — a live gesture reads as FILTERING, not redrawing (card
/// 0006 render fidelity). Returns an empty scene for empty `entries`.
pub fn build_multi_mark_scene_pinned(
    entries: &[&ChartData<'_>],
    draw_inline_legend: bool,
    scales: &ScaleSet,
) -> Scene {
    if entries.is_empty() {
        return Scene::new();
    }
    draw_multi_mark_scene(entries, draw_inline_legend, scales)
}

/// Compose pre-rendered plot scenes into one dashboard scene.
///
/// Each plot scene is built independently by the caller (its own axes/grid/
/// legend, domains unioned only *within* the plot) via [`build_multi_mark_scene`]
/// against a [`ChartLayout`] sized to the plot; this places each at its
/// `(origin_x, origin_y)` over a white dashboard background so inter-plot gaps
/// aren't transparent.
///
/// The live window hosts one element per plot (so each keeps independent
/// interaction); this single-composite path is used for the headless/PNG render
/// and shares the same per-plot scenes.
pub fn compose_dashboard(width: f64, height: f64, plots: &[(f64, f64, &Scene)]) -> Scene {
    let mut scene = Scene::new();
    render_background(&mut scene, &ChartLayout::new(width, height));
    for (origin_x, origin_y, plot_scene) in plots {
        scene.append(plot_scene, Some(Affine::translate((*origin_x, *origin_y))));
    }
    scene
}

/// Slider widget colours + geometry — kept in sync with the live GPUI
/// `SliderElement` so the headless PNG matches the window (card 0005).
const SLIDER_TRACK_COLOUR: Color = Color::new([0.82, 0.83, 0.86, 1.0]);
const SLIDER_THUMB_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 1.0]);
const SLIDER_THUMB_RADIUS: f64 = 7.0;
const SLIDER_TRACK_THICKNESS: f64 = 4.0;

/// Draw a slider widget (rounded track + circular thumb) into `scene` at the
/// dashboard-space rect `(x, y, width, height)`, the thumb at `frac` (0..1) along
/// the track. `frac` is the value's normalised position (`(value-min)/(max-min)`),
/// matching the live element's `thumb_fraction`. Used by the headless PNG dump to
/// preview the resting widget.
pub fn render_slider(scene: &mut Scene, x: f64, y: f64, width: f64, height: f64, frac: f64) {
    let inset = SLIDER_THUMB_RADIUS;
    let track_left = x + inset;
    let track_w = (width - inset * 2.0).max(0.0);
    let cy = y + height / 2.0;

    let track = RoundedRect::new(
        track_left,
        cy - SLIDER_TRACK_THICKNESS / 2.0,
        track_left + track_w,
        cy + SLIDER_TRACK_THICKNESS / 2.0,
        SLIDER_TRACK_THICKNESS / 2.0,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, SLIDER_TRACK_COLOUR, None, &track);

    let thumb_cx = track_left + track_w * frac.clamp(0.0, 1.0);
    let thumb = Circle::new((thumb_cx, cy), SLIDER_THUMB_RADIUS);
    scene.fill(Fill::NonZero, Affine::IDENTITY, SLIDER_THUMB_COLOUR, None, &thumb);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelMap};
    use crate::layout::ChartLayout;
    use crate::mark::{BarRenderer, DotRenderer, LineRenderer};

    // slw ac-10 (card 0005): render_slider draws exactly two shapes — the track
    // and the thumb — into the scene (headless proof the widget renders).
    #[test]
    fn slw_ac10_render_slider_draws_track_and_thumb() {
        let mut scene = Scene::new();
        render_slider(&mut scene, 0.0, 400.0, 200.0, 32.0, 0.5);
        assert_eq!(
            crate::mark::count_scene_paths(&scene),
            2,
            "slider draws a track + a thumb"
        );
    }
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    // A fill-colour plot draws an inline legend by default; `draw_inline_legend =
    // false` suppresses it, so the scale isn't drawn twice when a standalone
    // `legend:` node has relocated it. (multi-view inc 6 — two-legend fix)
    #[test]
    fn build_multi_mark_scene_suppresses_inline_legend_when_asked() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("grp", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "grp".to_string());
        let dot = DotRenderer;
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &dot,
            layout: ChartLayout::new(400.0, 300.0),
            view_extent: None,
            highlight: None,
        };
        let (with_legend, _) = build_multi_mark_scene(&[&data], true);
        let (without_legend, _) = build_multi_mark_scene(&[&data], false);
        let (n_with, n_without) = (
            crate::mark::count_scene_paths(&with_legend),
            crate::mark::count_scene_paths(&without_legend),
        );
        assert!(
            n_with > n_without,
            "suppressing the inline legend must drop its swatch/panel fills: {n_with} !> {n_without}"
        );
    }

    #[test]
    fn mvdash_dashboard_scene_composes_independent_plots() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let dot = DotRenderer;
        let d0 = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &dot,
            layout: ChartLayout::new(300.0, 200.0),
            view_extent: None,
            highlight: None,
        };
        let d1 = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &dot,
            layout: ChartLayout::new(300.0, 200.0),
            view_extent: None,
            highlight: None,
        };

        // Each plot's scene is built independently, then composited.
        let (s0, _) = build_multi_mark_scene(&[&d0], true);
        let (s1, _) = build_multi_mark_scene(&[&d1], true);

        let two = compose_dashboard(600.0, 200.0, &[(0.0, 0.0, &s0), (300.0, 0.0, &s1)]);
        let one = compose_dashboard(600.0, 200.0, &[(0.0, 0.0, &s0)]);
        assert!(
            two.encoding().path_tags.len() > one.encoding().path_tags.len(),
            "two composed plots produce more geometry than one"
        );

        // An empty dashboard still paints its background.
        let empty = compose_dashboard(600.0, 200.0, &[]);
        assert!(
            empty.encoding().path_tags.len() > 0,
            "dashboard background fills even with no plots"
        );
    }

    #[test]
    fn gpu_ac08_build_dot_chart_scene() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("colour", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "a", "b", "a"])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "colour".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, scales) = build_chart_scene(&data);

        // Scene should be non-empty.
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "dot chart scene should have content"
        );

        // Scales should include x, y, and fill.
        assert!(scales.get(Channel::X).is_some(), "x scale should exist");
        assert!(scales.get(Channel::Y).is_some(), "y scale should exist");
        assert!(scales.get(Channel::Fill).is_some(), "fill scale should exist");
    }

    #[test]
    fn gpu_ac08_build_bar_chart_scene() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = BarRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, _scales) = build_chart_scene(&data);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "bar chart scene should have content"
        );
    }

    #[test]
    fn gpu_bars_zero_baseline_anchors_value_axis() {
        // Bar values [10, 20, 30]: the y-domain must be extended to include 0 so
        // the baseline lands on the axis instead of extrapolating off the plot.
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &BarRenderer,
            layout: ChartLayout::new(640.0, 480.0),
            view_extent: None,
            highlight: None,
        };
        let (_scene, scales) = build_chart_scene(&data);
        let y = scales.get(Channel::Y).unwrap();
        assert!(
            (y.domain_min().unwrap() - 0.0).abs() < f64::EPSILON,
            "bar y-domain should start at 0, got {:?}",
            y.domain_min()
        );
        assert!((y.domain_max().unwrap() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_dots_do_not_zero_anchor() {
        // Dots keep their data-driven domain — no zero baseline.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &DotRenderer,
            layout: ChartLayout::new(640.0, 480.0),
            view_extent: None,
            highlight: None,
        };
        let (_scene, scales) = build_chart_scene(&data);
        let y = scales.get(Channel::Y).unwrap();
        assert!(
            (y.domain_min().unwrap() - 10.0).abs() < f64::EPSILON,
            "dot y-domain should be data-driven (10), not 0"
        );
    }

    #[test]
    fn gpu_ac08_build_line_chart_scene() {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_000_000, 2_000_000, 3_000_000, 4_000_000,
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = LineRenderer;

        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, _scales) = build_chart_scene(&data);

        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "line chart scene should have content"
        );
    }

    #[test]
    fn nav_ac03_view_extent_overrides_scale_domain() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        // Without view extent — full data domain.
        let data_full = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout: layout.clone(),
            view_extent: None,
            highlight: None,
        };
        let (_scene, scales_full) = build_chart_scene(&data_full);
        let x_full = scales_full.get(Channel::X).unwrap();
        assert!((x_full.domain_min().unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((x_full.domain_max().unwrap() - 5.0).abs() < f64::EPSILON);

        // With view extent — narrowed x domain.
        let extent = ViewExtent {
            x: Some((2.0, 4.0)),
            y: None,
        };
        let data_zoomed = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: Some(&extent),
            highlight: None,
        };
        let (_scene, scales_zoomed) = build_chart_scene(&data_zoomed);
        let x_zoomed = scales_zoomed.get(Channel::X).unwrap();
        assert!((x_zoomed.domain_min().unwrap() - 2.0).abs() < f64::EPSILON);
        assert!((x_zoomed.domain_max().unwrap() - 4.0).abs() < f64::EPSILON);

        // Y should be unchanged.
        let y_zoomed = scales_zoomed.get(Channel::Y).unwrap();
        assert!((y_zoomed.domain_min().unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((y_zoomed.domain_max().unwrap() - 50.0).abs() < f64::EPSILON);
    }

    // --- msv ac-03: build_multi_mark_scene ---

    #[test]
    fn msv_ac03_multi_mark_scene_dot_and_line() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch1 = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 3.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 30.0, 50.0])),
            ],
        )
        .unwrap();

        let batch2 = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![2.0, 4.0, 8.0])),
                Arc::new(Float64Array::from(vec![20.0, 40.0, 80.0])),
            ],
        )
        .unwrap();

        let mut cm1 = ChannelMap::new();
        cm1.insert(Channel::X, "x".to_string());
        cm1.insert(Channel::Y, "y".to_string());
        let mut cm2 = ChannelMap::new();
        cm2.insert(Channel::X, "x".to_string());
        cm2.insert(Channel::Y, "y".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let dot_renderer = DotRenderer;
        let line_renderer = LineRenderer;

        let data1 = ChartData {
            batch: &batch1,
            channel_map: &cm1,
            renderer: &dot_renderer,
            layout: layout.clone(),
            view_extent: None,
            highlight: None,
        };
        let data2 = ChartData {
            batch: &batch2,
            channel_map: &cm2,
            renderer: &line_renderer,
            layout,
            view_extent: None,
            highlight: None,
        };

        let (scene, scales) = build_multi_mark_scene(&[&data1, &data2], true);

        // Scene should be non-empty.
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "multi-mark scene should have content"
        );

        // Scales should span the union of both batches.
        let x = scales.get(Channel::X).expect("x scale should exist");
        assert!((x.domain_min().unwrap() - 1.0).abs() < f64::EPSILON, "x min = union min");
        assert!((x.domain_max().unwrap() - 8.0).abs() < f64::EPSILON, "x max = union max");

        let y = scales.get(Channel::Y).expect("y scale should exist");
        assert!((y.domain_min().unwrap() - 10.0).abs() < f64::EPSILON, "y min = union min");
        assert!((y.domain_max().unwrap() - 80.0).abs() < f64::EPSILON, "y max = union max");
    }

    #[test]
    fn msv_ac03_multi_mark_scene_empty_entries() {
        let (scene, scales) = build_multi_mark_scene(&[], true);
        let encoding = scene.encoding();
        assert_eq!(encoding.path_tags.len(), 0, "empty entries => empty scene");
        assert!(scales.get(Channel::X).is_none());
    }

    #[test]
    fn ifb_ac05_build_chart_scene_with_highlight() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer = DotRenderer;

        let hs = crate::mark::HighlightState {
            predicate: Box::new(|row| row == 1),
            dimmed_alpha: 0.15,
        };

        // With highlight
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout: layout.clone(),
            view_extent: None,
            highlight: Some(&hs),
        };
        let (scene, _scales) = build_chart_scene(&data);
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene with highlight should have content"
        );

        // Without highlight (backward compat)
        let data_no_hl = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: None,
        };
        let (scene2, _) = build_chart_scene(&data_no_hl);
        let encoding2 = scene2.encoding();
        assert!(
            encoding2.path_tags.len() > 0,
            "scene without highlight should also work"
        );
    }
}
