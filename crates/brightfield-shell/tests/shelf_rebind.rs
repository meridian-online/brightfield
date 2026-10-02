//! **A column the shelf's list is on redraws the hero, and a column kept from
//! it marks the window unsaved.**
//!
//! With the hero's pane focused, `e` gives the shelf band the keys and a
//! channel's letter opens that channel's list of columns in the Outline. The
//! list's cursor draws the hero with the column under it on the channel before
//! any key keeps it, and `Esc` draws the hero as it was. `Enter`, or a click on
//! the row, keeps the column: the hero is drawn with it, the band's cell names
//! it, and the window title carries the unsaved mark until a Save writes the
//! edit into the chart file.
//!
//! Every assertion reads what a frame drew: the hero's composition (its marks,
//! its scales, what its top layer encodes), the text the frame put on the
//! screen, the band's cells and the window title — or, for Save, the chart
//! file on disk.

use brightfield_protocol::write_chart_edit;
use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::PlotHandle;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::shelf::Binding;
use brightfield_shell::shelf_edit::put_colour;
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::Spec;
use brightfield_spec::MarkKind;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";
const INCOME: &str = "median_income";
const VALUE: &str = "median_house_value";
const LON: &str = "longitude";
const LAT: &str = "latitude";

fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(HOUSING_FILE)
}

/// A directory of this test's own, removed when the test ends: a Save writes
/// beside the data file, and the data file is copied here so that is not the
/// repository.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-rebind-{name}-{}-{}",
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

fn key_down(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder, where Save writes `panels/`.
    folder: std::path::PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// Held last, so the folder outlives the window.
    _root: TempDir,
}

impl Window {
    /// A window over a copy of the sample in a folder of `name`'s own, for a
    /// test that saves.
    fn housing(name: &str) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        Self::over(&data, root)
    }

    /// A window over the committed sample itself, for a test that does not
    /// save: two of them read one file, so their specs name the same path.
    fn fixture() -> Self {
        Self::over(&housing(), TempDir::new("unused"))
    }

    fn over(data: &std::path::Path, root: TempDir) -> Self {
        let folder = data
            .parent()
            .expect("the data file has a folder")
            .to_path_buf();
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            texts: Vec::new(),
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let _ = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        self.texts = texts;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Press a letter, as a keyboard brings it, and let the frame after run.
    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![key_down(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    /// A click at `at`: the pointer moved there, pressed, released, and the
    /// frames after it settled.
    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// The column under the open list's cursor.
    fn cursor(&self) -> Option<String> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.cursor().map(str::to_owned))
    }

    /// Move the open list's cursor to `column` by its keys, `k` up the rows
    /// or `j` down, as many presses as it takes.
    fn walk_to(&mut self, column: &str, key: egui::Key, text: &str) {
        for _ in 0..12 {
            if self.cursor().as_deref() == Some(column) {
                return;
            }
            self.type_letter(key, text);
        }
        panic!(
            "the list's cursor did not reach {column} by `{text}`: it is on {:?}",
            self.cursor()
        );
    }

    /// The hero's plot, as the page composed it.
    fn hero(&self) -> &PlotHandle {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
    }

    /// The column the hero's top layer draws on `channel`, as the composition
    /// that ran records it.
    fn hero_column(&self, channel: Channel) -> Option<String> {
        self.hero()
            .hover
            .as_ref()
            .and_then(|layer| layer.column(channel).map(str::to_owned))
    }

    /// Whether the hero is drawn as the map: through a projection.
    fn hero_is_a_map(&self) -> bool {
        self.hero().scales.projection().is_some()
    }

    /// Whether the window title carries the unsaved mark.
    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// The text the last frame drew inside `rect`, joined.
    fn text_in(&self, rect: egui::Rect) -> Vec<String> {
        self.texts
            .iter()
            .filter(|t| rect.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| t.text.clone())
            .collect()
    }

    /// The text the last frame drew in the hero pane's header.
    fn hero_header_text(&self) -> String {
        let pane = self
            .app
            .canvas_panes()
            .pane("map")
            .expect("the canvas drew the hero pane");
        self.text_in(pane.header).join(" ")
    }

    /// The text the last frame drew in `channel`'s cell of the band.
    fn cell_text(&self, channel: ShelfChannel) -> Vec<String> {
        let drawn = self.app.shelf_drawn().expect("the band drew");
        self.text_in(drawn.cells[channel.index()])
    }

    /// Whether `channel`'s cell drew `column` as its column: whole, or cut to
    /// the cell's room and ended in an ellipsis, as the band draws a name
    /// longer than the room it has.
    fn cell_names(&self, channel: ShelfChannel, column: &str) -> bool {
        let cell = self.app.shelf_drawn().expect("the band drew").cells[channel.index()];
        self.texts
            .iter()
            .filter(|t| cell.contains_rect(t.visible) && t.visible.is_positive())
            .any(|t| {
                t.text == column
                    || (t.elided && {
                        let kept = t.text.trim_end_matches('\u{2026}');
                        kept.len() > column.len() / 2 && column.starts_with(kept)
                    })
            })
    }

    /// The Outline's row for `column` in the open list, as the frame drew it.
    fn list_row(&self, column: &str) -> egui::Rect {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Column && r.label == column)
            .unwrap_or_else(|| panic!("the Outline's list drew no row for {column}"))
            .rect
    }

    /// The spec the page is drawn from.
    fn spec(&self) -> Spec {
        self.app
            .chart_doc()
            .live_dashboard()
            .expect("a live dashboard")
            .spec()
            .clone()
    }

    /// The text the generator wrote at open, which a first Save places its
    /// edits into.
    fn generated_text(&self) -> String {
        let path = self
            .app
            .chart_doc()
            .spec_path
            .clone()
            .expect("a generated dashboard carries its spec file");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read the generated spec {}: {e}", path.display()))
    }
}

