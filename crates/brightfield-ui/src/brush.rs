//! Brush-to-predicate adapter (cfs2_ac10) and selection dispatch
//! abstraction (cfs2_ac11).
//!
//! Converts a chart-coordinate brush rectangle into a Predicate IR value
//! that the runtime selection coordinator can store and resolve, and
//! provides a small dispatch trait so ChartView can route brush-release
//! into a Session without depending on the engine at the ChartView call
//! site. The trait keeps the test double cheap.

use brightfield_engine::error::EngineError;
use brightfield_engine::RecordBatch;
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

    /// cfs2_ac10: brush_rect_to_predicate on intervalX produces an And
    /// of two Expr clauses bounding the x channel column.
    #[test]
    fn cfs2_ac10_brush_rect_to_predicate_interval_x() {
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

    /// cfs2_ac10: intervalY produces an And of two Expr clauses on y.
    #[test]
    fn cfs2_ac10_brush_rect_to_predicate_interval_y() {
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

    /// cfs2_ac10: intervalXY combines all four bounds into a flat
    /// four-clause And.
    #[test]
    fn cfs2_ac10_brush_rect_to_predicate_interval_xy() {
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

    /// cfs2_ac10: missing channel → degenerate Predicate::True.
    #[test]
    fn cfs2_ac10_brush_rect_to_predicate_missing_channel() {
        let rect = Rect::new(10.0, 50.0, 90.0, 250.0);
        let channels = ChannelColumns::y_only("delay");
        // intervalX needs x channel — missing → True.
        let pred = brush_rect_to_predicate(rect, BrushKind::IntervalX, &channels);
        assert_eq!(pred, Predicate::True);
    }
}
