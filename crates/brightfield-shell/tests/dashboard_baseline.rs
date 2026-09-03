//! **What the generator chose for a table it had never met** — held twice over
//! one committed file: as the kind each column was given, by name, and as the
//! picture those choices compose into.
//!
//! # Why both, and why the structural half leads
//!
//! The picture is the product: it is what a reader is shown when they hand
//! Brightfield a data file. But an image diff reddens on a font bump and a
//! colour-token change exactly as loudly as on a moved tile choice, so a
//! reviewer holding one red baseline cannot tell which of those happened — and
//! the cheapest response to an image that "just moved" is to re-record it. The
//! choice table below is what makes the other failure legible: it names the
//! column, the kind and what decided the kind, so a moved choice fails with a
//! sentence instead of with pixels.
//!
//! [`assert_choices`] therefore runs **before** `image_snapshot` inside the
//! pixel test as well as in its own. Under `UPDATE_SNAPSHOTS=1` a snapshot call
//! writes whatever it was handed, so a guard sitting behind one would author a
//! baseline of a dashboard whose choices had already moved and complain about
//! it afterwards — the ordering `tests/surfaces.rs` records for its scripted
//! captures, here for the same reason.
//!
//! # How the dashboard reaches a capture: no second path
//!
//! `brightfield-shot` takes `--spec`, and this dashboard arrives by opening a
//! **data file**. The two were already reconciled: `Boot::open_sampled`
//! classifies a path naming a data file first and hands it to
//! [`Boot::data_file`], so `brightfield-shot --spec table.csv --out out.png`
//! renders this same picture through this same code today — the classification
//! step being what `a_path_on_the_command_line_opens_as_the_generated_dashboard`
//! holds, in `tests/scripted_open.rs`. What is photographed
//! here is that boot, run through [`brightfield_shell::capture::capture_png`] —
//! the crate's own headless path, which is what the shot binary runs and what
//! the live window runs — with the resulting image handed to
//! `egui_kittest::image_snapshot`. So the comparison, the `kittest.toml`
//! thresholds and the regeneration workflow are the sheet tier's, and the
//! diff is perceptual rather than byte-exact: a byte-exact dashboard baseline
//! would fail on text antialiasing and be switched off inside a week.
//!
//! Regenerate the baseline with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p
//! brightfield-shell --test dashboard_baseline`.
//!
//! Thresholds come from `kittest.toml` at the workspace root — read the policy
//! comment there before reaching for a per-test override. This baseline was
//! recorded at the repo floor and needs none.
//!
//! # Why the DuckDB type is what decides here
//!
//! `LoadOptions::packaged` looks for a FineType bundle beside the running
//! executable, and a `cargo test` binary has none beside it, so each column of
//! the fixture arrives carrying `SemanticType::NotAsked` and its storage type
//! takes the decision. That is asserted rather than assumed: [`EXPECTED`]
//! carries what decided each tile, so a run in which a bundle *is* present
//! fails naming the label it found instead of quietly photographing a
//! different dashboard.
//!
//! # This tier needs a GPU
//!
//! The capture rasterises through a real wgpu adapter, like `tests/snapshot.rs`
//! and `tests/surfaces.rs`, and there is deliberately no skip switch here
//! either — an env-var opt-out would render "no GPU here" as a passing test.

use std::path::PathBuf;

use brightfield_shell::capture::capture_png;
use brightfield_shell::dashboard::{self, ChosenBy, Dashboard, Omission};
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_shell::{chart_kinds, data_file, ranked_bars};
use brightfield_workbench::registry::ChartKindId;

/// Device pixels per logical point for this baseline.
///
/// `tests/surfaces.rs`'s scale, for the reason recorded there: the perceptual
/// gate is a per-pixel delta rather than a per-image one, so a lower raster
/// buys no slack — it just stores less of it.
const SCALE: f32 = 1.0;

/// The committed table the dashboard under test is generated from.
///
/// Four columns picked so the walk has a different answer for each: a `DATE`, a
/// `VARCHAR` of four regions, a `BIGINT` of readings, and a sensor id holding
/// one distinct value. Addressed from the crate root so the test does not
/// depend on the shell's working directory.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/dashboard_baseline.csv")
}

/// **The table the committed picture is of**: nine numeric columns named and
/// ordered as California Housing's are, two of them a coordinate pair.
///
/// A committed **sample**, not the dataset — the real Parquet is 16,640 rows
/// and belongs in `open-analytics` rather than in this repo's test data. What
/// it shares with the real file is everything the picture depends on: the nine
/// columns in file order, a coordinate pair among them so the generator draws
/// a map, and seven other columns that each earn a tile.
///
/// The choice table above stays on [`fixture`], whose four columns are four
/// different shapes and answer a different question — which kind each *type*
/// earns. This one answers what the first screen looks like, which is a
/// question about a file with a map in it.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// The tiles [`housing`] earns, in the order the composition places them: the
/// map first, then its seven columns in the file's own order.
const HOUSING_PLOTS: &[&str] = &[
    "longitude",
    "median_income",
    "house_age",
    "avg_rooms",
    "avg_bedrooms",
    "population",
    "avg_occupancy",
    "median_house_value",
];

/// **The structural half of the picture below**: the map is the hero, its pair
/// is the two coordinate columns, and the seven others stack beside it in file
/// order.
///
/// Runs ahead of `image_snapshot` for the reason [`assert_choices`] does, and
/// it is the assertion that makes a red baseline legible: a photograph of a
/// dashboard whose hero had moved and one of a dashboard whose font had moved
/// differ by the same kind of pixel diff.
fn assert_housing(dash: &Dashboard) {
    let drawn: Vec<&str> = dash.plot_order().iter().map(|t| t.column()).collect();
    assert_eq!(
        drawn,
        HOUSING_PLOTS.to_vec(),
        "the tiles this picture is of, or the order the composition places \
         them in, have moved. The first is the hero the map pane holds and \
         the rest are the column beside it."
    );
    let hero = &dash.plot_order()[0];
    assert_eq!(
        hero.kind(),
        chart_kinds::POINT_MAP,
        "the hero is not the point map, so the map pane is holding something \
         else and the picture below is not the first screen"
    );
    assert_eq!(
        hero.paired_column(),
        Some("latitude"),
        "the map's paired column moved"
    );
    assert_eq!(
        dash.column_tiles().len(),
        7,
        "the column holds {} tiles rather than the seven the file earns",
        dash.column_tiles().len()
    );
    assert!(
        dash.omitted().is_empty(),
        "a column was left out of this dashboard: {:?}",
        dash.omitted()
    );
}

