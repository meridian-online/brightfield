//! AST → IR lowering.
//!
//! The `MarkLower` trait is the extension point for per-mark lowering. V1
//! ships no concrete implementations — every `MarkKind` returns
//! `EmitError::UnsupportedMark`. As marks are implemented in future cards,
//! each gets a `MarkLower` impl registered in `default_lowerers`.

use indexmap::IndexMap;

use brightfield_spec::ast::{
    AggregateFunc, Mark, MarkData, ParamNode, SelectionNode, SpecValue, ValueOrParamRef,
};
use brightfield_spec::vocab::MarkKind;

use crate::error::EmitError;
use crate::ir::{
    AggregateCall, AggregateExpr, AggregateFunction, Predicate, QueryPlan, SelectionResolution,
    SortDir,
};

/// Context available during lowering — spec-level data sources, params, selections.
#[derive(Debug)]
pub struct LowerCtx<'a> {
    /// All named data sources from the spec.
    pub data_sources: &'a IndexMap<String, brightfield_spec::ast::DataSource>,
    /// All named params from the spec.
    pub params: &'a IndexMap<String, ParamNode>,
    /// The mark's enclosing plot AREA in pixels `(width, height)` — the plot's
    /// declared `width`/`height` minus the Observable-default margins — computed
    /// at emit time from the spec. `HexbinLowerer` needs it to bin in
    /// **pixel space** (Mosaic's `binWidth` is a pixel quantity, so hexes look
    /// regular on screen). `None` when the mark has no enclosing plot (rare) —
    /// the lowerer then falls back to the default plot area. Other lowerers
    /// ignore it.
    pub plot_px: Option<(f64, f64)>,
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

/// Lowerer for the geo mark — projected GeoJSON basemap / choropleth
/// (last mark).
///
/// Geo is NOT an aggregation mark: this is a near-clone of [`SimpleLowerer`]
/// (`SELECT *`, the private `apply_data_filter` / `project_param_channels`)
/// with ONE projection. When the mark's source is a SPATIAL source
/// (`type: spatial` → `ST_Read`, a DuckDB `GEOMETRY` column), the geometry
/// column is wrapped in `ST_AsGeoJSON(...)` so the `GEOMETRY` crosses
/// `query_arrow` as GeoJSON `VARCHAR` (the renderer parses it with serde_json).
/// An INLINE source whose geom column is already `VARCHAR` GeoJSON text passes
/// through unwrapped — `ST_AsGeoJSON` on a `VARCHAR` errors and would need the
/// spatial extension anyway; that pass-through is exactly what lets the
/// hermetic inline example baseline with no spatial extension / network.
///
/// The SOURCE geometry column is
/// [`resolve_geometry_column`](brightfield_spec::layout::resolve_geometry_column)
/// (default `geom`), but the OUTPUT is always aliased to the canonical
/// [`DEFAULT_GEOMETRY_COLUMN`](brightfield_spec::layout::DEFAULT_GEOMETRY_COLUMN)
/// (`geom`) — the reserved-column idiom the density / hexbin lowerers already
/// use (`__bf_count`, `__bf_hex_*`). So the renderer reads one fixed column name
/// and never depends on the author's `geometry:` choice: a `geometry: shape`
/// mark still renders (its `shape` column is renamed to `geom`), and no
/// launch-anchored rebuild or colour-cycle can blank it on a name mismatch. The
/// default `geometry: geom` path keeps the byte-identical `* REPLACE` form.
pub struct GeoLowerer;

impl MarkLower for GeoLowerer {
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let (source, extras) = match &mark.data {
            Some(MarkData::From { source, extras, .. }) => (source.clone(), extras),
            Some(MarkData::Inline(_)) | None => {
                return Err(EmitError::UnsupportedMark {
                    kind: "geo (requires data: { from: ... })".to_string(),
                })
            }
        };
        let base = QueryPlan::Source {
            table: source.clone(),
        };
        let filtered = apply_data_filter(extras, base);

        let src = brightfield_spec::layout::resolve_geometry_column(mark);
        let canon = brightfield_spec::layout::DEFAULT_GEOMETRY_COLUMN;

        if source_is_spatial(ctx, &source) {
            // GEOMETRY → GeoJSON VARCHAR, canonicalised to `geom`. The default
            // column keeps the `* REPLACE` form (preserves order, byte-identical);
            // a custom `geometry:` column is EXCLUDEd and re-added under `geom`.
            let col = if src == canon {
                format!("* REPLACE (ST_AsGeoJSON(\"{src}\") AS \"{canon}\")")
            } else {
                format!("* EXCLUDE (\"{src}\"), ST_AsGeoJSON(\"{src}\") AS \"{canon}\"")
            };
            Ok(QueryPlan::Projection {
                input: Box::new(filtered),
                columns: vec![col],
            })
        } else if src == canon {
            // Inline VARCHAR GeoJSON, default column — pass through unwrapped,
            // exactly like SimpleLowerer (byte-identical).
            Ok(project_param_channels(mark, filtered))
        } else {
            // Inline VARCHAR GeoJSON under a custom `geometry:` column — rename it
            // to the canonical `geom` so the renderer finds it (no ST_AsGeoJSON:
            // it is already text).
            Ok(QueryPlan::Projection {
                input: Box::new(filtered),
                columns: vec![format!("* EXCLUDE (\"{src}\"), \"{src}\" AS \"{canon}\"")],
            })
        }
    }
}

/// Whether the named source is a `type: spatial` source (`ST_Read`, a DuckDB
/// `GEOMETRY` column) — the same signal `brightfield_sql::source::emit_spatial`
/// dispatches on. Checks the parsed [`DataSource`](brightfield_spec::ast::DataSource)
/// `kind`/`extras`. A source absent from the map (rare) reads as non-spatial.
fn source_is_spatial(ctx: &LowerCtx<'_>, source: &str) -> bool {
    ctx.data_sources.get(source).is_some_and(|ds| {
        matches!(&ds.kind, brightfield_spec::ast::DataSourceKind::Typed(t) if t == "spatial")
            || matches!(ds.extras.get("type"), Some(SpecValue::String(t)) if t == "spatial")
    })
}

/// Positional-channel option keys whose `$param` binding is projected into the
/// query (Decision 2). Kept in sync with brightfield-render's
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
/// in a WHERE (Decision 2). A no-op when there is no filter.
///
/// A filter that references a `$param` parses to an `Expression` (rendered to raw
/// SQL text with the `$name` preserved for emit-time interpolation, so the param
/// re-filters rows on propagate_param); a param-FREE filter (e.g. `"x > 2"`)
/// parses to a plain `String` that is already raw predicate text, used verbatim
/// (NOT quoted — quoting would turn the WHERE into a constant-string no-op that
/// silently passes every row).
pub(crate) fn apply_data_filter(
    extras: &IndexMap<String, SpecValue>,
    plan: QueryPlan,
) -> QueryPlan {
    match data_filter_sql(extras) {
        Some(predicate) => QueryPlan::Filter {
            input: Box::new(plan),
            predicate: Predicate::Expr(predicate),
        },
        None => plan,
    }
}

