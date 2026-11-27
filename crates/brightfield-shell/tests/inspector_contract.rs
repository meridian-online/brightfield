//! The inspector against the shell contract, without a GPU.
//!
//! The sibling of `chart_contract.rs`'s state-only style: this file is driven
//! through `MeridianApp::draw` and `MeridianApp::inspector_selection`, neither
//! of which needs a device — `Workspace::focus` and `Subject` are both plain
//! data. The pixel tier (`surfaces.rs`) covers what this cannot: what the
//! panel *looks* like.
//!
//! One `egui::Context` per test, created once and threaded through each
//! frame — the same shape `interval_slider.rs`'s `frame` helper uses, and for
//! the same reason: a fresh `Context::default()` per frame would leave
//! `fonts_installed` pointing at the old context and skip installing fonts on
//! the new one, which is a false economy this file does not need.
//!
//! `the_controls_rail_is_drawn_by_the_inspector_pane` is AC4's own standing
//! witness: `chart_contract.rs`'s assertions about the registry's
//! `ControlsPane` do not move when `window.rs::assemble`'s swap is reverted —
//! they read `chart_registry()` directly, never the live app — so this file
//! carries the one test that does.

#[path = "support/operations.rs"]
mod operations;

use std::path::Path;
use std::time::{Duration, Instant};

use brightfield_shell::app::{ChartDoc, CHART, CONTROLS};
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::inspector::{reading_line, NOT_RUN};
use brightfield_shell::inspector::{InspectorPane, Section, Selection, TableHandle};
use brightfield_shell::operation;
use brightfield_shell::overlays::CHART_PALETTE_VERBS;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::{self, Occupant, INSPECTOR_RAIL};
use brightfield_workbench::{Item, PaneKey};
use operations::{runner, OpWindow};

const DASHBOARD: &str = "../../examples/dashboard.yaml";

/// The window rect every frame here runs at.
fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

/// Run one frame with no input.
fn frame(app: &mut MeridianApp, ctx: &egui::Context) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// A booted chart window over the real fixture, its own `egui::Context`, and
/// two settled frames — the same recipe `chart_contract.rs`'s pixel-adjacent
/// tests use, so "settled" means the same thing here it does there.
fn settled() -> (MeridianApp, egui::Context) {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    (app, ctx)
}

/// AC4's own witness: the `CONTROLS` slot in the *live* app is drawn by
/// `InspectorPane`, not the registry's dormant `ControlsPane` — the two
/// declare different titles ("Inspector" vs "Controls"), and this is the one
/// test in the suite that asks the running window which one is actually
/// there rather than asking the registry what it declares.
#[test]
fn the_controls_rail_is_drawn_by_the_inspector_pane() {
    let (app, _ctx) = settled();
    let title = app
        .chart_pane_title(PaneKey::new(CONTROLS))
        .expect("the rail is in the default arrangement");
    assert_eq!(
        title, "Inspector",
        "the CONTROLS slot is drawn by ControlsPane, not InspectorPane — the \
         swap in window.rs::assemble did not happen"
    );
}

