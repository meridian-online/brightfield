//! A preview connection kept open while a step is being made.
//!
//! The table is loaded once and each read runs the draft alone, on the terms
//! the cold reads hold to: the connection is shut and locked before a draft's
//! text runs, and a second file is read before that.

use std::path::{Path, PathBuf};

use brightfield_engine::{
    FileReader, PreviewOp, PreviewRefusal, PreviewStop, RecordBatch, WarmPreview,
    PREVIEW_FILE_LIMIT_BYTES,
};

/// The file with a missing `house_age`, and columns whose names need quotes.
const WITH_NULLS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../brightfield-shell/tests/data/with_nulls.csv"
);

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-warm-preview-{name}-{nanos}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("the file writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn model(sql: &str) -> PreviewOp {
    PreviewOp::Model(sql.to_owned())
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

fn open(data: &Path, upstream: &[(String, PreviewOp)]) -> WarmPreview {
    WarmPreview::open(
        data,
        FileReader::Csv,
        "t",
        upstream,
        PREVIEW_FILE_LIMIT_BYTES,
    )
    .expect("the preview opens")
}

#[test]
fn the_table_is_loaded_once_and_each_read_runs_only_the_draft() {
    let dir = TempDir::new("once");
    let data = dir.file("t.csv", "a,c\n1,x\n2,y\n");
    let mut warm = open(&data, &[]);
    let stop = PreviewStop::default();

    let hidden = model("CREATE OR REPLACE TABLE \"d\" AS\nSELECT * EXCLUDE (c)\nFROM t;\n");
    let page = warm
        .read("d", Some("t"), &hidden, 10, &stop)
        .expect("the first draft reads");
    assert_eq!(columns(&page.batches), ["a"]);
    assert_eq!((page.rows, page.rows_in), (2, 2));

    // The file changes under the preview; the table it holds does not.
    std::fs::write(&data, "a,c\n1,x\n2,y\n3,z\n4,w\n").expect("the file is rewritten");
    let kept = model("CREATE OR REPLACE TABLE \"d\" AS\nSELECT *\nFROM t\nWHERE a > 1;\n");
    let page = warm
        .read("d", Some("t"), &kept, 10, &stop)
        .expect("the second draft reads");
    assert_eq!(columns(&page.batches), ["a", "c"]);
    assert_eq!((page.rows, page.rows_in), (1, 2));

    // A draft under another name leaves no table of the last one behind.
    let other = model("CREATE OR REPLACE TABLE \"e\" AS\nSELECT *\nFROM t;\n");
    warm.read("e", Some("t"), &other, 10, &stop)
        .expect("the renamed draft reads");
    let reads_d = model("CREATE OR REPLACE TABLE \"f\" AS\nSELECT *\nFROM d;\n");
    let refused = warm
        .read("f", Some("t"), &reads_d, 10, &stop)
        .expect_err("the last draft's table is gone");
    assert!(engine(refused).contains('d'));
}

#[test]
fn a_read_reports_the_rows_the_rows_it_read_and_each_columns_missing_count() {
    let mut warm = open(Path::new(WITH_NULLS), &[]);
    let stop = PreviewStop::default();

    let as_is = model("CREATE OR REPLACE TABLE \"d\" AS\nSELECT *\nFROM t;\n");
    let page = warm
        .read("d", Some("t"), &as_is, 10, &stop)
        .expect("the table reads");
    let missing = |page: &brightfield_engine::WarmPage, column: &str| {
        page.missing
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, n)| *n)
    };
    assert_eq!(missing(&page, "house_age"), Some(1));
    let names: Vec<&str> = page.missing.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["city", "house_age", "order", "median income"]);

    let filled = model(
        "CREATE OR REPLACE TABLE \"d\" AS\nSELECT * REPLACE (coalesce(house_age, 0) AS house_age)\nFROM t;\n",
    );
    let page = warm
        .read("d", Some("t"), &filled, 10, &stop)
        .expect("the fill reads");
    assert_eq!((page.rows, page.rows_in), (3, 3));
    assert_eq!(rows(&page.batches), 3);
    assert_eq!(missing(&page, "house_age"), Some(0));
    assert_eq!(missing(&page, "median income"), Some(0));

    // The page is the first rows and the count is the table's.
    let page = warm
        .read("d", Some("t"), &as_is, 2, &stop)
        .expect("a short page reads");
    assert_eq!(rows(&page.batches), 2);
    assert_eq!(page.rows, 3);
}

