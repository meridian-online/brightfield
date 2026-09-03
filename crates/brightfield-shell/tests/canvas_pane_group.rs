//! **The canvas as a pane group**: the map pane, the column of tiles beside
//! it, and the count that reads at the map's lower-right.
//!
//! Every claim here is read off a **laid-out frame**, not off a declaration.
//! Whether a pane drew a header band, where its content rect fell, where the
//! hero landed and how far a wheel moved the column are facts about pixels a
//! frame put on a screen; a test that read them out of `arrangement.rs` would
//! be comparing the declaration with itself. `MeridianApp::canvas_panes` is
//! what the frame leaves behind for that, the way `region_rect` is for the
//! regions, and `composed_plot_rects` is where each plot was drawn.
//!
//! No GPU here on purpose. What `MeridianApp::headless` differs from the device
//! path in is the raster: the canvas pane reserves the same box and paints
//! nothing into it, so the layout, the pane split and the gesture routing are
//! the ones the window runs, and the *layout* half of this card is gated
//! without a wgpu adapter. The picture is `tests/dashboard_baseline.rs`'s.
//!
//! # The three panes
//!
//! The map, the rows beneath it, and the column of tiles beside both. The
//! assertion below states the count as a number rather than as an inequality,
//! so a fourth pane arriving is a change this reports instead of one it
//! absorbs.
//!
//! # Which of them the ledger rail is open for
//!
//! A data file opens as a Protocol of one step, so this window's ledger rail
//! opens **closed to its strip** and the canvas has the rail's other 124
//! points. That is the window the picture is photographed in and the window
//! every claim about the group's own geometry is read in — [`settled`].
//!
//! It is also a window in which, at [`SCREEN`], the column has no reach: the
//! canvas is tall enough for seven tiles at their floor, so the page does not
//! outgrow the pane and the wheel has nothing to move. The scroll and
//! page-bound claims are therefore read in the window with the rail reopened
//! — [`settled_scrollable`], one click on the strip's own control — because
//! they are claims about a page bigger than its pane and that is where this
//! fixture makes one.

use brightfield_shell::dashboard::{HERO_SHARE, MAP_COLUMN_SHARE, MIN_COLUMN_TILE_HEIGHT};
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp, CANVAS_PANE_GAP};
use brightfield_workbench::arrangement;
use brightfield_workbench::chrome;

/// A table shaped like California Housing: nine numeric columns, two of them a
/// coordinate pair the generator finds by name.
///
/// A committed **sample**, not the dataset — the real Parquet is 16,640 rows
/// and belongs in `open-analytics`, not in this repo's test data. What it
/// shares with the real file is what the layout depends on: the nine column
/// names, their order, a coordinate pair among them, and seven other columns
/// that each earn a tile.
fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// How many tiles stand in the column beside the hero for [`fixture`].
///
/// Nine columns, two of which the coordinate pair draws as one tile, so eight
/// tiles: the map and seven others.
const STACKED: usize = 7;

/// The window a settled frame is laid out in.
///
/// 1440 by 900 — the size the composition this card is cut from was drawn at,
/// and the second of the two windows AC2 names.
const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 900.0),
};

/// Whether a window's ledger rail is left as it opened or reopened first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ledger {
    /// As the window opens over a one-step Protocol: closed to its strip.
    Closed,
    /// Reopened, by a click on that strip's own collapse control.
    Reopened,
}

/// A settled window over the fixture, laid out in `screen`, **as it opens**.
fn settled(screen: egui::Rect) -> MeridianApp {
    settled_after(screen, Ledger::Closed, None, 0)
}

/// [`settled`] with the ledger rail reopened before anything is read off it.
///
/// The column has reach where the composed page outgrows the pane it is drawn
/// in, and at [`SCREEN`] the rail's own height is what decides whether it
/// does: closed to its strip the rail hands the canvas 124 points back, the
/// seven tiles clear their floor inside the pane and the page stops being
/// taller than the box. Reopening it is one click on the control the collapsed
/// strip keeps, and it is what makes the assertions below claims about a
/// scroll rather than about a window size.
fn settled_scrollable(screen: egui::Rect) -> MeridianApp {
    settled_after(screen, Ledger::Reopened, None, 0)
}

/// [`settled`] with a **wheel** turned over `at` first: the pointer is put
/// there, `notches` frames each carry one wheel event, and the window is then
/// settled with the pointer left where it was.
///
/// The wheel is driven over frames rather than as one event because egui
/// smooths a wheel: a single event is delivered as an exponential tail over
/// the frames after it, so a test that read the offset one frame later would
/// be asserting against the smoothing constant rather than against the
/// window. What is asserted downstream is the offset the frame actually
/// applied — [`MeridianApp::canvas_scroll`] — and never a number typed here.
fn settled_after(
    screen: egui::Rect,
    ledger: Ledger,
    at: Option<egui::Pos2>,
    notches: usize,
) -> MeridianApp {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    // Three frames, as `tests/region_gate.rs` runs: egui stores a resizable
    // panel's reported size and reads it back on the frame after.
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    if ledger == Ledger::Reopened {
        reopen_the_ledger(&mut app, &ctx, &raw);
    }
    let Some(at) = at else {
        return app;
    };
    let mut moved = raw.clone();
    moved.events = vec![egui::Event::PointerMoved(at)];
    let _ = ctx.run_ui(moved, |ui| app.draw(ui));
    for _ in 0..notches {
        let mut turned = raw.clone();
        turned.events = vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -WHEEL_NOTCH),
            modifiers: egui::Modifiers::default(),
            phase: egui::TouchPhase::Move,
        }];
        let _ = ctx.run_ui(turned, |ui| app.draw(ui));
    }
    // The pointer stays put — egui holds a hover position until it is told
    // otherwise — so these settle the smoothing tail with the wheel still over
    // the same pane.
    for _ in 0..6 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// Click the collapsed ledger rail's control, where the last frame drew it,
/// and settle.
///
/// The gesture a reader has, aimed at a rect the frame reported: a click at a
/// typed coordinate that missed would leave the rail collapsed and every
/// scroll assertion downstream would pass for want of a scroll rather than
/// because of one, so the caller checks the rail moved.
fn reopen_the_ledger(app: &mut MeridianApp, ctx: &egui::Context, raw: &egui::RawInput) {
    let before = app
        .region_rect(arrangement::LEDGER_RAIL)
        .expect("the ledger rail drew")
        .height();
    let at = app
        .rail_collapse_rect(arrangement::LEDGER_RAIL)
        .expect("the collapsed ledger drew the control that reopens it")
        .center();
    let mut frame = |events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    frame(vec![egui::Event::PointerMoved(at)]);
    frame(vec![
        button(at, egui::PointerButton::Primary, true),
        button(at, egui::PointerButton::Primary, false),
    ]);
    for _ in 0..3 {
        frame(Vec::new());
    }
    let after = app
        .region_rect(arrangement::LEDGER_RAIL)
        .expect("the ledger rail drew")
        .height();
    assert!(
        after > before,
        "the click at {at:?} left the ledger rail {after} points tall where it \
         was {before} — it did not reopen, so the canvas kept the room and \
         nothing below is asserted about a page taller than its pane"
    );
}

/// One turn of the wheel, in logical points of travel.
///
/// egui's own note: a single notch on a Logitech wheel into a MacBook arrives
/// as 14 raw points. This is four of them, so a test scrolls a visible
/// distance in a few frames rather than a hairline.
const WHEEL_NOTCH: f32 = 56.0;

/// The window the dashboard baseline is photographed in — derived from the
/// composition, exactly as `capture_png` derives it.
fn baseline_screen() -> egui::Rect {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let (w, h) = boot.window_size();
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
}

/// **AC1 — each pane of the group draws its own header band.**
///
/// Counted off the frame: a pane that stopped drawing one, or a pane that
/// vanished from the group, both come out here as a different count. The
/// number is stated rather than bounded, so a fourth pane arriving is a
/// change this test reports instead of one it absorbs.
#[test]
fn every_pane_of_the_canvas_group_draws_its_own_header_band() {
    let app = settled(SCREEN);
    let group = app.canvas_panes();
    let names: Vec<&str> = group.panes.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        vec!["map", "rows", "columns"],
        "the canvas draws the map pane, the rows beneath it and the column of \
         tiles beside both, in that order"
    );

    let band = chrome::header_band_height();
    for pane in &group.panes {
        assert!(
            (pane.header.height() - band).abs() < 0.5,
            "the {} pane's header band drew {} points high where `pane_frame` \
             gives one {band}",
            pane.name,
            pane.header.height()
        );
        assert!(
            (pane.header.top() - pane.rect.top()).abs() < 0.5,
            "the {} pane's header band is not at the pane's own head: band {:?}, \
             pane {:?}",
            pane.name,
            pane.header,
            pane.rect
        );
        assert!(
            pane.body.top() >= pane.header.bottom() - 0.5,
            "the {} pane's content rect starts inside its header band, so the \
             band is drawn over the picture rather than above it",
            pane.name
        );
    }
}

