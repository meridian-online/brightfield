//! Automatic pre-aggregation cube derivation — the pure (no-I/O) half of the
//! engine's pre-aggregation layer.
//!
//! Given a mark's lowered query plan and the *active* structured clause of an
//! interaction, this module decides whether the plan decomposes into a data
//! cube — a smaller GROUP BY materialisation over (group dimensions × active
//! clause columns) carrying **sufficient statistics** — and, when it does,
//! constructs two SQL strings:
//!
//! 1. the **build** SELECT (materialised once as a session-scoped TEMP table),
//! 2. the **re-query** SELECT that serves each interaction step from the cube
//!    by filtering on the active columns and re-aggregating the statistics,
//!    reproducing the direct query's schema (names, aliases, order) exactly.
//!
//! Anything that does not fit bails with `None` — the caller falls back to the
//! direct query. Bailing is the designed path, never an error.
//!
//! ## Semantics adopted from Mosaic
//!
//! The cube taxonomy and lifecycle implemented here adopt the semantics of the
//! pre-aggregation layer in [uwdata/mosaic](https://github.com/uwdata/mosaic)
//! (BSD-3-Clause, © UW Interactive Data Lab): derivation preconditions (a
//! single base table, group dimensions plus decomposable aggregates only),
//! the sufficient-statistics rewrite table (including mean-centering for the
//! regression family and FILTER-clause propagation onto every statistic),
//! active-column derivation from the interaction clause, and transparent
//! fallback to the direct query whenever derivation bails. The implementation
//! is native to this crate's IR; no Mosaic code is vendored.
//!
//! **One deliberate divergence:** active interval columns enter the cube at
//! their RAW data values, not binned to pixel resolution. Mosaic bins the
//! active column through the scale transform (bounding the cube at ~plot-width
//! cells) and accepts pixel-snapped answers; here the cube's answer must equal
//! the direct query's for *any* clause bounds, so exactness wins the first
//! cut. The cost is cube size proportional to the active column's distinct
//! count. The structured clause already carries the scale/pixel metadata
//! ([`crate::ir::ClauseMeta`]), so pixel-resolution binning is a later swap of
//! the active-dimension expression — made when measurement shows the size
//! matters and the brush's bounds are provably grid-aligned.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ir::{AggregateCall, AggregateExpr, AggregateFunction, Predicate, QueryPlan, SortDir};
use crate::render::render_query;

/// Column-name prefixes reserved for cube-internal columns.
const DIM_PREFIX: &str = "__bf_dim";
const ACTIVE_PREFIX: &str = "__bf_a";
const STAT_PREFIX: &str = "__bf_s";

