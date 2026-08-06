//! Whether a cell's picture arrived, and what it is a picture OF — the
//! readback that stands where a prediction used to.
//!
//! A count of drawn primitives computed before anything renders cannot decide
//! whether a render produced a picture, because the failure it guards against
//! is silent — vello records an overflow in a GPU-side counter, emits nothing,
//! and returns `Ok`. A cell was therefore timed and published on the strength
//! of arithmetic.
//!
//! [`probe`] composes the cell's spec through the production pipeline, renders
//! it once at the frame scale, and reads the target back. A cell whose picture
//! comes back empty is a per-cell failure with no timing beside it.
//!
//! It also reports what the compose **drew**. The pushed-down sampling policy
//! ships in `brightfield-render` and `brightfield-shell` and engages on its
//! own, so a large scene arrives at the renderer already thinned; the rows a
//! plot drew and the rows the same query answers unsampled are read off the
//! composition rather than derived here. A harness that inferred them would be
//! deciding the sampling question a second time, in a second place.
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

/// What one plot of this cell drew, when the shipped policy sampled it.
///
/// Both figures are **counted**. `drawn` is what the sampled query returned;
/// `of` is what the same query returns with no rate on it, taken by a second
/// query rather than by multiplying `drawn` by a modulus — a hash sample is not
/// a perfectly uniform partition, so the multiplied figure would be a guess
/// printed in the column where a measurement belongs.
#[derive(Debug, Clone, Serialize)]
pub struct PlotSample {
    /// The plot's component path (`root`, `root/hconcat[0]`, …), so a reader
    /// knows which of a dashboard's pictures was thinned.
    pub plot: String,
    /// Rows this plot's marks drew.
    pub drawn: u64,
    /// Rows the same query answered unsampled.
    pub of: u64,
}

/// One cell's readback: what reached the target, and what the picture about to
/// be timed is a sample of.
#[derive(Debug, Clone)]
pub struct Probe {
    /// What the render put on the target.
    pub ink: FrameInk,
    /// The plots the policy sampled, in composition order. Empty when the cell
    /// drew complete — which is the common case and the one that needs no
    /// qualification in the record.
    pub sample: Vec<PlotSample>,
}

/// Compose `spec_path` through the production pipeline, render it once at
/// `scale`, and report what reached the target and what it was drawn from.
///
/// # Errors
///
/// Returns a message if the spec does not compose or the renderer's lock is
/// poisoned.
pub fn probe(spec_path: &Path, scale: f32) -> Result<Probe, String> {
    let composed = compose_spec(spec_path.to_str().ok_or("spec path is not UTF-8")?)?;
    let sample: Vec<PlotSample> = composed
        .plots
        .iter()
        .filter_map(|plot| {
            plot.sample.map(|fact| PlotSample {
                plot: plot.path.clone(),
                drawn: fact.drawn,
                of: fact.of,
            })
        })
        .collect();
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
    Ok(Probe {
        ink: FrameInk {
            width,
            height,
            inked_pixels: ink.inked_pixels(),
            total_pixels: ink.total_pixels(),
            inked_fraction: ink.inked_fraction(),
            uniform: ink.is_uniform(),
            drew_ink: ink.drew_ink(),
        },
        sample,
    })
}

/// How a sampled cell's frame timing is labelled in the generated summary, or
/// `None` for a cell that drew complete.
///
/// States both counted figures per plot. It deliberately does not divide them
/// into a modulus: the quotient of two counts over a hash partition is not the
/// rate that was pushed down, and a reader given "1 in 128" would take it for
/// one.
#[must_use]
pub fn sample_label(sample: &[PlotSample]) -> Option<String> {
    if sample.is_empty() {
        return None;
    }
    Some(
        sample
            .iter()
            .map(|s| format!("{} drew {} of {}", s.plot, s.drawn, s.of))
            .collect::<Vec<_>>()
            .join(" · "),
    )
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

    /// A cell that drew complete must not be labelled at all — a qualifier on
    /// every row would stop meaning anything on the rows that need it.
    #[test]
    fn a_complete_cell_carries_no_sample_label() {
        assert!(sample_label(&[]).is_none());
    }

    /// The label states both counted figures, per plot, and states no modulus:
    /// the quotient of a drawn count and an unsampled count over a hash
    /// partition is not the rate that was pushed down.
    #[test]
    fn the_sample_label_states_both_counts_and_no_modulus() {
        let label = sample_label(&[
            PlotSample {
                plot: "root/hconcat[0]".to_string(),
                drawn: 78_125,
                of: 10_000_000,
            },
            PlotSample {
                plot: "root/hconcat[1]".to_string(),
                drawn: 78_120,
                of: 10_000_000,
            },
        ])
        .expect("a sampled cell is labelled");
        assert!(
            label.contains("root/hconcat[0] drew 78125 of 10000000"),
            "{label}"
        );
        assert!(
            label.contains("root/hconcat[1] drew 78120 of 10000000"),
            "{label}"
        );
        for inferred in ["1 in 128", "1 in 2", "128x", "/128"] {
            assert!(
                !label.contains(inferred),
                "the label must not divide two counted figures into a rate: {label}"
            );
        }
    }
}
