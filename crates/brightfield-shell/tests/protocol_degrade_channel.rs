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
//!   needs no GPU and localises a break to the report or the summary. Three
//!   tests: that a fault reaches both, what the summary totals do across two
//!   model lengths, and which class each state of the model's parent directory
//!   earns;
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

/// The same lineage, two statements longer. Only the readable render can draw
/// the extra intermediates, so this is the shape where the node totals move —
/// see `the_summary_totals_are_not_a_completeness_check`.
const MODEL_FOUR_STATEMENTS: &str = "CREATE TABLE staged AS SELECT * FROM read_csv('build/a.csv');
CREATE TABLE mid AS SELECT * FROM staged;
CREATE TABLE late AS SELECT * FROM mid;
CREATE TABLE tiered AS SELECT * FROM late;
";

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
    fixture_with_model(name, case, MODEL)
}

/// As [`fixture`], with the model's body chosen by the caller — the knob
/// `the_summary_totals_are_not_a_completeness_check` turns.
fn fixture_with_model(name: &str, case: Case, model: &str) -> (PathBuf, Option<RestoreMode>) {
    let dir = staging_dir(name);
    fs::create_dir_all(dir.join("models")).expect("create fixture");
    fs::write(dir.join("arcform.yaml"), MANIFEST).expect("write manifest");
    if case != Case::Absent {
        fs::write(dir.join(MODEL_PATH), model).expect("write model");
    }
    let guard = (case == Case::Refused).then(|| RestoreMode::set(&dir.join(MODEL_PATH), 0o000));
    (dir, guard)
}

/// An empty directory to stage into, with the manifest not yet written.
///
/// The chmod before the delete is not belt-and-braces: a run interrupted
/// between staging a `000` directory and dropping its guard leaves one behind,
/// and `remove_dir_all` cannot descend into it.
fn staging_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("degrade_channel_{name}"));
    let _ = fs::set_permissions(dir.join("models"), fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture");
    dir
}

