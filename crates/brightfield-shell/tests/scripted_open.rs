//! Opening a data file **without a dialog** — the route a script, a capture or
//! a demo reaches, and the guarantee that it is the same route the picker takes.
//!
//! Until this existed the generated dashboard had exactly one way in:
//! `data_file::pick()`, an operating-system modal behind the `dialogs_allowed`
//! gate that exists precisely because a headless tier raising one hangs until
//! something kills it. A capability in that state cannot be photographed, put
//! in a demo script, or regression-checked as a picture — it can only be
//! re-demonstrated by the same person doing the same thing again.
//!
//! Four things are held here, and they are separable:
//!
//! 1. a path handed to `Boot::open` — what `main` reaches through, and what
//!    `brightfield-shot --spec` reaches through — opens as the dashboard the
//!    generator chose for it;
//! 2. that boot photographs itself through the headless capture tier, with no
//!    window, no dialog and nobody present;
//! 3. the front door's promise about what an opened file becomes is true of
//!    what the generator actually drew, asserted over the generator rather than
//!    over the sentence;
//! 4. the two entry points leave the *same document* behind, down to the
//!    encoded scene — so they cannot drift into two pictures of one file.
//!
//! **No dialog is opened anywhere in this file**, for the reason `data_file.rs`
//! gives: a test that raised a modal would hijack the desktop of whoever ran
//! the suite. The picker's route is exercised through
//! `MeridianApp::open_data_file`, which is the seam on this side of the one
//! line that calls rfd.
//!
//! The fixtures are written to a temp directory at test time rather than
//! committed, because what is gated is DuckDB reading a real file off a real
//! filesystem.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::{capture_png, DEFAULT_SCALE};
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{names_a_data_file, Boot, MeridianApp, OPEN_FILE_PROMISE};

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-scripted-open-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("the fixture writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Four columns, and the fourth is there to be **left out**.
///
/// `station`, `reading` and `depth` each admit a picture; `survey` holds one
/// distinct value, which every chart of it draws as a single bar. So this
/// fixture exercises both halves of the door's promise at once — a chart for
/// every column the build can draw one for, and a column it cannot, declined
/// with a reason rather than dropped.
const HARBOUR_CSV: &str = "station,reading,depth,survey\n\
                           north,12,4.5,autumn\n\
                           north,18,6.0,autumn\n\
                           south,31,2.5,autumn\n\
                           south,44,9.5,autumn\n\
                           east,7,1.0,autumn\n\
                           east,25,7.5,autumn\n\
                           west,52,3.0,autumn\n\
                           west,63,8.0,autumn\n";

/// How many columns [`HARBOUR_CSV`] declares. Stated once, so the assertions
/// below cannot disagree about what "every column" means.
const HARBOUR_COLUMNS: usize = 4;

/// One categorical column — one tile, and therefore a document whose picture
/// **one** chart kind chose.
///
/// The parity test needs this shape and not only [`HARBOUR_CSV`]: a dashboard
/// of several tiles records no `Authored` kind, so over that fixture alone the
/// two routes agree on `None` and the field the assembly was unified for is not
/// compared at all.
const ONE_CATEGORY_CSV: &str = "name\nada\ngrace\nbarbara\nkaren\nada\n";

/// A window under test, with one `egui::Context` for its whole life and one
/// screen rect — the arrangement `data_file.rs` and `front_door.rs` use, and
/// the arrangement both routes below are driven through so that nothing about
/// the *frame* can account for a difference between them.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn over(boot: Boot) -> Self {
        Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    fn settle(&mut self) {
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }
}

