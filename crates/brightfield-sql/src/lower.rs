//! AST → IR lowering (ac-03, ac-05).
//!
//! The `MarkLower` trait is the extension point for per-mark lowering. V1
//! ships no concrete implementations — every `MarkKind` returns
//! `EmitError::UnsupportedMark`. As marks are implemented in future cards,
//! each gets a `MarkLower` impl registered in `default_lowerers`.

use indexmap::IndexMap;

use brightfield_spec::ast::{Mark, MarkData, ParamNode, SelectionNode, SpecValue, ValueOrParamRef};
use brightfield_spec::vocab::MarkKind;

use crate::error::EmitError;
use crate::ir::{Predicate, QueryPlan, SelectionResolution, SortDir};

/// Context available during lowering — spec-level data sources, params, selections.
#[derive(Debug)]
pub struct LowerCtx<'a> {
    /// All named data sources from the spec.
    pub data_sources: &'a IndexMap<String, brightfield_spec::ast::DataSource>,
    /// All named params from the spec.
    pub params: &'a IndexMap<String, ParamNode>,
}

/// Trait for per-mark AST → IR lowering.
///
/// Each `MarkKind` that brightfield can emit SQL for gets a `MarkLower` impl.
/// The default implementation returns `EmitError::UnsupportedMark`.
pub trait MarkLower: Send + Sync {
    /// Lower a mark to a `QueryPlan`.
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError>;
}

/// Default lowerer that rejects all marks as unsupported.
pub struct DefaultLowerer;

impl MarkLower for DefaultLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        Err(EmitError::UnsupportedMark {
            kind: mark.kind.wire_name().to_string(),
        })
    }
}

/// Simple lowerer for marks with `data: { from: table }`.
///
/// Emits `QueryPlan::Source { table }` — equivalent to `SELECT * FROM table`.
/// Returns `UnsupportedMark` for marks without data or with inline data.
pub struct SimpleLowerer;

impl MarkLower for SimpleLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        match &mark.data {
            Some(MarkData::From { source, extras, .. }) => {
                let base = QueryPlan::Source {
                    table: source.clone(),
                };
                let filtered = apply_data_filter(extras, base);
                Ok(project_param_channels(mark, filtered))
            }
            Some(MarkData::Inline(_)) | None => Err(EmitError::UnsupportedMark {
                kind: mark.kind.wire_name().to_string(),
            }),
        }
    }
}

/// Positional-channel option keys whose `$param` binding is projected into the
/// query (card 0014, Decision 2). Kept in sync with brightfield-render's
/// positional `Channel` set (x/y/x1/y1/x2/y2); non-positional channels
/// (fill/stroke/size/text) bound to a param are the deferred render-only case.
const POSITIONAL_CHANNEL_KEYS: &[&str] = &["x", "y", "x1", "y1", "x2", "y2"];

/// If `mark` binds any positional channel to a bare `$param`, wrap `base` in a
/// Projection that keeps `*` and adds `$param AS "<param>"` for each DISTINCT
/// param — so the value (interpolated from param_state at emit time) becomes a
/// real query column the renderer reads. A no-op when no positional channel is
/// param-bound, leaving the plain `SELECT *` plan (and every existing test)
/// untouched.
fn project_param_channels(mark: &Mark, base: QueryPlan) -> QueryPlan {
    let mut seen: Vec<String> = Vec::new();
    let mut cols: Vec<String> = vec!["*".to_string()];
    for key in POSITIONAL_CHANNEL_KEYS {
        if let Some(ValueOrParamRef::Param(pr)) = mark.options.get(*key) {
            if !seen.iter().any(|s| s == &pr.0) {
                seen.push(pr.0.clone());
                // CAST to DOUBLE so a FRACTIONAL param value doesn't type the
                // column as DECIMAL — the renderer's column_as_f64 reads
                // Float/Int but not Decimal, so a bare `3.5 AS "k"` would silently
                // render nothing. DOUBLE covers integer and float params alike.
                cols.push(format!("CAST(${} AS DOUBLE) AS \"{}\"", pr.0, pr.0));
            }
        }
    }
    if cols.len() == 1 {
        base
    } else {
        QueryPlan::Projection {
            input: Box::new(base),
            columns: cols,
        }
    }
}

