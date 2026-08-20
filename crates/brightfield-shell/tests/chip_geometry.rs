//! The horizontal geometry of the two chip primitives, measured at *this*
//! repo's call sites and stated from design-system constants.
//!
//! WHY THIS FILE EXISTS, AND WHY THE PICTURES ARE NOT ENOUGH
//!
//! The design system moved the status pill's and the keycap chip's horizontal
//! inset from a raw ladder index onto a named `CHIP_PADDING_X`, which is one
//! ladder step wider. Four committed baselines in this crate carry a pill or a
//! chip, so all four move. A baseline that moves is graded by its own image
//! diff, and an image diff cannot tell a pill that got 4.0 pt of padding from a
//! pill that got 4.0 pt of something else — so before those pictures were
//! re-captured, this file was written to hold the geometry independently of
//! them. The pictures are then evidence of a change that is already pinned,
//! rather than the only evidence for it.
//!
//! WHAT WAS HOLDING IT BEFORE: nothing on this axis. `gallery_gate.rs` asks
//! each specimen for its accessibility label and for its *height* against a
//! named rung; `token_discipline.rs` greps source text. The capsule's width is
//! the thing that moved and no test in this crate mentioned it. Reverting the
//! upstream primitives to their old inset leaves both of those suites green —
//! see the failure text on the branch that added this file.
//!
//! HOW IT MEASURES. Every expectation below is laid out from
//! `meridian_design` constants here, never asked of the code under test, and
//! every measurement is read back out of the shapes the frame actually
//! painted. The specimens are rendered through `gallery::solo` — the same
//! composition the goldens photograph — so a frame that measured clean and a
//! frame that was photographed are the same frame.
//!
//! This is a deliberately separate reader from the one in `arrangement.rs`: a
//! bug in a shared walk would hide from both.

use brightfield_shell::chart_item::run_state_pill;
use brightfield_shell::design::Mode;
use brightfield_shell::gallery::{catalog, region_catalog, region_row, solo};
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

/// The chip-radius boxes and the text runs a frame painted.
struct Painted {
    /// Boxes drawn at the chip corner radius, deduplicated: the pill paints
    /// its fill and its stroke as two rects over the same geometry.
    chips: Vec<egui::Rect>,
    /// Every text run, with the string it laid out.
    texts: Vec<(egui::Rect, String)>,
}

