//! Public entry points: data-source DDL emission and per-mark
//! query emission.

use std::path::Path;

use brightfield_spec::analysis::SELECTED_COLUMN;
use brightfield_spec::ast::{DataSourceKind, ParamNode, SelectionNode, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use indexmap::IndexMap;

use brightfield_spec::ast::{Component, Mark, PlotNode, ValueOrParamRef};
use brightfield_spec::vocab::{ImplStatus, InteractorKind, SelectionResolution};

use crate::binding::{Binding, EmittedQuery, ParamValues};
use crate::error::EmitError;
use crate::ir::{AggregateExpr, Predicate, QueryPlan, SampleRate, SelectionPredicate};
use crate::lower::{compile_selection, default_lowerers, find_lowerer, LowerCtx};
use crate::passes::apply_passes;
use crate::render::render_query;
use crate::source;

/// Reserved column carrying the per-GROUP count of selected rows — the
/// aggregating counterpart of
/// [`SELECTED_COLUMN`], which is a per-ROW boolean and cannot exist once the
/// rows are groups.
///
/// Shared across the SQL/render boundary by name, the way `__bf_count` already
/// is: `brightfield-render`'s `SELECTED_COUNT_COLUMN` holds the same literal and
/// must keep holding it.
pub const SELECTED_COUNT_COLUMN: &str = "__bf_selected_count";

/// Tag identifying which dispatch arm produced a [`SourceDdl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKindTag {
    Parquet,
    Csv,
    Json,
    Spatial,
    DuckDb,
    /// `ATTACH 'ducklake:…' (READ_ONLY)` — a DuckLake catalog attach.
    DuckLake,
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
    /// `Some(location)` when the resolved data location is reached over the
    /// network (an `http://` / `https://` URL, including inside a
    /// `ducklake:` URI). The engine uses this to gate DuckDB `httpfs`
    /// loading to specs that actually need it, and to name the network as
    /// the cause when such a source fails. `None` for local files, inline
    /// rows, and author-written `query:` SQL (which DuckDB's own autoload
    /// handles if it reaches out; the engine still names the network when
    /// such SQL fails with a network-shaped error).
    pub remote_location: Option<String>,
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
pub fn emit_sources(spec: &Spec, base_dir: Option<&Path>) -> Result<EmitOutput, EmitError> {
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
                remote_location: None,
            },
            DataSourceKind::Shorthand(s) => SourceDdl {
                view_name: name.clone(),
                sql: format!("CREATE OR REPLACE VIEW \"{}\" AS {}", name, s),
                source_kind: SourceKindTag::Query,
                remote_location: None,
            },
            DataSourceKind::InlineRows(rows) => {
                source::emit_inline_rows(name, rows, &data_source.extras)?
            }
            DataSourceKind::Typed(type_name) => {
                // Typed without a file: key — check if there's a file in extras
                if let Some(SpecValue::String(file_value)) = data_source.extras.get("file") {
                    source::emit_file_typed(
                        name,
                        file_value,
                        type_name,
                        &data_source.extras,
                        base_dir,
                    )?
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
pub(crate) const CSV_ALLOW_LIST: &[&str] =
    &["columns", "delim", "header", "nullstr", "skip", "types"];

/// Resolve a file path against a base directory.
///
/// HTTP(S) URLs pass through verbatim. Absolute paths pass through.
/// Relative paths are joined against `base_dir` if provided.
/// A `ducklake:` URI keeps its prefix and resolves the inner catalog
/// location by the same rules (so `ducklake:meta.ducklake` is
/// base-dir-relative while `ducklake:https://…` passes through verbatim).
pub(crate) fn resolve_path(file_value: &str, base_dir: Option<&Path>) -> String {
    // DuckLake URIs: resolve the catalog location inside the prefix.
    if let Some(inner) = file_value.strip_prefix("ducklake:") {
        return format!("ducklake:{}", resolve_path(inner, base_dir));
    }

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

/// A resolved data location as the BODY of a SQL single-quoted string: every
/// `'` doubled, per the SQL standard.
///
/// Every reader call this crate emits interpolates a path into `'…'`, and until
/// this existed it interpolated it raw. A spec author typing an apostrophe into
/// a `file:` value therefore emitted `read_csv('/data/Hugh's.csv')` — a syntax
/// error at best, and at worst SQL of the author's choosing running on the
/// reader's machine. `spec_value_to_sql_literal` had escaped kwarg strings for
/// exactly this reason since it was written; the path beside them had not.
///
/// It matters more now than it did: with a file dialog on the front door the
/// path is no longer something an author typed into a checked-in spec, it is
/// whatever the operating system handed back — and `Hugh's data.csv` is an
/// ordinary name for a file on a Mac.
pub(crate) fn sql_path_literal(resolved_path: &str) -> String {
    resolved_path.replace('\'', "''")
}

/// `Some(resolved_path)` when the resolved data location is reached over
/// the network — an `http://` / `https://` URL, directly or inside a
/// `ducklake:` URI. This is what a [`SourceDdl::remote_location`] carries.
pub(crate) fn remote_location(resolved_path: &str) -> Option<String> {
    let inner = resolved_path
        .strip_prefix("ducklake:")
        .unwrap_or(resolved_path);
    if inner.starts_with("http://") || inner.starts_with("https://") {
        Some(resolved_path.to_string())
    } else {
        None
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
    kwargs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect()
}

/// Convert a `SpecValue` to a SQL literal for use in kwargs.
pub(crate) fn spec_value_to_sql_literal(val: &SpecValue) -> String {
    match val {
        // Escape embedded single quotes (SQL-standard doubling) so string
        // literals — including interpolated param values and
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
        // An aggregate channel transform reaching a kwarg/literal position is
        // degenerate (aggregates belong on mark channels, consumed by the
        // hexbin / cell lowerers, not in data-source kwargs). Render the SQL
        // aggregate call so the output is at least valid SQL rather than a
        // panic. `count` with no column becomes `count(*)`.
        SpecValue::Aggregate { func, column } => match column {
            Some(col) => format!("{}(\"{}\")", func.wire_name(), col),
            None => format!("{}(*)", func.wire_name()),
        },
        // A positional bin reaching a kwarg/literal position is likewise
        // degenerate — bins belong on a rect's positional channel, consumed by
        // `RectLowerer`. Render the source column so the output is at least
        // valid SQL naming the right thing.
        SpecValue::Bin { column, .. } => format!("\"{column}\""),
        // A `sort:` reaching a kwarg/literal position is degenerate for the
        // same reason — it is a mark option consumed by `BarLowerer`, not a
        // value. Render the ordered column, which is the one identifier in it.
        SpecValue::Sort { by, .. } => format!("\"{by}\""),
    }
}

// Observable-Plot default plot margins, pinned to
// `brightfield_render::layout::Margins::default()` (top 20, right 20, bottom
// 30, left 40). Duplicated here because brightfield-sql does not depend on
// brightfield-render; the pixel-space hexbin lowerer needs the plot AREA
// (declared size minus margins) to match the renderer's data→pixel mapping so
// hexes are regular on screen. render's `gpu_layout_defaults_match_observable_plot`
// guards these values on its side.
const PLOT_MARGIN_X: f64 = 40.0 + 20.0;
const PLOT_MARGIN_Y: f64 = 20.0 + 30.0;

/// The pixel AREA (`width`, `height`) of the plot enclosing the mark at
/// `mark_index` — its declared `width`/`height` (or Mosaic defaults) minus the
/// default margins. This is the "static plot pixel extent at emit time" the
/// hexbin lowerer bins in. `None` when the mark is not inside a plot.
fn enclosing_plot_area_px(spec: &Spec, mark_index: usize) -> Option<(f64, f64)> {
    let dims = collect_mark_plot_dims(spec);
    let (w, h) = *dims.get(mark_index)?;
    Some(((w - PLOT_MARGIN_X).max(1.0), (h - PLOT_MARGIN_Y).max(1.0)))
}

/// Declared plot `(width, height)` for each mark, in depth-first mark order —
/// mirroring [`collect_marks`]. A mark inside a plot inherits that plot's
/// declared size (or the Mosaic defaults); a mark with no enclosing plot gets
/// the defaults.
fn collect_mark_plot_dims(spec: &Spec) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_mark_plot_dims_in(root, None, &mut out);
    }
    out
}

fn collect_mark_plot_dims_in(
    component: &Component,
    current: Option<(f64, f64)>,
    out: &mut Vec<(f64, f64)>,
) {
    use brightfield_spec::ast::PlotNode;
    use brightfield_spec::layout::{DEFAULT_PLOT_HEIGHT, DEFAULT_PLOT_WIDTH};

    fn plot_dim(node: &PlotNode, key: &str, default: f64) -> f64 {
        node.attributes
            .get(key)
            .and_then(|v| match v {
                SpecValue::Integer(n) => Some(*n as f64),
                SpecValue::Float(f) => Some(*f),
                _ => None,
            })
            .unwrap_or(default)
    }

    match component {
        Component::Plot(plot) => {
            let dims = (
                plot_dim(plot, "width", DEFAULT_PLOT_WIDTH),
                plot_dim(plot, "height", DEFAULT_PLOT_HEIGHT),
            );
            for item in &plot.items {
                collect_mark_plot_dims_in(item, Some(dims), out);
            }
        }
        Component::HConcat(concat) | Component::VConcat(concat) => {
            for item in &concat.items {
                collect_mark_plot_dims_in(item, current, out);
            }
        }
        Component::Mark(_) => {
            out.push(current.unwrap_or((
                brightfield_spec::layout::DEFAULT_PLOT_WIDTH,
                brightfield_spec::layout::DEFAULT_PLOT_HEIGHT,
            )));
        }
        _ => {}
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

/// The plot node each mark sits in, indexed like [`collect_marks`].
///
/// **The one join a renderer needs and the AST does not carry.** A `Mark` has no
/// back-pointer to its plot, and a plot-level attribute — `projectionType` above
/// all — has to reach the mark that draws under it. Built from
/// [`collect_plot_groups`] and `collect_plot_nodes` rather than by a third walk
/// of the tree, so the mark ordering here is the same ordering the engine
/// returns results in by construction and not by two traversals agreeing.
///
/// `None` for a bare mark that is not inside any plot (degenerate; Mosaic marks
/// normally live in a plot), which reads as "this mark's plot names no
/// projection".
#[must_use]
pub fn plot_of_each_mark(spec: &Spec) -> Vec<Option<&PlotNode>> {
    let nodes = brightfield_spec::layout::collect_plot_nodes(spec);
    let mut out: Vec<Option<&PlotNode>> = vec![None; collect_marks(spec).len()];
    for group in collect_plot_groups(spec) {
        let Some((_, node)) = nodes.iter().find(|(path, _)| *path == group.plot_path) else {
            continue;
        };
        for mi in group.mark_indices {
            if let Some(slot) = out.get_mut(mi) {
                *slot = Some(*node);
            }
        }
    }
    out
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
/// (closes the `_param_values` review finding). `None` falls
/// back to `Prepared` mode with `?` placeholders.
///
/// `selection_predicates` carries per-contributor predicates for any selection
/// the mark's `filter_by` references (v2 runtime coordinator). The
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
    selection_predicates: Option<&[SelectionPredicate]>,
) -> Result<EmittedQuery, EmitError> {
    let passes: Vec<Box<dyn crate::passes::Pass>> = vec![];
    emit_query_with_passes(
        spec,
        mark_index,
        param_values,
        selection_predicates,
        &passes,
    )
}

/// Lower a single mark to its query plan **before any selection threading** —
/// the pure lowerer output plus the mark's component path.
///
/// This is the seam the automatic pre-aggregation layer analyses: the cube
/// derivation needs the mark's plan *without* the live selection filter (the
/// active clause becomes cube dimensions instead), and it must see exactly the
/// plan [`emit_query`] starts from — so both call this one function.
///
/// # Errors
///
/// [`EmitError::InvariantViolation`] if `mark_index` is out of bounds;
/// otherwise whatever the mark's lowerer returns.
pub fn lower_mark_plan(spec: &Spec, mark_index: usize) -> Result<(QueryPlan, String), EmitError> {
    // Use the path-aware mark walker so we can compute the parent plot
    // identity for selection self-exclusion (v2 decision 4).
    let marks_with_paths = collect_marks_with_paths(spec);
    let (mark, mark_path) =
        marks_with_paths
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
        plot_px: enclosing_plot_area_px(spec, mark_index),
    };

    let lowerer = find_lowerer(mark.kind, &lowerers);
    let plan = lowerer.lower(mark, &ctx)?;
    Ok((plan, (*mark_path).to_string()))
}

/// Thread a compiled selection predicate into a lowered plan at the
/// semantically correct level.
///
/// For a **row-level plan** (Source / Filter / Projection chains) the filter
/// wraps the whole plan — the historical placement, byte-unchanged.
///
/// For an **aggregating plan** (`Aggregation` / `AggregateScalar`, possibly
/// under `Order` and `Limit`) the predicate is pushed onto the aggregation's *input*: a
/// selection filters the base rows that get aggregated, it does not filter the
/// aggregated output (whose columns are the group keys and aggregate aliases —
/// the brushed column is usually not among them, and even when an alias
/// collides, filtering bins is not filtering rows). This is the Mosaic
/// semantics the pre-aggregation layer's cube rewrite reproduces, so the
/// direct query and the cube-served query cannot disagree.
fn apply_selection_filter(plan: QueryPlan, predicate: Predicate) -> QueryPlan {
    match plan {
        QueryPlan::Order { input, keys } => QueryPlan::Order {
            input: Box::new(apply_selection_filter(*input, predicate)),
            keys,
        },
        // A `sort: {limit: n}` bar is `Limit{Order{Aggregation{…}}}`, and the
        // filter has to reach past the truncation as well as past the order.
        // Wrapping here instead would filter the top n rather than take the
        // top n of what the selection keeps — a different chart, and the one
        // both corpus `barX` marks would have drawn, since each carries a
        // `filterBy`.
        QueryPlan::Limit {
            input,
            limit,
            offset,
        } => QueryPlan::Limit {
            input: Box::new(apply_selection_filter(*input, predicate)),
            limit,
            offset,
        },
        QueryPlan::Aggregation {
            input,
            group_by,
            aggregates,
        } => QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Filter { input, predicate }),
            group_by,
            aggregates,
        },
        QueryPlan::AggregateScalar { input, aggregates } => QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Filter { input, predicate }),
            aggregates,
        },
        other => QueryPlan::Filter {
            input: Box::new(other),
            predicate,
        },
    }
}

