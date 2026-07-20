//! Scene builder — orchestrates data -> scales -> marks + axes + legend
//! into a single vello::Scene.

use arrow::record_batch::RecordBatch;
use kurbo::{Affine, BezPath, Circle, Rect, RoundedRect, Stroke};
use peniko::{Color, Fill};
use vello::Scene;

use crate::axis::{compute_ticks, render_plot_title, render_x_axis, render_y_axis};
use crate::channel::{Channel, ChannelMap};
use crate::grid::{render_x_grid, render_y_grid};
use crate::ink::ink;
use crate::layout::ChartLayout;
use crate::legend::render_colour_legend;
use crate::mark::{HighlightState, MarkRenderer};
use crate::scale::{infer_scales, infer_scales_multi, Scale, ScaleSet, ViewExtent};
use crate::title::ResolvedTitles;

/// Opaque chart background — the Meridian warm chart surface. Drawn first so
/// grid, marks, axes and legend composite on top. Without it the scene renders
/// onto transparency, which a PNG export shows as a black/checkerboard backdrop
/// and which makes a working chart look broken.
const BACKGROUND_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.surface);

/// Fill the full chart area with [`BACKGROUND_COLOUR`]. Must be the first
/// geometry added to the scene so everything else draws on top.
fn render_background(scene: &mut Scene, layout: &ChartLayout) {
    let rect = Rect::new(0.0, 0.0, layout.width, layout.height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        BACKGROUND_COLOUR,
        None,
        &rect,
    );
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

    // A frame-suppressing mark (geo) drops the grid + axes (geo).
    let suppress_frame = data.renderer.suppresses_frame();

    // Grid lines (behind marks).
    if !suppress_frame {
        if let Some(x_scale) = scales.get(Channel::X) {
            let x_ticks = compute_ticks(x_scale, 5);
            render_x_grid(&mut scene, &data.layout, &x_ticks);
        }
        if let Some(y_scale) = scales.get(Channel::Y) {
            let y_ticks = compute_ticks(y_scale, 5);
            render_y_grid(&mut scene, &data.layout, &y_ticks);
        }
    }

    // Marks, clipped to the plot area so geometry can't spill onto axes/margins.
    let plot_clip = plot_area_rect(&data.layout);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &plot_clip);
    data.renderer.render(
        &mut scene,
        data.batch,
        data.channel_map,
        &scales,
        data.highlight,
    );
    scene.pop_layer();

    // Axes (on top of grid/marks). The legacy single-mark path carries no
    // titles — the titled path is the multi-mark builder below.
    if !suppress_frame {
        if let Some(x_scale) = scales.get(Channel::X) {
            let x_ticks = compute_ticks(x_scale, 5);
            render_x_axis(&mut scene, &data.layout, &x_ticks, None);
        }
        if let Some(y_scale) = scales.get(Channel::Y) {
            let y_ticks = compute_ticks(y_scale, 5);
            render_y_axis(&mut scene, &data.layout, &y_ticks, None);
        }
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
    titles: &ResolvedTitles,
) -> (Scene, ScaleSet) {
    if entries.is_empty() {
        return (Scene::new(), ScaleSet::new());
    }
    let scales = infer_multi_mark_scales(entries);
    let scene = draw_multi_mark_scene(entries, draw_inline_legend, titles, &scales);
    (scene, scales)
}

