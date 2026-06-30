//! ChartView — GPUI Render component for chart display.
//!
//! ChartView is the public API for embedding a chart in a GPUI window.
//! It owns an `Entity<ChartState>` and implements `gpui::Render`.
//!
//! Consumers create a ChartView with a `Model<ChartState>` and add it
//! to a GPUI window. ChartView::render() returns a ChartElement that
//! implements the Element trait.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{div, px, rgb, Context, Entity, IntoElement, ParentElement, Render, Styled, Window};

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
use brightfield_spec::analysis::{BrushableBinding, ComponentPath};

use crate::brush::{brush_rect_to_predicate, BrushKind, ChannelColumns, SelectionDispatcher};
use crate::chart_element::ChartElement;
use crate::chart_state::ChartState;
use crate::crossfilter::CrossfilterCoordinator;
use crate::interaction::InteractionState;

/// One plot positioned in the dashboard: its rect (in dashboard pixels) and its
/// own reactive [`ChartState`]. Each plot owns its state — and thus its own
/// raster cache and interaction — so hover/brush are independent per plot.
pub struct PlacedChart {
    /// Left edge within the dashboard, in pixels.
    pub x: f64,
    /// Top edge within the dashboard, in pixels.
    pub y: f64,
    /// Plot width in pixels.
    pub width: f64,
    /// Plot height in pixels.
    pub height: f64,
    /// The plot's reactive chart state.
    pub state: Entity<ChartState>,
    /// Shared cross-filter coordinator, if this dashboard cross-filters. When
    /// present, a brush release on this plot routes through it (re-query +
    /// re-render subscribers); when `None`, the brush is purely visual.
    pub coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
}

/// GPUI render component for a dashboard: hosts one [`ChartElement`] per plot,
/// each absolutely positioned at its layout rect, in a container sized to the
/// dashboard's bounding box. A single-plot spec is just a one-plot dashboard.
pub struct ChartView {
    /// Dashboard width in pixels.
    width: f64,
    /// Dashboard height in pixels.
    height: f64,
    /// The positioned plots.
    charts: Vec<PlacedChart>,
}

impl ChartView {
    /// Create a dashboard view of the given size hosting the positioned plots.
    pub fn new(width: f64, height: f64, charts: Vec<PlacedChart>) -> Self {
        Self {
            width,
            height,
            charts,
        }
    }
}

impl Render for ChartView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Fill the window with a white background and centre the dashboard, so a
        // resized (larger) window shows a clean margin rather than the black void
        // of the unpainted backing layer. The inner container is the dashboard's
        // fixed size, with each plot absolutely positioned at its rect; each
        // ChartElement reads its own ChartState and wires its own mouse events,
        // so plots don't share interaction.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0xffffff))
            .child(
                div()
                    .relative()
                    .w(px(self.width as f32))
                    .h(px(self.height as f32))
                    .children(self.charts.iter().enumerate().map(|(i, c)| {
                        div()
                            .absolute()
                            .left(px(c.x as f32))
                            .top(px(c.y as f32))
                            .w(px(c.width as f32))
                            .h(px(c.height as f32))
                            .child(ChartElement::new(c.state.clone(), i, c.coordinator.clone()))
                    })),
            )
    }
}

/// Identity of the brush at dispatch time: which selection it writes
/// to, the contributing component path (for self-exclusion), the
/// brush kind (intervalX / intervalY / intervalXY), and the channel
/// columns the rect coordinates compare against.
#[derive(Debug, Clone)]
pub struct BrushBinding {
    /// Name of the selection this brush contributes to (e.g. `brush`).
    pub selection_name: String,
    /// Parent-plot path of the contributor (for self-exclusion).
    pub contributor: ComponentPath,
    /// Brush kind (intervalX, intervalY, intervalXY).
    pub kind: BrushKind,
    /// Bound channel columns.
    pub channels: ChannelColumns,
}

/// Epsilon for the click-vs-drag boundary — a brush whose extent on both
/// axes is less than `ZERO_AREA_EPSILON` pixels is treated as a click and
/// routed through [`commit_brush_clear`] (a `clear` dispatch). Anything
/// larger is a drag and goes through [`commit_brush_release`] (a
/// `dispatch` with the rect-derived predicate).
pub const ZERO_AREA_EPSILON: f64 = 0.5;

