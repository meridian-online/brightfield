//! A Protocol with a step the managed edition cannot run opens restricted, names the step and why,
//! and stays restricted however it is trusted.
//!
//! The checks of a manifest take the edition as an argument, so they run in either build. The
//! tests that ask the build's own edition, through the trust store and the window, run in the
//! managed build and are ignored in the full one:
//!
//! ```text
//! cargo test -p brightfield-shell --features managed-edition --test beyond_edition
//! cargo test -p brightfield-shell --test beyond_edition
//! ```

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use arc::spec::Manifest;
use brightfield_shell::beyond_edition::{
    self, beyond_edition, every_step_beyond, sentence, BeyondEdition, Reason, OPERATORS,
};
use brightfield_shell::edition::Edition;

/// A scratch folder of this test process's own, with the config folder the trust store is kept in
/// set once, before any test reads it.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-beyond-edition-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

/// A Protocol folder named `name` holding `manifest` and each `(path, text)` of `files`.
fn protocol(name: &str, manifest: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch().join("input").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a Protocol folder");
    std::fs::write(dir.join("arcform.yaml"), manifest).expect("the manifest");
    for (path, text) in files {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).expect("a folder for the file");
        std::fs::write(file, text).expect("a file of the Protocol");
    }
    dir
}

/// What `edition` finds beyond it in the Protocol in `dir`.
fn found(dir: &Path, edition: Edition) -> Option<BeyondEdition> {
    let manifest = Manifest::load(dir).unwrap_or_else(|e| panic!("{} loads: {e}", dir.display()));
    beyond_edition(dir, &manifest, edition)
}

const COMMAND_STEP: &str =
    "name: cleaning\nengine: duckdb\nsteps:\n  - name: clean\n    command: uv run clean.py\n";

fn a_command_step() -> PathBuf {
    protocol("command_step", COMMAND_STEP, &[])
}

#[test]
fn a_command_step_is_beyond_the_managed_edition() {
    let b = found(&a_command_step(), Edition::Managed).expect("a command step is found");
    assert_eq!(
        b,
        BeyondEdition {
            step: "clean".into(),
            reason: Reason::CommandStep("uv".into()),
        }
    );
    assert_eq!(
        sentence(&b),
        "Step clean runs uv, which this edition can't run."
    );
}

#[test]
fn a_shell_precondition_is_beyond_it() {
    let dir = protocol(
        "shell_precondition",
        "name: checked\nengine: duckdb\nsteps:\n  - name: load\n    sql: models/load.sql\n    preconditions:\n      - command: \"test -f build/done\"\n",
        &[("models/load.sql", "CREATE OR REPLACE TABLE t AS SELECT 1 AS x;\n")],
    );
    let b = found(&dir, Edition::Managed).expect("a shell precondition is found");
    assert_eq!(b.step, "load");
    assert_eq!(b.reason, Reason::Precondition);
    assert!(sentence(&b).starts_with("Step load "), "{}", sentence(&b));
}

/// A tool precondition that only fingerprints a file runs nothing; one that asks the tool its
/// version runs a shell command.
#[test]
fn a_tool_precondition_is_beyond_it_only_when_it_runs_shell() {
    let dir = protocol(
        "tool_precondition",
        "name: tooled\nengine: duckdb\nsteps:\n  - name: hashed\n    sql: models/a.sql\n    preconditions:\n      - tool:\n          path: vendor/x.bin\n          contents: true\n  - name: asked\n    sql: models/b.sql\n    preconditions:\n      - tool:\n          name: duckdb\n          version: \"$ARC_TOOL --version\"\n",
        &[
            ("models/a.sql", "CREATE OR REPLACE TABLE a AS SELECT 1 AS x;\n"),
            ("models/b.sql", "CREATE OR REPLACE TABLE b AS SELECT 1 AS x;\n"),
        ],
    );
    let b = found(&dir, Edition::Managed).expect("the version check is found");
    assert_eq!((b.step.as_str(), b.reason), ("asked", Reason::Precondition));
}

const UNBUNDLED: &str =
    "name: hexes\nengine: duckdb\nsteps:\n  - name: cells\n    sql: models/cells.sql\n";

