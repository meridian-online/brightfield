//! Writing a join's evidence measured again after a run: what changes, what warns, and what stays
//! the analyst's or Dovetail's to decide. What the statement measures over a run's database is
//! checked against real silver tables in the shell's `tests/model_rerun.rs`.

use brightfield_ossie::{
    apply, apply_evidence, evidence_sql, read, write, DecidedBy, Edit, Evidence, Grade, Model,
    Verified,
};

const STAND_IN: &str = include_str!("../fixtures/retail_extract/model.ossie.yaml");

fn model() -> Model {
    read(STAND_IN).unwrap()
}

fn rel<'a>(m: &'a Model, name: &str) -> &'a brightfield_ossie::Relationship {
    m.relationships.iter().find(|r| r.name == name).unwrap()
}

/// `name`'s evidence as it stands, with the four measured numbers replaced.
fn measured(m: &Model, name: &str, overlap: f64, orphans: u64, rows: u64, dups: u64) -> Evidence {
    Evidence {
        value_overlap: overlap,
        orphans,
        child_rows: rows,
        parent_duplicates: dups,
        ..rel(m, name).meta.evidence.clone()
    }
}

#[test]
fn new_orphans_warn_without_regrading() {
    let m = model();
    let before = rel(&m, "transactions_customers").meta.clone();
    let after = apply_evidence(
        &m,
        "transactions_customers",
        measured(&m, "transactions_customers", 0.8, 12, 20, 0),
    );
    let r = rel(&after, "transactions_customers");
    assert_eq!(
        r.meta.warning.as_deref(),
        Some("12 transactions rows now point at no customers row")
    );
    assert_eq!(r.meta.evidence.orphans, 12);
    assert_eq!(r.meta.evidence.child_rows, 20);
    assert!((r.meta.evidence.value_overlap - 0.8).abs() < f64::EPSILON);
    assert_eq!(r.meta.grade, before.grade);
    assert_eq!(r.meta.decided_by, before.decided_by);
    assert_eq!(r.meta.verified, before.verified);
    // Every other join is as it was.
    for other in &m.relationships {
        if other.name != "transactions_customers" {
            assert_eq!(rel(&after, &other.name), other);
        }
    }

    // Once the orphans are gone, so is the warning.
    let mended = apply_evidence(
        &after,
        "transactions_customers",
        measured(&after, "transactions_customers", 1.0, 0, 8, 0),
    );
    assert_eq!(rel(&mended, "transactions_customers").meta.warning, None);
    assert_eq!(
        mended, m,
        "measuring the fixture's own numbers gives the fixture"
    );

    // A possible join is not expected to find every parent, so its orphans do not warn.
    let possible = apply_evidence(
        &m,
        "customer_segments_customers",
        measured(&m, "customer_segments_customers", 0.5, 2, 4, 0),
    );
    let r = rel(&possible, "customer_segments_customers");
    assert_eq!(r.meta.grade, Grade::Possible);
    assert_eq!(r.meta.warning, None);
    assert_eq!(r.meta.evidence.orphans, 2);

    // A parent key that repeats breaks an N:1 join, whatever its grade.
    let repeated = apply_evidence(
        &m,
        "customer_segments_customers",
        measured(&m, "customer_segments_customers", 1.0, 0, 4, 1),
    );
    assert_eq!(
        rel(&repeated, "customer_segments_customers")
            .meta
            .warning
            .as_deref(),
        Some("1 customer_id value now repeats in customers")
    );
    let both = apply_evidence(
        &m,
        "transactions_customers",
        measured(&m, "transactions_customers", 0.9, 1, 9, 2),
    );
    assert_eq!(
        rel(&both, "transactions_customers").meta.warning.as_deref(),
        Some(
            "1 transactions row now points at no customers row; \
             2 customer_id values now repeat in customers"
        )
    );

    // The warning and the new measure are written to the file and read back.
    let back = read(&write(&both)).unwrap();
    assert_eq!(back, both);
}

#[test]
fn a_file_without_duplicates_writes_as_it_did() {
    // The measure added for parent uniqueness stays out of a file that has none to report.
    let m = model();
    assert_eq!(write(&m), write(&read(&write(&m)).unwrap()));
    assert!(!write(&m).contains("parent_duplicates"));
}

#[test]
fn added_join_gets_measured_evidence() {
    let m = apply(
        &model(),
        &Edit::AddJoin {
            from: "loyalty_visits".into(),
            from_columns: vec!["visit_id".into()],
            to: "transactions".into(),
            to_columns: vec!["txn_id".into()],
        },
    )
    .unwrap();
    let added = rel(&m, "loyalty_visits_transactions").clone();
    assert_eq!(added.meta.verified, Verified::Unmeasured);
    assert_eq!(added.meta.evidence.child_rows, 0);

    let after = apply_evidence(
        &m,
        "loyalty_visits_transactions",
        measured(&m, "loyalty_visits_transactions", 0.0, 6, 6, 0),
    );
    let r = rel(&after, "loyalty_visits_transactions");
    assert_eq!(r.meta.evidence.child_rows, 6);
    assert_eq!(r.meta.evidence.orphans, 6);
    // Still the analyst's, still possible, still not verified by a discovery pass; and a possible
    // join's orphans do not warn.
    assert_eq!(r.meta.decided_by, DecidedBy::Analyst);
    assert_eq!(r.meta.grade, Grade::Possible);
    assert_eq!(r.meta.verified, Verified::Unmeasured);
    assert_eq!(r.meta.warning, None);
}

#[test]
fn no_statement_for_a_join_the_model_lacks() {
    assert_eq!(evidence_sql(&model(), "no_such_join"), "");
    assert!(!evidence_sql(&model(), "transactions_customers").is_empty());
}
