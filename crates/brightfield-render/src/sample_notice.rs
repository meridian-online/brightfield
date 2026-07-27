//! Saying, in the plot's own ink, that the picture is a sample.
//!
//! # What this is for
//!
//! It **advises**. Sampling is a performance measure, and a reader who is
//! shown a sample is owed a plain statement of what they are looking at:
//! how many rows were drawn, out of how many, and — the part that means
//! anything without arithmetic — what fraction that is.
//!
//! It is **not** armour. An earlier design asked this device to make a sampled
//! view distinguishable from a complete one *without reading text*, so that a
//! cropped screenshot could not misrepresent. That requirement is withdrawn:
//! upstream sampling, post-processing and cropping are all outside what a
//! renderer can address, and designing against them buys ink without buying
//! honesty. Judged at the keyboard, the device it produced — a hatch, chosen
//! because nothing else here is hatched — did not do that job anyway: it
//! marked the band, not the picture.
//!
//! # Why it is still in the scene rather than in chrome
//!
//! Anything the shell draws — a status strip, a toast, the margin legend — is
//! absent from `capture_vello_only` **by construction**: that export rasterises
//! the composed Vello scene and never builds an egui context at all. Drawing
//! into the scene means the notice travels with the chart through the ordinary
//! export path, which serves informing. That is the whole of the argument now.
//!
//! # The treatment
//!
//! Google Analytics states sampling as a prominent text line — *"This report is
//! based on 225,931 sessions (2.66% of sessions)"* — in an attention-taking
//! fill. Text does the informing; colour does the noticing. This is that shape:
//! a band below the plot carrying [`notice_label`], filled with the design
//! system's **warning solid** and inked with that role's own foreground.
//!
//! The design system's hard rule is that a status colour is never colour-alone
//! — always paired with a label. A fill plus a sentence is colour plus text and
//! is compliant. The fill and its ink are taken from the token layer rather
//! than invented: `Role::Warning`'s background and foreground, the same pairing
//! the protocol canvas's issue badge wears, which is contrast-checked and is
//! identical in light and dark (a solid and the ink chosen for it are both
//! paints).
//!
//! The band is reserved exactly the way axis titles reserve theirs —
//! [`sample_band_margins`] grows the bottom margin by [`NOTICE_BAND`] when a
//! plot is sampled. Taking the device away later restores the previous geometry
//! and re-baselines only sampled fixtures.

use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::ink::ink;
use crate::layout::{ChartLayout, Margins};
use crate::text::{draw_text, TextAnchor, LABEL_SIZE};

/// What a sampled plot knows about its own sample.
///
/// `Copy` on purpose: it rides on [`ChartData`](crate::scene::ChartData) and on
/// the shell's plot handle, both of which are cheap value types, and it must
/// survive a crossfilter rebuild without anyone remembering to clone it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleFact {
    /// Rows the plot actually drew.
    pub drawn: u64,
    /// Rows the same query would have returned unsampled. Counted by an
    /// unsampled query, not inferred by multiplying `drawn` by the modulus —
    /// a hash sample is not a perfectly uniform partition and the inferred
    /// figure would be a guess presented as a count.
    pub of: u64,
}

/// The band reserved below the plot for the notice, in logical pixels.
pub const NOTICE_BAND: f64 = 22.0;

/// The attention fill the band wears — the design system's warning solid.
///
/// Not a hue picked here. `Role::Warning`'s background is the one token whose
/// job is *caution, look at this*, it is reserved (never a series colour), and
/// it is the same value in both modes, which is what lets a scene with no mode
/// plumbing use it without lying in one of them. Paired below with
/// [`label_ink`], which is that role's own foreground and NOT the near-white
/// every other role takes — amber is a bright solid and needs dark text.
fn band_fill() -> Color {
    ink(meridian_design::semantic(false)
        .role(meridian_design::Role::Warning)
        .background
        .base)
}

/// The ink the words wear: the warning role's own foreground.
fn label_ink() -> Color {
    ink(meridian_design::semantic(false)
        .role(meridian_design::Role::Warning)
        .foreground
        .base)
}

