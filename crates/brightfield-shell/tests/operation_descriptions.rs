//! arc's description of each operation, read whole, and a step recorded by
//! naming each table it reads by its role.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite,
//! which becomes `arc` under the runner variable, so every shape here is read
//! from arc at the pinned rev and not from a fixture shaped like it.

use std::path::{Path, PathBuf};

use brightfield_shell::operation::{
    self, OperationDescription, Parameter, SqlKind, StepRequest, ValueShape,
};
use brightfield_shell::run::Runner;

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// The 13 operations a window offers, by arc's long names.
const LONG_NAMES: [&str; 13] = [
    "filter-rows",
    "sort-rows",
    "hide-column",
    "rename-column",
    "derive-column",
    "cast-column",
    "fill-nulls",
    "dedupe-rows",
    "limit-rows",
    "summarise-rows",
    "join-tables",
    "append-rows",
    "read-file",
];

fn described(long_name: &str) -> OperationDescription {
    operation::describe(&runner(), long_name)
        .unwrap_or_else(|e| panic!("{long_name} describes: {e}"))
}

fn parameter<'a>(d: &'a OperationDescription, key: &str) -> &'a Parameter {
    d.parameters
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("{} takes {key}: {:?}", d.long_name, d.parameters))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-descriptions-{name}-{nanos}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
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

/// A Protocol of two SQL steps, recorded through arc: `housing`, and `by_age`
/// counting its rows by `house_age`.
fn two_tables(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    std::fs::write(dir.path().join("arcform.yaml"), "name: probe\nsteps: []\n")
        .expect("the manifest writes");
    operation::record_sql(
        &runner(),
        dir.path(),
        "SELECT 1 AS house_age, 2 AS rooms",
        "housing",
    )
    .expect("arc records housing");
    operation::record_sql(
        &runner(),
        dir.path(),
        "SELECT house_age, count(*) AS n FROM housing GROUP BY house_age",
        "by_age",
    )
    .expect("arc records by_age");
    dir
}

/// The model arc wrote for the step `name` in `dir`.
fn model_of(dir: &Path, name: &str) -> String {
    let target = format!("CREATE OR REPLACE TABLE \"{name}\" AS");
    std::fs::read_dir(dir.join("models"))
        .expect("models")
        .map(|e| std::fs::read_to_string(e.expect("entry").path()).expect("model"))
        .find(|text| text.contains(&target))
        .unwrap_or_else(|| panic!("a model making {name}"))
}

/// Every file under `dir` with its bytes, in path order.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("the folder reads") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a file reads");
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
}

#[test]
fn every_operation_arc_lists_is_described() {
    let mut listed = operation::list_operations(&runner()).expect("arc lists its operations");
    listed.sort();
    let mut want: Vec<String> = LONG_NAMES.iter().map(|n| (*n).to_string()).collect();
    want.sort();
    assert_eq!(listed, want);
    for name in LONG_NAMES {
        let d = described(name);
        assert_eq!(d.long_name, name);
        assert!(!d.summary.is_empty(), "{name} has a summary");
    }
}

#[test]
fn each_value_has_the_shape_arc_gives_it() {
    use SqlKind::{Condition, Expression, Order, Partition, SelectList, Type};
    use ValueShape::{Column, Columns, Integer, Path, Sql, Text};
    let want = [
        ("filter-rows", "where", Sql(Condition), true),
        ("sort-rows", "order_by", Sql(Order), true),
        ("hide-column", "column", Column, true),
        ("rename-column", "column", Column, true),
        ("rename-column", "to", Column, true),
        ("derive-column", "column", Column, true),
        ("derive-column", "expression", Sql(Expression), true),
        ("cast-column", "column", Column, true),
        ("cast-column", "type", Sql(Type), true),
        ("fill-nulls", "column", Column, true),
        ("fill-nulls", "value", Sql(Expression), true),
        ("dedupe-rows", "key", Sql(Partition), false),
        ("dedupe-rows", "order_by", Sql(Order), false),
        (
            "limit-rows",
            "count",
            Integer {
                minimum: Some(0),
                maximum: Some(i64::MAX),
            },
            true,
        ),
        ("limit-rows", "order_by", Sql(Order), false),
        ("summarise-rows", "aggregates", Sql(SelectList), true),
        ("summarise-rows", "group_by", Sql(SelectList), false),
        ("join-tables", "using", Columns, false),
        ("join-tables", "condition", Sql(Condition), false),
        ("join-tables", "prefix", Text, false),
        ("read-file", "path", Path, true),
    ];
    let wanted = want.len();
    for (long_name, key, shape, required) in want {
        let d = described(long_name);
        let p = parameter(&d, key);
        assert_eq!(p.shape, shape, "{long_name} {key}");
        assert_eq!(p.required, required, "{long_name} {key} required");
    }
    // Every value arc describes is one of those above, or join-tables' kind.
    let counted: usize = LONG_NAMES
        .iter()
        .map(|n| described(n).parameters.len())
        .sum();
    assert_eq!(counted, wanted + 1);

    let join = described("join-tables");
    let kind = parameter(&join, "kind");
    assert_eq!(
        kind.shape,
        ValueShape::Choice {
            words: ["left", "inner", "right", "full", "semi", "anti", "cross"]
                .iter()
                .map(|w| (*w).to_string())
                .collect(),
            default: Some("left".to_string()),
        }
    );
    assert!(!kind.required);
    assert!(described("append-rows").parameters.is_empty());
}

