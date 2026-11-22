//! A generated step previewed by running arc's own model.
//!
//! A model may hold several statements, so it is not held to one read-only
//! query. These tests hold it to the preview's own database instead: it runs
//! once the connection is shut and locked, the next step reads the table it
//! makes, and a model that makes no table of its step's name is refused.

use std::path::{Path, PathBuf};

use brightfield_engine::{preview_first_page, FileReader, PreviewOp, PreviewRefusal, RecordBatch};

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-preview-model-{name}-{nanos}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A CSV of two rows, columns `a` and `c`.
    fn csv(&self) -> PathBuf {
        let path = self.0.join("t.csv");
        std::fs::write(&path, "a,c\n1,x\n2,y\n").expect("the CSV writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn model(name: &str, sql: &str) -> (String, PreviewOp) {
    (name.to_owned(), PreviewOp::Model(sql.to_owned()))
}

fn columns(batches: &[RecordBatch]) -> Vec<String> {
    batches.first().map_or_else(Vec::new, |batch| {
        batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect()
    })
}

fn rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

fn engine(refused: PreviewRefusal) -> String {
    match refused {
        PreviewRefusal::Engine(msg) => msg,
        other => panic!("expected an engine refusal, got {other:?}"),
    }
}

/// The model arc writes for `rename-column`: the table made, then altered,
/// inside one transaction.
const RENAMED: &str = "BEGIN TRANSACTION;\nCREATE OR REPLACE TABLE \"renamed\" AS\nSELECT *\nFROM t;\nALTER TABLE \"renamed\" RENAME COLUMN a TO b;\nCOMMIT;\n";

#[test]
fn a_model_makes_its_step_and_the_next_step_reads_it() {
    let dir = TempDir::new("chain");
    let chain = [
        model("renamed", RENAMED),
        ("kept".to_owned(), PreviewOp::Where("b > 1".to_owned())),
    ];
    let page = preview_first_page(&dir.csv(), FileReader::Csv, "t", &chain, 10)
        .expect("the chain previews");
    assert_eq!(rows(&page), 1);
    assert_eq!(columns(&page), ["b", "c"]);

    // The model's own page, read in the order its rows were made.
    let page = preview_first_page(&dir.csv(), FileReader::Csv, "t", &chain[..1], 10)
        .expect("the model previews");
    assert_eq!(rows(&page), 2);
    assert_eq!(columns(&page), ["b", "c"]);
}

#[test]
fn a_model_cannot_reach_outside_the_preview() {
    let dir = TempDir::new("outside");
    let out = dir.path().join("x.csv");
    let copy = format!(
        "CREATE OR REPLACE TABLE \"m\" AS SELECT * FROM t;\nCOPY (SELECT 1) TO '{}';\n",
        out.display()
    );
    let refused = preview_first_page(&dir.csv(), FileReader::Csv, "t", &[model("m", &copy)], 10)
        .expect_err("the model wrote a file");
    let msg = engine(refused);
    assert!(msg.contains("external access"), "{msg}");
    assert!(!out.exists(), "no file was written");

    let unlock =
        "SET enable_external_access = true;\nCREATE OR REPLACE TABLE \"m\" AS SELECT * FROM t;\n";
    let refused = preview_first_page(&dir.csv(), FileReader::Csv, "t", &[model("m", unlock)], 10)
        .expect_err("the model switched access back on");
    let msg = engine(refused);
    assert!(msg.contains("locked"), "{msg}");

    let read = "CREATE OR REPLACE TABLE \"m\" AS SELECT * FROM read_csv('/etc/hosts');\n";
    let refused = preview_first_page(&dir.csv(), FileReader::Csv, "t", &[model("m", read)], 10)
        .expect_err("the model read another file");
    assert!(engine(refused).contains("external access"));
}

#[test]
fn a_model_that_makes_no_table_of_its_name_is_refused() {
    let dir = TempDir::new("unnamed");
    let refused = preview_first_page(
        &dir.csv(),
        FileReader::Csv,
        "t",
        &[model(
            "named",
            "CREATE OR REPLACE TABLE \"other\" AS SELECT 1;",
        )],
        10,
    )
    .expect_err("the model made another table");
    assert_eq!(
        refused,
        PreviewRefusal::Engine("named: arc's model made no table of its name".to_owned())
    );
}

#[test]
fn a_step_that_is_not_previewed_refuses_in_its_own_words() {
    let dir = TempDir::new("not-previewed");
    let why =
        "Brightfield previews a local CSV or Parquet file; Run reads https://example.com/x.csv.";
    let chain = [
        ("remote".to_owned(), PreviewOp::NotPreviewed(why.to_owned())),
        ("kept".to_owned(), PreviewOp::Where("a > 1".to_owned())),
    ];
    let refused = preview_first_page(&dir.csv(), FileReader::Csv, "t", &chain, 10)
        .expect_err("the step is not previewed");
    assert_eq!(refused, PreviewRefusal::Engine(why.to_owned()));
}
