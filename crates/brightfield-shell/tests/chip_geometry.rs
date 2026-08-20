//! The horizontal geometry of the two chip primitives, measured at *this*
//! repo's call sites and stated from design-system constants.
//!
//! WHY THIS FILE EXISTS, AND WHY THE PICTURES ARE NOT ENOUGH
//!
//! The design system moved the status pill's and the keycap chip's horizontal
//! inset from a raw ladder index onto a named `CHIP_PADDING_X`, which is one
//! ladder step wider. Ten committed baselines in this crate carry a pill or a
//! chip, so all ten move — a count that came out of running the suites, and
//! that is six higher than reading the call sites suggested. A baseline that
//! moves is graded by its own image diff, and an image diff cannot tell a pill
//! that got 4.0 pt of padding from a pill that got 4.0 pt of something else —
//! so before those pictures were re-captured, this file was written to hold
//! the geometry independently of them. The pictures are then evidence of a
//! change that is already pinned, rather than the only evidence for it.
//!
//! WHAT WAS HOLDING IT BEFORE: nothing on this axis. `gallery_gate.rs` asks
//! each specimen for its accessibility label and for its *height* against a
//! named rung; `token_discipline.rs` greps source text. The capsule's width is
//! the thing that moved and no test in this crate mentioned it. Measured, with
//! the upstream primitives patched back to the inset they used to spend:
//! `token_discipline` passed 2 of 2 and the nine non-pixel tests in
//! `gallery_gate` all passed. The picture was the whole of the guard.
//!
//! HOW IT MEASURES. Every expectation below is laid out from
//! `meridian_design` constants here, never asked of the code under test, and
//! every measurement is read back out of the shapes the frame actually
//! painted. The specimens are rendered through `gallery::solo` — the same
//! composition the goldens photograph — so a frame that measured clean and a
//! frame that was photographed are the same frame.
//!
//! WHICH DRAWINGS ARE COVERED. Every gated gallery specimen, enumerated by
//! rendering the catalog rather than by listing the ones expected to carry a
//! chip; plus two call sites with no specimen of their own, the run-state pill
//! on a chart item and the region listing's row.
//!
//! `shell_palette_open_{light,dark}` also carries keycaps and also moves, and
//! is NOT read here: `surfaces.rs` builds that frame as a rasterised image
//! rather than a shape list, so measuring it would mean a second copy of the
//! shell harness. It is held by the primitive assertions below plus the
//! wrong-state guard that baseline already carries, and not by a direct read
//! of its own frame.
//!
//! This is a deliberately separate reader from the one in `arrangement.rs`: a
//! bug in a shared walk would hide from both.

use brightfield_shell::chart_item::run_state_pill;
use brightfield_shell::design::Mode;
use brightfield_shell::gallery::{catalog, region_catalog, region_row, solo, Component};
use brightfield_workbench::subject::RunState;
use egui_kittest::Harness;
use meridian_design::control::{HEIGHT_XS, ICON_XS};
use meridian_design::spacing::{CHIP_PADDING_X, ICON_LABEL_GAP, SPACE_1, SPACE_2, SPACE_3};

/// The hairline every box in the design system is stroked with. Spelled out
/// here rather than borrowed from the code under test, so the keycap's
/// expected inset is stated term by term.
const HAIRLINE: f32 = 1.0;

/// The inset both chip primitives used to spend before the named token
/// existed: the ladder's second step, reached for by index at each call site.
/// It is here so the widening can be asserted as a measured difference rather
/// than described in a comment.
const FORMER_INSET: f32 = SPACE_2;

/// What each chip gained on each edge, and therefore twice that in width.
const WIDENING_PER_CHIP: f32 = 2.0 * (CHIP_PADDING_X - FORMER_INSET);

/// Logical pixels here are exact multiples of the ladder, so this is float
/// noise tolerance and nothing else. The smallest difference any assertion
/// below is built to catch is 2.0.
const EPS: f32 = 0.01;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

// ---------------------------------------------------------------------------
// Reading the frame
// ---------------------------------------------------------------------------

