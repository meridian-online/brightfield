//! **A spec whose plot cannot hold its own margins is refused where it is
//! loaded**, with a diagnostic naming the plot and the margin or dimension at
//! fault — rather than composed onto an inverted data area that every consumer
//! downstream, the brush among them, then has to defend itself against.
//!
//! Driven through [`compose_spec_str`], the load path the window opens a spec
//! through, so what is asserted is what an author sees. The refusal itself is
//! `brightfield_spec::layout::plot_frame_fault`, called from `parse_spec`.

use brightfield_shell::pipeline::compose_spec_str;
use brightfield_spec::ast::SpecValue;
use brightfield_spec::layout::{collect_plot_nodes, plot_frame_fault};
use brightfield_spec::parse_spec_path;

/// Inline rows every spec below reads: two numeric columns and a category.
const DATA: &str = "data:\n  t:\n    - { a: 1, b: 2, c: p }\n    - { a: 4, b: 9, c: q }\n    \
                    - { a: 7, b: 5, c: p }\n";

/// A dot plot at the root, with `attrs` as the plot's own attributes.
fn dot_plot(attrs: &[&str]) -> String {
    let mut out =
        format!("{DATA}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: b\n");
    for attr in attrs {
        out.push_str(attr);
        out.push('\n');
    }
    out
}

/// The load error for `source`, insisting there is one.
fn refused(source: &str) -> String {
    match compose_spec_str(source, None) {
        Ok(_) => panic!("the spec loaded, and it should have been refused:\n{source}"),
        Err(e) => e,
    }
}

/// Insist `source` loads.
fn loads(source: &str) {
    if let Err(e) = compose_spec_str(source, None) {
        panic!("the spec was refused, and it should have loaded: {e}\n{source}");
    }
}

/// **The claim.** A declared left margin wider than the plot is refused, and
/// the diagnostic names the plot, the margin and the width it overran.
///
/// The control is the same plot with a margin that fits: it must load, or the
/// refusal is measuring a harness that refuses everything.
#[test]
fn a_declared_margin_wider_than_the_plot_is_refused_naming_the_plot_and_the_margin() {
    let err = refused(&dot_plot(&[
        "marginLeft: 700",
        "width: 640",
        "xLabel: null",
        "yLabel: null",
    ]));
    for needle in [
        "plot `root`",
        "marginLeft 700 (declared)",
        "marginRight 20 (default)",
        "width 640",
    ] {
        assert!(
            err.contains(needle),
            "the diagnostic names {needle:?}: {err}"
        );
    }

    loads(&dot_plot(&[
        "marginLeft: 500",
        "width: 640",
        "xLabel: null",
        "yLabel: null",
    ]));
}

/// A width or height that is zero, negative or NaN is refused, naming which.
#[test]
fn a_size_that_is_not_a_positive_number_is_refused() {
    for (attr, key) in [
        ("width: 0", "width"),
        ("width: -40", "width"),
        ("width: .nan", "width"),
        ("height: 0", "height"),
        ("height: -1", "height"),
        ("height: .nan", "height"),
    ] {
        let err = refused(&dot_plot(&[attr]));
        assert!(
            err.contains(&format!("its {key} ")) && err.contains("is not a positive number"),
            "`{attr}` is refused naming {key}: {err}"
        );
    }
}

/// **Default margins grown by a title are judged, not just declared ones.**
/// A 60-pixel-tall plot fits the default top and bottom margins, 20 and 30,
/// with 10 to spare; the x-axis title it derives from `x: a` grows the bottom
/// by a 20-pixel band and it no longer does. The same plot with its x title
/// suppressed is the control. A plot title grows the top the same way.
#[test]
fn default_margins_grown_by_a_title_are_judged() {
    let err = refused(&dot_plot(&["height: 60", "yLabel: null"]));
    for needle in ["marginBottom 50 (default 30 + title band 20)", "height 60"] {
        assert!(
            err.contains(needle),
            "the diagnostic names {needle:?}: {err}"
        );
    }
    loads(&dot_plot(&["height: 60", "xLabel: null", "yLabel: null"]));

    let err = refused(&dot_plot(&[
        "height: 80",
        "title: Readings",
        "xLabel: Temperature",
        "yLabel: null",
    ]));
    for needle in [
        "marginTop 40 (default 20 + title band 20)",
        "marginBottom 50 (default 30 + title band 20)",
        "height 80",
    ] {
        assert!(
            err.contains(needle),
            "the diagnostic names {needle:?}: {err}"
        );
    }
}