/// If `extras` carries a `filter` (`data: { from, filter: "..." }`), wrap `plan`
/// in a WHERE (card 0014, Decision 2). A no-op when there is no filter.
///
/// A filter that references a `$param` parses to an `Expression` (rendered to raw
/// SQL text with the `$name` preserved for emit-time interpolation, so the param
/// re-filters rows on propagate_param); a param-FREE filter (e.g. `"x > 2"`)
/// parses to a plain `String` that is already raw predicate text, used verbatim
/// (NOT quoted — quoting would turn the WHERE into a constant-string no-op that
/// silently passes every row).
fn apply_data_filter(extras: &IndexMap<String, SpecValue>, plan: QueryPlan) -> QueryPlan {
    let predicate = match extras.get("filter") {
        Some(expr @ SpecValue::Expression(_)) => crate::emit::spec_value_to_sql_literal(expr),
        Some(SpecValue::String(s)) => s.clone(),
        _ => return plan,
    };
    QueryPlan::Filter {
        input: Box::new(plan),
        predicate: Predicate::Expr(predicate),
    }
}

/// Helper: extract a literal string-valued option (skips ParamRef).
fn opt_string<'a>(
    options: &'a IndexMap<String, ValueOrParamRef<SpecValue>>,
    key: &str,
) -> Option<&'a str> {
    match options.get(key)? {
        ValueOrParamRef::Value(SpecValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Helper: extract a literal numeric option as f64.
fn opt_f64(options: &IndexMap<String, ValueOrParamRef<SpecValue>>, key: &str) -> Option<f64> {
    match options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Float(f)) => Some(*f),
        ValueOrParamRef::Value(SpecValue::Integer(i)) => Some(*i as f64),
        _ => None,
    }
}

/// Lowerer for regression marks (regressionY, regressionX).
///
/// Emits a one-row AggregateScalar with regr_* aggregates. When `stroke`
/// resolves to a column name, wraps in an Aggregation with that grouping
/// (one row per category). The lowerer rejects polynomial/exponential
/// regression upfront.
pub struct RegressionLowerer;

impl MarkLower for RegressionLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        // Reject non-linear regression types.
        if let Some(t) = opt_string(&mark.options, "type") {
            if t != "linear" {
                return Err(EmitError::UnsupportedMark {
                    kind: format!("{} (type='{}' — only linear is supported)", mark.kind.wire_name(), t),
                });
            }
        }

        let source = match &mark.data {
            Some(MarkData::From { source, .. }) => source.clone(),
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: mark.kind.wire_name().to_string(),
                })
            }
        };

        let x_col = opt_string(&mark.options, "x").ok_or_else(|| EmitError::UnsupportedMark {
            kind: format!("{} (missing x)", mark.kind.wire_name()),
        })?;
        let y_col = opt_string(&mark.options, "y").ok_or_else(|| EmitError::UnsupportedMark {
            kind: format!("{} (missing y)", mark.kind.wire_name()),
        })?;

        // Filter out NULLs in x or y so DuckDB's regr_* aggregates run cleanly.
        let filtered = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source { table: source }),
            predicate: Predicate::Expr(format!(
                "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL"
            )),
        };

        let aggregates = vec![
            format!("regr_slope(\"{y_col}\", \"{x_col}\") AS slope"),
            format!("regr_intercept(\"{y_col}\", \"{x_col}\") AS intercept"),
            format!("regr_count(\"{y_col}\", \"{x_col}\") AS n"),
            format!("regr_avgx(\"{y_col}\", \"{x_col}\") AS x_bar"),
            format!("regr_sxx(\"{y_col}\", \"{x_col}\") AS sxx"),
            format!("regr_sxy(\"{y_col}\", \"{x_col}\") AS sxy"),
            format!("regr_syy(\"{y_col}\", \"{x_col}\") AS syy"),
            // Data extents for render-time scale construction. The executed
            // batch holds only coefficients (no raw x/y rows), so the renderer
            // can't infer x/y scales from a column — it builds them from these
            // extents instead (see `RegressionRenderer::augment_scales`). The
            // filter above already drops NULL x/y, so min/max are over the
            // fitted sample.
            format!("CAST(min(\"{x_col}\") AS DOUBLE) AS x_min"),
            format!("CAST(max(\"{x_col}\") AS DOUBLE) AS x_max"),
            format!("CAST(min(\"{y_col}\") AS DOUBLE) AS y_min"),
            format!("CAST(max(\"{y_col}\") AS DOUBLE) AS y_max"),
        ];

        // Group by stroke column when present.
        if let Some(stroke_col) = opt_string(&mark.options, "stroke") {
            let mut group_aggregates = aggregates.clone();
            // Prepend the group key so it appears in the projection.
            // (Aggregation IR variant places group_by columns first via render_query.)
            let _ = &mut group_aggregates; // keep as-is; group_by handled below
            return Ok(QueryPlan::Aggregation {
                input: Box::new(filtered),
                group_by: vec![format!("\"{stroke_col}\"")],
                aggregates,
            });
        }

        Ok(QueryPlan::AggregateScalar {
            input: Box::new(filtered),
            aggregates,
        })
    }
}

