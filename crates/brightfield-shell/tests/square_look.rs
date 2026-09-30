//! The square look at this repo's own call sites, read out of the paint list.
//!
//! The design system draws the chrome square: a rule and a bar do what a
//! corner and a wash did. This file asks the surfaces the shell draws whether
//! they carry it, because a pinned revision can name the look and a call site
//! can still hand a radius of its own to a painter, or draw a ring outside the
//! edge it belongs to. The claims below are made of what a frame painted: a
//! `RectShape` says its rect, its corner, its stroke and where the stroke sits,
//! so "the ring lies inside its edge" is a claim about that shape and not about
//! pixels an antialiasing fringe would blur.
//!
//! WHICH SURFACES. The gallery's specimens of the status pill, the key, the
//! picker, the modal card and the focus ring, the toast the feedback specimen
//! raises, and the two overlays the window opens on a key: the command palette
//! and the help sheet. And the rows of the Protocol's spine and of the Outline,
//! drawn in the shipped window over a data file: what each draws at rest, under
//! the pointer, when picked, and when the canvas holds it.
//!
//! WHAT IS NOT CLAIMED. A round mark stays round (a dot, a slider's thumb), and
//! none of these surfaces draws one.
//!
//! CPU tessellation only, so it runs on a machine with no GPU, like
//! `chip_geometry.rs`.

use brightfield_shell::design::{to_color32, Mode};
use brightfield_shell::gallery::{catalog, solo, Component};
use brightfield_shell::protocol::{SpineRole, SpineRowDrawn};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use egui::epaint::{RectShape, StrokeKind};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use meridian_design::control::ROW_BAR_WIDTH;
use meridian_design::focus::RING_WIDTH;
use meridian_design::semantic;

const MODES: [Mode; 2] = [Mode::Light, Mode::Dark];

/// Every rect a frame painted, flattened out of the shape tree.
fn rect_shapes(shapes: &[egui::epaint::ClippedShape]) -> Vec<RectShape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<RectShape>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The rects in `rects` drawn with any corner.
fn cornered(rects: &[RectShape]) -> Vec<&RectShape> {
    rects
        .iter()
        .filter(|r| r.corner_radius != egui::CornerRadius::ZERO)
        .collect()
}

fn assert_square(rects: &[RectShape], what: &str) {
    let rounded = cornered(rects);
    assert!(
        rounded.is_empty(),
        "{what}: {} of {} rects carry a corner, the first {:?} at {:?}",
        rounded.len(),
        rects.len(),
        rounded[0].corner_radius,
        rounded[0].rect
    );
}

fn specimen(id: &str) -> Box<dyn Component> {
    catalog()
        .into_iter()
        .find(|c| c.info().id == id)
        .unwrap_or_else(|| panic!("no gallery component with id {id:?}"))
}

/// One gallery specimen rendered solo, in the composition the goldens
/// photograph.
fn solo_harness(id: &str, mode: Mode) -> Harness<'static> {
    let mut component = specimen(id);
    let (w, h) = component.info().solo_size;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(w, h))
        .build_ui(move |ui| solo(ui, mode, component.as_mut()));
    harness.run();
    harness
}

/// The status pill, the key, the picker (which is the palette's list over the
/// palette's delegate) and the modal card are drawn without a corner.
///
/// Each is named by its catalogue id and rendered on its own, so a specimen
/// that stopped drawing anything would fail the census before it could pass
/// for square.
#[test]
fn the_pill_the_key_the_picker_and_the_modal_card_are_drawn_without_a_corner() {
    for mode in MODES {
        for id in ["status-pill", "key-chip", "picker", "modal-chrome"] {
            let harness = solo_harness(id, mode);
            let rects = rect_shapes(&harness.output().shapes);
            assert!(
                rects.len() >= 2,
                "{id} {mode:?}: the specimen painted {} rects, which is not a \
                 specimen worth calling square",
                rects.len()
            );
            assert_square(&rects, &format!("{id} {mode:?}"));
        }
    }
}

/// The toast the feedback specimen raises is drawn without a corner.
///
/// It is raised the way a person raises it, by pressing the specimen's button,
/// and the test reads the frame after the toast's own label is on it, so a
/// toast that never appeared cannot pass for a square one.
#[test]
fn a_toast_is_drawn_without_a_corner() {
    for mode in MODES {
        let mut harness = solo_harness("feedback-layers", mode);
        assert!(
            harness.query_by_label("A confirmation").is_none(),
            "{mode:?}: the toast is on the frame before anything raised it"
        );
        let before = rect_shapes(&harness.output().shapes).len();
        harness.get_by_label("Pop toast").click();
        harness.run();
        harness.get_by_label("A confirmation");
        let rects = rect_shapes(&harness.output().shapes);
        assert!(
            rects.len() > before,
            "{mode:?}: raising the toast painted no rect ({before} before, {} after)",
            rects.len()
        );
        assert_square(&rects, &format!("toast {mode:?}"));
    }
}

