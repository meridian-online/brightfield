//! What text a frame put on the screen, and whether any two of it landed in
//! the same pixels.
//!
//! A pane that draws two labels into one place is the defect a reader sees
//! before any other, and until this module existed nothing in this repository
//! could fail because of it: each site that collided was found by looking at a
//! screenshot. Looking does not scale to the panes nobody thought to open, and
//! it does not cover the pane written next.
//!
//! [`frame_text`] reads a pass's own paint lists — each galley, the layer it
//! was painted into, the clip it was painted under — and [`collisions`] asks
//! whether any two of them share pixels. That it reaches every galley the pass
//! painted is what `the_check_reads_every_galley_the_pass_painted` measures,
//! against egui's own flattening of the same pass. A test drives the real shell for one
//! pass and asserts the list is empty, so the check runs over whatever that
//! shell happens to draw rather than over the sites somebody remembered.
//!
//! # What it reads, and what it therefore cannot see
//!
//! The ink box, not the line box. [`epaint::Galley::mesh_bounds`] is the
//! bounding box of the glyph meshes; `Galley::rect` is the font's line box,
//! which is as tall as the face whatever the string sets. Vertically the ink
//! box is the tighter of the two — `the_ink_box_is_the_glyph_quads_not_the_line_box`
//! measures both — which is what lets two rows a row apart be read as two
//! rows rather than as a collision.
//!
//! **Horizontally it is the looser of the two**, and that is the surprise
//! worth carrying: epaint snaps each glyph quad out to the pixel grid, so a
//! mesh box can stand a fraction of a point wider than the line box that
//! produced it. Two labels set exactly flush therefore share a sliver of ink
//! box, and [`MIN_OVERLAP`] is sized from that measurement rather than from
//! zero.
//!
//! Text the egui pass painted. The Vello canvas draws through an
//! [`epaint::Shape::Callback`], so a mark's own labels are not in these lists
//! and this module does not see them. What is outside the canvas rect — the
//! rails, the grid, the header band, the inspector, the sheet, the top bar —
//! is.

use egui::epaint::{ClippedShape, Shape};

/// The overlap, in logical points on both axes, at or under which two ink
/// boxes are called adjacent rather than collided.
///
/// Not zero, and the reason is measured rather than assumed: epaint rounds
/// each glyph quad out to the pixel grid, so a galley's mesh box can stand up
/// to a point wider than its own line box at one point per pixel, and two
/// labels set flush against one another share that rounding at the seam.
/// `flush_labels_share_less_than_the_tolerance` lays a flush pair out through
/// the real font and measures what they share; `a_one_character_overlap_is_over_the_tolerance`
/// lays out the smallest overlap a reader would call one and measures that it
/// clears this. The value has to sit between those two numbers, and both
/// tests print theirs when they fail.
pub const MIN_OVERLAP: f32 = 2.0;

// ---------------------------------------------------------------------------
// What the frame drew.
// ---------------------------------------------------------------------------

/// One galley a pass painted, with everything the rule needs to judge it.
#[derive(Clone, Debug)]
pub struct DrawnText {
    /// The string the reader sees: the glyphs the layout placed, in order.
    ///
    /// **Not `Galley::text()`**, which returns the job's source string — a
    /// galley elided to `TIMESTA…` still answers `TIMESTAMP WITH TIME ZONE`
    /// there, so a report built from it would name text nobody drew and an
    /// [`ExemptPair`] written against it would excuse a string that is not on
    /// the screen. This is read off [`epaint::text::PlacedRow`], which is the
    /// glyphs.
    pub text: String,
    /// Whether the layout dropped part of the string to fit the room it was
    /// given. [`Self::text`] is the part that survived.
    pub elided: bool,
    /// The layer it was painted into. Text in a tooltip, a popup or a modal
    /// is in a different layer from the surface under it, which is what a
    /// layer is for.
    pub layer: egui::LayerId,
    /// The tight box around its glyph meshes, in window-space logical points.
    pub ink: egui::Rect,
    /// The clip it was painted under.
    pub clip: egui::Rect,
    /// The part of [`Self::ink`] that reaches the screen: the ink box under
    /// the clip. [`egui::Rect::is_negative`] where the clip left none of it,
    /// which `every_exemption_excuses_a_case_and_no_other` drives as its
    /// [`Rule::NotVisible`] case.
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
/// walks that table and reaches an exemption no other way, so a new reason
/// takes effect when somebody adds a row to it with a sentence saying why,
/// rather than by a condition growing quietly inside a predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The two are in different layers.
    DifferentLayer,
    /// At least one of them is clipped away entirely.
    NotVisible,
    /// Both reach the screen and their ink boxes overlap, but the parts that
    /// reach it do not: a clip stands between them.
    ClippedApart,
    /// Their ink boxes share no more than [`MIN_OVERLAP`] on an axis, so they
    /// did not overlap to begin with.
    Adjacent,
    /// The pair is named in [`EXEMPT_PAIRS`].
    NamedPair,
}

