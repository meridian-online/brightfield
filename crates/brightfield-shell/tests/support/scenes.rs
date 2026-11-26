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
use brightfield_shell::protocol::{SpineRole, OUTLINE};
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::LEDGER_RAIL;
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
    /// The commands the frames sent the window itself, oldest first: where a
    /// drag that moves it or a double-click that zooms it shows.
    pub viewport_commands: Vec<egui::ViewportCommand>,
    /// The verbs a key performed since the last [`Window::press`] began, in
    /// the order their handlers took them.
    pub answered: Vec<&'static str>,
    /// The verbs the window applied since a test last cleared this, oldest
    /// first.
    pub applied: Vec<&'static str>,
    /// The table of the data file the scene opened, when it opened one.
    pub table: Option<String>,
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
                .running_with(Some(runner()))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    scratch().join("versions"),
                ))),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            commands: Vec::new(),
            viewport_commands: Vec::new(),
            answered: Vec::new(),
            applied: Vec::new(),
            table: None,
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
        // egui runs a frame again when a pass asks to discard its layout, and hands the second
        // pass no events; the window clears what it answered and applied as each pass begins, so
        // each pass's is read before the next runs. Read only after the frame, a key the first
        // pass answered was lost whenever a layout change landed on its frame.
        let (app, answered, applied) = (&mut self.app, &mut self.answered, &mut self.applied);
        let out = self.ctx.run_ui(raw, |ui| {
            app.draw(ui);
            answered.extend_from_slice(app.keys_answered());
            applied.extend_from_slice(app.verbs_applied());
        });
        self.commands.extend(out.platform_output.commands);
        if let Some(root) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            self.viewport_commands.extend(root.commands.iter().cloned());
        }
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

    /// Choose `longname` as a menu item does, through the window's queue, and
    /// settle. [`Window::applied`] holds what the window then performed.
    pub fn choose(&mut self, longname: &'static str) {
        self.applied.clear();
        self.app.open_queue().ask_verb(longname);
        self.settle();
    }

    /// A click at `at`: the pointer moved there, pressed and released, and the
    /// frames after it settled.
    pub fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![vec![egui::Event::PointerMoved(at)]]);
        self.run(vec![vec![button(true)], vec![button(false)]]);
        self.settle();
    }

    /// Put the graph on the canvas by the chip in the spine's head, if it is
    /// not there. Answers whether the graph holds the canvas.
    pub fn graph_to_canvas(&mut self) -> bool {
        if self.app.graph_on_canvas() {
            return true;
        }
        let Some(chip) = self.app.spine_rows().iter().find_map(|r| r.chip) else {
            return false;
        };
        self.click(chip.rect.center());
        self.app.graph_on_canvas()
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
        Need::Sources(_) | Need::Grant(_) => true,
        Need::Column(_) => select_a_column(win),
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
            Selected::GridCell => click_a_grid_cell(win),
            Selected::Asset => select_an_asset(win),
            Selected::Step => select_a_removable_step(win),
            Selected::Protocol
            | Selected::Graph
            | Selected::Chart
            | Selected::Rows
            | Selected::NodeRows
            | Selected::Model => true,
        },
        Need::Condition(condition) => match condition {
            Condition::SelectionHeld => hold_a_selection(win),
            Condition::EditToTakeBack | Condition::UnsavedWork => keep_a_column(win),
            Condition::ProtocolToRun
            | Condition::ProtocolToSave
            | Condition::ProtocolFolder
            | Condition::Trusted
            | Condition::TakesSources
            | Condition::FileToReload
            | Condition::ShelfDrawn
            | Condition::ChartReadsFocus => true,
        },
    }
}

/// Click the first column row the Outline drew under a table, which puts the
/// Outline's cursor on it and gives the Outline the keys.
fn select_a_column(win: &mut Window) -> bool {
    if win.app.protocol_model().outline_column().is_some() {
        return true;
    }
    let row = win
        .app
        .spine_rows()
        .iter()
        .find(|r| r.role == SpineRole::Column && r.depth == 1)
        .cloned();
    let Some(row) = row else {
        return false;
    };
    win.click(row.name_rect.center());
    win.app.protocol_model().outline_column().is_some()
}

/// Click a cell of the grid's first drawn row, which puts the cursor on it and
/// gives the grid the keys.
fn click_a_grid_cell(win: &mut Window) -> bool {
    if win.app.front_door_is_live() {
        return false;
    }
    let Some(drawn) = win.app.chart_doc().grid_drawn().cloned() else {
        return false;
    };
    let (Some((_, head, head_clip)), Some((_, line, line_clip))) =
        (drawn.header_cells.first(), drawn.row_cells.first())
    else {
        return false;
    };
    let at = egui::pos2(
        head.intersect(*head_clip).center().x,
        line.intersect(*line_clip).center().y,
    );
    win.click(at);
    win.app.chart_doc().grid_cursor_on_cell()
}

/// Select an asset on the spine by a click on its row.
fn select_an_asset(win: &mut Window) -> bool {
    let row = win
        .app
        .spine_rows()
        .iter()
        .find(|r| r.role == SpineRole::Asset)
        .cloned();
    let Some(row) = row else {
        return false;
    };
    win.click(row.rect.center());
    true
}

