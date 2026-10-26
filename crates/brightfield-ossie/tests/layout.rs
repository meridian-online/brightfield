use brightfield_ossie::{
    edge_text, layout, list_rows, read, summary, Band, DecidedBy, ListRow, Model, Role,
};

const STAND_IN: &str = include_str!("../fixtures/retail_extract/model.ossie.yaml");

fn model() -> Model {
    read(STAND_IN).unwrap()
}

fn band_of(m: &Model, name: &str) -> Option<Band> {
    layout(m)
        .cards
        .iter()
        .find(|c| c.dataset == name)
        .map(|c| c.band)
}

#[test]
fn retail_bands() {
    let m = model();
    for n in ["transactions", "loyalty_visits"] {
        assert_eq!(band_of(&m, n), Some(Band::Events), "{n}");
    }
    for n in ["customers", "stores", "products"] {
        assert_eq!(band_of(&m, n), Some(Band::Entities), "{n}");
    }
    for n in ["customer_segments", "promo_codes"] {
        assert_eq!(band_of(&m, n), Some(Band::Below), "{n}");
    }
    assert_eq!(band_of(&m, "categories"), None);
    let l = layout(&m);
    assert_eq!(l.cards.len(), 7);
    assert_eq!(l.edges.len(), 6);
    for e in &l.edges {
        // One cubic, or two when the join turns in the lane above the row below.
        assert!(e.points.len() == 4 || e.points.len() == 7, "{e:?}");
    }
    // Customers has the most joins, so it leads its band.
    let first = l.cards.iter().find(|c| c.band == Band::Entities).unwrap();
    assert_eq!(first.dataset, "customers");
}

#[test]
fn layout_is_deterministic() {
    let m = model();
    assert_eq!(layout(&m), layout(&m));
    let mut shuffled = m.clone();
    shuffled.datasets.reverse();
    shuffled.relationships.reverse();
    let mut a = layout(&m);
    let mut b = layout(&shuffled);
    a.cards.sort_by(|x, y| x.dataset.cmp(&y.dataset));
    b.cards.sort_by(|x, y| x.dataset.cmp(&y.dataset));
    assert_eq!(a, b);
    assert_eq!(layout(&m), layout(&shuffled));
}

#[test]
fn whole_pixels() {
    let l = layout(&model());
    let whole = |v: f64| v == v.round();
    assert!(whole(l.width) && whole(l.height));
    for c in &l.cards {
        assert!(whole(c.x) && whole(c.y) && whole(c.width) && whole(c.height));
    }
    for e in &l.edges {
        assert!(e.points.iter().all(|p| whole(p.0) && whole(p.1)));
    }
}

#[test]
fn edge_text_for_composite_keys() {
    let mut m = model();
    let mut r = m.relationships[0].clone();
    assert_eq!(edge_text(&r), "customer_id = customer_id  N:1");
    r.from_columns = vec!["a".into(), "b".into()];
    r.to_columns = vec!["x".into(), "y".into()];
    assert_eq!(edge_text(&r), "a, b = x, y  N:1");
    m.relationships.clear();
    assert_eq!(summary(&m), "7 tables · 0 joins · 0 possible · 1 unclear");
}

#[test]
fn summary_counts_retail() {
    assert_eq!(
        summary(&model()),
        "7 tables · 6 joins · 1 possible · 1 unclear"
    );
}

