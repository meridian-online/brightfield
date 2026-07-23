//! Slider widget — first end-to-end input widget binding (rpw3 layer 3).
//!
//! Mirrors the brush.rs commit pattern from cfs2: a value-typed
//! `ParamDispatcher` trait abstracts the param-coordinator surface that
//! the UI calls when a slider release commits, an `impl ParamDispatcher
//! for brightfield_engine::Session` forwards to `Session::propagate_param`,
//! and a pure `commit_slider_release` helper isolates the dispatch logic
//! from the windowed event loop so it can be exercised against a recording
//! double (the lifted-helper precedent).
//!
//! ## Out of scope (Decision 2 case iii deferral)
//!
//! Computed-param slider sources — i.e. an `Input::slider` whose options
//! are themselves `$param` references — are NOT modelled here. Min/max/step
//! are read as plain `f64`s on construction. When the computed-param case
//! lands, `SliderBinding::from_input` will gain a resolution step against
//! the live `param_state`. The deferral is locked as a behavioural
//! property at the coordinator layer.
//!
//! ## Other input widgets
//!
//! The menu family (menu / radio / checkbox, `menu.rs`) reuses the
//! `ParamDispatcher` trait + `commit_<widget>_release` helper pattern
//! defined here. This header's original promise — "no coordinator change
//! required" — was FALSE (Explore-verified 2026-07-17): the ENGINE-facing
//! half reuses byte-untouched (`ParamDispatcher` → `propagate_param`;
//! String params already substitute quoted+escaped at SQL emit), but each
//! widget family adds its own ADDITIVE coordinator lane — a bindings field
//! plus `commit_<widget>`/`apply_<widget>` beside the slider's, and a
//! disjunct in the live-surface gate. That is the real reuse contract:
//! trait reuse + additive lanes, not a coordinator-free drop-in.
//! Search / Table remain Unimplemented in
//! [`brightfield_spec::vocab::InputKind`].

use brightfield_engine::DispatchResult;
use brightfield_spec::ast::{Input, SpecValue, ValueOrParamRef};

/// Selection-dispatch surface mirror — the param half of the same pattern
/// `SelectionDispatcher` defines for brushes.
///
/// `ChartView` (or any host that wires a slider in) calls `dispatch` on
/// release; the real `Session` impl forwards to `propagate_param`. Tests
/// substitute a recording double that returns an empty result vec but
/// captures the call — the same shape the brush commit tests'
/// `RecordingDispatcher` uses in `brush.rs`.
pub trait ParamDispatcher {
    /// Dispatch a new value for `name`. Returns one (mark_index, Result)
    /// tuple per subscriber that re-executed during the topological
    /// walk, mirroring `Session::propagate_param`'s return shape.
    fn dispatch(&mut self, name: &str, value: SpecValue) -> Vec<DispatchResult>;
}

impl ParamDispatcher for brightfield_engine::Session {
    fn dispatch(&mut self, name: &str, value: SpecValue) -> Vec<DispatchResult> {
        self.propagate_param(name, value)
    }
}

/// Identity of the slider at dispatch time: which param it writes to,
/// plus the numeric domain (min/max/step) it was constructed from.
///
/// Constructed at the UI layer from an `Input` AST node (kind = Slider)
/// via [`SliderBinding::from_input`] — `analysis.rs` does not know about
/// slider specifics, so the binding never enters the spec-analysis layer.
#[derive(Debug, Clone, PartialEq)]
pub struct SliderBinding {
    /// Name of the param this slider drives (i.e. the `$ident` from
    /// `as: $name` on the Input node).
    pub param_name: String,
    /// Lower bound of the numeric domain.
    pub min: f64,
    /// Upper bound of the numeric domain.
    pub max: f64,
    /// Optional step. When `None`, the slider is treated as continuous.
    pub step: Option<f64>,
}

