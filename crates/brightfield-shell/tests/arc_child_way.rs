//! Every `arc` child Brightfield starts records under the way `brightfield`.
//!
//! arc's run log names the interface a step came through, from the
//! `ARCFORM_HISTORY_WAY` variable in the child's environment. The runner is the
//! `brightfield-shell` binary cargo built for this suite, which becomes `arc`
//! under the runner variable, so the log line is arc's own.

use std::path::PathBuf;

use brightfield_shell::operation::record_sql;
use brightfield_shell::run::Runner;
use brightfield_shell::{data_file, starts};

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-arc-child-way-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_recorded_step_is_logged_as_brightfield() {
    let dir = TempDir::new("logged");
    let folder = dir.0.join("protocol");
    let history = dir.0.join("history");
    let db = dir.0.join("db");
    for d in [&folder, &history, &db] {
        std::fs::create_dir_all(d).expect("a fixture directory");
    }
    let data = folder.join(starts::HOUSING_FILE);
    std::fs::write(&data, include_bytes!("data/california_housing.parquet"))
        .expect("the housing file writes");
    data_file::open(&data.to_string_lossy())
        .expect("the housing file opens")
        .protocol
        .save_to(&folder)
        .expect("the Protocol saves");

    // This file holds one test, so no other thread reads the environment while
    // it is set; the arc child inherits both directories from it.
    std::env::set_var("ARCFORM_HISTORY_DIR", &history);
    std::env::set_var("ARCFORM_DB_DIR", &db);
    std::env::remove_var(arc::spec::HISTORY_WAY_ENV);

    record_sql(
        &Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")),
        &folder,
        "SELECT 1 AS one",
        "one",
    )
    .expect("arc records the statement");

    let log = std::fs::read_to_string(folder.join("arcform-log.txt")).expect("arc wrote its log");
    let last = log.lines().last().expect("the log has a line");
    assert!(last.contains("\"interface\":\"brightfield\""), "{last}");
    assert!(last.contains("\"step\":\"one\""), "{last}");
}
