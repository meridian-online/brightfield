//! Menu widget family — menu / radio / checkbox.
//!
//! The discrete counterpart of `slider.rs`, following the same gpui-free
//! model / thin element split: `MenuBinding` is the widget's dispatch-time
//! identity (param + resolved options + presentation style),
//! [`MenuState`] the interaction machine, and [`commit_menu_release`] the
//! pure dispatch helper exercised against a recording double. The GPUI
//! shim (`menu_element.rs`) and hosting (`chart_view.rs`) consume these
//! thinly — no param or option-resolution logic lives window-side
//! (the egui-exit posture).
//!
//! Radio and checkbox are `style:` PRESENTATIONS of `input: menu`, read
//! from the preserved-verbatim options bag — never new vocabulary
//! (`input: radio` stays a parse error). Every spec this module accepts
//! parses as plain Mosaic `input: menu`.
//!
//! Unlike `commit_slider_release` (hardcoded `SpecValue::Float`), a menu
//! dispatches the picked option's [`SpecValue`] VERBATIM — a String stays
//! a String, an Integer an Integer — because the variant identity is
//! load-bearing at SQL emit (quoting/escaping, emit.rs).

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
use brightfield_spec::ast::{Input, SpecValue, ValueOrParamRef};
use brightfield_spec::vocab::InputKind;

pub use crate::slider::ParamDispatcher;

/// How an `input: menu` presents (the `style:` key in the options bag).
/// Identical param semantics across all three — presentation-only divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuStyle {
    /// Closed box showing the current value; a click opens the option list.
    Menu,
    /// All options rendered expanded, one-of-N; a click commits directly.
    Radio,
    /// A two-option toggle; a click commits the OTHER option.
    Checkbox,
}

/// Marker for options derived from the data (`from:` + `column:` instead of
/// a literal `options:` list) — resolved static-at-load via
/// `Session::distinct_values` at assembly, never re-queried live in v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedOptions {
    /// The `from:` source name.
    pub source: String,
    /// The `column:` whose distinct values become the options.
    pub column: String,
}

/// Identity of a menu-family widget at dispatch time: the param it writes,
/// its presentation style, and its option values (typed [`SpecValue`]s).
///
/// Constructed at the UI layer from an `Input` AST node (kind = Menu) via
/// [`MenuBinding::from_input`] — like `SliderBinding`, the binding never
/// enters the spec-analysis layer. For a derived-options menu the
/// construction-time `options` are empty and `derived` carries the source;
/// app assembly resolves them (and reconciles the param default) before the
/// binding reaches the coordinator.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuBinding {
    /// Name of the param this widget drives (the `$ident` from `as:`).
    pub param_name: String,
    /// Presentation style (may be degraded to `Menu` at assembly — the
    /// post-reconciliation checkbox/radio rules).
    pub style: MenuStyle,
    /// The option values, preserving each option's SpecValue variant
    /// (a numeric option is NOT stringified). Empty until assembly
    /// resolution when `derived` is `Some`.
    pub options: Vec<SpecValue>,
    /// `Some` when the options derive from a data column (see
    /// [`DerivedOptions`]); provenance is retained after resolution so the
    /// derived-radio degrade rule stays decidable.
    pub derived: Option<DerivedOptions>,
}

