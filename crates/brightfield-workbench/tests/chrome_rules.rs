//! The drawing rules, asserted without a GPU.
//!
//! `egui::Context` is CPU-only — it lays out, hit-tests and produces shapes
//! with no adapter anywhere near it — so everything a chrome rule is *about*
//! can be checked by running real frames and reading real rects back. Only
//! the final rasterisation needs hardware, and the rules here are not about
//! rasterisation.

use brightfield_keys::BindingContext;
use brightfield_workbench::chrome;
use brightfield_workbench::{
    HideAffordance, Icon, Mode, StatusEntry, StatusSide, Subject, Tone, ToolbarEntry,
    ToolbarLocation, Verb,
};
use meridian_design::{control, focus, spacing};

const PANE: egui::Rect = egui::Rect {
    min: egui::pos2(0.0, 0.0),
    max: egui::pos2(400.0, 300.0),
};

fn subject() -> Subject {
    Subject::new("Rows", Icon("list"), BindingContext::Workspace)
}

/// Run one frame over a fixed screen and hand back whatever the body
/// produced.
fn frame<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut out = None;
    let mut body = Some(body);
    let input = egui::RawInput {
        screen_rect: Some(PANE),
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        if let Some(body) = body.take() {
            out = Some(body(ui));
        }
    });
    out.expect("the frame body always runs")
}

/// Run one frame and **tessellate** it, so an assertion can be about the
/// triangles that would reach the GPU rather than about an intermediate that
/// something downstream might still discard.
fn frame_pixels<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> (R, Vec<egui::ClippedPrimitive>) {
    let ctx = egui::Context::default();
    let mut out = None;
    let mut body = Some(body);
    let input = egui::RawInput {
        screen_rect: Some(PANE),
        ..Default::default()
    };
    let full = ctx.run_ui(input, |ui| {
        if let Some(body) = body.take() {
            out = Some(body(ui));
        }
    });
    let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
    (out.expect("the frame body always runs"), primitives)
}

/// The bounding rect of every vertex painted in exactly `ink`, or `None` if
/// nothing in the frame was.
///
/// Painting in a colour nothing else uses is what makes "did this reach the
/// screen" answerable without a GPU: a clipped shape contributes no vertices
/// at all, so absence is the assertion.
fn painted(primitives: &[egui::ClippedPrimitive], ink: egui::Color32) -> Option<egui::Rect> {
    let mut found: Option<egui::Rect> = None;
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        for vertex in mesh.vertices.iter().filter(|v| v.color == ink) {
            let point = egui::Rect::from_min_max(vertex.pos, vertex.pos);
            found = Some(found.map_or(point, |r| r.union(point)));
        }
    }
    found
}

// ---------------------------------------------------------------------------
// A pane has no surface to draw a header on
// ---------------------------------------------------------------------------

/// Half the enforcement, measured: the `Ui` an item is handed *lays out*
/// below the header band.
///
/// On its own this proves less than it looks. `max_rect` only seeds the
/// `Placer`, so it constrains widgets an item adds and says nothing about
/// what it paints directly — see the clip test below, which is the other and
/// larger half.
#[test]
fn an_item_is_handed_a_ui_that_starts_below_the_header_band() {
    let content = frame(|ui| {
        let child = chrome::pane_frame(ui, &subject(), true, Mode::Light);
        child.max_rect()
    });
    let band = control::binding(spacing::ROW_GRID).row;
    assert!(
        content.top() >= PANE.top() + band,
        "content starts at {} but the header band ends at {}",
        content.top(),
        PANE.top() + band
    );
}