/// The raw SQL predicate text of a `data: { from, filter: "..." }` clause, or
/// `None` when there is no filter. A `$param` filter is an `Expression`
/// (rendered with the `$name` preserved for emit-time interpolation); a
/// param-free filter is a plain `String` used verbatim. The self-binning marks
/// (hexbin / self-aggregating cell) need this text — not just the plan wrapper
/// [`apply_data_filter`] returns — so they can push it into their extent
/// subqueries as well as the row WHERE.
fn data_filter_sql(extras: &IndexMap<String, SpecValue>) -> Option<String> {
    match extras.get("filter") {
        Some(expr @ SpecValue::Expression(_)) => Some(crate::emit::spec_value_to_sql_literal(expr)),
        Some(SpecValue::String(s)) => Some(s.clone()),
        _ => None,
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
                    kind: format!(
                        "{} (type='{}' — only linear is supported)",
                        mark.kind.wire_name(),
                        t
                    ),
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

        // Typed aggregate calls (function + arguments as data) — every
        // expression this lowerer emits is analyzable, so nothing stays a raw
        // string. The regr_* family takes (y, x) in that order.
        let regr = |func: AggregateFunction, alias: &str| {
            AggregateExpr::Call(AggregateCall {
                func,
                args: vec![format!("\"{y_col}\""), format!("\"{x_col}\"")],
                filter: None,
                cast: None,
                alias: Some(alias.to_string()),
            })
        };
        let extent = |func: AggregateFunction, col: &str, alias: &str| {
            AggregateExpr::Call(AggregateCall {
                func,
                args: vec![format!("\"{col}\"")],
                filter: None,
                cast: Some("DOUBLE".to_string()),
                alias: Some(alias.to_string()),
            })
        };
        let aggregates = vec![
            regr(AggregateFunction::RegrSlope, "slope"),
            regr(AggregateFunction::RegrIntercept, "intercept"),
            regr(AggregateFunction::RegrCount, "n"),
            regr(AggregateFunction::RegrAvgx, "x_bar"),
            regr(AggregateFunction::RegrSxx, "sxx"),
            regr(AggregateFunction::RegrSxy, "sxy"),
            regr(AggregateFunction::RegrSyy, "syy"),
            // Data extents for render-time scale construction. The executed
            // batch holds only coefficients (no raw x/y rows), so the renderer
            // can't infer x/y scales from a column — it builds them from these
            // extents instead (see `RegressionRenderer::augment_scales`). The
            // filter above already drops NULL x/y, so min/max are over the
            // fitted sample.
            extent(AggregateFunction::Min, x_col, "x_min"),
            extent(AggregateFunction::Max, x_col, "x_max"),
            extent(AggregateFunction::Min, y_col, "y_min"),
            extent(AggregateFunction::Max, y_col, "y_max"),
        ];

        // Group by stroke column when present. (The Aggregation IR variant
        // places group_by columns first via render_query, so the group key
        // appears in the projection without duplicating it here.)
        if let Some(stroke_col) = opt_string(&mark.options, "stroke") {
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
/// statistical-marks implementation note). The bin width is computed
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

/// The per-bucket occupancy count as a typed aggregate call — renders
/// `CAST(COUNT(*) AS DOUBLE) AS __bf_count`, byte-identical to the string form
/// it replaced. Shared by the density lowerers and by [`RectLowerer`]'s binned
/// histogram, which reads the same reserved column on the renderer side.
fn density_count_expr() -> AggregateExpr {
    AggregateExpr::Call(AggregateCall {
        func: AggregateFunction::Count,
        args: vec!["*".to_string()],
        filter: None,
        cast: Some("DOUBLE".to_string()),
        alias: Some("__bf_count".to_string()),
    })
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
    let count_expr = density_count_expr();

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
    let x_centre = format!(
        "{} AS \"{x_col}\"",
        equiwidth_bin_centre(table, x_col, bins)
    );
    let y_centre = format!(
        "{} AS \"{y_col}\"",
        equiwidth_bin_centre(table, y_col, bins)
    );
    // Reserved occupancy alias — see build_density_1d.
    let count_expr = density_count_expr();

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

/// Contour's lowerer: `DensityLowerer { TwoD }` behind an attribute shield.
///
/// On a contour mark `thresholds` counts ISO-LEVELS (Mosaic semantics,
/// consumed by the renderer); `DensityLowerer` reads `thresholds` as its SQL
/// BIN count. The shield strips `thresholds` from the option bag before
/// delegating, so the collision can never reach the emitted SQL — `bins`
/// stays available to size the grid. This is a registration-layer concern
/// only; the parser is untouched (density marks).
pub struct ContourAttrShield {
    inner: DensityLowerer,
}

impl MarkLower for ContourAttrShield {
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let mut shielded = mark.clone();
        shielded.options.shift_remove("thresholds");
        self.inner.lower(&shielded, ctx)
    }
}

/// Reserved geometry columns the hexbin lowerer emits (constant per row): the
/// hexagon half-width and half-height in DATA units. The renderer reconstructs
/// the six pointy-top vertices from these, so the hex is regular on screen by
/// construction and survives live rebuilds without a bin-step recovery. Must
/// match `brightfield-render`'s `HEX_DX_COL` / `HEX_DY_COL`.
const HEX_DX_COL: &str = "__bf_hex_dx";
const HEX_DY_COL: &str = "__bf_hex_dy";

/// Reserved RAW-extent columns (constant per row): the raw table `min`/`max` of
/// the x/y channels — the extent the lowerer binned in pixel space over. The
/// renderer widens the positional scales from THESE (raw-anchored domain), not
/// the occupied-centre span, so the widened domain encodes the exact raw
/// pixel→data pitch and a sibling hexgrid can reconstruct the lattice exactly.
/// Must match `brightfield-render`'s `HEX_X0_COL` … `HEX_Y1_COL`.
const HEX_X0_COL: &str = "__bf_hex_x0";
const HEX_X1_COL: &str = "__bf_hex_x1";
const HEX_Y0_COL: &str = "__bf_hex_y0";
const HEX_Y1_COL: &str = "__bf_hex_y1";

/// Reserved count column (shared with the density lowerers) — a `fill: {count:}`
/// hexbin aggregates into this. Must match `DENSITY_COUNT_COL` in the renderer.
const HEX_COUNT_COL: &str = "__bf_count";

/// Default `binWidth` in pixels — Mosaic's hexbin default.
const DEFAULT_BIN_WIDTH: f64 = 20.0;

/// Fallback plot AREA (pixels) when the lower context carries no plot extent
/// (a mark outside any plot). Mosaic default plot 640×400 minus the default
/// margins (60 horizontal, 50 vertical) — see `brightfield-sql::emit`.
const FALLBACK_PLOT_PX: (f64, f64) = (640.0 - 60.0, 400.0 - 50.0);

/// Lowerer for the hexbin mark — Mosaic's flagship at-scale mark.
///
/// Hexbin is SELF-AGGREGATING: the `fill: {count:}` / `fill: {avg: col}`
/// channel carries the aggregate. `binWidth` is a PIXEL quantity (default 20),
/// so binning happens in **pixel space** (hexes look regular on screen, as
/// d3-hexbin / Observable Plot draw them): each point's `(x, y)` is normalised
/// into the plot's pixel extent (a static emit-time size × SQL-side min/max
/// extents), transformed to axial hex coordinates and **cube-rounded** to the
/// nearest pointy-top hex at hex size `r = binWidth / √3` (so the horizontal
/// centre spacing equals `binWidth` — verified against Observable Plot's
/// `hexbin`: `dx = binWidth`, `dy = binWidth·√3/2`, `r = binWidth/√3`).
///
/// The lowerer GROUP BYs the hex cell and emits, per hex, the centre in DATA
/// units (aliased to the x/y channel columns, so it flows through the ordinary
/// positional scales), the aggregate (`__bf_count` for count, the source column
/// for avg), and the constant hex half-extents `__bf_hex_dx`/`__bf_hex_dy` in
/// data units. Deterministic `ORDER BY` on the emitted centres (the #42 jitter
/// class). All arithmetic — no `width_bucket` (absent from the bundled
/// libduckdb; the density-lowerer precedent).
///
/// `binWidth` is literal-only in v1 (`binWidth: $param` threading is deferred).
pub struct HexbinLowerer;

impl MarkLower for HexbinLowerer {
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let (source, filter) = match &mark.data {
            Some(MarkData::From { source, extras, .. }) => {
                (source.clone(), data_filter_sql(extras))
            }
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: "hexbin (requires data: { from: ... })".to_string(),
                })
            }
        };
        let x_col = opt_string(&mark.options, "x").ok_or_else(|| EmitError::UnsupportedMark {
            kind: "hexbin (missing x)".to_string(),
        })?;
        let y_col = opt_string(&mark.options, "y").ok_or_else(|| EmitError::UnsupportedMark {
            kind: "hexbin (missing y)".to_string(),
        })?;
        let bin_width = opt_f64(&mark.options, "binWidth").unwrap_or(DEFAULT_BIN_WIDTH);
        let (plot_w, plot_h) = ctx.plot_px.unwrap_or(FALLBACK_PLOT_PX);
        // Default aggregate is count (Mosaic's hexbin default fill).
        let agg = opt_aggregate(&mark.options, "fill").unwrap_or((AggregateFunc::Count, None));

        Ok(build_hexbin_plan(
            &source,
            x_col,
            y_col,
            bin_width,
            plot_w,
            plot_h,
            &agg,
            filter.as_deref(),
        ))
    }
}

/// Read a self-aggregating channel (`SpecValue::Aggregate`) at `key`.
fn opt_aggregate(
    options: &IndexMap<String, ValueOrParamRef<SpecValue>>,
    key: &str,
) -> Option<(AggregateFunc, Option<String>)> {
    match options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Aggregate { func, column }) => {
            Some((*func, column.clone()))
        }
        _ => None,
    }
}