/// Keep a statement as a step, save it, and select what it makes on the spine
/// with the graph on the canvas: a step `d d` can remove.
fn select_a_removable_step(win: &mut Window) -> bool {
    if win.app.protocol_model().removable_step().is_some() {
        return true;
    }
    let Some(table) = win.table.clone() else {
        return false;
    };
    win.press("space");
    win.type_text(&format!(
        "select house_age, count(*) as n from {table} group by house_age"
    ));
    if !win
        .app
        .finish_typed_reads(std::time::Duration::from_secs(60))
    {
        return false;
    }
    win.press("cmd-enter");
    let Some(step) = win
        .app
        .chart_doc()
        .typed_steps()
        .last()
        .map(|s| s.name.clone())
    else {
        return false;
    };
    win.press("cmd-s");
    if !win.graph_to_canvas() {
        return false;
    }
    let rows = win.app.spine_rows().to_vec();
    let made = rows
        .iter()
        .position(|r| r.role == SpineRole::Step && r.label == step)
        .and_then(|at| rows[at + 1..].iter().find(|r| r.role == SpineRole::Asset));
    let Some(made) = made else {
        return false;
    };
    win.click(made.rect.center());
    win.graph_to_canvas();
    win.app.protocol_model().removable_step().is_some()
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
pub fn scene_for(longname: &'static str, context: BindingContext) -> Window {
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
    // A pane's own keys are read in a window that draws the pane: the graph
    // for the Protocol's, a data file's dashboard for the rest.
    let first = first.or(match context {
        BindingContext::Global | BindingContext::Workspace => None,
        BindingContext::Protocol => Some(2),
        BindingContext::Shelf
        | BindingContext::Grid
        | BindingContext::Versions
        | BindingContext::Browser
        | BindingContext::Editor
        | BindingContext::SpinePick => Some(1),
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
/// [`scene_for`] chose for `longname`. Answers nothing: a window that could
/// not be brought there fails the sweep's own assertion, which names the key.
pub fn hold_keys(win: &mut Window, context: BindingContext, longname: &'static str) {
    let chart = !win.app.front_door_is_live()
        && !win.app.graph_on_canvas()
        && win.app.model_pane().is_none();
    match context {
        // Global keys are read wherever the keys are.
        BindingContext::Global => {}
        // The Workspace's are the canvas's: the hero's pane takes focus when a
        // chart holds the canvas.
        BindingContext::Workspace => {
            if chart {
                win.app.focus_pane(PaneKey::new(CHART));
                win.settle();
            }
        }
        // The Protocol's are the graph's while it holds the canvas, and the
        // Outline's otherwise. The three that put a column on a channel act
        // from the Outline, over a chart.
        BindingContext::Protocol => {
            let from_outline = longname.starts_with("put-column-on-");
            if !from_outline {
                win.graph_to_canvas();
            }
            if !win.app.graph_on_canvas() {
                win.app.focus_pane(PaneKey::new(OUTLINE));
                win.settle();
            }
        }
        // The Shelf's: the hero focused, `e` for the band, `x` for the list of
        // x's columns, and the list turned to its settings for the verb that
        // acts on a settings row.
        BindingContext::Shelf => {
            win.app.focus_pane(PaneKey::new(CHART));
            win.settle();
            win.press("e");
            win.press("x");
            if longname == "set-shelf-setting-to-auto" {
                win.press("tab");
            }
        }
        // The Grid's: a cell clicked.
        BindingContext::Grid => {
            click_a_grid_cell(win);
        }
        // The Versions panel's: a version saved, the ledger turned to the
        // panel, a press inside it, and its cursor on a row.
        BindingContext::Versions => {
            keep_a_column(win);
            win.press("cmd-s");
            if let Some(name) = win.app.rail_name_rect(LEDGER_RAIL, VERSIONS_AT) {
                win.click(name.center());
            }
            if let Some(rail) = win.app.region_rect(LEDGER_RAIL) {
                win.click(rail.center());
            }
            win.press("j");
        }
        // The Browser's: the sources browser open, from the Outline's `A`.
        BindingContext::Browser => {
            win.app.focus_pane(PaneKey::new(OUTLINE));
            win.settle();
            win.press("shift-a");
        }
        // The spine's pick is open while an operation asks for a second table,
        // which the scenes here start none of: its keys are swept beside the
        // pick's own tests (`tests/spine_pick.rs`,
        // `every_spine_pick_key_is_answered`).
        BindingContext::SpinePick => {}
        // The Editor's: the ledger turned to the editor and a click in its text.
        BindingContext::Editor => {
            if let Some(name) = win.app.rail_name_rect(LEDGER_RAIL, EDITOR_AT) {
                win.click(name.center());
            }
            win.app
                .focus_pane(PaneKey::new(brightfield_shell::editor::EDITOR));
            win.settle();
            if let Some(rail) = win.app.region_rect(LEDGER_RAIL) {
                let body_top = rail.top() + brightfield_workbench::chrome::rail_selector_height();
                // Past the gutter's line numbers, which pass no click to a widget.
                win.click(egui::pos2(rail.left() + 150.0, body_top + 32.0));
            }
        }
    }
}

/// Where the ledger strip names the editor and the Versions panel.
const EDITOR_AT: usize = 3;
const VERSIONS_AT: usize = 4;

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
    win.table = Some(name);
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