/// Run one frame with `events` as its input.
fn frame_with(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        events,
        modifiers,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// ⌘⌥B pressed, then a settled frame.
fn press_cmd_alt_b(app: &mut MeridianApp, ctx: &egui::Context) {
    let modifiers = egui::Modifiers {
        alt: true,
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    frame_with(
        app,
        ctx,
        vec![egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        modifiers,
    );
    frame(app, ctx);
}

/// The chart window with its inspector rail open.
fn settled_open() -> (MeridianApp, egui::Context) {
    let (mut app, ctx) = settled();
    if app.rail_is_collapsed(INSPECTOR_RAIL) {
        press_cmd_alt_b(&mut app, &ctx);
    }
    assert!(
        !app.rail_is_collapsed(INSPECTOR_RAIL),
        "the rail did not open"
    );
    (app, ctx)
}

/// **The right rail is one inspector**: the arrangement declares one pane
/// there, so its strip names it and offers no choice between panes, and the
/// rail draws its sections under that one name.
#[test]
fn the_inspector_rail_draws_one_pane_and_no_tab_strip() {
    let rail = arrangement::default_arrangement().expect_region(INSPECTOR_RAIL);
    let Occupant::Panes(panes) = rail.occupant else {
        panic!("the inspector rail holds panes");
    };
    assert_eq!(panes.len(), 1, "the rail declares {panes:?}");
    let (app, _ctx) = settled_open();
    assert_eq!(
        app.rail_pane_title(INSPECTOR_RAIL).as_deref(),
        Some("Inspector")
    );
    assert!(
        !app.inspector_sections().is_empty(),
        "the open rail drew no section"
    );
}

/// **Step is last, and folded while a section above it is open.** A chart
/// with nothing selected is described by its Parameters, which open over the
/// Step section; opening Step folds Parameters, and opening Parameters again
/// folds Step.
#[test]
fn the_step_section_is_last_and_collapsed_while_another_is_open() {
    let (mut app, ctx) = settled_open();
    assert_eq!(
        app.inspector_sections(),
        [(Section::Parameters, true), (Section::Step, false)]
    );
    app.toggle_inspector_section(Section::Step);
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_sections(),
        [(Section::Parameters, false), (Section::Step, true)]
    );
    app.toggle_inspector_section(Section::Parameters);
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_sections(),
        [(Section::Parameters, true), (Section::Step, false)]
    );
}

/// **⌘⌥B hides the inspector and brings it back**, and the key is answered
/// by its handler.
#[test]
fn cmd_alt_b_hides_and_brings_back_the_inspector() {
    let (mut app, ctx) = settled_open();
    press_cmd_alt_b(&mut app, &ctx);
    assert!(
        app.rail_is_collapsed(INSPECTOR_RAIL),
        "⌘⌥B did not hide the rail"
    );
    assert!(app.inspector_sections().is_empty());
    press_cmd_alt_b(&mut app, &ctx);
    assert!(
        !app.rail_is_collapsed(INSPECTOR_RAIL),
        "⌘⌥B did not bring the rail back"
    );
    assert_eq!(
        app.inspector_sections(),
        [(Section::Parameters, true), (Section::Step, false)],
        "the rail came back without its sections"
    );
}

/// **A layout saved when the rail held three panes opens on the one
/// inspector.** The saved tree names the operator's pane, the chart's and the
/// operation panel, and those ids stay published, so the file loads whole and
/// the rail draws its sections from them.
#[test]
fn a_layout_saved_with_three_inspector_panes_opens_on_the_one_inspector() {
    use brightfield_workbench::persist::{self, LoadOutcome};
    // A window publishes the panes' ids at boot, before it reads a layout, so
    // one is booted first as the app's own start does.
    let _ = settled();
    let saved = brightfield_shell::startup::default_layout();
    let json = serde_json::to_string(&saved).expect("a layout serialises");
    for pane in [
        "protocol-inspector",
        "chart-controls",
        "operation-inspector",
    ] {
        assert!(
            json.contains(&format!("\"{pane}\"")),
            "the saved layout does not name {pane}, so this test proves nothing"
        );
    }
    let dir = std::env::temp_dir().join(format!("bf-inspector-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path = dir.join("layout.json");
    std::fs::write(&path, json).expect("writes");
    let (restored, outcome) = persist::load(&path, brightfield_shell::startup::default_layout);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(outcome, LoadOutcome::Restored, "{}", outcome.reason());

    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut app = MeridianApp::headless_with_layout(Boot::charts(composed), restored, Mode::Light);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    if app.rail_is_collapsed(INSPECTOR_RAIL) {
        press_cmd_alt_b(&mut app, &ctx);
    }
    assert_eq!(
        app.inspector_sections(),
        [(Section::Parameters, true), (Section::Step, false)]
    );
}

/// AC2: before the user has clicked on a pane, the inspector reports `None`
/// — not a stale answer from some earlier boot, because there is no earlier
/// boot.
#[test]
fn nothing_is_selected_before_any_pane_has_been_focused() {
    let (app, _ctx) = settled();
    assert_eq!(
        app.inspector_selection(),
        None,
        "a freshly booted window has focused nothing yet"
    );
}

/// AC1, the literal claim: select one pane, then another, and the panel's
/// text is not identical for both.
#[test]
fn selecting_a_different_pane_changes_what_the_inspector_shows() {
    let (mut app, ctx) = settled();

    assert!(
        app.focus_pane(PaneKey::new(CHART)),
        "the chart pane is in the default arrangement"
    );
    // Focus lands in `MeridianApp::apply`, reached at the end of the frame
    // the request was raised in — but `focus_pane` sets it directly, so the
    // very next frame's `inspector_selection` read already sees it.
    frame(&mut app, &ctx);
    let chart = app
        .inspector_selection()
        .expect("the chart pane is focused");
    assert_eq!(chart.title, "Chart");

    assert!(
        app.focus_pane(PaneKey::new(EDITOR)),
        "the editor pane is in the default arrangement"
    );
    frame(&mut app, &ctx);
    let editor = app
        .inspector_selection()
        .expect("the editor pane is focused");

    assert_ne!(
        chart.title, editor.title,
        "selecting a different pane must change what the panel's rendered \
         text says — the observable failure AC1 names"
    );
}

/// AC2's other half: clearing focus reverts the panel to "nothing selected"
/// rather than holding the last pane it named.
#[test]
fn clearing_focus_drops_the_selection_rather_than_holding_it_stale() {
    let (mut app, ctx) = settled();
    app.focus_pane(PaneKey::new(CHART));
    frame(&mut app, &ctx);
    assert!(app.inspector_selection().is_some(), "the chart is selected");

    app.clear_focus();
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_selection(),
        None,
        "focus was cleared, so the inspector must not go on naming the chart \
         pane — that would be exactly the stale previous selection AC2 rules \
         out"
    );
}

/// Focusing the inspector's own pane — clicking its checkbox, say — must not
/// blank the selection it is itself displaying: the last real pane focused
/// stays named.
#[test]
fn focusing_the_inspector_itself_leaves_the_last_real_selection_standing() {
    let (mut app, ctx) = settled();
    app.focus_pane(PaneKey::new(EDITOR));
    frame(&mut app, &ctx);
    let editor = app
        .inspector_selection()
        .expect("the editor pane is focused");

    assert!(
        app.focus_pane(PaneKey::new(CONTROLS)),
        "the rail itself is a pane in the arrangement"
    );
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_selection().as_ref().map(|s| &s.title),
        Some(&editor.title),
        "clicking inside the inspector's own pane must not blank what it \
         was showing"
    );
}

/// AC2's whole-pane gate, over the pane that actually ships rather than over
/// the registry's dormant `ControlsPane`: no dashboard open means the shell
/// draws the empty state instead of calling `ui`, and a real dashboard means
/// it does not.
#[test]
fn the_inspector_is_empty_only_when_the_document_is() {
    let pane = InspectorPane::new(Selection::default(), TableHandle::default());
    let empty = ChartDoc::empty();
    assert!(
        pane.empty_state(&empty).is_some(),
        "no dashboard is open, so the shipping pane must decline to draw"
    );

    let loaded = ChartDoc::headless(compose_spec(DASHBOARD).expect("compose the fixture"));
    assert!(
        pane.empty_state(&loaded).is_none(),
        "a real dashboard is open, so the shipping pane must draw its body — \
         an inverted predicate here would blank the whole rail, checkbox \
         included"
    );
}

/// Each chart-view pane's toolbar over the shipped fixture, straight from
/// `chart_registry()`'s freshly constructed, never-drawn items. That is fine
/// here specifically: `chart_item.rs`'s toolbar comes from `Self::
/// toolbar_entries(doc)`, a pure function of the document, `data_grid.rs`
/// declares an empty one, and so does the registry's own `ControlsPane` —
/// the one pane this shortcut does not hold for is the editor, which needs
/// [`editor_toolbar_verbs`] below instead.
fn registry_toolbar_verbs(doc: &ChartDoc) -> Vec<&'static str> {
    brightfield_shell::app::chart_registry()
        .specs()
        .iter()
        .flat_map(|spec| (spec.make)().subject(doc).toolbar)
        .map(|entry| entry.verb.as_str())
        .collect()
}