/// **A table small like [`housing`], with a coordinate pair AND a date
/// column** — the shape [`fixture`] does not have.
///
/// [`fixture`]'s `day` earns no picture of a real column tile: with no
/// coordinate pair in that file, [`dashboard::Dashboard::hero_index`] falls
/// back to the first tile, so `day` draws in the map pane at the hero's own
/// (much wider) share of the page rather than at a column tile's width. This
/// table gives the coordinate pair its own hero (`longitude`/`latitude`, one
/// of three repeated sites) so `day` stacks beside it as an ordinary column
/// tile instead — which is the width the counts_over_time axis actually
/// draws at whenever a file's map takes the hero's place, the shape the
/// time-axis collision was recorded against. `reading` earns the column's
/// second tile so the width
/// [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`] reads
/// is a real dashboard's, not a single-tile one.
fn site_readings() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/site_readings_sample.csv")
}

/// The tiles [`site_readings`] earns, in the order the composition places
/// them: the map first (named for its longitude column, [`assert_housing`]'s
/// convention), then `day` and `reading` beside it in file order.
const SITE_READINGS_PLOTS: &[&str] = &["longitude", "day", "reading"];

/// **The structural half of the picture [`the_site_readings_dashboard_light_baseline`]
/// and its dark twin capture**, [`assert_housing`]'s pattern read against
/// [`site_readings`] instead: the map is the hero, `day` and `reading` stack
/// beside it, and nothing was left out.
fn assert_site_readings(dash: &Dashboard) {
    let drawn: Vec<&str> = dash.plot_order().iter().map(|t| t.column()).collect();
    assert_eq!(
        drawn,
        SITE_READINGS_PLOTS.to_vec(),
        "the tiles this picture is of, or the order the composition places \
         them in, have moved. The first is the hero the map pane holds and \
         the rest are the column beside it."
    );
    let hero = &dash.plot_order()[0];
    assert_eq!(
        hero.kind(),
        chart_kinds::POINT_MAP,
        "the hero is not the point map, so the map pane is holding something \
         else and day no longer draws as a column tile"
    );
    assert_eq!(
        hero.paired_column(),
        Some("latitude"),
        "the map's paired column moved"
    );
    assert_eq!(
        dash.column_tiles().len(),
        2,
        "the column holds {} tiles rather than the two (day, reading) the \
         file earns",
        dash.column_tiles().len()
    );
    assert!(
        dash.omitted().is_empty(),
        "a column was left out of this dashboard: {:?}",
        dash.omitted()
    );
}

/// Where the capture's intermediate PNG goes. Under the target dir, already
/// git-ignored, so a concurrent test cannot race this one on a path.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// **The tile choice, as a table**: which kind each column of the fixture was
/// given, by name, and the DuckDB type that decided it — in the table's own
/// column order, which is the order the dashboard reads in.
///
/// This is the assertion the committed image cannot make. Two kinds can ink
/// similarly at one tile size, and a reader comparing photographs would not
/// see the swap; a reader of a failure naming `region: ranked-category-bars`
/// against `region: binned-histogram` cannot miss it.
const EXPECTED: &[(&str, ChartKindId, &str)] = &[
    ("day", chart_kinds::COUNTS_OVER_TIME, "DATE"),
    ("region", ranked_bars::KIND_ID, "VARCHAR"),
    ("reading", chart_kinds::BINNED_HISTOGRAM, "BIGINT"),
];

/// The column of the fixture that earns no tile, and why: one distinct value,
/// whose histogram is one bar and whose ranking is one row.
///
/// Pinned beside the tiles because a column vanishing from a generated
/// analysis is indistinguishable from a bug in the generator, and the picture
/// alone cannot tell the reader which — the omission is written into the
/// emitted spec's comment block, not drawn.
const OMITTED_COLUMN: &str = "sensor";

/// The declaration order of the kinds a lone column can fill.
///
/// [`dashboard::single_column_kinds`] answers in the registry's own order and
/// the chooser takes the first kind that accepts the field, so this list *is*
/// the preference between two applicable kinds. The three kinds declare
/// disjoint slot types today, so reordering them moves no tile in this fixture
/// — which is why the order is pinned here rather than left to the choice
/// table: a reorder is a change to the tiebreak, and this assertion is what
/// reports it.
const PREFERENCE: &[ChartKindId] = &[
    chart_kinds::BINNED_HISTOGRAM,
    chart_kinds::COUNTS_OVER_TIME,
    ranked_bars::KIND_ID,
];

/// [`EXPECTED`] as the lines [`chosen_lines`] produces, so a failure prints two
/// readable lists rather than two debug dumps.
fn expected_lines() -> Vec<String> {
    EXPECTED
        .iter()
        .map(|(column, kind, type_name)| format!("{column}: {kind} (from {type_name})"))
        .collect()
}

/// What the generator actually chose, one line per tile: the column, the kind,
/// and what decided the kind.
fn chosen_lines(dash: &Dashboard) -> Vec<String> {
    dash.tiles()
        .iter()
        .map(|tile| {
            let because = match tile.chosen_by() {
                ChosenBy::Storage { type_name } => format!("from {type_name}"),
                ChosenBy::Meaning { label, role } => {
                    format!("from the label {label}, read as {role:?}")
                }
                ChosenBy::CoordinatePair { latitude, rule } => {
                    format!("paired with {latitude} by its {rule}")
                }
            };
            format!("{}: {} ({because})", tile.column(), tile.kind())
        })
        .collect()
}