/// A window under test, driven the way `overlay_wiring.rs` drives one: one
/// context for the window's life, real frames, keys fed as a person presses
/// them.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
}

impl Window {
    fn open(mode: Mode) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(Boot::empty(), default_layout(), mode),
            ctx: egui::Context::default(),
        };
        win.frame(Vec::new());
        win.frame(Vec::new());
        win
    }

    /// One frame, and the rects it painted.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<RectShape> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            events,
            ..Default::default()
        };
        let output = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        rect_shapes(&output.shapes)
    }

    /// Press `key`, then settle one frame; the rects of the settled frame.
    fn press(&mut self, key: egui::Key) -> Vec<RectShape> {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        self.frame(Vec::new())
    }
}

/// The command palette and the help sheet, opened by the keys that open them
/// in the window, are drawn without a corner.
///
/// The frame under the overlay is measured first, and the window shown to be
/// square before the overlay opens, so a corner cannot be blamed on the wrong
/// surface. The overlay must then have painted rects of its own.
#[test]
fn the_palette_and_the_help_sheet_are_drawn_without_a_corner() {
    for mode in MODES {
        for (key, overlay) in [
            (egui::Key::Space, "palette"),
            (egui::Key::Questionmark, "help"),
        ] {
            let mut win = Window::open(mode);
            let closed = win.frame(Vec::new());
            assert_eq!(win.app.open_overlay(), None);
            assert_square(&closed, &format!("the window under the {overlay} {mode:?}"));

            let open = win.press(key);
            assert_eq!(
                win.app.open_overlay(),
                Some(overlay),
                "{mode:?}: the key did not open the {overlay}"
            );
            assert!(
                open.len() > closed.len(),
                "{overlay} {mode:?}: opening it painted no rect of its own \
                 ({} closed, {} open)",
                closed.len(),
                open.len()
            );
            assert_square(&open, &format!("{overlay} {mode:?}"));
        }
    }
}

