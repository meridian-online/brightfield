//! The `brightfield-shell` binary becomes dovetail, under its variable.
//!
//! Dovetail's entry point parses the calling process's argv and returns an exit code, so it runs in
//! a child of this binary rather than on a thread of the window, whose argv is its own, the way
//! `arc` does.

use std::process::Command;

use brightfield_shell::discovery::DOVETAIL_ENV;

#[test]
fn the_binary_serves_as_dovetail_under_its_variable() {
    let out = Command::new(env!("CARGO_BIN_EXE_brightfield-shell"))
        .arg("--help")
        .env(DOVETAIL_ENV, "1")
        .output()
        .expect("the brightfield-shell binary starts");
    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("dovetail — the modelling layer of meridian"),
        "stdout was: {stdout}"
    );
}