#[test]
fn a_second_file_is_read_before_access_is_shut() {
    let dir = TempDir::new("second");
    let data = dir.file("t.csv", "a,c\n1,x\n2,y\n");
    let other = dir.file("u.csv", "a,n\n1,10\n2,20\n");
    let upstream = [
        (
            "u".to_owned(),
            PreviewOp::ReadFile {
                path: other,
                reader: FileReader::Csv,
            },
        ),
        ("big".to_owned(), PreviewOp::Where("n > 10".to_owned())),
    ];
    let mut warm = open(&data, &upstream);
    let joined = model("CREATE OR REPLACE TABLE \"j\" AS\nSELECT *\nFROM t\nJOIN big USING (a);\n");
    let page = warm
        .read("j", Some("t"), &joined, 10, &PreviewStop::default())
        .expect("the join reads");
    assert_eq!(columns(&page.batches), ["a", "c", "n"]);
    assert_eq!((page.rows, page.rows_in), (1, 2));

    // A second file over the limit is refused as the data file is.
    let refused = WarmPreview::open(&data, FileReader::Csv, "t", &upstream[..1], 13)
        .expect_err("the second file is over the limit");
    assert!(
        matches!(refused, PreviewRefusal::TooLarge { .. }),
        "{refused:?}"
    );
}

#[test]
fn a_draft_cannot_reach_outside_the_preview() {
    let dir = TempDir::new("outside");
    let data = dir.file("t.csv", "a,c\n1,x\n2,y\n");
    let mut warm = open(&data, &[]);
    let stop = PreviewStop::default();

    let out = dir.path().join("x.csv");
    let copy = model(&format!(
        "CREATE OR REPLACE TABLE \"d\" AS SELECT * FROM t;\nCOPY (SELECT 1) TO '{}';\n",
        out.display()
    ));
    let refused = warm
        .read("d", Some("t"), &copy, 10, &stop)
        .expect_err("the draft wrote a file");
    assert!(engine(refused).contains("external access"));
    assert!(!out.exists(), "no file was written");

    let unlock = model(
        "SET enable_external_access = true;\nCREATE OR REPLACE TABLE \"d\" AS SELECT * FROM t;\n",
    );
    let refused = warm
        .read("d", Some("t"), &unlock, 10, &stop)
        .expect_err("the draft switched access back on");
    assert!(engine(refused).contains("locked"));

    // A draft that fails inside its transaction leaves the next one a clean start.
    let broken = model(
        "BEGIN TRANSACTION;\nCREATE OR REPLACE TABLE \"d\" AS\nSELECT *\nFROM t;\nALTER TABLE \"d\" RENAME COLUMN nope TO b;\nCOMMIT;\n",
    );
    let refused = warm
        .read("d", Some("t"), &broken, 10, &stop)
        .expect_err("there is no column nope");
    assert!(engine(refused).contains("nope"));
    let renamed = model(
        "BEGIN TRANSACTION;\nCREATE OR REPLACE TABLE \"d\" AS\nSELECT *\nFROM t;\nALTER TABLE \"d\" RENAME COLUMN a TO b;\nCOMMIT;\n",
    );
    let page = warm
        .read("d", Some("t"), &renamed, 10, &stop)
        .expect("the next draft reads");
    assert_eq!(columns(&page.batches), ["b", "c"]);

    // A read whose stop is already stopped ends as superseded.
    let stopped = PreviewStop::default();
    stopped.stop();
    let refused = warm
        .read("d", Some("t"), &renamed, 10, &stopped)
        .expect_err("the read was stopped");
    assert_eq!(refused, PreviewRefusal::Superseded);
    let page = warm
        .read("d", Some("t"), &renamed, 10, &stop)
        .expect("a read after a stopped one reads");
    assert_eq!(page.rows, 2);
}

#[test]
fn a_draft_named_for_a_table_it_stands_on_is_refused_and_drops_no_table() {
    let dir = TempDir::new("named");
    let data = dir.file("t.csv", "a,c\n1,x\n2,y\n");
    let upstream = [("kept".to_owned(), PreviewOp::Where("a > 1".to_owned()))];
    let mut warm = open(&data, &upstream);
    let stop = PreviewStop::default();
    for name in ["t", "T", "Kept"] {
        let as_is = model(&format!(
            "CREATE OR REPLACE TABLE \"{name}\" AS\nSELECT *\nFROM t;\n"
        ));
        let refused = warm
            .read(name, Some("t"), &as_is, 10, &stop)
            .expect_err("the name is a table the open made");
        assert!(engine(refused).contains("cannot be previewed under that name"));
    }
    // Both tables are still there for a draft under a free name.
    let both = model(
        "CREATE OR REPLACE TABLE \"d\" AS\nSELECT * FROM t\nUNION ALL\nSELECT * FROM kept;\n",
    );
    let page = warm
        .read("d", Some("t"), &both, 10, &stop)
        .expect("a free name reads");
    assert_eq!((page.rows, page.rows_in), (3, 2));
}

#[test]
fn an_open_that_is_stopped_ends_as_superseded() {
    let dir = TempDir::new("stopped-open");
    let data = dir.file("t.csv", "a,c\n1,x\n2,y\n");
    let stop = PreviewStop::default();
    stop.stop();
    let refused = WarmPreview::open_stoppable(
        &data,
        FileReader::Csv,
        "t",
        &[],
        PREVIEW_FILE_LIMIT_BYTES,
        &stop,
    )
    .expect_err("the open was stopped before it began");
    assert_eq!(refused, PreviewRefusal::Superseded);
}
