//! **The sources browser, drawn**: by click as well as by key, its foot naming the keys of the
//! column that holds them, its previews read off the frame thread, and its colours the window's
//! theme in both modes. And the window around it: New Protocol, `A` and Add source… open it, a
//! folder whose tables dovetail proposes joins between opens it on that folder's canvas, from the
//! picker and from a drop alike, a folder with no join is added at once, and what the browser adds is kept in the
//! Protocol with the joins accepted on its canvas.
//!
//! The view's tests draw a [`Sheet`] into a plain context over folders of this file's own, with
//! previews that answer at once or wait for the test. The window's tests run a headless window
//! with the `brightfield-shell` binary cargo built as its runner, and dovetail run by it. The
//! config directory, the Protocols folder and arc's directories point at a scratch folder of this
//! file's before anything reads them, so nothing is written under the home directory.
//!
//! Regenerate the baselines with `UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test
//! sources_browser_view`.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use brightfield_shell::design::{self, Mode};
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::run::Runner;
use brightfield_shell::sources_browser::{Browser, Disk, Effect, Pane, Place, ProposedJoin};
use brightfield_shell::sources_browser_view::{
    self as view, Page, Previews, ProposeJoins, ReadPage, Sheet, Shown,
};
use brightfield_shell::window::{Boot, MeridianApp};

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("bf-sources-browser-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input", "places"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
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

const QUAKES: &str = "id,magnitude,region\n1,4.5,north\n2,5.1,south\n3,3.9,north\n";
const CUSTOMERS: &str = "id,name\n1,Ada\n2,Bo\n3,Cy\n";
const ORDERS: &str = "id,customer_id\n10,1\n11,2\n12,3\n";

/// The folder the view's tests browse, made once and read only: `shop/` holding two CSVs, a
/// file Brightfield does not read, and a CSV.
fn data() -> &'static Path {
    static DATA: OnceLock<PathBuf> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = scratch().join("places").join("data");
        std::fs::create_dir_all(dir.join("shop")).expect("the data folder");
        std::fs::write(dir.join("shop").join("customers.csv"), CUSTOMERS).expect("a CSV");
        std::fs::write(dir.join("shop").join("orders.csv"), ORDERS).expect("a CSV");
        std::fs::write(dir.join("notes.txt"), "not data\n").expect("a file");
        std::fs::write(dir.join("quakes.csv"), QUAKES).expect("a CSV");
        dir
    })
}

/// The page every instant preview answers.
fn quakes_page() -> Page {
    Page {
        columns: vec!["id".into(), "magnitude".into(), "region".into()],
        rows: vec![
            vec!["1".into(), "4.5".into(), "north".into()],
            vec!["2".into(), "5.1".into(), "south".into()],
            vec!["3".into(), "3.9".into(), "north".into()],
        ],
        tables: Vec::new(),
    }
}

fn shop_join() -> ProposedJoin {
    ProposedJoin::new(("orders", "customer_id"), ("customers", "id"))
}

/// Previews that answer at once: every file the quakes page, every folder `joins`.
fn instant(joins: Vec<ProposedJoin>) -> (ReadPage, ProposeJoins) {
    (
        Arc::new(|_: &Path| Ok(quakes_page())),
        Arc::new(move |_: &Path| Ok(joins.clone())),
    )
}

fn previews((read, propose): (ReadPage, ProposeJoins)) -> Previews {
    Previews::new(read, propose)
}

fn press(key: egui::Key) -> egui::Event {
    press_with(key, egui::Modifiers::NONE)
}

fn press_with(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Every shape a frame painted, out of their groups.
fn flat(shapes: Vec<egui::epaint::ClippedShape>) -> Vec<egui::Shape> {
    fn walk(shape: egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
                }
            }
            other => out.push(other),
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(clipped.shape, &mut out);
    }
    out
}

/// The browser drawn on its own into a context, a frame a call.
struct View {
    ctx: egui::Context,
    sheet: Sheet,
    mode: Mode,
    time: f64,
}

impl View {
    /// The browser over [`data`], opened on `at` with `previews`, in `mode`.
    fn over(at: &Path, previews: Previews, mode: Mode) -> Self {
        let browser =
            Browser::open_among(vec![Place::new("Data", data())], data(), Some(at), &Disk);
        let mut view = Self {
            ctx: egui::Context::default(),
            sheet: Sheet::new(browser, previews),
            mode,
            time: 0.0,
        };
        view.frame(Vec::new());
        view
    }