/// The open window's map, before anything is put on it: x is longitude and y
/// latitude, through a projection, and the title carries no mark.
fn assert_the_map(win: &Window, when: &str) {
    assert!(
        win.hero_is_a_map(),
        "{when}: the hero is not drawn as a map"
    );
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(LON),
        "{when}: the hero's x"
    );
    assert_eq!(
        win.hero_column(Channel::Y).as_deref(),
        Some(LAT),
        "{when}: the hero's y"
    );
}

/// The hero as a dot plot of `median_income` against `latitude`.
fn assert_income_against_latitude(win: &Window, when: &str) {
    assert!(
        !win.hero_is_a_map(),
        "{when}: the hero is still drawn through a projection"
    );
    assert_eq!(
        win.hero().marks,
        [MarkKind::Dot, MarkKind::Dot],
        "{when}: the hero is not its two dot layers"
    );
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "{when}: the hero's x"
    );
    assert_eq!(
        win.hero_column(Channel::Y).as_deref(),
        Some(LAT),
        "{when}: the hero's y"
    );
}

/// `e`, then `x`: the x cell's list open in the Outline, its cursor on the
/// column x holds.
fn open_x(win: &mut Window) {
    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::X, "x");
    assert_eq!(
        win.cursor().as_deref(),
        Some(LON),
        "x's list opens on x's column"
    );
}

