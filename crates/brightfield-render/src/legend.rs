//! Colour legend rendering — draws colour-to-value mapping swatches
//! with labels when a mark encodes a data column as fill colour.

use std::collections::BTreeSet;

use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::axis::format_number;
use crate::ink::ChartInk;
use crate::layout::ChartLayout;
use crate::scale::Scale;
use crate::text::{draw_text, measure_width, TextAnchor, LABEL_SIZE};

/// The legend panel's translucency — the historical 0.85, kept so the panel
/// stays legible over marks and grid.
///
/// The alpha stays here, with its reasoning; the surface it tints is the mode's
/// and lives on [`ChartInk::legend_panel`].
pub(crate) const PANEL_ALPHA: f32 = 0.85;

/// Swatch size in pixels.
const SWATCH_SIZE: f64 = 12.0;

/// Vertical spacing between legend entries.
const ENTRY_SPACING: f64 = 18.0;

/// Inset of the legend panel from the plot-area corner.
const LEGEND_INSET: f64 = 8.0;

/// Padding inside the legend panel.
const LEGEND_PADDING: f64 = 6.0;

/// Gap between a swatch and its label.
const LEGEND_LABEL_GAP: f64 = 4.0;

/// Gradient bar width in pixels (continuous legend).
const BAR_WIDTH: f64 = 14.0;

/// Gradient bar height in pixels (continuous legend).
const BAR_HEIGHT: f64 = 96.0;

/// Number of stacked quads sampling the ramp down the bar. Enough to read as a
/// smooth gradient without a gradient-brush dependency.
const BAR_SAMPLES: usize = 48;

/// The pixel size `(width, height)` a colour legend panel needs for the given
/// scale, or `None` for a non-colour / empty scale. Dispatches on the scale
/// kind: a categorical [`Scale::Colour`] sizes a swatch column, a continuous
/// [`Scale::Sequential`] sizes a gradient bar.
#[must_use]
pub fn colour_legend_size(colour_scale: &Scale) -> Option<(f64, f64)> {
    match colour_scale {
        Scale::Colour { .. } => swatch_legend_size(colour_scale),
        Scale::Sequential { .. } => sequential_legend_size(colour_scale),
        _ => None,
    }
}

/// The pixel size of the swatch column for a categorical [`Scale::Colour`].
fn swatch_legend_size(colour_scale: &Scale) -> Option<(f64, f64)> {
    let categories = match colour_scale {
        Scale::Colour { categories, .. } if !categories.is_empty() => categories,
        _ => return None,
    };
    let max_label = categories
        .iter()
        .map(|c| measure_width(c, LABEL_SIZE))
        .fold(0.0_f64, f64::max);
    let n = categories.len() as f64;
    let width = LEGEND_PADDING * 2.0 + SWATCH_SIZE + LEGEND_LABEL_GAP + max_label;
    let height = LEGEND_PADDING * 2.0 + (n - 1.0) * ENTRY_SPACING + SWATCH_SIZE;
    Some((width, height))
}

/// The clickable entry rects of a categorical swatch legend whose panel
/// top-left is at `(box_x, box_y)` — one rect per category, in category
/// order. Row `i` sits at `LEGEND_PADDING + i * ENTRY_SPACING` below the
/// panel top, is `SWATCH_SIZE` tall, and spans the swatch plus the gap and
/// the label's measured extent, so clicking the category text also hits.
///
/// Pure geometry mirroring `render_swatch_legend_at`'s layout from the
/// same private constants, so the hit-boxes cannot drift from the drawing.
/// Empty for a non-`Colour` (or empty) scale.
#[must_use]
pub fn swatch_entry_rects(box_x: f64, box_y: f64, colour_scale: &Scale) -> Vec<Rect> {
    let categories = match colour_scale {
        Scale::Colour { categories, .. } => categories,
        _ => return Vec::new(),
    };
    let legend_x = box_x + LEGEND_PADDING;
    let legend_y_start = box_y + LEGEND_PADDING;
    categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let y = legend_y_start + i as f64 * ENTRY_SPACING;
            let label_w = measure_width(cat, LABEL_SIZE);
            Rect::new(
                legend_x,
                y,
                legend_x + SWATCH_SIZE + LEGEND_LABEL_GAP + label_w,
                y + SWATCH_SIZE,
            )
        })
        .collect()
}