/// The editor's toolbar needs a file actually open — `describe` reads `self.
/// file`, a field `ui`'s first call populates from `doc.spec_path` (see
/// `editor.rs::ui`), untouched afterwards. A fresh, undrawn `EditorPane`
/// declares an empty toolbar, which is the gap that let an earlier version
/// of this sweep pass under the same mutation its doc comment claimed would
/// redden it: walking `chart_registry().specs()` alone leaves `save-spec`
/// out of the set being matched, so removing its arm left the test's
/// outcome unchanged.
fn editor_toolbar_verbs(doc: &mut ChartDoc) -> Vec<&'static str> {
    let mut pane = brightfield_shell::editor::EditorPane::new();
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(480.0, 320.0),
        )),
        ..Default::default()
    };
    let mut requests = Vec::new();
    let _ = ctx.run_ui(raw, |ui| {
        let mut icx = brightfield_workbench::ItemCtx::new(
            Mode::Light,
            PaneKey::new(EDITOR),
            egui_tiles::TileId::from_u64(1),
            true,
            &mut requests,
        );
        pane.ui(doc, ui, &mut icx);
    });
    pane.describe(doc)
        .toolbar
        .into_iter()
        .map(|entry| entry.verb.as_str())
        .collect()
}

/// The defect this lane would otherwise have shipped: a
/// declared-but-undispatchable verb — one `MeridianApp::apply`'s Charts arm
/// has no case for — would look live if this rail drew it unfiltered. This
/// sweep requires a verdict, from [`registry_toolbar_verbs`] and
/// [`editor_toolbar_verbs`] combined: dispatches, so the inspector may draw
/// it, or does not, so it must stay filtered. An unrecognised verb panics
/// rather than passing over unproven, the same shape
/// `overlay_wiring.rs::every_chart_palette_candidate_actually_dispatches`
/// uses for the command palette.
///
/// `save-spec` stays filtered here, and the reason changed. It used to be
/// this file's worked example of a verb `MeridianApp::apply`'s Charts arm had
/// no case for. The arm exists now — it writes the Protocol a data file opened
/// as — but the entry this rail would draw for that verb is `EditorPane`'s,
/// which means the editor's own buffer. One name, two writes: drawn here it is
/// a dead button over a clean buffer and a wrong one over a dirty buffer,
/// because the click saves the Protocol and reports success over an edit that
/// was never written. The palette carries the Protocol save instead —
/// `overlay_wiring.rs`'s sweep picks it off there and reads the written spec
/// back.
///
/// Proof this sweep can fail: dropping the `"save-spec"` arm below (so it
/// falls to the `other => panic!` case) reddens this test with "save-spec is
/// declared on a chart-view pane's toolbar with no verdict in this sweep".
#[test]
fn every_declared_toolbar_verb_either_dispatches_or_is_filtered_out() {
    let mut doc =
        ChartDoc::headless(compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml"));
    doc.spec_path = Some(DASHBOARD.into());

    let mut verbs = registry_toolbar_verbs(&doc);
    verbs.extend(editor_toolbar_verbs(&mut doc));

    for verb in &verbs {
        let dispatches = CHART_PALETTE_VERBS.contains(verb);
        match *verb {
            "clear-selection" | "reset-extent" => assert!(
                dispatches,
                "{verb} is expected to dispatch at the chart altitude — \
                 CHART_PALETTE_VERBS moved and \
                 inspector::dispatchable's verdict for it needs re-checking"
            ),
            "save-spec" => assert!(
                !dispatches,
                "save-spec is drawable in the inspector rail again. The entry \
                 there is the editor's buffer Save and the dispatch is the \
                 Protocol's, so one of the two writes silently does not \
                 happen. The palette is where the Protocol save is offered — \
                 see overlays::chart_palette_verbs."
            ),
            other => panic!(
                "{other} is declared on a chart-view pane's toolbar with no \
                 verdict in this sweep — decide whether the inspector may \
                 draw it (it dispatches: add a case here) or must keep \
                 dropping it, and extend this match"
            ),
        }
    }
    assert!(
        verbs.contains(&"save-spec"),
        "the editor's toolbar declared no save-spec entry — the file did \
         not open, which would make this sweep vacuous for it: saw {verbs:?}"
    );
}

// ---------------------------------------------------------------------------
// A table a later step makes
// ---------------------------------------------------------------------------

const SAMPLE: &str = "california_housing_sample.csv";
const ROOT: &str = "california_housing_sample";

/// Save the sample's Protocol into `dir`, then record two steps on the loaded
/// table through arc: `top`, which keeps ten rows, and `old`, which keeps the
/// rows of houses older than forty years.
fn two_steps(dir: &Path) {
    let opened = data_file::open(&dir.join(SAMPLE).to_string_lossy()).expect("the sample opens");
    opened.protocol.save_to(dir).expect("the Protocol saves");
    operation::record(
        &runner(),
        dir,
        "limit-rows",
        ROOT,
        "top",
        &[("count", "10")],
    )
    .expect("arc records top");
    operation::record(
        &runner(),
        dir,
        "filter-rows",
        ROOT,
        "old",
        &[("where", "house_age > 40")],
    )
    .expect("arc records old");
}

/// Ask for the columns of `table` until they have been read.
fn read_columns(win: &mut OpWindow, table: &str) -> Vec<brightfield_shell::one_step::ColumnFacts> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(columns) = win.app.column_facts_for(table) {
            return columns.to_vec();
        }
        assert!(
            Instant::now() < deadline,
            "the columns of {table} never arrived"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Click the `grid` row under the asset row of `table`, putting its grid on
/// the canvas, without waiting for anything to be read.
fn click_grid_of(win: &mut OpWindow, table: &str) {
    let rows = win.rows();
    let at = rows
        .iter()
        .position(|row| row.role == SpineRole::Asset && row.label == table)
        .unwrap_or_else(|| panic!("no asset row {table}: {:?}", win.spine_labels()));
    let grid = rows
        .get(at + 1)
        .filter(|row| row.role == SpineRole::View && row.label == "grid")
        .unwrap_or_else(|| panic!("no grid row under {table}: {:?}", win.spine_labels()));
    win.click(grid.rect.center());
}

/// Draw frames until the window draws `text`.
fn wait_to_draw(win: &mut OpWindow, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !win.texts.iter().any(|t| t == text) {
        assert!(
            Instant::now() < deadline,
            "the window never drew {text:?}; it drew {:?}",
            win.texts
        );
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
    }
}

/// **A step's table is profiled the first time it is asked about, and once.**
/// Nothing is read before the ask; the ask starts a read and answers `None`
/// until it is in; the answer is the step's own table, ten rows of the
/// sample's columns, with the moments of a numeric column measured over those
/// rows; and asking again is served from what was read.
#[test]
fn a_column_of_a_steps_table_is_profiled_when_first_inspected() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, two_steps);
    assert_eq!(
        win.app.column_profiles_started(),
        0,
        "no table is profiled before something asks about it"
    );
    assert!(
        win.app.column_facts_for("top").is_none(),
        "the first ask starts the read; it does not wait for it"
    );
    assert_eq!(win.app.column_profiles_started(), 1);

    let columns = read_columns(&mut win, "top");
    let income = columns
        .iter()
        .find(|c| c.column == "median_income")
        .unwrap_or_else(|| panic!("top keeps the sample's columns: {columns:?}"));
    assert_eq!(income.rows, 10, "the profile is of the step's ten rows");
    let moments = income.moments.as_ref().expect("a numeric column's moments");
    let upper = moments
        .upper_quartile
        .expect("the upper quartile is measured");
    assert!(moments.median <= upper && upper <= moments.max);

    let _ = read_columns(&mut win, "top");
    assert_eq!(
        win.app.column_profiles_started(),
        1,
        "a second ask is served from the profile already read"
    );
    assert!(
        win.app.column_facts_for(ROOT).is_some(),
        "the file's own table is answered from the profile it opened with"
    );
    assert_eq!(win.app.column_profiles_started(), 1);
}