/// Lowerer for density marks (density, densityX, densityY).
///
/// Emits a binning + group-count plan whose binned axis is the bucket **centre
/// in data units, aliased to the channel column name** (so it flows through
/// generic scale inference like an ordinary positional column):
///   1D: SELECT <centre(x)> AS "x", COUNT(*) FROM source GROUP BY 1
///   2D: same with both x and y centres
///
/// The lowerer reads `bins` (or `thresholds`) from the mark's option bag.
/// Default is 100 to match Mosaic's reference implementation (spec 2026-04-28
/// statistical-marks ac-07 implementation note). The bin width is computed
/// from the column extent at render time — for the SQL pass we use a fixed
/// bucket count and let DuckDB compute extents via subqueries.
pub struct DensityLowerer {
    pub kind: DensityLowerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityLowerKind {
    /// `densityX` — 1D density along x.
    OneDX,
    /// `densityY` — 1D density along y.
    OneDY,
    /// `density` — 2D density on both axes.
    #[allow(clippy::upper_case_acronyms)]
    TwoD,
}

impl MarkLower for DensityLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let source = match &mark.data {
            Some(MarkData::From { source, .. }) => source.clone(),
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: mark.kind.wire_name().to_string(),
                })
            }
        };

        let bin_count = opt_f64(&mark.options, "thresholds")
            .or_else(|| opt_f64(&mark.options, "bins"))
            .map(|f| f as i64)
            .unwrap_or(100);

        // Resolve required columns.
        let x_col = opt_string(&mark.options, "x");
        let y_col = opt_string(&mark.options, "y");

        match self.kind {
            DensityLowerKind::OneDX => {
                let x = x_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "densityX (missing x)".to_string(),
                })?;
                Ok(build_density_1d(&source, x, bin_count))
            }
            DensityLowerKind::OneDY => {
                let y = y_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "densityY (missing y)".to_string(),
                })?;
                Ok(build_density_1d(&source, y, bin_count))
            }
            DensityLowerKind::TwoD => {
                let x = x_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "density (missing x)".to_string(),
                })?;
                let y = y_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "density (missing y)".to_string(),
                })?;
                Ok(build_density_2d(&source, x, y, bin_count))
            }
        }
    }
}

/// Portable data-unit **centre** of the equiwidth bucket containing `col`, over
/// the column's `[min, max]` range split into `bins` buckets, cast to DOUBLE.
///
/// The statistical-mark renderers expect bin centres *in data units* — the
/// curve then maps through the ordinary positional scale and the axis reads in
/// data units. The earlier lowerer emitted the 1-based bucket *index*, which the
/// renderer never positioned correctly: half of the statistical-mark contract
/// bug (the other half — aliasing the output to the channel column, below).
/// DuckDB's `width_bucket` is also absent from the bundled libduckdb (first-
/// render follow-up #4), so this stays expressed with portable `floor`.
///
/// With `w = (hi - lo)/bins` and 0-based bucket `b = floor((v - lo)/(hi - lo)·bins)`,
/// the centre is `lo + (b + 0.5)·w`, i.e.
/// `lo + (floor((v - lo)/(hi - lo)·bins) + 0.5)·(hi - lo)/bins`. `nullif` guards
/// the all-equal (`hi == lo`) degenerate case. `least(b, bins - 1)` folds the
/// maximum value (where `(v - lo)/(hi - lo) == 1` ⇒ `floor == bins`) into the top
/// legitimate bucket instead of a phantom bucket centred half a bin past `hi`.
fn equiwidth_bin_centre(table: &str, col: &str, bins: i64) -> String {
    let lo = format!("(SELECT min(\"{col}\") FROM \"{table}\")");
    let hi = format!("(SELECT max(\"{col}\") FROM \"{table}\")");
    let top = bins - 1;
    format!(
        "CAST({lo} + (least(floor((\"{col}\" - {lo}) / nullif({hi} - {lo}, 0) * {bins}), {top}) + 0.5) \
         * ({hi} - {lo}) / {bins} AS DOUBLE)"
    )
}

