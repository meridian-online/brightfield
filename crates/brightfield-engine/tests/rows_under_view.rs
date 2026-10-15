//! A step's rows read under a view: a condition and an order together.
//!
//! **A view is an argument to the read, not state.** The count and the window
//! take a `RowsView`; nothing about the session changes. The order leads the
//! sort and the step's columns follow it, so rows that tie on the order still
//! page in one fixed sequence. The columns a condition or an order names, and the
//! literal a cell equals, are read through the same session, which is what a
//! grid's foot band builds an operation from.
//!
//! Every count below was taken with the DuckDB CLI over the same file.

use brightfield_engine::error::{ConditionRefusal, EngineError};
use brightfield_engine::{CellLiteral, Engine, RecordBatch, RowsAudience, RowsView, Session};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use duckdb::arrow::array::{Array, Float64Array};
use std::collections::HashSet;

/// The housing file the shell's first start opens.
const HOUSING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../brightfield-shell/assets/starts/california_housing.parquet"
);

const ALL_ROWS: u64 = 16_640;

/// A session over one `dot` mark drawn straight from `file`. Mark `0` is the
/// step every test reads.
fn session_over(file: &str, x: &str, y: &str) -> Session {
    let spec = format!(
        "data:\n  t: {{ file: \"{file}\" }}\n\
         plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: {x}\n    y: {y}\n"
    );
    let parsed = parse_spec(&spec, Format::Yaml).expect("the spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("the spec analyses");
    Engine::new()
        .load_spec(parsed.spec, analysis, None)
        .expect("the spec loads")
        .session
}

fn session() -> Session {
    session_over(HOUSING, "median_income", "house_age")
}

fn view(condition: Option<&str>, order: Option<&str>) -> RowsView {
    RowsView {
        condition: condition.map(str::to_string),
        order: order.map(str::to_string),
    }
}

fn rows(batches: &[RecordBatch]) -> u64 {
    batches.iter().map(|b| b.num_rows() as u64).sum()
}

fn doubles(batches: &[RecordBatch], column: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for batch in batches {
        let index = batch.schema().index_of(column).expect("the column exists");
        let values = batch
            .column(index)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("a DOUBLE");
        out.extend((0..values.len()).map(|i| values.value(i)));
    }
    out
}

fn pages(session: &Session, view: &RowsView, limit: u64) -> Vec<RecordBatch> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let page = session
            .execute_step_rows_window_view(0, offset, limit, RowsAudience::Reader, view)
            .expect("the page reads");
        let held = rows(&page);
        all.extend(page);
        if held < limit {
            return all;
        }
        offset += limit;
    }
}

