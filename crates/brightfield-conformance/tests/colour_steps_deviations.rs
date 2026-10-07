//! **The register records the one place a stepped colour scale is drawn
//! differently from Mosaic — the count of its steps — and `DEVIATIONS.md` carries
//! it.**
//!
//! Mosaic's renderer rounds the thresholds of a `quantize` scale to tidy values,
//! so `colorN` is a target there; brightfield draws the count asked for, in steps
//! of equal width. A deviation nobody wrote down is one the next reader meets as
//! a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads the entry itself, so a record dropped from the
//! register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

const SURFACE: &str = "the count of steps of a stepped scale";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The one registry entry whose surface names the count of a stepped scale's steps.
fn entry() -> Deviation {
    let registry =
        load_deviations(&repo_root().join("deviations.yaml")).expect("the register loads");
    let found: Vec<Deviation> = registry
        .iter()
        .filter(|d| d.surface.contains(SURFACE))
        .cloned()
        .collect();
    assert_eq!(found.len(), 1, "one entry names `{SURFACE}`: {found:?}");
    found.into_iter().next().unwrap()
}

/// A record's prose on one line, since the register wraps it where it likes.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_count_of_steps_is_recorded_against_mosaics_tidy_thresholds() {
    let entry = entry();
    let (mosaic, ours) = (
        flat(&entry.mosaic_behaviour),
        flat(&entry.brightfield_behaviour),
    );
    assert!(
        mosaic.contains("tidy") && mosaic.contains("six steps"),
        "Mosaic rounds the thresholds, and `colorN: 5` over 0 to 500001 is six steps there: {mosaic}"
    );
    assert!(
        ours.contains("count asked for") && ours.contains("equal width"),
        "brightfield draws the count asked for, in steps of equal width: {ours}"
    );
    assert!(
        entry.surface.contains("`colorN`") && entry.surface.contains("`colorScale: quantize`"),
        "the entry names the keys: {}",
        entry.surface
    );
}

#[test]
fn the_generated_document_carries_the_entry() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    let id = entry().id;
    assert!(
        doc.contains(&format!("## {id} — ")),
        "DEVIATIONS.md has no section for {id}, the entry that names the count of steps"
    );
}