    fn output(&mut self, events: Vec<egui::Event>, step: f64) -> (Effect, egui::FullOutput) {
        self.time += step;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 760.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut effect = Effect::None;
        let (sheet, mode) = (&mut self.sheet, self.mode);
        let out = self.ctx.run_ui(raw, |ui| {
            design::apply(ui.ctx(), mode);
            effect = view::show(ui, sheet, mode);
        });
        (effect, out)
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Effect {
        self.output(events, 1.0).0
    }

    fn key(&mut self, key: egui::Key) -> Effect {
        let effect = self.frame(vec![press(key)]);
        self.frame(Vec::new());
        effect
    }

    fn click(&mut self, at: egui::Pos2) -> Effect {
        let effect = self.frame(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.frame(Vec::new());
        effect
    }

    /// Two clicks a tenth of a second apart: the effects of both frames.
    fn double_click(&mut self, at: egui::Pos2) -> Vec<Effect> {
        let first = self
            .output(
                vec![
                    egui::Event::PointerMoved(at),
                    button_at(at, true),
                    button_at(at, false),
                ],
                1.0,
            )
            .0;
        let second = self
            .output(vec![button_at(at, true), button_at(at, false)], 0.1)
            .0;
        self.frame(Vec::new());
        vec![first, second]
    }

    /// Draw frames until `done` holds of what was drawn, failing after ten seconds.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            assert!(Instant::now() < deadline, "{what}: not drawn in time");
            std::thread::sleep(Duration::from_millis(10));
            self.frame(Vec::new());
        }
    }

    fn under_cursor(&self) -> String {
        self.sheet
            .browser
            .current()
            .map(|e| e.name())
            .unwrap_or_default()
    }
}

#[test]
fn a_click_moves_and_a_double_click_acts_as_enter() {
    let mut v = View::over(
        &data().join("quakes.csv"),
        previews(instant(Vec::new())),
        Mode::Light,
    );
    assert_eq!(v.under_cursor(), "quakes.csv");
    let rows = v.sheet.drawn().rows.clone();
    assert_eq!(rows.len(), 3, "shop/, notes.txt and quakes.csv");

    assert_eq!(v.click(rows[1].center()), Effect::None);
    assert_eq!(v.under_cursor(), "notes.txt", "a click moves the cursor");
    assert_eq!(v.sheet.browser.dir(), data());

    v.double_click(rows[0].center());
    assert_eq!(
        v.sheet.browser.dir(),
        data().join("shop"),
        "a double-click on a folder goes into it, as Enter does"
    );

    let rows = v.sheet.drawn().rows.clone();
    assert_eq!(rows.len(), 2);
    let effects = v.double_click(rows[1].center());
    assert!(
        effects.contains(&Effect::Add(
            vec![data().join("shop").join("orders.csv")],
            Vec::new()
        )),
        "a double-click on a file adds it, as Enter does: {effects:?}"
    );
}

#[test]
fn a_click_on_a_join_toggles_it() {
    let mut v = View::over(
        &data().join("shop"),
        previews(instant(vec![shop_join()])),
        Mode::Light,
    );
    v.until("the canvas's join", |v| !v.sheet.drawn().joins.is_empty());
    assert_eq!(v.sheet.browser.canvas_joins(), &[(shop_join(), false)]);
    let join = v.sheet.drawn().joins[0];

    v.click(join.center());
    assert_eq!(
        v.sheet.browser.canvas_joins(),
        &[(shop_join(), true)],
        "a click accepts the join, as Space on it does"
    );
    assert_eq!(
        v.sheet.browser.focus,
        Pane::Canvas,
        "and the canvas has the keys"
    );
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Canvas(vec![
            "orders.customer_id \u{2192} customers.id accepted".into()
        ])
    );

    v.click(join.center());
    assert_eq!(
        v.sheet.browser.canvas_joins(),
        &[(shop_join(), false)],
        "a second click drops it"
    );
}