/// The half that `max_rect` does not give you: a pane that reaches for
/// `ui.painter()` and draws its own header band anyway gets clipped.
///
/// `Ui::new_child` clones the parent's painter and leaves its clip rect
/// alone, so before [`chrome::pane_frame`] called `shrink_clip_rect` the
/// child `Ui`'s clip rect was the whole pane and this paint landed — while
/// five doc comments said it could not. Delete the `shrink_clip_rect` call
/// and this test goes red.
///
/// What it does **not** claim: the clip does not reach `egui::Area` or
/// `ctx.layer_painter`, which take a fresh layer from the `Context`. Against
/// those the rule is a review rule, and both `pane_frame` and `Item::ui` now
/// say so.
#[test]
fn a_pane_that_paints_its_own_header_through_its_ui_is_clipped_away() {
    // Two colours nothing else in the frame uses, so presence and absence are
    // both unambiguous.
    const HEADER_INTRUDER: egui::Color32 = egui::Color32::from_rgb(255, 0, 255);
    const HONEST_CONTENT: egui::Color32 = egui::Color32::from_rgb(0, 255, 0);

    let band = control::binding(spacing::ROW_GRID).row;
    let ((), primitives) = frame_pixels(|ui| {
        let child = chrome::pane_frame(ui, &subject(), true, Mode::Light);
        // A pane drawing a header of its own, precisely over the shell's.
        child.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(4.0, 4.0), egui::pos2(160.0, 16.0)),
            0.0,
            HEADER_INTRUDER,
        );
        // …and something a pane is entitled to draw, so a frame that painted
        // nothing at all — or a clip that swallowed everything — cannot pass
        // this test by accident.
        child.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(40.0, 120.0), egui::pos2(160.0, 160.0)),
            0.0,
            HONEST_CONTENT,
        );
    });

    assert!(
        painted(&primitives, HONEST_CONTENT).is_some(),
        "the positive control never reached the frame, so the negative below proves nothing"
    );
    assert_eq!(
        painted(&primitives, HEADER_INTRUDER),
        None,
        "a pane painted into the header band (y 0..{band}) through its own Ui"
    );
}

/// The de-duplication rule's other half. A pane inside a tab strip already
/// has its name drawn directly above it, so it gets no band — and the space
/// the band would have taken goes back to the content rather than being left
/// blank.
#[test]
fn a_tabbed_pane_gets_no_header_band_and_keeps_the_space() {
    let (with_header, without) = frame(|ui| {
        let a = chrome::pane_frame(ui, &subject(), true, Mode::Light)
            .max_rect()
            .height();
        let b = chrome::pane_frame(ui, &subject(), false, Mode::Light)
            .max_rect()
            .height();
        (a, b)
    });
    let band = control::binding(spacing::ROW_GRID).row;
    assert!((without - with_header - band).abs() < 0.5);
}

/// The focus ring is painted inside the pane, so the pane frame reserves its
/// bleed. Without this the ring is drawn and then clipped away at exactly the
/// moment it is meant to be telling the user where they are.
///
/// Asserted as an equality against `PANEL_PADDING.max(RING_BLEED)` rather
/// than as `>= RING_BLEED`, which was theatre: the padding is 12 and the
/// bleed is 3, so `>= 3.0` holds however the inset is computed and the
/// assertion could not fail. This pins the inset exactly, so any change to
/// how it is derived — `.min` for `.max`, a different token, an arithmetic
/// slip — reddens it.
///
/// Stated plainly, because the reviewer asked for a mutation this test
/// catches and one of them it cannot: **deleting `.max(focus::RING_BLEED)`
/// alone is not detectable by any test**, at these token values. The max is a
/// no-op while `PANEL_PADDING` (12) exceeds `RING_BLEED` (3); the call
/// documents which of the two constraints is binding and keeps the frame
/// correct if the padding ever shrinks, and no assertion can distinguish it
/// from its own result today.
#[test]
fn the_pane_frame_reserves_room_for_a_focus_ring() {
    let content = frame(|ui| chrome::pane_frame(ui, &subject(), false, Mode::Light).max_rect());
    let inset = spacing::PANEL_PADDING.max(focus::RING_BLEED);
    for (edge, got) in [
        ("left", content.left() - PANE.left()),
        ("right", PANE.right() - content.right()),
        ("top", content.top() - PANE.top()),
        ("bottom", PANE.bottom() - content.bottom()),
    ] {
        assert!(
            (got - inset).abs() < 0.5,
            "the {edge} inset is {got}, not the reserved {inset}"
        );
    }
}

// ---------------------------------------------------------------------------
// One selection treatment, two entry points
// ---------------------------------------------------------------------------