#[test]
fn an_unbundled_extension_is_beyond_it() {
    let dir = protocol(
        "unbundled_extension",
        UNBUNDLED,
        &[(
            "models/cells.sql",
            "INSTALL h3 FROM community;\nLOAD h3;\nCREATE OR REPLACE TABLE cells AS SELECT 1 AS x;\n",
        )],
    );
    let b = found(&dir, Edition::Managed).expect("an unbundled extension is found");
    assert_eq!(b.step, "cells");
    assert_eq!(b.reason, Reason::Install("h3".into()));
    assert_eq!(
        sentence(&b),
        "Step cells installs the DuckDB extension h3, and this edition installs no extensions: it \
         runs only the ones built into it."
    );
}

#[test]
fn an_unbundled_extension_loaded_without_an_install_is_beyond_it() {
    let dir = protocol(
        "unbundled_load",
        UNBUNDLED,
        &[(
            "models/cells.sql",
            "LOAD h3;\nCREATE OR REPLACE TABLE cells AS SELECT 1 AS x;\n",
        )],
    );
    let b = found(&dir, Edition::Managed).expect("an unbundled extension is found");
    assert_eq!(b.reason, Reason::Extension("h3".into()));
    assert_eq!(
        sentence(&b),
        "Step cells loads the DuckDB extension h3, which this edition can't load."
    );
}

#[test]
fn loading_spatial_and_excel_is_not() {
    let dir = protocol(
        "built_in_extensions",
        UNBUNDLED,
        &[(
            "models/cells.sql",
            "-- LOAD h3 is not used here\nLOAD spatial;\nLOAD excel;\nLOAD json; LOAD parquet; LOAD icu; LOAD sqlite_scanner;\nCREATE OR REPLACE TABLE cells AS SELECT 1 AS x;\n",
        )],
    );
    assert_eq!(found(&dir, Edition::Managed), None);
}

/// The managed edition's DuckDB is built without its extension installer, so an `INSTALL` fails
/// there even for an extension it has built in, and would otherwise fail part way through a run.
#[test]
fn installing_a_built_in_extension_is_beyond_it() {
    let dir = protocol(
        "install_built_in",
        UNBUNDLED,
        &[(
            "models/cells.sql",
            "INSTALL spatial;\nLOAD spatial;\nCREATE OR REPLACE TABLE cells AS SELECT 1 AS x;\n",
        )],
    );
    let b = found(&dir, Edition::Managed).expect("the install is found");
    assert_eq!(b.reason, Reason::Install("spatial".into()));
    let dir = protocol(
        "update_extensions",
        UNBUNDLED,
        &[("models/cells.sql", "UPDATE EXTENSIONS;\nSELECT 1;\n")],
    );
    let b = found(&dir, Edition::Managed).expect("the update is found");
    assert_eq!(b.reason, Reason::UpdateExtensions);
    assert_eq!(
        sentence(&b),
        "Step cells updates DuckDB extensions, and this edition installs no extensions: it runs \
         only the ones built into it."
    );
    assert_eq!(found(&dir, Edition::Full), None);
}

/// An extension loaded from a file is beyond the managed edition even when one of that name is
/// built in, since it loads no extension from a file.
#[test]
fn an_extension_loaded_from_a_file_is_beyond_it() {
    let dir = protocol(
        "extension_file",
        UNBUNDLED,
        &[(
            "models/cells.sql",
            "LOAD 'vendor/finetype.duckdb_extension';\nCREATE OR REPLACE TABLE cells AS SELECT 1 AS x;\n",
        )],
    );
    let b = found(&dir, Edition::Managed).expect("a file extension is found");
    assert_eq!(
        b.reason,
        Reason::Extension("vendor/finetype.duckdb_extension".into())
    );
}

#[test]
fn the_full_edition_finds_nothing_beyond_it() {
    assert_eq!(found(&a_command_step(), Edition::Full), None);
    let dir = protocol(
        "full_unbundled",
        UNBUNDLED,
        &[("models/cells.sql", "LOAD h3;\nSELECT 1;\n")],
    );
    assert_eq!(found(&dir, Edition::Full), None);
    let manifest = Manifest::load(&edgar_gleif()).expect("edgar_gleif loads");
    assert!(every_step_beyond(&edgar_gleif(), &manifest, Edition::Full).is_empty());
}

