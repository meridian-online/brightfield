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
use brightfield_spec::ast::{Component, ConcatNode, LegendNode, SpecValue, ValueOrParamRef};
use brightfield_spec::edit::{
    apply, apply_for_fresh_load, classify_edit, colour_legend_covers, plot_at_path,
    plot_path_after, ChartEdit, LegendPlacement, RefuseReason,
};
use brightfield_spec::layout::collect_legend_nodes;
use brightfield_spec::vocab::LegendChannel;
use brightfield_spec::{parse_spec, Format, Spec};
use indexmap::IndexMap;

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

fn place(plot: &str, at: LegendPlacement) -> ChartEdit {
    ChartEdit::PlaceColourLegend {
        plot: ComponentPath(plot.to_string()),
        at,
    }
}

/// `spec` with `edit` applied by the fresh-load reducer, which has to accept it.
fn placed(spec: &Spec, edit: &ChartEdit) -> Spec {
    let mut out = spec.clone();
    apply_for_fresh_load(&mut out, edit).unwrap_or_else(|e| panic!("{edit:?} applies: {e:?}"));
    out
}

/// A root plot whose legend is to its right: a colour legend item carrying an
/// option, after the plot's one mark.
const RIGHT: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
    fill: c
  - legend: color
    label: Age
    columns: 1
width: 300
";

fn colour_legend(options: IndexMap<String, ValueOrParamRef<SpecValue>>) -> Component {
    Component::Legend(LegendNode {
        channel: LegendChannel::Color,
        status: LegendChannel::Color.status(),
        options,
    })
}

fn text_value(s: &str) -> ValueOrParamRef<SpecValue> {
    ValueOrParamRef::Value(SpecValue::String(s.to_string()))
}

/// `spec`'s root plot with its colour legend items taken out and `name:` set,
/// in a `vconcat` with a colour legend `for:` that name carrying `options` —
/// the shape a move to below writes, built here from its parts.
fn wrapped(spec: &Spec, name: &str, options: IndexMap<String, ValueOrParamRef<SpecValue>>) -> Spec {
    let mut out = spec.clone();
    let Some(Component::Plot(mut plot)) = out.root.take() else {
        panic!("the fixture's root is a plot");
    };
    plot.items
        .retain(|c| !matches!(c, Component::Legend(l) if l.channel == LegendChannel::Color));
    plot.attributes
        .insert("name".to_string(), SpecValue::String(name.to_string()));
    let mut legend = IndexMap::new();
    legend.insert("for".to_string(), text_value(name));
    legend.extend(options);
    out.root = Some(Component::VConcat(ConcatNode {
        items: vec![Component::Plot(plot), colour_legend(legend)],
    }));
    out
}

/// The options of the first colour legend item of `spec`'s root plot.
fn item_options(spec: &Spec) -> IndexMap<String, ValueOrParamRef<SpecValue>> {
    plot_at_path(spec, "root")
        .expect("the root plot")
        .items
        .iter()
        .find_map(|c| match c {
            Component::Legend(l) if l.channel == LegendChannel::Color => Some(l.options.clone()),
            _ => None,
        })
        .expect("the fixture's plot holds a colour legend item")
}

/// **A move to below leaves, at the plot's place, a `vconcat` of the plot,
/// without its legend item and carrying a `name:`, and a `legend: color` whose
/// `for:` is that name, carrying each option the item carried.** The expected
/// spec is built from the parsed fixture's own plot and item, so an option
/// dropped, a `for:` missing, a legend item left in the plot or a plot moved
/// anywhere but the root's place is a difference.
#[test]
fn a_move_below_wraps_the_plot_in_a_vconcat_with_a_legend_for_it() {
    let spec = parse(RIGHT);
    let options = item_options(&spec);
    assert_eq!(options.len(), 2, "the fixture's item carries two options");

    let below = placed(&spec, &place("root", LegendPlacement::Below));

    assert_eq!(below, wrapped(&spec, "chart", options));
    assert_eq!(
        plot_path_after(&spec, &place("root", LegendPlacement::Below)),
        "root/vconcat[0]"
    );
}

/// **A plot that had a name keeps it, and the legend names it.** No second
/// name is written, and the standalone legend's `for:` is the name the plot
/// already held.
#[test]
fn a_move_below_keeps_the_name_a_plot_already_has() {
    let spec = parse(&RIGHT.replace("width: 300\n", "width: 300\nname: scatter\n"));
    let options = item_options(&spec);

    let below = placed(&spec, &place("root", LegendPlacement::Below));

    assert_eq!(below, wrapped(&spec, "scatter", options));
}