/// Build a 1D density plan: bin `col` into equiwidth buckets, group by bucket,
/// and emit the bucket **centre aliased to the channel column name** (so generic
/// scale inference and `ChannelMap::get` treat it like any positional column)
/// alongside the per-bucket `count`.
fn build_density_1d(table: &str, col: &str, bins: i64) -> QueryPlan {
    let centre_expr = format!("{} AS \"{col}\"", equiwidth_bin_centre(table, col, bins));
    // Occupancy is aliased to the reserved `__bf_count` (not `count`) so it can't
    // collide with the bin centre when a density channel is bound to a column
    // literally named `count`. The renderer reads it as `DENSITY_COUNT_COL`.
    let count_expr = "CAST(COUNT(*) AS DOUBLE) AS __bf_count".to_string();

    // ORDER BY the bucket centre: GROUP BY output order is unspecified in
    // DuckDB, and the density/raster renderers draw in row order — a different
    // draw order blends anti-aliased cell edges differently, so unordered rows
    // make repeated renders differ at the byte level.
    QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Filter {
                input: Box::new(QueryPlan::Source {
                    table: table.to_string(),
                }),
                predicate: Predicate::Expr(format!("\"{col}\" IS NOT NULL")),
            }),
            group_by: vec![centre_expr],
            aggregates: vec![count_expr],
        }),
        keys: vec![(format!("\"{col}\""), SortDir::Asc)],
    }
}

/// Build a 2D density plan: equiwidth-bin both x and y, group by both, and emit
/// each axis's bucket centre aliased to its channel column name (plus `count`).
fn build_density_2d(table: &str, x_col: &str, y_col: &str, bins: i64) -> QueryPlan {
    let x_centre = format!("{} AS \"{x_col}\"", equiwidth_bin_centre(table, x_col, bins));
    let y_centre = format!("{} AS \"{y_col}\"", equiwidth_bin_centre(table, y_col, bins));
    // Reserved occupancy alias — see build_density_1d.
    let count_expr = "CAST(COUNT(*) AS DOUBLE) AS __bf_count".to_string();

    // Deterministic row order (x centre, then y centre) — see build_density_1d.
    QueryPlan::Order {
        input: Box::new(QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Filter {
                input: Box::new(QueryPlan::Source {
                    table: table.to_string(),
                }),
                predicate: Predicate::Expr(format!(
                    "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL"
                )),
            }),
            group_by: vec![x_centre, y_centre],
            aggregates: vec![count_expr],
        }),
        keys: vec![
            (format!("\"{x_col}\""), SortDir::Asc),
            (format!("\"{y_col}\""), SortDir::Asc),
        ],
    }
}

/// Build the registry of mark lowerers.
///
/// Registers SimpleLowerer for Dot, Line, BarX, BarY; the statistical-mark
/// lowerers (RegressionLowerer, DensityLowerer) for regression and density
/// kinds.
/// Marks not listed here fall back to DefaultLowerer (unsupported).
pub fn default_lowerers() -> Vec<(MarkKind, Box<dyn MarkLower>)> {
    vec![
        (MarkKind::Dot, Box::new(SimpleLowerer)),
        (MarkKind::Line, Box::new(SimpleLowerer)),
        (MarkKind::AreaY, Box::new(SimpleLowerer)),
        (MarkKind::AreaX, Box::new(SimpleLowerer)),
        (MarkKind::RuleX, Box::new(SimpleLowerer)),
        (MarkKind::RuleY, Box::new(SimpleLowerer)),
        (MarkKind::Text, Box::new(SimpleLowerer)),
        (MarkKind::BarX, Box::new(SimpleLowerer)),
        (MarkKind::BarY, Box::new(SimpleLowerer)),
        (MarkKind::Rect, Box::new(SimpleLowerer)),
        (MarkKind::RectX, Box::new(SimpleLowerer)),
        (MarkKind::RectY, Box::new(SimpleLowerer)),
        // Cell v1 is pass-through over PRE-AGGREGATED rows — one row per
        // (x category, y category) pair with a numeric fill column. The
        // self-aggregating form (fill: count/avg → CellLowerer) is deferred
        // with hexbin (card 0008, density marks).
        (MarkKind::Cell, Box::new(SimpleLowerer)),
        (MarkKind::RegressionY, Box::new(RegressionLowerer)),
        (MarkKind::RegressionX, Box::new(RegressionLowerer)),
        (
            MarkKind::DensityX,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::OneDX,
            }),
        ),
        (
            MarkKind::DensityY,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::OneDY,
            }),
        ),
        (
            MarkKind::Density,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::TwoD,
            }),
        ),
        // Raster reuses the 2D density binning — the same (x centre, y centre,
        // count) grid — and renders filled cells coloured by count.
        (
            MarkKind::Raster,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::TwoD,
            }),
        ),
        // Heatmap reuses the same 2D density binning; the renderer smooths the
        // reconstructed grid (kde_2d) and ramps EVERY cell — raster's smoothed
        // sibling. Zero new SQL (card 0008, density marks).
        (
            MarkKind::Heatmap,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::TwoD,
            }),
        ),
    ]
}

