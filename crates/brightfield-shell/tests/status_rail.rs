//! The status rail, drawn by the window at last — and the file watcher that
//! gives it something true to say.
//!
//! GPU-free throughout: `MeridianApp::headless_with_layout` lays out real
//! frames through `egui::Context::run_ui`, the documents report what they
//! hold without a texture, and the rail records what it drew
//! ([`MeridianApp::rail`]) the way the switcher records where it drew. The
//! external changes are real files with real mtimes, moved with
//! `set_modified` rather than sleeps wherever a sleep is not the thing being
//! tested — the one sleep each test keeps is the poll cadence or the honesty
//! line itself.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec, spec_data_files};
use brightfield_shell::startup::default_layout;
use brightfield_shell::watch::WATCH_POLL;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::{parse_spec, Format};
use brightfield_workbench::{Activity, ActivityIndicator, HONESTY_LINE_MS};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A directory unique to one test in this process.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("bf-status-rail-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The shipped dashboard example, copied to a temp path the test may write —
/// inline data, so it composes from anywhere and watches only its spec.
fn temp_dashboard(test: &str) -> PathBuf {
    let path = scratch(test).join("spec.yaml");
    fs::write(&path, include_str!("../../../examples/dashboard.yaml")).expect("copy spec");
    path
}

/// Move `path`'s mtime `secs_ago` into the past — a *different* mtime from
/// any the watcher has seen, without sleeping through filesystem timestamp
/// granularity.
fn touch_past(path: &std::path::Path, secs_ago: u64) {
    let f = fs::File::options()
        .write(true)
        .open(path)
        .expect("open for touch");
    f.set_modified(SystemTime::now() - Duration::from_secs(secs_ago))
        .expect("set mtime");
}

/// A window under test: the app and one `egui::Context` for its whole life —
/// the `front_door.rs` harness, for the `front_door.rs` reason.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open(boot: Boot) -> Self {
        Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
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
        self.run(vec![Vec::new(), Vec::new()]);
    }

    /// Focus the chart pane the way a person does: press inside it, where
    /// the last frame recorded it.
    fn focus_chart_pane(&mut self) {
        let target = self
            .app
            .chart_doc()
            .viewport
            .expect("a settled frame recorded the chart pane's box");
        let pos = target.center();
        let mut events = vec![egui::Event::PointerMoved(pos)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        self.run(vec![events, Vec::new()]);
    }
}

/// The idle rail entry's id — `window::IDLE_STATUS_ID`, not exported, so this
/// is the same literal-id convention `"watch-spec"` already uses below.
const IDLE_STATUS_ID: &str = "chart-idle";

/// A booted window over the dashboard spec at `path`, chart pane focused,
/// rail settled to the idle line and nothing louder.
fn dashboard_window(path: &std::path::Path) -> Window {
    let composed = compose_spec(path.to_str().expect("utf-8 temp path")).expect("compose");
    let mut boot = Boot::charts(composed);
    boot.spec_path = Some(path.to_path_buf());
    let mut w = Window::open(boot);
    w.settle();
    w.focus_chart_pane();
    assert_eq!(
        w.app.rail().drawn,
        vec![IDLE_STATUS_ID],
        "at rest the rail says only what is loaded — quiet of everything \
         louder is still the default"
    );
    w
}

// ---------------------------------------------------------------------------
// The watcher, through the window
// ---------------------------------------------------------------------------

/// An external edit to the open spec reaches the rail: the watcher sees the
/// mtime move on its own cadence, the chart pane's subject carries the
/// notice, and the window rails it — with no input in between, because a
/// change nobody typed is exactly the change the watcher exists for.
#[test]
fn an_external_spec_change_reaches_the_rail() {
    let path = temp_dashboard("external-spec");
    let mut w = dashboard_window(&path);

    // The external edit. The mtime moves into the past — any value the
    // baseline cannot share.
    touch_past(&path, 100);

    // Let the poll cadence elapse, then one frame: the poll runs at the top
    // of the frame and the rail draws at the bottom of the same one.
    std::thread::sleep(WATCH_POLL + Duration::from_millis(20));
    w.run(vec![Vec::new()]);

    assert!(
        w.app.rail().drawn.contains(&"watch-spec"),
        "the rail says the spec changed; it drew {:?}",
        w.app.rail().drawn
    );
    let entries = w.app.chart_doc().watch.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].text, "spec changed on disk",
        "a fact about a file — not a staleness verdict, which stays the \
         engine's to compute"
    );
}

/// The notice stands across frames until something resolves it — a fact does
/// not expire because the user did not look at it fast enough.
#[test]
fn the_notice_stands_until_resolved() {
    let path = temp_dashboard("standing");
    let mut w = dashboard_window(&path);

    touch_past(&path, 100);
    std::thread::sleep(WATCH_POLL + Duration::from_millis(20));
    w.run(vec![Vec::new()]);
    assert!(w.app.rail().drawn.contains(&"watch-spec"));

    w.settle();
    assert!(
        w.app.rail().drawn.contains(&"watch-spec"),
        "two frames later the fact still stands"
    );
}