/// Infer the shared scale set for a multi-mark plot: `infer_scales_multi` to
/// union column domains, then each mark's `augment_scales` (regression extents,
/// 1D-density perpendicular axis, the density-family Fill Sequential and half-bin
/// widening), then the zero-baseline anchor, then the first entry's view extent.
/// The inference half of [`build_multi_mark_scene`], extracted so a live rebuild
/// can re-infer FRESH scales from the current batches and fold them against the
/// launch set via [`crate::scale::anchor_scales`]. Callers guarantee `entries`
/// is non-empty.
fn infer_multi_mark_scales(entries: &[&ChartData<'_>]) -> ScaleSet {
    let layout = &entries[0].layout;

    // Collect (batch, channel_map) pairs for multi-scale inference.
    let pairs: Vec<(&RecordBatch, &ChannelMap)> =
        entries.iter().map(|d| (d.batch, d.channel_map)).collect();

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

    scales
}

/// Draw a multi-mark plot's background, grid, marks, axes, and inline legend
/// against an ALREADY-RESOLVED `scales`. The shared drawing half of
/// [`build_multi_mark_scene`] (which infers `scales` first) and
/// [`build_multi_mark_scene_anchored`] (which folds a fresh inference against a
/// launch set), so both render byte-identical geometry from the same scale set.
/// Callers guarantee `entries` is non-empty.
fn draw_multi_mark_scene(
    entries: &[&ChartData<'_>],
    draw_inline_legend: bool,
    titles: &ResolvedTitles,
    scales: &ScaleSet,
) -> Scene {
    let layout = &entries[0].layout;
    let mut scene = Scene::new();
    render_background(&mut scene, layout);

    // A frame-suppressing mark (geo — it projects its own coordinate space and
    // reads as a map) drops the grid + axes for the whole plot (geo).
    let suppress_frame = entries.iter().any(|e| e.renderer.suppresses_frame());

    // Grid lines (behind marks).
    if !suppress_frame {
        if let Some(x_scale) = scales.get(Channel::X) {
            let x_ticks = compute_ticks(x_scale, 5);
            render_x_grid(&mut scene, layout, &x_ticks);
        }
        if let Some(y_scale) = scales.get(Channel::Y) {
            let y_ticks = compute_ticks(y_scale, 5);
            render_y_grid(&mut scene, layout, &y_ticks);
        }
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

    // Axes (on top of marks), each carrying its resolved title (Derive already
    // resolved to a field name upstream; None = suppressed / underivable).
    if !suppress_frame {
        if let Some(x_scale) = scales.get(Channel::X) {
            let x_ticks = compute_ticks(x_scale, 5);
            render_x_axis(&mut scene, layout, &x_ticks, titles.x.as_deref());
        }
        if let Some(y_scale) = scales.get(Channel::Y) {
            let y_ticks = compute_ticks(y_scale, 5);
            render_y_axis(&mut scene, layout, &y_ticks, titles.y.as_deref());
        }
    }

    // Per-plot title, above the frame (the top margin has grown to make room).
    if let Some(plot_title) = titles.plot.as_deref() {
        render_plot_title(&mut scene, layout, plot_title);
    }

    // Colour legend — unless a standalone `legend:` node has relocated it.
    if draw_inline_legend {
        if let Some(fill_scale) = scales.get(Channel::Fill) {
            render_colour_legend(&mut scene, layout, fill_scale);
        }
    }

    scene
}

/// Launch-anchored sibling of [`build_multi_mark_scene`]: infers FRESH scales
/// from the current `entries`, folds them against the immutable `launch` set via
/// [`crate::scale::anchor_scales`] (widen-only), and draws against the result —
/// returning the scene AND the anchored scales it rendered against (the caller
/// stores them as the plot's displayed set so brush/click inversion matches what
/// is on screen).
///
/// The cross-filter coordinator rebuilds every gesture (brush, point click,
/// slider, legend click) through this. A subset filter yields exactly `launch`,
/// so axes, colour assignments, and ramp anchoring hold still — a gesture reads
/// as FILTERING, not redrawing. A gesture that REWRITES the query (a slider
/// changing a `$param`) can surface rows outside the launch domain; the anchor
/// widens to keep them on-plot instead of clipping them into invisibility.
/// Returns `(empty scene, launch.clone())` for empty `entries`.
pub fn build_multi_mark_scene_anchored(
    entries: &[&ChartData<'_>],
    draw_inline_legend: bool,
    titles: &ResolvedTitles,
    launch: &ScaleSet,
) -> (Scene, ScaleSet) {
    if entries.is_empty() {
        return (Scene::new(), launch.clone());
    }
    let fresh = infer_multi_mark_scales(entries);
    let anchored = crate::scale::anchor_scales(launch, fresh);
    let scene = draw_multi_mark_scene(entries, draw_inline_legend, titles, &anchored);
    (scene, anchored)
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
/// `SliderElement` (crates/brightfield-ui/src/slider_element.rs) so the
/// headless PNG matches the window. Both sides read the same
/// Meridian tokens: track = warm gray step 5, thumb = Maritime focus ink.
const SLIDER_TRACK_COLOUR: Color = ink(meridian_design::scales::GRAY_LIGHT[4]);
const SLIDER_THUMB_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.focus);
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
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        SLIDER_TRACK_COLOUR,
        None,
        &track,
    );

    let thumb_cx = track_left + track_w * frac.clamp(0.0, 1.0);
    let thumb = Circle::new((thumb_cx, cy), SLIDER_THUMB_RADIUS);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        SLIDER_THUMB_COLOUR,
        None,
        &thumb,
    );
}

/// Menu-family widget ink + geometry — kept in sync with the
/// live GPUI shim (crates/brightfield-ui/src/menu_element.rs `WIDGET_*`
/// constants) so the headless PNG matches the window, exactly like the
/// SLIDER_* pair above. Both sides read the same Meridian tokens: fill =
/// chart surface, border = warm gray step 5 (the slider-track gray), label
/// = primary ink, affordance (chevron / ring outline) = muted ink, active
/// (selected dot / check) = Maritime focus ink.
const WIDGET_FILL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.surface);
const WIDGET_BORDER_COLOUR: Color = ink(meridian_design::scales::GRAY_LIGHT[4]);
const WIDGET_LABEL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_primary);
const WIDGET_AFFORDANCE_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_muted);
const WIDGET_ACTIVE_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.focus);
/// Widget label text size (px) — matches the GPUI shim's `WIDGET_TEXT_SIZE`.
const WIDGET_TEXT_SIZE: f32 = 12.0;
/// Vertical offset from a row's centre to the label baseline at
/// `WIDGET_TEXT_SIZE` (approximately half the cap height).
const WIDGET_BASELINE_NUDGE: f64 = 4.0;

