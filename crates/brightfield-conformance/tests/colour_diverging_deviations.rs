//! **The register records the two places a diverging colour scale draws
//! differently from Mosaic, and `DEVIATIONS.md` carries them.**
//!
//! Mosaic's pivot is 0 and its diverging scheme is `rdbu`; brightfield's default
//! pivot is 0 across the origin and the rows' median otherwise, and its default
//! arms are the design system's blue and red. A spec that writes `colorPivot` or
//! `colorScheme: rdbu` draws as Mosaic does, so each default is a deviation, and
//! a deviation nobody wrote down is one the next reader meets as a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads the two entries themselves, so a record dropped from
//! the register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The one registry entry whose surface names `needle`.
fn entry_for(needle: &str) -> Deviation {
    let registry =
        load_deviations(&repo_root().join("deviations.yaml")).expect("the register loads");
    let found: Vec<Deviation> = registry
        .iter()
        .filter(|d| d.surface.contains(needle))
        .cloned()
        .collect();
    assert_eq!(found.len(), 1, "one entry names `{needle}`: {found:?}");
    found.into_iter().next().unwrap()
}

#[test]
fn the_pivots_default_is_recorded_against_mosaics_zero() {
    let entry = entry_for("`colorPivot`");
    assert!(
        entry.mosaic_behaviour.contains("0"),
        "Mosaic's pivot is 0: {}",
        entry.mosaic_behaviour
    );
    assert!(
        entry.brightfield_behaviour.contains("median"),
        "brightfield's default is the median where the rows do not cross 0: {}",
        entry.brightfield_behaviour
    );
}

#[test]
fn the_default_arms_are_recorded_against_mosaics_rdbu() {
    let entry = entry_for("default arms");
    assert!(
        entry.mosaic_behaviour.contains("`rdbu`"),
        "Mosaic's scheme is rdbu: {}",
        entry.mosaic_behaviour
    );
    assert!(
        entry.brightfield_behaviour.contains("blue") && entry.brightfield_behaviour.contains("red"),
        "brightfield's arms are the design system's blue and red: {}",
        entry.brightfield_behaviour
    );
}

#[test]
fn the_generated_document_carries_both_entries() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    for needle in ["`colorPivot`", "default arms"] {
        let id = entry_for(needle).id;
        assert!(
            doc.contains(&format!("## {id} — ")),
            "DEVIATIONS.md has no section for {id}, the entry that names {needle}"
        );
    }
}