/// How much of a box two galleys share, on the two readings the rules need.
///
/// Split out so each row of [`EXEMPTIONS`] is written against the geometry it
/// is actually about — the ink boxes for whether they were ever in the same
/// place, the visible boxes for whether the reader sees them there.
#[derive(Clone, Copy, Debug)]
struct Shared {
    /// What the two ink boxes share, clip ignored.
    ink: egui::Rect,
    /// What the two boxes share after each is clipped.
    visible: egui::Rect,
}

impl Shared {
    fn of(a: &DrawnText, b: &DrawnText) -> Self {
        Self {
            ink: a.ink.intersect(b.ink),
            visible: a.visible.intersect(b.visible),
        }
    }
}

/// Whether `overlap` is more than a rounding seam on both axes.
fn is_shared(overlap: egui::Rect) -> bool {
    !overlap.is_negative() && overlap.width() > MIN_OVERLAP && overlap.height() > MIN_OVERLAP
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
        rule: Rule::ClippedApart,
        because: "a pane that clips its own content can hold two overlapping \
                  galleys apart on the screen. A table cell clips its column's \
                  text to the cell, so a name too long for its cell lays ink \
                  into the next one and is cut off at the edge before it \
                  arrives — the ink boxes overlap and the reader sees two \
                  separate labels, which is what the clip is for.",
    },
    Exemption {
        rule: Rule::Adjacent,
        because: "epaint rounds every glyph quad out to the pixel grid, so a \
                  mesh box stands a fraction of a point wider than the line \
                  box it came from and two labels set flush share that \
                  rounding at the seam. MIN_OVERLAP is measured off a flush \
                  pair rather than guessed, so this row excuses the rounding \
                  and not a character of it more.",
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
    /// Whether this rule excuses `a` and `b` sharing `shared`, with `pairs`
    /// standing in for [`EXEMPT_PAIRS`].
    ///
    /// **The rows are disjoint on purpose.** Each geometric row states the
    /// visibility it applies at as well as the geometry, so exactly one of
    /// them speaks to any given pair and the reason a pair was let through is
    /// a single row rather than whichever one happened to be walked first.
    /// `every_exemption_excuses_a_case_and_no_other` is what holds that.
    fn excuses(self, a: &DrawnText, b: &DrawnText, shared: Shared, pairs: &[ExemptPair]) -> bool {
        let both_visible = a.is_visible() && b.is_visible();
        match self {
            Self::DifferentLayer => a.layer != b.layer,
            Self::NotVisible => !both_visible,
            Self::ClippedApart => {
                both_visible && is_shared(shared.ink) && !is_shared(shared.visible)
            }
            Self::Adjacent => both_visible && !is_shared(shared.ink),
            Self::NamedPair => pairs.iter().any(|pair| {
                (pair.a == a.text && pair.b == b.text) || (pair.a == b.text && pair.b == a.text)
            }),
        }
    }
}

/// Whether these two galleys sharing a box is a defect.
///
/// The route to an exemption is [`EXEMPTIONS`] and no other: a [`Rule`]
/// variant nobody has written a row for excuses no pair, which
/// `every_exemption_excuses_a_case_and_no_other` holds by driving one case per
/// row and reading back which rows spoke.
#[must_use]
pub fn is_collision(a: &DrawnText, b: &DrawnText) -> bool {
    let shared = Shared::of(a, b);
    !EXEMPTIONS
        .iter()
        .any(|exemption| exemption.rule.excuses(a, b, shared, EXEMPT_PAIRS))
}

