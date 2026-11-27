//! **`regions: off` is the one value a plot's `regions` key takes.**
//!
//! Brightfield outlines and names the dense regions of a two-column plot, and
//! an analyst turns that off for one plot by writing `regions: off` on it. The
//! key is a Brightfield extension, written only when the regions are off, so
//! a spec that never touches it stays plain Mosaic. Any other value is a key
//! nothing reads, and the author is told so rather than left wondering why the
//! regions still draw.

use brightfield_spec::layout::collect_plot_nodes;
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

/// Two plots under a `plotDefaults:` that sets `regions` to `value`.
fn under_defaults(value: &str) -> ParseOutput {
    let source = format!(
        "plotDefaults:\n  regions: {value}\n\
         data:\n  t:\n    - {{ x: 1, y: 2 }}\n\
         hconcat:\n  - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n\
         \x20 - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: y\n        y: x\n"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

#[test]
fn regions_under_plot_defaults_warns_and_is_not_inherited() {
    for value in ["off", "maybe"] {
        let out = under_defaults(value);
        let named = regions_warnings(&out);
        assert!(
            matches!(
                named.as_slice(),
                [ParseWarning::RegionsOnPlotDefaults { value: v }] if v == value
            ),
            "`regions: {value}` under `plotDefaults:` is named once, as belonging on a plot: {:?}",
            out.warnings
        );
        assert!(
            named[0].to_string().contains("belongs on a plot"),
            "the warning says where the key goes: {}",
            named[0]
        );
        let plots = collect_plot_nodes(&out.spec);
        assert_eq!(plots.len(), 2);
        for (path, plot) in plots {
            assert_eq!(
                plot.attributes.get("regions"),
                None,
                "{path} does not inherit `regions` from the defaults"
            );
        }
        assert!(
            out.spec.plot_defaults.0.contains_key("regions"),
            "the spec's own `plotDefaults:` keeps the key as written"
        );
    }
}
