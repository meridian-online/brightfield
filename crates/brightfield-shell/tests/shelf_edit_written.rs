//! **The shelf's edits on the generated map, written into the chart file's
//! text.**
//!
//! The tests here open `tests/data/california_housing_sample.csv` the way a
//! data file is opened, take the text the generator wrote for it, edit the hero
//! map through `shelf_edit::put_column` and `shelf_edit::put_colour` with the
//! table's own profile, and write each edit the shelf returns into that text
//! through `brightfield_protocol::write_chart_edit`, one at a time, the way
//! Save does. So the text is the live generator's, the edits are the live
//! shelf's, and the writer is the one Save calls.
//!
//! The writer's own tests in `brightfield-protocol` run over a capture of this
//! text; `the_protocol_writers_capture_is_the_text_the_generator_writes` holds
//! the capture to the generator, so a change to either is seen by both.

use std::path::PathBuf;

use brightfield_engine::{ColumnProfile, ProfileOutcome};
use brightfield_protocol::write_chart_edit;
use brightfield_shell::data_file;
use brightfield_shell::shelf::{auto_pivot, ColourScaleWrite, Number};
use brightfield_shell::shelf_edit::{
    number_value, put_colour, put_colour_scale, put_column, put_setting, SettingWrite,
};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Spec, SpecValue};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::edit::ChartEdit;
use brightfield_spec::layout::PlotAxis;
use brightfield_spec::{parse_spec, Format};

const FILE: &str = "california_housing_sample.csv";

/// The protocol writer's capture of the generator's text for [`FILE`], with
/// the `data:` block's path written as `/data/`.
const CAPTURE: &str =
    include_str!("../../brightfield-protocol/tests/chart_text/generated_california_housing.yaml");

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-written-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The housing file opened: the text the generator wrote, the spec parsed from
/// it, the table's profile, and the hero map's path.
struct Opened {
    text: String,
    spec: Spec,
    table: Vec<ColumnProfile>,
    hero: ComponentPath,
    _dir: TempDir,
}

fn open(name: &str) -> Opened {
    let dir = TempDir::new(name);
    let path = dir.0.join(FILE);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(FILE),
        &path,
    )
    .expect("the housing fixture copies");
    let mut file = data_file::open(path.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let spec_file = file
        .spec_file
        .as_ref()
        .expect("the generator wrote its text to a scratch file");
    let text = std::fs::read_to_string(spec_file).expect("the generated text reads");
    let table = file
        .live
        .coordinator()
        .session()
        .profile_sources()
        .into_iter()
        .find(|p| p.name == data_file::SOURCE)
        .map(|p| match p.outcome {
            ProfileOutcome::Profiled { columns, .. } => columns,
            other => panic!("the table did not profile: {other:?}"),
        })
        .expect("the opened file has a source to profile");
    let spec = parse_spec(&text, Format::Yaml)
        .expect("the generated text parses")
        .spec;
    let hero = ComponentPath(file.composed.plots[0].path.clone());
    Opened {
        text,
        spec,
        table,
        hero,
        _dir: dir,
    }
}

/// `text` with each of `edits` written into the text the one before it left.
fn write_all(text: &str, edits: &[ChartEdit]) -> String {
    edits.iter().fold(text.to_string(), |text, edit| {
        write_chart_edit(&text, edit).unwrap_or_else(|e| panic!("{edit:?} is written: {e}"))
    })
}

/// `text`'s `file:` line, the one line that names where the table was opened
/// from, written as the capture writes it.
fn with_capture_path(text: &str) -> String {
    let capture_line = CAPTURE
        .lines()
        .find(|l| l.trim_start().starts_with("file: "))
        .expect("the capture names its file");
    text.lines()
        .map(|l| {
            if l.trim_start().starts_with("file: ") {
                capture_line
            } else {
                l
            }
        })
        .fold(String::new(), |out, l| out + l + "\n")
}

