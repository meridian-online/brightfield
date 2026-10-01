//! A chart edit written into the chart file's text: `write_chart_edit`.
//!
//! Every expectation here is a whole file built by hand and compared byte for
//! byte, so a change anywhere outside the one line the edit names fails the
//! test, not only a change on that line.

use brightfield_protocol::{write_chart_edit, ChartTextRefusal};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::edit::{self, apply, ChartEdit, RefuseReason};
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, Format, Spec, SpecValue};

/// What the dashboard generator wrote for
/// `crates/brightfield-shell/tests/data/california_housing_sample.csv`, through
/// `data_file::open`, captured byte for byte: the header comment, the comment
/// the generator writes above each tile, and the tiles as the hero beside a
/// column of the rest.
/// The one byte changed from the capture is the `data:` block's file path,
/// which named the directory the capture ran in and now reads
/// `/data/california_housing_sample.csv`.
const GENERATED: &str = include_str!("chart_text/generated_california_housing.yaml");

/// The generated `median_income` tile: the first in the column beside the hero.
const TILE: &str = "root/hconcat[2]/vconcat[0]";
/// The comment the generator writes above the tile after [`TILE`].
const NEXT_TILE_COMMENT: &str =
    "    # house_age: no trusted label, and DuckDB stored it as BIGINT → binned-histogram";
/// The indent of a generated tile's plot attributes (`width:`, `height:`).
const TILE_ATTR_PAD: &str = "      ";
/// The generated hero, the point map.
const HERO: &str = "root/hconcat[0]/vconcat[0]";

fn set(plot: &str, key: &str, value: &str) -> ChartEdit {
    ChartEdit::SetPlotAttribute {
        plot: ComponentPath(plot.to_string()),
        key: key.to_string(),
        value: SpecValue::String(value.to_string()),
    }
}

fn channel(plot: &str, mark_ordinal: usize, channel: &str, column: &str) -> ChartEdit {
    ChartEdit::SetChannel {
        plot: ComponentPath(plot.to_string()),
        mark_ordinal,
        channel: channel.to_string(),
        column: column.to_string(),
    }
}

fn legend(plot: &str) -> ChartEdit {
    ChartEdit::AddColourLegend {
        plot: ComponentPath(plot.to_string()),
    }
}

fn unlegend(plot: &str) -> ChartEdit {
    ChartEdit::RemoveColourLegend {
        plot: ComponentPath(plot.to_string()),
    }
}

fn remove(plot: &str, key: &str) -> ChartEdit {
    ChartEdit::RemovePlotAttribute {
        plot: ComponentPath(plot.to_string()),
        key: key.to_string(),
    }
}

/// `text` with each of `edits` written into the text the one before it left,
/// the way Save places the edits since the last Save.
fn write_all(text: &str, edits: &[ChartEdit]) -> String {
    edits.iter().fold(text.to_string(), |text, edit| {
        write_chart_edit(&text, edit).unwrap_or_else(|e| panic!("{edit:?} is written: {e}"))
    })
}

/// The lines of `before` and `after` that differ, as `(-line, +line)` pairs
/// read top to bottom, with `None` on the side a line was taken out of or
/// added to. A line that stays is not listed, so an edit that changes one line
/// gives one pair.
fn changed_lines(before: &str, after: &str) -> Vec<(Option<String>, Option<String>)> {
    let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    // Longest common subsequence, so a line added or taken out does not read
    // as every line below it changing.
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push((Some(a[i].to_string()), None));
            i += 1;
        } else {
            // A line taken out and the line added in its place are one change.
            match out.last_mut() {
                Some((Some(_), added @ None)) => *added = Some(b[j].to_string()),
                _ => out.push((None, Some(b[j].to_string()))),
            }
            j += 1;
        }
    }
    out
}

fn parse(text: &str) -> Spec {
    parse_spec(text, Format::Yaml).expect("parses").spec
}