fn painted<S>(harness: &Harness<'_, S>) -> Painted {
    let chip_radius = egui::CornerRadius::from(meridian_design::radius::CHIP);

    fn walk(shape: &egui::Shape, radius: egui::CornerRadius, out: &mut Painted) {
        match shape {
            egui::Shape::Rect(r) if r.corner_radius == radius => out.push_chip(r.rect),
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
    /// Record a chip box, ignoring one already seen at the same geometry: the
    /// pill paints its fill and its stroke as two rects over one rectangle.
    fn push_chip(&mut self, rect: egui::Rect) {
        let seen = self.chips.iter().any(|c| {
            near(c.left(), rect.left())
                && near(c.right(), rect.right())
                && near(c.top(), rect.top())
                && near(c.bottom(), rect.bottom())
        });
        if !seen {
            self.chips.push(rect);
        }
    }

    /// Each chip box paired with the one text run whose centre it holds, keyed
    /// by that run's string.
    ///
    /// Pairing by containment rather than by paint order is what lets the same
    /// reader serve a lone pill and a row of them, and keys the failure message
    /// to the label a reader can see in the picture.
    fn chips_by_label(&self) -> Vec<(String, egui::Rect, egui::Rect)> {
        let mut out = Vec::new();
        for chip in &self.chips {
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
            out.push((inside[0].1.clone(), *chip, inside[0].0));
        }
        out
    }

    /// The same pairing, asserted to be exactly the specimens expected.
    ///
    /// The guard against a vacuous pass: a `for` loop over an empty reading
    /// asserts nothing, and a frame that drew a different set of chips is not
    /// the frame under test.
    fn expect(&self, labels: &[&str]) -> Vec<(String, egui::Rect, egui::Rect)> {
        let found = self.chips_by_label();
        let mut names: Vec<&str> = found.iter().map(|(l, _, _)| l.as_str()).collect();
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

/// One gallery specimen rendered solo, in the composition `gallery_gate.rs`
/// photographs.
fn specimen(id: &str, mode: Mode) -> Painted {
    let mut component = catalog()
        .into_iter()
        .find(|c| c.info().id == id)
        .unwrap_or_else(|| panic!("no gallery component with id {id:?}"));
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

fn assert_pill(where_: &str, mode: Mode, label: &str, capsule: egui::Rect, text: egui::Rect) {
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

// ---------------------------------------------------------------------------
// The gallery specimens — the two goldens that carry a chip
// ---------------------------------------------------------------------------

/// The labels the status-pill specimen draws, in the vocabulary that file
/// supplies to the generic widget.
const PILL_SPECIMEN: [&str; 3] = ["ok", "waiting", "failing"];

/// The keystrokes the key-chip specimen draws. `Save` is a button, not a
/// keycap, so it is not here — and the pairing in `Painted::chips_by_label`
/// would report it if it ever drew inside a chip box.
const CHIP_SPECIMEN: [&str; 3] = ["Esc", "⌘S", "Space"];

/// Every pill in the gallery's own specimen is inset from its capsule by the
/// named chip padding, on both edges.
///
/// This is the defect the token was introduced for, stated as the picture
/// shows it: the outer inset used to be one ladder step *below* the gap
/// between the icon and the label, so the group sat looser in its middle than
/// it sat inside the capsule.
#[test]
fn the_gallery_pills_spend_the_named_chip_padding_on_both_outer_edges() {
    for mode in [Mode::Light, Mode::Dark] {
        let painted = specimen("status-pill", mode);
        for (label, capsule, text) in painted.expect(&PILL_SPECIMEN) {
            assert_pill("status-pill specimen:", mode, &label, capsule, text);
        }
    }
}

/// The keycap spends the same named padding, which is the half of the token
/// that makes it shared rather than a pill constant with a general name.
///
/// The hairline is in the expectation because the keycap's box is a stroked
/// `egui::Frame` and the painted rect carries the stroke outside the margin.
#[test]
fn the_gallery_keycaps_spend_the_same_named_chip_padding() {
    for mode in [Mode::Light, Mode::Dark] {
        let painted = specimen("key-chip", mode);
        let expected = CHIP_PADDING_X + HAIRLINE;
        for (keystroke, chip, text) in painted.expect(&CHIP_SPECIMEN) {
            let leading = text.left() - chip.left();
            let trailing = chip.right() - text.right();
            assert!(
                near(leading, expected),
                "key-chip specimen: {mode:?} {keystroke:?}: leading inset {leading} \
                 is not CHIP_PADDING_X + hairline ({expected})"
            );
            assert!(
                near(trailing, expected),
                "key-chip specimen: {mode:?} {keystroke:?}: trailing inset \
                 {trailing} is not CHIP_PADDING_X + hairline ({expected})"
            );
            assert!(
                near(chip.width(), text.width() + 2.0 * expected),
                "key-chip specimen: {mode:?} {keystroke:?}: the keycap drew {} wide \
                 against the {} its terms add up to",
                chip.width(),
                text.width() + 2.0 * expected
            );
        }
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
            for (label, capsule, text) in painted.expect(&[state.label()]) {
                assert_pill("run-state pill:", mode, &label, capsule, text);
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
    for (label, capsule, text) in painted.expect(&labels) {
        assert_pill("region row:", mode, &label, capsule, text);
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
        for (label, capsule, text) in painted.expect(&PILL_SPECIMEN) {
            let former = FORMER_INSET + ICON_XS + ICON_LABEL_GAP + text.width() + FORMER_INSET;
            assert!(
                near(capsule.width() - former, WIDENING_PER_CHIP),
                "status-pill specimen: {mode:?} {label:?}: the capsule drew {} \
                 wide against the {former} it drew before the token, a difference \
                 of {} rather than {WIDENING_PER_CHIP}",
                capsule.width(),
                capsule.width() - former
            );
        }

        let painted = specimen("key-chip", mode);
        for (keystroke, chip, text) in painted.expect(&CHIP_SPECIMEN) {
            let former = text.width() + 2.0 * (FORMER_INSET + HAIRLINE);
            assert!(
                near(chip.width() - former, WIDENING_PER_CHIP),
                "key-chip specimen: {mode:?} {keystroke:?}: the keycap drew {} wide \
                 against the {former} it drew before the token, a difference of {} \
                 rather than {WIDENING_PER_CHIP}",
                chip.width(),
                chip.width() - former
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
        for (label, capsule, _) in painted.expect(&PILL_SPECIMEN) {
            assert!(
                near(capsule.height(), HEIGHT_XS),
                "status-pill specimen: {mode:?} {label:?}: the capsule drew {} tall \
                 against the {HEIGHT_XS} rung it sits on",
                capsule.height()
            );
        }

        let painted = specimen("key-chip", mode);
        for (keystroke, chip, text) in painted.expect(&CHIP_SPECIMEN) {
            let expected = text.height() + 2.0 * (SPACE_1 + HAIRLINE);
            assert!(
                near(chip.height(), expected),
                "key-chip specimen: {mode:?} {keystroke:?}: the keycap drew {} tall \
                 against the {expected} its terms add up to",
                chip.height()
            );
        }
    }
}
