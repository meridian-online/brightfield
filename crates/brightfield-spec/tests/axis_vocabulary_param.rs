//! **An axis key a plot gives through a `$param` is read as the value the param
//! holds now, as the same value written in the file is.**
//!
//! The six groups — `xTicks`/`yTicks`, `xTickFormat`/`yTickFormat`,
//! `grid`/`xGrid`/`yGrid`, `xZero`/`yZero`, `xNice`/`yNice`, `xReverse`/`yReverse`
//! — were read from the plot's own attributes, so a `$param` on any of them drew
//! brightfield's default with no line to say why. The five resolvers each have an
//! `_in` form that takes the spec's params, and
//! [`param_held_axis_warnings`] names a param that holds what the key's judge
//! refuses.
//!
//! The spec is parsed twice, once with the value written in the key and once
//! with the key a `$param` that holds it, and the readings are compared. Every
//! arm that is meant to move the plot is paired with the same plot with no key,
//! because a fixture whose default is the value asked for would pass without the
//! param being read.

use brightfield_spec::layout::{
    collect_plot_nodes, param_held_axis_warnings, resolve_axis_ends_in, resolve_axis_reverse_in,
    resolve_grid_lines_in, resolve_tick_counts_in, resolve_tick_formats_in,
};
use brightfield_spec::{parse_spec, Format, ParseOutput, ParseWarning};

fn parsed(params: &str, attrs: &str) -> ParseOutput {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    parse_spec(&source, Format::Yaml).unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"))
}

/// Everything the five resolvers read from the first plot, as one comparable
/// string, reading through the spec's own params.
fn read(out: &ParseOutput) -> String {
    let plot = collect_plot_nodes(&out.spec)
        .into_iter()
        .next()
        .expect("one plot")
        .1
        .clone();
    let params = &out.spec.params;
    format!(
        "{:?}",
        (
            resolve_tick_counts_in(&plot, params),
            resolve_tick_formats_in(&plot, params),
            resolve_grid_lines_in(&plot, params),
            resolve_axis_ends_in(&plot, params),
            resolve_axis_reverse_in(&plot, params),
        )
    )
}

/// The param-held warnings the first plot raises.
fn held_warnings(out: &ParseOutput) -> Vec<ParseWarning> {
    let plot = collect_plot_nodes(&out.spec)
        .into_iter()
        .next()
        .expect("one plot")
        .1
        .clone();
    param_held_axis_warnings(&plot, &out.spec.params)
}

/// The warnings of the six groups the parser raised, whichever key.
fn parse_family(out: &ParseOutput) -> Vec<ParseWarning> {
    out.warnings
        .iter()
        .filter(|w| {
            matches!(
                w,
                ParseWarning::InvalidTickCount { .. }
                    | ParseWarning::InvalidTickFormat { .. }
                    | ParseWarning::UnreadDateDirective { .. }
                    | ParseWarning::InvalidGridSwitch { .. }
                    | ParseWarning::InvalidAxisEndSwitch { .. }
                    | ParseWarning::InvalidAxisReverseSwitch { .. }
            )
        })
        .cloned()
        .collect()
}

/// One value on one key, as YAML, and whether it moves the plot from what a file
/// without the key draws.
type Case = (&'static str, &'static str, bool);

/// A value the key reads, for each of the six groups and both axes.
const READ: [Case; 28] = [
    ("xTicks", "7", true),
    ("yTicks", "3", true),
    ("xTicks", "2.0", true),
    ("xTickFormat", "\"s\"", true),
    ("xTickFormat", "\".2s\"", true),
    ("xTickFormat", "\"%b\"", true),
    ("xTickFormat", "\"%Y-%m-%d\"", true),
    ("yTickFormat", "\"+.1f\"", true),
    ("grid", "false", true),
    ("grid", "true", false),
    ("xGrid", "false", true),
    ("yGrid", "false", true),
    ("xZero", "true", true),
    ("yZero", "true", true),
    ("xZero", "false", false),
    ("xNice", "true", true),
    ("yNice", "true", true),
    ("yNice", "false", false),
    ("xReverse", "true", true),
    ("yReverse", "true", true),
    ("xReverse", "false", false),
    ("yReverse", "false", false),
    ("xTickFormat", "null", false),
    ("yTickFormat", "null", false),
    ("xTicks", "10", true),
    ("yZero", "false", false),
    ("xNice", "false", false),
    ("yGrid", "true", false),
];

/// **AC1.** A key given through a param that holds a value the key reads is read
/// as that value, and a key whose value moves the plot moves it through the param.
#[test]
fn a_param_holding_a_literal_resolves_as_the_literal_does() {
    for (key, value, moves) in READ {
        let literal = parsed("", &format!("{key}: {value}\n"));
        let through = parsed(&format!("params:\n  p: {value}\n"), &format!("{key}: $p\n"));
        let none = parsed("", "");
        assert_eq!(
            read(&through),
            read(&literal),
            "{key} through a param holding {value} reads as {value} written in"
        );
        if moves {
            assert_ne!(
                read(&literal),
                read(&none),
                "{key}: {value} must move the reading, or the arm above proves nothing"
            );
        }
    }
}

