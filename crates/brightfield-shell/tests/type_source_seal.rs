//! The seal `--check-type-source` runs under, exercised by running it.
//!
//! There was already a unit test over the allowlist predicate. It was not
//! enough, and the way it was not enough is the point: deleting BOTH
//! `.env_clear()` and the stowaway refusal left the entire shell suite green,
//! because the only thing tested was a pure function neither of them calls.
//! A guard living in a test rather than in the code was the defect this whole
//! branch kept rediscovering; a guard whose code is untested is the same defect
//! from the other side.
//!
//! These run the real binary — `CARGO_BIN_EXE_brightfield-shell`, so they test
//! what Cargo just built rather than whatever is on `PATH` — with a hostile
//! environment and a hostile working directory, and read the seal line the
//! sealed phase prints. No bundle, no extension and no model: the seal is
//! established before any of that is looked for, which is why these can run on
//! every CI machine.
//!
//! What they do NOT cover: the filesystem. See `--check-type-source`'s own
//! documentation — the environment is sealed and the filesystem is not, and no
//! test here or anywhere in this repository closes that.

use std::process::Command;

/// A path no sealed run may report, distinctive enough that finding it anywhere
/// in the output means it travelled.
const MARKER: &str = "/brightfield-seal-marker";

fn run(hostile_cwd: &std::path::Path, extra: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_brightfield-shell"));
    cmd.arg("--check-type-source").current_dir(hostile_cwd);
    // Inherit this test's environment and add to it — the caller's environment
    // is exactly what the seal exists to keep out, so handing it a clean one
    // would test nothing.
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the shell binary runs");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

/// A hostile environment and working directory do not reach the sealed phase.
///
/// Reads the seal line rather than the exit code, and that is deliberate:
/// removing `.env_clear()` alone turns the exit code from 2 into 1 (the
/// stowaway refusal fires), but removing the refusal as well turns it back to
/// 2 while the child now runs wide open. Only the reported `HOME` and cwd
/// separate a sealed run from an unsealed one in both cases.
#[test]
fn a_hostile_environment_and_working_directory_do_not_reach_the_sealed_phase() {
    let hostile_cwd = std::env::temp_dir().join(format!(
        "brightfield-seal-cwd-{}{MARKER}",
        std::process::id()
    ));
    std::fs::create_dir_all(&hostile_cwd).unwrap();

    let (code, out) = run(
        &hostile_cwd,
        &[
            ("HOME", &format!("{MARKER}/home")),
            ("FINETYPE_MODEL_DIR", &format!("{MARKER}/model")),
            ("FINETYPE_INJECT_LABEL", "identity.person.email"),
            ("HF_HOME", &format!("{MARKER}/hf")),
            ("DYLD_LIBRARY_PATH", MARKER),
            ("LD_PRELOAD", &format!("{MARKER}/nothing.so")),
        ],
    );

    assert!(
        out.contains("check-type-source: sealed —"),
        "the sealed phase never reported its seal, so nothing here was sealed:\n{out}"
    );
    assert!(
        !out.contains("refusing to report"),
        "the sealed phase saw a stowaway, so the environment was not cleared:\n{out}"
    );
    assert!(
        !out.contains(MARKER),
        "a hostile HOME or working directory reached the sealed phase:\n{out}"
    );

    // The seal line names what the child can see. Anything outside the
    // allowlist reaching it is the failure, whether or not its VALUE happens to
    // be visible anywhere else in the output — a leaked FINETYPE_MODEL_DIR
    // would otherwise be invisible here, which is exactly how the mutation that
    // removed both halves of the seal stayed green.
    let listed = out
        .lines()
        .find_map(|l| l.split_once("env [").and_then(|(_, r)| r.split_once(']')))
        .map(|(names, _)| names.to_string())
        .expect("the seal line lists the environment it can see");
    let permitted = [
        "PATH",
        "HOME",
        "TMPDIR",
        "LC_ALL",
        "LANG",
        "BRIGHTFIELD_TYPE_SOURCE_SEALED",
        "__CF_USER_TEXT_ENCODING",
    ];
    let stowaways: Vec<&str> = listed
        .split_whitespace()
        .filter(|n| !permitted.contains(n))
        .collect();
    assert!(
        stowaways.is_empty(),
        "the sealed phase can see {stowaways:?}, which the outer phase did not set:\n{out}"
    );
    assert_ne!(code, 1, "the seal could not be established at all:\n{out}");

    std::fs::remove_dir_all(&hostile_cwd).ok();
}

/// The stowaway refusal, pinned on its own.
///
/// Its job is the caller who exports the marker themselves — the one case
/// `.env_clear()` cannot help with, because that caller is the outer phase as
/// far as this process can tell. Setting the marker skips the re-exec, so what
/// runs is the sealed phase over an environment nobody sealed.
#[test]
fn a_forged_seal_marker_is_refused_rather_than_believed() {
    let cwd = std::env::temp_dir();
    let (code, out) = run(
        &cwd,
        &[
            ("BRIGHTFIELD_TYPE_SOURCE_SEALED", "1"),
            ("FINETYPE_MODEL_DIR", &format!("{MARKER}/model")),
        ],
    );

    assert_eq!(code, 1, "a forged marker was believed:\n{out}");
    assert!(
        out.contains("refusing to report"),
        "the refusal did not say what it was refusing:\n{out}"
    );
    assert!(
        out.contains("FINETYPE_MODEL_DIR"),
        "the refusal did not name the stowaway it found:\n{out}"
    );
}

/// A build with no bundle beside it says so, and says it distinctly.
///
/// `2` rather than `1` because a packaged build without a type source is
/// supported, and `scripts/verify-airgapped.sh` acts on the difference: it
/// treats `2` from an artefact that visibly carries a bundle as a staging bug.
#[test]
fn no_bundle_beside_the_binary_is_its_own_exit_code() {
    // The test binary lives in target/<profile>/deps and the shell binary in
    // target/<profile>; neither has a bundle staged beside it unless somebody
    // put one there by hand, which is what this skips on rather than fails on.
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_brightfield-shell"));
    if exe
        .parent()
        .is_some_and(|d| d.join("finetype/finetype.duckdb_extension").is_file())
    {
        eprintln!("a bundle is staged beside the shell binary; this case needs none");
        return;
    }
    let (code, out) = run(&std::env::temp_dir(), &[]);
    assert_eq!(code, 2, "expected the no-bundle exit code:\n{out}");
    assert!(out.contains("no type source bundled beside"), "{out}");
}
