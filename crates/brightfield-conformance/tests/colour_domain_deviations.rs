//! **The register records the one place a fixed colour domain is drawn differently
//! from Mosaic, and `DEVIATIONS.md` carries it.**
//!
//! Mosaic's renderer widens a `colorDomain` that is uneven about a diverging
//! scale's pivot until it is even; brightfield draws the two ends as written, so
//! each arm of the ramp runs over its own span. A deviation nobody wrote down is
//! one the next reader meets as a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads the entry itself, so a record dropped from the
//! register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

const SURFACE: &str = "a fixed domain on a diverging scale";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The one registry entry whose surface names a fixed domain on a diverging scale.
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

#[test]
fn a_fixed_domain_is_recorded_against_mosaics_widening_about_the_pivot() {
    let entry = entry();
    assert!(
        entry.mosaic_behaviour.contains("widened") && entry.mosaic_behaviour.contains("pivot"),
        "Mosaic widens the domain until it is even about the pivot: {}",
        entry.mosaic_behaviour
    );
    assert!(
        entry.brightfield_behaviour.contains("as written"),
        "brightfield draws the ends as written: {}",
        entry.brightfield_behaviour
    );
    assert!(
        entry.surface.contains("`colorDomain`"),
        "the entry names the key: {}",
        entry.surface
    );
}

#[test]
fn the_generated_document_carries_the_entry() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    let id = entry().id;
    assert!(
        doc.contains(&format!("## {id} — ")),
        "DEVIATIONS.md has no section for {id}, the entry that names a fixed domain"
    );
}
