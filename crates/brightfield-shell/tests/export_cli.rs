//! `export-png` and `export-data`: the shipped binary, run with no window,
//! writes a panel's picture or the rows it draws from a Protocol that has run.
//!
//! The Protocols are real runs, as in `tests/protocol_panels.rs`: the CO2
//! fixture copied into a directory of the test's own and run by `arc`.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::path::Path;
use std::process::{Command, Output};

use agent_protocol::{copied_fixture, ran_fixture};
use brightfield_shell::capture::render_export;
use brightfield_shell::legend::{right_blocks, LegendSpec};
use brightfield_shell::protocol_panels::load;
use brightfield_spec::layout::Rect;
use meridian_design::control::ICON_XS;

const LINE: &str = "co2_per_capita_since_1990";
const CHANGE: &str = "co2_per_capita_change_pct";

fn export(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brightfield-shell"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn path_arg(p: &Path) -> &str {
    p.to_str().expect("a UTF-8 path")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("the binary exited")
}

#[test]
fn png_of_a_panel() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&ran.dir), LINE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let png = ran.dir.join("exports").join(format!("{LINE}.png"));
    assert_eq!(stdout(&out).trim(), png.display().to_string());

    let mut panels = load(&ran.dir, Rect::zero()).expect("the Protocol opens");
    let index = panels
        .tiles
        .iter()
        .position(|t| t.file == Path::new(&format!("panels/{LINE}.yaml")))
        .expect("the line panel");
    let composed = panels.tiles.remove(index).outcome.expect("it draws");
    let (w, h) = (composed.width, composed.height);
    assert_eq!((w, h), (760, 440), "the panel declares its size");
    let want = render_export(composed, 2.0).expect("renders");

    let got = image::open(&png).expect("a PNG").to_rgba8();
    assert_eq!(got.dimensions(), (w * 2, h * 2));
    assert_eq!(got.dimensions(), want.dimensions());
    assert!(got.as_raw() == want.as_raw(), "the pixels differ");
}

/// The rows of swatches down the legend at the right of the panel `name`'s
/// export, as the colour of each run of rows that wear one of its entries'
/// colours, in order from the top, with whether something is drawn beside it.
fn legend_swatches(dir: &Path, name: &str, png: &Path, scale: f32) -> (Vec<[u8; 3]>, Vec<[u8; 3]>) {
    let mut panels = load(dir, Rect::zero()).expect("the Protocol opens");
    let index = panels
        .tiles
        .iter()
        .position(|t| t.file == Path::new(&format!("panels/{name}.yaml")))
        .expect("the panel");
    let composed = panels.tiles.remove(index).outcome.expect("it draws");
    let blocks = right_blocks(&composed);
    assert_eq!(blocks.len(), 1, "one legend at the plot's right");
    let (_, legend, band) = &blocks[0];
    let LegendSpec::Categorical { entries } = legend else {
        panic!("a categorical legend: {legend:?}");
    };
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let wanted: Vec<[u8; 3]> = entries
        .iter()
        .map(|e| [q(e.colour[0]), q(e.colour[1]), q(e.colour[2])])
        .collect();

    let image = image::open(png).expect("a PNG").to_rgba8();
    let x = ((band.x as f32 + ICON_XS / 2.0) * scale).round() as u32;
    let label_x = ((band.x as f32 + ICON_XS + 8.0) * scale).round() as u32;
    let top = (band.y as f32 * scale).round() as u32;
    let bottom = (((band.y + band.height) as f32) * scale).round() as u32;
    let near =
        |p: &image::Rgba<u8>, c: [u8; 3]| p[3] > 250 && (0..3).all(|k| p[k].abs_diff(c[k]) <= 3);
    let mut runs: Vec<[u8; 3]> = Vec::new();
    let mut previous: Option<[u8; 3]> = None;
    for y in top..bottom.min(image.height()) {
        let p = image.get_pixel(x, y);
        let hit = wanted.iter().copied().find(|c| near(p, *c));
        if let Some(c) = hit {
            if previous != Some(c) {
                runs.push(c);
                // Each swatch has its label beside it: some pixel there that
                // is not the background the band stands on.
                let background = *image.get_pixel(label_x + 60, y.saturating_sub(4));
                let labelled = (y..(y + (ICON_XS * scale) as u32).min(image.height())).any(|ly| {
                    (label_x..(label_x + (40.0 * scale) as u32).min(image.width())).any(|lx| {
                        image
                            .get_pixel(lx, ly)
                            .0
                            .iter()
                            .zip(background.0.iter())
                            .any(|(a, b)| a.abs_diff(*b) > 40)
                    })
                });
                assert!(labelled, "the swatch at row {y} has no label beside it");
            }
        }
        previous = hit;
    }
    (runs, wanted)
}