/// [`chrome::selection_wash`] and [`chrome::selection_wash_shape`] paint the
/// same thing.
///
/// The shape-returning form exists for a caller that cannot paint the wash
/// before the content it sits under — an `egui::Grid` row, whose rect is not
/// known until its widest cell has been measured, reserves the wash with
/// `Painter::add` and fills it in with `Painter::set` afterwards. Two entry
/// points to one treatment is exactly how a product acquires a sixth spelling
/// of "this is selected", so this pins that they cannot diverge.
///
/// Watched redden: giving the shape form `radius::CONTROL` while the painting
/// form keeps `radius::CHIP` — a one-token difference no baseline at this
/// raster would show — fails here.
#[test]
fn both_ways_of_painting_the_selection_wash_agree() {
    let row = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 22.0));
    for mode in [Mode::Light, Mode::Dark] {
        // Compared as triangles rather than as `Shape`s: `Shape` is not `Eq`,
        // and the triangles are what actually reaches the GPU.
        let (_, direct) = frame_pixels(|ui| chrome::selection_wash(ui, row, mode));
        let (_, reserved) = frame_pixels(|ui| {
            ui.painter().add(chrome::selection_wash_shape(row, mode));
        });
        let vertices = |p: &[egui::ClippedPrimitive]| -> Vec<(egui::Pos2, egui::Color32)> {
            p.iter()
                .filter_map(|c| match &c.primitive {
                    egui::epaint::Primitive::Mesh(m) => Some(m),
                    egui::epaint::Primitive::Callback(_) => None,
                })
                .flat_map(|m| m.vertices.iter().map(|v| (v.pos, v.color)))
                .collect()
        };
        assert_eq!(
            vertices(&direct),
            vertices(&reserved),
            "{mode:?}: the two selection-wash entry points paint different pixels"
        );
    }
}

// ---------------------------------------------------------------------------
// The toolbar row
// ---------------------------------------------------------------------------

fn verb() -> Verb {
    Verb::new("reload-spec")
}

/// The point of [`ToolbarLocation::Hidden`]: the control stays declared — one
/// `Subject` still tells you the whole vocabulary of the surface — but the
/// row does not draw it and therefore does not reflow as it comes and goes.
#[test]
fn a_hidden_toolbar_entry_is_declared_but_never_drawn() {
    let entries = vec![
        ToolbarEntry::button("shown", "Reload", verb()),
        ToolbarEntry::button("withheld", "Not now", verb()).at(ToolbarLocation::Hidden),
        ToolbarEntry::button("later", "Overflow", verb()).at(ToolbarLocation::Overflow),
        ToolbarEntry::button("right", "Flow", verb()).at(ToolbarLocation::Trailing),
    ];
    let drawn = frame(|ui| chrome::toolbar_row(ui, &entries, Mode::Light).drawn);
    assert_eq!(drawn, vec!["shown", "right"]);
}

/// A toggle that is on *and* disabled must still read as unavailable.
///
/// The bug this pins: `add_enabled(false, …)` greys the button, and the
/// on-state overlay then painted the selection wash and re-painted the label
/// at full primary ink over the top — which is precisely what an *enabled*
/// toggle looks like. A pane could reach it from its own plain data with
/// `.toggle(true).enabled(false)`, so nothing stood between it and a control
/// that invites a click it will refuse.
#[test]
fn a_disabled_toggle_does_not_paint_itself_back_to_looking_enabled() {
    let sem = meridian_design::semantic(Mode::Light.is_dark());
    let wash = chrome::colour(sem.rows.selected_background);
    let live_ink = chrome::colour(sem.text.primary);
    let dead_ink = chrome::colour(sem.text.disabled);

    let row = |enabled: bool| {
        vec![ToolbarEntry::button("grid", "Grid", verb())
            .toggle(true)
            .enabled(enabled)]
    };

    let (_, live) = frame_pixels(|ui| chrome::toolbar_row(ui, &row(true), Mode::Light));
    assert!(
        painted(&live, wash).is_some(),
        "an enabled toggle wears the selection wash, so the absence below means something"
    );
    assert!(
        painted(&live, live_ink).is_some(),
        "an enabled toggle's label is primary ink"
    );

    let (_, dead) = frame_pixels(|ui| chrome::toolbar_row(ui, &row(false), Mode::Light));
    assert_eq!(
        painted(&dead, wash),
        None,
        "a disabled toggle painted the selection wash over its own greyed button"
    );
    assert_eq!(
        painted(&dead, live_ink),
        None,
        "a disabled toggle re-painted its label at full primary ink"
    );
    assert!(
        painted(&dead, dead_ink).is_some(),
        "a disabled toggle's label takes the disabled ink"
    );
}