#[test]
fn list_puts_possible_first() {
    let rows = list_rows(&model());
    match &rows[0] {
        ListRow::Review {
            relationship,
            edge_text,
        } => {
            assert_eq!(relationship, "customer_segments_customers");
            assert_eq!(edge_text, "customer_id = customer_id  N:1");
        }
        other => panic!("{other:?}"),
    }
    let roles: Vec<Role> = rows
        .iter()
        .filter_map(|r| match r {
            ListRow::Table { role, .. } => Some(*role),
            _ => None,
        })
        .collect();
    assert_eq!(roles.len(), 7);
    assert_eq!(roles[0], Role::Entity);
    assert_eq!(*roles.last().unwrap(), Role::Unclear);
    let products = rows
        .iter()
        .find_map(|r| match r {
            ListRow::Table {
                name, folded, key, ..
            } if name == "products" => Some((folded.clone(), key.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        products,
        (vec!["categories".to_string()], vec!["sku".to_string()])
    );
}

/// Every edge's curve, sampled finely, stays out of every card but its own two ends.
fn assert_routes_clear_of_cards(m: &Model) {
    let l = layout(m);
    for e in &l.edges {
        let r = m
            .relationships
            .iter()
            .find(|r| r.name == e.relationship)
            .unwrap();
        for (k, step) in (0..e.points.len() / 3).flat_map(|k| (0..=400).map(move |s| (k, s))) {
            let t = f64::from(step) / 400.0;
            let p = &e.points[3 * k..=3 * k + 3];
            let u = 1.0 - t;
            let at = |f: fn(&(f64, f64)) -> f64| {
                u * u * u * f(&p[0])
                    + 3.0 * u * u * t * f(&p[1])
                    + 3.0 * u * t * t * f(&p[2])
                    + t * t * t * f(&p[3])
            };
            let (x, y) = (at(|q| q.0), at(|q| q.1));
            for c in &l.cards {
                if c.dataset == r.from || c.dataset == r.to {
                    continue;
                }
                let inside = x > c.x && x < c.x + c.width && y > c.y && y < c.y + c.height;
                assert!(
                    !inside,
                    "{} passes through {} at ({x:.0}, {y:.0})",
                    e.relationship, c.dataset
                );
            }
        }
    }
}

#[test]
fn retail_routes_clear_of_cards() {
    assert_routes_clear_of_cards(&model());
    assert_routes_distinct(&model());
}

/// Four stacked event cards, two entities, and a row of four below, with joins to the
/// row's cards from both upper columns, including ones under the opposite column.
#[test]
fn routes_to_the_row_below_clear_stacked_cards() {
    let base = model();
    let dataset = |from: &str, name: &str, role: Role| {
        let mut d = base
            .datasets
            .iter()
            .find(|d| d.name == from)
            .unwrap()
            .clone();
        d.name = name.into();
        d.meta.role = role;
        d.meta.folded_into = None;
        d
    };
    let mut m = base.clone();
    m.datasets.clear();
    for i in 1..=4 {
        m.datasets
            .push(dataset("transactions", &format!("e{i}"), Role::Event));
        m.datasets
            .push(dataset("promo_codes", &format!("b{i}"), Role::Reference));
    }
    for i in 1..=2 {
        m.datasets
            .push(dataset("customers", &format!("n{i}"), Role::Entity));
    }
    let template = base.relationships[0].clone();
    m.relationships.clear();
    for (from, to) in [
        ("e1", "n1"),
        ("e2", "n2"),
        ("e3", "n1"),
        ("e4", "n2"),
        ("e1", "b4"),
        ("e2", "b3"),
        ("e4", "b4"),
        ("n1", "b1"),
        ("n2", "b4"),
        ("n2", "b2"),
    ] {
        let mut r = template.clone();
        r.name = format!("{from}_{to}");
        r.from = from.into();
        r.to = to.into();
        m.relationships.push(r);
    }
    let l = layout(&m);
    assert_eq!(l.edges.len(), 10);
    assert_routes_clear_of_cards(&m);
    assert_routes_distinct(&m);
}

/// The curve of an edge sampled finely, as points.
fn sampled(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for k in 0..points.len() / 3 {
        let p = &points[3 * k..=3 * k + 3];
        for step in 0..=200 {
            let t = f64::from(step) / 200.0;
            let u = 1.0 - t;
            let at = |f: fn(&(f64, f64)) -> f64| {
                u * u * u * f(&p[0])
                    + 3.0 * u * u * t * f(&p[1])
                    + 3.0 * u * t * t * f(&p[2])
                    + t * t * t * f(&p[3])
            };
            out.push((at(|q| q.0), at(|q| q.1)));
        }
    }
    out
}

/// Joins to the row below never share a junction, a run down the gap or a lane, and no
/// two routes lie along one another: where two routes cross they come within a pixel
/// at a few samples, where they overlap along a run it is hundreds.
fn assert_routes_distinct(m: &Model) {
    let l = layout(m);
    let turns: Vec<_> = l.edges.iter().filter(|e| e.points.len() == 7).collect();
    for (i, a) in turns.iter().enumerate() {
        for b in &turns[i + 1..] {
            let (q, r) = (a.points[3], b.points[3]);
            assert!(
                q.0 != r.0 && q.1 != r.1,
                "{} and {} share a run or a lane",
                a.relationship,
                b.relationship
            );
        }
    }
    for (i, a) in l.edges.iter().enumerate() {
        let sa = sampled(&a.points);
        for b in &l.edges[i + 1..] {
            let sb = sampled(&b.points);
            let close = sa
                .iter()
                .flat_map(|p| sb.iter().map(move |q| (p, q)))
                .filter(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1) < 1.0)
                .count();
            assert!(
                close < 60,
                "{} and {} run along one another ({close} close samples)",
                a.relationship,
                b.relationship
            );
        }
    }
}

#[test]
fn a_kept_possible_join_leaves_the_review_queue() {
    let mut m = model();
    let kept = m
        .relationships
        .iter_mut()
        .find(|r| r.name == "customer_segments_customers")
        .unwrap();
    kept.meta.decided_by = DecidedBy::Analyst;
    let rows = list_rows(&m);
    assert!(
        !rows.iter().any(|r| matches!(r, ListRow::Review { .. })),
        "{rows:?}"
    );
    let edge = layout(&m)
        .edges
        .into_iter()
        .find(|e| e.relationship == "customer_segments_customers")
        .unwrap();
    // Still graded possible, as evidence, but no longer drawn as a proposal.
    assert_eq!(edge.grade, brightfield_ossie::Grade::Possible);
    assert!(!edge.proposal);
    assert!(layout(&model())
        .edges
        .iter()
        .any(|e| e.relationship == "customer_segments_customers" && e.proposal));
}

#[test]
fn folded_and_left_out_tables_have_rows() {
    let mut m = model();
    m.datasets
        .iter_mut()
        .find(|d| d.name == "promo_codes")
        .unwrap()
        .meta
        .excluded = true;
    let rows = list_rows(&m);
    let at = |want: &ListRow| rows.iter().position(|r| r == want);
    let host = rows
        .iter()
        .position(|r| matches!(r, ListRow::Table { name, .. } if name == "products"))
        .unwrap();
    // The folded table stands right under its host…
    assert_eq!(
        at(&ListRow::Folded {
            name: "categories".into(),
            into: "products".into()
        }),
        Some(host + 1)
    );
    // …and the table left out is last, in a group of its own.
    assert_eq!(
        rows.last(),
        Some(&ListRow::LeftOut {
            name: "promo_codes".into(),
            role: Role::Unclear
        })
    );
    assert!(!rows
        .iter()
        .any(|r| matches!(r, ListRow::Table { name, .. } if name == "promo_codes")));
}
