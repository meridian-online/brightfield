//! Integration tests for the `generate-deviations` binary (ac-11).
//!
//! Three gates:
//!   - determinism: same input → byte-identical output across runs
//!   - empty-registry stub: `(no deviations registered)` on empty input
//!   - drift: committed `DEVIATIONS.md` matches regeneration from
//!     committed `deviations.yaml`

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn bin_path() -> PathBuf {
    let target = PathBuf::from(
        std::env::var("CARGO_BIN_EXE_generate-deviations").expect("cargo exposes bin path"),
    );
    target
}

fn tmp_dir(suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dfconf-gen-{}-{}",
        std::process::id(),
        suffix
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn dfconf_output_is_deterministic() {
    let dir = tmp_dir("det");
    let input = dir.join("deviations.yaml");
    fs::write(
        &input,
        r#"deviations:
  - id: DEV-0001
    surface: legend
    mosaic_behaviour: a
    brightfield_behaviour: b
    rationale: c
    affected_specs: [line.yaml]
    conformance_layers_suppressed: [3]
  - id: DEV-0002
    surface: tooltip
    mosaic_behaviour: x
    brightfield_behaviour: y
    rationale: z
"#,
    )
    .unwrap();
    let out1 = dir.join("out1.md");
    let out2 = dir.join("out2.md");
    let run = |out: &PathBuf| {
        let status = Command::new(bin_path())
            .args([
                "--input",
                input.to_str().unwrap(),
                "--output",
                out.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
    };
    run(&out1);
    run(&out2);
    let a = fs::read(&out1).unwrap();
    let b = fs::read(&out2).unwrap();
    assert_eq!(a, b, "repeated runs must produce byte-identical output");
}

#[test]
fn dfconf_empty_registry_emits_stub() {
    let dir = tmp_dir("empty");
    let input = dir.join("deviations.yaml");
    fs::write(&input, "deviations: []\n").unwrap();
    let output = dir.join("out.md");
    let status = Command::new(bin_path())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let rendered = fs::read_to_string(&output).unwrap();
    assert!(
        rendered.contains("(no deviations registered)"),
        "empty registry must emit stub, got:\n{rendered}"
    );
}

#[test]
fn dfconf_committed_deviations_md_is_current() {
    let root = repo_root();
    let input = root.join("deviations.yaml");
    let committed = root.join("DEVIATIONS.md");
    assert!(input.exists(), "deviations.yaml missing at repo root");
    assert!(committed.exists(), "DEVIATIONS.md missing at repo root");
    let dir = tmp_dir("drift");
    let out = dir.join("regen.md");
    let status = Command::new(bin_path())
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let expected = fs::read(&committed).unwrap();
    let actual = fs::read(&out).unwrap();
    assert_eq!(
        expected, actual,
        "committed DEVIATIONS.md is drift from deviations.yaml — run `cargo run --bin generate-deviations`"
    );
}
