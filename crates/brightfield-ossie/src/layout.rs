//! Role-banded layout of a model: events on the left, entities on the right,
//! snapshots, bridges, references and unclear tables in a row below.
//!
//! No text measurement and no UI framework: card sizes are fixed or derived from
//! line counts, coordinates are whole pixels, and every tie breaks by name, so the
//! same model always yields the same layout whatever order its datasets came in.

use std::collections::{BTreeMap, BTreeSet};

use meridian_design::spacing;

use crate::model::{Dataset, Grade, Model, Relationship, Role};

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

/// A drawn join: its text and a cubic Bezier `[start, control1, control2, end]`.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeLabel {
    pub relationship: String,
    pub text: String,
    pub grade: Grade,
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
    /// A placed table.
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
}

const MARGIN: f64 = spacing::SPACE_8 as f64;
const CARD_WIDTH: f64 = 200.0;
const HEADER_HEIGHT: f64 = 32.0;
const LINE_HEIGHT: f64 = 20.0;
const CARD_PAD: f64 = spacing::SPACE_5 as f64;
const ROW_GAP: f64 = spacing::SPACE_6 as f64;
/// Space between the two upper bands, wide enough for edges to curve through.
const BAND_GAP: f64 = spacing::SPACE_9 as f64 * 3.0;
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
    let possible = joins
        .iter()
        .filter(|r| r.meta.grade == Grade::Possible)
        .count();
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

/// Possible-join review rows first, then tables grouped by role, by name within a role.
pub fn list_rows(model: &Model) -> Vec<ListRow> {
    let names = placed_names(model);
    let joins = drawn_joins(model, &names);
    let mut rows: Vec<ListRow> = joins
        .iter()
        .filter(|r| r.meta.grade == Grade::Possible)
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
        }
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
    let edges = joins
        .iter()
        .map(|r| {
            let (a, b) = (at(&r.from), at(&r.to));
            // The card in the earlier band starts the edge; within a band, the many side.
            let (s, e) = if rank(a.band) <= rank(b.band) {
                (a, b)
            } else {
                (b, a)
            };
            EdgeLabel {
                relationship: r.name.clone(),
                text: edge_text(r),
                grade: r.meta.grade,
                points: route(s, e),
            }
        })
        .collect();

    ModelLayout {
        width: right + MARGIN,
        height: bottom + MARGIN,
        cards,
        edges,
    }
}

fn mid_y(c: &Card) -> f64 {
    (c.y + c.height / 2.0).round()
}

fn mid_x(c: &Card) -> f64 {
    (c.x + c.width / 2.0).round()
}

/// `[start, control1, control2, end]` for an edge from `s` to `e`.
fn route(s: &Card, e: &Card) -> Vec<(f64, f64)> {
    if s.band == e.band {
        // Around the band's outer side: left for events, right for entities, under the row below.
        return match s.band {
            Band::Events => {
                let (a, b) = ((s.x, mid_y(s)), (e.x, mid_y(e)));
                vec![a, (a.0 - SWING, a.1), (b.0 - SWING, b.1), b]
            }
            Band::Entities => {
                let (a, b) = ((s.x + s.width, mid_y(s)), (e.x + e.width, mid_y(e)));
                vec![a, (a.0 + SWING, a.1), (b.0 + SWING, b.1), b]
            }
            Band::Below => {
                let (a, b) = ((mid_x(s), s.y + s.height), (mid_x(e), e.y + e.height));
                vec![a, (a.0, a.1 + SWING), (b.0, b.1 + SWING), b]
            }
        };
    }
    if e.band == Band::Below {
        // From the bottom of a card above to the top of one below.
        let (a, b) = ((mid_x(s), s.y + s.height), (mid_x(e), e.y));
        let dy = ((b.1 - a.1) / 2.0).round();
        return vec![a, (a.0, a.1 + dy), (b.0, b.1 - dy), b];
    }
    // Events to entities: right edge to left edge.
    let (a, b) = ((s.x + s.width, mid_y(s)), (e.x, mid_y(e)));
    let dx = ((b.0 - a.0) / 2.0).round();
    vec![a, (a.0 + dx, a.1), (b.0 - dx, b.1), b]
}