/// The selection name a mark subscribes to via `data: { filterBy: $name }`,
/// with the mark's component path — the pre-aggregation layer's per-mark
/// lookup. `None` for an out-of-bounds index or a mark without `filterBy`.
#[must_use]
pub fn mark_selection_name(spec: &Spec, mark_index: usize) -> Option<(String, String)> {
    let marks_with_paths = collect_marks_with_paths(spec);
    let (mark, mark_path) = marks_with_paths.get(mark_index)?;
    let name = mark_filter_by_name(mark)?;
    Some((name.to_string(), (*mark_path).to_string()))
}

/// Interpolate `$name` scalar params into a SQL string — the same
/// substitution [`emit_query`] applies to its rendered SQL, exposed for
/// callers (the pre-aggregation layer) that render additional SQL from the
/// same plans and must agree byte-for-byte on param handling.
#[must_use]
pub fn interpolate_sql(sql: &str, params: &ParamValues) -> String {
    interpolate_params(sql, params)
}

/// Emit a query for a single mark, applying the given passes to the plan.
///
/// Pass-aware variant of [`emit_query`]. Same `param_values` /
/// `selection_predicates` contract, and never sampled — see
/// [`emit_query_sampled`].
pub fn emit_query_with_passes(
    spec: &Spec,
    mark_index: usize,
    param_values: Option<&ParamValues>,
    selection_predicates: Option<&[SelectionPredicate]>,
    extra_passes: &[Box<dyn crate::passes::Pass>],
) -> Result<EmittedQuery, EmitError> {
    emit_query_sampled(
        spec,
        mark_index,
        param_values,
        selection_predicates,
        extra_passes,
        None,
    )
}

