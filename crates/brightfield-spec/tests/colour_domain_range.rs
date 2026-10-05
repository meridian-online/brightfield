//! **A plot's `colorDomain` is two numbers or a list of categories, and its
//! `colorRange` a list of colours; the resolvers read each from the plot or from
//! the value param it names, and read nothing from any other shape.**
//!
//! The shell draws a plot through these, so a value they read as no domain is a
//! value the plot draws as a file without the key draws: `Fixed`, a list of one
//! number or three, a pair with its high end first, and a list that mixes strings
//! with numbers.

use brightfield_spec::layout::{
    collect_plot_nodes, resolve_colour_domain, resolve_colour_range, ColourDomain,
};
use brightfield_spec::{parse_spec, Format};

/// Parse a one-plot spec carrying `params` and `attrs`, and resolve its two keys.
fn resolved(params: &str, attrs: &str) -> (Option<ColourDomain>, Option<Vec<String>>) {
    let source = format!(
        "{params}data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n{attrs}"
    );
    let out = parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the spec parses: {e}\n{source}"));
    let plot = collect_plot_nodes(&out.spec)
        .into_iter()
        .next()
        .expect("one plot")
        .1
        .clone();
    (
        resolve_colour_domain(&plot, &out.spec.params),
        resolve_colour_range(&plot, &out.spec.params)
            .map(|names| names.into_iter().map(str::to_string).collect()),
    )
}

fn domain(attrs: &str) -> Option<ColourDomain> {
    resolved("", attrs).0
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// Two numbers, low then high, are a domain's ends, written as integers or as
/// floats; a list of strings is a category list in its written order.
#[test]
fn a_pair_of_numbers_is_the_ends_and_a_list_of_strings_is_the_categories() {
    assert_eq!(
        domain("colorDomain: [0, 10]\n"),
        Some(ColourDomain::Ends(0.0, 10.0))
    );
    assert_eq!(
        domain("colorDomain: [-2.5, 7.25]\n"),
        Some(ColourDomain::Ends(-2.5, 7.25))
    );
    assert_eq!(
        domain("colorDomain: [c, a, b]\n"),
        Some(ColourDomain::Categories(names(&["c", "a", "b"])))
    );
    assert_eq!(
        domain("colorDomain: [only]\n"),
        Some(ColourDomain::Categories(names(&["only"]))),
        "one category is a list of categories"
    );
}

/// Every other shape is no domain: it draws as the key absent.
#[test]
fn every_other_shape_is_no_domain() {
    for attrs in [
        "",
        "colorDomain: Fixed\n",
        "colorDomain: null\n",
        "colorDomain: 5\n",
        "colorDomain: []\n",
        "colorDomain: [7]\n",
        "colorDomain: [0, 5, 10]\n",
        "colorDomain: [10, 0]\n",
        "colorDomain: [4, 4]\n",
        "colorDomain: [a, 3]\n",
        "colorDomain: [3, a]\n",
        "colorDomain: [true, false]\n",
        "colorDomain: [[0, 1], [2, 3]]\n",
    ] {
        assert_eq!(domain(attrs), None, "{attrs:?}");
    }
}

/// A `colorRange` is a non-empty list of strings, as written, in order.
#[test]
fn a_range_is_a_non_empty_list_of_strings_in_order() {
    let range = |attrs: &str| resolved("", attrs).1;
    assert_eq!(
        range("colorRange: ['#ff0000', green, '#0000ff']\n"),
        Some(names(&["#ff0000", "green", "#0000ff"]))
    );
    for attrs in [
        "",
        "colorRange: viridis\n",
        "colorRange: []\n",
        "colorRange: null\n",
        "colorRange: [red, 3]\n",
        "colorRange: [[1, 2]]\n",
    ] {
        assert_eq!(range(attrs), None, "{attrs:?}");
    }
}

/// A `$param` is read as the list its value param holds now; a param that holds
/// no list, a selection and a param nobody declared give nothing.
#[test]
fn a_param_is_read_as_the_list_its_value_param_holds() {
    let both = |params: &str| resolved(params, "colorDomain: $domain\ncolorRange: $colors\n");
    assert_eq!(
        both("params:\n  domain: [0, 10]\n  colors: ['#ff0000', '#0000ff']\n"),
        (
            Some(ColourDomain::Ends(0.0, 10.0)),
            Some(names(&["#ff0000", "#0000ff"]))
        )
    );
    assert_eq!(
        both("params:\n  domain: [a, b]\n  colors: [red]\n"),
        (
            Some(ColourDomain::Categories(names(&["a", "b"]))),
            Some(names(&["red"]))
        )
    );
    assert_eq!(
        both("params:\n  domain: Fixed\n  colors: viridis\n"),
        (None, None),
        "a param holding no list is no key"
    );
    assert_eq!(
        both("params:\n  other: [0, 10]\n"),
        (None, None),
        "a param nobody declared is no key"
    );
    assert_eq!(
        both("params:\n  domain: { select: crossfilter }\n  colors: { select: crossfilter }\n"),
        (None, None),
        "a selection holds no list"
    );
}
