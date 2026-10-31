//! **The dashboard a file of typed columns opens as**, held as the YAML it writes and as the
//! picture it draws: money as money, rates as percentages, a year as a year, a count grouped.
//!
//! The fixture is `data/typed_formats.csv`: sterling amounts with accounting negatives, dollar
//! prices, percentages, rates, a run of years with one missing, a large count, ISO dates and free
//! text. Its columns are labelled through a column record that nominates each label, as an analyst
//! who accepted them would have, so no FineType bundle is needed and the casts run from the
//! transforms vendored into `brightfield-protocol`. The test runs, and can fail, wherever the
//! suite does.
//!
//! The YAML snapshot is the baseline that reddens when a label's format changes: each tile's tick
//! format, title unit and scale is written into it from FineType's display block for the label.
//!
//! Regenerate the snapshots with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test
//! typed_formats_baseline`, with no FineType bundle beside the test binary.

use std::path::{Path, PathBuf};

use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::column_record;
use brightfield_shell::dashboard::{Dashboard, Tile};
use brightfield_shell::data_file::{self, OpenOptions};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec_str_in;
use brightfield_shell::{chart_kinds, ranked_bars};
use brightfield_spec::layout::Rect;

/// Device pixels per logical point, as `dashboard_baseline`'s.
const SCALE: f32 = 1.0;

/// The committed fixture, written by `data/typed_formats.py`.
const TYPED_FORMATS: &str = include_str!("data/typed_formats.csv");

/// The label each of the fixture's columns is given. `notes` has none.
const TYPED_LABELS: &[(&str, &str)] = &[
    ("revenue", "finance.currency.amount"),
    ("price_usd", "finance.currency.amount"),
    ("share", "representation.numeric.percentage"),
    ("yield", "finance.rate.yield"),
    ("year", "datetime.component.year"),
    ("downloads", "representation.numeric.integer_number"),
    ("day", "datetime.date.iso"),
];

/// The committed snapshot of the YAML the fixture's dashboard is written as.
fn yaml_snapshot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/typed_formats.yaml")
}

/// Point the config folder, where a lone file's column record is kept, at a scratch folder of
/// this process's, once and before any test reads it.
fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bf-typed-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// `text` written as `typed_formats.csv` in a folder of the calling test's own, with a record
/// nominating `labels`.
///
/// A folder per test, because opening a file writes its generated spec to a path keyed by the
/// file's path and the window watches that spec; the file keeps its name, which is all the window
/// and the spec's title show of the path.
fn typed_with(text: &str, labels: &[(&str, &str)]) -> PathBuf {
    scratch_config();
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("typed_formats_baseline")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("typed_formats.csv");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
        std::fs::write(&path, text).expect("write fixture");
    }
    let mut record = column_record::SourceRecord::default();
    for (column, label) in labels {
        record.fields.insert(
            (*column).to_string(),
            column_record::FieldRecord {
                label: Some((*label).to_string()),
                nominated: true,
                ..column_record::FieldRecord::default()
            },
        );
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

/// The committed fixture, labelled.
fn typed() -> PathBuf {
    typed_with(TYPED_FORMATS, TYPED_LABELS)
}

/// The options these opens take: no type source, so a bundle beside the test binary changes
/// nothing, and the labels are the record's alone.
fn unbundled() -> OpenOptions {
    OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    }
}

/// The dashboard `path` opens as, and the YAML it writes with the file's folder taken out, so the
/// snapshot does not depend on where the test ran.
fn opened(path: &Path) -> (Dashboard, String) {
    let (opened, _) = data_file::open_traced(&path.to_string_lossy(), &unbundled())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let folder = path
        .parent()
        .expect("a folder")
        .to_string_lossy()
        .into_owned();
    let yaml = opened.dashboard.to_spec().replace(&folder, "<folder>");
    (opened.dashboard, yaml)
}

/// The block `yaml` writes for `column`'s own tile: from its comment line to the next tile's.
fn block_of<'a>(yaml: &'a str, column: &str) -> &'a str {
    let start = yaml
        .find(&format!("# {column}: "))
        .unwrap_or_else(|| panic!("no tile of {column} in:\n{yaml}"));
    let rest = &yaml[start..];
    let end = rest[1..].find("    # ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

fn tile<'a>(dash: &'a Dashboard, column: &str) -> &'a Tile {
    dash.tiles()
        .iter()
        .find(|t| t.column() == column)
        .unwrap_or_else(|| panic!("no tile of {column}"))
}

/// **The YAML the fixture's dashboard is written as, against the committed snapshot.** A change
/// to a label's display block, to the representation rule or to what a tile writes changes these
/// bytes and fails here.
#[test]
fn generated_yaml_matches_snapshot() {
    let (_, yaml) = opened(&typed());
    brightfield_spec::parse_spec(&yaml, brightfield_spec::Format::Yaml)
        .expect("the generated spec parses");
    let path = yaml_snapshot();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &yaml).expect("write the snapshot");
        return;
    }
    let committed =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(
        yaml, committed,
        "the YAML the typed fixture's dashboard is written as has changed; if the change is \
         intended, re-record it with UPDATE_SNAPSHOTS=1"
    );
}

