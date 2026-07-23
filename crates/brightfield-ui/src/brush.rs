//! Brush-to-predicate adapter, the release/click commit path, and the
//! selection dispatch abstraction.
//!
//! Converts a chart-coordinate brush rectangle into a Predicate IR value
//! that the runtime selection coordinator can store and resolve, provides
//! a small dispatch trait so a host shell can route brush-release into a
//! Session without depending on the engine at the call site (the trait
//! keeps the test double cheap), and carries the framework-free commit
//! helpers ([`commit_brush_release_multi`], [`commit_click_multi`],
//! [`commit_brush_clear`]) a mouse-up gesture drives through that trait.

use brightfield_engine::{DispatchResult, RecordBatch};
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::nearest::{
    band_category_at, column_typed_value_at, find_nearest, NearestMode, SelectionValue,
};
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_spec::analysis::{BrushableBinding, ComponentPath};
use brightfield_sql::ir::{ClauseMeta, Predicate, ScalarValue};
use kurbo::Rect;

use crate::interaction::InteractionState;

/// Selection kind for a brush — mirrors the corresponding
/// [`brightfield_spec::vocab::InteractorKind`] variants. Kept as an
/// independent enum so this module does not import the spec vocab and
/// can be exercised purely from kurbo + brightfield-sql.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushKind {
    /// X-only interval brush (intervalX).
    IntervalX,
    /// Y-only interval brush (intervalY).
    IntervalY,
    /// 2D interval brush (intervalXY).
    IntervalXY,
    /// Point selection — a single (column, value) equality predicate
    /// produced by chart-side click-to-point or input-widget-driven
    /// selections (v3 surface). v3 lands the variant + adapter
    /// only; no rect-driven dispatch path is wired (decision 2).
    Point,
    /// X-channel point selection (toggleX) — `x = <clicked value>`.
    PointX,
    /// Y-channel point selection (toggleY) — `y = <clicked value>`.
    PointY,
}

/// Channel column names bound by the brushing plot — i.e. the SQL
/// column expressions that the brush coordinates compare against.
///
/// `x` is required for `IntervalX`/`IntervalXY`; `y` is required for
/// `IntervalY`/`IntervalXY`. A missing channel for the kind in question
/// causes the adapter to fall back to `Predicate::True` (no-op brush).
#[derive(Debug, Clone, Default)]
pub struct ChannelColumns {
    /// Column expression for the x channel.
    pub x: Option<String>,
    /// Column expression for the y channel.
    pub y: Option<String>,
}

impl ChannelColumns {
    /// Construct a ChannelColumns with both x and y bound.
    pub fn xy(x: impl Into<String>, y: impl Into<String>) -> Self {
        Self {
            x: Some(x.into()),
            y: Some(y.into()),
        }
    }

    /// Construct a ChannelColumns with only the x channel bound.
    pub fn x_only(x: impl Into<String>) -> Self {
        Self {
            x: Some(x.into()),
            y: None,
        }
    }

    /// Construct a ChannelColumns with only the y channel bound.
    pub fn y_only(y: impl Into<String>) -> Self {
        Self {
            x: None,
            y: Some(y.into()),
        }
    }
}

/// Convert a brush rectangle (in chart data coordinates) into a
/// Predicate. The rect's `x0..x1` is the inclusive x-range, `y0..y1`
/// the inclusive y-range.
///
/// - `IntervalX` → `Predicate::And([x >= rect.x0, x <= rect.x1])`
/// - `IntervalY` → `Predicate::And([y >= rect.y0, y <= rect.y1])`
/// - `IntervalXY` → `Predicate::And` of all four bounds, in order x-min,
///   x-max, y-min, y-max.
///
/// If the requested kind needs a channel that is not bound in
/// `channels`, the adapter returns `Predicate::True` (degenerate brush).
pub fn brush_rect_to_predicate(
    rect: Rect,
    kind: BrushKind,
    channels: &ChannelColumns,
) -> Predicate {
    match kind {
        BrushKind::IntervalX => match channels.x.as_deref() {
            Some(col) => x_range_predicate(col, rect.x0, rect.x1),
            None => Predicate::True,
        },
        BrushKind::IntervalY => match channels.y.as_deref() {
            Some(col) => y_range_predicate(col, rect.y0, rect.y1),
            None => Predicate::True,
        },
        BrushKind::IntervalXY => match (channels.x.as_deref(), channels.y.as_deref()) {
            (Some(xc), Some(yc)) => {
                let x_pred = x_range_predicate(xc, rect.x0, rect.x1);
                let y_pred = y_range_predicate(yc, rect.y0, rect.y1);
                // Flatten And-of-And into a single And so callers see a
                // four-clause predicate (matches the AC's "all four
                // bounds" expectation rather than a nested binary tree).
                let mut clauses = match x_pred {
                    Predicate::And(v) => v,
                    other => vec![other],
                };
                match y_pred {
                    Predicate::And(v) => clauses.extend(v),
                    other => clauses.push(other),
                }
                Predicate::And(clauses)
            }
            _ => Predicate::True,
        },
        // Point selections are not produced from a rect — callers use
        // [`point_to_predicate`] / [`point_predicate`] directly. The
        // rect-to-predicate path returns a degenerate `True` for completeness.
        BrushKind::Point | BrushKind::PointX | BrushKind::PointY => Predicate::True,
    }
}

