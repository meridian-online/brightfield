//! Public entry points: data-source DDL emission (card 0004) and per-mark
//! query emission (card 0003).

use std::path::Path;

use brightfield_spec::ast::{DataSourceKind, ParamNode, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use indexmap::IndexMap;

use brightfield_spec::ast::{Component, Mark};

use crate::binding::{Binding, EmittedQuery, ParamValues};
use crate::error::EmitError;
use crate::ir::{Predicate, QueryPlan};
use crate::lower::{compile_selection, default_lowerers, find_lowerer, LowerCtx};
use crate::passes::apply_passes;
use crate::render::render_query;
use crate::source;

/// Tag identifying which dispatch arm produced a [`SourceDdl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKindTag {
    Parquet,
    Csv,
    Json,
    Spatial,
    DuckDb,
    InlineRows,
    Query,
}

/// One emitted DDL statement per data source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDdl {
    /// The view name (or attach alias for DuckDB).
    pub view_name: String,
    /// The full SQL statement (CREATE OR REPLACE VIEW … or ATTACH …).
    pub sql: String,
    /// Which dispatch arm produced this.
    pub source_kind: SourceKindTag,
}

/// Result of a successful emission — DDL statements plus non-fatal warnings.
#[derive(Debug, Clone, Default)]
pub struct EmitOutput {
    /// The emitted DDL statements.
    pub statements: Vec<SourceDdl>,
    /// Non-fatal observations (e.g. unknown CSV extras).
    pub warnings: Vec<ParseWarning>,
}

/// Emit data-source DDL for every entry in `spec.data`.
///
/// This is a pure function — no I/O, no DuckDB connection. File paths are
/// resolved against `base_dir` (typically the spec file's parent directory).
/// If `base_dir` is `None`, relative paths are left as-is.
///
/// # Errors
///
/// Returns [`EmitError`] if any data source cannot be emitted (unknown format,
/// inline row limit exceeded, invariant violation).
pub fn emit_sources(
    spec: &Spec,
    base_dir: Option<&Path>,
) -> Result<EmitOutput, EmitError> {
    let mut results = Vec::with_capacity(spec.data.len());
    let mut warnings = Vec::new();

    for (name, data_source) in &spec.data {
        // Collect warnings for CSV unknown extras
        if matches!(&data_source.kind, DataSourceKind::File(f) if {
            let ext = file_extension(f).unwrap_or_default();
            ext == "csv" || ext == "tsv"
        }) {
            for key in data_source.extras.keys() {
                if !CSV_ALLOW_LIST.contains(&key.as_str())
                    && key != "type"
                    && key != "where"
                    && key != "select"
                {
                    warnings.push(ParseWarning::UnknownOption {
                        path: format!("data.{name}"),
                        key: key.clone(),
                    });
                }
            }
        }

        let ddl = match &data_source.kind {
            DataSourceKind::File(file_value) => {
                source::emit_file(name, file_value, &data_source.extras, base_dir)?
            }
            DataSourceKind::Query(sql) => SourceDdl {
                view_name: name.clone(),
                sql: format!("CREATE OR REPLACE VIEW \"{}\" AS {}", name, sql),
                source_kind: SourceKindTag::Query,
            },
            DataSourceKind::Shorthand(s) => SourceDdl {
                view_name: name.clone(),
                sql: format!("CREATE OR REPLACE VIEW \"{}\" AS {}", name, s),
                source_kind: SourceKindTag::Query,
            },
            DataSourceKind::InlineRows(rows) => {
                source::emit_inline_rows(name, rows, &data_source.extras)?
            }
            DataSourceKind::Typed(type_name) => {
                // Typed without a file: key — check if there's a file in extras
                if let Some(SpecValue::String(file_value)) = data_source.extras.get("file") {
                    source::emit_file_typed(name, file_value, type_name, &data_source.extras, base_dir)?
                } else {
                    return Err(EmitError::InvariantViolation {
                        detail: format!(
                            "data source '{}' has type '{}' but no file: key — cannot emit SQL",
                            name, type_name
                        ),
                    });
                }
            }
            DataSourceKind::Opaque => {
                return Err(EmitError::InvariantViolation {
                    detail: format!(
                        "data source '{}' is opaque (no recognised keys) — cannot emit SQL",
                        name
                    ),
                });
            }
        };
        results.push(ddl);
    }

    Ok(EmitOutput {
        statements: results,
        warnings,
    })
}