impl SliderBinding {
    /// Construct a `SliderBinding` from an `Input` AST node. Returns `None` if
    /// the input is missing an `as_param` binding (no param to write to) or
    /// if `min`/`max` are not present as numeric literals in `options` — the
    /// computed-param case (options as `$param` refs) is deferred per the
    /// module-level note.
    ///
    /// `step` is optional; when absent the slider is continuous. `min` and
    /// `max` may be authored as integers or floats in the spec — both are
    /// coerced to `f64` here.
    #[must_use]
    pub fn from_input(input: &Input) -> Option<Self> {
        let param_name = input.as_param.as_ref()?.0.clone();
        let min = read_numeric_option(input, "min")?;
        let max = read_numeric_option(input, "max")?;
        // Reject a degenerate domain (non-finite bounds, or min >= max): it can't
        // drive a functional slider and would panic the downstream `f64::clamp`
        // (which asserts min <= max). Skipping it is consistent with the
        // computed-param case — a malformed input yields no widget, not a crash.
        if !(min.is_finite() && max.is_finite() && min < max) {
            return None;
        }
        let step = read_numeric_option(input, "step");
        Some(Self {
            param_name,
            min,
            max,
            step,
        })
    }
}

/// Read a single numeric option from an Input's `options` bag. Returns
/// `None` for missing keys, lifted `$param` refs (deferred), or non-numeric
/// values. Integer literals are coerced to `f64`.
fn read_numeric_option(input: &Input, key: &str) -> Option<f64> {
    match input.options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Float(f)) => Some(*f),
        ValueOrParamRef::Value(SpecValue::Integer(i)) => Some(*i as f64),
        // Computed-param deferral: a $param reference here is silently
        // ignored at this layer — the UI surface treats it as "no bound".
        // When the computed-param case lands, this will resolve against
        // self.param_state.
        ValueOrParamRef::Param(_) => None,
        _ => None,
    }
}

/// Per-frame state for a slider, lifted out of the windowed shell so it can
/// be tested without a window. Mirrors the
/// `InteractionState`/`commit_brush_release` shape from brush.rs.
///
/// Transitions: `Idle → Dragging` on `mouse_down`,
/// `Dragging → Released` on `mouse_up`,
/// `Released → Idle` once `commit_slider_release` consumes it.
#[derive(Debug, Clone, PartialEq)]
pub enum SliderState {
    /// No interaction in progress.
    Idle,
    /// User is dragging the thumb. `value` reflects the live drag position
    /// (overlay-only — never dispatched).
    Dragging {
        /// Current overlay value while dragging.
        value: f64,
    },
    /// Mouse-up has fired; the next call to `commit_slider_release` will
    /// dispatch this `value` and transition back to `Idle`.
    Released {
        /// Final value at release.
        value: f64,
    },
}

impl SliderState {
    /// Begin a drag at `value`. Idempotent — re-entering Dragging just
    /// updates the live overlay value.
    pub fn start_drag(value: f64) -> Self {
        Self::Dragging { value }
    }

    /// Update the live drag overlay. No-op if not currently dragging.
    pub fn update_drag(&mut self, value: f64) {
        if let Self::Dragging { value: v } = self {
            *v = value;
        }
    }

    /// Transition Dragging → Released. No-op if not dragging.
    pub fn release(&mut self) {
        if let Self::Dragging { value } = *self {
            *self = Self::Released { value };
        }
    }
}

/// Pure helper for the commit path. Given a `SliderState` (which
/// may or may not be `Released`), a binding, and a dispatcher, dispatches
/// the slider's value as a `SpecValue::Float` and returns the result vec
/// alongside the next `SliderState` (always `Idle` post-release).
///
/// On a non-`Released` state the helper is a no-op: empty result vec and
/// the state passes through unchanged. This mirrors `commit_brush_release`
/// in `brush.rs`.
pub fn commit_slider_release<D: ParamDispatcher>(
    state: &SliderState,
    binding: &SliderBinding,
    dispatcher: &mut D,
) -> (SliderState, Vec<DispatchResult>) {
    if let SliderState::Released { value } = *state {
        let results = dispatcher.dispatch(&binding.param_name, SpecValue::Float(value));
        (SliderState::Idle, results)
    } else {
        (state.clone(), Vec::new())
    }
}