/// The capture the writer's tests run over is the text the generator writes
/// for the housing file, but for the path its `data:` block names.
#[test]
fn the_protocol_writers_capture_is_the_text_the_generator_writes() {
    let opened = open("capture");
    assert_eq!(with_capture_path(&opened.text), CAPTURE);
}

/// **`median_income` on the map's x, written**: the shelf's edits change both
/// layers' `x:` lines and take the `projectionType:` line out, and the text
/// reads back as the spec the shelf made. Every other line stays.
#[test]
fn the_shelfs_x_edit_on_the_generated_map_is_written_into_its_text() {
    let mut opened = open("income");
    let edits = put_column(
        &mut opened.spec,
        &opened.hero,
        PlotAxis::X,
        "median_income",
        &opened.table,
    )
    .expect("the shelf takes median_income");
    let written = write_all(&opened.text, &edits);

    let expected = opened
        .text
        .replace("        x: 'longitude'\n", "        x: 'median_income'\n")
        .replace("      projectionType: equirectangular\n", "");
    assert_eq!(
        opened.text.matches("        x: 'longitude'\n").count(),
        2,
        "the generated map's two layers each carry their x"
    );
    assert_eq!(written, expected);
    assert_eq!(
        parse_spec(&written, Format::Yaml)
            .expect("the written text parses")
            .spec,
        opened.spec,
        "the text reads back as the spec the shelf made"
    );
}

/// **`longitude` put back on x after that** gives the text the generator
/// wrote, byte for byte.
#[test]
fn the_shelfs_x_edit_put_back_gives_the_generated_text() {
    let mut opened = open("back");
    let away = put_column(
        &mut opened.spec,
        &opened.hero,
        PlotAxis::X,
        "median_income",
        &opened.table,
    )
    .expect("the shelf takes median_income");
    let rebound = write_all(&opened.text, &away);
    let back = put_column(
        &mut opened.spec,
        &opened.hero,
        PlotAxis::X,
        "longitude",
        &opened.table,
    )
    .expect("the shelf takes longitude");
    assert!(
        back.iter().any(
            |e| matches!(e, ChartEdit::SetPlotAttribute { key, .. } if key == "projectionType")
        ),
        "the pair back on the plot sets the projection again: {back:?}"
    );
    assert_eq!(write_all(&rebound, &back), opened.text);
}

/// The map's last plot attribute in the generator's text, after which the
/// writer adds a plot attribute the map does not carry.
const LAST_ATTRIBUTE: &str = "      projectionType: equirectangular\n";

/// The map's highlighted layer, whose `fill:` line the colour edit adds.
const HIGHLIGHTED: &str = "        data: { from: opened, filterBy: $sel }\n        x: 'longitude'\n        y: 'latitude'\n";

/// The map's last item, the interactor the list ends on, after which the
/// writer adds the plot's legend.
const LAST_ITEM: &str = "      - select: intervalXY\n        as: $sel\n";

/// `text` with the three lines a number column on the map's colour adds: a
/// `fill:` on the highlighted layer, the scheme after the plot's last
/// attribute, and the legend after the plot's last item.
fn with_colour(text: &str, column: &str, last_attribute: &str) -> String {
    assert_eq!(text.matches(LAST_ITEM).count(), 1);
    text.replace(
        HIGHLIGHTED,
        &format!("{HIGHLIGHTED}        fill: {column}\n"),
    )
    .replace(LAST_ITEM, &format!("{LAST_ITEM}      - legend: color\n"))
    .replace(
        last_attribute,
        &format!("{last_attribute}      colorScheme: viridis\n"),
    )
}