/// Everything about a settled window that either route is responsible for
/// producing, in one comparable value.
///
/// A struct rather than a pile of `assert_eq!`s so the two routes are described
/// by one reading of the document and a difference names itself. The scene's
/// encoding is in it because that is *the picture*: vello's `Encoding` holds
/// the path tags, the path data and the draw data the GPU is handed, so equal
/// encodings is a byte-level statement about what would be drawn, with no
/// device involved.
#[derive(PartialEq, Eq, Debug)]
struct Document {
    title: String,
    graph_on_canvas: bool,
    size: (u32, u32),
    plots: usize,
    /// Each plot's placed rect, rounded to whole logical points — the geometry
    /// of the picture, not just its extent.
    plot_rects: Vec<(i64, i64, i64, i64)>,
    n_paths: u32,
    path_tags: usize,
    path_data: Vec<u32>,
    draw_data: Vec<u32>,
    /// How the picture was chosen, when a single chart kind chose it — the
    /// record `MeridianApp::open_data_file` used to derive at its own call site
    /// and the boot route would have had no way to reproduce.
    authored: Option<String>,
    /// The **bytes** of the generated spec, read back from wherever the route
    /// wrote it. Not the path: the two routes write into per-process scratch
    /// files, so equal paths would be an accident and equal contents is the
    /// claim — the spec is what composed the picture.
    spec: Option<String>,
}

impl Document {
    fn of(win: &Window) -> Self {
        let doc = win.app.chart_doc();
        let encoding = doc.composed.scene.encoding();
        Self {
            title: win.app.title(),
            graph_on_canvas: win.app.graph_on_canvas(),
            size: (doc.composed.width, doc.composed.height),
            plots: doc.composed.plots.len(),
            plot_rects: doc
                .composed
                .plots
                .iter()
                .map(|p| {
                    (
                        p.rect.x.round() as i64,
                        p.rect.y.round() as i64,
                        p.rect.width.round() as i64,
                        p.rect.height.round() as i64,
                    )
                })
                .collect(),
            n_paths: encoding.n_paths,
            path_tags: encoding.path_tags.len(),
            path_data: encoding.path_data.clone(),
            draw_data: encoding.draw_data.clone(),
            authored: doc.authored().map(|a| format!("{a:?}")),
            spec: doc
                .spec_path
                .as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok()),
        }
    }
}

// ---------------------------------------------------------------------------
// 1. A path on the command line reaches the generator
// ---------------------------------------------------------------------------

/// `brightfield-shell harbour.csv` opens the generated dashboard.
///
/// Driven through [`Boot::open`] rather than through [`Boot::data_file`],
/// because `Boot::open` is what `main` reaches (via `startup::opening_boot`)
/// and what `brightfield-shot --spec` reaches. Calling the data-file
/// constructor directly would assert that the constructor works while leaving
/// the classification — the thing that was missing — untested: a `.csv`
/// reached the spec parser and came back *unknown spec format*.
#[test]
fn a_path_on_the_command_line_opens_as_the_generated_dashboard() {
    let dir = TempDir::new("command-line");
    let csv = dir.write("harbour.csv", HARBOUR_CSV);
    let named = csv.to_string_lossy().into_owned();

    let boot = Boot::open(&named, Flow::Vertical, None)
        .unwrap_or_else(|e| panic!("a data file named on the command line has to open: {e}"));

    assert!(
        !boot.graph_on_canvas(),
        "a file opened from the command line puts its chart on the canvas, \
         exactly as a chart spec does"
    );
    assert!(
        !boot.is_empty(),
        "the boot carries no document — the window would open on the front \
         door with the file silently unread"
    );
    assert!(
        boot.live.is_some(),
        "the session behind the file is what arms the Data pane and every \
         brush; a still boot is a screenshot of a table, not a table"
    );
    assert!(
        boot.spec_path.is_some(),
        "the generated spec is what makes the dashboard inspectable rather \
         than opaque, and the editor pane opens it off this field"
    );
    assert_eq!(
        boot.title(),
        "harbour.csv",
        "the window is titled for the file the person named"
    );
    assert!(
        boot.composed.plots.len() > 1,
        "the command-line route drew {} plot(s) — it is reaching something \
         other than the generator, whose walk is a tile per column",
        boot.composed.plots.len()
    );
}