/// **A data file rewritten under a kept profile is read again.** The profile
/// of `top` is kept against the file as it stood; once the file holds three
/// rows, the next ask starts a new read, and its answer is of those rows.
#[test]
fn a_rewritten_data_file_profiles_the_table_again() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, two_steps);
    let before = read_columns(&mut win, "top");
    assert_eq!(before.first().map(|c| c.rows), Some(10));

    let data = win.dir().join(SAMPLE);
    let text = std::fs::read_to_string(&data).expect("the sample reads");
    let shorter: String = text
        .lines()
        .take(4)
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&data, shorter).expect("the sample is rewritten");

    assert!(
        win.app.column_facts_for("top").is_none(),
        "the rewritten file is read again rather than served from the cache"
    );
    assert_eq!(win.app.column_profiles_started(), 2);
    let after = read_columns(&mut win, "top");
    assert_eq!(
        after.first().map(|c| c.rows),
        Some(3),
        "the new read is of the rewritten file's rows"
    );
}

/// **The inspector names the step that made the table, and where that step
/// stands**: `top` and not the load, its kind in the steps sheet's word, and
/// `not run`, since no run has recorded it. The inspector draws both under the
/// table's name once its grid is on the canvas.
#[test]
fn the_inspector_names_the_step_that_made_the_column_and_its_state() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, two_steps);
    let table = win
        .app
        .column_table_for("top")
        .expect("the model knows top");
    assert_eq!(table.table, "top");
    assert_eq!(
        table.step, "top",
        "the step that makes the table, not the load"
    );
    assert_eq!(table.kind, "sql", "arc records a limit as a SQL model");
    assert_eq!(table.state, NOT_RUN, "no run has recorded the step");

    // The spine's selection is another table's, and `top` is still `top`.
    win.select_made_by("old");
    assert_eq!(
        win.app.column_table_for("top").map(|t| t.step),
        Some("top".to_string()),
        "a table is described by its own step, whatever the spine has selected"
    );

    let root = win
        .app
        .column_table_for(ROOT)
        .expect("the model knows the file's table");
    assert_ne!(root.step, "top", "the file's table is made by its load");

    win.show_grid_of("top");
    wait_to_draw(&mut win, "top · sql");
    assert!(
        win.texts.iter().any(|t| t == NOT_RUN),
        "the inspector states the step's state: {:?}",
        win.texts
    );
}

