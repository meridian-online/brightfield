//! Meridian design-token boundary — the ONE place `meridian_design::Rgba`
//! converts to this crate's `peniko::Color` (design phase 4 PR B).
//!
//! The design crate is deliberately framework-neutral (its `Rgba` is not
//! `peniko::Color`, nor any UI framework's colour type); consumers convert at
//! their own boundary.
//! Every chart-ink constant in this crate goes through [`ink`] so a token
//! bump in the design crate propagates without hand-transcribed components.
//!
//! [`ChartInk`] is the mode-resolved half of that boundary: the twenty-one
//! paints the chart canvas lays down, each read off the light or the dark
//! token, resolved once per plot and carried to the modules that draw with
//! them. It is the [`crate::asset_scene::AssetInk`] idea applied to the data
//! canvas — no colour on a drawing path is settled before the mode is asked.

use meridian_design::colour::Rgba;
use peniko::Color;

/// Convert a Meridian design token (sRGB, straight alpha) to a peniko colour
/// (also sRGB straight alpha) — a component-wise copy, `const` so token-derived
/// colours stay `const` like the hand-written ones they replace.
#[must_use]
pub const fn ink(c: Rgba) -> Color {
    Color::new([c.r, c.g, c.b, c.a])
}

/// [`ink`] with the token's alpha replaced — for surface-tinted translucent
/// panels (the legend background keeps its historical 0.85 alpha behaviour).
#[must_use]
pub const fn ink_with_alpha(c: Rgba, alpha: f32) -> Color {
    Color::new([c.r, c.g, c.b, alpha])
}

/// Convert a design-token palette (`[Rgba; N]`) to the raw `[f32; 4]`
/// component arrays the scale tables store — `const` so the Harbour palette
/// and the meridian ramp stay compile-time constants like the hand-written
/// tables they replace.
#[must_use]
pub const fn components<const N: usize>(src: [Rgba; N]) -> [[f32; 4]; N] {
    let mut out = [[0.0_f32; 4]; N];
    let mut i = 0;
    while i < N {
        out[i] = [src[i].r, src[i].g, src[i].b, src[i].a];
        i += 1;
    }
    out
}

