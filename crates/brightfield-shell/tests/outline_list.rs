//! **The Outline lists a channel's columns with a query line while a shelf cell
//! is active.**
//!
//! The list is drawn into a `Ui` a test gives it, so the claims are read off the
//! frame it painted and off the state it answers a key or a click with. Four
//! kinds of reading, each at the altitude its claim lives at:
//!
//! - **Keys** go in as `egui` events through `ColumnList::feed_events`, as the
//!   window hands them: a key press followed by the text the same keystroke
//!   makes, because a real frame brings both, and a list that typed the letter it
//!   had just obeyed passes a test that sends the key alone. What is read back is the
//!   `ListReport`s, which are what the window acts on, and the state a report
//!   leaves behind.
//! - **Geometry and words** are read off the laid-out frame: the rectangles the
//!   list painted and the galleys it put in them (`text_ink::frame_text`), not
//!   off the numbers the list reports about itself.
//! - **Hue** is read off the painted fills, against colours this file computes
//!   from the design system's categorical palette by slot number, and not read
//!   back from the function that assigns a hue to a channel, which a wrong
//!   assignment would make agree with itself.
//! - **Pixels** are two baselines per mode: the list at rest and with `inc`
//!   typed. They are drawn through `egui_kittest`'s wgpu renderer and compared
//!   under `kittest.toml`'s thresholds. Regenerate them with
//!   `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test
//!   outline_list`, and read what moved before committing it.
//!
//! The Outline pane itself is driven in `the_outline_pane_*`: with a list open
//! the pane draws it in place of the plain rows, and with none, or after it is
//! closed, the pane draws as it did before there was a list.

use std::path::PathBuf;
use std::sync::OnceLock;

use brightfield_protocol::layout::Flow;
use brightfield_shell::column_header::{column_header_frame, draw_column_band, GridDensity};
use brightfield_shell::data_file;
use brightfield_shell::design::{self, Mode};
use brightfield_shell::one_step::ColumnFacts;
use brightfield_shell::protocol::{
    protocol_registry, ProtocolDoc, ProtocolModel, SpineRole, SpineRowDrawn, OUTLINE,
};
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListDrawn, ListReport,
    ListTab, ShelfChannels, QUERY_PLACEHOLDER,
};
use brightfield_shell::text_ink::{self, DrawnText};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::chrome;
use brightfield_workbench::ItemCtx;
use egui::epaint::{ClippedShape, Shape};
use egui_kittest::{Harness, SnapshotOptions};
use meridian_design::{spacing, viz};

/// The Outline rail's default width.
const WIDTH: f32 = 240.0;

/// Where the list's top left sits in the test's window, off the origin so no
/// reading assumes the list starts at zero.
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

/// The slot each channel takes in the design system's categorical order, which
/// is blue, gold, teal, red, violet, orange, plum, green, counted from zero.
const TEAL: usize = 2;
const VIOLET: usize = 4;
const ORANGE: usize = 5;

// ---------------------------------------------------------------------------
// The fixture: the table a data file opens as, and the channels the tile takes.
// ---------------------------------------------------------------------------

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

fn open() -> data_file::OpenedFile {
    let path = housing();
    data_file::open(path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// A mix of types: a date, two texts and a count, so the list has a numeric row
/// beside three that are not.
fn mixed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/dashboard_baseline.csv")
}

/// The profile facts of the table `path` opens as, in the table's order: what
/// the grid head draws a column from, and what the list is offered.
fn facts_of(path: &std::path::Path) -> Vec<ColumnFacts> {
    data_file::open(path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
        .protocol
        .inputs()
        .expect("the opened file's protocol")
        .columns
}

fn housing_facts() -> &'static [ColumnFacts] {
    static FACTS: OnceLock<Vec<ColumnFacts>> = OnceLock::new();
    FACTS.get_or_init(|| facts_of(&housing()))
}

/// `facts` as the Outline's list is offered them.
fn offered(facts: &[ColumnFacts]) -> Vec<ListColumn> {
    facts
        .iter()
        .map(|c| ListColumn {
            name: c.column.clone(),
            kind: c.leaf.clone(),
            moments: c.moments.clone(),
        })
        .collect()
}

/// The table's columns as the Outline lists them, in the table's order.
fn columns() -> &'static [ListColumn] {
    static COLUMNS: OnceLock<Vec<ListColumn>> = OnceLock::new();
    COLUMNS.get_or_init(|| offered(housing_facts()))
}

fn names() -> Vec<&'static str> {
    columns().iter().map(|c| c.name.as_str()).collect()
}

/// What the hero takes: population on x, latitude on y, and median income on
/// colour. None of the three is the table's first column, so a cursor that
/// opened on the first row would not read as the one the channel holds.
fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Column("median_income".to_string()),
    }
}

fn request(channel: ShelfChannel) -> ColumnListRequest {
    ColumnListRequest {
        tile: "hero".to_string(),
        channel,
        channels: channels(),
        columns: columns().to_vec(),
    }
}

/// What the hero's axes read when the plot's attributes are `attrs`, a block of
/// top-level lines of the spec, as the window reads them off the live plot and
/// hands them to the list.
fn settings_of(attrs: &str, channels: &ShelfChannels) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot(&spec, plot, channels)
}

/// The list the window hands the Outline: opened on `request`, and given what
/// the axes read, which here is brightfield's own on its three rows, so a column list
/// is drawn as it is with a settings tab behind it.
fn list(channel: ShelfChannel) -> ColumnList {
    let mut list = ColumnList::new(request(channel));
    list.set_settings(settings_of("", &channels()));
    list
}

// ---------------------------------------------------------------------------
// The stage: a list drawn into a `Ui` a test gives it, and what the frame held.
// ---------------------------------------------------------------------------

struct Stage {
    ctx: egui::Context,
    mode: Mode,
}

/// One frame of the list: what it reported, what it painted and what text it
/// put on the screen.
struct Frame {
    drawn: ListDrawn,
    shapes: Vec<ClippedShape>,
    texts: Vec<DrawnText>,
    collisions: Option<String>,
}

