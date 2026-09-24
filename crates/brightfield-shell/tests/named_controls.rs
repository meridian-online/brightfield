//! **Each control on the first screen carries a name a stranger reads** — in
//! its label, or in the hover text over it.
//!
//! The list is [`MeridianApp::named_controls`], which the window assembles
//! from the records its drawing returns. It is held here against a second,
//! independent reading of the same frame: the widgets egui registered in the
//! pass as sensing a click. The two are held against each other both ways,
//! and each of three failures reddens `assert_every_control_is_named`:
//!
//! - a click-sensing widget no entry covers is a control drawn without a name;
//! - an entry with an empty name is a control registered without one;
//! - an entry no click-sensing widget sits under names a control this frame
//!   did not draw — a record left standing from an earlier frame, or one taken
//!   at a rect the pane clipped away. Without this direction a stale entry
//!   already covering a spot would clear an unnamed control drawn there later.
//!
//! # The line between a control and a surface
//!
//! A widget counts as a control when it is enabled, senses a click, does not
//! sense a drag, and its interact rect has area. Each clause excludes
//! something on this window that is not a control:
//!
//! - **drag**: a selectable label senses click and drag (egui's
//!   `Label::layout_in_ui`, with `selectable_labels` left on and no touch
//!   input in this harness); so do the rails' resize handles, a shown
//!   scrollbar, the grid's column-resize lines and both rasters. The rasters'
//!   own hit-tested marks are covered by the list anyway — the graph's view
//!   chips are entries, read off the record the raster leaves.
//! - **enabled**: a greyed control senses nothing. The list still carries it
//!   by its label, because greyed words still read.
//! - **area**: a tile's scale switch is interacted over its segment clipped to
//!   the pane, and on the housing baseline the column tiles sit outside the
//!   hero's pane, so their switches register with an empty interact rect that
//!   no pointer can land in. The list names no such state —
//!   `a_tile_switch_state_the_pane_clips_away_names_no_control`.
//!
//! # What an entry has to sit over
//!
//! The reverse direction reads a wider pool: every widget that senses a click
//! and has area, enabled or not and dragged or not. A greyed control is still
//! drawn, and the status rail's dismissable line is a selectable label that
//! senses a drag and is rightly in the list. Hover is no evidence at all:
//! egui registers a hover-only widget over every `Ui`'s whole rect, so a rule
//! counting hover would pass over any rect inside the window.
//!
//! An entry has to contain such a widget, with the forward rule's half point
//! of slack, rather than merely touch one — both rasters sense a click, and a
//! rule satisfied by any widget underneath would be satisfied by the picture.
//! The one exception is the graph's view chips: the DAG raster hit-tests them
//! itself and registers no widget of their own, so a chip entry is held to
//! sitting inside a click-sensing widget, the raster, instead.
//!
//! The front door is not the first screen this file reads: it draws before a
//! file is open, and its cards and rows are its own tests' subject.

use brightfield_shell::app::{GridLayout, GridSpot};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::GRAPH_CHIP_HINT;
use brightfield_shell::window::{Boot, MeridianApp, HOME_CONTROL_NAME};
use brightfield_workbench::arrangement::{INSPECTOR_RAIL, LEDGER_RAIL, NAVIGATOR_RAIL};
use brightfield_workbench::chrome::{NamedBy, NamedControl};

/// How many empty frames a hovered pointer is held still before the frame
/// that is read — `tests/tile_scale_switch.rs`'s count, for its reason: egui
/// refuses a tooltip while its pointer history still holds the jump the
/// pointer arrived by.
const STILL_FRAMES: usize = 8;