/// Every colour the chart canvas paints, resolved for one mode.
///
/// The canvas used to hold twenty-one module-level `const Color`s read straight
/// off `chrome::INK_LIGHT`, `scales::GRAY_LIGHT` and the `viz` `*_LIGHT`
/// palettes, which is why the plot stayed a white slab inside a dark window:
/// nothing on a drawing path could see the mode. This struct is that same list,
/// resolved through the light or dark token as [`Self::for_mode`] is asked.
///
/// In light mode every field resolves to the byte-identical value its `const`
/// predecessor held — `light_resolves_to_the_retired_consts` pins that field by
/// field, which is what makes a light baseline that did not move evidence
/// rather than a coincidence.
///
/// It is `Copy` and lives on the [`crate::scale::ScaleSet`], so the mark
/// renderers and the colour scale reach it through the argument they already
/// take. The chrome modules (axis, grid, legend, selection, the widgets) take
/// it as a parameter, resolved once per plot by the scene builder.
///
/// # What is deliberately not here
///
/// The sequential ramps (viridis, turbo, the Meridian blue-240) and the
/// reserved status inks have one published value each and no mode twin — the
/// design crate says of `viz::STATUS` that it is "fixed across modes", and
/// `SEQUENTIAL_MERIDIAN` ships a single ramp. `sample_notice` is mode-invariant
/// by its own recorded decision. None of those are omissions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartInk {
    /// The chart surface the whole plot area is filled with, first, so grid,
    /// marks, axes and legend composite onto something opaque.
    pub background: Color,
    /// Gridline hairline.
    pub grid: Color,
    /// Tick marks. The recessive baseline ink — ticks sit back while the data
    /// ink carries the chart.
    pub tick: Color,
    /// The axis (domain) line — the same recessive baseline ink as the ticks.
    pub axis: Color,
    /// Tick labels and legend entry text — the muted ink.
    pub label: Color,
    /// Axis, plot and legend titles — the primary ink, darker (lighter, in
    /// dark) than the tick labels so a title reads as a heading.
    pub title: Color,
    /// Legend panel background: the chart surface at [`crate::legend::PANEL_ALPHA`],
    /// translucent so marks under it stay legible.
    pub legend_panel: Color,
    /// Legend panel border — the mode's gray step 4, a border-weight hairline.
    pub legend_border: Color,
    /// Border around the sequential legend's gradient bar, so a ramp anchored
    /// at the surface tone still reads against the panel.
    pub legend_bar_border: Color,
    /// The single-mark default fill — categorical slot 1.
    pub mark_default: Color,
    /// The fill for a row whose bound fill VALUE is genuinely NULL: a warm gray
    /// deliberately below the series chroma floor, so a NULL can never
    /// impersonate a scheme colour.
    pub null: Color,
    /// The wash over a committed selection's region, at
    /// [`crate::selection::WASH_ALPHA`].
    pub selection_wash: Color,
    /// The rule down each constrained edge of a committed selection — the same
    /// focus ink at full strength.
    pub selection_bound: Color,
    /// Slider track.
    pub slider_track: Color,
    /// Slider thumb — the focus ink.
    pub slider_thumb: Color,
    /// Widget (menu, radio, checkbox) fill.
    pub widget_fill: Color,
    /// Widget border.
    pub widget_border: Color,
    /// Widget label text.
    pub widget_label: Color,
    /// Widget affordance glyph (a menu's chevron) — the muted ink.
    pub widget_affordance: Color,
    /// The active/checked state of a widget — the focus ink.
    pub widget_active: Color,
    /// The "Harbour" categorical order for this mode, as the raw component
    /// arrays [`crate::scale::Scale::Colour`] stores. The ORDER is the
    /// colourblind-safety mechanism and is therefore data, never cosmetic; both
    /// modes carry the same eight slots in the same order.
    pub categorical: &'static [[f32; 4]],
}

/// The Harbour categorical order in light, as scale-table components.
const CATEGORICAL_LIGHT: [[f32; 4]; 8] = components(meridian_design::viz::CATEGORICAL_LIGHT);

/// The Harbour categorical order in dark, as scale-table components.
const CATEGORICAL_DARK: [[f32; 4]; 8] = components(meridian_design::viz::CATEGORICAL_DARK);

impl ChartInk {
    /// Resolve the canvas palette for a mode — `dark` is the same flag
    /// [`meridian_design::semantic()`] and [`crate::asset_scene::AssetInk::for_mode`]
    /// take, and shell callers pass `mode.is_dark()`.
    #[must_use]
    pub const fn for_mode(dark: bool) -> Self {
        // `chrome::InkTokens` is one struct with one field set in both modes, so
        // the two branches cannot drift into naming different slots: the same
        // field name is read either side of the `if`.
        let c = if dark {
            meridian_design::chrome::INK_DARK
        } else {
            meridian_design::chrome::INK_LIGHT
        };
        let gray = if dark {
            meridian_design::scales::GRAY_DARK
        } else {
            meridian_design::scales::GRAY_LIGHT
        };
        Self {
            background: ink(c.surface),
            grid: ink(c.gridline),
            tick: ink(c.baseline),
            axis: ink(c.baseline),
            label: ink(c.ink_muted),
            title: ink(c.ink_primary),
            legend_panel: ink_with_alpha(c.surface, crate::legend::PANEL_ALPHA),
            legend_border: ink(gray[3]),
            legend_bar_border: ink(c.baseline),
            mark_default: ink(if dark {
                meridian_design::viz::MARK_DEFAULT_DARK
            } else {
                meridian_design::viz::MARK_DEFAULT_LIGHT
            }),
            null: ink(if dark {
                meridian_design::viz::NULL_INK_DARK
            } else {
                meridian_design::viz::NULL_INK_LIGHT
            }),
            selection_wash: ink_with_alpha(c.focus, crate::selection::WASH_ALPHA),
            selection_bound: ink(c.focus),
            slider_track: ink(gray[4]),
            slider_thumb: ink(c.focus),
            widget_fill: ink(c.surface),
            widget_border: ink(gray[4]),
            widget_label: ink(c.ink_primary),
            widget_affordance: ink(c.ink_muted),
            widget_active: ink(c.focus),
            categorical: if dark {
                &CATEGORICAL_DARK
            } else {
                &CATEGORICAL_LIGHT
            },
        }
    }

