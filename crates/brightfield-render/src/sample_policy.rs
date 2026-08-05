//! Where the renderer stops drawing, and the sample rate that keeps a plot
//! under it.
//!
//! # The ceiling
//!
//! Vello's flattening and coarse-raster buffers are fixed sizes chosen inside
//! `vello_encoding` — 2^21 elements for `lines` / `tiles` / `seg_counts` /
//! `segments`, 2^18 for `bin_data`. They do not scale with the scene and no
//! wgpu limit moves them. When one overflows, vello sets a `failed` bit in a
//! GPU-side counter, does not re-run coarse, and returns `Ok`. Nothing in the
//! render path raises an error, so **the render succeeds and the frame comes
//! back BLANK**.
//!
//! **Blank, not partial.** Once `seg_counts` overflows, coarse emits nothing at
//! all: the counters come back with `segments = 0` and `ptcl = 0`, and the
//! production render agrees — every pixel of the written PNG is
//! `rgba(0, 0, 0, 0)`.
//!
//! Measured on the reference machine (Apple M1 Pro, Metal) through the
//! PRODUCTION path — `cargo run -p brightfield-shell --bin brightfield-shot --
//! --vello-only`, a debug build, one dot scatter in a 640×480 plot at scale 2:
//!
//! | dots | exit | written PNG |
//! |---|---|---|
//! | 50 000 | 0 | inked — 74 % of pixels carry mark or furniture |
//! | [`MEASURED_INKED_MAX`] | 0 | inked |
//! | [`MEASURED_BLANK_MIN`] | 0 | **blank** — 1 280 × 960 px, every one `rgba(0,0,0,0)` |
//! | 106 000 · 132 000 · 262 000 | 0 | blank |
//! | [`VELLO_BIN_DATA_PANIC_DOTS`] − 1 | 0 | blank |
//! | [`VELLO_BIN_DATA_PANIC_DOTS`] and up | 101 | `vello_encoding` `config.rs:185`, subtract overflow |
//!
//! Two distinct ceilings, and they are an order of magnitude apart. The first
//! is `seg_counts` (blank frame, exit 0, no diagnostic anywhere); its onset
//! depends on how much rule each dot contributes, so the bracket above is
//! fixture-specific rather than a constant. The second is exact and is not:
//! `bin_data` is 2^18 elements and one solid-colour draw consumes one, so the
//! subtraction goes negative at 2^18 filled paths. That plot carries 42 paths
//! of frame, grid and axis rule, and the panic first fires
//! [`VELLO_BIN_DATA_ELEMENTS`] − 42 dots in, to the row.
//!
//! `crates/brightfield-render/tests/vello_bump_ceiling.rs` reproduces the
//! counters; the table above is reproduced with `brightfield-shot` and a
//! `range(N)` spec. Re-run both before moving these numbers.
//!
//! # Widening is not the fix
//!
//! Raising the count a plot is allowed to draw does not merely mis-report.
//! Measured with the bench harness's cap bypassed at 150 000 primitives per
//! plot, the encoder hits the second ceiling in `bin_data` and panics. Nothing
//! above the first ceiling draws; the only thing that keeps a picture on the
//! screen is drawing fewer primitives, which is what [`sample_exponent`] is
//! for.
//!
//! # The policy
//!
//! [`renders_complete`] is the one predicate: a plot draws a picture when its
//! drawn-primitive count is at or below the largest count measured to ink a
//! frame. [`sample_exponent`] chooses the smallest power-of-two modulus that
//! brings a plot back under it. Both read [`MEASURED_INKED_MAX`] and neither
//! restates it.

/// The largest one-scatter dot count MEASURED to ink a frame — the last count
/// below the `seg_counts` ceiling.
///
/// **The drawn-primitive ceiling, and the only copy of it.** The sampling
/// policy here and `brightfield-bench`'s frame-suite cap both read this
/// constant; `crates/brightfield-render/tests/one_ceiling.rs` fails if a second
/// copy of the figure appears in the workspace's Rust sources.
///
/// A harness or a shell may choose to stay further under it, and
/// `brightfield-bench` does. Quoting such a cap as "what the renderer can do"
/// understates the renderer by that margin and names a policy number as though
/// it were a property of the machine.
pub const MEASURED_INKED_MAX: u64 = 104_600;

/// The smallest one-scatter dot count MEASURED to come back BLANK: vello
/// returns `Ok`, a frame is written, and every pixel is `rgba(0, 0, 0, 0)`.
///
/// The gap to [`MEASURED_INKED_MAX`] is the bracket the probe resolved to, not
/// a tolerance — the onset depends on how much rule each dot contributes, so
/// it is fixture-specific rather than a constant of the renderer.
pub const MEASURED_BLANK_MIN: u64 = 104_800;

/// `bin_data`, the second and exact ceiling: 2^18 elements, one consumed per
/// solid-colour draw, so the subtraction underflows a `u32` at 2^18 filled
/// paths.
pub const VELLO_BIN_DATA_ELEMENTS: u64 = 1 << 18;