#[test]
fn a_view_with_only_a_condition_reads_what_the_where_read_reads() {
    let session = session();
    let v = view(Some("house_age > 40"), None);
    assert_eq!(
        session
            .step_rows_count_view(0, RowsAudience::Reader, &v)
            .unwrap(),
        3_402
    );
    // The `_where` read delegates to the view read, so it cannot vouch for
    // it. These are checked against the file instead: every row on every page
    // meets the condition, and the pages are the unconditioned pages with the
    // rows that fail it removed, in the same total order.
    let kept = pages(&session, &v, 500);
    assert_eq!(rows(&kept), 3_402);
    assert!(doubles(&kept, "house_age").iter().all(|a| *a > 40.0));
    let everything = pages(&session, &view(None, None), 1000);
    let cols = ["longitude", "latitude", "house_age", "median_income"];
    let tuples = |b: &[RecordBatch]| {
        let c: Vec<Vec<f64>> = cols.iter().map(|n| doubles(b, n)).collect();
        (0..c[0].len())
            .map(|i| c.iter().map(|col| col[i].to_bits()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let ages = doubles(&everything, "house_age");
    let expected: Vec<_> = tuples(&everything)
        .into_iter()
        .zip(ages)
        .filter(|(_, age)| *age > 40.0)
        .map(|(t, _)| t)
        .collect();
    assert_eq!(tuples(&kept), expected);
}

#[test]
fn an_order_sorts_every_page_and_ties_do_not_tear() {
    let session = session();
    let v = view(None, Some("house_age DESC"));
    let all = pages(&session, &v, 1000);
    assert_eq!(rows(&all), ALL_ROWS);
    let ages = doubles(&all, "house_age");
    assert!(
        ages.windows(2).all(|w| w[0] >= w[1]),
        "house_age is non-increasing"
    );
    let mut seen = HashSet::new();
    let (lon, lat, age, inc) = (
        doubles(&all, "longitude"),
        doubles(&all, "latitude"),
        doubles(&all, "house_age"),
        doubles(&all, "median_income"),
    );
    for i in 0..lon.len() {
        let key = (
            lon[i].to_bits(),
            lat[i].to_bits(),
            age[i].to_bits(),
            inc[i].to_bits(),
        );
        seen.insert(key);
    }
    let unfiltered = pages(&session, &view(None, None), 1000);
    let (lon, lat, age, inc) = (
        doubles(&unfiltered, "longitude"),
        doubles(&unfiltered, "latitude"),
        doubles(&unfiltered, "house_age"),
        doubles(&unfiltered, "median_income"),
    );
    let expected: HashSet<_> = (0..lon.len())
        .map(|i| {
            (
                lon[i].to_bits(),
                lat[i].to_bits(),
                age[i].to_bits(),
                inc[i].to_bits(),
            )
        })
        .collect();
    assert_eq!(
        seen, expected,
        "the ordered pages hold the same rows, none twice or lost"
    );
}

#[test]
fn a_condition_and_an_order_compose() {
    let session = session();
    let v = view(Some("house_age = 28"), Some("median_income ASC"));
    assert_eq!(
        session
            .step_rows_count_view(0, RowsAudience::Reader, &v)
            .unwrap(),
        375
    );
    let all = pages(&session, &v, 100);
    assert_eq!(rows(&all), 375);
    let income = doubles(&all, "median_income");
    assert!(income.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn an_order_with_a_second_statement_is_refused() {
    let session = session();
    let v = view(None, Some("house_age; DROP TABLE x"));
    for result in [
        session
            .step_rows_count_view(0, RowsAudience::Reader, &v)
            .map(|_| ()),
        session
            .execute_step_rows_window_view(0, 0, 10, RowsAudience::Reader, &v)
            .map(|_| ()),
    ] {
        assert!(
            matches!(
                result,
                Err(EngineError::OrderRefused {
                    refusal: ConditionRefusal::SecondStatement { .. },
                    ..
                })
            ),
            "{result:?}"
        );
    }
}

#[test]
fn the_columns_a_condition_names_are_listed() {
    let session = session();
    assert_eq!(
        session.condition_columns("house_age > 28").unwrap(),
        ["house_age"]
    );
    assert_eq!(
        session
            .condition_columns("house_age > 28 AND median_income < 3")
            .unwrap(),
        ["house_age", "median_income"]
    );
    assert!(session.condition_columns("1 = 1").unwrap().is_empty());
}

#[test]
fn the_columns_an_order_names_are_listed() {
    let session = session();
    assert_eq!(
        session
            .order_columns("median_income DESC, house_age")
            .unwrap(),
        ["median_income", "house_age"]
    );
}

#[test]
fn a_cells_literal_keeps_its_own_row() {
    let session = session();
    let v = view(Some("house_age > 40"), Some("median_income DESC"));
    for column in ["house_age", "median_income", "latitude"] {
        let CellLiteral::Value(literal) = session
            .cell_literal(0, RowsAudience::Reader, &v, 0, column)
            .unwrap()
        else {
            panic!("`{column}` is not NULL in row 0");
        };
        let kept = view(
            Some(&format!("house_age > 40 AND {column} = {literal}")),
            None,
        );
        let n = session
            .step_rows_count_view(0, RowsAudience::Reader, &kept)
            .unwrap();
        assert!(n >= 1, "`{column} = {literal}` keeps its own row");
    }
}

#[test]
fn literals_quote_strings_and_type_dates() {
    let path = std::env::temp_dir().join(format!("bf_literals_{}.csv", std::process::id()));
    std::fs::write(
        &path,
        "id,name,d,ts,n\n1,O'Brien,2024-01-31,2024-01-31 12:00:00,5\n2,,,,\n",
    )
    .unwrap();
    let session = session_over(path.to_str().unwrap(), "id", "n");
    let v = view(None, Some("id"));
    let at = |row, column| {
        session
            .cell_literal(0, RowsAudience::Reader, &v, row, column)
            .unwrap()
    };
    assert_eq!(at(0, "name"), CellLiteral::Value("'O''Brien'".into()));
    assert_eq!(at(0, "d"), CellLiteral::Value("DATE '2024-01-31'".into()));
    assert_eq!(
        at(0, "ts"),
        CellLiteral::Value("TIMESTAMP '2024-01-31 12:00:00'".into())
    );
    assert_eq!(at(0, "n"), CellLiteral::Value("5".into()));
    assert_eq!(at(1, "name"), CellLiteral::Null);
    assert_eq!(at(1, "d"), CellLiteral::Null);
    let _ = std::fs::remove_file(path);
}

/// A parquet file at a unique path, built by DuckDB from `select`.
fn parquet_of(name: &str, select: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("bf_{name}_{}.parquet", std::process::id()));
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "COPY ({select}) TO '{}' (FORMAT PARQUET)",
        path.display()
    ))
    .unwrap();
    path
}

#[test]
fn a_floats_literal_keeps_its_own_row_at_any_magnitude() {
    let path = parquet_of(
        "floats",
        "SELECT * FROM (VALUES \
           (1, 5.743562329073703e-10::DOUBLE, 3.272414146871332e-11::REAL), \
           (2, 7.314219491610719e-16::DOUBLE, 1.5e-30::REAL), \
           (3, 1e300::DOUBLE, 3.4e38::REAL), \
           (4, 0.1::DOUBLE, 0.1::REAL)) t(id, d, r)",
    );
    let session = session_over(path.to_str().unwrap(), "id", "d");
    let v = view(None, Some("id"));
    for row in 0..4 {
        for column in ["d", "r"] {
            let CellLiteral::Value(literal) = session
                .cell_literal(0, RowsAudience::Reader, &v, row, column)
                .unwrap()
            else {
                panic!("not NULL");
            };
            assert!(literal.len() < 40, "`{literal}` stays short");
            let n = session
                .step_rows_count_view(
                    0,
                    RowsAudience::Reader,
                    &view(Some(&format!("{column} = {literal}")), None),
                )
                .unwrap();
            assert!(n >= 1, "`{column} = {literal}` keeps row {row}");
        }
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_nanosecond_timestamp_keeps_its_precision() {
    let path = parquet_of(
        "ns",
        "SELECT 1 AS id, TIMESTAMP_NS '2024-01-31 12:00:00.123456789' AS ts",
    );
    let session = session_over(path.to_str().unwrap(), "id", "id");
    let v = view(None, None);
    let CellLiteral::Value(literal) = session
        .cell_literal(0, RowsAudience::Reader, &v, 0, "ts")
        .unwrap()
    else {
        panic!("not NULL");
    };
    assert!(literal.contains("123456789"), "{literal}");
    let n = session
        .step_rows_count_view(
            0,
            RowsAudience::Reader,
            &view(Some(&format!("ts = {literal}")), None),
        )
        .unwrap();
    assert_eq!(n, 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_row_past_the_end_is_not_a_null_cell() {
    let session = session();
    let v = view(Some("house_age = 28"), None);
    let result = session.cell_literal(0, RowsAudience::Reader, &v, 375, "house_age");
    assert!(
        matches!(result, Err(EngineError::NoSuchRow { row: 375 })),
        "{result:?}"
    );
}

/// **DuckDB's own words are keywords, reserved or not, whatever their case,
/// and a column name that is none of them is not.** The shell writes a column
/// bare only when it is not one.
#[test]
fn duckdbs_words_are_keywords_and_a_column_name_is_not() {
    let session = session();
    for word in ["order", "ORDER", "select", "where", "year"] {
        assert!(session.is_keyword(word).unwrap(), "{word}");
    }
    for word in ["house_age", "median_income", "median income"] {
        assert!(!session.is_keyword(word).unwrap(), "{word}");
    }
}
