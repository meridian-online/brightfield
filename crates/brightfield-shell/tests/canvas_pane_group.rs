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
//! # The third pane
//!
//! The composition this is cut from has three panes: the map, the column, and
//! the rows beneath the map. This build draws two, and the assertion below
//! says so by number rather than by inequality, so that landing the rows pane
//! reddens here and the claim has to be restated rather than silently widened.

use brightfield_shell::dashboard::{HERO_SHARE, MIN_COLUMN_TILE_HEIGHT};
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

/// A settled window over the fixture, laid out in `screen`.
fn settled(screen: egui::Rect) -> MeridianApp {
    settled_after(screen, None, 0)
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
fn settled_after(screen: egui::Rect, at: Option<egui::Pos2>, notches: usize) -> MeridianApp {
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
/// number is stated rather than bounded, so the third pane arriving is a
/// change this test reports instead of one it absorbs.
#[test]
fn every_pane_of_the_canvas_group_draws_its_own_header_band() {
    let app = settled(SCREEN);
    let group = app.canvas_panes();
    let names: Vec<&str> = group.panes.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        vec!["map", "columns"],
        "the canvas draws the map pane and the column beside it. The rows pane \
         beneath the map is the sibling card's, and when it lands this list \
         grows to three and this line is the one that says so."
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
/// the criterion names. The height clause of the criterion is the map pane's
/// share of *its column*, and until the rows pane lands its column is the
/// canvas, so what is asserted here is that the map pane reaches the canvas's
/// bottom.
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
            (map.rect.bottom() - canvas.bottom()).abs() < 1.0,
            "the map pane's column stops at {} where the canvas ends at {} — \
             until the rows pane lands the map has the whole of it",
            map.rect.bottom(),
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
    let app = settled(SCREEN);
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
/// `longitude` title — 84 points below the pane and had them clipped away. The
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
    let before = settled(SCREEN);
    let still = before.composed_plot_rects();
    let columns = before
        .canvas_panes()
        .pane("columns")
        .expect("the column pane drew");
    let after = settled_after(SCREEN, Some(columns.body.center()), 4);

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
    let before = settled(SCREEN);
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

    let after = settled_after(SCREEN, Some(under), 4);
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
    let before = settled(SCREEN);
    let map = before
        .canvas_panes()
        .pane("map")
        .expect("the map pane drew");
    let still = before.composed_plot_rects();
    let after = settled_after(SCREEN, Some(map.body.center()), 4);

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

/// Every plot's x and y domain, in the order the composition placed them —
/// the readback a zoom moves and a scroll must not.
fn tile_domains(app: &MeridianApp) -> Vec<(String, String)> {
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
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
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