/// Flatten a predicate into its structured active clauses, when the whole
/// predicate is structured: a bare [`Predicate::Interval`] / [`Predicate::Point`],
/// or an `And` of such clauses (the two-axis brush). Anything else — string
/// expressions, `Or`, params, nesting — returns `None` and the caller bails to
/// the direct query.
#[must_use]
pub fn active_clauses(predicate: &Predicate) -> Option<Vec<&Predicate>> {
    match predicate {
        Predicate::Interval { .. } | Predicate::Point { .. } => Some(vec![predicate]),
        Predicate::And(parts) if !parts.is_empty() => {
            let mut out = Vec::with_capacity(parts.len());
            for p in parts {
                match p {
                    Predicate::Interval { .. } | Predicate::Point { .. } => out.push(p),
                    _ => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// One group-by dimension of the cube: the original expression (materialised
/// at build time) and the alias text the re-query must reproduce.
struct Dim {
    /// Cube column name (`__bf_dimN`).
    cube_col: String,
    /// The original group-by expression, without its alias.
    expr: String,
    /// The output alias text, exactly as the direct query names the column
    /// (`"x"`, `x`, …).
    alias: String,
}

/// One active clause column: the source expression that becomes a cube
/// dimension the re-query filters on.
struct ActiveCol {
    /// Cube column name (`__bf_aN`).
    cube_col: String,
    /// The clause's column expression (trusted SQL, exactly as brushed).
    expr: String,
}

/// A deduplicating registry of per-cell statistic columns.
#[derive(Default)]
struct StatRegistry {
    /// `(build_expr, cube_col)` in insertion order.
    stats: Vec<(String, String)>,
}

impl StatRegistry {
    /// The cube column holding `build_expr`, interning it if new.
    fn intern(&mut self, build_expr: String) -> String {
        if let Some((_, col)) = self.stats.iter().find(|(e, _)| *e == build_expr) {
            return col.clone();
        }
        let col = format!("{STAT_PREFIX}{}", self.stats.len());
        self.stats.push((build_expr, col.clone()));
        col
    }
}

/// A regression statistics group: one (y, x, filter) triple whose six
/// mean-centered moment sums are shared by every `regr_*` call over it.
struct RegrGroup {
    y: String,
    x: String,
    /// The pair filter `(orig_filter AND) y IS NOT NULL AND x IS NOT NULL`.
    pair_filter: String,
    /// Index of this group's `(center_x, center_y)` pair in the probe row.
    center_slot: usize,
    /// Cube columns, assigned at derivation: n, sx, sy, sxx, sxy, syy.
    cols: Option<RegrCols>,
}

struct RegrCols {
    n: String,
    sx: String,
    sy: String,
    sxx: String,
    sxy: String,
    syy: String,
}

/// One original aggregate's reassembly plan.
enum AggPlan {
    /// Fully derived at analysis time (count/sum/avg/min/max).
    Simple {
        /// Reassembled select expression including ` AS <alias>`.
        reasm: String,
    },
    /// A regression-family call — reassembly needs the centering constants.
    Regr {
        func: AggregateFunction,
        group: usize,
        cast: Option<String>,
        alias: String,
    },
}

/// A derived cube, pending its centering constants (when any regression
/// aggregate participates). Obtain from [`derive_cube`]; run
/// [`CubeDerivation::probe_sql`] if present, then [`CubeDerivation::finalize`].
pub struct CubeDerivation {
    input_sql: String,
    dims: Vec<Dim>,
    active_cols: Vec<ActiveCol>,
    active_rewritten: Predicate,
    aggs: Vec<AggPlan>,
    regr_groups: Vec<RegrGroup>,
    stats: StatRegistry,
    order: Vec<(String, SortDir)>,
    scalar: bool,
    probe: Option<String>,
}

impl CubeDerivation {
    /// The scalar probe query for the centering constants, when a regression
    /// aggregate participates: one row of `2 × n_groups` DOUBLE columns,
    /// `(center_x, center_y)` per group in order. Execute it over the live
    /// connection and pass the values to [`Self::finalize`] (NULL → 0.0).
    /// `None` means no centering is needed — call `finalize(&[])`.
    #[must_use]
    pub fn probe_sql(&self) -> Option<&str> {
        self.probe.as_deref()
    }

    /// Produce the final SQL pair. `centers` holds the probe row's values in
    /// column order (empty when [`Self::probe_sql`] was `None`). Returns
    /// `None` on a center-count mismatch — treat as a bail.
    #[must_use]
    pub fn finalize(mut self, centers: &[f64]) -> Option<CubeSqls> {
        if centers.len() != self.regr_groups.len() * 2 {
            return None;
        }

        // Build each regression group's stat columns now that its centering
        // constants are known. Every statistic carries the pair filter — the
        // FILTER propagation the decomposition table requires.
        for (i, group) in self.regr_groups.iter_mut().enumerate() {
            let cx = float_literal(centers[i * 2]);
            let cy = float_literal(centers[i * 2 + 1]);
            let x = format!("(CAST({} AS DOUBLE) - {cx})", group.x);
            let y = format!("(CAST({} AS DOUBLE) - {cy})", group.y);
            let f = &group.pair_filter;
            group.cols = Some(RegrCols {
                n: self.stats.intern(format!("COUNT(*) FILTER (WHERE {f})")),
                sx: self.stats.intern(format!("sum({x}) FILTER (WHERE {f})")),
                sy: self.stats.intern(format!("sum({y}) FILTER (WHERE {f})")),
                sxx: self
                    .stats
                    .intern(format!("sum({x} * {x}) FILTER (WHERE {f})")),
                sxy: self
                    .stats
                    .intern(format!("sum({x} * {y}) FILTER (WHERE {f})")),
                syy: self
                    .stats
                    .intern(format!("sum({y} * {y}) FILTER (WHERE {f})")),
            });
        }

        // Resolve every aggregate to its reassembled select expression.
        let mut reassembled = Vec::with_capacity(self.aggs.len());
        for agg in &self.aggs {
            match agg {
                AggPlan::Simple { reasm } => reassembled.push(reasm.clone()),
                AggPlan::Regr {
                    func,
                    group,
                    cast,
                    alias,
                } => {
                    let g = &self.regr_groups[*group];
                    let cols = g.cols.as_ref().expect("built above");
                    let cx = float_literal(centers[*group * 2]);
                    let cy = float_literal(centers[*group * 2 + 1]);
                    let expr = regr_reassembly(*func, cols, &cx, &cy)?;
                    let expr = match cast {
                        Some(c) => format!("CAST({expr} AS {c})"),
                        None => expr,
                    };
                    reassembled.push(format!("{expr} AS {alias}"));
                }
            }
        }

        // The build plan: group by (dims × active cols), materialising every
        // interned statistic.
        let mut group_by = Vec::with_capacity(self.dims.len() + self.active_cols.len());
        for d in &self.dims {
            group_by.push(format!("{} AS \"{}\"", d.expr, d.cube_col));
        }
        for a in &self.active_cols {
            group_by.push(format!("{} AS \"{}\"", a.expr, a.cube_col));
        }
        let aggregates = self
            .stats
            .stats
            .iter()
            .map(|(expr, col)| AggregateExpr::Raw(format!("{expr} AS \"{col}\"")))
            .collect();
        let build_plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: BUILD_INPUT_MARKER.to_string(),
            }),
            group_by,
            aggregates,
        };
        let mut bindings = Vec::new();
        let rendered = render_query(&build_plan, &mut bindings);
        if !bindings.is_empty() {
            return None;
        }
        // Splice the real (already rendered) input in place of the marker
        // source — the input chain is a full sub-select, not a table name.
        let build_select = rendered.replace(
            &format!("SELECT * FROM \"{BUILD_INPUT_MARKER}\""),
            &self.input_sql,
        );

        Some(CubeSqls {
            build_select,
            dims: self
                .dims
                .iter()
                .map(|d| (d.cube_col.clone(), d.alias.clone()))
                .collect(),
            active_rewritten: self.active_rewritten,
            reassembled,
            order: self.order,
            scalar: self.scalar,
        })
    }
}

/// Marker table name replaced by the rendered input chain when the build
/// SELECT is assembled. NUL-prefixed so it can never collide with a real view.
const BUILD_INPUT_MARKER: &str = "\u{0}__bf_cube_input";

/// The finalised SQL pair for one cube shape.
pub struct CubeSqls {
    /// The SELECT to materialise (`CREATE TEMP TABLE <name> AS <this>`).
    pub build_select: String,
    dims: Vec<(String, String)>,
    active_rewritten: Predicate,
    reassembled: Vec<String>,
    order: Vec<(String, SortDir)>,
    scalar: bool,
}

impl CubeSqls {
    /// The re-query over the materialised cube `table`: filter by the
    /// rewritten active clause, re-aggregate the statistics, restore the
    /// original output aliases and row order. Its result set equals the
    /// direct query's, column names and types included.
    #[must_use]
    pub fn query_select(&self, table: &str) -> Option<String> {
        let inner = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: table.to_string(),
            }),
            predicate: self.active_rewritten.clone(),
        };
        let core = if self.scalar {
            QueryPlan::AggregateScalar {
                input: Box::new(inner),
                aggregates: self
                    .reassembled
                    .iter()
                    .map(|r| AggregateExpr::Raw(r.clone()))
                    .collect(),
            }
        } else {
            QueryPlan::Aggregation {
                input: Box::new(inner),
                group_by: self
                    .dims
                    .iter()
                    .map(|(cube_col, alias)| format!("\"{cube_col}\" AS {alias}"))
                    .collect(),
                aggregates: self
                    .reassembled
                    .iter()
                    .map(|r| AggregateExpr::Raw(r.clone()))
                    .collect(),
            }
        };
        let plan = if self.order.is_empty() {
            core
        } else {
            QueryPlan::Order {
                input: Box::new(core),
                keys: self.order.clone(),
            }
        };
        let mut bindings = Vec::new();
        let sql = render_query(&plan, &mut bindings);
        if bindings.is_empty() {
            Some(sql)
        } else {
            None
        }
    }
}