impl Stage {
    /// A stage with the theme applied and the faces loaded: the fonts `apply`
    /// installs take effect on the pass after it, so two frames run before
    /// anything is measured.
    fn new(mode: Mode) -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, mode);
        let stage = Self { ctx, mode };
        let mut warm = list(ShelfChannel::X);
        stage.frame(&mut warm, Vec::new());
        stage.frame(&mut warm, Vec::new());
        stage
    }

    fn frame(&self, list: &mut ColumnList, events: Vec<egui::Event>) -> Frame {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH + 2.0 * ORIGIN.x, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut drawn = None;
        let mut texts = Vec::new();
        let mut collisions = None;
        let output = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(ORIGIN, egui::vec2(WIDTH, 560.0))),
                |ui| drawn = Some(list.show(ui, self.mode)),
            );
            texts = text_ink::frame_text(ui.ctx());
            collisions = text_ink::collision_report(ui.ctx(), "the list");
        });
        Frame {
            drawn: drawn.expect("the list drew"),
            shapes: output.shapes,
            texts,
            collisions,
        }
    }

    /// Draw the list once it has settled, with no input.
    fn draw(&self, list: &mut ColumnList) -> Frame {
        self.frame(list, Vec::new())
    }
}

/// The leaves of each shape the frame painted, with the `Vec`s opened.
fn leaves(frame: &Frame) -> Vec<&Shape> {
    fn open<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
        match shape {
            Shape::Vec(inner) => inner.iter().for_each(|s| open(s, out)),
            other => out.push(other),
        }
    }
    let mut out = Vec::new();
    for clipped in &frame.shapes {
        open(&clipped.shape, &mut out);
    }
    out
}

/// Every filled rectangle the frame painted, with its fill.
fn fills(frame: &Frame) -> Vec<(egui::Rect, egui::Color32)> {
    leaves(frame)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => Some((r.rect, r.fill)),
            _ => None,
        })
        .collect()
}

/// The text painted inside `region`, by where its ink is centred.
fn texts_in(frame: &Frame, region: egui::Rect) -> Vec<&str> {
    frame
        .texts
        .iter()
        .filter(|t| region.contains(t.ink.center()))
        .map(|t| t.text.as_str())
        .collect()
}

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// What a keystroke that types a character brings in one frame: the key press,
/// then the text it makes.
fn typed(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![key_event(key), egui::Event::Text(text.to_string())]
}

/// `/` as a frame brings it.
fn slash() -> Vec<egui::Event> {
    typed(egui::Key::Slash, "/")
}

/// Press `/` and type `word`, a letter a frame, as a reader does.
fn search(list: &mut ColumnList, word: &str) -> Vec<ListReport> {
    let mut reports = list.feed_events(&slash());
    for ch in word.chars() {
        let key = match ch {
            'i' => egui::Key::I,
            'n' => egui::Key::N,
            'c' => egui::Key::C,
            'y' => egui::Key::Y,
            'j' => egui::Key::J,
            other => panic!("the test has no key for {other:?}"),
        };
        reports.extend(list.feed_events(&typed(key, &ch.to_string())));
    }
    reports
}

fn moved(name: &str) -> ListReport {
    ListReport::Moved(name.to_string())
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn categorical(mode: Mode, slot: usize) -> egui::Color32 {
    chrome::colour(if mode.is_dark() {
        viz::CATEGORICAL_DARK[slot]
    } else {
        viz::CATEGORICAL_LIGHT[slot]
    })
}

/// The names of the rows a frame drew, top to bottom.
fn drawn_names(frame: &Frame) -> Vec<&str> {
    frame.drawn.rows.iter().map(|r| r.column.as_str()).collect()
}

// ---------------------------------------------------------------------------
// AC1: the heading, the query line, the columns, the cursor.
// ---------------------------------------------------------------------------

#[test]
fn the_table_fixture_has_the_columns_the_tests_name() {
    // Every claim below names one of these, so a change to the fixture reddens
    // this test and not all of them.
    assert_eq!(
        names(),
        [
            "median_income",
            "house_age",
            "avg_rooms",
            "avg_bedrooms",
            "population",
            "avg_occupancy",
            "latitude",
            "longitude",
            "median_house_value",
        ]
    );
}

#[test]
fn with_the_x_cell_active_the_outline_is_headed_with_the_channel_and_the_tile() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    let heading = frame.drawn.heading;
    assert_eq!(frame.drawn.heading_text, "OUTLINE   ·   x axis of hero");
    assert_eq!(
        texts_in(&frame, heading),
        ["OUTLINE   ·   x axis of hero"],
        "the heading row holds the heading and nothing else"
    );
}

#[test]
fn the_list_has_a_query_line_that_reads_as_one_before_anything_is_typed() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    assert_eq!(texts_in(&frame, frame.drawn.query), [QUERY_PLACEHOLDER]);
    assert!(
        frame.drawn.query.top() >= frame.drawn.heading.bottom() - 0.01
            && frame.drawn.query.bottom() <= frame.drawn.rows[0].rect.top() + 0.01,
        "the query line stands between the heading and the first row"
    );
}

#[test]
fn the_list_lists_each_column_of_the_table_in_the_tables_order() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    assert_eq!(drawn_names(&frame), names());
    for row in &frame.drawn.rows {
        let said = texts_in(&frame, row.rect);
        assert!(
            said.contains(&row.column.as_str()),
            "{} is drawn in its row; the row holds {said:?}",
            row.column
        );
        let trailing = row
            .rug
            .as_ref()
            .map(|rug| rug.rect)
            .or(row.kind_rect)
            .unwrap_or_else(|| panic!("{} draws neither a rug nor its type", row.column));
        assert!(
            row.name_rect.right() <= trailing.left() + 0.01,
            "{}'s name and what stands at its trailing end do not share ink",
            row.column
        );
    }
    assert!(frame.drawn.divider.is_none(), "no query, no divider");
}

#[test]
fn the_cursor_opens_on_the_column_the_channel_holds() {
    for (channel, held) in [
        (ShelfChannel::X, "population"),
        (ShelfChannel::Y, "latitude"),
        (ShelfChannel::Colour, "median_income"),
    ] {
        let list = list(channel);
        assert_eq!(list.cursor(), Some(held), "{channel:?}");
    }
    // A channel that holds no column has the cursor nowhere.
    let mut bare = request(ShelfChannel::Colour);
    bare.channels.colour = Binding::Unset;
    assert_eq!(ColumnList::new(bare).cursor(), None);
}

