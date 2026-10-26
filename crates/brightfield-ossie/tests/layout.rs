use brightfield_ossie::{edge_text, layout, list_rows, read, summary, Band, ListRow, Model, Role};

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
        assert_eq!(e.points.len(), 4);
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