/// **AC1.** Moving x's list cursor to `median_income` draws the hero with it
/// on x before any key keeps it, the x cell says it is a preview, and the
/// window is not marked unsaved. `Esc` then draws the map as it was, and the
/// window is still not marked.
#[test]
fn moving_xs_cursor_draws_the_column_on_x_and_esc_draws_the_hero_as_it_was() {
    let mut win = Window::fixture();
    assert_the_map(&win, "at open");
    assert!(
        !win.marked_unsaved(),
        "a window just opened is marked unsaved"
    );

    open_x(&mut win);
    win.walk_to(INCOME, egui::Key::K, "k");
    assert_income_against_latitude(&win, "with x's cursor on median_income");
    assert!(
        !win.marked_unsaved(),
        "a column only previewed marked the window unsaved"
    );
    let cell = win.cell_text(ShelfChannel::X);
    assert!(
        cell.iter().any(|t| t == INCOME) && cell.iter().any(|t| t == "preview"),
        "the x cell drew {cell:?}, not median_income as a preview"
    );

    win.press(egui::Key::Escape);
    assert_eq!(
        win.app.protocol_model().column_list().map(|l| l.channel()),
        None,
        "Esc with no query closes the list"
    );
    assert_the_map(&win, "after Esc");
    assert!(
        !win.marked_unsaved(),
        "a preview backed out of marked the window unsaved"
    );
    let cell = win.cell_text(ShelfChannel::X);
    assert!(
        cell.iter().any(|t| t == LON) && !cell.iter().any(|t| t == "preview"),
        "after Esc the x cell drew {cell:?}, not longitude as kept"
    );
}

/// **AC1, the kept spec survives a preview moved on.** A cursor moved through
/// two columns draws the second alone — the first is not left on another
/// layer or channel — and `Esc` still draws the map.
#[test]
fn a_cursor_moved_on_previews_the_next_column_on_the_kept_spec() {
    let mut win = Window::fixture();
    open_x(&mut win);
    win.walk_to("avg_occupancy", egui::Key::K, "k");
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some("avg_occupancy")
    );
    win.walk_to(INCOME, egui::Key::K, "k");
    assert_income_against_latitude(&win, "after the cursor moved on");
    win.press(egui::Key::Escape);
    assert_the_map(&win, "after Esc from a cursor moved twice");
    assert!(!win.marked_unsaved());
}

/// **AC2.** `Enter` on `median_income` keeps it: the hero draws a dot plot of
/// it against `latitude`, the x cell names it and no longer as a preview, the
/// pane's title names a dot plot of the two, and the window is marked
/// unsaved. The list closes and the band keeps the keys.
#[test]
fn enter_on_median_income_keeps_a_dot_plot_of_it_against_latitude() {
    let mut win = Window::fixture();
    assert!(
        win.hero_header_text().contains("Map"),
        "the hero's header drew {:?} at open",
        win.hero_header_text()
    );
    open_x(&mut win);
    win.walk_to(INCOME, egui::Key::K, "k");
    win.press(egui::Key::Enter);

    assert_income_against_latitude(&win, "after Enter");
    let cell = win.cell_text(ShelfChannel::X);
    assert!(
        cell.iter().any(|t| t == INCOME) && !cell.iter().any(|t| t == "preview"),
        "the x cell drew {cell:?}, not median_income as kept"
    );
    let header = win.hero_header_text();
    assert!(
        header.contains("Dot plot \u{b7} latitude \u{d7} median_income"),
        "the hero's header drew {header:?}, which does not name a dot plot of \
         latitude against median_income"
    );
    assert!(
        !header.contains("Map"),
        "the hero's header still calls it a map: {header:?}"
    );
    assert!(
        win.marked_unsaved(),
        "a kept column left the title unmarked"
    );
    assert_eq!(
        win.app.protocol_model().column_list().map(|l| l.channel()),
        None,
        "the list stays open after Enter"
    );
    assert!(win.app.shelf_holds_keys(), "the band let go of the keys");

    // Esc now leaves the band: the kept column stays.
    win.press(egui::Key::Escape);
    assert_income_against_latitude(&win, "after Esc from the band");
    assert!(win.marked_unsaved());
}

