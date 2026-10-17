//! `export-png` and `export-data`: the shipped binary, run with no window,
//! writes a panel's picture or the rows it draws from a Protocol that has run.
//!
//! The Protocols are real runs, as in `tests/protocol_panels.rs`: the CO2
//! fixture copied into a directory of the test's own and run by `arc`.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::Path;
use std::process::{Command, Output};

use agent_protocol::{copied_fixture, ran_fixture};
use brightfield_shell::capture::render_composed;
use brightfield_shell::protocol_panels::load;
use brightfield_spec::layout::Rect;

const LINE: &str = "co2_per_capita_since_1990";
const CHANGE: &str = "co2_per_capita_change_pct";

fn export(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brightfield-shell"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn path_arg(p: &Path) -> &str {
    p.to_str().expect("a UTF-8 path")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("the binary exited")
}

#[test]
fn png_of_a_panel() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&ran.dir), LINE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let png = ran.dir.join("exports").join(format!("{LINE}.png"));
    assert_eq!(stdout(&out).trim(), png.display().to_string());

    let mut panels = load(&ran.dir, Rect::zero()).expect("the Protocol opens");
    let index = panels
        .tiles
        .iter()
        .position(|t| t.file == Path::new(&format!("panels/{LINE}.yaml")))
        .expect("the line panel");
    let composed = panels.tiles.remove(index).outcome.expect("it draws");
    let (w, h) = (composed.width, composed.height);
    assert_eq!((w, h), (760, 440), "the panel declares its size");
    let want = render_composed(composed, 2.0).expect("renders");

    let got = image::open(&png).expect("a PNG").to_rgba8();
    assert_eq!(got.dimensions(), (w * 2, h * 2));
    assert_eq!(got.dimensions(), want.dimensions());
    assert!(got.as_raw() == want.as_raw(), "the pixels differ");
}

#[test]
fn png_takes_a_scale_and_an_out_path() {
    let ran = ran_fixture("co2-per-capita");
    let to = ran.dir.join("elsewhere").join("one.png");
    let out = export(&[
        "export-png",
        path_arg(&ran.dir.join("arcform.yaml")),
        &format!("panels/{LINE}.yaml"),
        "--scale",
        "1",
        "--out",
        path_arg(&to),
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let got = image::open(&to).expect("a PNG");
    let panels = load(&ran.dir, Rect::zero()).expect("opens");
    let composed = panels
        .tiles
        .iter()
        .find_map(|t| {
            t.outcome
                .as_ref()
                .ok()
                .filter(|_| t.file.ends_with(format!("{LINE}.yaml")))
        })
        .expect("the line panel");
    assert_eq!(
        (got.width(), got.height()),
        (composed.width, composed.height)
    );
}

#[test]
fn data_of_a_panel() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-data", path_arg(&ran.dir), CHANGE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let csv = ran.dir.join("exports").join("co2_per_capita_change.csv");
    assert_eq!(stdout(&out).trim(), csv.display().to_string());
    let text = std::fs::read_to_string(&csv).expect("the CSV");
    assert_eq!(text.lines().count(), 10, "a header and 9 rows:\n{text}");
}

#[test]
fn parquet_data() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&[
        "export-data",
        path_arg(&ran.dir),
        CHANGE,
        "--format",
        "parquet",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let file = ran
        .dir
        .join("exports")
        .join("co2_per_capita_change.parquet");
    let conn = duckdb::Connection::open_in_memory().expect("DuckDB");
    let rows: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM read_parquet('{}')", file.display()),
            [],
            |r| r.get(0),
        )
        .expect("DuckDB reads it back");
    assert_eq!(rows, 9);
}

#[test]
fn unknown_panel_fails() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&ran.dir), "no_such_panel"]);
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("no_such_panel"), "{err}");
    assert!(err.contains(LINE) && err.contains(CHANGE), "{err}");
}

#[test]
fn broken_panel_fails() {
    let ran = ran_fixture("co2-per-capita");
    let file = ran.dir.join(format!("panels/{CHANGE}.yaml"));
    let text = std::fs::read_to_string(&file).expect("the panel");
    std::fs::write(&file, text.replace("x: change_pct", "x: change_percent")).expect("edits");
    for args in [
        ["export-png", path_arg(&ran.dir), CHANGE],
        ["export-data", path_arg(&ran.dir), CHANGE],
    ] {
        let out = export(&args);
        assert_eq!(code(&out), 1);
        assert!(
            stderr(&out).contains(&format!(
                "panels/{CHANGE}.yaml: a mark reads the column change_percent"
            )),
            "{}",
            stderr(&out)
        );
        assert!(stdout(&out).is_empty());
    }
}

#[test]
fn unrun_protocol_fails() {
    let fixture = copied_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&fixture.dir), LINE]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("has not run yet"), "{}", stderr(&out));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        &["export-png"][..],
        &["export-png", "somewhere"],
        &["export-data", "somewhere", "p", "--format", "xlsx"],
        &["export-png", "somewhere", "p", "--scale", "0"],
        &["export-png", "somewhere", "p", "--bogus"],
    ] {
        let out = export(args);
        assert_eq!(code(&out), 2, "{args:?}: {}", stderr(&out));
        assert!(!stderr(&out).is_empty());
    }
}

#[test]
fn help_lists_the_exports() {
    let out = export(&["--help"]);
    assert_eq!(code(&out), 0);
    let help = stdout(&out);
    for name in ["export-pdf", "export-png", "export-data"] {
        assert!(help.contains(name), "{name} is missing:\n{help}");
    }
}