#[test]
fn the_row_the_cursor_is_on_is_the_one_drawn_with_the_bar_and_no_other() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    let barred: Vec<&str> = frame
        .drawn
        .rows
        .iter()
        .filter(|r| r.bar.is_some())
        .map(|r| r.column.as_str())
        .collect();
    assert_eq!(barred, ["population"]);
}

// ---------------------------------------------------------------------------
// The rug: a numeric column's row draws its spread where its type would stand.
// ---------------------------------------------------------------------------

/// What the grid head draws of `facts` at its compact density, read off the
/// drawing: the rug's rect and the alpha of each pixel column, in a cell wide
/// enough that the rug is `width` points across.
fn grid_head_rug(stage: &Stage, facts: &ColumnFacts, width: f32) -> (egui::Rect, Vec<f32>) {
    let frame = column_header_frame(GridDensity::Compact, stage.mode);
    let cell = egui::Rect::from_min_size(
        ORIGIN,
        egui::vec2(width + 2.0 * spacing::SPACE_4, frame.extent()),
    );
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(cell.right() + ORIGIN.x, cell.bottom() + ORIGIN.y),
        )),
        ..Default::default()
    };
    let mut drawn = None;
    let _ = stage.ctx.run_ui(raw, |ui| {
        drawn = Some(draw_column_band(ui.painter(), cell, 0, facts, 0, &frame));
    });
    let drawn = drawn.expect("the grid head drew");
    (
        drawn
            .rug
            .expect("a numeric column's compact head draws a rug"),
        drawn.rug_alphas,
    )
}

/// The channels a tile over the mixed table takes, the count on x.
fn mixed_channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("reading".to_string()),
        y: Binding::Column("day".to_string()),
        colour: Binding::Column("region".to_string()),
    }
}

fn mixed_list() -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels: mixed_channels(),
        columns: offered(&facts_of(&mixed())),
    });
    list.set_settings(settings_of("", &mixed_channels()));
    list
}

/// The one-pixel-wide fills the frame painted inside `rug`, left to right: the
/// rug's columns as they reached the painter.
fn painted_rug(frame: &Frame, rug: egui::Rect) -> Vec<(egui::Rect, egui::Color32)> {
    let mut out: Vec<_> = fills(frame)
        .into_iter()
        .filter(|(r, _)| near(r.width(), 1.0) && rug.expand(0.01).contains_rect(*r))
        .collect();
    out.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
    out
}

#[test]
fn the_fixtures_measure_the_columns_the_rug_claims_name() {
    // The claims below read a row against its column's profile, so a fixture
    // that stopped measuring a column, or started measuring a text, reddens
    // here and not in a claim that happens to pass over it.
    assert!(
        housing_facts().iter().all(|c| c.moments.is_some()),
        "a housing column has no moments: {:?}",
        housing_facts()
            .iter()
            .filter(|c| c.moments.is_none())
            .map(|c| c.column.as_str())
            .collect::<Vec<_>>()
    );
    let mixed = facts_of(&mixed());
    let measured: Vec<&str> = mixed
        .iter()
        .filter(|c| c.moments.is_some())
        .map(|c| c.column.as_str())
        .collect();
    assert_eq!(measured, ["reading"]);
    let names: Vec<&str> = mixed.iter().map(|c| c.column.as_str()).collect();
    assert_eq!(names, ["day", "region", "reading", "sensor"]);
}

#[test]
fn a_numeric_row_draws_a_rug_where_its_type_would_stand() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    assert_eq!(frame.drawn.rows.len(), columns().len());
    for (row, column) in frame.drawn.rows.iter().zip(columns()) {
        let rug = row
            .rug
            .as_ref()
            .unwrap_or_else(|| panic!("{} draws no rug", row.column));
        assert_eq!(row.kind, None, "{} still names its type", row.column);
        assert_eq!(row.kind_rect, None, "{} still lays out a type", row.column);
        let said = texts_in(&frame, row.rect);
        assert!(
            !said.contains(&column.kind.as_str()),
            "{}'s row says its type, {:?}, beside the rug: {said:?}",
            row.column,
            column.kind
        );
        assert!(
            row.rect.contains_rect(rug.rect) && rug.rect.center().x > row.rect.center().x,
            "{}'s rug stands in the trailing half of its row",
            row.column
        );
        assert!(
            row.name_rect.right() <= rug.rect.left() + 0.01,
            "{}'s name and its rug do not share ink",
            row.column
        );

        // The rug is painted and not only reported: the fills that reached the
        // painter are the columns the record names, at the alphas it names.
        let painted = painted_rug(&frame, rug.rect);
        let inked: Vec<f32> = rug.alphas.iter().copied().filter(|a| *a > 0.0).collect();
        assert!(!inked.is_empty(), "{}'s rug is empty", row.column);
        assert_eq!(
            painted.len(),
            inked.len(),
            "{}: the painter was handed a different number of rug columns than the row records",
            row.column
        );
        for ((_, fill), alpha) in painted.iter().zip(&inked) {
            assert!(
                (f32::from(fill.a()) / 255.0 - alpha).abs() < 0.01,
                "{}: a rug column was painted at alpha {} and recorded at {alpha}",
                row.column,
                fill.a()
            );
        }
    }
}

