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

use brightfield_keys::{Availability, BindingContext, Condition, Need, Selected};
use brightfield_protocol::layout::Flow;
use brightfield_render::model_scene::Selection;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::PaneKey;

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
    /// The verbs a key performed since the last [`Window::press`] began, in
    /// the order their handlers took them.
    pub answered: Vec<&'static str>,
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
            answered: Vec::new(),
            time: 0.0,
        };
        win.settle();
        win
    }

    /// Run one frame for each list of events.
    pub fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            self.frame(egui::Modifiers::NONE, events);
        }
    }

    /// Run one frame over `events` with `modifiers` held as the frame begins.
    pub fn frame(&mut self, modifiers: egui::Modifiers, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        self.commands.extend(out.platform_output.commands);
        self.answered.extend_from_slice(self.app.keys_answered());
    }

    /// Press a binding's keystrokes as a keyboard brings them, a frame a
    /// stroke, then settle. [`Window::answered`] holds what they performed.
    pub fn press(&mut self, keystrokes: &'static str) {
        self.answered.clear();
        for (modifiers, events) in events_for(keystrokes) {
            self.frame(modifiers, events);
        }
        self.settle();
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

/// The egui key a registry key token names.
fn egui_key(token: &str) -> egui::Key {
    let named = match token {
        "space" => Some(egui::Key::Space),
        "escape" => Some(egui::Key::Escape),
        "enter" => Some(egui::Key::Enter),
        "tab" => Some(egui::Key::Tab),
        "backspace" => Some(egui::Key::Backspace),
        "delete" => Some(egui::Key::Delete),
        "left" => Some(egui::Key::ArrowLeft),
        "right" => Some(egui::Key::ArrowRight),
        "up" => Some(egui::Key::ArrowUp),
        "down" => Some(egui::Key::ArrowDown),
        "home" => Some(egui::Key::Home),
        "end" => Some(egui::Key::End),
        "pageup" => Some(egui::Key::PageUp),
        "pagedown" => Some(egui::Key::PageDown),
        _ => egui::Key::from_name(token).or_else(|| egui::Key::from_name(&token.to_uppercase())),
    };
    named.unwrap_or_else(|| panic!("no egui key for the registry's `{token}`"))
}

/// The frames a keyboard brings for a binding's keystrokes: one a stroke, each
/// with its key event and, for a printable stroke with no command, control or
/// alt held, the text it types. The modifiers are the frame's as well as the
/// event's, which the copy chord's handler reads.
pub fn events_for(keystrokes: &'static str) -> Vec<(egui::Modifiers, Vec<egui::Event>)> {
    brightfield_keys::parse_keystrokes(keystrokes)
        .into_iter()
        .map(|stroke| {
            let mut modifiers = egui::Modifiers::NONE;
            if stroke.command {
                modifiers |= egui::Modifiers::COMMAND;
                if cfg!(target_os = "macos") {
                    modifiers |= egui::Modifiers::MAC_CMD;
                }
            }
            if stroke.control {
                modifiers |= egui::Modifiers::CTRL;
            }
            if stroke.alt {
                modifiers |= egui::Modifiers::ALT;
            }
            if stroke.shift {
                modifiers |= egui::Modifiers::SHIFT;
            }
            let mut events = vec![chord(modifiers, egui_key(stroke.key))];
            let typed = !(stroke.command || stroke.control || stroke.alt);
            let text = match stroke.key {
                "space" => Some(" ".to_owned()),
                key if key.chars().count() == 1 => Some(if stroke.shift {
                    key.to_uppercase()
                } else {
                    key.to_owned()
                }),
                _ => None,
            };
            if let (true, Some(text)) = (typed, text) {
                events.push(egui::Event::Text(text));
            }
            (modifiers, events)
        })
        .collect()
}

/// **Put the window where `need` is met**, where a key or a call can: a
/// selection applied, a column kept, a table or a join selected in the model.
/// A need the scene itself meets or does not is left as it stands. Answers
/// whether the window now meets the need, as far as this arranged it.
pub fn arrange(win: &mut Window, need: Need) -> bool {
    match need {
        Need::Sources(_) | Need::Column(_) | Need::Grant(_) => true,
        Need::Selected(selected) => match selected {
            Selected::ModelTable => {
                let Some(pane) = win.app.model_pane_mut() else {
                    return false;
                };
                let Some(card) = pane.layout().cards.first() else {
                    return false;
                };
                let dataset = card.dataset.clone();
                pane.focus.selected = Some(Selection::Dataset(dataset));
                true
            }
            Selected::ModelJoin => {
                let Some(pane) = win.app.model_pane_mut() else {
                    return false;
                };
                let Some(edge) = pane.layout().edges.first() else {
                    return false;
                };
                let relationship = edge.relationship.clone();
                pane.focus.selected = Some(Selection::Relationship(relationship));
                true
            }
            Selected::Protocol
            | Selected::Graph
            | Selected::Chart
            | Selected::Rows
            | Selected::GridCell
            | Selected::Asset
            | Selected::Step
            | Selected::Model => true,
        },
        Need::Condition(condition) => match condition {
            Condition::SelectionHeld => hold_a_selection(win),
            Condition::EditToTakeBack | Condition::UnsavedWork => keep_a_column(win),
            Condition::ProtocolToRun
            | Condition::ProtocolToSave
            | Condition::Trusted
            | Condition::TakesSources
            | Condition::FileToReload
            | Condition::ShelfDrawn => true,
        },
    }
}

/// Apply a selection to the first plot of a chart with a live session.
fn hold_a_selection(win: &mut Window) -> bool {
    use brightfield_engine::coordinator::Interaction;
    use brightfield_engine::SqlPredicate;
    use brightfield_spec::analysis::ComponentPath;

    if win.app.chart_doc().selection_active() {
        return true;
    }
    if win.app.front_door_is_live() || !win.app.chart_doc().is_live() {
        return false;
    }
    win.app
        .chart_doc_mut()
        .apply_interaction(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath("root/hconcat[0]".to_string()),
            predicate: SqlPredicate::Expr("1 = 1".to_string()),
        })
        && win.app.chart_doc().selection_active()
}

