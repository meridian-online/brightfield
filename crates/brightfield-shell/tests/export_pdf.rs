//! `export-pdf`: the shipped binary, run with no window, sets a Protocol's
//! `report.md` as a PDF with each chart its Charts section lists drawn in place.
//!
//! The Protocols are real runs, as in `tests/export_cli.rs`: the CO2 fixture
//! copied into a directory of the test's own and run by `arc`.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use agent_protocol::{copied_fixture, ran_fixture, run_protocol};
use brightfield_shell::export::{prepare_report, stale_tables};

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

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("the binary exited")
}

/// The PDF's text, with runs of white space made one space.
fn pdf_text(pdf: &[u8]) -> String {
    let text = pdf_extract::extract_text_from_mem(pdf).expect("the PDF's text reads");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the previews of the fixture's report go, for a person to look at.
fn preview_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/export-pdf-preview")
}

#[test]
fn the_co2_report_exports() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let file = ran.dir.join("report.pdf");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        file.display().to_string()
    );
    let pdf = std::fs::read(&file).expect("the PDF");
    assert!(pdf.starts_with(b"%PDF"));
    let text = pdf_text(&pdf);
    for wanted in [
        "Every G7 country emits less CO2 per person than it did in 1990",
        "Change in per-capita CO2, 1990 to 2024 (percent)",
        "Per-capita CO2 since 1990, G7 countries vs China and India (tonnes per person)",
    ] {
        assert!(text.contains(wanted), "{wanted} is missing:\n{text}");
    }
    assert!(!text.contains("changed since the run"), "{text}");
    // The line chart's nine series repeat the palette: a warning, not a failure.
    assert!(stderr(&out).contains("colours repeat"), "{}", stderr(&out));

    let report = prepare_report(&ran.dir).expect("the report prepares");
    let pages = brightfield_report::preview_pages(&report.input(), 110.0).expect("the pages draw");
    let previews = preview_dir();
    std::fs::create_dir_all(&previews).expect("the preview folder");
    for (n, page) in pages.iter().enumerate() {
        std::fs::write(previews.join(format!("co2-per-capita-{}.png", n + 1)), page)
            .expect("a preview writes");
    }
}

#[test]
fn the_out_path_is_where_it_writes() {
    let ran = ran_fixture("co2-per-capita");
    let to = ran.dir.join("elsewhere").join("co2.pdf");
    let out = export(&["export-pdf", path_arg(&ran.dir), "--out", path_arg(&to)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(std::fs::read(&to).expect("the PDF").starts_with(b"%PDF"));
}

#[test]
fn no_report_fails() {
    let ran = ran_fixture("co2-per-capita");
    std::fs::remove_file(ran.dir.join("report.md")).expect("the report goes");
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out).trim(),
        format!(
            "no report.md in {}; write one with arc report write",
            ran.dir.display()
        )
    );
}

#[test]
fn a_broken_panel_fails() {
    let ran = ran_fixture("co2-per-capita");
    let file = ran.dir.join(format!("panels/{CHANGE}.yaml"));
    let text = std::fs::read_to_string(&file).expect("the panel");
    std::fs::write(&file, text.replace("x: change_pct", "x: change_percent")).expect("edits");
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains(&format!(
            "panels/{CHANGE}.yaml: a mark reads the column change_percent"
        )),
        "{}",
        stderr(&out)
    );
    assert!(!ran.dir.join("report.pdf").exists());
}

#[test]
fn an_unrun_protocol_fails() {
    let fixture = copied_fixture("co2-per-capita");
    let out = export(&["export-pdf", path_arg(&fixture.dir)]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("has not run yet"), "{}", stderr(&out));
}

#[test]
fn a_changed_table_raises_the_notice() {
    let ran = ran_fixture("co2-per-capita");
    let model = ran.dir.join("models/03_co2_per_capita_change.sql");
    let sql = std::fs::read_to_string(&model).expect("the model");
    let changed = sql.replace(", 1)", ", 0)");
    assert_ne!(sql, changed, "the model rounds to one place");
    std::fs::write(&model, changed).expect("edits");
    run_protocol(&ran.dir);

    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        vec!["co2_per_capita_change".to_string()]
    );
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = pdf_text(&std::fs::read(ran.dir.join("report.pdf")).expect("the PDF"));
    assert!(text.contains("changed since the run"), "{text}");
    assert!(text.contains("co2_per_capita_change"), "{text}");
}

#[test]
fn an_unchanged_rerun_raises_no_notice() {
    let ran = ran_fixture("co2-per-capita");
    agent_protocol::force_run_protocol(&ran.dir);
    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        Vec::<String>::new()
    );
}