#[test]
fn a_row_that_is_not_numeric_keeps_its_type() {
    let stage = Stage::new(Mode::Light);
    let facts = facts_of(&mixed());
    let mut list = mixed_list();
    let frame = stage.draw(&mut list);
    assert_eq!(drawn_names(&frame), ["day", "region", "reading", "sensor"]);
    assert_eq!(
        frame.drawn.rows.iter().filter(|r| r.rug.is_some()).count(),
        1,
        "one column of the four is numeric, and it alone draws a rug"
    );
    for (row, facts) in frame.drawn.rows.iter().zip(&facts) {
        if facts.moments.is_some() {
            let rug = row.rug.as_ref().expect("the numeric row draws a rug");
            assert!(
                row.kind.is_none() && row.kind_rect.is_none(),
                "{} draws its rug and its type",
                row.column
            );
            assert!(!painted_rug(&frame, rug.rect).is_empty());
            continue;
        }
        assert_eq!(
            row.kind.as_deref(),
            Some(facts.leaf.as_str()),
            "{} keeps its type",
            row.column
        );
        assert!(row.rug.is_none(), "{} draws a rug", row.column);
        let said = texts_in(&frame, row.rect);
        assert!(
            said.contains(&facts.leaf.as_str()),
            "{}'s type is drawn in its row; the row holds {said:?}",
            row.column
        );
        let kind_rect = row.kind_rect.expect("a row with a type lays it out");
        assert!(
            row.name_rect.right() <= kind_rect.left() + 0.01,
            "{}'s name and type do not share ink",
            row.column
        );
        let ink_in_row = fills(&frame)
            .into_iter()
            .filter(|(r, _)| near(r.width(), 1.0) && row.rect.contains_rect(*r))
            .count();
        assert_eq!(ink_in_row, 0, "{} was painted a rug", row.column);
    }
}

#[test]
fn a_rug_in_the_list_is_drawn_from_the_values_the_grid_heads_rug_is() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    for (row, facts) in frame.drawn.rows.iter().zip(housing_facts()) {
        let rug = row.rug.as_ref().expect("a numeric row draws a rug");
        let (head, head_alphas) = grid_head_rug(&stage, facts, rug.rect.width());
        assert!(
            near(head.width(), rug.rect.width()),
            "{}: the two rugs are compared at one width, and the head's is {} against the list's {}",
            row.column,
            head.width(),
            rug.rect.width()
        );
        assert_eq!(
            rug.alphas, head_alphas,
            "{}: the list's rug and the grid head's are drawn from different values",
            row.column
        );
    }
    // The comparison can tell two columns apart: the nine rugs are not one.
    let first = &frame.drawn.rows[0].rug.as_ref().expect("a rug").alphas;
    assert!(
        frame
            .drawn
            .rows
            .iter()
            .any(|r| r.rug.as_ref().is_some_and(|rug| rug.alphas != *first)),
        "every column drew the same rug, so equality with the head says nothing"
    );
}

/// The Outline's column rows, as the pane recorded them.
fn pane_columns(doc: &ProtocolDoc) -> Vec<SpineRowDrawn> {
    doc.spine_drawn
        .iter()
        .filter(|r| r.role == SpineRole::Column)
        .cloned()
        .collect()
}

#[test]
fn the_outline_pane_draws_the_rugs_while_a_list_is_open_and_the_types_when_none_is() {
    let stage = Stage::new(Mode::Light);
    let mut doc = doc();
    let ctx = settled_pane(&mut doc);

    // With no list open a numeric row says its type, as it did before there was
    // a list to open: the type the profile gave it, and no rug.
    let plain = pane_columns(&doc);
    assert_eq!(plain.len(), housing_facts().len());
    for (row, facts) in plain.iter().zip(housing_facts()) {
        assert_eq!(row.kind, facts.leaf, "{} says its type", row.label);
        assert!(row.kind_rect.is_some(), "{} lays its type out", row.label);
        assert!(row.rug.is_none(), "{} draws a rug", row.label);
    }

    // With one open, the rows are the pane's own and the rugs are the values
    // the grid head is drawn from, reached through the model's own opening of
    // the list and not through a request a test built.
    doc.model
        .open_column_list("hero", ShelfChannel::X, channels());
    run_pane(&mut doc, &ctx, Mode::Light);
    let listed = pane_columns(&doc);
    assert_eq!(listed.len(), housing_facts().len());
    for (row, facts) in listed.iter().zip(housing_facts()) {
        let rug = row
            .rug
            .as_ref()
            .unwrap_or_else(|| panic!("{} draws no rug in the pane", row.label));
        assert_eq!(row.kind, "", "{} still says its type", row.label);
        assert_eq!(row.kind_rect, None);
        let (_, head_alphas) = grid_head_rug(&stage, facts, rug.rect.width());
        assert_eq!(
            rug.alphas, head_alphas,
            "{}: the pane's rug and the grid head's are drawn from different values",
            row.label
        );
    }

    // Closed again, the rows say their types.
    doc.model.close_column_list();
    run_pane(&mut doc, &ctx, Mode::Light);
    assert_eq!(pane_columns(&doc), plain);
}

// ---------------------------------------------------------------------------
// AC2: `j` and `k` move the cursor, and each move reports the column.
// ---------------------------------------------------------------------------

#[test]
fn j_and_k_move_the_cursor_a_row_and_each_move_reports_the_column_under_it() {
    let mut list = list(ShelfChannel::X);
    assert_eq!(
        list.feed_events(&typed(egui::Key::J, "j")),
        [moved("avg_occupancy")]
    );
    assert_eq!(list.cursor(), Some("avg_occupancy"));
    assert_eq!(
        list.feed_events(&typed(egui::Key::J, "j")),
        [moved("latitude")]
    );
    assert_eq!(
        list.feed_events(&typed(egui::Key::K, "k")),
        [moved("avg_occupancy")]
    );
    assert_eq!(list.cursor(), Some("avg_occupancy"));
    // The arrows are `j` and `k`'s twins.
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::ArrowDown)]),
        [moved("latitude")]
    );
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::ArrowUp)]),
        [moved("avg_occupancy")]
    );
    // A letter the cursor obeyed is not also typed.
    assert_eq!(list.query(), "");
    assert!(!list.querying());
}

#[test]
fn the_cursor_stops_at_the_first_and_last_rows_and_reports_nothing_there() {
    let mut list = list(ShelfChannel::X);
    for _ in 0..20 {
        list.feed_events(&typed(egui::Key::J, "j"));
    }
    assert_eq!(list.cursor(), Some("median_house_value"));
    assert_eq!(list.feed_events(&typed(egui::Key::J, "j")), []);
    for _ in 0..20 {
        list.feed_events(&typed(egui::Key::K, "k"));
    }
    assert_eq!(list.cursor(), Some("median_income"));
    assert_eq!(list.feed_events(&typed(egui::Key::K, "k")), []);
}