/// **`median_house_value` on the map's colour, written**: one
/// `fill: median_house_value` line in the highlighted layer, one
/// `colorScheme: viridis` line in the plot and one `- legend: color` line at
/// the end of its list, and no other line changed.
#[test]
fn the_shelfs_colour_edit_on_the_generated_map_adds_a_fill_line_a_scheme_line_and_a_legend_line() {
    let mut opened = open("colour");
    let edits = put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_house_value",
        &opened.table,
    )
    .expect("the shelf takes median_house_value");
    let written = write_all(&opened.text, &edits);

    assert_eq!(opened.text.matches(HIGHLIGHTED).count(), 1);
    assert_eq!(opened.text.matches(LAST_ATTRIBUTE).count(), 1);
    assert_eq!(
        written,
        with_colour(&opened.text, "median_house_value", LAST_ATTRIBUTE)
    );
    assert_eq!(
        parse_spec(&written, Format::Yaml)
            .expect("the written text parses")
            .spec,
        opened.spec,
        "the text reads back as the spec the shelf made"
    );
}

/// `write` put on the hero of `spec` and written into `text`, with the text read
/// back as the spec the shelf made.
fn scaled(text: &str, spec: &mut Spec, hero: &ComponentPath, write: &ColourScaleWrite) -> String {
    let edits = put_colour_scale(spec, hero, write).expect("the scale writes");
    let written = write_all(text, &edits);
    assert_eq!(
        parse_spec(&written, Format::Yaml).unwrap().spec,
        *spec,
        "{write:?}: the text reads back as the spec the shelf made"
    );
    written
}

/// `text` with the hero's `colorRange` set to `colours`, and the spec read from it.
fn with_range(text: &str, hero: &ComponentPath, colours: &[&str]) -> (String, Spec) {
    let edit = ChartEdit::SetPlotAttribute {
        plot: hero.clone(),
        key: "colorRange".to_string(),
        value: SpecValue::Array(
            colours
                .iter()
                .map(|c| SpecValue::String((*c).to_string()))
                .collect(),
        ),
    };
    let written = write_all(text, &[edit]);
    let spec = parse_spec(&written, Format::Yaml).unwrap().spec;
    (written, spec)
}

/// The three colours of `colorRange` in the text, as the file writes them.
fn range_of(text: &str) -> Vec<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("colorRange:"))
        .expect("the file writes a colorRange");
    line.split_once('[')
        .expect("a list")
        .1
        .trim_end_matches(']')
        .split(',')
        .map(|c| c.trim().trim_matches(['\'', '"']).to_string())
        .collect()
}

/// **The scale row's three values write and take out their keys**, each switch
/// taking out what the scale it leaves wrote: diverging names its pivot and
/// brightfield's pair and takes the scheme out; steps names its count and the
/// scheme; linear takes the scale's keys out and keeps the scheme.
#[test]
fn the_scale_rows_three_values_write_and_take_out_their_keys() {
    let mut opened = open("scale_rows");
    let edits = put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_income",
        &opened.table,
    )
    .expect("colour takes the column");
    let coloured = write_all(&opened.text, &edits);
    let hero = opened.hero.clone();
    let diverging = ColourScaleWrite::Diverging {
        pivot: Number::new(3.47),
    };
    let steps = ColourScaleWrite::Steps { count: 5 };

    // Linear to diverging.
    let mut spec = opened.spec.clone();
    let written = scaled(&coloured, &mut spec, &hero, &diverging);
    assert!(written.contains("colorScale: diverging"));
    assert!(written.contains("colorPivot: 3.47"));
    assert!(
        written.contains("colorRange: ["),
        "the pair is written as colours"
    );
    assert!(
        !written.contains("colorScheme"),
        "diverging takes the scheme out"
    );

    // Diverging to steps: the pivot and the pair go, the scheme comes back.
    let mut from_diverging = spec.clone();
    let stepped = scaled(&written, &mut from_diverging, &hero, &steps);
    assert!(stepped.contains("colorScale: quantize"));
    assert!(stepped.contains("colorN: 5"));
    assert!(stepped.contains("colorScheme: viridis"));
    assert!(!stepped.contains("colorPivot") && !stepped.contains("colorRange"));

    // Steps to diverging: the count goes, the scheme goes.
    let mut spec_again = from_diverging.clone();
    let back = scaled(&stepped, &mut spec_again, &hero, &diverging);
    assert!(back.contains("colorScale: diverging") && back.contains("colorPivot: 3.47"));
    assert!(!back.contains("colorN"), "steps' count is taken out");
    assert!(!back.contains("colorScheme"), "and the scheme with it");

    // Diverging to linear: the scale's keys and the pair go, the scheme is named.
    let mut to_linear = spec.clone();
    let linear = scaled(&written, &mut to_linear, &hero, &ColourScaleWrite::Linear);
    assert!(
        !linear.contains("colorPivot")
            && !linear.contains("colorRange")
            && !linear.contains("colorScale"),
        "{linear}"
    );
    assert!(
        linear.contains("colorScheme: viridis"),
        "linear names the scheme"
    );

    // Steps to linear: the count goes and the scheme stays.
    let mut steps_to_linear = from_diverging.clone();
    let linear = scaled(
        &stepped,
        &mut steps_to_linear,
        &hero,
        &ColourScaleWrite::Linear,
    );
    assert!(!linear.contains("colorScale") && !linear.contains("colorN"));
    assert!(
        linear.contains("colorScheme: viridis"),
        "linear keeps the scheme"
    );
}

