//! **With a typed row's field open on the settings list, the pointer leaves the
//! cursor, the field and the value where they are.**
//!
//! A field is drawn on the row under the cursor, so a pointer that moved the
//! cursor to another row would hide the field while its text still took keys, and
//! `Enter` would keep a title the analyst could not see. The list is drawn here
//! under the events a mouse brings, a movement a frame and a click as a press
//! frame and a release frame, and each assertion reads what the frame drew or
//! what the list holds after it.
//!
//! Each refusal has its control: the same movement or click with no field open
//! does what it did before, so a pointer the harness never delivered cannot read
//! as one the list ignored.

use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListDrawn, ListReport,
    ListTab, SettingRowDrawn, ShelfChannels, FORMAT_ROW, SCALE_ROW, TITLE_ROW,
};
use brightfield_shell::text_ink;
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;

const WIDTH: f32 = 240.0;
const ORIGIN: egui::Pos2 = egui::pos2(12.0, 9.0);

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

const SOURCE: &str = "data:\n  t:\n    - { a: 1 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: a\nwidth: 600\nheight: 300\n";

/// x's settings list turned to its settings, the cursor on the title row, which
/// is the first.
fn list_on_the_settings() -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels: channels(),
        columns: vec![ListColumn {
            name: "population".to_string(),
            kind: "BIGINT".to_string(),
            moments: None,
        }],
    });
    let spec = parse_spec(SOURCE, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    list.set_settings(ChannelSettings::of_plot(&spec, plot, &channels()));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    list
}

/// What a stage's frame drew, with the words printed in the foot.
struct Frame {
    drawn: ListDrawn,
    foot_words: Vec<String>,
}

/// A list drawn into a `Ui`, frame by frame, under the events a test brings.
struct Stage {
    ctx: egui::Context,
    mode: Mode,
}

impl Stage {
    /// The theme applied and the faces loaded: the fonts `apply` installs take
    /// effect on the pass after it, so two frames run before anything is measured.
    fn new() -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, Mode::Light);
        let stage = Self {
            ctx,
            mode: Mode::Light,
        };
        let mut warm = list_on_the_settings();
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
        let _ = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(ORIGIN, egui::vec2(WIDTH, 560.0))),
                |ui| drawn = Some(list.show(ui, self.mode)),
            );
            texts = text_ink::frame_text(ui.ctx());
        });
        let drawn = drawn.expect("the list drew");
        let foot_words = texts
            .iter()
            .filter(|t| drawn.foot.contains(t.ink.center()))
            .map(|t| t.text.clone())
            .collect();
        Frame { drawn, foot_words }
    }

    /// A frame with nothing new, so the rects the last frame drew are the ones
    /// the next pointer is tested against.
    fn settle(&self, list: &mut ColumnList) -> Frame {
        self.frame(list, Vec::new())
    }

    /// The pointer placed at `at` and a frame after it. It moves nothing a frame
    /// compares against, since the first position has no earlier one.
    fn rest(&self, list: &mut ColumnList, at: egui::Pos2) -> Frame {
        self.frame(list, vec![egui::Event::PointerMoved(at)]);
        self.settle(list)
    }

    /// The pointer moved to `at`, as a mouse nudged over another row moves it:
    /// the frame it moves in, then the frame after.
    fn glide(&self, list: &mut ColumnList, at: egui::Pos2) -> Frame {
        self.frame(list, vec![egui::Event::PointerMoved(at)]);
        self.settle(list)
    }

    /// A click at `at`: the pointer to it, a settled frame, a press, a release.
    /// The release's frame, and the reports the four frames answered with.
    fn click(&self, list: &mut ColumnList, at: egui::Pos2) -> (Frame, Vec<ListReport>) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut reports = Vec::new();
        reports.extend(
            self.frame(list, vec![egui::Event::PointerMoved(at)])
                .drawn
                .reports,
        );
        reports.extend(self.settle(list).drawn.reports);
        reports.extend(self.frame(list, vec![button(true)]).drawn.reports);
        let last = self.frame(list, vec![button(false)]);
        reports.extend(last.drawn.reports.clone());
        (last, reports)
    }
}

fn row<'a>(frame: &'a Frame, name: &str) -> &'a SettingRowDrawn {
    frame
        .drawn
        .settings
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| {
            let drawn: Vec<_> = frame.drawn.settings.iter().map(|r| r.name).collect();
            panic!("the list drew no {name} row, only {drawn:?}")
        })
}

/// A point on `name`'s row where the name's ink stands: a click here is on the
/// row and outside the zone that steps a value.
fn on_the_name(frame: &Frame, name: &str) -> egui::Pos2 {
    row(frame, name).name_rect.center()
}

/// A point inside the zone of `name`'s row that a click steps.
fn on_the_value(frame: &Frame, name: &str) -> egui::Pos2 {
    row(frame, name).value_zone.center()
}

/// The words the foot prints while a typed row's field is open.
fn the_field_foot(frame: &Frame) -> bool {
    ["Enter", "keep", "Esc", "drop"]
        .iter()
        .all(|w| frame.foot_words.iter().any(|t| t == w))
}