/// Derive a cube from a mark's lowered (pre-selection) plan and an
/// interaction's active clause.
///
/// * `plan` — the lowered plan, **without** any live selection filter.
/// * `active` — the active structured clause ([`active_clauses`] must accept it).
/// * `rest` — the rest of the mark's compiled selection (other contributors'
///   predicates), applied to the cube's base rows at build time; `None` when
///   the active clause is the only contribution.
///
/// Returns `None` — bail to the direct query — unless every precondition
/// holds: an `Aggregation`/`AggregateScalar` (optionally under `Order`) whose
/// input chain is Filter/Projection/Bin nodes over exactly one `Source`, with
/// no parameter placeholders, and whose aggregates are all typed, aliased
/// [`AggregateCall`]s of decomposable functions.
#[must_use]
pub fn derive_cube(
    plan: &QueryPlan,
    active: &Predicate,
    rest: Option<&Predicate>,
) -> Option<CubeDerivation> {
    let clauses = active_clauses(active)?;

    // --- shape analysis -----------------------------------------------------
    let (order, core) = match plan {
        QueryPlan::Order { input, keys } => (keys.clone(), &**input),
        p => (Vec::new(), p),
    };
    let (input, group_by, aggregates, scalar) = match core {
        QueryPlan::Aggregation {
            input,
            group_by,
            aggregates,
        } => (input, group_by.as_slice(), aggregates.as_slice(), false),
        QueryPlan::AggregateScalar { input, aggregates } => {
            (input, [].as_slice(), aggregates.as_slice(), true)
        }
        _ => return None,
    };
    if !chain_ok(input) {
        return None;
    }

    // --- the cube's base: the input chain plus the rest-of-selection filter --
    let base = match rest {
        Some(r) => QueryPlan::Filter {
            input: input.clone(),
            predicate: r.clone(),
        },
        None => (**input).clone(),
    };
    let mut bindings = Vec::new();
    let input_sql = render_query(&base, &mut bindings);
    if !bindings.is_empty() {
        return None;
    }

    // --- group dimensions ---------------------------------------------------
    let mut dims = Vec::with_capacity(group_by.len());
    for (i, entry) in group_by.iter().enumerate() {
        let (expr, alias) = split_alias(entry)?;
        dims.push(Dim {
            cube_col: format!("{DIM_PREFIX}{i}"),
            expr: expr.to_string(),
            alias: alias.to_string(),
        });
    }

    // --- active columns + clause rewrite ------------------------------------
    let mut active_cols: Vec<ActiveCol> = Vec::new();
    let mut col_map: HashMap<String, String> = HashMap::new();
    for clause in &clauses {
        let column = match clause {
            Predicate::Interval { column, .. } | Predicate::Point { column, .. } => column,
            _ => return None,
        };
        if !col_map.contains_key(column) {
            let cube_col = format!("{ACTIVE_PREFIX}{}", active_cols.len());
            col_map.insert(column.clone(), cube_col.clone());
            active_cols.push(ActiveCol {
                cube_col,
                expr: column.clone(),
            });
        }
    }
    let active_rewritten = rewrite_active(active, &col_map)?;

    // --- aggregate decomposition --------------------------------------------
    let mut stats = StatRegistry::default();
    let mut regr_groups: Vec<RegrGroup> = Vec::new();
    let mut aggs = Vec::with_capacity(aggregates.len());
    for agg in aggregates {
        let call = match agg {
            AggregateExpr::Call(call) => call,
            AggregateExpr::Raw(_) => return None,
        };
        aggs.push(decompose(call, &mut stats, &mut regr_groups)?);
    }

    // --- centering probe (regression groups only) ---------------------------
    let probe = if regr_groups.is_empty() {
        None
    } else {
        let mut sql = String::from("SELECT ");
        for (i, g) in regr_groups.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            let f = &g.pair_filter;
            let _ = write!(
                sql,
                "avg(CAST({x} AS DOUBLE)) FILTER (WHERE {f}) AS \"__bf_cx{i}\", \
                 avg(CAST({y} AS DOUBLE)) FILTER (WHERE {f}) AS \"__bf_cy{i}\"",
                x = g.x,
                y = g.y,
            );
        }
        let _ = write!(sql, " FROM ({input_sql}) AS __bf_probe");
        Some(sql)
    };

    Some(CubeDerivation {
        input_sql,
        dims,
        active_cols,
        active_rewritten,
        aggs,
        regr_groups,
        stats,
        order,
        scalar,
        probe,
    })
}

