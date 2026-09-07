//! What text a frame put on the screen, and whether any two of it landed in
//! the same pixels.
//!
//! A pane that draws two labels into one place is the defect a reader sees
//! before any other, and until this module existed nothing in this repository
//! could fail because of it: each site that collided was found by looking at a
//! screenshot. Looking does not scale to the panes nobody thought to open, and
//! it does not cover the pane written next.
//!
//! [`frame_text`] reads a pass's own paint lists — every galley, the layer it
//! was painted into, the clip it was painted under — and [`collisions`] asks
//! whether any two of them share pixels. A test drives the real shell for one
//! pass and asserts the list is empty, so the check runs over whatever that
//! shell happens to draw rather than over the sites somebody remembered.
//!
//! # What it reads, and what it therefore cannot see
//!
//! The ink box, not the line box. [`epaint::Galley::mesh_bounds`] is the tight
//! bounding box of the glyph meshes; `Galley::rect` is the font's line box,
//! which is taller than its glyphs by the leading and wider than them by the
//! side bearing. Two rows of captions a row apart share a fraction of a point
//! of *line box* and no pixel of ink, so a check reading `rect` would have to
//! be given a tolerance big enough to hide a real one-line collision. Reading
//! `mesh_bounds` is what makes [`MIN_OVERLAP`] small enough to be honest.
//!
//! Text the egui pass painted. The Vello canvas draws through an
//! [`epaint::Shape::Callback`], so a mark's own labels are not in these lists
//! and this module says nothing about them. Everything outside the canvas
//! rect — the rails, the grid, the header band, the inspector, the sheet, the
//! top bar — is.

use egui::epaint::{ClippedShape, Shape};

/// The overlap, in logical points on both axes, at or under which two ink
/// boxes are called adjacent rather than collided.
///
/// Small because the boxes are tight: at 0.5 points a collision has to put
/// ink into ink, not a line box into a line box. It is not zero because a
/// glyph mesh carries the anti-aliasing skirt epaint tessellates around it,
/// and two labels set flush against each other would otherwise read as a
/// defect at the shared edge.
pub const MIN_OVERLAP: f32 = 0.5;

// ---------------------------------------------------------------------------
// What the frame drew.
// ---------------------------------------------------------------------------

/// One galley a pass painted, with everything the rule needs to judge it.
#[derive(Clone, Debug)]
pub struct DrawnText {
    /// The string, as laid out — an elided galley carries its ellipsis.
    pub text: String,
    /// The layer it was painted into. Text in a tooltip, a popup or a modal
    /// is in a different layer from the surface under it, which is what a
    /// layer is for.
    pub layer: egui::LayerId,
    /// The tight box around its glyph meshes, in window-space logical points.
    pub ink: egui::Rect,
    /// The clip it was painted under.
    pub clip: egui::Rect,
    /// The part of [`Self::ink`] that reaches the screen: the ink box under
    /// the clip. [`egui::Rect::is_negative`] where the clip took all of it.
    pub visible: egui::Rect,
}

impl DrawnText {
    /// Whether any of this galley reaches the screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.visible.is_negative() && self.visible.width() > 0.0 && self.visible.height() > 0.0
    }
}

/// Two galleys that share pixels, and by how much.
#[derive(Clone, Debug)]
pub struct TextCollision {
    /// The first of them, in paint order.
    pub a: DrawnText,
    /// The second.
    pub b: DrawnText,
    /// The box they share.
    pub overlap: egui::Rect,
}

impl std::fmt::Display for TextCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {:?} and {:?} at {:?} share {:.1}x{:.1} points at {:?}",
            self.a.text,
            self.a.visible,
            self.b.text,
            self.b.visible,
            self.overlap.width(),
            self.overlap.height(),
            self.overlap,
        )
    }
}

// ---------------------------------------------------------------------------
// The exemptions.
// ---------------------------------------------------------------------------

