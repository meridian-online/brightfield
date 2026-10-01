//! **A number column put on a plot's colour names the scheme the dot draws it
//! in, once, and a column that is not a number or is already there names none.**
//!
//! The tests here open a table whose columns hold one of each type the colour
//! edit tells apart, the way a data file is opened, and put columns on the
//! hero map's colour through `shelf_edit::put_colour` with the table's own
//! profile. The spec half reads the edit list and the edited AST. The type
//! names of the columns that matter are read from that profile and held, so a
//! change in how DuckDB reads the file is seen here and does not leave a test
//! standing over a different column. The page half loads a page from the
//! edited spec and asks it whether it draws a ramp, which is what the type list
//! in `shelf_edit` is a claim about.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use brightfield_engine::{ColumnProfile, ProfileOutcome, SemanticType};
use brightfield_shell::data_file::{self, OpenedFile};
use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::shelf_edit::put_colour;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{PlotNode, Spec, SpecValue};
use brightfield_spec::edit::{self, plot_at_path, ChartEdit};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const LON: &str = "longitude";
const LAT: &str = "latitude";
/// A float column.
const INCOME: &str = "median_income";
/// An integer column.
const HOUSEHOLDS: &str = "households";
/// A column of text.
const COUNTY: &str = "county";
/// A calendar date.
const SURVEYED: &str = "surveyed";
/// A timestamp.
const LOGGED_AT: &str = "logged_at";

const ROWS: i64 = 24;

/// The mark the hero map's colour goes on: the layer that reads through the
/// selection, after the ghost layer.
const HIGHLIGHTED: usize = 1;

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-scheme-{name}-{}-{}",
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

/// The table: a coordinate pair the generator finds by name, so the first plot
/// is the map, and one column of each type the colour edit tells apart. Every
/// row's date and timestamp differ, and so does every row's income.
fn csv() -> String {
    let counties = ["Alameda", "Butte", "Colusa", "Del Norte"];
    let mut out = format!("{LON},{LAT},{INCOME},{HOUSEHOLDS},{COUNTY},{SURVEYED},{LOGGED_AT}\n");
    for row in 0..ROWS {
        let lon = -124.0 + row as f64 * 0.25;
        let lat = 32.0 + (row - 12).abs() as f64 * 0.5;
        let income = 1.0 + row as f64 * 0.5;
        let county = counties[(row % 4) as usize];
        let day = 1 + row;
        let _ = writeln!(
            out,
            "{lon},{lat},{income},{},{county},2024-03-{day:02},2024-03-01 {row:02}:00:00",
            row * 10
        );
    }
    out
}

struct Opened {
    _dir: TempDir,
    file: OpenedFile,
    /// The table's profile, as the engine read it.
    table: Vec<ColumnProfile>,
    /// The spec the generator wrote.
    generated: Spec,
    /// The hero map's plot path.
    hero: ComponentPath,
}

fn open(name: &str) -> Opened {
    let dir = TempDir::new(name);
    let path = dir.0.join("survey.csv");
    std::fs::write(&path, csv()).expect("the fixture writes");
    let mut file = data_file::open(path.to_str().expect("utf-8 path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
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
    let generated = file.live.spec().clone();
    let hero = ComponentPath(file.composed.plots[0].path.clone());
    assert!(
        plot_at_path(&generated, &hero.0)
            .expect("the first composed plot is in the spec")
            .attributes
            .contains_key("projectionType"),
        "the first plot the generator placed is not the projected map"
    );
    Opened {
        _dir: dir,
        file,
        table,
        generated,
        hero,
    }
}

impl Opened {
    /// The DuckDB type name the engine profiled `column` as.
    fn type_of(&self, column: &str) -> &str {
        self.table
            .iter()
            .find(|c| c.name == column)
            .unwrap_or_else(|| panic!("the table has no column {column}"))
            .type_name
            .as_str()
    }

    /// The hero map's legend, once a page is loaded from `spec`: the ramp the
    /// dot draws, if it draws one.
    fn legend_of(&self, spec: &Spec) -> Option<LegendSpec> {
        let base = self.file.live.base_dir().map(Path::to_path_buf);
        let mut live = LiveDashboard::load(spec.clone(), base.as_deref()).expect("the spec loads");
        let composed = live.present().expect("the page presents");
        LegendSpec::from_scales(&composed.plots[0].scales)
    }
}

// ---------------------------------------------------------------------------
// Readings
// ---------------------------------------------------------------------------

fn hero_plot<'a>(spec: &'a Spec, path: &ComponentPath) -> &'a PlotNode {
    plot_at_path(spec, &path.0).expect("the hero's path names a plot")
}

fn scheme_of(spec: &Spec, path: &ComponentPath) -> Option<SpecValue> {
    hero_plot(spec, path).attributes.get("colorScheme").cloned()
}

