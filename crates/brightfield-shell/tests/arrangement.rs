//! The window's regions, as drawn, against the arrangement that declares them.
//!
//! Every assertion here reads a rect off a **real laid-out frame** rather than
//! the constant the draw path was handed. That distinction is the whole point:
//! a resizable `egui::Panel` reports its *content's* extent rather than its
//! declared one unless the content claims the space, and it persists the
//! narrower number to the next frame — so a rail can be declared at 240, drawn
//! at 200, and pass any test that compares the declaration with itself.
//!
//! `Panel::resizable`'s own doc says so ("If you want your panel to be
//! resizable you also need to make the ui use the available space") and nowhere
//! a person writing a layout would look. Measured on this window before the
//! navigator rail claimed its width: 240 declared, 200 reported by the second
//! frame, which is its floor.

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::{self, Occupant};
use brightfield_workbench::ViewKind;

/// A path relative to the workspace root.
fn fixture(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const DASHBOARD: &str = "../../examples/dashboard.yaml";

/// A boot carrying a protocol **and** a chart, so every region of the
/// arrangement has an occupant with something in it — a rail drawing an empty
/// state is still a rail, but a window where two of them are empty is a
/// thinner question than the one this file asks.
fn both() -> Boot {
    let spec = fixture("examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    Boot {
        view: Some(ViewKind::Charts),
        composed: compose_spec(DASHBOARD).expect("compose the dashboard"),
        live: None,
        spec_path: Some(DASHBOARD.into()),
        authored: None,
        protocol: inputs,
        flow: Flow::Vertical,
        focus: None,
    }
}

/// A settled window at the size the boot asks for.
fn settled() -> MeridianApp {
    let boot = both();
    let (w, h) = boot.window_size(ViewKind::Charts);
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    // Three frames, one more than the layout needs: the trap this file exists
    // for takes two frames to appear, because egui stores a resizable panel's
    // reported size and reads it back on the frame after.
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// The extent a region takes on the axis its edge runs across.
fn drawn_extent(id: arrangement::RegionId, rect: egui::Rect) -> f32 {
    let region = arrangement::default_arrangement().expect_region(id);
    match region.edge {
        arrangement::Edge::Left | arrangement::Edge::Right => rect.width(),
        arrangement::Edge::Top | arrangement::Edge::Bottom => rect.height(),
        arrangement::Edge::Centre => {
            panic!("{id} takes the remainder, so it has no extent of its own")
        }
    }
}

/// AC7. Every region the frame drew is the extent the arrangement declares.
///
/// The draw path reads the declaration rather than holding the numbers, so
/// this comparison is between what was declared and what reached the screen —
/// not between a constant and a copy of itself. Break the read (hand
/// `Panel::left` a literal instead of `rail_default(navigator)`) and the
/// navigator rail's line fails; change the declaration and
/// `each_rail_draws_at_its_declared_extent` fails instead, which is the pair
/// that proves the draw path is following the list.
///
/// Regions that take no space are excluded **by the declaration**, not by a
/// name written here: `Extent::Overlay` is the status rail, which floats and
/// takes nothing from its siblings, and `Extent::Remainder` is the canvas,
/// whose extent is whatever is left.
#[test]
fn the_drawn_regions_match_the_declared_arrangement() {
    let app = settled();
    let plan = arrangement::default_arrangement();
    let mut checked = 0_usize;

    for region in plan.regions {
        let Some(declared) = region.extent.default_extent() else {
            continue; // Extent::Remainder — the canvas
        };
        if !region.extent.takes_space() {
            continue; // Extent::Overlay — the status rail floats
        }
        let Some(rect) = app.region_rect(region.id) else {
            continue; // a region this window does not draw
        };
        let drawn = drawn_extent(region.id, rect);
        assert!(
            (drawn - declared).abs() < 1e-3,
            "{} drew at {drawn}pt against the {declared}pt the arrangement \
             declares — the draw path is not reading the declaration",
            region.id
        );
        checked += 1;
    }

    assert!(
        checked >= 5,
        "only {checked} regions were compared; a frame that drew almost \
         nothing would satisfy every assertion above"
    );
}

/// AC8. Each rail draws at its declared default and refuses to narrow past its
/// floor — measured from the drawn rect.
///
/// The numbers are spelled out here on purpose, and it is the one place they
/// are: this test and the arrangement are the two ends of the pair, so
/// changing an extent in the declaration reddens this and nothing else. That
/// is what proves the draw path is following the list rather than agreeing
/// with it by coincidence.
///
/// The floor is asked of a window narrow enough to force it: `Panel::min_size`
/// is what refuses, and a rail drawn on a roomy window never reaches it.
#[test]
fn each_rail_draws_at_its_declared_extent() {
    let app = settled();
    for (id, want) in [
        (arrangement::NAVIGATOR_RAIL, 240.0_f32),
        (arrangement::INSPECTOR_RAIL, 280.0),
        (arrangement::LEDGER_RAIL, 180.0),
    ] {
        let rect = app
            .region_rect(id)
            .unwrap_or_else(|| panic!("{id} did not draw"));
        let drawn = drawn_extent(id, rect);
        assert!(
            (drawn - want).abs() < 1e-3,
            "{id} drew at {drawn}pt, not the {want}pt this arrangement is"
        );
    }
}

/// AC8's other half: a rail whose content does not claim the space converges to
/// its floor, so the floor has to be a real number and the claim has to be
/// there.
///
/// Laid out on a window narrow and short enough that every rail is pressed
/// against its minimum. What is asserted is the floor, in the drawn rect: a
/// rail that ignored `min_size` would come back narrower, and one whose
/// closure did not claim the space would come back at whatever its content
/// happened to want.
#[test]
fn each_rail_refuses_to_narrow_past_its_declared_floor() {
    let boot = both();
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    // Tight enough that the rails cannot all have their defaults: 240 + 280
    // across is already 520 of these 620.
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(620.0, 480.0),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }

    for (id, floor) in [
        (arrangement::NAVIGATOR_RAIL, 160.0_f32),
        (arrangement::INSPECTOR_RAIL, 200.0),
        (arrangement::LEDGER_RAIL, 120.0),
    ] {
        let rect = app
            .region_rect(id)
            .unwrap_or_else(|| panic!("{id} did not draw"));
        let drawn = drawn_extent(id, rect);
        assert!(
            drawn >= floor - 1e-3,
            "{id} drew at {drawn}pt, below the {floor}pt floor it declares"
        );
    }
}

/// The declaration and the shipped item vocabulary agree.
///
/// `audit_arrangement` cannot run inside `brightfield-workbench`: the ids it
/// checks are published by the shell's registries, and a workbench test has no
/// registry to publish. This is where both halves exist, so a pane renamed in
/// `app.rs` or `protocol.rs` reddens here rather than leaving a region drawing
/// nothing.
#[test]
fn the_shipped_arrangement_names_only_panes_this_build_has() {
    brightfield_shell::app::publish_item_ids();
    brightfield_shell::protocol::publish_item_ids();
    arrangement::audit_arrangement(arrangement::default_arrangement())
        .unwrap_or_else(|findings| panic!("{findings}"));
}

/// AC2. The canvas toggle offers a grid and a chart, and nothing else.
///
/// Counted where it drew rather than where it was declared: the segments come
/// back off the frame, so a third entry added to the declaration would arrive
/// here as a third rect. The declaration's own count is held separately, in
/// `brightfield_workbench::arrangement`'s unit tests — this is the half that
/// says the screen agrees with it.
#[test]
fn the_canvas_toggle_offers_two_projections_and_no_more() {
    let app = settled();
    let segments = app.canvas_toggle_segments();
    assert_eq!(
        segments.len(),
        2,
        "the canvas toggle drew {} segments",
        segments.len()
    );
    // One control, not two: the segments abut, so they are halves of one
    // capsule rather than two separate controls that happen to be adjacent.
    assert!(
        (segments[0].right() - segments[1].left()).abs() < 1e-3,
        "the two segments are {}pt apart, so the canvas is offering two \
         controls where the arrangement declares one",
        segments[1].left() - segments[0].right()
    );

    let canvas = arrangement::default_arrangement().expect_region(arrangement::CANVAS);
    let Occupant::Canvas { projections, .. } = canvas.occupant else {
        panic!("the canvas region is occupied by the canvas");
    };
    let labels: Vec<&str> = projections.iter().map(|p| p.label).collect();
    assert_eq!(labels, ["Grid", "Chart"]);
}

/// AC4. No numeric binding addresses a surface.
///
/// Two claims, and both are enumerable from `brightfield_keys::registry()`:
///
/// - no binding is a modified digit — `cmd-1`, `alt-2`, `ctrl-3`. That is the
///   positional idiom this window rejects, and rejects because the surfaces it
///   would address are not a bounded set of peers: the protocol is the
///   container the chart sits inside, and a design workspace is coming.
/// - a bare digit may be bound, but not to a verb that moves focus. `0` is
///   bound to `reset-extent` — a gesture on the chart's own extent, which is
///   content rather than a surface — and its provenance row says so.
///
/// What is *not* asserted is that no digit exists anywhere: the front door's
/// own numerics are scoped to its entries in its own context, which is the
/// pattern Zed uses for the Welcome screen, and they are a different card's
/// work. Nothing in this build binds one yet, which this test also records.
#[test]
fn no_numeric_binding_addresses_a_surface() {
    use brightfield_keys::Drives;

    let mut modified_digits: Vec<String> = Vec::new();
    let mut navigating_digits: Vec<String> = Vec::new();

    for verb in brightfield_keys::registry() {
        for binding in &verb.binding_specs {
            for stroke in binding.keystrokes.split_whitespace() {
                let (modifiers, key) = stroke.rsplit_once('-').unwrap_or(("", stroke));
                let is_digit = key.len() == 1 && key.chars().all(|c| c.is_ascii_digit());
                if !is_digit {
                    continue;
                }
                if !modifiers.is_empty() {
                    modified_digits.push(format!("{} → {stroke}", verb.longname));
                }
                if verb.drives == Drives::Navigation {
                    navigating_digits.push(format!("{} → {stroke}", verb.longname));
                }
            }
        }
    }

    assert!(
        modified_digits.is_empty(),
        "a modified digit is bound: {modified_digits:?}. Numbers address \
         positions in a bounded set of peers, and this window's surfaces are \
         neither peers nor a closed set"
    );
    assert!(
        navigating_digits.is_empty(),
        "a digit is bound to a verb that moves focus: {navigating_digits:?}. \
         A digit may act on content — `0` resets the chart's extent — and may \
         not address a surface"
    );
}
