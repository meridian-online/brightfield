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

// ---------------------------------------------------------------------------
// A pane has no surface to draw a header on
// ---------------------------------------------------------------------------

/// The structural enforcement, measured: the `Ui` an item is handed starts
/// below the header band. This is why "items do not draw their own headers"
/// is a fact about the types rather than a rule in a style guide.
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
#[test]
fn the_pane_frame_reserves_room_for_a_focus_ring() {
    let content = frame(|ui| chrome::pane_frame(ui, &subject(), false, Mode::Light).max_rect());
    assert!(content.left() - PANE.left() >= focus::RING_BLEED);
    assert!(PANE.right() - content.right() >= focus::RING_BLEED);
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
