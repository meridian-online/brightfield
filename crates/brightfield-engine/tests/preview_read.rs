//! The first page of a generated step's rows, read on a connection of its own.
//!
//! A step arc generated and nobody has run has no table to read, so the grid
//! previews it: the data file is loaded into a fresh in-memory database, that
//! database is shut to anything outside itself, and only then do the steps'
//! conditions and orders run, each as its own table, the last read one page
//! deep. These tests hold the page to the step's effect, the connection to
//! its lock, and the size limit to being checked before anything is read.

use std::path::{Path, PathBuf};

use brightfield_engine::{
    preview_first_page, preview_first_page_with_limit, FileReader, PreviewOp, PreviewRefusal,
    RecordBatch,
};
use duckdb::arrow::array::{Array, Float64Array};

/// The housing file the shell's first start opens.
const HOUSING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../brightfield-shell/assets/starts/california_housing.parquet"
);

/// The table the housing file's load makes.
const ROOT: &str = "california_housing";

fn housing() -> PathBuf {
    PathBuf::from(HOUSING)
}

/// The values of a `DOUBLE` column across `batches`.
fn doubles(batches: &[RecordBatch], column: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for batch in batches {
        let index = batch
            .schema()
            .index_of(column)
            .unwrap_or_else(|_| panic!("no column {column}"));
        let values = batch
            .column(index)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("a DOUBLE column");
        out.extend((0..values.len()).map(|i| values.value(i)));
    }
    out
}

fn rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

fn filter(name: &str, condition: &str) -> (String, PreviewOp) {
    (name.to_owned(), PreviewOp::Where(condition.to_owned()))
}

fn sort(name: &str, order: &str) -> (String, PreviewOp) {
    (name.to_owned(), PreviewOp::OrderBy(order.to_owned()))
}

/// The largest `median_income` in the file, read on a connection of the
/// test's own.
fn top_income() -> f64 {
    let conn = duckdb::Connection::open_in_memory().expect("a connection");
    conn.query_row(
        &format!("SELECT max(median_income) FROM read_parquet('{HOUSING}')"),
        [],
        |r| r.get(0),
    )
    .expect("the max reads")
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-preview-read-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_filter_previews_its_first_page() {
    let page = preview_first_page(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[filter("kept", "house_age > 28")],
        50,
    )
    .expect("the preview reads");

    // 8,831 rows keep `house_age > 28`, so a page of 50 is full.
    assert_eq!(rows(&page), 50);
    let ages = doubles(&page, "house_age");
    assert!(ages.iter().all(|a| *a > 28.0), "{ages:?}");
}

#[test]
fn a_sort_previews_in_its_own_order() {
    let page = preview_first_page(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[sort("sorted", "median_income DESC")],
        40,
    )
    .expect("the preview reads");

    let income = doubles(&page, "median_income");
    assert_eq!(income.len(), 40);
    assert!(
        income.windows(2).all(|w| w[0] >= w[1]),
        "the page is not in the step's order: {income:?}"
    );
    assert_eq!(income[0], top_income());
}

#[test]
fn a_chain_previews_through_its_chain() {
    let page = preview_first_page(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[
            filter("kept", "house_age > 28"),
            sort("sorted", "median_income DESC"),
        ],
        30,
    )
    .expect("the preview reads");

    let ages = doubles(&page, "house_age");
    let income = doubles(&page, "median_income");
    assert_eq!(ages.len(), 30);
    assert!(ages.iter().all(|a| *a > 28.0), "{ages:?}");
    assert!(
        income.windows(2).all(|w| w[0] >= w[1]),
        "the chain's sort did not order the page: {income:?}"
    );
}

#[test]
fn a_sort_whose_order_ends_in_a_comment_still_previews_one_page() {
    let page = preview_first_page(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[sort("sorted", "median_income DESC -- richest first")],
        10,
    )
    .expect("the preview reads");

    let income = doubles(&page, "median_income");
    assert_eq!(income.len(), 10, "the comment hid the LIMIT");
    assert!(income.windows(2).all(|w| w[0] >= w[1]), "{income:?}");
}

#[test]
fn a_condition_that_reads_another_file_fails() {
    let refused = preview_first_page(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[filter(
            "kept",
            "house_age > (SELECT count(*) FROM read_csv('/etc/hosts'))",
        )],
        10,
    )
    .expect_err("the condition read another file");

    match refused {
        PreviewRefusal::Engine(msg) => {
            assert!(msg.contains("external access"), "{msg}");
            assert!(
                msg.contains("/etc/hosts"),
                "DuckDB's own words are kept: {msg}"
            );
        }
        other => panic!("expected an engine refusal, got {other:?}"),
    }
}

#[test]
fn a_file_over_the_limit_is_not_read() {
    let bytes = std::fs::metadata(HOUSING).expect("the file is there").len();
    let refused = preview_first_page_with_limit(
        &housing(),
        FileReader::Parquet,
        ROOT,
        &[filter("kept", "house_age > 28")],
        10,
        1,
    )
    .expect_err("a file over the limit was read");

    assert_eq!(refused, PreviewRefusal::TooLarge { bytes });
}

#[test]
fn a_path_holding_a_quote_still_previews() {
    let root = TempDir::new("quote");
    let folder = root.path().join("it's here");
    std::fs::create_dir_all(&folder).expect("the folder");
    let data = folder.join("california_housing.parquet");
    std::fs::copy(HOUSING, &data).expect("the file copies");

    let page = preview_first_page(
        &data,
        FileReader::Parquet,
        ROOT,
        &[filter("kept", "house_age > 28")],
        5,
    )
    .expect("the preview reads");

    assert_eq!(rows(&page), 5);
}

#[test]
fn a_csv_is_read_with_the_csv_reader() {
    let root = TempDir::new("csv");
    let data = root.path().join("ages.csv");
    std::fs::write(&data, "name,age\na,10\nb,40\nc,30\n").expect("the CSV writes");
    assert_eq!(FileReader::for_path(&data), Some(FileReader::Csv));
    assert_eq!(
        FileReader::for_path(Path::new("x.PARQUET")),
        Some(FileReader::Parquet)
    );

    let page = preview_first_page(
        &data,
        FileReader::Csv,
        "ages",
        &[sort("by_age", "age DESC")],
        10,
    )
    .expect("the preview reads");

    assert_eq!(rows(&page), 3);
    let schema = page[0].schema();
    assert_eq!(schema.field(0).name(), "name");
}