/// **The structural half of this baseline**: the kind each column was given, by
/// name, and the column that was given none.
///
/// Called by the pixel test before it photographs anything, and asserted on its
/// own below.
fn assert_choices(dash: &Dashboard) {
    assert_eq!(
        chosen_lines(dash),
        expected_lines(),
        "the generator's tile choices for {} have moved. Left is what it chose \
         on this run; right is what the committed baseline image was recorded \
         against. A different kind for a column is a different dashboard, \
         however similar the two ink at this size — re-recording the image \
         without reading this line is the failure this assertion exists to \
         stop.",
        fixture().display()
    );

    let omitted: Vec<&str> = dash.omitted().iter().map(|o| o.column.as_str()).collect();
    assert_eq!(
        omitted,
        vec![OMITTED_COLUMN],
        "a different set of columns was left out of the dashboard for {}, and \
         an omission is invisible in the picture — it is written into the \
         emitted spec's comment block and nowhere else",
        fixture().display()
    );
    let left = &dash.omitted()[0];
    assert!(
        matches!(left.because, Omission::OneValue),
        "{OMITTED_COLUMN} was left out for {:?} rather than for holding one \
         distinct value, so the fixture no longer exercises the rule it was \
         written to exercise",
        left.because
    );
}

/// **Which kind each column of a table this build has never met is given** —
/// the choice, per column, by name.
///
/// The claim the committed image cannot carry on its own, and the one that has
/// to fail legibly: a semantic-type change, a new kind whose required slot a
/// lone column happens to fill, or a registry reordering each move these
/// answers, and this is the assertion that says which column moved and to what.
///
/// A column of each of three kinds, so the fixture says something about the
/// choosing rather than about one type: a date counted over time, a category
/// ranked, a measure binned — and a fourth column given nothing.
#[test]
fn each_column_of_the_table_gets_the_tile_its_type_earns() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_choices(&opened.dashboard);
}

/// **The preference between two applicable kinds is the registry's declaration
/// order**, and that order is here rather than only in the registry.
///
/// [`dashboard::single_column_kinds`] filters the registry to the kinds one
/// column can fill and keeps the registry's order; the chooser takes the first
/// of them that accepts the field. Swapping two declarations is therefore a
/// change to which picture a column gets — and, with the slot types the three
/// kinds declare today, a change that moves no pixel and no tile, so the choice
/// table above would stay green through it.
#[test]
fn the_preference_between_applicable_kinds_is_the_registrys_declaration_order() {
    let declared: Vec<&str> = dashboard::single_column_kinds()
        .iter()
        .map(|kind| kind.id.as_str())
        .collect();
    let pinned: Vec<&str> = PREFERENCE.iter().map(|id| id.as_str()).collect();
    assert_eq!(
        declared, pinned,
        "the kinds a lone column can fill are declared in a different order \
         than the dashboard baseline was recorded against. That order is the \
         preference — the chooser takes the first kind whose slots accept the \
         column's field — so a swap here changes which picture a column gets \
         wherever two kinds accept one field type."
    );
}

/// **The dashboard generated for a data file, as pixels** — the composed
/// picture a reader is shown for a table nobody wrote a spec for.
///
/// The boot is [`Boot::data_file`]'s, which is the boot the front door's picker
/// builds and the boot `brightfield-shot --spec table.csv` builds; the capture
/// is the crate's own headless path. The table is [`housing`] — the first
/// screen is a picture of a file with a map in it. The dashboard that chose the tiles is
/// asked for separately, because a `Boot` carries the composed document rather
/// than the walk that produced it.
///
/// The tile CHOICES are mode-independent — they are read off column types, not
/// off ink — so [`assert_housing`] runs here and the dark twin below inherits
/// its verdict rather than restating it. What the pair holds that neither half
/// can alone is that the ink moves and nothing else does.
#[test]
fn the_generated_dashboard_light_baseline() {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");

    // The structural guard, ahead of the photograph, for the reason in this
    // file's header: `UPDATE_SNAPSHOTS=1` writes whatever `image_snapshot` is
    // handed.
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_housing(&opened.dashboard);
    assert_eq!(
        opened.composed.plots.len(),
        HOUSING_PLOTS.len(),
        "the walk chose {} tiles and the composition placed {} plots, so the \
         image below is not a picture of those choices",
        HOUSING_PLOTS.len(),
        opened.composed.plots.len()
    );
    drop(opened);

    // Hermetic capture: keep `BRIGHTFIELD_DEVTOOLS` from baking the top bar's
    // renderer string into a regenerated golden, as `tests/surfaces.rs` does.
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("dashboard_light");
    let (w, h) = capture_png(boot, Mode::Light, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture dashboard_light: {e}"));
    assert!(w > 0 && h > 0, "dashboard_light: empty capture");

    // PNG is lossless, so reading the capture back is pixel-exact; the file on
    // disk is the way `capture_png` hands its result over.
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "dashboard_light");
}

/// Device-pixel count of exactly `token` in `image`.
///
/// Exact, not perceptual: an interior pixel of a filled rect is the fill
/// colour, and the two surface tokens under test differ by far more than any
/// rounding in the Rgba8Unorm round-trip. A tolerance here would let the defect
/// through under the name of robustness.
fn pixels_of(image: &image::RgbaImage, token: meridian_design::colour::Rgba) -> usize {
    let want = [
        (token.r * 255.0).round() as u8,
        (token.g * 255.0).round() as u8,
        (token.b * 255.0).round() as u8,
    ];
    image
        .pixels()
        .filter(|p| p.0[0] == want[0] && p.0[1] == want[1] && p.0[2] == want[2])
        .count()
}

