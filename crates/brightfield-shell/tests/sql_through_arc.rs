//! Typed SQL, asked of arc.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite,
//! which becomes `arc` under the runner variable, so every answer here is
//! arc's own and not a fixture shaped like one.

use brightfield_shell::operation::{recognise, Recognised};
use brightfield_shell::run::Runner;

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

#[test]
fn a_filter_is_read_as_filter_rows_with_its_condition_as_typed() {
    assert_eq!(
        recognise(
            &runner(),
            "select * from california_housing where house_age > 28"
        )
        .unwrap(),
        Recognised::FilterRows {
            on: "california_housing".into(),
            condition: "house_age > 28".into()
        }
    );
    assert_eq!(
        recognise(
            &runner(),
            "select *\nfrom california_housing\nwhere house_age > 28"
        )
        .unwrap(),
        Recognised::FilterRows {
            on: "california_housing".into(),
            condition: "house_age > 28".into()
        }
    );
}

#[test]
fn a_sort_a_projection_and_a_group_are_read_as_no_operation() {
    for sql in [
        "select * from california_housing order by house_age",
        "select house_age from california_housing",
        "select house_age, count(*) from california_housing group by house_age",
    ] {
        assert_eq!(
            recognise(&runner(), sql).unwrap(),
            Recognised::Other,
            "{sql}"
        );
    }
}

#[test]
fn text_arc_cannot_parse_is_refused_in_arcs_words() {
    let e = recognise(&runner(), "select from where").unwrap_err();
    assert!(!e.starts_with("error: ") && !e.is_empty(), "{e}");
}
