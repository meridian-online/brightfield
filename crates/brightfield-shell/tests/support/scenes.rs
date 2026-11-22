//! The windows a sweep chooses from: one headless window type and a
//! constructor for each arrangement a command's needs tell apart — the front
//! door, a Protocol made from a data file, a shipped Protocol with its graph on
//! the canvas, a Protocol of two tables with the model open, and a chart with
//! a live session behind it.
//!
//! The windows are headless, with the `brightfield-shell` binary cargo built
//! as their runner. The config directory, the Protocols folder and arc's
//! directories point at a scratch folder of the test binary's before anything
//! reads them, so nothing is written under the home directory.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The test binary's scratch folder, with the directories a window reads
/// pointed into it.
pub fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-scenes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any window in this binary reads them, and not
        // changed after.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

/// A name no other scene of this binary has used, so two scenes' Protocols do
/// not share a folder.
fn unique(stem: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("{stem}_{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A key press with no modifier held.
pub fn press(key: egui::Key) -> egui::Event {
    chord(egui::Modifiers::NONE, key)
}

/// A key press with `modifiers` held.
pub fn chord(modifiers: egui::Modifiers, key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// A window under test. The context lives as long as the window, as it does in
/// the app.
pub struct Window {
    pub app: MeridianApp,
    pub ctx: egui::Context,
    pub screen: egui::Rect,
    /// The commands the frames asked the platform to carry out, oldest first:
    /// where a clipboard write shows.
    pub commands: Vec<egui::OutputCommand>,
    /// The frame clock, at sixty frames a second, so a toast a test reads does
    /// not age out within it.
    time: f64,
}

impl Window {
    /// A window over `boot`, settled.
    pub fn open(boot: Boot) -> Self {
        scratch();
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            commands: Vec::new(),
            time: 0.0,
        };
        win.settle();
        win
    }

    /// Run one frame for each list of events.
    pub fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            self.time += 1.0 / 60.0;
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
            self.commands.extend(out.platform_output.commands);
        }
    }

    /// Two frames with no input.
    pub fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
    }

    /// Press `key`, then a frame for what it did.
    pub fn key(&mut self, key: egui::Key) {
        self.run(vec![vec![press(key)], Vec::new()]);
    }

    /// Press `key` with `modifiers` held, then a frame for what it did.
    pub fn chord(&mut self, modifiers: egui::Modifiers, key: egui::Key) {
        self.run(vec![vec![chord(modifiers, key)], Vec::new()]);
    }

    /// Type `text`, then a frame for what it did.
    pub fn type_text(&mut self, text: &str) {
        self.run(vec![vec![egui::Event::Text(text.to_owned())], Vec::new()]);
    }

    /// Ask the window's queue to open `path`, run the frames that take it, and
    /// close the sources browser if the open left it over the window.
    pub fn open_path(&mut self, path: &Path) {
        self.app
            .open_queue()
            .push(OpenRequest::Path(path.to_path_buf()));
        self.settle();
        self.settle();
        if self.app.sources_browser().is_some() {
            self.key(egui::Key::Escape);
        }
    }

    /// What the window raised and toasted, for a failed assertion to quote.
    pub fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The front door: nothing open.
pub fn door() -> Window {
    let win = Window::open(Boot::empty());
    assert!(win.app.front_door_is_live(), "an empty boot is the door");
    win
}

/// A Protocol Brightfield made from one CSV, with its dashboard on the canvas.
pub fn data_file() -> Window {
    let name = unique("housing");
    let folder = scratch().join("input").join(&name);
    std::fs::create_dir_all(&folder).expect("an input folder");
    let csv = folder.join(format!("{name}.csv"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv"),
        &csv,
    )
    .expect("the housing sample copies");
    let mut win = Window::open(Boot::empty());
    win.open_path(&csv);
    assert!(
        win.app.chart_doc().is_live(),
        "the data file's dashboard is drawn: {}",
        win.said()
    );
    win
}

/// The shipped crosswalk, with its graph on the canvas.
pub fn crosswalk() -> Window {
    let boot = Boot::start(starts::CROSSWALK, Flow::Vertical).expect("the crosswalk ships");
    let win = Window::open(boot);
    assert!(
        win.app.graph_on_canvas(),
        "the crosswalk did not put its graph on the canvas"
    );
    win
}

/// A Protocol Brightfield made from a folder of two tables, with the model
/// pane on the canvas.
pub fn own_files() -> Window {
    let folder = scratch().join("input").join(unique("shop"));
    std::fs::create_dir_all(&folder).expect("an input folder");
    std::fs::write(folder.join("customers.csv"), "id,name\n1,Ada\n2,Bo\n3,Cy\n")
        .expect("the customers table");
    std::fs::write(folder.join("orders.csv"), "id,customer_id\n10,1\n11,2\n")
        .expect("the orders table");
    let mut win = Window::open(Boot::empty());
    win.open_path(&folder);
    assert!(
        win.app.model_pane().is_some(),
        "two tables open the model: {}",
        win.said()
    );
    win
}

/// A chart over `fixture` (a name under `examples/`), with a live session
/// behind it.
pub fn live_chart(fixture: &str) -> Window {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(fixture);
    let (live, composed) = live_spec(path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("{fixture} loads live: {e}"));
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path);
    let win = Window::open(boot);
    assert!(
        !win.app.graph_on_canvas(),
        "the fixture did not put its chart on the canvas"
    );
    win
}