/// FineType's CLI ships with the managed edition as a helper, so arc's FineType operators run.
#[test]
fn a_finetype_operator_is_within_the_managed_edition() {
    let dir = protocol(
        "finetype_operator",
        "name: typed\nengine: duckdb\nsteps:\n  - name: validate\n    op: finetype_validate@1\n    with:\n      parquet: build/out.parquet\n      schema: schema.finetype.json\n  - name: describe\n    op: datapackage_describe@1\n    with:\n      parquet: build/out.parquet\n      overrides: datapackage.overrides.json\n      out: build/datapackage.json\n",
        &[],
    );
    assert_eq!(found(&dir, Edition::Managed), None);
}

/// A FineType operator pointed at an extension file of its own loads that file, which the managed
/// edition does not.
#[test]
fn a_finetype_operator_given_its_own_extension_file_is_beyond_it() {
    let dir = protocol(
        "finetype_extension_file",
        "name: typed\nengine: duckdb\nsteps:\n  - name: validate\n    op: finetype_validate@1\n    with:\n      parquet: build/out.parquet\n      schema: schema.finetype.json\n      extension: vendor/finetype.duckdb_extension\n",
        &[],
    );
    let b = found(&dir, Edition::Managed).expect("the extension file is found");
    assert_eq!(
        b.reason,
        Reason::Extension("vendor/finetype.duckdb_extension".into())
    );
}

/// arc's general Splink step starts `uv`, as `splink_resolve` does, so the managed edition names
/// `uv` rather than the operator.
#[test]
fn a_splink_step_is_beyond_it_naming_uv() {
    let manifest = "name: scoring\nengine: duckdb\nsteps:\n  - name: score_pairs\n    op: splink@1\n    with:\n      inputs:\n        - role: a\n          path: build/a.parquet\n        - role: b\n          path: build/b.parquet\n      model: models/model.json\n      out: build/scored.parquet\n";
    let dir = protocol("splink_step", manifest, &[("models/model.json", "{}")]);
    assert_eq!(
        found(&dir, Edition::Managed),
        Some(BeyondEdition {
            step: "score_pairs".into(),
            reason: Reason::OutsideProgram("uv".into()),
        })
    );
    assert_eq!(found(&dir, Edition::Full), None);
}

/// The table of operators names only operators arc's pinned catalog has, so a renamed one fails
/// here rather than being taken for a program nobody looked at.
#[test]
fn every_operator_named_here_is_in_arcs_catalog() {
    for (name, _) in OPERATORS {
        let text = format!("name: probe\nengine: duckdb\nsteps:\n  - name: s\n    op: {name}@1\n");
        if let Err(e) = arc::spec::Manifest::from_yaml_str(&text) {
            assert!(
                !e.to_string().contains("unknown operator"),
                "{name} is not in arc's catalog: {e}"
            );
        }
    }
}

/// edgar_gleif's first step that runs `uv` is the fetch of GLEIF's SEC registrations, and its
/// resolve step runs `uv` too.
#[test]
fn edgar_gleif_opens_restricted_naming_its_first_uv_step() {
    let dir = edgar_gleif();
    let manifest = Manifest::load(&dir).expect("edgar_gleif loads");
    let first = beyond_edition(&dir, &manifest, Edition::Managed).expect("edgar_gleif is beyond");
    assert_eq!(
        first,
        BeyondEdition {
            step: "fetch_gleif_sec_registrations".into(),
            reason: Reason::OutsideProgram("uv".into()),
        }
    );
    let every: Vec<BeyondEdition> = every_step_beyond(&dir, &manifest, Edition::Managed);
    assert_eq!(
        every.iter().map(|b| b.step.as_str()).collect::<Vec<_>>(),
        ["fetch_gleif_sec_registrations", "resolve"],
        "{every:?}"
    );
    assert_eq!(
        sentence(&every[1]),
        "Step resolve runs uv, which this edition can't run."
    );
}