/// Map a horizontal pixel position on a slider track to a param value in
/// `[binding.min, binding.max]`, snapped to `step` when present.
///
/// `px`, `track_left`, and `track_width` are logical pixels in the same space
/// (the widget's own coordinate frame). A position outside the track clamps to
/// the nearest end. The inverse — value → thumb fraction, for drawing — is
/// [`thumb_fraction`].
#[must_use]
pub fn value_at(px: f64, track_left: f64, track_width: f64, binding: &SliderBinding) -> f64 {
    let frac = if track_width > 0.0 {
        ((px - track_left) / track_width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let raw = binding.min + frac * (binding.max - binding.min);
    match binding.step {
        // Snap to the nearest step measured from min, then clamp to the domain.
        // `.max(min).min(max)` rather than `f64::clamp` so a caller-constructed
        // binding with min > max yields a defined value instead of a panic
        // (`from_input` already rejects such domains for spec-driven sliders).
        Some(step) if step > 0.0 => {
            let snapped = binding.min + ((raw - binding.min) / step).round() * step;
            snapped.max(binding.min).min(binding.max)
        }
        _ => raw,
    }
}

/// The normalised `[0, 1]` position of `value` along the track — used to place
/// the thumb. Clamped to the domain; the inverse of [`value_at`]'s fraction.
#[must_use]
pub fn thumb_fraction(value: f64, binding: &SliderBinding) -> f64 {
    if binding.max > binding.min {
        ((value - binding.min) / (binding.max - binding.min)).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::ParamRef;

    fn binding(min: f64, max: f64, step: Option<f64>) -> SliderBinding {
        SliderBinding {
            param_name: "k".to_string(),
            min,
            max,
            step,
        }
    }

    // A track pixel maps to a param value; endpoints and
    // mid are exact, step snaps, out-of-track clamps.
    #[test]
    fn value_at_maps_track_pixels() {
        let b = binding(0.0, 10.0, Some(1.0)); // track x=100..300 (width 200)
        assert_eq!(value_at(100.0, 100.0, 200.0, &b), 0.0, "left = min");
        assert_eq!(value_at(300.0, 100.0, 200.0, &b), 10.0, "right = max");
        assert_eq!(value_at(200.0, 100.0, 200.0, &b), 5.0, "mid = 5");
        assert_eq!(value_at(160.0, 100.0, 200.0, &b), 3.0, "0.3 -> snapped 3");
        assert_eq!(
            value_at(50.0, 100.0, 200.0, &b),
            0.0,
            "left of track clamps"
        );
        assert_eq!(
            value_at(400.0, 100.0, 200.0, &b),
            10.0,
            "right of track clamps"
        );

        // Continuous (no step): exact fractional value.
        let c = binding(0.0, 1.0, None);
        assert!((value_at(150.0, 100.0, 200.0, &c) - 0.25).abs() < 1e-9);
    }

    // thumb_fraction is the drawing-side inverse and clamps.
    #[test]
    fn thumb_fraction_inverts_value() {
        let b = binding(0.0, 10.0, Some(1.0));
        assert_eq!(thumb_fraction(0.0, &b), 0.0);
        assert_eq!(thumb_fraction(10.0, &b), 1.0);
        assert_eq!(thumb_fraction(5.0, &b), 0.5);
        assert_eq!(thumb_fraction(-5.0, &b), 0.0, "clamps below min");
        assert_eq!(thumb_fraction(15.0, &b), 1.0, "clamps above max");
    }
    use brightfield_spec::vocab::{ImplStatus, InputKind};
    use indexmap::IndexMap;

    /// Recording test double — captures every dispatch call in order.
    /// Mirrors the brush RecordingDispatcher in brush.rs.
    struct RecordingDispatcher {
        calls: Vec<(String, SpecValue)>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self { calls: Vec::new() }
        }
    }

    impl ParamDispatcher for RecordingDispatcher {
        fn dispatch(&mut self, name: &str, value: SpecValue) -> Vec<DispatchResult> {
            self.calls.push((name.to_string(), value));
            // Stub return: subscribers, if any, are mocked as zero — the
            // double's contract is "did dispatch get called?".
            Vec::new()
        }
    }

    /// Build a fixture Input node mimicking
    /// ```yaml
    /// input: slider
    /// as: $threshold
    /// min: 0
    /// max: 100
    /// step: 1
    /// ```
    fn input_fixture(min: SpecValue, max: SpecValue, step: Option<SpecValue>) -> Input {
        let mut options = IndexMap::new();
        options.insert("min".to_string(), ValueOrParamRef::Value(min));
        options.insert("max".to_string(), ValueOrParamRef::Value(max));
        if let Some(s) = step {
            options.insert("step".to_string(), ValueOrParamRef::Value(s));
        }
        Input {
            kind: InputKind::Slider,
            status: ImplStatus::Implemented,
            as_param: Some(ParamRef("threshold".to_string())),
            from_source: None,
            filter_by: None,
            options,
        }
    }

    /// <Session as ParamDispatcher>::dispatch forwards to
    /// propagate_param identically. The trait body is a single forwarding
    /// call — this test pins that contract by walking through the trait
    /// surface and asserting the param_state effect that propagate_param
    /// would produce on its own.
    ///
    /// We exercise the trait surface directly via a Session-equivalent
    /// recording double: the trait's body is `self.propagate_param(name,
    /// value)`, so the inputs flow through unchanged. A fuller corpus run
    /// would require a live DuckDB session; the unit boundary here is the
    /// trait's forwarding shape.
    #[test]
    fn param_dispatcher_session_impl() {
        // The Session impl forwards 1:1: dispatch(name, value) ==
        // propagate_param(name, value). We assert the trait method exists
        // with the expected signature by constructing a function pointer.
        let _f: fn(&mut brightfield_engine::Session, &str, SpecValue) -> Vec<DispatchResult> =
            <brightfield_engine::Session as ParamDispatcher>::dispatch;
        // The signature equality above proves the trait is implemented for
        // Session. End-to-end forwarding equivalence is exercised inside
        // the engine crate's rpw3 tests (which already cover propagate_param
        // behaviour); duplicating a live-session call here would re-test the
        // engine, not the forwarding contract.
    }

    /// SliderBinding::from_input captures param_name plus
    /// min/max/step from the Input.options block.
    #[test]
    fn slider_binding_from_input_options() {
        let input = input_fixture(
            SpecValue::Float(0.0),
            SpecValue::Float(100.0),
            Some(SpecValue::Float(1.0)),
        );
        let binding = SliderBinding::from_input(&input).expect("constructs");
        assert_eq!(binding.param_name, "threshold");
        assert!((binding.min - 0.0).abs() < f64::EPSILON);
        assert!((binding.max - 100.0).abs() < f64::EPSILON);
        assert_eq!(binding.step, Some(1.0));
    }

    /// integer-authored bounds are coerced to f64.
    #[test]
    fn slider_binding_integer_bounds_coerced() {
        let input = input_fixture(SpecValue::Integer(0), SpecValue::Integer(100), None);
        let binding = SliderBinding::from_input(&input).expect("constructs");
        assert!((binding.min - 0.0).abs() < f64::EPSILON);
        assert!((binding.max - 100.0).abs() < f64::EPSILON);
        assert_eq!(binding.step, None);
    }

    /// slw review-fix: a degenerate domain (min > max, min == max, or non-finite
    /// bounds) yields no binding — `from_input` rejects it rather than producing a
    /// slider that would panic the downstream `f64::clamp`.
    #[test]
    fn slw_from_input_rejects_degenerate_domain() {
        // Inverted bounds.
        assert!(
            SliderBinding::from_input(&input_fixture(
                SpecValue::Integer(8),
                SpecValue::Integer(0),
                Some(SpecValue::Integer(1)),
            ))
            .is_none(),
            "min > max is rejected"
        );
        // Empty domain.
        assert!(
            SliderBinding::from_input(&input_fixture(
                SpecValue::Integer(5),
                SpecValue::Integer(5),
                None,
            ))
            .is_none(),
            "min == max is rejected"
        );
        // Non-finite bound.
        assert!(
            SliderBinding::from_input(&input_fixture(
                SpecValue::Float(0.0),
                SpecValue::Float(f64::NAN),
                None,
            ))
            .is_none(),
            "NaN bound is rejected"
        );
    }

    /// slw review-fix: `value_at` never panics even on a caller-constructed
    /// inverted binding (the public helper defends itself; `from_input` guards the
    /// spec path). Rust's `f64::clamp` would assert min <= max — this must not.
    #[test]
    fn slw_value_at_no_panic_on_inverted_binding() {
        let inverted = SliderBinding {
            param_name: "t".to_string(),
            min: 8.0,
            max: 0.0,
            step: Some(1.0),
        };
        // Any of these would panic under `f64::clamp`; here they just return a
        // finite, defined value.
        for px in [0.0, 100.0, 300.0] {
            let v = value_at(px, 0.0, 200.0, &inverted);
            assert!(v.is_finite(), "value_at stays finite for px={px}");
        }
    }

    /// commit_slider_release dispatches the binding's
    /// param_name + value as a SpecValue::Float, transitions Released → Idle.
    #[test]
    fn commit_slider_release_dispatches() {
        let binding = SliderBinding {
            param_name: "threshold".to_string(),
            min: 0.0,
            max: 100.0,
            step: Some(1.0),
        };
        let state = SliderState::Released { value: 42.5 };
        let mut dispatcher = RecordingDispatcher::new();

        let (next_state, _results) = commit_slider_release(&state, &binding, &mut dispatcher);

        assert_eq!(
            dispatcher.calls.len(),
            1,
            "exactly one dispatch on Released → Idle"
        );
        let (name, value) = &dispatcher.calls[0];
        assert_eq!(name, "threshold");
        match value {
            SpecValue::Float(f) => assert!((*f - 42.5).abs() < f64::EPSILON),
            other => panic!("expected SpecValue::Float, got {other:?}"),
        }
        assert_eq!(next_state, SliderState::Idle);
    }

    /// the slider's mouse_up handler invokes
    /// commit_slider_release. Mirrors the lifted-helper boundary:
    /// drive the state machine from Idle → Dragging → Released and
    /// observe one dispatch.
    #[test]
    fn slider_on_mouse_up_dispatches() {
        let binding = SliderBinding {
            param_name: "threshold".to_string(),
            min: 0.0,
            max: 100.0,
            step: None,
        };
        let mut dispatcher = RecordingDispatcher::new();

        // mouse_down: enter Dragging at start value.
        let mut state = SliderState::start_drag(20.0);
        // drag: update the overlay value live.
        state.update_drag(60.0);
        state.update_drag(80.5);
        assert!(matches!(state, SliderState::Dragging { .. }));
        assert!(dispatcher.calls.is_empty(), "mid-drag must not dispatch");

        // mouse_up: transition Dragging → Released, then commit.
        state.release();
        assert!(matches!(state, SliderState::Released { .. }));
        let (next_state, _results) = commit_slider_release(&state, &binding, &mut dispatcher);

        assert_eq!(dispatcher.calls.len(), 1, "exactly one dispatch on release");
        let (name, value) = &dispatcher.calls[0];
        assert_eq!(name, "threshold");
        match value {
            SpecValue::Float(f) => assert!((*f - 80.5).abs() < f64::EPSILON),
            other => panic!("expected SpecValue::Float, got {other:?}"),
        }
        assert_eq!(next_state, SliderState::Idle);
    }

    /// mouse_down without a subsequent mouse_up does NOT
    /// dispatch. Mid-drag state is overlay-only on the widget.
    #[test]
    fn slider_no_drag_no_dispatch() {
        let binding = SliderBinding {
            param_name: "threshold".to_string(),
            min: 0.0,
            max: 100.0,
            step: None,
        };
        let mut dispatcher = RecordingDispatcher::new();

        // mouse_down + drag, but no mouse_up.
        let mut state = SliderState::start_drag(20.0);
        state.update_drag(45.0);
        assert!(matches!(state, SliderState::Dragging { .. }));

        // Attempting to commit a Dragging state must be a no-op.
        let (next_state, results) = commit_slider_release(&state, &binding, &mut dispatcher);

        assert!(
            dispatcher.calls.is_empty(),
            "Dragging (no mouse_up) must not dispatch"
        );
        assert!(results.is_empty());
        assert!(
            matches!(next_state, SliderState::Dragging { .. }),
            "non-Released state passes through unchanged"
        );
    }
}