#[test]
fn the_row_the_cursor_moves_to_is_the_row_drawn_with_the_bar() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    list.feed_events(&typed(egui::Key::J, "j"));
    let frame = stage.draw(&mut list);
    let barred: Vec<&str> = frame
        .drawn
        .rows
        .iter()
        .filter(|r| r.bar.is_some())
        .map(|r| r.column.as_str())
        .collect();
    assert_eq!(barred, ["avg_occupancy"]);
}

// ---------------------------------------------------------------------------
// AC3: `/` then `inc`, `Enter` in the query, `Esc` in the query.
// ---------------------------------------------------------------------------

#[test]
fn slash_then_inc_lists_median_income_above_a_divider_and_the_other_columns_below_hiding_none() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    search(&mut list, "inc");
    assert_eq!(
        list.query(),
        "inc",
        "the slash that opened the query is not in it"
    );
    assert!(list.querying());
    let frame = stage.draw(&mut list);

    let divider = frame.drawn.divider.expect("a query draws a divider");
    let above: Vec<&str> = frame
        .drawn
        .rows
        .iter()
        .filter(|r| r.rect.bottom() <= divider.top() + 0.01)
        .map(|r| r.column.as_str())
        .collect();
    let below: Vec<&str> = frame
        .drawn
        .rows
        .iter()
        .filter(|r| r.rect.top() >= divider.bottom() - 0.01)
        .map(|r| r.column.as_str())
        .collect();
    assert_eq!(above, ["median_income"]);
    let mut others: Vec<&str> = names();
    others.retain(|n| *n != "median_income");
    assert_eq!(below, others, "the rest keep the table's order");
    assert_eq!(
        above.len() + below.len(),
        names().len(),
        "the query hides no column"
    );
    assert_eq!(texts_in(&frame, frame.drawn.query), ["inc"]);
}

#[test]
fn a_name_that_begins_with_the_letters_leads_one_that_only_contains_them() {
    let mut list = list(ShelfChannel::X);
    search(&mut list, "n");
    let display = list.display();
    // `n` begins no name here; the names that hold it follow the table's order.
    assert_eq!(display.len(), names().len());
    let mut request = request(ShelfChannel::X);
    request.columns = vec![
        ListColumn {
            name: "unit".into(),
            kind: "text".into(),
            moments: None,
        },
        ListColumn {
            name: "name".into(),
            kind: "text".into(),
            moments: None,
        },
        ListColumn {
            name: "price".into(),
            kind: "text".into(),
            moments: None,
        },
    ];
    let mut list = ColumnList::new(request);
    search(&mut list, "n");
    assert_eq!(
        list.display(),
        ["name", "unit", "price"],
        "`name` begins with n and leads `unit`, which only holds it; `price` is the rest"
    );
}

#[test]
fn typing_moves_the_cursor_to_the_best_match_and_reports_it() {
    let mut list = list(ShelfChannel::X);
    let reports = search(&mut list, "inc");
    assert_eq!(reports, [moved("median_income")]);
    assert_eq!(list.cursor(), Some("median_income"));
}

#[test]
fn enter_in_the_query_keeps_the_row_under_the_cursor() {
    let mut list = list(ShelfChannel::X);
    search(&mut list, "inc");
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Enter)]),
        [ListReport::Kept("median_income".to_string())]
    );
}

#[test]
fn esc_in_the_query_clears_it_and_leaves_the_list_open() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    search(&mut list, "inc");
    let reports = list.feed_events(&[key_event(egui::Key::Escape)]);
    assert!(
        !reports.contains(&ListReport::BackedOut),
        "the first Esc steps out of the query, not out of the list: {reports:?}"
    );
    assert_eq!(list.query(), "");
    assert!(!list.querying());
    let frame = stage.draw(&mut list);
    assert_eq!(drawn_names(&frame), names(), "the list is whole again");
    assert!(frame.drawn.divider.is_none());
    assert_eq!(
        list.cursor(),
        Some("population"),
        "the cursor goes back to the column the channel holds"
    );
}

#[test]
fn a_letter_typed_in_the_query_is_text_and_not_a_verb() {
    let mut list = list(ShelfChannel::X);
    // `y` names the y cell while the rows have the keys; here it is a letter.
    let reports = search(&mut list, "y");
    assert_eq!(list.query(), "y");
    assert_eq!(list.channel(), ShelfChannel::X);
    assert!(
        !reports.iter().any(|r| matches!(r, ListReport::GoTo(_))),
        "{reports:?}"
    );
    // `j` moves the cursor on the rows and is a letter in the query.
    let mut list = self::list(ShelfChannel::X);
    search(&mut list, "j");
    assert_eq!(list.query(), "j");
}

#[test]
fn backspace_takes_a_letter_off_and_with_none_left_gives_the_keys_back_to_the_rows() {
    let mut list = list(ShelfChannel::X);
    search(&mut list, "inc");
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(list.query(), "in");
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(list.query(), "");
    assert!(list.querying(), "an empty query still has the keys");
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert!(!list.querying());
}

#[test]
fn the_slash_that_arrives_as_text_alone_opens_the_query_too() {
    // A keyboard whose slash needs a modifier brings the text and no plain key.
    let mut list = list(ShelfChannel::X);
    list.feed_events(&[egui::Event::Text("/".to_string())]);
    assert!(list.querying());
    assert_eq!(list.query(), "");
}

// ---------------------------------------------------------------------------
// AC4: `y` switches the list to y's, and writes nothing into the query.
// ---------------------------------------------------------------------------

#[test]
fn with_the_query_empty_y_switches_the_list_to_ys_and_nothing_is_written_into_the_query() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let reports = list.feed_events(&typed(egui::Key::Y, "y"));
    assert_eq!(reports, [ListReport::GoTo(ShelfChannel::Y)]);
    assert_eq!(list.channel(), ShelfChannel::Y);
    assert_eq!(
        list.query(),
        "",
        "the letter that named the channel is not typed"
    );
    assert!(!list.querying());
    assert_eq!(
        list.cursor(),
        Some("latitude"),
        "the cursor is on the column y holds"
    );
    let frame = stage.draw(&mut list);
    assert_eq!(frame.drawn.heading_text, "OUTLINE   ·   y axis of hero");
    assert_eq!(drawn_names(&frame), names());
    assert_eq!(texts_in(&frame, frame.drawn.query), [QUERY_PLACEHOLDER]);
}

