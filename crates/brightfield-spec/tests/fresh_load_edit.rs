//! Gate: an edit whose page is loaded afresh may change a derived axis title,
//! and the reload-from-disk path still may not.
//!
//! `edit::apply` refuses an edit that would change a derived axis title,
//! because a reload from disk swaps new scenes into chrome laid out when the
//! window launched. `edit::apply_for_fresh_load` is for the caller that builds
//! the page again from the edited spec, which draws the new title. Both are
//! read here over the same edit on the same map, so a change to either path
//! reddens a test that names it.

use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Component, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::{
    apply, apply_for_fresh_load, classify_edit, plot_at_path, ChartEdit, RefuseReason,
};
use brightfield_spec::{parse_spec, Format, Spec};

/// The generated map's shape: a ghost dot layer and a subset dot layer, each
/// binding its own x and y, and the projection at plot level with no axis
/// label. The spec crate cannot call the shell's generator, so the shape is
/// written out; `crates/brightfield-shell/tests/shelf_edit.rs` reads the
/// generator's own output.
const HERO_MAP: &str = "\
params:
  brush: { select: crossfilter }
data:
  t: SELECT -122.4 AS longitude, 37.8 AS latitude, 3.1 AS median_income
plot:
  - mark: dot
    data: { from: t }
    x: longitude
    y: latitude
    fill: \"#cccccc\"
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: longitude
    y: latitude
  - select: intervalXY
    as: $brush
projectionType: equirectangular
width: 640
height: 400
";

fn hero() -> Spec {
    parse_spec(HERO_MAP, Format::Yaml)
        .unwrap_or_else(|e| panic!("the fixture parses: {e}"))
        .spec
}

fn set_x(mark_ordinal: usize, column: &str) -> ChartEdit {
    ChartEdit::SetChannel {
        plot: ComponentPath("root".to_string()),
        mark_ordinal,
        channel: "x".to_string(),
        column: column.to_string(),
    }
}

/// The column each dot layer binds x to, in the plot's mark order.
fn x_columns(spec: &Spec) -> Vec<String> {
    plot_at_path(spec, "root")
        .expect("the root plot")
        .items
        .iter()
        .filter_map(|c| match c {
            Component::Mark(m) => match m.options.get("x") {
                Some(ValueOrParamRef::Value(SpecValue::String(s))) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// **A reload from disk of a spec whose axis title changed is still refused.**
///
/// The map's plot carries no `xLabel`, so its x title is derived from the
/// first layer's column; putting `median_income` there changes it from
/// `longitude`. `apply` is the reload-from-disk path's entry, and it refuses
/// with the reason that path reports as a restart, leaving the spec equal.
#[test]
fn the_reload_from_disk_path_still_refuses_a_derived_axis_title_change() {
    let mut spec = hero();
    let before = spec.clone();

    assert_eq!(
        classify_edit(&spec, &set_x(0, "median_income")),
        Err(RefuseReason::WouldChangeAxisTitle),
        "the classifier no longer sees the derived x title change"
    );
    assert_eq!(
        apply(&mut spec, &set_x(0, "median_income")),
        Err(RefuseReason::WouldChangeAxisTitle),
        "apply let a derived axis title change through on the reload path"
    );
    assert_eq!(spec, before, "a refused edit changed the spec");
}

/// **The same edit, for a page loaded afresh, is applied** — on both layers,
/// one edit each, so the ghost and the subset draw the same column.
#[test]
fn a_derived_axis_title_change_is_applied_for_a_fresh_load() {
    let mut spec = hero();

    apply_for_fresh_load(&mut spec, &set_x(0, "median_income"))
        .expect("a fresh load draws the new title, so nothing refuses it");
    apply_for_fresh_load(&mut spec, &set_x(1, "median_income"))
        .expect("the second layer's x is the same edit");

    assert_eq!(
        x_columns(&spec),
        ["median_income", "median_income"],
        "both dot layers should read x: median_income"
    );
}

/// **What no page could be built over is still refused**, with the spec left
/// equal: a plot path that names no plot, a mark ordinal past the plot's
/// marks, and a removal of a plot's last mark.
#[test]
fn a_fresh_load_edit_still_refuses_an_edit_with_no_target() {
    let cases = [
        (
            ChartEdit::SetChannel {
                plot: ComponentPath("root/vconcat[3]".to_string()),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "median_income".to_string(),
            },
            HERO_MAP,
            RefuseReason::PlotNotFound,
        ),
        (
            set_x(2, "median_income"),
            HERO_MAP,
            RefuseReason::NoSuchMark,
        ),
        (
            ChartEdit::RemoveMark {
                plot: ComponentPath("root".to_string()),
                mark_ordinal: 0,
            },
            "data:\n  t: SELECT 1 AS a\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n",
            RefuseReason::WouldEmptyPlot,
        ),
    ];
    for (edit, source, reason) in cases {
        let mut spec = parse_spec(source, Format::Yaml)
            .unwrap_or_else(|e| panic!("the fixture parses: {e}"))
            .spec;
        let before = spec.clone();
        assert_eq!(
            apply_for_fresh_load(&mut spec, &edit),
            Err(reason.clone()),
            "{edit:?} should be refused as {reason:?}"
        );
        assert_eq!(spec, before, "the refused {edit:?} changed the spec");
    }
}
