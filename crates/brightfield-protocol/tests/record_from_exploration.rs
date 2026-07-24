//! A grid exploration promoted to a durable step — held to the promises that
//! make promotion safe, on brightfield's own hand-authored hostile corpus.
//!
//! The record path is arc's; this suite proves brightfield drives it correctly
//! and adds nothing of its own to the durable document:
//!
//!   1. **byte preservation** — recording onto a hand-authored, commented spec
//!      changes exactly the appended step and nothing else, proven by removing
//!      the appended lines and demanding the original back byte-for-byte;
//!   2. **arc parity** — the grown spec reloads through arc's own gate (the same
//!      parse+validate `arc run` loads with), with the recorded step shaped like
//!      a hand-written `sql:` step. (Executing under the bare `arc` binary is a
//!      local proof, not a hermetic one: the binary is not a build product of
//!      this workspace — it lives in the arcform repo behind its `cli` feature —
//!      exactly as `roundtrip.rs` notes. The generated SQL is authored to run
//!      cleanly there.)
//!   3. **never-run honesty** — the promoted step is never-run on entry, even
//!      though equivalent rows were just on screen: recording runs nothing, so
//!      its produced asset carries the offline `NotRun` seam, never `Ok`;
//!   4. **ownership** — the marker is the license to regenerate: amending a
//!      generated model rewrites it and drags its downstream steps stale;
//!      amending a hand-authored model is refused with the file untouched.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use arc::spec::{Error, Manifest, MANIFEST_FILENAME};
use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_protocol::graph::{build_graph, load_model_sources};
use brightfield_protocol::record::{amend_recorded_filter, record_grid_filter, GridFilter};
use brightfield_protocol::{outline_rows, parse_manifest_str};
use brightfield_sql::ir::{Predicate, ScalarValue};

fn corpus(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read corpus {name}: {e}"))
}

/// A scratch protocol directory seeded with `text` as `arcform.yaml`.
struct SpecDir(PathBuf);

impl SpecDir {
    fn new(test: &str, text: &str) -> Self {
        // A per-instance counter keeps two tests (or two calls) that share a
        // name from colliding on one scratch dir under parallel execution.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bf-record-{}-{test}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        fs::write(dir.join(MANIFEST_FILENAME), text).expect("seed spec");
        Self(dir)
    }

    fn write_model(&self, rel: &str, body: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).expect("models dir");
        fs::write(path, body).expect("seed model");
    }

    fn read_manifest(&self) -> String {
        fs::read_to_string(self.0.join(MANIFEST_FILENAME)).expect("read back")
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.0.join(rel)).expect("read model")
    }
}

impl Drop for SpecDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The grid filter under test: `port = 'dover'`, the narrowest real capture.
fn dover_filter() -> GridFilter {
    GridFilter {
        upstream: "tides".to_string(),
        predicate: Predicate::Point {
            column: "port".to_string(),
            values: vec![ScalarValue::Text("dover".to_string())],
            meta: None,
        },
    }
}

#[test]
fn recording_appends_the_step_and_preserves_every_other_byte() {
    let original = corpus("pinned-fetch.yaml");
    let dir = SpecDir::new("bytes", &original);

    let promo = record_grid_filter(dir.0.as_path(), "dover_tides", &dover_filter())
        .expect("records the grid filter");
    assert_eq!(promo.model_path, PathBuf::from("models/01_dover_tides.sql"));

    // The model is the marker line, then the push-down SQL verbatim.
    let model = dir.read("models/01_dover_tides.sql");
    assert!(
        model.starts_with("-- generated: brightfield grid filter on tides: port = 'dover'\n"),
        "the marker header licenses regeneration: {model}"
    );
    assert!(
        model.contains("CREATE OR REPLACE TABLE \"dover_tides\" AS")
            && model.contains("FROM tides")
            && model.contains("WHERE port = 'dover';"),
        "the predicate is pushed down into a WHERE: {model}"
    );

    // Byte preservation: remove the two appended manifest lines and the original
    // must come back, byte for byte. Nothing else moved.
    let item = "  - name: dover_tides\n    sql: models/01_dover_tides.sql\n";
    let on_disk = dir.read_manifest();
    assert!(
        on_disk.contains(item),
        "the step was spliced in:\n{on_disk}"
    );
    assert_eq!(
        on_disk.replacen(item, "", 1),
        original,
        "every untargeted byte is identical"
    );
}

#[test]
fn the_grown_spec_reloads_through_arcs_own_gate() {
    let dir = SpecDir::new("parity", &corpus("pinned-fetch.yaml"));
    record_grid_filter(dir.0.as_path(), "dover_tides", &dover_filter()).expect("records");

    // arc's loader — the same gate `arc run` loads a protocol with.
    let reloaded = Manifest::load(dir.0.as_path()).expect("the grown spec loads under arc");
    let last = reloaded.steps.last().expect("a step");
    assert_eq!(last.name, "dover_tides");
    assert_eq!(last.sql.as_deref(), Some("models/01_dover_tides.sql"));
    assert!(
        last.produces.is_empty() && last.depends_on.is_empty(),
        "wiring is introspection's job, exactly as for a hand-written sql step"
    );
}