/// **The same generated dashboard in dark** — and, held in the same test, the
/// claim the picture is here to make: **not one pixel of it is the light chart
/// surface.**
///
/// The image half is the baseline; the pixel half is what makes a red baseline
/// legible. A dark window whose chart pane is a white slab differs from this
/// golden in tens of thousands of pixels and a reviewer reading a perceptual
/// diff cannot tell that from a font bump — so the surface count is asserted by
/// name, ahead of the photograph, for the reason [`assert_choices`] runs ahead
/// of it.
///
/// This dashboard reaches dark through a path the light twin does not exercise:
/// [`Boot::data_file`] composes before anything knows the mode, and
/// `ChartDoc::set_mode` re-presents through the live session it left behind on
/// the first frame that names one. So this is also the regression test for that
/// seam — remove it and the capture goes back to photographing a light
/// composition inside a dark window.
#[test]
fn the_generated_dashboard_dark_baseline() {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("dashboard_dark");
    let (w, h) = capture_png(boot, Mode::Dark, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture dashboard_dark: {e}"));
    assert!(w > 0 && h > 0, "dashboard_dark: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();

    let light = pixels_of(&image, meridian_design::chrome::INK_LIGHT.surface);
    assert_eq!(
        light, 0,
        "{light} pixels of this dark dashboard are the LIGHT chart surface \
         (#fcfcfb). That colour has one source — the plot background — so this \
         window is drawing a white slab exactly where the analyst is reading."
    );
    let dark = pixels_of(&image, meridian_design::chrome::INK_DARK.surface);
    assert!(
        dark > 0,
        "no pixel of this dark dashboard is the dark chart surface (#161413), \
         so the plot background is neither of the two colours it can be and \
         the assertion above is passing for the wrong reason"
    );

    egui_kittest::image_snapshot(&image, "dashboard_dark");
}

/// The window the scrolled capture below is taken in — the size the
/// composition this card is cut from was drawn at, and short enough that seven
/// tiles at their 96-point floor need a page taller than the pane.
const SHORT_WINDOW: (f32, f32) = (1440.0, 900.0);

/// One turn of the wheel in logical points, and how many frames carry one.
///
/// Enough travel to reach the end of the column's scroll, which is where the
/// page is furthest from where it was composed and therefore where a missing
/// clip paints over the most chrome. The offset the frame reached is clamped by
/// the window, so over-turning the wheel is how a test scrolls "to the end"
/// without naming a distance.
const WHEEL_TRAVEL: f32 = 400.0;
/// How many frames of [`WHEEL_TRAVEL`] the scripted capture turns.
const WHEEL_FRAMES: usize = 8;

/// A settled headless window over [`housing`] at `size`, for reading the pane
/// rects the capture below is measured against.
///
/// The layout is the one the capture runs: `MeridianApp::headless` differs from
/// the device path in the raster alone — the canvas pane reserves the same box
/// and paints nothing into it — which is the property `tests/canvas_pane_group.rs`
/// is built on.
fn pane_rects(size: (f32, f32)) -> (Vec<brightfield_shell::window::CanvasPane>, egui::Pos2) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0, size.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    // The control that reopens the ledger rail, read before it is clicked and
    // returned for the capture to aim at. A data file opens as a one-step
    // Protocol, so the rail opens closed to its strip and hands the canvas its
    // other 124 points; at this window that is exactly enough for the column's
    // seven tiles at their floor, and a page that fits its pane is a page no
    // clip and no scroll can be asserted about. See `settled_scrollable` in
    // `tests/canvas_pane_group.rs`, which reopens it for the same reason.
    let control = app
        .rail_collapse_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
        .expect("the collapsed ledger drew the control that reopens it")
        .center();
    for events in reopen_the_ledger(control) {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    }
    (app.canvas_panes().panes.clone(), control)
}

/// The frames that reopen a collapsed ledger rail by clicking `control` — one
/// to put the pointer there, one carrying the press and release, and three to
/// settle the panel egui reads back on the frame after.
fn reopen_the_ledger(control: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: control,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(control)],
        vec![button(true), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

/// **The two panes clip their own share of the one page, and a scroll moves
/// one of them** — held over a pair of captures at 1440 by 900, where seven
/// tiles at their 96-point floor need a page taller than either pane.
///
/// Three claims, and the pair is what makes the middle one decidable:
///
/// 1. the wheel over the column **moved** it — the column's content rect
///    differs between the two captures, so nothing below is being asserted
///    about a window where the scroll did nothing;
/// 2. it moved **nothing else** — every pixel outside the column's content
///    rect is identical in the two captures, the map's picture and both header
///    bands included. This is [`the_generated_dashboard_light_baseline`]'s
///    claim about a scrolled window, and it is what reddens when the column's
///    second view stops clipping: an unclipped copy of the scrolled page
///    paints across the map pane and over both bands;
/// 3. no pixel of the marks' own ink lands outside a pane's content rect in
///    either capture. This is what reddens when `draw_chart_body` stops
///    clipping the page it lays out: the page is 84 points taller than the
///    panes at this window, and what hangs below is the last tile's bars.
///
/// Both clips are one line each — `child.shrink_clip_rect(clip)` and
/// `Painter::with_clip_rect` — and deleting either left the rest of this
/// crate's test targets green.
#[test]
fn the_pane_group_clips_the_page_to_the_panes_it_is_drawn_in() {
    let (panes, ledger_control) = pane_rects(SHORT_WINDOW);
    assert_eq!(
        panes.len(),
        3,
        "the canvas drew {} panes at {SHORT_WINDOW:?}, so the rects below are \
         not the pane group's",
        panes.len()
    );
    let bodies: Vec<egui::Rect> = panes.iter().map(|p| p.body).collect();
    let column = panes
        .iter()
        .find(|p| p.name == "columns")
        .expect("the column pane drew")
        .body;

    // The pointer lands in the same place in both captures and the wheel is
    // the only difference between them, so a pixel that differs is one the
    // scroll moved. Two empty frames lead: a resizable panel's reported size
    // is read back on the frame after, so the pointer has to land on a settled
    // layout.
    let mut point = reopen_the_ledger(ledger_control);
    point.extend([
        Vec::new(),
        Vec::new(),
        vec![egui::Event::PointerMoved(column.center())],
    ]);
    let mut turn = point.clone();
    for _ in 0..WHEEL_FRAMES {
        turn.push(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -WHEEL_TRAVEL),
            modifiers: egui::Modifiers::default(),
            phase: egui::TouchPhase::Move,
        }]);
    }
    let still = capture_short(point, "dashboard_pane_group_still");
    let moved = capture_short(turn, "dashboard_pane_group_scrolled");
    assert_eq!(still.dimensions(), moved.dimensions());

    let mut in_column = 0usize;
    let mut elsewhere: Vec<(u32, u32)> = Vec::new();
    for (x, y, p) in still.enumerate_pixels() {
        if p == moved.get_pixel(x, y) {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let at = egui::pos2(x as f32 / SCALE, y as f32 / SCALE);
        if column.expand(1.0).contains(at) {
            in_column += 1;
        } else {
            elsewhere.push((x, y));
        }
    }
    assert!(
        in_column > 0,
        "the two captures are identical inside the column's content rect \
         {column:?}, so the wheel scrolled nothing and the comparison below \
         holds over a window this test is not about"
    );
    assert!(
        elsewhere.is_empty(),
        "{} device pixels outside the column's content rect changed when the \
         column scrolled — the first five at {:?}. Scrolling the column moves \
         the column; the map's picture, both header bands and every frame \
         around them are somebody else's.",
        elsewhere.len(),
        &elsewhere[..elsewhere.len().min(5)]
    );

    // …and each pane's own bottom frame survives the page laid out over it.
    // This is the one the mark ink cannot see: what the page paints into a
    // pane's inset is its BACKGROUND, and the chart surface and a pane's fill
    // are the same token, so the visible loss is the hairline the page covers.
    //
    // Measured at this window, with the clip: the map pane's strip carries the
    // border colour across its full width on one device row — its own stroke,
    // with the pane gap under it in the canvas's own fill — and the rows and
    // column panes on two, their own stroke and the hairline of the rail
    // below them. Without the clip the map pane shows NONE: the page is laid
    // out across the union of the map's and the column's content rects, which
    // since the map gave the foot of its column to the rows pane reaches over
    // the map's bottom frame. The `rows >= 1` below is that measurement as an
    // assertion, and the map pane is the one it bites on.
    let border = meridian_design::semantic(false).borders.subtle;
    for pane in &panes {
        let strip = pane.rect.bottom() - pane.body.bottom();
        assert!(
            strip > 1.0,
            "the {} pane's content rect ends at its own bottom edge, so there \
             is no frame below it to paint over and this claim is empty",
            pane.name
        );
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = (0..(strip * SCALE).ceil() as u32 + 1)
            .filter(|dy| {
                let y = (pane.body.bottom() * SCALE) as u32 + dy;
                let run = ((pane.body.left() * SCALE) as u32..(pane.body.right() * SCALE) as u32)
                    .filter(|x| {
                        y < still.height()
                            && *x < still.width()
                            && is_token(still.get_pixel(*x, y), border)
                    })
                    .count();
                #[allow(clippy::cast_precision_loss)]
                let across = run as f32 >= 0.9 * pane.body.width() * SCALE;
                across
            })
            .count();
        assert!(
            rows >= 1,
            "the {} pane's bottom frame runs the pane's full width on {rows} \
             device rows below its content rect — the page is painting over \
             the frame of the pane it is drawn in",
            pane.name
        );
    }

    for (name, image) in [("still", &still), ("scrolled", &moved)] {
        let mut ink = 0usize;
        let mut stray: Vec<(u32, u32)> = Vec::new();
        for (x, y, p) in image.enumerate_pixels() {
            if !is_mark_ink(p) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let at = egui::pos2(x as f32 / SCALE, y as f32 / SCALE);
            if bodies.iter().any(|body| body.expand(1.0).contains(at)) {
                ink += 1;
            } else {
                stray.push((x, y));
            }
        }
        assert!(
            ink > 0,
            "not one pixel of the {name} capture is the marks' own ink, so the \
             count below is passing for the wrong reason — the picture did not \
             draw"
        );
        assert!(
            stray.is_empty(),
            "{} device pixels of the marks' own ink landed outside both panes' \
             content rects {bodies:?} in the {name} capture — the first five \
             at {:?}. The page is painting where no picture is drawn.",
            stray.len(),
            &stray[..stray.len().min(5)]
        );
    }
}