/// Which primitive a chip-radius box came from, told apart by HOW it draws
/// rather than by the geometry under test.
///
/// The status pill lays its capsule down as two rects over one rectangle — an
/// opaque fill with no stroke, then a hairline stroke over a transparent fill.
/// The keycap is an `egui::Frame`, which emits a single rect carrying both. So
/// the discriminator is the pair (fill, stroke width), and it is independent of
/// every inset this file measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Primitive {
    /// A status pill: icon, gap, label, inside a capsule.
    Pill,
    /// A keycap chip: a monospace galley inside a stroked box.
    Keycap,
}

/// The chip-radius boxes and the text runs a frame painted.
struct Painted {
    /// Boxes drawn at the chip corner radius, deduplicated: the pill's stroke
    /// pass repeats the geometry its fill pass already reported.
    chips: Vec<(egui::Rect, Primitive)>,
    /// Every text run, with the string it laid out.
    texts: Vec<(egui::Rect, String)>,
}

fn painted<S>(harness: &Harness<'_, S>) -> Painted {
    let chip_radius = egui::CornerRadius::from(meridian_design::radius::CHIP);

    fn walk(shape: &egui::Shape, radius: egui::CornerRadius, out: &mut Painted) {
        match shape {
            egui::Shape::Rect(r) if r.corner_radius == radius => {
                out.push_chip(r.rect, r.fill, r.stroke.width);
            }
            egui::Shape::Text(t) => out.texts.push((
                egui::Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.text().to_owned(),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, radius, out)),
            _ => {}
        }
    }

    let mut out = Painted {
        chips: Vec::new(),
        texts: Vec::new(),
    };
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, chip_radius, &mut out);
    }
    out
}

impl Painted {
    /// Record a chip box under the primitive that drew it.
    ///
    /// A transparent fill is the pill's stroke pass, which repeats geometry
    /// already recorded, so it is dropped rather than deduplicated by position
    /// — two chips genuinely sharing a rectangle would be a frame worth seeing.
    fn push_chip(&mut self, rect: egui::Rect, fill: egui::Color32, stroke_width: f32) {
        if fill.a() == 0 {
            return;
        }
        let primitive = if stroke_width > 0.0 {
            Primitive::Keycap
        } else {
            Primitive::Pill
        };
        self.chips.push((rect, primitive));
    }

    /// Each chip box paired with the one text run whose centre it holds, keyed
    /// by that run's string.
    ///
    /// Pairing by containment rather than by paint order is what lets the same
    /// reader serve a lone pill and a row of them, and keys the failure message
    /// to the label a reader can see in the picture.
    fn chips_by_label(&self) -> Vec<Chip> {
        let mut out = Vec::new();
        for (chip, primitive) in &self.chips {
            let inside: Vec<&(egui::Rect, String)> = self
                .texts
                .iter()
                .filter(|(t, _)| chip.contains(t.center()))
                .collect();
            assert_eq!(
                inside.len(),
                1,
                "a chip box at {chip:?} holds {} text runs, not one — the frame \
                 is not the frame this test measures",
                inside.len()
            );
            out.push(Chip {
                label: inside[0].1.clone(),
                primitive: *primitive,
                box_: *chip,
                text: inside[0].0,
            });
        }
        out
    }

    /// The same pairing, asserted to be exactly the specimens expected.
    ///
    /// The guard against a vacuous pass: a `for` loop over an empty reading
    /// asserts nothing, and a frame that drew a different set of chips is not
    /// the frame under test.
    fn expect(&self, labels: &[&str]) -> Vec<Chip> {
        let found = self.chips_by_label();
        let mut names: Vec<&str> = found.iter().map(|c| c.label.as_str()).collect();
        names.sort_unstable();
        let mut want: Vec<&str> = labels.to_vec();
        want.sort_unstable();
        assert_eq!(
            names, want,
            "the frame drew chips {names:?}, wanted {want:?}"
        );
        found
    }
}

/// One chip box, the primitive that drew it and the label it holds.
struct Chip {
    label: String,
    primitive: Primitive,
    box_: egui::Rect,
    text: egui::Rect,
}

