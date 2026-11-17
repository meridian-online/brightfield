//! A model the system refuses is reported as refused access, in the window and by the headless
//! capture's exit status, in both editions; a model that is absent is not.
//!
//! The window tier opens the Protocol into a headless window and reads its banners. The shipped
//! tier runs the real `brightfield-shot` binary and reads its exit status, which is the only
//! tier that catches the exit being dropped.
//!
//! Unix only: a refused read is staged with `chmod`. Skipped as root, where `chmod 000` refuses
//! nobody.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::refusal::refusal_in_engine_text;
use brightfield_shell::window::{Boot, MeridianApp, REFUSED_TITLE};
use brightfield_workbench::RunState;

/// Three steps, one of them a `sql:` model: the smallest Protocol a model read can degrade.
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

/// Puts a folder's mode back when the scope ends, including on a panic, so a `000` folder is
/// never left behind for the next run's cleanup to trip on.
struct RestoreMode(PathBuf);

impl Drop for RestoreMode {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// A Protocol folder named `name`, its model written, and `models/` at mode `000` when
/// `refused`; with `absent`, the model is never written.
fn protocol(name: &str, refused: bool, absent: bool) -> (PathBuf, Option<RestoreMode>) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("refused_access_{name}"));
    let _ = fs::set_permissions(dir.join("models"), fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("models")).expect("create the Protocol folder");
    fs::write(dir.join("arcform.yaml"), MANIFEST).expect("write the manifest");
    if !absent {
        fs::write(dir.join("models/entity_tiering_rules.sql"), MODEL).expect("write the model");
    }
    let guard = refused.then(|| {
        let models = dir.join("models");
        fs::set_permissions(&models, fs::Permissions::from_mode(0o000)).expect("chmod 000");
        RestoreMode(models)
    });
    (dir, guard)
}

/// True when this process reads a `000` folder anyway, running as root.
fn refusal_is_impossible(dir: &Path) -> bool {
    fs::read_dir(dir.join("models")).is_ok()
}

fn window_over(dir: &Path) -> MeridianApp {
    let spec = dir.join("arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 path")).expect("loads");
    MeridianApp::headless(Boot::protocol(inputs, Flow::Vertical, None), Mode::Light)
}

fn shot(dir: &Path, name: &str) -> (Option<i32>, String) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("refused_shot_{name}.png"));
    let _ = fs::remove_file(&out);
    let result = Command::new(env!("CARGO_BIN_EXE_brightfield-shot"))
        .arg("--spec")
        .arg(dir.join("arcform.yaml"))
        .arg("--out")
        .arg(&out)
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
        .output()
        .expect("the brightfield-shot binary runs");
    assert!(
        fs::metadata(&out).is_ok_and(|m| m.len() > 0),
        "the PNG is written whatever the exit"
    );
    let _ = fs::remove_file(&out);
    (
        result.status.code(),
        String::from_utf8_lossy(&result.stderr).to_string(),
    )
}

#[test]
fn a_refused_model_raises_the_banner_naming_its_path() {
    let (dir, guard) = protocol("window", true, false);
    if refusal_is_impossible(&dir) {
        eprintln!("SKIPPED: this process reads a 000 folder (running as root)");
        return;
    }
    let app = window_over(&dir);
    drop(guard);

    let banners: Vec<_> = app
        .notifications()
        .iter()
        .filter(|n| n.title == REFUSED_TITLE)
        .collect();
    assert_eq!(banners.len(), 1, "one refused-access banner");
    let body = banners[0].body.clone().unwrap_or_default();
    let models = dir.join("models");
    assert!(
        body.contains(&format!("{}/", models.display())),
        "the banner names the refused path under models/: {body}"
    );
    assert_eq!(app.refused().len(), 1);
    assert_ne!(
        app.shown_run_state(),
        RunState::Fresh,
        "the Protocol is not shown as complete while the banner stands"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_absent_model_raises_no_refused_banner() {
    let (dir, _) = protocol("window_absent", false, true);
    let app = window_over(&dir);
    assert!(
        app.notifications().iter().all(|n| n.title != REFUSED_TITLE),
        "an absent model is the author's to fix, not a refusal"
    );
    assert!(app.refused().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_headless_path_exits_3_on_a_refused_model() {
    let (dir, guard) = protocol("shot_refused", true, false);
    if refusal_is_impossible(&dir) {
        eprintln!("SKIPPED: this process reads a 000 folder (running as root)");
        return;
    }
    let (code, err) = shot(&dir, "refused");
    drop(guard);
    assert_eq!(
        code,
        Some(3),
        "a refused model read exits 3. stderr:\n{err}"
    );
    assert!(
        err.contains("degraded step tier: unreadable: "),
        "the stderr detector line is unchanged: {err}"
    );
    assert_eq!(
        err.matches("refused access: ").count(),
        1,
        "and the refused path is named once: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_absent_model_still_exits_0() {
    let (dir, _) = protocol("shot_absent", false, true);
    let (code, err) = shot(&dir, "absent");
    assert_eq!(
        code,
        Some(0),
        "an absent model is still a render. stderr:\n{err}"
    );
    assert!(err.contains("degraded step tier: absent: "), "{err}");
    assert!(!err.contains("refused access"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn engine_text_with_operation_not_permitted_names_the_path() {
    assert_eq!(
        refusal_in_engine_text("IO Error: Cannot open file \"/a/b.csv\": Operation not permitted"),
        Some(PathBuf::from("/a/b.csv"))
    );
}