/// [`emit_query_with_passes`], plus the one place a pushed-down sample is
/// introduced.
///
/// `sample: None` reproduces the unsampled emission **byte for byte** — the
/// plan is not touched, so an unsampled chart's SQL and its query count are
/// unchanged by this feature's existence, which
/// `sample_leaves_unsampled_emission_byte_identical` asserts.
///
/// # Where the clause goes, and why both bounds are silent if broken
///
/// The sample is inserted immediately after the mark's plan is lowered, which
/// puts it in the one position that satisfies two constraints pulling in
/// opposite directions:
///
/// - **Above the mark's projection.** [`QueryPlan::Sample`] renders
///   `hash(_s)`, and `_s` is whatever the subquery beneath it projects. Placed
///   under the mark's channel projection it would hash the SOURCE row — which
///   DuckDB compiles to `hash(struct_pack(<every column>))` and which
///   therefore requires reading every column, destroying parquet projection
///   pushdown on the exact query that needed it most.
/// - **Below the highlight projection.** The highlight pass appends a
///   `(<pred>) AS __bf_selected` column. If the sample sat above that, the
///   selected-flag would be part of the hashed tuple and **the sample would
///   reshuffle on every selection change** — a brush would silently redraw a
///   different subset of points, which is precisely the determinism the whole
///   scheme exists to provide.
///   `sample_sits_below_the_highlight_projection_so_a_brush_cannot_reshuffle_it`
///   pins this structurally — by extracting the hashed subquery's own text and
///   requiring the flag to be absent from it, not by comparing where two
///   substrings land, which an earlier version did and which held under the
///   very relocation it existed to catch. What that placement costs when it is
///   wrong is measured over rows in `brightfield-engine`'s
///   `the_highlight_flag_must_stay_outside_the_hashed_tuple`: 41-43% of the
///   drawn set re-drawn by a single brush move.
///
/// The [`Predicate`] threading in between is order-free: the hash is a function
/// of the row, so sampling then filtering and filtering then sampling select
/// the same rows.
///
/// Emitting here also keeps the pre-aggregation layer consistent for free.
/// [`lower_mark_plan`] is what the cube derivation analyses, and it runs
/// *before* this, so the cube never sees a `Sample` node. The two paths cannot
/// both engage anyway: a sample is only applied to a NON-aggregating plan (an
/// aggregating mark's picture is O(bins), not O(rows), so it has nothing to
/// gain and its output rows are not the rows anyone means), while the cube only
/// serves aggregating plans — and the cube's own chain check rejects unknown
/// node kinds through a catch-all, so a `Sample` in a chain would decline the
/// cube rather than mis-serve it. `sampling_and_the_cube_stay_disjoint` asserts
/// the guard rather than trusting the argument.
pub fn emit_query_sampled(
    spec: &Spec,
    mark_index: usize,
    param_values: Option<&ParamValues>,
    selection_predicates: Option<&[SelectionPredicate]>,
    extra_passes: &[Box<dyn crate::passes::Pass>],
    sample: Option<SampleRate>,
) -> Result<EmittedQuery, EmitError> {
    let plan = plan_for_mark(spec, mark_index, selection_predicates, sample)?;

    // Apply optimisation passes (built-in + caller-provided).
    let plan = apply_passes(plan, extra_passes);

    let plan_hash = plan.hash_structural();
    let mut bindings: Vec<Binding> = Vec::new();
    let rendered = render_query(&plan, &mut bindings);

    // Interpolate scalar param values into the emitted SQL (Decision 1).
    // `$name` placeholders in lowerer-emitted projections / filter
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

/// The plan a mark's caller-provided passes are handed — everything
/// [`emit_query_sampled`] builds **before** `extra_passes` run, and nothing
/// after.
///
/// It exists so a caller can ask a pass a question ABOUT the plan it is going
/// to be applied to and get an answer from the same plan the emitter will use.
/// The one caller today is the navigation layer: a
/// [`NavigationFilterPass`](crate::navigation_filter_pass::NavigationFilterPass)
/// resolves each axis against the plan's shape and emits nothing for an axis it
/// cannot place honestly, and the chart pane has to be able to SAY that a mark
/// did not rescope. Re-deriving "does this mark aggregate" beside the emitter
/// would be one refactor away from disagreeing with it silently, which is
/// exactly how the decline came to be computed and thrown away in the first
/// place.
///
/// `param_values` is deliberately absent: scalar params are interpolated into
/// the rendered SQL, never into the plan, so the plan does not depend on them.
///
/// # Errors
///
/// As [`emit_query_sampled`].
pub fn plan_for_mark(
    spec: &Spec,
    mark_index: usize,
    selection_predicates: Option<&[SelectionPredicate]>,
    sample: Option<SampleRate>,
) -> Result<QueryPlan, EmitError> {
    let (mut plan, mark_path) = lower_mark_plan(spec, mark_index)?;
    let marks_with_paths = collect_marks_with_paths(spec);
    let (mark, _) = marks_with_paths
        .get(mark_index)
        .expect("bounds checked by lower_mark_plan");

    // The sample, at the one position that is above the mark's projection and
    // below the highlight projection. An aggregating plan is guarded out: its
    // rows are bins, and sampling bins is not sampling data.
    if let Some(rate) = sample {
        if !plan_aggregates(&plan) {
            plan = QueryPlan::Sample {
                input: Box::new(plan),
                modulus: rate.modulus(),
            };
        }
    }

    // Selection threading: if the mark has a filter_by reference and the
    // referenced name is a SelectionNode in spec.params, compile the live
    // contributors into a predicate and thread it into the lowered plan.
    // Self-exclusion identity is the stable plot-node path
    // (decision 4) — `plot_node_path`, NOT `parent_plot`, so a mark and the
    // brushing interactor in the same plot resolve to the same identity (else a
    // plot filters itself).
    if let Some(selection_name) = mark_filter_by_name(mark) {
        if let Some(ParamNode::Selection(sel_node)) = spec.params.get(selection_name) {
            let self_source = brightfield_spec::analysis::plot_node_path(&mark_path);
            let contributors: &[(String, Predicate)] = selection_predicates
                .and_then(|all| {
                    all.iter()
                        .find(|(n, _)| n == selection_name)
                        .map(|(_, c)| c.as_slice())
                })
                .unwrap_or(&[]);
            let predicate = compile_selection(sel_node, self_source, contributors);
            // Skip wrapping in Predicate::True case so unfiltered queries
            // stay structurally identical to pre-card behaviour (relied on
            // by existing render-shape tests).
            if predicate != Predicate::True {
                plan = apply_selection_filter(plan, predicate);
            }
        }
    }

    // Highlight membership projection: if this mark's plot carries a
    // `highlight, by: $sel` interactor, project a per-row boolean
    // `(<pred>) AS __bf_selected` OUTSIDE the (possibly filtered) plan instead of
    // filtering — the mark keeps its full batch and DIMS the non-matching rows.
    // Membership evaluates against the source table (so a
    // splom panel highlights on a column it does not plot). An empty selection
    // compiles to `True` → no projection → the mark renders exactly as at rest.
    //
    // AN AGGREGATING PLAN TAKES THE OTHER BRANCH, and the guard that sends it
    // there stays. The reason it was written is still true: the rows of an
    // aggregating plan are GROUPS, so there is no row for a per-row membership
    // boolean to be about, and a `(<pred>)` in the select list of a `GROUP BY`
    // is neither grouped nor aggregated — DuckDB rejects it whenever the
    // brushed column is not one of the mark's own grouping dimensions, which is
    // exactly the cross-filter case. What changes is that the branch no longer
    // ends in silence: a group gets a COUNT of its selected rows instead
    // (`selected_count_aggregate`), which is a number the renderer can draw a
    // part of a bar from.
    //
    // That second branch is narrowed to the families
    // `mark_honours_highlight` names, which is where the render side reads a
    // highlight at all. A column no renderer looks at is SQL a chart pays for
    // and nobody sees, and the aggregating families outside that list — the
    // heatmaps, rasters, hexbins and contours — have no bar to draw a part of.
    //
    // Two departures from the filterBy path above:
    //   FIX A — the `by:` selection may be created ONLY by an `as:` binding and
    //     never declared in `params:` (weather's `$range`), so it is absent from
    //     `spec.params`. A declared Selection uses its resolution; an as-bound-only
    //     name synthesises a default-resolution node. A declared VALUE param is not
    //     a selection → skipped (analysis warns HighlightBindingNonSelection).
    //   FIX B — highlight NEVER self-excludes (`HIGHLIGHT_NO_SELF_EXCLUDE`): the
    //     brushed plot must dim its OWN rows.
    // (The filterBy gate above shares FIX A's blind spot — an as-bound-only
    // `filterBy` is likewise inert there — but that is a pre-existing
    // limitation, out of this card's scope; left untouched deliberately.)
    let mark_highlight_by = collect_mark_highlight_by(spec);
    if let Some(Some(selection_name)) = mark_highlight_by.get(mark_index) {
        let default_node = as_bound_selection_default();
        let sel_node = match spec.params.get(selection_name) {
            Some(ParamNode::Selection(sel)) => Some(sel),
            // A declared value param is not a selection — never projects.
            Some(ParamNode::Value(_)) => None,
            // As-bound-only (or unknown): synthesise a resolution. An unknown name
            // simply has no live contributors → True → no projection.
            None => Some(&default_node),
        };
        if let Some(sel_node) = sel_node {
            let contributors: &[(String, Predicate)] = selection_predicates
                .and_then(|all| {
                    all.iter()
                        .find(|(n, _)| n == selection_name)
                        .map(|(_, c)| c.as_slice())
                })
                .unwrap_or(&[]);
            let predicate = compile_selection(sel_node, HIGHLIGHT_NO_SELF_EXCLUDE, contributors);
            if predicate != Predicate::True {
                plan = if plan_aggregates(&plan) {
                    if brightfield_spec::analysis::mark_honours_highlight(mark.kind) {
                        project_selected_count(plan, &predicate)
                    } else {
                        plan
                    }
                } else {
                    QueryPlan::Projection {
                        input: Box::new(plan),
                        columns: vec![
                            "*".to_string(),
                            format!("({predicate}) AS {SELECTED_COLUMN}"),
                        ],
                    }
                };
            }
        }
    }

    Ok(plan)
}

/// Add the per-group count of SELECTED rows to an aggregating plan, as a second
/// aggregate beside the ones the mark already asked for.
///
/// The predicate is summed as a boolean rather than applied as a `WHERE`:
/// `SUM(CAST((<pred>) AS INT))`. Two things follow, and both are the reason
/// this shape was chosen over filtering.
///
/// * **The group keeps its full count.** The mark stays unfiltered, so its bar
///   is still the whole total and the selected count is a part OF it. A `WHERE`
///   would give the subset and lose the denominator, which is the failure being
///   fixed.
/// * **The brushed column need not be a grouping dimension.** The expression is
///   inside an aggregate, so it is legal over any column of the aggregation's
///   input — which is what makes the genuine cross-filter case (brush one
///   column, group by another) expressible at all.
///
/// One query, not two: the total and the selected part come back in the same
/// grouped result.
///
/// Walks the row-preserving wrappers a lowerer puts above its aggregation
/// (`Order`, `Limit`) to reach it. `Projection` is NOT walked through: a
/// projection above an aggregation names its output columns, and an aggregate
/// added underneath one would be projected away. Anything else is returned
/// untouched — the caller has already established that the plan aggregates, so
/// the only way to arrive here without finding one is a plan shape no lowerer
/// currently emits.
fn project_selected_count(plan: QueryPlan, predicate: &Predicate) -> QueryPlan {
    match plan {
        QueryPlan::Order { input, keys } => QueryPlan::Order {
            input: Box::new(project_selected_count(*input, predicate)),
            keys,
        },
        QueryPlan::Limit {
            input,
            limit,
            offset,
        } => QueryPlan::Limit {
            input: Box::new(project_selected_count(*input, predicate)),
            limit,
            offset,
        },
        QueryPlan::Aggregation {
            input,
            group_by,
            mut aggregates,
        } => {
            aggregates.push(selected_count_aggregate(predicate));
            QueryPlan::Aggregation {
                input,
                group_by,
                aggregates,
            }
        }
        QueryPlan::AggregateScalar {
            input,
            mut aggregates,
        } => {
            aggregates.push(selected_count_aggregate(predicate));
            QueryPlan::AggregateScalar { input, aggregates }
        }
        other => other,
    }
}

/// `CAST(SUM(CAST((<pred>) AS INT)) AS DOUBLE) AS __bf_selected_count`.
///
/// [`AggregateExpr::Raw`] rather than a typed call, deliberately: the
/// pre-aggregation layer treats `Raw` as opaque and bails to the direct query,
/// which is the right answer here. The predicate is rewritten on every gesture,
/// so a cube keyed on the plan that carried it would be stale the moment the
/// brush moved.
fn selected_count_aggregate(predicate: &Predicate) -> AggregateExpr {
    AggregateExpr::Raw(format!(
        "CAST(SUM(CAST(({predicate}) AS INT)) AS DOUBLE) AS {SELECTED_COUNT_COLUMN}"
    ))
}

/// **Whose contribution a step-rows query is allowed to drop.**
///
/// Under `select: crossfilter` every consumer of a selection drops the clause
/// its OWN plot published, so the brushed plot keeps drawing the whole table
/// while its siblings narrow. That is right for a mark — a plot that erased the
/// points the hand is still holding would fight the gesture — and wrong for
/// anything that is not a mark, because a surface that published no clause has
/// none to drop. There is therefore no one query that answers for both, and the
/// choice cannot be inferred from the mark index: it is a fact about the
/// **caller**, so this type makes every call site state it.
///
/// The variants exist because reading it off the mark is exactly what the rows
/// pane did wrong. It read mark 0 of a generated dashboard — the hero's ghost
/// layer, which carries no `filterBy:` at all — and would still have read 240
/// of 240 rows had it read the hero's subset layer instead, because a brush on
/// the map is the hero's own contribution and the hero drops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowsAudience {
    /// A mark drawn on a plot. Crossfilter drops the clause that plot
    /// published — [`compile_selection`] is given the mark's own plot node
    /// path — so this is byte-identical to what
    /// [`emit_query`] compiles for the same mark, and
    /// the chart and a grid asking as `Plot` cannot resolve two WHERE clauses.
    Plot,
    /// A surface that draws no mark and publishes no clause: the rows pane, a
    /// status band's count, an export. [`compile_selection`] is given
    /// [`crate::lower::NO_SELF_EXCLUDE`], which matches no contributor path, so
    /// the query carries the selection's **value**: each contributor's
    /// predicate under the declared resolution.
    Reader,
}