/// The pixel size `(width, height)` of the gradient bar for a continuous
/// [`Scale::Sequential`], sized to hold the bar plus its min/mid/max tick labels;
/// `None` for any other scale.
#[must_use]
pub fn sequential_legend_size(scale: &Scale) -> Option<(f64, f64)> {
    let (dmin, dmax) = match scale {
        Scale::Sequential {
            domain_min,
            domain_max,
            ..
        } => (*domain_min, *domain_max),
        _ => return None,
    };
    let max_label = [dmin, (dmin + dmax) / 2.0, dmax]
        .iter()
        .map(|v| measure_width(&format_number(*v), LABEL_SIZE))
        .fold(0.0_f64, f64::max);
    let width = LEGEND_PADDING * 2.0 + BAR_WIDTH + LEGEND_LABEL_GAP + max_label;
    let height = LEGEND_PADDING * 2.0 + BAR_HEIGHT;
    Some((width, height))
}

/// Render a colour legend for a mark's fill encoding, anchored inside the plot
/// area's top-right corner (the right margin is too narrow to hold the legend,
/// and an inside panel reads as intentional). This is the plot's *inline*
/// legend; a standalone `legend:` node uses [`render_colour_legend_at`].
pub fn render_colour_legend(
    scene: &mut Scene,
    layout: &ChartLayout,
    colour_scale: &Scale,
    ink: ChartInk,
) {
    let Some((box_width, _)) = colour_legend_size(colour_scale) else {
        return;
    };
    let box_x = layout.plot_x_end() - box_width - LEGEND_INSET;
    let box_y = layout.plot_y_start() + LEGEND_INSET;
    render_colour_legend_at(scene, box_x, box_y, colour_scale, ink);
}

/// Render a colour legend at `(box_x, box_y)`, dispatching on the scale kind: a
/// categorical [`Scale::Colour`] draws swatches, a continuous
/// [`Scale::Sequential`] draws a gradient bar. No-op for any other scale.
///
/// This is the single entry point both the inline and standalone paths call so
/// the swatch/bar choice lives in one place. Delegates to
/// [`render_colour_legend_at_selected`] with no active selection, so its output
/// is byte-identical to the pre-selected-state renderer.
pub fn render_colour_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    colour_scale: &Scale,
    ink: ChartInk,
) {
    render_colour_legend_at_selected(
        scene,
        box_x,
        box_y,
        colour_scale,
        &BTreeSet::new(),
        None,
        ink,
    );
}

/// Alpha multiplier applied to every legend entry that is NOT the selected one,
/// when a selection is active — dims the others so the active category reads as
/// highlighted. Applies to the swatch AND its label only;
/// the panel, border, entry rects, and sizes are untouched.
const UNSELECTED_ENTRY_ALPHA: f32 = 0.35;

/// Fraction by which a HOVERED legend entry is lightened toward white — the
/// pre-click hover affordance for a bound legend. Distinct from
/// both full strength and the [`UNSELECTED_ENTRY_ALPHA`] dim, and colour-only,
/// so the panel/entry geometry (and the path_data invariant) is
/// untouched.
const HOVER_LIGHTEN: f32 = 0.4;

/// Lighten a legend entry colour toward white by [`HOVER_LIGHTEN`] for the
/// hover hot-state, preserving alpha. Colour/alpha only — no new geometry.
fn hover_emphasis(colour: Color) -> Color {
    let [r, g, b, a] = colour.components;
    Color::new([
        r + (1.0 - r) * HOVER_LIGHTEN,
        g + (1.0 - g) * HOVER_LIGHTEN,
        b + (1.0 - b) * HOVER_LIGHTEN,
        a,
    ])
}