/// CSV option allow-list. Extras outside this list produce a warning, not an error.
pub(crate) const CSV_ALLOW_LIST: &[&str] = &[
    "columns", "delim", "header", "nullstr", "skip", "types",
];

/// Resolve a file path against a base directory.
///
/// HTTP(S) URLs pass through verbatim. Absolute paths pass through.
/// Relative paths are joined against `base_dir` if provided.
pub(crate) fn resolve_path(file_value: &str, base_dir: Option<&Path>) -> String {
    // HTTP(S) URLs pass through verbatim
    if file_value.starts_with("http://") || file_value.starts_with("https://") {
        return file_value.to_string();
    }

    let path = Path::new(file_value);

    // Absolute paths pass through
    if path.is_absolute() {
        return file_value.to_string();
    }

    // Relative paths: join against base_dir if provided
    match base_dir {
        Some(base) => base.join(path).to_string_lossy().into_owned(),
        None => file_value.to_string(),
    }
}

/// Extract the file extension from a path, lowercased.
pub(crate) fn file_extension(file_value: &str) -> Option<String> {
    Path::new(file_value)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Format kwargs from an extras map, filtering to an allow-list and sorting
/// alphabetically. Returns pairs suitable for appending to a function call.
pub(crate) fn format_kwargs(
    extras: &IndexMap<String, SpecValue>,
    allow_list: &[&str],
) -> Vec<String> {
    let mut kwargs: Vec<(String, String)> = Vec::new();

    for key in allow_list {
        if let Some(val) = extras.get(*key) {
            kwargs.push(((*key).to_string(), spec_value_to_sql_literal(val)));
        }
    }

    kwargs.sort_by(|a, b| a.0.cmp(&b.0));
    kwargs.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
}

/// Convert a `SpecValue` to a SQL literal for use in kwargs.
pub(crate) fn spec_value_to_sql_literal(val: &SpecValue) -> String {
    match val {
        // Escape embedded single quotes (SQL-standard doubling) so string
        // literals — including interpolated param values (card 0014, ac-02) and
        // inline data — are injection-safe.
        SpecValue::String(s) => format!("'{}'", s.replace('\'', "''")),
        SpecValue::Integer(n) => format!("{n}"),
        // A finite float is a bare numeric literal; non-finite values are
        // emitted as DuckDB's quoted specials (castable to DOUBLE) so this never
        // yields invalid SQL like a bare `NaN`/`inf` (which DuckDB reads as an
        // identifier). Positional param channels wrap the literal in CAST(... AS
        // DOUBLE), and comparisons cast the quoted form, so both stay valid.
        SpecValue::Float(f) => {
            if f.is_finite() {
                format!("{f}")
            } else if f.is_nan() {
                "'NaN'".to_string()
            } else if *f > 0.0 {
                "'Infinity'".to_string()
            } else {
                "'-Infinity'".to_string()
            }
        }
        SpecValue::Bool(b) => format!("{b}"),
        SpecValue::Null => "NULL".to_string(),
        SpecValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(spec_value_to_sql_literal).collect();
            format!("[{}]", items.join(", "))
        }
        SpecValue::Object(map) => {
            // Objects as kwargs are unusual — render as a struct literal
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", spec_value_to_sql_literal(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        // Param refs and expressions in kwargs are unusual — render as placeholders
        SpecValue::Param(p) => format!("${}", p.0),
        SpecValue::Expression(e) => {
            // Render expression as its raw text — interleave spans and params
            let mut out = String::new();
            for (i, span) in e.spans.iter().enumerate() {
                out.push_str(span);
                if let Some(p) = e.params.get(i) {
                    out.push('$');
                    out.push_str(&p.0);
                }
            }
            out
        }
    }
}

/// Collect all `Mark` nodes from the spec's component tree (depth-first).
pub fn collect_marks(spec: &Spec) -> Vec<&Mark> {
    let mut marks = Vec::new();
    if let Some(root) = &spec.root {
        collect_marks_from_component(root, &mut marks);
    }
    marks
}

fn collect_marks_from_component<'a>(component: &'a Component, marks: &mut Vec<&'a Mark>) {
    match component {
        Component::Plot(plot) => {
            for item in &plot.items {
                collect_marks_from_component(item, marks);
            }
        }
        Component::HConcat(concat) | Component::VConcat(concat) => {
            for child in &concat.items {
                collect_marks_from_component(child, marks);
            }
        }
        Component::Mark(mark) => marks.push(mark),
        // Legends, interactors, inputs, spacers don't contain marks
        _ => {}
    }
}

