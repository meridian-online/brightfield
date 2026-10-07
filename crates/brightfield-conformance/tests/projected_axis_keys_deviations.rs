//! **The register records that a plot with a map projection names its x and y
//! axis keys and does not read them, and `DEVIATIONS.md` carries it.**
//!
//! A projection replaces a plot's x and y with planar units, which are no axis,
//! so a tick count, a tick format, a gridline switch, a request to zero or round
//! the ends and a reversal have nothing to act on. brightfield draws the map as
//! the file draws it without the key and names the key in the banner. A
//! difference nobody wrote down is one the next reader meets as a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads the entry itself, so a record dropped from the
//! register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

const SURFACE: &str = "the x and y axis keys on a plot with a map projection";

/// Each key the banner names on a projected plot. The entry names them all, so a
/// key added to the banner without the register fails here.
const KEYS: [&str; 7] = [
    "`xTicks`",
    "`xTickFormat`",
    "`xGrid`",
    "`grid`",
    "`xZero`",
    "`xNice`",
    "`xReverse`",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// `text` on one line: a block scalar in the register keeps the line breaks it
/// was wrapped at, and a phrase can fall across one.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The one registry entry whose surface names the keys of a projected plot.
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
fn a_projected_plots_axis_keys_are_recorded_as_named_and_not_read() {
    let entry = entry();
    for key in KEYS {
        assert!(
            entry.surface.contains(key),
            "the entry names {key}: {}",
            entry.surface
        );
    }
    let brightfield = flat(&entry.brightfield_behaviour);
    assert!(
        brightfield.contains("names each key")
            && brightfield.contains("changing nothing on a plot with a map projection"),
        "brightfield names the key and says it changes nothing: {brightfield}"
    );
    assert!(
        brightfield.contains("leave the map's extent"),
        "`xZero` and `xNice` leave the map's extent where it is: {brightfield}"
    );
    let mosaic = flat(&entry.mosaic_behaviour);
    assert!(
        mosaic.contains("no x or y scale"),
        "Mosaic's side is a plot with no x or y scale: {mosaic}"
    );
}

#[test]
fn the_generated_document_carries_the_entry() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    let id = entry().id;
    assert!(
        doc.contains(&format!("## {id} — ")),
        "DEVIATIONS.md has no section for {id}, the entry that names a projected plot's axis keys"
    );
}