/// Which row of [`EXEMPTIONS`] let this pair through, if one did.
///
/// The rows are disjoint, so there is at most one — and a report that says
/// *why* a pair was allowed is how a silenced collision stays auditable.
#[must_use]
pub fn excused_by(a: &DrawnText, b: &DrawnText) -> Option<Rule> {
    let shared = Shared::of(a, b);
    EXEMPTIONS
        .iter()
        .map(|exemption| exemption.rule)
        .find(|rule| rule.excuses(a, b, shared, EXEMPT_PAIRS))
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
    for corner in [
        local.left_top(),
        local.right_top(),
        local.left_bottom(),
        local.right_bottom(),
    ] {
        turned.extend_with(egui::pos2(
            cos.mul_add(corner.x, -(sin * corner.y)) + at.x,
            sin.mul_add(corner.x, cos * corner.y) + at.y,
        ));
    }
    turned
}

/// The glyphs a layout placed, in order — what the reader sees.
///
/// `Galley::text()` would be the string somebody asked for, which is a
/// different string whenever the layout elided one; see [`DrawnText::text`].
fn drawn_string(galley: &egui::Galley) -> String {
    let mut out = String::new();
    for row in &galley.rows {
        out.push_str(&row.text());
    }
    out
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
                    text: drawn_string(&text.galley),
                    elided: text.galley.elided,
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

/// Every galley in an already-flattened shape list, read as one layer.
///
/// **Not the check's route in, and a caller reaching for it should read
/// [`frame_text`] instead.** egui flattens its paint lists when the pass ends
/// and the layer is not in the flattened list, so what this returns claims the
/// background layer whatever it was drawn into — which is the confusion
/// [`frame_text`] exists to avoid.
///
/// It is here so that a test can ask the one question [`frame_text`] cannot
/// answer about itself, and
/// `the_check_reads_every_galley_the_pass_painted` is that test: whether
/// walking the layers found every galley the pass painted. `frame_text`
/// enumerates layers through `Memory::layer_ids`, and a layer missing from
/// that list would be a pane this check silently did not look at. Comparing
/// the two counts is what turns that from a hope into a measurement.
#[must_use]
pub fn flattened_text(shapes: &[ClippedShape]) -> Vec<DrawnText> {
    let mut out = Vec::new();
    texts_of(egui::LayerId::background(), shapes, &mut out);
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
/// `None` where no pair collided — which is the state
/// `no_two_texts_are_drawn_into_one_place` asserts of the whole window.
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

/// Where [`row_ends`] put the two labels it drew.
#[derive(Clone, Copy, Debug)]
pub struct RowEnds {
    /// The label at the leading edge, or [`egui::Rect::NOTHING`] where the row
    /// was too narrow for one.
    ///
    /// Negative rather than absent so that a caller recording *where the row
    /// drew its two ends* records the same shape either way. A row this narrow
    /// is under two ellipses and a gap — about sixteen points — which is
    /// narrower than any floor the shell imposes;
    /// `a_row_too_narrow_for_both_ends_drops_the_leading_one` is what drives
    /// it, and what says that dropping it beats stacking two ellipses.
    pub leading: egui::Rect,
    /// The label at the trailing edge. Always drawn: it is the one the row
    /// gives its width to first.
    pub trailing: egui::Rect,
}

/// **Draw two labels at the two ends of one row, so that they cannot touch.**
///
/// The one shape four sites in this shell were each writing by hand, three of
/// them wrong. A row with a label at each end and no measurement between them
/// collides the moment the row is narrower than the two strings — which is not
/// an edge case but the ordinary state of a column band at its width floor and
/// a rail at its own. Every one of those three drew both labels at fixed
/// anchors and hoped: `updated` and `TIMESTAMP WITH TIME ZONE` arriving as
/// `updateTIMESTAMP WITH TIME…` is what hoping looks like.
///
/// The trailing label is laid out first and the leading one is fitted to what
/// is left, because the trailing end is the one whose width the row cannot
/// predict — a type name, a bound, a deviation. Both are fitted, so the
/// trailing one cannot take the whole row either, and `gap` of clear space is
/// kept between them.
///
/// Returns where each landed, so a test can read the two rects rather than a
/// screenshot.
pub fn row_ends(
    painter: &egui::Painter,
    row: egui::Rect,
    leading: &str,
    trailing: &str,
    font: &egui::FontId,
    gap: f32,
    leading_ink: egui::Color32,
    trailing_ink: egui::Color32,
) -> RowEnds {
    // The narrowest thing this can draw, and therefore the width that has to
    // be kept back from the trailing label for the leading one.
    let ellipsis = painter
        .layout_no_wrap("\u{2026}".to_owned(), font.clone(), trailing_ink)
        .size()
        .x;

    let trailing_galley = fit(
        painter,
        trailing,
        font.clone(),
        row.width() - ellipsis - gap,
        trailing_ink,
    );
    let trailing_rect = egui::Rect::from_min_size(
        egui::pos2(
            row.right() - trailing_galley.size().x,
            row.center().y - trailing_galley.size().y / 2.0,
        ),
        trailing_galley.size(),
    );

    let room = trailing_rect.left() - gap - row.left();
    let leading_rect = if room < ellipsis {
        egui::Rect::NOTHING
    } else {
        let galley = fit(painter, leading, font.clone(), room, leading_ink);
        let at = egui::Rect::from_min_size(
            egui::pos2(row.left(), row.center().y - galley.size().y / 2.0),
            galley.size(),
        );
        painter.galley(at.min, galley, leading_ink);
        at
    };
    painter.galley(trailing_rect.min, trailing_galley, trailing_ink);

    RowEnds {
        leading: leading_rect,
        trailing: trailing_rect,
    }
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
            elided: false,
            layer,
            ink,
            clip,
            visible: ink.intersect(clip),
        }
    }

    /// The same, under a clip that takes all of it.
    fn clipped_away(layer: egui::LayerId, x: f32, y: f32, w: f32, h: f32, text: &str) -> DrawnText {
        let mut drawn = at(layer, x, y, w, h, text);
        drawn.clip = egui::Rect::from_min_size(egui::pos2(900.0, 900.0), egui::vec2(9.0, 9.0));
        drawn.visible = drawn.ink.intersect(drawn.clip);
        drawn
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
        let kind = at(
            base(),
            48.0,
            100.0,
            60.0,
            9.0,
            "TIMESTAMP WITH TIME\u{2026}",
        );
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
    /// nothing another row already excuses is a row that reads as a decision
    /// and is not one. That is not hypothetical: the first draft of this
    /// module measured its rules against the *clipped* boxes, which made
    /// [`Rule::NotVisible`] unreachable — a galley the clip took shares an
    /// empty box with whatever it is compared to, so [`Rule::Adjacent`]
    /// excused it first and
    /// `NotVisible` decided no pair at all. Each row now names the visibility
    /// it applies at as well as the geometry, and this test holds them apart:
    /// for each case, the rows that excuse it are exactly the one named.
    #[test]
    fn every_exemption_excuses_a_case_and_no_other() {
        // The pair `Rule::NamedPair`'s case is written against, passed in
        // rather than read off `EXEMPT_PAIRS` — which is empty, and the row
        // has to be shown doing its job without a live exemption being added
        // to the shell so that a test can pass.
        let pairs = [ExemptPair {
            a: "drawn over",
            b: "on purpose",
            because: "the case `every_exemption_excuses_a_case_and_no_other` \
                      drives this row with, and the only pair written anywhere: \
                      EXEMPT_PAIRS itself is empty.",
        }];

        // Both visible, ink and visible boxes shared by forty points: a
        // collision but for the one thing each case changes.
        let collides = |layer, a_text: &str, b_text: &str| {
            (
                at(base(), 10.0, 10.0, 50.0, 9.0, a_text),
                at(layer, 20.0, 10.0, 50.0, 9.0, b_text),
            )
        };

        // Ink boxes overlapping by forty points, held apart by two clips: the
        // reader sees one label in each pane and no overlap between them.
        let mut left = at(
            base(),
            10.0,
            10.0,
            50.0,
            9.0,
            "a long name in a nar\u{2026}",
        );
        left.clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(30.0, 100.0));
        left.visible = left.ink.intersect(left.clip);
        let mut right = at(base(), 20.0, 10.0, 50.0, 9.0, "the next cell");
        right.clip = egui::Rect::from_min_size(egui::pos2(40.0, 0.0), egui::vec2(60.0, 100.0));
        right.visible = right.ink.intersect(right.clip);

        let cases: Vec<(Rule, (DrawnText, DrawnText))> = vec![
            (Rule::DifferentLayer, collides(above(), "under", "over")),
            (
                Rule::NotVisible,
                (
                    at(base(), 10.0, 10.0, 50.0, 9.0, "in view"),
                    clipped_away(base(), 20.0, 10.0, 50.0, 9.0, "scrolled out"),
                ),
            ),
            (Rule::ClippedApart, (left, right)),
            (
                Rule::Adjacent,
                (
                    at(base(), 10.0, 10.0, 20.0, 9.0, "flush"),
                    // Sharing half a point: the pixel-grid rounding at the
                    // seam, which is under MIN_OVERLAP by construction.
                    at(base(), 29.5, 10.0, 20.0, 9.0, "against"),
                ),
            ),
            (
                Rule::NamedPair,
                collides(base(), "drawn over", "on purpose"),
            ),
        ];

        for (rule, (a, b)) in &cases {
            let shared = Shared::of(a, b);
            let excusing: Vec<Rule> = EXEMPTIONS
                .iter()
                .map(|e| e.rule)
                .filter(|r| r.excuses(a, b, shared, &pairs))
                .collect();
            assert_eq!(
                excusing,
                vec![*rule],
                "{:?} and {:?} should be excused by {rule:?} and by nothing \
                 else; they share {:?} of ink box and {:?} of visible box",
                a.text,
                b.text,
                shared.ink,
                shared.visible,
            );
        }

        // …and the same cases through the entry point the shell uses, which
        // reads the real `EXEMPT_PAIRS`. Every case but the named pair is
        // excused, and the named pair is a defect — because that list is
        // empty and nothing in the shell has yet earned a row in it.
        for (rule, (a, b)) in &cases {
            if *rule == Rule::NamedPair {
                assert!(
                    is_collision(a, b),
                    "EXEMPT_PAIRS is empty, so {:?} over {:?} is a defect until \
                     somebody writes the row",
                    a.text,
                    b.text,
                );
                assert_eq!(excused_by(a, b), None);
            } else {
                assert!(!is_collision(a, b), "{rule:?} did not excuse its own case");
                assert_eq!(excused_by(a, b), Some(*rule));
            }
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

    /// **The ink box is the glyph quads: tighter than the line box down the
    /// page, and looser across it.**
    ///
    /// Both halves are load-bearing and only the first was expected. The line
    /// box is as tall as the face whatever the string sets, so reading it
    /// would make two rows a row apart overlap and the tolerance would have to
    /// grow past the size of a real collision — that is why this module reads
    /// `mesh_bounds`.
    ///
    /// The second half is why [`MIN_OVERLAP`] is not near zero. epaint rounds
    /// each glyph quad out to the pixel grid, so a mesh box can stand *wider*
    /// than the line box that produced it, and two labels set flush share that
    /// rounding.
    #[test]
    fn the_ink_box_is_the_glyph_quads_not_the_line_box() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let font = egui::FontId::monospace(8.0);

        // Down the page: a string of caps and no descender does not reach the
        // bottom of its own line box.
        let caps =
            painter.layout_no_wrap("TIMESTAMP".to_owned(), font.clone(), egui::Color32::WHITE);
        assert!(
            caps.mesh_bounds.height() < caps.rect.height(),
            "the ink box {:?} is not shorter than the line box {:?}",
            caps.mesh_bounds,
            caps.rect
        );
        assert!(caps.mesh_bounds.height() > 0.0);

        // Across it: at least one of these stands wider than its line box.
        // Stated as *some string does* rather than as a figure, because the
        // amount is the pixel grid's and moves with `pixels_per_point`.
        let widened: Vec<(&str, f32)> = ["xxx", "median 1,425", "Ay"]
            .into_iter()
            .map(|text| {
                let galley =
                    painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE);
                (text, galley.mesh_bounds.width() - galley.rect.width())
            })
            .collect();
        assert!(
            widened.iter().any(|(_, grown)| *grown > 0.0),
            "no ink box stood wider than its line box, so MIN_OVERLAP is \
             carrying a tolerance for a rounding that no longer happens and \
             should come down: {widened:?}"
        );
    }

    /// **Two labels set flush share less than the tolerance.**
    ///
    /// One of the two measurements [`MIN_OVERLAP`] sits between. Laid out
    /// through a real font and placed edge to edge — the arrangement a row
    /// with a label at each end reaches when it *just* fits — so what is
    /// measured is the pixel-grid rounding rather than a number somebody
    /// chose.
    #[test]
    fn flush_labels_share_less_than_the_tolerance() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let font = egui::FontId::monospace(8.0);
        let mut worst = 0.0_f32;
        for (left, right) in [
            ("updated", "TIMESTAMP"),
            ("xxx", "median 1,425"),
            ("Ay", "gjpqy"),
        ] {
            let a = painter.layout_no_wrap(left.to_owned(), font.clone(), egui::Color32::WHITE);
            let b = painter.layout_no_wrap(right.to_owned(), font.clone(), egui::Color32::WHITE);
            // b starts exactly where a's line box ends.
            let a_ink = a.mesh_bounds.translate(egui::vec2(0.0, 0.0));
            let b_ink = b.mesh_bounds.translate(egui::vec2(a.rect.width(), 0.0));
            let shared = a_ink.intersect(b_ink);
            let width = if shared.is_negative() {
                0.0
            } else {
                shared.width()
            };
            worst = worst.max(width);
        }
        assert!(
            worst <= MIN_OVERLAP,
            "flush labels share {worst} points of ink box, which is over \
             MIN_OVERLAP ({MIN_OVERLAP}) — every row with a label at each end \
             will report a collision it does not have"
        );
    }

    /// **One character of overlap is over the tolerance.**
    ///
    /// The other measurement. A tolerance is only honest if it sits under the
    /// smallest overlap a reader would call one, and the smallest this module
    /// is asked to catch is one character of a caption face landing on
    /// another.
    #[test]
    fn a_one_character_overlap_is_over_the_tolerance() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let font = egui::FontId::monospace(8.0);
        let a = painter.layout_no_wrap("updated".to_owned(), font.clone(), egui::Color32::WHITE);
        let b = painter.layout_no_wrap("TIMESTAMP".to_owned(), font, egui::Color32::WHITE);
        let one_character = a.rect.width() / 7.0;
        let a_ink = a.mesh_bounds;
        let b_ink = b
            .mesh_bounds
            .translate(egui::vec2(a.rect.width() - one_character, 0.0));
        let shared = a_ink.intersect(b_ink);
        assert!(
            !shared.is_negative() && shared.width() > MIN_OVERLAP,
            "one character of overlap is {} points, which MIN_OVERLAP \
             ({MIN_OVERLAP}) would excuse — the tolerance is wider than the \
             defect it is meant to let through the net",
            shared.width()
        );
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
        let shape =
            egui::epaint::TextShape::new(egui::pos2(5.0, 5.0), galley, egui::Color32::WHITE);
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

    /// The row a two-ended draw is given, and what it drew there.
    fn ends(painter: &egui::Painter, width: f32, leading: &str, trailing: &str) -> RowEnds {
        row_ends(
            painter,
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(width, 13.0)),
            leading,
            trailing,
            &egui::FontId::monospace(8.0),
            6.0,
            egui::Color32::WHITE,
            egui::Color32::WHITE,
        )
    }

    /// **A row with a label at each end draws both of them, apart, at every
    /// width the shell reaches.**
    ///
    /// The whole point of the helper, driven across the widths a column band
    /// and a rail actually reach and then some. The pair is the one that
    /// collided in the shell: a column name and the longest type name DuckDB
    /// hands this rail.
    ///
    /// **Both halves, and the second one is why this test is written this
    /// way.** An earlier version skipped a width where the leading label was
    /// absent, on the grounds that an absent label cannot collide — which is
    /// true and useless: deleting the room `row_ends` keeps back for the
    /// leading end left this test green, because the trailing label then took
    /// the whole row and the leading one stopped being drawn. A row that
    /// silently drops a value is not a row that passed.
    #[test]
    fn a_two_ended_row_keeps_its_two_ends_apart() {
        let ctx = ctx();
        let painter = painter(&ctx);
        for width in [20.0_f32, 40.0, 80.0, 96.0, 140.0, 240.0, 400.0] {
            let drawn = ends(&painter, width, "updated", "TIMESTAMP WITH TIME ZONE");
            assert!(
                !drawn.trailing.is_negative(),
                "at {width} points the trailing end was not drawn at all"
            );
            assert!(
                drawn.trailing.right() <= 10.0 + width + 0.01,
                "at {width} points the trailing end runs past the row: {:?}",
                drawn.trailing
            );
            assert!(
                !drawn.leading.is_negative(),
                "at {width} points nothing was drawn at the leading end, so \
                 the row states one of its two values and drops the other"
            );
            assert!(
                drawn.leading.right() <= drawn.trailing.left(),
                "at {width} points the leading end ends at {} and the trailing \
                 one begins at {}",
                drawn.leading.right(),
                drawn.trailing.left(),
            );
            assert!(
                drawn.leading.left() >= 10.0 - 0.01,
                "at {width} points the leading end starts left of the row: {:?}",
                drawn.leading
            );
        }
    }

    /// **A row wide enough for both draws both, whole.**
    ///
    /// The other side of the previous test: a helper that always elided would
    /// pass that one and be useless.
    #[test]
    fn a_wide_row_draws_both_ends_whole() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let drawn = ends(&painter, 400.0, "updated", "TIMESTAMP WITH TIME ZONE");
        let whole = painter
            .layout_no_wrap(
                "updated".to_owned(),
                egui::FontId::monospace(8.0),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        assert!(
            (drawn.leading.width() - whole).abs() < 0.01,
            "the leading end was elided in a row with room to spare: {} \
             against {whole}",
            drawn.leading.width(),
        );
    }

    /// **A row too narrow for both ends drops the leading one rather than
    /// stacking two ellipses.**
    ///
    /// The case [`RowEnds::leading`] is negative for. Under about sixteen
    /// points there is not room for an ellipsis at each end and a gap between
    /// them, and two ellipses in one place is the defect this module exists to
    /// stop — so the row states the end whose width it could not predict and
    /// says nothing where the other one would have gone.
    #[test]
    fn a_row_too_narrow_for_both_ends_drops_the_leading_one() {
        let ctx = ctx();
        let painter = painter(&ctx);
        let drawn = ends(&painter, 8.0, "updated", "TIMESTAMP WITH TIME ZONE");
        assert!(
            drawn.leading.is_negative(),
            "an eight-point row drew a leading label at {:?}, which cannot be \
             clear of the trailing one at {:?}",
            drawn.leading,
            drawn.trailing,
        );
        assert!(!drawn.trailing.is_negative());
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
        let whole = fit(
            &painter,
            "updated",
            font.clone(),
            400.0,
            egui::Color32::WHITE,
        );
        assert_eq!(
            drawn_string(&whole),
            "updated",
            "room to spare should not elide"
        );
        assert!(!whole.elided);

        let room = 40.0;
        let cut = fit(
            &painter,
            "TIMESTAMP WITH TIME ZONE",
            font.clone(),
            room,
            egui::Color32::WHITE,
        );
        assert!(cut.size().x <= room, "fitted to {} of {room}", cut.size().x);
        // Read off the glyphs. `Galley::text()` answers the string that was
        // asked for — this galley still says "TIMESTAMP WITH TIME ZONE"
        // there — so a check written against it would pass over a fitter that
        // did not elide.
        let drawn = drawn_string(&cut);
        assert!(cut.elided, "the galley did not elide: {drawn:?}");
        assert!(
            drawn.ends_with('\u{2026}'),
            "an elided galley carries its ellipsis: {drawn:?}"
        );
        assert!(drawn.len() < "TIMESTAMP WITH TIME ZONE".len(), "{drawn:?}");

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