/// [`housing`] captured at [`SHORT_WINDOW`] under `script`, read back as
/// pixels.
fn capture_short(script: Vec<Vec<egui::Event>>, name: &str) -> image::RgbaImage {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at(
        boot,
        Mode::Light,
        SCALE,
        SHORT_WINDOW,
        &out,
        script,
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// Whether `p` is the first series colour of the chart palette — the ink every
/// mark of this dashboard is drawn in.
///
/// The right ink to count for a clip: it has exactly one source in the window.
/// The chart *surface* would be the wrong one — a pane's own frame is filled
/// with the same token, so a page painting over a pane's inset would be
/// invisible to it: the surface colour is already down there, below the panes'
/// content rects, with the clip in place. The mark ink is not, and that is the
/// assertion — `stray.is_empty()` in
/// [`the_pane_group_clips_the_page_to_the_panes_it_is_drawn_in`].
///
/// Exact rather than perceptual, for the reason [`pixels_of`] gives: a mark's
/// interior is the flat fill, and antialiasing at its edge produces neighbours
/// this deliberately does not count.
fn is_mark_ink(p: &image::Rgba<u8>) -> bool {
    is_token(p, meridian_design::viz::CATEGORICAL_LIGHT[0])
}

/// Whether `p` is exactly `token`, alpha ignored.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn is_token(p: &image::Rgba<u8>, token: meridian_design::colour::Rgba) -> bool {
    p.0[0] == (token.r * 255.0).round() as u8
        && p.0[1] == (token.g * 255.0).round() as u8
        && p.0[2] == (token.b * 255.0).round() as u8
}

// ---------------------------------------------------------------------------
// A time axis at a dashboard tile's real width
// ---------------------------------------------------------------------------

/// The `day` column's real dates, walked off the composed dashboard's own
/// resolved `Scale::Band` for [`fixture`] rather than typed a second time —
/// a change to the fixture's dates cannot leave this test checking a set the
/// picture no longer draws.
fn fixture_day_categories(composed: &brightfield_shell::pipeline::Composed) -> Vec<String> {
    let day_plot = composed
        .plots
        .iter()
        .find(|p| p.x_column.as_deref() == Some("day"))
        .expect("fixture check: the day column earns a tile with x: day");
    match day_plot.scales.get(brightfield_render::channel::Channel::X) {
        Some(brightfield_render::scale::Scale::Band { categories, .. }) => categories.clone(),
        other => panic!("fixture check: day's x scale is not a band scale: {other:?}"),
    }
}

/// The plot `composed` placed whose x channel is bound to `column`, found by
/// that binding rather than by position — a hero promotion or a column
/// reorder must point this at a different plot, not silently keep reading the
/// same index.
fn plot_for_x_column<'a>(
    composed: &'a brightfield_shell::pipeline::Composed,
    column: &str,
) -> &'a brightfield_shell::pipeline::PlotHandle {
    composed
        .plots
        .iter()
        .find(|p| p.x_column.as_deref() == Some(column))
        .unwrap_or_else(|| panic!("fixture check: no placed plot binds x to {column}"))
}

