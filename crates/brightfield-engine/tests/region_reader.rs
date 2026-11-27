//! The rows a plot's regions describe, and the reader that fetches them from a
//! second thread.
//!
//! Regions describe what the plot draws before any other plot's selection, so
//! brushing elsewhere must not move them. The reader runs the queries off the
//! thread that owns the session, so it has to see the tables and views the
//! session made and be stoppable from outside.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{Engine, RowsAudience, Session, SqlPredicate};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::ScalarValue;
use duckdb::arrow::array::{Array, Int64Array};

fn session_for(yaml: &str) -> Session {
    let spec = parse_spec(yaml, Format::Yaml).expect("parses").spec;
    let analysis = analyse_spec(&spec).expect("analyses");
    Engine::new()
        .load_spec(spec, analysis, None)
        .expect("loads")
        .session
}

/// The count `SELECT count(*) FROM (<sql>)` answers through the reader.
fn count_through_reader(session: &Session, sql: &str) -> i64 {
    let reader = session.region_reader().expect("a reader opens");
    let wrapped = format!("SELECT count(*) FROM ({sql})");
    let batches = std::thread::spawn(move || reader.query(&wrapped))
        .join()
        .expect("the reader thread did not panic")
        .expect("the read runs");
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count is a bigint")
        .value(0)
}

/// Plot A reads `t` through the brush that plot B contributes.
const TWO_PLOTS: &str = "params:\n  brush:\n    select: intersect\ndata:\n  t:\n    \
    - { x: 1, y: 1 }\n    - { x: 50, y: 50 }\n    - { x: 90, y: 90 }\n\
hconcat:\n  - plot:\n    - mark: dot\n      data: { from: t, filterBy: $brush }\n      x: x\n      y: y\n  \
  - plot:\n    - mark: dot\n      data: { from: t }\n      x: x\n      y: y\n    - select: intervalXY\n      as: $brush\n";

#[test]
fn region_rows_ignore_other_plots_selections() {
    let spec = parse_spec(TWO_PLOTS, Format::Yaml).expect("parses").spec;
    let analysis = analyse_spec(&spec).expect("analyses");
    let session = Engine::new()
        .load_spec(spec, analysis, None)
        .expect("loads")
        .session;
    let mut c = Coordinator::from_session(session);
    c.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor: ComponentPath("root.1".to_string()),
        predicate: SqlPredicate::Interval {
            column: "\"x\"".to_string(),
            lo: ScalarValue::Float(80.0),
            hi: ScalarValue::Float(100.0),
            meta: None,
        },
    });

    // The control: the plot's own rows really are narrowed by the brush, so
    // the unfiltered count below is not just the brush failing to apply.
    let drawn = c
        .session()
        .step_rows_count(0, RowsAudience::Plot)
        .expect("counts");
    assert_eq!(drawn, 1, "the brush narrows what plot A draws");

    let sql = c.session().region_rows_sql(0).expect("emits");
    assert_eq!(count_through_reader(c.session(), &sql), 3);
}

fn earthquake_start() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/starts/usgs-earthquakes-2025/data/earthquakes_2025.parquet")
        .canonicalize()
        .expect("the earthquake start's table is in the repository")
}

#[test]
fn the_reader_sees_the_sessions_tables_and_views() {
    // The start's panel reads `earthquakes` from the Protocol's database; the
    // shell copies it to Parquet and points the panel's `data:` at the file.
    // This is the same shape, loaded through the engine's own path.
    let yaml = format!(
        "data:\n  earthquakes:\n    file: \"{}\"\nplot:\n  - mark: dot\n    \
         data: {{ from: earthquakes }}\n    x: longitude\n    y: latitude\n",
        earthquake_start().display()
    );
    let session = session_for(&yaml);
    let sql = session.region_rows_sql(0).expect("emits");
    assert_eq!(count_through_reader(&session, &sql), 8_502);
}

#[test]
fn interrupt_stops_a_running_read() {
    let session = session_for(TWO_PLOTS);
    let reader = session.region_reader().expect("a reader opens");
    let stop = reader.interrupt_handle();
    let started = Instant::now();
    let handle = std::thread::spawn(move || {
        reader.query("SELECT count(*) FROM range(100000000000) a, range(1000) b")
    });
    std::thread::sleep(Duration::from_millis(300));
    stop.interrupt();
    let outcome = handle.join().expect("the reader thread did not panic");
    assert!(outcome.is_err(), "an interrupted read fails");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the interrupt stopped the read promptly"
    );
}

#[test]
fn the_reader_sees_a_materialised_source() {
    let yaml = format!(
        "data:\n  earthquakes:\n    file: \"{}\"\nplot:\n  - mark: dot\n    \
         data: {{ from: earthquakes }}\n    x: longitude\n    y: latitude\n",
        earthquake_start().display()
    );
    let mut session = session_for(&yaml);
    session
        .materialise_source("earthquakes", 1 << 30)
        .expect("the copy fits");
    let sql = session.region_rows_sql(0).expect("emits");
    assert_eq!(count_through_reader(&session, &sql), 8_502);
}