impl RowsAudience {
    /// The `self_source` [`compile_selection`] is given for a mark at
    /// `mark_path`.
    fn self_source(self, mark_path: &str) -> &str {
        match self {
            Self::Plot => brightfield_spec::analysis::plot_node_path(mark_path),
            Self::Reader => crate::lower::NO_SELF_EXCLUDE,
        }
    }
}

/// Emit the ROW-LEVEL query for a mark's step — every column of the step's
/// materialisation, under the *same* static `data.filter` and the *same* live
/// selection predicate the chart's [`emit_query`] applies to that mark.
///
/// This is the seam that makes a tabular ("grid") surface and a chart surface at
/// one step **unable to disagree about the PREDICATE when both ask as the same
/// [`RowsAudience`]**: both read from the identical source view, both compile
/// through the one `mark_filter_by_name` + `compile_selection` path below, and
/// both push the result into DuckDB rather than filtering a materialised result
/// client-side. The chart query then projects its channels (and may aggregate);
/// the grid query keeps `SELECT *`.
///
/// The audience is the term that used to be implied, and implying it is what
/// produced a rows pane listing 240 rows under a brush that selected 45. A mark
/// asks as [`RowsAudience::Plot`] and a surface that is not a mark asks as
/// [`RowsAudience::Reader`]; under `select: crossfilter` those are two different
/// WHERE clauses **on purpose**, and the enum's own doc says why.
///
/// It is **not** a claim that the two draw the same rows, and since sampling
/// landed it would be false as one. Above the renderer's drawable ceiling the
/// chart's plan carries an extra [`QueryPlan::Sample`] clause — a
/// `hash(row) % 2^k = 0` predicate on the row's own bytes — and says so in the
/// plot's ink. This function emits no such clause and is never given the sample
/// rate: **the grid is the unsampled view**, deliberately, because it is where
/// someone goes when they doubt the picture. One divergence, in one direction,
/// declared on the surface that has it.
///
/// Only marks with `data: { from: <source> }` have a materialisation to grid;
/// an inline-data or data-less mark returns [`EmitError::UnsupportedMark`].
///
/// # Errors
///
/// [`EmitError::InvariantViolation`] if `mark_index` is out of bounds, or
/// [`EmitError::UnsupportedMark`] for a mark without a `from`-source.
pub fn emit_rows_query(
    spec: &Spec,
    mark_index: usize,
    param_values: Option<&ParamValues>,
    selection_predicates: Option<&[SelectionPredicate]>,
    audience: RowsAudience,
) -> Result<EmittedQuery, EmitError> {
    let marks_with_paths = collect_marks_with_paths(spec);
    let (mark, mark_path) =
        marks_with_paths
            .get(mark_index)
            .ok_or_else(|| EmitError::InvariantViolation {
                detail: format!(
                    "mark_index {} out of bounds (spec has {} marks)",
                    mark_index,
                    marks_with_paths.len()
                ),
            })?;

    // The grid reads the step's SOURCE view directly, so it needs a `from`
    // source. Inline / data-less marks have no materialisation to tabulate.
    let (source, extras) = match &mark.data {
        Some(brightfield_spec::ast::MarkData::From { source, extras, .. }) => (source, extras),
        _ => {
            return Err(EmitError::UnsupportedMark {
                kind: format!(
                    "{} (grid rows need data: {{ from: ... }})",
                    mark.kind.wire_name()
                ),
            })
        }
    };

    // SELECT * FROM <source>, plus the step's static `data.filter` — the row
    // shape of the materialisation before any channel projection or aggregation.
    let base = QueryPlan::Source {
        table: source.clone(),
    };
    let mut plan = crate::lower::apply_data_filter(extras, base);

    // The same live selection filter and the same `compile_selection` the chart
    // mark receives. What `audience` decides is the self-exclusion identity: a
    // mark passes its own plot node path and drops the clause that plot
    // published, a reader passes the sentinel and drops nothing.
    if let Some(selection_name) = mark_filter_by_name(mark) {
        if let Some(ParamNode::Selection(sel_node)) = spec.params.get(selection_name) {
            let self_source = audience.self_source(mark_path);
            let contributors: &[(String, Predicate)] = selection_predicates
                .and_then(|all| {
                    all.iter()
                        .find(|(n, _)| n == selection_name)
                        .map(|(_, c)| c.as_slice())
                })
                .unwrap_or(&[]);
            let predicate = compile_selection(sel_node, self_source, contributors);
            if predicate != Predicate::True {
                plan = QueryPlan::Filter {
                    input: Box::new(plan),
                    predicate,
                };
            }
        }
    }

    let plan_hash = plan.hash_structural();
    let mut bindings: Vec<Binding> = Vec::new();
    let rendered = render_query(&plan, &mut bindings);
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
/// from `params` (Decision 1). Matching is identifier-boundary aware
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

/// The `by:` selection name of a mark's enclosing plot's `highlight` interactor,
/// for each mark in depth-first order (mirroring [`collect_marks`]). `None` when
/// the plot has no `highlight` interactor or its `by:` is not a lifted `Param`.
/// A mark inherits its innermost enclosing plot's highlight.
fn collect_mark_highlight_by(spec: &Spec) -> Vec<Option<String>> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_mark_highlight_by_in(root, None, &mut out);
    }
    out
}