/// `(x0, y0, x1, y1)` — an axis-aligned rect in the composed PAGE's own
/// coordinate space, the same space [`brightfield_shell::pipeline::PlotHandle::rect`]
/// lives in.
type Rect4 = (f64, f64, f64, f64);

/// Whether `a` and `b` share any interior point.
fn rects_intersect(a: Rect4, b: Rect4) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Whether `inner` lies inside `outer`, both edges included up to float
/// round-trip slop.
fn rect_inside(inner: Rect4, outer: Rect4) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.2 <= outer.2 + 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.3 <= outer.3 + 0.5
}

/// Every rect `render_x_axis` (`brightfield-render`'s `axis` module) drew for
/// `plot`'s x axis, read straight out of the real composed `scene` —
/// [`Composed::scene`]'s glyph runs, not a second render — rather than
/// assumed from which branch it is expected to have taken: the tick labels
/// (whichever family `render_x_axis` chose — thinned, degraded to one, or
/// rotated) as the first element, and the axis title's rect, when the tile
/// draws one, as the second.
///
/// A run's own [`vello_encoding::GlyphRun::font_size`] is what tells a title
/// apart from a tick label — the SAME field `render_x_axis` sets when it
/// draws each (`TITLE_SIZE` for the title, `size` for every tick label,
/// whichever way it drew them), rather than the two being told apart by
/// where they landed. A ROTATED run's rect runs the other way from a
/// horizontal one: its OWN glyph-height footprint (`size` wide) is centred on
/// its pivot along x, and its measured text length runs DOWN from the pivot
/// along y — the near end is nearest the tick
/// (`draw_text_rotated`'s `TextAnchor::End`) — so a run that has clipped past
/// the tile or under the title is exactly one whose far end has run past
/// `plot.rect.height` or into the title's own rect, which
/// `rect_inside`/`rects_intersect` read directly off the rects this returns.
fn drawn_x_axis_rects(
    scene: &vello::Scene,
    plot: &brightfield_shell::pipeline::PlotHandle,
    ticks: &[brightfield_render::axis::Tick],
    title_text: &str,
    size: f32,
) -> (Vec<Rect4>, Option<Rect4>) {
    let title_size = brightfield_render::text::TITLE_SIZE;
    let title_width = brightfield_render::text::measure_width(title_text, title_size);

    let x_lo = plot.rect.x - 200.0;
    let x_hi = plot.rect.x + plot.rect.width + 200.0;
    // Half a label's cap height below the axis line, not the line itself:
    // the y-axis's own occasional stray label (a y-tick landing exactly on
    // `plot_y_end` draws its row-label `LABEL_SIZE / 3` below the line) sits
    // short of this floor, while the x-axis's tick-label row (`LABEL_SIZE`
    // below the line, or the rotated pivots' `ROTATED_LABEL_GAP` short of
    // that) clears it.
    let y_lo = plot.rect.y + plot.layout.plot_y_end() + f64::from(size) / 2.0;
    let y_hi = plot.rect.y + plot.rect.height;

    // A horizontal draw (`TextAnchor::Middle`) anchors its LEFT edge at
    // `position - width / 2`; a rotated one (`TextAnchor::End`) anchors its
    // PIVOT exactly at `position` — two different predicted x0s for the same
    // tick, so two candidate lists, matched against whichever family a given
    // run turns out to belong to.
    let horiz_candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| {
            (
                plot.rect.x + t.position
                    - brightfield_render::text::measure_width(&t.label, size) / 2.0,
                t.label.as_str(),
            )
        })
        .collect();
    let rotated_candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| (plot.rect.x + t.position, t.label.as_str()))
        .collect();

    let mut label_rects = Vec::new();
    let mut title_rect = None;
    for run in &scene.encoding().resources.glyph_runs {
        let x0 = f64::from(run.transform.translation[0]);
        let y0 = f64::from(run.transform.translation[1]);
        if x0 < x_lo || x0 > x_hi || y0 < y_lo || y0 > y_hi {
            continue;
        }
        if (run.font_size - title_size).abs() < 0.01 {
            // The axis title itself: `TextAnchor::Middle`, so x0 is already
            // its own left edge.
            title_rect = Some((x0, y0 - f64::from(title_size), x0 + title_width, y0));
            continue;
        }
        let m = run.transform.matrix;
        let rotated = m[0].abs() < 1e-3 && m[3].abs() < 1e-3;
        if rotated {
            let (_, label) = rotated_candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            let run_len = brightfield_render::text::measure_width(label, size);
            label_rects.push((
                x0 - f64::from(size) / 2.0,
                y0,
                x0 + f64::from(size) / 2.0,
                y0 + run_len,
            ));
        } else {
            let (_, label) = horiz_candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            let width = brightfield_render::text::measure_width(label, size);
            label_rects.push((x0, y0 - f64::from(size), x0 + width, y0));
        }
    }
    (label_rects, title_rect)
}