    /// The light canvas, `const` — what a [`Default`] scale set carries and what
    /// the light baselines are recorded against.
    pub const LIGHT: Self = Self::for_mode(false);

    /// The dark canvas, `const`.
    pub const DARK: Self = Self::for_mode(true);
}

impl Default for ChartInk {
    /// Light. A [`crate::scale::ScaleSet`] nobody told the mode draws the mode
    /// this renderer has always drawn, so a caller that has not been taught the
    /// mode yet is unchanged rather than broken.
    fn default() -> Self {
        Self::LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conversion is a straight component copy — no colour-space maths.
    #[test]
    fn ink_copies_components_verbatim() {
        let c = ink(meridian_design::chrome::INK_LIGHT.focus);
        // Maritime #4b7a9b.
        assert!((c.components[0] - 0x4b as f32 / 255.0).abs() < 1e-6);
        assert!((c.components[1] - 0x7a as f32 / 255.0).abs() < 1e-6);
        assert!((c.components[2] - 0x9b as f32 / 255.0).abs() < 1e-6);
        assert!((c.components[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ink_with_alpha_overrides_alpha_only() {
        let c = ink_with_alpha(meridian_design::chrome::INK_LIGHT.surface, 0.85);
        assert!((c.components[3] - 0.85).abs() < 1e-6);
        assert!((c.components[0] - 0xfc as f32 / 255.0).abs() < 1e-6);
    }

    /// **Which token slot each canvas paint reads**, held one field at a time.
    ///
    /// The values on the right are the module-level `const`s this struct
    /// replaced, transcribed from the retired lines. The risk this catches is
    /// not a token bump — that propagates through [`ink`] on both sides — it is
    /// a field wired to the wrong SLOT: `background` reading `page`, `label`
    /// reading `ink_secondary`, `legend_border` reading gray step 5. Every one
    /// of those still compiles, still resolves to a plausible colour, and moves
    /// the chart.
    #[test]
    fn light_resolves_to_the_retired_consts() {
        use meridian_design::chrome::INK_LIGHT as L;
        use meridian_design::scales::GRAY_LIGHT as G;
        let p = ChartInk::for_mode(false);
        assert_eq!(p.background, ink(L.surface), "background");
        assert_eq!(p.grid, ink(L.gridline), "grid");
        assert_eq!(p.tick, ink(L.baseline), "tick");
        assert_eq!(p.axis, ink(L.baseline), "axis");
        assert_eq!(p.label, ink(L.ink_muted), "label");
        assert_eq!(p.title, ink(L.ink_primary), "title");
        assert_eq!(
            p.legend_panel,
            ink_with_alpha(L.surface, 0.85),
            "legend_panel"
        );
        assert_eq!(p.legend_border, ink(G[3]), "legend_border");
        assert_eq!(p.legend_bar_border, ink(L.baseline), "legend_bar_border");
        assert_eq!(
            p.mark_default,
            ink(meridian_design::viz::MARK_DEFAULT_LIGHT),
            "mark_default"
        );
        assert_eq!(p.null, ink(meridian_design::viz::NULL_INK_LIGHT), "null");
        assert_eq!(
            p.selection_wash,
            ink_with_alpha(L.focus, 0.14),
            "selection_wash"
        );
        assert_eq!(p.selection_bound, ink(L.focus), "selection_bound");
        assert_eq!(p.slider_track, ink(G[4]), "slider_track");
        assert_eq!(p.slider_thumb, ink(L.focus), "slider_thumb");
        assert_eq!(p.widget_fill, ink(L.surface), "widget_fill");
        assert_eq!(p.widget_border, ink(G[4]), "widget_border");
        assert_eq!(p.widget_label, ink(L.ink_primary), "widget_label");
        assert_eq!(p.widget_affordance, ink(L.ink_muted), "widget_affordance");
        assert_eq!(p.widget_active, ink(L.focus), "widget_active");
        assert_eq!(
            p.categorical,
            &components(meridian_design::viz::CATEGORICAL_LIGHT),
            "categorical"
        );
        assert_eq!(
            p,
            ChartInk::default(),
            "the default canvas is the light one"
        );
    }

    /// **Every plane and every ink moves with the mode.**
    ///
    /// Field by field, and deliberately not as a whole-struct `assert_ne!`: one
    /// field left on its light token would still make the struct differ, and
    /// the defect this card fixes is exactly one field left behind. The three
    /// pairs that are byte-identical across the published scales are named and
    /// excluded rather than silently passing — there are none in this palette
    /// today, so the list is empty and the assertion is total.
    #[test]
    fn dark_moves_every_paint_off_its_light_value() {
        let l = ChartInk::for_mode(false);
        let d = ChartInk::for_mode(true);
        for (name, light, dark) in [
            ("background", l.background, d.background),
            ("grid", l.grid, d.grid),
            ("tick", l.tick, d.tick),
            ("axis", l.axis, d.axis),
            ("label", l.label, d.label),
            ("title", l.title, d.title),
            ("legend_panel", l.legend_panel, d.legend_panel),
            ("legend_border", l.legend_border, d.legend_border),
            (
                "legend_bar_border",
                l.legend_bar_border,
                d.legend_bar_border,
            ),
            ("mark_default", l.mark_default, d.mark_default),
            ("null", l.null, d.null),
            ("selection_wash", l.selection_wash, d.selection_wash),
            ("selection_bound", l.selection_bound, d.selection_bound),
            ("slider_track", l.slider_track, d.slider_track),
            ("slider_thumb", l.slider_thumb, d.slider_thumb),
            ("widget_fill", l.widget_fill, d.widget_fill),
            ("widget_border", l.widget_border, d.widget_border),
            ("widget_label", l.widget_label, d.widget_label),
            (
                "widget_affordance",
                l.widget_affordance,
                d.widget_affordance,
            ),
            ("widget_active", l.widget_active, d.widget_active),
        ] {
            assert_ne!(
                light, dark,
                "{name} paints the same colour in dark as in light — it is \
                 still reading a light token, and the dark canvas draws it wrong"
            );
        }
        assert_ne!(
            l.categorical, d.categorical,
            "the categorical palette does not move with the mode"
        );
        assert_eq!(
            l.categorical.len(),
            d.categorical.len(),
            "the two modes carry a different number of Harbour slots, so a \
             category's colour would depend on the mode as well as its index"
        );
    }

    /// The dark surface is genuinely dark: the plot background a reader sees in
    /// dark mode must not be a near-white sheet, which is the defect itself.
    #[test]
    fn the_dark_background_is_dark() {
        let d = ChartInk::for_mode(true);
        let [r, g, b, _] = d.background.components;
        assert!(
            r < 0.25 && g < 0.25 && b < 0.25,
            "the dark chart surface is ({r}, {g}, {b}) — a light slab inside a \
             dark window is the whole of what this palette exists to stop"
        );
        let [lr, lg, lb, _] = ChartInk::for_mode(false).background.components;
        assert!(
            lr > 0.9 && lg > 0.9 && lb > 0.9,
            "the light chart surface is ({lr}, {lg}, {lb}), not the warm near-white it was"
        );
    }
}
