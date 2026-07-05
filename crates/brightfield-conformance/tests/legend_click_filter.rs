//! lcf_ac06 (card 0009, legend click-to-filter).
//!
//! A spec binding a standalone legend to a selection — `legend: color
//! as: $sel for: scatter` — parses clean (the `as:` binding rides the legend
//! option bag; no Unimplemented vocabulary) and preflights Implemented at the
//! legend surface. The channel gate is unchanged: `legend: symbol as: $sel`
//! still reports Unimplemented — a binding never smuggles an unrenderable
//! channel past preflight.

use brightfield_conformance::{preflight, ComponentIdentity, Surface};
use brightfield_spec::{parse_spec, ComponentKind, Format, ParseWarning};

const LEGEND_AS_BINDING: &str = r#"
params:
  sel: { select: crossfilter }
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
    as: $sel
  - plot:
    - mark: dot
      data: { from: t, filterBy: $sel }
      x: x
      y: y
"#;

#[test]
fn lcf_ac06_legend_as_binding_parses_and_preflights_implemented() {
    let parsed = parse_spec(LEGEND_AS_BINDING, Format::Yaml).expect("bound-legend spec parses");

    // No known-but-unimplemented vocabulary is flagged by the binding.
    for w in &parsed.warnings {
        assert!(
            !matches!(w, ParseWarning::Unimplemented { .. }),
            "a bound colour legend must not warn Unimplemented: {w:?}"
        );
    }

    // Preflight records only non-Implemented components, so the bound legend
    // must not appear at all (mirrors fww_ac05 for the display-only form).
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
        "a bound Implemented legend must not appear in preflight: {legend_entries:?}"
    );

    // The ONLY blocking entries this fixture may report are the bare
    // `plot:`/`hconcat:` layout scaffolding (DEV-0001) — anything else is a
    // regression hiding behind the Legend-only filter above.
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
}

#[test]
fn lcf_ac06_symbol_legend_binding_still_channel_blocked() {
    let symbol = LEGEND_AS_BINDING.replace("legend: color", "legend: symbol");
    let parsed = parse_spec(&symbol, Format::Yaml).expect("symbol-legend spec still parses");

    // The channel gate fires exactly as before the binding existed.
    assert!(
        parsed.warnings.iter().any(|w| matches!(
            w,
            ParseWarning::Unimplemented { name, .. } if name == "symbol"
        )),
        "`legend: symbol as: $sel` must still warn Unimplemented: {:?}",
        parsed.warnings
    );

    // And preflight still blocks on the legend node.
    let report = preflight(&parsed.spec);
    assert!(
        report.blocking().iter().any(|e| matches!(
            e.identity,
            ComponentIdentity::Component(ComponentKind::Legend)
        )),
        "a symbol legend must block preflight, bound or not: {:?}",
        report.entries
    );
}