/// Convert a resolved point-selection value into an equality predicate. `PointX`
/// compares the plot's x column, `PointY` its y column, to `value`. The
/// [`SelectionValue`] formats a type-correct literal: a bare number/integer, a
/// quoted+escaped string for a categorical axis, or `make_timestamp(us)` for a
/// temporal axis. A missing channel for the kind, or a non-point kind, yields
/// `Predicate::True` (degenerate).
///
/// This is the data-path adapter for `toggleX`/`toggleY` selections: the value is
/// resolved from the click (nearest datum on a continuous axis; nearest category
/// on a band axis) by the window click gesture.
pub fn point_to_predicate(
    value: &SelectionValue,
    kind: BrushKind,
    channels: &ChannelColumns,
) -> Predicate {
    let column = match kind {
        BrushKind::PointX => channels.x.as_deref(),
        BrushKind::PointY => channels.y.as_deref(),
        _ => None,
    };
    match column {
        Some(col) => point_predicate(col, &value.literal()),
        None => Predicate::True,
    }
}

/// Convert a (column, value) pair into a `Predicate::Expr` of shape
/// `column = value`, where `value` is treated as an **already-formatted SQL
/// literal** — the helper does not quote it. Callers pass `'Athletics'`
/// (with quotes) for a string match or `42` (no quotes) for a number.
///
/// This is the v3 forward-compat adapter for input-widget-driven point
/// selections (v3); chart-side click-to-point dispatch is
/// deferred (decision 2).
pub fn point_predicate(column: &str, value: &str) -> Predicate {
    Predicate::Expr(format!("{column} = {value}"))
}

// ---------------------------------------------------------------------------
// Structured clause constructors — the lossless channel
// ---------------------------------------------------------------------------
//
// The adapters above flatten the structured brush (rect, channels, resolved
// values) into opaque SQL strings the moment a gesture commits. The
// constructors below build the structured `Predicate::Interval` /
// `Predicate::Point` clauses instead, carrying the column, typed bounds or
// values, and optional scale/pixel metadata through to the query layer
// without erasure — while emitting SQL equivalent to the string forms (see
// their `ir.rs` docs for the exact byte-level rendering contract). The string
// path above REMAINS the default; these are the opt-in structured producers.

/// Convert a resolved [`SelectionValue`] into the IR's typed
/// [`ScalarValue`], preserving the value's type. `ScalarValue::to_sql_literal`
/// reproduces [`SelectionValue::literal`]'s text exactly, so a structured
/// clause built from this conversion renders the identical literal the string
/// path interpolates.
#[must_use]
pub fn scalar_value_from(value: &SelectionValue) -> ScalarValue {
    match value {
        SelectionValue::Number(n) => ScalarValue::Float(*n),
        SelectionValue::Int(i) => ScalarValue::Int(*i),
        SelectionValue::Text(s) => ScalarValue::Text(s.clone()),
        SelectionValue::Timestamp(us) => ScalarValue::TimestampMicros(*us),
        SelectionValue::TimestampTz(us) => ScalarValue::TimestampTzMicros(*us),
    }
}

/// Structured counterpart of [`brush_rect_to_predicate`]: the same rect /
/// kind / channel resolution, producing `Predicate::Interval` clauses that
/// keep the column and bounds machine-readable. Per-axis metadata is optional
/// (`None` where the caller has no scale context yet — the clause is no less
/// valid without it).
///
/// - `IntervalX` → `Interval` on the x channel over `rect.x0..rect.x1`.
/// - `IntervalY` → `Interval` on the y channel over `rect.y0..rect.y1`.
/// - `IntervalXY` → `And([Interval(x), Interval(y)])` — one structured clause
///   per axis, so each stays independently readable downstream.
/// - Missing channels and point kinds degrade to `Predicate::True`, exactly
///   like the string adapter.
pub fn brush_rect_to_structured(
    rect: Rect,
    kind: BrushKind,
    channels: &ChannelColumns,
    x_meta: Option<ClauseMeta>,
    y_meta: Option<ClauseMeta>,
) -> Predicate {
    match kind {
        BrushKind::IntervalX => match channels.x.as_deref() {
            Some(col) => interval_clause(col, rect.x0, rect.x1, x_meta),
            None => Predicate::True,
        },
        BrushKind::IntervalY => match channels.y.as_deref() {
            Some(col) => interval_clause(col, rect.y0, rect.y1, y_meta),
            None => Predicate::True,
        },
        BrushKind::IntervalXY => match (channels.x.as_deref(), channels.y.as_deref()) {
            (Some(xc), Some(yc)) => Predicate::And(vec![
                interval_clause(xc, rect.x0, rect.x1, x_meta),
                interval_clause(yc, rect.y0, rect.y1, y_meta),
            ]),
            _ => Predicate::True,
        },
        BrushKind::Point | BrushKind::PointX | BrushKind::PointY => Predicate::True,
    }
}

