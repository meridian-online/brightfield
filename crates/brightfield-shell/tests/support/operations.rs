//! The harness the operation tests drive a window with.
//!
//! Each [`OpWindow`] holds a headless window over a copy of a fixture in a
//! temp folder of its own, since Save writes the Protocol beside the data
//! file. arc is the `brightfield-shell` binary cargo built for the suite, and
//! its history and database stores are pointed at a temp root for the test
//! binary, so a test leaves no entry in the developer's own.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use brightfield_shell::app::EditorDrawn;
use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{SpineRole, SpineRowDrawn};
use brightfield_shell::run::Runner;
use brightfield_shell::startup::default_layout;
use brightfield_shell::typed_sql::TypedStep;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::LEDGER_RAIL;
use tempfile::TempDir;

/// The suite's runner: the binary cargo built for it, which becomes arc.
pub fn runner() -> Runner {
    arc_stores();
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// Point arc's history and database stores at a temp root of this test
/// binary's, once, before the first child is started.
pub fn arc_stores() -> &'static Path {
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::Builder::new()
            .prefix("bf-operations-")
            .tempdir()
            .expect("a temp root for arc's stores");
        for (variable, sub) in [("ARCFORM_DB_DIR", "db"), ("ARCFORM_HISTORY_DIR", "history")] {
            let dir = root.path().join(sub);
            std::fs::create_dir_all(&dir).expect("an arc store directory");
            std::env::set_var(variable, &dir);
        }
        root
    })
    .path()
}

/// Copy the folder `from` into `to`, its subfolders with it.
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("the folder reads") {
        let entry = entry.expect("an entry reads");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("its type reads").is_dir() {
            std::fs::create_dir_all(&target).expect("the subfolder is made");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("the file copies");
        }
    }
}

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn collect_text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => out.push((
            text.galley.text().to_owned(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, out);
            }
        }
        _ => {}
    }
}

/// A headless window over a copy of a fixture, in a temp folder of its own.
pub struct OpWindow {
    pub app: MeridianApp,
    pub ctx: egui::Context,
    /// The texts the last frame drew.
    pub texts: Vec<String>,
    /// Where each of them was drawn.
    pub text_rects: Vec<(String, egui::Rect)>,
    /// The keys the last [`Self::run`] answered, read after each pass of its frame.
    pub answered: Vec<&'static str>,
    fixture: String,
    time: f64,
    tmp: std::rc::Rc<TempDir>,
}