// ---------------------------------------------------------------------------
// The indicator, through the window
// ---------------------------------------------------------------------------

/// In-flight work reaches the rail as **one** indicator entry: the pane's
/// typed activity reports are collected, merged, and said once — never
/// railed per-pane beside the merged line.
#[test]
fn activity_reaches_the_rail_as_the_one_indicator() {
    let path = temp_dashboard("activity");
    let mut w = dashboard_window(&path);

    w.app.chart_doc_mut().activity.begin(Activity::EngineQuery);
    std::thread::sleep(Duration::from_millis(
        u64::try_from(HONESTY_LINE_MS).expect("small") + 20,
    ));
    w.run(vec![Vec::new()]);

    let drawn = &w.app.rail().drawn;
    assert!(
        drawn.contains(&ActivityIndicator::ID),
        "the indicator drew; the rail drew {drawn:?}"
    );
    assert!(
        !drawn.contains(&Activity::EngineQuery.id()),
        "the pane's own activity entry is folded into the indicator, not \
         drawn beside it"
    );

    // A second kind joins the same entry rather than adding one.
    w.app.chart_doc_mut().activity.begin(Activity::FileWatch);
    w.run(vec![Vec::new()]);
    let drawn = &w.app.rail().drawn;
    assert_eq!(
        drawn
            .iter()
            .filter(|id| **id == ActivityIndicator::ID)
            .count(),
        1,
        "two kinds of work, one indicator; the rail drew {drawn:?}"
    );

    // Work resolves, the rail falls back to the idle line — quiet of
    // activity, never quiet outright: AC2's "takes precedence", proved in
    // both directions in one test.
    w.app.chart_doc_mut().activity.end(Activity::EngineQuery);
    w.app.chart_doc_mut().activity.end(Activity::FileWatch);
    w.settle();
    assert_eq!(
        w.app.rail().drawn,
        vec![IDLE_STATUS_ID],
        "resolved work leaves only the idle line on the rail"
    );
}

// ---------------------------------------------------------------------------
// The idle line
// ---------------------------------------------------------------------------

/// AC1: an idle window with a chart open still has a rail to read. No
/// activity, no watcher notice, no navigation refusal — the only thing
/// standing between "nothing running" and an empty rail is the idle line.
///
/// This is the test AC4 asks the builder to break on purpose: short-circuit
/// `idle_status_entry` back to always returning `None` (its pre-fix
/// behaviour) and this reddens — see the PR body for the pasted failure.
#[test]
fn an_idle_chart_window_names_what_it_loaded() {
    let path = temp_dashboard("idle-load");
    let w = dashboard_window(&path);
    assert!(
        w.app.rail().drawn.contains(&IDLE_STATUS_ID),
        "an idle window with a chart open should say what it loaded; it drew \
         {:?}",
        w.app.rail().drawn
    );
}

// ---------------------------------------------------------------------------
// Focus scoping — AC6 and AC7
// ---------------------------------------------------------------------------

/// AC6: a status entry declared by a pane is drawn without the user first
/// clicking inside that pane — this test never calls `focus_chart_pane`,
/// only `settle`, and the watcher's own notice still reaches the rail.
///
/// AC7: this checks `app.rail().drawn`, populated by
/// `chrome::status_rail`'s real draw pass through `egui::Context::run_ui`
/// (see the module doc) — not `ChartItem::new().subject(doc).status`, which
/// only proves the entry was *declared*. A window that collected entries from
/// the focused pane only would leave `drawn` without `"watch-spec"` here,
/// because nothing in this test ever focuses a pane.
#[test]
fn a_panes_notice_reaches_the_rail_before_anything_is_focused() {
    let path = temp_dashboard("unfocused-notice");
    let composed = compose_spec(path.to_str().expect("utf-8 temp path")).expect("compose");
    let mut boot = Boot::charts(composed);
    boot.spec_path = Some(path.to_path_buf());
    let mut w = Window::open(boot);
    w.settle(); // no click anywhere — nothing holds focus

    touch_past(&path, 100);
    std::thread::sleep(WATCH_POLL + Duration::from_millis(20));
    w.run(vec![Vec::new()]);

    assert!(
        w.app.rail().drawn.contains(&"watch-spec"),
        "a pane's own notice reaches the rail even though nothing has been \
         clicked; it drew {:?}",
        w.app.rail().drawn
    );
}

// ---------------------------------------------------------------------------
// The watch list
// ---------------------------------------------------------------------------

/// The watcher's data list comes off the spec's own `file:` sources: local
/// paths resolved against the spec's directory, URLs skipped — they are not
/// files an mtime poll can watch.
#[test]
fn the_watch_list_is_the_specs_local_file_sources() {
    let source = "\
data:
  local: { file: rows.csv }
  remote: { file: \"https://example.org/rows.csv\" }
";
    let parsed = parse_spec(source, Format::Yaml).expect("a bare data spec parses");
    let dir = PathBuf::from("/specs/here");
    let files = spec_data_files(&parsed.spec, Some(&dir));
    assert_eq!(files, vec![PathBuf::from("/specs/here/rows.csv")]);
}