/// The spec `apply` makes of `text`'s parse — what the written text has to
/// read back as.
/// The spec `apply_for_fresh_load` makes of `text`'s parse — the reducer the
/// writer checks against.
fn applied_fresh(text: &str, edit: &ChartEdit) -> Spec {
    let mut spec = parse(text);
    edit::apply_for_fresh_load(&mut spec, edit).expect("the reducer accepts the edit");
    spec
}

fn applied(text: &str, edit: &ChartEdit) -> Spec {
    let mut spec = parse(text);
    apply(&mut spec, edit).expect("apply accepts the edit");
    spec
}

/// `text` with `line` inserted before the first line that equals `before`.
fn insert_before(text: &str, before: &str, line: &str) -> String {
    let at = text
        .find(&format!("\n{before}\n"))
        .unwrap_or_else(|| panic!("no line {before:?} in:\n{text}"))
        + 1;
    format!("{}{line}\n{}", &text[..at], &text[at..])
}

// ------------------------------------------------------------------ AC1

/// The y scale switch on one generated tile adds one line inside that tile's
/// plot, and the rest of the file — the header comment and the comment on
/// every tile — is byte-identical.
#[test]
fn a_scale_switch_on_a_generated_tile_adds_one_line_inside_its_plot() {
    let edit = set(TILE, "yScale", "log");
    let written = write_chart_edit(GENERATED, &edit).expect("the edit is written");

    let expected = insert_before(
        GENERATED,
        NEXT_TILE_COMMENT,
        &format!("{TILE_ATTR_PAD}yScale: log"),
    );
    assert_eq!(written, expected);
    assert_eq!(written.lines().count(), GENERATED.lines().count() + 1);
    assert!(written.starts_with("# Brightfield wrote this dashboard"));
}

// ------------------------------------------------------------------ AC2

const WITH_SCALE: &str = "\
# A hand-kept chart.
meta:
  title: readings
vconcat:
  # the first tile
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    yScale: linear   # linear until the outliers are gone
    width: 300
  # the second tile
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: c
    width: 300
";

/// A plot that already carries `yScale: linear` with a comment after it gets
/// its value changed, and the comment stays on the line.
#[test]
fn a_value_already_on_the_plot_is_replaced_and_its_comment_stays() {
    let edit = set("root/vconcat[0]", "yScale", "log");
    let written = write_chart_edit(WITH_SCALE, &edit).expect("the edit is written");

    let expected = WITH_SCALE.replace(
        "    yScale: linear   # linear until the outliers are gone\n",
        "    yScale: log   # linear until the outliers are gone\n",
    );
    assert_ne!(expected, WITH_SCALE);
    assert_eq!(written, expected);
}

// ------------------------------------------------------------------ AC3

/// Parsing the written text gives the spec `apply` gives, for each key the
/// switches write, on a plot that lacks the key and on one that carries it.
#[test]
fn the_written_text_parses_to_the_spec_apply_makes() {
    let cases = [
        (GENERATED, TILE, "xScale", "log"),
        (GENERATED, TILE, "yScale", "log"),
        (GENERATED, TILE, "stackOffset", "normalize"),
        (GENERATED, HERO, "yScale", "symlog"),
        (WITH_SCALE, "root/vconcat[0]", "xScale", "log"),
        (WITH_SCALE, "root/vconcat[0]", "yScale", "log"),
        (WITH_SCALE, "root/vconcat[1]", "stackOffset", "normalize"),
    ];
    for (text, plot, key, value) in cases {
        let edit = set(plot, key, value);
        let written = write_chart_edit(text, &edit)
            .unwrap_or_else(|e| panic!("{plot} {key}: {value} is written: {e}"));
        assert_ne!(written, text, "{plot} {key}: {value} changes the text");
        assert_eq!(
            parse(&written),
            applied(text, &edit),
            "{plot} {key}: {value} reads back as apply's spec"
        );
    }
}

// ------------------------------------------------------------------ AC4

