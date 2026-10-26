//! Ossie model scene — draws a [`brightfield_ossie::ModelLayout`] into a vello scene.
//!
//! Cards carry a role marker, the table's name, its row and field counts and, when
//! references are folded into it, a `+ name` line. Joins are cubic curves between
//! cards with a label pill on the curve: a likely join is solid, a possible one is
//! dashed in the Neutral role's ink and says so in its pill, and a join carrying a
//! warning wears an amber badge on its pill's corner. The design system has no token
//! named for "a proposal" or "dimmed", so the possible joins take the Neutral role's
//! ink, which no role marker or status colour shares (the Info blue sits too close to
//! the event marker's blue), and a dimmed element takes the disabled ink.
//!
//! With a selection, everything not connected to it is flattened to the dimmed ink:
//! a card becomes a plate of it with its words knocked out, an edge and its pill the
//! same. [`hit_test`] answers from the same geometry the scene draws.

use brightfield_ossie::{Card, EdgeLabel, Grade, Model, ModelLayout, Role};
use kurbo::{Affine, BezPath, Circle, CubicBez, ParamCurve, Point, Rect, Stroke};
use meridian_design::{semantic, viz};
use peniko::{Color, Fill};
use vello::Scene;

use crate::ink::ink;
use crate::text::{
    draw_heading, draw_text, measure_heading_width, measure_width, TextAnchor, LABEL_SIZE,
};

/// What the analyst has picked on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Dataset(String),
    Relationship(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelFocus {
    pub selected: Option<Selection>,
}

const NAME_SIZE: f32 = 13.0;
const MARKER: f64 = 8.0;
const PAD: f64 = 12.0;
const HEADER: f64 = 32.0;
const LINE: f64 = 20.0;
const PILL_HEIGHT: f64 = 20.0;
const PILL_PAD: f64 = 8.0;
const BADGE: f64 = 14.0;
const EDGE_WIDTH: f64 = 1.5;
const DASH: [f64; 2] = [6.0, 4.0];

struct Palette {
    canvas: Color,
    fill: Color,
    border: Color,
    selected: Color,
    text: Color,
    quiet: Color,
    proposal: Color,
    dimmed: Color,
    warning: Color,
    warning_glyph: Color,
    dark: bool,
}

impl Palette {
    fn new(dark: bool) -> Self {
        let sem = semantic(dark);
        Self {
            canvas: ink(sem.surfaces.app),
            fill: ink(sem.surfaces.raised),
            border: ink(sem.borders.default_),
            selected: ink(sem.borders.focus),
            text: ink(sem.text.primary),
            quiet: ink(sem.text.secondary),
            proposal: ink(sem.role(meridian_design::Role::Neutral).foreground.base),
            dimmed: ink(sem.text.disabled),
            warning: ink(sem.role(meridian_design::Role::Warning).background.base),
            warning_glyph: ink(sem.role(meridian_design::Role::Warning).foreground.base),
            dark,
        }
    }

    fn role(&self, role: Role) -> Color {
        let set = if self.dark {
            &viz::CATEGORICAL_DARK
        } else {
            &viz::CATEGORICAL_LIGHT
        };
        match role {
            Role::Event => ink(set[0]),
            Role::Entity => ink(set[1]),
            Role::Reference => ink(set[2]),
            Role::Bridge => ink(set[3]),
            Role::Snapshot => ink(set[4]),
            Role::Unclear => self.quiet,
        }
    }
}

/// Fit `text` into `width` pixels at `size`, ending in an ellipsis when it must be cut.
fn fit(text: &str, width: f64, heading: bool, size: f32) -> String {
    let measure = |s: &str| {
        if heading {
            measure_heading_width(s, size)
        } else {
            measure_width(s, size)
        }
    };
    if measure(text) <= width {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() && measure(&format!("{out}\u{2026}")) > width {
        out.pop();
    }
    out.push('\u{2026}');
    out
}

fn card_rect(c: &Card) -> Rect {
    Rect::new(c.x, c.y, c.x + c.width, c.y + c.height)
}

fn curve(e: &EdgeLabel) -> Option<CubicBez> {
    let p = |i: usize| e.points.get(i).map(|&(x, y)| Point::new(x, y));
    Some(CubicBez::new(p(0)?, p(1)?, p(2)?, p(3)?))
}

/// The pill's words: the join, and its grade when it is only a possible one.
fn pill_text(e: &EdgeLabel) -> String {
    match e.grade {
        Grade::Likely => e.text.clone(),
        Grade::Possible => format!("{}  \u{b7}  possible", e.text),
    }
}

fn has_warning(model: &Model, e: &EdgeLabel) -> bool {
    model
        .relationships
        .iter()
        .any(|r| r.name == e.relationship && r.meta.warning.is_some())
}

/// Where along its curve a pill may sit, best first: the middle, then alternately
/// either side of it.
const PILL_STOPS: [f64; 9] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.1, 0.9];

fn pill_at(e: &EdgeLabel, t: f64) -> Option<Rect> {
    let mid = curve(e)?.eval(t);
    let width = measure_width(&pill_text(e), LABEL_SIZE) + 2.0 * PILL_PAD;
    let (x, y) = (mid.x.round() - (width / 2.0).round(), mid.y.round());
    Some(Rect::new(
        x,
        y - PILL_HEIGHT / 2.0,
        x + width,
        y + PILL_HEIGHT / 2.0,
    ))
}

fn overlap(a: Rect, b: Rect) -> f64 {
    let i = a.intersect(b);
    i.width().max(0.0) * i.height().max(0.0)
}

/// Each join's label pill, in edge order. A pill sits on its curve, at the stop
/// that covers least of the pills already placed (counted heavily, since they hide
/// each other's words) and of the cards (words under a pill stay readable at its
/// edges only); the middle wins ties. The same rectangles answer [`hit_test`].
fn pill_rects(layout: &ModelLayout) -> Vec<Option<Rect>> {
    let mut placed: Vec<Rect> = Vec::new();
    layout
        .edges
        .iter()
        .map(|e| {
            let best = PILL_STOPS
                .iter()
                .filter_map(|&t| pill_at(e, t))
                .map(|r| {
                    let pills: f64 = placed
                        .iter()
                        .map(|&p| overlap(r, p.inflate(3.0, 3.0)))
                        .sum();
                    let cards: f64 = layout.cards.iter().map(|c| overlap(r, card_rect(c))).sum();
                    (r, pills * 4.0 + cards)
                })
                .fold(None::<(Rect, f64)>, |best, cand| match best {
                    Some(b) if b.1 <= cand.1 => Some(b),
                    _ => Some(cand),
                })
                .map(|(r, _)| r);
            placed.extend(best);
            best
        })
        .collect()
}

/// Dataset names a relationship joins, by relationship name.
fn ends<'a>(model: &'a Model, relationship: &str) -> Option<(&'a str, &'a str)> {
    model
        .relationships
        .iter()
        .find(|r| r.name == relationship)
        .map(|r| (r.from.as_str(), r.to.as_str()))
}