/// Structured counterpart of [`point_to_predicate`]: the same kind / channel
/// resolution, producing a single-value `Predicate::Point` clause whose typed
/// value survives to the query layer. A missing channel for the kind, or a
/// non-point kind, yields `Predicate::True` (degenerate), exactly like the
/// string adapter.
pub fn point_to_structured(
    value: &SelectionValue,
    kind: BrushKind,
    channels: &ChannelColumns,
    meta: Option<ClauseMeta>,
) -> Predicate {
    let column = match kind {
        BrushKind::PointX => channels.x.as_deref(),
        BrushKind::PointY => channels.y.as_deref(),
        _ => None,
    };
    match column {
        Some(col) => Predicate::Point {
            column: col.to_string(),
            values: vec![scalar_value_from(value)],
            meta,
        },
        None => Predicate::True,
    }
}

fn interval_clause(col: &str, lo: f64, hi: f64, meta: Option<ClauseMeta>) -> Predicate {
    Predicate::Interval {
        column: col.to_string(),
        lo: ScalarValue::Float(lo),
        hi: ScalarValue::Float(hi),
        meta,
    }
}

/// Trait abstracting the selection-dispatch surface ChartView calls
/// when a brush release commits. The real implementation forwards to
/// `Session::propagate_selection`. Tests substitute a recording double.
pub trait SelectionDispatcher {
    /// Dispatch a brushed selection. Returns one (mark_index, Result)
    /// tuple per subscriber, mirroring `Session::propagate_selection`.
    fn dispatch(
        &mut self,
        name: &str,
        contributor: ComponentPath,
        predicate: Predicate,
    ) -> Vec<DispatchResult>;

    /// Retract a contributor's predicate from the named selection — the
    /// click-outside-active-brush path. Mirrors
    /// `Session::clear_selection`'s shape: returns one
    /// `(mark_index, Result)` per subscriber that re-executes against the
    /// reduced selection state.
    fn clear(&mut self, name: &str, contributor: ComponentPath) -> Vec<DispatchResult>;
}

impl SelectionDispatcher for brightfield_engine::Session {
    fn dispatch(
        &mut self,
        name: &str,
        contributor: ComponentPath,
        predicate: Predicate,
    ) -> Vec<DispatchResult> {
        self.propagate_selection(name, contributor, predicate)
    }

    fn clear(&mut self, name: &str, contributor: ComponentPath) -> Vec<DispatchResult> {
        self.clear_selection(name, contributor)
    }
}

fn x_range_predicate(col: &str, lo: f64, hi: f64) -> Predicate {
    Predicate::And(vec![
        Predicate::Expr(format!("{col} >= {lo}")),
        Predicate::Expr(format!("{col} <= {hi}")),
    ])
}

