//! **A plot whose file asks for a scale brightfield does not draw is named by
//! analysis with the key, the word as written and the plot; a scale it draws,
//! and no scale at all, are not.**
//!
//! `read_plot_scales_in` is the judge, and the drawn scale is taken from the same
//! reading, so a name the warning raises is a name the plot draws linear. The
//! reading is made after the whole spec is built, which is what lets a `$param`
//! be read through its declared value and a `plotDefaults:` scale be named at
//! the plots that inherit it.

use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::layout::{collect_plot_nodes, resolve_plot_scales_in, ScaleType};
use brightfield_spec::{parse_spec, Format, ParseWarning};

/// A spec with two plots stacked, so a line naming the wrong one is told apart.
/// `TOP` and `BOTTOM` mark each plot's attributes, `PRELUDE` the blocks before
/// `vconcat:`.
const STACKED: &str = r"
PRELUDE
data:
  t:
    - { x: 1, y: 2 }
vconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
    name: upper
TOP
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
BOTTOM
";

fn spec_of(prelude: &str, top: &str, bottom: &str) -> brightfield_spec::Spec {
    let source = STACKED
        .replace("PRELUDE", prelude)
        .replace("TOP", top)
        .replace("BOTTOM", bottom);
    parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
        .spec
}

/// The `(attribute, value, plot)` of each undrawn-scale warning analysis raised.
fn warned(spec: &brightfield_spec::Spec) -> Vec<(String, String, String)> {
    analyse_spec(spec)
        .expect("the spec analyses")
        .warnings
        .into_iter()
        .filter_map(|w| match w {
            ParseWarning::UndrawnScale {
                attribute,
                value,
                plot,
            } => Some((attribute, value, plot)),
            _ => None,
        })
        .collect()
}

fn line(attribute: &str, value: &str, plot: &str) -> (String, String, String) {
    (attribute.to_string(), value.to_string(), plot.to_string())
}

/// A scale this build does not draw is named with its key, its word as written
/// and the plot, by path and by `name:`, and only the plot that sets it.
#[test]
fn a_scale_this_build_does_not_draw_is_named_with_its_key_word_and_plot() {
    let spec = spec_of("", "    yScale: sqrt", "");
    assert_eq!(
        warned(&spec),
        [line("yScale", "sqrt", "root/vconcat[0] (`upper`)")]
    );

    let both = spec_of("", "    xScale: pow", "    yScale: band");
    assert_eq!(
        warned(&both),
        [
            line("xScale", "pow", "root/vconcat[0] (`upper`)"),
            line("yScale", "band", "root/vconcat[1]"),
        ]
    );
}

/// The sentence the banner draws names the key and the word, and says the axis
/// draws linear.
#[test]
fn the_warning_reads_as_a_sentence_naming_the_key_and_the_word() {
    let spec = spec_of("", "    yScale: sqrt", "");
    let said: Vec<String> = analyse_spec(&spec)
        .expect("the spec analyses")
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].contains("`yScale: sqrt`") && said[0].contains("linear"),
        "{said:?}"
    );
}

/// A scale this build draws, the default written out, and no key raise nothing.
#[test]
fn a_scale_it_draws_the_default_and_no_key_raise_nothing() {
    for (top, what) in [
        ("    yScale: log", "log"),
        ("    yScale: symlog", "symlog"),
        ("    yScale: linear", "linear"),
        ("    xScale: log\n    yScale: log", "both axes log"),
        ("", "no key"),
    ] {
        let found = warned(&spec_of("", top, ""));
        assert!(found.is_empty(), "{what}: {found:?}");
    }
}

/// A `$param` is read through its declared value: one holding `sqrt` is named as
/// the literal is, and one nobody declared, or one holding `log`, is not.
#[test]
fn a_param_holding_an_undrawn_scale_is_named_as_a_literal_is() {
    let held = spec_of("params:\n  s: sqrt", "    yScale: $s", "");
    assert_eq!(
        warned(&held),
        [line("yScale", "sqrt", "root/vconcat[0] (`upper`)")]
    );

    for (prelude, what) in [
        ("params:\n  s: log", "a param holding log"),
        ("", "a param nobody declared"),
    ] {
        let found = warned(&spec_of(prelude, "    yScale: $s", ""));
        assert!(found.is_empty(), "{what}: {found:?}");
    }
}

/// A `plotDefaults:` scale is named at each plot that inherits it, and a plot's
/// own scale wins over the default's.
#[test]
fn a_default_scale_is_named_at_each_plot_that_inherits_it() {
    let inherited = spec_of("plotDefaults:\n  yScale: sqrt", "", "");
    assert_eq!(
        warned(&inherited),
        [
            line("yScale", "sqrt", "root/vconcat[0] (`upper`)"),
            line("yScale", "sqrt", "root/vconcat[1]"),
        ]
    );

    let overridden = spec_of("plotDefaults:\n  yScale: sqrt", "    yScale: log", "");
    assert_eq!(
        warned(&overridden),
        [line("yScale", "sqrt", "root/vconcat[1]")],
        "the top plot sets its own and is not named"
    );
}

/// What the warning names is what the plot draws: every plot it is raised for
/// resolves linear, and every plot it is not raised for keeps a scale it set.
#[test]
fn a_plot_that_is_named_is_drawn_linear() {
    let spec = spec_of("", "    xScale: log\n    yScale: sqrt", "    yScale: log");
    let named: Vec<String> = warned(&spec).into_iter().map(|(_, _, plot)| plot).collect();
    assert_eq!(named, ["root/vconcat[0] (`upper`)"]);

    let plots = collect_plot_nodes(&spec);
    let upper = resolve_plot_scales_in(plots[0].1, &spec.params);
    assert_eq!((upper.x, upper.y), (ScaleType::Log, ScaleType::Linear));
    let lower = resolve_plot_scales_in(plots[1].1, &spec.params);
    assert_eq!((lower.x, lower.y), (ScaleType::Linear, ScaleType::Log));
}