/// **AC2 — the map pane takes the larger share of the canvas.**
///
/// `HERO_SHARE` of the canvas's width, within one pane gap, at both windows
/// the criterion names. The height clause is
/// `the_rows_pane_sits_under_the_map_and_takes_the_rest_of_its_column`'s.
#[test]
fn the_map_pane_takes_the_larger_share_of_the_canvas_width() {
    for screen in [baseline_screen(), SCREEN] {
        let app = settled(screen);
        let canvas = app
            .region_rect(arrangement::CANVAS)
            .expect("the canvas drew");
        let group = app.canvas_panes();
        let map = group.pane("map").expect("the map pane drew");
        let columns = group.pane("columns").expect("the column pane drew");

        let want = HERO_SHARE * canvas.width();
        assert!(
            (map.rect.width() - want).abs() <= CANVAS_PANE_GAP,
            "at {screen:?} the map pane drew {} points wide where {HERO_SHARE} \
             of the {} the canvas offers is {want} — more than the {CANVAS_PANE_GAP} \
             point gap between the panes",
            map.rect.width(),
            canvas.width()
        );
        assert!(
            map.rect.width() > columns.rect.width(),
            "the map lost the larger share: map {:?}, columns {:?}",
            map.rect,
            columns.rect
        );
        assert!(
            (columns.rect.bottom() - canvas.bottom()).abs() < 1.0,
            "the column pane stops at {} where the canvas ends at {} — the \
             tiles keep the canvas's full height, and it is the map's column \
             that is split",
            columns.rect.bottom(),
            canvas.bottom()
        );
        assert!(
            columns.rect.left() >= map.rect.right(),
            "the column pane is not beside the map: map {:?}, columns {:?}",
            map.rect,
            columns.rect
        );
    }
}

/// **AC1 — the rows pane sits under the map and takes the rest of its
/// column**, at [`MAP_COLUMN_SHARE`] of that column's height, and no wider
/// than the map.
///
/// Read off the drawn rects at both windows. Four ways to fail it, and each
/// is a separate assertion so a failure says which: the pane is absent, it is
/// above the map instead of beneath it, it is wider than the map's column, or
/// the map is no longer its declared share of the column the two of them make
/// between them.
///
/// The column is measured as the two panes' own span — the map's top to the
/// rows' bottom — rather than as the canvas's height, because *"its column"*
/// is what these two panes occupy and reading the canvas instead would fold
/// the head band's height into the claim.
#[test]
fn the_rows_pane_sits_under_the_map_and_takes_the_rest_of_its_column() {
    for screen in [baseline_screen(), SCREEN] {
        let app = settled(screen);
        let canvas = app
            .region_rect(arrangement::CANVAS)
            .expect("the canvas drew");
        let group = app.canvas_panes();
        let map = group.pane("map").expect("the map pane drew");
        let rows = group.pane("rows").expect("the rows pane drew");
        let columns = group.pane("columns").expect("the column pane drew");

        assert!(
            rows.rect.top() >= map.rect.bottom(),
            "at {screen:?} the rows pane at {:?} is not under the map at {:?}",
            rows.rect,
            map.rect
        );
        assert!(
            rows.rect.width() <= map.rect.width() + 0.5,
            "at {screen:?} the rows pane drew {} points wide where the map's \
             column is {} — it reached past the column into the tiles at {:?}",
            rows.rect.width(),
            map.rect.width(),
            columns.rect
        );
        assert!(
            rows.rect.right() <= columns.rect.left(),
            "at {screen:?} the rows pane at {:?} overlaps the column pane at \
             {:?}",
            rows.rect,
            columns.rect
        );

        // **Both halves of the share are read off the frame.** A comparison
        // against `MAP_COLUMN_SHARE` alone cannot fail: the constant decides
        // the rect and then the same constant is asked whether the rect is
        // right, so the two move together and the assertion is the
        // declaration against itself. Watched pass with the constant at 0.5.
        //
        // What is asserted instead is the design the constant is a spelling
        // of: the map is the same fraction of the canvas ACROSS as it is of
        // its column DOWN, and that fraction is 0.62. The first line is
        // immune to either constant moving, because it compares two drawn
        // rects; the second is the number itself, written out here so that
        // moving both constants together still reddens.
        let column = rows.rect.bottom() - map.rect.top();
        let down = map.rect.height() / column;
        let across = map.rect.width() / canvas.width();
        assert!(
            (down - across).abs() <= CANVAS_PANE_GAP / column,
            "at {screen:?} the map pane took {down} of its column's {column} \
             points down and {across} of the canvas's {} across — the two \
             shares are one design and the panes have stopped drawing it",
            canvas.width()
        );
        const SHARE: f32 = 0.62;
        assert!(
            (down - SHARE).abs() <= CANVAS_PANE_GAP / column,
            "at {screen:?} the map pane drew {} points tall, which is {down} \
             of the {column} points its column spans where the composition \
             this is cut from says {SHARE}",
            map.rect.height()
        );
        assert!(
            (MAP_COLUMN_SHARE - SHARE).abs() < f32::EPSILON,
            "`MAP_COLUMN_SHARE` is {MAP_COLUMN_SHARE} where the composition \
             says {SHARE} — the drawn share above is measured, and this is the \
             declaration it is supposed to be a spelling of"
        );
        assert!(
            (rows.rect.bottom() - columns.rect.bottom()).abs() < 1.0,
            "at {screen:?} the rows pane stops at {} and the column of tiles \
             beside it at {} — the two columns of the group do not end level",
            rows.rect.bottom(),
            columns.rect.bottom()
        );
    }
}

/// **AC3 — the column holds one tile per tiled column, in file order, at one
/// height, inside the pane.**
///
/// Read off the composition's own placed plots rather than off the tile list:
/// a tile that is chosen and never placed, or placed outside the pane it
/// belongs to, is exactly the failure this is here for, and the tile list
/// cannot tell the two apart.
///
/// At the **baseline window** — the one derived from the composition, which is
/// the window the committed picture is taken in and therefore the one the
/// criterion is about. A shorter window is the other half of the rule, and is
/// `the_column_scrolls_when_its_tiles_reach_their_floor`'s.
#[test]
fn the_column_holds_one_tile_per_column_at_one_height_inside_the_pane() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let filed: Vec<&str> = opened
        .dashboard
        .column_tiles()
        .iter()
        .map(|t| t.column())
        .collect();
    assert_eq!(
        filed,
        vec![
            "median_income",
            "house_age",
            "avg_rooms",
            "avg_bedrooms",
            "population",
            "avg_occupancy",
            "median_house_value",
        ],
        "the column holds one tile per tiled column in the file's own order, \
         the map's own two excluded"
    );
    assert_eq!(filed.len(), STACKED);
    drop(opened);

    let app = settled(baseline_screen());
    let group = app.canvas_panes();
    let columns = group.pane("columns").expect("the column pane drew");
    assert!(group.page.is_some(), "the page reached the screen");

    // Plot 0 is the hero; the rest are the column, in the order the
    // composition placed them.
    let placed = app.composed_plot_rects();
    assert_eq!(
        placed.len(),
        STACKED + 1,
        "eight tiles were chosen and {} plots were placed",
        placed.len()
    );
    let stacked = &placed[1..];
    let first = stacked[0];
    for (i, tile) in stacked.iter().enumerate() {
        assert!(
            (tile.height() - first.height()).abs() < 1.0,
            "tile {i} stands {} points high where the first stands {} — the \
             column's tiles share one height",
            tile.height(),
            first.height()
        );
        assert!(
            columns.body.contains_rect(tile.shrink(0.5)),
            "tile {i} at {tile:?} is not inside the column pane's content rect \
             {:?}",
            columns.body
        );
    }
    assert!(
        first.height() >= MIN_COLUMN_TILE_HEIGHT - 0.5,
        "the column's tiles drew {} points high, under the {MIN_COLUMN_TILE_HEIGHT} \
         point floor, so the page did not grow to hold them",
        first.height()
    );
}

/// **AC4 — the count reads at the map pane's lower-right, over the picture and
/// clear of its axes.**
///
/// Two claims. It is at the lower-right *of the hero's own frame*, which is
/// what an overlay on a chart means — inside the data area, on the marks,
/// above the axis band. And it is an overlay rather than a band: it sits ON the
/// hero's drawn rect instead of beside it, which is the falsifiable half of
/// "costs the pane no room".
///
/// **What this used to assert, and why that clause is gone.** It compared the
/// map pane's content rect against the rect the frame alone gives, to hold that
/// the overlay allocated nothing. That assertion could not fail: `map.body` is
/// captured from `pane_frame` before the overlay is drawn, so nothing the
/// overlay did could move the value being compared. The property does hold,
/// and it holds by construction one level down — `count_overlay` takes
/// `&egui::Ui`, and allocating space needs `&mut` — so it is a signature, not a
/// test. What is asserted here instead is what a reader would actually lose.
#[test]
fn the_count_reads_at_the_map_panes_lower_right_and_costs_it_no_room() {
    let app = settled(SCREEN);
    let group = app.canvas_panes();
    let map = group.pane("map").expect("the map pane drew");
    let count = group.count.expect("the map pane drew its count overlay");

    assert!(
        map.body.contains_rect(count),
        "the count at {count:?} is not inside the map pane's content rect {:?}",
        map.body
    );
    let lower_right = egui::pos2(map.body.right(), map.body.bottom());
    assert!(
        count.right() < lower_right.x && count.bottom() < lower_right.y,
        "the count is not inset from the map pane's lower-right corner: \
         {count:?} against {lower_right:?}"
    );
    assert!(
        count.center().x > map.body.center().x && count.center().y > map.body.center().y,
        "the count reads somewhere other than the map pane's lower-right \
         quadrant: {count:?} in {:?}",
        map.body
    );
    // …and the picture still fills the pane it is drawn in, which is the
    // falsifiable half of "costs the pane no room": a chip given a band of its
    // own at the foot of the pane would stop the hero short of the pane's
    // bottom edge by exactly the band's height.
    let hero = app.composed_plot_rects()[0];
    assert!(
        (hero.bottom() - map.body.bottom()).abs() <= 1.5,
        "the hero's drawn rect ends at {} where the map pane's content rect \
         ends at {} — {:.1} points of the pane are not picture, and the count \
         is what is drawn in them",
        hero.bottom(),
        map.body.bottom(),
        map.body.bottom() - hero.bottom()
    );
    assert!(
        hero.contains_rect(count),
        "the count at {count:?} is not on the hero at {hero:?} — an overlay \
         beside the picture is a band, and a band is layout"
    );
}