/// Render a colour legend at `(box_x, box_y)` with an optional selected entry
/// (selected-state). `selected` is the index of the categorical entry
/// drawn at full strength while every OTHER swatch and label dims to
/// the private `UNSELECTED_ENTRY_ALPHA`; `None` draws every entry at full
/// strength — byte-identical to the pre-selection output, since the `None`
/// path multiplies no alpha. Panel geometry, entry rects, and sizes are
/// untouched (selected state is colour/alpha only). Sequential (gradient)
/// legends have no discrete entries and ignore `selected`.
pub fn render_colour_legend_at_selected(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    colour_scale: &Scale,
    selected: &BTreeSet<usize>,
    hovered: Option<usize>,
    ink: ChartInk,
) {
    match colour_scale {
        Scale::Colour { .. } => {
            render_swatch_legend_at(scene, box_x, box_y, colour_scale, selected, hovered, ink)
        }
        Scale::Sequential { .. } => {
            render_sequential_legend_at(scene, box_x, box_y, colour_scale, ink)
        }
        _ => {}
    }
}

/// Render a categorical swatch legend with its panel's top-left corner at
/// `(box_x, box_y)`. Each entry is a coloured swatch and its category label.
/// No-op for a non-colour or empty scale. When `selected` is `Some(i)`, entry
/// `i` draws at full strength and every other swatch and label dims to
/// [`UNSELECTED_ENTRY_ALPHA`]; `None` draws every entry at full strength (no
/// alpha arithmetic, so the output matches the pre-selected-state renderer).
fn render_swatch_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    colour_scale: &Scale,
    selected: &BTreeSet<usize>,
    hovered: Option<usize>,
    ink: ChartInk,
) {
    let (categories, palette) = match colour_scale {
        Scale::Colour {
            categories,
            palette,
        } => (categories, palette),
        _ => return,
    };

    if categories.is_empty() {
        return;
    }

    let Some((box_width, box_height)) = swatch_legend_size(colour_scale) else {
        return;
    };

    // Translucent background panel + thin border for legibility over marks/grid.
    let panel = Rect::new(box_x, box_y, box_x + box_width, box_y + box_height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        ink.legend_panel,
        None,
        &panel,
    );
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        ink.legend_border,
        None,
        &panel,
    );

    let legend_x = box_x + LEGEND_PADDING;
    let legend_y_start = box_y + LEGEND_PADDING;

    for (i, (cat, colour)) in categories.iter().zip(palette.iter().cycle()).enumerate() {
        let y = legend_y_start + i as f64 * ENTRY_SPACING;

        // Emphasis precedence, colour/alpha only (no geometry change): the
        // HOVERED entry brightens toward white (the pre-click
        // affordance); otherwise, when a selection is active, every NON-member
        // dims to UNSELECTED_ENTRY_ALPHA; otherwise full strength. With no hover
        // and an empty selection nothing changes — byte-identical to the plain
        // renderer.
        let dim = !selected.is_empty() && !selected.contains(&i);
        let hot = hovered == Some(i);
        let swatch_colour = if hot {
            hover_emphasis(Color::new(*colour))
        } else if dim {
            Color::new(*colour).multiply_alpha(UNSELECTED_ENTRY_ALPHA)
        } else {
            Color::new(*colour)
        };
        let label_colour = if hot {
            hover_emphasis(ink.label)
        } else if dim {
            ink.label.multiply_alpha(UNSELECTED_ENTRY_ALPHA)
        } else {
            ink.label
        };

        // Colour swatch.
        let swatch = Rect::new(legend_x, y, legend_x + SWATCH_SIZE, y + SWATCH_SIZE);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            swatch_colour,
            None,
            &swatch,
        );

        // Entry label, vertically centred on the swatch.
        draw_text(
            scene,
            cat,
            legend_x + SWATCH_SIZE + LEGEND_LABEL_GAP,
            y + SWATCH_SIZE * 0.5 + f64::from(LABEL_SIZE) / 3.0,
            LABEL_SIZE,
            label_colour,
            TextAnchor::Start,
        );
    }
}