/// **The generated YAML is read with no warning**, by the composition that draws it as well as
/// the parser: every format is one this build's d3 ports read, and every key lands on an axis
/// that takes it.
#[test]
fn generated_yaml_draws_with_no_warning() {
    let (_, yaml) = opened(&typed());
    let parsed = brightfield_spec::parse_spec(&yaml, brightfield_spec::Format::Yaml)
        .expect("the generated spec parses");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let path = typed();
    let (opened, _) =
        data_file::open_traced(&path.to_string_lossy(), &unbundled()).expect("the fixture opens");
    let said = opened.composed.diagnostics.lines();
    assert!(
        said.is_empty(),
        "the dashboard draws with warnings: {said:?}"
    );
}

/// **The sterling column is drawn in billions, and its title says so.** Its bin divides the
/// column by 10⁹ in the query, so the bins, the ticks and a brush are all in the unit the title
/// names; no `£` reaches a tick format.
#[test]
fn revenue_tile_is_scaled() {
    let (dash, yaml) = opened(&typed());
    assert_eq!(tile(&dash, "revenue").kind(), chart_kinds::BINNED_HISTOGRAM);
    let block = block_of(&yaml, "revenue");
    assert!(block.contains("xLabel: \"revenue (£bn)\""), "{block}");
    assert!(
        block.contains("x: { bin: { sql: '\"revenue\" / 1e9' } }"),
        "{block}"
    );
    assert!(block.contains("xTickFormat: "), "{block}");
    assert!(
        !block.contains("TickFormat: \"£") && !block.contains("TickFormat: \"($"),
        "{block}"
    );
    assert!(
        block.contains("finance.currency.amount chose its format, and its scale (bn)"),
        "{block}"
    );
}

/// **A year column draws as counts per year in time order.** It takes the counts-per-date kind
/// in place of ranked bars, prints `2024` and not `2,024`, and lists every year of its run as the
/// x domain, so 2009, which no row holds, keeps an empty slot.
#[test]
fn year_tile_counts_in_time_order() {
    let (dash, yaml) = opened(&typed());
    assert_eq!(tile(&dash, "year").kind(), chart_kinds::COUNTS_OVER_TIME);
    assert_ne!(tile(&dash, "year").kind(), ranked_bars::KIND_ID);
    let block = block_of(&yaml, "year");
    assert!(block.contains("xTickFormat: d\n"), "{block}");
    let years: Vec<String> = (2001..=2024).map(|y| y.to_string()).collect();
    assert!(
        block.contains(&format!("xDomain: [{}]\n", years.join(", "))),
        "{block}"
    );
    assert!(!block.contains("sort:"), "{block}");
    assert!(
        !TYPED_FORMATS
            .lines()
            .any(|l| l.split(',').any(|cell| cell == "2009")),
        "fixture check: no row holds 2009"
    );
}

/// **A run of years longer than the ceiling leaves the domain to the data**, and a column of one
/// year is left out, as any column of one value is.
#[test]
fn long_year_run_leaves_domain_to_data() {
    let mut text = String::from("year\n");
    for i in 0..400 {
        text.push_str(&format!("{}\n", 1601 + i));
    }
    let path = typed_with(&text, &[("year", "datetime.component.year")]);
    let (dash, yaml) = opened(&path);
    assert_eq!(tile(&dash, "year").kind(), chart_kinds::COUNTS_OVER_TIME);
    let block = block_of(&yaml, "year");
    assert!(!block.contains("xDomain"), "{block}");
    assert!(block.contains("xTickFormat: d\n"), "{block}");
}

/// **Every tile writes the gridlines Brightfield draws**, since Observable Plot draws none by
/// default and the file should draw the same in either.
#[test]
fn every_tile_writes_its_grid() {
    let (dash, yaml) = opened(&typed());
    assert!(!dash.tiles().is_empty());
    for t in dash.tiles() {
        let block = block_of(&yaml, t.column());
        assert!(
            block.contains("xGrid: true\n") && block.contains("yGrid: true\n"),
            "{}:\n{block}",
            t.column()
        );
    }
}

/// **The dashboard as pixels, light and dark**: every tile of the composition the generated
/// YAML draws, so the scaled axis, the units in the titles, the formatted ticks and the run of
/// years in order are all in the picture. The YAML snapshot above is the structural half.
#[test]
fn light_and_dark_pngs_match() {
    let path = typed();
    let (opened, _) = data_file::open_traced(&path.to_string_lossy(), &unbundled())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let yaml = opened.dashboard.to_spec();
    drop(opened);
    for (mode, name) in [
        (Mode::Light, "typed_formats_light"),
        (Mode::Dark, "typed_formats_dark"),
    ] {
        let composed = compose_spec_str_in(&yaml, None, Rect::zero(), mode)
            .unwrap_or_else(|e| panic!("compose {name}: {e}"));
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.capture.png"));
        let (w, h) = capture_vello_only(composed, SCALE, &out)
            .unwrap_or_else(|e| panic!("capture {name}: {e}"));
        assert!(w > 0 && h > 0, "{name}: empty capture");
        let image = image::open(&out)
            .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
            .to_rgba8();
        egui_kittest::image_snapshot(&image, name);
    }
}