/// One reason two galleys sharing a box is not a defect.
///
/// A variant is inert until it appears in [`EXEMPTIONS`]: [`is_collision`]
/// reaches an exemption only by walking that table, so a new reason takes
/// effect when somebody adds a row to it with a sentence saying why — never by
/// a condition growing quietly inside a predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The two are in different layers.
    DifferentLayer,
    /// One of them is clipped away.
    NotVisible,
    /// They share less than [`MIN_OVERLAP`] on an axis.
    Adjacent,
    /// The pair is named in [`EXEMPT_PAIRS`].
    NamedPair,
}

/// An exemption, and the reason it is one.
#[derive(Clone, Copy, Debug)]
pub struct Exemption {
    /// Which condition this row turns on.
    pub rule: Rule,
    /// Why a pair meeting it is not a defect. Read by a person, and the point
    /// of the table: an exemption whose reason does not survive being read
    /// aloud is one to delete.
    pub because: &'static str,
}

/// **Everything this check does not call a collision.** Four rows today.
///
/// A pair that no row here excuses is a defect, so this table is the whole of
/// the check's judgement and the only place to look to audit it. Each row is
/// exercised on its own by `every_exemption_excuses_a_case_and_no_other`,
/// which is what keeps a row that no longer excuses anything from sitting here
/// looking load-bearing.
pub const EXEMPTIONS: &[Exemption] = &[
    Exemption {
        rule: Rule::DifferentLayer,
        because: "a tooltip, a popup and a modal are drawn ABOVE the surface \
                  they cover, and a layer is how egui says so. Two galleys in \
                  one layer have no such order between them, which is why the \
                  check looks there and only there.",
    },
    Exemption {
        rule: Rule::NotVisible,
        because: "text the clip took is not on the screen. A column scrolled \
                  out of a pane still paints its galley, at the position it \
                  would occupy, under a clip that excludes it — judging it \
                  would fail a pane for text no reader can see.",
    },
    Exemption {
        rule: Rule::Adjacent,
        because: "a glyph mesh is tessellated with an anti-aliasing skirt \
                  around its outline, so two labels set flush against one \
                  another share a sliver of box at the seam. MIN_OVERLAP says \
                  how much sharing is the seam rather than a collision.",
    },
    Exemption {
        rule: Rule::NamedPair,
        because: "a specific pair somebody decided is drawn over another on \
                  purpose. Empty today: nothing in the shell has yet needed \
                  one, and the row is here so that adding one is an edit to \
                  EXEMPT_PAIRS with a sentence in it.",
    },
];

/// A pair of exact strings allowed to share pixels, and why.
#[derive(Clone, Copy, Debug)]
pub struct ExemptPair {
    /// One galley's text, exactly as laid out.
    pub a: &'static str,
    /// The other's. Order does not matter.
    pub b: &'static str,
    /// Why this pair is drawn over that one on purpose.
    pub because: &'static str,
}

/// The pairs [`Rule::NamedPair`] excuses.
///
/// **Empty.** Every collision this check has been pointed at so far was a
/// defect, and the two it was written for were fixed rather than listed. A
/// pair added here is a claim that a reader is meant to see two strings in one
/// place, which is a claim worth having to write down.
pub const EXEMPT_PAIRS: &[ExemptPair] = &[];

impl Rule {
    /// Whether this rule excuses `a` overlapping `b` by `overlap`.
    fn excuses(self, a: &DrawnText, b: &DrawnText, overlap: egui::Rect) -> bool {
        match self {
            Self::DifferentLayer => a.layer != b.layer,
            Self::NotVisible => !a.is_visible() || !b.is_visible(),
            Self::Adjacent => {
                overlap.is_negative()
                    || overlap.width() <= MIN_OVERLAP
                    || overlap.height() <= MIN_OVERLAP
            }
            Self::NamedPair => EXEMPT_PAIRS.iter().any(|pair| {
                (pair.a == a.text && pair.b == b.text) || (pair.a == b.text && pair.b == a.text)
            }),
        }
    }
}

