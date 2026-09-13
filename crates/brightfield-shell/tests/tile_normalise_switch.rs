//! **A grouped histogram carries a normalise control; a tile with no colour
//! group carries none**, and throwing it writes one key into the canonical
//! spec.
//!
//! The sibling of `tests/tile_scale_switch.rs`, reading the same way: the
//! control's geometry comes off [`ChartDoc::normalise_switches`], which the
//! chart pane writes as it draws, and the plot boxes it is held inside come off
//! [`MeridianApp::composed_plot_rects`] — two surfaces, so a control drawn
//! somewhere the plot is not fails here rather than needing an eye on a
//! screenshot.
//!
//! # What is covered and what is not
//!
//! Covered: which plots offer the control and which do not, where it sits
//! relative to the scale switch's own place, what it says on hover, what a
//! click writes into the spec, what the page then draws, and that a brush swept
//! on a sibling plot afterwards leaves the choice standing.
//!
//! Not covered here: the heights. `tests/grouped_histogram_shares.rs` is that
//! half — it reads the stack tops off the raster.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::layout::{StackOffset, STACK_OFFSET_KEY};

/// The authored grouped fixture, opened as a document — the shape the card's
/// claim is about, and the one no generated dashboard has: the generator gives
/// each tile a single column, so no tile it composes carries a colour group.
fn grouped_fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rect-bin-count-grouped-shares.yaml")
}

/// The committed table the housing window is opened over — the same fixture
/// `tests/tile_scale_switch.rs` uses.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

fn housing_boot() -> Boot {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

fn grouped_boot() -> Boot {
    let path = grouped_fixture();
    let spec = path.to_str().expect("utf-8 example path");
    Boot::open(spec, Flow::Vertical, None).expect("the example opens")
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a *previous* frame registered —
/// `tests/tile_scale_switch.rs`'s harness, over this file's own assertions.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    fn open(boot: Boot) -> Self {
        let size = boot.window_size();
        let ctx = egui::Context::default();
        Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx,
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        }
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    /// Three frames with no events — one more than the layout needs, for the
    /// reason `tests/arrangement.rs` runs three.
    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    fn doc(&self) -> &ChartDoc {
        self.app.chart_doc()
    }

    /// The normalise control the page drew for its grouped plot, panicking with
    /// what was drawn instead — a dropped control fails with a sentence.
    fn control(&self) -> brightfield_shell::app::NormaliseSwitchDrawn {
        let drawn = self.doc().normalise_switches.clone();
        drawn.first().cloned().unwrap_or_else(|| {
            panic!(
                "no normalise control; the page drew {} plots and {} scale switches",
                self.doc().composed.plots.len(),
                self.doc().scale_switches.len()
            )
        })
    }

    /// Press and release the primary button over `pos`, as five frames: egui
    /// resolves a click against the widget id the previous frame registered,
    /// and the canvas reads its own press edge across frames.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerMoved(pos), button(true)],
            vec![egui::Event::PointerMoved(pos), button(false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// A point `fraction` of the way across plot `plot`'s box, at its middle
    /// height — clear of the controls at its head, which sit in the top corner.
    fn at(&self, plot: usize, fraction: f32) -> egui::Pos2 {
        let rect = self.app.composed_plot_rects()[plot];
        egui::pos2(rect.left() + rect.width() * fraction, rect.center().y)
    }

    /// Sweep a brush across plot `plot` and release — the press, the move and
    /// the release are each a frame, because the canvas reads its own press
    /// edge across frames.
    fn brush(&mut self, plot: usize, from: f32, to: f32) {
        let start = self.at(plot, from);
        let end = self.at(plot, to);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerMoved(end), button(end, false)],
            Vec::new(),
            Vec::new(),
        ]);
    }

    /// Throw the control to `state` and let the page settle.
    fn switch_to(&mut self, state: StackOffset) {
        let at = self
            .control()
            .states
            .iter()
            .find(|(s, _)| *s == state)
            .unwrap_or_else(|| panic!("the control offers no {state:?}"))
            .1
            .center();
        self.click(at);
    }

    /// The plot attribute the live spec now carries on the plot at `path`.
    fn attribute(&self, path: &str, key: &str) -> Option<brightfield_spec::ast::SpecValue> {
        let spec = self
            .doc()
            .live_dashboard()
            .expect("a live session")
            .spec()
            .clone();
        brightfield_spec::layout::collect_plot_nodes(&spec)
            .into_iter()
            .find(|(p, _)| p == path)
            .and_then(|(_, node)| node.attributes.get(key).cloned())
    }
}