#[test]
fn the_channel_letters_name_their_channel_and_the_one_the_list_is_on_names_nothing() {
    let mut list = list(ShelfChannel::X);
    assert_eq!(list.feed_events(&typed(egui::Key::X, "x")), []);
    assert_eq!(
        list.feed_events(&typed(egui::Key::C, "c")),
        [ListReport::GoTo(ShelfChannel::Colour)]
    );
    assert_eq!(list.channel(), ShelfChannel::Colour);
    assert_eq!(list.cursor(), Some("median_income"));
    // The mark has a list of marks and no columns, which is not this list's:
    // the report asks for it and the list stays where it is.
    assert_eq!(
        list.feed_events(&typed(egui::Key::M, "m")),
        [ListReport::GoTo(ShelfChannel::Mark)]
    );
    assert_eq!(list.channel(), ShelfChannel::Colour);
}

// ---------------------------------------------------------------------------
// AC5: `Enter`, `Esc`, `h` and `l`.
// ---------------------------------------------------------------------------

#[test]
fn enter_on_a_row_reports_the_column_kept() {
    let mut list = list(ShelfChannel::X);
    list.feed_events(&typed(egui::Key::J, "j"));
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Enter)]),
        [ListReport::Kept("avg_occupancy".to_string())]
    );
}

#[test]
fn enter_with_the_cursor_on_no_row_keeps_nothing() {
    let mut bare = request(ShelfChannel::Colour);
    bare.channels.colour = Binding::Unset;
    let mut list = ColumnList::new(bare);
    assert_eq!(list.feed_events(&[key_event(egui::Key::Enter)]), []);
}

#[test]
fn esc_with_the_query_empty_reports_backing_out() {
    let mut list = list(ShelfChannel::X);
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Escape)]),
        [ListReport::BackedOut]
    );
}

#[test]
fn h_and_l_report_the_channel_beside_with_the_list_kept_open() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    assert_eq!(
        list.feed_events(&typed(egui::Key::L, "l")),
        [ListReport::Beside(ShelfChannel::Y)]
    );
    assert_eq!(list.channel(), ShelfChannel::Y, "the list moved onto it");
    assert_eq!(
        list.feed_events(&typed(egui::Key::L, "l")),
        [ListReport::Beside(ShelfChannel::Colour)]
    );
    assert_eq!(
        list.feed_events(&typed(egui::Key::L, "l")),
        [],
        "colour is the last cell; the key stops there"
    );
    assert_eq!(
        list.feed_events(&typed(egui::Key::H, "h")),
        [ListReport::Beside(ShelfChannel::Y)]
    );
    assert_eq!(
        list.query(),
        "",
        "a letter that moved the list is not typed"
    );
    let frame = stage.draw(&mut list);
    assert_eq!(frame.drawn.heading_text, "OUTLINE   ·   y axis of hero");
    assert_eq!(drawn_names(&frame), names(), "the list is open, whole");
    // The arrows are `h` and `l`'s twins.
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::ArrowLeft)]),
        [ListReport::Beside(ShelfChannel::X)]
    );
}

#[test]
fn h_from_x_names_the_mark_beside_and_the_list_stays_on_x() {
    let mut list = list(ShelfChannel::X);
    assert_eq!(
        list.feed_events(&typed(egui::Key::H, "h")),
        [ListReport::Beside(ShelfChannel::Mark)]
    );
    assert_eq!(list.channel(), ShelfChannel::X);
}

#[test]
fn a_key_with_a_modifier_held_is_not_the_lists() {
    let mut list = list(ShelfChannel::X);
    let ctrl_j = egui::Event::Key {
        key: egui::Key::J,
        physical_key: Some(egui::Key::J),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    assert_eq!(list.feed_events(&[ctrl_j]), []);
    assert_eq!(list.cursor(), Some("population"));
}

// ---------------------------------------------------------------------------
// AC6: the bar in the channel's hue, and the foot's keys.
// ---------------------------------------------------------------------------

#[test]
fn the_row_under_the_cursor_carries_a_three_point_bar_in_the_channels_hue() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        for (channel, slot, held) in [
            (ShelfChannel::X, TEAL, "population"),
            (ShelfChannel::Y, VIOLET, "latitude"),
            (ShelfChannel::Colour, ORANGE, "median_income"),
        ] {
            let mut list = list(channel);
            let frame = stage.draw(&mut list);
            let row = frame
                .drawn
                .rows
                .iter()
                .find(|r| r.column == held)
                .expect("the held column is listed");
            let bar = row.bar.expect("the cursor's row has a bar");
            assert!(
                near(bar.width(), 3.0),
                "{channel:?} {mode:?}: {}",
                bar.width()
            );
            assert!(
                near(bar.height(), row.rect.height()),
                "{channel:?} {mode:?}"
            );
            assert!(near(bar.left(), row.rect.left()), "{channel:?} {mode:?}");
            let hue = categorical(mode, slot);
            // The strip's bar under the open tab is in the channel's hue as
            // well, and belongs to the strip: it is the one fill in the hue
            // that is not the row's.
            let strip_bar = frame
                .drawn
                .tabs
                .as_ref()
                .and_then(|strip| strip.tabs.iter().find_map(|tab| tab.bar));
            let painted: Vec<egui::Rect> = fills(&frame)
                .into_iter()
                .filter(|(rect, fill)| *fill == hue && Some(*rect) != strip_bar)
                .map(|(rect, _)| rect)
                .collect();
            assert_eq!(
                painted.len(),
                1,
                "{channel:?} {mode:?}: one fill in {hue:?} but the strip's, the bar"
            );
            assert!(
                near(painted[0].width(), 3.0) && near(painted[0].left(), row.rect.left()),
                "{channel:?} {mode:?}: {:?}",
                painted[0]
            );
        }
    }
}

