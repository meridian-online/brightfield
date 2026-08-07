//! AC4: a shipped run tells a complete render from a degraded one **without
//! anyone comparing pictures**.
//!
//! `brightfield-protocol`'s `degrades` already enumerated the stand-ins, and
//! its own tests already asserted the classes. That is not the same thing as a
//! channel: measured on this branch's first round, `brightfield-shot` over
//! three fixtures — models readable, models absent, models refused — exited 0
//! three times and printed the same summary line each time,
//! `protocol acc (5 collapsed / 5 full nodes, 3 steps, Vertical flow)`. Three
//! different PNGs, one indistinguishable stderr. A function that exists is not
//! a function that runs, so this suite holds the **binary** to the claim, not
//! the library.
//!
//! Two tiers, deliberately:
//!
//! * the pure one — `ProtocolInputs::degrade_report` off the same offline
//!   loader `--spec` uses, and the summary line `Boot::describe` builds — which
//!   needs no GPU and localises a break to the report or the summary;
//! * the shipped one — the real `brightfield-shot` binary over the same three
//!   directories, which is the only tier that can catch the caller being
//!   dropped again.
//!
//! Unix only: a refused read is staged with `chmod`, and there is no portable
//! equivalent. The one CI runner this repo builds on is macOS.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use brightfield_protocol::layout::Flow;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::window::Boot;
use brightfield_workbench::ViewKind;

/// Three steps, one of them a `sql:` model — the smallest shape that can
/// degrade. Deliberately the same manifest
/// `crates/brightfield-protocol/tests/model_source_faults.rs` builds its graphs
/// from, so the two suites are answering about one fixture at two tiers.
const MANIFEST: &str = r"
name: acc
steps:
  - name: fetch
    op: http_fetch@1
    with:
      url: https://example.com/a.csv
      out: build/a.csv
  - name: tier
    sql: models/entity_tiering_rules.sql
    depends_on: [build/a.csv]
  - name: export
    op: parquet_export@1
    with:
      input: tiered
      dest: build/tiered.parquet
";

const MODEL: &str = "CREATE TABLE staged AS SELECT * FROM read_csv('build/a.csv');\n\
                     CREATE TABLE tiered AS SELECT * FROM staged;\n";

const MODEL_PATH: &str = "models/entity_tiering_rules.sql";

/// Restore a path's mode when the scope ends — including on a panic. A test
/// that leaves a `000` path behind poisons every test after it and the next
/// `cargo` invocation's cleanup with it.
struct RestoreMode {
    path: PathBuf,
    mode: u32,
}

impl RestoreMode {
    fn set(path: &Path, mode: u32) -> Self {
        let was = fs::metadata(path).expect("stat before chmod").permissions();
        let guard = Self {
            path: path.to_path_buf(),
            mode: was.mode(),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
        guard
    }
}

impl Drop for RestoreMode {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

/// Which fault a fixture is staged with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    /// Models readable — the control, and the render that is complete.
    Complete,
    /// The model file the manifest names was never written.
    Absent,
    /// The model file is there and the read is refused.
    Refused,
}

/// A protocol directory staged for `case`, plus the mode guard the refused case
/// needs held for as long as the directory is read.
fn fixture(name: &str, case: Case) -> (PathBuf, Option<RestoreMode>) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("degrade_channel_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("models")).expect("create fixture");
    fs::write(dir.join("arcform.yaml"), MANIFEST).expect("write manifest");
    if case != Case::Absent {
        fs::write(dir.join(MODEL_PATH), MODEL).expect("write model");
    }
    let guard = (case == Case::Refused).then(|| RestoreMode::set(&dir.join(MODEL_PATH), 0o000));
    (dir, guard)
}

/// True when this process reads `path` whatever its mode — running as root,
/// where `chmod 000` refuses nobody and the refused half cannot be staged.
fn refusal_is_impossible(dir: &Path) -> bool {
    fs::read_to_string(dir.join(MODEL_PATH)).is_ok()
}

/// The report and the summary line, off the same offline load `--spec` performs.
fn report_and_summary(dir: &Path) -> (Vec<String>, String) {
    let spec = dir.join("arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 fixture path"))
        .expect("the fixture manifest loads");
    let report = inputs.degrade_report();
    let summary = Boot::protocol(inputs, Flow::Vertical, None).describe(ViewKind::Protocol);
    (report, summary)
}

