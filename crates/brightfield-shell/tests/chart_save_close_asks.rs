//! **Closing a window that carries the unsaved mark asks first.**
//!
//! The shell read no close request, so a window with a chart edit its file
//! did not hold closed on the first click of its close button and the edit went
//! with it. The window now reads the request in its frame, cancels the close,
//! and asks in its one modal slot: save, discard or cancel.
//!
//! A close request is raised the way the operating system raises it: a
//! `ViewportEvent::Close` in the root viewport's input for one frame, which is
//! `ViewportInfo::close_requested`. The window answers a request it wants to
//! stop by sending `ViewportCommand::CancelClose` — eframe closes the root
//! viewport unless the frame sends it — and closes itself by sending
//! `ViewportCommand::Close`, which comes back the next frame as a request like
//! the first. So each test reads the commands the frames sent, and each test
//! that ends in a close feeds that request back, as eframe does.
//!
//! The answers are given the way a person gives them: a click on the button
//! the frame drew, found by the text it painted. Save is compared with the
//! Save verb's own write, which is driven through the chart palette, so
//! "writes the chart as the Save verb does" is two windows writing to two
//! folders and the bytes compared.
//!
//! # What is covered and what is not
//!
//! Covered: the question and its three answers; each answer; a window with no
//! mark, before any edit and after a Save, closing with no question; a write
//! that fails, for the chart and for the Protocol, and a window that has no
//! Protocol to write. Not covered here: the screenshot countdown, which lives
//! in the binary and is held by its own test in `src/main.rs`; and the
//! editor pane's unsaved text, which is not this question's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use brightfield_shell::app::GridLayout;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::{PlotAxis, ScaleType};

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// The overlay's name for the question, as `MeridianApp::open_overlay` says it.
const QUESTION: &str = "close-question";

/// The banner's headline when the chart could not be written.
const CHART_BANNER: &str = "Could not save this chart";

/// The banner's headline when the Protocol could not be written.
const PROTOCOL_BANNER: &str = "Could not save this Protocol";

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-close-asks-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Puts a directory's permissions back when dropped, so a test that made one
/// read-only leaves a folder its `TempDir` can remove even when an assertion
/// fails.
#[cfg(unix)]
struct Restore(PathBuf);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// Every file under `dir`, path relative to it, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("a readable folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, into);
            } else {
                into.insert(
                    path.strip_prefix(base)
                        .expect("under the folder")
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(dir, dir, &mut found);
    found
}

/// What the frames sent the operating system, since it was last taken.
#[derive(Default, Debug, PartialEq, Eq)]
struct Sent {
    cancel_close: bool,
    close: bool,
}

/// A window over the housing file copied into a folder of its own, kept alive
/// with one `egui::Context` because a click is resolved against the widget id
/// a *previous* frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
    sent: Sent,
    /// Held last so the folder outlives the window.
    _root: TempDir,
}