/// **AC3.** By pointer: a click on the x cell opens its list, the pointer
/// moving over a row draws the chart for it, and a click on the row keeps it,
/// leaving the spec the keys leave.
#[test]
fn by_pointer_a_row_hovered_previews_and_a_row_clicked_keeps_as_the_keys_do() {
    let mut by_keys = Window::fixture();
    open_x(&mut by_keys);
    by_keys.walk_to(INCOME, egui::Key::K, "k");
    by_keys.press(egui::Key::Enter);

    let mut win = Window::fixture();
    let cell = win.app.shelf_drawn().expect("the band drew").cells[ShelfChannel::X.index()];
    win.click(cell.center());
    assert_eq!(
        win.app.protocol_model().column_list().map(|l| l.channel()),
        Some(ShelfChannel::X),
        "a click on the x cell opens x's list"
    );

    let row = win.list_row(INCOME);
    win.run(vec![egui::Event::PointerMoved(
        row.center() - egui::vec2(0.0, 2.0),
    )]);
    win.run(vec![egui::Event::PointerMoved(row.center())]);
    win.settle();
    assert_income_against_latitude(&win, "with the pointer over median_income's row");
    assert!(
        !win.marked_unsaved(),
        "a row the pointer moved over marked the window unsaved"
    );

    let row = win.list_row(INCOME);
    win.run(vec![button(row.center(), true)]);
    win.run(vec![button(row.center(), false)]);
    win.settle();
    assert_income_against_latitude(&win, "after a click on median_income's row");
    assert!(
        win.marked_unsaved(),
        "a click that kept a column left the title unmarked"
    );
    let (pointer, keys) = (win.spec(), by_keys.spec());
    if pointer != keys {
        let (p, k) = (format!("{pointer:#?}"), format!("{keys:#?}"));
        let differ: Vec<(&str, &str)> = p
            .lines()
            .zip(k.lines())
            .filter(|(a, b)| a != b)
            .take(12)
            .collect();
        panic!("the pointer left a different spec from the keys: {differ:#?}");
    }
}

/// **AC4.** `e c` and keeping `median_house_value` paints the hero's points by
/// it, with a legend beside the plot, and the colour cell names it.
#[test]
fn e_c_and_keeping_median_house_value_paints_the_points_with_a_legend_beside_them() {
    let mut win = Window::fixture();
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "the map is painted by no column"
    );
    assert!(
        LegendSpec::from_scales(&win.hero().scales).is_none(),
        "the generated map already derives a legend"
    );

    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::C, "c");
    assert_eq!(
        win.app.protocol_model().column_list().map(|l| l.channel()),
        Some(ShelfChannel::Colour)
    );
    win.walk_to(VALUE, egui::Key::J, "j");
    win.press(egui::Key::Enter);

    assert_eq!(
        win.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "the hero's points are not painted by median_house_value"
    );
    assert!(
        win.hero_is_a_map(),
        "a colour put on the map took it off the map"
    );
    let Some(LegendSpec::Sequential { .. }) = LegendSpec::from_scales(&win.hero().scales) else {
        panic!("the hero derives no sequential legend for median_house_value");
    };
    let doc = win.app.chart_doc();
    let (legend, raster) = (
        doc.legend_rect.expect("the page reserved no legend band"),
        doc.raster_rect.expect("the page recorded no raster"),
    );
    assert!(
        legend.left() >= raster.right() - 0.5,
        "the legend {legend:?} is not beside the plot's raster {raster:?}"
    );
    assert!(
        win.cell_names(ShelfChannel::Colour, VALUE),
        "the colour cell drew {:?}, not median_house_value",
        win.cell_text(ShelfChannel::Colour)
    );
    assert_eq!(
        win.app.shelf_band().map(|b| b.channels().colour.clone()),
        Some(Binding::Column(VALUE.to_string()))
    );
    assert!(win.marked_unsaved());
}