#[test]
fn the_reads_say_which_tables_and_whether_a_list() {
    for name in LONG_NAMES {
        let d = described(name);
        let table = d.reads.iter().find(|r| r.name == "table");
        if name == "read-file" {
            assert!(d.reads.is_empty(), "read-file reads nothing: {:?}", d.reads);
            continue;
        }
        let table = table.unwrap_or_else(|| panic!("{name} reads a table"));
        assert!(table.required && !table.list, "{name}: {table:?}");
        assert_eq!(table.kind, "table");
    }
    let join = described("join-tables");
    let other = join
        .reads
        .iter()
        .find(|r| r.name == "other")
        .expect("other");
    assert!(other.required && !other.list, "{other:?}");

    let append = described("append-rows");
    let other = append
        .reads
        .iter()
        .find(|r| r.name == "other")
        .expect("other");
    assert!(other.required && other.list, "{other:?}");
    assert_eq!(other.min_items, 1);
}

#[test]
fn dedupe_rows_needs_its_key_and_order_together() {
    let d = described("dedupe-rows");
    assert_eq!(
        d.needed_together,
        [
            ("key".to_string(), vec!["order_by".to_string()]),
            ("order_by".to_string(), vec!["key".to_string()]),
        ]
    );
    assert!(described("hide-column").needed_together.is_empty());
}

#[test]
fn a_request_names_each_read_by_its_role() {
    let dir = two_tables("roles");
    let join = StepRequest {
        long_name: "join-tables".to_string(),
        on: Some("housing".to_string()),
        reads: pairs(&[("other", "by_age")]),
        args: pairs(&[("using", "house_age")]),
    };
    let recorded =
        operation::record_step(&runner(), dir.path(), "j", &join).expect("arc records the join");
    assert!(!recorded.said.is_empty());
    assert!(recorded.notes.is_empty(), "{:?}", recorded.notes);
    let model = model_of(dir.path(), "j");
    assert!(
        model.contains("LEFT JOIN by_age USING (house_age)"),
        "{model}"
    );

    let append = StepRequest {
        long_name: "append-rows".to_string(),
        on: Some("housing".to_string()),
        reads: pairs(&[("other", "by_age"), ("other", "housing")]),
        args: Vec::new(),
    };
    operation::record_step(&runner(), dir.path(), "a", &append).expect("arc records the append");
    let model = model_of(dir.path(), "a");
    assert_eq!(model.matches("UNION ALL BY NAME").count(), 2, "{model}");
    let by_age = model.find("FROM by_age").expect("by_age is read");
    let again = model.rfind("FROM housing").expect("housing is read again");
    assert!(by_age < again, "by_age comes first: {model}");
}

#[test]
fn a_refused_request_returns_arcs_words_and_writes_nothing() {
    let dir = two_tables("refused");
    let before = snapshot(dir.path());
    let limit = StepRequest {
        long_name: "limit-rows".to_string(),
        on: Some("housing".to_string()),
        reads: Vec::new(),
        args: pairs(&[("count", "ten")]),
    };
    let err = operation::record_step(&runner(), dir.path(), "l", &limit)
        .expect_err("ten is no whole number");
    assert!(err.contains("is an integer, not \"ten\""), "{err}");
    assert!(!err.starts_with("error: "), "{err}");
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn arcs_notes_are_kept_when_it_records() {
    let dir = two_tables("notes");
    let read = StepRequest {
        long_name: "read-file".to_string(),
        on: None,
        reads: Vec::new(),
        args: pairs(&[("path", "https://example.com/x.csv")]),
    };
    let recorded =
        operation::record_step(&runner(), dir.path(), "f", &read).expect("arc records the read");
    assert!(
        recorded.notes[0].contains("is read live at each run"),
        "{:?}",
        recorded.notes
    );
    assert!(!recorded.notes[0].starts_with("note: "));
    assert!(!recorded.said.contains("note: "), "{}", recorded.said);
}