/// **AC1.** The reading follows the value the param holds, not the value it held
/// when the file was parsed: a param read after it is written answers with the
/// new value.
#[test]
fn a_param_read_after_it_is_written_answers_with_the_new_value() {
    let mut out = parsed("params:\n  p: 3\n", "xTicks: $p\n");
    let at_three = read(&out);
    let literal = |value: &str| read(&parsed("", &format!("xTicks: {value}\n")));
    assert_eq!(at_three, literal("3"));
    out.spec.params.insert(
        "p".to_string(),
        brightfield_spec::ast::ParamNode::Value(brightfield_spec::SpecValue::Integer(9)),
    );
    assert_eq!(read(&out), literal("9"), "after the write the key reads 9");
    assert_ne!(read(&out), at_three);
}

/// **AC2.** A param the file never declares reads as the key absent.
#[test]
fn a_param_nobody_declared_reads_as_the_key_absent() {
    let none = read(&parsed("", ""));
    for (key, _, _) in READ {
        let out = parsed("", &format!("{key}: $nobody\n"));
        assert_eq!(read(&out), none, "{key} through an undeclared param");
        assert!(
            held_warnings(&out).is_empty(),
            "{key} through an undeclared param raises nothing"
        );
    }
}

/// **AC2.** A param that holds a selection holds no value, so the key reads as
/// absent.
#[test]
fn a_param_that_holds_a_selection_reads_as_the_key_absent() {
    let none = read(&parsed("", ""));
    for (key, _, _) in READ {
        let out = parsed(
            "params:\n  p:\n    select: intersect\n",
            &format!("{key}: $p\n"),
        );
        assert_eq!(read(&out), none, "{key} through a selection");
        assert!(
            held_warnings(&out).is_empty(),
            "{key} through a selection raises nothing"
        );
    }
}

/// A value the key's judge refuses, for each of the six groups.
const REFUSED: [(&str, &str); 24] = [
    ("xTicks", "0"),
    ("yTicks", "-3"),
    ("xTicks", "2.5"),
    ("xTicks", "ten"),
    ("xTicks", "100000"),
    ("xTickFormat", "\"~~\""),
    ("xTickFormat", "\".f\""),
    ("yTickFormat", "7"),
    ("xTickFormat", "\"%K\""),
    ("yTickFormat", "\"%Y-%K\""),
    ("xTickFormat", "\"abc%\""),
    ("grid", "yes"),
    ("xGrid", "1"),
    ("yGrid", "\"false\""),
    ("grid", "0.5"),
    ("xZero", "yes"),
    ("yZero", "1"),
    ("xNice", "5"),
    ("yNice", "\"true\""),
    ("xReverse", "1"),
    ("yReverse", "yes"),
    ("xReverse", "\"true\""),
    ("xTicks", "true"),
    ("grid", "[true]"),
];

/// **AC1.** A param that holds a value its key refuses raises the warning the
/// same value written in the file raises — the same variant, key and value text —
/// where the parser, having no value to judge, said nothing.
#[test]
fn a_param_holding_a_bad_literal_raises_the_warning_the_literal_raises() {
    for (key, value) in REFUSED {
        let literal = parsed("", &format!("{key}: {value}\n"));
        let through = parsed(&format!("params:\n  p: {value}\n"), &format!("{key}: $p\n"));
        let named = parse_family(&literal);
        assert_eq!(
            named.len(),
            1,
            "{key}: {value} written in the file is named once by the parser: {named:?}"
        );
        assert!(
            parse_family(&through).is_empty(),
            "the parser says nothing of {key}: $p, which is a deferral"
        );
        assert_eq!(
            held_warnings(&through),
            named,
            "{key} through a param holding {value} is named as {value} written in is"
        );
    }
}

/// **AC1.** A value the key reads raises nothing through a param, as it raises
/// nothing written in the file — including a format of either kind.
#[test]
fn a_param_holding_a_value_the_key_reads_raises_nothing() {
    for (key, value, _) in READ {
        let through = parsed(&format!("params:\n  p: {value}\n"), &format!("{key}: $p\n"));
        let literal = parsed("", &format!("{key}: {value}\n"));
        assert!(
            parse_family(&literal).is_empty(),
            "{key}: {value} is read written in"
        );
        assert!(
            held_warnings(&through).is_empty(),
            "{key} through a param holding {value}: {:?}",
            held_warnings(&through)
        );
    }
}

/// A key written as a literal is the parser's to name, once; the reading that
/// follows a param does not name it again.
#[test]
fn a_literal_key_is_named_by_the_parser_and_not_again_through_params() {
    for (key, value) in REFUSED {
        let literal = parsed("", &format!("{key}: {value}\n"));
        assert!(
            held_warnings(&literal).is_empty(),
            "{key}: {value} is named at parse time and has no param to name again"
        );
    }
}