impl Session {
    fn open(name: &str) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
        let mut session = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            folder,
            sent: Sent::default(),
            _root: root,
        };
        session.settle();
        session
    }

    /// A window over the housing file with one chart edit thrown, so the title
    /// carries the mark.
    fn marked(name: &str) -> Self {
        let mut session = Self::open(name);
        session.transpose();
        session.throw("population", ScaleType::Log);
        assert!(session.carries_mark());
        session
    }

    fn chart_file(&self) -> PathBuf {
        self.folder.join("panels").join(CHART_FILE)
    }

    fn arcform_file(&self) -> PathBuf {
        self.folder.join("arcform.yaml")
    }

    /// What a file this session wrote says, with this session's own folder
    /// written as `<folder>`: the two windows a comparison holds are in two
    /// folders, and the generated text spells the folder it was made in.
    fn written(&self, file: &Path) -> String {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} was not written: {e}", file.display()));
        let root = self._root.0.to_string_lossy().into_owned();
        text.replace(&root, "<folder>")
    }

    fn carries_mark(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// Run one frame with `events`, `close` raising a close request in it, and
    /// note what the frame sent the operating system.
    fn frame(&mut self, events: Vec<egui::Event>, close: bool) -> Vec<String> {
        let mut raw = egui::RawInput {
            screen_rect: Some(self.screen),
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
                    egui::ViewportCommand::CancelClose => self.sent.cancel_close = true,
                    egui::ViewportCommand::Close => self.sent.close = true,
                    _ => {}
                }
            }
        }
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut text);
        }
        text.into_iter().map(|(_, t)| t).collect()
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.frame(events, false);
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Raise a close request, as the close button does, and let the frames
    /// after it draw what it opened.
    fn request_close(&mut self) {
        self.frame(Vec::new(), true);
        self.settle();
    }

    /// Take what the frames have sent since the last take.
    fn take_sent(&mut self) -> Sent {
        std::mem::take(&mut self.sent)
    }

    /// What eframe does with a `ViewportCommand::Close` the window sent: it
    /// comes back as a close request on the next frame.
    fn feed_the_close_back(&mut self) -> Sent {
        assert!(self.sent.close, "no Close was sent to feed back");
        self.take_sent();
        self.request_close();
        self.take_sent()
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        self.run(Vec::new());
    }

    /// Press and release the primary button over `pos`, as five frames.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ] {
            self.run(events);
        }
    }

    /// Where the frame painted `label`, the topmost of them: the question is
    /// drawn over the window, so the last text with these words is the
    /// question's.
    fn painted(&mut self, label: &str) -> Option<egui::Pos2> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut text);
        }
        text.into_iter()
            .filter(|(_, t)| t == label)
            .map(|(rect, _)| rect.center())
            .next_back()
    }

    /// Answer the question by clicking the button labelled `label`.
    fn answer(&mut self, label: &str) {
        assert_eq!(self.app.open_overlay(), Some(QUESTION), "no question is up");
        let at = self
            .painted(label)
            .unwrap_or_else(|| panic!("the question drew no {label:?} button"));
        self.click(at);
    }

    /// Put the grid in its columns layout, the one where each histogram tile
    /// is on screen with its switch.
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
        assert_eq!(self.app.grid_layout(), GridLayout::Columns);
    }

    /// Throw the scale switch on `column`'s tile to `kind`, by the gesture.
    fn throw(&mut self, column: &str, kind: ScaleType) {
        let at = self
            .switch(column)
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
        assert!(
            self.carries_mark(),
            "the click at {at:?} did not throw {column}'s switch: the title is {:?}",
            self.app.title()
        );
    }

    fn switch(&self, column: &str) -> brightfield_shell::app::ScaleSwitchDrawn {
        self.app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .clone()
    }

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter — the Save verb.
    fn save_verb(&mut self) {
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(self.app.open_overlay(), None);
        self.settle();
    }

    /// The headline and body of every banner the window is showing.
    fn banners(&self) -> Vec<(String, String)> {
        self.app
            .notifications()
            .iter()
            .map(|n| (n.title.clone(), n.body.clone().unwrap_or_default()))
            .collect()
    }

    fn banner(&self, headline: &str) -> Option<String> {
        self.banners()
            .into_iter()
            .find(|(title, _)| title == headline)
            .map(|(_, body)| body)
    }
}

