//! **Each Save of a chart records a version of the chart file in arcform's
//! local history.**
//!
//! Save wrote the chart file beside the Protocol and kept nothing of the text it
//! replaced, so after two Saves the first Save's text was gone. It now records,
//! under the chart file's own path in arcform's store, the text the write
//! replaces as a checkpoint before it and the text written as a save after it.
//!
//! The tests drive Save through the gesture a person has — the chart palette on
//! `space`, `save-spec` typed, enter — as `tests/chart_save_writes_chart.rs`
//! does, and read the history back through arcform's own [`LocalHistory`], not
//! through anything brightfield wrote, so the list a test reads is the list a
//! person's `arc history list --file` would print.
//!
//! **No test writes under the home directory.** Each window is given a store at
//! a root inside its own temporary folder, beside the data file's folder and not
//! in it ([`Session::open`]); a window given no store records nothing.
//!
//! # What is covered and what is not
//!
//! Covered: the text a second Save replaces being in the chart file's history
//! after two Saves made inside arcform's merge window; a chart file changed by
//! hand between two Saves being kept as a checkpoint; the Protocol's own history
//! being as it was; the data file's folder gaining the Protocol and the chart
//! and nothing else; a store that cannot be opened leaving the chart written and
//! the window saying that no version was recorded. Not covered: the list of
//! saved versions and the step back, which are a design session's.

use std::path::{Path, PathBuf};

use brightfield_protocol::chart_history::{HistoryKind, LocalHistory};
use brightfield_protocol::HistoryStore;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::layout::ScaleType;

const HOUSING_FILE: &str = "california_housing_sample.csv";

/// The chart file's name: the Protocol's name, which is the data file's stem.
const CHART_FILE: &str = "california_housing_sample.yaml";

/// The banner's headline when a Save kept the chart and no version of it.
const NOT_RECORDED: &str = "No version of this chart was recorded";

/// The committed table the window is opened over.
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
            "bf-chart-version-{name}-{}-{}",
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

/// A window over the housing file copied into a folder of its own, given a
/// history store of its own beside that folder.
///
/// It keeps one `egui::Context` for its whole life, because a click is resolved
/// against the widget id a *previous* frame registered.
struct Session {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder: where `arcform.yaml` and `panels/` land.
    folder: PathBuf,
    /// The root of the store the window records into, outside `folder`.
    history_root: PathBuf,
    /// Held last so the folders outlive the window.
    root: TempDir,
}

impl Session {
    /// A window whose store is a folder beside the data file's.
    fn open(name: &str) -> Self {
        Self::open_with(name, |root| HistoryStore::At(root.join("history")))
    }