#[test]
fn the_foot_shows_the_keys_for_the_focused_pane() {
    let mut v = View::over(
        &data().join("shop"),
        previews(instant(vec![shop_join()])),
        Mode::Light,
    );
    v.until("the canvas's join", |v| !v.sheet.drawn().joins.is_empty());
    let list = v.sheet.drawn().foot.clone();
    for key in [
        "j next",
        "h up a folder",
        "l Enter open or add",
        "a add",
        "Space mark",
        "/ glob",
    ] {
        assert!(list.contains(&key.to_string()), "the list's foot: {list:?}");
    }
    assert!(list.contains(&"Esc close".to_string()), "{list:?}");
    assert!(!list.iter().any(|k| k.contains("accept")), "{list:?}");

    v.key(egui::Key::Tab);
    assert_eq!(v.sheet.browser.focus, Pane::Canvas);
    let canvas = v.sheet.drawn().foot.clone();
    for key in [
        "j next join",
        "Space accept or drop",
        "l Enter add the folder",
    ] {
        assert!(
            canvas.contains(&key.to_string()),
            "the canvas's foot: {canvas:?}"
        );
    }
    assert!(!canvas.contains(&"Space mark".to_string()), "{canvas:?}");
    assert_ne!(list, canvas, "the list's keys and the canvas's differ");

    // A key on the foot is a button: a click does what the key does.
    let tab = canvas
        .iter()
        .position(|k| k == "Tab next column")
        .expect("Tab on the canvas's foot");
    let at = v.sheet.drawn().foot_rects[tab].center();
    v.click(at);
    assert_eq!(
        v.sheet.browser.focus,
        Pane::Places,
        "the foot's Tab moved on"
    );
    assert!(
        v.sheet
            .drawn()
            .foot
            .contains(&"l Enter go there".to_string()),
        "{:?}",
        v.sheet.drawn().foot
    );
}

#[test]
fn a_file_preview_shows_its_first_rows_without_blocking_the_frame() {
    let (release, held) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let read: ReadPage = Arc::new(move |path: &Path| {
        let _ = held.lock().expect("the gate").recv();
        view::read_first_page(path)
    });
    let started = Instant::now();
    let mut v = View::over(
        &data().join("quakes.csv"),
        Previews::new(read, Arc::new(|_: &Path| Ok(Vec::new()))),
        Mode::Light,
    );
    v.frame(Vec::new());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the frames waited for the read"
    );
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Waiting("Reading the first rows of quakes.csv\u{2026}".into()),
        "the frame drew while the read was held"
    );

    release.send(()).expect("the read is waiting");
    v.until("the rows", |v| {
        matches!(v.sheet.drawn().preview, Shown::Rows { .. })
    });
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Rows {
            columns: vec!["id".into(), "magnitude".into(), "region".into()],
            rows: 3,
        },
        "the file's first rows, read by DuckDB"
    );
}

#[test]
fn a_folder_preview_draws_dovetails_joins() {
    let asked = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let joins = vec![
        shop_join(),
        ProposedJoin::new(("orders", "id"), ("customers", "id")),
    ];
    let propose: ProposeJoins = {
        let asked = Arc::clone(&asked);
        let joins = joins.clone();
        Arc::new(move |folder: &Path| {
            asked.lock().expect("the record").push(folder.to_path_buf());
            Ok(joins.clone())
        })
    };
    let mut v = View::over(
        &data().join("shop"),
        Previews::new(Arc::new(|_: &Path| Ok(quakes_page())), propose),
        Mode::Light,
    );
    v.until(
        "the canvas",
        |v| matches!(&v.sheet.drawn().preview, Shown::Canvas(j) if !j.is_empty()),
    );
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Canvas(vec![
            "orders.customer_id \u{2192} customers.id proposed".into(),
            "orders.id \u{2192} customers.id proposed".into(),
        ]),
        "a folder previews as the joins dovetail proposes between its tables"
    );
    assert_eq!(
        *asked.lock().expect("the record"),
        vec![data().join("shop")],
        "dovetail is asked about the folder under the cursor, once"
    );

    // Its canvas takes the keys: `j` and Space accept the second join.
    v.key(egui::Key::Tab);
    v.key(egui::Key::J);
    v.key(egui::Key::Space);
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Canvas(vec![
            "orders.customer_id \u{2192} customers.id proposed".into(),
            "orders.id \u{2192} customers.id accepted".into(),
        ])
    );

    // `v` previews the folder as its files instead.
    v.key(egui::Key::H);
    v.key(egui::Key::V);
    assert_eq!(
        v.sheet.drawn().preview,
        Shown::Files(vec!["customers.csv".into(), "orders.csv".into()])
    );
}

