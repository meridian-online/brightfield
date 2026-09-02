//! **The canvas as a pane group**: the map pane, the column of tiles beside
//! it, and the count that reads at the map's lower-right.
//!
//! Every claim here is read off a **laid-out frame**, not off a declaration.
//! Whether a pane drew a header band, where its content rect fell, and whether
//! an overlay cost the pane any room are all facts about pixels a frame put on
//! a screen; a test that read them out of `arrangement.rs` would be comparing
//! the declaration with itself. `MeridianApp::canvas_panes` is what the frame
//! leaves behind for that, the way `region_rect` is for the regions.
//!
//! No GPU here on purpose. `MeridianApp::headless` lays every rect out exactly
//! as the device path does — the canvas pane reserves and paints nothing, and
//! the geometry around it is unchanged — which is what lets the *layout* half
//! of this card be gated without a wgpu adapter. The picture is
//! `tests/dashboard_baseline.rs`'s.
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
    app
}

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

/// **AC4 — the count reads at the map pane's lower-right, and costs the pane
/// no room.**
///
/// Two claims, and the second is the one that makes it an *overlay* rather
/// than a line of chrome: the map pane's content rect is the same rect it
/// would be with nothing painted in it. Asserted against the geometry the
/// pane frame produces, which is what an overlay may not move.
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

    // The content rect is the pane's own, unshrunk: an overlay takes no
    // layout space, so this is the rect `pane_frame` hands over, and no
    // smaller.
    let inset = chrome::pane_content_inset();
    let band = chrome::header_band_height();
    let mut expected = map.rect;
    expected.min.y += band;
    let expected = expected.shrink(inset);
    assert!(
        (map.body.width() - expected.width()).abs() < 0.5
            && (map.body.height() - expected.height()).abs() < 0.5,
        "the map pane's content rect is {:?} where the frame alone gives \
         {expected:?} — something took layout space out of it",
        map.body
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