fn collect_mark_highlight_by_in(
    component: &Component,
    current: Option<&str>,
    out: &mut Vec<Option<String>>,
) {
    match component {
        Component::Plot(plot) => {
            // A highlight interactor is scoped to its own plot; a mark takes its
            // INNERMOST enclosing plot's highlight (matching the analysis
            // subscriber registration), so a nested sub-plot without its own
            // highlight does not inherit an outer plot's.
            let by = plot_highlight_by_name(&plot.items);
            for item in &plot.items {
                collect_mark_highlight_by_in(item, by.as_deref(), out);
            }
        }
        Component::HConcat(concat) | Component::VConcat(concat) => {
            for item in &concat.items {
                collect_mark_highlight_by_in(item, current, out);
            }
        }
        Component::Mark(_) => out.push(current.map(str::to_string)),
        _ => {}
    }
}

/// The selection name a plot's `highlight` interactor consumes (`by: $sel`), if
/// present. Scans a plot's items for a `highlight` [`InteractorKind`] whose
/// `by:` lifted to a `Param` ref.
fn plot_highlight_by_name(items: &[Component]) -> Option<String> {
    for item in items {
        if let Component::Interactor(i) = item {
            if i.kind == InteractorKind::Highlight {
                if let Some(ValueOrParamRef::Param(pr)) = i.options.get("by") {
                    return Some(pr.0.clone());
                }
            }
        }
    }
    None
}

/// A `self_source` that matches no real contributor path, so
/// `compile_selection`'s crossfilter branch excludes NOTHING. Highlight passes
/// this (unlike filterBy) because "brush a region, grey the rest" must dim the
/// brushed plot's OWN rows too — a highlight-bound mark self-excluding its own
/// plot's contribution would leave the brushed plot un-dimmed (FIX B).
///
/// The sentinel itself is [`crate::lower::NO_SELF_EXCLUDE`], shared rather than
/// re-spelled: two independently declared "can never collide" strings are two
/// things that have to go on not colliding.
const HIGHLIGHT_NO_SELF_EXCLUDE: &str = crate::lower::NO_SELF_EXCLUDE;