#[test]
fn the_foot_prints_slash_while_the_rows_have_the_keys_and_not_while_the_query_has_them() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);

    let rows = stage.draw(&mut list);
    let on_rows = texts_in(&rows, rows.drawn.foot);
    assert!(
        on_rows.contains(&"/"),
        "the foot prints / on the rows: {on_rows:?}"
    );
    assert!(on_rows.contains(&"Enter"), "{on_rows:?}");
    assert!(on_rows.contains(&"Esc"), "{on_rows:?}");

    list.feed_events(&slash());
    let query = stage.draw(&mut list);
    let on_query = texts_in(&query, query.drawn.foot);
    assert!(
        !on_query.contains(&"/"),
        "the foot does not print / while the query has the keys: {on_query:?}"
    );
    assert!(on_query.contains(&"Enter"), "{on_query:?}");
    assert!(on_query.contains(&"Esc"), "{on_query:?}");

    // And back: Esc gives the keys to the rows, and the foot says so.
    list.feed_events(&[key_event(egui::Key::Escape)]);
    let again = stage.draw(&mut list);
    assert!(texts_in(&again, again.drawn.foot).contains(&"/"));
}

#[test]
fn the_foot_stands_under_the_last_row_and_inside_the_section() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let frame = stage.draw(&mut list);
    let last = frame.drawn.rows.last().expect("rows");
    assert!(frame.drawn.foot.top() >= last.rect.bottom() - 0.01);
    assert!(frame.drawn.rect.contains_rect(frame.drawn.foot));
    assert!(frame.drawn.rect.contains_rect(frame.drawn.heading));
}

#[test]
fn no_two_texts_in_the_list_are_drawn_into_one_place() {
    for mode in [Mode::Light, Mode::Dark] {
        let stage = Stage::new(mode);
        for events in [Vec::new(), slash()] {
            let mut list = list(ShelfChannel::X);
            list.feed_events(&events);
            let frame = stage.draw(&mut list);
            assert_eq!(
                frame.collisions,
                None,
                "{mode:?}, querying={}",
                list.querying()
            );
        }
        let mut list = list(ShelfChannel::X);
        search(&mut list, "inc");
        assert_eq!(
            stage.draw(&mut list).collisions,
            None,
            "{mode:?}, with inc typed"
        );
    }
}

// ---------------------------------------------------------------------------
// A click on a row.
// ---------------------------------------------------------------------------

/// The centre of `column`'s row, as `list` draws it on `stage`.
fn row_centre(stage: &Stage, list: &mut ColumnList, column: &str) -> egui::Pos2 {
    stage
        .draw(list)
        .drawn
        .rows
        .iter()
        .find(|r| r.column == column)
        .unwrap_or_else(|| panic!("{column} is listed"))
        .rect
        .center()
}

fn press_at(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// **A click on a row keeps its column.** The cursor lands on it and the list
/// reports it moved there and kept, in that order, so the window draws it and
/// keeps it as `Enter` would.
#[test]
fn a_click_on_a_row_moves_the_cursor_there_and_keeps_the_column() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let target = row_centre(&stage, &mut list, "house_age");
    stage.frame(&mut list, vec![egui::Event::PointerMoved(target)]);
    stage.frame(&mut list, vec![press_at(target, true)]);
    let released = stage.frame(&mut list, vec![press_at(target, false)]);
    assert_eq!(
        released.drawn.reports,
        [
            moved("house_age"),
            ListReport::Kept("house_age".to_string())
        ]
    );
    assert_eq!(list.cursor(), Some("house_age"));
    assert_eq!(
        stage.draw(&mut list).drawn.reports,
        [],
        "a frame with no click reports none"
    );
}

/// **The pointer moving over a row moves the cursor to it**, and the list
/// reports the column as moved to — the preview a key's move makes — and not
/// kept. A pointer resting on a row does not take the cursor back from the
/// keys: `j` moves it off the row the pointer is on and it stays there.
#[test]
fn the_pointer_moving_over_a_row_moves_the_cursor_and_a_resting_pointer_does_not() {
    let stage = Stage::new(Mode::Light);
    let mut list = list(ShelfChannel::X);
    let target = row_centre(&stage, &mut list, "house_age");
    stage.frame(
        &mut list,
        vec![egui::Event::PointerMoved(target - egui::vec2(0.0, 2.0))],
    );
    let over = stage.frame(&mut list, vec![egui::Event::PointerMoved(target)]);
    assert_eq!(over.drawn.reports, [moved("house_age")]);
    assert_eq!(list.cursor(), Some("house_age"));

    let after_j = list.feed_events(&typed(egui::Key::J, "j"));
    assert_eq!(after_j.len(), 1, "`j` moves the cursor one row");
    let moved_to = list.cursor().map(str::to_owned);
    assert_ne!(moved_to.as_deref(), Some("house_age"));
    let resting = stage.draw(&mut list);
    assert_eq!(
        resting.drawn.reports,
        [],
        "a pointer standing on a row does not move the cursor"
    );
    assert_eq!(list.cursor(), moved_to.as_deref());
}

// ---------------------------------------------------------------------------
// The Outline pane: the list takes the columns' place, and gives it back.
// ---------------------------------------------------------------------------

fn doc() -> ProtocolDoc {
    let inputs = open()
        .protocol
        .inputs()
        .expect("the opened file's protocol");
    ProtocolDoc::headless(ProtocolModel::new(inputs, Flow::Vertical))
}

/// Draw the Outline pane once, as the window does, into a context sized like the
/// rail.
fn run_pane(doc: &mut ProtocolDoc, ctx: &egui::Context, mode: Mode) {
    let registry = protocol_registry();
    let mut items = registry.instantiate();
    let key = registry.pane_key(OUTLINE);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(WIDTH, 600.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut requests = Vec::new();
            let mut cx = ItemCtx::new(
                mode,
                key,
                egui_tiles::TileId::from_u64(1),
                true,
                &mut requests,
            );
            items
                .get_mut(&key)
                .expect("the registry holds the Outline")
                .ui(doc, ui, &mut cx);
        });
    });
}

fn settled_pane(doc: &mut ProtocolDoc) -> egui::Context {
    let ctx = egui::Context::default();
    design::apply(&ctx, Mode::Light);
    run_pane(doc, &ctx, Mode::Light);
    run_pane(doc, &ctx, Mode::Light);
    ctx
}

