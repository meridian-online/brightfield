//! Text rendering — draws label strings into a Vello scene using a bundled
//! sans-serif font.
//!
//! Axis ticks, axis titles and legend entries call [`draw_text`] to paint real
//! glyphs. Before this, labels were invisible placeholder rects, so axes were
//! unreadable. The font (Inter Regular, the Meridian design system's UI sans)
//! is embedded at compile time via the `meridian-design` crate so the binary
//! stays self-contained; its SIL OFL 1.1 licence ships inside that crate.

use std::sync::{Arc, OnceLock};

use kurbo::Affine;
use peniko::{Blob, Color, Fill, FontData};
use skrifa::metrics::GlyphMetrics;
use skrifa::prelude::*;
use vello::{Glyph, Scene};

/// Bundled UI font: Inter Regular (SIL Open Font License 1.1, licence bundled
/// in the `meridian-design` crate alongside the bytes).
static FONT_DATA: &[u8] = meridian_design::fonts::INTER_REGULAR;

/// Default label size in pixels.
///
/// The sizes stay here as `const`s and the colours did not: a type size is the
/// same number in both modes (ADR 0003 sets one type scale), and an ink is not.
pub const LABEL_SIZE: f32 = 11.0;

/// Axis / plot title size in pixels (a touch larger than tick labels).
pub const TITLE_SIZE: f32 = 12.0;

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
fn ui_font() -> &'static FontData {
    static FONT: OnceLock<FontData> = OnceLock::new();
    FONT.get_or_init(|| FontData::new(Blob::new(Arc::new(FONT_DATA)), 0))
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

/// Transform for text rotated a quarter-turn anticlockwise about the pivot
/// (`x`, `y`): translate to the pivot, THEN rotate −90°. A glyph advancing along
/// its local +x maps to DECREASING screen y at fixed screen x — so the run reads
/// bottom-to-top, the y-axis title orientation. Factored out so the mapping is
/// unit-testable without introspecting a Scene.
fn quarter_turn_ccw(x: f64, y: f64) -> Affine {
    Affine::translate((x, y)) * Affine::rotate(-std::f64::consts::FRAC_PI_2)
}

/// Draw `text` rotated a quarter-turn anticlockwise (reading bottom-to-top) with
/// its baseline pivot at (`x`, `y`) — the y-axis title primitive. `anchor`
/// places the run ALONG its own (now vertical) baseline: [`TextAnchor::Middle`]
/// centres it on `y`. No-op for an empty string or an unparseable font.
pub fn draw_text_rotated(
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

    // Anchor offset along the (rotated) baseline — same rule as draw_text, but
    // applied to the pen START so it rotates WITH the run rather than shifting
    // screen x.
    let measured = measure(&charmap, &metrics, text);
    let start = match anchor {
        TextAnchor::Start => 0.0_f32,
        TextAnchor::Middle => (-measured / 2.0) as f32,
        TextAnchor::End => (-measured) as f32,
    };

    let mut pen_x = start;
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
        .transform(quarter_turn_ccw(x, y))
        .brush(colour)
        .draw(Fill::NonZero, glyphs.into_iter());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::ChartInk;

    #[test]
    fn rotated_text_reads_bottom_to_top() {
        // A point advancing +1 along the local run maps to 1px UP the screen
        // (decreasing y) at a fixed x — the y-axis title orientation.
        let t = quarter_turn_ccw(50.0, 200.0);
        let base = t * kurbo::Point::new(0.0, 0.0);
        let advanced = t * kurbo::Point::new(1.0, 0.0);
        assert!((base.x - 50.0).abs() < 1e-9 && (base.y - 200.0).abs() < 1e-9);
        assert!(
            (advanced.x - 50.0).abs() < 1e-9,
            "screen x fixed along the run"
        );
        assert!(
            (advanced.y - 199.0).abs() < 1e-9,
            "advancing the run moves UP the screen"
        );

        // Renders geometry; empty is a no-op (mirrors draw_text).
        let mut s = Scene::new();
        draw_text_rotated(
            &mut s,
            "kWh",
            10.0,
            100.0,
            TITLE_SIZE,
            ChartInk::LIGHT.title,
            TextAnchor::Middle,
        );
        assert!(
            !s.encoding().draw_tags.is_empty(),
            "rotated text adds geometry"
        );
        let mut e = Scene::new();
        draw_text_rotated(
            &mut e,
            "",
            0.0,
            0.0,
            TITLE_SIZE,
            ChartInk::LIGHT.title,
            TextAnchor::Start,
        );
        assert_eq!(
            e.encoding().draw_tags.len(),
            0,
            "empty rotated text draws nothing"
        );
    }

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

        assert_eq!(
            measure_width("", LABEL_SIZE),
            0.0,
            "empty string has no width"
        );
    }

    #[test]
    fn draw_text_emits_glyph_geometry() {
        let mut scene = Scene::new();
        draw_text(
            &mut scene,
            "42",
            100.0,
            100.0,
            LABEL_SIZE,
            ChartInk::LIGHT.label,
            TextAnchor::Middle,
        );
        // Glyph runs encode as draw tags in the scene.
        assert!(
            !scene.encoding().draw_tags.is_empty(),
            "drawing text should add geometry to the scene"
        );

        // Empty string is a no-op.
        let mut empty = Scene::new();
        draw_text(
            &mut empty,
            "",
            0.0,
            0.0,
            LABEL_SIZE,
            ChartInk::LIGHT.label,
            TextAnchor::Start,
        );
        assert_eq!(
            empty.encoding().draw_tags.len(),
            0,
            "empty text draws nothing"
        );
    }
}
