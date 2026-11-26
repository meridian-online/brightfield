//! **`regions: off` is the one value a plot's `regions` key takes.**
//!
//! Brightfield outlines and names the dense regions of a two-column plot, and
//! an analyst turns that off for one plot by writing `regions: off` on it. The
//! key is a Brightfield extension, written only when the regions are off, so
//! a spec that never touches it stays plain Mosaic. Any other value is a key
//! nothing reads, and the author is told so rather than left wondering why the
//! regions still draw.

use brightfield_spec::{parse_spec, Format, ParseOutput, ParseWarning};

fn parsed(attrs: &str) -> ParseOutput {
    let source = format!(
        "data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

fn regions_warnings(out: &ParseOutput) -> Vec<&ParseWarning> {
    out.warnings
        .iter()
        .filter(|w| w.to_string().contains("regions"))
        .collect()
}

#[test]
fn regions_off_parses_without_a_warning() {
    let out = parsed("regions: off\n");
    assert!(
        out.warnings.is_empty(),
        "`regions: off` is read, so nothing is said about it: {:?}",
        out.warnings
    );
}

#[test]
fn an_unknown_regions_value_warns() {
    for value in ["maybe", "on", "true", "1"] {
        let out = parsed(&format!("regions: {value}\n"));
        let named = regions_warnings(&out);
        assert!(
            matches!(
                named.as_slice(),
                [ParseWarning::InvalidRegionsSwitch { value: v }] if v == value
            ),
            "`regions: {value}` is a value nothing reads, named once: {:?}",
            out.warnings
        );
    }
}

#[test]
fn an_unknown_regions_value_under_plot_defaults_warns_once() {
    let source = "plotDefaults:\n  regions: maybe\n\
                  data:\n  t:\n    - { x: 1, y: 2 }\n\
                  hconcat:\n  - plot:\n      - mark: dot\n        data: { from: t }\n        x: x\n        y: y\n\
                  \x20 - plot:\n      - mark: dot\n        data: { from: t }\n        x: y\n        y: x\n";
    let out = parse_spec(source, Format::Yaml).expect("the spec parses");
    assert_eq!(
        regions_warnings(&out).len(),
        1,
        "named once, under `plotDefaults:`, not again for each plot: {:?}",
        out.warnings
    );
}
