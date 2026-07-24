//! Integration tests for the `brightfield-shell` binary's `--version` / `--help`.
//!
//! Both must answer on stdout and exit 0 **without opening a window**. Asserting
//! "no window" without a display is done by a proxy that needs none: the
//! window-open path's first act is `eprintln!("layout: …")` (`main` reads the
//! saved layout before it builds the viewport), so the absence of that line from
//! stderr means execution short-circuited above it — the spec boot, the
//! viewport and `eframe::run_native` were never reached.
//!
//! Running the real binary here is safe precisely because these flags exit
//! before any GPU or windowing code: nothing is rendered and no window appears.

use std::path::PathBuf;
use std::process::Command;

fn bin_path() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_brightfield-shell").expect("cargo exposes the bin path"),
    )
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(bin_path())
        .args(args)
        .output()
        .expect("the brightfield-shell binary runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn version_prints_crate_version_and_opens_no_window() {
    let (ok, stdout, stderr) = run(&["--version"]);
    assert!(ok, "--version must exit 0; stderr:\n{stderr}");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout should carry the crate version {}, got: {stdout:?}",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        !stderr.contains("layout:"),
        "the window-open path must never be reached; stderr:\n{stderr}"
    );
}

#[test]
fn help_prints_usage_and_opens_no_window() {
    let (ok, stdout, stderr) = run(&["--help"]);
    assert!(ok, "--help must exit 0; stderr:\n{stderr}");
    assert!(
        stdout.contains("Usage:"),
        "stdout should carry a usage summary, got: {stdout:?}"
    );
    assert!(
        !stderr.contains("layout:"),
        "the window-open path must never be reached; stderr:\n{stderr}"
    );
}