/// Build the hexbin QueryPlan. See [`HexbinLowerer`] for the geometry.
#[allow(clippy::too_many_arguments)]
fn build_hexbin_plan(
    table: &str,
    x_col: &str,
    y_col: &str,
    bin_width: f64,
    plot_w: f64,
    plot_h: f64,
    agg: &(AggregateFunc, Option<String>),
    filter: Option<&str>,
) -> QueryPlan {
    // √3 and the hex size. size = binWidth/√3 makes the horizontal centre
    // spacing exactly binWidth (Observable Plot parity). (`f64::consts::SQRT_3`
    // is still unstable at the 1.95 floor, so the literal is pinned here.)
    let sqrt3 = 1.732_050_807_568_877_2_f64;
    let size = bin_width / sqrt3;

    // The data filter (`data: { filter }`) must scope BOTH the binned rows and
    // the extent subqueries — binning over the unfiltered extent would shift
    // every centre and let filtered-out rows widen the plot (F4). Push it into
    // the extent WHERE and AND it into the row predicate.
    let extent_where = filter.map(|f| format!(" WHERE ({f})")).unwrap_or_default();

    // Raw-table extents as correlated scalar subqueries (density precedent).
    let xmin = format!("(SELECT min(\"{x_col}\") FROM \"{table}\"{extent_where})");
    let xmax = format!("(SELECT max(\"{x_col}\") FROM \"{table}\"{extent_where})");
    let ymin = format!("(SELECT min(\"{y_col}\") FROM \"{table}\"{extent_where})");
    let ymax = format!("(SELECT max(\"{y_col}\") FROM \"{table}\"{extent_where})");

    // Point → pixel space over the plot AREA. A DEGENERATE axis (constant value
    // / single row → span 0) maps every point to the plot MIDPOINT rather than
    // NULL (d3's degenerate-domain → range-midpoint): binning then still yields
    // real centres — one hex, or one line of hexes when the other axis varies —
    // instead of the whole mark vanishing on a NULL centre (F3; the density
    // lowerer's graceful precedent).
    let half_w = plot_w / 2.0;
    let half_h = plot_h / 2.0;
    let px = format!(
        "(CASE WHEN ({xmax} - {xmin}) = 0 THEN {half_w} \
         ELSE (\"{x_col}\" - {xmin}) / ({xmax} - {xmin}) * {plot_w} END)"
    );
    let py = format!(
        "(CASE WHEN ({ymax} - {ymin}) = 0 THEN {half_h} \
         ELSE (\"{y_col}\" - {ymin}) / ({ymax} - {ymin}) * {plot_h} END)"
    );

    // Pixel → axial (pointy-top), hex size `size`.
    let qf = format!(
        "(({frac} * {px} - {third} * {py}) / {size})",
        frac = sqrt3 / 3.0,
        third = 1.0 / 3.0
    );
    let rf = format!("(({twothirds} * {py}) / {size})", twothirds = 2.0 / 3.0);

    let row_filter = filter.map(|f| format!(" AND ({f})")).unwrap_or_default();
    let filtered = QueryPlan::Filter {
        input: Box::new(QueryPlan::Source {
            table: table.to_string(),
        }),
        predicate: Predicate::Expr(format!(
            "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL{row_filter}"
        )),
    };
    // Layer 1: fractional axial coordinates.
    let axial = QueryPlan::Projection {
        input: Box::new(filtered),
        columns: vec![
            "*".to_string(),
            format!("{qf} AS __bf_qf"),
            format!("{rf} AS __bf_rf"),
        ],
    };
    // Layer 2: the three cube-rounded integer coordinates (x=q, z=r, y=-q-r).
    let rounded = QueryPlan::Projection {
        input: Box::new(axial),
        columns: vec![
            "*".to_string(),
            "round(__bf_qf) AS __bf_rx".to_string(),
            "round(-__bf_qf - __bf_rf) AS __bf_ry".to_string(),
            "round(__bf_rf) AS __bf_rz".to_string(),
        ],
    };
    // Layer 3: cube-round resolution → final axial (q, r). The component with
    // the largest rounding error is recomputed from the other two.
    let xd = "abs(__bf_rx - __bf_qf)";
    let yd = "abs(__bf_ry - (-__bf_qf - __bf_rf))";
    let zd = "abs(__bf_rz - __bf_rf)";
    let q_expr =
        format!("CASE WHEN {xd} > {yd} AND {xd} > {zd} THEN (-__bf_ry - __bf_rz) ELSE __bf_rx END");
    let r_expr = format!(
        "CASE WHEN {xd} > {yd} AND {xd} > {zd} THEN __bf_rz \
         WHEN {yd} > {zd} THEN __bf_rz ELSE (-__bf_rx - __bf_ry) END"
    );
    let hexed = QueryPlan::Projection {
        input: Box::new(rounded),
        columns: vec![
            "*".to_string(),
            format!("{q_expr} AS __bf_q"),
            format!("{r_expr} AS __bf_r"),
        ],
    };

    // Hex centre in pixel space, then back to data units (inverse of px/py).
    let cx_px = format!(
        "({size} * ({sqrt3} * __bf_q + {half_sqrt3} * __bf_r))",
        half_sqrt3 = sqrt3 / 2.0
    );
    let cy_px = format!("({size} * ({onehalf} * __bf_r))", onehalf = 1.5);
    let cx_data = format!("({xmin} + {cx_px} / {plot_w} * ({xmax} - {xmin}))");
    let cy_data = format!("({ymin} + {cy_px} / {plot_h} * ({ymax} - {ymin}))");

    // Constant hex half-extents in data units: half-width = binWidth/2 px,
    // half-height = size px, each scaled by the axis's data-per-pixel ratio.
    let dx_data = format!(
        "CAST({half_bw} / {plot_w} * ({xmax} - {xmin}) AS DOUBLE)",
        half_bw = bin_width / 2.0
    );
    let dy_data = format!("CAST({size} / {plot_h} * ({ymax} - {ymin}) AS DOUBLE)");

    // The aggregate: count → reserved column; column-taking aggregates → the
    // source column aliased to itself (so the fill channel reads it).
    let agg_expr = hex_aggregate_expr(agg);

    let aggregation = QueryPlan::Aggregation {
        input: Box::new(hexed),
        group_by: vec![
            format!("CAST({cx_data} AS DOUBLE) AS \"{x_col}\""),
            format!("CAST({cy_data} AS DOUBLE) AS \"{y_col}\""),
        ],
        aggregates: vec![
            // The fill aggregate is a typed call; the geometry/extent columns
            // below are constant-per-group SCALAR expressions (correlated
            // subqueries over the raw table), not aggregate calls — they stay
            // raw strings, which downstream aggregate analysis treats as
            // opaque (the designed bail-to-direct-query fallback).
            agg_expr,
            AggregateExpr::Raw(format!("{dx_data} AS {HEX_DX_COL}")),
            AggregateExpr::Raw(format!("{dy_data} AS {HEX_DY_COL}")),
            // Raw extent (constant per group) so the renderer widens the scales
            // RAW-anchored — the exactness the hexgrid reconstruction rides on.
            AggregateExpr::Raw(format!("CAST({xmin} AS DOUBLE) AS {HEX_X0_COL}")),
            AggregateExpr::Raw(format!("CAST({xmax} AS DOUBLE) AS {HEX_X1_COL}")),
            AggregateExpr::Raw(format!("CAST({ymin} AS DOUBLE) AS {HEX_Y0_COL}")),
            AggregateExpr::Raw(format!("CAST({ymax} AS DOUBLE) AS {HEX_Y1_COL}")),
        ],
    };

    // Deterministic draw order (x centre, then y centre) — the #42 jitter class.
    QueryPlan::Order {
        input: Box::new(aggregation),
        keys: vec![
            (format!("\"{x_col}\""), SortDir::Asc),
            (format!("\"{y_col}\""), SortDir::Asc),
        ],
    }
}

/// Reserved HIGH bin-edge columns a binned rect emits. Keyed by AXIS rather
/// than by column, so a future doubly-binned `rect` (both axes) is not
/// foreclosed. The LOW edge takes the source column's own name instead — that
/// is what makes `group_key_provides` match, so a navigation extent resolves
/// `UnderGroupBy` rather than `Decline`, and what gives the axis its derived
/// title. Must match `brightfield-render`'s `BIN_HI_X_COL` / `BIN_HI_Y_COL`.
const BIN_HI_X_COL: &str = "__bf_bin_x2";
const BIN_HI_Y_COL: &str = "__bf_bin_y2";

/// Reserved columns carrying the resolved bin scheme — the snapped low extent
/// and the bin width — projected ONCE beneath the aggregation and read by both
/// edge expressions above it.
///
/// They exist so the scheme is written once. Both are constant over the whole
/// table, and inlining them where they are used would repeat a nine-deep
/// subquery eight times per mark: ~9 KB of SQL and eight `min`/`max` scans for
/// two numbers. They never reach the renderer — the aggregation projects only
/// its group keys and its count.
const BIN_LO_EXTENT_COL: &str = "__bf_bin_lo";
const BIN_STEP_COL: &str = "__bf_bin_step";

/// Mosaic's default bin-count **hint** — `binSpec`'s `steps = 25`, read from
/// `mosaic-sql/src/transforms/util/bin-step.ts`.
///
/// Emphatically NOT the density lowerer's 100: that number is a KDE grid
/// resolution, and reusing it here would put four times Mosaic's bar count on
/// every histogram brightfield draws, with non-round tick values, from the same
/// spec. The vendored corpus is a portability claim, so a difference from the
/// reference belongs in `deviations.yaml` as a considered choice — not arriving
/// by copy-paste from a lowerer that had a different job.
const DEFAULT_BIN_STEPS: i64 = 25;

/// `Math.LN10` to the bit. The reference implementation derives every logarithm
/// as `Math.log(x) / Math.LN10`; emitting DuckDB's own `log10` instead would be
/// the same value up to a rounding that decides which side of a `floor` a
/// borderline span lands on, and so which step a histogram gets.
const JS_LN10: &str = "2.302585092994046";

/// Lowerer for the rect family, handling BOTH the pre-aggregated path and the
/// **binned histogram** — `x: {bin: col}` with `y: {count:}` and its transpose.
///
/// When the parser lifted a positional [`SpecValue::Bin`], this GROUP BYs the
/// two bin edges and counts; otherwise it delegates to [`SimpleLowerer`] and the
/// shipped explicit-interval rect path stays byte-for-byte identical.
///
/// **Why the lowerer and not the renderer.** Binning here is forced, not
/// preferred: `apply_selection_filter` threads a crossfilter predicate *beneath*
/// an `Aggregation`, and `axis_pushdown` resolves a navigation extent to
/// `UnderGroupBy` when a group key aliases back to the channel column. Both
/// arrive free with an `Aggregation` and are unreachable if binning happens at
/// draw time — which is the whole point of a histogram you can brush.
pub struct RectLowerer;

impl MarkLower for RectLowerer {
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let Some((axis, column, steps)) = positional_bin(&mark.options) else {
            // Pre-aggregated rect — unchanged (rect-histogram.png byte-identical).
            return SimpleLowerer.lower(mark, ctx);
        };
        let (source, filter) = match &mark.data {
            Some(MarkData::From { source, extras, .. }) => {
                (source.clone(), data_filter_sql(extras))
            }
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: "rect (a binned rect requires data: { from: ... })".to_string(),
                })
            }
        };

        let row_filter = filter.map(|f| format!(" AND ({f})")).unwrap_or_default();
        let filtered = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: source.clone(),
            }),
            predicate: Predicate::Expr(format!("\"{column}\" IS NOT NULL{row_filter}")),
        };

        // The bin scheme is resolved over the WHOLE table and carried alongside
        // the rows. Two consequences, both wanted: the edges are written once
        // rather than eight times, and the extent does not move when a brush
        // narrows the rows — a cross-filtered histogram whose bars re-bin on
        // every drag is not a histogram. Mosaic reaches the same place from the
        // other direction, asking for the field's min/max as *stats* and handing
        // `binHistogram` a fixed numeric extent.
        //
        // A selection predicate lands between this projection and the
        // aggregation (`apply_selection_filter` filters an aggregation's direct
        // input), and the projection passes `*` through, so the brushed column
        // is still in scope where the predicate is applied.
        let steps = steps.filter(|s| *s > 0).unwrap_or(DEFAULT_BIN_STEPS);
        let binned = QueryPlan::Projection {
            input: Box::new(filtered),
            columns: vec![
                "*".to_string(),
                format!(
                    "{} AS {BIN_LO_EXTENT_COL}",
                    bin_spec_field(&source, &column, steps, BinSpecField::Min)
                ),
                format!(
                    "{} AS {BIN_STEP_COL}",
                    bin_spec_field(&source, &column, steps, BinSpecField::Alpha)
                ),
            ],
        };

        let hi_col = match axis {
            BinAxis::X => BIN_HI_X_COL,
            BinAxis::Y => BIN_HI_Y_COL,
        };
        Ok(QueryPlan::Order {
            input: Box::new(QueryPlan::Aggregation {
                input: Box::new(binned),
                group_by: vec![
                    format!("{} AS \"{column}\"", bin_edge(&column, BinEdge::Low)),
                    format!("{} AS {hi_col}", bin_edge(&column, BinEdge::High)),
                ],
                aggregates: vec![density_count_expr()],
            }),
            // Deterministic row order — the same reason the density lowerers
            // order: GROUP BY output order is unspecified in DuckDB and the
            // renderer draws in row order.
            keys: vec![(format!("\"{column}\""), SortDir::Asc)],
        })
    }
}

