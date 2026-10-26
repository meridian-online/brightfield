//! **The compact header band names each column's type by its catalogue title.**
//!
//! The title is read from the pinned FineType bundle's catalogue, staged beside
//! the test executable by `scripts/fetch-finetype-bundle.sh`. These tests
//! assert against that real catalogue, so a machine without the bundle fails
//! them with the command that stages it rather than skipping.

use brightfield_engine::semantic::catalogue_beside;
use brightfield_shell::column_header::{
    column_header_frame, draw_column_band, ColumnBandDrawn, GridDensity,
};
use brightfield_shell::design::Mode;
use brightfield_shell::one_step::ColumnFacts;
use meridian_design::spacing;

const EMAIL: &str = "identity.person.email";

fn catalogue() -> brightfield_engine::semantic::SchemaCatalogue {
    let exe = std::env::current_exe().expect("the test executable's path");
    catalogue_beside(&exe).unwrap_or_else(|| {
        panic!(
            "no FineType bundle beside {}; stage it with \
             scripts/fetch-finetype-bundle.sh <rust-target> <target-dir>/debug/deps/finetype",
            exe.display()
        )
    })
}

fn facts(column: &str, label: Option<&str>) -> ColumnFacts {
    ColumnFacts {
        column: column.to_string(),
        label: label.map(str::to_string),
        leaf: label
            .map_or("VARCHAR", |l| l.rsplit('.').next().unwrap_or(l))
            .to_string(),
        storage: "VARCHAR".to_string(),
        tile: None,
        because: String::new(),
        paired: None,
        coordinate: false,
        rows: 240,
        nulls: 4,
        min: None,
        max: None,
        moments: None,
    }
}

/// Draw `facts` as a compact band cell `width` points wide; hand back what it
/// drew and every string painted inside the cell.
fn draw(facts: &ColumnFacts, width: f32, mode: Mode) -> (ColumnBandDrawn, Vec<String>) {
    let frame = column_header_frame(GridDensity::Compact, mode);
    let origin = egui::pos2(8.0, 8.0);
    let cell = egui::Rect::from_min_size(origin, egui::vec2(width, frame.extent()));
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(cell.right() + 8.0, cell.bottom() + 8.0),
        )),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut drawn = None;
    let out = ctx.run_ui(raw, |ui| {
        drawn = Some(draw_column_band(ui.painter(), cell, 0, facts, 0, &frame));
    });
    let mut texts = Vec::new();
    for clipped in &out.shapes {
        collect(&clipped.shape, &mut texts);
    }
    (drawn.expect("the band drew"), texts)
}

fn collect(shape: &egui::epaint::Shape, into: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) => into.push(t.galley.text().to_string()),
        egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, into)),
        _ => {}
    }
}

#[test]
fn compact_band_reads_title_by_text() {
    let catalogue = catalogue();
    let title = catalogue
        .title(EMAIL)
        .expect("the pinned catalogue titles the email label");
    assert_eq!(title, "Email Address");

    let (drawn, texts) = draw(&facts("reply_to", Some(EMAIL)), 220.0, Mode::Light);
    assert_eq!(drawn.title.as_deref(), Some(title));
    assert!(
        texts.iter().any(|t| t == title),
        "the band painted no galley reading {title:?}; it painted {texts:?}"
    );

    // An unlabelled column names no type.
    let (plain, texts) = draw(&facts("note", None), 220.0, Mode::Light);
    assert_eq!(plain.title, None);
    assert!(!texts.iter().any(|t| t == title));
}

#[test]
fn no_icon_reserves_no_width() {
    let catalogue = catalogue();
    assert_eq!(
        catalogue.icon(EMAIL),
        None,
        "the pinned catalogue carries no icon, so the slot is empty"
    );
    let (drawn, texts) = draw(&facts("reply_to", Some(EMAIL)), 220.0, Mode::Light);
    assert_eq!(drawn.icon, None);
    // With no icon the title sits flush against the cell's trailing inset, the
    // same edge the missing count aligns to, and nothing else was painted
    // between the name and it.
    let rect = drawn.title_rect.expect("the title was drawn");
    assert!(
        (rect.right() - (drawn.cell.right() - spacing::SPACE_4)).abs() < 0.5,
        "title ends at {} where the trailing inset is {}",
        rect.right(),
        drawn.cell.right() - spacing::SPACE_4
    );
    let name_and_title = texts
        .iter()
        .filter(|t| *t == "reply_to" || *t == "Email Address")
        .count();
    assert_eq!(name_and_title, 2);
}

#[test]
fn a_title_that_does_not_fit_gives_way_to_the_name() {
    let (drawn, _) = draw(&facts("reply_to", Some(EMAIL)), 100.0, Mode::Light);
    assert_eq!(drawn.title, None, "a narrow column keeps its name whole");
}

fn snapshot(mode: Mode, name: &str) {
    let columns = [facts("reply_to", Some(EMAIL)), facts("note", None)];
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(480.0, 100.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            brightfield_shell::design::apply(ui.ctx(), mode);
            let frame = column_header_frame(GridDensity::Compact, mode);
            for (i, column) in columns.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let left = 8.0 + i as f32 * 232.0;
                let cell = egui::Rect::from_min_size(
                    egui::pos2(left, 8.0),
                    egui::vec2(224.0, frame.extent()),
                );
                draw_column_band(ui.painter(), cell, i, column, i, &frame);
            }
        });
    harness.run();
    harness.snapshot(name);
}

#[test]
fn compact_band_light_baseline() {
    catalogue();
    snapshot(Mode::Light, "column_header_type_light");
}

#[test]
fn compact_band_dark_baseline() {
    catalogue();
    snapshot(Mode::Dark, "column_header_type_dark");
}
