//! Whether a render put anything on its target — read from the frame, not
//! inferred from the scene.
//!
//! Vello can return `Ok` from a render whose output carries no picture. Its
//! flattening and coarse-raster buffers are fixed sizes chosen inside
//! `vello_encoding`; when one overflows, coarse emits nothing, the overflow is
//! recorded in a GPU-side counter, and the render reports success. So a caller
//! that judges a frame by what its scene *asked* for cannot separate a frame
//! that drew quickly from one that drew nothing:
//! `crates/brightfield-render/tests/vello_bump_ceiling.rs` measures where the
//! overflow starts for a dot scatter, and the frames past it come back empty
//! with no error anywhere.
//!
//! [`FrameInk`] answers from the pixels the render was read back into.
//! [`VelloRenderer::frame_ink`](crate::vello_renderer::VelloRenderer::frame_ink)
//! renders a scene and counts what came out of it. A count is not a diagnosis:
//! it says what the target holds and names no cause, which is the probe's job.

use peniko::Color;

/// What a render put on its target, counted from the read-back pixels.
///
/// Built by [`FrameInk::measure`] over a tightly-packed RGBA8 buffer — the
/// shape
/// [`VelloRenderer::render_to_pixels`](crate::vello_renderer::VelloRenderer::render_to_pixels)
/// returns. Trailing bytes that do not complete a pixel are not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInk {
    inked_pixels: u64,
    total_pixels: u64,
    uniform: bool,
}

impl FrameInk {
    /// Count the pixels of `pixels` that differ from `base` — the colour the
    /// render was told to clear its target to.
    ///
    /// `base` is compared in its 8-bit encoding, which is exact for the two
    /// bases the render paths in this crate use (`TRANSPARENT` and a page
    /// tone whose components are already 8-bit). [`Self::is_uniform`] is the
    /// cross-check for a base that does not round cleanly: it needs no colour
    /// at all.
    #[must_use]
    pub fn measure(pixels: &[u8], base: Color) -> Self {
        let base_px = base.to_rgba8().to_u8_array();
        let mut inked_pixels = 0_u64;
        let mut total_pixels = 0_u64;
        let mut first: Option<[u8; 4]> = None;
        let mut uniform = true;
        for chunk in pixels.chunks_exact(4) {
            let px = [chunk[0], chunk[1], chunk[2], chunk[3]];
            total_pixels += 1;
            if px != base_px {
                inked_pixels += 1;
            }
            match first {
                None => first = Some(px),
                Some(f) => uniform &= px == f,
            }
        }
        Self {
            inked_pixels,
            total_pixels,
            uniform,
        }
    }

    /// Whether the render put anything on the target the clear did not.
    ///
    /// This is the verdict a caller wants: `false` says the frame it is
    /// holding is empty, whatever the scene handed in asked for.
    #[must_use]
    pub fn drew_ink(self) -> bool {
        self.inked_pixels > 0
    }

    /// How many pixels differ from the base colour.
    #[must_use]
    pub fn inked_pixels(self) -> u64 {
        self.inked_pixels
    }

    /// How many whole pixels the measured buffer held.
    #[must_use]
    pub fn total_pixels(self) -> u64 {
        self.total_pixels
    }

    /// Inked pixels as a fraction of the buffer; `0.0` for an empty buffer.
    #[must_use]
    pub fn inked_fraction(self) -> f64 {
        if self.total_pixels == 0 {
            0.0
        } else {
            self.inked_pixels as f64 / self.total_pixels as f64
        }
    }

    /// Whether the buffer holds one repeated pixel value.
    ///
    /// The base-free reading of the same frame: a target that was cleared and
    /// then written by nothing holds one value edge to edge, and that is true
    /// however the clear colour rounded on the way to the texture. A frame
    /// carrying a picture holds at least two values unless the picture covers
    /// the target in a single flat colour.
    #[must_use]
    pub fn is_uniform(self) -> bool {
        self.uniform
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 8;
    const H: usize = 4;

    fn filled(px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take(W * H * 4).collect()
    }

    #[test]
    fn a_target_holding_nothing_but_the_clear_drew_no_ink() {
        let ink = FrameInk::measure(&filled([0, 0, 0, 0]), Color::TRANSPARENT);
        assert!(!ink.drew_ink());
        assert_eq!(ink.inked_pixels(), 0);
        assert_eq!(ink.total_pixels(), (W * H) as u64);
        assert!(ink.is_uniform());
        assert!((ink.inked_fraction() - 0.0).abs() < f64::EPSILON);
    }

    /// One pixel is enough. The detector's whole job is separating "empty"
    /// from "not empty", so the threshold has to sit at the first pixel.
    #[test]
    fn a_single_pixel_of_ink_is_detected() {
        let mut pixels = filled([0, 0, 0, 0]);
        pixels[4 * 5] = 1;
        let ink = FrameInk::measure(&pixels, Color::TRANSPARENT);
        assert!(ink.drew_ink());
        assert_eq!(ink.inked_pixels(), 1);
        assert!(!ink.is_uniform());
    }

    /// The base is a parameter, not a constant: a frame cleared to white and
    /// left alone is as empty as one cleared to transparent.
    #[test]
    fn the_base_colour_is_what_ink_is_counted_against() {
        let white = filled([255, 255, 255, 255]);
        assert!(!FrameInk::measure(&white, Color::WHITE).drew_ink());
        assert_eq!(
            FrameInk::measure(&white, Color::TRANSPARENT).inked_pixels(),
            (W * H) as u64,
            "against a transparent base every one of those pixels is ink"
        );
    }

    /// A buffer whose length is not a whole number of pixels contributes only
    /// the pixels it completes — a truncated readback must not be counted as
    /// a partial pixel of ink.
    #[test]
    fn a_trailing_partial_pixel_is_not_counted() {
        let mut pixels = filled([0, 0, 0, 0]);
        pixels.extend_from_slice(&[9, 9, 9]);
        let ink = FrameInk::measure(&pixels, Color::TRANSPARENT);
        assert_eq!(ink.total_pixels(), (W * H) as u64);
        assert!(!ink.drew_ink());
    }

    /// An empty buffer measured nothing, and must not report a fraction it
    /// has no denominator for.
    #[test]
    fn an_empty_buffer_reports_no_pixels_and_no_fraction() {
        let ink = FrameInk::measure(&[], Color::TRANSPARENT);
        assert_eq!(ink.total_pixels(), 0);
        assert!(!ink.drew_ink());
        assert!((ink.inked_fraction() - 0.0).abs() < f64::EPSILON);
    }

    /// `is_uniform` is the reading that needs no base, so it must not quietly
    /// consult one: a frame of a single non-base colour is uniform AND inked,
    /// and the two answers are allowed to differ.
    #[test]
    fn uniformity_and_ink_are_independent_readings() {
        let ink = FrameInk::measure(&filled([7, 7, 7, 255]), Color::TRANSPARENT);
        assert!(ink.is_uniform());
        assert!(ink.drew_ink());
    }
}