/// **A `colorRange` that is not brightfield's pair is the analyst's and stays**
/// through a switch to linear and to steps.
#[test]
fn a_colour_range_of_the_analysts_survives_a_switch_to_linear_and_to_steps() {
    let mut opened = open("scale_rows_own_range");
    let edits = put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_income",
        &opened.table,
    )
    .expect("colour takes the column");
    let coloured = write_all(&opened.text, &edits);
    let (text, spec) = with_range(&coloured, &opened.hero, &["#000000", "#ffffff", "#ff0000"]);
    for write in [
        ColourScaleWrite::Linear,
        ColourScaleWrite::Steps { count: 5 },
    ] {
        let mut spec = spec.clone();
        let written = scaled(&text, &mut spec, &opened.hero, &write);
        assert!(
            written.contains("colorRange: ["),
            "{write:?} left the analyst's range alone"
        );
        assert!(written.contains("#ffffff"), "{written}");
    }
}

/// **The pair is recognised as the renderer reads it**: upper-case hex is still
/// brightfield's own and is taken out with the scale.
#[test]
fn an_upper_case_pair_is_still_the_pair_and_goes_with_the_scale() {
    let mut opened = open("scale_rows_upper");
    let edits = put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_income",
        &opened.table,
    )
    .expect("colour takes the column");
    let coloured = write_all(&opened.text, &edits);
    let mut spec = opened.spec.clone();
    let diverging = scaled(
        &coloured,
        &mut spec,
        &opened.hero,
        &ColourScaleWrite::Diverging {
            pivot: Number::new(3.47),
        },
    );
    let upper: Vec<String> = range_of(&diverging)
        .iter()
        .map(|c| c.to_uppercase())
        .collect();
    let upper: Vec<&str> = upper.iter().map(String::as_str).collect();
    let (text, mut spec) = with_range(&diverging, &opened.hero, &upper);
    assert!(text.contains(upper[0]), "fixture check: upper-case hex");
    let linear = scaled(&text, &mut spec, &opened.hero, &ColourScaleWrite::Linear);
    assert!(!linear.contains("colorRange"), "{linear}");
}