/// One gallery specimen rendered solo, in the composition `gallery_gate.rs`
/// photographs.
fn specimen(id: &str, mode: Mode) -> Painted {
    let component = catalog()
        .into_iter()
        .find(|c| c.info().id == id)
        .unwrap_or_else(|| panic!("no gallery component with id {id:?}"));
    drawn_solo(component, mode)
}

/// One catalog entry rendered solo.
fn drawn_solo(mut component: Box<dyn Component>, mode: Mode) -> Painted {
    let (w, h) = component.info().solo_size;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(w, h))
        .build_ui(move |ui| solo(ui, mode, component.as_mut()));
    harness.run();
    painted(&harness)
}

/// One frame of arbitrary shell drawing, themed the way the shell themes it.
fn frame(mode: Mode, size: (f32, f32), draw: impl Fn(&mut egui::Ui) + 'static) -> Painted {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui(move |ui| {
            brightfield_shell::design::apply(ui.ctx(), mode);
            egui::CentralPanel::default().show(ui, |ui| draw(ui));
        });
    harness.run();
    painted(&harness)
}

/// The capsule's leading inset: what is left of the run from the capsule's
/// edge to the label once the icon and the icon-label gap are taken out of it.
/// The trailing inset is the padding directly.
fn pill_insets(capsule: egui::Rect, text: egui::Rect) -> (f32, f32) {
    let leading = (text.left() - capsule.left()) - ICON_XS - ICON_LABEL_GAP;
    let trailing = capsule.right() - text.right();
    (leading, trailing)
}

fn assert_pill(where_: &str, mode: Mode, chip: &Chip) {
    let (label, capsule, text) = (&chip.label, chip.box_, chip.text);
    assert_eq!(
        chip.primitive,
        Primitive::Pill,
        "{where_} {mode:?} {label:?}: measured as a pill but drawn as a keycap"
    );
    let (leading, trailing) = pill_insets(capsule, text);
    assert!(
        near(leading, CHIP_PADDING_X),
        "{where_} {mode:?} {label:?}: leading inset {leading} is not CHIP_PADDING_X \
         ({CHIP_PADDING_X})"
    );
    assert!(
        near(trailing, CHIP_PADDING_X),
        "{where_} {mode:?} {label:?}: trailing inset {trailing} is not CHIP_PADDING_X \
         ({CHIP_PADDING_X})"
    );
    assert!(
        trailing + EPS >= ICON_LABEL_GAP,
        "{where_} {mode:?} {label:?}: the rhythm is inverted — the capsule insets \
         the group by {trailing} while the group is {ICON_LABEL_GAP} loose in its \
         middle"
    );
    let expected = CHIP_PADDING_X + ICON_XS + ICON_LABEL_GAP + text.width() + CHIP_PADDING_X;
    assert!(
        near(capsule.width(), expected),
        "{where_} {mode:?} {label:?}: the capsule drew {} wide against the \
         {expected} its terms add up to",
        capsule.width()
    );
}

fn assert_keycap(where_: &str, mode: Mode, chip: &Chip) {
    let (keystroke, box_, text) = (&chip.label, chip.box_, chip.text);
    assert_eq!(
        chip.primitive,
        Primitive::Keycap,
        "{where_} {mode:?} {keystroke:?}: measured as a keycap but drawn as a pill"
    );
    // The hairline is in the expectation because the keycap's box is a stroked
    // `egui::Frame` and the painted rect carries the stroke outside the margin.
    let expected = CHIP_PADDING_X + HAIRLINE;
    let leading = text.left() - box_.left();
    let trailing = box_.right() - text.right();
    assert!(
        near(leading, expected),
        "{where_} {mode:?} {keystroke:?}: leading inset {leading} is not \
         CHIP_PADDING_X + hairline ({expected})"
    );
    assert!(
        near(trailing, expected),
        "{where_} {mode:?} {keystroke:?}: trailing inset {trailing} is not \
         CHIP_PADDING_X + hairline ({expected})"
    );
    assert!(
        near(box_.width(), text.width() + 2.0 * expected),
        "{where_} {mode:?} {keystroke:?}: the keycap drew {} wide against the {} \
         its terms add up to",
        box_.width(),
        text.width() + 2.0 * expected
    );
}

