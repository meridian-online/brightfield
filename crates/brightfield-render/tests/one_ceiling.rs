//! **The drawn-primitive ceiling is written down once.**
//!
//! It is a measured property of the renderer that two consumers act on: the
//! automatic sampling policy, which decides whether a plot needs a sample, and
//! `brightfield-bench`, which declines to time a frame it cannot prove has a
//! picture in it. A second transcription of the figure is a thing that is true
//! the day it is typed and silently false after the next measurement — and the
//! two consumers would then disagree about where a plot goes blank, which is
//! the failure with no error message.
//!
//! So: the digits appear in the workspace's Rust sources exactly once, at the
//! constant. This scan is what makes that checkable rather than remembered.

use std::path::{Path, PathBuf};

use brightfield_render::sample_policy::{
    MEASURED_BLANK_MIN, MEASURED_INKED_MAX, VELLO_BIN_DATA_PANIC_DOTS,
};

/// The file the constants live in — the one place the digits are allowed.
const HOME: &str = "crates/brightfield-render/src/sample_policy.rs";

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root resolves")
}

/// Every tracked `.rs` file under `crates/`, as (relative path, contents).
fn rust_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // `target/` is build output, not source, and a workspace built
                // inside a crate directory would otherwise swamp the scan.
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((rel, text));
                }
            }
        }
    }
    assert!(
        out.len() > 50,
        "the scan found {} Rust sources under crates/ — the path is wrong, and a \
         scan that reads nothing passes everything",
        out.len()
    );
    out
}

/// The forms a five-or-six-digit count gets written in: bare, underscored the
/// way Rust literals are, and comma-grouped the way prose is.
fn spellings(n: u64) -> Vec<String> {
    let plain = n.to_string();
    let group = |sep: char| {
        let mut out = String::new();
        for (i, c) in plain.chars().enumerate() {
            if i > 0 && (plain.len() - i) % 3 == 0 {
                out.push(sep);
            }
            out.push(c);
        }
        out
    };
    vec![plain.clone(), group('_'), group(','), group('\u{202f}')]
}

#[test]
fn the_measured_ceiling_is_written_down_once() {
    let sources = rust_sources();
    for (label, value) in [
        ("MEASURED_INKED_MAX", MEASURED_INKED_MAX),
        ("MEASURED_BLANK_MIN", MEASURED_BLANK_MIN),
        ("VELLO_BIN_DATA_PANIC_DOTS", VELLO_BIN_DATA_PANIC_DOTS),
    ] {
        let mut elsewhere: Vec<String> = Vec::new();
        for (path, text) in &sources {
            if path == HOME {
                continue;
            }
            for spelling in spellings(value) {
                for (i, line) in text.lines().enumerate() {
                    if line.contains(&spelling) {
                        elsewhere.push(format!("{path}:{}", i + 1));
                    }
                }
            }
        }
        elsewhere.sort();
        elsewhere.dedup();
        assert!(
            elsewhere.is_empty(),
            "{label} is spelled out away from {HOME}, at {elsewhere:?}. Read the \
             constant instead — a copy cannot be re-measured."
        );
    }
}

/// …and the home really does hold it, so the scan above is not passing because
/// it is looking for something no longer there.
#[test]
fn the_home_declares_the_constants_the_scan_is_looking_for() {
    let text = std::fs::read_to_string(workspace_root().join(HOME)).expect("the home is readable");
    for (label, value) in [
        ("MEASURED_INKED_MAX", MEASURED_INKED_MAX),
        ("MEASURED_BLANK_MIN", MEASURED_BLANK_MIN),
        ("VELLO_BIN_DATA_PANIC_DOTS", VELLO_BIN_DATA_PANIC_DOTS),
    ] {
        assert!(
            spellings(value).iter().any(|s| text.contains(s)),
            "{label} is not spelled out in {HOME} at all"
        );
    }
}