/// Every text the frame painted, with its box — `tests/tile_scale_switch.rs`'s
/// reader, narrowed to what this file asks of it.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().to_string(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------
// AC3 — offered on a grouped plot, in its box, at the switch's own place; and
// nowhere on a page whose tiles carry no group.
// ---------------------------------------------------------------------------

/// **The grouped plot carries the control, inside its own box.**
///
/// Containment is the assertion: the control's rect comes off the chart pane's
/// own record and the plot's off [`MeridianApp::composed_plot_rects`], which
/// resolves a plot's two possible origins independently.
#[test]
fn a_grouped_histogram_carries_a_normalise_control_inside_its_own_box() {
    let mut live = Live::open(grouped_boot());
    live.settle();

    let control = live.control();
    assert_eq!(
        live.doc().normalise_switches.len(),
        1,
        "the fixture has one grouped plot and one control, not {:?}",
        live.doc().normalise_switches
    );
    assert_eq!(control.group, "site", "it names the column it splits by");

    let box_of = live.app.composed_plot_rects()[control.plot];
    assert!(
        box_of.contains_rect(control.rect),
        "the control at {:?} is outside its plot's box {box_of:?}",
        control.rect
    );

    // The scatter beside it carries no stack, so it is offered nothing.
    assert!(
        live.doc()
            .normalise_switches
            .iter()
            .all(|c| c.plot == control.plot),
        "a plot with no group was offered a control"
    );
}

/// **It stands where the scale switch stands** — the same inset from the head
/// of its own plot's box, so the two are one row of chrome rather than two.
///
/// This is the only reading of *beside the scale switch* a document can give
/// today: a plot carries a scale switch when the generator named it a tile, and
/// the generator gives each tile one column, so no plot has both at once. The
/// offset from the plot's own top-right corner is what the two share, and it is
/// read here off two different documents.
#[test]
fn the_normalise_control_sits_at_the_scale_switch_s_own_inset() {
    let mut grouped = Live::open(grouped_boot());
    grouped.settle();
    let control = grouped.control();
    let grouped_box = grouped.app.composed_plot_rects()[control.plot];

    let mut housing = Live::open(housing_boot());
    housing.settle();
    let switch = housing
        .doc()
        .scale_switches
        .first()
        .cloned()
        .expect("the housing dashboard draws scale switches");
    let switch_box = housing.app.composed_plot_rects()[switch.plot];

    assert!(
        (control.rect.top() - grouped_box.top() - (switch.rect.top() - switch_box.top())).abs()
            < 0.5,
        "the control's inset from its plot's head is {} and the switch's is {}",
        control.rect.top() - grouped_box.top(),
        switch.rect.top() - switch_box.top()
    );
    assert!(
        (control.rect.height() - switch.rect.height()).abs() < 0.5,
        "the two controls are one height: {} and {}",
        control.rect.height(),
        switch.rect.height()
    );
    assert!(
        (grouped_box.right() - control.rect.right() - (switch_box.right() - switch.rect.right()))
            .abs()
            < 0.5,
        "with no switch beside it the control takes the switch's own trailing \
         inset: {} against {}",
        grouped_box.right() - control.rect.right(),
        switch_box.right() - switch.rect.right()
    );
}

/// **It says what it is and whose it is**, in words the frame actually painted.
#[test]
fn the_control_offers_its_two_readings_and_names_its_column() {
    let mut live = Live::open(grouped_boot());
    live.settle();
    let control = live.control();

    assert_eq!(
        control.hover, "normalise: site",
        "the hover names the control and the column the bins are split by"
    );
    assert_eq!(
        control.states.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        vec![StackOffset::None, StackOffset::Normalize],
        "two readings, counts first"
    );

    let painted = texts(&{
        let raw = egui::RawInput {
            screen_rect: Some(live.screen),
            ..Default::default()
        };
        live.ctx.run_ui(raw, |ui| live.app.draw(ui)).shapes
    });
    for word in ["count", "share"] {
        assert!(
            painted
                .iter()
                .any(|(t, r)| t == word && control.rect.expand(1.0).contains_rect(*r)),
            "the control's box holds no {word:?}; it painted {:?}",
            painted
                .iter()
                .filter(|(_, r)| control.rect.expand(1.0).contains_rect(*r))
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
        );
    }
}

