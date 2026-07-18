//! Boundary conversion from Meridian design tokens to GPUI colour types
//! (design phase 4 PR A).
//!
//! The design crate is deliberately framework-free — its `Rgba` is plain
//! sRGB data with straight alpha — so each consumer converts at its own
//! boundary (meridian-design ADR 0003). This module is that boundary for
//! the GPUI-side overlay quads: the brush rectangles and the keyboard
//! focus ring. Overlay colours are painted as GPUI quads only and never
//! enter the Vello scene, so a token change routed through here keeps the
//! example PNGs byte-identical (the design-adoption acceptance gate).
//!
//! Chart ink (colours inside the rendered scene) is out of scope here —
//! that migration is PR B's, over in `brightfield-render`.

use gpui::Hsla;

/// Convert a Meridian token colour (sRGB, straight alpha, 0–1) to a GPUI
/// colour, keeping the token's own alpha.
pub(crate) fn rgba(c: meridian_design::Rgba) -> Hsla {
    gpui::Rgba { r: c.r, g: c.g, b: c.b, a: c.a }.into()
}

#[cfg(test)]
mod tests {
    use super::rgba;
    use meridian_design::chrome::OVERLAY_LIGHT;

    /// The conversion is a straight component copy into `gpui::Rgba` before
    /// the Hsla conversion — pin it by round-tripping the focus-ring token
    /// (Maritime #4b7a9b, opaque) back out of the Hsla.
    #[test]
    fn token_conversion_round_trips_components() {
        let hsla = rgba(OVERLAY_LIGHT.focus_ring);
        let back: gpui::Rgba = hsla.into();
        assert!((back.r - OVERLAY_LIGHT.focus_ring.r).abs() < 1e-3);
        assert!((back.g - OVERLAY_LIGHT.focus_ring.g).abs() < 1e-3);
        assert!((back.b - OVERLAY_LIGHT.focus_ring.b).abs() < 1e-3);
        assert!((back.a - OVERLAY_LIGHT.focus_ring.a).abs() < 1e-3);
    }
}