/// Grow `margins` to reserve the notice band when the plot is sampled.
///
/// The one place the geometry and the drawing agree. Callers apply this
/// wherever they apply [`grow_margins`](crate::title::grow_margins) — if a
/// caller forgets, the band draws over the x-axis title rather than beside it,
/// which is why both live next to each other here rather than being derived
/// twice.
#[must_use]
pub fn sample_band_margins(margins: Margins, sampled: bool) -> Margins {
    if sampled {
        Margins {
            bottom: margins.bottom + NOTICE_BAND,
            ..margins
        }
    } else {
        margins
    }
}

/// Draw the notice into `scene` for a plot whose layout already reserves the
/// band (see [`sample_band_margins`]).
///
/// The fill spans the full plot width, because the thing being described is the
/// whole picture, not a corner of it.
pub fn render_sample_notice(scene: &mut Scene, layout: &ChartLayout, fact: SampleFact) {
    let top = layout.height - NOTICE_BAND;
    let band = Rect::new(
        layout.margins.left,
        top,
        layout.width - layout.margins.right,
        layout.height,
    );
    if band.width() <= 0.0 || band.height() <= 0.0 {
        return;
    }

    scene.fill(Fill::NonZero, Affine::IDENTITY, band_fill(), None, &band);
    draw_text(
        scene,
        &notice_label(fact),
        band.x0 + 7.0,
        band.y1 - 7.0,
        LABEL_SIZE,
        label_ink(),
        TextAnchor::Start,
    );
}

/// The words in the band.
///
/// `4,688 of 75,000 rows drawn` is arithmetic homework; `(6.3%)` is the answer.
/// Both are here, in that order, because the count is what a reader checks and
/// the proportion is what they understand — the same shape Google Analytics
/// uses for the same reason.
///
/// A fact with nothing to be a proportion *of* drops the parenthetical rather
/// than printing a fabricated one.
#[must_use]
pub fn notice_label(fact: SampleFact) -> String {
    let counts = format!(
        "SAMPLED — {} of {} rows drawn",
        thousands(fact.drawn),
        thousands(fact.of)
    );
    match percentage(fact) {
        Some(p) => format!("{counts} ({p})"),
        None => counts,
    }
}

