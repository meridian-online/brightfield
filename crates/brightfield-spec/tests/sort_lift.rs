//! Gate: a `sort:` is lifted or it is SAID — never quietly dropped.
//!
//! `sort` is on `CONSUMED_MARK_OPTION_KEYS`, so the key-level check that used
//! to report it passes now and cannot report the shapes no lowerer computes.
//! That is the same split `x` has lived with since the channel-transform check
//! went in, and it has the same failure mode: a key on the list plus a value
//! shape with no reader is silence, which is exactly what this card's defect
//! was.
//!
//! So each refusal in the `mark_sort` resolver is exercised here from the
//! outside, through `parse_spec`, and asserted to earn a line.

use brightfield_spec::ast::{Component, Mark, SpecValue, ValueOrParamRef};
use brightfield_spec::{parse_spec, Format, ParseWarning};

/// A one-mark spec with `sort:` written as given.
fn spec_with_sort(mark: &str, channels: &str, sort: &str) -> String {
    format!("plot:\n- mark: {mark}\n  data: {{ from: t }}\n{channels}  sort: {sort}\n")
}

/// Every mark in the component tree, in source order. `brightfield-sql`'s
/// `collect_marks` does this for the emitter, and this crate cannot see it.
fn marks(component: &Component, out: &mut Vec<Mark>) {
    match component {
        Component::Mark(m) => out.push(m.clone()),
        Component::Plot(p) => p.items.iter().for_each(|i| marks(i, out)),
        Component::HConcat(c) | Component::VConcat(c) => {
            c.items.iter().for_each(|i| marks(i, out));
        }
        _ => {}
    }
}

/// The `sort:` option as the parse left it on the spec's first mark.
fn sort_option(source: &str) -> Option<SpecValue> {
    let out = parse_spec(source, Format::Yaml).expect("spec parses");
    let mut found = Vec::new();
    marks(out.spec.root.as_ref().expect("a visible root"), &mut found);
    match found.first().expect("one mark").options.get("sort") {
        Some(ValueOrParamRef::Value(v)) => Some(v.clone()),
        _ => None,
    }
}

/// The same, narrowed to the LIFTED form. A refused `sort:` is still carried
/// through the AST as the map the author wrote, so "is there a `sort` key" is
/// not the question — "did it become an order a lowerer can read" is.
fn lifted(source: &str) -> Option<SpecValue> {
    sort_option(source).filter(|v| matches!(v, SpecValue::Sort { .. }))
}

/// The rendered `UnconsumedSort` lines a spec earns.
fn sort_lines(source: &str) -> Vec<String> {
    let out = parse_spec(source, Format::Yaml).expect("spec parses");
    out.warnings
        .iter()
        .filter(|w| matches!(w, ParseWarning::UnconsumedSort { .. }))
        .map(std::string::ToString::to_string)
        .collect()
}

/// The corpus case, read off the vendored upstream spec rather than a fixture
/// written to match the implementation.
#[test]
fn the_vendored_sorted_bars_sort_is_lifted_whole() {
    let out = parse_spec(
        include_str!("../vendor/mosaic-specs/yaml/sorted-bars.yaml"),
        Format::Yaml,
    )
    .expect("spec parses");
    let mut found = Vec::new();
    marks(out.spec.root.as_ref().expect("a visible root"), &mut found);
    assert_eq!(found.len(), 1, "the spec has one mark");
    assert_eq!(
        found[0].options.get("sort").cloned(),
        Some(ValueOrParamRef::Value(SpecValue::Sort {
            channel: "y".to_string(),
            by: "x".to_string(),
            descending: true,
            limit: Some(10),
        })),
        "`sort: {{ y: -x, limit: 10 }}` on a barX is the band ordered by the \
         value, descending, ten bars"
    );
}

/// A `sort:` with no `limit:` is a sort with no limit, not a sort with a
/// defaulted one — `athlete-height.yaml` writes exactly that form.
#[test]
fn a_sort_without_a_limit_carries_no_limit() {
    let src = spec_with_sort("barX", "  x: { sum: v }\n  y: region\n", "{ y: -x }");
    assert_eq!(
        lifted(&src),
        Some(SpecValue::Sort {
            channel: "y".to_string(),
            by: "x".to_string(),
            descending: true,
            limit: None,
        })
    );
    assert!(sort_lines(&src).is_empty(), "and it says nothing");
}

/// No `-` is ascending. Mosaic spells descending with the prefix and nothing
/// else, so the absence has to mean the other direction rather than "unset".
#[test]
fn no_minus_prefix_is_ascending() {
    let src = spec_with_sort("barX", "  x: { sum: v }\n  y: region\n", "{ y: x }");
    assert_eq!(
        lifted(&src),
        Some(SpecValue::Sort {
            channel: "y".to_string(),
            by: "x".to_string(),
            descending: false,
            limit: None,
        })
    );
}