/// A `day`-axis rendered in isolation at `width`, over `categories` — the same
/// [`brightfield_render::axis::compute_ticks`] / `render_x_axis` path a
/// counts_over_time tile's own scene draws its x axis through, at
/// [`brightfield_render::layout::ChartLayout`]'s own inset-adjusted x range.
fn day_axis_scene(
    categories: &[String],
    width: f64,
) -> (vello::Scene, Vec<brightfield_render::axis::Tick>) {
    let layout = brightfield_render::layout::ChartLayout::new(width, 300.0);
    let (range_start, range_end) = layout.x_range();
    let scale = brightfield_render::scale::Scale::Band {
        categories: categories.to_vec(),
        range_start,
        range_end,
        padding: 0.1,
    };
    let ticks = brightfield_render::axis::compute_ticks(&scale, 5);
    let mut scene = vello::Scene::new();
    brightfield_render::axis::render_x_axis(
        &mut scene,
        &layout,
        &ticks,
        None,
        brightfield_render::ink::ChartInk::LIGHT,
    );
    (scene, ticks)
}

/// Matches each horizontal glyph run in `scene` back to whichever `ticks`
/// entry its draw position (`TextAnchor::Middle`, `render_x_axis`'s own
/// anchor) is nearest, then asserts no two runs' `[x, x + width]` intervals
/// intersect — the width read with `measure_width`, the shaping
/// `render_x_axis` measured it with, rather than estimated from the run's raw
/// glyph count. A run whose transform carries a quarter turn is skipped: a
/// rotated axis is a different claim, made in `brightfield-render`'s own
/// `axis` tests.
fn assert_no_tick_label_overlap(
    scene: &vello::Scene,
    ticks: &[brightfield_render::axis::Tick],
    size: f32,
) {
    let candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| {
            (
                t.position - brightfield_render::text::measure_width(&t.label, size) / 2.0,
                t.label.as_str(),
            )
        })
        .collect();
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for run in &scene.encoding().resources.glyph_runs {
        let m = run.transform.matrix;
        let rotated = m[0].abs() < 1e-3 && m[3].abs() < 1e-3;
        if rotated {
            continue;
        }
        let x0 = f64::from(run.transform.translation[0]);
        let (_, label) = candidates
            .iter()
            .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
            .expect("fixture check: at least one candidate tick");
        spans.push((
            x0,
            x0 + brightfield_render::text::measure_width(label, size),
        ));
    }
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for pair in spans.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "two drawn tick labels overlap: {pair:?} (all spans: {spans:?})"
        );
    }
}

/// The logical window [`site_readings`]'s own baseline captures
/// ([`the_site_readings_dashboard_light_baseline`] and its dark twin) open
/// at — [`Boot::window_size`], the natural size their `capture_png` call
/// derives with no explicit size of its own.
fn site_readings_baseline_window() -> (f32, f32) {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    boot.window_size()
}

/// [`site_readings`] driven through the REAL pane group at a window `size` —
/// the same `MeridianApp::headless` + `ctx.run_ui` settle loop [`pane_rects`]
/// drives, three frames so a resizable panel's reported size is read back
/// on the frame after (the same reason [`pane_rects`] settles before reading
/// anything back). Returns the settled app so its `chart_doc().composed` can
/// be read by reference: `day`'s tile at whatever width THIS window's
/// constrained `hconcat` resolved for it, not [`data_file::open`]'s own
/// one-shot, UNCONSTRAINED composition — which is what this card's round 2
/// read, and which is always `crate::dashboard::COLUMN_TILE_WIDTH` (380
/// points) regardless of window, because an unconstrained `hconcat` sizes
/// each item to its OWN declared weight rather than sharing a box out. The
/// live app never lays this dashboard out unconstrained —
/// `ChartDoc::reflow_to` always hands `LiveDashboard::set_viewport` a real
/// box first.
fn site_readings_app_at(size: (f32, f32)) -> brightfield_shell::window::MeridianApp {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0, size.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// **The counts_over_time tile's time axis never overlaps or clips at the
/// widths the live app actually resolves for it** — read off the REAL pane
/// group at four real window widths, not [`data_file::open`]'s one-shot
/// unconstrained composition ([`site_readings_app_at`]'s doc names why
/// round 2's own version of this test read the wrong quantity).
///
/// [`site_readings`] gives the coordinate pair its own hero, so `day` earns
/// an ordinary tile in the STACKED column ([`assert_site_readings`] pins
/// that shape) rather than falling back into the map pane the way
/// [`fixture`]'s `day` does. Four widths: the baseline window
/// [`site_readings`]'s own baseline captures open at
/// ([`site_readings_baseline_window`]), and 1400/1200/1000 — measured on
/// round 2's build, the column tile these four resolve to was 380, 309, 233
/// and 157 points wide, and at 157 the pre-fix axis rotated a real date past
/// the tile's own bottom edge. At every width: [`drawn_x_axis_rects`] reads
/// every tick-label rect straight out of the composed scene, and each must
/// lie inside `day`'s own tile rect, clear of every other tick-label rect,
/// and clear of the axis title's rect — whichever way `render_x_axis` chose
/// to draw them.
#[test]
fn the_time_axis_never_overlaps_or_clips_across_real_window_widths() {
    // Fixture check, once: the shape the whole sweep below assumes.
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_site_readings(&opened.dashboard);
    drop(opened);

    let baseline = site_readings_baseline_window();
    let widths = [baseline.0, 1400.0, 1200.0, 1000.0];

    for width in widths {
        let app = site_readings_app_at((width, baseline.1));
        let composed = &app.chart_doc().composed;
        let day_plot = plot_for_x_column(composed, "day");
        let tile_width = day_plot.rect.width;
        assert!(
            tile_width > 0.0,
            "fixture check: day's tile drew at a non-positive width at \
             window width {width}"
        );

        let x_scale = day_plot
            .scales
            .get(brightfield_render::channel::Channel::X)
            .expect("fixture check: day's placed plot carries an x scale");
        let ticks = brightfield_render::axis::compute_ticks(x_scale, 5);
        assert_eq!(
            ticks.len(),
            6,
            "fixture check: site_readings_sample.csv's day column no longer \
             carries six distinct dates"
        );
        let (label_rects, title_rect) = drawn_x_axis_rects(
            &composed.scene,
            day_plot,
            &ticks,
            "day",
            brightfield_render::text::LABEL_SIZE,
        );
        assert!(
            !label_rects.is_empty(),
            "no tick-label rect was drawn for day's x axis at window width \
             {width} (its tile drew at {tile_width} points wide)"
        );

        let tile: Rect4 = (
            day_plot.rect.x,
            day_plot.rect.y,
            day_plot.rect.x + day_plot.rect.width,
            day_plot.rect.y + day_plot.rect.height,
        );
        for rect in &label_rects {
            assert!(
                rect_inside(*rect, tile),
                "a tick-label rect {rect:?} does not lie inside day's own \
                 tile rect {tile:?} at window width {width} (tile \
                 {tile_width} points wide)"
            );
            if let Some(title_rect) = title_rect {
                assert!(
                    !rects_intersect(*rect, title_rect),
                    "a tick-label rect {rect:?} intersects the axis title's \
                     rect {title_rect:?} at window width {width}"
                );
            }
        }
        for i in 0..label_rects.len() {
            for other in &label_rects[i + 1..] {
                assert!(
                    !rects_intersect(label_rects[i], *other),
                    "two of day's drawn tick-label rects intersect at window \
                     width {width}: {:?} and {other:?}",
                    label_rects[i],
                );
            }
        }
    }
}

/// **The same axis, at 240 and at 720 points wide** — so the claim above is a
/// rule about the render path rather than a fact about today's one measured
/// column width.
#[test]
fn the_time_axis_does_not_collide_at_240_or_720_points() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let categories = fixture_day_categories(&opened.composed);

    for width in [240.0_f64, 720.0_f64] {
        let (scene, ticks) = day_axis_scene(&categories, width);
        assert_no_tick_label_overlap(&scene, &ticks, brightfield_render::text::LABEL_SIZE);
    }
}

