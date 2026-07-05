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