/// Whether these two galleys sharing a box is a defect.
///
/// The only route to an exemption is [`EXEMPTIONS`]: a [`Rule`] variant that
/// nobody has written a row for excuses nothing.
#[must_use]
pub fn is_collision(a: &DrawnText, b: &DrawnText) -> bool {
    let overlap = a.visible.intersect(b.visible);
    !EXEMPTIONS
        .iter()
        .any(|exemption| exemption.rule.excuses(a, b, overlap))
}

// ---------------------------------------------------------------------------
// Reading a pass.
// ---------------------------------------------------------------------------

/// The window-space ink box of one text shape.
///
/// `mesh_bounds` is galley-local; `pos` places it. A rotated galley turns
/// about `pos`, so its box is the axis-aligned hull of the four turned
/// corners rather than the turned rect, which is not one.
fn ink_box(text: &egui::epaint::TextShape) -> egui::Rect {
    let local = text.galley.mesh_bounds;
    if local.is_negative() {
        return egui::Rect::NOTHING;
    }
    let at = text.pos.to_vec2();
    if text.angle == 0.0 {
        return local.translate(at);
    }
    let (sin, cos) = text.angle.sin_cos();
    let mut turned = egui::Rect::NOTHING;
    for corner in [local.left_top(), local.right_top(), local.left_bottom(), local.right_bottom()] {
        turned.extend_with(egui::pos2(
            cos.mul_add(corner.x, -(sin * corner.y)) + at.x,
            sin.mul_add(corner.x, cos * corner.y) + at.y,
        ));
    }
    turned
}

/// Every galley in `shapes`, read as painted into `layer`.
///
/// `Shape::Vec` nests, so this walks. `Shape::Callback` does not — a Vello
/// canvas's own text is not an epaint galley and is not here.
fn texts_of(layer: egui::LayerId, shapes: &[ClippedShape], into: &mut Vec<DrawnText>) {
    fn walk(layer: egui::LayerId, clip: egui::Rect, shape: &Shape, into: &mut Vec<DrawnText>) {
        match shape {
            Shape::Text(text) => {
                let ink = ink_box(text);
                if ink.is_negative() {
                    // No mesh: a galley of nothing but whitespace lays out a
                    // box and puts no ink in it.
                    return;
                }
                into.push(DrawnText {
                    text: text.galley.text().to_owned(),
                    layer,
                    ink,
                    clip,
                    visible: ink.intersect(clip),
                });
            }
            Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(layer, clip, shape, into);
                }
            }
            _ => {}
        }
    }
    for clipped in shapes {
        walk(layer, clipped.clip_rect, &clipped.shape, into);
    }
}

/// Every text the pass now in flight has painted, layer by layer.
///
/// **Call this from inside the frame closure, after the app has drawn.** egui
/// flattens its paint lists into one `Vec<ClippedShape>` when the pass ends,
/// and the layer each shape came from is not in that list — so a caller
/// reading `FullOutput::shapes` cannot tell a tooltip's text from the text
/// under it, and would have to excuse by name what a layer already says.
#[must_use]
pub fn frame_text(ctx: &egui::Context) -> Vec<DrawnText> {
    let mut layers: Vec<egui::LayerId> = vec![egui::LayerId::background()];
    ctx.memory(|memory| {
        for layer in memory.layer_ids() {
            if !layers.contains(&layer) {
                layers.push(layer);
            }
        }
    });
    let mut out = Vec::new();
    for layer in layers {
        ctx.graphics(|graphics| {
            if let Some(list) = graphics.get(layer) {
                let shapes: Vec<ClippedShape> = list.all_entries().cloned().collect();
                texts_of(layer, &shapes, &mut out);
            }
        });
    }
    out
}

/// Every pair of `texts` that shares pixels under the rule.
///
/// Quadratic in the number of galleys on the frame, which is a few hundred for
/// the whole window and is what a test can afford once per pass.
#[must_use]
pub fn collisions(texts: &[DrawnText]) -> Vec<TextCollision> {
    let mut out = Vec::new();
    for (i, a) in texts.iter().enumerate() {
        for b in &texts[i + 1..] {
            if is_collision(a, b) {
                out.push(TextCollision {
                    a: a.clone(),
                    b: b.clone(),
                    overlap: a.visible.intersect(b.visible),
                });
            }
        }
    }
    out
}

