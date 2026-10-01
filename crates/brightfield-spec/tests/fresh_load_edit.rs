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
    apply, apply_for_fresh_load, classify_edit, colour_legend_covers, plot_at_path, ChartEdit,
    RefuseReason,
};
use brightfield_spec::vocab::LegendChannel;
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

fn add_legend(plot: &str) -> ChartEdit {
    ChartEdit::AddColourLegend {
        plot: ComponentPath(plot.to_string()),
    }
}

/// The plot's legend items, as the channel each draws, in item order.
fn legends(spec: &Spec, plot: &str) -> Vec<LegendChannel> {
    plot_at_path(spec, plot)
        .expect("the plot")
        .items
        .iter()
        .filter_map(|c| match c {
            Component::Legend(l) => Some(l.channel),
            _ => None,
        })
        .collect()
}

/// **A colour legend is appended as the plot's last item**, after the marks and
/// after the interactor that closes the map's list, and nothing else about the
/// spec changes: taking the item off again gives the spec it was appended to.
#[test]
fn a_colour_legend_edit_appends_a_legend_as_the_plots_last_item() {
    let before = hero();
    let mut spec = before.clone();

    apply_for_fresh_load(&mut spec, &add_legend("root")).expect("the plot is there");

    let items = &plot_at_path(&spec, "root").expect("the root plot").items;
    assert_eq!(items.len(), 4, "a legend should add one item to the three");
    let Some(Component::Legend(legend)) = items.last() else {
        panic!("the last item is {:?}, not a legend", items.last());
    };
    assert_eq!(legend.channel, LegendChannel::Color);
    assert!(
        legend.options.is_empty(),
        "the legend carries options: {:?}",
        legend.options
    );

    let mut taken_off = spec.clone();
    brightfield_spec::edit::plot_at_path_mut(&mut taken_off, "root")
        .expect("the root plot")
        .items
        .pop();
    assert_eq!(taken_off, before, "the edit changed more than the item");
}

/// **A plot that already holds the item is left equal**, so a second edit is not
/// a second legend. A legend for another channel is not the colour legend and
/// does not stop it.
#[test]
fn a_colour_legend_edit_on_a_plot_that_holds_the_item_changes_no_part_of_the_spec() {
    let mut spec = hero();
    apply_for_fresh_load(&mut spec, &add_legend("root")).expect("the plot is there");
    let once = spec.clone();

    apply_for_fresh_load(&mut spec, &add_legend("root")).expect("the plot is there");

    assert_eq!(spec, once, "a second edit changed the spec");
    assert_eq!(legends(&spec, "root"), [LegendChannel::Color]);

    let mut opacity = parse_spec(
        "data:\n  t: SELECT 1 AS a\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n  - legend: opacity\n",
        Format::Yaml,
    )
    .expect("the fixture parses")
    .spec;
    apply_for_fresh_load(&mut opacity, &add_legend("root")).expect("the plot is there");
    assert_eq!(
        legends(&opacity, "root"),
        [LegendChannel::Opacity, LegendChannel::Color],
        "an opacity legend stopped the colour legend being added"
    );
}

/// **An edit with no plot to take it is refused**, with the spec left equal.
#[test]
fn a_colour_legend_edit_on_no_plot_is_refused() {
    let mut spec = hero();
    let before = spec.clone();

    assert_eq!(
        apply_for_fresh_load(&mut spec, &add_legend("root/vconcat[3]")),
        Err(RefuseReason::PlotNotFound)
    );
    assert_eq!(spec, before);
}