/// **A profile still being read says so.** With the read held, the inspector
/// draws its reading line for the table on the canvas, not an empty list; once
/// the read is let go, the columns replace it.
#[test]
fn a_profile_still_reading_says_so_rather_than_showing_nothing() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, two_steps);
    win.app.chart_doc().hold_preview_reads(true);
    click_grid_of(&mut win, "old");
    wait_to_draw(&mut win, &reading_line("old"));
    // The pane asks as it draws, and the read starts on the frame after.
    win.settle();
    let started = win.app.column_profiles_started();
    assert_eq!(started, 1, "the inspector's ask started the one read");
    assert!(
        win.texts.iter().any(|t| *t == reading_line("old")),
        "the line stays while the read is held"
    );

    win.app.chart_doc().hold_preview_reads(false);
    let deadline = Instant::now() + Duration::from_secs(60);
    while win.texts.iter().any(|t| *t == reading_line("old")) {
        assert!(
            Instant::now() < deadline,
            "the columns of old never arrived"
        );
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
    }
    let columns = win
        .app
        .column_facts_for("old")
        .expect("the inspector's read of old is kept")
        .to_vec();
    assert_eq!(
        win.app.column_profiles_started(),
        started,
        "the columns drawn are the read the inspector started"
    );
    let drawn = columns.iter().all(|c| win.texts.contains(&c.column));
    assert!(
        drawn,
        "the columns replace the reading line: {:?}",
        win.texts
    );
}

