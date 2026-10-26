use brightfield_ossie::{
    apply, merge_discovery, read, DecidedBy, Edit, Grade, Model, Role, Verified,
};

const STAND_IN: &str = include_str!("../fixtures/retail_extract/model.ossie.yaml");

fn model() -> Model {
    read(STAND_IN).unwrap()
}

fn dataset<'a>(m: &'a Model, name: &str) -> &'a brightfield_ossie::Dataset {
    m.datasets.iter().find(|d| d.name == name).unwrap()
}

fn rel<'a>(m: &'a Model, name: &str) -> Option<&'a brightfield_ossie::Relationship> {
    m.relationships.iter().find(|r| r.name == name)
}

#[test]
fn drop_moves_to_rejected() {
    let m = apply(
        &model(),
        &Edit::DropJoin {
            relationship: "transactions_customers".into(),
        },
    )
    .unwrap();
    assert!(rel(&m, "transactions_customers").is_none());
    assert_eq!(m.relationships.len(), 6);
    let r = m
        .meta
        .rejected
        .iter()
        .find(|r| r.name == "transactions_customers")
        .unwrap();
    assert_eq!(r.meta.decided_by, DecidedBy::Analyst);
}

#[test]
fn keep_marks_analyst() {
    let m = apply(
        &model(),
        &Edit::KeepJoin {
            relationship: "customer_segments_customers".into(),
        },
    )
    .unwrap();
    let r = rel(&m, "customer_segments_customers").unwrap();
    assert_eq!(r.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(r.meta.grade, Grade::Possible);
}

#[test]
fn add_join_names_and_dedupes() {
    let add = |cols: &str| Edit::AddJoin {
        from: "loyalty_visits".into(),
        from_columns: vec![cols.into()],
        to: "products".into(),
        to_columns: vec!["sku".into()],
    };
    let m = apply(&model(), &add("a")).unwrap();
    let m = apply(&m, &add("b")).unwrap();
    let first = rel(&m, "loyalty_visits_products").unwrap();
    assert_eq!(first.meta.grade, Grade::Possible);
    assert_eq!(first.meta.verified, Verified::Unmeasured);
    assert_eq!(first.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(first.meta.cardinality, "N:1");
    assert_eq!(first.meta.evidence.child_rows, 0);
    assert!(rel(&m, "loyalty_visits_products_2").is_some());
    let e = apply(
        &model(),
        &Edit::AddJoin {
            from: "nope".into(),
            from_columns: vec!["a".into()],
            to: "products".into(),
            to_columns: vec!["sku".into()],
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("nope"));
}

#[test]
fn change_keys_resets_verification() {
    let m = apply(
        &model(),
        &Edit::ChangeKeys {
            relationship: "transactions_customers".into(),
            from_columns: vec!["x".into()],
            to_columns: vec!["y".into()],
        },
    )
    .unwrap();
    let r = rel(&m, "transactions_customers").unwrap();
    assert_eq!(r.from_columns, vec!["x"]);
    assert_eq!(r.meta.verified, Verified::Unmeasured);
    assert_eq!(r.meta.decided_by, DecidedBy::Analyst);
}

#[test]
fn fold_without_join_is_refused() {
    let e = apply(
        &model(),
        &Edit::Fold {
            dataset: "promo_codes".into(),
            into: "products".into(),
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("promo_codes"));
    let m = apply(
        &model(),
        &Edit::Fold {
            dataset: "stores".into(),
            into: "transactions".into(),
        },
    )
    .unwrap();
    let d = dataset(&m, "stores");
    assert_eq!(d.meta.folded_into.as_deref(), Some("transactions"));
    assert_eq!(d.meta.role, Role::Reference);
    let m = apply(
        &m,
        &Edit::Unfold {
            dataset: "stores".into(),
        },
    )
    .unwrap();
    assert_eq!(dataset(&m, "stores").meta.folded_into, None);
}

#[test]
fn exclude_and_include_toggle() {
    let m = apply(
        &model(),
        &Edit::Exclude {
            dataset: "stores".into(),
        },
    )
    .unwrap();
    assert!(dataset(&m, "stores").meta.excluded);
    let m = apply(
        &m,
        &Edit::Include {
            dataset: "stores".into(),
        },
    )
    .unwrap();
    assert!(!dataset(&m, "stores").meta.excluded);
    assert_eq!(dataset(&m, "stores").meta.decided_by, DecidedBy::Analyst);
}

#[test]
fn retype_marks_field_analyst() {
    let base = model();
    let d = &base.datasets[0];
    let f = d.fields[0].name.clone();
    let m = apply(
        &base,
        &Edit::Retype {
            dataset: d.name.clone(),
            field: f.clone(),
            finetype: "identity.person.email".into(),
        },
    )
    .unwrap();
    let got = m.datasets[0].fields.iter().find(|x| x.name == f).unwrap();
    assert_eq!(got.meta.finetype.as_deref(), Some("identity.person.email"));
    assert_eq!(got.meta.decided_by, DecidedBy::Analyst);
    assert!(apply(
        &base,
        &Edit::Retype {
            dataset: d.name.clone(),
            field: "no_such".into(),
            finetype: "x".into()
        }
    )
    .is_err());
}

#[test]
fn merge_keeps_analyst_role() {
    let current = apply(
        &model(),
        &Edit::SetRole {
            dataset: "stores".into(),
            role: Role::Snapshot,
        },
    )
    .unwrap();
    let mut proposed = model();
    proposed
        .datasets
        .iter_mut()
        .find(|d| d.name == "stores")
        .unwrap()
        .meta
        .role = Role::Entity;
    let merged = merge_discovery(&current, &proposed);
    assert_eq!(dataset(&merged, "stores").meta.role, Role::Snapshot);
}

#[test]
fn merge_does_not_resurrect_dropped() {
    let current = apply(
        &model(),
        &Edit::DropJoin {
            relationship: "transactions_customers".into(),
        },
    )
    .unwrap();
    let merged = merge_discovery(&current, &model());
    assert!(rel(&merged, "transactions_customers").is_none());
    assert_eq!(merged.meta.rejected.len(), 1);
}

#[test]
fn merge_takes_new_dovetail_grades() {
    let current = model();
    let mut proposed = model();
    proposed
        .relationships
        .iter_mut()
        .find(|r| r.name == "customer_segments_customers")
        .unwrap()
        .meta
        .grade = Grade::Likely;
    // A join Dovetail no longer finds goes; a new one arrives.
    let gone = proposed
        .relationships
        .iter()
        .position(|r| r.name == "loyalty_visits_stores")
        .unwrap();
    let mut fresh = proposed.relationships.remove(gone);
    fresh.name = "loyalty_visits_products".into();
    fresh.to = "products".into();
    fresh.to_columns = vec!["sku".into()];
    proposed.relationships.push(fresh);
    let merged = merge_discovery(&current, &proposed);
    assert_eq!(
        rel(&merged, "customer_segments_customers")
            .unwrap()
            .meta
            .grade,
        Grade::Likely
    );
    assert!(rel(&merged, "loyalty_visits_stores").is_none());
    assert!(rel(&merged, "loyalty_visits_products").is_some());
}

#[test]
fn merge_keeps_analyst_fields_excluded_and_extra_datasets() {
    let base = model();
    let d = base.datasets[0].clone();
    let f = d.fields[0].name.clone();
    let current = apply(
        &apply(
            &apply(
                &base,
                &Edit::Retype {
                    dataset: d.name.clone(),
                    field: f.clone(),
                    finetype: "mine".into(),
                },
            )
            .unwrap(),
            &Edit::Exclude {
                dataset: "promo_codes".into(),
            },
        )
        .unwrap(),
        &Edit::AddJoin {
            from: "loyalty_visits".into(),
            from_columns: vec!["a".into()],
            to: "products".into(),
            to_columns: vec!["sku".into()],
        },
    )
    .unwrap();
    let mut proposed = model();
    proposed.datasets.retain(|x| x.name != "promo_codes");
    proposed.datasets[0].fields[0].meta.finetype = Some("theirs".into());
    let merged = merge_discovery(&current, &proposed);
    assert_eq!(
        dataset(&merged, &d.name).fields[0].meta.finetype.as_deref(),
        Some("mine")
    );
    assert!(dataset(&merged, "promo_codes").meta.excluded);
    assert!(rel(&merged, "loyalty_visits_products").is_some());
}

#[test]
fn merge_is_idempotent() {
    let m = apply(
        &model(),
        &Edit::DropJoin {
            relationship: "transactions_stores".into(),
        },
    )
    .unwrap();
    assert_eq!(merge_discovery(&m, &m), m);
    let plain = model();
    assert_eq!(merge_discovery(&plain, &plain), plain);
}

#[test]
fn change_keys_then_merge() {
    let current = apply(
        &model(),
        &Edit::ChangeKeys {
            relationship: "transactions_customers".into(),
            from_columns: vec!["x".into()],
            to_columns: vec!["y".into()],
        },
    )
    .unwrap();
    let merged = merge_discovery(&current, &model());
    let n = merged
        .relationships
        .iter()
        .filter(|r| r.from == "transactions" && r.to == "customers")
        .count();
    assert_eq!(n, 1);
    assert_eq!(
        rel(&merged, "transactions_customers").unwrap().from_columns,
        vec!["x"]
    );
}

#[test]
fn merge_renames_a_colliding_proposed_join() {
    let current = model();
    let mut proposed = model();
    let mut fresh = proposed.relationships[0].clone();
    fresh.to_columns = vec!["other".into()];
    proposed.relationships.push(fresh);
    let merged = merge_discovery(&current, &proposed);
    assert_eq!(merged.relationships.len(), 8);
    let mut names: Vec<_> = merged.relationships.iter().map(|r| &r.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 8);
}

#[test]
fn drop_of_the_join_behind_a_fold_is_refused() {
    let e = apply(
        &model(),
        &Edit::DropJoin {
            relationship: "products_categories".into(),
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("unfold"));
}

#[test]
fn merge_carries_excluded_on_a_dovetail_dataset() {
    let mut current = model();
    let d = current
        .datasets
        .iter_mut()
        .find(|d| d.name == "promo_codes")
        .unwrap();
    assert_eq!(d.meta.decided_by, DecidedBy::Dovetail);
    d.meta.excluded = true;
    let merged = merge_discovery(&current, &model());
    assert!(dataset(&merged, "promo_codes").meta.excluded);
}