/// Two items in a column, the first a plot named `m` and coloured by `fill`,
/// and the second the line the test names.
fn column_with(second: &str) -> Spec {
    let source = format!(
        "data:\n  t: SELECT 1 AS a, 2 AS b\nvconcat:\n  - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: a\n        fill: b\n    name: m\n  - {second}\n"
    );
    parse_spec(&source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the fixture parses: {e}"))
        .spec
}

const COLUMN_PLOT: &str = "root/vconcat[0]";

/// **A colour legend is drawn for a plot when its items hold one and when a
/// standalone one names it with `for:`**; not when the standalone one names
/// another plot, names none, or is for another channel.
#[test]
fn a_plot_has_a_colour_legend_when_its_items_or_a_standalone_legend_hold_one() {
    let mut item = column_with("vspace: 8");
    assert!(
        !colour_legend_covers(&item, COLUMN_PLOT),
        "a plot with no legend anywhere reads as covered"
    );
    apply_for_fresh_load(&mut item, &add_legend(COLUMN_PLOT)).expect("the plot is there");
    assert!(
        colour_legend_covers(&item, COLUMN_PLOT),
        "the item the edit appended is not read as a legend"
    );

    let cases = [
        (
            "legend: color\n    for: m",
            true,
            "a standalone legend that names the plot",
        ),
        (
            "legend: color\n    for: other",
            false,
            "a standalone legend that names another plot",
        ),
        (
            "legend: color",
            false,
            "a standalone legend that names no plot",
        ),
        (
            "legend: opacity\n    for: m",
            false,
            "a standalone legend for another channel",
        ),
    ];
    for (legend, covered, what) in cases {
        let spec = column_with(legend);
        assert_eq!(
            colour_legend_covers(&spec, COLUMN_PLOT),
            covered,
            "{what}: should be covered={covered}"
        );
    }

    assert!(
        !colour_legend_covers(&hero(), "root/vconcat[3]"),
        "a path that names no plot reads as covered"
    );
}

fn remove_legend(plot: &str) -> ChartEdit {
    ChartEdit::RemoveColourLegend {
        plot: ComponentPath(plot.to_string()),
    }
}

fn parse(source: &str) -> Spec {
    parse_spec(source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the fixture parses: {e}"))
        .spec
}

/// A plot that holds a colour legend between its two marks, with an interactor
/// after them: the shape where taking the item out would move a neighbour if
/// the edit took out the wrong index or rebuilt the list.
const LEGEND_BETWEEN: &str = "\
params:
  brush: { select: crossfilter }
data:
  t: SELECT 1 AS a, 2 AS b
plot:
  - mark: dot
    data: { from: t }
    x: a
    fill: b
  - legend: color
  - mark: line
    data: { from: t }
    x: a
    y: b
  - select: intervalXY
    as: $brush
width: 300
";

/// **The removal leaves the plot's items without a colour legend, and each mark
/// and each other item is where it was.** The expected spec is the one it
/// started from with that one item taken out by index, so a mark or the
/// interactor that moved, or a second item that went, is a difference.
#[test]
fn a_colour_legend_removal_takes_the_item_out_and_leaves_each_other_item_where_it_was() {
    let before = parse(LEGEND_BETWEEN);
    assert_eq!(legends(&before, "root"), [LegendChannel::Color]);
    let mut spec = before.clone();

    apply_for_fresh_load(&mut spec, &remove_legend("root")).expect("the plot is there");

    assert!(
        legends(&spec, "root").is_empty(),
        "the plot still holds a legend: {:?}",
        legends(&spec, "root")
    );
    let mut expected = before.clone();
    brightfield_spec::edit::plot_at_path_mut(&mut expected, "root")
        .expect("the root plot")
        .items
        .remove(1);
    assert_eq!(spec, expected, "the edit changed more than the one item");
    let items = &plot_at_path(&spec, "root").expect("the root plot").items;
    assert_eq!(
        items.len(),
        3,
        "the plot should keep its two marks and its interactor"
    );
}

/// **The reload-from-disk path takes the item out too**, and puts the spec in
/// the same state: an inline legend is not chrome the reload gate compares.
#[test]
fn the_reload_from_disk_path_takes_a_colour_legend_out_as_well() {
    let before = parse(LEGEND_BETWEEN);
    let mut fresh = before.clone();
    let mut reload = before.clone();

    apply_for_fresh_load(&mut fresh, &remove_legend("root")).expect("the plot is there");
    classify_edit(&reload, &remove_legend("root")).expect("the reload gate has no objection");
    apply(&mut reload, &remove_legend("root")).expect("the plot is there");

    assert_eq!(reload, fresh, "the two paths disagree on the removal");
    assert!(legends(&reload, "root").is_empty());
}

/// **A plot that holds no colour legend is left equal.** The hero map has
/// three items and none of them a legend, so an edit that took the last item
/// or any one regardless of what it is would change it.
#[test]
fn a_colour_legend_removal_on_a_plot_that_holds_none_changes_no_part_of_the_spec() {
    let before = hero();
    let mut spec = before.clone();

    apply_for_fresh_load(&mut spec, &remove_legend("root")).expect("the plot is there");

    assert_eq!(
        spec, before,
        "a removal with nothing to take out changed the spec"
    );
}

/// **It takes out what the append puts in, and no more.** A legend for another
/// channel among the plot's items stays, and so does a standalone colour legend
/// that names the plot with `for:`: it is not an item of the plot, and the
/// plot is still covered by it.
#[test]
fn a_colour_legend_removal_leaves_another_channels_legend_and_one_outside_the_plot() {
    let mut spec = parse(
        "\
data:
  t: SELECT 1 AS a, 2 AS b
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: b
      - legend: opacity
      - legend: color
    name: tile
  - legend: color
    for: tile
",
    );
    let plot = "root/vconcat[0]";
    assert_eq!(
        legends(&spec, plot),
        [LegendChannel::Opacity, LegendChannel::Color]
    );

    apply_for_fresh_load(&mut spec, &remove_legend(plot)).expect("the plot is there");

    assert_eq!(
        legends(&spec, plot),
        [LegendChannel::Opacity],
        "the removal took out a legend that is not the plot's colour legend"
    );
    assert!(
        colour_legend_covers(&spec, plot),
        "the standalone colour legend that names the plot went"
    );
}

/// **A plot's several colour legend items all go**, so the plot holds none
/// after the edit and the edit to put one back writes a single item.
#[test]
fn a_colour_legend_removal_takes_every_colour_legend_item_the_plot_holds() {
    let mut spec = parse(
        "data:\n  t: SELECT 1 AS a\nplot:\n  - legend: color\n  - mark: dot\n    data: { from: t }\n    x: a\n  - legend: color\n",
    );
    assert_eq!(
        legends(&spec, "root"),
        [LegendChannel::Color, LegendChannel::Color]
    );

    apply_for_fresh_load(&mut spec, &remove_legend("root")).expect("the plot is there");

    assert!(
        legends(&spec, "root").is_empty(),
        "a colour legend item stayed"
    );
    assert_eq!(
        plot_at_path(&spec, "root").expect("the plot").items.len(),
        1
    );
}

/// **Put back with the edit to right, the item is the plot's last**, after the
/// marks and the interactor, wherever it stood before it was taken out, and
/// nothing else about the spec changed from the plot without it.
#[test]
fn a_colour_legend_taken_out_and_put_back_is_the_plots_last_item() {
    let before = parse(LEGEND_BETWEEN);
    let mut spec = before.clone();

    apply_for_fresh_load(&mut spec, &remove_legend("root")).expect("the plot is there");
    let without = spec.clone();
    apply_for_fresh_load(&mut spec, &add_legend("root")).expect("the plot is there");

    let items = &plot_at_path(&spec, "root").expect("the root plot").items;
    assert!(
        matches!(items.last(), Some(Component::Legend(l)) if l.channel == LegendChannel::Color),
        "the last item is {:?}, not the colour legend",
        items.last()
    );
    let mut taken_off = spec.clone();
    brightfield_spec::edit::plot_at_path_mut(&mut taken_off, "root")
        .expect("the root plot")
        .items
        .pop();
    assert_eq!(
        taken_off, without,
        "putting the item back changed more than the item"
    );

    // On the plot the legend already closes, the two edits give the spec back.
    let mut closing = hero();
    apply_for_fresh_load(&mut closing, &add_legend("root")).expect("the plot is there");
    let with = closing.clone();
    apply_for_fresh_load(&mut closing, &remove_legend("root")).expect("the plot is there");
    assert_eq!(closing, hero(), "the removal did not undo the append");
    apply_for_fresh_load(&mut closing, &add_legend("root")).expect("the plot is there");
    assert_eq!(
        closing, with,
        "the append after a removal is not the first append"
    );
}

/// **A removal is a count-stable edit that names no mark**, and a path that
/// names no plot is refused with the spec equal, as for every other kind.
#[test]
fn a_colour_legend_removal_is_count_stable_names_no_mark_and_needs_a_plot() {
    let edit = remove_legend("root");
    assert!(!edit.is_count_changing(), "a legend is not a mark");
    assert_eq!(edit.kind_name(), "remove-colour-legend");
    assert_eq!(edit.summary(), "remove-colour-legend");
    assert_eq!(edit.plot_path(), "root");

    let mut spec = parse(LEGEND_BETWEEN);
    let before = spec.clone();
    assert_eq!(
        apply_for_fresh_load(&mut spec, &remove_legend("root/vconcat[3]")),
        Err(RefuseReason::PlotNotFound)
    );
    assert_eq!(spec, before, "the refused removal changed the spec");
}
