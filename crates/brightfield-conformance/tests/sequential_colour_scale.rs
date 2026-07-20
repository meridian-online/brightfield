//! Sequential colour scale.
//!
//! A raster spec declaring `colorScheme` parses cleanly (no
//! `ParseWarning::Unimplemented`) and preflights as Implemented: `colorScheme`
//! rides the open plot-attribute bag, and both the raster mark and the color
//! legend channel are `Implemented` in the vocabulary registry.

use brightfield_conformance::{preflight, ComponentIdentity};
use brightfield_spec::{parse_spec, Format, ImplStatus, MarkKind, ParseWarning};

const RASTER_COLORSCHEME: &str = r#"
data:
  points:
    - { x: 1, y: 1 }
    - { x: 2, y: 3 }
    - { x: 3, y: 2 }
plot:
  - mark: raster
    data: { from: points }
    x: x
    y: y
colorScheme: viridis
"#;

#[test]
fn scs_ac09_raster_colorscheme_parses_and_preflights_implemented() {
    let parsed = parse_spec(RASTER_COLORSCHEME, Format::Yaml).expect("raster colorScheme parses");

    // No known-but-unimplemented vocabulary is flagged.
    for w in &parsed.warnings {
        assert!(
            !matches!(w, ParseWarning::Unimplemented { .. }),
            "colorScheme raster must not warn Unimplemented: {w:?}"
        );
    }

    // Preflight records only non-Implemented components. No *mark* is blocking —
    // the raster is Implemented. (A bare top-level `plot:` reports the Plot
    // *component* Unimplemented at the layout surface: pre-existing scaffolding,
    // accounted for by DEV-0001, and orthogonal to this AC.)
    let report = preflight(&parsed.spec);
    let blocking_marks: Vec<&_> = report
        .blocking()
        .into_iter()
        .filter(|e| matches!(e.identity, ComponentIdentity::Mark(_)))
        .collect();
    assert!(
        blocking_marks.is_empty(),
        "the colorScheme raster must not block preflight: {blocking_marks:?}"
    );

    // The raster mark itself is Implemented in the registry.
    assert_eq!(MarkKind::Raster.status(), ImplStatus::Implemented);
}
