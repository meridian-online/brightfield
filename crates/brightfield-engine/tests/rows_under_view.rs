//! A step's rows read under a view: a condition and an order together.
//!
//! **A view is an argument to the read, not state.** The count and the window
//! take a `RowsView`; nothing about the session changes. The order leads the
//! sort and `ORDER BY ALL` follows it, so rows that tie on the order still page
//! in one fixed sequence. The columns a condition or an order names, and the
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
    for offset in [0, 500, 3_000] {
        let a = session
            .execute_step_rows_window_view(0, offset, 500, RowsAudience::Reader, &v)
            .unwrap();
        let b = session
            .execute_step_rows_window_where(0, offset, 500, RowsAudience::Reader, "house_age > 40")
            .unwrap();
        assert_eq!(a, b, "the page at {offset} is the same under both reads");
    }
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
