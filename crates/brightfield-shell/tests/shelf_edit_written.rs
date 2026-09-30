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
use brightfield_shell::shelf_edit::{put_colour, put_column};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::Spec;
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(FILE),
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
        back.iter()
            .any(|e| matches!(e, ChartEdit::SetPlotAttribute { key, .. } if key == "projectionType")),
        "the pair back on the plot sets the projection again: {back:?}"
    );
    assert_eq!(write_all(&rebound, &back), opened.text);
}

/// **`median_house_value` on the map's colour, written**: one
/// `fill: median_house_value` line in the highlighted layer, and no other line
/// changed.
#[test]
fn the_shelfs_colour_edit_on_the_generated_map_adds_one_fill_line() {
    let mut opened = open("colour");
    let edits = put_colour(
        &mut opened.spec,
        &opened.hero,
        "median_house_value",
        &opened.table,
    )
    .expect("the shelf takes median_house_value");
    let written = write_all(&opened.text, &edits);

    let highlighted = "        data: { from: opened, filterBy: $sel }\n        x: 'longitude'\n        y: 'latitude'\n";
    assert_eq!(opened.text.matches(highlighted).count(), 1);
    assert_eq!(
        written,
        opened.text.replace(
            highlighted,
            &format!("{highlighted}        fill: median_house_value\n")
        )
    );
    assert_eq!(
        parse_spec(&written, Format::Yaml)
            .expect("the written text parses")
            .spec,
        opened.spec,
        "the text reads back as the spec the shelf made"
    );
}