const NESTED: &str = "\
vconcat:
  - hconcat:
      # left
      - plot:
          - mark: barY
            data: { from: t }
            x: a
            y: b
        width: 200
      # middle, the one edited
      - plot:
          - mark: barY
            data: { from: t }
            x: a
            y: c
        width: 200
        # a note under the last attribute
      # right
      - hspace: 8
  - hconcat:
      - plot:
          - mark: dot
            data: { from: t }
            x: a
            y: d
        width: 400
";

/// A plot inside an `hconcat` inside a `vconcat` is changed in place, and the
/// bytes of its siblings on either side are identical. The line lands below a
/// comment written at the plot's own attribute indent, which arcform's splice
/// reads as the end of the plot's mapping; the comment above the next sibling,
/// one indent out, stays with that sibling.
#[test]
fn a_plot_two_levels_down_is_changed_in_place() {
    let edit = set("root/vconcat[0]/hconcat[1]", "yScale", "log");
    let written = write_chart_edit(NESTED, &edit).expect("the edit is written");

    let expected = NESTED.replace(
        "        # a note under the last attribute\n",
        "        # a note under the last attribute\n        yScale: log\n",
    );
    assert_ne!(expected, NESTED);
    assert_eq!(written, expected);
}

// ------------------------------------------------------------------ AC5