impl MenuBinding {
    /// Construct a `MenuBinding` from an `Input` AST node (kind = Menu).
    ///
    /// Returns `(binding, degrade_reasons)` — the return is widened beyond
    /// slider parity so this gpui-free model is the single source of the
    /// CONSTRUCTION-TIME degrade decisions and assembly only logs what the
    /// model surfaced. Today the only construction-time class is
    /// an unknown `style:` value (falls back to `Menu` + one reason); the
    /// assembly-time classes — post-reconciliation checkbox/radio, cap
    /// truncation — surface at assembly, where the param default and
    /// `distinct_values` result are in scope.
    ///
    /// `None` (no binding) when:
    /// - `as:` is missing (no param to write to — slider parity), or
    /// - the input is not `kind: Menu`, or
    /// - neither a non-empty literal `options:` list nor a derivable
    ///   `from:`+`column:` source exists AND the style is not checkbox.
    ///
    /// `style: checkbox` with `options:` absent (and no derivable source)
    /// SYNTHESISES the default pair `[Bool(true), Bool(false)]` — the
    /// flagship WHERE-flag shape needs no options list, so it must yield a
    /// working binding, never a silent drop.
    #[must_use]
    pub fn from_input(input: &Input) -> (Option<Self>, Vec<String>) {
        let mut reasons = Vec::new();
        if input.kind != InputKind::Menu {
            return (None, reasons);
        }
        let Some(param_name) = input.as_param.as_ref().map(|p| p.0.clone()) else {
            return (None, reasons);
        };

        let style = match input.options.get("style") {
            None => MenuStyle::Menu,
            Some(ValueOrParamRef::Value(SpecValue::String(s))) => match s.as_str() {
                "menu" => MenuStyle::Menu,
                "radio" => MenuStyle::Radio,
                "checkbox" => MenuStyle::Checkbox,
                other => {
                    reasons.push(format!(
                        "menu '{param_name}': unknown style '{other}' — \
                         falling back to menu presentation"
                    ));
                    MenuStyle::Menu
                }
            },
            Some(other) => {
                reasons.push(format!(
                    "menu '{param_name}': non-string style value {other:?} — \
                     falling back to menu presentation"
                ));
                MenuStyle::Menu
            }
        };

        // Literal options list, each value's SpecValue variant preserved.
        let literal: Option<Vec<SpecValue>> = match input.options.get("options") {
            Some(ValueOrParamRef::Value(SpecValue::Array(items))) if !items.is_empty() => {
                Some(items.clone())
            }
            _ => None,
        };

        // Derived marker: options absent, from/column present.
        let column = match input.options.get("column") {
            Some(ValueOrParamRef::Value(SpecValue::String(c))) => Some(c.clone()),
            _ => None,
        };
        let derived = match (&literal, &input.from_source, column) {
            (None, Some(source), Some(column)) => Some(DerivedOptions {
                source: source.clone(),
                column,
            }),
            _ => None,
        };

        let (options, derived) = match (literal, derived) {
            (Some(list), _) => (list, None),
            (None, Some(d)) => (Vec::new(), Some(d)),
            (None, None) if style == MenuStyle::Checkbox => {
                // Checkbox default pair (decisions_locked checkbox arm).
                (vec![SpecValue::Bool(true), SpecValue::Bool(false)], None)
            }
            (None, None) => return (None, reasons),
        };

        (
            Some(Self {
                param_name,
                style,
                options,
                derived,
            }),
            reasons,
        )
    }
}

/// Interaction state for a menu-family widget, lifted out of GPUI so it is
/// testable without a window (the `SliderState` shape).
///
/// Transitions: `Closed → Open` on a menu-box click (menu style only;
/// radio/checkbox commit directly from `Closed`), `Open → Committed` on an
/// option click, `Open → Closed` on click-away (no dispatch),
/// `Committed → Closed` once [`commit_menu_release`] consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuState {
    /// At rest, showing the current value.
    Closed,
    /// The option list is open. `hover` is the option row under the pointer
    /// (overlay-only — never dispatched).
    Open {
        /// Hovered option index, if any.
        hover: Option<usize>,
    },
    /// An option was picked; the next [`commit_menu_release`] dispatches
    /// `options[index]` and returns to `Closed`.
    Committed {
        /// Index of the picked option into [`MenuBinding::options`].
        index: usize,
    },
}

impl MenuState {
    /// A menu-box click: `Closed → Open`, `Open → Closed` (toggle).
    /// `Committed` passes through untouched (mid-commit clicks are inert).
    ///
    /// COMPOSED WIRING NOTE: the gpui shim only reaches this from
    /// a box click when the menu RENDERED closed. A box click while open
    /// double-fires — gpui runs the popup's capture-phase `on_mouse_down_out`
    /// ([`Self::click_away`] → `Closed`) before the box's bubble-phase
    /// handler — so the shim closes from its render-time open snapshot
    /// instead of toggling the capture-mutated state (which would always
    /// re-open). See `diw_ac07_composed_box_click_closes_after_capture_click_away`.
    #[must_use]
    pub fn toggle_open(&self) -> Self {
        match self {
            Self::Closed => Self::Open { hover: None },
            Self::Open { .. } => Self::Closed,
            Self::Committed { .. } => self.clone(),
        }
    }

