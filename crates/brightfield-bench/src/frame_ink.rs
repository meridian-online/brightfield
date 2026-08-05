//! Whether a cell's picture arrived — the readback that stands where a
//! prediction used to.
//!
//! The drawn-primitive cap declines a frame suite from a count computed before
//! anything renders. That decides which suites to *attempt*; it cannot decide
//! whether an attempted one produced a picture, because the failure it guards
//! against is silent — vello records an overflow in a GPU-side counter, emits
//! nothing, and returns `Ok`. A cell under the cap was therefore timed and
//! published on the strength of arithmetic.
//!
//! [`probe`] composes the cell's spec through the production pipeline, renders
//! it once at the frame scale, and reads the target back. A cell whose picture
//! comes back empty is a per-cell failure with no timing beside it.
//!
//! **What it is not.** This is a separate submission from the frames the suite
//! times. Those go through the shell's egui path, which does not read back, by
//! design — a readback there would time a cost the live window does not pay.
//! So this answers whether the cell's picture can be produced at all,
//! immediately before the cell is timed, on the same composed scene through the
//! same renderer at the same device scale.

use std::path::Path;

use brightfield_render::vello_renderer::VelloRenderer;
use brightfield_shell::pipeline::compose_spec;
use serde::Serialize;

/// The base the offscreen capture path clears to. `brightfield-shot
/// --vello-only` renders on it, and it is the base the recorded blank frames
/// were read against.
const BASE: vello::peniko::Color = vello::peniko::Color::TRANSPARENT;

/// What one cell's composed dashboard put on the target, counted from the
/// pixels it was read back into.
#[derive(Debug, Clone, Serialize)]
pub struct FrameInk {
    /// Device pixels the probe rendered into.
    pub width: u32,
    /// Device pixels the probe rendered into.
    pub height: u32,
    /// Pixels differing from the colour the render cleared to.
    pub inked_pixels: u64,
    /// Whole pixels the read-back buffer held.
    pub total_pixels: u64,
    /// [`Self::inked_pixels`] over [`Self::total_pixels`].
    pub inked_fraction: f64,
    /// Whether the target came back holding one repeated pixel value — the
    /// reading that consults no clear colour, so a base that did not round
    /// cleanly cannot flip it.
    pub uniform: bool,
    /// The verdict: did this cell's picture reach the target.
    pub drew_ink: bool,
}

/// Compose `spec_path` through the production pipeline, render it once at
/// `scale`, and report what reached the target.
///
/// # Errors
///
/// Returns a message if the spec does not compose or the renderer's lock is
/// poisoned.
pub fn probe(spec_path: &Path, scale: f32) -> Result<FrameInk, String> {
    let composed = compose_spec(spec_path.to_str().ok_or("spec path is not UTF-8")?)?;
    let width = ((composed.width as f32) * scale).round().max(1.0) as u32;
    let height = ((composed.height as f32) * scale).round().max(1.0) as u32;
    let mut scaled = vello::Scene::new();
    scaled.append(
        &composed.scene,
        Some(vello::kurbo::Affine::scale(f64::from(scale))),
    );
    let renderer = VelloRenderer::new();
    let ink = renderer
        .lock()
        .map_err(|_| "renderer lock poisoned".to_string())?
        .frame_ink(&scaled, width, height, BASE);
    Ok(FrameInk {
        width,
        height,
        inked_pixels: ink.inked_pixels(),
        total_pixels: ink.total_pixels(),
        inked_fraction: ink.inked_fraction(),
        uniform: ink.is_uniform(),
        drew_ink: ink.drew_ink(),
    })
}

/// Why this cell has no frame cells, when the reason is that its picture was
/// produced and was empty.
///
/// Deliberately not phrased as a skip. A skipped suite was declined before it
/// ran; this one ran, the render reported success, and the target came back
/// with nothing on it — the outcome the drawn-primitive cap exists to keep out
/// of the record and cannot detect on its own.
#[must_use]
pub fn blank_reason(ink: &FrameInk) -> String {
    format!(
        "FAILED, not skipped: this cell's picture was composed and rendered, the \
         render returned success, and the target came back EMPTY — {} of {} device \
         pixels ({}x{}) carry anything the clear did not put there. No frame timing \
         is published for it, because a blank frame is a fast frame by the clock \
         and nothing else. The run continues; a per-cell failure does not fail it.",
        ink.inked_pixels, ink.total_pixels, ink.width, ink.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(inked: u64) -> FrameInk {
        FrameInk {
            width: 100,
            height: 50,
            inked_pixels: inked,
            total_pixels: 5_000,
            inked_fraction: inked as f64 / 5_000.0,
            uniform: inked == 0,
            drew_ink: inked > 0,
        }
    }

    /// The word a reader of the record sees has to separate this from a skip,
    /// because the two look identical in the table — both leave the frame
    /// columns empty — and they are opposite events. A skip declined to
    /// measure; this measured and found nothing.
    #[test]
    fn the_blank_reason_calls_itself_a_failure_and_not_a_skip() {
        let why = blank_reason(&record(0));
        assert!(why.contains("FAILED, not skipped"), "{why}");
        assert!(why.contains("EMPTY"), "{why}");
        assert!(
            why.contains("does not fail it"),
            "the reason must say the run survives: {why}"
        );
    }

    /// The counts in the sentence come from the measurement, so a reader can
    /// check the verdict against them rather than take the word for it.
    #[test]
    fn the_blank_reason_states_the_counts_it_judged_on() {
        let why = blank_reason(&record(0));
        assert!(why.contains("0 of 5000"), "{why}");
        assert!(why.contains("100x50"), "{why}");
    }
}