/// Change mark type, add mark and remove mark are each refused with a reason
/// that names the kind, and no text comes back. A set channel is not among
/// them: `a_set_channel_is_not_refused_by_kind` holds that.
#[test]
fn a_mark_edit_is_refused_by_kind() {
    let plot = || ComponentPath("root/vconcat[0]".to_string());
    let edits = [
        (
            ChartEdit::ChangeMarkType {
                plot: plot(),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
            "change-mark-type",
        ),
        (
            ChartEdit::AddMark {
                plot: plot(),
                kind: MarkKind::Line,
            },
            "add-mark",
        ),
        (
            ChartEdit::RemoveMark {
                plot: plot(),
                mark_ordinal: 0,
            },
            "remove-mark",
        ),
    ];
    for (edit, kind) in edits {
        let refusal = write_chart_edit(WITH_SCALE, &edit).expect_err("a mark edit is refused");
        assert_eq!(refusal, ChartTextRefusal::UnwrittenKind { kind });
        assert!(
            refusal.to_string().contains(kind),
            "the reason names {kind}: {refusal}"
        );
    }
}

// ------------------------------------------------------------------ AC6

/// An edit naming a plot the text does not hold is refused with a reason —
/// a path past the end of a list, and a path to an item that is not a plot.
#[test]
fn an_edit_naming_a_plot_the_text_does_not_hold_is_refused() {
    for plot in [
        "root/vconcat[0]/hconcat[7]",
        "root/vconcat[0]/hconcat[2]",
        "root",
    ] {
        let refusal = write_chart_edit(NESTED, &set(plot, "yScale", "log"))
            .expect_err("a missing plot is refused");
        assert_eq!(
            refusal,
            ChartTextRefusal::NoSuchPlot {
                plot: plot.to_string()
            }
        );
        assert!(
            refusal.to_string().contains(plot),
            "the reason names {plot}: {refusal}"
        );
    }
}

// --------------------------------------------------------- the edges of it

/// The root plot's attributes sit at the document root beside `meta:`, and a
/// key it lacks is added there.
#[test]
fn a_root_plot_gains_its_key_at_the_document_root() {
    let text = "\
# one plot
meta:
  title: one
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
width: 300
";
    let edit = set("root", "xScale", "log");
    let written = write_chart_edit(text, &edit).expect("the edit is written");
    assert_eq!(written, format!("{text}xScale: log\n"));
    assert_eq!(parse(&written), applied(text, &edit));
}

/// A bare `yScale:` has no value bytes to replace, and the written line still
/// reads `yScale: log`.
#[test]
fn a_bare_key_gains_its_value_after_one_space() {
    let text = WITH_SCALE.replace(
        "    yScale: linear   # linear until the outliers are gone\n",
        "    yScale:\n",
    );
    let edit = set("root/vconcat[0]", "yScale", "log");
    let written = write_chart_edit(&text, &edit).expect("the edit is written");
    assert_eq!(written, text.replace("    yScale:\n", "    yScale: log\n"));
}

/// Picking the value the plot already has is not an edit, and the text comes
/// back as it was — including when the value came from `plotDefaults:` and the
/// plot's own mapping has no line for it.
#[test]
fn an_edit_that_changes_nothing_writes_nothing() {
    let same = set("root/vconcat[0]", "yScale", "linear");
    assert_eq!(
        write_chart_edit(WITH_SCALE, &same).as_deref(),
        Ok(WITH_SCALE)
    );

    let defaulted = format!("plotDefaults:\n  yScale: log\n{WITH_SCALE}");
    let from_default = set("root/vconcat[1]", "yScale", "log");
    assert_eq!(
        write_chart_edit(&defaulted, &from_default).as_deref(),
        Ok(defaulted.as_str())
    );
}

/// An edit the reducer finds no target for is refused for the reducer's
/// reason, and nothing is written.
#[test]
fn an_edit_with_no_target_is_refused_with_the_reducers_reason() {
    let edit = channel("root/vconcat[0]", 3, "x", "c");
    assert_eq!(
        write_chart_edit(WITH_SCALE, &edit),
        Err(ChartTextRefusal::Refused {
            reason: RefuseReason::NoSuchMark
        })
    );
}

/// An edit that changes an axis title is written: Save writes the edits the
/// page was drawn with, and the page is loaded afresh after each, so the title
/// refusal `apply` makes for a reload from disk is not the text's.
#[test]
fn an_edit_that_changes_an_axis_title_is_written() {
    let edit = set("root/vconcat[0]", "xLabel", "income");
    assert_eq!(
        apply(&mut parse(WITH_SCALE), &edit),
        Err(RefuseReason::WouldChangeAxisTitle),
        "the reload-from-disk gate still refuses it"
    );
    let written = write_chart_edit(WITH_SCALE, &edit).expect("the edit is written");
    assert_eq!(
        written,
        WITH_SCALE.replace(
            "    width: 300\n  # the second tile\n",
            "    width: 300\n    xLabel: income\n  # the second tile\n",
        )
    );
}

/// A value that YAML spells over several lines is not placed as one line.
#[test]
fn a_value_that_spans_lines_is_refused() {
    let edit = ChartEdit::SetPlotAttribute {
        plot: ComponentPath("root/vconcat[1]".to_string()),
        key: "xDomain".to_string(),
        value: SpecValue::Array(vec![SpecValue::Integer(0), SpecValue::Integer(10)]),
    };
    assert_eq!(
        write_chart_edit(WITH_SCALE, &edit),
        Err(ChartTextRefusal::ValueNotOneLine {
            key: "xDomain".to_string()
        })
    );
}

/// Text the parser would read back as some other chart is refused rather than
/// returned: `plot` is the key that makes a mapping a plot, and `apply` treats
/// it as one more attribute.
#[test]
fn text_that_reads_back_as_another_chart_is_refused() {
    let edit = set("root/vconcat[1]", "plot", "dot");
    assert_eq!(
        write_chart_edit(WITH_SCALE, &edit),
        Err(ChartTextRefusal::ReadsBackDifferently {
            key: "plot".to_string()
        })
    );
}

// ------------------------------------------------ the shelf's edits, written

/// What the shelf's edit returns for `median_income` put on the generated
/// map's x: a set channel on each of its two layers, then the projection taken
/// out, in that order.
fn income_on_x() -> Vec<ChartEdit> {
    vec![
        channel(HERO, 0, "x", "median_income"),
        channel(HERO, 1, "x", "median_income"),
        remove(HERO, "projectionType"),
    ]
}

/// What the shelf's edit returns for `longitude` put back on the map's x: a
/// set channel on each layer, then the projection set, since the plot holds
/// the coordinate pair again.
fn longitude_on_x() -> Vec<ChartEdit> {
    vec![
        channel(HERO, 0, "x", "longitude"),
        channel(HERO, 1, "x", "longitude"),
        set(HERO, "projectionType", "equirectangular"),
    ]
}

/// **`median_income` on the generated map's x** changes both layers' `x:`
/// lines, keeping the quotes the generator wrote them in, and takes the
/// `projectionType:` line out. Every other line, the header comment and the
/// comment on each tile included, is byte for byte.
#[test]
fn putting_median_income_on_x_changes_both_layers_and_takes_the_projection_out() {
    let written = write_all(GENERATED, &income_on_x());

    let x = |column: &str| format!("        x: '{column}'");
    assert_eq!(
        changed_lines(GENERATED, &written),
        vec![
            (Some(x("longitude")), Some(x("median_income"))),
            (Some(x("longitude")), Some(x("median_income"))),
            (
                Some("      projectionType: equirectangular".to_string()),
                None
            ),
        ]
    );
    let expected = GENERATED
        .replace("        x: 'longitude'\n", "        x: 'median_income'\n")
        .replace("      projectionType: equirectangular\n", "");
    assert_eq!(written, expected);
    assert!(written.starts_with("# Brightfield wrote this dashboard"));

    let mut spec = parse(GENERATED);
    for edit in income_on_x() {
        edit::apply_for_fresh_load(&mut spec, &edit).expect("the shelf's edits apply");
    }
    assert_eq!(
        parse(&written),
        spec,
        "the text reads back as the edited chart"
    );
}

/// **`longitude` put back on x after that** gives the text the generator
/// wrote: the layers' `x:` lines return to their quotes and the projection to
/// the line it was taken from.
#[test]
fn putting_longitude_back_on_x_gives_the_text_the_generator_wrote() {
    let rebound = write_all(GENERATED, &income_on_x());
    assert_ne!(rebound, GENERATED);
    assert_eq!(write_all(&rebound, &longitude_on_x()), GENERATED);
}

/// **`median_house_value` on the map's colour** adds one
/// `fill: median_house_value` line to the highlighted layer, the one that reads
/// through the selection, and changes no other line.
#[test]
fn a_column_on_the_maps_colour_adds_one_fill_line_to_the_highlighted_layer() {
    let edit = channel(HERO, 1, "fill", "median_house_value");
    let written = write_chart_edit(GENERATED, &edit).expect("the edit is written");

    assert_eq!(
        changed_lines(GENERATED, &written),
        vec![(None, Some("        fill: median_house_value".to_string()))]
    );
    // The line lands inside the second layer, after its `y:`, and not in the
    // ghost layer, which keeps its one `fill:`.
    let highlighted = "      - mark: dot\n        data: { from: opened, filterBy: $sel }\n        x: 'longitude'\n        y: 'latitude'\n";
    assert_eq!(
        written,
        GENERATED.replace(
            highlighted,
            &format!("{highlighted}        fill: median_house_value\n")
        )
    );
    assert_eq!(parse(&written), applied_fresh(GENERATED, &edit));
}

/// A set channel is written, not refused by kind, on a mark that carries the
/// channel and on one that does not.
#[test]
fn a_set_channel_is_not_refused_by_kind() {
    for edit in [
        channel("root/vconcat[0]", 0, "y", "c"),
        channel("root/vconcat[0]", 0, "stroke", "c"),
    ] {
        let written = write_chart_edit(WITH_SCALE, &edit)
            .unwrap_or_else(|e| panic!("{edit:?} is written: {e}"));
        assert_ne!(written, WITH_SCALE);
        assert_eq!(parse(&written), applied_fresh(WITH_SCALE, &edit));
    }
}

/// A column whose name the value's quotes cannot hold is written in the
/// serialiser's spelling instead, and reads back as the column.
#[test]
fn a_column_the_quotes_cannot_hold_is_written_in_the_serialisers_spelling() {
    let text = WITH_SCALE.replace("        y: b\n", "        y: 'b'\n");
    let edit = channel("root/vconcat[0]", 0, "y", "it's");
    let written = write_chart_edit(&text, &edit).expect("the edit is written");
    assert_eq!(
        written,
        text.replace("        y: 'b'\n", "        y: it's\n")
    );
    assert_eq!(parse(&written), applied_fresh(&text, &edit));
}

/// Taking out an attribute takes out its line and, by arcform's rule of
/// comment ownership, a comment flush above it; the comment after the plot's
/// last attribute, the comment on a kept line and the comment on each tile
/// stay.
#[test]
fn a_comment_flush_above_a_removed_line_goes_with_it_and_the_rest_stay() {
    let text = "\
vconcat:
  # the tile
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    # log until the outliers are gone
    yScale: log
    width: 300   # the column's width
    # a note under the last attribute
  # the next tile
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: c
";
    let edit = remove("root/vconcat[0]", "yScale");
    let written = write_chart_edit(text, &edit).expect("the edit is written");
    assert_eq!(
        written,
        text.replace(
            "    # log until the outliers are gone\n    yScale: log\n",
            ""
        )
    );
    assert_eq!(parse(&written), applied_fresh(text, &edit));
}

/// An attribute the plot takes from `plotDefaults:` has no line in the plot to
/// take out, and the removal is refused rather than written as nothing.
#[test]
fn a_removal_of_an_attribute_the_plot_inherits_is_refused() {
    let text = format!("plotDefaults:\n  yScale: log\n{WITH_SCALE}");
    let edit = remove("root/vconcat[1]", "yScale");
    let refusal = write_chart_edit(&text, &edit).expect_err("the removal is refused");
    assert_eq!(
        refusal,
        ChartTextRefusal::Inherited {
            key: "yScale".to_string()
        }
    );
    assert!(refusal.to_string().contains("plotDefaults"), "{refusal}");
}

/// A mark ordinal counts marks, not the `plot:` list's items: with an
/// interactor listed first, the first mark is the list's second item, and the
/// channel lands on it.
#[test]
fn a_channel_on_a_mark_listed_after_an_interactor_lands_on_that_mark() {
    let text = "\
plot:
  - select: intervalX
    as: $s
  - mark: dot
    data: { from: t }
    x: a
    y: b
width: 300
";
    let edit = channel("root", 0, "y", "c");
    let written = write_chart_edit(text, &edit).expect("the edit is written");
    assert_eq!(written, text.replace("    y: b\n", "    y: c\n"));
    assert_eq!(parse(&written), applied_fresh(text, &edit));
}

// ------------------------------------------------------- the colour legend

/// On the generated map, a colour legend is one `- legend: color` line after
/// the last item of the hero's list, in the list's own indent, and the rest of
/// the file — the header comment and the comment above each tile — is
/// byte-identical. The text parses with no warning to the spec the reducer
/// makes, whose last item on the hero is the legend.
#[test]
fn a_colour_legend_on_the_generated_map_adds_one_line_after_the_last_item() {
    let edit = legend(HERO);
    let written = write_chart_edit(GENERATED, &edit).expect("the edit is written");

    // The hero's plot attributes follow its list, so the line goes before them.
    let expected = insert_before(GENERATED, "      width: 620", "      - legend: color");
    assert_eq!(written, expected);
    assert_eq!(
        changed_lines(GENERATED, &written),
        [(None, Some("      - legend: color".to_string()))],
        "the edit changed a line other than the one it added"
    );
    assert!(written.starts_with("# Brightfield wrote this dashboard"));
    assert!(written.contains(NEXT_TILE_COMMENT), "a tile's comment went");

    let parsed = parse_spec(&written, Format::Yaml).expect("the written text parses");
    assert!(
        parsed.warnings.is_empty(),
        "the file holding the legend item parses with warnings: {:?}",
        parsed.warnings
    );
    let spec = parsed.spec;
    assert_eq!(spec, applied_fresh(GENERATED, &edit));
    let hero = edit::plot_at_path(&spec, HERO).expect("the hero");
    assert!(
        matches!(
            hero.items.last(),
            Some(brightfield_spec::ast::Component::Legend(l))
                if l.channel == brightfield_spec::vocab::LegendChannel::Color
        ),
        "the last item of the hero is {:?}, not the colour legend",
        hero.items.last()
    );
}

/// A hand-kept chart indents its lists another way than the generator does, and
/// keeps a comment under the last item. The line takes the indent the list's
/// items already have, which is not the generated map's, goes above the comment
/// that closes the list, and leaves every other byte alone.
#[test]
fn a_colour_legend_takes_the_indent_of_the_list_it_joins_and_leaves_the_comments() {
    let text = "\
# A hand-kept chart.
vconcat:
  # the first tile
  - plot:
        - mark: dot
          data: { from: t }   # the readings
          x: a
          fill: b
        # the marks end here
    width: 300
  # the second tile
  - plot:
        - mark: dot
          data: { from: t }
          x: a
    width: 300
";
    let edit = legend("root/vconcat[0]");
    let written = write_chart_edit(text, &edit).expect("the edit is written");

    assert_eq!(
        written,
        text.replace(
            "          fill: b\n",
            "          fill: b\n        - legend: color\n"
        )
    );
    assert_eq!(parse(&written), applied_fresh(text, &edit));
}

/// A plot that already holds the item is the spec as it was, so the text comes
/// back byte for byte, and a second edit on the text the first wrote changes
/// nothing.
#[test]
fn a_colour_legend_on_a_plot_that_holds_one_returns_the_text_unchanged() {
    let edit = legend(HERO);
    let once = write_chart_edit(GENERATED, &edit).expect("the edit is written");

    let twice = write_chart_edit(&once, &edit).expect("the edit is written again");

    assert_eq!(twice, once, "a second legend was written");
    assert_eq!(once.matches("legend: color").count(), 1);
}

/// A plot whose items are written as a flow list has no line to put the legend
/// on, and the edit is refused rather than written into the wrong place.
#[test]
fn a_colour_legend_on_a_flow_list_is_refused() {
    let text = "plot: [ { mark: dot, data: { from: t }, x: a } ]\nwidth: 300\n";

    let refusal = write_chart_edit(text, &legend("root")).expect_err("a flow list is refused");

    assert!(
        matches!(&refusal, ChartTextRefusal::Splice { detail } if detail.contains("block list")),
        "the refusal is {refusal:?}, not the writer's own for a list it cannot indent"
    );
}

// ----------------------------------------------- the colour legend, taken out

/// On the generated map, taking the colour legend out of a file that holds it
/// is the one `- legend: color` line gone, and the rest of the file — the header
/// comment, the comment above each tile — is byte-identical. The text parses
/// to the spec the reducer makes, and it is the text the generator wrote:
/// writing the legend in and taking it out leaves nothing behind.
#[test]
fn taking_the_colour_legend_off_the_generated_map_takes_out_one_line() {
    let with = write_chart_edit(GENERATED, &legend(HERO)).expect("the legend is written in");
    let edit = unlegend(HERO);

    let written = write_chart_edit(&with, &edit).expect("the edit is written");

    assert_eq!(
        changed_lines(&with, &written),
        [(Some("      - legend: color".to_string()), None)],
        "the edit changed a line other than the one it took out"
    );
    assert_eq!(written, GENERATED);
    assert!(written.starts_with("# Brightfield wrote this dashboard"));
    assert!(written.contains(NEXT_TILE_COMMENT), "a tile's comment went");
    assert_eq!(written.matches("legend:").count(), 0);
    assert_eq!(parse(&written), applied_fresh(&with, &edit));
    let hero = edit::plot_at_path(&parse(&written), HERO)
        .expect("the hero")
        .clone();
    assert!(
        hero.items
            .iter()
            .all(|c| !matches!(c, brightfield_spec::ast::Component::Legend(_))),
        "the hero still holds a legend: {:?}",
        hero.items
    );
}

/// A hand-kept chart keeps its legend between two marks and a blank line after
/// it. The removal takes out the legend's line and that blank line, so the
/// marks do not stand a line further apart, and not another line of the plot:
/// the comment at the head of the file, the comment on the mark's line, and the
/// comment above the second mark are in the text still. Put back with the edit
/// to right, the line comes back after the list's last item, above the comment
/// that closes the list.
#[test]
fn taking_a_colour_legend_off_a_hand_kept_chart_takes_its_line_and_the_blank_one_after() {
    let text = "\
# A hand-kept chart.
vconcat:
  # the first tile
  - plot:
        - mark: dot
          data: { from: t }   # the readings
          x: a
          fill: b
        - legend: color

        # the line over it
        - mark: line
          data: { from: t }
          x: a
          y: b
        # the marks end here
    width: 300
  # the second tile
  - plot:
        - mark: dot
          data: { from: t }
          x: a
    width: 300
";
    let edit = unlegend("root/vconcat[0]");

    let written = write_chart_edit(text, &edit).expect("the edit is written");

    assert_eq!(written, text.replace("        - legend: color\n\n", ""));
    assert_eq!(
        changed_lines(text, &written),
        [
            (Some("        - legend: color".to_string()), None),
            (Some(String::new()), None),
        ],
        "the edit changed a line other than the legend's and the blank one after it"
    );
    for kept in [
        "# A hand-kept chart.",
        "# the first tile",
        "# the readings",
        "# the line over it",
        "# the marks end here",
        "# the second tile",
    ] {
        assert!(written.contains(kept), "the comment {kept:?} went");
    }
    assert_eq!(parse(&written), applied_fresh(text, &edit));

    let put_back = write_chart_edit(&written, &legend("root/vconcat[0]")).expect("it is put back");
    assert_eq!(
        put_back,
        written.replace(
            "          y: b\n",
            "          y: b\n        - legend: color\n"
        ),
        "the item should return at the end of the plot's list, above the comment closing it"
    );
}

/// A plot that holds no colour legend is the spec as it was, so the text comes
/// back byte for byte.
#[test]
fn taking_a_colour_legend_off_a_plot_that_holds_none_returns_the_text_unchanged() {
    let written = write_chart_edit(GENERATED, &unlegend(HERO)).expect("the edit is written");

    assert_eq!(written, GENERATED);
}

/// A plot that lists two colour legends has both taken out, and the legend for
/// another channel between them stays. The second is taken out of the text the
/// first left, so the batch goes last item first.
#[test]
fn taking_the_colour_legend_off_a_plot_that_lists_it_twice_takes_both_lines() {
    let text = "\
plot:
  - legend: color
  - mark: dot
    data: { from: t }
    x: a
  - legend: opacity
  - legend: color
width: 300
";
    let edit = unlegend("root");

    let written = write_chart_edit(text, &edit).expect("the edit is written");

    assert_eq!(
        written,
        "plot:\n  - mark: dot\n    data: { from: t }\n    x: a\n  - legend: opacity\nwidth: 300\n"
    );
    assert_eq!(parse(&written), applied_fresh(text, &edit));
}

/// A flow list has lines for neither the item nor its neighbours, and arcform
/// refuses to take an element out of one, so the edit is refused and not
/// written into the wrong place.
#[test]
fn taking_a_colour_legend_off_a_flow_list_is_refused() {
    let text = "plot: [ { mark: dot, data: { from: t }, x: a }, { legend: color } ]\nwidth: 300\n";

    let refusal = write_chart_edit(text, &unlegend("root")).expect_err("a flow list is refused");

    assert!(
        matches!(&refusal, ChartTextRefusal::Splice { .. }),
        "the refusal is {refusal:?}, not arcform's own for a flow collection"
    );
}