/// **The count is over the map's marks and clear of its axes** — the chip
/// covers no tick label and no axis title.
///
/// It did: at the map pane's lower-right the chip landed on the x-axis band
/// and covered the `longitude` title outright in the dashboard baselines, and
/// left it reading as the orphan "lo" in the point-map pair. The axis region is
/// readable from the composition — a plot's own layout says where its frame
/// ends and its axis band starts — so this is asserted rather than left to a
/// photograph.
///
/// The frame is derived from the SAME layout the chip is placed against, which
/// is deliberate: what is being held is not the arithmetic but that the chip is
/// placed against the plot's frame at all. Move it back to the pane's rect —
/// where it was — and this reddens, because a pane is taller than the frame
/// inside it by exactly the axis band.
#[test]
fn the_count_reads_over_the_map_and_leaves_its_axes_whole() {
    let app = settled(SCREEN);
    let count = app
        .canvas_panes()
        .count
        .expect("the map pane drew its count overlay");
    let hero = app.composed_plot_rects()[0];
    let doc = app.chart_doc();
    let layout = &doc.composed.plots[0].layout;
    #[allow(clippy::cast_possible_truncation)]
    let frame = egui::Rect::from_min_max(
        egui::pos2(
            hero.left() + layout.plot_x_start() as f32,
            hero.top() + layout.plot_y_start() as f32,
        ),
        egui::pos2(
            hero.left() + layout.plot_x_end() as f32,
            hero.top() + layout.plot_y_end() as f32,
        ),
    );
    assert!(
        frame.contains_rect(count),
        "the count at {count:?} reaches outside the map's data area {frame:?} \
         — below it is the x-axis band, whose ticks and `longitude` title the \
         chip then covers; to its left is the y-axis band"
    );
}

/// **The other half of AC3's rule: past the floor, the page grows and the
/// group scrolls.**
///
/// At 1440 by 900 the ledger rail is open at its default and the canvas has
/// less height than seven tiles at their floor need, so the column does not
/// compress: the page is composed taller than the pane and what does not fit
/// is scrolled to. Held here rather than left implicit, because "the tiles
/// shrank instead" and "the page grew" are the same picture at the top of the
/// pane and differ only in what is below the fold.
#[test]
fn the_column_scrolls_when_its_tiles_reach_their_floor() {
    let app = settled_scrollable(SCREEN);
    let group = app.canvas_panes();
    let columns = group.pane("columns").expect("the column pane drew");
    let placed = app.composed_plot_rects();
    let stacked = &placed[1..];
    assert_eq!(stacked.len(), STACKED);
    for (i, tile) in stacked.iter().enumerate() {
        assert!(
            (tile.height() - MIN_COLUMN_TILE_HEIGHT).abs() < 1.0,
            "tile {i} drew {} points high where the floor is \
             {MIN_COLUMN_TILE_HEIGHT} — the column compressed instead of the \
             page growing",
            tile.height()
        );
    }
    let spanned = stacked
        .iter()
        .map(egui::Rect::bottom)
        .fold(f32::NEG_INFINITY, f32::max)
        - stacked
            .iter()
            .map(egui::Rect::top)
            .fold(f32::INFINITY, f32::min);
    assert!(
        spanned > columns.body.height() + 1.0,
        "the column spans {spanned} points inside a pane {} points tall, so \
         nothing was scrolled past and this window is not the one this test \
         is about",
        columns.body.height()
    );
}

/// **The gutter in the emitted spec is what lands the column's tiles on the
/// column pane's content rect.**
///
/// The arithmetic in `window::canvas_pane_rects` and the `hspace` in
/// `Dashboard::to_spec` are two halves of one number, and a laid-out frame is
/// what says whether they agree. A page whose stack landed a few
/// points off would clip a tile's axis labels and look, in a photograph, like
/// a font change.
#[test]
fn the_stack_lands_exactly_on_the_column_panes_content_rect() {
    let app = settled(SCREEN);
    let group = app.canvas_panes();
    let columns = group.pane("columns").expect("the column pane drew");
    let placed = app.composed_plot_rects();
    let stacked = &placed[1..];
    let left = stacked
        .iter()
        .map(|r| r.left())
        .fold(f32::INFINITY, f32::min);
    let right = stacked
        .iter()
        .map(|r| r.right())
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (left - columns.body.left()).abs() < 1.0 && (right - columns.body.right()).abs() < 1.0,
        "the column's tiles span {left}..{right} where the pane's content rect \
         is {}..{} — the spec's gutter and the shell's pane split disagree",
        columns.body.left(),
        columns.body.right()
    );
}

/// **A document that is one picture is drawn as one pane, not a group.**
///
/// The pane group is the shape of a *generated dashboard*; an authored spec is
/// one composition and gets the canvas it always had. Held here because the
/// branch is in the draw path and a frame is the only place it is taken.
#[test]
fn an_authored_spec_still_draws_one_pane() {
    let composed = brightfield_shell::pipeline::compose_spec("../../examples/dashboard.yaml")
        .expect("compose the example dashboard");
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    assert!(
        app.canvas_panes().panes.is_empty(),
        "an authored spec drew a pane group: {:?}",
        app.canvas_panes()
    );
}

/// **The map pane holds the whole hero, axes and all** — at the window the
/// baseline is photographed in and at the shorter one where the column
/// overflows.
///
/// The failure this is here for is not subtle and was invisible to every test
/// above: at 1440 by 900 the page is composed 672 points tall for the column's
/// seven tiles at their floor, the map pane's content rect is 588, and a hero
/// that took the page's height put its x-axis — ticks, labels and the
/// `longitude` title — 112 points below the pane and had them clipped away.
/// (The page is 84 points taller than the pane's content rect; the raster
/// starts 28 points down it, below the chart's toolbar band, so what hangs
/// past the bottom is the sum. `the_columns_scroll_stops_at_the_end_of_its_page`
/// reads that overflow off a frame as the column's reach.) The
/// containment tests in this file all open with `let stacked = &placed[1..];`,
/// so the hero is excluded from every one of them by construction. This is the
/// one that looks at it.
///
/// Read off the drawn rect, which is the composition's own placed rect moved
/// by the origin the pane painted the page at.
#[test]
fn the_hero_is_composed_whole_inside_the_map_pane() {
    for screen in [baseline_screen(), SCREEN] {
        let app = settled(screen);
        let group = app.canvas_panes();
        let map = group.pane("map").expect("the map pane drew");
        let placed = app.composed_plot_rects();
        assert_eq!(
            placed.len(),
            STACKED + 1,
            "eight tiles were chosen and {} plots were placed",
            placed.len()
        );
        let hero = placed[0];
        assert!(
            map.body.contains_rect(hero.shrink(0.5)),
            "at {screen:?} the hero drew {hero:?}, which is not inside the map \
             pane's content rect {:?} — it overflows by {:.1} points at the \
             bottom, and what is down there is the x-axis",
            map.body,
            (hero.bottom() - map.body.bottom()).max(0.0)
        );
    }
}

/// **A wheel over the column moves the column, and the map stays where it
/// was.**
///
/// One page, two views: the column's tiles are drawn at an origin the scroll
/// moves and the hero at one it does not, so this reads both back off the same
/// frame. Before this, the page moved under both panes together — the whole
/// picture rose, the map's title went with it, and the page's top reached
/// above the panes' header bands.
///
/// The distances are the frame's own: the tile is asserted to have moved by
/// exactly [`MeridianApp::canvas_scroll`], not by the wheel travel this test
/// sent, because what the smoothing delivered in six frames is egui's business
/// and what the pane did with it is this window's.
#[test]
fn a_wheel_over_the_column_moves_the_column_and_leaves_the_map_where_it_was() {
    let before = settled_scrollable(SCREEN);
    let still = before.composed_plot_rects();
    let columns = before
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew");
    let after = settled_after(SCREEN, Ledger::Reopened, Some(columns.body.center()), 4);

    let scrolled = after.canvas_scroll();
    assert!(
        scrolled > 0.0,
        "the wheel over the column pane moved it {scrolled} points, so nothing \
         below is being asserted about a scrolled window"
    );
    let moved = after.composed_plot_rects();
    assert_eq!(moved.len(), still.len());
    for (i, (was, now)) in still.iter().zip(&moved).enumerate().skip(1) {
        assert!(
            (was.top() - now.top() - scrolled).abs() < 0.5,
            "tile {i} was at {was:?} and is at {now:?}, a move of {} where the \
             column scrolled {scrolled}",
            was.top() - now.top()
        );
    }
    assert!(
        (still[0].top() - moved[0].top()).abs() < 0.5
            && (still[0].bottom() - moved[0].bottom()).abs() < 0.5,
        "the hero was at {:?} and is at {:?} after a wheel over the COLUMN — \
         the map moved with the scroll",
        still[0],
        moved[0]
    );
    let map = after.canvas_panes().pane("map").expect("the map pane drew");
    assert!(
        map.body.contains_rect(moved[0].shrink(0.5)),
        "after the scroll the hero at {:?} is no longer inside the map pane's \
         content rect {:?}",
        moved[0],
        map.body
    );
}