/// A spec still reaches the spec parser, and an extension nobody reads still
/// gets the spec parser's refusal.
///
/// The classification widened; it did not move. The refusal naming `.yaml`,
/// `.yml` and `.json` is the right answer for a genuinely unrecognised
/// extension and is kept for one.
#[test]
fn a_spec_is_still_a_spec_and_an_unknown_extension_is_still_refused() {
    let dir = TempDir::new("still-a-spec");
    let spec = dir.write(
        "one.yaml",
        "data:\n  t:\n    - { a: 1 }\n    - { a: 2 }\nplot:\n  - mark: dot\n    data: { from: t }\n",
    );
    let named = spec.to_string_lossy().into_owned();
    assert!(
        !names_a_data_file(&named),
        "a .yaml is a document, not data"
    );
    Boot::open(&named, Flow::Vertical, None).expect("a chart spec still opens as a chart spec");

    let notes = dir.write("notes.txt", "not a spec and not a table\n");
    let named = notes.to_string_lossy().into_owned();
    assert!(!names_a_data_file(&named));
    let refusal = Boot::open(&named, Flow::Vertical, None)
        .err()
        .expect("a .txt is neither a document this build reads nor data it opens");
    assert!(
        refusal.contains("spec format") || refusal.contains("yaml"),
        "an unrecognised extension has to keep the spec parser's own words, \
         which name what this build does read: {refusal}"
    );
}

/// The classifier and the opener agree about what counts as data.
///
/// Two lists that could drift into disagreement — one deciding which loader
/// gets the path, one deciding whether that loader will take it — so they are
/// held to the single `OPENABLE_EXTENSIONS` both are written from. The
/// direction that matters is a path the classifier sends to the data loader
/// which the loader then refuses *by extension*: that is the round trip that
/// produces a refusal about a file type the user was just offered.
#[test]
fn the_classifier_and_the_opener_agree_on_what_is_data() {
    for ext in data_file::OPENABLE_EXTENSIONS {
        for spelled in [ext.to_string(), ext.to_ascii_uppercase()] {
            let path = format!("/tmp/bf-no-such-table.{spelled}");
            assert!(
                names_a_data_file(&path),
                "{path} is on OPENABLE_EXTENSIONS and has to reach the data loader"
            );
            let refusal = data_file::accept(&path).expect_err("nothing is at that path");
            assert!(
                refusal.contains("there is no file there to open"),
                "{path} has to get past the opener's extension check and be \
                 refused for the reason it is actually refused for: {refusal}"
            );
        }
    }

    for path in [
        "/tmp/a.yaml",
        "/tmp/a.yml",
        "/tmp/a.json",
        "/tmp/a",
        "/tmp/a.txt",
    ] {
        assert!(
            !names_a_data_file(path),
            "{path} is not data and must not be taken from the spec parser"
        );
    }
}

/// A sample rate over a data file is refused rather than ignored.
///
/// `--force-sample` pushes a rate into a plot's own query. A generated
/// dashboard has no authored plot to push it into, so honouring the flag is
/// impossible — and ignoring it would draw the complete table under a command
/// line that asked for a sample of it, which is the silent form of the same
/// failure.
#[test]
fn a_sample_rate_over_a_data_file_is_refused_by_name() {
    let dir = TempDir::new("sampled");
    let csv = dir.write("harbour.csv", HARBOUR_CSV);
    let named = csv.to_string_lossy().into_owned();
    let rate = brightfield_sql::ir::SampleRate::from_modulus(8).expect("8 is a power of two");

    let refusal = Boot::open_sampled(&named, Flow::Vertical, None, Some(rate))
        .err()
        .expect("a rate that cannot be honoured must not be dropped");
    assert!(
        refusal.contains(&named),
        "the refusal has to name the file: {refusal}"
    );
    assert!(
        refusal.contains("sample rate"),
        "the refusal has to say what it refused: {refusal}"
    );
    // …and the same file without the flag opens.
    Boot::open(&named, Flow::Vertical, None).expect("the file itself is fine");
}

// ---------------------------------------------------------------------------
// 2. …and it photographs itself with nobody present
// ---------------------------------------------------------------------------