#[test]
fn it_takes_the_window_theme() {
    let mut fills = Vec::new();
    for mode in [Mode::Light, Mode::Dark] {
        let sem = meridian_design::semantic(mode.is_dark());
        let surface = design::to_color32(sem.surfaces.overlay);
        let ink = design::to_color32(sem.text.primary);
        let mut v = View::over(
            &data().join("quakes.csv"),
            previews(instant(Vec::new())),
            mode,
        );
        let (_, out) = v.output(Vec::new(), 1.0);
        let sheet = v.sheet.drawn().sheet.expect("the sheet was drawn");
        assert!(
            sheet.right() <= 1200.5 && sheet.bottom() <= 760.5,
            "{mode:?}: the sheet runs past the screen: {sheet:?}"
        );
        let shapes = flat(out.shapes);
        let background = shapes.iter().any(|s| match s {
            egui::Shape::Rect(r) => {
                r.fill == surface
                    && r.rect.width() >= sheet.width() - 2.0
                    && r.rect.contains(sheet.center())
            }
            _ => false,
        });
        assert!(
            background,
            "{mode:?}: the sheet is painted on the theme's overlay surface"
        );
        let title = shapes.iter().any(|s| match s {
            egui::Shape::Text(t) => {
                t.galley.job.text.contains("Add sources")
                    && t.galley.job.sections.iter().all(|s| s.format.color == ink)
            }
            _ => false,
        });
        assert!(title, "{mode:?}: the title is in the theme's primary ink");
        fills.push(surface);
    }
    assert_ne!(fills[0], fills[1], "the two modes' surfaces differ");
}

// ---------------------------------------------------------------------------
// The window around the browser.
// ---------------------------------------------------------------------------

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A folder of the test's own under the scratch input folder, emptied first.
fn input(test: &str) -> PathBuf {
    let dir = scratch().join("input").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("an input folder");
    dir
}

/// Three tables of the retail extract that dovetail joins, copied into `parent/retail`: a
/// visit's customer and store.
fn retail(parent: &Path) -> PathBuf {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract");
    let dir = parent.join("retail");
    std::fs::create_dir_all(&dir).expect("the retail folder");
    for file in ["customers.csv", "loyalty_visits.csv", "stores.csv"] {
        std::fs::copy(fixture.join(file), dir.join(file)).expect("the fixture copies");
    }
    dir
}