/// A plot inside a concat is named by its component path and by the `name:`
/// its author gave it, and a well-formed sibling does not mask it.
#[test]
fn the_diagnostic_names_a_nested_plot_by_its_path_and_its_name() {
    let source = format!(
        "{DATA}hconcat:\n  - plot:\n    - mark: dot\n      data: {{ from: t }}\n      x: a\n      y: b\n    \
         width: 300\n  - plot:\n    - mark: dot\n      data: {{ from: t }}\n      x: a\n      y: b\n    \
         name: wide\n    width: 300\n    marginRight: 900\n"
    );
    let err = refused(&source);
    for needle in [
        "root/hconcat[1] (`wide`)",
        "marginRight 900 (declared)",
        "width 300",
    ] {
        assert!(
            err.contains(needle),
            "the diagnostic names {needle:?}: {err}"
        );
    }
}

/// **The parse counts every title band the layout draws.** For five mark
/// shapes — a dot plot on two columns, a histogram binning one column and
/// counting, bars summing one column by a category, and a dot plot with its x
/// and then its y bound to a `$param` — the plot is composed once at a size
/// that fits, and the margins the composition actually laid it out with are
/// read back. One pixel less than those margins, along either dimension, must
/// be refused: a parse that counted fewer title bands than the layout grows
/// would let that plot load onto an inverted data area.
///
/// Between them the shapes bind a positional axis in each form the render
/// crate's channel map titles: a column, a bin, an aggregate and a `$param`.
/// The parse reads the bin as `SpecValue::Bin` and the param as a
/// `ValueOrParamRef::Param`, neither of them a column name, which is why each
/// has a shape of its own.
#[test]
fn the_parse_counts_every_title_band_the_layout_draws() {
    const PARAM: &str = "params:\n  p: 3\n";
    let shapes = [
        (
            "dots",
            "",
            "mark: dot\n    data: { from: t }\n    x: a\n    y: b",
        ),
        (
            "histogram",
            "",
            "mark: rectY\n    data: { from: t }\n    x: { bin: a }\n    y: { count: }",
        ),
        (
            "summed bars",
            "",
            "mark: barY\n    data: { from: t }\n    x: c\n    y: { sum: b }",
        ),
        (
            "dots on a param x",
            PARAM,
            "mark: dot\n    data: { from: t }\n    x: $p\n    y: b",
        ),
        (
            "dots on a param y",
            PARAM,
            "mark: dot\n    data: { from: t }\n    x: a\n    y: $p",
        ),
    ];
    for (shape, params, mark) in shapes {
        let spec =
            |w: f64, h: f64| format!("{params}{DATA}plot:\n  - {mark}\nwidth: {w}\nheight: {h}\n");
        let composed = compose_spec_str(&spec(400.0, 300.0), None)
            .unwrap_or_else(|e| panic!("{shape} composes at 400 x 300: {e}"));
        let m = composed.plots[0].layout.margins();
        assert!(
            m.left > 40.0 && m.bottom > 30.0,
            "{shape}: the layout grew no title band ({m:?}), so this measures nothing"
        );

        let across = m.left + m.right;
        let down = m.top + m.bottom;
        let err = refused(&spec(across - 1.0, 300.0));
        assert!(
            err.contains("width"),
            "{shape}, width {}: {err}",
            across - 1.0
        );
        let err = refused(&spec(400.0, down - 1.0));
        assert!(
            err.contains("height"),
            "{shape}, height {}: {err}",
            down - 1.0
        );
    }
}

/// **The curated `legends.yaml` fixture loads**, and would still load if the
/// `margin: 0`, `width: 0` and `height: 20` it writes under `plotDefaults`
/// reached its plots.
///
/// No plot reads `plotDefaults` today, so the fixture's plots are judged at
/// the default size; the second half writes the three keys onto every plot
/// and asks again. Each of those plots hosts a legend and holds no mark, so it
/// draws no data area and is not judged — which is what keeps this fixture's
/// deliberate zero-size frames out of a refusal meant for plots that draw.
#[test]
fn the_curated_legends_fixture_loads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-conformance/vendor/curated/yaml/legends.yaml");
    let parsed = parse_spec_path(&path).unwrap_or_else(|e| panic!("{} loads: {e}", path.display()));

    let plots = collect_plot_nodes(&parsed.spec);
    assert!(!plots.is_empty(), "the fixture holds plots");
    for (at, plot) in plots {
        let mut zero_frame = plot.clone();
        for (key, value) in [("margin", 0), ("width", 0), ("height", 20)] {
            zero_frame
                .attributes
                .insert(key.to_string(), SpecValue::Integer(value));
        }
        assert_eq!(
            plot_frame_fault(&zero_frame),
            None,
            "{at}, with the fixture's plotDefaults written onto it, is refused"
        );
    }
}
