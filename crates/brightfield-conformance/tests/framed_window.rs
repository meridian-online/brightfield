//! Framed window.
//!
//! A spec with a standalone `legend: color` node preflights clean at the
//! legend surface: `ComponentKind::Legend` is Implemented (the legend renders
//! in the headless composite AND as a hosted window element), so preflight no
//! longer reports a Legend component entry — Unimplemented or otherwise.

use brightfield_conformance::{preflight, ComponentIdentity, Surface};
use brightfield_spec::{parse_spec, ComponentKind, Format, ImplStatus};

const LEGEND_COLOR: &str = r#"
data:
  t:
    - { x: 1, y: 3, g: a }
    - { x: 2, y: 5, g: b }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: g
    name: scatter
  - legend: color
    for: scatter
"#;

#[test]
fn legend_color_preflights_clean() {
    let parsed = parse_spec(LEGEND_COLOR, Format::Yaml).expect("legend spec parses");

    // No Legend component entry at all — preflight records only components
    // that are not Implemented. (The bare `plot:`/`hconcat:` layout entries
    // are pre-existing DEV-0001 scaffolding, orthogonal to this AC.)
    let report = preflight(&parsed.spec);
    let legend_entries: Vec<&_> = report
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.identity,
                ComponentIdentity::Component(ComponentKind::Legend)
            )
        })
        .collect();
    assert!(
        legend_entries.is_empty(),
        "an Implemented legend must not appear in preflight: {legend_entries:?}"
    );

    // Post-review hardening: the ONLY blocking entries this fixture may report
    // are the bare `plot:`/`hconcat:` layout scaffolding (DEV-0001). Anything
    // else — a regressed legend channel, a demoted mark — must fail here
    // instead of hiding behind the Legend-only filter above.
    for entry in report.blocking() {
        assert!(
            matches!(
                entry.identity,
                ComponentIdentity::Component(ComponentKind::Plot)
                    | ComponentIdentity::Component(ComponentKind::HConcat)
            ) && entry.surface == Surface::Layout,
            "unexpected blocking entry beyond DEV-0001 layout scaffolding: {entry:?}"
        );
    }

    // The registry promotion itself.
    assert_eq!(ComponentKind::Legend.status(), ImplStatus::Implemented);
}
