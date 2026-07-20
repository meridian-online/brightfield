//! Standalone-legend scene construction — gpui-free.
//!
//! Builds the vello scene a hosted [`crate::legend_element::LegendElement`]
//! paints, by REUSING the composite path's positioned legend renderers
//! (`brightfield-render/src/legend.rs` is a read-only seam — never forked):
//! a categorical [`Scale::Colour`] draws the swatch column via
//! [`render_colour_legend_at`], a continuous [`Scale::Sequential`] draws the
//! gradient bar via [`render_sequential_legend_at`], each sized by its
//! matching size function. Same code, same pixels as the headless PNG.
//!
//! No gpui import may enter this file (the semantic-layer rule — the GPUI
//! wrapper lives in `legend_element.rs`).

use std::collections::BTreeSet;

use vello::Scene;

use brightfield_render::legend::{
    colour_legend_size, render_colour_legend_at_selected, render_sequential_legend_at,
    sequential_legend_size,
};
use brightfield_render::scale::Scale;

/// Build the scene for one standalone legend, drawn at origin (0, 0) so the
/// hosting element positions it purely by its layout rect. Returns the scene
/// and the content size `(width, height)` the size functions report, or
/// `None` for a scale no legend renderer draws (non-colour scales).
///
/// `selected` is the set of currently-active categories of a bound categorical
/// legend (selected-state, extended to a multi-select union): each member
/// entry draws at full strength while the rest dim. `hovered`
/// is the entry index under the pointer (the pre-click hover
/// affordance), lightened distinctly. An empty set + `None` hover — no
/// selection, an unbound legend, or a Sequential legend — draws every entry at
/// full strength, byte-identical to the plain renderer.
#[must_use]
pub fn build_legend_scene(
    scale: &Scale,
    selected: &BTreeSet<String>,
    hovered: Option<usize>,
) -> Option<(Scene, (f64, f64))> {
    let mut scene = Scene::new();
    let size = match scale {
        Scale::Colour { categories, .. } => {
            let size = colour_legend_size(scale)?;
            // Map the selected category NAMES to their entry indices — names
            // that appear in no category of this legend are dropped.
            let selected_indices: BTreeSet<usize> = selected
                .iter()
                .filter_map(|s| categories.iter().position(|c| c == s))
                .collect();
            render_colour_legend_at_selected(
                &mut scene,
                0.0,
                0.0,
                scale,
                &selected_indices,
                hovered,
            );
            size
        }
        Scale::Sequential { .. } => {
            let size = sequential_legend_size(scale)?;
            render_sequential_legend_at(&mut scene, 0.0, 0.0, scale);
            size
        }
        _ => return None,
    };
    Some((scene, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_render::scale::SequentialScheme;

    fn categorical() -> Scale {
        Scale::Colour {
            categories: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
            palette: vec![
                [0.306, 0.475, 0.655, 1.0],
                [0.949, 0.557, 0.169, 1.0],
                [0.882, 0.341, 0.349, 1.0],
            ],
        }
    }

    fn sequential() -> Scale {
        Scale::Sequential {
            domain_min: 0.0,
            domain_max: 9.0,
            stops: SequentialScheme::Viridis.stops(),
        }
    }

    /// scene construction is a plain function of the scale — the
    /// categorical scale yields swatch content, the sequential scale yields
    /// the gradient bar, and the two scenes differ (swatches vs bar). This
    /// module imports no gpui, so the construction compiles gpui-free.
    #[test]
    fn legend_scenes_swatches_vs_gradient_bar() {
        let (swatches, swatch_size) = build_legend_scene(&categorical(), &BTreeSet::new(), None)
            .expect("categorical scale builds a scene");
        assert!(
            !swatches.encoding().path_tags.is_empty(),
            "swatch legend draws content"
        );
        let expected = colour_legend_size(&categorical()).unwrap();
        assert_eq!(swatch_size, expected, "sized by colour_legend_size");

        let (bar, bar_size) = build_legend_scene(&sequential(), &BTreeSet::new(), None)
            .expect("sequential scale builds a scene");
        assert!(
            !bar.encoding().path_tags.is_empty(),
            "gradient bar draws content"
        );
        let expected = sequential_legend_size(&sequential()).unwrap();
        assert_eq!(bar_size, expected, "sized by sequential_legend_size");

        // Swatches vs gradient bar: 48 sampled bar quads dwarf 3 swatches, so
        // the encodings must differ — proof the dispatch draws different marks.
        assert_ne!(
            swatches.encoding().path_tags.len(),
            bar.encoding().path_tags.len(),
            "categorical and sequential legends draw different content"
        );

        // A non-colour scale draws nothing.
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };
        assert!(
            build_legend_scene(&linear, &BTreeSet::new(), None).is_none(),
            "no legend for a linear scale"
        );
    }
}