fn housing_boot() -> Boot {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv");
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// A window, the context it draws in and the screen it is drawn at —
/// `tests/navigator_spine.rs`'s `Live`, kept separate as the repo's readers
/// are.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    /// The housing file at the size its boot asks for — the dashboard
    /// baseline's window — settled.
    fn housing() -> Self {
        let boot = housing_boot();
        let size = boot.window_size();
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        };
        live.settle();
        live
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

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    /// Click the graph chip where the last frame drew it, and settle.
    fn click_graph_chip(&mut self) {
        let at = self.graph_chip().rect.center();
        self.run(vec![click_at(at), Vec::new(), Vec::new()]);
    }

    fn graph_chip(&self) -> brightfield_shell::protocol::GraphChipDrawn {
        self.app
            .spine_rows()
            .first()
            .and_then(|head| head.chip)
            .expect("the spine's head row drew the graph chip")
    }

    /// The frame drawn with the pointer resting on `pos`, as painted shapes.
    ///
    /// The tooltip delay is zeroed here rather than at construction, because
    /// the window installs the design system's whole `Style` on its first
    /// draw — `tests/tile_scale_switch.rs`'s `hover_shapes`, whose stillness
    /// assertion is kept so a refused tooltip is not read as a missing one.
    fn hover_shapes(&mut self, pos: egui::Pos2) -> Vec<egui::epaint::ClippedShape> {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            self.ctx
                .style_mut_of(theme, |style| style.interaction.tooltip_delay = 0.0);
        }
        self.run(vec![vec![egui::Event::PointerMoved(pos)]]);
        self.run(vec![Vec::new(); STILL_FRAMES]);
        assert!(
            self.ctx.input(|i| i.pointer.is_still()),
            "the pointer is still moving by egui's reading after {STILL_FRAMES} \
             frames at rest on {pos:?}"
        );
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    /// The widgets the last pass registered as controls, by the rule in the
    /// module docs, each as its id and its interact rect in screen space.
    fn click_widgets(&self) -> Vec<(egui::Id, egui::Rect)> {
        self.widgets(|w| {
            w.enabled
                && w.sense.senses_click()
                && !w.sense.senses_drag()
                && w.interact_rect.is_positive()
        })
    }

    /// The widgets an entry may sit over, by the wider rule in the module
    /// docs: each that senses a click and has area, disabled and dragged ones
    /// kept.
    fn click_sensing_widgets(&self) -> Vec<(egui::Id, egui::Rect)> {
        self.widgets(|w| w.sense.senses_click() && w.interact_rect.is_positive())
    }

    /// The widgets the last pass registered that `keep` keeps, each as its id
    /// and its interact rect in screen space.
    fn widgets(&self, keep: impl Fn(&egui::WidgetRect) -> bool) -> Vec<(egui::Id, egui::Rect)> {
        let local: Vec<(egui::LayerId, egui::Id, egui::Rect)> = self.ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, rects)| rects.iter())
                .filter(|w| keep(w))
                .map(|w| (w.layer_id, w.id, w.interact_rect))
                .collect()
        });
        local
            .into_iter()
            .map(|(layer, id, rect)| {
                let to_screen = self
                    .ctx
                    .layer_transform_to_global(layer)
                    .unwrap_or_default();
                (id, to_screen * rect)
            })
            .collect()
    }

    /// The entry named `name` — panics listing the names that were drawn.
    fn named(&self, name: &str) -> NamedControl {
        let controls = self.app.named_controls();
        controls
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .unwrap_or_else(|| {
                let names: Vec<&str> = controls.iter().map(|c| c.name.as_str()).collect();
                panic!("no control is named {name:?}; the list names {names:?}")
            })
    }
}

/// One frame's worth of a pointer move and a primary click at `pos`.
fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    events
}

/// The text of each galley the frame painted, tooltips included.
fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
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

/// **The two halves agree**: each click-sensing widget the pass registered is
/// inside an entry's rect, each entry's name has words in it, and each entry
/// has area and a click-sensing widget inside it — or, for a graph view chip,
/// around it.
///
/// Half a point of slack on each containment, because a button's widget rect
/// and the rect its caller placed it at are rounded separately.
fn assert_every_control_is_named(live: &Live, state: &str) {
    let controls = live.app.named_controls();
    assert!(
        !controls.is_empty(),
        "{state}: the window named no controls at all"
    );
    for control in controls {
        assert!(
            !control.name.trim().is_empty(),
            "{state}: the control at {:?} is registered with no name",
            control.rect
        );
    }
    let widgets = live.click_widgets();
    assert!(
        !widgets.is_empty(),
        "{state}: the pass registered no click-sensing widget, so this read \
         nothing to hold the list against"
    );
    for (id, rect) in widgets {
        assert!(
            controls
                .iter()
                .any(|c| c.rect.expand(0.5).contains_rect(rect)),
            "{state}: the widget {id:?} senses a click at {rect:?} and no named \
             control covers it; the list names {:?}",
            controls
                .iter()
                .map(|c| (c.name.as_str(), c.rect))
                .collect::<Vec<_>>()
        );
    }

    let under = live.click_sensing_widgets();
    let chips = live.app.canvas_chips();
    for control in controls {
        let (name, rect) = (control.name.as_str(), control.rect);
        assert!(
            rect.is_positive(),
            "{state}: the control named {name:?} is recorded at {rect:?}, which \
             has no area a pointer can land in"
        );
        if under
            .iter()
            .any(|(_, widget)| rect.expand(0.5).contains_rect(*widget))
        {
            continue;
        }
        let chip = chips
            .iter()
            .any(|chip| chip.rect == rect && chip.view.label() == name);
        let in_raster = under
            .iter()
            .any(|(_, widget)| widget.expand(0.5).contains_rect(rect));
        assert!(
            chip && in_raster,
            "{state}: the control named {name:?} is recorded at {rect:?} and no \
             click-sensing widget sits under it{}; the widgets that sense a \
             click are {:?}",
            if chip {
                ", nor is this view chip inside a raster that senses one"
            } else {
                ""
            },
            under.iter().map(|(_, r)| *r).collect::<Vec<_>>()
        );
    }
}