/// AC4, pure tier. A complete render reports nothing and its summary claims
/// nothing; each fault reports exactly one stand-in, names its class at the
/// head of the line, and moves the summary.
#[test]
fn the_report_and_the_summary_separate_complete_from_degraded() {
    let (complete_dir, _) = fixture("pure_complete", Case::Complete);
    let (report, summary) = report_and_summary(&complete_dir);
    assert!(
        report.is_empty(),
        "a complete render reports nothing: {report:?}"
    );
    assert!(
        !summary.contains("degraded"),
        "the summary of a complete render claims no degrade: {summary}"
    );

    let (absent_dir, _) = fixture("pure_absent", Case::Absent);
    let (absent, absent_summary) = report_and_summary(&absent_dir);
    assert_eq!(absent.len(), 1, "one stand-in: {absent:?}");
    assert!(
        absent[0].starts_with("degraded step tier: absent: "),
        "the line leads with the step and the class: {:?}",
        absent[0]
    );
    assert!(
        absent[0].ends_with(
            " — the protocol names a model file that is not there, so this step is drawn as one \
             chip in place of the statements inside it."
        ) && absent[0].contains(MODEL_PATH),
        "and closes with the path and what it means: {:?}",
        absent[0]
    );
    assert!(
        absent_summary.contains(", 1 degraded"),
        "the summary carries the count: {absent_summary}"
    );

    let (refused_dir, guard) = fixture("pure_refused", Case::Refused);
    if refusal_is_impossible(&refused_dir) {
        eprintln!(
            "SKIPPED the refused half of the_report_and_the_summary_separate_complete_from_degraded: \
             this process reads a 000 file (running as root)"
        );
        return;
    }
    let (refused, refused_summary) = report_and_summary(&refused_dir);
    assert_eq!(refused.len(), 1, "one stand-in: {refused:?}");
    assert!(
        refused[0].starts_with("degraded step tier: unreadable: "),
        "a refused read is not reported as an absent file — the reader can fix \
         one of them: {:?}",
        refused[0]
    );
    assert!(
        refused[0].contains("Access, not authorship"),
        "the line says whose problem it is: {:?}",
        refused[0]
    );
    assert_ne!(
        absent, refused,
        "absent and refused are two reports, not one"
    );
    assert!(
        refused_summary.contains(", 1 degraded"),
        "the summary carries the count: {refused_summary}"
    );

    drop(guard);
    for dir in [&complete_dir, &absent_dir, &refused_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// `brightfield-shot` over a fixture: its exit status, and its stderr.
fn shot(dir: &Path, name: &str) -> (Option<i32>, String, PathBuf) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("degrade_shot_{name}.png"));
    let _ = fs::remove_file(&out);
    let result = Command::new(env!("CARGO_BIN_EXE_brightfield-shot"))
        .arg("--spec")
        .arg(dir.join("arcform.yaml"))
        .arg("--out")
        .arg(&out)
        // The manifest has no run behind it, which is the state the whole
        // offline path exists for — and which the binary refuses without this.
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
        .output()
        .expect("the brightfield-shot binary runs");
    (
        result.status.code(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        out,
    )
}

/// AC4, shipped tier. The measurement that refuted round one, run again: three
/// fixtures through the real binary. Each still writes its PNG and still exits
/// 0 — the degrade must not become a hard failure — and the stderr of the three
/// is now three different things, with the class in it.
///
/// This test is the one that reddens if the caller is dropped. The pure tier
/// above passes on a `degrade_report` nothing calls.
#[test]
fn the_shipped_binary_reports_the_degrade_and_still_writes_the_png() {
    let (complete_dir, _) = fixture("shot_complete", Case::Complete);
    let (complete_code, complete_err, complete_png) = shot(&complete_dir, "complete");
    assert_eq!(
        complete_code,
        Some(0),
        "the control render must succeed — without it nothing below is a \
         measurement of the degrade. stderr:\n{complete_err}"
    );
    assert!(
        fs::metadata(&complete_png).is_ok_and(|m| m.len() > 0),
        "the control wrote a PNG"
    );
    assert!(
        !complete_err.contains("degraded"),
        "a complete render says nothing about degrades: {complete_err}"
    );

    let (absent_dir, _) = fixture("shot_absent", Case::Absent);
    let (absent_code, absent_err, absent_png) = shot(&absent_dir, "absent");
    assert_eq!(
        absent_code,
        Some(0),
        "a degraded render still succeeds — drawing what it can is the design, \
         and 0 here means the PNG was written. stderr:\n{absent_err}"
    );
    assert!(
        fs::metadata(&absent_png).is_ok_and(|m| m.len() > 0),
        "the degraded render still wrote its PNG"
    );
    assert!(
        absent_err.contains("degraded step tier: absent: "),
        "the run names what it could not draw and why: {absent_err}"
    );
    assert!(
        absent_err.contains(", 1 degraded)"),
        "the summary line no longer reads as complete: {absent_err}"
    );

    let (refused_dir, guard) = fixture("shot_refused", Case::Refused);
    if refusal_is_impossible(&refused_dir) {
        eprintln!(
            "SKIPPED the refused half of \
             the_shipped_binary_reports_the_degrade_and_still_writes_the_png: this process reads \
             a 000 file (running as root)"
        );
        return;
    }
    let (refused_code, refused_err, refused_png) = shot(&refused_dir, "refused");
    assert_eq!(
        refused_code,
        Some(0),
        "a refused model is still a render: {refused_err}"
    );
    assert!(
        fs::metadata(&refused_png).is_ok_and(|m| m.len() > 0),
        "the refused render still wrote its PNG"
    );
    assert!(
        refused_err.contains("degraded step tier: unreadable: "),
        "the run says the read was refused, not that the file is missing: {refused_err}"
    );
    assert!(
        refused_err.contains("Access, not authorship"),
        "and says whose problem that is: {refused_err}"
    );

    // The refutation, inverted. Round one's three runs differed only in the
    // spec path they echoed; strip that and they were byte-identical.
    let strip = |err: &str, dir: &Path| err.replace(dir.to_str().expect("utf-8 path"), "<DIR>");
    let (c, a, r) = (
        strip(&complete_err, &complete_dir),
        strip(&absent_err, &absent_dir),
        strip(&refused_err, &refused_dir),
    );
    assert_ne!(c, a, "complete and absent no longer print the same run");
    assert_ne!(c, r, "complete and refused no longer print the same run");
    assert_ne!(a, r, "absent and refused no longer print the same run");

    drop(guard);
    for dir in [&complete_dir, &absent_dir, &refused_dir] {
        let _ = fs::remove_dir_all(dir);
    }
    for png in [&complete_png, &absent_png, &refused_png] {
        let _ = fs::remove_file(png);
    }
}