/// Whether a selection leaves a card lit.
fn card_lit(model: &Model, sel: Option<&Selection>, dataset: &str) -> bool {
    match sel {
        None => true,
        Some(Selection::Dataset(d)) => {
            d == dataset
                || model
                    .relationships
                    .iter()
                    .any(|r| (r.from == *d && r.to == dataset) || (r.to == *d && r.from == dataset))
        }
        Some(Selection::Relationship(r)) => {
            ends(model, r).is_some_and(|(a, b)| a == dataset || b == dataset)
        }
    }
}

fn edge_lit(model: &Model, sel: Option<&Selection>, e: &EdgeLabel) -> bool {
    match sel {
        None => true,
        Some(Selection::Relationship(r)) => *r == e.relationship,
        Some(Selection::Dataset(d)) => {
            ends(model, &e.relationship).is_some_and(|(a, b)| a == d || b == d)
        }
    }
}

fn is_selected(sel: Option<&Selection>, s: &Selection) -> bool {
    sel == Some(s)
}

/// Draw the model's cards, joins and join labels. The caller supplies the canvas
/// behind; this paints nothing across it.
pub fn render_model(
    scene: &mut Scene,
    model: &Model,
    layout: &ModelLayout,
    focus: &ModelFocus,
    dark: bool,
) {
    let pal = Palette::new(dark);
    let sel = focus.selected.as_ref();

    for e in &layout.edges {
        draw_edge(scene, model, e, sel, &pal);
    }
    for c in &layout.cards {
        draw_card(scene, model, c, sel, &pal);
    }
    for (e, rect) in layout.edges.iter().zip(pill_rects(layout)) {
        if let Some(rect) = rect {
            draw_pill(scene, model, e, rect, sel, &pal);
        }
    }
}

fn draw_edge(
    scene: &mut Scene,
    model: &Model,
    e: &EdgeLabel,
    sel: Option<&Selection>,
    pal: &Palette,
) {
    let Some(c) = curve(e) else { return };
    let lit = edge_lit(model, sel, e);
    let chosen = is_selected(sel, &Selection::Relationship(e.relationship.clone()));
    let colour = if !lit {
        pal.dimmed
    } else if chosen {
        pal.selected
    } else if e.grade == Grade::Possible {
        pal.proposal
    } else {
        pal.border
    };
    let mut stroke = Stroke::new(if chosen { EDGE_WIDTH * 1.5 } else { EDGE_WIDTH });
    if e.grade == Grade::Possible {
        stroke = stroke.with_dashes(0.0, DASH);
    }
    let mut path = BezPath::new();
    path.move_to(c.p0);
    path.curve_to(c.p1, c.p2, c.p3);
    scene.stroke(&stroke, Affine::IDENTITY, colour, None, &path);
}

