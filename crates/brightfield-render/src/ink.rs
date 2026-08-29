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
//! canvas.
//!
//! The last two drawing-path literals — `mark.rs`'s `HEXGRID_STROKE` and
//! `GEO_STROKE_COLOUR` — are now [`ChartInk::hexgrid_stroke`] and
//! [`ChartInk::geo_stroke`]. They were the pair the earlier threading left
//! behind because routing them meant *choosing* a dark colour rather than
//! resolving an existing pair, and a stroke-only basemap therefore sat at
//! 1.21:1 against the dark chart surface. Their light halves are still written
//! as components, below and nowhere else, because the light values ship today
//! and were not what that choice was about; their dark halves are design
//! tokens. `tests/mode_blind_ink.rs` is what stops a third one appearing: it
//! drives every renderer in `default_renderers()` in both modes and asks the
//! scene what was encoded, so a colour written as digits is caught the same way
//! a mis-bound token is.

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

/// The WCAG 2.1 relative luminance of an sRGB colour, ignoring alpha.
fn relative_luminance(c: Color) -> f64 {
    fn channel(v: f32) -> f64 {
        let v = f64::from(v);
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    let [r, g, b, _] = c.components;
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG 2.1 contrast ratio between two **opaque** sRGB colours: 1.0 for a
/// colour against itself, 21.0 for black against white.
///
/// Alpha is ignored, so a translucent paint has to be composited onto its
/// backdrop before it arrives here or the answer describes a colour nobody
/// sees. Every caller today passes an opaque paint or a pixel read back off a
/// render.
///
/// Public because the ratio a chart ink is CHOSEN for and the ratio a picture
/// is MEASURED at have to be the same arithmetic. `dark_mark_ink.rs` in
/// `brightfield-shell` reads real pixels out of a dark render and calls this;
/// if the expectation had its own private copy the two could agree with nothing.
#[must_use]
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
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
/// palettes, which is why the plot stayed a white slab inside a dark window: a
/// drawing path had no mode to ask. This struct is that same list, resolved
/// through the light or dark token as [`Self::for_mode`] is asked.
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
    /// Legend panel background: the chart surface at the private
    /// `legend::PANEL_ALPHA`, translucent so marks under it stay legible.
    /// Named rather than linked because that constant is `pub(crate)`, and a
    /// doc link does not get to widen an API.
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
    /// The wash over a committed selection's region, at the private
    /// `selection::WASH_ALPHA` — named rather than linked, as
    /// [`Self::legend_panel`] records.
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
    /// The decorative hexgrid mesh's stroke — the dataless lattice
    /// `HexgridRenderer` draws, and the hexbin's on-lattice sibling.
    ///
    /// Recessive on purpose: it is scaffolding under the data, not data. Light
    /// is the private `HEXGRID_STROKE_LIGHT` unchanged — named rather than
    /// linked, because a doc link does not get to widen an API — at 1.93:1 on
    /// the light surface;
    /// dark is warm gray step 8, which is where the light value already sat in
    /// that scale, and gives 2.95:1 on the dark surface.
    pub hexgrid_stroke: Color,
    /// A stroke-only geo basemap's outline — the case where the stroke IS the
    /// content, because a `mark: geo` with no `fill:` channel draws nothing
    /// else.
    ///
    /// Full-strength ink in both modes: light is the private `GEO_STROKE_LIGHT`
    /// unchanged (14.74:1 on the light surface), dark is `ink_primary`
    /// (15.84:1). Before
    /// this field existed the dark basemap drew the LIGHT literal on the dark
    /// surface at 1.21:1 — painted and invisible.
    pub geo_stroke: Color,
    /// The "Harbour" categorical order for this mode, as the raw component
    /// arrays [`crate::scale::Scale::Colour`] stores. The ORDER is the
    /// colourblind-safety mechanism and is therefore data, never cosmetic; both
    /// modes carry the same eight slots in the same order.
    pub categorical: &'static [[f32; 4]],
}

/// The hexgrid mesh's LIGHT stroke, byte-for-byte what `mark.rs`'s retired
/// `HEXGRID_STROKE` held.
///
/// Written as components rather than resolved from a token, deliberately. Its
/// nearest step twin is `scales::GRAY_LIGHT[7]` (#b7b3ae — warm gray step 8,
/// Radix's border band), within 10/255 on every channel and 2.03:1 against the
/// light surface where this is 1.93:1. Binding to it would move a shipping
/// light value, which is a separate decision from the dark one this pair exists
/// to make. The dark half below IS that token's dark twin, so the mapping is
/// recorded even though the light side is not yet taken from it.
const HEXGRID_STROKE_LIGHT: Color = Color::new([0.72, 0.72, 0.72, 1.0]);

/// The geo basemap outline's LIGHT stroke, byte-for-byte what `mark.rs`'s
/// retired `GEO_STROKE_COLOUR` held. Its nearest token is
/// `chrome::INK_LIGHT.ink_primary` (#231f1c against this #262626), whose dark
/// twin the dark half takes; the light side stays put for the reason
/// [`HEXGRID_STROKE_LIGHT`] gives.
const GEO_STROKE_LIGHT: Color = Color::new([0.15, 0.15, 0.15, 1.0]);

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
            hexgrid_stroke: if dark {
                ink(gray[7])
            } else {
                HEXGRID_STROKE_LIGHT
            },
            geo_stroke: if dark {
                ink(c.ink_primary)
            } else {
                GEO_STROKE_LIGHT
            },
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
    /// Light — the mode this renderer drew before it could be told one, so a
    /// [`crate::scale::ScaleSet`] nobody has taught is unchanged rather than
    /// broken. `the_light_canvas_draws_every_light_paint` in
    /// `tests/dark_canvas.rs` holds that this default is what the light scenes
    /// draw.
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
        // These two are transcribed from `mark.rs`'s retired `HEXGRID_STROKE`
        // and `GEO_STROKE_COLOUR` rather than compared to the consts above,
        // for the reason every other line in this test transcribes: an
        // assertion that reads the same constant the code reads is green on
        // any value at all, including one that moved. Caught by mutation —
        // rebinding `HEXGRID_STROKE_LIGHT` to its nearest design token left the
        // earlier form of this line green over a changed light value.
        assert_eq!(
            p.hexgrid_stroke,
            Color::new([0.72, 0.72, 0.72, 1.0]),
            "hexgrid_stroke"
        );
        assert_eq!(
            p.geo_stroke,
            Color::new([0.15, 0.15, 0.15, 1.0]),
            "geo_stroke"
        );
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
    /// the defect this card fixes is exactly one field left behind. Any pair
    /// byte-identical across the published scales would be named and excluded
    /// rather than silently passing; there are none in this palette today, so
    /// the list is empty and the assertion is total.
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
            ("hexgrid_stroke", l.hexgrid_stroke, d.hexgrid_stroke),
            ("geo_stroke", l.geo_stroke, d.geo_stroke),
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

    /// Black on white is 21:1 and a colour on itself is 1:1 — the two ends of
    /// the WCAG scale, so a sign error or a missing gamma step cannot pass.
    #[test]
    fn contrast_ratio_spans_the_wcag_range() {
        let black = Color::new([0.0, 0.0, 0.0, 1.0]);
        let white = Color::new([1.0, 1.0, 1.0, 1.0]);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
        assert!(
            (contrast_ratio(white, black) - 21.0).abs() < 1e-9,
            "symmetric"
        );
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
        // A mid gray separates the two implementations that both look right at
        // the ends: #777777 on white is 4.478:1 through the sRGB transfer
        // function and 2.032:1 through a linear one, and 21/1 is the same
        // either way.
        let mid = Color::new([119.0 / 255.0, 119.0 / 255.0, 119.0 / 255.0, 1.0]);
        let got = contrast_ratio(mid, white);
        assert!(
            (got - 4.478).abs() < 0.001,
            "#777777 against white is 4.478:1 by WCAG (2.032:1 if the sRGB \
             transfer function is skipped); got {got}"
        );
    }

    /// **The two marks this palette last routed clear a stated ratio in dark,
    /// and the literals they replaced do not.**
    ///
    /// Both bounds are DERIVED from paints already on this struct rather than
    /// typed, so they cannot be reverse-engineered from the values they judge:
    ///
    /// - a hexgrid mesh is scaffolding, so it must be at least as legible
    ///   against the dark surface as it is against the light one, and must stay
    ///   BELOW the default mark ink — data ink wins. The literal fails the
    ///   second: #b8b8b8 on the dark surface is 9.26:1 against the mark ink's
    ///   4.75:1, a bright white lattice laid over the data.
    /// - a stroke-only basemap IS the content, so it takes the WCAG AAA 7:1 and
    ///   must also be at least as legible as the light basemap. The literal
    ///   fails both: #262626 on the dark surface is 1.21:1.
    ///
    /// The second half of each pair is what stops this passing on the code it
    /// was written against: a test that only asserts the new value clears a
    /// floor is green on any value that happens to, including one nobody chose.
    #[test]
    fn the_dark_marks_clear_a_stated_contrast_and_the_literals_do_not() {
        let d = ChartInk::DARK;
        let l = ChartInk::LIGHT;

        let mesh = contrast_ratio(d.hexgrid_stroke, d.background);
        let mesh_light = contrast_ratio(l.hexgrid_stroke, l.background);
        let data_ink = contrast_ratio(d.mark_default, d.background);
        assert!(
            mesh >= mesh_light,
            "the dark hexgrid mesh is {mesh:.2}:1 on the dark surface, less \
             legible than the light mesh's {mesh_light:.2}:1 on the light one"
        );
        assert!(
            mesh < data_ink,
            "the dark hexgrid mesh is {mesh:.2}:1 against the default mark \
             ink's {data_ink:.2}:1 — decoration is not allowed to out-shout the \
             data it sits under"
        );

        let basemap = contrast_ratio(d.geo_stroke, d.background);
        let basemap_light = contrast_ratio(l.geo_stroke, l.background);
        assert!(
            basemap >= 7.0,
            "the dark basemap outline is {basemap:.2}:1; a stroke-only geo mark \
             is the whole of what the reader came to see, so it takes WCAG AAA"
        );
        assert!(
            basemap >= basemap_light,
            "the dark basemap outline is {basemap:.2}:1, weaker than the light \
             one's {basemap_light:.2}:1"
        );

        // The literals, judged by the same bounds on the same surface.
        let stale_mesh = contrast_ratio(l.hexgrid_stroke, d.background);
        assert!(
            stale_mesh >= data_ink,
            "the retired hexgrid literal is {stale_mesh:.2}:1 on the dark \
             surface, which no longer breaks the ceiling this test holds — the \
             bound has stopped distinguishing the fix from the defect"
        );
        let stale_basemap = contrast_ratio(l.geo_stroke, d.background);
        assert!(
            stale_basemap < 7.0,
            "the retired geo literal is {stale_basemap:.2}:1 on the dark \
             surface, which now clears the floor this test holds"
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
