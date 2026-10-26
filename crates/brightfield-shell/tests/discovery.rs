//! The discovery seam and its stand-in.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_ossie::{DecidedBy, Role};
use brightfield_shell::discovery::{Discovery, Pass, StandIn};
use brightfield_shell::own_files::{self, OwnProtocol};
use brightfield_shell::run::Runner;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract")
}

#[test]
fn stand_in_reads_fixture_model() {
    let dir = scratch().join("input").join("fixture-protocol");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        fixture().join("model.ossie.yaml"),
        dir.join("model.ossie.yaml"),
    )
    .unwrap();
    let protocol = OwnProtocol {
        dir,
        sources: Default::default(),
        profiles: Vec::new(),
    };
    for pass in [Pass::Sample, Pass::Full] {
        let model = StandIn
            .discover(&protocol, pass)
            .expect("the fixture reads");
        assert_eq!(model.datasets.len(), 8);
        assert_eq!(model.relationships.len(), 7);
    }
}

#[test]
fn stand_in_without_model_lists_unclear_tables() {
    let root = scratch().join("input").join("plain");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("customers.csv"), "id,name\n1,Ada\n2,Bo\n").unwrap();
    std::fs::write(root.join("orders.csv"), "id,customer_id\n10,1\n").unwrap();
    let mut protocol = own_files::create(&root).unwrap();
    own_files::run_loads(
        &protocol,
        &Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")),
    )
    .unwrap();
    own_files::profile(&mut protocol).unwrap();

    let model = StandIn.discover(&protocol, Pass::Sample).unwrap();
    let names: Vec<&str> = model.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["customers", "orders"]);
    assert!(model.relationships.is_empty());
    for d in &model.datasets {
        assert_eq!(d.meta.role, Role::Unclear);
        assert_eq!(d.meta.decided_by, DecidedBy::Dovetail);
        assert_eq!(d.source, format!("raw_{}", d.name));
        assert!(d.fields.iter().all(|f| !f.name.starts_with("__source")));
        assert!(d.meta.rows.is_some());
        assert!(d.meta.source_file.ends_with(&format!("{}.csv", d.name)));
    }
    let customers = &model.datasets[0];
    let fields: Vec<&str> = customers.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(fields, ["id", "name"]);
}