#[test]
fn every_control_on_the_housing_baseline_carries_a_name() {
    let live = Live::housing();
    assert_every_control_is_named(&live, "the housing baseline");
}

#[test]
fn the_list_names_the_first_screens_controls_by_name() {
    let mut live = Live::housing();

    let home = live.named(HOME_CONTROL_NAME);
    assert_eq!(home.by, NamedBy::Hover, "Home draws a mark and no words");
    assert_eq!(Some(home.rect), live.app.home_rect());

    // The carets draw no words, so each is named by its hover text — and the
    // entry has to be the one at that rail's caret, not a stray string.
    for (rail, name) in [
        (NAVIGATOR_RAIL, "Hide the navigator"),
        (INSPECTOR_RAIL, "Show the inspector"),
        (LEDGER_RAIL, "Show the ledger"),
    ] {
        let caret = live.named(name);
        assert_eq!(caret.by, NamedBy::Hover, "{rail}'s caret draws no words");
        assert_eq!(
            Some(caret.rect),
            live.app.rail_collapse_rect(rail),
            "the control named {name:?} is not {rail}'s caret"
        );
    }

    for (i, name) in ["Log", "Quality", "Rows", "Editor"].into_iter().enumerate() {
        let entry = live.named(name);
        assert_eq!(entry.by, NamedBy::Label);
        assert_eq!(
            Some(entry.rect),
            live.app.rail_name_rect(LEDGER_RAIL, i),
            "the control named {name:?} is not the ledger strip's name {i}"
        );
    }

    let chip = live.named(GRAPH_CHIP_HINT);
    assert_eq!(
        chip.by,
        NamedBy::Hover,
        "the chip's one word does not say which way it switches"
    );
    assert_eq!(chip.rect, live.graph_chip().rect);

    for label in ["dashboard", "grid"] {
        let row = live
            .app
            .spine_rows()
            .iter()
            .find(|row| row.label == label)
            .cloned()
            .unwrap_or_else(|| panic!("the spine drew no {label:?} row"));
        let entry = live.named(label);
        assert_eq!(entry.by, NamedBy::Label);
        assert_eq!(
            entry.rect, row.rect,
            "the control named {label:?} is not its spine row"
        );
    }

    // The flow toggle is drawn only once the graph holds the canvas.
    assert!(
        !live
            .app
            .named_controls()
            .iter()
            .any(|c| c.name.starts_with("flow:")),
        "the flow toggle is named on a frame whose canvas does not hold the graph"
    );
    live.click_graph_chip();
    assert!(
        live.app.graph_on_canvas(),
        "the chip did not put the graph on the canvas"
    );
    let toggle = live.named("flow: vertical ⇄");
    assert_eq!(toggle.by, NamedBy::Label);
}

#[test]
fn hovering_the_graph_chip_and_the_inspectors_caret_paints_their_names() {
    let mut live = Live::housing();
    let chip = live.graph_chip().rect.center();
    let caret = live
        .app
        .rail_collapse_rect(INSPECTOR_RAIL)
        .expect("the inspector drew its caret")
        .center();
    // A frame with the pointer on neither, so a name found below was painted
    // by the hover and not by something else on the screen.
    let away = live.hover_shapes(egui::pos2(live.screen.center().x, 2.0));
    let away = painted_texts(&away);

    for (pos, name, what) in [
        (chip, GRAPH_CHIP_HINT, "the graph chip"),
        (caret, "Show the inspector", "the inspector's caret"),
    ] {
        assert!(
            !away.iter().any(|t| t == name),
            "{name:?} is painted with the pointer away from {what}"
        );
        let texts = painted_texts(&live.hover_shapes(pos));
        assert!(
            texts.iter().any(|t| t == name),
            "hovering {what} at {pos:?} painted no {name:?}; it painted {texts:?}"
        );
    }
}