/// The proportion drawn, rendered so it can never read as zero on a picture
/// that has points in it.
///
/// **Two significant figures, capped at three decimals, floored at a strict
/// bound.** A fixed one-decimal `%` prints `0.0%` for a 1-in-5000 sample, and a
/// reader looking at several thousand visible points is being told, in the
/// renderer's own voice, that none of them are there. That kind of small lie
/// costs more trust than the space it saves. So the number of decimals grows as
/// the value shrinks — `62%`, `6.3%`, `0.42%`, `0.042%` — and below the point
/// where three decimals would round to nothing the label states a BOUND,
/// `<0.001%`, which is true, obviously approximate, and visibly not zero.
///
/// `0%` appears in exactly one case: `drawn == 0`, where it is the fact.
fn percentage(fact: SampleFact) -> Option<String> {
    if fact.of == 0 {
        return None;
    }
    if fact.drawn == 0 {
        return Some("0%".to_string());
    }
    let p = 100.0 * fact.drawn as f64 / fact.of as f64;
    let decimals = if p >= 10.0 {
        0
    } else if p >= 1.0 {
        1
    } else if p >= 0.1 {
        2
    } else {
        3
    };
    let rendered = format!("{p:.decimals$}");
    if rendered.chars().all(|c| c == '0' || c == '.') {
        return Some("<0.001%".to_string());
    }
    Some(format!("{rendered}%"))
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mark::count_scene_glyphs;

    /// Pack a peniko colour exactly as vello encodes a solid fill into
    /// `draw_data` (premultiplied little-endian RGBA8), so a test can compare a
    /// rendered fill against an expected colour byte-for-byte.
    fn packed(colour: Color) -> u32 {
        colour.premultiply().to_rgba8().to_u32()
    }

    #[test]
    fn band_is_reserved_only_when_sampled() {
        let base = Margins::default();
        assert_eq!(sample_band_margins(base, false).bottom, base.bottom);
        assert_eq!(
            sample_band_margins(base, true).bottom,
            base.bottom + NOTICE_BAND
        );
        // Only the bottom moves — the device is removable without disturbing
        // any other side's geometry.
        let grown = sample_band_margins(base, true);
        assert_eq!(
            (grown.top, grown.left, grown.right),
            (base.top, base.left, base.right)
        );
    }

    /// The band is the design system's warning solid, and the words are that
    /// role's own foreground — both probed out of what was actually ENCODED
    /// into the scene, not re-derived from the same constants the code reads.
    ///
    /// The pairing is the point. Amber is a bright solid: the near-white
    /// `text.on_solid` every other role takes measures 2.47:1 on it and this
    /// role's own foreground 6.45:1, so taking the generic on-solid ink would
    /// resolve through the design system and still be wrong.
    ///
    /// **This test does not hold the fill, and the next reader should not
    /// assume it does.** It asks whether those two paints were encoded, which a
    /// notice reduced to a hairline in the same colour also satisfies —
    /// measured. What holds the band's *coverage* is the shell's
    /// `the_sampling_notice_is_in_the_chart_only_export`, which counts the
    /// warning solid's pixels in an exported PNG against a floor. Name the test
    /// that fails, not the one that looks like it would.
    #[test]
    fn the_band_is_the_warning_solid_and_the_words_are_its_own_ink() {
        let layout =
            ChartLayout::with_margins(400.0, 300.0, sample_band_margins(Margins::default(), true));
        let mut scene = Scene::new();
        render_sample_notice(&mut scene, &layout, SampleFact { drawn: 10, of: 100 });

        let drawn: Vec<u32> = scene.encoding().draw_data.to_vec();
        let sem = meridian_design::semantic(false);
        let fill = packed(ink(sem
            .role(meridian_design::Role::Warning)
            .background
            .base));
        let text = packed(ink(sem
            .role(meridian_design::Role::Warning)
            .foreground
            .base));
        assert!(
            drawn.contains(&fill),
            "the band must be filled with the warning solid"
        );
        assert!(
            drawn.contains(&text),
            "the label must wear the warning role's own foreground"
        );
        assert_ne!(
            text,
            packed(ink(sem.text.on_solid)),
            "on_solid is the ink slot this must NOT take — it is 2.47:1 on amber"
        );
        assert!(
            count_scene_glyphs(&scene) > 0,
            "colour without the sentence is the one thing the design system \
             forbids: a status hue alone"
        );
    }

    /// Both paints are the same in either mode, which is what lets a scene with
    /// no mode plumbing wear them without lying in one of them.
    #[test]
    fn the_notices_paints_are_the_same_in_both_modes() {
        for role_colour in [
            |dark: bool| {
                meridian_design::semantic(dark)
                    .role(meridian_design::Role::Warning)
                    .background
                    .base
            },
            |dark: bool| {
                meridian_design::semantic(dark)
                    .role(meridian_design::Role::Warning)
                    .foreground
                    .base
            },
        ] {
            assert_eq!(role_colour(false), role_colour(true));
        }
    }

    #[test]
    fn the_label_carries_the_proportion_beside_the_counts() {
        let s = notice_label(SampleFact {
            drawn: 4_688,
            of: 75_000,
        });
        assert_eq!(s, "SAMPLED — 4,688 of 75,000 rows drawn (6.3%)");
    }

    /// **A thin sample must not round to zero on a picture full of points.**
    ///
    /// Each of these is a real sample with rows on screen, and the printed
    /// proportion must never be readable as "none of it". A fixed one-decimal
    /// format fails the last three outright.
    #[test]
    fn a_thin_sample_never_reads_as_none_of_it() {
        let cases = [
            (750_u64, 1_000_u64, "75%"),
            (631, 10_000, "6.3%"),
            (42, 10_000, "0.42%"),
            (42, 100_000, "0.042%"),
            (1, 1_000_000, "<0.001%"),
            (3, 100_000_000, "<0.001%"),
        ];
        for (drawn, of, expected) in cases {
            let label = notice_label(SampleFact { drawn, of });
            assert!(
                label.ends_with(&format!("({expected})")),
                "{drawn} of {of} printed {label}, expected ({expected})"
            );
            assert!(
                !label.contains("(0%)") && !label.contains("(0.0%)"),
                "a sample with {drawn} rows on screen printed as nothing: {label}"
            );
        }
    }

    /// The one case where zero is the truth, and the one where there is no
    /// proportion to state at all.
    #[test]
    fn zero_drawn_says_zero_and_zero_rows_says_nothing() {
        assert!(notice_label(SampleFact { drawn: 0, of: 500 }).ends_with("(0%)"));
        let none = notice_label(SampleFact { drawn: 0, of: 0 });
        assert!(
            !none.contains('%'),
            "nothing to be a proportion of, so no proportion: {none}"
        );
    }
}