/// The collisions on the pass now in flight, as a message to fail with.
///
/// `None` when nothing collided.
#[must_use]
pub fn collision_report(ctx: &egui::Context, what: &str) -> Option<String> {
    let found = collisions(&frame_text(ctx));
    if found.is_empty() {
        return None;
    }
    let mut message = format!("{what} drew {} overlapping texts:", found.len());
    for collision in &found {
        message.push_str("\n  ");
        message.push_str(&collision.to_string());
    }
    Some(message)
}

// ---------------------------------------------------------------------------
// Fitting text to the room it has.
// ---------------------------------------------------------------------------

/// Lay `text` out in `font` so it takes at most `room` points of width,
/// eliding with a `…` where it does not fit.
///
/// **Measured, not counted.** The two sites this was written for each budgeted
/// characters — `truncate(right, 20)` in the protocol rail, and the header
/// band's range row, which budgeted nothing at all — and a character budget is
/// a guess at a width that is wrong by however much the glyphs differ from the
/// guess. `TIMESTAMP WITH TIME ZONE` elided to twenty characters is still
/// wider than the room a narrow rail leaves it, which is what put it on top of
/// the column's name. This asks the font.
///
/// A `room` too small for even the ellipsis lays out the ellipsis, because a
/// row that silently drew nothing would read as a missing value rather than as
/// a narrow pane.
#[must_use]
pub fn fit(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    room: f32,
    colour: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(font, colour),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    painter.layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A galley laid out by a real font system, so a test drives the same
    /// measurement the shell does.
    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        // One pass, so the font system exists before anything asks it to lay
        // text out.
        let _ = ctx.run_ui(egui::RawInput::default(), |_ui| {});
        ctx
    }

    fn painter(ctx: &egui::Context) -> egui::Painter {
        egui::Painter::new(
            ctx.clone(),
            egui::LayerId::background(),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0)),
        )
    }

    fn at(layer: egui::LayerId, x: f32, y: f32, w: f32, h: f32, text: &str) -> DrawnText {
        let ink = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        let clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0));
        DrawnText {
            text: text.to_owned(),
            layer,
            ink,
            clip,
            visible: ink.intersect(clip),
        }
    }

    fn base() -> egui::LayerId {
        egui::LayerId::background()
    }

    fn above() -> egui::LayerId {
        egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("a tooltip"))
    }

    /// **Two texts in one layer, sharing more than a seam, are a collision.**
    ///
    /// The base case the whole module exists for, driven at the numbers the
    /// two live defects produced: a label and a type name set at opposite ends
    /// of a row too narrow for both, running into each other in the middle.
    #[test]
    fn two_texts_in_one_layer_sharing_pixels_are_a_collision() {
        let name = at(base(), 10.0, 100.0, 44.0, 9.0, "updated");
        let kind = at(base(), 48.0, 100.0, 60.0, 9.0, "TIMESTAMP WITH TIME\u{2026}");
        assert!(is_collision(&name, &kind));
        let found = collisions(&[name, kind]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            (found[0].overlap.width() - 6.0).abs() < 1e-3,
            "the report says how much they share: {}",
            found[0]
        );
    }

    /// **Each exemption excuses its own case and no other one's.**
    ///
    /// The table is the whole of the check's judgement, so a row that excuses
    /// nothing is a row that reads as a decision and is not one. Each case
    /// here is a pair that collides on geometry and is let through by exactly
    /// the row named — which is checked both ways: the row excuses its case,
    /// and no other row does.
    #[test]
    fn every_exemption_excuses_a_case_and_no_other() {
        // Geometry every case shares: a five-point overlap in both axes,
        // which is a collision on its own.
        let collided = |a: DrawnText, b: DrawnText| -> (DrawnText, DrawnText) {
            let overlap = a.visible.intersect(b.visible);
            assert!(
                overlap.width() > MIN_OVERLAP && overlap.height() > MIN_OVERLAP,
                "the case has to collide on geometry or it tests nothing: {overlap:?}"
            );
            (a, b)
        };

        let mut clipped_away = at(base(), 10.0, 10.0, 20.0, 9.0, "scrolled out");
        clipped_away.clip = egui::Rect::from_min_size(egui::pos2(500.0, 500.0), egui::vec2(9.0, 9.0));
        clipped_away.visible = clipped_away.ink.intersect(clipped_away.clip);

        let cases: Vec<(Rule, (DrawnText, DrawnText))> = vec![
            (
                Rule::DifferentLayer,
                collided(
                    at(base(), 10.0, 10.0, 20.0, 9.0, "under"),
                    at(above(), 25.0, 10.0, 20.0, 9.0, "over"),
                ),
            ),
            (
                Rule::NotVisible,
                collided(
                    at(base(), 10.0, 10.0, 20.0, 9.0, "in view"),
                    clipped_away.clone(),
                ),
            ),
            (
                Rule::Adjacent,
                (
                    at(base(), 10.0, 10.0, 20.0, 9.0, "flush"),
                    at(base(), 29.7, 10.0, 20.0, 9.0, "against"),
                ),
            ),
            (
                Rule::NamedPair,
                collided(
                    at(base(), 10.0, 10.0, 20.0, 9.0, "one"),
                    at(base(), 25.0, 10.0, 20.0, 9.0, "other"),
                ),
            ),
        ];

        for (rule, (a, b)) in &cases {
            let overlap = a.visible.intersect(b.visible);
            let excusing: Vec<Rule> = EXEMPTIONS
                .iter()
                .map(|e| e.rule)
                .filter(|r| r.excuses(a, b, overlap))
                .collect();
            if *rule == Rule::NamedPair {
                // EXEMPT_PAIRS is empty, so this case is a collision today.
                // What is checked is that nothing ELSE excuses it — the row is
                // reachable, and the moment somebody adds a pair it is the row
                // that lets it through.
                assert!(
                    excusing.is_empty(),
                    "{:?} and {:?} are excused by {excusing:?} and should be a \
                     defect while EXEMPT_PAIRS is empty",
                    a.text,
                    b.text
                );
                assert!(is_collision(a, b));
                let named = ExemptPair {
                    a: "one",
                    b: "other",
                    because: "the case this row is here for",
                };
                assert!(
                    Rule::NamedPair.excuses(
                        &DrawnText { text: named.a.to_owned(), ..a.clone() },
                        &DrawnText { text: named.b.to_owned(), ..b.clone() },
                        overlap,
                    ) == EXEMPT_PAIRS
                        .iter()
                        .any(|p| p.a == named.a && p.b == named.b),
                    "the row reads EXEMPT_PAIRS and nothing else"
                );
                continue;
            }
            assert_eq!(
                excusing,
                vec![*rule],
                "{:?} and {:?} should be excused by {rule:?} alone",
                a.text,
                b.text
            );
            assert!(!is_collision(a, b), "{rule:?} did not excuse its own case");
        }

        assert_eq!(
            EXEMPTIONS.len(),
            cases.len(),
            "every row of EXEMPTIONS has a case here, and every case a row"
        );
        for exemption in EXEMPTIONS {
            assert!(
                exemption.because.len() > 40,
                "{:?} is exempt for a reason nobody wrote down",
                exemption.rule
            );
            assert!(
                cases.iter().any(|(rule, _)| rule == &exemption.rule),
                "{:?} is in the table with no case driving it",
                exemption.rule
            );
        }
        for pair in EXEMPT_PAIRS {
            assert!(
                pair.because.len() > 40,
                "{:?} over {:?} is exempt for a reason nobody wrote down",
                pair.a,
                pair.b
            );
        }
    }

    /// **The ink box is the glyphs, not the line box.**
    ///
    /// The claim [`MIN_OVERLAP`] rests on. If this module read `Galley::rect`
    /// instead, two rows a row apart would overlap and the tolerance would
    /// have to grow past the size of a real collision.
    #[test]
    fn the_ink_box_is_tighter_than_the_line_box() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let galley = painter.layout_no_wrap(
            "median 1,425".to_owned(),
            egui::FontId::monospace(8.0),
            egui::Color32::WHITE,
        );
        assert!(
            galley.mesh_bounds.height() < galley.rect.height(),
            "the ink box {:?} is not tighter than the line box {:?}",
            galley.mesh_bounds,
            galley.rect
        );
        assert!(galley.mesh_bounds.height() > 0.0);
    }

    /// A galley of nothing but spaces has no mesh, and a box with no ink in it
    /// cannot be collided with.
    #[test]
    fn whitespace_puts_no_ink_down() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let galley = painter.layout_no_wrap(
            "   ".to_owned(),
            egui::FontId::monospace(8.0),
            egui::Color32::WHITE,
        );
        let shape = egui::epaint::TextShape::new(egui::pos2(5.0, 5.0), galley, egui::Color32::WHITE);
        assert!(
            ink_box(&shape).is_negative(),
            "a galley of spaces reported an ink box: {:?}",
            ink_box(&shape)
        );
        let mut out = Vec::new();
        texts_of(
            base(),
            &[ClippedShape {
                clip_rect: egui::Rect::EVERYTHING,
                shape: Shape::Text(shape),
            }],
            &mut out,
        );
        assert!(out.is_empty(), "whitespace reached the list: {out:?}");
    }

    /// A galley nested inside a `Shape::Vec` is read. Panels hand egui their
    /// content as one nested shape, so a walk that stopped at the top level
    /// would see almost none of the window.
    #[test]
    fn a_nested_galley_is_read() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let galley = painter.layout_no_wrap(
            "nested".to_owned(),
            egui::FontId::monospace(8.0),
            egui::Color32::WHITE,
        );
        let text = Shape::Text(egui::epaint::TextShape::new(
            egui::pos2(5.0, 5.0),
            galley,
            egui::Color32::WHITE,
        ));
        let mut out = Vec::new();
        texts_of(
            base(),
            &[ClippedShape {
                clip_rect: egui::Rect::EVERYTHING,
                shape: Shape::Vec(vec![Shape::Vec(vec![text])]),
            }],
            &mut out,
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].text, "nested");
    }

    /// **What is fitted is measured against the font, not counted in
    /// characters.**
    ///
    /// Both halves matter. A string that fits comes back whole, and a string
    /// that does not comes back elided AND narrower than the room it was
    /// given — a fitter that returned the ellipsis without shrinking would
    /// leave the collision it was called to prevent.
    #[test]
    fn fitting_measures_the_font_rather_than_counting_characters() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let font = egui::FontId::monospace(8.0);
        let whole = fit(&painter, "updated", font.clone(), 400.0, egui::Color32::WHITE);
        assert_eq!(whole.text(), "updated", "room to spare should not elide");

        let room = 40.0;
        let cut = fit(
            &painter,
            "TIMESTAMP WITH TIME ZONE",
            font.clone(),
            room,
            egui::Color32::WHITE,
        );
        assert!(cut.size().x <= room, "fitted to {} of {room}", cut.size().x);
        assert!(
            cut.text().ends_with('\u{2026}'),
            "an elided galley says so: {:?}",
            cut.text()
        );
        assert!(
            cut.text().len() < "TIMESTAMP WITH TIME ZONE".len(),
            "{:?}",
            cut.text()
        );

        // The character-budget answer this replaces: twenty characters of
        // this string is still wider than the room, which is how it came to
        // be drawn over the name beside it.
        let counted = painter.layout_no_wrap(
            "TIMESTAMP WITH TIME\u{2026}".to_owned(),
            font,
            egui::Color32::WHITE,
        );
        assert!(
            counted.size().x > room,
            "a twenty-character budget fitted {room} points, so this test is \
             not driving the case it was written for: {}",
            counted.size().x
        );
    }
}
