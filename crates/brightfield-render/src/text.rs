//! Text rendering — draws label strings into a Vello scene using a bundled
//! sans-serif font.
//!
//! Axis ticks, axis titles and legend entries call [`draw_text`] to paint real
//! glyphs. Before this, labels were invisible placeholder rects, so axes were
//! unreadable. The font (IBM Plex Sans Regular, the face GPUI itself uses) is
//! embedded at compile time so the binary stays self-contained — see
//! `assets/fonts/LICENSE-IBMPlexSans.txt` (SIL Open Font License 1.1).

use std::sync::{Arc, OnceLock};

use kurbo::Affine;
use peniko::{Blob, Color, Fill, Font};
use skrifa::metrics::GlyphMetrics;
use skrifa::prelude::*;
use vello::{Glyph, Scene};

/// Bundled UI font: IBM Plex Sans Regular (SIL Open Font License 1.1).
static FONT_DATA: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");

/// Default label text colour (dark grey, matching axis/tick strokes).
pub const LABEL_COLOUR: Color = Color::new([0.2, 0.2, 0.2, 1.0]);

/// Default label size in pixels.
pub const LABEL_SIZE: f32 = 11.0;

/// Horizontal placement of a label relative to its anchor x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor {
    /// Anchor is the left edge of the text.
    Start,
    /// Anchor is the horizontal centre of the text.
    Middle,
    /// Anchor is the right edge of the text.
    End,
}

/// The bundled font, parsed once and reused across draws.
fn ui_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| Font::new(Blob::new(Arc::new(FONT_DATA)), 0))
}

/// Sum the pixel advance widths of `text`'s glyphs at the metrics' size.
fn measure(charmap: &skrifa::charmap::Charmap<'_>, metrics: &GlyphMetrics, text: &str) -> f64 {
    text.chars()
        .map(|c| {
            let gid = charmap.map(c).unwrap_or_default();
            f64::from(metrics.advance_width(gid).unwrap_or(0.0))
        })
        .sum()
}

/// Pixel width of `text` rendered at `size`. Useful for reserving margin space.
#[must_use]
pub fn measure_width(text: &str, size: f32) -> f64 {
    let Ok(font_ref) = FontRef::new(FONT_DATA) else {
        return 0.0;
    };
    let metrics = font_ref.glyph_metrics(Size::new(size), LocationRef::default());
    measure(&font_ref.charmap(), &metrics, text)
}

/// Draw `text` with its baseline at (`x`, `y`), horizontally placed per `anchor`.
///
/// `x` is the anchor point: with [`TextAnchor::Start`] the text begins at `x`,
/// with [`TextAnchor::Middle`] it is centred on `x`, with [`TextAnchor::End`] it
/// ends at `x`. No-op for an empty string or an unparseable font.
pub fn draw_text(
    scene: &mut Scene,
    text: &str,
    x: f64,
    y: f64,
    size: f32,
    colour: Color,
    anchor: TextAnchor,
) {
    if text.is_empty() {
        return;
    }
    let Ok(font_ref) = FontRef::new(FONT_DATA) else {
        return;
    };
    let metrics = font_ref.glyph_metrics(Size::new(size), LocationRef::default());
    let charmap = font_ref.charmap();

    let start_x = match anchor {
        TextAnchor::Start => x,
        TextAnchor::Middle => x - measure(&charmap, &metrics, text) / 2.0,
        TextAnchor::End => x - measure(&charmap, &metrics, text),
    };

    let mut pen_x = 0.0_f32;
    let glyphs: Vec<Glyph> = text
        .chars()
        .map(|c| {
            let gid = charmap.map(c).unwrap_or_default();
            let glyph = Glyph {
                id: gid.to_u32(),
                x: pen_x,
                y: 0.0,
            };
            pen_x += metrics.advance_width(gid).unwrap_or(0.0);
            glyph
        })
        .collect();

    scene
        .draw_glyphs(ui_font())
        .font_size(size)
        .transform(Affine::translate((start_x, y)))
        .brush(colour)
        .draw(Fill::NonZero, glyphs.into_iter());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_parses() {
        // The embedded font must be a valid TTF the renderer can load.
        assert!(FontRef::new(FONT_DATA).is_ok(), "bundled font should parse");
        // OnceLock construction must not panic.
        let _ = ui_font();
    }

    #[test]
    fn measure_width_scales_with_text_and_size() {
        let narrow = measure_width("1", LABEL_SIZE);
        let wide = measure_width("1000", LABEL_SIZE);
        assert!(wide > narrow, "longer text should be wider");
        assert!(narrow > 0.0, "a digit should have positive width");

        let small = measure_width("1000", 8.0);
        let big = measure_width("1000", 20.0);
        assert!(big > small, "larger size should be wider");

        assert_eq!(measure_width("", LABEL_SIZE), 0.0, "empty string has no width");
    }

    #[test]
    fn draw_text_emits_glyph_geometry() {
        let mut scene = Scene::new();
        draw_text(&mut scene, "42", 100.0, 100.0, LABEL_SIZE, LABEL_COLOUR, TextAnchor::Middle);
        // Glyph runs encode as draw tags in the scene.
        assert!(
            scene.encoding().draw_tags.len() > 0,
            "drawing text should add geometry to the scene"
        );

        // Empty string is a no-op.
        let mut empty = Scene::new();
        draw_text(&mut empty, "", 0.0, 0.0, LABEL_SIZE, LABEL_COLOUR, TextAnchor::Start);
        assert_eq!(empty.encoding().draw_tags.len(), 0, "empty text draws nothing");
    }
}