#[test]
fn nothing_is_activated_without_a_click() {
    let entries = vec![ToolbarEntry::button("shown", "Reload", verb())];
    let out = frame(|ui| chrome::toolbar_row(ui, &entries, Mode::Light));
    assert!(out.activated.is_empty());
}

// ---------------------------------------------------------------------------
// The status rail
// ---------------------------------------------------------------------------

#[test]
fn the_status_rail_draws_both_sides() {
    let entries = vec![
        StatusEntry {
            id: "message",
            side: StatusSide::Leading,
            text: "Yanked".into(),
            tone: Tone::Good,
            hide: HideAffordance::Transient { ms: 1500 },
        },
        StatusEntry {
            id: "rows",
            side: StatusSide::Trailing,
            text: "40 rows".into(),
            tone: Tone::Neutral,
            hide: HideAffordance::WithRail,
        },
    ];
    let out = frame(|ui| chrome::status_rail(ui, &entries, Mode::Light));
    assert_eq!(out.drawn, vec!["message", "rows"]);
    assert!(out.dismissed.is_empty());
}

// ---------------------------------------------------------------------------
// The focus spike
// ---------------------------------------------------------------------------

/// **The question this step existed to answer.** `Subject` is meaningless
/// without a reliable answer to "which pane is focused", and `egui_tiles`
/// never reports focus, so the shell has to derive it. The candidate
/// mechanism is a pane-background rect sensed for clicks. The doubt was
/// whether a `ScrollArea` filling the pane swallows the click before the
/// background sees it.
///
/// Measured answer, and it is the one that makes the mechanism usable: the
/// background rect is interacted with *before* the scroll area is built, so
/// it is registered first and the scroll area sits above it — but egui's hit
/// test resolves to the background wherever the scroll area has no widget
/// under the pointer, because a `ScrollArea` that does not need to scroll
/// senses nothing. A click on empty space inside a scrolling pane therefore
/// focuses the pane.
///
/// The caveat, also measured below: a click that lands on a widget inside the
/// scroll area does *not* reach the background. That is correct behaviour —
/// the widget is what was clicked — which is why focus also has to be moved
/// by the pane-cycle verbs and by any pane that calls
/// `ItemCtx::take_focus()`, rather than by the background rect alone.
#[test]
fn clicking_a_panes_empty_background_focuses_it_even_under_a_scroll_area() {
    assert!(background_click_seen_at(egui::pos2(200.0, 250.0)));
}

#[test]
fn clicking_a_widget_inside_the_pane_does_not_also_hit_the_background() {
    // The button is at the very top-left of the scroll area's content.
    assert!(!background_click_seen_at(egui::pos2(20.0, 12.0)));
}

/// Draw a pane whose body is a `ScrollArea` with one widget in it, then click
/// at `click` on the next frame and report whether the pane background saw
/// the click.
fn background_click_seen_at(click: egui::Pos2) -> bool {
    let ctx = egui::Context::default();
    let mut saw = false;

    let draw = |input: egui::RawInput, saw: &mut bool| {
        let _ = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            let bg = ui.interact(rect, ui.id().with("pane-bg"), egui::Sense::click());
            if bg.clicked() {
                *saw = true;
            }
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            egui::ScrollArea::vertical().show(&mut child, |ui| {
                let _ = ui.button("a widget");
            });
        });
    };

    // Frame one registers the widget rects the next frame hit-tests against.
    draw(
        egui::RawInput {
            screen_rect: Some(PANE),
            ..Default::default()
        },
        &mut saw,
    );

    let modifiers = egui::Modifiers::default();
    draw(
        egui::RawInput {
            screen_rect: Some(PANE),
            events: vec![
                egui::Event::PointerMoved(click),
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                },
            ],
            ..Default::default()
        },
        &mut saw,
    );

    saw
}