/// Look up a lowerer for a given `MarkKind`, falling back to `DefaultLowerer`.
pub fn find_lowerer<'a>(
    kind: MarkKind,
    registry: &'a [(MarkKind, Box<dyn MarkLower>)],
) -> &'a dyn MarkLower {
    registry
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| l.as_ref())
        .unwrap_or(&DefaultLowerer)
}

/// Compile a selection into a predicate.
///
/// Crossfilter resolution drops predicates whose source matches `self_source`.
/// Empty selections render as `Predicate::True` (WHERE TRUE).
/// Union combines with OR; Intersect combines with AND.
pub fn compile_selection(
    selection: &SelectionNode,
    self_source: &str,
    predicates: &[(String, Predicate)], // (source_name, predicate)
) -> Predicate {
    let resolution = SelectionResolution::from(selection.select);

    let active: Vec<Predicate> = match resolution {
        SelectionResolution::Crossfilter => {
            // Drop predicates from self_source
            predicates
                .iter()
                .filter(|(source, _)| source != self_source)
                .map(|(_, p)| p.clone())
                .collect()
        }
        _ => predicates.iter().map(|(_, p)| p.clone()).collect(),
    };

    if active.is_empty() {
        return Predicate::True;
    }

    match resolution {
        SelectionResolution::Union => Predicate::Or(active),
        SelectionResolution::Intersect | SelectionResolution::Crossfilter => {
            Predicate::And(active)
        }
        SelectionResolution::Single => {
            // Single: take the last predicate
            active.into_iter().last().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::MarkData;
    use brightfield_spec::vocab::{ImplStatus, SelectionResolution as AstRes};

    fn make_mark(kind: MarkKind) -> Mark {
        Mark {
            kind,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::From {
                source: "flights".to_string(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options: IndexMap::new(),
        }
    }

    fn make_ctx() -> LowerCtx<'static> {
        // Use leaked static for test convenience
        let data_sources: &'static IndexMap<String, brightfield_spec::ast::DataSource> =
            Box::leak(Box::new(IndexMap::new()));
        let params: &'static IndexMap<String, ParamNode> =
            Box::leak(Box::new(IndexMap::new()));
        LowerCtx {
            data_sources,
            params,
        }
    }

    #[test]
    fn dfir_ac03_default_lowerer_returns_unsupported() {
        let mark = make_mark(MarkKind::Line);
        let ctx = make_ctx();
        let result = DefaultLowerer.lower(&mark, &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "line"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn dfir_ac03_find_lowerer_falls_back_to_default() {
        let registry = default_lowerers();
        // Hexbin is not registered — should fall back to DefaultLowerer
        let lowerer = find_lowerer(MarkKind::Hexbin, &registry);
        let mark = make_mark(MarkKind::Hexbin);
        let ctx = make_ctx();
        let result = lowerer.lower(&mark, &ctx);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    // --- AC-01 tests: SimpleLowerer ---

    #[test]
    fn msv_ac01_simple_lowerer_produces_source_for_from() {
        let mark = make_mark(MarkKind::Dot);
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        assert_eq!(
            result.unwrap(),
            QueryPlan::Source {
                table: "flights".to_string()
            }
        );
    }

    #[test]
    fn msv_ac01_simple_lowerer_rejects_no_data() {
        let mark = Mark {
            kind: MarkKind::Dot,
            status: ImplStatus::Unimplemented,
            data: None,
            options: IndexMap::new(),
        };
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    #[test]
    fn msv_ac01_simple_lowerer_rejects_inline_data() {
        let mark = Mark {
            kind: MarkKind::Line,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::Inline(vec![])),
            options: IndexMap::new(),
        };
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "line"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn msv_ac01_default_lowerers_contains_all_registered_kinds() {
        let registry = default_lowerers();
        let kinds: Vec<MarkKind> = registry.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&MarkKind::Dot));
        assert!(kinds.contains(&MarkKind::Line));
        assert!(kinds.contains(&MarkKind::AreaY));
        assert!(kinds.contains(&MarkKind::AreaX));
        assert!(kinds.contains(&MarkKind::RuleX));
        assert!(kinds.contains(&MarkKind::RuleY));
        assert!(kinds.contains(&MarkKind::Text));
        assert!(kinds.contains(&MarkKind::BarX));
        assert!(kinds.contains(&MarkKind::BarY));
        assert!(kinds.contains(&MarkKind::Rect));
        assert!(kinds.contains(&MarkKind::RectX));
        assert!(kinds.contains(&MarkKind::RectY));
        assert!(kinds.contains(&MarkKind::RegressionY));
        assert!(kinds.contains(&MarkKind::RegressionX));
        assert!(kinds.contains(&MarkKind::DensityX));
        assert!(kinds.contains(&MarkKind::DensityY));
        assert!(kinds.contains(&MarkKind::Density));
        assert!(kinds.contains(&MarkKind::Raster));
        assert!(kinds.contains(&MarkKind::Heatmap));
        assert!(kinds.contains(&MarkKind::Cell));
        assert_eq!(kinds.len(), 20);
    }

    #[test]
    fn msv_ac01_find_lowerer_returns_simple_for_registered() {
        let registry = default_lowerers();
        let lowerer = find_lowerer(MarkKind::Dot, &registry);
        let mark = make_mark(MarkKind::Dot);
        let ctx = make_ctx();
        // SimpleLowerer should succeed for a mark with data.from
        let result = lowerer.lower(&mark, &ctx);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            QueryPlan::Source {
                table: "flights".to_string()
            }
        );
    }

    #[test]
    fn dfir_ac05_crossfilter_excludes_self() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_a", &predicates);
        // view_a's predicate should be excluded
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("y < 10".to_string())])
        );
    }

    #[test]
    fn dfir_ac05_crossfilter_includes_in_other_view() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_b", &predicates);
        // view_b's predicate excluded, view_a's included
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("x > 1".to_string())])
        );
    }

    #[test]
    fn dfir_ac05_empty_selection_is_true() {
        let selection = SelectionNode {
            select: AstRes::Intersect,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates: Vec<(String, Predicate)> = vec![];
        let result = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(result, Predicate::True);
    }

    // -----------------------------------------------------------------------
    // gomb ac-06 / ac-07 — statistical-mark lowerers
    // -----------------------------------------------------------------------

    fn make_mark_with_options(
        kind: MarkKind,
        opts: Vec<(&str, SpecValue)>,
    ) -> Mark {
        let mut options: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
        for (k, v) in opts {
            options.insert(k.to_string(), ValueOrParamRef::Value(v));
        }
        Mark {
            kind,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::From {
                source: "athletes".to_string(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options,
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_emits_aggregate_scalar() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
            ],
        );
        let ctx = make_ctx();
        let plan = RegressionLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::AggregateScalar { aggregates, .. } => {
                assert!(aggregates.iter().any(|a| a.contains("regr_slope")));
                assert!(aggregates.iter().any(|a| a.contains("regr_intercept")));
                assert!(aggregates.iter().any(|a| a.contains("regr_count")));
                assert!(aggregates.iter().any(|a| a.contains("regr_avgx")));
                assert!(aggregates.iter().any(|a| a.contains("regr_sxx")));
                assert!(aggregates.iter().any(|a| a.contains("regr_sxy")));
                assert!(aggregates.iter().any(|a| a.contains("regr_syy")));
                // Data extents the renderer builds its x/y scales from.
                assert!(aggregates.iter().any(|a| a.contains("AS x_min")));
                assert!(aggregates.iter().any(|a| a.contains("AS x_max")));
                assert!(aggregates.iter().any(|a| a.contains("AS y_min")));
                assert!(aggregates.iter().any(|a| a.contains("AS y_max")));
            }
            other => panic!("expected AggregateScalar, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_with_stroke_groups_by() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("stroke", SpecValue::String("sport".to_string())),
            ],
        );
        let ctx = make_ctx();
        let plan = RegressionLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::Aggregation { group_by, .. } => {
                assert_eq!(group_by.len(), 1);
                assert!(group_by[0].contains("sport"));
            }
            other => panic!("expected grouped Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_rejects_polynomial() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("a".to_string())),
                ("y", SpecValue::String("b".to_string())),
                ("type", SpecValue::String("polynomial".to_string())),
            ],
        );
        let ctx = make_ctx();
        let result = RegressionLowerer.lower(&mark, &ctx);
        match result {
            Err(EmitError::UnsupportedMark { kind }) => {
                assert!(kind.contains("polynomial"));
            }
            other => panic!("expected UnsupportedMark with polynomial, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac07_density_lowerer_1d_x_uses_equiwidth_bucket() {
        let mark = make_mark_with_options(
            MarkKind::DensityX,
            vec![("x", SpecValue::String("weight".to_string()))],
        );
        let ctx = make_ctx();
        let plan = DensityLowerer {
            kind: DensityLowerKind::OneDX,
        }
        .lower(&mark, &ctx)
        .expect("lowers");
        // Outermost node orders by the bucket centre — GROUP BY row order is
        // unspecified, and draw order must be deterministic (raster AA edges).
        let QueryPlan::Order { input, keys } = plan else {
            panic!("expected Order-wrapped Aggregation");
        };
        assert_eq!(keys, vec![("\"weight\"".to_string(), SortDir::Asc)]);
        match *input {
            QueryPlan::Aggregation {
                group_by,
                aggregates,
                ..
            } => {
                assert_eq!(group_by.len(), 1);
                // Portable equiwidth binning (no width_bucket — see follow-up #4),
                // emitting the bucket CENTRE (not the index) aliased to the channel
                // column name. `0.5` pins the centre offset (an index form would
                // have `+ 1` and no `0.5`); `least` pins the top-bucket clamp.
                assert!(group_by[0].contains("floor"));
                assert!(group_by[0].contains("0.5"));
                assert!(group_by[0].contains("least"));
                assert!(group_by[0].contains("AS \"weight\""));
                assert_eq!(aggregates.len(), 1);
                assert!(aggregates[0].contains("COUNT"));
            }
            other => panic!("expected Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac07_density_lowerer_2d_uses_two_buckets() {
        let mark = make_mark_with_options(
            MarkKind::Density,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("thresholds", SpecValue::Integer(16)),
            ],
        );
        let ctx = make_ctx();
        let plan = DensityLowerer {
            kind: DensityLowerKind::TwoD,
        }
        .lower(&mark, &ctx)
        .expect("lowers");
        // Deterministic draw order: x centre, then y centre.
        let QueryPlan::Order { input, keys } = plan else {
            panic!("expected Order-wrapped Aggregation");
        };
        assert_eq!(
            keys,
            vec![
                ("\"weight\"".to_string(), SortDir::Asc),
                ("\"height\"".to_string(), SortDir::Asc),
            ]
        );
        match *input {
            QueryPlan::Aggregation { group_by, .. } => {
                assert_eq!(group_by.len(), 2);
                // Each axis's bucket CENTRE (note `0.5`, not an index) is aliased
                // to its channel column.
                assert!(group_by[0].contains("AS \"weight\""));
                assert!(group_by[0].contains("0.5"));
                assert!(group_by[1].contains("AS \"height\""));
                assert!(group_by[1].contains("0.5"));
                // honours thresholds=16 in the bucket count
                assert!(group_by[0].contains("16") || group_by[1].contains("16"));
            }
            other => panic!("expected 2D Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_default_lowerers_includes_statistical_kinds() {
        let registry = default_lowerers();
        let kinds: Vec<MarkKind> = registry.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&MarkKind::RegressionY));
        assert!(kinds.contains(&MarkKind::RegressionX));
        assert!(kinds.contains(&MarkKind::Density));
        assert!(kinds.contains(&MarkKind::DensityX));
        assert!(kinds.contains(&MarkKind::DensityY));
    }

    #[test]
    fn dfir_ac05_union_combines_with_or() {
        let selection = SelectionNode {
            select: AstRes::Union,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(
            result,
            Predicate::Or(vec![
                Predicate::Expr("x > 1".to_string()),
                Predicate::Expr("y < 10".to_string()),
            ])
        );
    }
}