/// Collect marks paired with their depth-first component paths.
///
/// Path format mirrors `brightfield-engine`'s `build_mark_index_map`:
/// `root/<container>[<idx>]/.../mark[<wirename>]`. Used by `emit_query` to
/// compute a mark's parent plot path for selection self-exclusion.
pub fn collect_marks_with_paths(spec: &Spec) -> Vec<(&Mark, String)> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_marks_with_paths_in(root, "root", &mut out);
    }
    out
}

fn collect_marks_with_paths_in<'a>(
    component: &'a Component,
    prefix: &str,
    out: &mut Vec<(&'a Mark, String)>,
) {
    match component {
        Component::Plot(plot) => {
            for (i, item) in plot.items.iter().enumerate() {
                let child = format!("{prefix}/plot[{i}]");
                collect_marks_with_paths_in(item, &child, out);
            }
        }
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                let child = format!("{prefix}/hconcat[{i}]");
                collect_marks_with_paths_in(item, &child, out);
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                let child = format!("{prefix}/vconcat[{i}]");
                collect_marks_with_paths_in(item, &child, out);
            }
        }
        Component::Mark(mark) => {
            let path = format!("{prefix}/mark[{}]", mark.kind.wire_name());
            out.push((mark, path));
        }
        _ => {}
    }
}

/// A plot and the flat mark indices it owns.
///
/// `mark_indices` are indices into the depth-first mark order produced by
/// [`collect_marks`] — i.e. the same order the engine executes and returns
/// results in — so a consumer can pull each plot's results out of a flat
/// `execute_all` vector. `plot_path` is the plot node's component path (e.g.
/// `root`, `root/hconcat[0]`), matching the identity attached to the layout
/// tree, so a positioned plot rect joins back to its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlotGroup {
    /// Component path of the owning plot node.
    pub plot_path: String,
    /// Indices (into [`collect_marks`] order) of the marks in this plot.
    pub mark_indices: Vec<usize>,
}

/// Group the spec's marks by their owning plot, in first-appearance order.
///
/// One [`PlotGroup`] per `Plot` node, carrying the flat mark indices of every
/// mark inside it. Note the path scheme labels each *item* of a plot as
/// `plot[i]`, so the per-mark path's `plot[i]` segment is an item index, not the
/// plot's identity — grouping therefore keys on the plot node's own path
/// (computed by this walk) rather than `analysis::parent_plot`. A mark that is
/// not inside any plot (degenerate; Mosaic marks normally live in a plot) gets
/// its own single-mark group so it still renders.
pub fn collect_plot_groups(spec: &Spec) -> Vec<PlotGroup> {
    let mut groups: Vec<PlotGroup> = Vec::new();
    let mut next_mark: usize = 0;
    if let Some(root) = &spec.root {
        collect_plot_groups_in(root, "root", None, &mut next_mark, &mut groups);
    }
    groups
}

fn collect_plot_groups_in(
    component: &Component,
    path: &str,
    current_group: Option<usize>,
    next_mark: &mut usize,
    groups: &mut Vec<PlotGroup>,
) {
    match component {
        Component::Plot(plot) => {
            // A plot node is a render unit: open a group keyed on its own path,
            // then route its marks into it.
            let group_idx = groups.len();
            groups.push(PlotGroup {
                plot_path: path.to_string(),
                mark_indices: Vec::new(),
            });
            for (i, item) in plot.items.iter().enumerate() {
                collect_plot_groups_in(
                    item,
                    &format!("{path}/plot[{i}]"),
                    Some(group_idx),
                    next_mark,
                    groups,
                );
            }
        }
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_plot_groups_in(
                    item,
                    &format!("{path}/hconcat[{i}]"),
                    current_group,
                    next_mark,
                    groups,
                );
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_plot_groups_in(
                    item,
                    &format!("{path}/vconcat[{i}]"),
                    current_group,
                    next_mark,
                    groups,
                );
            }
        }
        Component::Mark(_) => {
            let idx = *next_mark;
            *next_mark += 1;
            match current_group {
                Some(g) => groups[g].mark_indices.push(idx),
                None => groups.push(PlotGroup {
                    plot_path: path.to_string(),
                    mark_indices: vec![idx],
                }),
            }
        }
        _ => {}
    }
}