fn y_range_predicate(col: &str, lo: f64, hi: f64) -> Predicate {
    Predicate::And(vec![
        Predicate::Expr(format!("{col} >= {lo}")),
        Predicate::Expr(format!("{col} <= {hi}")),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// brush_rect_to_predicate on intervalX produces an And
    /// of two Expr clauses bounding the x channel column.
    #[test]
    fn brush_rect_to_predicate_interval_x() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::xy("speed", "delay");
        let pred = brush_rect_to_predicate(rect, BrushKind::IntervalX, &channels);

        match pred {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 2, "intervalX → two-clause And");
                match (&clauses[0], &clauses[1]) {
                    (Predicate::Expr(lo), Predicate::Expr(hi)) => {
                        assert!(lo.contains("speed"), "lo: {lo}");
                        assert!(lo.contains(">= 10"), "lo bound: {lo}");
                        assert!(hi.contains("speed"), "hi: {hi}");
                        assert!(hi.contains("<= 90"), "hi bound: {hi}");
                    }
                    other => panic!("expected two Expr clauses, got {other:?}"),
                }
            }
            other => panic!("expected Predicate::And, got {other:?}"),
        }
    }

    /// intervalY produces an And of two Expr clauses on y.
    #[test]
    fn brush_rect_to_predicate_interval_y() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::xy("speed", "delay");
        let pred = brush_rect_to_predicate(rect, BrushKind::IntervalY, &channels);

        match pred {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 2, "intervalY → two-clause And");
                for c in &clauses {
                    let s = match c {
                        Predicate::Expr(s) => s,
                        _ => panic!("expected Expr, got {c:?}"),
                    };
                    assert!(s.contains("delay"), "y bound should reference y col: {s}");
                }
            }
            other => panic!("expected Predicate::And, got {other:?}"),
        }
    }

    /// intervalXY combines all four bounds into a flat
    /// four-clause And.
    #[test]
    fn brush_rect_to_predicate_interval_xy() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::xy("speed", "delay");
        let pred = brush_rect_to_predicate(rect, BrushKind::IntervalXY, &channels);

        match pred {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 4, "intervalXY → four-clause flat And");
                let texts: Vec<&str> = clauses
                    .iter()
                    .map(|c| match c {
                        Predicate::Expr(s) => s.as_str(),
                        _ => panic!("expected Expr"),
                    })
                    .collect();
                // Two reference x col, two reference y col.
                let x_clauses = texts.iter().filter(|s| s.contains("speed")).count();
                let y_clauses = texts.iter().filter(|s| s.contains("delay")).count();
                assert_eq!(x_clauses, 2, "two clauses on x");
                assert_eq!(y_clauses, 2, "two clauses on y");
            }
            other => panic!("expected Predicate::And, got {other:?}"),
        }
    }

    /// missing channel → degenerate Predicate::True.
    #[test]
    fn brush_rect_to_predicate_missing_channel() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::y_only("delay");
        // intervalX needs x channel — missing → True.
        let pred = brush_rect_to_predicate(rect, BrushKind::IntervalX, &channels);
        assert_eq!(pred, Predicate::True);
    }

    // ---------------------------------------------------------------------
    // cfs3 — BrushKind::Point + point_predicate (v3)
    // ---------------------------------------------------------------------

    /// BrushKind::Point is a constructible enum variant distinct
    /// from the interval variants, and `point_predicate(column, value)`
    /// returns a `Predicate::Expr` containing the column and the **already-
    /// formatted SQL literal** value (no quoting performed by the helper).
    /// The "non-brushable kinds excluded" sub-clause is asserted spec-side
    /// in `non_brushable_kinds_excluded` (analysis.rs).
    #[test]
    fn brush_kind_point_constructs() {
        // (a) Sealed-enum coverage: Point is distinct from each interval.
        let p = BrushKind::Point;
        assert_ne!(p, BrushKind::IntervalX);
        assert_ne!(p, BrushKind::IntervalY);
        assert_ne!(p, BrushKind::IntervalXY);

        // (b) point_predicate preserves the caller's quoting verbatim.
        // String literal — caller supplies the surrounding quotes.
        let pred_str = point_predicate("category", "'Athletics'");
        match &pred_str {
            Predicate::Expr(s) => {
                assert!(s.contains("category"), "column name must appear: {s}");
                assert!(
                    s.contains("'Athletics'"),
                    "literal value preserved verbatim with quotes: {s}"
                );
            }
            other => panic!("expected Predicate::Expr, got {other:?}"),
        }

        // Numeric literal — caller supplies no quotes.
        let pred_num = point_predicate("count", "42");
        match &pred_num {
            Predicate::Expr(s) => {
                assert!(s.contains("count"), "column name must appear: {s}");
                assert!(s.contains("42"), "literal value preserved verbatim: {s}");
                assert!(
                    !s.contains("'42'"),
                    "helper must NOT add quotes to numeric literal: {s}"
                );
            }
            other => panic!("expected Predicate::Expr, got {other:?}"),
        }
    }

    /// cfs point-selection: point_to_predicate maps a clicked data
    /// value onto the plot's x column (PointX) or y column (PointY) as an
    /// equality predicate; a missing channel or non-point kind degenerates to
    /// True. Each `SelectionValue` variant formats a type-correct literal.
    #[test]
    fn point_to_predicate_maps_value_onto_channel() {
        let channels = ChannelColumns::xy("speed", "delay");

        // PointX numeric → `speed = 3`.
        match point_to_predicate(&SelectionValue::Int(3), BrushKind::PointX, &channels) {
            Predicate::Expr(s) => {
                assert_eq!(
                    s, "speed = 3",
                    "PointX equality on x column, integer literal"
                );
            }
            other => panic!("expected Expr, got {other:?}"),
        }
        // PointY numeric → `delay = 40`.
        match point_to_predicate(&SelectionValue::Int(40), BrushKind::PointY, &channels) {
            Predicate::Expr(s) => assert_eq!(s, "delay = 40", "PointY equality on y column"),
            other => panic!("expected Expr, got {other:?}"),
        }
        // Categorical → quoted, embedded quote escaped by doubling.
        match point_to_predicate(
            &SelectionValue::Text("O'Hara".to_string()),
            BrushKind::PointX,
            &channels,
        ) {
            Predicate::Expr(s) => {
                assert_eq!(s, "speed = 'O''Hara'", "string literal is quoted+escaped")
            }
            other => panic!("expected Expr, got {other:?}"),
        }
        // Temporal → make_timestamp, not a bare integer.
        match point_to_predicate(
            &SelectionValue::Timestamp(1_700_000_000_000_000),
            BrushKind::PointX,
            &channels,
        ) {
            Predicate::Expr(s) => {
                assert_eq!(
                    s, "speed = make_timestamp(1700000000000000)",
                    "temporal literal"
                )
            }
            other => panic!("expected Expr, got {other:?}"),
        }
        // Missing channel → True.
        assert_eq!(
            point_to_predicate(
                &SelectionValue::Number(1.0),
                BrushKind::PointY,
                &ChannelColumns::x_only("speed")
            ),
            Predicate::True,
            "PointY with no y channel is a degenerate no-op"
        );
        // Non-point kind → True (this helper only produces point predicates).
        assert_eq!(
            point_to_predicate(
                &SelectionValue::Number(1.0),
                BrushKind::IntervalX,
                &channels
            ),
            Predicate::True
        );
    }

    // ---------------------------------------------------------------------
    // Structured clause constructors
    // ---------------------------------------------------------------------

    /// The structured 1-D brush renders the SAME SQL text as the string
    /// adapter's predicate for the same gesture — Display is the render
    /// contract (`ir.rs`), so string-vs-structured substitution is
    /// byte-invisible downstream.
    #[test]
    fn structured_interval_matches_string_adapter_sql() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::xy("speed", "delay");

        for kind in [BrushKind::IntervalX, BrushKind::IntervalY] {
            let string_form = brush_rect_to_predicate(rect, kind, &channels);
            let structured = brush_rect_to_structured(rect, kind, &channels, None, None);
            assert_eq!(
                format!("{structured}"),
                format!("{string_form}"),
                "{kind:?}: structured and string forms render identical SQL"
            );
        }
        // The structured form is the Interval variant, not a flattened string.
        match brush_rect_to_structured(rect, BrushKind::IntervalX, &channels, None, None) {
            Predicate::Interval { column, .. } => assert_eq!(column, "speed"),
            other => panic!("expected Interval, got {other:?}"),
        }
    }

    /// The structured XY brush produces one Interval clause per axis under a
    /// flat And, and renders SQL semantically equal to the string adapter's
    /// four-clause form — byte-identical to the NESTED hand-written string
    /// shape (each axis's two bounds grouped), which is the string form of
    /// this structure.
    #[test]
    fn structured_interval_xy_one_clause_per_axis() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::xy("speed", "delay");
        let structured =
            brush_rect_to_structured(rect, BrushKind::IntervalXY, &channels, None, None);

        match &structured {
            Predicate::And(clauses) => {
                assert_eq!(clauses.len(), 2, "one structured clause per axis");
                match (&clauses[0], &clauses[1]) {
                    (
                        Predicate::Interval { column: xc, .. },
                        Predicate::Interval { column: yc, .. },
                    ) => {
                        assert_eq!(xc, "speed");
                        assert_eq!(yc, "delay");
                    }
                    other => panic!("expected two Intervals, got {other:?}"),
                }
            }
            other => panic!("expected And, got {other:?}"),
        }

        // Byte-identity with the equivalent hand-written NESTED string form.
        let nested_string_form = Predicate::And(vec![
            Predicate::And(vec![
                Predicate::Expr("speed >= 10".to_string()),
                Predicate::Expr("speed <= 90".to_string()),
            ]),
            Predicate::And(vec![
                Predicate::Expr("delay >= 50".to_string()),
                Predicate::Expr("delay <= 250".to_string()),
            ]),
        ]);
        assert_eq!(format!("{structured}"), format!("{nested_string_form}"));
    }

    /// Missing channels and point kinds degrade to True — mirroring the
    /// string adapter exactly.
    #[test]
    fn structured_brush_degenerate_cases_mirror_string_adapter() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let y_only = ChannelColumns::y_only("delay");
        assert_eq!(
            brush_rect_to_structured(rect, BrushKind::IntervalX, &y_only, None, None),
            Predicate::True
        );
        assert_eq!(
            brush_rect_to_structured(rect, BrushKind::IntervalXY, &y_only, None, None),
            Predicate::True
        );
        let channels = ChannelColumns::xy("speed", "delay");
        for kind in [BrushKind::Point, BrushKind::PointX, BrushKind::PointY] {
            assert_eq!(
                brush_rect_to_structured(rect, kind, &channels, None, None),
                Predicate::True,
                "{kind:?} is click-driven; the rect path degrades to True"
            );
        }
    }

    /// scalar_value_from preserves each SelectionValue's type, and the
    /// structured point clause renders the exact SQL the string adapter
    /// produces for the same click — across every literal kind.
    #[test]
    fn structured_point_matches_string_adapter_sql() {
        let channels = ChannelColumns::xy("speed", "delay");
        let values = [
            SelectionValue::Number(1.5),
            SelectionValue::Int(3),
            SelectionValue::Text("O'Hara".to_string()),
            SelectionValue::Timestamp(1_700_000_000_000_000),
            SelectionValue::TimestampTz(1_700_000_000_000_000),
        ];
        for value in &values {
            // Literal-text parity between the two formatting paths.
            assert_eq!(
                scalar_value_from(value).to_sql_literal(),
                value.literal(),
                "ScalarValue reproduces SelectionValue::literal for {value:?}"
            );
            for kind in [BrushKind::PointX, BrushKind::PointY] {
                let string_form = point_to_predicate(value, kind, &channels);
                let structured = point_to_structured(value, kind, &channels, None);
                assert_eq!(
                    format!("{structured}"),
                    format!("{string_form}"),
                    "{kind:?}/{value:?}: identical rendered SQL"
                );
            }
        }
        // Degenerate cases mirror the string adapter.
        assert_eq!(
            point_to_structured(
                &SelectionValue::Number(1.0),
                BrushKind::PointY,
                &ChannelColumns::x_only("speed"),
                None,
            ),
            Predicate::True
        );
        assert_eq!(
            point_to_structured(
                &SelectionValue::Number(1.0),
                BrushKind::IntervalX,
                &channels,
                None,
            ),
            Predicate::True
        );
    }
}