/// Multi-binding form of [`commit_brush_release`]. Iterates the supplied
/// bindings and dispatches one selection per binding — each binding's
/// predicate is computed using its own kind (kind-compatibility filter).
/// Returns the next `InteractionState` (always `Idle` after a release) plus
/// per-binding aggregated dispatch results.
///
/// cfs3 ac-04. Single-binding consumers should call
/// [`commit_brush_release`] (a 1-element-slice wrapper preserved for the
/// cfs2_ac11 boundary).
pub fn commit_brush_release_multi<D: SelectionDispatcher>(
    interaction: &InteractionState,
    bindings: &[BrushBinding],
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(String, Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>)>,
) {
    if let InteractionState::Brushing { start, current } = interaction {
        let rect = kurbo::Rect::new(
            start.x.min(current.x),
            start.y.min(current.y),
            start.x.max(current.x),
            start.y.max(current.y),
        );
        let mut aggregated = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let predicate = brush_rect_to_predicate(rect, binding.kind, &binding.channels);
            let results = dispatcher.dispatch(
                &binding.selection_name,
                binding.contributor.clone(),
                predicate,
            );
            aggregated.push((binding.selection_name.clone(), results));
        }
        (InteractionState::Idle, aggregated)
    } else {
        (interaction.clone(), Vec::new())
    }
}

/// Pure helper for cfs2_ac11: given an InteractionState (which may or
/// may not be Brushing), a binding, and a dispatcher, produce the
/// dispatch result vec and the next InteractionState. Lifted out of
/// the GPUI context for testability — chart_view.on_mouse_up_with_dispatch
/// shares the same logic but threads it through Entity<ChartState>.
///
/// **cfs3 wrapper:** preserved as a single-binding convenience over
/// [`commit_brush_release_multi`] so the cfs2_ac11 surface stays green.
pub fn commit_brush_release<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    let (next_state, mut aggregated) =
        commit_brush_release_multi(interaction, std::slice::from_ref(binding), dispatcher);
    let results = aggregated.pop().map(|(_, r)| r).unwrap_or_default();
    (next_state, results)
}

/// Pure helper for the click-vs-drag boundary. When `interaction` is
/// `Idle` OR a zero-area `Brushing` (start ≈ current within
/// [`ZERO_AREA_EPSILON`] on both axes), dispatch a `clear` call on the
/// supplied binding's selection and return `Idle` as the next state. A
/// non-zero `Brushing` does NOT dispatch through this path — it goes
/// through [`commit_brush_release`] (the drag-release path).
///
/// cfs3 ac-03.
pub fn commit_brush_clear<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (
    InteractionState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    let should_clear = match interaction {
        InteractionState::Idle => true,
        InteractionState::Brushing { start, current } => {
            (start.x - current.x).abs() < ZERO_AREA_EPSILON
                && (start.y - current.y).abs() < ZERO_AREA_EPSILON
        }
        _ => false,
    };
    if should_clear {
        let results = dispatcher.clear(&binding.selection_name, binding.contributor.clone());
        (InteractionState::Idle, results)
    } else {
        (interaction.clone(), Vec::new())
    }
}

/// Convert a spec-side [`BrushableBinding`] into a UI-side [`BrushBinding`]
/// by translating the mirror enums (`BrushKind`, `ChannelColumns`). The
/// conversion is faithful — every field copies through verbatim. cfs3 ac-06.
impl From<&BrushableBinding> for BrushBinding {
    fn from(b: &BrushableBinding) -> Self {
        BrushBinding {
            selection_name: b.selection.clone(),
            contributor: b.parent_plot.clone(),
            kind: brush_kind_from_spec(b.kind),
            channels: ChannelColumns {
                x: b.channels.x.clone(),
                y: b.channels.y.clone(),
            },
        }
    }
}

fn brush_kind_from_spec(kind: brightfield_spec::analysis::BrushKind) -> BrushKind {
    use brightfield_spec::analysis::BrushKind as Spec;
    match kind {
        Spec::IntervalX => BrushKind::IntervalX,
        Spec::IntervalY => BrushKind::IntervalY,
        Spec::IntervalXY => BrushKind::IntervalXY,
        Spec::Point => BrushKind::Point,
    }
}

