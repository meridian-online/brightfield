//! `scripts/package.sh` must stage exactly one binary into the release root.
//!
//! Three binaries build from this package — `brightfield-shell` (renamed
//! `brightfield` in the tarball), `brightfield-shot` and `bf-imgdiff` — and
//! only the first ships. `brightfield-shot` just grew a `--crop` flag so a
//! published picture of the product is a build output rather than a manual
//! screenshot; that change touches this binary, not the packaging script, and
//! this is the guard that a change reaching for the wrong file cannot pass
//! silently. It reads `scripts/package.sh` as text rather than running it —
//! running it needs a release build of every platform target, which belongs
//! to `release.yml` on a tag, not to a suite that runs on every push — and
//! greps for **which shell variable each `cp … "$STAGE/…"` line copies**,
//! rather than for a fixed count of `cp` lines, because `cp LICENSE
//! "$STAGE/LICENSE"` and `cp examples/*.yaml "$STAGE/examples/"` are two more
//! `cp` lines into the same tree that are not binaries and must not trip this.
//!
//! A binary variable is one assigned a path under `target/.../release/`
//! (`BIN="target/release/brightfield-shell"`, and its cross-compile sibling
//! under `target/${TARGET}/release/`); this counts how many *of those* are
//! ever copied straight into `$STAGE` (not into a subdirectory — `$STAGE/foo`,
//! never `$STAGE/examples/foo`), and fails if that count is not exactly one.

use std::path::PathBuf;

/// `scripts/package.sh`, addressed from the crate root so the test does not
/// depend on the shell's working directory.
fn package_script() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/package.sh");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The shell variable names `package.sh` assigns a built-binary path to:
/// `NAME="target/…/release/…"`, on one line, `NAME` all-caps with underscores
/// (matching the script's own naming convention for `BIN`/`TARGET`/etc).
fn binary_variables(script: &str) -> Vec<&str> {
    script
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            let (name, rhs) = t.split_once('=')?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                return None;
            }
            let rhs = rhs.trim();
            (rhs.starts_with('"') && rhs.contains("target/") && rhs.contains("/release/"))
                .then_some(name)
        })
        .collect()
}

/// `cp` lines that copy one of `vars` straight into `$STAGE` — not into a
/// subdirectory of it, which is how `examples/` and `finetype/` are staged.
fn binary_copies_into_stage_root<'a>(script: &'a str, vars: &[&str]) -> Vec<&'a str> {
    script
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            if !t.starts_with("cp ") {
                return false;
            }
            let Some(dest_start) = t.find("\"$STAGE/") else {
                return false;
            };
            let dest = &t[dest_start + "\"$STAGE/".len()..];
            let Some(dest_end) = dest.find('"') else {
                return false;
            };
            // A copy straight into $STAGE has no further '/' before the
            // closing quote; `examples/foo.yaml` or `finetype` do.
            if dest[..dest_end].contains('/') {
                return false;
            }
            vars.iter().any(|v| t.contains(&format!("\"${v}\"")))
        })
        .collect()
}

#[test]
fn exactly_one_binary_is_staged_into_the_release_root() {
    let script = package_script();
    let vars = binary_variables(&script);
    assert!(
        !vars.is_empty(),
        "found no `NAME=\"target/…/release/…\"` assignment in scripts/package.sh — \
         this test no longer recognises the script's own convention for naming a \
         built binary and needs updating, not the script"
    );
    let copies = binary_copies_into_stage_root(&script, &vars);
    assert_eq!(
        copies.len(),
        1,
        "scripts/package.sh stages {} binaries at the release root, expected exactly 1: {copies:?}",
        copies.len()
    );
    assert!(
        copies[0].contains("$BIN"),
        "the one binary staged at the release root should be $BIN (brightfield-shell, \
         renamed brightfield in the tarball); found: {}",
        copies[0]
    );
}
