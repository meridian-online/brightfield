//! aws_ac01 (card 0017): the `BRIGHTFIELD_DUMP_PNG` path returns before any
//! workspace/dock construction — proven behaviourally against the REAL
//! binary. The dump arm of `main` returns before the window path is
//! reachable; if shell/dock construction leaked into it, the GPUI run loop
//! would keep the process alive and this test would hang instead of
//! observing a clean exit with a written PNG. (The decision itself is the
//! `boot` module's seam, pinned by its own unit test.)

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SPEC: &str = r#"
data:
  points:
    - { x: 1, y: 2 }
    - { x: 2, y: 3 }
    - { x: 3, y: 1 }
plot:
  - mark: dot
    data: { from: points }
    x: x
    y: y
"#;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-aws-ac01-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn aws_ac01_dump_mode_exits_before_workspace_construction() {
    let dir = temp_dir();
    let spec_path = dir.join("seam.yaml");
    fs::write(&spec_path, SPEC).unwrap();
    let png_path = dir.join("seam.png");
    let _ = fs::remove_file(&png_path);

    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&spec_path)
        .env("BRIGHTFIELD_DUMP_PNG", &png_path)
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");

    assert!(
        output.status.success(),
        "dump mode exits cleanly (no run loop reached): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let len = fs::metadata(&png_path).map(|m| m.len()).unwrap_or(0);
    assert!(len > 0, "the PNG was written before the process returned");

    let _ = fs::remove_dir_all(&dir);
}

/// aws_ac07 / aws_ac01 (determinism pin): the dump binary run TWICE on the
/// same spec writes byte-identical PNGs — the behavioural form of "no
/// shell, presentation, or other ambient state can move a pixel in the
/// dump path". This replaces a vacuous repeated-call purity probe on the
/// visibility mapping (purity there is a property of the fn signature;
/// only the real binary can pin the pixels). The spec is a pre-existing
/// multi-plot example, deliberately NON-raster: the raster family's
/// GROUP BY row order is not byte-stable run-to-run on this branch (the
/// determinism chore lands separately).
#[test]
fn aws_ac07_dump_run_twice_is_byte_identical() {
    // Own directory (not `temp_dir()`): the aws_ac01 test removes its
    // directory on completion, and the two tests run in parallel.
    let dir = std::env::temp_dir().join(format!("bf-aws-ac07-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let spec_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/dashboard.yaml");
    assert!(spec_path.exists(), "example spec present at {spec_path:?}");

    let mut pngs: Vec<Vec<u8>> = Vec::new();
    for run in 0..2 {
        let png_path = dir.join(format!("twice-{run}.png"));
        let _ = fs::remove_file(&png_path);
        let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
            .arg(&spec_path)
            .env("BRIGHTFIELD_DUMP_PNG", &png_path)
            .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
            .output()
            .expect("binary runs");
        assert!(
            output.status.success(),
            "dump run {run} exits cleanly: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        pngs.push(fs::read(&png_path).expect("PNG written"));
    }

    assert!(!pngs[0].is_empty(), "runs produced pixels");
    assert!(
        pngs[0] == pngs[1],
        "two dumps of the same spec must be byte-identical ({} vs {} bytes)",
        pngs[0].len(),
        pngs[1].len()
    );

    let _ = fs::remove_dir_all(&dir);
}