/// **A file's comments are as they were after the colour edit is written**, an
/// own-line comment between the plot's list and its attributes and a comment
/// after its last attribute among them: the lines that hold a `#` are the same
/// lines in the same order, and the text reads back as the spec the shelf made.
#[test]
fn the_colour_lines_are_written_among_a_files_comments_and_leave_them() {
    let opened = open("comments");
    let commented_last = "      projectionType: equirectangular # drawn as a map\n";
    let text = opened.text.replace(LAST_ATTRIBUTE, commented_last).replace(
        "      width: 620\n",
        "      # the size the map is drawn at\n      width: 620\n",
    );
    assert_eq!(text.matches("drawn as a map").count(), 1);
    assert_eq!(text.matches("the size the map is drawn at").count(), 1);
    let mut spec = parse_spec(&text, Format::Yaml)
        .expect("the commented text parses")
        .spec;

    let edits = put_colour(&mut spec, &opened.hero, "median_house_value", &opened.table)
        .expect("the shelf takes median_house_value");
    let written = write_all(&text, &edits);

    let with_comment = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| l.contains('#'))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        with_comment(&written),
        with_comment(&text),
        "writing the colour changed a line that holds a comment"
    );
    assert_eq!(
        written,
        with_colour(&text, "median_house_value", commented_last)
    );
    assert_eq!(
        parse_spec(&written, Format::Yaml)
            .expect("the written text parses")
            .spec,
        spec,
        "the text reads back as the spec the shelf made"
    );
}

#[test]
fn a_new_column_on_colour_takes_a_set_domain_out_and_re_reads_the_pivot() {
    let mut opened = open("colour_domain_reset");
    put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_income",
        &opened.table,
    )
    .unwrap();
    put_colour_scale(
        &mut opened.spec,
        &opened.hero,
        &ColourScaleWrite::Diverging {
            pivot: Number::new(2.0),
        },
    )
    .unwrap();
    put_setting(
        &mut opened.spec,
        &opened.hero,
        "colorDomain",
        &SpecValue::Null,
        &SettingWrite::Value(SpecValue::String("Fixed".into())),
    )
    .unwrap();

    put_colour(&mut opened.spec, &opened.hero, "house_age", &opened.table).unwrap();
    let plot = plot_at_path(&opened.spec, &opened.hero.0).unwrap();
    assert!(!plot.attributes.contains_key("colorDomain"));
    let age = opened
        .table
        .iter()
        .find(|p| p.name == "house_age")
        .and_then(|p| p.moments.as_ref())
        .unwrap();
    assert_eq!(
        plot.attributes.get("colorPivot"),
        Some(&number_value(auto_pivot(age)))
    );
}

#[test]
fn a_column_change_writes_a_pivot_only_on_a_diverging_plot() {
    // Linear and stepped plots hold no pivot, and a column change gives them none.
    for write in [None, Some(ColourScaleWrite::Steps { count: 5 })] {
        let mut opened = open("colour_pivot_off");
        put_colour(
            &mut opened.spec,
            &opened.hero,
            "median_income",
            &opened.table,
        )
        .unwrap();
        if let Some(write) = write {
            put_colour_scale(&mut opened.spec, &opened.hero, &write).unwrap();
        }
        put_colour(&mut opened.spec, &opened.hero, "house_age", &opened.table).unwrap();
        let plot = plot_at_path(&opened.spec, &opened.hero.0).unwrap();
        assert!(!plot.attributes.contains_key("colorPivot"));
    }

    // A diverging plot's pivot comes out for a column the table holds no
    // moments of.
    let mut opened = open("colour_pivot_no_moments");
    put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_income",
        &opened.table,
    )
    .unwrap();
    put_colour_scale(
        &mut opened.spec,
        &opened.hero,
        &ColourScaleWrite::Diverging {
            pivot: Number::new(2.0),
        },
    )
    .unwrap();
    for column in &mut opened.table {
        if column.name == "house_age" {
            column.moments = None;
        }
    }
    put_colour(&mut opened.spec, &opened.hero, "house_age", &opened.table).unwrap();
    let plot = plot_at_path(&opened.spec, &opened.hero.0).unwrap();
    assert!(!plot.attributes.contains_key("colorPivot"));
}