/// A focused control's ring is a two-pixel rule in the focus ink drawn inside
/// the control's own edge.
///
/// Read off the focus-ring specimen, which draws a standing exemplar (a
/// swatch with a ring) and a button that takes the ring when it holds the
/// keys. A ring is a rect stroked at the ring's width in the focus ink; it is
/// inside its edge when its stroke is laid inside the rect it was given, and
/// that rect is the control's own rect. Before the button is focused only the
/// exemplar's ring is on the frame, so a ring that appeared without focus, or
/// one that focus failed to draw, both fail here.
#[test]
fn a_focused_controls_ring_is_inside_its_edge() {
    for mode in MODES {
        let focus_ink = to_color32(semantic(mode.is_dark()).borders.focus);
        let raised = to_color32(semantic(mode.is_dark()).surfaces.raised);
        let rings = |harness: &Harness<'_>| -> Vec<RectShape> {
            rect_shapes(&harness.output().shapes)
                .into_iter()
                .filter(|r| r.stroke.width == RING_WIDTH && r.stroke.color == focus_ink)
                .collect()
        };

        let mut harness = solo_harness("focus-ring", mode);
        let standing = rings(&harness);
        assert_eq!(
            standing.len(),
            1,
            "{mode:?}: the specimen's standing exemplar draws one ring before \
             anything holds the keys"
        );

        harness.get_by_label("Focus me").focus();
        harness.run();
        let button = harness.get_by_label("Focus me").rect();
        let ringed = rings(&harness);
        assert_eq!(
            ringed.len(),
            2,
            "{mode:?}: focusing the button did not add its ring to the exemplar's"
        );

        for ring in &ringed {
            assert_eq!(
                ring.stroke_kind,
                StrokeKind::Inside,
                "{mode:?}: the ring at {:?} is stroked {:?}, so part of it lies \
                 outside the edge it belongs to",
                ring.rect,
                ring.stroke_kind
            );
            assert_eq!(ring.corner_radius, egui::CornerRadius::ZERO, "{mode:?}");
        }

        // The ring is on the control's own rect and not on a larger one.
        assert!(
            ringed
                .iter()
                .any(|r| (r.rect.min - button.min).abs().max_elem() < 0.5
                    && (r.rect.max - button.max).abs().max_elem() < 0.5),
            "{mode:?}: no ring is drawn on the button's rect {button:?}: {:?}",
            ringed.iter().map(|r| r.rect).collect::<Vec<_>>()
        );
        let swatch: Vec<egui::Rect> = rect_shapes(&harness.output().shapes)
            .iter()
            .filter(|r| r.fill == raised && r.stroke.width == 0.0)
            .map(|r| r.rect)
            .collect();
        assert!(
            swatch.iter().any(|s| ringed.iter().any(|r| r.rect == *s)),
            "{mode:?}: the exemplar's ring is not on the swatch it rings: \
             swatches {swatch:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The rows of the Protocol's spine and of the Outline.
// ---------------------------------------------------------------------------

/// The committed table the rail's rows are drawn over: nine columns, so the
/// Outline lists them and the spine lists a file, a table and a dashboard.
fn housing_boot() -> Boot {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv");
    Boot::data_file(path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// A window over [`housing_boot`] that keeps one context for its life, because
/// a pointer is resolved against the widgets the frame before it registered.
struct Rail {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    mode: Mode,
}

impl Rail {
    fn open(mode: Mode) -> Self {
        let boot = housing_boot();
        let size = boot.window_size();
        let mut rail = Self {
            app: MeridianApp::headless(boot, mode),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
            mode,
        };
        for _ in 0..3 {
            rail.frame(Vec::new());
        }
        rail
    }

    /// One frame with `events`, and every shape it painted.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    /// The pointer nowhere; the settled frame's rects.
    fn at_rest(&mut self) -> Vec<RectShape> {
        self.frame(vec![egui::Event::PointerGone]);
        rect_shapes(&self.frame(Vec::new()))
    }

    /// The pointer on `at`, two frames later; the settled frame's rects.
    fn hover(&mut self, at: egui::Pos2) -> Vec<RectShape> {
        self.frame(vec![egui::Event::PointerMoved(at)]);
        rect_shapes(&self.frame(Vec::new()))
    }

    /// A click on `at` and the pointer taken away, so the row that was picked
    /// is not also the row under the pointer.
    fn click_then_leave(&mut self, at: egui::Pos2) -> Vec<RectShape> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
        self.at_rest()
    }

    fn rows(&self) -> Vec<SpineRowDrawn> {
        self.app.spine_rows().to_vec()
    }

    fn row(&self, label: &str) -> SpineRowDrawn {
        self.rows()
            .into_iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("the rail drew no row labelled {label:?}"))
    }

    fn ink(&self) -> &'static semantic::Semantic {
        semantic(self.mode.is_dark())
    }
}

fn same_rect(a: egui::Rect, b: egui::Rect) -> bool {
    (a.min - b.min).abs().max_elem() < 0.01 && (a.max - b.max).abs().max_elem() < 0.01
}

/// The fills painted over exactly `rect` — that rect and no panel behind it.
fn fills_over(rects: &[RectShape], rect: egui::Rect) -> Vec<egui::Color32> {
    rects
        .iter()
        .filter(|r| r.fill != egui::Color32::TRANSPARENT && same_rect(r.rect, rect))
        .map(|r| r.fill)
        .collect()
}

/// The bar a row draws on its leading edge: the design system's row-bar width
/// and the row's own height.
fn bar_of(row: &SpineRowDrawn) -> egui::Rect {
    egui::Rect::from_min_size(row.rect.min, egui::vec2(ROW_BAR_WIDTH, row.rect.height()))
}

/// **A row of the spine and of the Outline draws no fill at rest and the rows
/// token's hover fill under the pointer** — the spine's asset rows and the
/// Outline's column rows, in both modes.
///
/// A row that senses no click, a step, takes no hover fill: the spine draws
/// it as a readout, and a fill under the pointer would say a click does
/// something.
#[test]
fn a_row_of_the_spine_and_the_outline_is_flat_at_rest_and_fills_under_the_pointer() {
    for mode in MODES {
        let mut rail = Rail::open(mode);
        let hover = to_color32(rail.ink().rows.hover_background);
        let rest = rail.at_rest();
        let rows = rail.rows();

        let acts: Vec<&SpineRowDrawn> = rows
            .iter()
            .filter(|r| r.control && r.on_canvas.is_none())
            .collect();
        let asset = acts
            .iter()
            .find(|r| r.role == SpineRole::Asset)
            .unwrap_or_else(|| panic!("{mode:?}: no asset row that acts and is off the canvas"));
        let column = acts
            .iter()
            .find(|r| r.role == SpineRole::Column)
            .unwrap_or_else(|| panic!("{mode:?}: the Outline drew no column row"));
        for row in &rows {
            assert!(
                fills_over(&rest, row.rect).is_empty(),
                "{mode:?}: {:?} ({:?}) drew a fill at rest: {:?}",
                row.label,
                row.role,
                fills_over(&rest, row.rect)
            );
        }

        for target in [asset, column] {
            let under = rail.hover(target.rect.center());
            assert_eq!(
                fills_over(&under, target.rect),
                vec![hover],
                "{mode:?}: {:?} ({:?}) under the pointer",
                target.label,
                target.role
            );
            assert!(
                !under.iter().any(|r| same_rect(r.rect, bar_of(target))),
                "{mode:?}: {:?} drew a bar for a pointer, which is not the cursor",
                target.label
            );
            for other in rows.iter().filter(|r| r.rect != target.rect) {
                assert!(
                    fills_over(&under, other.rect).is_empty(),
                    "{mode:?}: {:?} drew a fill with the pointer on {:?}",
                    other.label,
                    target.label
                );
            }
        }

        let step = rows
            .iter()
            .find(|r| r.role == SpineRole::Step)
            .unwrap_or_else(|| panic!("{mode:?}: the spine drew no step row"));
        assert!(!step.control, "{mode:?}: the step row senses a click");
        let under = rail.hover(step.rect.center());
        assert!(
            fills_over(&under, step.rect).is_empty(),
            "{mode:?}: a step row, which no click acts on, drew a hover fill"
        );
    }
}

/// **A picked row draws the cursor fill and a 3px bar in the focus ink on its
/// leading edge, with no outline and no corner** — an asset of the spine and a
/// column of the Outline, each picked by a click.
#[test]
fn a_picked_row_draws_the_cursor_fill_and_a_three_wide_bar_in_the_focus_ink() {
    for mode in MODES {
        let mut rail = Rail::open(mode);
        rail.at_rest();
        let cursor = to_color32(rail.ink().rows.cursor_background);
        let focus = to_color32(rail.ink().borders.focus);
        let asset = rail
            .rows()
            .into_iter()
            .find(|r| r.role == SpineRole::Asset && r.control && r.on_canvas.is_none())
            .expect("an asset row that acts and is off the canvas");
        for label in [asset.label.as_str(), "house_age"] {
            let mut rail = Rail::open(mode);
            let at = rail.row(label).rect.center();
            let rects = rail.click_then_leave(at);
            let row = rail.row(label);
            assert!(row.washed, "{mode:?}: a click on {label:?} did not pick it");
            assert_eq!(
                fills_over(&rects, row.rect),
                vec![cursor],
                "{mode:?}: the picked row {label:?} is filled with the cursor fill and nothing else"
            );
            let bar = bar_of(&row);
            assert!(
                (bar.width() - 3.0).abs() < 0.01,
                "{mode:?}: the bar is {} wide",
                bar.width()
            );
            assert_eq!(
                fills_over(&rects, bar),
                vec![focus],
                "{mode:?}: the picked row {label:?} carries a 3px bar in the focus ink"
            );
            let on_row: Vec<&RectShape> = rects
                .iter()
                .filter(|r| same_rect(r.rect, row.rect))
                .collect();
            assert!(
                on_row
                    .iter()
                    .all(|r| r.stroke.is_empty() && r.corner_radius == egui::CornerRadius::ZERO),
                "{mode:?}: the picked row {label:?} draws an outline or a corner: {on_row:?}"
            );
        }
    }
}

/// **The spine row the canvas holds carries a 3px bar in the secondary ink and
/// no fill**, and so does the head row while the canvas holds the graph — the
/// other place the bar is drawn.
#[test]
fn the_row_the_canvas_holds_carries_a_three_wide_bar_in_the_secondary_ink_and_no_fill() {
    for mode in MODES {
        let mut rail = Rail::open(mode);
        let secondary = to_color32(rail.ink().text.secondary);
        let focus = to_color32(rail.ink().borders.focus);
        let rects = rail.at_rest();
        let dashboard = rail.row("dashboard");
        let bar = dashboard.on_canvas.expect("the canvas holds the dashboard");
        assert!(
            same_rect(bar, bar_of(&dashboard)),
            "{mode:?}: the bar is {bar:?}"
        );
        assert!(
            (bar.width() - 3.0).abs() < 0.01,
            "{mode:?}: the bar is {} wide",
            bar.width()
        );
        assert_eq!(
            fills_over(&rects, bar),
            vec![secondary],
            "{mode:?}: the row the canvas holds draws its bar in the secondary ink"
        );
        assert!(
            fills_over(&rects, dashboard.rect).is_empty(),
            "{mode:?}: the row the canvas holds draws a fill at rest"
        );
        assert!(
            !fills_over(&rects, bar).contains(&focus),
            "{mode:?}: the bar is the picked ink"
        );

        let chip = rail
            .rows()
            .first()
            .and_then(|head| head.chip.as_ref())
            .expect("the spine's head row draws the graph chip")
            .rect;
        let rects = rail.click_then_leave(chip.center());
        let head = rail.rows().first().cloned().expect("the head row");
        let bar = head
            .on_canvas
            .expect("the canvas holds the graph, so the head carries the bar");
        assert!(
            (bar.width() - 3.0).abs() < 0.01,
            "{mode:?}: the head's bar is {} wide",
            bar.width()
        );
        assert_eq!(
            fills_over(&rects, bar),
            vec![secondary],
            "{mode:?}: the head row draws its bar in the secondary ink"
        );
    }
}