fn draw_pill(
    scene: &mut Scene,
    model: &Model,
    e: &EdgeLabel,
    rect: Rect,
    sel: Option<&Selection>,
    pal: &Palette,
) {
    let warned = has_warning(model, e);
    let lit = edge_lit(model, sel, e);
    let chosen = is_selected(sel, &Selection::Relationship(e.relationship.clone()));
    let rim = if !lit {
        pal.dimmed
    } else if chosen {
        pal.selected
    } else if e.grade == Grade::Possible {
        pal.proposal
    } else {
        pal.border
    };
    let (fill, words) = if lit {
        (pal.fill, pal.text)
    } else {
        (pal.dimmed, pal.canvas)
    };
    scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &rect);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, rim, None, &rect);
    draw_text(
        scene,
        &pill_text(e),
        rect.x0 + PILL_PAD,
        rect.y0 + 14.0,
        LABEL_SIZE,
        words,
        TextAnchor::Start,
    );
    if warned {
        let (cx, cy) = (rect.x1, rect.y0);
        let colour = if lit { pal.warning } else { pal.canvas };
        let glyph = if lit { pal.warning_glyph } else { pal.dimmed };
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            colour,
            None,
            &Circle::new((cx, cy), BADGE / 2.0),
        );
        draw_text(
            scene,
            "!",
            cx,
            cy + 4.0,
            LABEL_SIZE,
            glyph,
            TextAnchor::Middle,
        );
    }
}

fn draw_card(scene: &mut Scene, model: &Model, c: &Card, sel: Option<&Selection>, pal: &Palette) {
    let Some(d) = model.datasets.iter().find(|d| d.name == c.dataset) else {
        return;
    };
    let rect = card_rect(c);
    let lit = card_lit(model, sel, &c.dataset);
    let chosen = is_selected(sel, &Selection::Dataset(c.dataset.clone()));
    let (fill, rim, name_ink, line_ink) = if lit {
        (
            pal.fill,
            if chosen { pal.selected } else { pal.border },
            pal.text,
            pal.quiet,
        )
    } else {
        (pal.dimmed, pal.dimmed, pal.canvas, pal.canvas)
    };
    scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &rect);
    scene.stroke(
        &Stroke::new(if chosen { 2.0 } else { 1.0 }),
        Affine::IDENTITY,
        rim,
        None,
        &rect,
    );

    let marker = if lit {
        pal.role(d.meta.role)
    } else {
        pal.canvas
    };
    let top = Rect::new(
        c.x + PAD,
        c.y + 12.0,
        c.x + PAD + MARKER,
        c.y + 12.0 + MARKER,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, marker, None, &top);
    let name_x = c.x + PAD + MARKER + 8.0;
    let name = fit(&d.name, c.x + c.width - PAD - name_x, true, NAME_SIZE);
    draw_heading(scene, &name, name_x, c.y + 21.0, NAME_SIZE, name_ink);

    let fields = d.fields.len();
    let counts = match d.meta.rows {
        Some(rows) => format!(
            "{rows} rows \u{b7} {fields} {}",
            if fields == 1 { "field" } else { "fields" }
        ),
        None => format!("{fields} {}", if fields == 1 { "field" } else { "fields" }),
    };
    let mut lines = vec![counts];
    let mut folded: Vec<&str> = model
        .datasets
        .iter()
        .filter(|f| !f.meta.excluded && f.meta.folded_into.as_deref() == Some(c.dataset.as_str()))
        .map(|f| f.name.as_str())
        .collect();
    folded.sort_unstable();
    if !folded.is_empty() {
        lines.push(format!("+ {}", folded.join(", ")));
    }
    for (i, line) in lines.iter().enumerate() {
        let text = fit(line, c.width - 2.0 * PAD, false, LABEL_SIZE);
        draw_text(
            scene,
            &text,
            c.x + PAD,
            c.y + HEADER + LINE * i as f64 + 14.0,
            LABEL_SIZE,
            line_ink,
            TextAnchor::Start,
        );
    }
}

/// What lies under the point `(x, y)` in layout coordinates: a join's label pill
/// first, since pills sit over cards, then a card.
#[must_use]
pub fn hit_test(layout: &ModelLayout, x: f64, y: f64) -> Option<Selection> {
    let at = Point::new(x, y);
    // Later pills are drawn over earlier ones, so the last to contain the point wins.
    layout
        .edges
        .iter()
        .zip(pill_rects(layout))
        .rev()
        .find(|(_, r)| r.is_some_and(|r| r.contains(at)))
        .map(|(e, _)| Selection::Relationship(e.relationship.clone()))
        .or_else(|| {
            layout
                .cards
                .iter()
                .find(|c| card_rect(c).contains(at))
                .map(|c| Selection::Dataset(c.dataset.clone()))
        })
}