/// **A plot that had no name is given one no other plot of the file holds and
/// no legend's `for:` names.** `chart` is held by the other plot and `chart-2`
/// is named by a standalone legend whose plot is not in the file, which a plot
/// given that name would start to draw; `chart-3` is the first name free of
/// both.
#[test]
fn a_move_below_names_the_plot_with_a_name_no_plot_holds_and_no_legend_names() {
    let text = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
hconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: c
      - legend: color
  - plot:
      - mark: line
        data: { from: t }
        x: a
        y: b
    name: chart
  - legend: color
    for: chart-2
";
    let spec = parse(text);

    let below = placed(&spec, &place("root/hconcat[0]", LegendPlacement::Below));

    let plot = plot_at_path(&below, "root/hconcat[0]/vconcat[0]").expect("the plot, wrapped");
    assert_eq!(
        plot.attributes.get("name"),
        Some(&SpecValue::String("chart-3".to_string()))
    );
    let fors: Vec<_> = collect_legend_nodes(&below)
        .into_iter()
        .map(|(path, legend)| (path, legend.options.get("for").cloned()))
        .collect();
    assert_eq!(
        fors,
        vec![
            (
                "root/hconcat[0]/vconcat[1]".to_string(),
                Some(text_value("chart-3"))
            ),
            ("root/hconcat[2]".to_string(), Some(text_value("chart-2"))),
        ]
    );
}

/// **From below, the move to right puts the item back among the plot's items,
/// takes the standalone legend out, and leaves the plot at the place the
/// `vconcat` held: the spec it started from, but for the name.** The item
/// carries its options back and not the `for:`.
#[test]
fn a_move_right_from_below_is_the_spec_before_the_move_but_for_the_name() {
    let spec = parse(RIGHT);
    let below = placed(&spec, &place("root", LegendPlacement::Below));
    let to_right = place("root/vconcat[0]", LegendPlacement::Right);

    let right = placed(&below, &to_right);

    let mut named = spec.clone();
    brightfield_spec::edit::plot_at_path_mut(&mut named, "root")
        .expect("the root plot")
        .attributes
        .insert("name".to_string(), SpecValue::String("chart".to_string()));
    assert_eq!(right, named);
    assert_eq!(plot_path_after(&below, &to_right), "root");
}

/// A `vconcat` of a named plot, the colour legend drawn under it, and a second
/// plot under that: below, in a `vconcat` the move did not write.
const BELOW_WITH_NEIGHBOUR: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: c
    name: scatter
  - legend: color
    for: scatter
    label: Age
  - plot:
      - mark: line
        data: { from: t }
        x: a
        y: b
";

/// **From below on a `vconcat` that holds an entry besides the plot and its
/// legend, the move to right takes the standalone legend out, puts the item in
/// the plot, and leaves the other entry and the `vconcat` where they were.**
#[test]
fn a_move_right_from_below_beside_another_entry_leaves_the_vconcat() {
    let spec = parse(BELOW_WITH_NEIGHBOUR);
    let to_right = place("root/vconcat[0]", LegendPlacement::Right);

    let right = placed(&spec, &to_right);

    let Some(Component::VConcat(before)) = &spec.root else {
        panic!("the fixture's root is a vconcat");
    };
    let Component::Plot(mut plot) = before.items[0].clone() else {
        panic!("the vconcat's first entry is the plot");
    };
    let mut item = IndexMap::new();
    item.insert("label".to_string(), text_value("Age"));
    plot.items.push(colour_legend(item));
    let mut expected = spec.clone();
    expected.root = Some(Component::VConcat(ConcatNode {
        items: vec![Component::Plot(plot), before.items[2].clone()],
    }));
    assert_eq!(right, expected);
    assert_eq!(plot_path_after(&spec, &to_right), "root/vconcat[0]");
}

/// **From below, the move to none leaves the plot at the place the `vconcat`
/// held with no colour legend in the file; from none, the move to below writes
/// the `vconcat`.**
#[test]
fn a_move_to_none_from_below_unwraps_the_plot_and_a_move_below_from_none_wraps_it() {
    let spec = parse(RIGHT);
    let below = placed(&spec, &place("root", LegendPlacement::Below));
    let to_none = place("root/vconcat[0]", LegendPlacement::None);

    let none = placed(&below, &to_none);

    let Some(Component::Plot(plot)) = &none.root else {
        panic!("the plot is not at the root's place: {:?}", none.root);
    };
    assert!(
        !plot
            .items
            .iter()
            .any(|c| matches!(c, Component::Legend(l) if l.channel == LegendChannel::Color)),
        "the plot holds a colour legend item"
    );
    assert_eq!(
        collect_legend_nodes(&none),
        Vec::new(),
        "a standalone legend stayed"
    );
    assert_eq!(plot_path_after(&below, &to_none), "root");

    let again = placed(&none, &place("root", LegendPlacement::Below));

    assert_eq!(again, wrapped(&none, "chart", IndexMap::new()));
}