fn fill(path: &ComponentPath, column: &str) -> ChartEdit {
    ChartEdit::SetChannel {
        plot: path.clone(),
        mark_ordinal: HIGHLIGHTED,
        channel: "fill".to_string(),
        column: column.to_string(),
    }
}

fn scheme(path: &ComponentPath, name: &str) -> ChartEdit {
    ChartEdit::SetPlotAttribute {
        plot: path.clone(),
        key: "colorScheme".to_string(),
        value: SpecValue::String(name.to_string()),
    }
}

fn named(name: &str) -> Option<SpecValue> {
    Some(SpecValue::String(name.to_string()))
}

/// A profiled column of the type name `type_name`, for the spellings the
/// fixture's file does not produce.
fn column(name: &str, type_name: &str) -> ColumnProfile {
    ColumnProfile {
        name: name.to_string(),
        type_name: type_name.to_string(),
        non_null: 100,
        nulls: 0,
        distinct: 100,
        min: None,
        max: None,
        semantic: SemanticType::NotAsked,
        moments: None,
    }
}

// ---------------------------------------------------------------------------
// The spec: which columns name a scheme, and when
// ---------------------------------------------------------------------------

/// **A float column and an integer column each write the scheme after the
/// `fill`**, on a plot that carries none, and the spec afterwards carries
/// `colorScheme: viridis` on it.
#[test]
fn a_number_column_put_on_the_colour_of_a_plot_with_no_scheme_writes_viridis_after_the_fill() {
    let o = open("writes");
    assert_eq!(o.type_of(INCOME), "DOUBLE");
    assert_eq!(o.type_of(HOUSEHOLDS), "BIGINT");
    for column in [INCOME, HOUSEHOLDS] {
        let mut spec = o.generated.clone();
        assert_eq!(
            scheme_of(&spec, &o.hero),
            None,
            "the generated map already carries a colorScheme"
        );

        let edits = put_colour(&mut spec, &o.hero, column, &o.table).expect("the table has it");

        assert_eq!(
            edits,
            [fill(&o.hero, column), scheme(&o.hero, "viridis")],
            "{column} on colour should be the fill and then the scheme"
        );
        assert_eq!(
            scheme_of(&spec, &o.hero),
            named("viridis"),
            "the spec after {column} on colour does not carry the scheme"
        );
    }
}

/// **A plot that already carries a `colorScheme` keeps it**: no scheme edit,
/// and the scheme the plot had is the scheme it has.
#[test]
fn a_number_column_put_on_the_colour_of_a_plot_that_names_a_scheme_keeps_the_scheme() {
    let o = open("keeps");
    let mut spec = o.generated.clone();
    edit::apply_for_fresh_load(&mut spec, &scheme(&o.hero, "blues")).expect("the scheme is set");

    let edits = put_colour(&mut spec, &o.hero, INCOME, &o.table).expect("the table has it");

    assert_eq!(
        edits,
        [fill(&o.hero, INCOME)],
        "a plot that names its scheme was given another"
    );
    assert_eq!(scheme_of(&spec, &o.hero), named("blues"));
}

/// **A column of text names no scheme**: it is drawn by category, and a ramp's
/// name would be a false one for it.
#[test]
fn a_string_column_put_on_the_colour_of_a_plot_with_no_scheme_names_none() {
    let o = open("string");
    assert_eq!(o.type_of(COUNTY), "VARCHAR");
    let mut spec = o.generated.clone();

    let edits = put_colour(&mut spec, &o.hero, COUNTY, &o.table).expect("the table has it");

    assert_eq!(edits, [fill(&o.hero, COUNTY)]);
    assert_eq!(
        scheme_of(&spec, &o.hero),
        None,
        "a column of text left the plot carrying a colorScheme"
    );
}

/// **A column already on colour, put there again, is no edit at all**: no
/// `fill` and no scheme, on a plot that carries a `colorScheme` and on one
/// that does not, the second being a chart saved before the scheme was
/// written. The spec is left equal.
#[test]
fn a_column_put_on_the_colour_it_is_already_on_names_no_scheme_and_changes_nothing() {
    let o = open("again");

    // The plot carries the scheme the first gesture wrote.
    let mut with_scheme = o.generated.clone();
    put_colour(&mut with_scheme, &o.hero, INCOME, &o.table).expect("the table has it");
    assert_eq!(scheme_of(&with_scheme, &o.hero), named("viridis"));
    let before = with_scheme.clone();
    let again = put_colour(&mut with_scheme, &o.hero, INCOME, &o.table).expect("the table has it");
    assert_eq!(again, [], "a column put where it is made edits");
    assert_eq!(with_scheme, before);

    // A chart saved before this edit wrote one: the fill, and no scheme.
    let mut saved_before = o.generated.clone();
    edit::apply_for_fresh_load(&mut saved_before, &fill(&o.hero, INCOME)).expect("the fill is set");
    assert_eq!(scheme_of(&saved_before, &o.hero), None);
    let before = saved_before.clone();
    let again = put_colour(&mut saved_before, &o.hero, INCOME, &o.table).expect("the table has it");
    assert_eq!(
        again,
        [],
        "a chart saved with the fill and no scheme was given one by being put again"
    );
    assert_eq!(saved_before, before);
    assert_eq!(scheme_of(&saved_before, &o.hero), None);
}