/// The `N collapsed / N full nodes, N steps` prefix of a summary line, with the
/// flow and the degrade clause dropped — the part this suite compares.
fn totals(summary: &str) -> String {
    let (head, _) = summary
        .split_once(", Vertical flow")
        .unwrap_or((summary, ""));
    let (_, counts) = head.split_once('(').unwrap_or(("", head));
    counts.to_string()
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

/// Why the summary needs the degrade count and cannot infer it: a caller
/// holding one render has nothing telling it what those totals should have
/// been.
///
/// Two model lengths, deliberately. The claim these tests replace was written
/// at three sites off the two-statement fixture alone and was false on the
/// next model length. A two-statement model makes the degraded totals *equal*
/// to the complete ones; a four-statement model makes them *lower*. Neither is
/// a check a caller holding a single render can perform, and a sentence that
/// names only one of the two shapes is a generalisation from a fixture.
#[test]
fn the_summary_totals_are_not_a_completeness_check() {
    // Every summary this test reads, keyed by (model length, case).
    let read = |name: &str, case: Case, model: &str| -> (String, String) {
        let (dir, guard) = fixture_with_model(name, case, model);
        if case == Case::Refused && refusal_is_impossible(&dir) {
            return (String::new(), String::new());
        }
        let (_, summary) = report_and_summary(&dir);
        drop(guard);
        (totals(&summary), summary)
    };

    // Two statements: the chip stands in for the one intermediate a readable
    // model would have drawn, so nothing on the line moves but the count.
    let (short_complete, short_complete_line) = read("totals_2_complete", Case::Complete, MODEL);
    assert_eq!(
        short_complete, "5 collapsed / 5 full nodes, 3 steps",
        "the control's totals: {short_complete_line}"
    );
    let (short_absent, _) = read("totals_2_absent", Case::Absent, MODEL);
    let (short_refused, refused_line) = read("totals_2_refused", Case::Refused, MODEL);
    assert_eq!(
        short_absent, short_complete,
        "a two-statement model degrades at an UNCHANGED node total"
    );
    if refused_line.is_empty() {
        eprintln!(
            "SKIPPED the refused half of the_summary_totals_are_not_a_completeness_check: \
             this process reads a 000 file (running as root)"
        );
    } else {
        assert_eq!(short_refused, short_complete, "and so does a refused read");
    }

    // Four statements: the readable render draws two more intermediates, so
    // the totals DO move — and still say nothing, because the number they
    // moved from is the one the caller does not have.
    let (long_complete, long_complete_line) =
        read("totals_4_complete", Case::Complete, MODEL_FOUR_STATEMENTS);
    assert_eq!(
        long_complete, "7 collapsed / 7 full nodes, 3 steps",
        "two more statements, two more nodes: {long_complete_line}"
    );
    let (long_absent, _) = read("totals_4_absent", Case::Absent, MODEL_FOUR_STATEMENTS);
    let (long_refused, long_refused_line) =
        read("totals_4_refused", Case::Refused, MODEL_FOUR_STATEMENTS);
    assert_eq!(
        long_absent, "5 collapsed / 5 full nodes, 3 steps",
        "the degraded render is one chip whatever it stood in for"
    );
    assert_ne!(
        long_absent, long_complete,
        "on this model the totals DO separate the two — which is why the \
         unscoped claim that they never move was false"
    );
    if !long_refused_line.is_empty() {
        assert_eq!(long_refused, long_absent, "a refused read, likewise");
    }

    // The step count is the number that moved on none of these renders — it
    // counts the manifest's steps, and a degraded step is still a step.
    for line in [
        &short_complete,
        &short_absent,
        &short_refused,
        &long_complete,
        &long_absent,
        &long_refused,
    ] {
        assert!(
            line.is_empty() || line.ends_with(", 3 steps"),
            "the step count never moves: {line}"
        );
    }
}

/// The classification rule, stated where it is decided: **`unreadable` is
/// claimed on evidence that access was REFUSED, and `absent` is what remains.**
///
/// The states of the directory holding the model, each through the shipped
/// offline loader:
///
/// | `models/`                       | class        | what the reader is told           |
/// |---------------------------------|--------------|-----------------------------------|
/// | does not exist                  | `absent`     | the protocol names a file that is not there |
/// | exists, readable, empty         | `absent`     | the protocol names a file that is not there |
/// | exists, mode `000`, empty       | `unreadable` | the read was refused — access, not authorship |
/// | exists, mode `000`, model inside| `unreadable` | the read was refused — access, not authorship |
///
/// A missing directory reporting *absent* is the deliberate half. There is no
/// grant to widen on a directory that is not there, so *unreadable* would send
/// the reader to check a permission that is not the problem — the same
/// wrong-instruction failure the split exists to remove.
///
/// The two mode-`000` rows are why the badge may claim nothing about the file
/// existing: they differ only in whether the model is inside, and the refused
/// process cannot tell them apart. The empty one is the case that made the old
/// wording false — the badge said "the file is there" for a path this test
/// asserts does not exist.
#[test]
fn the_parent_directorys_state_decides_absent_from_unreadable() {
    // 1. `models/` never created.
    let missing = staging_dir("parent_missing");
    fs::write(missing.join("arcform.yaml"), MANIFEST).expect("write manifest");
    let (missing_report, _) = report_and_summary(&missing);
    assert_eq!(missing_report.len(), 1, "one stand-in: {missing_report:?}");
    assert!(
        missing_report[0].starts_with("degraded step tier: absent: "),
        "a `models/` directory that does not exist is the protocol's problem, \
         not a permission to widen: {:?}",
        missing_report[0]
    );

    // 3. `models/` there and readable, model file never written. (Staged before
    //    case 2 so the mode-000 directory is live for as short a time as
    //    possible.)
    let empty = staging_dir("parent_readable_empty");
    fs::create_dir_all(empty.join("models")).expect("models dir");
    fs::write(empty.join("arcform.yaml"), MANIFEST).expect("write manifest");
    let (empty_report, _) = report_and_summary(&empty);
    assert!(
        empty_report[0].starts_with("degraded step tier: absent: "),
        "a readable directory with nothing in it establishes absence: {:?}",
        empty_report[0]
    );

    // 2. `models/` there at mode 000 and EMPTY — the read is refused before
    //    anything can be learned about what is inside.
    let refused = staging_dir("parent_unreadable_empty");
    fs::create_dir_all(refused.join("models")).expect("models dir");
    fs::write(refused.join("arcform.yaml"), MANIFEST).expect("write manifest");
    let model = refused.join(MODEL_PATH);
    assert!(
        model.symlink_metadata().is_err(),
        "the fixture is the hard case: no model file exists at {}",
        model.display()
    );
    let guard = RestoreMode::set(&refused.join("models"), 0o000);
    if fs::read_to_string(&model).is_ok() || fs::read_dir(refused.join("models")).is_ok() {
        eprintln!(
            "SKIPPED the mode-000 row of the_parent_directorys_state_decides_absent_from_unreadable: \
             this process reads a 000 directory (running as root)"
        );
        return;
    }
    let (refused_report, _) = report_and_summary(&refused);
    drop(guard);
    assert!(
        refused_report[0].starts_with("degraded step tier: unreadable: "),
        "a directory that refuses the read is the reader's to fix: {:?}",
        refused_report[0]
    );
    // The regression this row exists for, asserted before the positive wording
    // so that a re-added presence claim fails on the sentence that names it.
    // `classify_read_failure` declines to claim the file is absent here; on the
    // same evidence nothing downstream may claim it is present — and the file
    // measurably is not.
    assert!(
        !refused_report[0].contains("is there"),
        "the badge must not assert a presence the classifier declined to \
         establish — the file at {} does not exist: {:?}",
        model.display(),
        refused_report[0]
    );
    assert!(
        refused_report[0].contains("the read was refused")
            && refused_report[0].contains("Access, not authorship"),
        "and the badge says so: {:?}",
        refused_report[0]
    );

    // 4. `models/` at mode 000 with the model INSIDE it. Same verdict and the
    //    same badge as the empty one — which is the whole reason the badge may
    //    not speak about the file. These two fixtures differ in exactly the
    //    fact the refused process cannot observe.
    let occupied = staging_dir("parent_unreadable_occupied");
    fs::create_dir_all(occupied.join("models")).expect("models dir");
    fs::write(occupied.join("arcform.yaml"), MANIFEST).expect("write manifest");
    fs::write(occupied.join(MODEL_PATH), MODEL).expect("write model");
    let occupied_guard = RestoreMode::set(&occupied.join("models"), 0o000);
    let (occupied_report, _) = report_and_summary(&occupied);
    drop(occupied_guard);
    assert_eq!(
        occupied_report[0].replace(occupied.to_str().expect("utf-8 path"), "<DIR>"),
        refused_report[0].replace(refused.to_str().expect("utf-8 path"), "<DIR>"),
        "a model inside the refused directory and no model at all are one \
         report, because the refused process cannot tell them apart"
    );

    for dir in [&missing, &empty, &refused, &occupied] {
        let _ = fs::remove_dir_all(dir);
    }
}