/// **One wheel event, one consumer** — the column's, when the pointer is over
/// the column.
///
/// The wheel had two readers: the canvas scrolled the page from
/// `smooth_scroll_delta` and the chart's own gesture machine zoomed the plot
/// under the cursor from the same frame's wheel events, and neither consumed
/// it. Four notches over the column scrolled it AND left the tile under the
/// pointer zoomed onto a domain with no bars in it.
#[test]
fn a_wheel_over_the_column_does_not_zoom_the_tile_under_it() {
    let before = settled_scrollable(SCREEN);
    let columns = before
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew");
    let placed = before.composed_plot_rects();
    let under = placed[1].center();
    assert!(
        columns.body.contains(under),
        "the point this test turns the wheel over, {under:?}, is not on a tile \
         of the column pane at {:?}",
        columns.body
    );
    let was = tile_domains(&before);

    let after = settled_after(SCREEN, Ledger::Reopened, Some(under), 4);
    assert!(
        after.canvas_scroll() > 0.0,
        "the wheel over the column did not scroll it, so a domain that did not \
         move says nothing"
    );
    let now = tile_domains(&after);
    assert_eq!(
        was, now,
        "a wheel over the column pane moved a tile's domain: the column \
         scrolled and the plot under the pointer zoomed on the same event"
    );
}

/// **The other half of one wheel, one consumer**: over the map the wheel is
/// the chart's, and the column does not move under it.
#[test]
fn a_wheel_over_the_map_does_not_scroll_the_column() {
    let before = settled_scrollable(SCREEN);
    let map = before
        .canvas_panes()
        .pane("map")
        .expect("the map pane drew");
    let still = before.composed_plot_rects();
    let after = settled_after(SCREEN, Ledger::Reopened, Some(map.body.center()), 4);

    assert_eq!(
        after.canvas_scroll(),
        0.0,
        "a wheel over the MAP pane scrolled the column by {} points",
        after.canvas_scroll()
    );
    let moved = after.composed_plot_rects();
    for (i, (was, now)) in still.iter().zip(&moved).enumerate().skip(1) {
        assert!(
            (was.top() - now.top()).abs() < 0.5,
            "tile {i} moved from {was:?} to {now:?} on a wheel over the map"
        );
    }
    // …and the wheel reached the chart, which is what makes the assertion
    // above a routing claim rather than a wheel that went nowhere.
    assert_ne!(
        tile_domains(&before)[0],
        tile_domains(&after)[0],
        "the wheel over the map pane left the hero's domain alone, so nothing \
         consumed it and this test would pass with the wheel unwired"
    );
}

/// One plot's x and y domain, as the debug spelling of the two scales — the
/// readback a zoom and a pan move, and a scroll must not.
type Domain = (String, String);

/// One [`Domain`] per plot, in the order the composition placed them.
fn tile_domains(app: &MeridianApp) -> Vec<Domain> {
    app.chart_doc()
        .composed
        .plots
        .iter()
        .map(|plot| {
            let read = |channel| format!("{:?}", plot.scales.get(channel));
            (
                read(brightfield_render::channel::Channel::X),
                read(brightfield_render::channel::Channel::Y),
            )
        })
        .collect()
}

/// **A brush on a scrolled tile lands on the tile under the pointer** — the
/// gesture half of drawing one page in two views.
///
/// A page drawn at two origins has two pointer mappings, and a press read
/// against the wrong one lands wherever on the page that distance falls: at
/// this window the column is scrolled 112 points, which is more than a tile, so
/// the unshifted mapping puts a sweep on the *last* tile onto a different
/// column's bars. That is a cross-filter narrowing a column the reader never
/// touched — the same picture as a working brush, over the wrong data.
///
/// Read off the committed selection rather than off the drag: what is asserted
/// is the clause the engine is holding, which is what every other tile then
/// draws through.
#[test]
fn a_brush_on_a_scrolled_tile_lands_on_the_tile_under_the_pointer() {
    let (mut app, ctx, raw) = window();
    let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };

    // Scroll the column to the end of its reach, over the column pane.
    let columns = app
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    frame(&mut app, vec![egui::Event::PointerMoved(columns.center())]);
    for _ in 0..6 {
        frame(
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -WHEEL_NOTCH),
                modifiers: egui::Modifiers::default(),
                phase: egui::TouchPhase::Move,
            }],
        );
    }
    for _ in 0..6 {
        frame(&mut app, Vec::new());
    }
    let scrolled = app.canvas_scroll();
    assert!(
        scrolled > MIN_COLUMN_TILE_HEIGHT,
        "the column scrolled {scrolled} points, which is less than one tile — \
         a sweep read against the unmoved origin would land on the same tile \
         and this test would pass either way"
    );

    // The last tile, which is only reachable at all because the column
    // scrolled, and a point across the middle of its data area.
    let last = app.composed_plot_rects().len() - 1;
    let (at, want) = {
        let drawn = app.composed_plot_rects()[last];
        let doc = app.chart_doc();
        let plot = &doc.composed.plots[last];
        let l = &plot.layout;
        #[allow(clippy::cast_possible_truncation)]
        let x = |f: f64| {
            drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * f) as f32
        };
        #[allow(clippy::cast_possible_truncation)]
        let y = drawn.top() + ((l.plot_y_start() + l.plot_y_end()) / 2.0) as f32;
        (
            (egui::pos2(x(0.25), y), egui::pos2(x(0.7), y)),
            plot.x_column.clone().expect("the tile draws a column"),
        )
    };
    assert!(
        columns.contains(at.0) && columns.contains(at.1),
        "the sweep {at:?} is not inside the column pane's content rect \
         {columns:?}, so it is not a gesture on the tile this test is about"
    );

    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(
        &mut app,
        vec![egui::Event::PointerMoved(at.0), press(at.0, true)],
    );
    frame(&mut app, vec![egui::Event::PointerMoved(at.1)]);
    frame(&mut app, vec![press(at.1, false)]);
    for _ in 0..2 {
        frame(&mut app, Vec::new());
    }

    let held = app
        .chart_doc()
        .selection_sql()
        .expect("the sweep committed a selection");
    assert!(
        held.contains(&want),
        "the sweep on the last tile committed {held:?}, which does not name \
         {want} — the press was read against the page's own origin rather than \
         the origin the column pane draws it at, so it landed on whichever tile \
         is {scrolled} points up the page"
    );
}

/// A window over the fixture and a closure that drives one raw frame into it.
///
/// The gesture tests below need frames they choose the events for, one at a
/// time, which is what [`settled`] and [`settled_after`] do not offer: a press
/// and its release are different frames, and a gesture that crosses the pane
/// boundary needs the frames in between to carry the pointer across.
fn window() -> (MeridianApp, egui::Context, egui::RawInput) {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    // The rail reopened before the gesture, for the reason
    // [`settled_scrollable`] gives: every caller of this helper is asserting
    // something about a page taller than the pane it is drawn in, and at this
    // window the rail's own height is what makes one.
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    reopen_the_ledger(&mut app, &ctx, &raw);
    (app, ctx, raw)
}

/// Turn the wheel over the column pane until the scroll stops moving, then
/// settle — the whole reach, without naming a distance.
///
/// `notches` frames of travel, then six carrying no wheel: the six are egui's
/// smoothing tail, which decays across frames after the wheel stops, so a frame
/// read before them is reading the smoothing constant.
fn scroll_the_column(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    raw: &egui::RawInput,
    over: egui::Pos2,
    notches: usize,
) {
    let mut frame = |events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    frame(vec![egui::Event::PointerMoved(over)]);
    for _ in 0..notches {
        frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -WHEEL_NOTCH),
            modifiers: egui::Modifiers::default(),
            phase: egui::TouchPhase::Move,
        }]);
    }
    for _ in 0..6 {
        frame(Vec::new());
    }
}

/// A point of the hero's own data area, at `fx` and `fy` of its width and
/// height — resolved against the frame rather than typed.
///
/// The hero is drawn in the map pane and does not move with the column's
/// scroll, so it is the same window-space point whatever the column's scroll.
fn hero_data_point(app: &MeridianApp, fx: f64, fy: f64) -> egui::Pos2 {
    let drawn = app.composed_plot_rects()[0];
    let doc = app.chart_doc();
    let l = &doc.composed.plots[0].layout;
    #[allow(clippy::cast_possible_truncation)]
    let at = egui::pos2(
        drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx) as f32,
        drawn.top() + (l.plot_y_start() + (l.plot_y_end() - l.plot_y_start()) * fy) as f32,
    );
    at
}

