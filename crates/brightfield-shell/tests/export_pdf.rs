//! `export-pdf`: the shipped binary, run with no window, sets a Protocol's
//! `report.md` as a PDF with each chart its Charts section lists drawn in place.
//!
//! The Protocols are real runs, as in `tests/export_cli.rs`: the CO2 fixture
//! copied into a directory of the test's own and run by `arc`.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use agent_protocol::{copied_fixture, force_run_protocol, ran_fixture, run_protocol, write_report};
use brightfield_report::{listed_charts, reported_run};
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
    // The text extraction runs the heading and the line under it together.
    let opening = text.get(..120).unwrap_or(&text);
    assert!(
        opening.starts_with("Per-capita CO2 since 1990: G7 countries, China and India")
            && opening.contains("Run finished 16 October 2026"),
        "the report opens with the README's heading and the run's date:\n{text}"
    );
    for wanted in [
        "Every G7 country emits less CO2 per person than it did in 1990",
        "9 series share 8 colours, so the last 2 are drawn as one",
        "Change in per-capita CO2, 1990 to 2024 (percent)",
        "Per-capita CO2 since 1990, G7 countries vs China and India (tonnes per person)",
    ] {
        assert!(text.contains(wanted), "{wanted} is missing:\n{text}");
    }
    assert!(!text.contains("changed since the run"), "{text}");
    // The line chart's nine series fold two into "Other": a warning, not a failure.
    assert!(stderr(&out).contains("drawn as one"), "{}", stderr(&out));

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

/// Round the change table's percentages to whole numbers, which changes its
/// content hash.
fn change_the_change_table(dir: &Path) {
    let model = dir.join("models/03_co2_per_capita_change.sql");
    let sql = std::fs::read_to_string(&model).expect("the model");
    let changed = sql.replace(", 1)", ", 0)");
    assert_ne!(sql, changed, "the model rounds to one place");
    std::fs::write(&model, changed).expect("edits");
}

const PROSE: &str = "Every G7 country emits less CO2 per person than it did in 1990.\n";

#[test]
fn a_change_after_arc_writes_the_report_raises_the_notice() {
    let ran = ran_fixture("co2-per-capita");
    write_report(&ran.dir, PROSE, None);
    let run = arc::spec::report_run(&ran.dir)
        .expect("arc reads the report")
        .expect("the report names its run");
    change_the_change_table(&ran.dir);
    run_protocol(&ran.dir);

    // The report's run is in arc's history, so its hashes are arc's record.
    arc::spec::run_tables(&ran.dir, Some(&run)).expect("arc holds the report's run");
    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        vec!["co2_per_capita_change".to_string()]
    );
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = pdf_text(&std::fs::read(ran.dir.join("report.pdf")).expect("the PDF"));
    assert!(text.contains("changed since the run"), "{text}");
    assert!(text.contains(&run), "{text}");
}

#[test]
fn a_report_with_crlf_line_endings_raises_the_same_notice() {
    let ran = ran_fixture("co2-per-capita");
    write_report(&ran.dir, PROSE, None);
    let file = ran.dir.join("report.md");
    let lf = std::fs::read_to_string(&file).expect("the report");
    std::fs::write(&file, lf.replace('\n', "\r\n")).expect("rewrites the report");
    let run = arc::spec::report_run(&ran.dir)
        .expect("arc reads the report")
        .expect("the report names its run");
    change_the_change_table(&ran.dir);
    run_protocol(&ran.dir);

    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        vec!["co2_per_capita_change".to_string()]
    );
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = pdf_text(&std::fs::read(ran.dir.join("report.pdf")).expect("the PDF"));
    assert!(text.contains("changed since the run"), "{text}");
    assert!(text.contains(&run), "{text}");
    assert!(text.contains("Run finished"), "{text}");
}

#[test]
fn an_unchanged_rerun_after_arc_writes_the_report_raises_none() {
    let ran = ran_fixture("co2-per-capita");
    write_report(&ran.dir, PROSE, None);
    force_run_protocol(&ran.dir);
    let run = arc::spec::report_run(&ran.dir)
        .expect("arc reads the report")
        .expect("the report names its run");
    let newest = arc::spec::run_tables(&ran.dir, None).expect("the newest run");
    assert_ne!(newest.run_id, run, "the forced run is a new run");
    arc::spec::run_tables(&ran.dir, Some(&run)).expect("arc holds the report's run");
    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        Vec::<String>::new()
    );
}