impl OpWindow {
    /// Copy `tests/data/<fixture>` into a temp folder and open it, with the
    /// suite's runner.
    pub fn on_copy_of(fixture: &str) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("bf-operation-")
            .tempdir()
            .expect("a temp folder for the fixture");
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(fixture);
        std::fs::copy(&from, tmp.path().join(fixture)).expect("the fixture copies");
        Self::open_in(std::rc::Rc::new(tmp), fixture)
    }

    /// [`Self::on_copy_of`], with `prepare` run on the folder before the window
    /// opens it: how a test records steps into the Protocol first.
    pub fn on_copy_prepared(fixture: &str, prepare: impl FnOnce(&Path)) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("bf-operation-")
            .tempdir()
            .expect("a temp folder for the fixture");
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(fixture);
        std::fs::copy(&from, tmp.path().join(fixture)).expect("the fixture copies");
        prepare(tmp.path());
        Self::open_in(std::rc::Rc::new(tmp), fixture)
    }

    /// A window over the chart spec `examples/<fixture>`, with a live session
    /// behind it and no Protocol of its own: its Save has no spec to write.
    pub fn on_live_chart(fixture: &str) -> Self {
        use_own_preview_gate_on_this_thread();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(fixture);
        let (live, composed) =
            brightfield_shell::pipeline::live_spec(path.to_str().expect("a utf-8 path"))
                .unwrap_or_else(|e| panic!("{fixture} loads live: {e}"));
        let mut boot = Boot::charts(composed);
        boot.live = Some(live);
        boot.spec_path = Some(path);
        let tmp = tempfile::Builder::new()
            .prefix("bf-operation-")
            .tempdir()
            .expect("a temp folder");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            texts: Vec::new(),
            text_rects: Vec::new(),
            answered: Vec::new(),
            fixture: fixture.to_string(),
            time: 0.0,
            tmp: std::rc::Rc::new(tmp),
        };
        win.settle();
        win
    }

    /// A window over a Protocol Brightfield made of a copy of
    /// `tests/data/<fixture>`, as opening the file from the picker makes one:
    /// a Protocol that takes sources. Its folder is under a Protocols folder of
    /// this test binary's own.
    pub fn on_own_protocol(fixture: &str) -> Self {
        static PROTOCOLS: OnceLock<TempDir> = OnceLock::new();
        let protocols = PROTOCOLS.get_or_init(|| {
            let root = tempfile::Builder::new()
                .prefix("bf-protocols-")
                .tempdir()
                .expect("a Protocols folder");
            std::env::set_var(
                brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
                root.path(),
            );
            root
        });
        let _ = protocols;
        use_own_preview_gate_on_this_thread();
        let tmp = tempfile::Builder::new()
            .prefix("bf-operation-")
            .tempdir()
            .expect("a temp folder for the fixture");
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(fixture);
        let data = tmp.path().join(fixture);
        std::fs::copy(&from, &data).expect("the fixture copies");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(Boot::empty(), default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            texts: Vec::new(),
            text_rects: Vec::new(),
            answered: Vec::new(),
            fixture: fixture.to_string(),
            time: 0.0,
            tmp: std::rc::Rc::new(tmp),
        };
        win.settle();
        win.app
            .open_queue()
            .push(brightfield_shell::open_queue::OpenRequest::Path(data));
        win.settle();
        assert!(
            win.app.own_protocol().is_some(),
            "the open made no Protocol of Brightfield's own"
        );
        win
    }

    /// A window over a copy of the Protocol folder `tests/data/<folder>`, opened
    /// by its manifest: a Protocol Brightfield did not make when the folder is
    /// one written by hand.
    pub fn on_protocol_folder(folder: &str) -> Self {
        use_own_preview_gate_on_this_thread();
        let tmp = tempfile::Builder::new()
            .prefix("bf-operation-")
            .tempdir()
            .expect("a temp folder for the Protocol");
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(folder);
        copy_tree(&from, tmp.path());
        let manifest = tmp.path().join("arcform.yaml");
        let boot = Boot::open(
            &manifest.to_string_lossy(),
            brightfield_protocol::layout::Flow::Vertical,
            None,
        )
        .expect("the Protocol opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            texts: Vec::new(),
            text_rects: Vec::new(),
            answered: Vec::new(),
            fixture: "arcform.yaml".to_string(),
            time: 0.0,
            tmp: std::rc::Rc::new(tmp),
        };
        win.settle();
        win
    }

    /// A second window on the same folder, as reopening the file gives.
    pub fn reopen(&self) -> Self {
        Self::open_in(self.tmp.clone(), &self.fixture)
    }

    fn open_in(tmp: std::rc::Rc<TempDir>, fixture: &str) -> Self {
        use_own_preview_gate_on_this_thread();
        let data = tmp.path().join(fixture);
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the fixture opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            texts: Vec::new(),
            text_rects: Vec::new(),
            answered: Vec::new(),
            fixture: fixture.to_string(),
            time: 0.0,
            tmp,
        };
        win.settle();
        win
    }

    /// The folder the data file and its Protocol are in.
    pub fn dir(&self) -> &Path {
        self.tmp.path()
    }

    /// The table the data file loads as.
    pub fn table(&self) -> String {
        self.app
            .protocol_model()
            .source()
            .map(|source| source.name.clone())
            .expect("a data file's Protocol has a source")
    }

    pub fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(self.time),
            modifiers,
            events,
            ..Default::default()
        };
        // egui runs a frame again when a pass asks to discard its layout, and hands the second
        // pass no events; the window clears what keys it answered as each pass begins, so what
        // the first pass answered is read before the second runs.
        let (app, answered) = (&mut self.app, &mut self.answered);
        answered.clear();
        let out = self.ctx.run_ui(raw, |ui| {
            app.draw(ui);
            answered.extend_from_slice(app.keys_answered());
        });
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        self.texts = texts.iter().map(|(text, _)| text.clone()).collect();
        self.text_rects = texts;
    }

    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    pub fn key(&mut self, key: egui::Key) {
        self.key_with(key, egui::Modifiers::NONE);
    }

    pub fn key_with(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.run(vec![press(key, modifiers)]);
        self.settle();
    }

    /// Type a letter as a keyboard does: the key, and the text it makes.
    pub fn letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![
            press(key, egui::Modifiers::NONE),
            egui::Event::Text(text.to_owned()),
        ]);
        self.settle();
    }

    /// Type `text` as a keyboard does, a key press and its text for each
    /// letter, so a handler of a bare key sees what a person's typing sends.
    pub fn type_keys(&mut self, text: &str) {
        for c in text.chars() {
            match egui::Key::from_name(&c.to_ascii_uppercase().to_string()) {
                Some(key) => self.letter(key, &c.to_string()),
                None => self.type_text(&c.to_string()),
            }
        }
    }

    pub fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    pub fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            egui::Event::PointerMoved(at),
            button(true),
            button(false),
        ]);
        self.settle();
    }

    /// What the window's banners say, for a failed assertion.
    pub fn banners(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The steps kept and not yet saved.
    pub fn kept(&self) -> Vec<TypedStep> {
        self.app.chart_doc().typed_steps().to_vec()
    }

    /// Save, as the Save verb does. Answers what the Save said.
    pub fn save(&mut self) -> Result<PathBuf, String> {
        let said = self
            .app
            .save_protocol(&self.ctx)
            .expect("the window has a Protocol to save");
        self.settle();
        said
    }

    fn manifest(&self) -> brightfield_protocol::Manifest {
        let text =
            std::fs::read_to_string(self.dir().join("arcform.yaml")).expect("the manifest reads");
        brightfield_protocol::parse_manifest_str(&text).expect("the manifest parses")
    }

    /// The names of the manifest's steps, in order; empty before a first Save.
    pub fn manifest_steps(&self) -> Vec<String> {
        if !self.dir().join("arcform.yaml").exists() {
            return Vec::new();
        }
        self.manifest().steps.into_iter().map(|s| s.name).collect()
    }

    /// The text of the model the manifest's step `step` names.
    pub fn model_of(&self, step: &str) -> String {
        let manifest = self.manifest();
        let found = manifest
            .steps
            .iter()
            .find(|s| s.name == step)
            .unwrap_or_else(|| panic!("no step {step} in the manifest"));
        std::fs::read_to_string(self.dir().join(found.sql.as_ref().expect("a sql step")))
            .expect("the model reads")
    }

    /// Each file under the folder with its bytes, in path order.
    pub fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![self.dir().to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).expect("the folder reads") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = std::fs::read(&path).expect("a file reads");
                    out.push((path, bytes));
                }
            }
        }
        out.sort();
        out
    }

    pub fn rows(&self) -> Vec<SpineRowDrawn> {
        self.app.spine_rows().to_vec()
    }

    pub fn spine_labels(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(|r| format!("{:?} {}", r.role, r.label))
            .collect()
    }

    /// Put the grid of the table the step `step` makes on the canvas, by a
    /// click on the `grid` row under its asset row, and wait for its preview.
    pub fn show_grid_of(&mut self, step: &str) {
        let rows = self.rows();
        let at = rows
            .iter()
            .position(|row| row.role == SpineRole::Asset && row.label == step)
            .unwrap_or_else(|| panic!("no asset row {step}: {:?}", self.spine_labels()));
        let grid = rows
            .get(at + 1)
            .filter(|row| row.role == SpineRole::View && row.label == "grid")
            .unwrap_or_else(|| panic!("no grid row under {step}: {:?}", self.spine_labels()));
        self.click(grid.rect.center());
        self.wait_for_preview();
    }

    /// Draw frames until the preview the grid shows is no longer being read.
    pub fn wait_for_preview(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self
            .app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.is_reading())
        {
            assert!(Instant::now() < deadline, "the preview never arrived");
            std::thread::sleep(Duration::from_millis(5));
            self.run(Vec::new());
        }
        self.settle();
    }

    /// The columns of the preview page the grid holds.
    pub fn preview_columns(&self) -> Vec<String> {
        self.app
            .chart_doc()
            .grid_preview()
            .and_then(|p| p.page())
            .map(|page| page.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn step_ran(&self, step: &str) -> bool {
        self.app.protocol_model().step_ran(step)
    }

    /// Select what the step `step` makes on the spine, with the graph on the
    /// canvas, where the spine's `d d` and the palette's `remove-step` act.
    pub fn select_made_by(&mut self, step: &str) {
        self.graph_to_canvas();
        let rows = self.rows();
        let at = rows
            .iter()
            .position(|r| r.role == SpineRole::Step && r.label == step)
            .unwrap_or_else(|| panic!("no step row {step}: {:?}", self.spine_labels()));
        let made = rows[at + 1..]
            .iter()
            .find(|r| r.role == SpineRole::Asset)
            .unwrap_or_else(|| panic!("no asset row under {step}"))
            .clone();
        self.click(made.rect.center());
        self.graph_to_canvas();
        assert_eq!(
            self.app
                .protocol_model()
                .selected_step()
                .map(String::as_str),
            Some(step),
            "the click selected what {step} makes"
        );
    }

    pub fn graph_to_canvas(&mut self) {
        if self.app.graph_on_canvas() {
            return;
        }
        let chip = self
            .app
            .spine_rows()
            .iter()
            .find_map(|r| r.chip)
            .expect("the spine's head drew its graph chip");
        self.click(chip.rect.center());
        assert!(self.app.graph_on_canvas(), "the chip put the graph up");
    }

    /// `d d`, the spine's remove-step.
    pub fn d_d(&mut self) {
        self.letter(egui::Key::D, "d");
        self.letter(egui::Key::D, "d");
    }

    /// Bring the Editor tab to the front of the ledger rail, and answer what
    /// it drew.
    pub fn editor(&mut self) -> EditorDrawn {
        let tab = self
            .app
            .rail_name_rect(LEDGER_RAIL, 3)
            .expect("the ledger rail drew its editor tab")
            .center();
        self.click(tab);
        self.settle();
        self.app
            .chart_doc()
            .editor_drawn()
            .cloned()
            .unwrap_or_else(|| panic!("the Editor drew no text: {:?}", self.texts))
    }

    /// The text the Editor shows.
    pub fn editor_text(&mut self) -> String {
        self.editor().text
    }

    /// Click the middle of the text drawn that holds `text`.
    pub fn click_text(&mut self, text: &str) {
        // The last drawn is the one on top: a menu's row over the ribbon's command of the same
        // name, which the click lands on.
        let at = self
            .text_rects
            .iter()
            .rev()
            .find(|(drawn, _)| drawn.contains(text))
            .unwrap_or_else(|| panic!("no text holding {text:?} was drawn: {:?}", self.texts))
            .1
            .center();
        self.click(at);
    }

    /// Make `gesture` on the `part` of `column`'s header.
    pub fn header(
        &mut self,
        column: &str,
        part: brightfield_shell::operation_ui::HeaderPart,
        gesture: brightfield_shell::operation_ui::header::Gesture,
    ) {
        use brightfield_shell::operation_ui::header::Gesture;
        use brightfield_shell::operation_ui::HeaderPart;
        let drawn = self.app.chart_doc().header_drawn().clone();
        let at = if part == HeaderPart::Menu {
            drawn
                .cells
                .iter()
                .find(|(name, _)| name == column)
                .map(|(_, rect)| rect.center())
        } else {
            drawn
                .parts
                .iter()
                .find(|(name, p, _)| name == column && *p == part)
                .map(|(_, _, rect)| rect.center())
        }
        .unwrap_or_else(|| panic!("the header drew no {part:?} of {column}: {drawn:?}"));
        let button = |button, pressed| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        let primary = egui::PointerButton::Primary;
        let events = match gesture {
            Gesture::Click => vec![button(primary, true), button(primary, false)],
            Gesture::DoubleClick => vec![
                button(primary, true),
                button(primary, false),
                button(primary, true),
                button(primary, false),
            ],
            Gesture::Secondary => vec![
                button(egui::PointerButton::Secondary, true),
                button(egui::PointerButton::Secondary, false),
            ],
        };
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(events);
        self.settle();
    }

    /// Click the new-column cell after the last header.
    pub fn header_new_column(&mut self) {
        let at = self
            .app
            .chart_doc()
            .header_drawn()
            .new_column
            .expect("the header drew its new-column cell")
            .center();
        self.click(at);
    }

    /// Open `column`'s header menu and choose the row holding `words`.
    pub fn header_menu(&mut self, column: &str, words: &str) {
        self.header(
            column,
            brightfield_shell::operation_ui::HeaderPart::Menu,
            brightfield_shell::operation_ui::header::Gesture::Secondary,
        );
        self.click_text(words);
    }

    /// The text in the field an operation asks with in place.
    pub fn field_text(&self) -> Option<String> {
        self.app
            .chart_doc()
            .header_drawn()
            .field
            .as_ref()
            .map(|field| field.text.clone())
    }

    /// What that field offers under it.
    pub fn suggestions(&self) -> Vec<String> {
        self.app
            .chart_doc()
            .header_drawn()
            .field
            .as_ref()
            .map(|field| field.suggestions.clone())
            .unwrap_or_default()
    }

    /// Put the loaded table's grid in front, focus it, and put the cursor on
    /// the first row of `column`.
    pub fn cursor_on(&mut self, column: &str) {
        let table = self.table();
        self.show_grid_of(&table);
        self.focus_grid();
        let header = std::fs::read_to_string(self.dir().join(&self.fixture))
            .expect("the fixture reads")
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let col = header
            .split(',')
            .position(|name| name == column)
            .unwrap_or_else(|| panic!("no column {column} in {header}"));
        self.app.chart_doc_mut().grid_cursor =
            Some(brightfield_shell::data_grid::GridCursor { row: 0, col });
        self.settle();
    }

    /// Give the grid pane the keys.
    pub fn focus_grid(&mut self) {
        assert!(
            self.app.focus_pane(brightfield_workbench::PaneKey::new(
                brightfield_shell::data_grid::DATA
            )),
            "the grid pane is in the window"
        );
        self.settle();
    }

    /// Press a registry key token: a bare letter or sign, `shift-<letter>`,
    /// or `cmd-s`.
    pub fn press(&mut self, token: &str) {
        let letter = |c: char| {
            egui::Key::from_name(&c.to_ascii_uppercase().to_string())
                .unwrap_or_else(|| panic!("no key for {c}"))
        };
        match token {
            "cmd-s" => self.key_with(egui::Key::S, egui::Modifiers::COMMAND),
            "-" => self.letter(egui::Key::Minus, "-"),
            "=" => self.letter(egui::Key::Equals, "="),
            "&" => self.type_text("&"),
            shifted if shifted.starts_with("shift-") => {
                let c = shifted.chars().last().expect("a letter");
                self.run(vec![
                    press(letter(c), egui::Modifiers::SHIFT),
                    egui::Event::Text(c.to_ascii_uppercase().to_string()),
                ]);
                self.settle();
            }
            bare => {
                let c = bare.chars().next().expect("a key");
                self.letter(letter(c), bare);
            }
        }
    }

    /// Choose `verb` from the chart palette: Space, the verb typed, Enter.
    pub fn palette(&mut self, verb: &str) {
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"), "space opened it");
        self.settle();
        self.type_text(verb);
        self.key(egui::Key::Enter);
        self.settle();
    }

    /// Draw frames until the reads answer the draft as it stands.
    pub fn settle_reads(&mut self) {
        self.settle();
        assert!(
            self.app.finish_operation_reads(Duration::from_secs(30)),
            "the draft was never answered"
        );
        self.settle();
    }

    /// What the status band says of an operation: its own sentence, arc's or
    /// DuckDB's refusal of the draft being made, or the foot row's over a
    /// step's preview.
    pub fn said(&self) -> Option<String> {
        self.app
            .operation_said()
            .or_else(|| self.app.operation_refusal())
            .map(str::to_string)
            .or_else(|| self.app.chart_doc().foot_said())
    }

    /// The draft being made, while one is.
    pub fn making(&self) -> Option<brightfield_shell::operation_ui::Draft> {
        self.app.making().map(|making| making.draft.clone())
    }

    /// Whether the grid draws its rows faded.
    pub fn grid_faded(&self) -> bool {
        self.app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.is_faded())
    }

    /// How many values of `column` the preview in the grid counts missing.
    pub fn preview_missing(&self, column: &str) -> Option<u64> {
        self.app
            .chart_doc()
            .grid_preview()
            .and_then(|p| p.missing_of(column))
    }

    /// The columns the grid shows: a preview's, while it shows one.
    pub fn grid_columns(&self) -> Vec<String> {
        self.preview_columns()
    }

    /// The text of the preview's cell at `row` of `column`.
    pub fn grid_cell(&self, row: usize, column: &str) -> Option<String> {
        let page = self.app.chart_doc().grid_preview()?.page()?;
        let at = page.columns.iter().position(|c| c.name == column)?;
        Some(page.rows.get(row)?.get(at)?.full().to_owned())
    }
}
