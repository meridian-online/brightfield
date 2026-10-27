//! Role-banded layout of a model: events on the left, entities on the right,
//! snapshots, bridges, references and unclear tables in a row below.
//!
//! No text measurement and no UI framework: card sizes are fixed or derived from
//! line counts, coordinates are whole pixels, and every tie breaks by name, so the
//! same model always yields the same layout whatever order its datasets came in.

use std::collections::{BTreeMap, BTreeSet};

use meridian_design::spacing;

use crate::model::{Dataset, DecidedBy, Grade, Model, Relationship, Role};

/// Which region of the canvas a card sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Events,
    Entities,
    Below,
}

/// A placed dataset, whole pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub dataset: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub band: Band,
}

/// A drawn join: its text and a chain of cubic Beziers, each `[control1, control2, end]`
/// after the first point (four points, or seven when it turns in the lane below the bands).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeLabel {
    pub relationship: String,
    pub text: String,
    pub grade: Grade,
    /// Whether the join is a proposal still waiting on the analyst ([`is_proposal`]), drawn as one.
    pub proposal: bool,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelLayout {
    pub width: f64,
    pub height: f64,
    pub cards: Vec<Card>,
    pub edges: Vec<EdgeLabel>,
}

/// One row of the list view.
#[derive(Debug, Clone, PartialEq)]
pub enum ListRow {
    /// A possible join between placed tables, waiting on the analyst.
    Review {
        relationship: String,
        edge_text: String,
    },
    /// A placed table. Its folded tables follow it as [`ListRow::Folded`] rows.
    Table {
        role: Role,
        name: String,
        rows: Option<u64>,
        key: Vec<String>,
        fields: usize,
        /// Datasets folded into this one.
        folded: Vec<String>,
        /// Names of the joins this table takes part in.
        joins: Vec<String>,
    },
    /// A table folded into another, listed under its host's row.
    Folded { name: String, into: String },
    /// A table left out of the model, in a group of its own at the end of the list.
    LeftOut { name: String, role: Role },
}

/// Whether `r` is a proposal waiting on the analyst: a possible join that discovery decided. A
/// possible join the analyst kept stays graded possible, as evidence, but is no longer one.
#[must_use]
pub fn is_proposal(r: &Relationship) -> bool {
    r.meta.grade == Grade::Possible && r.meta.decided_by == DecidedBy::Dovetail
}

const MARGIN: f64 = spacing::SPACE_8 as f64;
const CARD_WIDTH: f64 = 200.0;
const HEADER_HEIGHT: f64 = 32.0;
const LINE_HEIGHT: f64 = 20.0;
const CARD_PAD: f64 = spacing::SPACE_5 as f64;
const ROW_GAP: f64 = spacing::SPACE_6 as f64;
/// Space between the two upper bands: wider than the widest join label drawn on it
/// (a possible join's pill, about 240 pixels), so a label never covers a card.
/// Layout cannot measure text, so this is an estimate with room to spare.
const BAND_GAP: f64 = 280.0;
/// Inset of the first and last edge port from a card's side.
const PORT_INSET: f64 = 8.0;
/// How far an edge to the row below runs sideways before turning down.
const TURN: f64 = 60.0;
/// How far an edge to the row below keeps heading down as it leaves and enters the lane.
const LANE_DIP: f64 = 12.0;
/// Clearance of the first and last lane from the upper cards and from the row.
const LANE_INSET: f64 = 8.0;
/// Space between the upper bands and the row below.
const ROW_BAND_GAP: f64 = spacing::SPACE_9 as f64;
/// How far a same-band edge swings out beyond its cards.
const SWING: f64 = spacing::SPACE_9 as f64;

fn placed(d: &Dataset) -> bool {
    !d.meta.excluded && d.meta.folded_into.is_none()
}

fn band_of(role: Role) -> Band {
    match role {
        Role::Event => Band::Events,
        Role::Entity => Band::Entities,
        Role::Snapshot | Role::Bridge | Role::Reference | Role::Unclear => Band::Below,
    }
}

fn folded_into<'a>(model: &'a Model, host: &str) -> Vec<&'a str> {
    let mut v: Vec<&str> = model
        .datasets
        .iter()
        .filter(|d| !d.meta.excluded && d.meta.folded_into.as_deref() == Some(host))
        .map(|d| d.name.as_str())
        .collect();
    v.sort_unstable();
    v
}