/// `barY` is the transpose: the band is `x` and the value is `y`, decided by
/// the KIND. The same map on the two kinds means opposite things, and getting
/// this from the map rather than the kind is how a chart comes out sideways.
#[test]
fn bar_y_takes_the_transposed_pair() {
    let bar_y = spec_with_sort("barY", "  y: { sum: v }\n  x: region\n", "{ x: -y }");
    assert_eq!(
        lifted(&bar_y),
        Some(SpecValue::Sort {
            channel: "x".to_string(),
            by: "y".to_string(),
            descending: true,
            limit: None,
        })
    );
    // And the barX form on a barY is refused rather than transposed for it.
    let wrong_way = spec_with_sort("barY", "  y: { sum: v }\n  x: region\n", "{ y: -x }");
    assert!(lifted(&wrong_way).is_none());
    assert_eq!(sort_lines(&wrong_way).len(), 1);
}

/// Every refusal, each one line. The table is the point: a resolver with six
/// bail-outs and one test proves one of them.
#[test]
fn each_refused_shape_earns_exactly_one_line() {
    let value_axis = "  x: { sum: v }\n  y: region\n";
    for (case, mark, channels, sort) in [
        // The sort key is the VALUE axis — a continuous scale with no order to
        // set.
        ("sorts the value axis", "barX", value_axis, "{ x: -y }"),
        // The value names a channel the mark does not bind.
        ("orders by an absent channel", "barX", value_axis, "{ y: -z }"),
        // A limit of zero is a chart with nothing in it.
        ("limit: 0", "barX", value_axis, "{ y: -x, limit: 0 }"),
        ("negative limit", "barX", value_axis, "{ y: -x, limit: -3 }"),
        // A key beside the channel and `limit`. Mosaic's `sort:` takes
        // `reverse` and `reduce` too, and neither reaches a lowerer.
        ("an unread modifier", "barX", value_axis, "{ y: -x, reverse: true }"),
        // Not a map at all.
        ("a bare string", "barX", value_axis, "x"),
        // A kind with no band axis.
        ("no band axis", "dot", "  x: a\n  y: b\n", "{ y: -x }"),
    ] {
        let src = spec_with_sort(mark, channels, sort);
        assert!(
            lifted(&src).is_none(),
            "{case}: must not lift, but did — {:?}",
            lifted(&src)
        );
        assert!(
            sort_option(&src).is_some(),
            "{case}: a refused sort is still carried through the AST"
        );
        let lines = sort_lines(&src);
        assert_eq!(lines.len(), 1, "{case}: must earn exactly one line: {lines:?}");
        assert!(
            lines[0].contains("`sort:`") && lines[0].contains("`limit:`"),
            "{case}: the line must name the option and say the nested `limit:` \
             went with it: {}",
            lines[0]
        );
    }
}

/// An unimplemented mark already says it draws nothing. `athlete-height.yaml`
/// sorts an `errorbarX`, and itemising the options of a mark with no renderer
/// is noise on top of the line that matters.
#[test]
fn an_unimplemented_mark_does_not_report_its_sort() {
    let src = spec_with_sort("errorbarX", "  x: a\n  y: b\n", "{ y: -x }");
    let out = parse_spec(&src, Format::Yaml).expect("spec parses");
    assert!(
        out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::Unimplemented { .. })),
        "the mark still says it is unimplemented"
    );
    assert!(sort_lines(&src).is_empty(), "and the sort is not itemised");
}

/// Layer 1 is `parse → serialise → parse` idempotence, and a typed lift is
/// where that breaks: the map has to come back out of a variant that no longer
/// holds it as a map. Asserted on the vendored spec, which is the one the
/// conformance runner reads.
#[test]
fn the_lift_survives_a_serialise_and_reparse() {
    let source = include_str!("../vendor/mosaic-specs/yaml/sorted-bars.yaml");
    let first = parse_spec(source, Format::Yaml).expect("spec parses").spec;
    let round_tripped = serde_yaml::to_string(&first).expect("spec serialises");
    assert!(
        round_tripped.contains("-x"),
        "the descending prefix must be written back: {round_tripped}"
    );
    assert!(
        round_tripped.contains("limit: 10"),
        "and so must the limit: {round_tripped}"
    );
    let second = parse_spec(&round_tripped, Format::Yaml)
        .expect("the serialised spec parses")
        .spec;
    assert_eq!(first, second, "parse → serialise → parse must be idempotent");
}