// ---------------------------------------------------------------------------
// The release/click commit path — moved here from the retired gpui chart view.
// ---------------------------------------------------------------------------

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
/// Single-binding consumers should call [`commit_brush_release`] — a
/// 1-element-slice wrapper kept so the single-binding call surface stays
/// green (see its own docs).
pub fn commit_brush_release_multi<D: SelectionDispatcher>(
    interaction: &InteractionState,
    bindings: &[BrushBinding],
    dispatcher: &mut D,
) -> (InteractionState, Vec<(String, Vec<DispatchResult>)>) {
    if let InteractionState::Brushing { start, current } = interaction {
        let rect = kurbo::Rect::new(
            start.x.min(current.x),
            start.y.min(current.y),
            start.x.max(current.x),
            start.y.max(current.y),
        );
        let mut aggregated = Vec::with_capacity(bindings.len());
        for binding in bindings {
            // Kind-compatibility filter: a rect DRAG only drives interval
            // selections. Point selections (toggleX/Y) are click-driven, and a
            // plot may carry both a point and an interval interactor — so
            // skipping the point bindings here stops a y-drag from dispatching a
            // degenerate `Predicate::True` (select-all) into the point selection.
            // Their real predicate is produced by the click gesture (deferred).
            if matches!(
                binding.kind,
                BrushKind::Point | BrushKind::PointX | BrushKind::PointY
            ) {
                continue;
            }
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

/// Pure helper for the single-binding path: given an InteractionState (which may or
/// may not be Brushing), a binding, and a dispatcher, produce the
/// dispatch result vec and the next InteractionState. Lifted out of
/// the windowed event loop for testability — a host's mouse-up handler
/// shares the same logic threaded through its own reactive cell.
///
/// **cfs3 wrapper:** preserved as a single-binding convenience over
/// [`commit_brush_release_multi`] so the single-binding surface stays green.
pub fn commit_brush_release<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (InteractionState, Vec<DispatchResult>) {
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
pub fn commit_brush_clear<D: SelectionDispatcher>(
    interaction: &InteractionState,
    binding: &BrushBinding,
    dispatcher: &mut D,
) -> (InteractionState, Vec<DispatchResult>) {
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

/// Commit a CLICK gesture across a plot's bindings (the point-
/// selection gesture that finishes cross-filter). Unlike a drag (which drives
/// interval selections), a click drives POINT selections:
///
/// - A `PointX`/`PointY` binding **snaps to the nearest datum**: `find_nearest`
///   locates the closest rendered point to `click_px` (in pixels, so the hit
///   radius is uniform on screen), and the datum's EXACT stored value is read
///   and dispatched as `col = value`. Selecting the *continuous* click
///   coordinate would never equal a discrete datum under `=`, so snapping is
///   what makes point selection actually match rows.
/// - A click that **misses every datum** (nothing within the hit radius) clears
///   the point selection — click-empty-space to deselect.
/// - An interval (or bare `Point`, deferred) binding **clears** on a click —
///   the click-outside-brush retract that [`commit_brush_clear`] performs.
///
/// Point selection forms a numeric `col = value` predicate, so it is scoped to
/// numeric, categorical, and temporal axes: a continuous axis snaps to the
/// nearest rendered datum and reads its exact typed value; a categorical (band)
/// axis resolves the clicked category directly from the scale. The dispatched
/// `col = value` uses a type-correct literal (bare number/int, quoted+escaped
/// string, or `make_timestamp(us)`) via [`SelectionValue`].
///
/// `marks` are the plot's `(batch, channel_map)` pairs to search (a plot may
/// layer several); `scales` map data → the same pixel space as `click_px`.
/// Returns `(Idle, per-binding results)`, mirroring
/// [`commit_brush_release_multi`].
pub fn commit_click_multi<D: SelectionDispatcher>(
    click_px: kurbo::Point,
    marks: &[(&RecordBatch, &ChannelMap)],
    scales: &ScaleSet,
    bindings: &[BrushBinding],
    dispatcher: &mut D,
) -> (InteractionState, Vec<(String, Vec<DispatchResult>)>) {
    // (selection, contributor) pairs a point binding SELECTED this click, so a
    // sibling interval (or point-miss) on the SAME target doesn't clear the point
    // we just set — a plot may carry both a toggle and an interval interactor.
    let mut selected: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut aggregated = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let key = (
            binding.selection_name.clone(),
            binding.contributor.0.clone(),
        );
        let results = match binding.kind {
            BrushKind::PointX | BrushKind::PointY => {
                let mode = if matches!(binding.kind, BrushKind::PointX) {
                    NearestMode::X
                } else {
                    NearestMode::Y
                };
                match resolve_point_value(click_px, marks, scales, binding, mode) {
                    // Hit: select that datum's (or category's) exact value.
                    Some(value) => {
                        selected.insert(key);
                        let predicate = point_to_predicate(&value, binding.kind, &binding.channels);
                        dispatcher.dispatch(
                            &binding.selection_name,
                            binding.contributor.clone(),
                            predicate,
                        )
                    }
                    // Miss (clicked empty space on a continuous axis, or no column
                    // on the axis): deselect — unless a sibling point binding
                    // already selected this same target.
                    None if selected.contains(&key) => continue,
                    None => dispatcher.clear(&binding.selection_name, binding.contributor.clone()),
                }
            }
            // Interval kinds and bare `Point` (deferred): a click clears — unless
            // a sibling point binding already selected this same target.
            _ if selected.contains(&key) => continue,
            _ => dispatcher.clear(&binding.selection_name, binding.contributor.clone()),
        };
        aggregated.push((binding.selection_name.clone(), results));
    }
    (InteractionState::Idle, aggregated)
}

/// The [`SelectionValue`] a point click resolves to along the axis a binding
/// selects.
///
/// - **Categorical (band) axis**: the axis position *is* the value, so the
///   clicked category is resolved directly from the band scale (a pixel→category
///   inverse) — a `Text` value. There is no datum to snap to, so a categorical
///   click always resolves (to the nearest category).
/// - **Continuous (linear/time) axis**: snaps to the nearest rendered datum
///   across all the plot's `marks` (globally closest in pixel space) and reads
///   its EXACT stored cell — a `Number`/`Int`/`Timestamp`. So the dispatched
///   `col = value` matches a real datum, not the continuous click coordinate.
///
/// `None` when the binding has no column on its axis, or on a continuous-axis
/// miss (no datum within the hit radius).
fn resolve_point_value(
    click_px: kurbo::Point,
    marks: &[(&RecordBatch, &ChannelMap)],
    scales: &ScaleSet,
    binding: &BrushBinding,
    mode: NearestMode,
) -> Option<SelectionValue> {
    let (column, axis, cursor_on_axis) = match binding.kind {
        BrushKind::PointX => (binding.channels.x.as_deref()?, Channel::X, click_px.x),
        BrushKind::PointY => (binding.channels.y.as_deref()?, Channel::Y, click_px.y),
        _ => return None,
    };

    // Categorical axis: resolve the clicked category from the band scale. A click
    // OUTSIDE the band's pixel range (e.g. in the axis margin) resolves to `None`
    // so it clears — otherwise every click snaps to the nearest category and the
    // filter could never be retracted (full toggle-off is a follow-up).
    if let Some(
        scale @ Scale::Band {
            range_start,
            range_end,
            ..
        },
    ) = scales.get(axis)
    {
        let (lo, hi) = (range_start.min(*range_end), range_start.max(*range_end));
        if cursor_on_axis < lo || cursor_on_axis > hi {
            return None;
        }
        return band_category_at(scale, cursor_on_axis).map(SelectionValue::Text);
    }

    // Continuous axis: snap to the nearest rendered datum, read its typed value.
    let mut best: Option<(f64, SelectionValue)> = None; // (pixel distance, value)
    for (batch, channel_map) in marks {
        let hit = match find_nearest(click_px, batch, channel_map, scales, mode, None) {
            Some(h) => h,
            None => continue,
        };
        let value = match column_typed_value_at(batch, column, hit.row) {
            Some(v) => v,
            None => continue,
        };
        if best.as_ref().is_none_or(|(d, _)| hit.distance < *d) {
            best = Some((hit.distance, value));
        }
    }
    best.map(|(_, v)| v)
}

/// Convert a spec-side [`BrushableBinding`] into a UI-side [`BrushBinding`]
/// by translating the mirror enums (`BrushKind`, `ChannelColumns`). The
/// conversion is faithful — every field copies through verbatim.
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
        Spec::PointX => BrushKind::PointX,
        Spec::PointY => BrushKind::PointY,
    }
}

// --- Tests: coordinate transform and interaction state ---
#[cfg(test)]
mod commit_tests {
    use super::*;
    use crate::chart_layout::ChartLayout;
    use brightfield_sql::ir::Predicate;
    use kurbo::Point;

    // Unit tests for coordinate transform logic — these don't require
    // a windowed runtime, just the math.

    #[test]
    fn coordinate_transform_inside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        let window_pos = Point::new(400.0, 300.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 300.0).abs() < f64::EPSILON);
        assert!((local.y - 250.0).abs() < f64::EPSILON);
        assert!(layout.contains(local), "point should be inside plot area");
    }

    #[test]
    fn coordinate_transform_outside_plot() {
        let layout = ChartLayout::new(640.0, 480.0);
        let element_origin = Point::new(100.0, 50.0);
        // Point in the left margin area
        let window_pos = Point::new(110.0, 100.0);

        let local = layout.window_to_local(window_pos, element_origin);
        assert!((local.x - 10.0).abs() < f64::EPSILON);
        assert!(
            !layout.contains(local),
            "point should be outside plot area (in left margin)"
        );
    }

    #[test]
    fn interaction_state_idle_to_brushing() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(
            matches!(state, InteractionState::Brushing { .. }),
            "should transition to Brushing on mouse_down inside plot area"
        );
    }

    #[test]
    fn interaction_state_brushing_to_idle() {
        let state = InteractionState::start_brush(Point::new(100.0, 200.0));
        assert!(matches!(state, InteractionState::Brushing { .. }));

        // On mouse_up, we'd set to Idle
        let idle = InteractionState::Idle;
        assert!(matches!(idle, InteractionState::Idle));
    }

    #[test]
    fn brush_update_during_drag() {
        let mut state = InteractionState::start_brush(Point::new(100.0, 200.0));
        state.update_brush(Point::new(300.0, 400.0));

        let rect = state.brush_rect().expect("should have brush rect");
        assert!((rect.x0 - 100.0).abs() < f64::EPSILON);
        assert!((rect.y0 - 200.0).abs() < f64::EPSILON);
        assert!((rect.x1 - 300.0).abs() < f64::EPSILON);
        assert!((rect.y1 - 400.0).abs() < f64::EPSILON);
    }

    // --- Resize ---

    #[test]
    fn layout_dimensions_change() {
        let layout = ChartLayout::new(640.0, 480.0);
        assert!((layout.width - 640.0).abs() < f64::EPSILON);

        let resized = ChartLayout::new(1024.0, 768.0);
        assert!((resized.width - 1024.0).abs() < f64::EPSILON);
        assert!((resized.height - 768.0).abs() < f64::EPSILON);
    }

    #[test]
    fn render_respects_new_dimensions() {
        // Verify plot area scales with dimensions
        let layout = ChartLayout::new(1024.0, 768.0);
        let area = layout.plot_area();
        assert!((area.x1 - (1024.0 - 20.0)).abs() < f64::EPSILON);
        assert!((area.y1 - (768.0 - 30.0)).abs() < f64::EPSILON);
    }

    // --- brush release dispatches a propagate_selection call ---

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
        ) -> Vec<DispatchResult> {
            self.calls.push((name.to_string(), contributor, predicate));
            // Stub return: subscribers, if any, are mocked as zero —
            // this double's contract is "did dispatch get called?".
            Vec::new()
        }

        fn clear(&mut self, name: &str, contributor: ComponentPath) -> Vec<DispatchResult> {
            self.clear_calls.push((name.to_string(), contributor));
            Vec::new()
        }
    }

    #[test]
    fn on_mouse_up_dispatches_selection() {
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

        let (next_state, _results) = commit_brush_release(&interaction, &binding, &mut dispatcher);

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
    fn on_mouse_up_no_brush_no_dispatch() {
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

        let (next_state, results) = commit_brush_release(&interaction, &binding, &mut dispatcher);

        assert!(dispatcher.calls.is_empty(), "no brush → no dispatch");
        assert!(results.is_empty());
        assert!(matches!(next_state, InteractionState::Idle));
    }

    /// cfs point-selection: a rect DRAG skips point-kind bindings —
    /// only interval selections are rect-driven. A plot carrying BOTH a toggleX
    /// (PointX) and an intervalY binding must dispatch only the interval on a
    /// drag, never a degenerate `True` into the point selection.
    #[test]
    fn drag_skips_point_bindings() {
        let mut interaction = InteractionState::start_brush(Point::new(20.0, 30.0));
        interaction.update_brush(Point::new(120.0, 230.0));

        let bindings = [
            BrushBinding {
                selection_name: "pt".to_string(),
                contributor: ComponentPath("root/plot[0]".to_string()),
                kind: BrushKind::PointX,
                channels: ChannelColumns::xy("speed", "delay"),
            },
            BrushBinding {
                selection_name: "iv".to_string(),
                contributor: ComponentPath("root/plot[0]".to_string()),
                kind: BrushKind::IntervalY,
                channels: ChannelColumns::xy("speed", "delay"),
            },
        ];
        let mut dispatcher = RecordingDispatcher::new();
        let (_state, aggregated) =
            commit_brush_release_multi(&interaction, &bindings, &mut dispatcher);

        assert_eq!(
            dispatcher.calls.len(),
            1,
            "only the interval binding dispatches on a drag; the point is skipped"
        );
        assert_eq!(
            dispatcher.calls[0].0, "iv",
            "the dispatched selection is the interval, not the point"
        );
        assert_eq!(aggregated.len(), 1);
    }

    // ---------------------------------------------------------------------
    // cfs3 — clearing, multi-binding dispatch, BrushableBinding conversion
    // ---------------------------------------------------------------------

    /// commit_brush_clear dispatches a `clear` call when the
    /// interaction is Idle OR a zero-area Brushing (click). A non-zero
    /// Brushing does NOT clear (that path is the drag-release, handled by
    /// commit_brush_release). Returns Idle as the next state on a clear.
    #[test]
    fn click_outside_active_brush_clears() {
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
        let (next_state, _) = commit_brush_clear(&zero_area, &binding, &mut dispatcher);
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

    /// commit_brush_release_multi (the lifted multi-binding
    /// helper) dispatches one propagate_selection per binding, with each
    /// binding's predicate computed against its own kind. Verifies the
    /// kind-compatibility filter — an IntervalX binding produces an x-only
    /// predicate even when the rect has a non-zero y extent.
    #[test]
    fn plot_drives_multiple_selections() {
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

    /// BrushBinding::from(&BrushableBinding) preserves every
    /// field — selection_name, contributor (= parent_plot), kind, and
    /// channels — translating between the spec-side and ui-side mirror
    /// enums verbatim.
    #[test]
    fn brushable_binding_to_brush_binding() {
        let spec_binding = BrushableBinding {
            interactor_path: ComponentPath("root/plot[0]/interactor[intervalXY]".to_string()),
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