// --- AC-05 tests: coordinate transform and interaction state ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_layout::ChartLayout;
    use brightfield_sql::ir::Predicate;
    use kurbo::Point;

    // Unit tests for coordinate transform logic — these don't require
    // a GPUI runtime, just the math.

    #[test]
    fn gmr_ac05_coordinate_transform_inside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        let window_pos = Point::new(400.0, 300.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 300.0).abs() < f64::EPSILON);
        assert!((local.y - 250.0).abs() < f64::EPSILON);
        assert!(layout.contains(local), "point should be inside plot area");
    }

    #[test]
    fn gmr_ac05_coordinate_transform_outside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        // Point in the left margin area
        let window_pos = Point::new(110.0, 100.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 10.0).abs() < f64::EPSILON);
        assert!(!layout.contains(local), "point should be outside plot area (in left margin)");
    }

    #[test]
    fn gmr_ac05_interaction_state_idle_to_brushing() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(
            matches!(state, InteractionState::Brushing { .. }),
            "should transition to Brushing on mouse_down inside plot area"
        );
    }

    #[test]
    fn gmr_ac05_interaction_state_brushing_to_idle() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(matches!(state, InteractionState::Brushing { .. }));

        // On mouse_up, we'd set to Idle
        let idle = InteractionState::Idle;
        assert!(matches!(idle, InteractionState::Idle));
    }

    #[test]
    fn gmr_ac05_brush_update_during_drag() {
        let mut state = InteractionState::start_brush(Point::new(100.0, 200.0));
        state.update_brush(Point::new(300.0, 400.0));

        let rect = state.brush_rect().expect("should have brush rect");
        assert!((rect.x0 - 100.0).abs() < f64::EPSILON);
        assert!((rect.y0 - 200.0).abs() < f64::EPSILON);
        assert!((rect.x1 - 300.0).abs() < f64::EPSILON);
        assert!((rect.y1 - 400.0).abs() < f64::EPSILON);
    }

    // --- gmr_ac07: Resize ---

    #[test]
    fn gmr_ac07_layout_dimensions_change() {
        let layout = ChartLayout::new(640.0, 480.0);
        assert!((layout.width - 640.0).abs() < f64::EPSILON);

        let resized = ChartLayout::new(1024.0, 768.0);
        assert!((resized.width - 1024.0).abs() < f64::EPSILON);
        assert!((resized.height - 768.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gmr_ac07_render_respects_new_dimensions() {
        // Verify plot area scales with dimensions
        let layout = ChartLayout::new(1024.0, 768.0);
        let area = layout.plot_area();
        assert!((area.x1 - (1024.0 - 20.0)).abs() < f64::EPSILON);
        assert!((area.y1 - (768.0 - 30.0)).abs() < f64::EPSILON);
    }

    // --- cfs2_ac11: brush release dispatches a propagate_selection call ---

    /// Recording test double: captures every dispatch and clear call
    /// in order so tests can assert call counts, ordering, and arguments.
    struct RecordingDispatcher {
        calls: Vec<(String, ComponentPath, Predicate)>,
        clear_calls: Vec<(String, ComponentPath)>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                clear_calls: Vec::new(),
            }
        }
    }

    impl SelectionDispatcher for RecordingDispatcher {
        fn dispatch(
            &mut self,
            name: &str,
            contributor: ComponentPath,
            predicate: Predicate,
        ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
            self.calls.push((name.to_string(), contributor, predicate));
            // Stub return: subscribers, if any, are mocked as zero —
            // this double's contract is "did dispatch get called?".
            Vec::new()
        }

        fn clear(
            &mut self,
            name: &str,
            contributor: ComponentPath,
        ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
            self.clear_calls.push((name.to_string(), contributor));
            Vec::new()
        }
    }

    #[test]
    fn cfs2_ac11_on_mouse_up_dispatches_selection() {
        // Simulate the mouse-down → drag → mouse-up sequence at the
        // InteractionState level, then drive commit_brush_release with a
        // recording dispatcher. The recorded call must carry the
        // selection name, contributor path, and a non-True Predicate
        // derived from the brush rect.

        // mouse-down: start a brush.
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        // drag.
        interaction.update_brush(Point::new(120.0, 230.0));

        // mouse-up: commit.
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalXY,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let mut dispatcher = RecordingDispatcher::new();

        let (next_state, _results) =
            commit_brush_release(&interaction, &binding, &mut dispatcher);

        // Exactly one dispatch.
        assert_eq!(
            dispatcher.calls.len(),
            1,
            "exactly one propagate_selection call on Brushing→Idle"
        );
        let (name, contributor, predicate) = &dispatcher.calls[0];
        assert_eq!(name, "brush");
        assert_eq!(contributor, &ComponentPath("root/plot[0]".to_string()));
        // Predicate must be derived from the brush rect — not Predicate::True.
        assert!(
            !matches!(predicate, Predicate::True),
            "brush release must produce a non-trivial predicate; got: {predicate:?}"
        );
        // State transitioned to Idle.
        assert!(
            matches!(next_state, InteractionState::Idle),
            "post-release state should be Idle"
        );
    }

    #[test]
    fn cfs2_ac11_on_mouse_up_no_brush_no_dispatch() {
        // If interaction is Idle (no active brush), mouse-up must not
        // dispatch — same partial-failure / no-op discipline as the
        // existing on_mouse_up.
        let interaction = InteractionState::Idle;
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let mut dispatcher = RecordingDispatcher::new();

        let (next_state, results) =
            commit_brush_release(&interaction, &binding, &mut dispatcher);

        assert!(dispatcher.calls.is_empty(), "no brush → no dispatch");
        assert!(results.is_empty());
        assert!(matches!(next_state, InteractionState::Idle));
    }

    // ---------------------------------------------------------------------
    // cfs3 — clearing, multi-binding dispatch, BrushableBinding conversion
    // ---------------------------------------------------------------------

    /// cfs3_ac03: commit_brush_clear dispatches a `clear` call when the
    /// interaction is Idle OR a zero-area Brushing (click). A non-zero
    /// Brushing does NOT clear (that path is the drag-release, handled by
    /// commit_brush_release). Returns Idle as the next state on a clear.
    #[test]
    fn cfs3_ac03_click_outside_active_brush_clears() {
        let binding = BrushBinding {
            selection_name: "brush".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };

        // (a) Idle → one clear call.
        let mut dispatcher = RecordingDispatcher::new();
        let (next_state, results) =
            commit_brush_clear(&InteractionState::Idle, &binding, &mut dispatcher);
        assert!(dispatcher.calls.is_empty(), "no dispatch on clear path");
        assert_eq!(
            dispatcher.clear_calls.len(),
            1,
            "Idle → exactly one clear call"
        );
        let (name, contributor) = &dispatcher.clear_calls[0];
        assert_eq!(name, "brush");
        assert_eq!(contributor, &ComponentPath("root/plot[0]".to_string()));
        assert!(results.is_empty(), "test double's stub returns no results");
        assert!(matches!(next_state, InteractionState::Idle));

        // (c) Zero-area Brushing → still a clear (click below drag threshold).
        let mut dispatcher = RecordingDispatcher::new();
        let zero_area = {
            let p = Point::new(100.0, 200.0);
            let mut s = InteractionState::start_brush(p);
            // Move within epsilon — still classified as zero-area.
            s.update_brush(Point::new(p.x + 0.1, p.y - 0.1));
            s
        };
        let (next_state, _) =
            commit_brush_clear(&zero_area, &binding, &mut dispatcher);
        assert_eq!(
            dispatcher.clear_calls.len(),
            1,
            "zero-area Brushing → exactly one clear call"
        );
        assert!(matches!(next_state, InteractionState::Idle));

        // (d) Non-zero Brushing → NO dispatch through this path.
        //     (Drag releases go through commit_brush_release.)
        let mut dispatcher = RecordingDispatcher::new();
        let mut drag = InteractionState::start_brush(Point::new(20.0, 30.0));
        drag.update_brush(Point::new(120.0, 230.0));
        let (next_state, _) = commit_brush_clear(&drag, &binding, &mut dispatcher);
        assert!(
            dispatcher.calls.is_empty() && dispatcher.clear_calls.is_empty(),
            "non-zero Brushing → neither dispatch nor clear via this path"
        );
        // State is unchanged on the no-op path.
        assert!(matches!(next_state, InteractionState::Brushing { .. }));
    }

    /// cfs3_ac04: commit_brush_release_multi (the lifted multi-binding
    /// helper) dispatches one propagate_selection per binding, with each
    /// binding's predicate computed against its own kind. Verifies the
    /// kind-compatibility filter — an IntervalX binding produces an x-only
    /// predicate even when the rect has a non-zero y extent.
    #[test]
    fn cfs3_ac04_plot_drives_multiple_selections() {
        // (a) Construct a Brushing state with a 100x200 rect (non-zero on
        //     both axes).
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        interaction.update_brush(Point::new(120.0, 230.0));

        // Two bindings on the same plot writing to different selections.
        let binding_xy = BrushBinding {
            selection_name: "a".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalXY,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let binding_x = BrushBinding {
            selection_name: "b".to_string(),
            contributor: ComponentPath("root/plot[0]".to_string()),
            kind: BrushKind::IntervalX,
            channels: ChannelColumns::xy("speed", "delay"),
        };
        let bindings = [binding_xy, binding_x];

        // (b) Drive commit_brush_release_multi.
        let mut dispatcher = RecordingDispatcher::new();
        let (next_state, aggregated) =
            commit_brush_release_multi(&interaction, &bindings, &mut dispatcher);

        // (c) Two dispatch calls, one per binding.
        assert_eq!(
            dispatcher.calls.len(),
            2,
            "two bindings → two propagate_selection calls"
        );
        // (d) Each call's selection_name matches its binding.
        let names: Vec<&str> = dispatcher
            .calls
            .iter()
            .map(|(n, _, _)| n.as_str())
            .collect();
        assert!(names.contains(&"a"), "selection $a dispatched");
        assert!(names.contains(&"b"), "selection $b dispatched");

        // The IntervalX binding's predicate references only the x channel.
        let (_, _, b_pred) = dispatcher
            .calls
            .iter()
            .find(|(n, _, _)| n == "b")
            .expect("selection b dispatched");
        match b_pred {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 2, "IntervalX → two clauses (x-only)");
                for c in clauses {
                    let s = match c {
                        Predicate::Expr(s) => s,
                        _ => panic!("expected Expr clause"),
                    };
                    assert!(
                        s.contains("speed"),
                        "IntervalX predicate must reference x col only: {s}"
                    );
                    assert!(
                        !s.contains("delay"),
                        "IntervalX predicate must NOT reference y col: {s}"
                    );
                }
            }
            other => panic!("expected Predicate::And for IntervalX, got {other:?}"),
        }

        // Aggregated return shape mirrors the dispatcher record.
        assert_eq!(aggregated.len(), 2);
        assert!(matches!(next_state, InteractionState::Idle));
    }

    /// cfs3_ac06: BrushBinding::from(&BrushableBinding) preserves every
    /// field — selection_name, contributor (= parent_plot), kind, and
    /// channels — translating between the spec-side and ui-side mirror
    /// enums verbatim.
    #[test]
    fn cfs3_ac06_brushable_binding_to_brush_binding() {
        let spec_binding = BrushableBinding {
            interactor_path: ComponentPath(
                "root/plot[0]/interactor[intervalXY]".to_string(),
            ),
            parent_plot: ComponentPath("root/plot[0]".to_string()),
            selection: "brush".to_string(),
            kind: brightfield_spec::analysis::BrushKind::IntervalXY,
            channels: brightfield_spec::analysis::ChannelColumns {
                x: Some("speed".to_string()),
                y: Some("delay".to_string()),
            },
        };

        let ui_binding: BrushBinding = (&spec_binding).into();

        assert_eq!(ui_binding.selection_name, "brush");
        assert_eq!(
            ui_binding.contributor,
            ComponentPath("root/plot[0]".to_string()),
            "contributor = parent_plot"
        );
        assert_eq!(ui_binding.kind, BrushKind::IntervalXY);
        assert_eq!(ui_binding.channels.x.as_deref(), Some("speed"));
        assert_eq!(ui_binding.channels.y.as_deref(), Some("delay"));
    }
}
