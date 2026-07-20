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
    // Layer 2 is now active for curated corpus (DDL conformance via
    // brightfield-sql emitter + .layer2.expected.sql fixtures).
    let (ok, stdout) = run(&["--layers", "2", "--corpus", "curated"]);
    assert!(ok, "layer-2 curated must exit 0; stdout:\n{stdout}");
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY:"))
        .expect("missing SUMMARY footer");
    assert!(summary.contains("failed=0"), "expected failed=0: {summary}");
    assert!(
        summary.contains("passed=10"),
        "expected passed=10 on layer-2 curated: {summary}"
    );
}
