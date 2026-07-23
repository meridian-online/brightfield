//! Assembly-time menu options resolution + default reconciliation.
//!
//! Runs on BOTH the launch path and the watcher/reload rebuild path (both
//! flow through `build_everything`), against the pass's own `Session`:
//! literal options pass through; derived menus resolve via the read-only
//! `Session::distinct_values` (capped, launch-fixed — slider parity); the
//! param's current value is prepended when absent (the slider-clamp
//! analogue INVERTED: the author's chosen value is legitimate, so the LIST
//! moves, never the param); and the post-reconciliation style rules degrade
//! checkbox/radio to menu presentation honestly. Every degrade path yields
//! exactly one runtime Log Warning per assembly pass and a USABLE widget;
//! only a per-input resolution FAILURE skips its widget — isolated, every
//! other widget and plot unaffected (the profile precedent).

use brightfield_engine::Session;
use brightfield_spec::ast::Spec;
use brightfield_spec::layout::{placed_input_nodes, Rect};
use brightfield_spec::vocab::InputKind;
use brightfield_ui::{MenuBinding, MenuStyle};

/// Cap on derived option lists (decisions_locked): beyond it the list
/// truncates with a runtime Log Warning — no scrollable popup in v1.
pub const MENU_OPTIONS_CAP: usize = 50;

/// One resolved menu-family widget placement: its dashboard rect, the fully
/// resolved binding (literal options, final — possibly degraded — style),
/// and the value the resting widget displays.
pub struct MenuPlacement {
    /// The layout rect (per-style sized).
    pub rect: Rect,
    /// The resolved binding the coordinator + hosted element share.
    pub binding: MenuBinding,
    /// The current display value: the param's value after reconciliation,
    /// or the first option when the param is undeclared.
    pub value: brightfield_spec::ast::SpecValue,
}