    /// Update the hovered option row. No-op unless `Open`.
    pub fn set_hover(&mut self, index: Option<usize>) {
        if let Self::Open { hover } = self {
            *hover = index;
        }
    }

    /// An option click: `Open → Committed` (the menu path) and
    /// `Closed → Committed` (radio/checkbox commit directly — their options
    /// are always visible). `Committed` passes through untouched.
    #[must_use]
    pub fn pick(&self, index: usize) -> Self {
        match self {
            Self::Closed | Self::Open { .. } => Self::Committed { index },
            Self::Committed { .. } => self.clone(),
        }
    }

    /// A click outside the open list: `Open → Closed` WITHOUT dispatch.
    /// Every other state passes through untouched.
    #[must_use]
    pub fn click_away(&self) -> Self {
        match self {
            Self::Open { .. } => Self::Closed,
            other => other.clone(),
        }
    }
}

/// Whether a checkbox renders checked: the current param value equals the
/// FIRST option (decisions_locked checkbox arm).
#[must_use]
pub fn checkbox_checked(options: &[SpecValue], current: Option<&SpecValue>) -> bool {
    matches!((options.first(), current), (Some(first), Some(cur)) if first == cur)
}

/// The option index a checkbox click commits: the OTHER option — index 1
/// when the current value is the first option, else index 0. `None` for a
/// degenerate (fewer than two options) binding, which assembly's
/// post-reconciliation rule already degrades to menu presentation.
#[must_use]
pub fn checkbox_toggle_index(options: &[SpecValue], current: Option<&SpecValue>) -> Option<usize> {
    if options.len() < 2 {
        return None;
    }
    Some(if checkbox_checked(options, current) { 1 } else { 0 })
}