#[test]
fn png_of_a_panel_draws_its_legend_where_the_window_does() {
    let ran = ran_fixture("co2-per-capita");
    // The line's nine countries are seven names and one "Other".
    for (name, count) in [(LINE, 8), (CHANGE, 3)] {
        let out = export(&["export-png", path_arg(&ran.dir), name]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let png = ran.dir.join("exports").join(format!("{name}.png"));
        let (runs, wanted) = legend_swatches(&ran.dir, name, &png, 2.0);
        assert_eq!(wanted.len(), count, "{name}: the legend's entries");
        assert_eq!(
            runs, wanted,
            "{name}: one swatch per entry, in its colour, in order"
        );
    }
}

#[test]
fn png_of_a_panel_whose_colours_repeat_warns_and_still_succeeds() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&ran.dir), LINE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!(
            "panels/{LINE}.yaml: 9 series share 8 colours, so the last 2 are drawn as one "
        )),
        "{}",
        stderr(&out)
    );
}

#[test]
fn png_of_a_panel_with_an_option_nothing_draws_says_so_and_still_succeeds() {
    let ran = ran_fixture("co2-per-capita");
    std::fs::write(
        ran.dir.join("panels/labelled.yaml"),
        "plot:\n  - mark: text\n    data: { from: co2_per_capita_change }\n    \
         x: change_pct\n    y: country\n    text: change_pct\n    dy: -10\n",
    )
    .expect("the panel writes");
    let out = export(&["export-png", path_arg(&ran.dir), "labelled"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        ran.dir.join("exports/labelled.png").is_file(),
        "the panel still draws"
    );
    assert_eq!(
        stderr(&out),
        "panels/labelled.yaml: mark `text` sets `dy`, which nothing in the render path reads \
         — it has no effect\n"
    );
}

#[test]
fn png_of_a_panel_whose_mark_draws_nothing_says_why_and_still_succeeds() {
    let ran = ran_fixture("co2-per-capita");
    // A bar's band axis on a scale the file wrote linear: it has rows and no
    // band to stand them in.
    std::fs::write(
        ran.dir.join("panels/blank.yaml"),
        "plot:\n  - mark: barY\n    data: { from: co2_per_capita_change }\n    \
         x: change_pct\n    y: change_pct\nxScale: linear\n",
    )
    .expect("the panel writes");
    let out = export(&["export-png", path_arg(&ran.dir), "blank"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(ran.dir.join("exports/blank.png").is_file());
    assert_eq!(
        stderr(&out),
        "panels/blank.yaml: `barY` drew nothing: x reads a number column on a linear scale, \
         and a barY draws x as bands\n"
    );
}

#[test]
fn png_of_a_panel_that_fits_the_palette_says_nothing() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&[
        "export-png",
        path_arg(&ran.dir),
        "co2_per_capita_change_pct",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
}

#[test]
fn png_takes_a_scale_and_an_out_path() {
    let ran = ran_fixture("co2-per-capita");
    let to = ran.dir.join("elsewhere").join("one.png");
    let out = export(&[
        "export-png",
        path_arg(&ran.dir.join("arcform.yaml")),
        &format!("panels/{LINE}.yaml"),
        "--scale",
        "1",
        "--out",
        path_arg(&to),
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let got = image::open(&to).expect("a PNG");
    let panels = load(&ran.dir, Rect::zero()).expect("opens");
    let composed = panels
        .tiles
        .iter()
        .find_map(|t| {
            t.outcome
                .as_ref()
                .ok()
                .filter(|_| t.file.ends_with(format!("{LINE}.yaml")))
        })
        .expect("the line panel");
    assert_eq!(
        (got.width(), got.height()),
        (composed.width, composed.height)
    );
}

#[test]
fn data_of_a_panel() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-data", path_arg(&ran.dir), CHANGE]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let csv = ran.dir.join("exports").join("co2_per_capita_change.csv");
    assert_eq!(stdout(&out).trim(), csv.display().to_string());
    let text = std::fs::read_to_string(&csv).expect("the CSV");
    assert_eq!(text.lines().count(), 10, "a header and 9 rows:\n{text}");
}

#[test]
fn data_into_a_folder_named_with_a_trailing_separator() {
    let ran = ran_fixture("co2-per-capita");
    let folder = ran.dir.join("rows");
    let out_arg = format!("{}{}", folder.display(), std::path::MAIN_SEPARATOR);
    let out = export(&["export-data", path_arg(&ran.dir), CHANGE, "--out", &out_arg]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let csv = folder.join("co2_per_capita_change.csv");
    assert!(
        csv.is_file(),
        "a trailing separator names a folder to write into: {}",
        stdout(&out)
    );
}

#[test]
fn parquet_data() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&[
        "export-data",
        path_arg(&ran.dir),
        CHANGE,
        "--format",
        "parquet",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let file = ran
        .dir
        .join("exports")
        .join("co2_per_capita_change.parquet");
    let conn = duckdb::Connection::open_in_memory().expect("DuckDB");
    let rows: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM read_parquet('{}')", file.display()),
            [],
            |r| r.get(0),
        )
        .expect("DuckDB reads it back");
    assert_eq!(rows, 9);
}

#[test]
fn unknown_panel_fails() {
    let ran = ran_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&ran.dir), "no_such_panel"]);
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("no_such_panel"), "{err}");
    assert!(err.contains(LINE) && err.contains(CHANGE), "{err}");
}

