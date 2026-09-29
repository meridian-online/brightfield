//! A chart edit written into the chart file's text: `write_chart_edit`.
//!
//! Every expectation here is a whole file built by hand and compared byte for
//! byte, so a change anywhere outside the one line the edit names fails the
//! test, not only a change on that line.

use brightfield_protocol::{write_chart_edit, ChartTextRefusal};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::edit::{apply, ChartEdit, RefuseReason};
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

fn parse(text: &str) -> Spec {
    parse_spec(text, Format::Yaml).expect("parses").spec
}

/// The spec `apply` makes of `text`'s parse — what the written text has to
/// read back as.
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

/// Each of the four mark edits is refused with a reason that names its kind,
/// and no text comes back.
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
            ChartEdit::SetChannel {
                plot: plot(),
                mark_ordinal: 0,
                channel: "y".to_string(),
                column: "c".to_string(),
            },
            "set-channel",
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

/// An edit `apply` refuses is refused here for the same reason, and nothing is
/// written.
#[test]
fn an_edit_apply_refuses_is_refused_with_its_reason() {
    let edit = set("root/vconcat[0]", "xLabel", "income");
    assert_eq!(
        write_chart_edit(WITH_SCALE, &edit),
        Err(ChartTextRefusal::Refused {
            reason: RefuseReason::WouldChangeAxisTitle
        })
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