/// Display label for an option value (and the menu box's current-value
/// text). Strings pass through; numerics/bools format naturally; anything
/// exotic falls back to its debug form (an authoring aid, not a crash).
#[must_use]
pub fn option_label(value: &SpecValue) -> String {
    match value {
        SpecValue::String(s) => s.clone(),
        SpecValue::Integer(i) => i.to_string(),
        SpecValue::Float(f) => f.to_string(),
        SpecValue::Bool(b) => b.to_string(),
        SpecValue::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

/// Pure dispatch helper, mirroring `commit_slider_release`. On a
/// `Committed` state, dispatches the SELECTED option's [`SpecValue`]
/// verbatim (a String stays a String — contrast the slider's hardcoded
/// `Float`) and returns to `Closed`.
///
/// `current` is the param's live value (read from the session, never a UI
/// mirror): picking the already-current option dispatches NOTHING — the
/// decisions_locked same-value no-op, so a repeat pick never re-queries.
/// Non-`Committed` states are pass-through no-ops; an out-of-range option
/// index quietly settles to `Closed` without dispatch.
pub fn commit_menu_release<D: ParamDispatcher>(
    state: &MenuState,
    binding: &MenuBinding,
    current: Option<&SpecValue>,
    dispatcher: &mut D,
) -> (
    MenuState,
    Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>,
) {
    if let MenuState::Committed { index } = *state {
        let Some(value) = binding.options.get(index) else {
            return (MenuState::Closed, Vec::new());
        };
        if current == Some(value) {
            // Same-value pick: no dispatch, no spurious re-query.
            return (MenuState::Closed, Vec::new());
        }
        let results = dispatcher.dispatch(&binding.param_name, value.clone());
        (MenuState::Closed, results)
    } else {
        (state.clone(), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::ParamRef;
    use brightfield_spec::vocab::ImplStatus;
    use indexmap::IndexMap;

    /// Recording test double — captures every dispatch call in order
    /// (the slider.rs RecordingDispatcher precedent).
    struct RecordingDispatcher {
        calls: Vec<(String, SpecValue)>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self { calls: Vec::new() }
        }
    }

    impl ParamDispatcher for RecordingDispatcher {
        fn dispatch(
            &mut self,
            name: &str,
            value: SpecValue,
        ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
            self.calls.push((name.to_string(), value));
            Vec::new()
        }
    }

    /// Build a fixture Input node mimicking
    /// ```yaml
    /// input: menu
    /// as: $region
    /// <extra options bag entries>
    /// ```
    /// (the slider.rs `input_fixture` precedent).
    fn menu_fixture(
        as_param: Option<&str>,
        from: Option<&str>,
        extras: &[(&str, SpecValue)],
    ) -> Input {
        let mut options = IndexMap::new();
        for (k, v) in extras {
            options.insert((*k).to_string(), ValueOrParamRef::Value(v.clone()));
        }
        Input {
            kind: InputKind::Menu,
            status: ImplStatus::Implemented,
            as_param: as_param.map(|p| ParamRef(p.to_string())),
            from_source: from.map(str::to_string),
            filter_by: None,
            options,
        }
    }

    fn str_options(vals: &[&str]) -> SpecValue {
        SpecValue::Array(vals.iter().map(|v| SpecValue::String((*v).to_string())).collect())
    }

    fn binding(style: MenuStyle, options: Vec<SpecValue>) -> MenuBinding {
        MenuBinding {
            param_name: "region".to_string(),
            style,
            options,
            derived: None,
        }
    }

    // a literal options list preserves each option's SpecValue
    // variant — a numeric option is NOT stringified.
    #[test]
    fn diw_ac01_literal_options_preserve_typing() {
        let input = menu_fixture(
            Some("k"),
            None,
            &[(
                "options",
                SpecValue::Array(vec![
                    SpecValue::String("east".to_string()),
                    SpecValue::Integer(7),
                    SpecValue::Float(2.5),
                    SpecValue::Bool(false),
                ]),
            )],
        );
        let (b, reasons) = MenuBinding::from_input(&input);
        let b = b.expect("constructs");
        assert!(reasons.is_empty(), "no degrade reasons: {reasons:?}");
        assert_eq!(b.param_name, "k");
        assert_eq!(b.style, MenuStyle::Menu);
        assert_eq!(
            b.options,
            vec![
                SpecValue::String("east".to_string()),
                SpecValue::Integer(7),
                SpecValue::Float(2.5),
                SpecValue::Bool(false),
            ],
            "option SpecValue variants preserved verbatim"
        );
        assert!(b.derived.is_none());
    }

    // style parsing — radio and checkbox read from the options bag;
    // an unknown style falls back to Menu WITH a surfaced reason (the widened
    // return carries the construction-time degrade decision).
    #[test]
    fn diw_ac01_style_parses_and_unknown_style_degrades_with_reason() {
        let radio = menu_fixture(
            Some("k"),
            None,
            &[
                ("style", SpecValue::String("radio".to_string())),
                ("options", str_options(&["a", "b"])),
            ],
        );
        let (b, reasons) = MenuBinding::from_input(&radio);
        assert_eq!(b.expect("constructs").style, MenuStyle::Radio);
        assert!(reasons.is_empty());

        let checkbox = menu_fixture(
            Some("k"),
            None,
            &[
                ("style", SpecValue::String("checkbox".to_string())),
                ("options", str_options(&["on", "off"])),
            ],
        );
        let (b, _) = MenuBinding::from_input(&checkbox);
        assert_eq!(b.expect("constructs").style, MenuStyle::Checkbox);

        let unknown = menu_fixture(
            Some("k"),
            None,
            &[
                ("style", SpecValue::String("dropdown".to_string())),
                ("options", str_options(&["a", "b"])),
            ],
        );
        let (b, reasons) = MenuBinding::from_input(&unknown);
        assert_eq!(
            b.expect("still a usable widget").style,
            MenuStyle::Menu,
            "unknown style falls back to menu presentation"
        );
        assert_eq!(reasons.len(), 1, "exactly one surfaced reason");
        assert!(
            reasons[0].contains("dropdown"),
            "the reason names the unknown style: {}",
            reasons[0]
        );

        // A NON-STRING style value (`style: 1`) takes the Some(other) arm:
        // same fallback to Menu, exactly one surfaced reason.
        let non_string = menu_fixture(
            Some("k"),
            None,
            &[
                ("style", SpecValue::Integer(1)),
                ("options", str_options(&["a", "b"])),
            ],
        );
        let (b, reasons) = MenuBinding::from_input(&non_string);
        assert_eq!(
            b.expect("still a usable widget").style,
            MenuStyle::Menu,
            "a non-string style falls back to menu presentation"
        );
        assert_eq!(reasons.len(), 1, "exactly one surfaced reason: {reasons:?}");
        assert!(
            reasons[0].contains("non-string style"),
            "the reason names the degrade class: {}",
            reasons[0]
        );
    }

    // options absent + from/column present records the Derived
    // marker (resolution happens at assembly, not here).
    #[test]
    fn diw_ac01_derived_marker_from_source_and_column() {
        let input = menu_fixture(
            Some("region"),
            Some("athletes"),
            &[("column", SpecValue::String("nationality".to_string()))],
        );
        let (b, reasons) = MenuBinding::from_input(&input);
        let b = b.expect("constructs");
        assert!(reasons.is_empty());
        assert!(b.options.is_empty(), "no options until assembly resolves");
        assert_eq!(
            b.derived,
            Some(DerivedOptions {
                source: "athletes".to_string(),
                column: "nationality".to_string(),
            })
        );
    }

    // missing `as:` yields no binding (slider parity).
    #[test]
    fn diw_ac01_missing_as_param_yields_none() {
        let input = menu_fixture(None, None, &[("options", str_options(&["a", "b"]))]);
        let (b, _) = MenuBinding::from_input(&input);
        assert!(b.is_none(), "no param target, no widget");
    }

    // no options and no derivable source yields None for style
    // menu/radio ONLY (checkbox synthesises — next test).
    #[test]
    fn diw_ac01_empty_options_none_for_menu_and_radio_only() {
        // No options key at all.
        let bare = menu_fixture(Some("k"), None, &[]);
        assert!(MenuBinding::from_input(&bare).0.is_none());

        // Explicitly empty list.
        let empty = menu_fixture(Some("k"), None, &[("options", SpecValue::Array(vec![]))]);
        assert!(MenuBinding::from_input(&empty).0.is_none());

        // Radio, empty list.
        let radio_empty = menu_fixture(
            Some("k"),
            None,
            &[
                ("style", SpecValue::String("radio".to_string())),
                ("options", SpecValue::Array(vec![])),
            ],
        );
        assert!(MenuBinding::from_input(&radio_empty).0.is_none());

        // `from:` without `column:` is not derivable.
        let no_column = menu_fixture(Some("k"), Some("t"), &[]);
        assert!(MenuBinding::from_input(&no_column).0.is_none());
    }

    // `style: checkbox` with `options:` absent synthesises the
    // default pair [Bool(true), Bool(false)] — a working binding, never a
    // silent drop (the flagship WHERE-flag shape).
    #[test]
    fn diw_ac01_checkbox_default_pair_synthesised() {
        let input = menu_fixture(
            Some("flag"),
            None,
            &[("style", SpecValue::String("checkbox".to_string()))],
        );
        let (b, reasons) = MenuBinding::from_input(&input);
        let b = b.expect("checkbox without options still constructs");
        assert!(reasons.is_empty());
        assert_eq!(b.style, MenuStyle::Checkbox);
        assert_eq!(
            b.options,
            vec![SpecValue::Bool(true), SpecValue::Bool(false)],
            "the synthesised default pair, in checked-first order"
        );
    }

    // exactly one dispatch per commit, the typed value verbatim
    // (a String stays a String), Committed → Closed.
    #[test]
    fn diw_ac02_commit_dispatches_typed_value_and_closes() {
        let b = binding(
            MenuStyle::Menu,
            vec![
                SpecValue::String("east".to_string()),
                SpecValue::String("west".to_string()),
            ],
        );
        let current = SpecValue::String("east".to_string());
        let state = MenuState::Committed { index: 1 };
        let mut d = RecordingDispatcher::new();

        let (next, _results) = commit_menu_release(&state, &b, Some(&current), &mut d);

        assert_eq!(d.calls.len(), 1, "exactly one dispatch per commit");
        let (name, value) = &d.calls[0];
        assert_eq!(name, "region");
        assert_eq!(
            value,
            &SpecValue::String("west".to_string()),
            "the SpecValue dispatches verbatim — String stays String"
        );
        assert_eq!(next, MenuState::Closed);
    }

    // an Integer option dispatches as Integer (variant identity is
    // load-bearing at SQL emit).
    #[test]
    fn diw_ac02_integer_option_dispatches_as_integer() {
        let b = binding(
            MenuStyle::Menu,
            vec![SpecValue::Integer(1), SpecValue::Integer(2)],
        );
        let mut d = RecordingDispatcher::new();
        let (_, _) = commit_menu_release(
            &MenuState::Committed { index: 1 },
            &b,
            Some(&SpecValue::Integer(1)),
            &mut d,
        );
        assert_eq!(d.calls[0].1, SpecValue::Integer(2));
    }

    // selecting the already-current value dispatches NOTHING
    // (decisions_locked same-value no-op), still settling to Closed.
    #[test]
    fn diw_ac02_same_value_pick_is_a_no_op() {
        let b = binding(
            MenuStyle::Menu,
            vec![
                SpecValue::String("east".to_string()),
                SpecValue::String("west".to_string()),
            ],
        );
        let current = SpecValue::String("west".to_string());
        let mut d = RecordingDispatcher::new();

        let (next, results) =
            commit_menu_release(&MenuState::Committed { index: 1 }, &b, Some(&current), &mut d);

        assert!(d.calls.is_empty(), "same-value pick must not dispatch");
        assert!(results.is_empty());
        assert_eq!(next, MenuState::Closed);
    }

    // non-committed states are pass-through no-ops.
    #[test]
    fn diw_ac02_non_committed_states_no_dispatch() {
        let b = binding(MenuStyle::Menu, vec![SpecValue::Bool(true)]);
        let mut d = RecordingDispatcher::new();

        for state in [MenuState::Closed, MenuState::Open { hover: Some(0) }] {
            let (next, results) = commit_menu_release(&state, &b, None, &mut d);
            assert!(d.calls.is_empty(), "{state:?} must not dispatch");
            assert!(results.is_empty());
            assert_eq!(next, state, "non-Committed state passes through");
        }

        // Out-of-range option index: no dispatch, settles Closed.
        let (next, results) =
            commit_menu_release(&MenuState::Committed { index: 9 }, &b, None, &mut d);
        assert!(d.calls.is_empty());
        assert!(results.is_empty());
        assert_eq!(next, MenuState::Closed);
    }

    // the checkbox toggles between exactly its two option values —
    // checked iff current == FIRST option, a click dispatches the OTHER.
    #[test]
    fn diw_ac02_checkbox_toggles_between_its_two_options() {
        let opts = vec![SpecValue::Bool(true), SpecValue::Bool(false)];
        let b = binding(MenuStyle::Checkbox, opts.clone());
        let mut d = RecordingDispatcher::new();

        // current == first (checked) → click commits the OTHER (index 1).
        assert!(checkbox_checked(&opts, Some(&SpecValue::Bool(true))));
        let i = checkbox_toggle_index(&opts, Some(&SpecValue::Bool(true))).unwrap();
        assert_eq!(i, 1);
        commit_menu_release(
            &MenuState::Committed { index: i },
            &b,
            Some(&SpecValue::Bool(true)),
            &mut d,
        );
        assert_eq!(d.calls.last().unwrap().1, SpecValue::Bool(false));

        // current == second (unchecked) → click commits the first.
        assert!(!checkbox_checked(&opts, Some(&SpecValue::Bool(false))));
        let i = checkbox_toggle_index(&opts, Some(&SpecValue::Bool(false))).unwrap();
        assert_eq!(i, 0);
        commit_menu_release(
            &MenuState::Committed { index: i },
            &b,
            Some(&SpecValue::Bool(false)),
            &mut d,
        );
        assert_eq!(d.calls.last().unwrap().1, SpecValue::Bool(true));

        assert_eq!(d.calls.len(), 2, "two toggles, two dispatches");

        // A degenerate single-option binding has no toggle target.
        assert!(checkbox_toggle_index(&opts[..1], Some(&SpecValue::Bool(true))).is_none());
    }

    // Shim decision logic, extracted pure: open / pick /
    // click-away transitions.
    #[test]
    fn diw_ac07_state_transitions_open_pick_click_away() {
        // Menu box click toggles open, then closed.
        let s = MenuState::Closed.toggle_open();
        assert_eq!(s, MenuState::Open { hover: None });
        assert_eq!(s.toggle_open(), MenuState::Closed);

        // Hover tracks only while open.
        let mut s = MenuState::Open { hover: None };
        s.set_hover(Some(2));
        assert_eq!(s, MenuState::Open { hover: Some(2) });
        let mut closed = MenuState::Closed;
        closed.set_hover(Some(1));
        assert_eq!(closed, MenuState::Closed, "hover is a no-op when closed");

        // An option click commits from Open (menu) AND from Closed
        // (radio/checkbox — their options are always visible).
        assert_eq!(s.pick(1), MenuState::Committed { index: 1 });
        assert_eq!(MenuState::Closed.pick(0), MenuState::Committed { index: 0 });

        // Click-away closes an open list WITHOUT dispatch; other states pass.
        assert_eq!(MenuState::Open { hover: Some(1) }.click_away(), MenuState::Closed);
        assert_eq!(
            MenuState::Committed { index: 1 }.click_away(),
            MenuState::Committed { index: 1 }
        );
    }

    // Composed shim semantics: one physical click on the OPEN
    // menu's box double-fires in gpui — the popup's on_mouse_down_out runs
    // in the CAPTURE phase (click_away → Closed) BEFORE the box's
    // BUBBLE-phase on_mouse_down. The box decision must therefore come from
    // the RENDER-TIME open snapshot (`was_open`), never a toggle_open()
    // re-read of the capture-mutated gesture — the re-read would flip the
    // now-Closed state straight back to Open, leaving the box click unable
    // to ever close the list (the ▴ chevron's advertised toggle-to-close).
    #[test]
    fn diw_ac07_composed_box_click_closes_after_capture_click_away() {
        // Render time: the list is open; the box renders was_open = true.
        let rendered = MenuState::Open { hover: None };
        let was_open = matches!(rendered, MenuState::Open { .. });

        // Capture phase: the popup's click-away fires first (the box sits
        // outside the popup's hitbox).
        let after_capture = rendered.click_away();
        assert_eq!(after_capture, MenuState::Closed);

        // Bubble phase: the box decides from the snapshot (the shim's rule).
        let after_bubble = if was_open {
            MenuState::Closed
        } else {
            after_capture.toggle_open()
        };
        assert_eq!(
            after_bubble,
            MenuState::Closed,
            "the open box click must CLOSE the list — a toggle_open() re-read \
             of the capture-mutated state would wrongly yield Open"
        );

        // The closed-box half of the same wiring: no popup is rendered, so
        // no capture handler fires; the snapshot path still opens the list.
        let rendered = MenuState::Closed;
        let was_open = matches!(rendered, MenuState::Open { .. });
        let after_bubble = if was_open {
            MenuState::Closed
        } else {
            rendered.toggle_open()
        };
        assert_eq!(after_bubble, MenuState::Open { hover: None });
    }

    // Option labels: strings pass through, numerics/bools format naturally.
    #[test]
    fn option_label_formats_scalar_variants() {
        assert_eq!(option_label(&SpecValue::String("east".to_string())), "east");
        assert_eq!(option_label(&SpecValue::Integer(7)), "7");
        assert_eq!(option_label(&SpecValue::Float(2.5)), "2.5");
        assert_eq!(option_label(&SpecValue::Bool(true)), "true");
    }
}