/// The resolution synthesised for a selection that is created ONLY by an `as:`
/// binding and never declared in `params:` — e.g. `highlight.yaml`'s `$range`,
/// which exists solely via `intervalXY as: $range`. `compile_selection` still
/// reads the live contributors; only the resolution needs a default.
///
/// `Single` matches every EXPLICIT resolution in the highlight corpus (splom's
/// `$brush`, weather's `$click` are both `single`), never self-excludes, and —
/// for a single-contributor brush (the corpus shape) — resolves identically to
/// any other resolution. Multi-contributor as-bound selections are a documented
/// edge (they combine as "most recent" under `single`).
///
/// Public because a surface that READS what a selection holds has to reach the
/// same answer the emit path reaches. A readout that skipped the undeclared
/// names would go silent on exactly the specs whose selection is born from a
/// gesture; one that guessed a second default would eventually disagree with
/// the SQL about what the brush means.
#[must_use]
pub fn as_bound_selection_default() -> SelectionNode {
    SelectionNode {
        select: SelectionResolution::Single,
        status: ImplStatus::Implemented,
        options: IndexMap::new(),
    }
}

/// Whether a plan AGGREGATES in SQL anywhere in its tree — a GROUP BY or scalar
/// aggregate restricts the output columns, so appending `(<pred>) AS
/// __bf_selected` over it could reference a column the aggregate dropped and
/// SQL-error. Highlight skips the membership projection for such a plan (the
/// runtime guard; analysis also warns `HighlightOnAggregate`). A
/// row-level plan (Source / Filter / Projection over a source) exposes every
/// source column, so the projection is always safe there.
fn plan_aggregates(plan: &QueryPlan) -> bool {
    match plan {
        QueryPlan::Aggregation { .. } | QueryPlan::AggregateScalar { .. } => true,
        QueryPlan::Filter { input, .. }
        | QueryPlan::Projection { input, .. }
        | QueryPlan::Bin { input, .. }
        | QueryPlan::Order { input, .. }
        | QueryPlan::Limit { input, .. }
        | QueryPlan::Sample { input, .. } => plan_aggregates(input),
        QueryPlan::Source { .. } | QueryPlan::Singleton { .. } => false,
    }
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
    // Param-effect routing — interpolation + plan_hash fold
    // -----------------------------------------------------------------------

    /// a `$param` placeholder is substituted with its concrete value.
    #[test]
    fn interpolate_scalar_param() {
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(20));
        let sql = interpolate_params("SELECT * FROM t WHERE x > $k", &params);
        assert_eq!(sql, "SELECT * FROM t WHERE x > 20");
    }

    /// a `$name` not in param_values is left intact (no partial
    /// interpolation), and identifier boundaries are respected — `$k` must not
    /// consume `$k2`.
    #[test]
    fn interpolate_unknown_and_boundaries() {
        let mut params = ParamValues::new();
        params.insert("k".to_string(), SpecValue::Integer(1));
        let sql = interpolate_params("$k2 + $k + $unknown", &params);
        assert_eq!(
            sql, "$k2 + 1 + $unknown",
            "only the exact known $k is inlined"
        );
    }

    /// a string-valued param is emitted quoted and escaped
    /// (single-quote doubling) — never raw — so interpolation is injection-safe.
    #[test]
    fn interpolation_escapes_string_param() {
        let mut params = ParamValues::new();
        params.insert("s".to_string(), SpecValue::String("O'Brien".to_string()));
        let sql = interpolate_params("WHERE name = $s", &params);
        assert_eq!(sql, "WHERE name = 'O''Brien'");
        assert!(!sql.contains("$s"), "raw placeholder must be gone");
    }

    /// pefr review regression: interpolation is string-literal aware —
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

    /// a bare `$param` positional channel is projected into the
    /// SELECT as `$param AS "<param>"`, and interpolation yields `<value> AS
    /// "<param>"` — the channel reaches the query rather than being dropped.
    #[test]
    fn param_channel_projected() {
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

    /// a `data.filter` expression lowers into a WHERE clause and
    /// its `$param` is interpolated (Decision 2).
    #[test]
    fn data_filter_lowers_to_where() {
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

    /// A selection filter on an AGGREGATING mark (density) lands INSIDE the
    /// aggregation — filtering the base rows that get aggregated — never
    /// outside it (where the brushed column is not among the output columns
    /// and the filter would either error or filter bins instead of rows).
    /// This is the placement the pre-aggregation cube rewrite reproduces, so
    /// direct and cube-served queries cannot disagree.
    #[test]
    fn selection_filter_on_aggregate_mark_filters_base_rows() {
        let src = "params:\n  brush:\n    select: intersect\ndata:\n  t: [{ x: 1, y: 9 }, { x: 2, y: 1 }]\nplot:\n  - mark: densityX\n    data: { from: t, filterBy: $brush }\n    x: x\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let selections: Vec<SelectionPredicate> = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("\"y\" <= 5".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).unwrap();
        let sql = &emitted.sql;
        let group_pos = sql.find("GROUP BY").expect("density aggregates");
        let pred_pos = sql.find("\"y\" <= 5").expect("selection predicate present");
        assert!(
            pred_pos < group_pos,
            "the selection WHERE must sit inside (before) the aggregation, got: {sql}"
        );
        // Still parseable SQL.
        crate::conform::parse_and_normalise(sql).expect("valid SQL");
    }

    /// The same selection on a ROW-LEVEL mark (dot) keeps the historical
    /// outer-wrap placement, byte-identical.
    #[test]
    fn selection_filter_on_row_mark_wraps_outside_unchanged() {
        let src = "params:\n  brush:\n    select: intersect\ndata:\n  t: [{ x: 1, y: 9 }]\nplot:\n  - mark: dot\n    data: { from: t, filterBy: $brush }\n    x: x\n    y: y\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let selections: Vec<SelectionPredicate> = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("\"y\" <= 5".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).unwrap();
        assert_eq!(
            emitted.sql, "SELECT * FROM (SELECT * FROM \"t\") AS _f WHERE (\"y\" <= 5)",
            "row-level placement is unchanged"
        );
    }

    /// `lower_mark_plan` returns the pure pre-selection plan (identical to
    /// what `emit_query` starts from) plus the mark's component path, and
    /// `mark_selection_name` resolves the mark's filterBy binding.
    #[test]
    fn lower_mark_plan_and_selection_name_expose_the_preagg_seam() {
        let src = "params:\n  brush:\n    select: intersect\ndata:\n  t: [{ x: 1 }]\nplot:\n  - mark: densityX\n    data: { from: t, filterBy: $brush }\n    x: x\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let (plan, path) = lower_mark_plan(&spec, 0).expect("lowers");
        assert!(
            matches!(plan, QueryPlan::Order { .. }),
            "density lowers to Order(Aggregation(..)) with no selection filter"
        );
        assert_eq!(path, "root/plot[0]/mark[densityX]");
        let (name, sel_path) = mark_selection_name(&spec, 0).expect("filterBy resolves");
        assert_eq!(name, "brush");
        assert_eq!(sel_path, path);
        assert!(
            mark_selection_name(&spec, 9).is_none(),
            "out of bounds → None"
        );
    }

    /// pefr review regression (#4): a filter with NO `$param` parses to
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

    /// plan_hash is STRUCTURAL — a param that does NOT appear in a
    /// mark's SQL (the SQL-invariant / pure tier) yields an identical plan_hash
    /// across values, so nothing perturbs the plan record. This is the property
    /// relies on for selection params (which route through concrete
    /// predicates carrying no `$name`).
    #[test]
    fn sql_invariant_param_keeps_plan_hash() {
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

    /// Refined: a channel-param value change is reflected in the
    /// concrete emitted SQL (the data-shape effect), while plan_hash stays
    /// STRUCTURAL. execute_emitted runs this concrete SQL directly and dedups on
    /// the literal string via the LRU sql_cache, so the value need not enter the
    /// hash — the earlier value-fold was removed as redundant + unbounded.
    #[test]
    fn emit_channel_param_changes_sql_not_structural_hash() {
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
    fn emit_query_unsupported_mark() {
        // Use a mark kind that has no lowerer (voronoi is the unimplemented
        // stand-in now that geo is wired — the placeholder swap dance).
        let src = "plot:\n  - mark: voronoi\n    data: { from: flights }\ndata:\n  flights: { file: flights.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "voronoi"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn emit_query_succeeds_for_from_data() {
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
    fn emit_query_out_of_bounds() {
        let src = "plot:\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 99, None, None);
        assert!(matches!(result, Err(EmitError::InvariantViolation { .. })));
    }

    #[test]
    fn emit_rows_query_shares_predicate_with_chart() {
        // A dot mark that filters by a selection. The chart query projects its
        // channels; the grid (rows) query keeps SELECT *. Both must push the
        // SAME selection predicate into the SQL WHERE — that is the "grid and
        // chart cannot disagree" seam.
        let src = r#"
params:
  brush:
    select: intersect
data:
  t: { file: t.parquet }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;

        // A distinctive predicate contributed from a DIFFERENT plot node so
        // self-exclusion does not drop it.
        let pred_text = "x > 42".to_string();
        let selections: Vec<SelectionPredicate> = vec![(
            "brush".to_string(),
            vec![(
                "root/plot[99]".to_string(),
                Predicate::Expr(pred_text.clone()),
            )],
        )];

        let chart = emit_query(&spec, 0, None, Some(&selections)).expect("chart emit");
        let rows = emit_rows_query(&spec, 0, None, Some(&selections), RowsAudience::Plot)
            .expect("rows emit");

        // Both surfaces carry the identical predicate — pushed into DuckDB.
        assert!(
            chart.sql.contains(&pred_text),
            "chart SQL must push the predicate: {}",
            chart.sql
        );
        assert!(
            rows.sql.contains(&pred_text),
            "grid SQL must push the same predicate: {}",
            rows.sql
        );
        // The grid keeps every column; the chart projected x/y channels.
        assert!(
            rows.sql.contains("SELECT *"),
            "grid rows query is row-level SELECT *: {}",
            rows.sql
        );
        assert!(
            rows.sql.contains("\"t\"") || rows.sql.contains(" t "),
            "grid rows query reads the same source view: {}",
            rows.sql
        );
    }

    #[test]
    fn emit_rows_query_rejects_inline_mark() {
        // Inline data has no materialisation view to tabulate.
        let src = "plot:\n  - mark: dot\n    data:\n      - { x: 1, y: 2 }\n    x: x\n    y: y\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_rows_query(&spec, 0, None, None, RowsAudience::Plot);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    #[test]
    fn emit_all_queries_returns_per_mark_results() {
        let src = "plot:\n  - mark: line\n    data: { from: t }\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let results = emit_all_queries(&spec, None);
        assert_eq!(results.len(), 2, "should have one result per mark");
        // SimpleLowerer handles both line and dot with data.from
        for result in &results {
            assert!(
                result.is_ok(),
                "line and dot with data.from should succeed via SimpleLowerer"
            );
        }
    }

    #[test]
    fn emit_query_no_marks_spec() {
        // A spec with no marks — just data
        let src = "data:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None, None);
        assert!(matches!(result, Err(EmitError::InvariantViolation { .. })));
    }

    // ----- Conformance snapshots for statistical marks -----
    //
    // Snapshots capture the emitted SQL string only. They do NOT capture
    // the rendered Vello scene — that's a future-card concern (cross-platform
    // float reproducibility around the Gaussian kernel will need tolerance
    // comparison if we ever snapshot rendered pixels).

    #[test]
    fn density1d_x_snapshot() {
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
        // Deterministic row order — GROUP BY output order is unspecified, and
        // draw order must not jitter between renders.
        assert!(emitted.sql.contains("ORDER BY \"weight\" ASC"));
    }

    #[test]
    fn density2d_snapshot() {
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
        // Deterministic row order: x centre, then y centre.
        assert!(emitted
            .sql
            .contains("ORDER BY \"weight\" ASC, \"height\" ASC"));
    }

    #[test]
    fn linear_regression_snapshot() {
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

    // -----------------------------------------------------------------------
    // Highlight membership projection
    // -----------------------------------------------------------------------

    const HIGHLIGHT_SPEC: &str = r#"
params:
  brush: { select: single }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: intervalXY
    as: $brush
  - select: highlight
    by: $brush
"#;

    /// a highlight-bound mark with an ACTIVE selection projects
    /// `(<pred>) AS __bf_selected` — and does NOT wrap the plan in a WHERE, so
    /// the mark keeps its full batch and dims (highlight-not-filter).
    #[test]
    fn highlight_projects_membership_column() {
        let spec = parse_spec(HIGHLIGHT_SPEC, Format::Yaml).unwrap().spec;
        let selections = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("a > 1".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            emitted.sql.contains(SELECTED_COLUMN),
            "membership projection present: {}",
            emitted.sql
        );
        assert!(emitted.sql.contains("a > 1"), "the predicate is projected");
        assert!(
            !emitted.sql.to_uppercase().contains("WHERE"),
            "highlight must not filter rows: {}",
            emitted.sql
        );
    }

    /// with NO live selection a highlight plot's mark emits SQL
    /// byte-identical to the same plot without any highlight interactor — the
    /// at-rest look, so example PNGs don't move.
    #[test]
    fn empty_selection_no_projection() {
        let spec = parse_spec(HIGHLIGHT_SPEC, Format::Yaml).unwrap().spec;
        let at_rest = emit_query(&spec, 0, None, None).expect("emit");
        assert!(
            !at_rest.sql.contains(SELECTED_COLUMN),
            "empty selection projects nothing: {}",
            at_rest.sql
        );
        // Same spec sans the highlight interactor emits the identical SQL.
        let plain_yaml = r#"
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
"#;
        let plain = parse_spec(plain_yaml, Format::Yaml).unwrap().spec;
        let plain_sql = emit_query(&plain, 0, None, None).expect("emit").sql;
        assert_eq!(
            at_rest.sql, plain_sql,
            "at rest, highlight is invisible in the SQL"
        );
    }

    /// a mark that is BOTH `filterBy` one selection and `highlight` on
    /// another resolves per its explicit bindings — a WHERE for the filter AND a
    /// `__bf_selected` projection for the highlight, composed.
    #[test]
    fn filter_and_highlight_compose() {
        let yaml = r#"
params:
  click: { select: single }
  range: { select: single }
plot:
  - mark: dot
    data: { from: t, filterBy: $click }
    x: a
    y: b
  - select: highlight
    by: $range
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;
        let selections = vec![
            (
                "click".to_string(),
                vec![("root/c".to_string(), Predicate::Expr("a = 1".to_string()))],
            ),
            (
                "range".to_string(),
                vec![("root/r".to_string(), Predicate::Expr("b > 2".to_string()))],
            ),
        ];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            emitted.sql.to_uppercase().contains("WHERE"),
            "filter applied: {}",
            emitted.sql
        );
        assert!(emitted.sql.contains("a = 1"), "filter predicate");
        assert!(
            emitted.sql.contains(SELECTED_COLUMN),
            "highlight projection: {}",
            emitted.sql
        );
        assert!(emitted.sql.contains("b > 2"), "highlight predicate");
    }

    /// an aggregate mark (heatmap) is guarded — no membership
    /// projection is appended even with an active selection, so the query can't
    /// reference a grouped-away column and SQL-error.
    #[test]
    fn emit_skips_projection_for_aggregate() {
        let yaml = r#"
params:
  brush: { select: single }
plot:
  - mark: heatmap
    data: { from: t }
    x: a
    y: b
  - select: highlight
    by: $brush
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;
        let selections = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("a > 1".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            !emitted.sql.contains(SELECTED_COLUMN),
            "aggregate plan is guarded out of the projection: {}",
            emitted.sql
        );
    }

    /// A BINNED RECT aggregates, and is a honouring family, so its highlight
    /// arrives as a per-GROUP count of selected rows rather than a per-row
    /// boolean — summed as a boolean inside the aggregate, over a column the
    /// mark does not group by.
    #[test]
    fn aggregating_honouring_mark_counts_the_selected_rows_per_group() {
        let yaml = r#"
params:
  brush: { select: single }
plot:
  - mark: rectY
    data: { from: t }
    x: { bin: power }
    y: { count: }
  - select: highlight
    by: $brush
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;
        let selections = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("temp > 1".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            emitted.sql.contains(&format!(
                "SUM(CAST((temp > 1) AS INT)) AS DOUBLE) AS {SELECTED_COUNT_COLUMN}"
            )),
            "the selected count is summed as a boolean inside the aggregate: {}",
            emitted.sql
        );
        // The predicate is inside the aggregate, never a WHERE: the group keeps
        // its full count, so the bar drawn from it is still the whole.
        assert!(
            !emitted.sql.to_uppercase().contains("WHERE (TEMP > 1)"),
            "highlight must not filter the rows it counts: {}",
            emitted.sql
        );
        // And no per-row boolean: `__bf_selected_count` contains that name as a
        // prefix, so the absence has to be asked with the counts removed first.
        assert!(
            !emitted
                .sql
                .replace(SELECTED_COUNT_COLUMN, "")
                .contains(SELECTED_COLUMN),
            "no per-row membership boolean on a grouped plan: {}",
            emitted.sql
        );
    }

    /// The same binned rect, at rest, emits SQL byte-identical to the plot
    /// without any highlight interactor.
    #[test]
    fn an_aggregating_mark_at_rest_is_byte_identical() {
        let with = r#"
params:
  brush: { select: single }
plot:
  - mark: rectY
    data: { from: t }
    x: { bin: power }
    y: { count: }
  - select: highlight
    by: $brush
"#;
        let without = r#"
plot:
  - mark: rectY
    data: { from: t }
    x: { bin: power }
    y: { count: }
"#;
        let a = parse_spec(with, Format::Yaml).unwrap().spec;
        let b = parse_spec(without, Format::Yaml).unwrap().spec;
        assert_eq!(
            emit_query(&a, 0, None, None).expect("emit").sql,
            emit_query(&b, 0, None, None).expect("emit").sql,
            "at rest, highlight is invisible in an aggregating mark's SQL too"
        );
    }

    /// FIX A: a `by:` selection created ONLY by an `as:` binding and
    /// never declared in `params:` (weather's `$range` shape) still projects the
    /// membership column — the emit gate must not require a `spec.params` entry.
    #[test]
    fn asbound_only_selection_projects() {
        let yaml = r#"
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: intervalX
    as: $range
  - select: highlight
    by: $range
    fill: '#ccc'
    fillOpacity: 0.2
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;
        assert!(
            spec.params.get("range").is_none(),
            "range is as-bound only — not in params"
        );
        let selections = vec![(
            "range".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Expr("a > 1".to_string()),
            )],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            emitted.sql.contains(SELECTED_COLUMN),
            "an as-bound-only highlight selection still projects: {}",
            emitted.sql
        );
        assert!(emitted.sql.contains("a > 1"));
    }

    /// FIX B: highlight does NOT self-exclude — a plot that BRUSHES and
    /// HIGHLIGHTS the same crossfilter selection must still dim its OWN rows. With
    /// the mark's own plot as the contributor, the crossfilter self-exclusion that
    /// is correct for filterBy would (wrongly) drop it to empty → no projection.
    #[test]
    fn highlight_does_not_self_exclude() {
        let yaml = r#"
params:
  sel: { select: crossfilter }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: intervalX
    as: $sel
  - select: highlight
    by: $sel
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;
        // Contributor == the mark's OWN plot node ("root") — the self-source that
        // filterBy would exclude.
        let selections = vec![(
            "sel".to_string(),
            vec![("root".to_string(), Predicate::Expr("a > 1".to_string()))],
        )];
        let emitted = emit_query(&spec, 0, None, Some(&selections)).expect("emit");
        assert!(
            emitted.sql.contains(SELECTED_COLUMN),
            "highlight includes the mark's own plot contribution (no self-exclusion): {}",
            emitted.sql
        );
        assert!(emitted.sql.contains("a > 1"));
    }

    // -----------------------------------------------------------------------
    // Structured clause equivalence — full emission path
    // -----------------------------------------------------------------------

    /// Swapping a selection contributor's string-form predicate for its
    /// structured Interval emits byte-identical SQL through the FULL path
    /// (compile_selection → lower → passes → render): structure (and its
    /// scale/pixel metadata) rides along without changing one emitted byte.
    #[test]
    fn structured_interval_selection_emits_identical_sql() {
        use crate::ir::{ClauseMeta, ScalarValue, ScaleDescriptor};

        let yaml = r#"
params:
  brush: { select: intersect }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;

        let string_selections: Vec<SelectionPredicate> = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::And(vec![
                    Predicate::Expr("x >= 3".to_string()),
                    Predicate::Expr("x <= 7".to_string()),
                ]),
            )],
        )];
        let structured_selections: Vec<SelectionPredicate> = vec![(
            "brush".to_string(),
            vec![(
                "root/other".to_string(),
                Predicate::Interval {
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
                },
            )],
        )];

        let from_string = emit_query(&spec, 0, None, Some(&string_selections)).expect("emit");
        let from_structured =
            emit_query(&spec, 0, None, Some(&structured_selections)).expect("emit");
        assert_eq!(
            from_structured.sql, from_string.sql,
            "structured Interval and its string form emit byte-identical SQL"
        );
        assert_eq!(from_structured.bindings, from_string.bindings);
        assert!(from_string.sql.contains("x >= 3"), "{}", from_string.sql);
    }

    /// Same equivalence for the structured Point clause: the single-value and
    /// union forms each emit byte-identically to their string counterparts.
    #[test]
    fn structured_point_selection_emits_identical_sql() {
        use crate::ir::ScalarValue;

        let yaml = r#"
params:
  click: { select: single }
plot:
  - mark: dot
    data: { from: t, filterBy: $click }
    x: x
    y: y
"#;
        let spec = parse_spec(yaml, Format::Yaml).unwrap().spec;

        let cases: Vec<(Predicate, Predicate)> = vec![
            (
                Predicate::Expr("sport = 'Athletics'".to_string()),
                Predicate::Point {
                    column: "sport".to_string(),
                    values: vec![ScalarValue::Text("Athletics".to_string())],
                    meta: None,
                },
            ),
            (
                Predicate::Or(vec![
                    Predicate::Expr("sport = 'Athletics'".to_string()),
                    Predicate::Expr("sport = 'Rowing'".to_string()),
                ]),
                Predicate::Point {
                    column: "sport".to_string(),
                    values: vec![
                        ScalarValue::Text("Athletics".to_string()),
                        ScalarValue::Text("Rowing".to_string()),
                    ],
                    meta: None,
                },
            ),
        ];
        for (string_form, structured) in cases {
            let sel = |p: Predicate| -> Vec<SelectionPredicate> {
                vec![("click".to_string(), vec![("root/other".to_string(), p)])]
            };
            let s1 = emit_query(&spec, 0, None, Some(&sel(string_form))).expect("emit");
            let s2 = emit_query(&spec, 0, None, Some(&sel(structured))).expect("emit");
            assert_eq!(
                s2.sql, s1.sql,
                "structured Point and its string form emit byte-identical SQL"
            );
        }
    }
}