/// **A plot already at the placement asked for is left equal**, and a move
/// between right and none is the item edit's.
#[test]
fn a_move_to_where_the_legend_is_leaves_the_spec_equal() {
    let spec = parse(RIGHT);
    assert_eq!(placed(&spec, &place("root", LegendPlacement::Right)), spec);
    let below = placed(&spec, &place("root", LegendPlacement::Below));
    assert_eq!(
        placed(&below, &place("root/vconcat[0]", LegendPlacement::Below)),
        below
    );

    let none = placed(&spec, &place("root", LegendPlacement::None));
    let mut removed = spec.clone();
    apply_for_fresh_load(&mut removed, &remove_legend("root")).expect("the plot is there");
    assert_eq!(none, removed);
    assert_eq!(placed(&none, &place("root", LegendPlacement::Right)), {
        let mut added = none.clone();
        apply_for_fresh_load(&mut added, &add_legend("root")).expect("the plot is there");
        added
    });
}

/// **The reload gate refuses a move into or out of the band under the plot**,
/// with the spec equal, because the plot's rect changes; a move between right
/// and none passes it as the item edits do.
#[test]
fn the_reload_gate_refuses_a_move_into_or_out_of_the_band() {
    let spec = parse(RIGHT);
    let below = placed(&spec, &place("root", LegendPlacement::Below));
    for (from, edit) in [
        (&spec, place("root", LegendPlacement::Below)),
        (&below, place("root/vconcat[0]", LegendPlacement::Right)),
        (&below, place("root/vconcat[0]", LegendPlacement::None)),
    ] {
        let mut gated = from.clone();
        assert_eq!(
            apply(&mut gated, &edit),
            Err(RefuseReason::WouldChangeLayout),
            "{edit:?}"
        );
        assert_eq!(&gated, from, "the refused move changed the spec");
    }
    assert_eq!(
        classify_edit(&spec, &place("root", LegendPlacement::None)),
        Ok(())
    );

    let edit = place("root", LegendPlacement::Below);
    assert!(!edit.is_count_changing(), "a legend is not a mark");
    assert_eq!(edit.kind_name(), "place-colour-legend");
    assert_eq!(edit.summary(), "place-colour-legend: below");
}

/// Two plots in one `vconcat`, each with its own colour legend drawn under it:
/// plot, legend, plot, legend. The legends carry different labels, so a legend
/// read as the other plot's shows in the options a move carries.
const TWO_BELOW: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: c
    name: first
  - legend: color
    for: first
    label: First
  - plot:
      - mark: dot
        data: { from: t }
        x: b
        fill: c
    name: second
  - legend: color
    for: second
    label: Second
";

/// [`TWO_BELOW`] with the second plot's legend to its right: the first plot's
/// legend stays under the first plot, and the item carries the second legend's
/// own label.
const TWO_BELOW_SECOND_RIGHT: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: c
    name: first
  - legend: color
    for: first
    label: First
  - plot:
      - mark: dot
        data: { from: t }
        x: b
        fill: c
      - legend: color
        label: Second
    name: second
";

/// [`TWO_BELOW`] with the second plot's legend taken out.
const TWO_BELOW_SECOND_NONE: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 3 AS c
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        fill: c
    name: first
  - legend: color
    for: first
    label: First
  - plot:
      - mark: dot
        data: { from: t }
        x: b
        fill: c
    name: second
";

/// **A move of one plot's legend out from below takes that plot's legend and
/// no other.** With two plots each carrying a legend below, moving the second
/// plot's legend to right or to none leaves the first plot's legend under the
/// first plot, and the second plot gets the options of its own legend and not
/// the first's.
#[test]
fn a_legend_moved_out_from_below_leaves_the_other_plots_legend_where_it_is() {
    let spec = parse(TWO_BELOW);

    assert_eq!(
        placed(&spec, &place("root/vconcat[2]", LegendPlacement::Right)),
        parse(TWO_BELOW_SECOND_RIGHT)
    );
    assert_eq!(
        placed(&spec, &place("root/vconcat[2]", LegendPlacement::None)),
        parse(TWO_BELOW_SECOND_NONE)
    );
}

/// **Each placement is spelled by `wire_name`, and the command log prints that
/// word.** A `Below` summary was asserted alone, so a changed word for `Right`
/// or `None` left every test green. `wire_name` is also the word the legend
/// row's three values are named by (its own doc), so the word is pinned
/// itself and not only through the log line.
#[test]
fn each_placement_is_spelled_by_wire_name_and_the_command_log_prints_it() {
    for (at, word) in [
        (LegendPlacement::Right, "right"),
        (LegendPlacement::Below, "below"),
        (LegendPlacement::None, "none"),
    ] {
        assert_eq!(at.wire_name(), word);
        assert_eq!(
            place("root", at).summary(),
            format!("place-colour-legend: {word}")
        );
    }
}