/// Emit a query for a single mark in the spec.
///
/// `param_values` carries current runtime values for param substitution
/// (closes the `_param_values` LOW from card 0005 v2 review). `None` falls
/// back to `Prepared` mode with `?` placeholders.
///
/// `selection_predicates` carries per-contributor predicates for any selection
/// the mark's `filter_by` references (card 0006 v2 runtime coordinator). The
/// outer slice is `(selection_name, contributors_for_that_selection)` —
/// `compile_selection` is invoked when the mark's filter_by selection name
/// matches an entry, and the mark's stable plot-node path is used as
/// `self_source` for crossfilter exclusion. `None` (or an empty slice) means "no live
/// selections" — the predicate falls back to `Predicate::True` and the
/// emitted query is unfiltered, the same behaviour as before this card.
///
/// # Errors
///
/// Returns `EmitError::UnsupportedMark` for marks whose `MarkKind` has no
/// registered lowering (defence-in-depth; preflight should reject first).
pub fn emit_query(
    spec: &Spec,
    mark_index: usize,
    param_values: Option<&ParamValues>,
    selection_predicates: Option<&[(String, Vec<(String, Predicate)>)]>,
) -> Result<EmittedQuery, EmitError> {
    let passes: Vec<Box<dyn crate::passes::Pass>> = vec![];
    emit_query_with_passes(spec, mark_index, param_values, selection_predicates, &passes)
}

/// Emit a query for a single mark, applying the given passes to the plan.
///
/// Pass-aware variant of [`emit_query`]. Same `param_values` /
/// `selection_predicates` contract.
pub fn emit_query_with_passes(
    spec: &Spec,
    mark_index: usize,
    param_values: Option<&ParamValues>,
    selection_predicates: Option<&[(String, Vec<(String, Predicate)>)]>,
    extra_passes: &[Box<dyn crate::passes::Pass>],
) -> Result<EmittedQuery, EmitError> {
    // Use the path-aware mark walker so we can compute the parent plot
    // identity for selection self-exclusion (card 0006 v2 decision 4).
    let marks_with_paths = collect_marks_with_paths(spec);
    let (mark, mark_path) = marks_with_paths
        .get(mark_index)
        .ok_or_else(|| EmitError::InvariantViolation {
            detail: format!(
                "mark_index {} out of bounds (spec has {} marks)",
                mark_index,
                marks_with_paths.len()
            ),
        })?;

    let lowerers = default_lowerers();
    let ctx = LowerCtx {
        data_sources: &spec.data,
        params: &spec.params,
    };

    let lowerer = find_lowerer(mark.kind, &lowerers);
    let mut plan = lowerer.lower(mark, &ctx)?;

    // Selection threading: if the mark has a filter_by reference and the
    // referenced name is a SelectionNode in spec.params, compile the live
    // contributors into a predicate and wrap the lowered plan in
    // `QueryPlan::Filter`. Self-exclusion identity is the stable plot-node path
    // (decision 4) — `plot_node_path`, NOT `parent_plot`, so a mark and the
    // brushing interactor in the same plot resolve to the same identity (else a
    // plot filters itself; card 0006).
    if let Some(selection_name) = mark_filter_by_name(mark) {
        if let Some(ParamNode::Selection(sel_node)) = spec.params.get(selection_name) {
            let self_source = brightfield_spec::analysis::plot_node_path(mark_path);
            let contributors: &[(String, Predicate)] = selection_predicates
                .and_then(|all| all.iter().find(|(n, _)| n == selection_name).map(|(_, c)| c.as_slice()))
                .unwrap_or(&[]);
            let predicate = compile_selection(sel_node, self_source, contributors);
            // Skip wrapping in Predicate::True case so unfiltered queries
            // stay structurally identical to pre-card behaviour (relied on
            // by existing render-shape tests).
            if predicate != Predicate::True {
                plan = QueryPlan::Filter {
                    input: Box::new(plan),
                    predicate,
                };
            }
        }
    }

    // Apply optimisation passes (built-in + caller-provided).
    let plan = apply_passes(plan, extra_passes);

    let plan_hash = plan.hash_structural();
    let mut bindings: Vec<Binding> = Vec::new();
    let rendered = render_query(&plan, &mut bindings);

    // Interpolate scalar param values into the emitted SQL (card 0014,
    // Decision 1). `$name` placeholders in lowerer-emitted projections / filter
    // expressions are substituted with escaped literals via
    // `spec_value_to_sql_literal`; names absent from `param_values` are left
    // intact. `plan_hash` stays STRUCTURAL — `execute_emitted` runs this concrete
    // `sql` directly and dedups on the literal SQL string via the LRU sql_cache,
    // so the value need not enter the hash. (An earlier value-fold into plan_hash
    // was removed: it was redundant with the direct execution and made the
    // plan-stability cache grow unbounded under a swept param — see
    // engine::execute_emitted.)
    let sql = match param_values {
        Some(params) => interpolate_params(&rendered, params),
        None => rendered,
    };

    Ok(EmittedQuery {
        sql,
        bindings,
        plan_hash,
    })
}