/// **The inspector's read stops when the canvas leaves the table**: a profile
/// nobody is looking at does not hold the preview gate the grid reads behind.
#[test]
fn leaving_a_steps_table_stops_the_inspectors_read_of_it() {
    let mut win = OpWindow::on_copy_prepared(SAMPLE, two_steps);
    win.app.chart_doc().hold_preview_reads(true);
    click_grid_of(&mut win, "old");
    wait_to_draw(&mut win, &reading_line("old"));
    assert!(
        win.app.column_profile_under_way(),
        "the inspector asked for old"
    );

    win.graph_to_canvas();
    assert!(
        !win.app.column_profile_under_way(),
        "the read of old went on after the canvas left it"
    );
    win.app.chart_doc().hold_preview_reads(false);
}

// ---------------------------------------------------------------------------
// The Column section
// ---------------------------------------------------------------------------

/// A numeric column of the sample with many values, so its bar is binned.
const INCOME: &str = "median_income";
/// A numeric column of the sample with few values, so its bar is its values.
const AGE: &str = "house_age";

/// The sample's window with `column` selected on its dashboard and the
/// inspector rail open, settled, and the selected column's facts.
fn with_column(column: &str) -> (OpWindow, brightfield_shell::one_step::ColumnFacts) {
    let mut win = OpWindow::on_copy_of(SAMPLE);
    if win.app.rail_is_collapsed(INSPECTOR_RAIL) {
        win.key_with(
            egui::Key::B,
            egui::Modifiers {
                alt: true,
                command: true,
                mac_cmd: cfg!(target_os = "macos"),
                ctrl: !cfg!(target_os = "macos"),
                ..Default::default()
            },
        );
    }
    win.app.chart_doc_mut().select_column(column);
    win.settle();
    assert!(
        !win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "the inspector rail is open"
    );
    let facts = win
        .app
        .chart_doc()
        .selected_column()
        .cloned()
        .unwrap_or_else(|| panic!("the dashboard selected {column}"));
    (win, facts)
}

/// Whether the last frame drew `text`.
fn drew(win: &OpWindow, text: &str) -> bool {
    win.texts.iter().any(|t| t == text)
}

/// **The Column section says what the column is**: its FineType type, the
/// type DuckDB stores it as, and the step that made its table, with where
/// that step stands.
#[test]
fn the_column_section_says_its_finetype_its_storage_type_and_its_step() {
    let (win, facts) = with_column(INCOME);
    let table = win
        .app
        .column_table_for(&win.table())
        .expect("the model knows the file's table");
    for (label, value) in [
        ("FINETYPE", facts.full_type().to_string()),
        ("STORAGE", facts.storage.clone()),
        ("STEP", format!("{} · {}", table.step, table.kind)),
        ("STATUS", table.state.to_string()),
    ] {
        assert!(drew(&win, label), "no {label} field: {:?}", win.texts);
        assert!(
            drew(&win, &value),
            "the {label} field does not say {value:?}: {:?}",
            win.texts
        );
    }
    assert!(drew(&win, INCOME), "the section names its column");
}