/// Which positional axis a rect's `bin` sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinAxis {
    X,
    Y,
}

/// Which edge of a bin an expression produces. Mosaic emits the two as one
/// function differing only by an integer bin `offset` (0 and 1), and so do we.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinEdge {
    Low,
    High,
}

impl BinEdge {
    /// Mosaic's `offset` option — the number of bin steps to shift the result.
    fn offset(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::High => 1,
        }
    }
}

/// The positional bin a rect carries, as `(axis, column, steps hint)`.
fn positional_bin(
    options: &IndexMap<String, ValueOrParamRef<SpecValue>>,
) -> Option<(BinAxis, String, Option<i64>)> {
    for (key, axis) in [("x", BinAxis::X), ("y", BinAxis::Y)] {
        if let Some(ValueOrParamRef::Value(SpecValue::Bin { column, steps })) = options.get(key) {
            return Some((axis, column.clone(), *steps));
        }
    }
    None
}

/// One edge of the bin containing `col`, in data units, as Mosaic's
/// `binHistogram` writes it: `b.min + alpha * FLOOR((col - b.min) / alpha)`,
/// with the high edge taking the same expression at bin offset 1.
///
/// **There is deliberately no top-bin fold here**, and its absence is the
/// difference between this and `equiwidth_bin_centre` above. The density
/// lowerer bins over the RAW extent, so the maximum value always lands one
/// bucket past the last legitimate one and has to be clamped. Mosaic's extent
/// is snapped OUTWARD to a nice step first, so the fold is not needed — and
/// where the raw maximum does fall exactly on a step boundary, Mosaic puts it
/// in a bin of its own. Clamping would be a silent divergence from the
/// reference on a shape the vendored corpus contains.
fn bin_edge(col: &str, edge: BinEdge) -> String {
    let index =
        format!("floor((\"{col}\" - {BIN_LO_EXTENT_COL}) / CAST({BIN_STEP_COL} AS DOUBLE))");
    let index = match edge.offset() {
        0 => index,
        n => format!("({n} + {index})"),
    };
    format!("CAST({BIN_LO_EXTENT_COL} + {BIN_STEP_COL} * {index} AS DOUBLE)")
}

/// The two numbers a bin scheme reduces to: the snapped low extent and the bin
/// width. Mosaic computes both in JavaScript from a stats query; brightfield has
/// no stats phase, so each is a scalar subquery over the raw table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinSpecField {
    Min,
    Alpha,
}

/// One field of the bin scheme as a scalar subquery.
///
/// The degenerate `max == min` column takes Mosaic's degenerate branch —
/// `binHistogram` returns a bare `floor(field)` when the extent has no width,
/// which is what `bmin = 0`, `alpha = 1` reproduces exactly.
fn bin_spec_field(table: &str, col: &str, steps: i64, field: BinSpecField) -> String {
    let projection = match field {
        BinSpecField::Min => "CASE WHEN span IS NULL THEN 0 ELSE bmin END",
        BinSpecField::Alpha => {
            "CASE WHEN span IS NULL THEN 1 ELSE (bmax - bmin) / round((bmax - bmin) / step) END"
        }
    };
    format!(
        "(SELECT {projection} FROM ({}) AS _bs)",
        bin_spec_query(table, col, steps)
    )
}

/// Mosaic's `binSpec`, transliterated into a chain of one-row subqueries.
///
/// Each layer names the value the next one needs, which is what keeps the SQL
/// linear in the number of steps rather than exponential — `step` is derived
/// from `span`, and both are referenced several times.
///
/// Two departures from a literal transliteration, both forced and neither
/// arithmetic: `Math.LN10` is written as a literal ([`JS_LN10`]) so the
/// logarithms match the reference's rounding, and `binStep`'s `while (ceil(span
/// / step) > steps) step *= 10` becomes a three-way `CASE`. That loop runs **at
/// most twice**: `step` starts at `10^(round(log10 span) − ceil(log10 steps))`,
/// so `span / step ≤ 10^(0.5 + level)` while `steps > 10^(level − 1)`, giving a
/// ratio below `10^1.5 < 32`; two decades of division take it under 1.
///
/// `minstep` is 0 throughout (brightfield exposes no such knob), so `binStep`'s
/// two `v >= minstep` guards are vacuous and are not emitted.
fn bin_spec_query(table: &str, col: &str, steps: i64) -> String {
    // `Math.ceil(Math.log(steps) / Math.LN10)`. `steps` is a spec literal, so
    // this is a plan-time constant rather than more SQL.
    let level = ((steps as f64).ln() / std::f64::consts::LN_10).ceil();
    // The `eps` nudge Mosaic applies before flooring the low extent, expressed
    // over `step`: `precision = step >= 1 ? 0 : trunc(-ln(step)/LN10) + 1`,
    // `eps = 10^(-precision - 1)`.
    let eps = format!(
        "pow(10, -(CASE WHEN ln(step) >= 0 THEN 0 ELSE trunc(-ln(step) / {JS_LN10}) + 1 END) - 1)"
    );
    format!(
        "SELECT span, step, CASE WHEN lo < v THEN v - step ELSE v END AS bmin, \
         ceil(hi / step) * step AS bmax FROM (\
         SELECT lo, hi, span, step, floor(lo / step + {eps}) * step AS v FROM (\
         SELECT lo, hi, span, CASE WHEN span / (s2 / 2) <= {steps} THEN s2 / 2 ELSE s2 END AS step FROM (\
         SELECT lo, hi, span, CASE WHEN span / (s1 / 5) <= {steps} THEN s1 / 5 ELSE s1 END AS s2 FROM (\
         SELECT lo, hi, span, CASE WHEN ceil(span / s0) <= {steps} THEN s0 \
         WHEN ceil(span / (s0 * 10)) <= {steps} THEN s0 * 10 ELSE s0 * 100 END AS s1 FROM (\
         SELECT lo, hi, span, pow(10, floor(ln(span) / {JS_LN10} + 0.5) - {level}) AS s0 FROM (\
         SELECT lo, hi, nullif(hi - lo, 0) AS span FROM (\
         SELECT min(\"{col}\") AS lo, max(\"{col}\") AS hi FROM \"{table}\"\
         ) AS _b0) AS _b1) AS _b2) AS _b3) AS _b4) AS _b5) AS _b6"
    )
}

/// Lowerer for the cell mark, handling BOTH the pre-aggregated path and the
/// self-aggregating form. When `fill` is a self-aggregating channel
/// (`{count:}` / `{avg: col}`) on Band × Band categorical axes, it GROUP BYs
/// the two category columns and aggregates (aliased so the existing
/// `CellRenderer` numeric-fill path renders it unchanged). Otherwise it
/// delegates to [`SimpleLowerer`] — the shipped pre-aggregated cell path stays
/// byte-for-byte identical. cellX/cellY remain Unimplemented.
pub struct CellLowerer;

impl MarkLower for CellLowerer {
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let Some(agg) = opt_aggregate(&mark.options, "fill") else {
            // Pre-aggregated cell — unchanged (cell.png byte-identical).
            return SimpleLowerer.lower(mark, ctx);
        };
        let (source, filter) = match &mark.data {
            Some(MarkData::From { source, extras, .. }) => {
                (source.clone(), data_filter_sql(extras))
            }
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: "cell (self-aggregating requires data: { from: ... })".to_string(),
                })
            }
        };
        let x_col = opt_string(&mark.options, "x").ok_or_else(|| EmitError::UnsupportedMark {
            kind: "cell (missing x)".to_string(),
        })?;
        let y_col = opt_string(&mark.options, "y").ok_or_else(|| EmitError::UnsupportedMark {
            kind: "cell (missing y)".to_string(),
        })?;

        // The data filter scopes the aggregated rows (F4). Cell GROUP BYs the
        // category columns — no pixel extents to scope, so the row WHERE is the
        // only place it applies.
        let row_filter = filter.map(|f| format!(" AND ({f})")).unwrap_or_default();
        let filtered = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source { table: source }),
            predicate: Predicate::Expr(format!(
                "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL{row_filter}"
            )),
        };
        // GROUP BY the two category columns; aggregate aliased (count →
        // __bf_count, column aggregate → its source column) — the same
        // convention as hexbin, so the renderer's numeric-fill path reads it.
        Ok(QueryPlan::Order {
            input: Box::new(QueryPlan::Aggregation {
                input: Box::new(filtered),
                group_by: vec![format!("\"{x_col}\""), format!("\"{y_col}\"")],
                aggregates: vec![hex_aggregate_expr(&agg)],
            }),
            keys: vec![
                (format!("\"{x_col}\""), SortDir::Asc),
                (format!("\"{y_col}\""), SortDir::Asc),
            ],
        })
    }
}

