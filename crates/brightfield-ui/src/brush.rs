//! Brush-to-predicate adapter and selection dispatch
//! abstraction.
//!
//! Converts a chart-coordinate brush rectangle into a Predicate IR value
//! that the runtime selection coordinator can store and resolve, and
//! provides a small dispatch trait so ChartView can route brush-release
//! into a Session without depending on the engine at the ChartView call
//! site. The trait keeps the test double cheap.

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
use brightfield_render::nearest::SelectionValue;
use brightfield_sql::ir::Predicate;
use brightfield_spec::analysis::ComponentPath;
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
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>;

    /// Retract a contributor's predicate from the named selection — the
    /// click-outside-active-brush path. Mirrors
    /// `Session::clear_selection`'s shape: returns one
    /// `(mark_index, Result)` per subscriber that re-executes against the
    /// reduced selection state.
    fn clear(
        &mut self,
        name: &str,
        contributor: ComponentPath,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>;
}

impl SelectionDispatcher for brightfield_engine::Session {
    fn dispatch(
        &mut self,
        name: &str,
        contributor: ComponentPath,
        predicate: Predicate,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        self.propagate_selection(name, contributor, predicate)
    }

    fn clear(
        &mut self,
        name: &str,
        contributor: ComponentPath,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
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
                assert_eq!(s, "speed = 3", "PointX equality on x column, integer literal");
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
            Predicate::Expr(s) => assert_eq!(s, "speed = 'O''Hara'", "string literal is quoted+escaped"),
            other => panic!("expected Expr, got {other:?}"),
        }
        // Temporal → make_timestamp, not a bare integer.
        match point_to_predicate(
            &SelectionValue::Timestamp(1_700_000_000_000_000),
            BrushKind::PointX,
            &channels,
        ) {
            Predicate::Expr(s) => {
                assert_eq!(s, "speed = make_timestamp(1700000000000000)", "temporal literal")
            }
            other => panic!("expected Expr, got {other:?}"),
        }
        // Missing channel → True.
        assert_eq!(
            point_to_predicate(&SelectionValue::Number(1.0), BrushKind::PointY, &ChannelColumns::x_only("speed")),
            Predicate::True,
            "PointY with no y channel is a degenerate no-op"
        );
        // Non-point kind → True (this helper only produces point predicates).
        assert_eq!(
            point_to_predicate(&SelectionValue::Number(1.0), BrushKind::IntervalX, &channels),
            Predicate::True
        );
    }
}
