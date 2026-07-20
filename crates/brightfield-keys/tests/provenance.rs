//! Provenance doc check: every BOUND key in the shipped registry traces
//! to a recorded score row in `keymap-research.md` — no key is bound by taste.
//!
//! This is the mechanical half of the check (the longname cross-ref). The scope-model
//! decisions match (selection-first, g-broadcast, view floor) are a manual doc
//! review, recorded in the same file.

use brightfield_keys::{registry, VerbEntry};

/// The research digest, baked in at compile time (path relative to this test file).
const RESEARCH: &str = include_str!("data/keymap-research.md");

#[test]
fn every_bound_longname_has_a_score_row() {
    let reg = registry();
    let bound: Vec<&VerbEntry> = reg.iter().filter(|v| v.is_bound()).collect();
    assert!(!bound.is_empty(), "expected some bound verbs");

    for v in &bound {
        // The provenance table lists each longname in backticks alongside its scores.
        let needle = format!("`{}`", v.longname);
        assert!(
            RESEARCH.contains(&needle),
            "bound verb `{}` has no score row in keymap-research.md (provenance gap)",
            v.longname
        );
        // Sanity: a bound verb carries the scores the table records.
        assert!(
            v.scores.is_some(),
            "bound verb `{}` is unscored in the registry",
            v.longname
        );
    }
}

#[test]
fn reserved_scope_decisions_are_recorded() {
    // The manual-review half: the load-bearing scope-model decisions appear in the doc.
    for decision in [
        "selection-first",
        "g` = dashboard-broadcast",
        "view-altitude floor",
    ] {
        assert!(
            RESEARCH.contains(decision),
            "scope-model decision '{decision}' not recorded in keymap-research.md"
        );
    }
}