// ---------------------------------------------------------------------------
// The gallery specimens — every one of them, not a list
// ---------------------------------------------------------------------------

/// The labels the status-pill specimen draws, in the vocabulary that file
/// supplies to the generic widget.
const PILL_SPECIMEN: [&str; 3] = ["ok", "waiting", "failing"];

/// The keystrokes the key-chip specimen draws. `Save` is a button, not a
/// keycap, so it is not here — and the pairing in `Painted::chips_by_label`
/// would report it if it ever drew inside a chip box.
const CHIP_SPECIMEN: [&str; 3] = ["Esc", "⌘S", "Space"];

/// Which gated specimens draw a chip, and how many of each primitive.
///
/// This census is the thing that predicts which committed baselines move when
/// the chip geometry changes, and it is asserted rather than written down: a
/// specimen that starts or stops drawing a chip reddens
/// `every_chip_in_every_gallery_specimen_spends_the_named_padding` and says so
/// in the failure, instead of being discovered as a golden nobody expected to
/// move. Counts are per frame, and identical in light and dark.
const CHIP_CENSUS: [(&str, usize, usize); 4] = [
    // (specimen id, pills, keycaps)
    ("key-chip", 0, 3),
    ("modal-chrome", 0, 1),
    ("picker", 0, 7),
    ("status-pill", 3, 0),
];

/// Every chip drawn by every gated gallery specimen is inset from its box by
/// the named chip padding, on both edges.
///
/// **Enumerated by rendering the catalog, not by listing the specimens that
/// were expected to carry a chip.** The list would have been wrong: this change
/// was scoped as moving two pairs of goldens, and running it moved five. Three
/// of those pairs draw a keycap somewhere a reader of the card would not think
/// to look — a modal's footer, a picker's shortcut column. A test that measures
/// what the code draws cannot be short in that way, which is why this replaced
/// a pair of per-specimen tests.
///
/// The inversion the token was introduced to end is asserted per chip inside
/// `assert_pill`: the outer inset used to be one ladder step BELOW the gap
/// between the icon and the label, so the group sat looser in its middle than
/// it sat inside the capsule.
#[test]
fn every_chip_in_every_gallery_specimen_spends_the_named_padding() {
    for mode in [Mode::Light, Mode::Dark] {
        let mut census: Vec<(&str, usize, usize)> = Vec::new();
        for component in catalog() {
            let info = component.info();
            if !info.status.gated() {
                continue;
            }
            let id = info.id;
            let painted = drawn_solo(component, mode);
            let chips = painted.chips_by_label();
            if chips.is_empty() {
                continue;
            }
            let mut pills = 0;
            let mut keycaps = 0;
            for chip in &chips {
                match chip.primitive {
                    Primitive::Pill => {
                        pills += 1;
                        assert_pill(id, mode, chip);
                    }
                    Primitive::Keycap => {
                        keycaps += 1;
                        assert_keycap(id, mode, chip);
                    }
                }
            }
            census.push((id, pills, keycaps));
        }
        census.sort_unstable();
        assert_eq!(
            census,
            CHIP_CENSUS.to_vec(),
            "{mode:?}: the specimens drawing a chip are not the ones this file \
             was written against — every entry here owns a pair of committed \
             baselines that move when the chip geometry moves"
        );
    }
}

// ---------------------------------------------------------------------------
// The call sites outside the gallery specimens
// ---------------------------------------------------------------------------

/// The run-state pill on a chart item spends the same padding.
///
/// A separate call site from the gallery's, with its own vocabulary and its
/// own icons, and it rides other committed baselines. It is measured here so
/// the token's reach is a fact about the drawing rather than an inference from
/// the two specimens the gallery happens to expose.
#[test]
fn the_run_state_pill_spends_the_named_chip_padding() {
    for mode in [Mode::Light, Mode::Dark] {
        for state in RunState::ALL {
            let painted = frame(mode, (420.0, 120.0), move |ui| {
                run_state_pill(ui, state);
            });
            for chip in &painted.expect(&[state.label()]) {
                assert_pill("run-state pill", mode, chip);
            }
        }
    }
}