/// Draw a resting `style: menu` widget: a rounded box with the
/// current value's label and a downward chevron affordance. Used by the
/// headless PNG dump to preview the closed menu exactly as the window hosts
/// it (the render_slider convention).
pub fn render_menu(scene: &mut Scene, x: f64, y: f64, width: f64, height: f64, label: &str) {
    let bx = RoundedRect::new(x + 0.5, y + 0.5, x + width - 0.5, y + height - 0.5, 4.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        WIDGET_FILL_COLOUR,
        None,
        &bx,
    );
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        WIDGET_BORDER_COLOUR,
        None,
        &bx,
    );
    let cy = y + height / 2.0;
    crate::text::draw_text(
        scene,
        label,
        x + 10.0,
        cy + WIDGET_BASELINE_NUDGE,
        WIDGET_TEXT_SIZE,
        WIDGET_LABEL_COLOUR,
        crate::text::TextAnchor::Start,
    );
    // Chevron: a small downward triangle at the right edge.
    let (cx0, cx1) = (x + width - 18.0, x + width - 8.0);
    let mut chevron = BezPath::new();
    chevron.move_to((cx0, cy - 2.5));
    chevron.line_to((cx1, cy - 2.5));
    chevron.line_to(((cx0 + cx1) / 2.0, cy + 3.5));
    chevron.close_path();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        WIDGET_AFFORDANCE_COLOUR,
        None,
        &chevron,
    );
}

/// Draw a resting `style: radio` widget: one
/// [`brightfield_spec::layout::RADIO_ROW_HEIGHT`] row per option — ring +
/// filled-when-selected Maritime dot + label — inside the
/// `22·N + RADIO_CHROME_PAD` rect the layout reserved (the shared-constant
/// contract).
pub fn render_radio(
    scene: &mut Scene,
    x: f64,
    y: f64,
    _width: f64,
    labels: &[String],
    selected: Option<usize>,
) {
    use brightfield_spec::layout::{RADIO_CHROME_PAD, RADIO_ROW_HEIGHT};
    for (i, label) in labels.iter().enumerate() {
        let cy = y + RADIO_CHROME_PAD / 2.0 + RADIO_ROW_HEIGHT * (i as f64 + 0.5);
        let ring = Circle::new((x + 10.0, cy), 6.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            WIDGET_FILL_COLOUR,
            None,
            &ring,
        );
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            WIDGET_AFFORDANCE_COLOUR,
            None,
            &ring,
        );
        if selected == Some(i) {
            let dot = Circle::new((x + 10.0, cy), 3.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                WIDGET_ACTIVE_COLOUR,
                None,
                &dot,
            );
        }
        crate::text::draw_text(
            scene,
            label,
            x + 24.0,
            cy + WIDGET_BASELINE_NUDGE,
            WIDGET_TEXT_SIZE,
            WIDGET_LABEL_COLOUR,
            crate::text::TextAnchor::Start,
        );
    }
}