#[test]
fn with_the_graph_on_the_canvas_every_control_still_carries_a_name() {
    let mut live = Live::housing();
    live.click_graph_chip();
    assert!(
        live.app.graph_on_canvas(),
        "the chip did not put the graph on the canvas"
    );

    // The grid pane is not drawn on this branch, so neither is its layout
    // switch, however recently it was.
    for layout in [GridLayout::Rows, GridLayout::Columns] {
        let word = layout.word();
        let stale: Vec<_> = live
            .app
            .named_controls()
            .iter()
            .filter(|c| c.name == word)
            .map(|c| c.rect)
            .collect();
        assert!(
            stale.is_empty(),
            "with the graph on the canvas the list names the grid's {word:?} \
             layout state at {stale:?}, where no grid pane is drawn"
        );
    }

    let chips = live.app.canvas_chips().to_vec();
    assert!(!chips.is_empty(), "the graph drew no view chip on any node");
    for chip in &chips {
        let entry = live
            .app
            .named_controls()
            .iter()
            .find(|c| c.rect == chip.rect)
            .cloned()
            .unwrap_or_else(|| panic!("the view chip at {:?} is not in the list", chip.rect));
        assert_eq!(entry.name, chip.view.label());
        assert_eq!(entry.by, NamedBy::Label);
    }
    assert_every_control_is_named(&live, "the graph on the canvas");
}

/// **A tile switch's state the pane clips to nothing names no control.** A
/// state is interacted over its rect's intersection with the pane's clip,
/// so when that leaves nothing, no pointer can land on the state and an
/// entry for it — at the painted rect or at the empty one — names a control
/// that is not there.
///
/// On the housing baseline the column tiles sit outside the hero's pane, so
/// every switch they draw is such a state; the guard below says so rather
/// than passing over a page that clipped nothing. A state the clip leaves
/// some of is held by `assert_every_control_is_named` both ways instead.
/// Normalise controls are read the same way, though the housing dashboard
/// composes no grouped histogram and so draws none.
#[test]
fn a_tile_switch_state_the_pane_clips_away_names_no_control() {
    let live = Live::housing();
    let doc = live.app.chart_doc();
    let clipped: Vec<(egui::Rect, egui::Rect)> = doc
        .scale_switches
        .iter()
        .flat_map(|s| {
            s.states
                .iter()
                .map(|(_, seg)| (*seg, seg.intersect(s.clip)))
        })
        .chain(doc.normalise_switches.iter().flat_map(|s| {
            s.states
                .iter()
                .map(|(_, seg)| (*seg, seg.intersect(s.clip)))
        }))
        .filter(|(_, hit)| !hit.is_positive())
        .collect();
    assert!(
        !clipped.is_empty(),
        "no tile switch state on the housing baseline is clipped to nothing, so \
         this reads nothing"
    );
    let controls = live.app.named_controls();
    for (seg, hit) in clipped {
        let named: Vec<_> = controls
            .iter()
            .filter(|c| c.rect == seg || c.rect == hit)
            .map(|c| (c.name.as_str(), c.rect))
            .collect();
        assert!(
            named.is_empty(),
            "the tile switch state painted at {seg:?} is clipped to nothing and \
             the list still names it: {named:?}"
        );
    }
}

/// **Going home from a document names neither of the grid pane's switches.**
/// The front door draws no grid pane, and the window clears both switches'
/// records at the top of every frame, the door's included — which is what
/// lets the list read them on every frame rather than on the dock's alone.
#[test]
fn going_home_from_a_document_names_neither_grid_switch() {
    let mut live = Live::housing();
    let words = [
        GridLayout::Rows.word(),
        GridLayout::Columns.word(),
        GridSpot::Canvas.word(),
        GridSpot::Ledger.word(),
    ];
    let switch_entries = |live: &Live| -> Vec<(String, egui::Rect)> {
        live.app
            .named_controls()
            .iter()
            .filter(|c| words.contains(&c.name.as_str()))
            .map(|c| (c.name.clone(), c.rect))
            .collect()
    };
    let before = switch_entries(&live);
    for word in words {
        assert!(
            before.iter().any(|(name, _)| name == word),
            "the housing baseline names no {word:?} grid switch state, so going \
             home from it proves nothing about that state; it names {before:?}"
        );
    }

    let home = live.app.home_rect().expect("the band drew a Home button");
    live.run(vec![click_at(home.center()), Vec::new(), Vec::new()]);
    assert!(
        live.app.front_door_is_live(),
        "clicking Home did not return to the front door"
    );
    let stale = switch_entries(&live);
    assert!(
        stale.is_empty(),
        "the front door draws no grid pane and the list still names {stale:?}"
    );
}