/// **[`site_readings`]'s own generated dashboard, as pixels** — the pair
/// [`the_generated_dashboard_light_baseline`] / `_dark_baseline` draw for
/// [`housing`], drawn instead for the table whose `day` column is what this
/// card's regression is about. Unlike the four-shapes fixture's own baseline
/// pair (which this replaces — see `git log` for that pair's history), `day`
/// draws in the STACKED COLUMN here, at a column tile's real width, rather
/// than falling back into the map pane: a baseline that cannot redden on the
/// time-axis defect is not a pin of it. [`assert_site_readings`] runs ahead of
/// any capture, for the reason this file's header gives.
#[test]
fn the_site_readings_dashboard_light_baseline() {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");

    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_site_readings(&opened.dashboard);
    drop(opened);

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("site_readings_dashboard_light");
    let (w, h) = capture_png(boot, Mode::Light, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture site_readings_dashboard_light: {e}"));
    assert!(
        w > 0 && h > 0,
        "site_readings_dashboard_light: empty capture"
    );

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "site_readings_dashboard_light");
}

/// The most pixels of a dark [`site_readings`] capture allowed to land
/// exactly on the light chart surface's bytes before this test reads it as
/// the white-slab regression `the_generated_dashboard_dark_baseline` guards
/// against on [`housing`], rather than as antialiasing.
///
/// [`site_readings`]'s `reading` tile is a binned histogram, whose two
/// `rectY` layers (the unfiltered ghost behind its filtered subset — see
/// `histogram_tile`) meet at an antialiased edge inside the tile. Measured on
/// this build, zero device pixels blend to the light surface's precise bytes
/// even so — this fixture does not reproduce the coincidence the four-shapes
/// fixture once measured (one pixel inside the `region` tile's ranked bars,
/// beside its `"10"` value label, where THAT fixture's ranked-bars tile drew
/// its own highlight-over-total pair; this fixture has no ranked-bars tile).
/// The budget stays at what is actually measured here rather than carrying
/// that figure over, so it still catches the defect this check exists for: a
/// pane painted the light surface wholesale runs to thousands of pixels, nowhere
/// near this floor.
const DARK_CAPTURE_LIGHT_PIXEL_BUDGET: usize = 0;

/// **The dark twin of [`the_site_readings_dashboard_light_baseline`]** — the
/// same white-slab regression check `the_generated_dashboard_dark_baseline`
/// runs for [`housing`], run here for [`site_readings`] instead, with the
/// tolerance [`DARK_CAPTURE_LIGHT_PIXEL_BUDGET`] documents.
#[test]
fn the_site_readings_dashboard_dark_baseline() {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("site_readings_dashboard_dark");
    let (w, h) = capture_png(boot, Mode::Dark, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture site_readings_dashboard_dark: {e}"));
    assert!(
        w > 0 && h > 0,
        "site_readings_dashboard_dark: empty capture"
    );

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();

    let light = pixels_of(&image, meridian_design::chrome::INK_LIGHT.surface);
    assert_eq!(
        light, DARK_CAPTURE_LIGHT_PIXEL_BUDGET,
        "{light} pixels of this dark dashboard are the LIGHT chart surface \
         (#fcfcfb), past the {DARK_CAPTURE_LIGHT_PIXEL_BUDGET}-pixel budget \
         measured for this fixture — a pane is painting the light surface"
    );
    let dark = pixels_of(&image, meridian_design::chrome::INK_DARK.surface);
    assert!(
        dark > 0,
        "no pixel of this dark dashboard is the dark chart surface (#161413)"
    );

    egui_kittest::image_snapshot(&image, "site_readings_dashboard_dark");
}
