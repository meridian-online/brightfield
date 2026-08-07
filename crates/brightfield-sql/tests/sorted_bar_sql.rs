//! Gate: a lifted `sort:` becomes an `ORDER BY` on the VALUE and a `LIMIT`.
//!
//! Asserted through `emit_query`, the entry point the engine calls, rather
//! than on the plan a lowerer returned: a `QueryPlan::Limit` in the right
//! shape proves nothing about the bytes DuckDB is handed, and the selection
//! threading that runs between the two is exactly where the placement of a
//! `LIMIT` goes wrong.
//!
//! What the arithmetic of that ordering does to real rows is
//! `brightfield-engine`'s `sort_limit_oracle`; what it does to pixels is
//! `brightfield-shell`'s `sorted_bar_ink`. This file is about the SQL.

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::emit::emit_query;
use brightfield_sql::ir::Predicate;

/// The vendored upstream spec this idiom exists for, so what is asserted is
/// portability truth rather than a fixture written to match the code.
const SORTED_BARS: &str =
    include_str!("../../brightfield-spec/vendor/mosaic-specs/yaml/sorted-bars.yaml");

/// `emit_query` for one mark of a YAML source, with no selection.
fn sql(source: &str, mark: usize) -> String {
    let spec = parse_spec(source, Format::Yaml).expect("spec parses").spec;
    emit_query(&spec, mark, None, None).expect("mark emits").sql
}

/// `sorted-bars.yaml`: `sum(gold)` per nationality, ranked, ten bars.
#[test]
fn the_vendored_spec_orders_by_the_aggregate_alias_and_limits() {
    let emitted = sql(SORTED_BARS, 0);
    assert_eq!(
        emitted,
        "SELECT * FROM (SELECT * FROM (SELECT \"nationality\", \
         CAST(sum(\"gold\") AS DOUBLE) AS \"gold\" FROM (SELECT * FROM \
         (SELECT * FROM \"athletes\") AS _f WHERE \"nationality\" IS NOT NULL) \
         AS _a GROUP BY 1) AS _o ORDER BY \"gold\" DESC, \"nationality\" ASC) \
         AS _l LIMIT 10",
        "the order reads the aggregate's OUTPUT alias, the band is the \
         tiebreak, and the LIMIT is outside the ORDER BY"
    );
}

/// The order is on the aggregate, not on the band, and the direction is the
/// one the `-` asked for. Written as a separate spec so the assertion is about
/// the mechanism rather than about one vendored file's columns.
#[test]
fn ascending_and_descending_are_different_queries() {
    let base = "data:\n  t: [{ region: a, v: 1 }]\nplot:\n- mark: barX\n  \
                data: { from: t }\n  x: { sum: v }\n  y: region\n";
    let desc = sql(&format!("{base}  sort: {{ y: -x }}\n"), 0);
    let asc = sql(&format!("{base}  sort: {{ y: x }}\n"), 0);
    assert!(
        desc.contains("ORDER BY \"v\" DESC, \"region\" ASC"),
        "descending: {desc}"
    );
    assert!(
        asc.contains("ORDER BY \"v\" ASC, \"region\" ASC"),
        "ascending: {asc}"
    );
    assert!(
        !desc.contains("LIMIT") && !asc.contains("LIMIT"),
        "neither wrote a `limit:`, so neither may emit one"
    );
}

/// A `{count:}` bar orders by the reserved alias, because that is the column
/// the aggregate writes. Restating the alias rule at the `ORDER BY` instead of
/// reading it off the aggregate is how the two drift apart.
#[test]
fn a_counting_bar_orders_by_the_reserved_count_column() {
    let emitted = sql(
        "data:\n  t: [{ region: a }]\nplot:\n- mark: barX\n  data: { from: t }\n  \
         x: { count: }\n  y: region\n  sort: { y: -x, limit: 3 }\n",
        0,
    );
    assert!(
        emitted.contains("ORDER BY __bf_count DESC, \"region\" ASC"),
        "{emitted}"
    );
    assert!(emitted.ends_with("LIMIT 3"), "{emitted}");
}

/// A pre-aggregated bar — a plain column on the value axis — ranks by that
/// column. It takes `SimpleLowerer`'s pass-through plan and the same wrapper.
#[test]
fn a_pre_aggregated_bar_ranks_by_its_value_column() {
    let emitted = sql(
        "data:\n  t: [{ region: a, v: 1 }]\nplot:\n- mark: barX\n  data: { from: t }\n  \
         x: v\n  y: region\n  sort: { y: -x, limit: 2 }\n",
        0,
    );
    assert_eq!(
        emitted,
        "SELECT * FROM (SELECT * FROM (SELECT * FROM \"t\") AS _o \
         ORDER BY \"v\" DESC, \"region\" ASC) AS _l LIMIT 2",
        "the pass-through rows, ranked and truncated, with no GROUP BY \
         invented for them"
    );
}

/// **The placement claim.** A brush must scope the rows the totals are built
/// from, and then the top n is taken of THAT — not the top n of everything,
/// filtered afterwards.
///
/// Read off `observable-latency.yaml`, whose ranked `barX` subscribes to a
/// selection its `params:` block DECLARES — `sorted-bars.yaml`'s `$query` is
/// created by an `as:` binding only, which no `filterBy` threads today
/// (`emit::plan_for_mark` calls that out as a pre-existing limitation), so it
/// cannot show this. The two placements are distinguishable in the text: the
/// predicate must land before the `GROUP BY`, which is inside both the
/// `ORDER BY` and the `LIMIT`.
#[test]
fn a_selection_reaches_past_the_limit_to_the_rows_being_aggregated() {
    let spec = parse_spec(
        include_str!("../../brightfield-spec/vendor/mosaic-specs/yaml/observable-latency.yaml"),
        Format::Yaml,
    )
    .expect("spec parses")
    .spec;
    let selections = vec![(
        "filter".to_string(),
        vec![(
            // The raster's plot, not the bar's own — a crossfilter selection
            // self-excludes, and a contributor from the same plot would
            // compile away and leave nothing to place.
            "root/vconcat[0]/plot[0]".to_string(),
            Predicate::Expr("\"time\" > 0".to_string()),
        )],
    )];
    let emitted = emit_query(&spec, 2, None, Some(&selections))
        .expect("mark emits")
        .sql;
    let predicate = emitted
        .find("\"time\" > 0")
        .expect("the selection predicate is in the query");
    let group_by = emitted.find("GROUP BY").expect("the bar aggregates");
    let limit = emitted.rfind("LIMIT").expect("the limit survives");
    assert!(
        predicate < group_by,
        "the selection must scope the rows the totals are built from: {emitted}"
    );
    assert!(
        group_by < limit,
        "and the limit must still be the outermost operator: {emitted}"
    );
}