/// One headless window, its frames run through one context.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn with(app: MeridianApp) -> Self {
        let mut win = Self {
            app,
            ctx: egui::Context::default(),
            time: 0.0,
        };
        win.settle();
        win
    }

    /// A window whose browser reads and proposes as the app does.
    fn new() -> Self {
        scratch();
        Self::with(
            MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(runner()))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    scratch().join("history"),
                ))),
        )
    }

    /// A window whose browser's previews are `previews`.
    fn previewing(previews: (ReadPage, ProposeJoins)) -> Self {
        scratch();
        Self::with(
            MeridianApp::headless(Boot::empty(), Mode::Light)
                .running_with(Some(runner()))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    scratch().join("history"),
                )))
                .previewing_sources_with(previews.0, previews.1),
        )
    }

    fn run_raw(&mut self, mut raw: egui::RawInput) {
        self.time += 1.0;
        raw.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        ));
        raw.time = Some(self.time);
        raw.modifiers = raw
            .events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.run_raw(egui::RawInput {
            events,
            ..Default::default()
        });
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key)]);
        self.settle();
    }

    /// Draw frames until `done` holds, failing after `secs` seconds.
    fn until(&mut self, what: &str, secs: u64, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "{what}: not in time; {}",
                self.said()
            );
            std::thread::sleep(Duration::from_millis(20));
            self.run(Vec::new());
        }
    }

    fn browser(&self) -> &Browser {
        &self
            .app
            .sources_browser()
            .unwrap_or_else(|| panic!("the browser is not open: {}", self.said()))
            .browser
    }

    fn said(&self) -> String {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Whether the model the window holds keeps `left` to `right` as a join the analyst decided.
fn analyst_keeps(win: &Window, left: (&str, &str), right: (&str, &str)) -> bool {
    let model = &win.app.model_pane().expect("the model is open").model;
    model.relationships.iter().any(|r| {
        let one = |c: &str| vec![c.to_string()];
        let joins = (r.from == left.0
            && r.to == right.0
            && r.from_columns == one(left.1)
            && r.to_columns == one(right.1))
            || (r.from == right.0
                && r.to == left.0
                && r.from_columns == one(right.1)
                && r.to_columns == one(left.1));
        joins && r.meta.decided_by == brightfield_ossie::DecidedBy::Analyst
    })
}

#[test]
fn a_folder_with_a_proposed_join_opens_the_browser_on_it_before_adding() {
    // From the picker: the request the door's Open, Cmd+O and Finder push.
    let folder = retail(&input("proposed-picker"));
    let mut win = Window::new();
    win.app.open_queue().push(OpenRequest::Path(folder.clone()));
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("sources-browser"),
        "the browser opens on the folder: {}",
        win.said()
    );
    let made = win.app.open_protocol_dir().expect("the Protocol was made");
    assert_eq!(
        made.file_name().unwrap(),
        "retail",
        "named after the folder"
    );
    let b = win.browser();
    assert_eq!(b.canvas_folder(), Some(folder.as_path()));
    assert_eq!(b.focus, Pane::Canvas, "its canvas holds the keys");
    let joins: Vec<ProposedJoin> = b.canvas_joins().iter().map(|(j, _)| j.clone()).collect();
    assert!(!joins.is_empty(), "dovetail's joins are on the canvas");
    assert!(
        b.canvas_joins().iter().all(|(_, accepted)| !accepted),
        "none accepted yet"
    );

    // Space accepts the first, and Enter adds the folder: it is a source already, so the join
    // is kept in its model as the analyst's, and nothing is added twice.
    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None, "{}", win.said());
    let first = &joins[0];
    assert!(
        analyst_keeps(
            &win,
            (&first.left.0, &first.left.1),
            (&first.right.0, &first.right.1)
        ),
        "the accepted join is the analyst's in the model"
    );
    let pane = win.app.model_pane().expect("the model is drawn");
    assert_eq!(
        pane.protocol.sources.tables.len(),
        3,
        "the folder's three tables, once each"
    );

    // From a drop on the door, alike.
    let dropped = retail(&input("proposed-drop"));
    let mut win = Window::new();
    assert!(win.app.front_door_is_live());
    win.run_raw(egui::RawInput {
        dropped_files: vec![egui::DroppedFile {
            path: Some(dropped.clone()),
            ..Default::default()
        }],
        ..Default::default()
    });
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("sources-browser"),
        "a dropped folder opens the browser on it too: {}",
        win.said()
    );
    assert_eq!(win.browser().canvas_folder(), Some(dropped.as_path()));
    assert!(!win.browser().canvas_joins().is_empty());
}

#[test]
fn a_folder_of_one_table_or_no_joins_is_added_at_once() {
    let one = input("one-table").join("quakes");
    std::fs::create_dir_all(&one).expect("a folder");
    std::fs::write(one.join("quakes.csv"), QUAKES).expect("a CSV");
    let mut win = Window::new();
    win.app.open_queue().push(OpenRequest::Path(one));
    win.settle();
    assert_eq!(win.app.open_overlay(), None, "{}", win.said());
    assert!(
        win.app.own_protocol().is_some(),
        "one table opens on its grid: {}",
        win.said()
    );

    let apart = input("no-joins").join("apart");
    std::fs::create_dir_all(&apart).expect("a folder");
    std::fs::write(
        apart.join("weather.csv"),
        "city_name,temperature_c\nHobart,12.5\nDarwin,31.0\nPerth,24.25\n",
    )
    .expect("a CSV");
    std::fs::write(
        apart.join("colours.csv"),
        "colour,hex\nred,#ff0000\ngreen,#00ff00\nblue,#0000ff\n",
    )
    .expect("a CSV");
    let mut win = Window::new();
    win.app.open_queue().push(OpenRequest::Path(apart));
    win.settle();
    let pane = win.app.model_pane().expect("two tables open the model");
    assert!(
        pane.model.relationships.is_empty(),
        "dovetail proposes no join between them: {:?}",
        pane.model.relationships
    );
    assert_eq!(
        win.app.open_overlay(),
        None,
        "with no join to accept, the model is drawn at once"
    );
}