/// Whether the aggregation's input chain is cube-compatible: Filter /
/// Projection / Bin nodes over exactly one `Source`, no nested aggregation,
/// no parameter placeholders in filter predicates.
fn chain_ok(plan: &QueryPlan) -> bool {
    match plan {
        QueryPlan::Source { .. } => true,
        QueryPlan::Filter { input, predicate } => {
            !predicate_has_param(predicate) && chain_ok(input)
        }
        QueryPlan::Projection { input, .. } | QueryPlan::Bin { input, .. } => chain_ok(input),
        _ => false,
    }
}

fn predicate_has_param(p: &Predicate) -> bool {
    match p {
        Predicate::Param { .. } => true,
        Predicate::And(v) | Predicate::Or(v) => v.iter().any(predicate_has_param),
        _ => false,
    }
}

/// Rewrite the active clause's column expressions onto the cube's active
/// columns, preserving values and structure.
fn rewrite_active(p: &Predicate, col_map: &HashMap<String, String>) -> Option<Predicate> {
    match p {
        Predicate::Interval {
            column,
            lo,
            hi,
            meta,
        } => Some(Predicate::Interval {
            column: format!("\"{}\"", col_map.get(column)?),
            lo: lo.clone(),
            hi: hi.clone(),
            meta: meta.clone(),
        }),
        Predicate::Point {
            column,
            values,
            meta,
        } => Some(Predicate::Point {
            column: format!("\"{}\"", col_map.get(column)?),
            values: values.clone(),
            meta: meta.clone(),
        }),
        Predicate::And(parts) => {
            let mut out = Vec::with_capacity(parts.len());
            for part in parts {
                out.push(rewrite_active(part, col_map)?);
            }
            Some(Predicate::And(out))
        }
        _ => None,
    }
}

/// Split a group-by entry into `(expression, output alias text)`.
///
/// An entry with a top-level ` AS ` splits there (last occurrence, outside
/// quotes and parentheses — `CAST(x AS DOUBLE)` is depth-guarded). A bare
/// entry must be a simple identifier (`x` or `"x"`), which is its own alias.
fn split_alias(entry: &str) -> Option<(&str, &str)> {
    if let Some(idx) = find_top_level_as(entry) {
        let expr = entry[..idx].trim_end();
        let alias = entry[idx + 4..].trim_start();
        if expr.is_empty() || alias.is_empty() {
            return None;
        }
        return Some((expr, alias));
    }
    let trimmed = entry.trim();
    if is_simple_identifier(trimmed) {
        Some((trimmed, trimmed))
    } else {
        None
    }
}

