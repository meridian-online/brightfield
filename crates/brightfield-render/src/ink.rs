//! Meridian design-token boundary — the ONE place `meridian_design::Rgba`
//! converts to this crate's `peniko::Color` (design phase 4 PR B).
//!
//! The design crate is deliberately framework-neutral (its `Rgba` is neither
//! `peniko::Color` nor `gpui::Hsla`); consumers convert at their own boundary.
//! Every chart-ink constant in this crate goes through [`ink`] so a token
//! bump in the design crate propagates without hand-transcribed components.

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
}