/// The list with the title field open and a frame drawn over it, the pointer
/// resting on the title row.
fn open_on_the_title(stage: &Stage) -> (ColumnList, Frame) {
    let mut list = list_on_the_settings();
    let first = stage.settle(&mut list);
    let rest_at = on_the_name(&first, TITLE_ROW);
    list.feed_events(&[key_event(egui::Key::Enter)]);
    assert!(list.field().is_some(), "Enter opened the title field");
    let frame = stage.rest(&mut list, rest_at);
    assert!(
        row(&frame, TITLE_ROW).field.is_some(),
        "the title row draws its field"
    );
    assert!(the_field_foot(&frame), "the foot prints the field's keys");
    (list, frame)
}

// ---------------------------------------------------------------------------
// The pointer moves.
// ---------------------------------------------------------------------------

/// **Control.** With no field open, the same movement moves the cursor to the row
/// the pointer is over.
#[test]
fn a_pointer_moved_over_another_row_moves_the_cursor_when_no_field_is_open() {
    let stage = Stage::new();
    let mut list = list_on_the_settings();
    let first = stage.settle(&mut list);
    stage.rest(&mut list, on_the_name(&first, TITLE_ROW));
    let frame = stage.glide(&mut list, on_the_name(&first, FORMAT_ROW));
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(FORMAT_ROW),
        "the pointer moved the cursor to the format row"
    );
    assert!(row(&frame, FORMAT_ROW).bar.is_some());
}

/// **AC1.** With the title field open, a pointer moved over another row leaves
/// the cursor and the field on the title row, the foot prints the field's keys,
/// and a character typed afterwards lands in the field.
#[test]
fn a_pointer_moved_over_another_row_leaves_the_cursor_and_the_open_field_where_they_are() {
    let stage = Stage::new();
    let (mut list, frame) = open_on_the_title(&stage);

    for over in [FORMAT_ROW, SCALE_ROW] {
        let at = on_the_name(&frame, over);
        let after = stage.glide(&mut list, at);
        assert_eq!(
            list.setting_cursor().map(|r| r.name),
            Some(TITLE_ROW),
            "a pointer over the {over} row left the cursor on the title row"
        );
        assert!(
            row(&after, TITLE_ROW).field.is_some(),
            "the title row still draws its field after the pointer moved over {over}"
        );
        assert!(
            row(&after, over).bar.is_none(),
            "the {over} row drew no cursor bar"
        );
        assert!(
            the_field_foot(&after),
            "the foot still reads Enter keep and Esc drop, saw {:?}",
            after.foot_words
        );
    }

    list.feed_events(&[egui::Event::Text("Q".to_string())]);
    let typed = stage.settle(&mut list);
    assert_eq!(
        list.field().map(|f| f.text.as_str()),
        Some("Q"),
        "a character typed afterwards lands in the field"
    );
    assert!(
        row(&typed, TITLE_ROW).field.is_some(),
        "and the field it landed in is drawn"
    );
}

// ---------------------------------------------------------------------------
// The pointer clicks.
// ---------------------------------------------------------------------------

/// **Control.** With no field open, a click on the value of a row that steps
/// moves the cursor to it and steps the value.
#[test]
fn a_click_on_the_value_of_a_row_that_steps_steps_it_when_no_field_is_open() {
    let stage = Stage::new();
    let mut list = list_on_the_settings();
    let first = stage.settle(&mut list);
    let (_, reports) = stage.click(&mut list, on_the_value(&first, SCALE_ROW));
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCALE_ROW));
    assert!(
        reports
            .iter()
            .any(|r| matches!(r, ListReport::Set(e) if e.row == SCALE_ROW)),
        "the click stepped the scale row, answered with {reports:?}"
    );
}

/// **AC2, the row.** With the title field open, a click on another row, on its
/// name, moves nothing: the cursor and the field stay on the title row.
#[test]
fn a_click_on_another_row_with_a_field_open_leaves_the_cursor_and_the_field() {
    let stage = Stage::new();
    let (mut list, frame) = open_on_the_title(&stage);

    for over in [FORMAT_ROW, SCALE_ROW] {
        let (after, reports) = stage.click(&mut list, on_the_name(&frame, over));
        assert_eq!(
            list.setting_cursor().map(|r| r.name),
            Some(TITLE_ROW),
            "a click on the {over} row left the cursor on the title row"
        );
        assert!(
            row(&after, TITLE_ROW).field.is_some(),
            "the title row still draws its field after a click on {over}"
        );
        assert!(
            list.field().is_some(),
            "the field is still open after a click on {over}"
        );
        assert_eq!(reports, [], "a click on the {over} row reported nothing");
    }
}

/// **AC2, the value.** With the title field open, a click on the value of a row
/// that steps steps nothing and moves nothing.
#[test]
fn a_click_on_the_value_of_a_row_that_steps_with_a_field_open_steps_nothing() {
    let stage = Stage::new();
    let (mut list, frame) = open_on_the_title(&stage);
    assert!(
        list.settings()
            .iter()
            .any(|r| r.name == SCALE_ROW && r.steps()),
        "the scale row is one that steps"
    );

    let (after, reports) = stage.click(&mut list, on_the_value(&frame, SCALE_ROW));
    assert_eq!(
        reports,
        [],
        "a click on the scale row's value reported a step"
    );
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(TITLE_ROW),
        "and left the cursor on the title row"
    );
    assert!(
        row(&after, TITLE_ROW).field.is_some() && list.field().is_some(),
        "and left the field open"
    );
}