#[test]
fn a_promoted_step_is_never_run_on_entry_not_fresh() {
    let dir = SpecDir::new("neverrun", &corpus("pinned-fetch.yaml"));
    record_grid_filter(dir.0.as_path(), "dover_tides", &dover_filter()).expect("records");

    // Build the offline graph from the grown manifest — no run contract exists,
    // because recording ran nothing.
    let manifest = parse_manifest_str(&dir.read_manifest()).expect("parse");
    let sources = load_model_sources(&manifest, dir.0.as_path());
    let graph = build_graph(&manifest, &sources);

    // The promotion registered a real lineage node — the asset the recorded step
    // produces exists…
    let produced = graph
        .nodes
        .values()
        .find(|n| n.step.as_deref() == Some("dover_tides"))
        .expect("the recorded step produces an asset");

    // …and it is NEVER-RUN, never fresh. Equivalent rows were just on screen, but
    // the step's data is a promise until `arc run`. The offline seam is NotRun.
    let statuses = BTreeMap::new();
    let rows = outline_rows(&graph, &statuses, None);
    let row = rows
        .iter()
        .find(|r| r.id == produced.id)
        .expect("the produced asset has an outline row");
    assert_eq!(row.status, SeamStatus::NotRun, "never run on entry");
    assert_ne!(row.status, SeamStatus::Ok, "never presented as fresh");
}

/// A three-step chain whose first model is machine-generated (carries the
/// marker), the other two hand-written and reading downstream of it.
fn chain() -> SpecDir {
    let manifest = "\
name: chain
engine: duckdb
# A generated filter feeds two hand-written summaries.
steps:
  - name: filtered
    sql: models/filtered.sql
  - name: summary
    sql: models/summary.sql
  - name: report
    sql: models/report.sql
";
    let dir = SpecDir::new("chain", manifest);
    dir.write_model(
        "models/filtered.sql",
        "-- generated: brightfield grid filter on raw: x > 0\n\
         CREATE OR REPLACE TABLE filtered AS SELECT * FROM raw WHERE x > 0;\n",
    );
    dir.write_model(
        "models/summary.sql",
        "CREATE OR REPLACE TABLE summary AS SELECT count(*) AS n FROM filtered;\n",
    );
    dir.write_model(
        "models/report.sql",
        "CREATE OR REPLACE TABLE report AS SELECT n FROM summary;\n",
    );
    dir
}

#[test]
fn amending_a_generated_model_rewrites_it_and_drags_downstream_stale() {
    let dir = chain();
    let manifest = parse_manifest_str(&dir.read_manifest()).expect("parse");
    let sources = load_model_sources(&manifest, dir.0.as_path());
    let graph = build_graph(&manifest, &sources);

    let tighter = GridFilter {
        upstream: "raw".to_string(),
        predicate: Predicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Int(1),
            hi: ScalarValue::Int(9),
            meta: None,
        },
    };
    let outcome = amend_recorded_filter(dir.0.as_path(), &graph, "filtered", &tighter)
        .expect("a generated model may be regenerated");

    // The model now carries the new predicate…
    let model = dir.read("models/filtered.sql");
    assert!(
        model.contains("WHERE (x >= 1 AND x <= 9);"),
        "the amend rewrote the WHERE: {model}"
    );
    // …and the edit dragged both downstream steps stale, via the existing
    // lineage walk — no second staleness computation.
    assert!(
        outcome.downstream_stale.contains("summary") && outcome.downstream_stale.contains("report"),
        "downstream steps go stale: {:?}",
        outcome.downstream_stale
    );
    assert!(
        !outcome.downstream_stale.contains("filtered"),
        "the amended step is not its own downstream"
    );
}

#[test]
fn amending_a_hand_authored_model_is_refused_untouched() {
    let dir = chain();
    // summary.sql is hand-written — no marker. Its bytes are authorship.
    let before = dir.read("models/summary.sql");
    let manifest = parse_manifest_str(&dir.read_manifest()).expect("parse");
    let sources = load_model_sources(&manifest, dir.0.as_path());
    let graph = build_graph(&manifest, &sources);

    let err = amend_recorded_filter(dir.0.as_path(), &graph, "summary", &dover_filter())
        .expect_err("a hand-authored model cannot be regenerated");
    assert!(
        matches!(err, Error::HandAuthoredSql { .. }),
        "refused as hand-authored: {err:?}"
    );
    assert_eq!(
        dir.read("models/summary.sql"),
        before,
        "the refusal left every byte untouched"
    );
}