/// A pointer button event, primary or secondary.
fn button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// **A gesture is a relation between frames, and a frame is not a gesture** —
/// the sweep a brush commits is the one the hand made, whichever pane the
/// pointer ended in.
///
/// The page is drawn at two origins, and until the gesture latched one, `start`
/// was captured in the origin of the frame the button went down in and
/// `current` was re-resolved on each frame in the origin of THAT frame.
/// Subtract the two and you have subtracted across two coordinate systems.
/// Measured on the code before the latch, with this exact gesture: the
/// committed `latitude` band came out taller than the identical screen gesture
/// on an unscrolled column by the scroll converted through the hero's y scale,
/// while `longitude` was bit-identical — the offset between the views is
/// vertical.
///
/// Asserted against the same gesture at scroll zero rather than against a
/// number, because at scroll zero the two origins coincide and the gesture has
/// one reading. The predicate is read off `selection_sql`, which is the clause
/// the engine is holding and therefore what the other tiles draw through.
#[test]
fn a_brush_across_the_pane_boundary_commits_what_it_swept() {
    // The screen points, derived once from an unscrolled window and used
    // unchanged in both runs — so the two runs are literally the same gesture.
    let reference = settled_scrollable(SCREEN);
    let columns = reference
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    let map = reference
        .canvas_panes()
        .pane("map")
        .expect("the map pane drew")
        .body;
    let press = hero_data_point(&reference, 0.30, 0.30);
    let enter = egui::pos2(columns.left() + 2.0, press.y + 30.0);
    let release = egui::pos2(columns.left() + 20.0, press.y + 60.0);
    assert!(
        map.contains(press),
        "the press {press:?} is not in the map pane's content rect {map:?}"
    );
    assert!(
        columns.contains(enter) && columns.contains(release),
        "the sweep does not end inside the column pane's content rect \
         {columns:?}, so it never crosses the boundary this test is about"
    );

    let sweep = |notches: usize| -> (f32, String) {
        let (mut app, ctx, raw) = window();
        let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
            let mut input = raw.clone();
            input.events = events;
            let _ = ctx.run_ui(input, |ui| app.draw(ui));
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        if notches > 0 {
            scroll_the_column(&mut app, &ctx, &raw, columns.center(), notches);
        }
        let scrolled = app.canvas_scroll();
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(press),
                button(press, egui::PointerButton::Primary, true),
            ],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(enter)]);
        frame(&mut app, vec![egui::Event::PointerMoved(release)]);
        frame(
            &mut app,
            vec![button(release, egui::PointerButton::Primary, false)],
        );
        for _ in 0..2 {
            frame(&mut app, Vec::new());
        }
        let held = app
            .chart_doc()
            .selection_sql()
            .expect("the sweep committed a selection");
        (scrolled, held)
    };

    let (still, unscrolled) = sweep(0);
    let (moved, scrolled) = sweep(12);
    assert_eq!(
        still, 0.0,
        "the unscrolled run scrolled {still} points, so it is not the reading \
         the scrolled run is being compared against"
    );
    assert!(
        moved > MIN_COLUMN_TILE_HEIGHT,
        "the scrolled run moved the column {moved} points, less than one tile \
         — an origin read against the wrong view would land within the same \
         tile and this comparison would hold either way"
    );
    assert_eq!(
        scrolled, unscrolled,
        "the same screen sweep committed one predicate with the column \
         scrolled {moved} points and another with it unscrolled — the drag's \
         current point was re-read in the origin of the frame the pointer had \
         reached, and differenced against a start latched in the other"
    );
}

/// **The other gesture that spans frames**: a secondary-button pan moves the
/// frame by what the hand moved, and crossing the pane boundary is not hand
/// movement.
///
/// The pan's step is `p - last`, and `last` was read a frame ago. Read `p` in
/// this frame's origin and the frame the pointer crosses on contributes the
/// hand's travel plus the offset between the views, in one step the reader did
/// not make. This had no test under the pane group.
///
/// Asserted against the same pan at scroll zero, for the reason
/// `a_brush_across_the_pane_boundary_commits_what_it_swept` gives, plus the
/// second half of the rule: the pan belongs to the plot it started on, so no
/// tile of the column moves under it.
#[test]
fn a_pan_across_the_pane_boundary_moves_by_what_the_hand_moved() {
    let reference = settled_scrollable(SCREEN);
    let columns = reference
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    // Near the hero's trailing edge, so the hand crosses the boundary in a
    // short move. The point map keeps an equal aspect inside a frame the rows
    // pane has shortened, so one point of travel is worth more data than it
    // was: a press three tenths across has seven tenths of the frame to cover
    // before it reaches the column pane, and that much pan carries the
    // longitude domain off the data and drops the hero from the composition.
    // The plot count under the two runs is what says so if it happens again.
    let press = hero_data_point(&reference, 0.90, 0.30);
    let enter = egui::pos2(columns.left() + 2.0, press.y + 30.0);
    let release = egui::pos2(columns.left() + 20.0, press.y + 60.0);

    let pan = |notches: usize| -> (f32, Vec<Domain>, Vec<Domain>) {
        let (mut app, ctx, raw) = window();
        let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
            let mut input = raw.clone();
            input.events = events;
            let _ = ctx.run_ui(input, |ui| app.draw(ui));
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        if notches > 0 {
            scroll_the_column(&mut app, &ctx, &raw, columns.center(), notches);
        }
        let scrolled = app.canvas_scroll();
        let before = tile_domains(&app);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(press),
                button(press, egui::PointerButton::Secondary, true),
            ],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(enter)]);
        frame(&mut app, vec![egui::Event::PointerMoved(release)]);
        frame(
            &mut app,
            vec![button(release, egui::PointerButton::Secondary, false)],
        );
        for _ in 0..2 {
            frame(&mut app, Vec::new());
        }
        (scrolled, before, tile_domains(&app))
    };

    let (still, _, unscrolled) = pan(0);
    let (moved, was, scrolled) = pan(12);
    assert_eq!(
        still, 0.0,
        "the unscrolled run scrolled {still} points, so it is not the reading \
         the scrolled run is being compared against"
    );
    assert!(
        moved > 0.0,
        "the scrolled run did not scroll the column, so both runs read the \
         page at one origin and a latch that did nothing would pass here"
    );
    assert_eq!(
        was.len(),
        scrolled.len(),
        "the composition held {} plots before the pan and {} after, so the \
         pan carried the hero off its own data and the plot went with it — \
         the readings below are lists of different lengths",
        was.len(),
        scrolled.len()
    );
    assert_ne!(
        was[0], scrolled[0],
        "the pan left the hero's domain where it was, so nothing consumed the \
         gesture and the comparison below is between two unmoved frames"
    );
    assert_eq!(
        scrolled[0], unscrolled[0],
        "the same screen pan moved the hero's frame one distance with the \
         column scrolled {moved} points and another with it unscrolled — the \
         step across the pane boundary carried the offset between the views"
    );
    assert_eq!(
        &was[1..],
        &scrolled[1..],
        "a pan that started on the hero and ended over the column moved a \
         column tile's domain — the pan belongs to the plot its press landed on"
    );
}

/// **The column's scroll stops at the end of its page.**
///
/// `canvas_scroll` is clamped to the page's reach — how far the page hangs
/// below the column pane's content rect — and no test held that ceiling until
/// this one: with the clamp loosened, more wheel than the reach needs carries
/// the last tile off the top of the pane and leaves the pane's foot blank,
/// while the scroll tests above stay green because each turns the wheel a
/// distance the loose ceiling still clamps.
///
/// The reach is read off the frame — the page's own bottom against the pane's
/// content bottom — rather than recomputed from the tile floor and the pane
/// arithmetic, which would be the clamp's own sum written twice.
#[test]
fn the_columns_scroll_stops_at_the_end_of_its_page() {
    let before = settled_scrollable(SCREEN);
    let columns = before
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    let page = before
        .chart_doc()
        .raster_rect
        .expect("the page was laid out");
    let reach = page.bottom() - columns.bottom();
    assert!(
        reach > 0.0,
        "the page ends at {} and the column pane's content rect at {} — \
         nothing hangs below the fold at this window, so there is no ceiling \
         to reach",
        page.bottom(),
        columns.bottom()
    );

    // Four times the travel the reach needs, so a ceiling raised by any margin
    // short of that is overshot rather than approached.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let notches = (reach / WHEEL_NOTCH).ceil() as usize * 4 + 4;
    let after = settled_after(SCREEN, Ledger::Reopened, Some(columns.center()), notches);
    assert!(
        (after.canvas_scroll() - reach).abs() < 0.5,
        "{notches} notches of wheel scrolled the column {} points where the \
         page's reach below the pane is {reach} — the scroll ran past the end \
         of its own page",
        after.canvas_scroll()
    );

    let last = *after
        .composed_plot_rects()
        .last()
        .expect("the composition placed its tiles");
    assert!(
        (last.bottom() - columns.bottom()).abs() < 0.5,
        "scrolled to the end, the last tile's bottom is at {} and the column \
         pane's content rect ends at {} — the column tore away from the foot \
         of the pane and left {} points of it blank",
        last.bottom(),
        columns.bottom(),
        columns.bottom() - last.bottom()
    );
}

