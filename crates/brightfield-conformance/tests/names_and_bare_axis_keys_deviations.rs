//! **The register records that a tick format on an axis of names, and the six
//! bare axis attributes the schema declares, are named in the banner and not
//! read, and `DEVIATIONS.md` carries both.**
//!
//! A band axis prints its names, so a number or date format on one has nothing to
//! act on; `axis`, `facetGrid`, `facetLabel`, `padding`, `align` and `xyDomain`
//! are plot attributes no resolver reads. brightfield draws the chart as the
//! file draws it without the key and names the key. A difference nobody wrote
//! down is one the next reader meets as a bug.
//!
//! The drift gate in `generate_deviations.rs` holds `DEVIATIONS.md` equal to a
//! regeneration; this reads each entry itself, so a record dropped from the
//! register fails here and not only by regenerating over the loss.

use std::path::PathBuf;

use brightfield_conformance::deviations::{load_deviations, Deviation};

const NAMES_SURFACE: &str = "a tick format (`xTickFormat`, `yTickFormat`) of either kind";
const BARE_SURFACE: &str = "named rather than read";

/// The six keys the schema declares and no resolver reads. The entry names them
/// all, so a key added to the banner without the register fails here.
const BARE_KEYS: [&str; 6] = [
    "`axis`",
    "`facetGrid`",
    "`facetLabel`",
    "`padding`",
    "`align`",
    "`xyDomain`",
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

/// The one registry entry whose surface contains `surface`.
fn entry(surface: &str) -> Deviation {
    let registry =
        load_deviations(&repo_root().join("deviations.yaml")).expect("the register loads");
    let found: Vec<Deviation> = registry
        .iter()
        .filter(|d| d.surface.contains(surface))
        .cloned()
        .collect();
    assert_eq!(found.len(), 1, "one entry names `{surface}`: {found:?}");
    found.into_iter().next().unwrap()
}

#[test]
fn a_format_on_an_axis_of_names_is_recorded_as_named_and_not_applied() {
    let entry = entry(NAMES_SURFACE);
    let brightfield = flat(&entry.brightfield_behaviour);
    assert!(
        brightfield.contains("`xTickFormat` or `yTickFormat`")
            && brightfield.contains("changing nothing on a category axis"),
        "brightfield names the key and says it changes nothing: {brightfield}"
    );
    assert!(
        brightfield.contains("A number format and a date format are named alike")
            && brightfield.contains("whether or not the names read as numbers"),
        "both kinds, and names that read as numbers, are named: {brightfield}"
    );
    let mosaic = flat(&entry.mosaic_behaviour);
    assert!(
        mosaic.contains("the names"),
        "Mosaic's side is a band axis that prints its names: {mosaic}"
    );
}

#[test]
fn the_bare_axis_attributes_are_recorded_as_named_and_not_read() {
    let entry = entry(BARE_SURFACE);
    for key in BARE_KEYS {
        assert!(
            entry.surface.contains(key),
            "the entry names {key}: {}",
            entry.surface
        );
    }
    let brightfield = flat(&entry.brightfield_behaviour);
    assert!(
        brightfield.contains("names the key as one this build does not read"),
        "brightfield names each key as one it does not read: {brightfield}"
    );
    assert!(
        brightfield.contains("`plotDefaults` is named once"),
        "a key under `plotDefaults` is named once: {brightfield}"
    );
}

#[test]
fn the_generated_document_carries_both_entries() {
    let doc = std::fs::read_to_string(repo_root().join("DEVIATIONS.md")).expect("DEVIATIONS.md");
    for (surface, what) in [
        (NAMES_SURFACE, "a tick format on an axis of names"),
        (BARE_SURFACE, "the six bare axis attributes"),
    ] {
        let id = entry(surface).id;
        assert!(
            doc.contains(&format!("## {id} — ")),
            "DEVIATIONS.md has no section for {id}, the entry that names {what}"
        );
    }
}