/// Substitute `$name` param placeholders in `sql` with escaped literals drawn
/// from `params` (card 0014, Decision 1). Matching is identifier-boundary aware
/// (`$name` consumes a maximal `[A-Za-z0-9_]` run); a `$name` whose identifier
/// is absent from `params` is emitted verbatim, mirroring binding.rs's
/// Interpolated fallthrough.
///
/// Substitution is **string-literal aware**: a `$name` inside a single-quoted
/// SQL string literal (e.g. within a `data.filter` expression like
/// `label = 'cost is $k'`) is left untouched — only placeholders in SQL code are
/// interpolated. Doubled quotes (`''`, the SQL escape for an embedded quote) are
/// handled so an escaped quote does not prematurely close the literal.
///
/// Values route through [`spec_value_to_sql_literal`], which quotes/escapes
/// strings, so interpolation is injection-safe for typed `SpecValue`s.
fn interpolate_params(sql: &str, params: &ParamValues) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.char_indices().peekable();
    let mut in_string = false;
    while let Some((idx, c)) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\'' {
                // A doubled '' is an escaped quote — stay inside the literal;
                // a lone ' closes it.
                if let Some(&(_, '\'')) = chars.peek() {
                    out.push('\'');
                    chars.next();
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        if c == '\'' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c != '$' {
            out.push(c);
            continue;
        }
        // `$` in SQL code — collect the following identifier run.
        let name_start = idx + 1;
        let mut name_end = name_start;
        while let Some(&(k, nc)) = chars.peek() {
            if nc.is_ascii_alphanumeric() || nc == '_' {
                name_end = k + nc.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        if name_end > name_start {
            let name = &sql[name_start..name_end];
            if let Some(val) = params.get(name) {
                out.push_str(&spec_value_to_sql_literal(val));
                continue;
            }
            // Unknown param — leave `$name` intact.
            out.push('$');
            out.push_str(name);
        } else {
            // Lone `$`.
            out.push('$');
        }
    }
    out
}

/// Extract the selection name that this mark's `data.filter_by` references,
/// if any. Returns `None` for marks without `filter_by` or with inline data.
fn mark_filter_by_name(mark: &Mark) -> Option<&str> {
    match &mark.data {
        Some(brightfield_spec::ast::MarkData::From {
            filter_by: Some(pr),
            ..
        }) => Some(pr.0.as_str()),
        _ => None,
    }
}

/// Emit queries for all marks in the spec.
///
/// Returns one `Result` per mark. Unimplemented marks return
/// `Err(EmitError::UnsupportedMark)`.
pub fn emit_all_queries(
    spec: &Spec,
    param_values: Option<&ParamValues>,
) -> Vec<Result<EmittedQuery, EmitError>> {
    let marks = collect_marks(spec);
    (0..marks.len())
        .map(|i| emit_query(spec, i, param_values, None))
        .collect()
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    // -----------------------------------------------------------------------
    // Param-effect routing (card 0014) — interpolation + plan_hash fold
    // -----------------------------------------------------------------------

    /// pefr ac-01: a `$param` placeholder is substituted with its concrete value.
    #[test]
    fn pefr_ac01_interpolate_scalar_param() {
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(20));
        let sql = interpolate_params("SELECT * FROM t WHERE x > $k", &params);
        assert_eq!(sql, "SELECT * FROM t WHERE x > 20");
    }

    /// pefr ac-01: a `$name` not in param_values is left intact (no partial
    /// interpolation), and identifier boundaries are respected — `$k` must not
    /// consume `$k2`.
    #[test]
    fn pefr_ac01_interpolate_unknown_and_boundaries() {
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(1));
        let sql = interpolate_params("$k2 + $k + $unknown", &params);
        assert_eq!(sql, "$k2 + 1 + $unknown", "only the exact known $k is inlined");
    }

    /// pefr ac-02: a string-valued param is emitted quoted and escaped
    /// (single-quote doubling) — never raw — so interpolation is injection-safe.
    #[test]
    fn pefr_ac02_interpolation_escapes_string_param() {
        let mut params = ParamValues::new();
        params.insert("s".to_string(), SpecValue::String("O'Brien".to_string()));
        let sql = interpolate_params("WHERE name = $s", &params);
        assert_eq!(sql, "WHERE name = 'O''Brien'");
        assert!(!sql.contains("$s"), "raw placeholder must be gone");
    }

    /// pefr review regression (card 0014): interpolation is string-literal aware —
    /// a `$name`-looking substring inside a single-quoted SQL string literal (e.g.
    /// a `data.filter` value) is left untouched, while a real placeholder in SQL
    /// code is still substituted. Doubled quotes (`''`) inside the literal do not
    /// prematurely close it.
    #[test]
    fn pefr_interpolation_skips_string_literals() {
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(99));
        let sql = interpolate_params("WHERE label = 'cost is $k' AND v > $k", &params);
        assert_eq!(
            sql, "WHERE label = 'cost is $k' AND v > 99",
            "the quoted $k stays literal; only the code-position $k is inlined"
        );
        // An escaped quote inside the literal must not end the string early.
        let sql2 = interpolate_params("WHERE a = 'x''$k y' AND b > $k", &params);
        assert_eq!(sql2, "WHERE a = 'x''$k y' AND b > 99");
    }

    /// pefr ac-03: a bare `$param` positional channel is projected into the
    /// SELECT as `$param AS "<param>"`, and interpolation yields `<value> AS
    /// "<param>"` — the channel reaches the query rather than being dropped.
    #[test]
    fn pefr_ac03_param_channel_projected() {
        let src = "params:\n  k: 3\ndata:\n  t: [{ x: 1 }, { x: 2 }]\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: $k\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(20));
        let emitted = emit_query(&spec, 0, Some(&params), None).unwrap();
        assert!(
            emitted.sql.contains("AS \"k\""),
            "param channel must be projected with the param-named alias: {}",
            emitted.sql
        );
        assert!(
            emitted.sql.contains("20"),
            "param value must be interpolated into the projection: {}",
            emitted.sql
        );
        assert!(
            !emitted.sql.contains("$k"),
            "no raw placeholder should survive: {}",
            emitted.sql
        );
    }

    /// pefr ac-07: a `data.filter` expression lowers into a WHERE clause and
    /// its `$param` is interpolated (card 0014, Decision 2).
    #[test]
    fn pefr_ac07_data_filter_lowers_to_where() {
        let src = "params:\n  k: 0\ndata:\n  t: [{ x: 1 }, { x: 5 }]\nplot:\n  - mark: dot\n    data: { from: t, filter: \"x > $k\" }\n    x: x\n    y: x\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(2));
        let emitted = emit_query(&spec, 0, Some(&params), None).unwrap();
        assert!(
            emitted.sql.to_uppercase().contains("WHERE"),
            "data.filter must lower to a WHERE: {}",
            emitted.sql
        );
        assert!(
            emitted.sql.contains("x > 2"),
            "the filter param must be interpolated: {}",
            emitted.sql
        );
    }

    /// pefr review regression (card 0014 #4): a filter with NO `$param` parses to
    /// a plain String and must still lower to a WHERE with the raw predicate —
    /// never quoted into a constant string (which would silently pass every row).
    #[test]
    fn pefr_param_free_filter_lowers_to_where() {
        let src = "data:\n  t: [{ x: 1 }, { x: 5 }]\nplot:\n  - mark: dot\n    data: { from: t, filter: \"x > 2\" }\n    x: x\n    y: x\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let emitted = emit_query(&spec, 0, None, None).unwrap();
        assert!(
            emitted.sql.to_uppercase().contains("WHERE"),
            "a param-free filter must lower to a WHERE: {}",
            emitted.sql
        );
        assert!(
            emitted.sql.contains("x > 2") && !emitted.sql.contains("'x > 2'"),
            "the filter must appear as raw predicate text, not a quoted constant: {}",
            emitted.sql
        );
    }

    /// pefr ac-10: plan_hash is STRUCTURAL — a param that does NOT appear in a
    /// mark's SQL (the SQL-invariant / pure tier) yields an identical plan_hash
    /// across values, so nothing perturbs the plan record. This is the property
    /// gomb_ac12 relies on for selection params (which route through concrete
    /// predicates carrying no `$name`).
    #[test]
    fn pefr_ac10_sql_invariant_param_keeps_plan_hash() {
        let src = "params:\n  k: 1\ndata:\n  t: [{ x: 1 }]\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: x\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let mut p1 = ParamValues::new();
        p1.insert("k".to_string(), SpecValue::Integer(1));
        let mut p2 = ParamValues::new();
        p2.insert("k".to_string(), SpecValue::Integer(999));
        let h1 = emit_query(&spec, 0, Some(&p1), None).unwrap().plan_hash;
        let h2 = emit_query(&spec, 0, Some(&p2), None).unwrap().plan_hash;
        assert_eq!(
            h1, h2,
            "a param absent from the mark's SQL must not perturb plan_hash"
        );
    }

    /// pefr ac-11 (refined): a channel-param value change is reflected in the
    /// concrete emitted SQL (the data-shape effect), while plan_hash stays
    /// STRUCTURAL. execute_emitted runs this concrete SQL directly and dedups on
    /// the literal string via the LRU sql_cache, so the value need not enter the
    /// hash — the earlier value-fold was removed as redundant + unbounded.
    #[test]
    fn pefr_ac11_emit_channel_param_changes_sql_not_structural_hash() {
        let src = "params:\n  k: 3\ndata:\n  t: [{ x: 1 }]\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: $k\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let mut p3 = ParamValues::new();
        p3.insert("k".to_string(), SpecValue::Integer(3));
        let mut p20 = ParamValues::new();
        p20.insert("k".to_string(), SpecValue::Integer(20));
        let e3 = emit_query(&spec, 0, Some(&p3), None).unwrap();
        let e20 = emit_query(&spec, 0, Some(&p20), None).unwrap();
        assert_ne!(
            e3.sql, e20.sql,
            "the concrete emitted SQL must reflect the channel-param value"
        );
        assert_eq!(
            e3.plan_hash, e20.plan_hash,
            "plan_hash is structural — unchanged by a param value"
        );
    }

    #[test]
    fn dfir_ac08_emit_query_unsupported_mark() {
        // Use a mark kind that SimpleLowerer is NOT registered for
        let src = "plot:\n  - mark: hexbin\n    data: { from: flights }\ndata:\n  flights: { file: flights.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "hexbin"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn msv_ac01_emit_query_succeeds_for_from_data() {
        // SimpleLowerer handles line marks with data.from
        let src = "plot:\n  - mark: line\n    data: { from: flights }\ndata:\n  flights: { file: flights.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None, None);
        assert!(result.is_ok(), "SimpleLowerer should handle line+data.from");
        let emitted = result.unwrap();
        assert!(
            emitted.sql.contains("flights"),
            "emitted SQL should reference the source table"
        );
    }

    #[test]
    fn dfir_ac08_emit_query_out_of_bounds() {
        let src = "plot:\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 99, None, None);
        assert!(matches!(
            result,
            Err(EmitError::InvariantViolation { .. })
        ));
    }

    #[test]
    fn dfir_ac08_emit_all_queries_returns_per_mark_results() {
        let src = "plot:\n  - mark: line\n    data: { from: t }\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let results = emit_all_queries(&spec, None);
        assert_eq!(results.len(), 2, "should have one result per mark");
        // SimpleLowerer handles both line and dot with data.from
        for result in &results {
            assert!(result.is_ok(), "line and dot with data.from should succeed via SimpleLowerer");
        }
    }

    #[test]
    fn dfir_ac08_emit_query_no_marks_spec() {
        // A spec with no marks — just data
        let src = "data:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None, None);
        assert!(matches!(
            result,
            Err(EmitError::InvariantViolation { .. })
        ));
    }

    // ----- gomb ac-13 — conformance snapshots for statistical marks -----
    //
    // Snapshots capture the emitted SQL string only. They do NOT capture
    // the rendered Vello scene — that's a future-card concern (cross-platform
    // float reproducibility around the Gaussian kernel will need tolerance
    // comparison if we ever snapshot rendered pixels).

    #[test]
    fn gomb_ac13_density1d_x_snapshot() {
        let src = r#"
data:
  athletes: { file: athletes.parquet }
plot:
  - mark: densityX
    data: { from: athletes }
    x: weight
"#;
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let emitted = emit_query(&spec, 0, None, None).expect("emit");
        // Stable shape: filter NULLs, group by an equiwidth bucket centre
        // aliased to the channel column, count. (Portable `floor(...)` binning,
        // not width_bucket — follow-up #4.)
        assert!(emitted.sql.contains("floor"));
        assert!(emitted.sql.contains("\"weight\""));
        assert!(emitted.sql.contains("AS \"weight\""));
        // Bucket CENTRE, not index: `0.5` offset + `least` top-bucket clamp.
        assert!(emitted.sql.contains("0.5"));
        assert!(emitted.sql.contains("least"));
        assert!(emitted.sql.contains("COUNT(*)"));
        assert!(emitted.sql.contains("IS NOT NULL"));
        assert!(emitted.sql.contains("GROUP BY 1"));
    }

    #[test]
    fn gomb_ac13_density2d_snapshot() {
        let src = r#"
data:
  athletes: { file: athletes.parquet }
plot:
  - mark: density
    data: { from: athletes }
    x: weight
    y: height
    thresholds: 16
"#;
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let emitted = emit_query(&spec, 0, None, None).expect("emit");
        assert!(emitted.sql.contains("AS \"weight\""));
        assert!(emitted.sql.contains("AS \"height\""));
        assert!(emitted.sql.contains("\"weight\""));
        assert!(emitted.sql.contains("\"height\""));
        // Bucket CENTRES, not indices.
        assert!(emitted.sql.contains("0.5"));
        assert!(emitted.sql.contains("16"));
        assert!(emitted.sql.contains("GROUP BY 1, 2"));
    }

    #[test]
    fn gomb_ac13_linear_regression_snapshot() {
        let src = r#"
data:
  athletes: { file: athletes.parquet }
plot:
  - mark: regressionY
    data: { from: athletes }
    x: weight
    y: height
"#;
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let emitted = emit_query(&spec, 0, None, None).expect("emit");
        // Aggregate-scalar shape: SELECT regr_* FROM (...) AS _as
        assert!(emitted.sql.contains("regr_slope"));
        assert!(emitted.sql.contains("regr_intercept"));
        assert!(emitted.sql.contains("regr_count"));
        assert!(emitted.sql.contains("regr_avgx"));
        assert!(emitted.sql.contains("regr_sxx"));
        assert!(emitted.sql.contains("regr_sxy"));
        assert!(emitted.sql.contains("regr_syy"));
        // Data extents the renderer builds its x/y scales from.
        assert!(emitted.sql.contains("AS x_min"));
        assert!(emitted.sql.contains("AS x_max"));
        assert!(emitted.sql.contains("AS y_min"));
        assert!(emitted.sql.contains("AS y_max"));
        assert!(!emitted.sql.contains("GROUP BY"));
    }
}
