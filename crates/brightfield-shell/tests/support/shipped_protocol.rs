//! Shipped Protocols built for a test, and the offline stand-in for one that
//! fetches.
//!
//! A [`ShippedProtocol`] is `'static` because a start's is compiled in. A test
//! builds its own from files on disk at run time, so each one here is leaked:
//! a few kilobytes for the life of a test binary.
//!
//! **The stand-in.** A Protocol start whose steps include an `http_fetch` reads
//! the network on its first run, which a hermetic test may not. [`stand_in`]
//! makes the variant such a test boots instead: each `http_fetch` step whose
//! URL has a committed copy is dropped from the manifest, and the copy is
//! written where the step would have put the file, the way
//! `support/agent_protocol.rs` keeps the CO2 fixture off the network. Every
//! other file is the start's own, so what the variant draws is what the start
//! draws over the copy.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use brightfield_shell::starts::{is_fetch_op, ShippedProtocol};

/// The committed fixture folder: one bundled CSV, one SQL step, one panel.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol")
}

/// The fixture's files, relative to its folder.
const FIXTURE_FILES: &[&str] = &[
    "arcform.yaml",
    "models/measures.sql",
    "data/measures.csv",
    "panels/overview.yaml",
];

/// The data file the fixture's SQL step reads, relative to its folder.
pub const DATA_FILE: &str = "data/measures.csv";

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_files(files: Vec<(String, Vec<u8>)>) -> &'static [(&'static str, &'static [u8])] {
    let files: Vec<(&'static str, &'static [u8])> = files
        .into_iter()
        .map(|(path, bytes)| {
            let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            (leak_str(path), bytes)
        })
        .collect();
    Box::leak(files.into_boxed_slice())
}

/// The files of `protocol`, owned, so a variant can be made of them.
pub fn files_of(protocol: &ShippedProtocol) -> Vec<(String, Vec<u8>)> {
    protocol
        .files
        .iter()
        .map(|(path, bytes)| ((*path).to_string(), bytes.to_vec()))
        .collect()
}

/// A shipped Protocol of `files`, written as `dir_name` at `revision`.
pub fn shipped(
    dir_name: &str,
    revision: u32,
    files: Vec<(String, Vec<u8>)>,
) -> &'static ShippedProtocol {
    Box::leak(Box::new(ShippedProtocol {
        dir_name: leak_str(dir_name.to_string()),
        revision,
        files: leak_files(files),
    }))
}

/// The committed fixture as a shipped Protocol named `dir_name`.
pub fn fixture(dir_name: &str, revision: u32) -> &'static ShippedProtocol {
    let files = FIXTURE_FILES
        .iter()
        .map(|file| {
            let bytes = std::fs::read(fixture_dir().join(file))
                .unwrap_or_else(|e| panic!("the fixture's {file}: {e}"));
            ((*file).to_string(), bytes)
        })
        .collect();
    shipped(dir_name, revision, files)
}

/// The fixture as a Protocol that **fetches** its data: an `http_fetch` step
/// of `url` into the data file comes first, and the bundled copy is gone.
pub fn fetching_fixture(dir_name: &str, url: &str) -> &'static ShippedProtocol {
    let mut files = files_of(fixture(dir_name, 1));
    files.retain(|(path, _)| path != DATA_FILE);
    for (path, bytes) in &mut files {
        if path == "arcform.yaml" {
            let text = String::from_utf8(bytes.clone()).expect("the manifest is text");
            let fetch = format!(
                "steps:\n  - name: fetch_measures\n    op: http_fetch\n    with:\n      \
                 out: {DATA_FILE}\n      url: {url}\n    description: Fetch the measurements\n"
            );
            assert!(text.contains("steps:\n"), "the manifest has a steps list");
            *bytes = text.replacen("steps:\n", &fetch, 1).into_bytes();
        }
    }
    shipped(dir_name, 1, files)
}

/// The URL and output path of each `http_fetch` step in `manifest`.
fn fetch_steps(manifest: &serde_yaml::Value) -> Vec<(String, String)> {
    manifest
        .get("steps")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|step| {
            step.get("op")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(is_fetch_op)
        })
        .filter_map(|step| {
            let with = step.get("with")?;
            Some((
                with.get("url")?.as_str()?.to_string(),
                with.get("out")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// `protocol` with each `http_fetch` step whose URL is in `copies` dropped,
/// and the committed copy of what it fetches written at its `out`.
///
/// A fetch with no copy in `copies` fails here by URL, rather than leaving a
/// step that would reach the network.
pub fn stand_in(protocol: &ShippedProtocol, copies: &[(&str, &Path)]) -> &'static ShippedProtocol {
    let mut files = files_of(protocol);
    let manifest_at = files
        .iter()
        .position(|(path, _)| path == "arcform.yaml")
        .expect("a shipped Protocol has an arcform.yaml");
    let mut manifest: serde_yaml::Value =
        serde_yaml::from_slice(&files[manifest_at].1).expect("the manifest parses");
    let mut written = Vec::new();
    for (url, out) in fetch_steps(&manifest) {
        let (_, copy) = copies.iter().find(|(u, _)| *u == url).unwrap_or_else(|| {
            panic!(
                "{} fetches {url} and nothing stands in for it, so it cannot \
                     run offline",
                protocol.dir_name
            )
        });
        let bytes = std::fs::read(copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
        written.push((out, bytes));
    }
    if let Some(steps) = manifest
        .get_mut("steps")
        .and_then(serde_yaml::Value::as_sequence_mut)
    {
        steps.retain(|step| {
            !step
                .get("op")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(is_fetch_op)
        });
    }
    files[manifest_at].1 = serde_yaml::to_string(&manifest)
        .expect("the manifest serialises")
        .into_bytes();
    for (out, bytes) in written {
        files.retain(|(path, _)| *path != out);
        files.push((out, bytes));
    }
    shipped(protocol.dir_name, protocol.revision, files)
}
