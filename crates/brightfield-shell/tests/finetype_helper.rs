//! The `arc` child is pointed at the `finetype` helper an app bundle carries.
//!
//! arc runs FineType's command line from `$FINETYPE_BIN`, or `finetype` on the
//! search path when that is unset. The sandboxed Store build cannot reach a
//! `finetype` the analyst installed, so it carries one at
//! `Contents/Helpers/finetype`, beside the DuckDB CLI, and Brightfield names it
//! in the child's environment. Each test lays out a fake bundle whose
//! `Contents/MacOS/brightfield` is a shell script that prints what it was
//! handed, and starts it the way Brightfield starts arc.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use brightfield_shell::run::{arc_with_args, Runner, FINETYPE_ENV};

/// The program inside a fake `Brightfield.app` under `root`: a script that
/// prints the `FINETYPE_BIN` it was started with, or `unset`.
fn fake_app(root: &Path) -> PathBuf {
    let contents = root.join("Brightfield.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    let program = contents.join("MacOS/brightfield");
    executable(
        &program,
        "#!/bin/sh\nprintf '%s' \"${FINETYPE_BIN-unset}\"\n",
    );
    program
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn arc_is_pointed_at_the_bundled_finetype() {
    let root = tempfile::tempdir().unwrap();
    let program = fake_app(root.path());
    let helpers = root.path().join("Brightfield.app/Contents/Helpers");
    std::fs::create_dir_all(&helpers).unwrap();
    executable(
        &helpers.join("finetype"),
        "#!/bin/sh\necho finetype 0.0.0\n",
    );

    let out = arc_with_args(&Runner::at(&program), root.path(), &["--version"]);

    assert!(out.succeeded, "{out:?}");
    assert_eq!(FINETYPE_ENV, "FINETYPE_BIN");
    let named = PathBuf::from(&out.stdout);
    assert_eq!(
        named.canonicalize().unwrap(),
        helpers.join("finetype").canonicalize().unwrap(),
        "the child was handed {}",
        out.stdout
    );
}

/// Watched redden with `is_file` replaced by `exists`: a directory where the
/// helper would be was then handed to arc as the binary.
#[test]
fn without_a_bundled_finetype_the_child_keeps_what_it_inherits() {
    let root = tempfile::tempdir().unwrap();
    let program = fake_app(root.path());
    // A directory by the helper's name is not a helper.
    std::fs::create_dir_all(
        root.path()
            .join("Brightfield.app/Contents/Helpers/finetype"),
    )
    .unwrap();

    let out = arc_with_args(&Runner::at(&program), root.path(), &["--version"]);

    assert!(out.succeeded, "{out:?}");
    let inherited = std::env::var(FINETYPE_ENV).unwrap_or_else(|_| "unset".to_string());
    assert_eq!(out.stdout, inherited);
}