/// Byte index of the LAST top-level ` AS ` (outside quotes/parens), if any.
fn find_top_level_as(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    let mut last = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' if !in_single && !in_double => depth += 1,
            b')' if !in_single && !in_double => depth = depth.saturating_sub(1),
            b' ' if depth == 0
                && !in_single
                && !in_double
                && s[i..].len() >= 4
                && s[i..i + 4].eq_ignore_ascii_case(" as ") =>
            {
                last = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    last
}

/// `x` / `x_2` / `"anything quoted"` — an entry that names a column directly.
fn is_simple_identifier(s: &str) -> bool {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return !s[1..s.len() - 1].contains('"');
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Decompose one typed aggregate call into cube statistics + reassembly.
fn decompose(
    call: &AggregateCall,
    stats: &mut StatRegistry,
    regr_groups: &mut Vec<RegrGroup>,
) -> Option<AggPlan> {
    // The reassembled column must keep its name; a call without an alias
    // would take an engine-derived name that the rewrite cannot reproduce.
    let alias = call.alias.as_ref()?;
    let filter_suffix = call
        .filter
        .as_ref()
        .map(|f| format!(" FILTER (WHERE {f})"))
        .unwrap_or_default();
    let one_arg = || -> Option<&String> {
        match call.args.as_slice() {
            [a] => Some(a),
            _ => None,
        }
    };
    let cast_wrap = |expr: String, default_cast: Option<&str>| -> String {
        match (&call.cast, default_cast) {
            (Some(c), _) => format!("CAST({expr} AS {c})"),
            (None, Some(c)) => format!("CAST({expr} AS {c})"),
            (None, None) => expr,
        }
    };

    match call.func {
        AggregateFunction::Count => {
            let arg = one_arg()?;
            let stat = stats.intern(format!("COUNT({arg}){filter_suffix}"));
            // COUNT is BIGINT; a sum of BIGINTs widens, so cast back unless
            // the original call already casts.
            let reasm = cast_wrap(format!("sum(\"{stat}\")"), Some("BIGINT"));
            Some(AggPlan::Simple {
                reasm: format!("{reasm} AS {alias}"),
            })
        }
        AggregateFunction::Sum => {
            let arg = one_arg()?;
            let stat = stats.intern(format!("sum({arg}){filter_suffix}"));
            let reasm = cast_wrap(format!("sum(\"{stat}\")"), None);
            Some(AggPlan::Simple {
                reasm: format!("{reasm} AS {alias}"),
            })
        }
        AggregateFunction::Avg => {
            let arg = one_arg()?;
            let s = stats.intern(format!("sum({arg}){filter_suffix}"));
            let c = stats.intern(format!("COUNT({arg}){filter_suffix}"));
            // sum-of-sums over count-of-non-nulls; a NULL numerator (no
            // non-null rows anywhere) yields NULL exactly like direct avg.
            let reasm = cast_wrap(format!("(sum(\"{s}\") / sum(\"{c}\"))"), Some("DOUBLE"));
            Some(AggPlan::Simple {
                reasm: format!("{reasm} AS {alias}"),
            })
        }
        AggregateFunction::Min => {
            let arg = one_arg()?;
            let stat = stats.intern(format!("min({arg}){filter_suffix}"));
            let reasm = cast_wrap(format!("min(\"{stat}\")"), None);
            Some(AggPlan::Simple {
                reasm: format!("{reasm} AS {alias}"),
            })
        }
        AggregateFunction::Max => {
            let arg = one_arg()?;
            let stat = stats.intern(format!("max({arg}){filter_suffix}"));
            let reasm = cast_wrap(format!("max(\"{stat}\")"), None);
            Some(AggPlan::Simple {
                reasm: format!("{reasm} AS {alias}"),
            })
        }
        AggregateFunction::RegrSlope
        | AggregateFunction::RegrIntercept
        | AggregateFunction::RegrCount
        | AggregateFunction::RegrAvgx
        | AggregateFunction::RegrSxx
        | AggregateFunction::RegrSxy
        | AggregateFunction::RegrSyy => {
            let (y, x) = match call.args.as_slice() {
                [y, x] => (y.clone(), x.clone()),
                _ => return None,
            };
            // The regr_* family ignores pairs with a NULL on either side; the
            // pair filter reproduces that, AND-composed with any original
            // FILTER clause (propagated onto every statistic).
            let pair = format!("{y} IS NOT NULL AND {x} IS NOT NULL");
            let pair_filter = match &call.filter {
                Some(f) => format!("({f}) AND {pair}"),
                None => pair,
            };
            let group = regr_groups
                .iter()
                .position(|g| g.y == y && g.x == x && g.pair_filter == pair_filter)
                .unwrap_or_else(|| {
                    let slot = regr_groups.len();
                    regr_groups.push(RegrGroup {
                        y,
                        x,
                        pair_filter,
                        center_slot: slot,
                        cols: None,
                    });
                    slot
                });
            debug_assert_eq!(regr_groups[group].center_slot, group);
            Some(AggPlan::Regr {
                func: call.func,
                group,
                cast: call.cast.clone(),
                alias: alias.clone(),
            })
        }
    }
}

/// The reassembly expression for one `regr_*` function over a group's
/// mean-centered moment sums. With `N = Σn`, `SX = Σsx`, … the centered
/// second moments reassemble exactly:
/// `Sxx = ΣSxx − SX²/N`, `Sxy = ΣSxy − SX·SY/N`, `Syy = ΣSyy − SY²/N`,
/// and the means shift back by the centering constants.
fn regr_reassembly(func: AggregateFunction, cols: &RegrCols, cx: &str, cy: &str) -> Option<String> {
    let n = format!("sum(\"{}\")", cols.n);
    let sx = format!("sum(\"{}\")", cols.sx);
    let sy = format!("sum(\"{}\")", cols.sy);
    let sxx = format!("sum(\"{}\")", cols.sxx);
    let sxy = format!("sum(\"{}\")", cols.sxy);
    let syy = format!("sum(\"{}\")", cols.syy);
    let cap_sxx = format!("({sxx} - {sx} * {sx} / {n})");
    let cap_sxy = format!("({sxy} - {sx} * {sy} / {n})");
    let cap_syy = format!("({syy} - {sy} * {sy} / {n})");
    let slope = format!("({cap_sxy} / nullif({cap_sxx}, 0))");
    let avgx = format!("({cx} + {sx} / {n})");
    let avgy = format!("({cy} + {sy} / {n})");
    Some(match func {
        AggregateFunction::RegrCount => format!("CAST(coalesce({n}, 0) AS BIGINT)"),
        AggregateFunction::RegrAvgx => avgx,
        AggregateFunction::RegrSxx => cap_sxx,
        AggregateFunction::RegrSxy => cap_sxy,
        AggregateFunction::RegrSyy => cap_syy,
        AggregateFunction::RegrSlope => slope,
        AggregateFunction::RegrIntercept => format!("({avgy} - {slope} * {avgx})"),
        _ => return None,
    })
}

/// A round-trip-exact SQL literal for an `f64` that always parses as DOUBLE
/// (`10` would parse as INTEGER; `10.0` cannot).
fn float_literal(v: f64) -> String {
    if v.is_finite() {
        let s = format!("{v:?}");
        if s.contains('.') || s.contains('e') || s.contains('E') {
            s
        } else {
            format!("{s}.0")
        }
    } else {
        // A non-finite center would poison every statistic — center at zero
        // instead (centering is a numerical aid; any constant is exact).
        "0.0".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conform::parse_and_normalise;
    use crate::ir::ScalarValue;

    fn assert_valid_sql(sql: &str) {
        parse_and_normalise(sql).unwrap_or_else(|e| {
            panic!("SQL failed to parse:\n  SQL: {sql}\n  Error: {e}");
        });
    }

    fn count_star(alias: &str) -> AggregateExpr {
        AggregateExpr::Call(AggregateCall {
            func: AggregateFunction::Count,
            args: vec!["*".to_string()],
            filter: None,
            cast: Some("DOUBLE".to_string()),
            alias: Some(alias.to_string()),
        })
    }

    fn interval(column: &str, lo: f64, hi: f64) -> Predicate {
        Predicate::Interval {
            column: column.to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        }
    }

    fn density_like_plan() -> QueryPlan {
        QueryPlan::Order {
            input: Box::new(QueryPlan::Aggregation {
                input: Box::new(QueryPlan::Filter {
                    input: Box::new(QueryPlan::Source {
                        table: "t".to_string(),
                    }),
                    predicate: Predicate::Expr("\"x\" IS NOT NULL".to_string()),
                }),
                group_by: vec!["CAST(\"x\" AS DOUBLE) AS \"x\"".to_string()],
                aggregates: vec![count_star("__bf_count")],
            }),
            keys: vec![("\"x\"".to_string(), SortDir::Asc)],
        }
    }

    #[test]
    fn active_clauses_accepts_structured_shapes() {
        let i = interval("x", 1.0, 2.0);
        assert_eq!(active_clauses(&i).unwrap().len(), 1);
        let p = Predicate::Point {
            column: "s".to_string(),
            values: vec![ScalarValue::Text("a".to_string())],
            meta: None,
        };
        assert_eq!(active_clauses(&p).unwrap().len(), 1);
        let and = Predicate::And(vec![interval("x", 1.0, 2.0), interval("y", 3.0, 4.0)]);
        assert_eq!(active_clauses(&and).unwrap().len(), 2);
    }

    #[test]
    fn active_clauses_bails_on_unstructured() {
        assert!(active_clauses(&Predicate::Expr("x > 1".to_string())).is_none());
        assert!(active_clauses(&Predicate::True).is_none());
        assert!(active_clauses(&Predicate::And(vec![
            interval("x", 1.0, 2.0),
            Predicate::Expr("y > 3".to_string()),
        ]))
        .is_none());
        assert!(active_clauses(&Predicate::And(vec![])).is_none());
        assert!(active_clauses(&Predicate::Or(vec![interval("x", 1.0, 2.0)])).is_none());
    }

    #[test]
    fn derive_builds_and_requeries_a_density_shape() {
        let plan = density_like_plan();
        let active = interval("\"y\"", 10.0, 50.0);
        let derivation = derive_cube(&plan, &active, None).expect("derives");
        assert!(derivation.probe_sql().is_none(), "no regr → no probe");
        let sqls = derivation.finalize(&[]).expect("finalizes");

        // The build groups by the dim and the active column and materialises
        // the count statistic.
        assert_valid_sql(&sqls.build_select);
        assert!(sqls.build_select.contains("AS \"__bf_dim0\""));
        assert!(sqls.build_select.contains("AS \"__bf_a0\""));
        assert!(sqls.build_select.contains("COUNT(*) AS \"__bf_s0\""));
        assert!(sqls.build_select.contains("\"x\" IS NOT NULL"));

        // The re-query filters the cube on the active column, restores the
        // original alias, and re-aggregates with the original cast + alias.
        let q = sqls.query_select("cube_tbl").expect("query renders");
        assert_valid_sql(&q);
        assert!(q.contains("FROM \"cube_tbl\""));
        assert!(q.contains("\"__bf_a0\" >= 10 AND \"__bf_a0\" <= 50"));
        assert!(q.contains("\"__bf_dim0\" AS \"x\""));
        assert!(q.contains("CAST(sum(\"__bf_s0\") AS DOUBLE) AS __bf_count"));
        assert!(q.contains("ORDER BY \"x\" ASC"));
        assert!(
            !q.contains("FROM \"t\""),
            "the re-query never touches the base table"
        );
    }

    #[test]
    fn rest_predicate_lands_in_the_build_where() {
        let plan = density_like_plan();
        let active = interval("\"y\"", 10.0, 50.0);
        let rest = Predicate::Expr("\"z\" = 4".to_string());
        let sqls = derive_cube(&plan, &active, Some(&rest))
            .expect("derives")
            .finalize(&[])
            .expect("finalizes");
        assert!(sqls.build_select.contains("\"z\" = 4"));
        let q = sqls.query_select("c").expect("query");
        assert!(
            !q.contains("\"z\" = 4"),
            "rest is baked into the cube, not re-applied"
        );
    }

    #[test]
    fn bails_on_raw_aggregates_and_non_aggregate_plans() {
        // Raw aggregate → opaque → bail.
        let raw_plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["\"x\"".to_string()],
            aggregates: vec![AggregateExpr::Raw("SUM(y) AS total".to_string())],
        };
        assert!(derive_cube(&raw_plan, &interval("\"y\"", 0.0, 1.0), None).is_none());

        // Row-level plan (no aggregation) → nothing to pre-aggregate.
        let row_plan = QueryPlan::Source {
            table: "t".to_string(),
        };
        assert!(derive_cube(&row_plan, &interval("\"y\"", 0.0, 1.0), None).is_none());
    }

    #[test]
    fn bails_on_unaliased_calls_params_and_bad_dims() {
        let unaliased = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["\"x\"".to_string()],
            aggregates: vec![AggregateExpr::Call(AggregateCall {
                func: AggregateFunction::Sum,
                args: vec!["\"y\"".to_string()],
                filter: None,
                cast: None,
                alias: None,
            })],
        };
        assert!(derive_cube(&unaliased, &interval("\"y\"", 0.0, 1.0), None).is_none());

        let with_param = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Filter {
                input: Box::new(QueryPlan::Source {
                    table: "t".to_string(),
                }),
                predicate: Predicate::Param {
                    name: "p".to_string(),
                    placeholder_index: 0,
                },
            }),
            group_by: vec!["\"x\"".to_string()],
            aggregates: vec![count_star("n")],
        };
        assert!(derive_cube(&with_param, &interval("\"y\"", 0.0, 1.0), None).is_none());

        let weird_dim = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["x + 1".to_string()], // expression without alias
            aggregates: vec![count_star("n")],
        };
        assert!(derive_cube(&weird_dim, &interval("\"y\"", 0.0, 1.0), None).is_none());
    }

    #[test]
    fn regr_family_probes_then_reassembles() {
        let regr = |func: AggregateFunction, alias: &str| {
            AggregateExpr::Call(AggregateCall {
                func,
                args: vec!["\"h\"".to_string(), "\"w\"".to_string()],
                filter: None,
                cast: None,
                alias: Some(alias.to_string()),
            })
        };
        let plan = QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Filter {
                input: Box::new(QueryPlan::Source {
                    table: "t".to_string(),
                }),
                predicate: Predicate::Expr("\"w\" IS NOT NULL AND \"h\" IS NOT NULL".to_string()),
            }),
            aggregates: vec![
                regr(AggregateFunction::RegrSlope, "slope"),
                regr(AggregateFunction::RegrCount, "n"),
            ],
        };
        let derivation = derive_cube(&plan, &interval("\"g\"", 0.0, 9.0), None).expect("derives");
        let probe = derivation
            .probe_sql()
            .expect("regr needs centers")
            .to_string();
        assert_valid_sql(&probe);
        assert!(probe.contains("avg(CAST(\"w\" AS DOUBLE))"));
        assert!(probe.contains("avg(CAST(\"h\" AS DOUBLE))"));

        let sqls = derivation.finalize(&[100.0, 5.0]).expect("finalizes");
        assert_valid_sql(&sqls.build_select);
        // Six moment statistics share one group; centering constants baked in.
        assert!(sqls.build_select.contains("- 100.0"));
        assert!(sqls.build_select.contains("- 5.0"));
        assert!(sqls
            .build_select
            .contains("FILTER (WHERE \"h\" IS NOT NULL AND \"w\" IS NOT NULL)"));

        let q = sqls.query_select("c").expect("query");
        assert_valid_sql(&q);
        assert!(q.contains("nullif"), "slope guards zero variance");
        assert!(q.contains("CAST(coalesce(sum(\"__bf_s0\"), 0) AS BIGINT) AS n"));
        assert!(
            !q.contains("GROUP BY"),
            "scalar shape re-queries without GROUP BY"
        );
    }

    #[test]
    fn center_count_mismatch_bails() {
        let plan = density_like_plan();
        let derivation = derive_cube(&plan, &interval("\"y\"", 0.0, 1.0), None).unwrap();
        assert!(
            derivation.finalize(&[1.0]).is_none(),
            "unexpected centers → bail"
        );
    }

    #[test]
    fn split_alias_handles_cast_and_bare_identifiers() {
        assert_eq!(
            split_alias("CAST(\"x\" AS DOUBLE) AS \"x\""),
            Some(("CAST(\"x\" AS DOUBLE)", "\"x\""))
        );
        assert_eq!(split_alias("\"x\""), Some(("\"x\"", "\"x\"")));
        assert_eq!(split_alias("x"), Some(("x", "x")));
        assert_eq!(split_alias("x + 1"), None);
        assert_eq!(
            split_alias("'lit AS not' AS y"),
            Some(("'lit AS not'", "y")),
            "a quoted literal containing ' AS ' does not split there"
        );
    }

    #[test]
    fn shared_statistics_are_deduplicated() {
        // avg + count over the same argument share the COUNT statistic.
        let plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["\"g\"".to_string()],
            aggregates: vec![
                AggregateExpr::Call(AggregateCall {
                    func: AggregateFunction::Avg,
                    args: vec!["\"v\"".to_string()],
                    filter: None,
                    cast: Some("DOUBLE".to_string()),
                    alias: Some("mean".to_string()),
                }),
                AggregateExpr::Call(AggregateCall {
                    func: AggregateFunction::Count,
                    args: vec!["\"v\"".to_string()],
                    filter: None,
                    cast: None,
                    alias: Some("n".to_string()),
                }),
            ],
        };
        let sqls = derive_cube(&plan, &interval("\"y\"", 0.0, 1.0), None)
            .unwrap()
            .finalize(&[])
            .unwrap();
        // Exactly two statistics: sum(v) and COUNT(v) — not three.
        assert!(sqls.build_select.contains("\"__bf_s0\""));
        assert!(sqls.build_select.contains("\"__bf_s1\""));
        assert!(!sqls.build_select.contains("\"__bf_s2\""));
    }

    #[test]
    fn point_clause_rewrites_onto_the_active_column() {
        let plan = QueryPlan::Order {
            input: Box::new(QueryPlan::Aggregation {
                input: Box::new(QueryPlan::Source {
                    table: "t".to_string(),
                }),
                group_by: vec!["\"cat\"".to_string()],
                aggregates: vec![count_star("__bf_count")],
            }),
            keys: vec![("\"cat\"".to_string(), SortDir::Asc)],
        };
        let active = Predicate::Point {
            column: "\"sport\"".to_string(),
            values: vec![
                ScalarValue::Text("Rowing".to_string()),
                ScalarValue::Text("Judo".to_string()),
            ],
            meta: None,
        };
        let sqls = derive_cube(&plan, &active, None)
            .unwrap()
            .finalize(&[])
            .unwrap();
        let q = sqls.query_select("c").unwrap();
        assert!(q.contains("\"__bf_a0\" = 'Rowing' OR \"__bf_a0\" = 'Judo'"));
        assert!(
            !q.contains("\"sport\" ="),
            "original column never re-filtered"
        );
    }

    #[test]
    fn float_literals_always_parse_as_double() {
        assert_eq!(float_literal(10.0), "10.0");
        assert_eq!(float_literal(0.5), "0.5");
        assert!(float_literal(f64::NAN) == "0.0");
        assert!(float_literal(1e300).contains('e') || float_literal(1e300).contains('.'));
    }
}