/// **A number with many values draws its bins, and states its minimum,
/// median, upper quartile, maximum and missing count.** The bins are equal
/// width, low to high, and count every row that is not missing.
#[test]
fn a_numeric_column_shows_its_bins_and_minimum_median_upper_quartile_maximum_and_missing() {
    use brightfield_shell::inspector::{column_bars, column_figures, ColumnBars};
    let (win, facts) = with_column(INCOME);
    let Some(ColumnBars::Bins(bins)) = column_bars(&facts) else {
        panic!("{INCOME} is binned: {:?}", column_bars(&facts));
    };
    assert_eq!(bins.len(), brightfield_engine::profile::DISPLAY_BINS);
    assert_eq!(
        bins.iter().sum::<u64>(),
        facts.rows - facts.nulls,
        "the bins count every row that is not missing"
    );
    assert!(
        win.app.chart_doc().column_bar_rect().is_some(),
        "the section draws the bar"
    );

    let figures = column_figures(&facts);
    let labels: Vec<&str> = figures.iter().map(|(label, _)| *label).collect();
    assert_eq!(
        labels,
        [
            "rows",
            "minimum",
            "median",
            "upper quartile",
            "maximum",
            "missing"
        ]
    );
    for (label, value) in &figures {
        assert!(
            drew(&win, &label.to_uppercase()),
            "no {label} figure: {:?}",
            win.texts
        );
        assert!(drew(&win, value), "{label} does not read {value:?}");
    }
}

/// **A number with few values draws its top values**, most frequent first,
/// and every other value's rows as one bar, so the bars still count every row
/// that is not missing.
#[test]
fn a_categorical_column_shows_its_top_values() {
    use brightfield_shell::inspector::{column_bars, ColumnBars, TOP_VALUES};
    let (win, facts) = with_column(AGE);
    let Some(ColumnBars::Top { values, rest }) = column_bars(&facts) else {
        panic!("{AGE} draws its values: {:?}", column_bars(&facts));
    };
    assert!(!values.is_empty() && values.len() <= TOP_VALUES);
    assert!(
        values.windows(2).all(|w| w[0].1 >= w[1].1),
        "the values are most frequent first: {values:?}"
    );
    assert_eq!(
        values.iter().map(|(_, n)| n).sum::<u64>() + rest,
        facts.rows - facts.nulls,
        "the values and the rest count every row that is not missing"
    );
    assert!(
        win.app.chart_doc().column_bar_rect().is_some(),
        "the section draws the bar"
    );
}

/// **What can be done with the column lists its commands with their keys.**
/// The operations started from the grid come from the built forms, so a form
/// joins the list with no edit to the inspector; filter and sort are among
/// them. After them come the chart's channels. Each row says its
/// key, and a row that cannot act says why, in the sentence the window's
/// availability gives. A click on an operation's row starts it on the column.
#[test]
fn what_can_be_done_lists_the_columns_commands_with_their_keys() {
    use brightfield_shell::inspector::{key_label, COLUMN_VERBS};
    use brightfield_shell::operation_ui::{self, Start};
    let (mut win, _) = with_column(INCOME);
    let commands = win.app.inspector_column_commands();
    let verbs: Vec<&str> = commands.iter().map(|c| c.verb).collect();

    let mut want: Vec<&str> = operation_ui::built()
        .filter(|op| {
            op.starts
                .iter()
                .any(|s| matches!(s, Start::Header(_) | Start::Cell | Start::FootField(_)))
        })
        .map(|op| op.verb)
        .collect();
    want.extend(COLUMN_VERBS);
    assert_eq!(
        verbs, want,
        "the section lists the grid's forms, then the verbs"
    );
    for verb in ["filter-rows-at-cursor", "sort-rows-at-cursor"] {
        assert!(verbs.contains(&verb), "{verb} is listed");
    }
    assert!(
        !verbs.contains(&"read-file"),
        "a form started from the sources browser alone is not the column's"
    );

    for command in &commands {
        assert!(
            drew(&win, &command.label),
            "{} is not drawn: {:?}",
            command.label,
            win.texts
        );
        if let Some(key) = &command.key {
            assert!(
                drew(&win, key),
                "{}'s key {key} is not drawn",
                command.label
            );
        }
        if !command.operation {
            let available = win
                .app
                .availability_of(command.verb)
                .expect("a registry verb");
            assert_eq!(
                command.unavailable,
                match available {
                    brightfield_keys::Availability::Available => None,
                    brightfield_keys::Availability::Missing(why) => Some(why),
                },
                "{} is greyed as the window's availability says",
                command.verb
            );
        }
    }
    let put_x = commands
        .iter()
        .find(|c| c.verb == "put-column-on-x")
        .expect("put on x is listed");
    assert_eq!(put_x.key.as_deref(), Some(key_label("z x").as_str()));
    assert!(
        !verbs.contains(&"retype-column"),
        "retype acts on the model pane's table, which this section never shows beside it"
    );

    let rename = commands
        .iter()
        .find(|c| c.verb == "rename-column")
        .expect("rename is listed");
    assert_eq!(
        rename.unavailable, None,
        "a column of the loaded table can be renamed"
    );
    let at = win
        .text_rects
        .iter()
        .find(|(t, _)| *t == rename.label)
        .map(|(_, rect)| rect.center())
        .expect("rename's row is drawn");
    win.click(at);
    let deadline = Instant::now() + Duration::from_secs(60);
    while win.app.making().is_none() {
        assert!(
            Instant::now() < deadline,
            "the click never started rename: {}",
            win.banners()
        );
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
    }
    let making = win.app.making().expect("an operation is being made");
    assert_eq!(making.spec.long_name, "rename-column");
    assert_eq!(
        making.column.as_deref(),
        Some(INCOME),
        "started on the column"
    );
}