#[test]
fn the_outline_pane_draws_the_list_in_place_of_the_columns_while_a_shelf_cell_is_active() {
    let mut doc = doc();
    let ctx = settled_pane(&mut doc);
    doc.model
        .open_column_list("hero", ShelfChannel::X, channels());
    run_pane(&mut doc, &ctx, Mode::Light);

    let captions: Vec<&str> = doc
        .spine_drawn
        .iter()
        .filter(|r| r.role == brightfield_shell::protocol::SpineRole::Caption)
        .map(|r| r.label.as_str())
        .collect();
    assert!(
        captions.contains(&"OUTLINE   ·   x axis of hero"),
        "the section is headed with the channel and the tile: {captions:?}"
    );
    assert!(
        !captions.iter().any(|c| c.ends_with("9 columns")),
        "the plain caption is gone while the list is open: {captions:?}"
    );
    let columns: Vec<(&str, bool)> = doc
        .spine_drawn
        .iter()
        .filter(|r| r.role == brightfield_shell::protocol::SpineRole::Column)
        .map(|r| (r.label.as_str(), r.washed))
        .collect();
    let listed: Vec<&str> = columns.iter().map(|(n, _)| *n).collect();
    assert_eq!(listed, names());
    let under_cursor: Vec<&str> = columns
        .iter()
        .filter(|(_, w)| *w)
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(under_cursor, ["population"]);
}

#[test]
fn the_outline_pane_feeds_the_list_the_windows_events_and_takes_back_its_reports() {
    let mut doc = doc();
    let ctx = settled_pane(&mut doc);
    assert_eq!(doc.model.feed_column_list(&typed(egui::Key::J, "j")), []);
    doc.model
        .open_column_list("hero", ShelfChannel::X, channels());
    run_pane(&mut doc, &ctx, Mode::Light);
    assert_eq!(
        doc.model.feed_column_list(&typed(egui::Key::J, "j")),
        [moved("avg_occupancy")]
    );
    run_pane(&mut doc, &ctx, Mode::Light);
    let washed: Vec<&str> = doc
        .spine_drawn
        .iter()
        .filter(|r| r.washed && r.role == brightfield_shell::protocol::SpineRole::Column)
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(
        washed,
        ["avg_occupancy"],
        "the pane draws where the keys put the cursor"
    );
}

#[test]
fn with_no_shelf_cell_active_the_outline_draws_as_it_does_today() {
    let mut doc = doc();
    let ctx = settled_pane(&mut doc);
    let today = doc.spine_drawn.clone();
    assert!(
        today.iter().any(|r| r.label.ends_with("9 columns")),
        "the plain caption heads the columns: {:?}",
        today.iter().map(|r| r.label.as_str()).collect::<Vec<_>>()
    );
    let rows: Vec<&str> = today
        .iter()
        .filter(|r| r.role == brightfield_shell::protocol::SpineRole::Column)
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(rows, names());
    assert!(
        today
            .iter()
            .all(|r| !r.washed || r.role != brightfield_shell::protocol::SpineRole::Column),
        "no column row is washed: nothing has the cursor"
    );

    // Opened and closed again, the pane is what it was.
    doc.model
        .open_column_list("hero", ShelfChannel::X, channels());
    run_pane(&mut doc, &ctx, Mode::Light);
    assert_ne!(doc.spine_drawn, today, "the open list changes the pane");
    doc.model.close_column_list();
    assert!(doc.model.column_list().is_none());
    run_pane(&mut doc, &ctx, Mode::Light);
    assert_eq!(doc.spine_drawn, today);
}

// ---------------------------------------------------------------------------
// The baselines.
// ---------------------------------------------------------------------------

/// `kittest.toml`'s thresholds, unloosened.
fn options() -> SnapshotOptions {
    SnapshotOptions::default()
}

/// Draw the list through the wgpu renderer and compare it with the committed
/// baseline `name`.
fn baseline(name: &str, mode: Mode, word: Option<&'static str>) {
    baseline_of(name, mode, list(ShelfChannel::X), word);
}

/// [`baseline`] over `list`, which a test opens on the columns it names.
fn baseline_of(name: &str, mode: Mode, mut list: ColumnList, word: Option<&'static str>) {
    if let Some(word) = word {
        list.feed_events(&slash());
        for ch in word.chars() {
            list.feed_events(&[egui::Event::Text(ch.to_string())]);
        }
    }
    let size = egui::vec2(WIDTH, 420.0);
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                |ui| {
                    list.show(ui, mode);
                },
            );
        });
    harness.run();
    harness.snapshot_options(name, &options());
}

#[test]
fn the_list_at_rest_light_matches_its_baseline() {
    baseline("outline_list_rest_light", Mode::Light, None);
}

#[test]
fn the_list_at_rest_dark_matches_its_baseline() {
    baseline("outline_list_rest_dark", Mode::Dark, None);
}

#[test]
fn the_list_with_inc_typed_light_matches_its_baseline() {
    baseline("outline_list_inc_light", Mode::Light, Some("inc"));
}

#[test]
fn the_list_with_inc_typed_dark_matches_its_baseline() {
    baseline("outline_list_inc_dark", Mode::Dark, Some("inc"));
}

#[test]
fn the_list_with_one_numeric_row_among_others_light_matches_its_baseline() {
    baseline_of("outline_list_mixed_light", Mode::Light, mixed_list(), None);
}

#[test]
fn the_list_with_one_numeric_row_among_others_dark_matches_its_baseline() {
    baseline_of("outline_list_mixed_dark", Mode::Dark, mixed_list(), None);
}

/// The Outline's list turned to x's settings, with a title and a scale the file
/// sets and a format it leaves to the axis, so the list draws a row of each
/// state.
fn settings_list() -> ColumnList {
    let mut list = ColumnList::new(request(ShelfChannel::X));
    list.set_settings(settings_of("xLabel: Residents\nxScale: log", &channels()));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    list
}

#[test]
fn the_settings_list_light_matches_its_baseline() {
    baseline_of(
        "outline_list_settings_light",
        Mode::Light,
        settings_list(),
        None,
    );
}

#[test]
fn the_settings_list_dark_matches_its_baseline() {
    baseline_of(
        "outline_list_settings_dark",
        Mode::Dark,
        settings_list(),
        None,
    );
}
