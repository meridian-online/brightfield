//! **The register records where a pinned colour domain is read differently from
//! Mosaic, and `DEVIATIONS.md` carries it.**
//!
//! Mosaic fixes a colour scale's domain under `colorDomain: Fixed` whether the
//! colour paints numbers or names; brightfield pins a colour of numbers and
//! leaves a colour of names inferred. A deviation nobody wrote down is one the
//! next reader meets as a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads the entry itself, so a record dropped from the
//! register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

const SURFACE: &str = "a pinned colour domain";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The one registry entry whose surface names a pinned colour domain.
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
fn a_pinned_colour_is_recorded_against_mosaics_pin_on_names_too() {
    let entry = entry();
    assert!(
        entry.surface.contains("`colorDomain: Fixed`"),
        "the entry names the key: {}",
        entry.surface
    );
    assert!(
        entry.mosaic_behaviour.contains("names"),
        "Mosaic pins a colour of names as well: {}",
        entry.mosaic_behaviour
    );
    assert!(
        entry.brightfield_behaviour.contains("NOT pinned")
            && entry.brightfield_behaviour.contains("category names"),
        "brightfield leaves a colour of names unpinned: {}",
        entry.brightfield_behaviour
    );
}

#[test]
fn the_generated_document_carries_the_entry() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    let id = entry().id;
    assert!(
        doc.contains(&format!("## {id} — ")),
        "DEVIATIONS.md has no section for {id}, the entry that names a pinned colour domain"
    );
}
