//! The arithmetic oracle for a positional aggregate over a band channel.
//!
//! `x: {sum: col}` with a categorical `y` on a `barX` is lowered to a
//! `GROUP BY` on the band column and one aggregate call on the value column.
//! A SQL-TEXT assertion cannot tell a correct grouping from a plausible-but-
//! wrong one — grouping by the value column instead, aggregating the band,
//! double-counting a category that appears in two blocks of rows — so every
//! case below **executes** the emitted query against the bundled DuckDB and
//! compares the bars it produces against sums computed by hand from the same
//! five rows.
//!
//! The rows are deliberately interleaved (`b, a, b, a, c`) rather than sorted
//! by category. A lowerer that grouped correctly and one that merely
//! de-duplicated adjacent rows agree on sorted input and disagree here.

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::emit::{emit_all_queries, emit_query, emit_sources};
use brightfield_sql::ir::{Predicate, SelectionPredicate};
use duckdb::Connection;

/// One output bar: `(band value, aggregate)`.
type Bar = (String, f64);

/// The five rows every case below aggregates.
///
/// Hand-computed answers, in the ascending band order the lowerer emits:
///
/// | cat | rows      | sum | avg | min | max | count |
/// |-----|-----------|-----|-----|-----|-----|-------|
/// | a   | 10, 1     |  11 | 5.5 |   1 |  10 |     2 |
/// | b   | 3, 4      |   7 | 3.5 |   3 |   4 |     2 |
/// | c   | 7         |   7 | 7.0 |   7 |   7 |     1 |
const ROWS: &str = "  - { cat: b, v: 3 }\n  \
                    - { cat: a, v: 10 }\n  \
                    - { cat: b, v: 4 }\n  \
                    - { cat: a, v: 1 }\n  \
                    - { cat: c, v: 7 }";

/// A `barX` binding `agg` on its value axis (`x`) over the band `cat` (`y`).
fn bar_x_spec(agg: &str) -> String {
    format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barX\n  data: {{ from: t }}\n  \
         x: {{ {agg} }}\n  y: cat\n  fill: steelblue\n"
    )
}

/// Emit mark 0 of `spec_yaml`, run it against an in-memory DuckDB, and return
/// its `(band, aggregate)` rows in emitted order.
fn bars(spec_yaml: &str) -> Vec<Bar> {
    let spec = parse_spec(spec_yaml, Format::Yaml)
        .expect("spec parses")
        .spec;
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let query = emit_all_queries(&spec, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    run(&conn, &query.sql)
}

/// Execute `sql` and read it as `(band, aggregate)` rows.
fn run(conn: &Connection, sql: &str) -> Vec<Bar> {
    let mut stmt = conn.prepare(sql).expect("query prepares");
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query runs")
        .collect::<Result<Vec<Bar>, _>>()
        .expect("rows read")
}

/// Compare against the hand-computed answer, tolerating only float
/// representation.
fn assert_bars(found: &[Bar], expected: &[(&str, f64)], case: &str) {
    assert_eq!(
        found.len(),
        expected.len(),
        "{case}: bar COUNT differs — found {found:?}, expected {expected:?}"
    );
    for (i, ((fband, fval), (eband, eval))) in found.iter().zip(expected).enumerate() {
        assert_eq!(fband, eband, "{case}: bar {i} band");
        assert!(
            (fval - eval).abs() < 1e-9,
            "{case}: bar {i} ({eband}) is {fval}, hand-computed answer is {eval}"
        );
    }
}

/// **AC1.** The commonest chart device in the corpus: one bar per category,
/// its length the total of a value column.
///
/// Five interleaved rows collapse to three bars carrying 11, 7 and 7 — the
/// hand-computed totals. A `SELECT *` (what this mark emitted before the lift)
/// returns five rows and fails on the count alone.
#[test]
fn a_positional_sum_totals_each_category() {
    let found = bars(&bar_x_spec("sum: v"));
    assert_bars(&found, &[("a", 11.0), ("b", 7.0), ("c", 7.0)], "sum");
}

/// **AC2.** Every aggregate the vocabulary recognises lifts the same way, and
/// each returns its OWN number.
///
/// Run together rather than as four tests because the point is the contrast:
/// `avg` and `min` disagree on every bar, so a lowerer that silently emitted
/// one function for all of them cannot pass this and pass the `sum` case too.
/// `count` takes no column and lands under the reserved `__bf_count` name;
/// the others alias to their source column.
#[test]
fn every_recognised_aggregate_lifts_over_the_same_band() {
    assert_bars(
        &bars(&bar_x_spec("avg: v")),
        &[("a", 5.5), ("b", 3.5), ("c", 7.0)],
        "avg",
    );
    assert_bars(
        &bars(&bar_x_spec("mean: v")),
        &[("a", 5.5), ("b", 3.5), ("c", 7.0)],
        "mean is an accepted alias for avg",
    );
    assert_bars(
        &bars(&bar_x_spec("min: v")),
        &[("a", 1.0), ("b", 3.0), ("c", 7.0)],
        "min",
    );
    assert_bars(
        &bars(&bar_x_spec("max: v")),
        &[("a", 10.0), ("b", 4.0), ("c", 7.0)],
        "max",
    );
    assert_bars(
        &bars(&bar_x_spec("count:")),
        &[("a", 2.0), ("b", 2.0), ("c", 1.0)],
        "count",
    );
}

/// The transpose. A `barY` runs its bars up `y`, so the aggregate belongs
/// there and the band on `x` — the same three bars, read the other way.
///
/// Pinned because the orientation comes from the mark KIND rather than from
/// which channel carries the map: a lowerer that looked for the aggregate on
/// `x` only would return `SELECT *` here and draw nothing.
#[test]
fn a_bar_y_aggregates_up_its_value_axis() {
    let spec = format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barY\n  data: {{ from: t }}\n  \
         x: cat\n  y: {{ sum: v }}\n  fill: steelblue\n"
    );
    assert_bars(&bars(&spec), &[("a", 11.0), ("b", 7.0), ("c", 7.0)], "barY");
}