/// Every string a shape paints, with the rect it is painted in.
fn collect_text(shape: &egui::epaint::Shape, into: &mut Vec<(egui::Rect, String)>) {
    match shape {
        egui::epaint::Shape::Text(t) => {
            let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
            into.push((rect, t.galley.text().to_string()));
        }
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text(s, into);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// AC1 — the question
// ---------------------------------------------------------------------------

/// **AC1.** A close request on a window that carries the mark leaves the window
/// open — `CancelClose` sent, no `Close` — and shows the question with save,
/// discard and cancel.
#[test]
fn a_close_request_over_the_mark_leaves_the_window_open_and_asks() {
    let mut session = Session::marked("ac1");
    session.take_sent();

    session.request_close();

    let sent = session.take_sent();
    assert!(
        sent.cancel_close,
        "the close request was not cancelled, so eframe closes the window: {sent:?}"
    );
    assert!(!sent.close, "the window closed itself over the edit");
    assert_eq!(session.app.open_overlay(), Some(QUESTION));
    let text = session.frame(Vec::new(), false);
    for expected in [
        brightfield_shell::overlays::CLOSE_QUESTION_TITLE,
        "Save",
        "Discard",
        "Cancel",
    ] {
        assert!(
            text.iter().any(|t| t == expected),
            "the question drew no {expected:?}: {text:?}"
        );
    }
    assert!(session.carries_mark(), "asking cleared the mark");
}

// ---------------------------------------------------------------------------
// AC2 — save
// ---------------------------------------------------------------------------

/// **AC2.** Save writes the chart and `arcform.yaml` exactly as the Save verb
/// does, and then the window closes: `Close` is sent, and when eframe hands it
/// back as a request it is not asked about again.
#[test]
fn save_writes_the_chart_as_the_save_verb_does_and_closes() {
    // The Save verb's own write, in a window of its own, is the reference.
    let mut reference = Session::marked("ac2-verb");
    reference.save_verb();
    assert!(
        reference.chart_file().exists(),
        "the Save verb wrote no chart"
    );
    let verb_chart = reference.written(&reference.chart_file());
    let verb_arcform = reference.written(&reference.arcform_file());

    let mut session = Session::marked("ac2-question");
    session.take_sent();
    session.request_close();
    session.answer("Save");

    assert_eq!(
        session.written(&session.chart_file()),
        verb_chart,
        "the chart the question wrote is not the chart the Save verb writes"
    );
    assert_eq!(
        session.written(&session.arcform_file()),
        verb_arcform,
        "arcform.yaml the question wrote is not the one the Save verb writes"
    );
    assert!(
        !session.carries_mark(),
        "the chart is in and the mark stayed"
    );
    assert!(
        session.sent.close,
        "the window did not close after the save"
    );
    let back = session.feed_the_close_back();
    assert!(
        !back.cancel_close && session.app.open_overlay().is_none(),
        "the close the save asked for was asked about again: {back:?}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — discard
// ---------------------------------------------------------------------------

/// **AC3.** Discard closes the window and touches nothing on disk: the chart
/// file and `arcform.yaml`, both there from an earlier Save, are byte-identical,
/// and no file was added. The close it sends is not asked about again, though
/// the edit is still pending when it comes back.
#[test]
fn discard_closes_and_leaves_the_files_byte_identical() {
    let mut session = Session::marked("ac3");
    session.save_verb();
    assert!(session.chart_file().exists() && session.arcform_file().exists());
    // A second edit, unsaved: the window carries the mark again.
    session.throw("median_income", ScaleType::Log);
    let before = snapshot(&session.folder);
    session.take_sent();

    session.request_close();
    assert_eq!(session.app.open_overlay(), Some(QUESTION));
    session.answer("Discard");

    assert!(session.sent.close, "discard did not close the window");
    assert_eq!(
        snapshot(&session.folder),
        before,
        "discard changed what is on disk"
    );
    assert!(
        session.carries_mark(),
        "the edit was not pending: this test discarded nothing"
    );
    let back = session.feed_the_close_back();
    assert!(
        !back.cancel_close && session.app.open_overlay().is_none(),
        "the close discard asked for was asked about again: {back:?}"
    );
    assert_eq!(snapshot(&session.folder), before);
}

/// **AC3, before any Save.** With no chart file and no `arcform.yaml` yet,
/// discard leaves the folder as it was: neither is created on the way out.
#[test]
fn discard_before_any_save_writes_no_file() {
    let mut session = Session::marked("ac3-first");
    assert!(!session.chart_file().exists() && !session.arcform_file().exists());
    let before = snapshot(&session.folder);

    session.request_close();
    session.answer("Discard");

    assert!(session.sent.close);
    assert_eq!(snapshot(&session.folder), before);
}

// ---------------------------------------------------------------------------
// AC4 — cancel
// ---------------------------------------------------------------------------

/// **AC4.** Cancel returns to the window with the edit still drawn and the
/// mark still in the title, and the next close request asks again. Escape and a
/// click outside the card are cancel too.
#[test]
fn cancel_returns_to_the_window_with_the_edit_and_the_mark() {
    for how in ["button", "escape", "backdrop"] {
        let mut session = Session::marked(&format!("ac4-{how}"));
        let title = session.app.title();
        session.take_sent();

        session.request_close();
        assert_eq!(session.app.open_overlay(), Some(QUESTION));
        session.take_sent();
        match how {
            "button" => session.answer("Cancel"),
            "escape" => session.key(egui::Key::Escape),
            _ => session.click(egui::pos2(4.0, 4.0)),
        }

        assert_eq!(
            session.app.open_overlay(),
            None,
            "{how}: the question stayed up"
        );
        assert!(!session.sent.close, "{how}: cancel closed the window");
        assert!(session.carries_mark(), "{how}: cancel cleared the mark");
        assert_eq!(session.app.title(), title, "{how}: the title changed");
        assert_eq!(
            session.switch("population").active,
            ScaleType::Log,
            "{how}: the edit is no longer drawn"
        );
        assert!(
            !session.chart_file().exists() && !session.arcform_file().exists(),
            "{how}: cancel wrote a file"
        );

        session.request_close();
        assert_eq!(
            session.app.open_overlay(),
            Some(QUESTION),
            "{how}: the next close request did not ask again"
        );
    }
}

// ---------------------------------------------------------------------------
// AC5 — no mark, no question
// ---------------------------------------------------------------------------

/// **AC5.** A close request on a window without the mark closes with no
/// question: nothing cancels it and no overlay opens — on a window that was
/// never edited, and on one whose edit a Save wrote.
#[test]
fn a_close_request_without_the_mark_closes_with_no_question() {
    let mut clean = Session::open("ac5-clean");
    assert!(!clean.carries_mark());
    clean.take_sent();
    clean.request_close();
    assert!(
        !clean.take_sent().cancel_close,
        "a window with nothing to lose was kept open"
    );
    assert_eq!(clean.app.open_overlay(), None);

    let mut saved = Session::marked("ac5-saved");
    saved.save_verb();
    assert!(!saved.carries_mark());
    saved.take_sent();
    saved.request_close();
    assert!(
        !saved.take_sent().cancel_close,
        "a window whose edit was saved was kept open"
    );
    assert_eq!(saved.app.open_overlay(), None);
}

// ---------------------------------------------------------------------------
// AC7 — a save that fails
// ---------------------------------------------------------------------------

/// **AC7, the chart's write refused.** The Protocol is written and the chart's
/// folder refuses the chart: the window stays open, the mark stays, and the
/// banner says why. A close request after the fault is mended asks again and
/// the save then closes.
#[cfg(unix)]
#[test]
fn a_save_whose_chart_write_fails_keeps_the_window_open_and_says_why() {
    use std::os::unix::fs::PermissionsExt;

    let mut session = Session::marked("ac7-chart");
    let panels = session.folder.join("panels");
    std::fs::create_dir_all(&panels).expect("panels/");
    let _restore = Restore(panels.clone());
    std::fs::set_permissions(&panels, std::fs::Permissions::from_mode(0o555))
        .expect("panels/ is made read-only");
    if std::fs::write(panels.join(".probe"), "").is_ok() {
        let _ = std::fs::remove_file(panels.join(".probe"));
        eprintln!("this process ignores directory permissions; the write cannot be refused");
        return;
    }
    session.take_sent();

    session.request_close();
    session.answer("Save");

    assert!(
        !session.sent.close,
        "the window closed over a chart that was not written"
    );
    assert!(session.carries_mark(), "the failed write cleared the mark");
    let said = session
        .banner(CHART_BANNER)
        .unwrap_or_else(|| panic!("no banner says why: {:?}", session.banners()));
    assert!(
        said.contains("panels"),
        "the banner does not say where the write failed: {said:?}"
    );
    assert!(!session.chart_file().exists());
    assert_eq!(
        session.app.open_overlay(),
        None,
        "the question stayed over the banner"
    );

    std::fs::set_permissions(&panels, std::fs::Permissions::from_mode(0o755))
        .expect("panels/ is made writable again");
    session.take_sent();
    session.request_close();
    assert_eq!(
        session.app.open_overlay(),
        Some(QUESTION),
        "the window did not ask again"
    );
    session.answer("Save");
    assert!(session.sent.close, "the mended save did not close");
    assert!(session.chart_file().exists());
}

/// **AC7, the Protocol's write refused.** The data file's folder refuses
/// `arcform.yaml`: the window stays open with the Protocol's banner.
#[cfg(unix)]
#[test]
fn a_save_whose_protocol_write_fails_keeps_the_window_open_and_says_why() {
    use std::os::unix::fs::PermissionsExt;

    let mut session = Session::marked("ac7-protocol");
    let _restore = Restore(session.folder.clone());
    std::fs::set_permissions(&session.folder, std::fs::Permissions::from_mode(0o555))
        .expect("the folder is made read-only");
    if std::fs::write(session.folder.join(".probe"), "").is_ok() {
        let _ = std::fs::remove_file(session.folder.join(".probe"));
        eprintln!("this process ignores directory permissions; the write cannot be refused");
        return;
    }
    session.take_sent();

    session.request_close();
    session.answer("Save");

    assert!(
        !session.sent.close,
        "the window closed over an unwritten Protocol"
    );
    assert!(session.carries_mark());
    assert!(
        session.banner(PROTOCOL_BANNER).is_some(),
        "no banner says the Protocol was not saved: {:?}",
        session.banners()
    );
}

/// **AC7, a window with no Protocol to write into.** A chart spec opened
/// directly has no folder for a Save to write beside, and the Save verb does
/// nothing there. Choosing save must not close over an edit nothing kept: the
/// window stays open and the banner says there is no Protocol.
#[test]
fn a_save_in_a_window_with_no_protocol_keeps_it_open_and_says_why() {
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-grouped-shares.yaml");
    let boot = Boot::open(
        spec.to_str().expect("utf-8 example path"),
        brightfield_protocol::layout::Flow::Vertical,
        None,
    )
    .expect("the example opens");
    let mut session = Session {
        app: MeridianApp::headless(boot, Mode::Light),
        ctx: egui::Context::default(),
        screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        folder: PathBuf::new(),
        sent: Sent::default(),
        _root: TempDir::new("ac7-no-protocol"),
    };
    session.settle();
    session.y_log_first_plot();
    assert!(session.carries_mark());
    assert!(!session.app.has_protocol_to_save());
    session.take_sent();

    session.request_close();
    assert_eq!(session.app.open_overlay(), Some(QUESTION));
    session.answer("Save");

    assert!(
        !session.sent.close,
        "the window closed over an edit nothing kept"
    );
    assert!(session.carries_mark());
    let said = session
        .banner(CHART_BANNER)
        .unwrap_or_else(|| panic!("no banner says why: {:?}", session.banners()));
    assert!(said.contains("Protocol"), "the banner is {said:?}");
}

impl Session {
    /// Write `yScale: log` onto the first plot of a chart-spec window.
    fn y_log_first_plot(&mut self) {
        assert!(self
            .app
            .chart_doc_mut()
            .set_plot_scale(0, PlotAxis::Y, ScaleType::Log));
        self.settle();
    }
}