#[test]
fn a_report_from_another_machine_falls_back_on_its_own_hashes() {
    // The fixture's report names the run the agent made on its own machine,
    // which this test's history does not hold.
    let ran = ran_fixture("co2-per-capita");
    let run = arc::spec::report_run(&ran.dir)
        .expect("arc reads the report")
        .expect("the report names its run");
    let refused = arc::spec::run_tables(&ran.dir, Some(&run)).expect_err("no such run here");
    assert!(
        refused.to_string().starts_with("there is no run "),
        "arc's wording changed: {refused}"
    );
    assert_eq!(
        stale_tables(&ran.dir).expect("the report's own hashes compare"),
        Vec::<String>::new()
    );
}

#[test]
fn arc_writes_what_the_parsers_read() {
    let ran = ran_fixture("co2-per-capita");
    write_report(&ran.dir, PROSE, None);
    let report = std::fs::read_to_string(ran.dir.join("report.md")).expect("the report");

    let listed = listed_charts(&report);
    let paths: Vec<&str> = listed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "panels/co2_per_capita_change_pct.yaml",
            "panels/co2_per_capita_since_1990.yaml"
        ]
    );
    assert_eq!(
        listed[0].title,
        "Change in per-capita CO2, 1990 to 2024 (percent)"
    );
    assert_eq!(
        listed[1].title,
        "Per-capita CO2 since 1990, G7 countries vs China and India (tonnes per person)"
    );
    assert!(listed.iter().all(|c| c.checks.is_empty()));

    let run = reported_run(&report);
    let newest = arc::spec::run_tables(&ran.dir, None).expect("the newest run");
    assert_eq!(
        arc::spec::report_run(&ran.dir).expect("arc reads the report"),
        Some(newest.run_id.clone())
    );
    assert!(run.finished.is_some(), "{report}");
    assert_eq!(run.tables, newest.tables, "{report}");
}

#[test]
fn a_check_line_arc_writes_fails_the_export() {
    let ran = ran_fixture("co2-per-capita");
    let file = ran.dir.join(format!("panels/{CHANGE}.yaml"));
    let text = std::fs::read_to_string(&file).expect("the panel");
    std::fs::write(&file, text.replace("x: change_pct", "x: change_percent")).expect("edits");
    write_report(&ran.dir, PROSE, None);
    let report = std::fs::read_to_string(ran.dir.join("report.md")).expect("the report");
    let listed = listed_charts(&report);
    assert_eq!(listed[0].checks.len(), 1, "{report}");
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains(&format!("panels/{CHANGE}.yaml: report.md says Check: ")),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_caption_is_the_panel_title_now() {
    let ran = ran_fixture("co2-per-capita");
    let file = ran.dir.join(format!("panels/{CHANGE}.yaml"));
    let text = std::fs::read_to_string(&file).expect("the panel");
    std::fs::write(
        &file,
        text.replace(
            "title: Change in per-capita CO2, 1990 to 2024 (percent)",
            "title: How per-capita CO2 moved since 1990",
        ),
    )
    .expect("edits");
    let out = export(&["export-pdf", path_arg(&ran.dir)]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = pdf_text(&std::fs::read(ran.dir.join("report.pdf")).expect("the PDF"));
    assert!(
        text.contains("How per-capita CO2 moved since 1990"),
        "{text}"
    );
    assert!(
        !text.contains("Change in per-capita CO2, 1990 to 2024 (percent)"),
        "{text}"
    );
}

#[test]
fn a_changed_table_raises_the_notice() {
    let ran = ran_fixture("co2-per-capita");
    change_the_change_table(&ran.dir);
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
    force_run_protocol(&ran.dir);
    assert_eq!(
        stale_tables(&ran.dir).expect("the hashes compare"),
        Vec::<String>::new()
    );
}