/// A bar whose value channel is a plain column is PRE-AGGREGATED and must
/// keep passing its rows through untouched.
///
/// This is the shape `examples/bars.yaml` and `examples/bars-x.yaml` ship, and
/// the reason the lift is resolved from the channel's contents rather than
/// from the mark kind: grouping every bar chart would silently collapse a
/// table that was already one row per bar.
#[test]
fn a_pre_aggregated_bar_still_passes_its_rows_through() {
    let spec = format!(
        "data:\n  t:\n{ROWS}\nplot:\n- mark: barX\n  data: {{ from: t }}\n  \
         x: v\n  y: cat\n"
    );
    let parsed = parse_spec(&spec, Format::Yaml).expect("spec parses");
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&parsed.spec, None)
        .expect("sources emit")
        .statements
    {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let query = emit_all_queries(&parsed.spec, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    let mut stmt = conn.prepare(&query.sql).expect("query prepares");
    let count = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query runs")
        .count();
    assert_eq!(
        count, 5,
        "a bar with a plain value column is already one row per bar and must \
         not be grouped: got {count} rows from 5"
    );
}

/// **AC3, the half that is provable in SQL.** A selection clause lands
/// *inside* the aggregation, so the bars are recomputed over the filtered
/// rows rather than the totals being filtered after the fact.
///
/// `v > 2` drops `a`'s 1 and keeps the other four rows, so the totals move to
/// 10, 7 and 7. Applying the same clause AFTER the GROUP BY would leave 11, 7
/// and 7 and drop nothing, because every total already exceeds 2 — which is
/// exactly why this case is asserted on the numbers and not on the SQL text.
///
/// The threshold is chosen so the ungrouped answer differs in LENGTH as well
/// as in value: four rows survive `v > 2` and three bars come out of them. At
/// `v > 3` the two answers coincide row for row, and the case passed against
/// `SimpleLowerer` — a guard that agrees with the code it is meant to
/// contradict.
///
/// What this does NOT cover is the readout itself: the string a reader sees is
/// `brightfield_shell::chart_item::PREDICATE_READOUT`, and asserting that the
/// shown text equals the pushed clause belongs with it.
#[test]
fn a_selection_is_applied_before_the_grouping() {
    let spec_yaml = format!(
        "data:\n  t:\n{ROWS}\nparams:\n  brush: {{ select: intersect }}\n\
         plot:\n- mark: barX\n  data: {{ from: t, filterBy: $brush }}\n  \
         x: {{ sum: v }}\n  y: cat\n  fill: steelblue\n"
    );
    let spec = parse_spec(&spec_yaml, Format::Yaml)
        .expect("spec parses")
        .spec;
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let brushed: Vec<SelectionPredicate> = vec![(
        "brush".to_string(),
        vec![("t".to_string(), Predicate::Expr("\"v\" > 2".to_string()))],
    )];
    let query = emit_query(&spec, 0, None, Some(&brushed)).expect("mark lowers");
    assert_bars(
        &run(&conn, &query.sql),
        &[("a", 10.0), ("b", 7.0), ("c", 7.0)],
        "brushed",
    );
}

/// The output columns, read off the executed statement rather than off the
/// SQL text: the band under its own name (which is what the renderer binds the
/// band axis to, and what a category selection pushes on), and the aggregate
/// aliased to its SOURCE column — the convention hexbin and cell already use
/// and the one `ChannelMap::from_mark` resolves a positional aggregate
/// through.
#[test]
fn the_emitted_columns_carry_the_band_and_the_source_column() {
    let spec = parse_spec(&bar_x_spec("sum: v"), Format::Yaml)
        .expect("spec parses")
        .spec;
    let conn = Connection::open_in_memory().expect("duckdb opens");
    for ddl in emit_sources(&spec, None).expect("sources emit").statements {
        conn.execute_batch(&ddl.sql).expect("ddl runs");
    }
    let query = emit_all_queries(&spec, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    // `DESCRIBE` asks DuckDB itself what the statement produces, so the names
    // asserted here are the ones that reach the Arrow batch rather than the
    // ones the emitter believes it wrote.
    let mut stmt = conn
        .prepare(&format!("DESCRIBE {}", query.sql))
        .expect("describe prepares");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("describe runs")
        .collect::<Result<_, _>>()
        .expect("names read");
    assert_eq!(
        names,
        vec!["cat", "v"],
        "the statement's output columns are the renderer's contract"
    );

    let counted = parse_spec(&bar_x_spec("count:"), Format::Yaml)
        .expect("spec parses")
        .spec;
    let query = emit_all_queries(&counted, None)
        .into_iter()
        .next()
        .expect("one mark")
        .expect("mark lowers");
    let mut stmt = conn
        .prepare(&format!("DESCRIBE {}", query.sql))
        .expect("describe prepares");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("describe runs")
        .collect::<Result<_, _>>()
        .expect("names read");
    assert_eq!(
        names,
        vec!["cat", "__bf_count"],
        "a count takes no source column, so it lands under the reserved name"
    );
}
