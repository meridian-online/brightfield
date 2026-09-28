//! **A tile's scale or normalise switch marks the window title unsaved.**
//!
//! A switch rewrites the live spec and rebuilds the page from it, and the
//! file on disk keeps the spec it had. The window says so by appending
//! [`UNSAVED_MARK`] to [`MeridianApp::title`], and every assertion here reads
//! that title: the sentence a reader sees, rather than the flag behind it.
//!
//! # What is covered and what is not
//!
//! Covered: the two switches marking the title, through the real gesture and
//! through the document's own entry point; a refused switch leaving it
//! unmarked, on both refusals that leave the previous page standing; a file
//! opened, a brush swept, a focus moved and a pick of the state already showing
//! leaving it unmarked; and a second file opened after an edit starting clean.
//! Not covered: Save clearing the mark, because Save does not write the chart
//! yet — the mark stays until the document is replaced.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{GridLayout, CHART};
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::{PlotAxis, ScaleType, StackOffset};
use brightfield_workbench::PaneKey;

/// The committed table the window is opened over — the fixture
/// `tests/tile_scale_switch.rs` uses.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

fn housing_boot() -> Boot {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// The authored grouped histogram, the only shape that draws a normalise
/// control — see `tests/tile_normalise_switch.rs`.
fn grouped_boot() -> Boot {
    let spec = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-grouped-shares.yaml");
    Boot::open(
        spec.to_str().expect("utf-8 example path"),
        Flow::Vertical,
        None,
    )
    .expect("the example opens")
}

/// A scratch directory for a fixture that has to be written, unique to this
/// process and this call.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!(
        "bf-chart-save-mark-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
    dir
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a *previous* frame registered —
/// the harness `tests/tile_scale_switch.rs` uses, over this file's assertions.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    fn open(boot: Boot) -> Self {
        let size = boot.window_size();
        Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        }
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    /// Press and release the primary button over `pos`, as five frames.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Put the grid in its columns layout, the one where each histogram tile
    /// is on screen with its switch — untransposed the tiles are composed
    /// outside the hero pane's clip and a click has nothing to land on. See
    /// `Live::transpose` in `tests/tile_scale_switch.rs`.
    fn transpose(&mut self) {
        let at = self
            .app
            .chart_doc()
            .grid_layout_switch
            .as_ref()
            .expect("the grid pane's header band drew a layout switch")
            .states
            .iter()
            .find(|(state, _)| *state == GridLayout::Columns)
            .expect("the switch offers a columns state")
            .1
            .center();
        self.click(at);
        self.settle();
        assert_eq!(
            self.app.grid_layout(),
            GridLayout::Columns,
            "the click at {at:?} did not throw the switch"
        );
    }

    /// Throw the scale switch on `column`'s tile to `kind`.
    fn switch_to(&mut self, column: &str, kind: ScaleType) {
        let drawn = self.app.chart_doc().scale_switches.clone();
        let switch = drawn
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"));
        let at = switch
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
    }

    /// Throw the normalise control to `offset`.
    fn normalise_to(&mut self, offset: StackOffset) {
        let control = self
            .app
            .chart_doc()
            .normalise_switches
            .first()
            .cloned()
            .expect("the grouped plot draws a normalise control");
        let at = control
            .states
            .iter()
            .find(|(state, _)| *state == offset)
            .unwrap_or_else(|| panic!("the control offers no {offset:?}"))
            .1
            .center();
        self.click(at);
    }

    /// Sweep a brush across plot `plot` and release.
    fn brush(&mut self, plot: usize, from: f32, to: f32) {
        let rect = self.app.composed_plot_rects()[plot];
        let at = |f: f32| egui::pos2(rect.left() + rect.width() * f, rect.center().y);
        let (start, end) = (at(from), at(to));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerMoved(end), button(end, false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Open the housing file again in the same window, as the front door's
    /// picker does.
    fn open_again(&mut self) {
        let path = housing();
        let ctx = self.ctx.clone();
        self.app
            .open_data_file(&ctx, path.to_str().expect("utf-8 fixture path"));
        self.settle();
    }

    /// The plot the tile for `column` draws on.
    fn plot_of(&self, column: &str) -> usize {
        self.app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .plot
    }
}

/// What a marked title looks like beside its unmarked self.
fn marked(title: &str) -> String {
    format!("{title} {UNSAVED_MARK}")
}

/// The unmarked title of a freshly opened window, asserted unmarked so the
/// comparison after a switch is against a title that could have been marked.
fn clean_title(live: &Live) -> String {
    let title = live.app.title();
    assert!(
        !UNSAVED_MARK.is_empty() && !title.contains(UNSAVED_MARK),
        "a window with no edit is titled {title:?}, which already carries the mark"
    );
    title
}

/// **AC1, through the gesture.** Throwing a tile's scale switch marks the
/// title, and the mark is the constant after the subject the title had.
#[test]
fn throwing_a_tiles_scale_switch_marks_the_title() {
    let mut live = Live::open(housing_boot());
    live.settle();
    live.transpose();
    let before = clean_title(&live);

    live.switch_to("population", ScaleType::Log);

    assert_eq!(
        live.app.title(),
        marked(&before),
        "the switch changed the chart and the title says nothing"
    );
}

/// **AC1, on the y axis.** The generated tiles bin their x axis, so no click
/// reaches a `yScale` write; the document's own entry point is the one the
/// control calls, and it takes the axis.
#[test]
fn a_y_scale_write_marks_the_title() {
    let mut live = Live::open(housing_boot());
    live.settle();
    let before = clean_title(&live);
    let plot = live.plot_of("population");

    let thrown = live
        .app
        .chart_doc_mut()
        .set_plot_scale(plot, PlotAxis::Y, ScaleType::Log);

    assert!(
        thrown,
        "the y scale write was refused: {:?}",
        live.app.chart_doc().chart_fault()
    );
    assert_eq!(live.app.title(), marked(&before));
}

/// **AC2.** Throwing the normalise control marks the title the same way. The
/// fixture is authored with the offset on, so a click on the reading already
/// showing leaves the title alone and the click that turns it off marks it.
#[test]
fn throwing_the_normalise_control_marks_the_title() {
    let mut live = Live::open(grouped_boot());
    live.settle();
    let before = clean_title(&live);

    live.normalise_to(StackOffset::Normalize);
    assert_eq!(
        live.app.title(),
        before,
        "a pick of the reading already showing marked the title"
    );

    live.normalise_to(StackOffset::None);

    assert_eq!(
        live.app.title(),
        marked(&before),
        "the normalise control changed the chart and the title says nothing"
    );
}

/// **AC3, the two refusals that leave the previous page standing.** A plot the
/// page does not have, and a rebuild the engine refuses: the second is the one
/// where the spec edit was accepted and changed the spec, so the mark has to
/// wait for the rebuild rather than follow the edit.
#[test]
fn a_switch_the_chart_refuses_leaves_the_title_unmarked() {
    // A plot index off the end of the page.
    let mut live = Live::open(housing_boot());
    live.settle();
    let before = clean_title(&live);
    let off_the_end = live.app.chart_doc().composed.plots.len();
    assert!(!live
        .app
        .chart_doc_mut()
        .set_plot_scale(off_the_end, PlotAxis::X, ScaleType::Log));
    assert_eq!(live.app.title(), before, "a plot that is not there marked it");

    // An authored spec whose data file is gone by the time the switch rebuilds.
    let dir = scratch_dir("engine-refusal");
    let data = dir.join("readings.csv");
    std::fs::write(&data, "v\n1\n2\n5\n9\n20\n40\n90\n200\n400\n900\n")
        .expect("the fixture writes");
    let spec = dir.join("readings.yaml");
    std::fs::write(
        &spec,
        "data:\n  rows:\n    file: readings.csv\nplot:\n  - mark: rectY\n    \
         data: { from: rows }\n    x: { bin: v }\n    y: { count: }\n    \
         fill: steelblue\nwidth: 640\nheight: 400\n",
    )
    .expect("the spec writes");
    let boot = Boot::open(
        spec.to_str().expect("utf-8 temp path"),
        Flow::Vertical,
        None,
    )
    .unwrap_or_else(|e| panic!("open {}: {e}", spec.display()));
    let mut live = Live::open(boot);
    live.settle();
    let before = clean_title(&live);

    std::fs::remove_file(&data).expect("the data file goes");
    let thrown = live
        .app
        .chart_doc_mut()
        .set_plot_scale(0, PlotAxis::X, ScaleType::Log);

    assert!(
        !thrown,
        "the rebuild over a missing file was not refused, so this test refuses nothing"
    );
    assert!(
        live.app.chart_doc().chart_fault().is_some(),
        "the refusal left no fault to show"
    );
    assert_eq!(
        live.app.title(),
        before,
        "the engine refused the switch and the title says the chart changed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **AC4.** A file opened with no chart change, a brush swept on a tile, and
/// focus moved between panes each leave the title unmarked.
#[test]
fn opening_brushing_and_moving_focus_leave_the_title_unmarked() {
    let mut live = Live::open(housing_boot());
    live.settle();
    live.transpose();
    let before = clean_title(&live);

    let plot = live.plot_of("population");
    live.brush(plot, 0.2, 0.6);
    live.settle();
    assert_eq!(live.app.title(), before, "a brush marked the title");

    assert!(live.app.focus_pane(PaneKey::new(EDITOR)));
    live.settle();
    assert!(live.app.focus_pane(PaneKey::new(CHART)));
    live.settle();
    assert_eq!(live.app.title(), before, "a move of focus marked the title");
}

/// **A write of the value the spec already holds is not an edit.** The switch
/// accepts the pick and rebuilds the page over the same spec, and a title that
/// marked on it would tell a reader they had something to lose when they had
/// not. The same entry point with a value the spec does not hold marks, so the
/// first half cannot pass by the mark never being set.
#[test]
fn writing_the_value_the_spec_already_holds_leaves_the_title_unmarked() {
    let dir = scratch_dir("same-value");
    std::fs::write(
        dir.join("readings.csv"),
        "v\n1\n2\n5\n9\n20\n40\n90\n200\n400\n900\n",
    )
    .expect("the fixture writes");
    let spec = dir.join("readings.yaml");
    std::fs::write(
        &spec,
        "data:\n  rows:\n    file: readings.csv\nplot:\n  - mark: rectY\n    \
         data: { from: rows }\n    x: { bin: v }\n    y: { count: }\n    \
         fill: steelblue\nxScale: log\nwidth: 640\nheight: 400\n",
    )
    .expect("the spec writes");
    let boot = Boot::open(
        spec.to_str().expect("utf-8 temp path"),
        Flow::Vertical,
        None,
    )
    .unwrap_or_else(|e| panic!("open {}: {e}", spec.display()));
    let mut live = Live::open(boot);
    live.settle();
    let before = clean_title(&live);

    let same = live
        .app
        .chart_doc_mut()
        .set_plot_scale(0, PlotAxis::X, ScaleType::Log);
    assert!(
        same,
        "the write was refused, so this test writes nothing: {:?}",
        live.app.chart_doc().chart_fault()
    );
    assert_eq!(
        live.app.title(),
        before,
        "a write of the scale the spec already declared marked the title"
    );

    let changed = live
        .app
        .chart_doc_mut()
        .set_plot_scale(0, PlotAxis::X, ScaleType::Linear);
    assert!(changed, "the change was refused");
    assert_eq!(live.app.title(), marked(&before));
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Opening a file replaces the document the edit was made to**, so the second
/// window starts clean and the mark does not follow the switch across files.
#[test]
fn opening_a_data_file_after_an_edit_starts_unmarked() {
    let mut live = Live::open(housing_boot());
    live.settle();
    live.transpose();
    live.switch_to("population", ScaleType::Log);
    assert!(
        live.app.title().ends_with(UNSAVED_MARK),
        "the edit did not mark the title, so the reopen proves nothing"
    );

    live.open_again();

    assert!(
        !live.app.title().contains(UNSAVED_MARK),
        "the document was replaced and the title still says unsaved: {:?}",
        live.app.title()
    );
}
