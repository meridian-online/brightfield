//! The protocol dump arm, proven behaviourally against the REAL binary in the
//! `dump_seam.rs` idiom. The default input is the emitted Protocol+Run
//! contract; the interim manifest parser is the gated offline/fixture fallback
//! (`BRIGHTFIELD_PROTOCOL_OFFLINE=1`), so the fixture tests below set that flag.
//! The dump arm returns before any workspace/window construction (a leak would
//! keep the run loop alive and hang these tests); window mode exits cleanly
//! with a pointer to the dump variable.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn fixture(rel: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/protocol").join(rel);
    assert!(p.exists(), "fixture present at {p:?}");
    p
}

/// Parse the `PNG dumped: … (WxH, N/M non-zero bytes, Z% coverage)` stderr line
/// the dump arm emits, returning `(width, height, coverage_pct)`. This is the
/// observable proof that the composed scene actually received
/// `render_asset_graph`'s output: a mutation that fed `render_to_pixels` a
/// blank/empty scene (or skipped the render) collapses coverage to ~0, and a
/// degenerate/empty layout collapses the dimensions — neither of which the
/// byte-identity/non-empty checks alone can catch.
fn parse_dump_line(stderr: &str) -> (u32, u32, f64) {
    let line = stderr
        .lines()
        .find(|l| l.contains("PNG dumped:"))
        .unwrap_or_else(|| panic!("dump arm printed no `PNG dumped:` line: {stderr}"));
    let inner = line.split('(').nth(1).expect("dims in parens");
    let wh = inner.split(',').next().expect("WxH token");
    let (w, h) = wh.split_once('x').expect("WxH split");
    let w: u32 = w.trim().parse().expect("width");
    let h: u32 = h.trim().parse().expect("height");
    let coverage: f64 = inner
        .split("% coverage")
        .next()
        .and_then(|s| s.rsplit(|c: char| c == ' ' || c == ',').next())
        .and_then(|s| s.trim().parse().ok())
        .expect("coverage percent");
    (w, h, coverage)
}

