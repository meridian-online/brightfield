//! Integration tests for the `conformance` binary.
//!
//! Asserts CLI surface contracts: exit code 0 on green layer-1 curated,
//! exit code 0 on pending layer-2 curated, and the `SUMMARY:` footer shape.

use std::path::PathBuf;
use std::process::Command;

fn bin_path() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_BIN_EXE_conformance").expect("cargo exposes bin path"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(bin_path())
        .current_dir(repo_root())
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (output.status.success(), stdout)
}

#[test]
fn dfconf_cli_layer_1_curated() {
    let (ok, stdout) = run(&["--layers", "1", "--corpus", "curated"]);
    assert!(ok, "layer-1 curated must exit 0; stdout:\n{stdout}");
    // Footer: failed=0 and pending=0 (all curated layer-1 pass).
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY:"))
        .expect("missing SUMMARY footer");
    assert!(
        summary.contains("failed=0"),
        "expected failed=0 in summary: {summary}"
    );
    assert!(
        summary.contains("pending=0"),
        "expected pending=0 (layer 1 only) in summary: {summary}"
    );
    // At least one pass (10 curated specs).
    assert!(
        summary.contains("passed=") && !summary.contains("passed=0 "),
        "expected passed>0 in summary: {summary}"
    );
}

#[test]
fn dfconf_cli_layer_2_curated_all_pass() {
    // Layer 2 is active for the curated corpus (DDL conformance via the
    // brightfield-sql emitter + .layer2.expected.sql fixtures). Nine of the
    // ten pass; `legends.yaml` declares no data sources, so layer 2 has
    // nothing to be equivalent about and says `pending` rather than banking a
    // green cell behind a zero-byte fixture.
    let (ok, stdout) = run(&["--layers", "2", "--corpus", "curated"]);
    assert!(ok, "layer-2 curated must exit 0; stdout:\n{stdout}");
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY:"))
        .expect("missing SUMMARY footer");
    assert!(summary.contains("failed=0"), "expected failed=0: {summary}");
    assert!(
        summary.contains("passed=9") && summary.contains("pending=1"),
        "expected passed=9 pending=1 on layer-2 curated: {summary}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("legends") && l.contains("no data sources")),
        "the pending cell must say WHY, by name:\n{stdout}"
    );
}

/// The CI step's contract: a full four-layer curated run reports its cell
/// counts per layer, and its exit code is that run's pass/fail.
#[test]
fn dfconf_cli_reports_per_layer_cell_counts() {
    let (ok, stdout) = run(&["--layers", "1,2,3,4", "--corpus", "curated"]);
    assert!(ok, "the full curated run must exit 0; stdout:\n{stdout}");

    let layer_lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("LAYER ")).collect();
    assert_eq!(
        layer_lines.len(),
        4,
        "one cell-count line per layer:\n{stdout}"
    );
    for (n, line) in layer_lines.iter().enumerate() {
        assert!(
            line.starts_with(&format!("LAYER {}:", n + 1)),
            "layers report in order 1..=4: {line}"
        );
        assert!(
            line.contains("cells=10"),
            "ten curated specs per layer: {line}"
        );
    }
    // Layers 1 and 2 are gated; 3 and 4 are accounted for by the registry.
    assert!(
        layer_lines[0].contains("passed=10"),
        "layer 1 gates all ten: {}",
        layer_lines[0]
    );
    assert!(
        layer_lines[2].contains("suppressed=10") && layer_lines[3].contains("suppressed=10"),
        "layers 3 and 4 are suppressed against a written deviation record, not \
         silently pending:\n{stdout}"
    );

    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY:"))
        .expect("missing SUMMARY footer");
    assert!(
        summary.contains("cells=40") && summary.contains("failed=0"),
        "forty cells, none failing: {summary}"
    );
}

/// A non-zero exit is the step's failure signal, so an unrunnable invocation
/// must not look like a green run.
#[test]
fn dfconf_cli_rejects_a_bad_layer_list() {
    let (ok, _stdout) = run(&["--layers", "9", "--corpus", "curated"]);
    assert!(!ok, "a layer outside 1..=4 must exit non-zero");
}