#[test]
fn every_other_shipped_protocol_opens_unrestricted() {
    let root = repo();
    let mut checked = Vec::new();
    for parent in [
        "examples/starts",
        "examples/protocol",
        "tests/fixtures/agent-protocols",
    ] {
        for entry in std::fs::read_dir(root.join(parent)).expect("a folder of Protocols") {
            let dir = entry.expect("an entry").path();
            if !dir.join("arcform.yaml").is_file() || dir == edgar_gleif() {
                continue;
            }
            let manifest =
                Manifest::load(&dir).unwrap_or_else(|e| panic!("{} loads: {e}", dir.display()));
            assert_eq!(
                beyond_edition(&dir, &manifest, Edition::Managed),
                None,
                "{}",
                dir.display()
            );
            checked.push(dir);
        }
    }
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol");
    let manifest = Manifest::load(&fixture).expect("the fixture loads");
    assert_eq!(beyond_edition(&fixture, &manifest, Edition::Managed), None);
    assert!(
        checked.len() >= 6,
        "the shipped Protocols were found: {checked:?}"
    );
}

/// A statement kept from the palette that loads an unbundled extension is named as it is added.
#[test]
fn a_statement_loading_an_unbundled_extension_is_named_as_it_is_added() {
    let b = beyond_edition::statement("cells", "LOAD h3; SELECT 1", Edition::Managed)
        .expect("the statement is beyond");
    assert_eq!(b.reason, Reason::Extension("h3".into()));
    assert_eq!(
        beyond_edition::statement("cells", "LOAD h3; SELECT 1", Edition::Full),
        None
    );
    assert_eq!(
        beyond_edition::statement("cells", "SELECT 1", Edition::Managed),
        None
    );
}

#[test]
#[cfg_attr(
    not(feature = "managed-edition"),
    ignore = "asks the managed build's own edition"
)]
fn trust_does_not_lift_the_edition_restriction() {
    let dir = a_command_step();
    brightfield_shell::trust::Store::current()
        .trust(&dir)
        .expect("the Protocol is trusted");
    assert!(brightfield_shell::trust::Store::current().is_trusted(&dir));
    assert!(
        brightfield_shell::trust::refuses_run(&dir),
        "a trusted Protocol beyond the edition still does not run"
    );
}

/// The window opens a trusted Protocol beyond the edition restricted, naming the step, why and
/// where the full edition is, and offers no Trust.
#[test]
#[cfg_attr(
    not(feature = "managed-edition"),
    ignore = "asks the managed build's own edition"
)]
fn a_trusted_protocol_beyond_the_edition_opens_restricted_in_the_window() {
    use brightfield_shell::design::Mode;
    use brightfield_shell::window::{Boot, MeridianApp};
    let dir = protocol("window_command_step", COMMAND_STEP, &[]);
    brightfield_shell::trust::Store::current()
        .trust(&dir)
        .expect("the Protocol is trusted");
    let mut app = MeridianApp::headless(Boot::empty(), Mode::Light);
    let ctx = egui::Context::default();
    app.open_protocol_path(&ctx, &dir.to_string_lossy());
    let restricted = app.restricted().expect("it opens restricted");
    let beyond = restricted
        .beyond
        .clone()
        .expect("it is held for the edition");
    assert_eq!(beyond.step, "clean");
    let said = restricted.said();
    assert!(
        said.contains("Step clean runs uv, which this edition can't run."),
        "{said}"
    );
    assert!(!restricted.offers_trust(), "trust would not lift it");
    assert!(
        !app.run_protocol(&ctx),
        "a Protocol beyond the edition does not run"
    );
    let raised: Vec<(String, String)> = app
        .notifications()
        .iter()
        .map(|n| {
            (
                n.title.to_string(),
                n.body.clone().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert!(
        raised.iter().any(
            |(title, body)| title == brightfield_shell::window::BEYOND_EDITION_TITLE
                && body.contains("Step clean runs uv")
                && body.contains(brightfield_shell::edition::full_edition_url())
        ),
        "the Run names the step and the full edition, not trust: {raised:?}"
    );
    assert!(
        !raised
            .iter()
            .any(|(_, body)| body.contains("stopped trusting")),
        "{raised:?}"
    );
}

/// In the full edition the same trusted Protocol opens unrestricted.
#[test]
#[cfg_attr(
    feature = "managed-edition",
    ignore = "asks the full build's own edition"
)]
fn a_trusted_protocol_with_a_command_step_opens_unrestricted_in_the_full_edition() {
    let dir = a_command_step();
    brightfield_shell::trust::Store::current()
        .trust(&dir)
        .expect("the Protocol is trusted");
    assert!(!brightfield_shell::trust::refuses_run(&dir));
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn edgar_gleif() -> PathBuf {
    repo().join("examples/protocol/edgar_gleif")
}