/// Joins whose two ends are both placed and distinct.
fn drawn_joins<'a>(model: &'a Model, names: &BTreeSet<&str>) -> Vec<&'a Relationship> {
    let mut v: Vec<&Relationship> = model
        .relationships
        .iter()
        .filter(|r| {
            r.from != r.to && names.contains(r.from.as_str()) && names.contains(r.to.as_str())
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn placed_names(model: &Model) -> BTreeSet<&str> {
    model
        .datasets
        .iter()
        .filter(|d| placed(d))
        .map(|d| d.name.as_str())
        .collect()
}

/// `customer_id = customer_id  N:1`; composite keys join their columns with `, `.
pub fn edge_text(r: &Relationship) -> String {
    format!(
        "{} = {}  {}",
        r.from_columns.join(", "),
        r.to_columns.join(", "),
        r.meta.cardinality
    )
}

/// `7 tables · 6 joins · 1 possible · 1 unclear`, counted among placed tables.
pub fn summary(model: &Model) -> String {
    let names = placed_names(model);
    let joins = drawn_joins(model, &names);
    // Counted as the review queue counts them, so a kept possible join leaves the count.
    let possible = joins.iter().filter(|r| is_proposal(r)).count();
    let unclear = model
        .datasets
        .iter()
        .filter(|d| placed(d) && d.meta.role == Role::Unclear)
        .count();
    format!(
        "{} {} · {} {} · {possible} possible · {unclear} unclear",
        names.len(),
        if names.len() == 1 { "table" } else { "tables" },
        joins.len(),
        if joins.len() == 1 { "join" } else { "joins" },
    )
}

const ROLE_ORDER: [Role; 6] = [
    Role::Entity,
    Role::Event,
    Role::Snapshot,
    Role::Bridge,
    Role::Reference,
    Role::Unclear,
];

/// Review rows for the proposals ([`is_proposal`]) first, then tables grouped by role, by name
/// within a role, each followed by the tables folded into it, then the tables left out.
pub fn list_rows(model: &Model) -> Vec<ListRow> {
    let names = placed_names(model);
    let joins = drawn_joins(model, &names);
    let mut rows: Vec<ListRow> = joins
        .iter()
        .filter(|r| is_proposal(r))
        .map(|r| ListRow::Review {
            relationship: r.name.clone(),
            edge_text: edge_text(r),
        })
        .collect();
    for role in ROLE_ORDER {
        let mut group: Vec<&Dataset> = model
            .datasets
            .iter()
            .filter(|d| placed(d) && d.meta.role == role)
            .collect();
        group.sort_by(|a, b| a.name.cmp(&b.name));
        for d in group {
            rows.push(ListRow::Table {
                role,
                name: d.name.clone(),
                rows: d.meta.rows,
                key: d.primary_key.clone(),
                fields: d.fields.len(),
                folded: folded_into(model, &d.name)
                    .into_iter()
                    .map(String::from)
                    .collect(),
                joins: joins
                    .iter()
                    .filter(|r| r.from == d.name || r.to == d.name)
                    .map(|r| r.name.clone())
                    .collect(),
            });
            for folded in folded_into(model, &d.name) {
                rows.push(ListRow::Folded {
                    name: folded.to_string(),
                    into: d.name.clone(),
                });
            }
        }
    }
    // A table folded into one that is not placed has no host row to stand under.
    let mut orphans: Vec<&Dataset> = model
        .datasets
        .iter()
        .filter(|d| {
            !d.meta.excluded
                && d.meta
                    .folded_into
                    .as_deref()
                    .is_some_and(|host| !names.contains(host))
        })
        .collect();
    orphans.sort_by(|a, b| a.name.cmp(&b.name));
    for d in orphans {
        rows.push(ListRow::Folded {
            name: d.name.clone(),
            into: d.meta.folded_into.clone().unwrap_or_default(),
        });
    }
    let mut left_out: Vec<&Dataset> = model.datasets.iter().filter(|d| d.meta.excluded).collect();
    left_out.sort_by(|a, b| a.name.cmp(&b.name));
    for d in left_out {
        rows.push(ListRow::LeftOut {
            name: d.name.clone(),
            role: d.meta.role,
        });
    }
    rows
}

fn card_height(folded: usize) -> f64 {
    // One line for the key and field count, one more when references are folded in.
    let lines = 1 + usize::from(folded > 0);
    HEADER_HEIGHT + LINE_HEIGHT * lines as f64 + CARD_PAD
}

fn rank(b: Band) -> u8 {
    match b {
        Band::Events => 0,
        Band::Entities => 1,
        Band::Below => 2,
    }
}

pub fn layout(model: &Model) -> ModelLayout {
    let names = placed_names(model);
    let joins = drawn_joins(model, &names);
    let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &joins {
        *degree.entry(r.from.as_str()).or_default() += 1;
        *degree.entry(r.to.as_str()).or_default() += 1;
    }

    let mut by_band: BTreeMap<u8, Vec<&Dataset>> = BTreeMap::new();
    for d in model.datasets.iter().filter(|d| placed(d)) {
        by_band
            .entry(rank(band_of(d.meta.role)))
            .or_default()
            .push(d);
    }
    for v in by_band.values_mut() {
        v.sort_by(|a, b| {
            let da = degree.get(a.name.as_str()).copied().unwrap_or(0);
            let db = degree.get(b.name.as_str()).copied().unwrap_or(0);
            db.cmp(&da).then_with(|| a.name.cmp(&b.name))
        });
    }

    let mut cards = Vec::new();
    let mut top_bottom = MARGIN;
    let mut right = MARGIN;
    for (band, x) in [
        (Band::Events, MARGIN),
        (Band::Entities, MARGIN + CARD_WIDTH + BAND_GAP),
    ] {
        let mut y = MARGIN;
        for d in by_band.get(&rank(band)).into_iter().flatten() {
            let height = card_height(folded_into(model, &d.name).len());
            cards.push(Card {
                dataset: d.name.clone(),
                x,
                y,
                width: CARD_WIDTH,
                height,
                band,
            });
            y += height + ROW_GAP;
        }
        if y > MARGIN {
            top_bottom = top_bottom.max(y - ROW_GAP);
            right = right.max(x + CARD_WIDTH);
        }
    }
    let below = by_band.get(&2).map(Vec::as_slice).unwrap_or_default();
    let below_y = if top_bottom > MARGIN {
        top_bottom + ROW_BAND_GAP
    } else {
        MARGIN
    };
    let mut x = MARGIN;
    let mut bottom = top_bottom.max(MARGIN);
    for d in below {
        let height = card_height(folded_into(model, &d.name).len());
        cards.push(Card {
            dataset: d.name.clone(),
            x,
            y: below_y,
            width: CARD_WIDTH,
            height,
            band: Band::Below,
        });
        bottom = bottom.max(below_y + height);
        x += CARD_WIDTH + ROW_GAP;
        right = right.max(x - ROW_GAP);
    }

    let at = |name: &str| cards.iter().find(|c| c.dataset == name).unwrap();
    // Each join leaves one card and enters another by a side chosen from their bands.
    struct Join<'a> {
        r: &'a Relationship,
        s: &'a Card,
        e: &'a Card,
        sides: (Side, Side),
    }
    let planned: Vec<Join> = joins
        .iter()
        .map(|r| {
            let (a, b) = (at(&r.from), at(&r.to));
            // The card in the earlier band starts the edge; within a band, the many side.
            let (s, e) = if rank(a.band) <= rank(b.band) {
                (a, b)
            } else {
                (b, a)
            };
            let sides = match (s.band, e.band) {
                (Band::Events, Band::Events) => (Side::Left, Side::Left),
                (Band::Entities, Band::Entities) => (Side::Right, Side::Right),
                (Band::Below, Band::Below) => (Side::Bottom, Side::Bottom),
                (Band::Events, Band::Entities) => (Side::Right, Side::Left),
                // To the row below: leave by the side facing the gap between the bands.
                (Band::Events, Band::Below) => (Side::Right, Side::Top),
                (_, Band::Below) => (Side::Left, Side::Top),
                _ => (Side::Left, Side::Right),
            };
            Join { r, s, e, sides }
        })
        .collect();
    // Every edge on a card side gets its own port, in the order of where the other end
    // lies, so edges fan out instead of converging on one point and their labels part.
    let ports = |card: &Card, side: Side| -> Vec<usize> {
        let mut on: Vec<(usize, f64)> = planned
            .iter()
            .enumerate()
            .filter_map(|(i, j)| {
                let other = if j.s.dataset == card.dataset && j.sides.0 == side {
                    j.e
                } else if j.e.dataset == card.dataset && j.sides.1 == side {
                    j.s
                } else {
                    return None;
                };
                let along = match side {
                    Side::Left | Side::Right => other.y + other.height / 2.0,
                    Side::Top | Side::Bottom => other.x + other.width / 2.0,
                };
                Some((i, along))
            })
            .collect();
        on.sort_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| planned[a.0].r.name.cmp(&planned[b.0].r.name))
        });
        on.into_iter().map(|(i, _)| i).collect()
    };
    let port = |card: &Card, side: Side, edge: usize| -> (f64, f64) {
        let on = ports(card, side);
        let k = on.iter().position(|&i| i == edge).unwrap() as f64;
        let frac = (k + 1.0) / (on.len() as f64 + 1.0);
        match side {
            Side::Left | Side::Right => {
                let x = if side == Side::Left {
                    card.x
                } else {
                    card.x + card.width
                };
                (
                    x,
                    card.y + PORT_INSET + ((card.height - 2.0 * PORT_INSET) * frac).round(),
                )
            }
            Side::Top | Side::Bottom => {
                let y = if side == Side::Top {
                    card.y
                } else {
                    card.y + card.height
                };
                (card.x + (card.width * frac).round(), y)
            }
        }
    };
    // Joins to the row below each get their own run down the gap and their own lane,
    // in the order of where they land, so no two share a segment.
    let mut to_row: Vec<(usize, f64)> = planned
        .iter()
        .enumerate()
        .filter(|(_, j)| j.sides.1 == Side::Top)
        .map(|(i, j)| (i, port(j.e, Side::Top, i).0))
        .collect();
    to_row.sort_by(|a, b| {
        a.1.total_cmp(&b.1)
            .then_with(|| planned[a.0].r.name.cmp(&planned[b.0].r.name))
    });
    let edges = planned
        .iter()
        .enumerate()
        .map(|(i, j)| EdgeLabel {
            relationship: j.r.name.clone(),
            text: edge_text(j.r),
            grade: j.r.meta.grade,
            proposal: is_proposal(j.r),
            points: route(
                (port(j.s, j.sides.0, i), j.sides.0),
                (port(j.e, j.sides.1, i), j.sides.1),
                (
                    to_row.iter().position(|&(k, _)| k == i).unwrap_or(0),
                    to_row.len(),
                ),
            ),
        })
        .collect();

    ModelLayout {
        width: right + MARGIN,
        height: bottom + MARGIN,
        cards,
        edges,
    }
}

