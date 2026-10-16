//! A statement typed in the palette and not yet saved is a step of the
//! Protocol: the graph draws its node and its table, the spine lists both at
//! the foot of what they read and marks them not recorded, and a grid over its
//! table previews the steps it reads and no others.
//!
//! The data file is the housing start's own, written into a folder of this
//! test's own and opened the way a window opens it.

use std::path::{Path, PathBuf};

use brightfield_engine::{
    preview_row_count_stoppable, FileReader, PreviewOp, PreviewStop, PREVIEW_DEADLINE,
    PREVIEW_FILE_LIMIT_BYTES,
};
use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::generated::{
    preview_chain, GeneratedProtocol, GeneratedStep, KnownOperation, StepEffect,
};
use brightfield_shell::one_step::OneStepProtocol;
use brightfield_shell::protocol::{ProtocolModel, SpineMarker, SpineRole};
use brightfield_shell::starts;
use brightfield_shell::typed_sql::TypedStep;
use brightfield_shell::window::{Boot, MeridianApp};

const BY_AGE: &str = "select house_age, count(*) as n\nfrom california_housing\ngroup by house_age";

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-typed-sql-nodes-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

/// The Protocol a window opened over the housing file holds, in a folder with
/// no manifest yet.
fn source() -> OneStepProtocol {
    let dir = TempDir::new();
    let data = dir.path().join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
    let app = MeridianApp::headless(boot, Mode::Light);
    // The folder goes when this returns. A folder with no manifest is read as
    // the source's own one step, which is what a gone folder reads as too.
    app.protocol_model()
        .source()
        .cloned()
        .expect("a data file's Protocol has a source")
}

fn typed(name: &str, statement: &str, reads: &[&str]) -> TypedStep {
    TypedStep {
        name: name.to_string(),
        statement: statement.to_string(),
        reads: reads.iter().map(|r| (*r).to_string()).collect(),
        rows: None,
    }
}

/// The rows the last step of `chain` makes, previewed from the housing file.
fn rows_of(chain: &[(String, PreviewOp)]) -> u64 {
    let data =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/starts/california_housing.parquet");
    preview_row_count_stoppable(
        &data,
        FileReader::Parquet,
        "california_housing",
        chain,
        PREVIEW_FILE_LIMIT_BYTES,
        PREVIEW_DEADLINE,
        &PreviewStop::default(),
    )
    .expect("the chain previews")
}

fn filter_step(name: &str, condition: &str) -> GeneratedStep {
    GeneratedStep {
        name: name.to_string(),
        effect: StepEffect::Operation(KnownOperation::FilterRows),
        on: "california_housing".to_string(),
        reads: vec!["california_housing".to_string()],
        argument: condition.to_string(),
    }
}

#[test]
fn a_typed_step_and_its_table_stand_at_the_spines_foot_marked_not_recorded() {
    let model = ProtocolModel::new(
        source()
            .inputs_with_typed(&[typed(
                "california_housing_sql_by_house_age",
                BY_AGE,
                &["california_housing"],
            )])
            .unwrap(),
        Flow::Vertical,
    );
    let spine = model.spine();
    let step = spine
        .iter()
        .position(|r| r.role == SpineRole::Step && r.label == "california_housing_sql_by_house_age")
        .unwrap();
    assert_eq!(
        (spine[step].kind.as_str(), spine[step].marker),
        ("sql \u{b7} not recorded", SpineMarker::Unrecorded)
    );
    let table = &spine[step + 1];
    assert_eq!(
        (table.role, table.marker, table.kind.as_str()),
        (SpineRole::Asset, SpineMarker::Unrecorded, "table")
    );
    assert!(
        spine
            .iter()
            .position(|r| r.label == "california_housing")
            .unwrap()
            < step
    );
    assert!(model.is_viewable(
        model
            .node_of_step_table("california_housing_sql_by_house_age")
            .unwrap()
    ));
}

#[test]
fn a_chain_of_typed_steps_reads_in_the_order_made() {
    let inputs = source()
        .inputs_with_typed(&[
            typed("a_sql", BY_AGE, &["california_housing"]),
            typed("b_sql", "from a_sql select house_age", &["a_sql"]),
        ])
        .unwrap();
    let g = inputs.generated.as_ref().unwrap();
    assert_eq!(
        g.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["a_sql", "b_sql"]
    );
    assert_eq!(
        preview_chain(g, "b_sql")
            .unwrap()
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["a_sql", "b_sql"]
    );
}

#[test]
fn a_from_first_statement_still_makes_a_node() {
    let model = ProtocolModel::new(
        source()
            .inputs_with_typed(&[typed(
                "t_sql",
                "from california_housing select house_age order by house_age",
                &["california_housing"],
            )])
            .unwrap(),
        Flow::Vertical,
    );
    assert!(model
        .node_of_step_table("t_sql")
        .is_some_and(|n| model.is_viewable(n)));
}

#[test]
fn a_typed_steps_row_count_shows_once_it_is_known() {
    let mut model = ProtocolModel::new(
        source()
            .inputs_with_typed(&[typed("a_sql", BY_AGE, &["california_housing"])])
            .unwrap(),
        Flow::Vertical,
    );
    model.set_typed_rows("a_sql", 52);
    assert!(model
        .spine()
        .iter()
        .any(|r| r.label == "a_sql" && r.kind == "table \u{b7} 52 rows"));
}

#[test]
fn a_preview_chain_leaves_out_a_step_its_target_does_not_read() {
    // A generated filter on the load, and a typed step that reads only the load.
    let g = GeneratedProtocol {
        data: PathBuf::from(starts::HOUSING_FILE),
        dir: PathBuf::from("."),
        root_table: "california_housing".to_string(),
        steps: vec![
            filter_step("f", "house_age > 28"),
            typed("a_sql", BY_AGE, &["california_housing"]).as_generated(),
        ],
    };
    assert_eq!(
        preview_chain(&g, "a_sql").unwrap(),
        vec![(
            "a_sql".to_string(),
            PreviewOp::Statement(BY_AGE.to_string())
        )]
    );
}

#[test]
fn a_statement_over_two_branches_previews_each_from_the_table_it_reads() {
    // Two filters of the load, which part its rows between them, and a
    // statement that puts them back together: each filter must read the load,
    // not the step listed before it in the chain.
    let g = GeneratedProtocol {
        data: PathBuf::from(starts::HOUSING_FILE),
        dir: PathBuf::from("."),
        root_table: "california_housing".to_string(),
        steps: vec![
            filter_step("old", "house_age > 28"),
            filter_step("young", "house_age <= 28"),
            typed(
                "both_sql",
                "select * from old union all select * from young",
                &["old", "young"],
            )
            .as_generated(),
        ],
    };
    let chain = preview_chain(&g, "both_sql").unwrap();
    assert_eq!(rows_of(&chain), rows_of(&[]));
}
