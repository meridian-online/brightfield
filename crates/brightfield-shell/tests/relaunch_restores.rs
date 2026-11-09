//! A launch reopens the Protocol that was open when the app last ended, as it was last saved, and
//! closing the window shows the front door rather than ending the app.
//!
//! A relaunch here is what `main` does: the layout file read through `startup::boot_layout`, the
//! boot chosen by `startup::opening_boot`, and a Protocol to restore found by
//! `startup::restored_protocol` and pushed onto the new window's open queue.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner, and the
//! config directory, the Protocols folder and arc's directories point at a scratch folder of this
//! file's, so nothing is written under the home directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::persist::Opened;

/// The condition kept on the grid, unsaved.
const CONDITION: &str = "house_age > 28";

/// The overlay's name for the close question, as `MeridianApp::open_overlay` says it.
const QUESTION: &str = "close-question";

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-relaunch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input", "versions"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(startup::CONFIG_DIR_VAR, root.join("config"));
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

/// A layout file of the test's own, so the tests in this file, which run at once, do not read
/// each other's launches.
fn layout_file(test: &str) -> PathBuf {
    scratch().join("config").join(format!("{test}.json"))
}

/// A copy of the committed California Housing sample, named `<name>.csv`, in a folder of its own.
fn housing_copy(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    std::fs::create_dir_all(&dir).expect("an input folder");
    let csv = dir.join(format!("{name}.csv"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    csv
}

fn protocol_dir(name: &str) -> PathBuf {
    scratch().join("protocols").join(name)
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// One headless window and what its frames have sent the operating system.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
    cancelled_close: bool,
    closed: bool,
}

impl Window {
    fn over(app: MeridianApp) -> Self {
        Self {
            app: app.running_with(Some(runner())).keeping_history(Some(
                brightfield_protocol::HistoryStore::At(scratch().join("versions")),
            )),
            ctx: egui::Context::default(),
            time: 0.0,
            cancelled_close: false,
            closed: false,
        }
    }

    /// A first launch, with the Protocol made of a copy of the housing sample named `name` opened
    /// on it through the open queue.
    fn opened(name: &str) -> Self {
        scratch();
        let csv = housing_copy(name);
        let mut win = Self::over(MeridianApp::headless(Boot::empty(), Mode::Light));
        win.app.open_queue().push(OpenRequest::Path(csv));
        win.settle();
        assert_eq!(
            win.holds(),
            Some(protocol_dir(name)),
            "the window holds the Protocol {name}"
        );
        win
    }

    /// The app launched again over the layout file `file`, as `main` launches it.
    fn relaunched(file: &Path) -> Self {
        let (mut layout, _) = startup::boot_layout(Some(file));
        let restore = startup::restored_protocol(None, &mut layout);
        let boot = opening(&layout);
        let mut win = Self::over(MeridianApp::headless_with_layout(boot, layout, Mode::Light));
        if let Some(dir) = restore {
            win.app.open_queue().push(OpenRequest::Path(dir));
        }
        win.settle();
        win
    }

    /// End this launch: the layout written as the app's exit writes it, and the window dropped.
    fn end(mut self, file: &Path) {
        if let Some(Err(e)) = self.app.flush_layout(file) {
            panic!("the layout did not write: {e}");
        }
    }

    fn holds(&self) -> Option<PathBuf> {
        self.app.own_protocol().map(|p| p.dir.clone())
    }

    fn frame(&mut self, events: Vec<egui::Event>, close: bool) {
        self.time += 1.0;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        if close {
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        if let Some(root) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            for command in &root.commands {
                match command {
                    egui::ViewportCommand::CancelClose => self.cancelled_close = true,
                    egui::ViewportCommand::Close => self.closed = true,
                    _ => {}
                }
            }
        }
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new(), false);
        }
    }

    /// The window's close button, as one frame, then let the window settle.
    fn close_button(&mut self) {
        self.frame(Vec::new(), true);
        self.settle();
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            false,
        );
        self.settle();
    }

    fn forget_sent(&mut self) {
        self.cancelled_close = false;
        self.closed = false;
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// Keep a filter on the grid, unsaved.
    fn keep_filter(&mut self) {
        self.app
            .chart_doc_mut()
            .apply_grid_view(brightfield_engine::RowsView {
                condition: Some(CONDITION.to_owned()),
                order: None,
            });
        self.settle();
        assert!(self.marked(), "the kept filter marks the title unsaved");
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// The boot `main` opens with for `layout` when the command line names nothing.
fn opening(layout: &brightfield_workbench::SavedLayout) -> Boot {
    startup::opening_boot(None, layout.opened_start(), Flow::Vertical, None)
        .expect("a launch with nothing named boots")
}

#[test]
fn relaunch_reopens_the_protocol_as_saved() {
    let file = layout_file("as_saved");
    let mut win = Window::opened("relaunch_as_saved");
    assert_eq!(
        win.app.layout().opened,
        Some(Opened::Protocol(protocol_dir("relaunch_as_saved"))),
        "opening the Protocol records it to restore"
    );
    win.keep_filter();
    win.end(&file);

    let again = Window::relaunched(&file);

    assert_eq!(
        again.holds(),
        Some(protocol_dir("relaunch_as_saved")),
        "the relaunch reopened the Protocol: {}",
        again.said()
    );
    assert!(!again.app.front_door_is_live());
    assert!(!again.marked(), "with no draft carried across the launch");
    assert_eq!(
        again.app.chart_doc().grid_view_line(),
        None,
        "and without the filter that was never saved"
    );
}

#[test]
fn closing_the_window_shows_the_front_door() {
    let mut win = Window::opened("close_to_door");

    // Nothing pending: the close goes to the door at once, and the app does not end.
    win.close_button();
    assert!(win.cancelled_close, "the close request is cancelled");
    assert!(!win.closed, "and the window does not close itself");
    assert!(win.app.front_door_is_live(), "the door shows");
    assert_eq!(
        win.app.layout().opened,
        None,
        "and nothing is left to restore"
    );

    // With a draft the close asks first, and its answer goes to the door.
    win.app
        .open_queue()
        .push(OpenRequest::Path(protocol_dir("close_to_door")));
    win.settle();
    assert_eq!(win.holds(), Some(protocol_dir("close_to_door")));
    win.keep_filter();
    win.forget_sent();
    win.close_button();
    assert_eq!(
        win.app.open_overlay(),
        Some(QUESTION),
        "a close over a draft asks"
    );
    assert!(!win.app.front_door_is_live(), "and waits for the answer");
    win.key(egui::Key::D, egui::Modifiers::NONE);
    assert!(
        win.app.front_door_is_live(),
        "D went to the door: {}",
        win.said()
    );
    assert!(!win.closed, "without ending the app");
    assert!(!win.app.is_closing());

    // Cmd+Q still ends the app.
    win.forget_sent();
    win.key(egui::Key::Q, egui::Modifiers::COMMAND);
    assert!(win.closed, "Cmd+Q quits");
}

#[test]
fn closing_on_the_door_ends_the_app() {
    let mut win = Window::over(MeridianApp::headless(Boot::empty(), Mode::Light));
    win.settle();
    assert!(win.app.front_door_is_live());
    win.frame(Vec::new(), true);
    assert!(
        !win.cancelled_close,
        "a close on the door has nowhere else to go, so it goes through"
    );
}

#[test]
fn closing_to_the_door_relaunches_on_the_door() {
    let file = layout_file("to_the_door");
    let mut win = Window::opened("relaunch_to_door");
    win.close_button();
    assert!(win.app.front_door_is_live());
    win.end(&file);

    let again = Window::relaunched(&file);

    assert!(
        again.app.front_door_is_live(),
        "the relaunch is on the door"
    );
    assert_eq!(again.holds(), None);
}

#[test]
fn going_home_relaunches_on_the_door() {
    let file = layout_file("home");
    let mut win = Window::opened("relaunch_home");
    win.key(
        egui::Key::H,
        egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        },
    );
    assert!(win.app.front_door_is_live());
    assert_eq!(win.app.layout().opened, None);
    win.end(&file);

    assert!(Window::relaunched(&file).app.front_door_is_live());
}

#[test]
fn an_old_layout_file_with_a_start_id_still_restores_it() {
    let file = layout_file("old_start");
    scratch();
    // A layout file as a build before Protocols were restored wrote it: `opened` a bare start id.
    let mut old: serde_json::Value =
        serde_json::from_str(&startup::default_layout().to_json().expect("serialises"))
            .expect("json");
    old["opened"] = serde_json::Value::String(starts::DASHBOARD.to_string());
    std::fs::write(&file, old.to_string()).expect("the old file writes");

    let (mut layout, outcome) = startup::boot_layout(Some(&file));

    assert!(
        outcome.restored(),
        "the old file loads: {}",
        outcome.reason()
    );
    assert_eq!(
        layout.opened,
        Some(Opened::Start(starts::DASHBOARD.to_string()))
    );
    assert_eq!(startup::restored_protocol(None, &mut layout), None);
    assert!(
        !opening(&layout).is_empty(),
        "the start is reopened from the old file"
    );

    // And this build writes the start the same way, so an older build reads it back too.
    let written: serde_json::Value =
        serde_json::from_str(&layout.to_json().expect("serialises")).expect("json");
    assert_eq!(written["opened"], serde_json::json!(starts::DASHBOARD));
}

#[test]
fn a_restored_protocol_that_was_deleted_opens_the_door() {
    // The folder gone.
    let file = layout_file("deleted");
    let win = Window::opened("relaunch_deleted");
    win.end(&file);
    std::fs::remove_dir_all(protocol_dir("relaunch_deleted")).expect("the folder deletes");

    let again = Window::relaunched(&file);

    assert!(
        again.app.front_door_is_live(),
        "the relaunch is on the door"
    );
    assert_eq!(
        again.app.notifications().iter().count(),
        0,
        "with no banner: {}",
        again.said()
    );
    assert!(
        !protocol_dir("relaunch_deleted").exists(),
        "and the folder is not made again"
    );

    // The folder there, its manifest gone.
    let file = layout_file("unmade");
    let win = Window::opened("relaunch_unmade");
    win.end(&file);
    std::fs::remove_file(protocol_dir("relaunch_unmade").join("arcform.yaml"))
        .expect("the manifest deletes");

    let again = Window::relaunched(&file);

    assert!(
        again.app.front_door_is_live(),
        "the relaunch is on the door"
    );
    assert_eq!(
        again.app.notifications().iter().count(),
        0,
        "with no banner: {}",
        again.said()
    );
    assert!(!protocol_dir("relaunch_unmade")
        .join("arcform.yaml")
        .exists());
}