/// Which side of a card an edge meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// `[start, control1, control2, end]` between two ports (a join to the row below is
/// two such curves end to end, seven points), each leaving its card square to
/// the side it sits on.
fn route(
    from: ((f64, f64), Side),
    to: ((f64, f64), Side),
    slot: (usize, usize),
) -> Vec<(f64, f64)> {
    let ((a, sa), (b, sb)) = (from, to);
    match (sa, sb) {
        // Round the band's outer side, or under the row below.
        (Side::Left, Side::Left) => vec![a, (a.0 - SWING, a.1), (b.0 - SWING, b.1), b],
        (Side::Right, Side::Right) => vec![a, (a.0 + SWING, a.1), (b.0 + SWING, b.1), b],
        (Side::Bottom, Side::Bottom) => vec![a, (a.0, a.1 + SWING), (b.0, b.1 + SWING), b],
        // Across the gap between the two upper bands.
        (Side::Right, Side::Left) => {
            let dx = ((b.0 - a.0) / 2.0).round();
            vec![a, (a.0 + dx, a.1), (b.0 - dx, b.1), b]
        }
        // Out into the gap, down it to the lane between the upper bands and the row,
        // then along the lane and into the top of a card in the row. Two curves
        // joined at the lane, so the sweep sideways happens below every upper card
        // and cannot cut across the stacked cards of either column.
        (Side::Right, Side::Top) => down_the_gap(a, b, Side::Right, slot),
        _ => down_the_gap(a, b, Side::Left, slot),
    }
}