/// dumping the vendored edgar_gleif Protocol writes a non-empty
/// PNG, byte-identical across two runs (the determinism shape).
#[test]
fn edgar_gleif_dump_is_deterministic() {
    let dir = std::env::temp_dir().join(format!("bf-protocol-dump-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let manifest = fixture("edgar_gleif/arcform.yaml");

    let mut pngs: Vec<Vec<u8>> = Vec::new();
    let mut last_stderr = String::new();
    for run in 0..2 {
        let png_path = dir.join(format!("dag-{run}.png"));
        let _ = fs::remove_file(&png_path);
        let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
            .arg(&manifest)
            .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
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
        last_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        pngs.push(fs::read(&png_path).expect("PNG written"));
    }

    assert!(!pngs[0].is_empty(), "the protocol DAG produced pixels");
    assert!(
        pngs[0] == pngs[1],
        "two dumps of the same manifest must be byte-identical ({} vs {} bytes)",
        pngs[0].len(),
        pngs[1].len()
    );

    // Not a blank canvas: the graph pipeline ran, and the composed scene
    // actually received render_asset_graph's output (graph-scale dimensions +
    // real coverage). This is the assertion that fails if the graph-to-scene
    // wiring is silently a no-op.
    assert!(
        last_stderr.contains("Protocol parsed: edgar_gleif"),
        "the graph was built from the fixture: {last_stderr}"
    );
    let (w, h, coverage) = parse_dump_line(&last_stderr);
    assert!(w > 500 && h > 100, "graph-scale render, not an empty/degenerate layout: {w}x{h}");
    assert!(coverage > 0.0, "the composed scene carries real ink, not a blank/transparent render");

    let _ = fs::remove_dir_all(&dir);
}

/// Pixels half: the degrade fixture — one deliberately
/// unparseable middle statement — still dumps a non-empty PNG (the chip
/// renders; the file is never black-boxed into a failed run).
#[test]
fn degrade_fixture_dumps() {
    let dir = std::env::temp_dir().join(format!("bf-degrade-fixture-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let manifest = fixture("degrade.yaml");
    let png_path = dir.join("degrade.png");
    let _ = fs::remove_file(&png_path);

    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&manifest)
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
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

    // The chip and its lineage actually reached the scene — graph-scale render
    // with real coverage, not a vacuously-passing blank canvas.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Protocol parsed: degrade"), "graph built: {stderr}");
    let (w, h, coverage) = parse_dump_line(&stderr);
    assert!(w > 200 && h > 60, "graph-scale render, not an empty layout: {w}x{h}");
    assert!(coverage > 0.0, "the composed scene carries real ink, not a blank render");

    let _ = fs::remove_dir_all(&dir);
}

/// A protocol-shaped file that is MALFORMED (a step entry that isn't a mapping)
/// must surface a parse error and exit 1 even in window mode — the sniff routes
/// it to the protocol arm, and parsing before the later-card message turns a
/// misleading clean exit into a real diagnostic.
#[test]
fn pds_window_mode_malformed_manifest_exits_nonzero() {
    let dir = std::env::temp_dir().join(format!("bf-pds-badwin-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.yaml");
    // steps: is a sequence (sniffs as protocol) but the entry is a bare string,
    // which fails Step deserialization.
    fs::write(&bad, "name: bad\nsteps:\n  - just_a_string\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&bad)
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
        .env_remove("BRIGHTFIELD_DUMP_PNG")
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");

    assert!(
        !output.status.success(),
        "a malformed protocol manifest must not exit 0 in window mode"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("protocol parse error"),
        "the diagnostic names the parse failure: {stderr}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// a protocol manifest WITHOUT the dump variable (window mode)
/// prints a clear later-card message and exits 0 — no crash, no partial
/// window (a window would keep the run loop alive and hang this test).
#[test]
fn window_mode_prints_later_card_message() {
    let manifest = fixture("edgar_gleif/arcform.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&manifest)
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
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

/// The path to the crate-vendored Protocol+Run contract fixture (the default
/// input) — a real emitted `arc run` sample.
fn contract_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-protocol/fixtures")
        .join(name);
    assert!(p.exists(), "contract fixture present at {p:?}");
    p
}

/// The DEFAULT input is the emitted contract: dumping one (no offline flag)
/// renders a non-empty DAG PNG and names the run.
#[test]
fn contract_is_the_default_input_and_dumps_a_dag() {
    let dir = std::env::temp_dir().join(format!("bf-pds-contract-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let contract = contract_fixture("sample_run.contract.json");
    let png_path = dir.join("contract.png");
    let _ = fs::remove_file(&png_path);

    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&contract)
        .env("BRIGHTFIELD_DUMP_PNG", &png_path)
        .env_remove("BRIGHTFIELD_PROTOCOL_OFFLINE")
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "contract dump exits cleanly: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Protocol run: widgets_demo"),
        "the run was built from the contract (default path): {stderr}"
    );
    let (w, h, coverage) = parse_dump_line(&stderr);
    assert!(w > 40 && h > 40, "graph-scale render, not an empty layout: {w}x{h}");
    assert!(coverage > 0.0, "the composed scene carries real ink");
    assert!(fs::metadata(&png_path).map(|m| m.len()).unwrap_or(0) > 0, "PNG written");

    let _ = fs::remove_dir_all(&dir);
}

/// A raw manifest WITHOUT the offline flag is gated: the arm prints guidance
/// pointing at the contract as the default input and exits 0 without rendering.
#[test]
fn manifest_without_offline_flag_is_gated() {
    let manifest = fixture("edgar_gleif/arcform.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_brightfield"))
        .arg(&manifest)
        .env_remove("BRIGHTFIELD_PROTOCOL_OFFLINE")
        .env_remove("BRIGHTFIELD_DUMP_PNG")
        .env_remove("BRIGHTFIELD_PARAM_OVERRIDE")
        .output()
        .expect("binary runs");
    assert!(output.status.success(), "the gate is guidance, not an error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BRIGHTFIELD_PROTOCOL_OFFLINE"),
        "the gate names the offline flag: {stderr}"
    );
    assert!(
        stderr.contains("contract"),
        "the gate points at the contract as the default input: {stderr}"
    );
    assert!(
        !stderr.contains("PNG dumped:"),
        "a gated manifest renders nothing: {stderr}"
    );
}
