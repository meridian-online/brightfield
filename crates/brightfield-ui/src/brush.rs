//! Brush-to-predicate adapter and selection dispatch
//! abstraction.
//!
//! Converts a chart-coordinate brush rectangle into a Predicate IR value
//! that the runtime selection coordinator can store and resolve, and
//! provides a small dispatch trait so ChartView can route brush-release
//! into a Session without depending on the engine at the ChartView call
//! site. The trait keeps the test double cheap.

use brightfield_engine::DispatchResult;
use brightfield_render::nearest::SelectionValue;
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::{ClauseMeta, Predicate, ScalarValue};
use kurbo::Rect;

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
    /// only; no chart_view dispatch path is wired (decision 2).
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