#[test]
fn new_protocol_a_and_add_source_open_the_browser() {
    let parent = input("opening");
    let mut win = Window::previewing(instant(Vec::new()));
    win.app.make_new_protocol_in(&win.ctx.clone(), &parent);
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("sources-browser"),
        "New Protocol opens the browser over the empty canvas: {}",
        win.said()
    );
    let home = std::env::home_dir().expect("a home folder");
    assert_eq!(
        win.browser().dir(),
        home,
        "on the home folder, with no source"
    );
    assert!(
        win.app.empty_canvas_said().is_some(),
        "the empty canvas is drawn under it"
    );

    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None, "Esc closes it");
    assert!(win.app.empty_protocol().is_some(), "and adds nothing");

    win.run(vec![
        press_with(egui::Key::A, egui::Modifiers::SHIFT),
        egui::Event::Text("A".into()),
    ]);
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("sources-browser"),
        "A opens it"
    );
}

#[test]
fn an_add_from_the_browser_keeps_the_folder_with_its_accepted_joins() {
    let place = input("browser-add");
    let quakes = place.join("quakes.csv");
    std::fs::write(&quakes, QUAKES).expect("a CSV");
    let folder = retail(&place);
    let join = ProposedJoin::new(
        ("loyalty_visits", "customer_id"),
        ("customers", "customer_id"),
    );
    let mut win = Window::previewing(instant(vec![join.clone()]));
    win.app
        .make_new_protocol_in(&win.ctx.clone(), &scratch().join("protocols"));
    win.settle();
    win.key(egui::Key::Escape);
    win.app
        .open_queue()
        .push(OpenRequest::AddSource(quakes.clone()));
    win.settle();
    assert!(win.app.own_protocol().is_some(), "{}", win.said());

    // The browser opens on the Protocol's first source, in its folder.
    win.app.open_sources_browser(&win.ctx.clone());
    win.settle();
    assert_eq!(win.browser().dir(), place.as_path());
    assert_eq!(
        win.browser().current().map(|e| e.path.clone()),
        Some(quakes)
    );

    // Up to the retail folder, its canvas, the join accepted, and the folder added.
    win.key(egui::Key::K);
    assert_eq!(win.browser().canvas_folder(), Some(folder.as_path()));
    win.until("the canvas's join", 10, |w| {
        !w.browser().canvas_joins().is_empty()
    });
    win.key(egui::Key::Tab);
    win.key(egui::Key::Space);
    win.key(egui::Key::A);
    win.until("the add", 60, |w| w.app.model_pane().is_some());
    assert_eq!(win.app.open_overlay(), None, "{}", win.said());
    let pane = win.app.model_pane().expect("the model");
    assert_eq!(
        pane.protocol.sources.tables.len(),
        4,
        "quakes and the folder's three tables"
    );
    assert!(
        analyst_keeps(
            &win,
            ("loyalty_visits", "customer_id"),
            ("customers", "customer_id")
        ),
        "the join accepted on the canvas is the analyst's in the model"
    );
}

#[test]
fn a_folder_dropped_on_an_empty_protocol_opens_the_browser_on_its_canvas() {
    let folder = retail(&input("dropped-on-empty"));
    let mut win = Window::previewing(instant(Vec::new()));
    win.app
        .make_new_protocol_in(&win.ctx.clone(), &input("dropped-on-empty-protocols"));
    win.settle();
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
    assert!(win.app.empty_protocol().is_some());

    win.run_raw(egui::RawInput {
        dropped_files: vec![egui::DroppedFile {
            path: Some(folder.clone()),
            ..Default::default()
        }],
        ..Default::default()
    });
    win.settle();
    assert_eq!(
        win.app.open_overlay(),
        Some("sources-browser"),
        "the folder is added and the browser opens on its canvas: {}",
        win.said()
    );
    let b = win.browser();
    assert_eq!(b.canvas_folder(), Some(folder.as_path()));
    assert_eq!(b.focus, Pane::Canvas);
    let first = b
        .canvas_joins()
        .first()
        .map(|(j, _)| j.clone())
        .expect("dovetail's joins are on the canvas");
    assert!(b.canvas_joins().iter().all(|(_, accepted)| !accepted));

    win.key(egui::Key::Space);
    win.key(egui::Key::Enter);
    assert_eq!(win.app.open_overlay(), None, "{}", win.said());
    assert!(
        analyst_keeps(
            &win,
            (&first.left.0, &first.left.1),
            (&first.right.0, &first.right.1)
        ),
        "the accepted join is the analyst's"
    );
    assert_eq!(
        win.app
            .model_pane()
            .expect("the model")
            .protocol
            .sources
            .tables
            .len(),
        3,
        "the folder's tables, added once"
    );
}

