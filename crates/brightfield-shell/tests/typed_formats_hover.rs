//! **The hover tooltip reads a value in its column's unit**, on the axis the chart drew.
//!
//! The columns are labelled through a column record that nominates each label, as an analyst who
//! accepted them would have, so no FineType bundle is needed. The charts are hand-written dot
//! charts over the generated dashboard's data, opened as a window with the file's column facts, and
//! the pointer is aimed at a row through the plot's own scales.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{self, OpenOptions};
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};

/// Three rows: a sterling amount, a percentage, a count, a year and a download count nobody labels.
const CSV: &str = "\
revenue,share,downloads,count,year
\"£2,412,345,000.00\",45%,1234567,1234567,2001
\"£500,000,000.00\",10%,10,10,2002
\"£3,100,000,000.00\",80%,900,900,2003
";

const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 900.0),
};

fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bf-hover-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// The fixture written in a folder of the calling test's own, with a record nominating labels
/// for every column but `downloads`.
fn fixture() -> PathBuf {
    scratch_config();
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("typed_formats_hover")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("typed_hover.csv");
    std::fs::write(&path, CSV).expect("write fixture");
    let mut record = column_record::SourceRecord::default();
    for (column, label) in [
        ("revenue", "finance.currency.amount"),
        ("share", "representation.numeric.percentage"),
        ("count", "representation.numeric.integer_number"),
        ("year", "datetime.component.year"),
    ] {
        record.fields.insert(
            column.to_string(),
            column_record::FieldRecord {
                label: Some(label.to_string()),
                nominated: true,
                ..column_record::FieldRecord::default()
            },
        );
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

/// A window over a dot chart of `x` against `y`, written with `attrs` (axis keys), over the
/// generated dashboard's own data, with the file's column facts.
fn window(x: &str, y: &str, attrs: &str) -> (MeridianApp, tick_formats::Written) {
    let path = fixture();
    let options = OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    };
    let (opened, _) = data_file::open_traced(&path.to_string_lossy(), &options)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let generated = opened.dashboard.to_spec();
    let data = &generated[..generated.find("\nhconcat:").expect("a generated layout")];
    let written = tick_formats::of(&opened.protocol.columns);
    let attrs = attrs
        .replace("{x}", written.get("revenue"))
        .replace("{y}", written.get("share"));
    let spec = format!(
        "{data}\nplot:\n  - mark: dot\n    data: {{ from: \"opened\" }}\n    x: \"{x}\"\n    y: \"{y}\"\n{attrs}width: 600\nheight: 400\n"
    );
    let spec_path = path.with_file_name("hand_written.yaml");
    std::fs::write(&spec_path, spec).expect("write the chart");
    let boot = Boot::open(
        spec_path.to_str().expect("utf-8 path"),
        Flow::Vertical,
        None,
    )
    .unwrap_or_else(|e| panic!("open {}: {e}", spec_path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    app.chart_doc_mut()
        .set_column_facts(&opened.protocol.columns);
    (app, written)
}

/// The tick formats the rule chose, by column.
mod tick_formats {
    use brightfield_shell::one_step::ColumnFacts;

    pub struct Written(Vec<(String, String)>);

    impl Written {
        pub fn get(&self, column: &str) -> &str {
            self.0
                .iter()
                .find(|(c, _)| c == column)
                .map(|(_, f)| f.as_str())
                .unwrap_or_else(|| panic!("the rule chose no tick format for {column}"))
        }
    }

    pub fn of(columns: &[ColumnFacts]) -> Written {
        Written(
            columns
                .iter()
                .filter_map(|c| Some((c.column.clone(), c.representation.tick_format.clone()?)))
                .collect(),
        )
    }
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(SCREEN),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| app.draw(ui));
}

fn rest_at(app: &mut MeridianApp, ctx: &egui::Context, at: egui::Pos2) {
    frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    frame(app, ctx, Vec::new());
}

/// The lines the readout names when the pointer rests on the point `(x, y)` of the hero plot.
fn lines_at(app: &mut MeridianApp, x: f64, y: f64) -> Vec<String> {
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(app, &ctx, Vec::new());
    }
    let rect = app.composed_plot_rects()[0];
    let scales = &app.chart_doc().composed.plots[0].scales;
    let sx = scales.get(Channel::X).expect("an x scale");
    let sy = scales.get(Channel::Y).expect("a y scale");
    #[allow(clippy::cast_possible_truncation)]
    let at = egui::pos2(
        rect.min.x + sx.map_f64(x) as f32,
        rect.min.y + sy.map_f64(y) as f32,
    );
    rest_at(app, &ctx, egui::pos2(1.0, 1.0));
    rest_at(app, &ctx, at);
    app.chart_doc()
        .hover_readout
        .as_ref()
        .map(|r| r.lines.clone())
        .unwrap_or_default()
}

/// **A raw axis that wears the rule's own format reads its value unscaled**, as its ticks do: only
/// a bin over the scaled column is drawn in billions. A rate, which has no scale, keeps its
/// attached percent sign.
#[test]
fn tooltip_reads_a_raw_axis_unscaled() {
    let (mut app, _) = window(
        "revenue",
        "share",
        "xTickFormat: \"{x}\"\nyTickFormat: \"{y}\"\n",
    );
    let lines = lines_at(&mut app, 2_412_345_000.0, 0.45);
    assert_eq!(
        lines,
        vec![
            "revenue: £2,412,345,000".to_string(),
            "share: 45%".to_string()
        ]
    );
}

/// **A column nobody labelled is printed as the engine wrote it.**
#[test]
fn tooltip_leaves_an_unlabelled_column_alone() {
    let (mut app, _) = window("revenue", "downloads", "xTickFormat: \"{x}\"\n");
    let lines = lines_at(&mut app, 2_412_345_000.0, 1_234_567.0);
    assert_eq!(
        lines,
        vec![
            "revenue: £2,412,345,000".to_string(),
            "downloads: 1234567".to_string()
        ]
    );
}

/// **A chart with no format and no scale is read as its own axis is**: the grouped default, with
/// the recorded symbol, unscaled.
#[test]
fn tooltip_follows_a_hand_written_axis() {
    let (mut app, _) = window("revenue", "share", "");
    let lines = lines_at(&mut app, 2_412_345_000.0, 0.45);
    assert_eq!(lines, vec!["revenue: £2,412,345,000", "share: 45%"]);
}

/// **A stored fraction is read as the axis the analyst wrote reads it**, with no `%` of its own.
#[test]
fn tooltip_follows_a_written_format_on_a_fraction() {
    let (mut app, _) = window("revenue", "share", "yTickFormat: \".1f\"\n");
    let lines = lines_at(&mut app, 2_412_345_000.0, 0.45);
    assert_eq!(lines[1], "share: 0.5");
}

/// **A labelled count is grouped and a labelled year is not**, on an axis with no format.
#[test]
fn tooltip_groups_a_count_and_not_a_year_on_a_bare_axis() {
    let (mut app, _) = window("year", "count", "");
    let lines = lines_at(&mut app, 2001.0, 1_234_567.0);
    assert_eq!(lines, vec!["year: 2001", "count: 1,234,567"]);
}

/// **A format the analyst wrote is the one the tooltip prints with**, with the symbol added.
#[test]
fn tooltip_follows_a_written_format() {
    let (mut app, _) = window("revenue", "share", "xTickFormat: \".3s\"\n");
    let lines = lines_at(&mut app, 2_412_345_000.0, 0.45);
    assert_eq!(lines[0], "revenue: £2.41G");
}