/// **The second view's own box bounds its pointer mapping** — a sweep across
/// the column pane's header band brushes no tile, with the column scrolled.
///
/// `PaneViews::second_holds` is horizontal, because which view a *plot* is in
/// is a question about the page's width: a tile scrolled below the pane's
/// content bottom is still the column's tile. A *pointer* is placed by a rect
/// test instead — `PaneViews::offset_at`, through `page_offset` — because this
/// view paints nothing outside its own box, so a pointer above it is on the
/// pane's title band and not on anything this view drew. Drop the vertical
/// half and the band maps onto whatever the scroll has carried above the fold:
/// at this window a sweep across the pane's own title commits an interval on
/// the first stacked tile's column, which is a cross-filter nobody asked for.
///
/// The second run is the control. It is the same sweep at the same x, moved
/// down into the pane's content rect, and it does commit — so the silence above
/// is the mapping refusing the band and not the gesture machine being wired to
/// nothing.
#[test]
fn a_sweep_on_the_column_panes_header_band_lands_on_no_tile() {
    let reference = settled_scrollable(SCREEN);
    let pane = reference
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew");
    let (header, body) = (pane.header, pane.body);
    assert!(
        header.bottom() <= body.top(),
        "the column pane's header band {header:?} reaches into its content \
         rect {body:?}, so a point in the band is not above the second view"
    );

    let sweep_at = |y: f32| -> Option<String> {
        let (mut app, ctx, raw) = window();
        let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
            let mut input = raw.clone();
            input.events = events;
            let _ = ctx.run_ui(input, |ui| app.draw(ui));
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        scroll_the_column(&mut app, &ctx, &raw, body.center(), 12);
        assert!(
            app.canvas_scroll() > 0.0,
            "the column did not scroll, so the band and the page's top are the \
             same place and this test is about neither"
        );
        let from = egui::pos2(body.center().x - 40.0, y);
        let to = egui::pos2(body.center().x + 40.0, y);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(from),
                button(from, egui::PointerButton::Primary, true),
            ],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(to)]);
        frame(
            &mut app,
            vec![button(to, egui::PointerButton::Primary, false)],
        );
        for _ in 0..2 {
            frame(&mut app, Vec::new());
        }
        app.chart_doc().selection_sql()
    };

    let on_the_band = sweep_at(header.center().y);
    let in_the_pane = sweep_at(body.top() + MIN_COLUMN_TILE_HEIGHT / 2.0);
    assert!(
        in_the_pane.is_some(),
        "the control sweep inside the column pane's content rect committed \
         nothing, so the band committing nothing says only that the gesture \
         machine is dead"
    );
    assert_eq!(
        on_the_band, None,
        "a sweep across the column pane's header band committed {on_the_band:?} \
         — the band was mapped onto the page at the second view's origin, which \
         puts it on the part of the column the scroll carried above the fold"
    );
}

/// The column the inspector is showing, by name — what a press on a tile sets,
/// and what a press on nothing must leave alone.
fn selected_column(app: &MeridianApp) -> Option<String> {
    app.chart_doc()
        .selected_column()
        .map(|facts| facts.column.clone())
}

/// What a press-and-drag left behind: the clause the engine is holding, and the
/// column the inspector is showing.
type Landed = (Option<String>, Option<String>);

/// Scroll the column by `notches` with the pointer `over` it, click `hero` so
/// the group has a selected tile, then press at `from`, drag to `to` and
/// release. Reports the scroll the gesture ran at and what it left behind.
///
/// The hero click is what makes "the selected tile is unchanged" an assertion
/// rather than a tautology: with nothing selected to begin with, a probe that
/// selected nothing and a document with nothing to select read the same.
fn landing_of(
    notches: usize,
    over: egui::Pos2,
    hero: egui::Pos2,
    from: egui::Pos2,
    to: egui::Pos2,
) -> (f32, Landed) {
    let (mut app, ctx, raw) = window();
    let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }
    if notches > 0 {
        scroll_the_column(&mut app, &ctx, &raw, over, notches);
    }
    let scrolled = app.canvas_scroll();

    frame(
        &mut app,
        vec![
            egui::Event::PointerMoved(hero),
            button(hero, egui::PointerButton::Primary, true),
        ],
    );
    frame(
        &mut app,
        vec![button(hero, egui::PointerButton::Primary, false)],
    );
    for _ in 0..2 {
        frame(&mut app, Vec::new());
    }

    frame(
        &mut app,
        vec![
            egui::Event::PointerMoved(from),
            button(from, egui::PointerButton::Primary, true),
        ],
    );
    frame(&mut app, vec![egui::Event::PointerMoved(to)]);
    frame(
        &mut app,
        vec![button(to, egui::PointerButton::Primary, false)],
    );
    for _ in 0..2 {
        frame(&mut app, Vec::new());
    }
    (
        scrolled,
        (app.chart_doc().selection_sql(), selected_column(&app)),
    )
}

/// **A pointer is over a page only where a pane drew one.**
///
/// The pane group draws one page in two boxes — the map pane's content rect at
/// the page's own origin, the column pane's moved up by the scroll — and the
/// page is bigger than their union in two directions at once. It is taller
/// than the panes, because the column's tiles have a height floor and the page
/// grows to hold them; and the gutter that keeps the two pane frames apart runs
/// down the middle of its width. The leftover is a real region of a real page:
/// at 1440 by 900 the band below the panes' content bottom is 112 points deep,
/// three quarters of it underneath the ledger rail.
///
/// Until the mapping could answer absence, a press there was answered with the
/// first view's origin, because "outside the second view" and "at the page's
/// own origin" were the same value. So a press-and-drag on the ledger rail
/// committed a crossfilter on whichever tile the page happened to have at that
/// depth, and the bare press changed the column the inspector was showing. The
/// gutter and the panes' own inset strips did the same at every window.
///
/// Each probe is checked to be inside the page and outside both panes before it
/// is used, so a layout change that moves the leftover out from under it
/// reddens this rather than quietly making it a press on nothing that could
/// never have been a press on something.
///
/// **Three of the six probes are the ones that bite, and the test says which.**
/// A press is only harmful where the page carries a tile at the origin it was
/// wrongly read against, and at this composition the gutter is a gap in the
/// page too: the hero's plot ends at x=781.5 and the column's tiles begin at
/// x=823.5, so a press between them was already refused for want of a plot
/// rather than for want of a pane. The band below the content rects is not —
/// the column's tiles run the whole height of the page. Each probe carries
/// whether the frame puts a tile under it, and that flag is checked against the
/// frame rather than trusted, so a layout that moves a plot into the gutter
/// turns those three probes load-bearing and says so instead of leaving them
/// decorative.
///
/// The last case is the control and the boundary: a sweep ending just above the
/// column's content bottom lands on the tile drawn there. Without it, "nothing
/// commits" would be satisfied by a mapping that refused the whole column.
#[test]
fn a_press_over_no_pane_of_the_group_is_over_no_page() {
    let reference = settled_scrollable(SCREEN);
    let panes = reference.canvas_panes();
    let map = *panes.pane("map").expect("the map pane drew");
    let columns = *panes.pane("columns").expect("the column pane drew");
    let page = reference
        .chart_doc()
        .raster_rect
        .expect("the page was laid out");
    let hero = hero_data_point(&reference, 0.30, 0.30);
    let hero_column = reference.chart_doc().tile_columns()[0].column.clone();

    // The leftover, as the sweeps that stay inside it: across the band below
    // both content rects and across the column pane's own inset strip in it,
    // then down the gap between the two pane frames and down each pane's inset
    // strip beside that gap. The band is deep and full width, so its sweeps run
    // across; the gap is 25 points wide, so its sweeps run down.
    //
    // The flag is whether the page carries a tile under the sweep at the page's
    // own origin — which is the origin the mapping used to answer with, so it
    // is exactly "would removing the bound land this press on something".
    let mid = columns.body.center().y;
    let across = |x: f32, y: f32| (egui::pos2(x - 30.0, y), egui::pos2(x + 30.0, y));
    let down = |x: f32, y: f32| (egui::pos2(x, y - 30.0), egui::pos2(x, y + 30.0));
    let probes = [
        (
            "the band below the panes' content rects",
            across(
                columns.body.center().x,
                (columns.body.bottom() + page.bottom()) / 2.0,
            ),
            true,
        ),
        (
            "the column pane's inset strip below its content rect",
            across(
                columns.body.center().x,
                (columns.body.bottom() + columns.rect.bottom()) / 2.0,
            ),
            true,
        ),
        (
            "one point below the column pane's content bottom",
            across(columns.body.center().x, columns.body.bottom() + 1.0),
            true,
        ),
        (
            // The strip the rows pane's own frame is drawn under, between the
            // map pane's content bottom and the bottom of the map pane
            // itself. `PaneViews::first` is the map's CONTENT rect, and this
            // is the band that says so: widen it to the pane rect and the
            // hero — bounded to `first`'s height by `ChartDoc::reflow_to` —
            // is composed 48 points taller and reaches down into here, so the
            // page carries a tile where the frame says it carries none and
            // the flag below stops agreeing with the frame.
            "the map pane's inset strip below its content rect",
            across(
                map.body.center().x,
                (map.body.bottom() + map.rect.bottom()) / 2.0,
            ),
            false,
        ),
        (
            "the gap between the two pane frames",
            down((map.rect.right() + columns.rect.left()) / 2.0, mid),
            false,
        ),
        (
            "the map pane's inset strip beside that gap",
            down((map.body.right() + map.rect.right()) / 2.0, mid),
            false,
        ),
        (
            "the column pane's inset strip beside that gap",
            down((columns.rect.left() + columns.body.left()) / 2.0, mid),
            false,
        ),
    ];
    let unmoved = reference.composed_plot_rects();
    let over_a_tile = |at: egui::Pos2| unmoved.iter().any(|rect| rect.contains(at));
    for (what, (from, to), bites) in probes {
        for at in [from, to] {
            assert!(
                page.contains(at),
                "{what} puts an end of the sweep at {at:?}, which is outside \
                 the page {page:?} — the mapping refuses it for want of a page \
                 rather than for want of a pane, and this probe would hold with \
                 the bound gone"
            );
            assert!(
                !map.body.contains(at) && !columns.body.contains(at),
                "{what} puts an end of the sweep at {at:?}, which is inside a \
                 pane's content rect ({:?} or {:?}) — a pane did draw the page \
                 there",
                map.body,
                columns.body
            );
        }
        assert_eq!(
            over_a_tile(from) || over_a_tile(to),
            bites,
            "{what} was written as a probe the bound {} the deciding refusal \
             for, and the frame says the opposite: the page's tiles at its own \
             origin are {unmoved:?} and the sweep runs {from:?} to {to:?}",
            if bites { "is" } else { "is not" }
        );
    }

    // Unscrolled and at the end of the column's reach, because the two are
    // different pages under the same probe. Collected rather than asserted one
    // at a time so a run that has lost the bound reports the whole leftover
    // instead of the first point of it.
    let mut landed = Vec::new();
    for notches in [0, 12] {
        for (what, (from, to), _) in probes {
            let (scroll, (held, selected)) =
                landing_of(notches, columns.body.center(), hero, from, to);
            if held.is_some() {
                landed.push(format!(
                    "a press-and-drag across {what}, with the column scrolled \
                     {scroll} points, committed {held:?}"
                ));
            }
            if selected.as_deref() != Some(hero_column.as_str()) {
                landed.push(format!(
                    "a press across {what}, with the column scrolled {scroll} \
                     points, moved the inspector from {hero_column} to \
                     {selected:?}"
                ));
            }
        }
    }
    assert!(
        landed.is_empty(),
        "the pointer was mapped onto a page no pane drew under it:\n  {}",
        landed.join("\n  ")
    );

    // The boundary, from the other side: the tile drawn just inside the
    // column's content bottom is the tile that sweep lands on.
    let scrolled = settled_after(SCREEN, Ledger::Reopened, Some(columns.body.center()), 12);
    let at = egui::pos2(columns.body.center().x, columns.body.bottom() - 1.0);
    let tile = scrolled
        .composed_plot_rects()
        .iter()
        .position(|rect| rect.contains(at))
        .expect("a tile is drawn at the foot of the column pane");
    let want = scrolled.chart_doc().tile_columns()[tile].column.clone();
    let (scroll, (held, selected)) = landing_of(
        12,
        columns.body.center(),
        hero,
        egui::pos2(at.x - 30.0, at.y),
        egui::pos2(at.x + 30.0, at.y),
    );
    assert_eq!(
        scroll,
        scrolled.canvas_scroll(),
        "the probe run and the run the tile was resolved against scrolled to \
         different places, so the tile under {at:?} is not the tile the probe \
         swept"
    );
    let held = held.unwrap_or_default();
    assert!(
        held.contains(&want),
        "a sweep one point above the column pane's content bottom committed \
         {held:?}, which does not name {want} — the tile drawn there"
    );
    assert_eq!(
        selected.as_deref(),
        Some(want.as_str()),
        "the same press left the inspector on {selected:?} rather than on \
         {want}, the tile drawn under it"
    );
}