/// Where that underflow first fired in the probe fixture, measured to the row:
/// [`VELLO_BIN_DATA_ELEMENTS`] less the 42 paths of frame, grid and axis rule
/// the plot carries. One dot below this exits 0 (blank); this count exits 101.
pub const VELLO_BIN_DATA_PANIC_DOTS: u64 = 262_102;

// The relationships between the numbers above are the guard, and they are
// compile-time facts — so the build checks them, rather than a test that has to
// be run to say so.
const _: () = assert!(MEASURED_INKED_MAX < MEASURED_BLANK_MIN);
// The second ceiling is derived, not observed loose: 2^18 elements less the 42
// paths of furniture. Edit either number without the other and this stops
// holding.
const _: () = assert!(VELLO_BIN_DATA_ELEMENTS - 42 == VELLO_BIN_DATA_PANIC_DOTS);
const _: () = assert!(VELLO_BIN_DATA_PANIC_DOTS > MEASURED_BLANK_MIN);

/// Whether a plot drawing `primitives` row-level primitives renders a picture.
///
/// At or below [`MEASURED_INKED_MAX`], because that constant is the largest
/// count measured to ink — a plot drawing exactly it drew. Above it the frame
/// is not degraded, it is absent, so this is the boundary both the automatic
/// sampler and any harness cap are drawn from.
#[must_use]
pub const fn renders_complete(primitives: u64) -> bool {
    primitives <= MEASURED_INKED_MAX
}

/// The sample exponent a plot estimated to draw `primitives` row-level
/// primitives needs, or `None` when it needs none.
///
/// The answer is the **smallest** exponent for which
/// [`renders_complete`] holds of the sampled count, so the picture keeps as
/// many of its points as it can while still being a picture. Powers of two
/// because they nest: every row a `% 2^n` sample keeps is kept by every smaller
/// exponent, so a later change of rate densifies the picture rather than
/// reshuffling it.
///
/// `None` means the plot draws complete — the caller passes no rate at all
/// rather than an exponent of zero, so an unsampled chart's emitted SQL and its
/// query count stay byte-unchanged by this policy's existence.
#[must_use]
pub fn sample_exponent(primitives: u64) -> Option<u32> {
    if renders_complete(primitives) {
        return None;
    }
    // `primitives > MEASURED_INKED_MAX >= 1`, so the shift terminates well
    // inside u64's width and the exponent it lands on is representable as a
    // `SampleRate`.
    (1..=u32::BITS).find(|&e| renders_complete(primitives >> e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundary, on both sides of it — and the step that is not enough.
    ///
    /// The rows are `(estimated primitives, row-level marks, expected
    /// modulus)`: the count is what the marks draw between them, so a plot's
    /// second row-drawing mark moves it across the boundary as surely as
    /// doubling its rows does. `None` is "draws complete, no clause".
    #[test]
    fn the_modulus_is_the_smallest_that_brings_the_plot_back_under_the_ceiling() {
        let cases: &[(u64, u64, Option<u64>)] = &[
            // Nowhere near it.
            (1_000, 1, None),
            // The last count measured to ink a frame draws.
            (MEASURED_INKED_MAX, 1, None),
            // One primitive further and the frame would be blank.
            (MEASURED_INKED_MAX + 1, 1, Some(2)),
            // The smallest count measured to come back blank.
            (MEASURED_BLANK_MIN, 1, Some(2)),
            // Two marks over the same rows: each row is two primitives, and
            // the count that drew as one mark does not draw as two.
            (2 * MEASURED_INKED_MAX, 2, Some(2)),
            // One step is not enough: halving 500_000 leaves 250_000, still
            // over. Quartering leaves 125_000, still over. Only the third
            // step lands.
            (500_000, 1, Some(8)),
            // Three marks at a million rows each — the demo's magnitude.
            (3_000_000, 3, Some(32)),
            // Ten million, one mark.
            (10_000_000, 1, Some(128)),
        ];
        for &(primitives, marks, want) in cases {
            let got = sample_exponent(primitives).map(|e| 1_u64 << e);
            assert_eq!(
                got, want,
                "{primitives} primitives across {marks} row-level mark(s): expected \
                 modulus {want:?}, got {got:?}"
            );
            if let Some(modulus) = got {
                assert!(
                    renders_complete(primitives / modulus),
                    "the chosen modulus must land the plot under the ceiling"
                );
                assert!(
                    !renders_complete(primitives / (modulus / 2)),
                    "…and it must be the SMALLEST that does: {} would have left \
                     {} primitives, which also draws",
                    modulus / 2,
                    primitives / (modulus / 2)
                );
            }
        }
    }

    /// A sampled plot is under the ceiling for every magnitude a spec can
    /// plausibly ask for, not only the tabled ones.
    #[test]
    fn every_magnitude_lands_under_the_ceiling() {
        let mut primitives = 1_u64;
        while primitives < 1 << 40 {
            match sample_exponent(primitives) {
                Some(e) => assert!(
                    renders_complete(primitives >> e),
                    "{primitives} primitives sampled at 1 in {} still draws {}",
                    1_u64 << e,
                    primitives >> e
                ),
                None => assert!(
                    renders_complete(primitives),
                    "{primitives} primitives was left unsampled but does not draw"
                ),
            }
            primitives = primitives.saturating_mul(3) + 1;
        }
    }
}