/// Lowerer for the hexgrid mark — a decorative, DATALESS hex mesh drawn from
/// the plot extent (not from data). It emits a trivial single-row query
/// (`SELECT 1`) so the mark still produces a batch and is not skipped
/// downstream; the renderer draws the mesh from the plot-area pixel rect and
/// `binWidth`, ignoring the batch content. This is the minimal named
/// dataless-mark pathway.
pub struct HexgridLowerer;

impl MarkLower for HexgridLowerer {
    fn lower(&self, _mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        Ok(QueryPlan::Singleton {
            columns: vec!["1 AS __bf_hexgrid".to_string()],
        })
    }
}

/// The aggregate expression for a hexbin/cell fill aggregate, as a typed
/// call. `count` folds to the reserved `__bf_count`; a column aggregate
/// aliases to its source column so the fill channel reads it. A
/// column-taking aggregate with NO column degrades to count so the query is
/// still valid (the parser should have caught this).
fn hex_aggregate_expr(agg: &(AggregateFunc, Option<String>)) -> AggregateExpr {
    let count = || {
        AggregateExpr::Call(AggregateCall {
            func: AggregateFunction::Count,
            args: vec!["*".to_string()],
            filter: None,
            cast: Some("DOUBLE".to_string()),
            alias: Some(HEX_COUNT_COL.to_string()),
        })
    };
    match agg {
        (AggregateFunc::Count, _) | (_, None) => count(),
        (func, Some(col)) => {
            let func = match func {
                AggregateFunc::Sum => AggregateFunction::Sum,
                AggregateFunc::Avg => AggregateFunction::Avg,
                AggregateFunc::Min => AggregateFunction::Min,
                AggregateFunc::Max => AggregateFunction::Max,
                AggregateFunc::Count => unreachable!("count matched above"),
            };
            AggregateExpr::Call(AggregateCall {
                func,
                args: vec![format!("\"{col}\"")],
                filter: None,
                cast: Some("DOUBLE".to_string()),
                alias: Some(format!("\"{col}\"")),
            })
        }
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
        // Rect family — pre-aggregated x1/x2 rects delegate to SimpleLowerer;
        // a positional `{bin: col}` + `{count:}` pair becomes a GROUP BY over
        // the two bin edges. Registered for exactly the kinds
        // `MarkKind::bins_positionally` names, so the parser cannot lift a pair
        // this lowerer would then ignore.
        (MarkKind::Rect, Box::new(RectLowerer)),
        (MarkKind::RectX, Box::new(RectLowerer)),
        (MarkKind::RectY, Box::new(RectLowerer)),
        // Cell handles BOTH the pre-aggregated path (pass-through via
        // SimpleLowerer — one row per (x category, y category) pair with a
        // numeric fill column) AND the self-aggregating form (fill: {count:}/
        // {avg: col} → GROUP BY the two categories). See CellLowerer.
        (MarkKind::Cell, Box::new(CellLowerer)),
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
        // sibling. Zero new SQL (density marks).
        (
            MarkKind::Heatmap,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::TwoD,
            }),
        ),
        // Contour rides the same binning behind the thresholds attr-shield —
        // see ContourAttrShield.
        (
            MarkKind::Contour,
            Box::new(ContourAttrShield {
                inner: DensityLowerer {
                    kind: DensityLowerKind::TwoD,
                },
            }),
        ),
        // Hexbin — Mosaic's flagship at-scale mark, pixel-space hex binning
        // fully in SQL (hexbin follow-up).
        (MarkKind::Hexbin, Box::new(HexbinLowerer)),
        // Hexgrid — decorative dataless mesh; emits a singleton row.
        (MarkKind::Hexgrid, Box::new(HexgridLowerer)),
        // Geo — SimpleLowerer clone + ST_AsGeoJSON on a spatial geometry column
        // (inline VARCHAR GeoJSON passes through). See GeoLowerer.
        (MarkKind::Geo, Box::new(GeoLowerer)),
    ]
}

/// Look up a lowerer for a given `MarkKind`, falling back to `DefaultLowerer`.
pub fn find_lowerer(kind: MarkKind, registry: &[(MarkKind, Box<dyn MarkLower>)]) -> &dyn MarkLower {
    registry
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| l.as_ref())
        .unwrap_or(&DefaultLowerer)
}