/// The generated dashboard is written to a PNG by a process with no window, no
/// dialog and no person in front of it — **at the scale the shipped command
/// uses**, and at 1.0 beside it.
///
/// This is what the route is *for*. The boot handed to the capture tier is the
/// same value `main` hands `eframe::run_native` and the same value
/// `brightfield-shot --spec` hands `capture_png`; every other argument of that
/// command is at its default here too — [`Mode::Light`] is `--theme`'s,
/// [`Flow::Vertical`] is `--flow`'s, an empty script is `--script`'s absence,
/// and [`DEFAULT_SCALE`] is `--scale`'s. So the image this test reads at
/// `DEFAULT_SCALE` is the image `brightfield-shot --spec harbour.csv --out
/// shot.png` writes.
///
/// **Both scales, because the scale is where this hid.** The capture used to
/// take its device ratio through `Context::set_pixels_per_point`, which is a
/// zoom control: applying it discarded the caller's `screen_rect` for one
/// frame, the dashboard reflowed into the substituted 5000×5000 box, and the
/// following frame rastered that reflowed size onto a texture large enough for
/// vello to return a blank frame — a PNG with an empty chart pane, at exit 0,
/// at every scale except exactly 1.0, where the setter was a no-op. A guard
/// pinned at 1.0 was a certificate issued at the one setting the defect spared.
/// 1.0 stays in the loop because it is the value the committed baselines in
/// `front_door.rs` and `surfaces.rs` are captured at.
///
/// **Not a baseline.** What is asserted is that a picture arrived and that it
/// is a picture of something — the committed golden that reddens when the
/// generator's tile choices change is its own piece of work, and pinning one
/// here would redden on every deliberate change to the chooser while this test
/// is about reachability.
///
/// **And the assertion is about the raster, not about the window.** A first
/// version of this counted distinct colours over the whole image and passed on
/// a capture whose chart pane was entirely blank: a window's chrome — a top
/// bar, a tab strip, two pane borders and a checkbox — carries dozens of
/// colours on its own, so "the PNG is not uniform" is satisfied by a window
/// that drew no chart at all. Measured, on a real blank capture. The image is
/// cropped to the box the chart pane actually gave the raster, read off a
/// GPU-free layout pass at the same window size, and the ink is looked for
/// there.
#[test]
fn the_generated_dashboard_writes_itself_to_a_png_with_nobody_present() {
    for scale in [1.0, DEFAULT_SCALE] {
        photographs_itself_at(scale);
    }
}

/// One scale's worth of
/// [`the_generated_dashboard_writes_itself_to_a_png_with_nobody_present`].
///
/// Every threshold below is scale-free by construction: the window and the
/// raster box are asserted in device pixels derived from `scale`, and the ink
/// floor is a count that can only grow with it.
fn photographs_itself_at(scale: f32) {
    let dir = TempDir::new("capture");
    let csv = dir.write("harbour.csv", HARBOUR_CSV);
    let named = csv.to_string_lossy().into_owned();

    let boot = Boot::open(&named, Flow::Vertical, None).expect("the file opens");
    let (want_w, want_h) = boot.window_size();

    // Where the raster lands, from a device-free frame at the same window size.
    // `MeridianApp::headless` lays out identically to the one `capture_png`
    // builds — every rect around the canvas is a pure function of the loaded
    // document — so this is the box the capture drew the chart into. Logical
    // points, as the layout is: the crop below takes it into device pixels.
    let mut laid_out = Window::over(Boot::open(&named, Flow::Vertical, None).expect("it opens"));
    laid_out.screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(want_w, want_h));
    laid_out.settle();
    let raster = laid_out
        .app
        .chart_doc()
        .raster_rect
        .expect("a settled frame presented the raster");

    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("scripted-open-{scale}.png"));
    let _ = std::fs::remove_file(&out);
    let (w, h) = capture_png(boot, Mode::Light, scale, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture the generated dashboard at {scale}x: {e}"));

    assert!(out.is_file(), "no PNG at {}", out.display());
    assert_eq!(
        (w, h),
        (
            (want_w * scale).round() as u32,
            (want_h * scale).round() as u32
        ),
        "the {scale}x capture is not the window the dashboard asked for"
    );

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read back {}: {e}", out.display()))
        .to_rgba8();
    let (x0, y0) = ((raster.min.x * scale) as u32, (raster.min.y * scale) as u32);
    let (x1, y1) = ((raster.max.x * scale) as u32, (raster.max.y * scale) as u32);
    assert!(
        x1 > x0 && y1 > y0 && x1 <= w && y1 <= h,
        "the raster box {raster:?} is not inside the {w}x{h} capture at {scale}x"
    );

    let mut seen: Vec<[u8; 4]> = Vec::new();
    let mut coloured = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = image.get_pixel(x, y).0;
            if !seen.contains(&px) {
                seen.push(px);
            }
            // The chrome is greys and near-whites; a mark is drawn in the
            // design system's series ink. A channel spread this wide is
            // therefore ink rather than furniture, and a blank pane has none.
            let (lo, hi) = (px[0].min(px[1]).min(px[2]), px[0].max(px[1]).max(px[2]));
            if hi - lo > 60 {
                coloured += 1;
            }
        }
    }
    assert!(
        seen.len() >= 32,
        "the {scale}x chart raster holds {} distinct colour(s) — the window came \
         up with a blank chart pane, which writes a PNG and exits 0 exactly as a \
         drawn one does",
        seen.len()
    );
    assert!(
        coloured > 1000,
        "the {scale}x chart raster holds {coloured} pixel(s) of saturated ink over \
         {}x{} — the marks the generator chose are not in the picture",
        x1 - x0,
        y1 - y0
    );
}