/// Keep another column on x from the shelf, by its keys, and hand the keys
/// back to the pane: `e`, `x`, a step of the list's cursor, `Enter`, then
/// `Esc` until the band lets go.
fn keep_a_column(win: &mut Window) -> bool {
    if win.app.chart_doc().last_shelf_edit().is_some() {
        return true;
    }
    if win.app.front_door_is_live()
        || win.app.shelf_band().is_none()
        || !win.app.focus_pane(PaneKey::new(CHART))
    {
        return false;
    }
    win.settle();
    let cursor = |win: &Window| {
        win.app
            .protocol_model()
            .column_list()
            .and_then(|list| list.cursor().map(str::to_owned))
    };
    win.press("e");
    win.press("x");
    let opened_on = cursor(win);
    for step in ["j", "k"] {
        if cursor(win) != opened_on {
            break;
        }
        win.press(step);
    }
    if cursor(win).is_none() || cursor(win) == opened_on {
        return false;
    }
    win.press("enter");
    for _ in 0..4 {
        if !win.app.shelf_holds_keys() {
            break;
        }
        win.press("escape");
    }
    win.app.chart_doc().last_shelf_edit().is_some()
}

/// **A window in which `longname` can act**: the first of the scenes in which,
/// once each of the command's needs is arranged, the window reads it available.
/// Panics naming the command when no scene does.
pub fn scene_for(longname: &'static str) -> Window {
    let reg = brightfield_keys::registry();
    let verb = reg
        .iter()
        .find(|v| v.longname == longname)
        .unwrap_or_else(|| panic!("{longname} is not in the registry"));
    let mut scenes: Vec<fn() -> Window> = vec![door, data_file, crosswalk, own_files, || {
        live_chart("crossfilter.yaml")
    }];
    // The scene a need names is tried first, which saves opening the ones
    // before it; the order decides nothing else, since a scene is taken on
    // what the window reads.
    let first = verb.needs.iter().find_map(|r| match r.need {
        Need::Selected(Selected::Graph) => Some(2),
        Need::Selected(Selected::Model | Selected::ModelTable | Selected::ModelJoin) => Some(3),
        Need::Condition(Condition::SelectionHeld) => Some(4),
        _ => None,
    });
    if let Some(first) = first {
        scenes.swap(0, first);
    }
    let mut said = Vec::new();
    for scene in scenes {
        let mut win = scene();
        for requirement in verb.needs {
            arrange(&mut win, requirement.need);
        }
        win.settle();
        match win.app.availability_of(longname) {
            Some(Availability::Available) => return win,
            other => said.push(format!("{other:?}")),
        }
    }
    panic!("no scene makes {longname} available: {}", said.join("; "));
}

/// **Give the keys to where `context`'s bindings are read**, in a window
/// [`scene_for`] chose for `longname`. Global keys are read wherever the keys
/// are; the Workspace's are the canvas's, so the hero's pane takes focus when a
/// chart holds the canvas.
pub fn hold_keys(win: &mut Window, context: BindingContext, _longname: &'static str) {
    match context {
        BindingContext::Global => {}
        BindingContext::Workspace => {
            let chart = !win.app.front_door_is_live()
                && !win.app.graph_on_canvas()
                && win.app.model_pane().is_none();
            if chart {
                win.app.focus_pane(PaneKey::new(CHART));
                win.settle();
            }
        }
        other => panic!("{other:?} is not a context this support holds the keys for"),
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