/// The region listing's pill spends the same padding.
///
/// The gallery draws a pill for a component's status and a pill for a region's
/// status through one helper. This is the second of those, reached through the
/// public row so a reader can see which drawing is under test.
#[test]
fn the_region_row_pill_spends_the_named_chip_padding() {
    let mode = Mode::Light;
    let painted = frame(mode, (560.0, 1400.0), |ui| {
        for entry in region_catalog() {
            region_row(ui, entry);
        }
    });
    let labels: Vec<&str> = region_catalog().iter().map(|e| e.status.label()).collect();
    for chip in &painted.expect(&labels) {
        assert_pill("region row", mode, chip);
    }
}

// ---------------------------------------------------------------------------
// The size of the change
// ---------------------------------------------------------------------------

/// Each chip is wider by twice the step the named token added, measured off
/// the box it drew rather than asserted between two constants.
///
/// This is what separates "the pill got wider" from "the pill got wider by the
/// amount the design ruling costs". Its bound, stated rather than left to be
/// discovered: it holds `CHIP_PADDING_X` against the ladder step the call
/// sites used to reach for, so it reddens if the token moves off that step and
/// it does not judge the ladder itself.
#[test]
fn each_chip_grew_by_the_step_the_named_token_added() {
    assert!(
        near(CHIP_PADDING_X, SPACE_3),
        "CHIP_PADDING_X is {CHIP_PADDING_X}, not the {SPACE_3} step the chip \
         geometry was signed off at"
    );
    assert!(
        near(WIDENING_PER_CHIP, 4.0),
        "the widening is {WIDENING_PER_CHIP} pt per chip, not the 4.0 the \
         re-captured baselines were authored against"
    );

    for mode in [Mode::Light, Mode::Dark] {
        let painted = specimen("status-pill", mode);
        for chip in &painted.expect(&PILL_SPECIMEN) {
            let former = FORMER_INSET + ICON_XS + ICON_LABEL_GAP + chip.text.width() + FORMER_INSET;
            let grew = chip.box_.width() - former;
            assert!(
                near(grew, WIDENING_PER_CHIP),
                "status-pill specimen: {mode:?} {:?}: the capsule drew {} wide \
                 against the {former} it drew before the token, a difference of \
                 {grew} rather than {WIDENING_PER_CHIP}",
                chip.label,
                chip.box_.width()
            );
        }

        let painted = specimen("key-chip", mode);
        for chip in &painted.expect(&CHIP_SPECIMEN) {
            let former = chip.text.width() + 2.0 * (FORMER_INSET + HAIRLINE);
            let grew = chip.box_.width() - former;
            assert!(
                near(grew, WIDENING_PER_CHIP),
                "key-chip specimen: {mode:?} {:?}: the keycap drew {} wide against \
                 the {former} it drew before the token, a difference of {grew} \
                 rather than {WIDENING_PER_CHIP}",
                chip.label,
                chip.box_.width()
            );
        }
    }
}

/// Naming the chip inset is a horizontal change, and the height ladder is not
/// part of it.
///
/// The capsule sits on a control rung; the keycap is its galley plus the
/// ladder's smallest step and the hairline, top and bottom. Both are stated
/// from constants, so a horizontal edit that reached the vertical axis by
/// accident reddens here rather than inside a picture.
#[test]
fn neither_chip_moved_on_the_vertical_ladder() {
    for mode in [Mode::Light, Mode::Dark] {
        let painted = specimen("status-pill", mode);
        for chip in &painted.expect(&PILL_SPECIMEN) {
            assert!(
                near(chip.box_.height(), HEIGHT_XS),
                "status-pill specimen: {mode:?} {:?}: the capsule drew {} tall \
                 against the {HEIGHT_XS} rung it sits on",
                chip.label,
                chip.box_.height()
            );
        }

        let painted = specimen("key-chip", mode);
        for chip in &painted.expect(&CHIP_SPECIMEN) {
            let expected = chip.text.height() + 2.0 * (SPACE_1 + HAIRLINE);
            assert!(
                near(chip.box_.height(), expected),
                "key-chip specimen: {mode:?} {:?}: the keycap drew {} tall against \
                 the {expected} its terms add up to",
                chip.label,
                chip.box_.height()
            );
        }
    }
}