/// Render a continuous gradient-bar legend for a [`Scale::Sequential`], panel
/// top-left at `(box_x, box_y)`. The bar is a vertical ramp — high value at the
/// top — drawn as `BAR_SAMPLES` stacked sampled quads (no gradient-brush
/// dependency), with min / mid / max numeric tick labels beside it read from the
/// scale's domain extent. No-op for any non-Sequential scale.
pub fn render_sequential_legend_at(
    scene: &mut Scene,
    box_x: f64,
    box_y: f64,
    scale: &Scale,
    ink: ChartInk,
) {
    let (dmin, dmax) = match scale {
        Scale::Sequential {
            domain_min,
            domain_max,
            ..
        } => (*domain_min, *domain_max),
        _ => return,
    };
    let Some((box_width, box_height)) = sequential_legend_size(scale) else {
        return;
    };

    // Translucent background panel + thin border, matching the swatch legend.
    let panel = Rect::new(box_x, box_y, box_x + box_width, box_y + box_height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        ink.legend_panel,
        None,
        &panel,
    );
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        ink.legend_border,
        None,
        &panel,
    );

    let bar_x = box_x + LEGEND_PADDING;
    let bar_top = box_y + LEGEND_PADDING;
    let slice_h = BAR_HEIGHT / BAR_SAMPLES as f64;

    // Stack sampled quads top (max) → bottom (min). Each slice samples the ramp
    // at its centre; the top slice reads the ramp's high end.
    for s in 0..BAR_SAMPLES {
        let y = bar_top + s as f64 * slice_h;
        let t = 1.0 - (s as f64 + 0.5) / BAR_SAMPLES as f64;
        let value = dmin + t * (dmax - dmin);
        let quad = Rect::new(bar_x, y, bar_x + BAR_WIDTH, y + slice_h + 0.5);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new(scale.map_continuous(value)),
            None,
            &quad,
        );
    }

    // Thin border around the bar so a light-anchored ramp reads against the panel.
    let bar = Rect::new(bar_x, bar_top, bar_x + BAR_WIDTH, bar_top + BAR_HEIGHT);
    scene.stroke(
        &kurbo::Stroke::new(0.5),
        Affine::IDENTITY,
        ink.legend_bar_border,
        None,
        &bar,
    );

    // Min / mid / max tick labels beside the bar (max at top, min at bottom).
    let label_x = bar_x + BAR_WIDTH + LEGEND_LABEL_GAP;
    let baseline_nudge = f64::from(LABEL_SIZE) / 3.0;
    for (frac, value) in [(0.0, dmax), (0.5, (dmin + dmax) / 2.0), (1.0, dmin)] {
        draw_text(
            scene,
            &format_number(value),
            label_x,
            bar_top + frac * BAR_HEIGHT + baseline_nudge,
            LABEL_SIZE,
            ink.label,
            TextAnchor::Start,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ChartLayout;
    use crate::scale::Scale;

    #[test]
    fn colour_legend_4_categories() {
        let layout = ChartLayout::new(800.0, 480.0);
        let colour_scale = Scale::Colour {
            categories: vec![
                "cat_a".to_string(),
                "cat_b".to_string(),
                "cat_c".to_string(),
                "cat_d".to_string(),
            ],
            palette: vec![
                [0.306, 0.475, 0.655, 1.0],
                [0.949, 0.557, 0.169, 1.0],
                [0.882, 0.341, 0.349, 1.0],
                [0.463, 0.718, 0.698, 1.0],
            ],
        };

        let mut scene = Scene::new();
        render_colour_legend(&mut scene, &layout, &colour_scale, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "legend should produce scene content for 4 categories"
        );
    }

    #[test]
    fn legend_skips_non_colour_scale() {
        let layout = ChartLayout::new(800.0, 480.0);
        let linear_scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };

        let mut scene = Scene::new();
        render_colour_legend(&mut scene, &layout, &linear_scale, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert_eq!(
            encoding.path_tags.len(),
            0,
            "legend should not render for non-colour scale"
        );
    }

    fn colour_scale_3() -> Scale {
        Scale::Colour {
            categories: vec!["a".to_string(), "bb".to_string(), "ccc".to_string()],
            palette: vec![
                [0.3, 0.4, 0.6, 1.0],
                [0.9, 0.5, 0.1, 1.0],
                [0.8, 0.3, 0.3, 1.0],
            ],
        }
    }

    // Standalone-legend hosting: the positioned variant draws content at an
    // arbitrary origin (its layout rect), independent of any plot.
    #[test]
    fn standalone_legend_at_origin_draws_content() {
        let mut scene = Scene::new();
        render_colour_legend_at(&mut scene, 640.0, 40.0, &colour_scale_3(), ChartInk::LIGHT);
        assert!(
            !scene.encoding().path_tags.is_empty(),
            "positioned legend should draw swatches + panel at its origin"
        );
    }

    #[test]
    fn standalone_legend_at_skips_non_colour_scale() {
        let mut scene = Scene::new();
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        render_colour_legend_at(&mut scene, 10.0, 10.0, &linear, ChartInk::LIGHT);
        assert_eq!(
            scene.encoding().path_tags.len(),
            0,
            "no content for a non-colour scale"
        );
    }

    #[test]
    fn colour_legend_size_scales_with_entries() {
        let (w, h) = colour_legend_size(&colour_scale_3()).expect("colour scale has a size");
        assert!(w > 0.0 && h > 0.0);
        // Height grows with entry count; 3 entries taller than 1.
        let one = Scale::Colour {
            categories: vec!["a".to_string()],
            palette: vec![[0.3, 0.4, 0.6, 1.0]],
        };
        let (_, h1) = colour_legend_size(&one).unwrap();
        assert!(h > h1, "3-entry legend is taller than 1-entry");
        assert!(colour_legend_size(&one).is_some());
    }

    // --- continuous gradient-bar legend ---

    fn sequential_scale() -> Scale {
        Scale::Sequential {
            domain_min: 0.0,
            domain_max: 42.0,
            stops: vec![
                [0.0, 0.0, 0.0, 1.0],
                [0.5, 0.5, 0.5, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            ],
        }
    }

    #[test]
    fn sequential_legend_draws_bar_and_dispatches() {
        // The bar (sampled quads + border) produces scene content.
        let mut scene = Scene::new();
        render_sequential_legend_at(&mut scene, 40.0, 40.0, &sequential_scale(), ChartInk::LIGHT);
        assert!(
            !scene.encoding().path_tags.is_empty(),
            "gradient bar should draw sampled quads at its origin"
        );

        // It is a no-op for a non-Sequential scale.
        let mut scene2 = Scene::new();
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        render_sequential_legend_at(&mut scene2, 10.0, 10.0, &linear, ChartInk::LIGHT);
        assert_eq!(
            scene2.encoding().path_tags.len(),
            0,
            "no bar for a linear scale"
        );

        // sequential_legend_size is Some for Sequential, None for Band.
        assert!(sequential_legend_size(&sequential_scale()).is_some());
        let band = Scale::Band {
            categories: vec!["a".into()],
            range_start: 0.0,
            range_end: 10.0,
            padding: 0.1,
        };
        assert!(sequential_legend_size(&band).is_none());

        // colour_legend_size dispatches a Sequential to the bar size.
        assert_eq!(
            colour_legend_size(&sequential_scale()),
            sequential_legend_size(&sequential_scale()),
            "colour_legend_size routes Sequential to the gradient-bar size"
        );

        // The shared entry point routes a Sequential to the bar (draws content).
        let mut scene3 = Scene::new();
        render_colour_legend_at(
            &mut scene3,
            40.0,
            40.0,
            &sequential_scale(),
            ChartInk::LIGHT,
        );
        assert!(
            !scene3.encoding().path_tags.is_empty(),
            "dispatch draws the bar"
        );
    }

    // --- swatch entry hit-geometry ---

    /// `swatch_entry_rects` yields one rect per category at exactly
    /// the positions the layout constants place the drawn swatches — row i at
    /// padding + i*ENTRY_SPACING, SWATCH_SIZE tall, spanning swatch + gap +
    /// label extent — and is empty for Sequential/Band scales.
    #[test]
    fn swatch_entry_rects_match_rendered_layout() {
        let scale = colour_scale_3(); // categories "a", "bb", "ccc"
        let (box_x, box_y) = (640.0, 40.0);
        let rects = swatch_entry_rects(box_x, box_y, &scale);
        assert_eq!(rects.len(), 3, "one rect per category");

        for (i, (rect, cat)) in rects.iter().zip(["a", "bb", "ccc"]).enumerate() {
            // Same arithmetic as render_swatch_legend_at: the row origin.
            let x0 = box_x + LEGEND_PADDING;
            let y0 = box_y + LEGEND_PADDING + i as f64 * ENTRY_SPACING;
            assert_eq!((rect.x0, rect.y0), (x0, y0), "entry {i} origin");
            assert_eq!(rect.height(), SWATCH_SIZE, "entry {i} is swatch-height");
            assert_eq!(
                rect.width(),
                SWATCH_SIZE + LEGEND_LABEL_GAP + measure_width(cat, LABEL_SIZE),
                "entry {i} spans swatch + gap + label extent"
            );
        }
        // Wider label → wider clickable rect (the label extent is included).
        assert!(
            rects[2].width() > rects[0].width(),
            "\"ccc\" entry wider than \"a\""
        );

        // Rows are ENTRY_SPACING apart, so consecutive rects never overlap
        // (SWATCH_SIZE < ENTRY_SPACING) — a click resolves to at most one entry.
        assert!(rects[0].y1 < rects[1].y0, "rows are disjoint");

        // Non-Colour scales have no discrete entries: empty.
        assert!(swatch_entry_rects(0.0, 0.0, &sequential_scale()).is_empty());
        let band = Scale::Band {
            categories: vec!["a".into()],
            range_start: 0.0,
            range_end: 10.0,
            padding: 0.1,
        };
        assert!(swatch_entry_rects(0.0, 0.0, &band).is_empty());
    }

    // --- selected-state rendering ---

    /// a bound categorical legend with an active selection draws the
    /// selected entry at full strength and dims every other swatch and label —
    /// changing the encoded COLOURS but not the geometry. The `None` path is
    /// checked against independently-computed packed colours (not against the
    /// delegating `render_colour_legend_at`, which would be circular).
    #[test]
    fn selected_state_dims_others_without_moving_geometry() {
        let scale = colour_scale_3(); // "a", "bb", "ccc"

        let mut none = Scene::new();
        render_colour_legend_at_selected(
            &mut none,
            40.0,
            40.0,
            &scale,
            &BTreeSet::new(),
            None,
            ChartInk::LIGHT,
        );
        let mut selected = Scene::new();
        let sel_set = BTreeSet::from([1usize]);
        render_colour_legend_at_selected(
            &mut selected,
            40.0,
            40.0,
            &scale,
            &sel_set,
            None,
            ChartInk::LIGHT,
        );
        // --- Non-delegating oracle (F5) ---
        // `render_colour_legend_at`'s `None` path merely forwards here, so
        // comparing the two would be circular and prove nothing. Instead compute
        // the packed colours vello encodes into `draw_data` independently and
        // assert their presence / absence, so a regression in the dim arithmetic
        // (or a stray dim leaking onto the `None` path) is actually caught.
        let pack = |c: Color| c.premultiply().to_rgba8().to_u32();
        let palette = match &scale {
            Scale::Colour { palette, .. } => palette.clone(),
            _ => unreachable!("colour_scale_3 is a Colour scale"),
        };
        let none_data = &none.encoding().draw_data;

        // No selection: every swatch and the shared label colour appear at FULL
        // strength, and no 0.35-dimmed variant is anywhere in the buffer.
        for c in &palette {
            assert!(
                none_data.contains(&pack(Color::new(*c))),
                "None draws swatch {c:?} at full strength"
            );
            assert!(
                !none_data.contains(&pack(Color::new(*c).multiply_alpha(UNSELECTED_ENTRY_ALPHA))),
                "None never encodes a dimmed swatch"
            );
        }
        assert!(
            none_data.contains(&pack(ChartInk::LIGHT.label)),
            "None draws labels at full strength"
        );
        assert!(
            !none_data.contains(&pack(
                ChartInk::LIGHT.label.multiply_alpha(UNSELECTED_ENTRY_ALPHA)
            )),
            "None never encodes a dimmed label"
        );

        // Selecting entry 1 ("bb"): it stays full while entries 0 and 2 dim to
        // 0.35, and the dimmed label colour now appears (it never did with None).
        let sel_data = &selected.encoding().draw_data;
        assert!(
            sel_data.contains(&pack(Color::new(palette[1]))),
            "the selected swatch keeps full strength"
        );
        for i in [0usize, 2] {
            assert!(
                sel_data.contains(&pack(
                    Color::new(palette[i]).multiply_alpha(UNSELECTED_ENTRY_ALPHA)
                )),
                "non-selected swatch {i} dims to {UNSELECTED_ENTRY_ALPHA}"
            );
        }
        assert!(
            sel_data.contains(&pack(
                ChartInk::LIGHT.label.multiply_alpha(UNSELECTED_ENTRY_ALPHA)
            )),
            "non-selected labels dim to {UNSELECTED_ENTRY_ALPHA}"
        );

        // Dimming changes colour only — the geometry is untouched: same shape
        // count, same coordinates, so panel size and entry rects hold still.
        assert_ne!(
            *none_data, *sel_data,
            "an active selection changes the entry colours (dimmed swatches + labels)"
        );
        assert_eq!(
            none.encoding().n_paths,
            selected.encoding().n_paths,
            "dimming changes colour only, not the number of drawn shapes"
        );
        assert_eq!(
            none.encoding().path_data,
            selected.encoding().path_data,
            "panel / swatch / entry-rect geometry is unchanged under selection"
        );

        // Hover hot-state: hovering entry 0 with NO selection active
        // encodes a NEW emphasis colour — in neither the full-strength nor the
        // 0.35-dim sets — and changes colour only, not geometry.
        let mut hover = Scene::new();
        render_colour_legend_at_selected(
            &mut hover,
            40.0,
            40.0,
            &scale,
            &BTreeSet::new(),
            Some(0),
            ChartInk::LIGHT,
        );
        let hover_data = &hover.encoding().draw_data;
        let emphasis0 = pack(hover_emphasis(Color::new(palette[0])));
        assert!(
            hover_data.contains(&emphasis0),
            "the hovered swatch encodes the lightened emphasis colour"
        );
        assert!(
            !none_data.contains(&emphasis0),
            "the emphasis colour never appears without hover"
        );
        assert!(
            !hover_data.contains(&pack(
                Color::new(palette[0]).multiply_alpha(UNSELECTED_ENTRY_ALPHA)
            )),
            "hover emphasis is distinct from the 0.35 dim"
        );
        assert_eq!(
            none.encoding().path_data,
            hover.encoding().path_data,
            "hover changes colour only, not geometry"
        );

        // A Sequential legend has no discrete entries — `selected` is inert and
        // its output matches the plain gradient bar.
        let mut seq_none = Scene::new();
        render_colour_legend_at_selected(
            &mut seq_none,
            40.0,
            40.0,
            &sequential_scale(),
            &BTreeSet::new(),
            None,
            ChartInk::LIGHT,
        );
        let mut seq_sel = Scene::new();
        render_colour_legend_at_selected(
            &mut seq_sel,
            40.0,
            40.0,
            &sequential_scale(),
            &BTreeSet::from([0usize]),
            None,
            ChartInk::LIGHT,
        );
        assert_eq!(
            seq_none.encoding().draw_data,
            seq_sel.encoding().draw_data,
            "a gradient legend ignores selection"
        );
    }
}