/// A `self_source` that matches no real contributor path, so
/// [`compile_selection`]'s crossfilter branch excludes NOTHING.
///
/// Contributor paths are component paths (`root`, `root/hconcat[0]`, …), so a
/// NUL-prefixed sentinel can never collide with one.
///
/// Two callers pass it, for the same reason spelled two ways. A `highlight`
/// interactor must dim the brushed plot's OWN rows, so its membership
/// projection never self-excludes. And a reader asking what a selection
/// currently *holds* is not asking what one plot's WHERE resolved to: under
/// crossfilter every consumer drops a different clause, so there is no single
/// consumer whose resolution could stand for the selection's value. Compiling
/// against this sentinel gives the value itself — the resolution applied
/// (`AND` / `OR` / most-recent), nothing excluded.
pub const NO_SELF_EXCLUDE: &str = "\u{0}__bf_no_self_exclude";

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
        SelectionResolution::Intersect | SelectionResolution::Crossfilter => Predicate::And(active),
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
        let params: &'static IndexMap<String, ParamNode> = Box::leak(Box::new(IndexMap::new()));
        LowerCtx {
            data_sources,
            params,
            plot_px: None,
        }
    }

    #[test]
    fn default_lowerer_returns_unsupported() {
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
    fn find_lowerer_falls_back_to_default() {
        let registry = default_lowerers();
        // Voronoi is not registered — should fall back to DefaultLowerer.
        // (Geo now has a GeoLowerer; voronoi is the always-unimplemented stand-in.)
        let lowerer = find_lowerer(MarkKind::Voronoi, &registry);
        let mark = make_mark(MarkKind::Voronoi);
        let ctx = make_ctx();
        let result = lowerer.lower(&mark, &ctx);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    // --- tests: GeoLowerer ---

    /// A LowerCtx whose `data_sources` holds one named source (leaked for test
    /// convenience, like `make_ctx`).
    fn make_ctx_with_source(
        name: &str,
        ds: brightfield_spec::ast::DataSource,
    ) -> LowerCtx<'static> {
        let mut map: IndexMap<String, brightfield_spec::ast::DataSource> = IndexMap::new();
        map.insert(name.to_string(), ds);
        let data_sources = Box::leak(Box::new(map));
        let params = Box::leak(Box::new(IndexMap::new()));
        LowerCtx {
            data_sources,
            params,
            plot_px: None,
        }
    }

    fn geo_mark_from(source: &str, options: Vec<(&str, SpecValue)>) -> Mark {
        let mut opts: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
        for (k, v) in options {
            opts.insert(k.to_string(), ValueOrParamRef::Value(v));
        }
        Mark {
            kind: MarkKind::Geo,
            status: ImplStatus::Implemented,
            data: Some(MarkData::From {
                source: source.to_string(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options: opts,
        }
    }

    #[test]
    fn spatial_source_wraps_geometry_in_st_asgeojson() {
        use brightfield_spec::ast::{DataSource, DataSourceKind};
        // A `type: spatial` source (both kind + extras carry the type, as the
        // parser records it) → the geom column is wrapped in ST_AsGeoJSON.
        let mut extras = IndexMap::new();
        extras.insert("type".to_string(), SpecValue::String("spatial".to_string()));
        let ds = DataSource {
            kind: DataSourceKind::File("us-states.json".to_string()),
            extras,
        };
        let ctx = make_ctx_with_source("states", ds);
        let mark = geo_mark_from("states", vec![]);
        let plan = GeoLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::Projection { columns, .. } => {
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0], "* REPLACE (ST_AsGeoJSON(\"geom\") AS \"geom\")");
            }
            other => panic!("expected Projection wrapping ST_AsGeoJSON, got {other:?}"),
        }
    }

    #[test]
    fn spatial_source_honours_geometry_channel() {
        use brightfield_spec::ast::{DataSource, DataSourceKind};
        let ds = DataSource {
            kind: DataSourceKind::Typed("spatial".to_string()),
            extras: {
                let mut e = IndexMap::new();
                e.insert("type".to_string(), SpecValue::String("spatial".to_string()));
                e
            },
        };
        let ctx = make_ctx_with_source("world", ds);
        // A non-default `geometry:` channel reads the custom SOURCE column but
        // canonicalises the OUTPUT to `geom`, so the renderer's fixed column name
        // always matches (EXCLUDE the source, re-add it as geom).
        let mark = geo_mark_from(
            "world",
            vec![("geometry", SpecValue::String("shape".into()))],
        );
        let plan = GeoLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::Projection { columns, .. } => {
                assert_eq!(
                    columns[0],
                    "* EXCLUDE (\"shape\"), ST_AsGeoJSON(\"shape\") AS \"geom\""
                );
            }
            other => panic!("expected Projection, got {other:?}"),
        }
    }

    #[test]
    fn inline_custom_geometry_column_renamed_to_canonical() {
        use brightfield_spec::ast::{DataSource, DataSourceKind};
        // An inline source under a custom `geometry: shape` column — no
        // ST_AsGeoJSON (already text), but renamed to canonical `geom` so the
        // renderer finds it.
        let ds = DataSource {
            kind: DataSourceKind::InlineRows(vec![]),
            extras: IndexMap::new(),
        };
        let ctx = make_ctx_with_source("areas", ds);
        let mark = geo_mark_from(
            "areas",
            vec![("geometry", SpecValue::String("shape".into()))],
        );
        let plan = GeoLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::Projection { columns, .. } => {
                assert_eq!(columns[0], "* EXCLUDE (\"shape\"), \"shape\" AS \"geom\"");
            }
            other => panic!("expected Projection (custom inline rename), got {other:?}"),
        }
    }

    #[test]
    fn inline_varchar_source_passes_through_unwrapped() {
        use brightfield_spec::ast::{DataSource, DataSourceKind};
        // An inline source (no `type: spatial`) — the geom column is already
        // VARCHAR GeoJSON text, so it must PASS THROUGH (no ST_AsGeoJSON, which
        // would error on a VARCHAR and need the spatial extension). This is what
        // makes the hermetic inline baseline work with no spatial extension.
        let ds = DataSource {
            kind: DataSourceKind::InlineRows(vec![]),
            extras: IndexMap::new(),
        };
        let ctx = make_ctx_with_source("areas", ds);
        let mark = geo_mark_from("areas", vec![]);
        let plan = GeoLowerer.lower(&mark, &ctx).expect("lowers");
        // Pass-through with no filter / param channels is a bare Source — no
        // ST_AsGeoJSON anywhere.
        assert_eq!(
            plan,
            QueryPlan::Source {
                table: "areas".to_string()
            }
        );
    }

    #[test]
    fn rejects_inline_mark_data() {
        let ctx = make_ctx();
        let mark = Mark {
            kind: MarkKind::Geo,
            status: ImplStatus::Implemented,
            data: Some(MarkData::Inline(vec![])),
            options: IndexMap::new(),
        };
        assert!(matches!(
            GeoLowerer.lower(&mark, &ctx),
            Err(EmitError::UnsupportedMark { .. })
        ));
    }

    // --- SimpleLowerer tests ---

    #[test]
    fn simple_lowerer_produces_source_for_from() {
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
    fn simple_lowerer_rejects_no_data() {
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
    fn simple_lowerer_rejects_inline_data() {
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
    fn default_lowerers_contains_all_registered_kinds() {
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
        assert!(kinds.contains(&MarkKind::Contour));
        assert!(kinds.contains(&MarkKind::Hexbin));
        assert!(kinds.contains(&MarkKind::Hexgrid));
        assert!(kinds.contains(&MarkKind::Geo));
        assert_eq!(kinds.len(), 24);
    }

    #[test]
    fn find_lowerer_returns_simple_for_registered() {
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
    fn crossfilter_excludes_self() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            ("view_a".to_string(), Predicate::Expr("x > 1".to_string())),
            ("view_b".to_string(), Predicate::Expr("y < 10".to_string())),
        ];
        let result = compile_selection(&selection, "view_a", &predicates);
        // view_a's predicate should be excluded
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("y < 10".to_string())])
        );
    }

    #[test]
    fn crossfilter_includes_in_other_view() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            ("view_a".to_string(), Predicate::Expr("x > 1".to_string())),
            ("view_b".to_string(), Predicate::Expr("y < 10".to_string())),
        ];
        let result = compile_selection(&selection, "view_b", &predicates);
        // view_b's predicate excluded, view_a's included
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("x > 1".to_string())])
        );
    }

    #[test]
    fn empty_selection_is_true() {
        let selection = SelectionNode {
            select: AstRes::Intersect,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates: Vec<(String, Predicate)> = vec![];
        let result = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(result, Predicate::True);
    }

    /// A structured clause passes through selection compilation INTACT — the
    /// column, typed bounds, and scale/pixel metadata all survive resolution
    /// (crossfilter exclusion included), so a downstream consumer can read the
    /// structure back out of the compiled predicate. This is the no-erasure
    /// guarantee at the query layer.
    #[test]
    fn compile_selection_preserves_structured_clause() {
        use crate::ir::{ClauseMeta, ScalarValue, ScaleDescriptor};

        let structured = Predicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(3.0),
            hi: ScalarValue::Float(7.0),
            meta: Some(ClauseMeta {
                scale: Some(ScaleDescriptor {
                    kind: "linear".to_string(),
                    domain: Some((0.0, 10.0)),
                    range: Some((40.0, 340.0)),
                }),
                pixel_size: Some(1.0),
            }),
        };
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            ("view_a".to_string(), structured.clone()),
            ("view_b".to_string(), Predicate::Expr("y < 10".to_string())),
        ];
        // Resolved for view_b: view_a's structured clause is included, intact.
        let result = compile_selection(&selection, "view_b", &predicates);
        assert_eq!(result, Predicate::And(vec![structured.clone()]));
        // Resolved for view_a (self-exclusion): the structured clause drops.
        let excluded = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(
            excluded,
            Predicate::And(vec![Predicate::Expr("y < 10".to_string())])
        );
    }

    // -----------------------------------------------------------------------
    // Statistical-mark lowerers
    // -----------------------------------------------------------------------

    fn make_mark_with_options(kind: MarkKind, opts: Vec<(&str, SpecValue)>) -> Mark {
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
    fn regression_lowerer_emits_aggregate_scalar() {
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
                // Every aggregate is a TYPED call — function identity and
                // argument expressions as data (nothing left as a raw string).
                let calls: Vec<&AggregateCall> = aggregates
                    .iter()
                    .map(|a| match a {
                        AggregateExpr::Call(c) => c,
                        AggregateExpr::Raw(s) => {
                            panic!("regression aggregate left raw: {s}")
                        }
                    })
                    .collect();
                let has = |func: AggregateFunction, alias: &str| {
                    calls
                        .iter()
                        .any(|c| c.func == func && c.alias.as_deref() == Some(alias))
                };
                assert!(has(AggregateFunction::RegrSlope, "slope"));
                assert!(has(AggregateFunction::RegrIntercept, "intercept"));
                assert!(has(AggregateFunction::RegrCount, "n"));
                assert!(has(AggregateFunction::RegrAvgx, "x_bar"));
                assert!(has(AggregateFunction::RegrSxx, "sxx"));
                assert!(has(AggregateFunction::RegrSxy, "sxy"));
                assert!(has(AggregateFunction::RegrSyy, "syy"));
                // regr_* argument order is (y, x).
                let slope = calls
                    .iter()
                    .find(|c| c.func == AggregateFunction::RegrSlope)
                    .unwrap();
                assert_eq!(
                    slope.args,
                    vec!["\"height\"".to_string(), "\"weight\"".to_string()]
                );
                // Data extents the renderer builds its x/y scales from —
                // typed min/max calls with the DOUBLE cast carried as data.
                assert!(has(AggregateFunction::Min, "x_min"));
                assert!(has(AggregateFunction::Max, "x_max"));
                assert!(has(AggregateFunction::Min, "y_min"));
                assert!(has(AggregateFunction::Max, "y_max"));
                let x_min = calls
                    .iter()
                    .find(|c| c.alias.as_deref() == Some("x_min"))
                    .unwrap();
                assert_eq!(x_min.cast.as_deref(), Some("DOUBLE"));
                assert_eq!(x_min.args, vec!["\"weight\"".to_string()]);
            }
            other => panic!("expected AggregateScalar, got {other:?}"),
        }
    }

    /// The regression lowerer's typed aggregates render byte-identically to
    /// the strings the lowerer used to format — the SQL is pinned verbatim.
    #[test]
    fn regression_lowerer_sql_is_byte_stable() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
            ],
        );
        let ctx = make_ctx();
        let plan = RegressionLowerer.lower(&mark, &ctx).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert_eq!(
            sql,
            "SELECT regr_slope(\"height\", \"weight\") AS slope, \
             regr_intercept(\"height\", \"weight\") AS intercept, \
             regr_count(\"height\", \"weight\") AS n, \
             regr_avgx(\"height\", \"weight\") AS x_bar, \
             regr_sxx(\"height\", \"weight\") AS sxx, \
             regr_sxy(\"height\", \"weight\") AS sxy, \
             regr_syy(\"height\", \"weight\") AS syy, \
             CAST(min(\"weight\") AS DOUBLE) AS x_min, \
             CAST(max(\"weight\") AS DOUBLE) AS x_max, \
             CAST(min(\"height\") AS DOUBLE) AS y_min, \
             CAST(max(\"height\") AS DOUBLE) AS y_max \
             FROM (SELECT * FROM (SELECT * FROM \"athletes\") AS _f \
             WHERE \"weight\" IS NOT NULL AND \"height\" IS NOT NULL) AS _as"
        );
    }

    #[test]
    fn regression_lowerer_with_stroke_groups_by() {
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
    fn regression_lowerer_rejects_polynomial() {
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
    fn density_lowerer_1d_x_uses_equiwidth_bucket() {
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
                // The occupancy count is a TYPED call (count-star, DOUBLE
                // cast, reserved alias — all as data).
                match &aggregates[0] {
                    AggregateExpr::Call(c) => {
                        assert_eq!(c.func, AggregateFunction::Count);
                        assert_eq!(c.args, vec!["*".to_string()]);
                        assert_eq!(c.cast.as_deref(), Some("DOUBLE"));
                        assert_eq!(c.alias.as_deref(), Some("__bf_count"));
                    }
                    other => panic!("expected typed count call, got {other:?}"),
                }
            }
            other => panic!("expected Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn density_lowerer_2d_uses_two_buckets() {
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

    // Density marks: the contour attr-shield keeps
    // `thresholds` (ISO-LEVEL count on a contour mark) out of the density
    // lowerer's BIN-count read — the emitted bucket count stays at the default
    // 100 — while `bins` still sizes the grid, and a plain density mark keeps
    // reading `thresholds` as bins.
    #[test]
    fn contour_attr_shield_keeps_thresholds_out_of_sql() {
        let ctx = make_ctx();
        let registry = default_lowerers();
        let group_by_of = |plan: QueryPlan| -> Vec<String> {
            let QueryPlan::Order { input, .. } = plan else {
                panic!("expected Order-wrapped Aggregation");
            };
            match *input {
                QueryPlan::Aggregation { group_by, .. } => group_by,
                other => panic!("expected Aggregation, got {other:?}"),
            }
        };

        // Contour with thresholds: 12 → bucket count stays the default 100.
        let contour = make_mark_with_options(
            MarkKind::Contour,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("thresholds", SpecValue::Integer(12)),
            ],
        );
        let shielded = group_by_of(
            find_lowerer(MarkKind::Contour, &registry)
                .lower(&contour, &ctx)
                .expect("lowers"),
        );
        assert!(
            shielded[0].contains("* 100") && shielded[1].contains("* 100"),
            "shielded contour bins at the default 100, got {shielded:?}"
        );
        assert!(
            !shielded[0].contains("12") && !shielded[1].contains("12"),
            "thresholds must not reach the contour SQL, got {shielded:?}"
        );

        // The shield leaves the original mark untouched (it clones).
        assert!(contour.options.contains_key("thresholds"));

        // `bins` still sizes the contour grid.
        let contour_bins = make_mark_with_options(
            MarkKind::Contour,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("bins", SpecValue::Integer(20)),
                ("thresholds", SpecValue::Integer(12)),
            ],
        );
        let with_bins = group_by_of(
            find_lowerer(MarkKind::Contour, &registry)
                .lower(&contour_bins, &ctx)
                .expect("lowers"),
        );
        assert!(
            with_bins[0].contains("* 20"),
            "bins still sizes the contour grid, got {with_bins:?}"
        );

        // A density mark's thresholds still reads as its bin count.
        let density = make_mark_with_options(
            MarkKind::Density,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("thresholds", SpecValue::Integer(12)),
            ],
        );
        let unshielded = group_by_of(
            find_lowerer(MarkKind::Density, &registry)
                .lower(&density, &ctx)
                .expect("lowers"),
        );
        assert!(
            unshielded[0].contains("* 12"),
            "density keeps thresholds-as-bins, got {unshielded:?}"
        );
    }

    #[test]
    fn default_lowerers_includes_statistical_kinds() {
        let registry = default_lowerers();
        let kinds: Vec<MarkKind> = registry.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&MarkKind::RegressionY));
        assert!(kinds.contains(&MarkKind::RegressionX));
        assert!(kinds.contains(&MarkKind::Density));
        assert!(kinds.contains(&MarkKind::DensityX));
        assert!(kinds.contains(&MarkKind::DensityY));
    }

    // -----------------------------------------------------------------------
    // HexbinLowerer SQL shape
    // -----------------------------------------------------------------------

    fn hexbin_mark() -> Mark {
        let mut options: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
        options.insert(
            "x".into(),
            ValueOrParamRef::Value(SpecValue::String("time".into())),
        );
        options.insert(
            "y".into(),
            ValueOrParamRef::Value(SpecValue::String("delay".into())),
        );
        options.insert(
            "fill".into(),
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            }),
        );
        options.insert(
            "binWidth".into(),
            ValueOrParamRef::Value(SpecValue::Integer(20)),
        );
        Mark {
            kind: MarkKind::Hexbin,
            status: brightfield_spec::vocab::ImplStatus::Implemented,
            data: Some(MarkData::From {
                source: "flights".into(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options,
        }
    }

    #[test]
    fn hexbin_lowers_to_ordered_aggregation() {
        let plan = HexbinLowerer
            .lower(&hexbin_mark(), &make_ctx())
            .expect("lowers");
        // Outermost is Order on the emitted centres (x then y) — determinism.
        let QueryPlan::Order { input, keys } = plan else {
            panic!("expected Order-wrapped Aggregation");
        };
        assert_eq!(
            keys,
            vec![
                ("\"time\"".to_string(), SortDir::Asc),
                ("\"delay\"".to_string(), SortDir::Asc),
            ]
        );
        let QueryPlan::Aggregation {
            group_by,
            aggregates,
            ..
        } = *input
        else {
            panic!("expected Aggregation under Order");
        };
        // Centres are emitted in DATA units aliased to the x/y channel columns.
        assert!(group_by[0].contains("AS \"time\""), "{group_by:?}");
        assert!(group_by[1].contains("AS \"delay\""), "{group_by:?}");
        // Count → reserved column, as a TYPED call; the constant hex
        // half-extents travel in-band as RAW scalar expressions (they are not
        // aggregate calls — the designed analysis bail).
        assert!(aggregates.iter().any(|a| matches!(
            a,
            AggregateExpr::Call(c)
                if c.func == AggregateFunction::Count
                    && c.alias.as_deref() == Some("__bf_count")
        )));
        assert!(aggregates
            .iter()
            .any(|a| matches!(a, AggregateExpr::Raw(s) if s.contains("__bf_hex_dx"))));
        assert!(aggregates
            .iter()
            .any(|a| matches!(a, AggregateExpr::Raw(s) if s.contains("__bf_hex_dy"))));
    }

    #[test]
    fn hexbin_sql_has_axial_and_cube_round() {
        let plan = HexbinLowerer
            .lower(&hexbin_mark(), &make_ctx())
            .expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        // Axial transform (pixel → hex) and the three cube-round coordinates.
        assert!(sql.contains("__bf_qf") && sql.contains("__bf_rf"), "{sql}");
        assert!(sql.contains("round("), "{sql}");
        assert!(sql.contains("__bf_rx") && sql.contains("__bf_ry") && sql.contains("__bf_rz"));
        // Cube-round resolution CASE.
        assert!(sql.contains("CASE WHEN"), "{sql}");
        // Pixel-space binning reads the plot extent (fallback 580×350 area).
        assert!(
            sql.contains("580") || sql.contains("350"),
            "plot px extent absent: {sql}"
        );
        // Extents via correlated subqueries over the raw table.
        assert!(
            sql.contains("SELECT min(\"time\") FROM \"flights\""),
            "{sql}"
        );
    }

    #[test]
    fn hexbin_avg_aliases_to_source_column() {
        let mut mark = hexbin_mark();
        mark.options.insert(
            "fill".into(),
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Avg,
                column: Some("score".into()),
            }),
        );
        let plan = HexbinLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(
            sql.contains("avg(\"score\")") && sql.contains("AS \"score\""),
            "{sql}"
        );
        assert!(
            !sql.contains("__bf_count"),
            "avg fill must not emit count: {sql}"
        );
    }

    #[test]
    fn hexbin_requires_x_y_and_data() {
        let ctx = make_ctx();
        // Missing y.
        let mut m = hexbin_mark();
        m.options.shift_remove("y");
        assert!(HexbinLowerer.lower(&m, &ctx).is_err());
        // Missing data source.
        let mut m = hexbin_mark();
        m.data = None;
        assert!(HexbinLowerer.lower(&m, &ctx).is_err());
    }

    /// F3 (review): a degenerate axis maps to the plot MIDPOINT, not NULL —
    /// `nullif` is gone, replaced by a CASE that yields `plot/2` (fallback
    /// 580×350 → 290 / 175) when the span is 0, so binning still emits real
    /// centres instead of the whole mark vanishing on a NULL centre.
    #[test]
    fn f3_hexbin_degenerate_axis_maps_to_plot_midpoint() {
        let plan = HexbinLowerer
            .lower(&hexbin_mark(), &make_ctx())
            .expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(
            !sql.contains("nullif"),
            "degenerate axis must not NULL the pixel: {sql}"
        );
        assert!(
            sql.contains("= 0 THEN 290") && sql.contains("= 0 THEN 175"),
            "degenerate-axis midpoint mapping absent: {sql}"
        );
    }

    /// F4 (review): a `data: { filter }` scopes BOTH the binned rows AND the
    /// extent subqueries — otherwise binning over the unfiltered extent shifts
    /// every centre and filtered-out rows still widen the plot.
    #[test]
    fn f4_hexbin_data_filter_reaches_rows_and_extents() {
        let mut mark = hexbin_mark();
        if let Some(MarkData::From { extras, .. }) = &mut mark.data {
            extras.insert("filter".into(), SpecValue::String("delay > 10".into()));
        }
        let plan = HexbinLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        // Every extent subquery (min/max on each axis) is scoped by the filter —
        // no bare `... FROM "flights")` extent survives.
        for agg in [
            "min(\"time\")",
            "max(\"time\")",
            "min(\"delay\")",
            "max(\"delay\")",
        ] {
            assert!(
                sql.contains(&format!("{agg} FROM \"flights\" WHERE (delay > 10)")),
                "extent {agg} not filtered: {sql}"
            );
            assert!(
                !sql.contains(&format!("{agg} FROM \"flights\")")),
                "extent {agg} has an unfiltered form: {sql}"
            );
        }
        // The binned rows are filtered too (ANDed onto the NULL guard).
        assert!(sql.contains("AND (delay > 10)"), "row filter absent: {sql}");
    }

    fn cell_mark(fill: Option<ValueOrParamRef<SpecValue>>) -> Mark {
        let mut options: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
        options.insert(
            "x".into(),
            ValueOrParamRef::Value(SpecValue::String("day".into())),
        );
        options.insert(
            "y".into(),
            ValueOrParamRef::Value(SpecValue::String("hour".into())),
        );
        if let Some(f) = fill {
            options.insert("fill".into(), f);
        }
        Mark {
            kind: MarkKind::Cell,
            status: brightfield_spec::vocab::ImplStatus::Implemented,
            data: Some(MarkData::From {
                source: "events".into(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options,
        }
    }

    #[test]
    fn cell_self_aggregating_count_groups_by_categories() {
        let mark = cell_mark(Some(ValueOrParamRef::Value(SpecValue::Aggregate {
            func: AggregateFunc::Count,
            column: None,
        })));
        let plan = CellLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let QueryPlan::Order { input, keys } = plan else {
            panic!("expected Order-wrapped Aggregation");
        };
        assert_eq!(keys.len(), 2, "deterministic order over both categories");
        let QueryPlan::Aggregation {
            group_by,
            aggregates,
            ..
        } = *input
        else {
            panic!("expected Aggregation");
        };
        assert_eq!(
            group_by,
            vec!["\"day\"".to_string(), "\"hour\"".to_string()]
        );
        match &aggregates[0] {
            AggregateExpr::Call(c) => {
                assert_eq!(c.func, AggregateFunction::Count);
                assert_eq!(c.args, vec!["*".to_string()]);
                assert_eq!(c.alias.as_deref(), Some("__bf_count"));
            }
            other => panic!("expected typed count call, got {other:?}"),
        }
    }

    #[test]
    fn cell_self_aggregating_avg_aliases_column() {
        let mark = cell_mark(Some(ValueOrParamRef::Value(SpecValue::Aggregate {
            func: AggregateFunc::Avg,
            column: Some("value".into()),
        })));
        let plan = CellLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(
            sql.contains("avg(\"value\")") && sql.contains("AS \"value\""),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY 1, 2"), "{sql}");
    }

    /// F4 (review): a self-aggregating cell honours `data: { filter }` — the
    /// predicate is ANDed onto the row NULL-guard so aggregated counts exclude
    /// filtered rows. (Cell has no pixel extents, so the row WHERE is the only
    /// site.)
    #[test]
    fn f4_cell_self_aggregating_honours_data_filter() {
        let mut mark = cell_mark(Some(ValueOrParamRef::Value(SpecValue::Aggregate {
            func: AggregateFunc::Count,
            column: None,
        })));
        if let Some(MarkData::From { extras, .. }) = &mut mark.data {
            extras.insert("filter".into(), SpecValue::String("value > 5".into()));
        }
        let plan = CellLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(
            sql.contains("AND (value > 5)"),
            "cell row filter absent: {sql}"
        );
    }

    #[test]
    fn cell_pre_aggregated_path_unchanged() {
        // A cell with NO aggregate fill delegates to SimpleLowerer — the shipped
        // pre-aggregated path, byte-identical (cell.png gate).
        let mark = cell_mark(Some(ValueOrParamRef::Value(SpecValue::String("v".into()))));
        let plan = CellLowerer.lower(&mark, &make_ctx()).expect("lowers");
        // SimpleLowerer produces a bare Source (no GROUP BY).
        assert_eq!(
            plan,
            QueryPlan::Source {
                table: "events".to_string()
            }
        );
    }

    // --- RectLowerer: the positional bin + count histogram ---

    /// A `rectY` binning `delay` and counting, with an optional `steps` hint.
    fn binned_rect(kind: MarkKind, bin_channel: &str, steps: Option<i64>) -> Mark {
        let count_channel = if bin_channel == "x" { "y" } else { "x" };
        let mut options = IndexMap::new();
        options.insert(
            bin_channel.into(),
            ValueOrParamRef::Value(SpecValue::Bin {
                column: "delay".into(),
                steps,
            }),
        );
        options.insert(
            count_channel.into(),
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            }),
        );
        Mark {
            kind,
            status: brightfield_spec::vocab::ImplStatus::Implemented,
            data: Some(MarkData::From {
                source: "flights".into(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options,
        }
    }

    /// The emitted shape: two group keys — the low edge under the SOURCE
    /// column's own name, the high edge under the axis-keyed reserved name —
    /// and the shared `__bf_count` aggregate, all under a deterministic order.
    #[test]
    fn binned_rect_groups_by_both_edges_and_counts() {
        let plan = RectLowerer
            .lower(&binned_rect(MarkKind::RectY, "x", None), &make_ctx())
            .expect("lowers");
        let QueryPlan::Order { input, keys } = plan else {
            panic!("expected Order-wrapped Aggregation");
        };
        assert_eq!(keys, vec![("\"delay\"".to_string(), SortDir::Asc)]);
        let QueryPlan::Aggregation {
            group_by,
            aggregates,
            ..
        } = *input
        else {
            panic!("expected Aggregation under Order");
        };
        assert_eq!(group_by.len(), 2, "one key per bin edge: {group_by:?}");
        assert!(
            group_by[0].ends_with("AS \"delay\""),
            "the LOW edge takes the source column's own name — that alias is \
             what `group_key_provides` matches: {group_by:?}"
        );
        assert!(
            group_by[1].ends_with("AS __bf_bin_x2"),
            "the HIGH edge is keyed by AXIS, not by column: {group_by:?}"
        );
        assert!(matches!(
            aggregates.as_slice(),
            [AggregateExpr::Call(c)]
                if c.func == AggregateFunction::Count
                    && c.alias.as_deref() == Some("__bf_count")
        ));
    }

    /// The transpose keys its high edge to the y axis, so a doubly-binned rect
    /// would not collide with itself.
    #[test]
    fn a_binned_rect_x_keys_its_high_edge_to_the_y_axis() {
        let plan = RectLowerer
            .lower(&binned_rect(MarkKind::RectX, "y", None), &make_ctx())
            .expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(sql.contains("AS __bf_bin_y2"), "{sql}");
        assert!(!sql.contains("__bf_bin_x2"), "{sql}");
    }

    /// The rendered SQL: positional `GROUP BY`, the bin scheme resolved once
    /// beneath the aggregation, and the extent read from the RAW table so a
    /// brush cannot move the bin edges.
    #[test]
    fn binned_rect_sql_resolves_the_scheme_once_over_the_raw_table() {
        let plan = RectLowerer
            .lower(&binned_rect(MarkKind::RectY, "x", None), &make_ctx())
            .expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(sql.contains("GROUP BY 1, 2"), "{sql}");
        assert!(
            sql.contains("SELECT min(\"delay\") AS lo, max(\"delay\") AS hi FROM \"flights\""),
            "the extent is the raw table's, not the filtered rows': {sql}"
        );
        assert_eq!(
            sql.matches("__bf_bin_lo").count(),
            5,
            "the scheme is projected ONCE and referenced by name — one alias \
             plus two references per edge: {sql}"
        );
        // Mosaic's default hint, not the density lowerer's 100.
        assert!(sql.contains("<= 25"), "{sql}");
        assert!(!sql.contains("<= 100"), "{sql}");
    }

    /// A `steps:` hint reaches the emitted arithmetic. Honouring the `bin` and
    /// dropping the hint would draw a different chart from the one written.
    #[test]
    fn the_steps_hint_reaches_the_emitted_sql() {
        let plan = RectLowerer
            .lower(&binned_rect(MarkKind::RectY, "x", Some(60)), &make_ctx())
            .expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(sql.contains("<= 60"), "{sql}");
        assert!(!sql.contains("<= 25"), "{sql}");
    }

    /// The reason binning lives in the lowerer at all: a navigation extent on
    /// the binned axis resolves UNDER the `GROUP BY` rather than declining, so
    /// a brush filters the rows that get counted instead of the bins that came
    /// out. Asserted on `axis_pushdown` directly — the property, not a proxy.
    #[test]
    fn a_binned_axis_pushes_navigation_under_the_group_by() {
        let plan = RectLowerer
            .lower(&binned_rect(MarkKind::RectY, "x", None), &make_ctx())
            .expect("lowers");
        assert_eq!(
            crate::navigation_filter_pass::axis_pushdown(&plan, "delay"),
            crate::navigation_filter_pass::Pushdown::UnderGroupBy,
            "the low edge aliases back to `delay`, so the extent is pushable"
        );
    }

    /// A rect with no positional bin delegates to [`SimpleLowerer`] and the
    /// shipped explicit-interval path is untouched — `rect-histogram.yaml` and
    /// `rect-cells.yaml` emit exactly what they emitted before.
    #[test]
    fn an_unbinned_rect_still_lowers_to_a_bare_source() {
        let mut mark = binned_rect(MarkKind::RectY, "x", None);
        mark.options.insert(
            "x".into(),
            ValueOrParamRef::Value(SpecValue::String("lo".into())),
        );
        let plan = RectLowerer.lower(&mark, &make_ctx()).expect("lowers");
        assert_eq!(
            plan,
            QueryPlan::Source {
                table: "flights".to_string()
            }
        );
    }

    /// `data: { filter }` scopes the counted rows. The extent deliberately does
    /// NOT move with it — see the lowerer.
    #[test]
    fn a_binned_rect_honours_its_data_filter_on_the_rows() {
        let mut mark = binned_rect(MarkKind::RectY, "x", None);
        let mut extras = IndexMap::new();
        extras.insert("filter".to_string(), SpecValue::String("delay > 5".into()));
        mark.data = Some(MarkData::From {
            source: "flights".into(),
            filter_by: None,
            extras,
        });
        let plan = RectLowerer.lower(&mark, &make_ctx()).expect("lowers");
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        assert!(
            sql.contains("\"delay\" IS NOT NULL AND (delay > 5)"),
            "row filter absent: {sql}"
        );
    }

    #[test]
    fn hexgrid_lowers_to_singleton_row() {
        let mark = Mark {
            kind: MarkKind::Hexgrid,
            status: brightfield_spec::vocab::ImplStatus::Implemented,
            data: None, // DATALESS
            options: IndexMap::new(),
        };
        let plan = HexgridLowerer
            .lower(&mark, &make_ctx())
            .expect("dataless lowers");
        assert!(matches!(plan, QueryPlan::Singleton { .. }));
        let mut bindings = Vec::new();
        let sql = crate::render::render_query(&plan, &mut bindings);
        // One-row query, no FROM — the mark still produces a batch.
        assert_eq!(sql, "SELECT 1 AS __bf_hexgrid");
    }

    #[test]
    fn union_combines_with_or() {
        let selection = SelectionNode {
            select: AstRes::Union,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            ("view_a".to_string(), Predicate::Expr("x > 1".to_string())),
            ("view_b".to_string(), Predicate::Expr("y < 10".to_string())),
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