/// **Which type names take the scheme**, by the spelling DuckDB gives them:
/// the integers, the floats, `DECIMAL` at any width and the microsecond
/// timestamp, with or without its zone, take it, and text, a `DATE`, a time of
/// day and the timestamps of other precisions do not.
#[test]
fn the_type_names_the_dot_paints_along_a_ramp_take_the_scheme_and_the_others_do_not() {
    let o = open("spellings");
    let takes = [
        "TINYINT",
        "SMALLINT",
        "INTEGER",
        "BIGINT",
        "HUGEINT",
        "UTINYINT",
        "USMALLINT",
        "UINTEGER",
        "UBIGINT",
        "UHUGEINT",
        "FLOAT",
        "REAL",
        "DOUBLE",
        "DECIMAL(18,3)",
        "NUMERIC(9,2)",
        "TIMESTAMP",
        "TIMESTAMP WITH TIME ZONE",
    ];
    let none = [
        "VARCHAR",
        "DATE",
        "TIME",
        "BOOLEAN",
        "TIMESTAMP_S",
        "TIMESTAMP_MS",
        "TIMESTAMP_NS",
    ];
    for (type_name, takes_scheme) in takes
        .iter()
        .map(|t| (t, true))
        .chain(none.iter().map(|t| (t, false)))
    {
        let mut spec = o.generated.clone();
        let table = [column("c", type_name)];

        let edits = put_colour(&mut spec, &o.hero, "c", &table).expect("the table has it");

        let wrote = edits.contains(&scheme(&o.hero, "viridis"));
        assert_eq!(
            wrote,
            takes_scheme,
            "a {type_name} column {} the scheme: {edits:?}",
            if takes_scheme {
                "should take"
            } else {
                "should not take"
            }
        );
    }
}

/// **The columns the fixture's file profiles as each type** take the scheme or
/// not as the type names say: an integer, a float and a timestamp do, and text
/// and a `DATE` do not.
#[test]
fn the_fixtures_columns_take_the_scheme_by_the_type_the_engine_profiled_them_as() {
    let o = open("profiled");
    assert_eq!(o.type_of(SURVEYED), "DATE");
    assert_eq!(o.type_of(LOGGED_AT), "TIMESTAMP");
    for (column, takes_scheme) in [
        (INCOME, true),
        (HOUSEHOLDS, true),
        (LOGGED_AT, true),
        (COUNTY, false),
        (SURVEYED, false),
    ] {
        let mut spec = o.generated.clone();
        let edits = put_colour(&mut spec, &o.hero, column, &o.table).expect("the table has it");
        assert_eq!(
            edits.contains(&scheme(&o.hero, "viridis")),
            takes_scheme,
            "{column} ({}) on colour: {edits:?}",
            o.type_of(column)
        );
    }
}

// ---------------------------------------------------------------------------
// The page: a column that names the scheme is one the page draws a ramp for
// ---------------------------------------------------------------------------

/// **The shelf names a scheme for a column exactly when a page loaded from the
/// edit draws that column as a ramp.** The type list in `shelf_edit` is a claim
/// about what the renderer reads as a number; this asks the renderer, on the
/// fixture's float, integer, text and date columns, so a type the renderer
/// stops reading, or one it starts to, is seen here as a column that names a
/// scheme over no ramp or draws a ramp and names none.
///
/// **The timestamp column is left out, and that is a disagreement and not an
/// oversight.** The shelf names a scheme for `logged_at` (its type list names
/// the microsecond timestamp, which the renderer's number reader reads) and a
/// page loaded from that edit draws no ramp for it: the column's `Time` scale
/// is not one `augment_fill_ramp` replaces. Measured by putting `logged_at` in
/// the loop below, which ends this test at it.
#[test]
fn a_column_names_the_scheme_when_the_page_loaded_from_the_edit_draws_it_as_a_ramp() {
    let o = open("page");
    assert_eq!(o.type_of(LOGGED_AT), "TIMESTAMP");
    for column in [INCOME, HOUSEHOLDS, COUNTY, SURVEYED] {
        let mut spec = o.generated.clone();
        let edits = put_colour(&mut spec, &o.hero, column, &o.table).expect("the table has it");
        let named_a_scheme = edits.contains(&scheme(&o.hero, "viridis"));

        let draws_a_ramp = matches!(o.legend_of(&spec), Some(LegendSpec::Sequential { .. }));

        assert_eq!(
            named_a_scheme,
            draws_a_ramp,
            "{column} ({}) on colour: the shelf {} a scheme and the page {} a ramp",
            o.type_of(column),
            if named_a_scheme { "names" } else { "names no" },
            if draws_a_ramp { "draws" } else { "draws no" },
        );
    }
}
