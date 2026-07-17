//! Card 0025: the protocol dump arm, proven behaviourally against the REAL
//! binary in the `dump_seam.rs` idiom. The sniff decides protocol-vs-Mosaic
//! BEFORE spec parsing; the dump arm returns before any workspace/window
//! construction (a leak would keep the run loop alive and hang these tests);
//! window mode on a protocol manifest exits cleanly with a pointer to the
//! dump variable (pds-ac07).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn fixture(rel: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/protocol").join(rel);
    assert!(p.exists(), "fixture present at {p:?}");
    p
}

/// pds-ac01: dumping the vendored edgar_gleif Protocol writes a non-empty
/// PNG, byte-identical across two runs (the aws_ac07 determinism shape).
#[test]
fn pds_ac01_edgar_gleif_dump_is_deterministic() {
    let dir = std::env::temp_dir().join(format!("bf-pds-ac01-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let manifest = fixture("edgar_gleif/arcform.yaml");

    let mut pngs: Vec<Vec<u8>> = Vec::new();
    for run in 0..2 {
        let png_path = dir.join(format!("dag-{run}.png"));
        let _ = fs::remove_file(&png_path);
        let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
            .arg(&manifest)
            .env("BRIGHTFIELD_DUMP_PNG", &png_path)
            .env_remove("BRIGHTFIELD_DUMP_SCALE")
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

    assert!(!pngs[0].is_empty(), "the protocol DAG produced pixels");
    assert!(
        pngs[0] == pngs[1],
        "two dumps of the same manifest must be byte-identical ({} vs {} bytes)",
        pngs[0].len(),
        pngs[1].len()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// pds-ac04 (pixels half): the degrade fixture — one deliberately
/// unparseable middle statement — still dumps a non-empty PNG (the chip
/// renders; the file is never black-boxed into a failed run).
#[test]
fn pds_ac04_degrade_fixture_dumps() {
    let dir = std::env::temp_dir().join(format!("bf-pds-ac04-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let manifest = fixture("degrade.yaml");
    let png_path = dir.join("degrade.png");
    let _ = fs::remove_file(&png_path);

    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&manifest)
        .env("BRIGHTFIELD_DUMP_PNG", &png_path)
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "degrade dump exits cleanly: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let len = fs::metadata(&png_path).map(|m| m.len()).unwrap_or(0);
    assert!(len > 0, "the degrade fixture still renders a PNG");

    let _ = fs::remove_dir_all(&dir);
}

/// pds-ac07: a protocol manifest WITHOUT the dump variable (window mode)
/// prints a clear later-card message and exits 0 — no crash, no partial
/// window (a window would keep the run loop alive and hang this test).
#[test]
fn pds_ac07_window_mode_prints_later_card_message() {
    let manifest = fixture("edgar_gleif/arcform.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&manifest)
        .env_remove("BRIGHTFIELD_DUMP_PNG")
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "window mode exits 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the windowed protocol view lands in a later card"),
        "the message names the gap: {stderr}"
    );
    assert!(
        stderr.contains("BRIGHTFIELD_DUMP_PNG"),
        "the message points at the dump variable: {stderr}"
    );
}