#[test]
fn broken_panel_fails() {
    let ran = ran_fixture("co2-per-capita");
    let file = ran.dir.join(format!("panels/{CHANGE}.yaml"));
    let text = std::fs::read_to_string(&file).expect("the panel");
    std::fs::write(&file, text.replace("x: change_pct", "x: change_percent")).expect("edits");
    for args in [
        ["export-png", path_arg(&ran.dir), CHANGE],
        ["export-data", path_arg(&ran.dir), CHANGE],
    ] {
        let out = export(&args);
        assert_eq!(code(&out), 1);
        assert!(
            stderr(&out).contains(&format!(
                "panels/{CHANGE}.yaml: a mark reads the column change_percent"
            )),
            "{}",
            stderr(&out)
        );
        assert!(stdout(&out).is_empty());
    }
}

#[test]
fn unrun_protocol_fails() {
    let fixture = copied_fixture("co2-per-capita");
    let out = export(&["export-png", path_arg(&fixture.dir), LINE]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("has not run yet"), "{}", stderr(&out));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        &["export-png"][..],
        &["export-png", "somewhere"],
        &["export-data", "somewhere", "p", "--format", "xlsx"],
        &["export-png", "somewhere", "p", "--scale", "0"],
        &["export-png", "somewhere", "p", "--bogus"],
    ] {
        let out = export(args);
        assert_eq!(code(&out), 2, "{args:?}: {}", stderr(&out));
        assert!(!stderr(&out).is_empty());
    }
}

#[test]
fn help_lists_the_exports() {
    let out = export(&["--help"]);
    assert_eq!(code(&out), 0);
    let help = stdout(&out);
    for name in ["export-pdf", "export-png", "export-data"] {
        assert!(help.contains(name), "{name} is missing:\n{help}");
    }
}
