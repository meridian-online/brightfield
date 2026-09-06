//! **The Meridian mark, as this build actually rasterises it.**
//!
//! The mark reaches the front door and the title band as a coverage raster
//! built from the design system's own path data at run time — see
//! `brightfield_shell::brand`. Two failures are worth a test each, and neither
//! needs a photograph of a window to decide:
//!
//! - **The path stops parsing.** A re-export that emits an arc, or a design
//!   system that loses the `d` attribute, degrades to a window with no mark
//!   rather than to a build that fails. Held by asserting the parse and its
//!   subpath count against the count the design system's own test pins.
//! - **The fill runs the wrong way round.** An even-odd raster that treats the
//!   teeth as the holes draws the mark inside out, which is a picture of the
//!   right size in the right place — the shape a baseline diff is worst at and
//!   a reader skims past. Held by reading the coverage at a point inside a
//!   tooth and at a point in the gap beside it.

use brightfield_shell::brand;

/// The design system pins the mark at six closed subpaths — one tooth each —
/// in `meridian_design`'s own `the_mark_is_six_closed_subpaths`. This is the
/// consuming half of that pin: the shipped path parses here, and it parses
/// into the same count.
///
/// A re-export that changes the geometry may be intended, and it is never
/// incidental — so it reddens on both sides of the dependency rather than
/// only on the side that drew it.
#[test]
fn the_shipped_mark_parses() {
    let mark = brand::mark().expect(
        "the design system's mark_path() did not parse — either the SVG lost \
         its d attribute, or it now carries a path command brand.rs does not \
         implement",
    );
    assert_eq!(
        mark.subpath_count(),
        6,
        "the prime mark is six closed subpaths; re-check the export"
    );
}

/// **A tooth is ink and the gap beside it is not.**
///
/// The mask is even-odd, so a fill that ran the wrong way round would produce
/// the mark's exact negative: the same square, the same edges, every value
/// inverted. That is the failure a baseline diff reports as "the front door
/// changed" and a reader approves without looking closely, which is why it is
/// asserted here as two numbers instead.
///
/// The two probes are read off the mark's own construction rather than picked
/// off a picture. `meridian-design`'s brand README states the geometry: the
/// teeth are bars running from a vertical axis out to the limb, alternating
/// side top to bottom, clipped by a disc of radius half the viewbox. So the
/// first tooth's bar occupies the upper left of the disc, and the point
/// mirrored across the vertical axis at the same height is in the gap between
/// two teeth on the other side.
#[test]
fn the_masks_ink_is_the_teeth_and_not_the_gaps() {
    let mark = brand::mark().expect("the mark parses");
    let image = mark.mask();
    let [w, h] = image.size;
    assert_eq!(w, h, "the mask is square");

    let alpha_at = |fx: f32, fy: f32| -> u8 {
        let x = (fx * w as f32) as usize;
        let y = (fy * h as f32) as usize;
        image.pixels[y * w + x].a()
    };

    // Inside the first tooth: left of the axis, a sixth of the way down.
    let tooth = alpha_at(0.30, 0.16);
    // The same height, mirrored to the right of the axis — between two teeth.
    let gap = alpha_at(0.70, 0.16);

    assert_eq!(
        tooth, 255,
        "the point inside the first tooth is not solid ink — the even-odd \
         fill has run the wrong way round, or the flattening has lost a \
         subpath (gap reads {gap})"
    );
    assert_eq!(
        gap, 0,
        "the point in the gap beside the first tooth carries ink — the \
         even-odd fill has run the wrong way round (tooth reads {tooth})"
    );

    // And the mark is not the whole square either way: a mask that came back
    // solid, or empty, would satisfy one of the two assertions above on its
    // own.
    let inked = image.pixels.iter().filter(|p| p.a() > 0).count();
    let total = w * h;
    assert!(
        inked > total / 8 && inked < total / 2,
        "the mask covers {inked} of {total} texels, which is not a mark — it \
         is a solid block or an empty one"
    );
}