/// A point across the middle of a stacked tile's data area, at `fx` of its
/// width — resolved against the frame, like [`hero_data_point`], for a tile the
/// column had to scroll to reach.
fn tile_data_point(app: &MeridianApp, tile: usize, fx: f64) -> egui::Pos2 {
    let drawn = app.composed_plot_rects()[tile];
    let doc = app.chart_doc();
    let l = &doc.composed.plots[tile].layout;
    #[allow(clippy::cast_possible_truncation)]
    let at = egui::pos2(
        drawn.left() + (l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fx) as f32,
        drawn.top() + ((l.plot_y_start() + l.plot_y_end()) / 2.0) as f32,
    );
    at
}

/// **A held click on a scrolled tile clears the selection** — the value the
/// press latches, held to the origin the frame drew rather than to a constant.
///
/// A click is a drag that swept nothing, and "swept nothing" is `start` and
/// `current` differing by less than the slop. Both are page-local points, so
/// they are comparable only in one origin: `start` is captured at the press
/// edge and `current` is re-read on every frame the button stays down, and the
/// value the press latched is what makes those two readings the same origin.
///
/// Latch zero instead of the frame's answer and the two readings sit a scroll
/// apart, so a click on a tile in a scrolled column arrives at the release with
/// a phantom vertical travel the hand never made. It resolves as a sweep, and
/// an interval sweep of no width commits a clause that selects nothing where
/// the click was meant to retract this plot's contribution and let the other
/// tiles back out. The gesture tests either side of this one hold a *sweep*
/// across the boundary, and a sweep is unmoved by a constant added to both of
/// its ends — which is why the wrong latch survived them.
///
/// Held for three frames with the button down, because the phantom travel is
/// written by the frames between the press and the release. A press and a
/// release in consecutive frames never re-read `current` at all.
#[test]
fn a_held_click_on_a_scrolled_tile_clears_the_selection() {
    let (mut app, ctx, raw) = window();
    let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }
    let columns = app
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    scroll_the_column(&mut app, &ctx, &raw, columns.center(), 12);
    let scrolled = app.canvas_scroll();
    assert!(
        scrolled > MIN_COLUMN_TILE_HEIGHT,
        "the column scrolled {scrolled} points, which is less than one tile — \
         the phantom travel a mislatched click carries is the scroll, and under \
         the slop it is not a sweep at all"
    );

    // The last tile, reachable only because the column scrolled, and a click at
    // the middle of its data area.
    let last = app.composed_plot_rects().len() - 1;
    let at = tile_data_point(&app, last, 0.5);
    assert!(
        columns.contains(at),
        "the click at {at:?} is outside the column pane's content rect \
         {columns:?}, so it is not a click on the tile this test is about"
    );

    // A sweep first, so there is a contribution for the click to retract.
    let from = tile_data_point(&app, last, 0.25);
    let to = tile_data_point(&app, last, 0.70);
    frame(
        &mut app,
        vec![
            egui::Event::PointerMoved(from),
            button(from, egui::PointerButton::Primary, true),
        ],
    );
    frame(&mut app, vec![egui::Event::PointerMoved(to)]);
    frame(
        &mut app,
        vec![button(to, egui::PointerButton::Primary, false)],
    );
    for _ in 0..2 {
        frame(&mut app, Vec::new());
    }
    let swept = app
        .chart_doc()
        .selection_sql()
        .expect("the sweep committed a selection for the click to clear");

    // The click: press, three frames with the button down and the pointer
    // still, release.
    frame(
        &mut app,
        vec![
            egui::Event::PointerMoved(at),
            button(at, egui::PointerButton::Primary, true),
        ],
    );
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }
    frame(
        &mut app,
        vec![button(at, egui::PointerButton::Primary, false)],
    );
    for _ in 0..2 {
        frame(&mut app, Vec::new());
    }

    let after = app.chart_doc().selection_sql();
    assert_eq!(
        after, None,
        "a held click on a tile in a column scrolled {scrolled} points left \
         {after:?} standing where it should have retracted {swept} — the press \
         latched an origin the frame did not draw the page at, so the frames \
         the button was held for read the pointer {scrolled} points off the \
         point it was pressed at and the click resolved as a sweep"
    );
}

/// **The brush rectangle stays where the hand is** — the transient ink is
/// painted in the origin the gesture latched, not in the origin of the frame
/// the pointer has reached.
///
/// A page drawn at two origins puts the same page-local rect in two window
/// places. The sweep's numbers are latched, so painting the ink against the
/// frame's origin instead makes the rectangle jump by the scroll at the instant
/// the pointer crosses the pane boundary and stay there for the rest of the
/// drag, while the clause the release commits is unmoved. The picture and the
/// predicate would be describing different rows.
///
/// Asserted as the same gesture at two scrolls rather than against a rect typed
/// here: the press is on the hero, the hero does not move with the column's
/// scroll, and the drag is latched to the map's origin — so the ink is the same
/// window-space rectangle in both runs. `gesture_ink` is what the painter is
/// handed, recorded rather than recomputed, which is what lets this run without
/// a device.
#[test]
fn the_brush_rectangle_stays_where_the_hand_is() {
    let reference = settled_scrollable(SCREEN);
    let columns = reference
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    let press = hero_data_point(&reference, 0.30, 0.30);
    let enter = egui::pos2(columns.left() + 20.0, press.y + 40.0);
    assert!(
        columns.contains(enter),
        "the drag ends at {enter:?}, outside the column pane's content rect \
         {columns:?} — it never crosses into the other origin"
    );

    let ink_mid_drag = |notches: usize| -> (f32, Option<egui::Rect>) {
        let (mut app, ctx, raw) = window();
        let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
            let mut input = raw.clone();
            input.events = events;
            let _ = ctx.run_ui(input, |ui| app.draw(ui));
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        if notches > 0 {
            scroll_the_column(&mut app, &ctx, &raw, columns.center(), notches);
        }
        let scrolled = app.canvas_scroll();
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(press),
                button(press, egui::PointerButton::Primary, true),
            ],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(enter)]);
        // Read with the button still down: the ink is the picture of an
        // uncommitted sweep and the release is where it stops existing.
        (scrolled, app.chart_doc().gesture_ink)
    };

    let (still, unscrolled) = ink_mid_drag(0);
    let (moved, scrolled) = ink_mid_drag(12);
    assert_eq!(
        still, 0.0,
        "the unscrolled run scrolled {still} points, so it is not the reading \
         the scrolled run is being compared against"
    );
    assert!(
        moved > MIN_COLUMN_TILE_HEIGHT,
        "the scrolled run moved the column {moved} points, less than one tile \
         — the two origins are close enough here that ink painted in either \
         would compare equal"
    );
    let unscrolled = unscrolled.expect("the unscrolled drag recorded its ink");
    let scrolled_ink = scrolled.expect("the scrolled drag recorded its ink");
    assert!(
        unscrolled.width() > 1.0 && unscrolled.height() > 1.0,
        "the drag recorded an ink rect of {unscrolled:?}, which has no area — \
         two empty rectangles compare equal wherever they are"
    );
    assert_eq!(
        scrolled_ink, unscrolled,
        "the same drag from the map into the column painted its rectangle at \
         {scrolled_ink:?} with the column scrolled {moved} points and at \
         {unscrolled:?} with it unscrolled — the ink was painted against the \
         origin of the frame the pointer had reached rather than the one the \
         press latched, so it jumped by the scroll at the boundary while the \
         clause the release commits stayed where the hand was"
    );
}