// ---------------------------------------------------------------------------
// 3. The door promises what the generator draws
// ---------------------------------------------------------------------------

/// The front door's Start blurb is true of what the generator actually did.
///
/// The sentence said *a table you can read and a first look drawn from it* —
/// singular, and correct for the single chart this route drew before the
/// generator shipped. It is the one screen that describes the feature to a
/// stranger, so understating it there understates the product.
///
/// **The generator runs first and the sentence is held to it.** A string
/// assertion on its own would pass unchanged on a build whose generator had
/// stopped drawing a tile per column — which is the failure worth catching,
/// since the sentence would then be the only thing still claiming it did.
#[test]
fn the_door_promises_what_the_generator_draws() {
    let dir = TempDir::new("promise");
    let csv = dir.write("harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&csv.to_string_lossy()).expect("the fixture opens");
    let tiles = opened.dashboard.tiles();
    let omitted = opened.dashboard.omitted();

    // Every column is accounted for: drawn, or declined with a reason.
    assert_eq!(
        tiles.len() + omitted.len(),
        HARBOUR_COLUMNS,
        "the generator neither drew nor declined every column, so no sentence \
         about 'every column' can be checked against it"
    );
    assert!(
        tiles.len() > 1,
        "the generator drew {} tile(s) over {HARBOUR_COLUMNS} columns — with \
         one, the door's older singular wording would have been the accurate \
         one and this sentence would be the overstatement",
        tiles.len()
    );
    assert_eq!(
        tiles.len(),
        HARBOUR_COLUMNS - omitted.len(),
        "a column that was not declined got no chart, so 'a chart for every \
         column it can draw one for' is not what happened"
    );
    assert_eq!(
        omitted.len(),
        1,
        "the fixture carries exactly one column no chart in this build fits, \
         so that the promise's narrowing clause is exercised rather than \
         vacuous: {omitted:?}"
    );

    // …and only now, the words.
    assert!(
        OPEN_FILE_PROMISE.contains("every column it can draw one for"),
        "the door no longer promises the per-column dashboard the generator \
         just drew: {OPEN_FILE_PROMISE}"
    );
    assert!(
        !OPEN_FILE_PROMISE.contains("a first look"),
        "the pre-generator wording is back on the front door: {OPEN_FILE_PROMISE}"
    );
    for word in ["CSV", "Parquet", "nothing is uploaded"] {
        assert!(
            OPEN_FILE_PROMISE.contains(word),
            "the promise dropped `{word}`: {OPEN_FILE_PROMISE}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. The two entry points cannot drift
// ---------------------------------------------------------------------------

/// One file, two ways in, one document.
///
/// The scripted route builds the window over `Boot::open`'s boot. The picker's
/// route builds an empty window and calls `MeridianApp::open_data_file`, which
/// is the seam on this side of the single line that opens a modal — the
/// function the door's control reaches through, with the dialog's only
/// contribution (a path) supplied directly.
///
/// Both are settled through identical frames at an identical screen rect, so
/// nothing about the *frame* can account for a difference. What is compared is
/// everything either route is responsible for producing, including the encoded
/// scene: equal `path_data` and `draw_data` is a byte-level statement that the
/// same picture would be drawn, and it needs no GPU to make.
///
/// The drift this refuses is not hypothetical. Before this, the picker's route
/// derived the `Authored` record at its own call site, so any second route into
/// an opened file started out unable to reproduce it — the chart pane would
/// have hosted one of them through the chart kind's module and presented the
/// other directly, over the same file.
///
/// **Two fixtures, and the second is the one that earns the claim.** With the
/// four-column table alone this test passed with the boot route's `Authored`
/// wiring deleted — measured, by deleting it — because a dashboard of several
/// tiles records no kind, so both routes answered `None` and the comparison was
/// vacuous for exactly the field the refactor was about. A one-column table
/// records a kind, and that is the case this holds.
#[test]
fn the_two_routes_into_a_data_file_produce_one_document() {
    // Several tiles: no single kind built the picture, so `authored` is `None`
    // on both sides and what is held is the dashboard, its placement and its
    // scene.
    let many = agree_on("harbour.csv", HARBOUR_CSV);
    assert!(
        many.plots > 1 && many.n_paths > 0,
        "both routes produced an empty document, so the parity says nothing: {many:?}"
    );
    assert_eq!(
        many.authored, None,
        "a dashboard of several tiles was recorded as one chart kind's picture"
    );

    // One tile: that tile's picture IS the document's picture, so a kind is
    // recorded — and this is the arm that reddens when a route stops recording
    // it. Without it the `authored` comparison above is `None == None`.
    let one = agree_on("names.csv", ONE_CATEGORY_CSV);
    assert_eq!(
        one.plots, 1,
        "the one-tile fixture stopped being one tile, so the authored \
         comparison it exists for is vacuous again: {one:?}"
    );
    assert!(
        one.authored.is_some(),
        "a one-tile dashboard recorded no chart kind through EITHER route, so \
         the equality between them holds for the wrong reason: {one:?}"
    );
}

/// Open `csv` both ways, hold every field of the two documents equal, and hand
/// back the one they agreed on for the caller to make claims about.
///
/// The scripted route is [`Boot::open`] into a fresh window. The picker's route
/// is an empty window plus [`MeridianApp::open_data_file`] — the seam on this
/// side of the single line that calls rfd, with the dialog's only contribution
/// (a path) supplied directly. Both settle through identical frames at an
/// identical screen rect, so nothing about the *frame* can account for a
/// difference between them.
fn agree_on(name: &str, csv: &str) -> Document {
    let dir = TempDir::new("parity");
    let path = dir.write(name, csv);
    let named = path.to_string_lossy().into_owned();

    let mut scripted = Window::over(
        Boot::open(&named, Flow::Vertical, None).unwrap_or_else(|e| panic!("{name}: {e}")),
    );
    scripted.settle();

    let mut picked = Window::over(Boot::empty());
    picked.settle();
    picked.app.open_data_file(&picked.ctx.clone(), &named);
    picked.settle();

    let a = Document::of(&scripted);
    let b = Document::of(&picked);

    assert!(
        !a.graph_on_canvas,
        "{name}: the scripted route left the graph on the canvas"
    );
    assert_eq!(
        a.title, b.title,
        "{name}: the two routes titled the window differently"
    );
    assert_eq!(
        a.size, b.size,
        "{name}: the two routes composed different dashboards"
    );
    assert_eq!(
        a.plots, b.plots,
        "{name}: the two routes drew a different number of plots"
    );
    assert_eq!(
        a.plot_rects, b.plot_rects,
        "{name}: the two routes placed the plots differently"
    );
    assert_eq!(
        a.authored, b.authored,
        "{name}: the two routes recorded the picture's authorship differently"
    );
    assert_eq!(
        a.spec, b.spec,
        "{name}: the two routes composed from different spec bytes"
    );
    assert_eq!(
        (a.n_paths, a.path_tags),
        (b.n_paths, b.path_tags),
        "{name}: the two routes encoded a different number of paths"
    );
    assert!(
        a.path_data == b.path_data && a.draw_data == b.draw_data,
        "{name}: the two entry points encoded different scenes for one file — \
         the picture a script produces is not the picture the picker produces"
    );
    // The whole comparison in one line, so a field added to `Document` is
    // covered without a matching assertion having to be remembered.
    assert_eq!(
        a, b,
        "{name}: the two routes left different documents behind"
    );
    a
}