/// **AC5.** Save writes the column kept and not the column previewed and
/// backed out of. `median_income` is kept on x; `median_house_value` is then
/// previewed on colour and backed out of with `Esc`. The chart file Save
/// writes is the generator's text with the x edit alone placed into it — each
/// layer's `x:` reads `median_income` and the projection line is gone — and
/// no line for the colour, its scheme or its legend; and the Save clears the
/// unsaved mark.
#[test]
fn save_writes_the_kept_column_and_not_the_one_backed_out_of() {
    let mut win = Window::housing("save");
    let generated = win.generated_text();
    assert_eq!(
        generated.matches("        x: 'longitude'\n").count(),
        2,
        "the generated map's two layers each carry their x"
    );
    open_x(&mut win);
    win.walk_to(INCOME, egui::Key::K, "k");
    win.press(egui::Key::Enter);
    assert!(win.marked_unsaved());

    // The band still holds the keys: `c` opens colour's list.
    win.type_letter(egui::Key::C, "c");
    win.walk_to(VALUE, egui::Key::J, "j");
    assert_eq!(
        win.hero_column(Channel::Fill).as_deref(),
        Some(VALUE),
        "the colour preview was not drawn"
    );
    win.press(egui::Key::Escape);
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "Esc left the colour preview drawn"
    );
    assert_income_against_latitude(&win, "after the colour preview was backed out of");

    let ctx = win.ctx.clone();
    win.app
        .save_protocol(&ctx)
        .expect("a data file's window has a Protocol to save")
        .expect("the Protocol saves");
    win.settle();
    assert!(
        !win.marked_unsaved(),
        "a Save that wrote the chart left the mark"
    );

    let chart = std::fs::read_dir(win.folder.join("panels"))
        .expect("Save wrote a panels folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "yaml"))
        .expect("Save wrote a chart file");
    let written = std::fs::read_to_string(&chart).expect("the chart file reads");
    let expected = generated
        .replace("        x: 'longitude'\n", "        x: 'median_income'\n")
        .replace("      projectionType: equirectangular\n", "");
    assert_eq!(
        written, expected,
        "the chart file is not the generated text with the kept x edit alone"
    );
}

/// **AC5, the edits a kept column carries are the kept spec's.** The cursor of
/// colour's list passes through the columns above `median_house_value`, each
/// drawn as a preview, before `Enter` keeps it. The chart file Save writes is
/// the generator's text with the edits `put_colour` makes for that column on
/// the generated spec — its `fill`, its scheme and its legend — written into
/// it by the writer Save calls, and not the one `fill` a put onto the last
/// preview's spec would make, which already had a scheme and a legend.
#[test]
fn a_column_kept_after_the_cursor_passed_others_saves_the_edits_the_kept_spec_needs() {
    let mut win = Window::housing("passed");
    let generated = win.generated_text();
    let mut spec = win.spec();
    let table = win
        .app
        .protocol_model()
        .source()
        .expect("a data file's window has its one-step Protocol")
        .profiles
        .clone();
    let hero = ComponentPath(win.hero().path.clone());
    let edits = put_colour(&mut spec, &hero, VALUE, &table).expect("the table has the column");
    assert!(
        edits.len() > 1,
        "put_colour made {edits:?}: the scheme and legend this test pins are not among them"
    );
    let expected = edits.iter().fold(generated, |text, edit| {
        write_chart_edit(&text, edit).expect("the writer places the shelf's edit")
    });

    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::C, "c");
    win.walk_to(VALUE, egui::Key::J, "j");
    win.press(egui::Key::Enter);
    let ctx = win.ctx.clone();
    win.app
        .save_protocol(&ctx)
        .expect("a data file's window has a Protocol to save")
        .expect("the Protocol saves");

    let chart = std::fs::read_dir(win.folder.join("panels"))
        .expect("Save wrote a panels folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "yaml"))
        .expect("Save wrote a chart file");
    let written = std::fs::read_to_string(&chart).expect("the chart file reads");
    assert_eq!(
        written, expected,
        "the chart file is not the generated text with the kept colour's edits"
    );
}