    /// A window whose store is the one `store` names, given the temporary root
    /// the data file's folder and the default store's folder are both under.
    fn open_with(name: &str, store: impl FnOnce(&Path) -> HistoryStore) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path"))
            .unwrap_or_else(|e| panic!("open {}: {e}", data.display()));
        let mut session = Self {
            app: MeridianApp::headless(boot, Mode::Light)
                .keeping_history(Some(store(&root.0))),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
            folder,
            history_root: root.0.join("history"),
            root,
        };
        session.settle();
        session
    }

    fn chart_file(&self) -> PathBuf {
        self.folder.join("panels").join(CHART_FILE)
    }

    fn chart_text(&self) -> String {
        std::fs::read_to_string(self.chart_file())
            .unwrap_or_else(|e| panic!("read the chart file {}: {e}", self.chart_file().display()))
    }

    /// arcform's store, opened the way a person's `arc history` would reach it.
    fn history(&self) -> LocalHistory {
        LocalHistory::at_root(&self.history_root)
    }

    /// Every entry in the chart file's history, oldest first, with the text it
    /// kept.
    fn chart_versions(&self) -> Vec<(HistoryKind, String)> {
        let history = self.history();
        history
            .entries_for_file(&self.chart_file())
            .expect("the chart file's history lists")
            .into_iter()
            .map(|entry| {
                let text = history
                    .read_for_file(&self.chart_file(), &entry.id)
                    .expect("an entry the list names reads back");
                (entry.kind, text)
            })
            .collect()
    }

    /// The names directly under the data file's folder, sorted.
    fn folder_entries(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.folder)
            .expect("a readable folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn marked(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
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

    /// Put the grid in its columns layout, the one where each histogram tile is
    /// on screen with its switch.
    fn transpose(&mut self) {
        use brightfield_shell::app::GridLayout;
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
        let switch = self
            .app
            .chart_doc()
            .scale_switches
            .iter()
            .find(|s| s.column == column)
            .unwrap_or_else(|| panic!("no scale switch for {column:?}"))
            .clone();
        let at = switch
            .states
            .iter()
            .find(|(state, _)| *state == kind)
            .unwrap_or_else(|| panic!("{column}'s switch offers no {kind:?}"))
            .1
            .center();
        self.click(at);
        assert!(
            self.marked(),
            "the click at {at:?} did not throw {column}'s switch: the title is {:?}",
            self.app.title()
        );
    }

    /// **Save, through the gesture a person has**: the chart palette on `space`,
    /// the verb typed, confirmed with enter.
    fn save(&mut self) {
        assert!(
            self.app.has_protocol_to_save(),
            "this window has no Protocol behind it to save"
        );
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
    }

    /// The body of the banner that says no version was recorded, if the window
    /// is showing one.
    fn not_recorded_banner(&self) -> Option<String> {
        self.app
            .notifications()
            .iter()
            .find(|n| n.title == NOT_RECORDED)
            .map(|n| n.body.clone().unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// AC1 — the text a second Save replaces is in the chart file's history
// ---------------------------------------------------------------------------

/// **AC1.** A Save, a second chart edit and a second Save: the chart file's
/// history lists the text of the file as it stood before the second Save, and
/// the text the second Save wrote.
///
/// The two Saves are made a moment apart, which is inside arcform's merge
/// window, and that is the case that matters: arcform merges a save into the
/// newest entry when that is a save, so a second Save that recorded its text as
/// a save would replace the first Save's entry and the text the first Save wrote
/// would be in the history nowhere.
#[test]
fn after_two_saves_the_chart_files_history_lists_the_text_the_second_save_replaced() {
    let mut session = Session::open("ac1");
    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();
    let before_second = session.chart_text();

    session.throw("house_age", ScaleType::Log);
    session.save();
    let after_second = session.chart_text();
    assert_ne!(
        before_second, after_second,
        "the second edit did not change the chart file"
    );

    let kept: Vec<String> = session
        .chart_versions()
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    assert!(
        kept.contains(&before_second),
        "the text the second Save replaced is not in the history: {} entries",
        kept.len()
    );
    assert_eq!(
        kept.last(),
        Some(&after_second),
        "the newest entry is not the text the second Save wrote"
    );
}

/// **A chart file the analyst changed by hand between two Saves is kept as a
/// checkpoint**, before the second Save writes over it, and the text written is
/// kept as a save.
///
/// The hand edit is in the file and in no entry, so arcform does not skip it as
/// a duplicate: it is the text the write replaces, and the one version that
/// would otherwise be lost for good.
#[test]
fn a_chart_file_changed_by_hand_between_two_saves_is_kept_as_a_checkpoint() {
    let mut session = Session::open("by-hand");
    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();

    let by_hand = format!("{}# the analyst's own note\n", session.chart_text());
    std::fs::write(session.chart_file(), &by_hand).expect("the analyst's edit is written");

    session.throw("house_age", ScaleType::Log);
    session.save();
    let written = session.chart_text();

    let versions = session.chart_versions();
    assert!(
        versions.contains(&(HistoryKind::Checkpoint, by_hand)),
        "the text the second Save replaced is not a checkpoint: {:?}",
        versions.iter().map(|(kind, _)| kind).collect::<Vec<_>>()
    );
    assert_eq!(
        versions.last(),
        Some(&(HistoryKind::Save, written)),
        "the text the second Save wrote is not the newest entry, as a save"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the Protocol's own history is as it was
// ---------------------------------------------------------------------------

/// **AC2.** The Protocol's own history for the folder lists what it listed
/// before the two Saves.
///
/// The Protocol's history is given an entry first, so the test compares two
/// lists with something in them; and the chart file's history is read to hold
/// the two Saves, so an empty one cannot pass for a history left alone.
#[test]
fn the_protocols_own_history_lists_what_it_listed_before_two_saves() {
    let mut session = Session::open("ac2");
    let recorded = session
        .history()
        .record_save(&session.folder, "name: earlier\n")
        .expect("the Protocol's history takes an entry")
        .expect("and records it");
    let before = session
        .history()
        .entries(&session.folder)
        .expect("the Protocol's history lists");
    assert_eq!(before, vec![recorded]);

    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();
    session.throw("house_age", ScaleType::Log);
    session.save();

    assert!(
        session.chart_versions().len() >= 2,
        "the two Saves left the chart file's history without two versions"
    );
    let after = session
        .history()
        .entries(&session.folder)
        .expect("the Protocol's history lists");
    assert_eq!(after, before, "the two Saves changed the Protocol's history");
}

// ---------------------------------------------------------------------------
// AC3 — the folder gains the Protocol and the chart, and nothing else
// ---------------------------------------------------------------------------

/// **AC3.** After two Saves the data file's folder has gained `arcform.yaml`,
/// `models/` and `panels/` and no other entry: the history lives in the store
/// and not beside the data.
#[test]
fn two_saves_leave_the_data_folder_with_the_protocol_the_chart_and_nothing_else() {
    let mut session = Session::open("ac3");
    let before = session.folder_entries();
    assert_eq!(before, vec![HOUSING_FILE.to_string()]);

    session.transpose();
    session.throw("population", ScaleType::Log);
    session.save();
    session.throw("house_age", ScaleType::Log);
    session.save();

    let after = session.folder_entries();
    let gained: Vec<&String> = after.iter().filter(|name| !before.contains(name)).collect();
    assert_eq!(
        gained,
        vec!["arcform.yaml", "models", "panels"],
        "the folder holds {after:?}"
    );
    assert!(
        after.contains(&HOUSING_FILE.to_string()),
        "the data file is gone from its folder"
    );
    assert!(
        !session.chart_versions().is_empty(),
        "nothing was recorded, so the folder is clean for the wrong reason"
    );
    assert!(
        !session.history_root.starts_with(&session.folder),
        "the store is inside the data file's folder"
    );
}

// ---------------------------------------------------------------------------
// AC4 — a store that cannot be opened
// ---------------------------------------------------------------------------

/// **AC4.** When the store cannot be opened, Save writes the chart and the
/// window says that no version was recorded; a later Save that can record takes
/// the banner down.
///
/// The store's root is under a file, so no folder can be made there.
#[test]
fn a_store_that_cannot_be_opened_leaves_the_chart_written_and_the_window_says_so() {
    let mut session = Session::open_with("ac4", |root| {
        let blocker = root.join("blocker");
        std::fs::write(&blocker, "not a folder").expect("a file where the store's root belongs");
        HistoryStore::At(blocker.join("history"))
    });
    session.transpose();
    session.throw("population", ScaleType::Log);

    session.save();

    assert!(
        session.chart_text().contains("xScale: log"),
        "the chart was not written when the store could not be opened"
    );
    assert!(
        !session.marked(),
        "the title is {:?} after a Save that wrote the chart",
        session.app.title()
    );
    let said = session.not_recorded_banner().unwrap_or_else(|| {
        panic!(
            "no banner says that no version was recorded: {:?}",
            session
                .app
                .notifications()
                .iter()
                .map(|n| n.title.clone())
                .collect::<Vec<_>>()
        )
    });
    assert!(
        said.contains("history"),
        "the banner does not say what could not be opened: {said:?}"
    );

    let blocker = session.root.0.join("blocker");
    std::fs::remove_file(&blocker).expect("the blocker is removed");
    session.throw("house_age", ScaleType::Log);
    session.save();

    assert_eq!(
        session.not_recorded_banner(),
        None,
        "the banner stayed up after a Save that recorded"
    );
    let store = LocalHistory::at_root(blocker.join("history"));
    let kept = store
        .entries_for_file(&session.chart_file())
        .expect("the store opens now");
    assert!(
        !kept.is_empty(),
        "the Save after the blocker went recorded nothing"
    );
}