#[test]
fn an_accepted_join_names_the_table_the_add_makes() {
    use brightfield_shell::own_files::{self, AcceptedJoin};
    let place = input("renamed-join");
    let first = place.join("customers.csv");
    std::fs::write(&first, CUSTOMERS).expect("a CSV");
    let shop = place.join("shop");
    std::fs::create_dir_all(&shop).expect("a folder");
    std::fs::write(shop.join("customers.csv"), CUSTOMERS).expect("a CSV");
    std::fs::write(shop.join("orders.csv"), ORDERS).expect("a CSV");
    let runner = runner();
    let stand_in = brightfield_shell::discovery::StandIn;
    let mut p = own_files::create_empty(&input("renamed-join-protocols")).expect("made");
    own_files::add_source_with(&mut p, &first, &[], &runner, &stand_in).expect("added");

    // The folder's canvas names its tables as a preview of the folder alone finds them.
    let join = AcceptedJoin {
        left: ("orders".into(), "customer_id".into()),
        right: ("customers".into(), "id".into()),
    };
    own_files::add_source_with(&mut p, &shop, &[join], &runner, &stand_in).expect("added");
    let names: Vec<&str> = p.sources.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["customers", "customers_2", "orders"]);

    let text = std::fs::read_to_string(p.dir.join("model.ossie.yaml")).expect("the model");
    let model = brightfield_ossie::read(&text).expect("it reads");
    let analysts: Vec<(&str, &str)> = model
        .relationships
        .iter()
        .filter(|r| r.meta.decided_by == brightfield_ossie::DecidedBy::Analyst)
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    assert_eq!(
        analysts,
        [("orders", "customers_2")],
        "the join runs to the folder's customers, which the add named customers_2"
    );
}

// ---------------------------------------------------------------------------
// The baselines: a CSV's preview and a folder's canvas, in both themes.
// ---------------------------------------------------------------------------

/// The browser over [`data`] at `at`, drawn through kittest's wgpu backend once `ready` holds of
/// what it drew, and matched against the baseline `name`.
fn baseline(
    mode: Mode,
    name: &str,
    at: &Path,
    joins: Vec<ProposedJoin>,
    prepare: impl FnOnce(&mut Sheet),
    ready: impl Fn(&Sheet) -> bool,
) {
    let browser = Browser::open_among(vec![Place::new("Data", data())], data(), Some(at), &Disk);
    let mut sheet = Sheet::new(browser, previews(instant(joins)));
    prepare(&mut sheet);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1100.0, 600.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_ui_state(
            move |ui, sheet: &mut Sheet| {
                design::apply(ui.ctx(), mode);
                view::show(ui, sheet, mode);
            },
            sheet,
        );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready(harness.state()) {
        assert!(
            Instant::now() < deadline,
            "{name}: the preview did not land"
        );
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run();
    harness.snapshot(name);
}

fn csv_baseline(mode: Mode, name: &str) {
    baseline(
        mode,
        name,
        &data().join("quakes.csv"),
        Vec::new(),
        |_| {},
        |s| matches!(s.drawn().preview, Shown::Rows { .. }),
    );
}

fn canvas_baseline(mode: Mode, name: &str) {
    let joins = vec![
        shop_join(),
        ProposedJoin::new(("orders", "id"), ("customers", "id")),
    ];
    let shop = data().join("shop");
    baseline(
        mode,
        name,
        &shop.clone(),
        joins.clone(),
        // The first join accepted and the canvas holding the keys, its cursor on the second.
        move |sheet| {
            sheet.previews_mut().know_joins(&shop, joins.clone());
            sheet.browser.set_joins(&shop, joins);
            sheet.browser.toggle_join(0);
            sheet.browser.focus = Pane::Canvas;
            sheet.browser.join_cursor = 1;
        },
        |s| matches!(&s.drawn().preview, Shown::Canvas(j) if !j.is_empty()),
    );
}

#[test]
fn the_sources_browser_on_a_csv_light_baseline() {
    csv_baseline(Mode::Light, "sources_browser_csv_light");
}

#[test]
fn the_sources_browser_on_a_csv_dark_baseline() {
    csv_baseline(Mode::Dark, "sources_browser_csv_dark");
}

#[test]
fn the_sources_browser_on_a_folders_canvas_light_baseline() {
    canvas_baseline(Mode::Light, "sources_browser_canvas_light");
}

#[test]
fn the_sources_browser_on_a_folders_canvas_dark_baseline() {
    canvas_baseline(Mode::Dark, "sources_browser_canvas_dark");
}