/// **The housing dashboard draws none of these.** Its seven tiles each carry
/// one column and no colour group, so there is no stack to re-measure — and the
/// seven scale switches beside them are what says the page drew its chrome at
/// all, rather than this passing over a page that drew nothing.
#[test]
fn the_housing_dashboard_draws_no_normalise_control() {
    let mut live = Live::open(housing_boot());
    live.settle();
    assert_eq!(
        live.doc().scale_switches.len(),
        7,
        "fixture check: the seven histogram tiles drew their scale switches"
    );
    assert!(
        live.doc()
            .composed
            .plots
            .iter()
            .all(|p| p.group_column.is_none()),
        "fixture check: no tile on this page carries a colour group"
    );
    assert!(
        live.doc().normalise_switches.is_empty(),
        "it drew {:?}",
        live.doc().normalise_switches
    );
}

// ---------------------------------------------------------------------------
// AC4 — a click writes the plot's own spec, and the choice survives a brush.
// ---------------------------------------------------------------------------

/// **A click writes one key onto the clicked plot and leaves the rest alone.**
#[test]
fn a_click_writes_the_stack_offset_into_the_canonical_spec() {
    use brightfield_spec::ast::SpecValue;
    use brightfield_spec::layout::collect_plot_nodes;

    let mut live = Live::open(grouped_boot());
    live.settle();
    let control = live.control();
    let target = live.doc().composed.plots[control.plot].path.clone();

    // The fixture is authored WITH the offset, so the reading on screen is
    // shares and the click under test turns it off. The drawn record says so
    // before the click and the other way after it, which is the readback that
    // would not survive a control that reported the spec instead of the page.
    assert_eq!(control.active, StackOffset::Normalize);
    assert_eq!(
        live.attribute(&target, STACK_OFFSET_KEY),
        Some(SpecValue::String("normalize".to_string()))
    );

    let before = live
        .doc()
        .live_dashboard()
        .expect("a live session")
        .spec()
        .clone();
    live.switch_to(StackOffset::None);

    let after = live
        .doc()
        .live_dashboard()
        .expect("a live session")
        .spec()
        .clone();
    let was = collect_plot_nodes(&before);
    let now = collect_plot_nodes(&after);
    assert_eq!(was.len(), now.len(), "the edit is count-stable");

    let mut touched = Vec::new();
    for ((path_a, plot_a), (path_b, plot_b)) in was.iter().zip(now.iter()) {
        assert_eq!(path_a, path_b, "the plot order moved");
        assert_eq!(plot_a.items, plot_b.items, "{path_a}'s marks moved");
        if plot_a.attributes == plot_b.attributes {
            continue;
        }
        touched.push(path_a.clone());
        assert_eq!(
            path_a, &target,
            "the edit landed on {path_a} and not on the plot that was clicked"
        );
        assert_eq!(
            plot_b.attributes.len(),
            plot_a.attributes.len(),
            "the key was already there, so it was replaced and not added: \
             {:?} -> {:?}",
            plot_a.attributes,
            plot_b.attributes
        );
    }
    assert_eq!(
        touched,
        vec![target.clone()],
        "exactly one plot node changed"
    );
    assert_eq!(
        live.attribute(&target, STACK_OFFSET_KEY),
        Some(SpecValue::String("none".to_string())),
        "the plot's own spec carries the choice"
    );
    assert_eq!(
        live.control().active,
        StackOffset::None,
        "and the page redrew against it"
    );
}

/// **The choice survives a brush on a sibling plot.**
///
/// A brush rebuilds the page from the live spec, which is exactly where a
/// choice held in a view-local field rather than in the spec would be lost —
/// and it would be lost silently, because the control would still draw and
/// would simply read the other way.
#[test]
fn the_choice_survives_a_brush_on_a_sibling_plot() {
    use brightfield_spec::ast::SpecValue;

    let mut live = Live::open(grouped_boot());
    live.settle();
    let control = live.control();
    let target = live.doc().composed.plots[control.plot].path.clone();
    live.switch_to(StackOffset::None);
    assert_eq!(live.control().active, StackOffset::None);

    // The scatter is plot 0 and carries the interval brush. Sweep its middle.
    let scatter = live
        .doc()
        .composed
        .plots
        .iter()
        .position(|p| p.gesture.is_some())
        .expect("the fixture has a brushable sibling");
    assert_ne!(scatter, control.plot, "the brush is on the OTHER plot");
    live.brush(scatter, 0.2, 0.6);

    assert!(
        live.doc().composed.plots[scatter].committed_rect.is_some(),
        "fixture check: the sweep committed a selection on the sibling"
    );
    assert_eq!(
        live.attribute(&target, STACK_OFFSET_KEY),
        Some(SpecValue::String("none".to_string())),
        "the brush did not rewrite the plot's own key"
    );
    assert_eq!(
        live.control().active,
        StackOffset::None,
        "and the control still reads what was chosen"
    );
}