/// Resolve every `input: menu` in `spec` into hosted placements, returning
/// the per-pass resolution warnings alongside (transported to the Log dock
/// by the caller — appended BEFORE the reload gates on the watcher path).
///
/// Read-only on the session (`distinct_values` + `current_params`): the
/// param is never snapped — reconciliation PREPENDS a missing default to
/// the list instead (decisions_locked), running BEFORE style finalisation
/// so the checkbox/radio degrade rules judge the post-prepend count.
pub fn resolve_menu_placements(
    spec: &Spec,
    session: &Session,
) -> (Vec<MenuPlacement>, Vec<String>) {
    let mut placements = Vec::new();
    let mut warnings = Vec::new();

    for (rect, input) in placed_input_nodes(spec, Rect::new(0.0, 0.0, 0.0, 0.0)) {
        if input.kind != InputKind::Menu {
            continue;
        }
        // Construction: the framework-free model is the single source of the
        // construction-time degrade decisions (unknown style) — assembly
        // only logs what it surfaced.
        let (binding, reasons) = MenuBinding::from_input(input);
        warnings.extend(reasons);
        let Some(mut binding) = binding else {
            continue; // no param target / no options source — slider parity
        };

        // Layout-time literal row count, BEFORE reconciliation can prepend:
        // the count layout_component reserved rows for. Radio's
        // post-prepend rule compares against this.
        let layout_n = binding.options.len();

        // Derived options resolve static-at-load through the engine.
        if let Some(d) = binding.derived.clone() {
            match session.distinct_values(&d.source, &d.column, MENU_OPTIONS_CAP) {
                Ok(dv) => {
                    if dv.truncated {
                        warnings.push(format!(
                            "menu '{}': options truncated at {MENU_OPTIONS_CAP} distinct \
                             values of {}.{}",
                            binding.param_name, d.source, d.column
                        ));
                    }
                    binding.options = dv.values;
                }
                Err(e) => {
                    // Per-input failure isolation: warn, skip THIS widget,
                    // leave every other widget and plot untouched.
                    warnings.push(format!(
                        "menu '{}': options resolution failed — widget skipped ({e})",
                        binding.param_name
                    ));
                    continue;
                }
            }
        }

        // Default reconciliation (decisions_locked): the param's current
        // value, when absent from the resolved OR literal list, is PREPENDED
        // — strict-variant SpecValue equality, no numeric cross-variant
        // coercion; the param itself is never snapped.
        let current = session.current_params().get(&binding.param_name).cloned();
        if let Some(v) = &current {
            if !binding.options.contains(v) {
                binding.options.insert(0, v.clone());
            }
        }

        if binding.options.is_empty() {
            // A derived column of all NULLs with no declared param default:
            // nothing to show and nothing to dispatch.
            warnings.push(format!(
                "menu '{}': resolved no options — widget skipped",
                binding.param_name
            ));
            continue;
        }

        // Style finalisation — AFTER reconciliation, so the rules judge the
        // post-prepend option count (decisions_locked ordering). Each
        // degrade yields exactly one warning and a usable menu.
        match binding.style {
            MenuStyle::Radio if binding.derived.is_some() => {
                // Unknown N at layout time (layout.rs is a pure spec fn) —
                // the rect was menu-sized, so the presentation follows.
                warnings.push(format!(
                    "menu '{}': style: radio needs a literal options list \
                     (derived options have unknown row count at layout time) — \
                     showing a menu",
                    binding.param_name
                ));
                binding.style = MenuStyle::Menu;
            }
            MenuStyle::Radio if binding.options.len() > layout_n => {
                // The prepend added a row layout_component never reserved
                // (it cannot see the params block) — degrade rather than
                // overflow the 22·N + RADIO_CHROME_PAD rect. Fires only on
                // the slider-clamp authoring-inconsistency class.
                warnings.push(format!(
                    "menu '{}': param default {} is absent from the declared radio \
                     options (prepended) — the layout reserved {layout_n} rows, \
                     showing a menu",
                    binding.param_name,
                    current
                        .as_ref()
                        .map(brightfield_ui::option_label)
                        .unwrap_or_default()
                ));
                binding.style = MenuStyle::Menu;
            }
            MenuStyle::Checkbox if binding.options.len() != 2 => {
                // A foreign default over [true, false] becomes an honest
                // 3-option menu showing it selected (decisions_locked).
                warnings.push(format!(
                    "menu '{}': style: checkbox needs exactly two options \
                     (found {} after reconciliation) — showing a menu",
                    binding.param_name,
                    binding.options.len()
                ));
                binding.style = MenuStyle::Menu;
            }
            _ => {}
        }

        let value = match current {
            Some(v) => v,
            None => binding.options[0].clone(),
        };
        placements.push(MenuPlacement {
            rect,
            binding,
            value,
        });
    }

    (placements, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_engine::Engine;
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::ast::SpecValue;
    use brightfield_spec::{parse_spec, Format};

    /// Parse + load a spec into a live session (the assembly context).
    fn session_for(yaml: &str) -> (Spec, Session) {
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let session = Engine::new()
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load")
            .session;
        (parsed.spec, session)
    }

    /// Shared data + plot prelude; each fixture appends its input under the
    /// SAME vconcat (one root component).
    const DATA_BLOCK: &str = r#"
data:
  t:
    - { x: 1, region: east, n: 1 }
    - { x: 2, region: west, n: 2 }
    - { x: 3, region: north, n: 3 }
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: x
        y: x
"#;

    /// a derived menu resolves its options from the column's
    /// distinct values (ordered), and the declared default — present in the
    /// column — is NOT prepended (strict equality found it).
    #[test]
    fn derived_options_resolve_from_column() {
        let yaml = format!(
            r#"
params:
  region: east
{DATA_BLOCK}  - input: menu
    as: $region
    from: t
    column: region
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert!(warnings.is_empty(), "clean resolve: {warnings:?}");
        assert_eq!(placements.len(), 1);
        let b = &placements[0].binding;
        assert_eq!(
            b.options,
            vec![
                SpecValue::String("east".to_string()),
                SpecValue::String("north".to_string()),
                SpecValue::String("west".to_string()),
            ],
            "distinct values, ordered, no prepend (default already present)"
        );
        assert_eq!(placements[0].value, SpecValue::String("east".to_string()));
    }

    /// prepend-on-absent for BOTH a derived list (param default not
    /// in the column) AND a literal list omitting the default — the param is
    /// never snapped, the LIST moves.
    #[test]
    fn default_reconciliation_prepends_for_derived_and_literal() {
        // Derived: default "everywhere" is in no row.
        let yaml = format!(
            r#"
params:
  region: everywhere
{DATA_BLOCK}  - input: menu
    as: $region
    from: t
    column: region
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, _) = resolve_menu_placements(&spec, &session);
        assert_eq!(
            placements[0].binding.options[0],
            SpecValue::String("everywhere".to_string()),
            "the absent default is PREPENDED to the derived list"
        );
        assert_eq!(
            session.current_params().get("region"),
            Some(&SpecValue::String("everywhere".to_string())),
            "the param itself is never snapped"
        );

        // Literal: options omit the default.
        let yaml = format!(
            r#"
params:
  region: elsewhere
{DATA_BLOCK}  - input: menu
    as: $region
    options: [east, west]
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(
            placements[0].binding.options,
            vec![
                SpecValue::String("elsewhere".to_string()),
                SpecValue::String("east".to_string()),
                SpecValue::String("west".to_string()),
            ]
        );
        assert!(
            warnings.is_empty(),
            "a plain-menu prepend is not a degrade: {warnings:?}"
        );
    }

    /// strict-variant SpecValue equality — an Integer default
    /// against a derived same-typed Integer list compares EQUAL (no prepend),
    /// with no cross-variant coercion in sight.
    #[test]
    fn strict_variant_equality_integer_default() {
        let yaml = format!(
            r#"
params:
  pick: 2
{DATA_BLOCK}  - input: menu
    as: $pick
    from: t
    column: n
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert!(warnings.is_empty());
        assert_eq!(
            placements[0].binding.options,
            vec![
                SpecValue::Integer(1),
                SpecValue::Integer(2),
                SpecValue::Integer(3)
            ],
            "Integer(2) == Integer(2) strict-variant — the default was found, no prepend"
        );
    }

    /// per-input failure isolation — a menu on a broken
    /// column earns one warning and is skipped; the sibling widgets resolve
    /// untouched.
    #[test]
    fn failure_isolation_broken_column_skips_one_widget() {
        let yaml = format!(
            r#"
params:
  region: east
  flag: true
{DATA_BLOCK}  - input: menu
    as: $region
    from: t
    column: no_such_column
  - input: menu
    style: checkbox
    as: $flag
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(
            placements.len(),
            1,
            "the broken widget is skipped, the sibling lives"
        );
        assert_eq!(placements[0].binding.style, MenuStyle::Checkbox);
        assert_eq!(warnings.len(), 1, "exactly one warning: {warnings:?}");
        assert!(
            warnings[0].contains("region") && warnings[0].contains("skipped"),
            "the warning names the widget and the outcome: {}",
            warnings[0]
        );
    }

    /// unknown style value → menu presentation + exactly one
    /// warning + a usable widget (the model's surfaced reason, logged once).
    #[test]
    fn unknown_style_degrades_to_menu_with_one_warning() {
        let yaml = format!(
            r#"
params:
  region: east
{DATA_BLOCK}  - input: menu
    style: dropdown
    as: $region
    options: [east, west]
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(placements.len(), 1, "the widget survives");
        assert_eq!(placements[0].binding.style, MenuStyle::Menu);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("dropdown"), "{}", warnings[0]);
    }

    /// style: checkbox with ≠2 options POST-RECONCILIATION — a
    /// foreign default 'maybe' over [true, false] becomes an honest 3-option
    /// menu showing 'maybe' selected (the decisions_locked ordering: the
    /// prepend runs FIRST, then the count rule judges 3).
    #[test]
    fn checkbox_foreign_default_degrades_post_reconciliation() {
        let yaml = format!(
            r#"
params:
  flag: maybe
{DATA_BLOCK}  - input: menu
    style: checkbox
    as: $flag
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(placements.len(), 1);
        let p = &placements[0];
        assert_eq!(p.binding.style, MenuStyle::Menu, "degraded presentation");
        assert_eq!(
            p.binding.options,
            vec![
                SpecValue::String("maybe".to_string()),
                SpecValue::Bool(true),
                SpecValue::Bool(false),
            ],
            "prepend first, THEN the ≠2 rule fires — an honest 3-option menu"
        );
        assert_eq!(
            p.value,
            SpecValue::String("maybe".to_string()),
            "'maybe' shows selected"
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("exactly two options"),
            "{}",
            warnings[0]
        );
    }

    /// a well-formed checkbox (declared [true, false] default pair,
    /// bool param) does NOT degrade — the flagship shape stays a checkbox.
    #[test]
    fn wellformed_checkbox_stays_checkbox() {
        let yaml = format!(
            r#"
params:
  flag: false
{DATA_BLOCK}  - input: menu
    style: checkbox
    as: $flag
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(placements[0].binding.style, MenuStyle::Checkbox);
        assert_eq!(placements[0].value, SpecValue::Bool(false));
    }

    /// The v2-review composition case: literal radio [a, b, c] +
    /// absent param default d → the prepend makes 4 options where layout
    /// reserved 3 rows → menu presentation + one warning. The geometry never
    /// overflows the 22·N + RADIO_CHROME_PAD rect.
    #[test]
    fn radio_prepend_exceeding_layout_rows_degrades() {
        let yaml = format!(
            r#"
params:
  kind: d
{DATA_BLOCK}  - input: menu
    style: radio
    as: $kind
    options: [a, b, c]
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(placements.len(), 1);
        let p = &placements[0];
        assert_eq!(
            p.binding.style,
            MenuStyle::Menu,
            "radio degrades — layout reserved 3 rows"
        );
        assert_eq!(
            p.binding.options.len(),
            4,
            "the prepended default rides along"
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("reserved 3 rows"), "{}", warnings[0]);
        // The layout rect stayed radio-tall for 3 literal rows — cosmetic
        // slack for the hosted 32px menu, never overlap (decisions_locked).
        use brightfield_spec::layout::{RADIO_CHROME_PAD, RADIO_ROW_HEIGHT};
        assert_eq!(p.rect.height, RADIO_ROW_HEIGHT * 3.0 + RADIO_CHROME_PAD);
    }

    /// style: radio with derived options → menu
    /// presentation (menu-sized rect) + one warning.
    #[test]
    fn derived_radio_degrades_to_menu() {
        let yaml = format!(
            r#"
params:
  region: east
{DATA_BLOCK}  - input: menu
    style: radio
    as: $region
    from: t
    column: region
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].binding.style, MenuStyle::Menu);
        assert_eq!(placements[0].rect.height, 32.0, "menu-sized");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("literal options list"),
            "{}",
            warnings[0]
        );
    }

    /// the resolution warnings land in the FeedbackLog as exactly
    /// one Warning entry each (the log_model append precedent) — the same
    /// entries both the launch pass and the watcher pass append, mapped
    /// through `resolution_warning_entries`.
    #[test]
    fn warnings_append_to_feedback_log_once_each() {
        let yaml = format!(
            r#"
params:
  region: east
{DATA_BLOCK}  - input: menu
    style: dropdown
    as: $region
    options: [east, west]
"#
        );
        let (spec, session) = session_for(&yaml);
        let (_placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(warnings.len(), 1);

        let mut log = crate::log_model::FeedbackLog::default();
        for (severity, message) in crate::log_model::resolution_warning_entries(&warnings) {
            log.append(severity, message);
        }
        assert_eq!(
            log.entries().len(),
            1,
            "exactly one entry per warning per pass"
        );
        assert!(matches!(
            log.entries()[0].severity,
            crate::log_model::Severity::Warning
        ));
        assert_eq!(log.entries()[0].message, warnings[0]);
    }

    /// a derived list truncated at the cap keeps a usable
    /// (truncated) menu + one warning. 60 distinct values → 50 options.
    #[test]
    fn cap_truncation_warns_and_keeps_widget() {
        let rows: String = (0..60)
            .map(|i| format!("    - {{ x: {i}, region: \"r{i:02}\", n: {i} }}\n"))
            .collect();
        let yaml = format!(
            r#"
params:
  region: r00
data:
  t:
{rows}
vconcat:
  - hconcat:
      - plot:
          - {{ mark: dot, data: {{ from: t }}, x: x, y: x }}
      - input: menu
        as: $region
        from: t
        column: region
"#
        );
        let (spec, session) = session_for(&yaml);
        let (placements, warnings) = resolve_menu_placements(&spec, &session);
        assert_eq!(placements.len(), 1, "the truncated widget survives");
        assert_eq!(placements[0].binding.options.len(), MENU_OPTIONS_CAP);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("truncated at 50"),
            "the warning names the cap: {}",
            warnings[0]
        );
    }
}