/// **A wheel during a drag does not move the column** — a latched origin is a
/// scroll value, and the page it names has to still be the page on screen.
///
/// The drag reads every frame's pointer against the origin the press latched,
/// which is what keeps a sweep across the pane boundary the sweep the hand
/// made. Nothing was stopping the column from scrolling underneath it: the
/// canvas takes the wheel whenever the pointer is over the column pane and asks
/// no question about the button. Turn the wheel mid-drag and the page moves
/// while the numbers do not, so the rectangle sits over tiles the gesture is
/// not about and the release commits the tile the press landed on. The x-only
/// clauses the other gesture tests assert cannot see it, because the offset
/// between the views is vertical.
///
/// A window resize does the same thing by re-clamping the scroll to a reach the
/// new height changed, which is why the clamp stands down with the wheel rather
/// than the wheel alone.
///
/// The third run is what makes this a routing claim: the same wheel with no
/// button down does scroll the column, so the silence in the first run is the
/// gesture holding the page and not a wheel that went nowhere.
#[test]
fn a_wheel_during_a_drag_does_not_move_the_column() {
    let reference = settled_scrollable(SCREEN);
    let columns = reference
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew")
        .body;
    let first = tile_data_point(&reference, 1, 0.25);
    let mid = tile_data_point(&reference, 1, 0.50);
    let last = tile_data_point(&reference, 1, 0.70);
    for at in [first, mid, last] {
        assert!(
            columns.contains(at),
            "the sweep passes through {at:?}, outside the column pane's content \
             rect {columns:?} — the wheel this test turns would not be the \
             column's"
        );
    }

    // The scroll read with the button still down, the scroll after the release,
    // and the clause the release committed.
    let drag = |wheel_during: bool, press: bool| -> (f32, f32, Option<String>) {
        let (mut app, ctx, raw) = window();
        let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
            let mut input = raw.clone();
            input.events = events;
            let _ = ctx.run_ui(input, |ui| app.draw(ui));
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        let mut down = vec![egui::Event::PointerMoved(first)];
        if press {
            down.push(button(first, egui::PointerButton::Primary, true));
        }
        frame(&mut app, down);
        for _ in 0..4 {
            let mut events = vec![egui::Event::PointerMoved(mid)];
            if wheel_during {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -WHEEL_NOTCH),
                    modifiers: egui::Modifiers::default(),
                    phase: egui::TouchPhase::Move,
                });
            }
            frame(&mut app, events);
        }
        frame(&mut app, vec![egui::Event::PointerMoved(last)]);
        let held = app.canvas_scroll();
        if press {
            frame(
                &mut app,
                vec![button(last, egui::PointerButton::Primary, false)],
            );
        }
        for _ in 0..6 {
            frame(&mut app, Vec::new());
        }
        (held, app.canvas_scroll(), app.chart_doc().selection_sql())
    };

    let (turned_held, _, turned) = drag(true, true);
    let (still_held, _, still) = drag(false, true);
    let (_, loose, _) = drag(true, false);

    assert!(
        loose > MIN_COLUMN_TILE_HEIGHT,
        "the same wheel with no button down scrolled the column {loose} points \
         — under a tile, so a run that refused to scroll during the drag would \
         read the same as one where the wheel reached nothing"
    );
    assert_eq!(
        turned_held, 0.0,
        "the column scrolled {turned_held} points while a drag was holding its \
         origin — the page moved out from under a gesture whose start point is \
         a page position in the origin the press latched"
    );
    assert_eq!(
        turned_held, still_held,
        "a drag with the wheel turned during it left the column at \
         {turned_held} and the same drag without at {still_held}"
    );
    assert!(
        still.is_some(),
        "the drag with no wheel committed nothing, so the comparison below is \
         between two absences"
    );
    assert_eq!(
        turned, still,
        "the same sweep committed {turned:?} with the wheel turned during it \
         and {still:?} without"
    );
}

/// **AC2 — the rows pane says how many of the table's columns are on screen.**
///
/// The two figures are asserted against the header cells the table drew and
/// the clips it drew them under, which is where "on screen" is a fact rather
/// than a plan: a column whose header cell is not wholly inside its own clip
/// is a column the reader is seeing part of. The count is recomputed here from
/// those rects rather than read from `TableDrawn::on_screen`, so a readout
/// derived from something other than the frame — the declared widths, a stale
/// record, a constant — comes out as a different number.
///
/// **A clipped column with no readout is the failure this exists for**, and it
/// is the second assertion: the first establishes that at this window some
/// column really is off screen, so the `expect` below is a claim about a note
/// that is missing rather than about a window where none was due.
///
/// At the baseline window — the one the committed picture is taken in, and the
/// one the criterion names.
#[test]
fn the_rows_pane_says_how_many_of_the_tables_columns_are_on_screen() {
    let app = settled(baseline_screen());
    let group = app.canvas_panes();
    let rows = group.pane("rows").expect("the rows pane drew");
    let drawn = app
        .chart_doc()
        .grid_drawn
        .clone()
        .expect("the rows pane's grid laid a table out");

    // The frame's own answer, recomputed: a header cell wholly inside the clip
    // it was drawn under is a column the reader can read the head of.
    let whole = drawn
        .header_cells
        .iter()
        .filter(|(_, rect, clip)| clip.contains_rect(rect.shrink(0.5)))
        .count();
    assert_eq!(
        drawn.columns, HOUSING_COLUMNS,
        "the fixture's table has {} columns, not the {HOUSING_COLUMNS} this \
         test is written against",
        drawn.columns
    );
    assert!(
        whole < drawn.columns,
        "every one of the {} columns fits the rows pane at the baseline \
         window, so there is no readout due and this test would hold with the \
         readout deleted. The pane's content rect is {:?} and the header cells \
         are {:?}",
        drawn.columns,
        rows.body,
        drawn.header_cells
    );
    // …and some of them do fit, which is what stops the readout and this
    // test agreeing on a record that says nothing drew at all. Watched fail
    // with `widest_per_column` keeping the first offer `egui_table` makes for
    // each header cell — the one under the zero-width corner region's clip.
    assert!(
        whole > 0,
        "no column of the table drew whole inside the clip it was drawn \
         under, so the record the readout is built from says the grid put \
         nothing on screen: {:?}",
        drawn.header_cells
    );

    let (rect, note) = group.rows_note.clone().unwrap_or_else(|| {
        panic!(
            "{whole} of the table's {} columns drew whole and the rows pane \
             said nothing — a reader sees a table cut off at the pane's edge \
             with no sign there is more of it",
            drawn.columns
        )
    });
    assert_eq!(
        note,
        format!("{whole} of {} columns", drawn.columns),
        "the rows pane's readout says {note:?} where the frame drew {whole} of \
         {} columns whole",
        drawn.columns
    );
    assert!(
        rows.header.contains_rect(rect),
        "the readout drew at {rect:?}, outside the rows pane's header band \
         {:?}",
        rows.header
    );
    assert!(
        rect.right() <= rows.header.right(),
        "the readout at {rect:?} reaches past the trailing end of the band \
         {:?}",
        rows.header
    );
}

/// How many columns the California Housing sample declares. Nine, which is the
/// table's own shape and the reason the rows pane has a readout at all: the
/// pane holds a little over half the canvas's width and the columns at their
/// natural widths do not all fit in it.
const HOUSING_COLUMNS: usize = 9;

/// **The columns the pane cannot fit are reached by scrolling sideways** —
/// which is what makes the readout a readout rather than an apology.
///
/// A wheel with a horizontal component over the rows pane, then the same
/// header cells read again: the last column, which the pane cut off before,
/// draws whole afterwards. Asserted as a change in *which* columns are whole
/// rather than as a scroll offset, because the offset is `egui_table`'s and
/// what the reader gets is the column.
#[test]
fn the_rows_grid_scrolls_sideways_to_a_column_the_pane_cannot_fit() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(baseline_screen()),
        ..Default::default()
    };
    let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }

    let whole_now = |app: &MeridianApp| -> Vec<usize> {
        app.chart_doc()
            .grid_drawn
            .as_ref()
            .expect("the rows pane's grid laid a table out")
            .header_cells
            .iter()
            .filter(|(_, rect, clip)| clip.contains_rect(rect.shrink(0.5)))
            .map(|(col, _, _)| *col)
            .collect()
    };
    let before = whole_now(&app);
    let last = HOUSING_COLUMNS - 1;
    assert!(
        !before.contains(&last),
        "the table's last column already drew whole at {before:?}, so nothing \
         below is a claim about reaching one that did not"
    );

    let over = app
        .canvas_panes()
        .pane("rows")
        .expect("the rows pane drew")
        .body
        .center();
    frame(&mut app, vec![egui::Event::PointerMoved(over)]);
    for _ in 0..12 {
        frame(
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-WHEEL_NOTCH, 0.0),
                modifiers: egui::Modifiers::default(),
                phase: egui::TouchPhase::Move,
            }],
        );
    }
    for _ in 0..6 {
        frame(&mut app, Vec::new());
    }
    let after = whole_now(&app);
    assert!(
        after.contains(&last),
        "after scrolling the rows pane sideways the whole columns are {after:?} \
         and the table's last one is not among them — the grid does not scroll \
         across, so the columns the readout counts out are unreachable"
    );
    assert_ne!(
        before, after,
        "the same columns drew whole before and after the sideways scroll"
    );
}