/// **Pointing at a bin says its range and its count**, on the line under the
/// bar, and pointing away leaves the line empty.
#[test]
fn hovering_a_bin_says_its_range_and_count() {
    use brightfield_shell::inspector::{bar_line, column_bars};
    let (mut win, facts) = with_column(INCOME);
    let bars = column_bars(&facts).expect("a numeric column has bars");
    let rect = win
        .app
        .chart_doc()
        .column_bar_rect()
        .expect("the section draws the bar");
    let first = bar_line(&facts, &bars, 0).expect("the first bin has a line");
    assert!(
        first.ends_with(" rows") || first.ends_with(" row"),
        "the line counts rows: {first}"
    );
    assert!(
        first.contains(" to "),
        "a bin's line says its range: {first}"
    );
    assert!(!drew(&win, &first), "nothing is pointed at yet");

    let at = egui::pos2(rect.left() + 1.0, rect.center().y);
    win.run(vec![egui::Event::PointerMoved(at)]);
    win.run(vec![egui::Event::PointerMoved(at)]);
    assert!(
        drew(&win, &first),
        "pointing at the first bin did not say {first:?}: {:?}",
        win.texts
    );
}

/// **Filter rows chosen in the Column section asks on the selected column**,
/// in the row the grid's cursor is on, whichever column the cursor was on.
#[test]
fn filter_rows_chosen_from_the_column_section_asks_on_that_column() {
    use brightfield_shell::data_grid::GridCursor;
    use brightfield_shell::grid_foot::FootField;
    let (mut win, _) = with_column(INCOME);
    let root = win.table();
    win.show_grid_of(&root);
    win.app.chart_doc_mut().grid_cursor = Some(GridCursor { row: 0, col: 1 });
    win.app.chart_doc_mut().select_column(INCOME);
    win.settle();

    let filter = win
        .app
        .inspector_column_commands()
        .into_iter()
        .find(|c| c.verb == "filter-rows-at-cursor")
        .expect("filter is listed");
    assert_eq!(filter.unavailable, None, "the cursor is on a cell");
    let at = win
        .text_rects
        .iter()
        .find(|(t, _)| *t == filter.label)
        .map(|(_, rect)| rect.center())
        .unwrap_or_else(|| panic!("filter's row is drawn: {:?}", win.texts));
    win.click(at);
    let (field, text) = win
        .app
        .chart_doc()
        .foot_asking()
        .map(|(field, text)| (field, text.to_string()))
        .expect("the where field opened");
    assert_eq!(field, FootField::Where);
    assert!(
        text.starts_with(INCOME),
        "the field asks on {INCOME}, not the cursor's column: {text}"
    );
}

/// **A foot-row operation started on a column the grid does not show opens
/// on no other column**: it says why and opens nothing.
#[test]
fn a_foot_row_operation_on_a_column_the_grid_does_not_show_opens_nothing() {
    use brightfield_shell::data_grid::GridCursor;
    use brightfield_shell::grid_foot::FootField;
    use brightfield_shell::operation_ui::{self, Start};
    let (mut win, _) = with_column(INCOME);
    let root = win.table();
    win.show_grid_of(&root);
    win.app.chart_doc_mut().grid_cursor = Some(GridCursor { row: 0, col: 1 });
    win.settle();
    let filter = operation_ui::by_verb("filter-rows-at-cursor").expect("filter is built");
    let ctx = win.ctx.clone();
    let started = win.app.start_operation_spec(
        &ctx,
        filter,
        Start::FootField(FootField::Where),
        Some("no_such_column"),
    );
    assert!(
        !started,
        "filter started on a column the grid does not show"
    );
    assert_eq!(
        win.app
            .chart_doc()
            .foot_asking()
            .map(|(_, text)| text.to_string()),
        None,
        "a field opened on the cursor's column instead"
    );
}