/// Draw a resting `style: checkbox` widget: a rounded box with
/// a Maritime check glyph when checked, plus a label (the bound param's
/// name — widget `label:` rendering is its own polish item).
pub fn render_checkbox(
    scene: &mut Scene,
    x: f64,
    y: f64,
    _width: f64,
    height: f64,
    checked: bool,
    label: &str,
) {
    let cy = y + height / 2.0;
    let (bx0, by0) = (x + 4.0, cy - 7.0);
    let bx = RoundedRect::new(bx0, by0, bx0 + 14.0, by0 + 14.0, 3.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        WIDGET_FILL_COLOUR,
        None,
        &bx,
    );
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        WIDGET_BORDER_COLOUR,
        None,
        &bx,
    );
    if checked {
        let mut check = BezPath::new();
        check.move_to((bx0 + 3.0, by0 + 7.5));
        check.line_to((bx0 + 6.0, by0 + 10.5));
        check.line_to((bx0 + 11.0, by0 + 3.5));
        scene.stroke(
            &Stroke::new(2.0),
            Affine::IDENTITY,
            WIDGET_ACTIVE_COLOUR,
            None,
            &check,
        );
    }
    crate::text::draw_text(
        scene,
        label,
        bx0 + 22.0,
        cy + WIDGET_BASELINE_NUDGE,
        WIDGET_TEXT_SIZE,
        WIDGET_LABEL_COLOUR,
        crate::text::TextAnchor::Start,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelMap};
    use crate::layout::ChartLayout;
    use crate::mark::{BarRenderer, DotRenderer, GeoRenderer, LineRenderer};

    // render_slider draws exactly two shapes — the track
    // and the thumb — into the scene (headless proof the widget renders).
    #[test]
    fn render_slider_draws_track_and_thumb() {
        let mut scene = Scene::new();
        render_slider(&mut scene, 0.0, 400.0, 200.0, 32.0, 0.5);
        assert_eq!(
            crate::mark::count_scene_paths(&scene),
            2,
            "slider draws a track + a thumb"
        );
    }

    // the resting menu twin draws the box (fill +
    // border) + the chevron affordance, and real glyphs for the label.
    #[test]
    fn render_menu_draws_box_chevron_and_label() {
        let mut scene = Scene::new();
        render_menu(&mut scene, 0.0, 400.0, 200.0, 32.0, "east");
        assert_eq!(
            crate::mark::count_scene_paths(&scene),
            3,
            "box fill + box border + chevron"
        );
        assert_eq!(
            crate::mark::count_scene_glyphs(&scene),
            4,
            "the current-value label renders real glyphs"
        );
    }

    // the resting radio twin draws one 22px row per option —
    // ring (fill + outline) per row plus ONE filled dot on the selected row —
    // and label glyphs for every option.
    #[test]
    fn render_radio_draws_rows_and_selected_dot() {
        let labels: Vec<String> = ["circle", "square", "triangle"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut scene = Scene::new();
        render_radio(&mut scene, 0.0, 400.0, 200.0, &labels, Some(1));
        assert_eq!(
            crate::mark::count_scene_paths(&scene),
            3 * 2 + 1,
            "three rings (fill + outline) + one selected dot"
        );
        assert_eq!(
            crate::mark::count_scene_glyphs(&scene),
            "circlesquaretriangle".chars().count(),
            "every option label renders"
        );

        // No selection → no dot.
        let mut unselected = Scene::new();
        render_radio(&mut unselected, 0.0, 400.0, 200.0, &labels, None);
        assert_eq!(crate::mark::count_scene_paths(&unselected), 3 * 2);
    }

    // the checkbox twin draws box fill + border, PLUS the check
    // glyph exactly when checked; the label renders either way.
    #[test]
    fn render_checkbox_check_glyph_tracks_checked_state() {
        let mut checked = Scene::new();
        render_checkbox(&mut checked, 0.0, 400.0, 200.0, 32.0, true, "flag");
        let mut unchecked = Scene::new();
        render_checkbox(&mut unchecked, 0.0, 400.0, 200.0, 32.0, false, "flag");
        assert_eq!(
            crate::mark::count_scene_paths(&checked),
            3,
            "box fill + border + check stroke when checked"
        );
        assert_eq!(
            crate::mark::count_scene_paths(&unchecked),
            2,
            "no check stroke when unchecked"
        );
        assert_eq!(
            crate::mark::count_scene_glyphs(&checked),
            4,
            "the label renders"
        );
        assert_eq!(crate::mark::count_scene_glyphs(&unchecked), 4);
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
        let (with_legend, _) = build_multi_mark_scene(&[&data], true, &ResolvedTitles::default());
        let (without_legend, _) =
            build_multi_mark_scene(&[&data], false, &ResolvedTitles::default());
        let (n_with, n_without) = (
            crate::mark::count_scene_paths(&with_legend),
            crate::mark::count_scene_paths(&without_legend),
        );
        assert!(
            n_with > n_without,
            "suppressing the inline legend must drop its swatch/panel fills: {n_with} !> {n_without}"
        );
    }

    // a frame-suppressing mark (geo) drops the grid + axes for the
    // whole plot, so a geo basemap scene carries only the background + the
    // projected feature outline — no grid/axis ink — while a cartesian plot's
    // scene carries frame ink.
    #[test]
    fn frame_suppressed_for_geo_plot() {
        let square = r#"{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}"#;
        let geo_schema = Arc::new(Schema::new(vec![Field::new("geom", DataType::Utf8, true)]));
        let geo_batch =
            RecordBatch::try_new(geo_schema, vec![Arc::new(StringArray::from(vec![square]))])
                .unwrap();
        let geo_cm = ChannelMap::new(); // basemap — no fill channel
        let geo = GeoRenderer::default();
        let geo_data = ChartData {
            batch: &geo_batch,
            channel_map: &geo_cm,
            renderer: &geo,
            layout: ChartLayout::new(400.0, 300.0),
            view_extent: None,
            highlight: None,
        };
        let (geo_scene, _) = build_multi_mark_scene(&[&geo_data], true, &ResolvedTitles::default());
        let geo_paths = crate::mark::count_scene_paths(&geo_scene);

        // Contrast: the SAME plot rect + a single cartesian mark carries grid +
        // two axis lines + tick ink — the frame the geo plot omits. A single dot
        // (one circle) isolates the difference to the frame gating.
        let xy_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let xy_batch = RecordBatch::try_new(
            xy_schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(Float64Array::from(vec![10.0])),
            ],
        )
        .unwrap();
        let mut xy_cm = ChannelMap::new();
        xy_cm.insert(Channel::X, "x".to_string());
        xy_cm.insert(Channel::Y, "y".to_string());
        let dot = DotRenderer;
        let dot_data = ChartData {
            batch: &xy_batch,
            channel_map: &xy_cm,
            renderer: &dot,
            layout: ChartLayout::new(400.0, 300.0),
            view_extent: None,
            highlight: None,
        };
        let (dot_scene, _) = build_multi_mark_scene(&[&dot_data], true, &ResolvedTitles::default());
        // The geo plot (one outline, no frame) draws strictly fewer paths than a
        // one-mark cartesian plot (one circle + grid + axes) — the frame gating.
        assert!(
            crate::mark::count_scene_paths(&dot_scene) > geo_paths,
            "a framed cartesian plot carries grid + axis ink the geo plot omits"
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
        let (s0, _) = build_multi_mark_scene(&[&d0], true, &ResolvedTitles::default());
        let (s1, _) = build_multi_mark_scene(&[&d1], true, &ResolvedTitles::default());

        let two = compose_dashboard(600.0, 200.0, &[(0.0, 0.0, &s0), (300.0, 0.0, &s1)]);
        let one = compose_dashboard(600.0, 200.0, &[(0.0, 0.0, &s0)]);
        assert!(
            two.encoding().path_tags.len() > one.encoding().path_tags.len(),
            "two composed plots produce more geometry than one"
        );

        // An empty dashboard still paints its background.
        let empty = compose_dashboard(600.0, 200.0, &[]);
        assert!(
            !empty.encoding().path_tags.is_empty(),
            "dashboard background fills even with no plots"
        );
    }

    #[test]
    fn build_dot_chart_scene() {
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
            !encoding.path_tags.is_empty(),
            "dot chart scene should have content"
        );

        // Scales should include x, y, and fill.
        assert!(scales.get(Channel::X).is_some(), "x scale should exist");
        assert!(scales.get(Channel::Y).is_some(), "y scale should exist");
        assert!(
            scales.get(Channel::Fill).is_some(),
            "fill scale should exist"
        );
    }

    #[test]
    fn build_bar_chart_scene() {
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
            !encoding.path_tags.is_empty(),
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
    fn build_line_chart_scene() {
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
            !encoding.path_tags.is_empty(),
            "line chart scene should have content"
        );
    }

    #[test]
    fn view_extent_overrides_scale_domain() {
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
            layout,
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

    // --- build_multi_mark_scene ---

    #[test]
    fn multi_mark_scene_dot_and_line() {
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
            layout,
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

        let (scene, scales) =
            build_multi_mark_scene(&[&data1, &data2], true, &ResolvedTitles::default());

        // Scene should be non-empty.
        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "multi-mark scene should have content"
        );

        // Scales should span the union of both batches.
        let x = scales.get(Channel::X).expect("x scale should exist");
        assert!(
            (x.domain_min().unwrap() - 1.0).abs() < f64::EPSILON,
            "x min = union min"
        );
        assert!(
            (x.domain_max().unwrap() - 8.0).abs() < f64::EPSILON,
            "x max = union max"
        );

        let y = scales.get(Channel::Y).expect("y scale should exist");
        assert!(
            (y.domain_min().unwrap() - 10.0).abs() < f64::EPSILON,
            "y min = union min"
        );
        assert!(
            (y.domain_max().unwrap() - 80.0).abs() < f64::EPSILON,
            "y max = union max"
        );
    }

    #[test]
    fn multi_mark_scene_empty_entries() {
        let (scene, scales) = build_multi_mark_scene(&[], true, &ResolvedTitles::default());
        let encoding = scene.encoding();
        assert_eq!(encoding.path_tags.len(), 0, "empty entries => empty scene");
        assert!(scales.get(Channel::X).is_none());
    }

    #[test]
    fn titled_scene_carries_more_ink_and_anchored_reemits() {
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
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &dot,
            layout: ChartLayout::new(400.0, 300.0),
            view_extent: None,
            highlight: None,
        };
        let titles = ResolvedTitles {
            x: Some("x".into()),
            y: Some("y".into()),
            plot: Some("Weather".into()),
        };

        // A titled scene carries strictly more glyph ink than an untitled one.
        let (untitled, launch) = build_multi_mark_scene(&[&data], true, &ResolvedTitles::default());
        let (titled, _) = build_multi_mark_scene(&[&data], true, &titles);
        assert!(
            titled.encoding().draw_tags.len() > untitled.encoding().draw_tags.len(),
            "titles add glyph ink to the assembled scene"
        );

        // A widen-only anchored rebuild re-emits the same titles.
        let (anch_untitled, _) =
            build_multi_mark_scene_anchored(&[&data], true, &ResolvedTitles::default(), &launch);
        let (anch_titled, _) = build_multi_mark_scene_anchored(&[&data], true, &titles, &launch);
        assert!(
            anch_titled.encoding().draw_tags.len() > anch_untitled.encoding().draw_tags.len(),
            "an anchored rebuild re-emits titles"
        );
    }

    #[test]
    fn build_chart_scene_with_highlight() {
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
            otherwise: crate::mark::HighlightStyle {
                opacity: Some(0.15),
                ..Default::default()
            },
        };

        // With highlight
        let data = ChartData {
            batch: &batch,
            channel_map: &cm,
            renderer: &renderer,
            layout,
            view_extent: None,
            highlight: Some(&hs),
        };
        let (scene, _scales) = build_chart_scene(&data);
        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
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
            !encoding2.path_tags.is_empty(),
            "scene without highlight should also work"
        );
    }
}