/// Two joined cubics, seven points, from a port out into the gap, down it, along a
/// lane and into `b`, the top of a card in the row below. `slot` is this join's place
/// `(k, n)` among the `n` joins to the row: the k-th takes the k-th of `n` evenly
/// spaced runs down the gap and the k-th of `n` lanes between the upper bands' lowest
/// cards and the row, as ports are spaced on a card side.
fn down_the_gap(
    a: (f64, f64),
    b: (f64, f64),
    leaving: Side,
    slot: (usize, usize),
) -> Vec<(f64, f64)> {
    let frac = (slot.0 as f64 + 1.0) / (slot.1 as f64 + 1.0);
    let (gap_left, sign) = if leaving == Side::Right {
        (a.0, 1.0)
    } else {
        (a.0 - BAND_GAP, -1.0)
    };
    let q = (
        gap_left + (BAND_GAP * frac).round(),
        b.1 - ROW_BAND_GAP + LANE_INSET + ((ROW_BAND_GAP - 2.0 * LANE_INSET) * frac).round(),
    );
    let out = TURN.min((q.0 - a.0).abs());
    let dip = ((b.1 - q.1) / 2.0).round().min(LANE_DIP);
    vec![
        a,
        (a.0 + sign * out, a.1),
        (q.0, q.1 - TURN),
        q,
        (q.0, q.1 + dip),
        (b.0, b.1 - dip),
        b,
    ]
}
