//! Static analysis of param dependencies and subscriber relationships.
//!
//! Pure functions of a parsed [`Spec`] — no I/O, no DuckDB.
//! Produces a [`SpecAnalysis`] containing the subscriber graph, dependency DAG,
//! topological order, and diagnostic warnings.

use std::collections::{HashMap, HashSet, VecDeque};

use indexmap::IndexMap;

use crate::ast::{
    Component, Input, Interactor, Mark, ParamNode, ParamRef, Spec, SpecValue,
    ValueOrParamRef,
};
use crate::error::ParseError;
use crate::parse::ParseWarning;
use crate::vocab::{InputKind, InteractorKind, MarkKind};

// ---------------------------------------------------------------------------
// Type enums (ac-08)
// ---------------------------------------------------------------------------

/// Classification of an input widget's output type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetOutputType {
    ScalarNumeric,
    ScalarString,
    Selection,
    ArrayString,
}

impl WidgetOutputType {
    /// Derive the output type from an `InputKind`.
    pub fn from_input_kind(kind: InputKind) -> Self {
        match kind {
            InputKind::Slider => WidgetOutputType::ScalarNumeric,
            InputKind::Menu => WidgetOutputType::ScalarString,
            InputKind::Search => WidgetOutputType::ScalarString,
            InputKind::Table => WidgetOutputType::Selection,
        }
    }
}

/// Classification of a param's declared type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamDeclaredType {
    ScalarNumeric,
    ScalarString,
    ScalarBool,
    Selection,
    Array,
}

impl ParamDeclaredType {
    /// Derive the declared type from a `ParamNode`.
    pub fn from_param_node(node: &ParamNode) -> Self {
        match node {
            ParamNode::Value(v) => match v {
                SpecValue::Integer(_) | SpecValue::Float(_) => ParamDeclaredType::ScalarNumeric,
                SpecValue::String(_) => ParamDeclaredType::ScalarString,
                SpecValue::Bool(_) => ParamDeclaredType::ScalarBool,
                SpecValue::Array(_) => ParamDeclaredType::Array,
                SpecValue::Object(_) | SpecValue::Null
                | SpecValue::Param(_) | SpecValue::Expression(_)
                | SpecValue::Aggregate { .. } => ParamDeclaredType::ScalarString,
            },
            ParamNode::Selection(_) => ParamDeclaredType::Selection,
        }
    }

    /// Whether a widget output type is provably incompatible with this
    /// declared type. Conservative: only flags clear mismatches.
    pub fn is_incompatible_with(&self, widget: WidgetOutputType) -> bool {
        match (self, widget) {
            // Slider (numeric) writing to a selection param
            (ParamDeclaredType::Selection, WidgetOutputType::ScalarNumeric) => true,
            // Slider (numeric) writing to a string param — could be valid in some cases
            // but we're conservative, so skip.
            // Table (selection) writing to a scalar param
            (ParamDeclaredType::ScalarNumeric, WidgetOutputType::Selection) => true,
            (ParamDeclaredType::ScalarString, WidgetOutputType::Selection) => true,
            (ParamDeclaredType::ScalarBool, WidgetOutputType::Selection) => true,
            // Selection param receiving a string/array — not necessarily wrong
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Subscriber graph (ac-03)
// ---------------------------------------------------------------------------

/// A component path identifying where a param is referenced.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentPath(pub String);

/// Return the longest prefix of `path` that ends in a `/plot[<digits>]`
/// segment, or `path` unchanged if no such segment exists.
///
/// Used by the runtime selection coordinator (card 0006 v2) for self-exclusion
/// identity. A mark inside `root/vconcat[0]/plot[1]/mark[dot]` belongs to the
/// plot at `root/vconcat[0]/plot[1]`; an interactor inside the same plot at
/// `root/vconcat[0]/plot[1]/interactor[intervalX]` shares the same parent
/// prefix. String equality on the result of `parent_plot` is the
/// "view's own predicate" identity rule (card 0006 v2 decision 4).
///
/// Behaviour:
/// - `parent_plot("root/vconcat[0]/plot[1]/mark[dot]")` → `"root/vconcat[0]/plot[1]"`
/// - `parent_plot("root/plot[0]/interactor[intervalX]")` → `"root/plot[0]"`
/// - `parent_plot("root/mark[dot]")` → `"root/mark[dot]"` (no plot in path → unchanged)
/// - `parent_plot("root")` → `"root"` (degenerate)
pub fn parent_plot(path: &str) -> &str {
    // Walk byte indices forward, recording the end of the last `/plot[N]`
    // segment we have seen. A plot segment is `/plot[<digits>]` followed
    // by either `/` (continuation) or end-of-string (terminal).
    //
    // Linear single-pass; no allocation; returns a &str slice into `path`.
    let bytes = path.as_bytes();
    let pat = b"/plot[";
    let mut last_end: Option<usize> = None;
    let mut i = 0;
    while i + pat.len() <= bytes.len() {
        if &bytes[i..i + pat.len()] == pat {
            // Scan digits after the `[`.
            let mut j = i + pat.len();
            let digits_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_start && j < bytes.len() && bytes[j] == b']' {
                // The plot segment runs from `i` (inclusive of the leading `/`)
                // to `j + 1` (exclusive — past the closing `]`).
                let end = j + 1;
                last_end = Some(end);
                // Skip past this segment to keep matching nested plots.
                // Plot inside plot is rare but the longest-match rule says
                // we should still pick the deepest one.
                i = end;
                continue;
            }
        }
        i += 1;
    }
    match last_end {
        Some(end) => &path[..end],
        None => path,
    }
}

/// Return the STABLE identity of the plot a component lives in: the path prefix
/// BEFORE the synthetic `/plot[i]` item segment.
///
/// Unlike [`parent_plot`] — which keeps the `/plot[i]` segment and so returns a
/// *different* string for a plot's mark (`…/plot[0]`) than for its interactor
/// (`…/plot[1]`), because `i` is the component's item-index — every component
/// inside one plot maps to the SAME `plot_node_path`. This is the identity
/// crossfilter self-exclusion must compare on: the contributor is an interactor
/// and the subscriber is a mark, and they have to agree for a plot to recognise
/// its own predicate. It equals the plot-node path `collect_plot_groups` keys
/// on (card 0006).
///
/// Behaviour:
/// - `plot_node_path("root/hconcat[0]/plot[1]/mark[dot]")` → `"root/hconcat[0]"`
/// - `plot_node_path("root/hconcat[0]/plot[0]/interactor[intervalX]")` → `"root/hconcat[0]"`
/// - `plot_node_path("root/plot[2]/mark[bar]")` → `"root"`
/// - `plot_node_path("root/mark[dot]")` → `"root/mark[dot]"` (no plot segment → unchanged)
pub fn plot_node_path(path: &str) -> &str {
    // Same digit-validated scan as `parent_plot`, but we keep the START of the
    // last `/plot[N]` segment rather than its end, and slice up to it.
    let bytes = path.as_bytes();
    let pat = b"/plot[";
    let mut last_start: Option<usize> = None;
    let mut i = 0;
    while i + pat.len() <= bytes.len() {
        if &bytes[i..i + pat.len()] == pat {
            let mut j = i + pat.len();
            let digits_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_start && j < bytes.len() && bytes[j] == b']' {
                last_start = Some(i);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    match last_start {
        Some(start) => &path[..start],
        None => path,
    }
}

/// Map from param name to the set of component paths that consume it.
pub type SubscriberGraph = HashMap<String, Vec<ComponentPath>>;

/// Build the subscriber graph from a parsed Spec.
pub fn build_subscriber_graph(spec: &Spec) -> SubscriberGraph {
    let mut graph: SubscriberGraph = HashMap::new();

    // Seed all declared params so dead params appear with empty vecs.
    for name in spec.params.keys() {
        graph.entry(name.clone()).or_default();
    }

    // Walk data sources for param refs in expressions.
    for (ds_name, ds) in &spec.data {
        // DataSource doesn't contain direct param refs in the current AST,
        // but future extensions might. Skip for now.
        let _ = (ds_name, ds);
    }

    // Walk the component tree.
    if let Some(root) = &spec.root {
        collect_subscribers(root, "root", &mut graph);
    }

    graph
}

fn collect_subscribers(component: &Component, path: &str, graph: &mut SubscriberGraph) {
    match component {
        Component::Mark(m) => {
            collect_mark_subscribers(m, path, graph);
        }
        Component::Input(inp) => {
            collect_input_subscribers(inp, path, graph);
        }
        Component::Interactor(i) => {
            // Interactor `as:` writes to a selection, which is a param.
            if let Some(ValueOrParamRef::Param(pr)) = i.options.get("as") {
                graph
                    .entry(pr.0.clone())
                    .or_default()
                    .push(ComponentPath(format!("{path}/interactor")));
            }
            // filterBy on interactor
            if let Some(ValueOrParamRef::Param(pr)) = i.options.get("filterBy") {
                graph
                    .entry(pr.0.clone())
                    .or_default()
                    .push(ComponentPath(format!("{path}/interactor")));
            }
        }
        Component::Plot(p) => {
            for (i, item) in p.items.iter().enumerate() {
                collect_subscribers(item, &format!("{path}/plot[{i}]"), graph);
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_subscribers(item, &format!("{path}/hconcat[{i}]"), graph);
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_subscribers(item, &format!("{path}/vconcat[{i}]"), graph);
            }
        }
        Component::Legend(l) => {
            for (k, v) in &l.options {
                // `as: $sel` on a legend is a selection PRODUCER binding
                // (card 0009, legend click-to-filter), not a subscription —
                // registering it here wired the legend backwards, making
                // `$sel` look consumed by the very legend that writes it.
                // Producer bindings surface via [`build_legend_bindings`];
                // any other param ref in a legend option keeps its
                // subscriber semantics.
                if k == "as" {
                    continue;
                }
                if let ValueOrParamRef::Param(pr) = v {
                    graph
                        .entry(pr.0.clone())
                        .or_default()
                        .push(ComponentPath(format!("{path}/legend")));
                }
            }
        }
        Component::HSpace(_) | Component::VSpace(_) => {}
    }
}

fn collect_mark_subscribers(mark: &Mark, path: &str, graph: &mut SubscriberGraph) {
    let mark_path = format!("{path}/mark[{}]", mark.kind.wire_name());

    // Mark data filterBy + any params in the `filter` expression (which lowers
    // to a WHERE — card 0014). Only the `filter` extra affects the emitted query,
    // so subscribing on other extras would trigger re-executions that change
    // nothing; scope the walk to `filter`.
    if let Some(ref data) = mark.data {
        if let crate::ast::MarkData::From {
            filter_by, extras, ..
        } = data
        {
            if let Some(pr) = filter_by {
                graph
                    .entry(pr.0.clone())
                    .or_default()
                    .push(ComponentPath(mark_path.clone()));
            }
            if let Some(filter) = extras.get("filter") {
                collect_spec_value_subscribers(filter, &mark_path, graph);
            }
        }
    }

    // Mark options — scan for param refs in expressions and direct refs.
    for v in mark.options.values() {
        collect_value_or_param_ref_subscribers(v, &mark_path, graph);
    }
}

fn collect_input_subscribers(inp: &Input, path: &str, graph: &mut SubscriberGraph) {
    let input_path = format!("{path}/input[{}]", inp.kind.wire_name());

    // filter_by consumes a param
    if let Some(ref pr) = inp.filter_by {
        graph
            .entry(pr.0.clone())
            .or_default()
            .push(ComponentPath(input_path.clone()));
    }

    // Remaining options may contain param refs
    for v in inp.options.values() {
        collect_value_or_param_ref_subscribers(v, &input_path, graph);
    }
}

fn collect_value_or_param_ref_subscribers(
    v: &ValueOrParamRef<SpecValue>,
    path: &str,
    graph: &mut SubscriberGraph,
) {
    match v {
        ValueOrParamRef::Param(pr) => {
            graph
                .entry(pr.0.clone())
                .or_default()
                .push(ComponentPath(path.to_string()));
        }
        ValueOrParamRef::Value(sv) => {
            collect_spec_value_subscribers(sv, path, graph);
        }
    }
}

fn collect_spec_value_subscribers(sv: &SpecValue, path: &str, graph: &mut SubscriberGraph) {
    match sv {
        SpecValue::Object(map) => {
            for v in map.values() {
                collect_spec_value_subscribers(v, path, graph);
            }
        }
        SpecValue::Array(arr) => {
            for v in arr {
                collect_spec_value_subscribers(v, path, graph);
            }
        }
        // A param referenced directly, or embedded in a SQL expression (e.g. a
        // `filter: "x > $k"` or an expression channel), subscribes the owning
        // component to that param (card 0014, ac-06) — so a data-shape param
        // change re-executes the mark.
        SpecValue::Param(pr) => {
            graph
                .entry(pr.0.clone())
                .or_default()
                .push(ComponentPath(path.to_string()));
        }
        SpecValue::Expression(e) => {
            for pr in &e.params {
                graph
                    .entry(pr.0.clone())
                    .or_default()
                    .push(ComponentPath(path.to_string()));
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Dependency DAG (ac-05, ac-06)
// ---------------------------------------------------------------------------

/// A directed edge in the param dependency DAG: consuming `from` produces `to`
/// via an intermediate component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamEdge {
    pub from: String,
    pub to: String,
}

/// Build the dependency DAG and return a topological order.
/// Returns Err if a cycle is detected.
pub fn build_dependency_dag(
    spec: &Spec,
) -> Result<(Vec<ParamEdge>, Vec<String>), ParseError> {
    let mut edges: Vec<ParamEdge> = Vec::new();

    // Find input widgets that both consume and produce params.
    if let Some(root) = &spec.root {
        collect_dag_edges(root, &mut edges);
    }

    // Build adjacency list for topological sort.
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    // Seed all declared params.
    for name in spec.params.keys() {
        adj.entry(name.clone()).or_default();
        in_degree.entry(name.clone()).or_insert(0);
    }

    for edge in &edges {
        adj.entry(edge.from.clone()).or_default().push(edge.to.clone());
        *in_degree.entry(edge.to.clone()).or_insert(0) += 1;
        in_degree.entry(edge.from.clone()).or_insert(0);
    }

    // Kahn's algorithm for topological sort.
    let mut queue: VecDeque<String> = VecDeque::new();
    for (name, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(name.clone());
        }
    }

    let mut order: Vec<String> = Vec::new();
    while let Some(node) = queue.pop_front() {
        order.push(node.clone());
        if let Some(neighbors) = adj.get(&node) {
            for neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }
    }

    if order.len() != in_degree.len() {
        // Cycle detected — find the participants.
        let ordered_set: HashSet<&String> = order.iter().collect();
        let cycle_participants: Vec<String> = in_degree
            .keys()
            .filter(|k| !ordered_set.contains(k))
            .cloned()
            .collect();
        return Err(ParseError::SchemaViolation {
            path: "params".into(),
            detail: format!(
                "circular param dependency detected among: {}",
                cycle_participants.join(", ")
            ),
            span: None,
        });
    }

    Ok((edges, order))
}

/// Topological order of `root` and all its descendants in the param
/// dependency DAG. Returns a vec with `root` as the first element followed by
/// every transitively-downstream param, in an order that places parents
/// before children.
///
/// Used by [`crate::ast::Spec`] consumers (the runtime coordinator) when a
/// param-change event fires: walking this order re-executes subscribing
/// marks at every level, so a change to a root param flows through chained
/// `filterBy`/`as_param` widgets in their declared dependency order.
///
/// Behaviour:
/// - Root is always included as the first element, even when it has no
///   outgoing edges (a leaf root returns `vec![root]`).
/// - The return order is a topological sort of the *descendant subgraph*,
///   not a slice of `analysis.topological_order` — this keeps the function
///   well-defined when the DAG contains other roots whose own descendants
///   precede `root` globally but should not appear in this walk.
/// - The DAG is acyclic by construction (cycles are rejected by
///   [`build_dependency_dag`] / [`analyse_spec`]); we use Kahn's algorithm
///   over the descendant subgraph for determinism.
/// - Self-edges are impossible by construction (`collect_dag_edges` skips
///   them — see `wnba-shots.yaml`).
///
/// rpw3 ac-01, ac-02, ac-03.
#[must_use]
pub fn topological_descendants(analysis: &SpecAnalysis, root: &str) -> Vec<String> {
    // Build adjacency over `analysis.dependency_edges` keyed by `from`.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &analysis.dependency_edges {
        adj.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }

    // Reachability sweep from `root` to delineate the descendant subgraph.
    // Iterative DFS; a node is added to `descendants` only on first visit.
    let mut descendants: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = vec![root];
    while let Some(node) = stack.pop() {
        if !descendants.insert(node) {
            continue;
        }
        if let Some(children) = adj.get(node) {
            for &child in children {
                if !descendants.contains(child) {
                    stack.push(child);
                }
            }
        }
    }

    // If `root` has no DAG presence at all (not in any edge), still include it.
    if descendants.is_empty() {
        return vec![root.to_string()];
    }

    // In-degree restricted to the descendant subgraph.
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for &node in &descendants {
        in_degree.insert(node, 0);
    }
    for edge in &analysis.dependency_edges {
        if descendants.contains(edge.from.as_str())
            && descendants.contains(edge.to.as_str())
        {
            *in_degree.entry(edge.to.as_str()).or_insert(0) += 1;
        }
    }

    // Kahn's algorithm seeded with `root` first so it always appears at
    // index 0 (even if other descendant nodes also have in-degree 0,
    // which can only happen if they are unreachable from `root` — and
    // by construction `descendants` only contains reachable nodes).
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(root);
    // Other zero-in-degree nodes inside the subgraph (none, by construction
    // of reachability) — guarded loop below for robustness.
    for (&node, &deg) in &in_degree {
        if deg == 0 && node != root {
            queue.push_back(node);
        }
    }

    let mut order: Vec<String> = Vec::with_capacity(descendants.len());
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node) {
            continue;
        }
        order.push(node.to_string());
        if let Some(children) = adj.get(node) {
            for &child in children {
                if !descendants.contains(child) {
                    continue;
                }
                if let Some(deg) = in_degree.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
    }

    order
}

fn collect_dag_edges(component: &Component, edges: &mut Vec<ParamEdge>) {
    match component {
        Component::Input(inp) => {
            // An input that has as_param (writes to) and also consumes params
            // via filter_by or from creates a dependency edge.
            // Self-referential edges (filter_by == as_param) are skipped — a
            // widget that filters itself by the selection it contributes to is
            // valid Mosaic (e.g. wnba-shots.yaml).
            if let Some(ref target) = inp.as_param {
                if let Some(ref source) = inp.filter_by {
                    if source.0 != target.0 {
                        edges.push(ParamEdge {
                            from: source.0.clone(),
                            to: target.0.clone(),
                        });
                    }
                }
                // Also check options for param refs that create dependencies.
                for v in inp.options.values() {
                    if let ValueOrParamRef::Param(pr) = v {
                        if pr.0 != target.0 {
                            edges.push(ParamEdge {
                                from: pr.0.clone(),
                                to: target.0.clone(),
                            });
                        }
                    }
                }
            }
        }
        Component::Plot(p) => {
            for item in &p.items {
                collect_dag_edges(item, edges);
            }
        }
        Component::HConcat(c) | Component::VConcat(c) => {
            for item in &c.items {
                collect_dag_edges(item, edges);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Type checking (ac-07, ac-08)
// ---------------------------------------------------------------------------

/// Check for type mismatches between input widgets and their target params.
pub fn check_param_type_mismatches(
    spec: &Spec,
) -> Vec<ParseWarning> {
    let mut warnings = Vec::new();
    if let Some(root) = &spec.root {
        check_type_mismatches_in(root, &spec.params, &mut warnings);
    }
    warnings
}

fn check_type_mismatches_in(
    component: &Component,
    params: &IndexMap<String, ParamNode>,
    warnings: &mut Vec<ParseWarning>,
) {
    match component {
        Component::Input(inp) => {
            if let Some(ref pr) = inp.as_param {
                if let Some(param_node) = params.get(&pr.0) {
                    let widget_type = WidgetOutputType::from_input_kind(inp.kind);
                    let param_type = ParamDeclaredType::from_param_node(param_node);
                    if param_type.is_incompatible_with(widget_type) {
                        warnings.push(ParseWarning::ParamTypeMismatch {
                            param: pr.0.clone(),
                            expected: format!("{param_type:?}"),
                            widget_kind: inp.kind.wire_name().to_string(),
                        });
                    }
                }
            }
        }
        Component::Plot(p) => {
            for item in &p.items {
                check_type_mismatches_in(item, params, warnings);
            }
        }
        Component::HConcat(c) | Component::VConcat(c) => {
            for item in &c.items {
                check_type_mismatches_in(item, params, warnings);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// filterBy validation (cfs ac-01..ac-04)
// ---------------------------------------------------------------------------

/// Collect all param/selection names that are created by `as:` bindings
/// on interactors and input widgets (implicit param creation in Mosaic).
fn collect_as_bound_names(spec: &Spec) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(root) = &spec.root {
        collect_as_names(root, &mut names);
    }
    names
}

fn collect_as_names(component: &Component, names: &mut HashSet<String>) {
    match component {
        Component::Interactor(i) => {
            if let Some(ValueOrParamRef::Param(pr)) = i.options.get("as") {
                names.insert(pr.0.clone());
            }
        }
        Component::Input(inp) => {
            if let Some(ref pr) = inp.as_param {
                names.insert(pr.0.clone());
            }
        }
        Component::Plot(p) => {
            for item in &p.items {
                collect_as_names(item, names);
            }
        }
        Component::HConcat(c) | Component::VConcat(c) => {
            for item in &c.items {
                collect_as_names(item, names);
            }
        }
        _ => {}
    }
}

/// Validate that every `filterBy` reference in the component tree points to
/// a known selection — either declared in `params:` as a Selection, or
/// implicitly created by an interactor/input's `as:` binding.
///
/// Returns `Err` on the first violation (missing or non-selection ref).
pub fn validate_filter_by_refs(spec: &Spec) -> Result<(), ParseError> {
    // Collect all known selection names: declared selections + as:-bound names.
    let mut known_selections: HashSet<String> = HashSet::new();
    for (name, node) in &spec.params {
        if matches!(node, ParamNode::Selection(_)) {
            known_selections.insert(name.clone());
        }
    }
    // Interactors and inputs create implicit params/selections via `as:`.
    known_selections.extend(collect_as_bound_names(spec));

    if let Some(root) = &spec.root {
        validate_filter_by_in(root, &spec.params, &known_selections, "root")?;
    }
    Ok(())
}

fn validate_filter_by_in(
    component: &Component,
    params: &IndexMap<String, ParamNode>,
    known_selections: &HashSet<String>,
    path: &str,
) -> Result<(), ParseError> {
    match component {
        Component::Mark(m) => {
            if let Some(ref data) = m.data {
                if let crate::ast::MarkData::From { filter_by, .. } = data {
                    if let Some(pr) = filter_by {
                        check_filter_by_ref(&pr.0, params, known_selections, &format!("{path}/mark[{}].data.filterBy", m.kind.wire_name()))?;
                    }
                }
            }
            // Also check filterBy in mark options (direct mark-level filterBy)
            if let Some(ValueOrParamRef::Param(pr)) = m.options.get("filterBy") {
                check_filter_by_ref(&pr.0, params, known_selections, &format!("{path}/mark[{}].filterBy", m.kind.wire_name()))?;
            }
        }
        Component::Input(inp) => {
            if let Some(ref pr) = inp.filter_by {
                check_filter_by_ref(&pr.0, params, known_selections, &format!("{path}/input[{}].filterBy", inp.kind.wire_name()))?;
            }
        }
        Component::Interactor(_) => {
            // Interactors don't have filterBy in the standard model
        }
        Component::Plot(p) => {
            for (i, item) in p.items.iter().enumerate() {
                validate_filter_by_in(item, params, known_selections, &format!("{path}/plot[{i}]"))?;
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                validate_filter_by_in(item, params, known_selections, &format!("{path}/hconcat[{i}]"))?;
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                validate_filter_by_in(item, params, known_selections, &format!("{path}/vconcat[{i}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_filter_by_ref(
    name: &str,
    params: &IndexMap<String, ParamNode>,
    known_selections: &HashSet<String>,
    path: &str,
) -> Result<(), ParseError> {
    // Check if it's a known selection (declared or interactor-created).
    if known_selections.contains(name) {
        return Ok(());
    }
    // If it's a declared value param, that's a type error.
    if let Some(ParamNode::Value(_)) = params.get(name) {
        return Err(ParseError::SchemaViolation {
            path: path.to_string(),
            detail: format!(
                "filterBy references value param `{name}`, but filterBy requires a selection"
            ),
            span: None,
        });
    }
    // Not declared at all and not an interactor-created selection.
    Err(ParseError::SchemaViolation {
        path: path.to_string(),
        detail: format!("filterBy references undeclared param `{name}`"),
        span: None,
    })
}

// ---------------------------------------------------------------------------
// Interactor binding validation (cfs ac-05, ac-06)
// ---------------------------------------------------------------------------

/// An interactor's write binding to a named selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractorBinding {
    /// Component path of the interactor.
    pub path: ComponentPath,
    /// The selection name it writes to.
    pub selection: String,
}

/// Validate interactor `as:` bindings and collect warnings for missing or
/// non-selection targets.
pub fn validate_interactor_bindings(
    spec: &Spec,
) -> Vec<ParseWarning> {
    let mut warnings = Vec::new();
    if let Some(root) = &spec.root {
        validate_interactor_bindings_in(root, &spec.params, "root", &mut warnings);
    }
    warnings
}

fn validate_interactor_bindings_in(
    component: &Component,
    params: &IndexMap<String, ParamNode>,
    path: &str,
    warnings: &mut Vec<ParseWarning>,
) {
    match component {
        Component::Interactor(i) => {
            if let Some(ValueOrParamRef::Param(pr)) = i.options.get("as") {
                match params.get(&pr.0) {
                    None => {
                        warnings.push(ParseWarning::InteractorBindingMissing {
                            name: pr.0.clone(),
                        });
                    }
                    Some(ParamNode::Value(_)) => {
                        warnings.push(ParseWarning::InteractorBindingNonSelection {
                            name: pr.0.clone(),
                        });
                    }
                    Some(ParamNode::Selection(_)) => {} // valid
                }
            }
        }
        Component::Plot(p) => {
            for (i, item) in p.items.iter().enumerate() {
                validate_interactor_bindings_in(item, params, &format!("{path}/plot[{i}]"), warnings);
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                validate_interactor_bindings_in(item, params, &format!("{path}/hconcat[{i}]"), warnings);
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                validate_interactor_bindings_in(item, params, &format!("{path}/vconcat[{i}]"), warnings);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Selection subscriber graph (cfs ac-07)
// ---------------------------------------------------------------------------

/// Map from selection name to component paths that subscribe via filterBy.
pub type SelectionSubscriberGraph = HashMap<String, Vec<ComponentPath>>;

/// Build the selection subscriber graph — only tracks filterBy subscriptions
/// to selection params (distinct from the general subscriber graph in card 0005).
/// Recognizes both declared selections and interactor-created implicit selections.
pub fn build_selection_subscriber_graph(spec: &Spec) -> SelectionSubscriberGraph {
    let mut graph: SelectionSubscriberGraph = HashMap::new();

    // Seed with declared selections.
    for (name, node) in &spec.params {
        if matches!(node, ParamNode::Selection(_)) {
            graph.entry(name.clone()).or_default();
        }
    }

    // Seed with as:-bound names (interactors and inputs create implicit selections).
    for name in collect_as_bound_names(spec) {
        graph.entry(name).or_default();
    }

    // Walk component tree collecting filterBy refs that target known selections.
    let known_selections: HashSet<String> = graph.keys().cloned().collect();
    if let Some(root) = &spec.root {
        collect_selection_subscribers(root, "root", &known_selections, &mut graph);
        // Card 0021: a `highlight, by: $sel` interactor makes its plot's
        // honouring marks subscribers to `$sel` too — a selection change must
        // re-query them (to re-project `__bf_selected`), not just the filterBy
        // subscribers. Registered after the filterBy walk so both sets compose.
        collect_highlight_subscribers(root, "root", &known_selections, &mut graph);
    }

    graph
}

fn collect_selection_subscribers(
    component: &Component,
    path: &str,
    known_selections: &HashSet<String>,
    graph: &mut SelectionSubscriberGraph,
) {
    match component {
        Component::Mark(m) => {
            let mark_path = format!("{path}/mark[{}]", m.kind.wire_name());
            // Mark data filterBy
            if let Some(ref data) = m.data {
                if let crate::ast::MarkData::From { filter_by, .. } = data {
                    if let Some(pr) = filter_by {
                        if known_selections.contains(&pr.0) {
                            graph
                                .entry(pr.0.clone())
                                .or_default()
                                .push(ComponentPath(mark_path.clone()));
                        }
                    }
                }
            }
            // Direct mark-level filterBy
            if let Some(ValueOrParamRef::Param(pr)) = m.options.get("filterBy") {
                if known_selections.contains(&pr.0) {
                    graph
                        .entry(pr.0.clone())
                        .or_default()
                        .push(ComponentPath(mark_path));
                }
            }
        }
        Component::Input(inp) => {
            if let Some(ref pr) = inp.filter_by {
                if known_selections.contains(&pr.0) {
                    graph
                        .entry(pr.0.clone())
                        .or_default()
                        .push(ComponentPath(format!("{path}/input[{}]", inp.kind.wire_name())));
                }
            }
        }
        Component::Plot(p) => {
            for (i, item) in p.items.iter().enumerate() {
                collect_selection_subscribers(item, &format!("{path}/plot[{i}]"), known_selections, graph);
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_selection_subscribers(item, &format!("{path}/hconcat[{i}]"), known_selections, graph);
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_selection_subscribers(item, &format!("{path}/vconcat[{i}]"), known_selections, graph);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Interactor bindings graph (cfs ac-08)
// ---------------------------------------------------------------------------

/// Build the list of interactor-to-selection bindings.
pub fn build_interactor_bindings(spec: &Spec) -> Vec<InteractorBinding> {
    let mut bindings = Vec::new();
    if let Some(root) = &spec.root {
        collect_interactor_bindings(root, "root", &mut bindings);
    }
    bindings
}

fn collect_interactor_bindings(
    component: &Component,
    path: &str,
    bindings: &mut Vec<InteractorBinding>,
) {
    match component {
        Component::Interactor(i) => {
            if let Some(ValueOrParamRef::Param(pr)) = i.options.get("as") {
                bindings.push(InteractorBinding {
                    path: ComponentPath(format!("{path}/interactor[{}]", i.kind.wire_name())),
                    selection: pr.0.clone(),
                });
            }
        }
        Component::Plot(p) => {
            for (i, item) in p.items.iter().enumerate() {
                collect_interactor_bindings(item, &format!("{path}/plot[{i}]"), bindings);
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_interactor_bindings(item, &format!("{path}/hconcat[{i}]"), bindings);
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_interactor_bindings(item, &format!("{path}/vconcat[{i}]"), bindings);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Brushable interactor bindings (cfs3 ac-05, ac-09)
// ---------------------------------------------------------------------------

/// Mirror of `brightfield_ui::brush::BrushKind`. Lives in brightfield-spec so
/// `SpecAnalysis` can name brushable interactor variants without a reverse
/// dependency on brightfield-ui. Variant order matches the UI-side enum.
///
/// `Point` is forward-compat for input-widget-driven point selections (card
/// 0005 v3); the spec-side analysis filters it out of `brushable_bindings`
/// because no chart-side dispatch path exists for it in v3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushKind {
    /// X-only interval brush (intervalX).
    IntervalX,
    /// Y-only interval brush (intervalY).
    IntervalY,
    /// 2D interval brush (intervalXY).
    IntervalXY,
    /// Point selection (forward-compat — card 0005 v3 input-widget surface).
    Point,
    /// X-channel point selection (toggleX): `x = <clicked value>`.
    PointX,
    /// Y-channel point selection (toggleY): `y = <clicked value>`.
    PointY,
}

impl BrushKind {
    /// Map an `InteractorKind` to a brushable variant. Returns `None` for
    /// non-brushable kinds (Highlight, Nearest*, Pan*, Region, etc.).
    ///
    /// `toggleX`/`toggleY` become the single-channel point kinds. `toggle`
    /// (both axes) stays unmapped for now: its value-pair predicate producer
    /// lands with the window click-gesture increment.
    #[must_use]
    pub fn from_interactor_kind(kind: InteractorKind) -> Option<Self> {
        match kind {
            InteractorKind::IntervalX => Some(Self::IntervalX),
            InteractorKind::IntervalY => Some(Self::IntervalY),
            InteractorKind::IntervalXY => Some(Self::IntervalXY),
            InteractorKind::ToggleX => Some(Self::PointX),
            InteractorKind::ToggleY => Some(Self::PointY),
            _ => None,
        }
    }
}

/// Mirror of `brightfield_ui::brush::ChannelColumns`. The plot-resolved x/y
/// SQL column expressions that a brush's coordinates compare against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelColumns {
    /// Column expression for the x channel (the first child mark's `x:` slot).
    pub x: Option<String>,
    /// Column expression for the y channel (the first child mark's `y:` slot).
    pub y: Option<String>,
}

/// A brushable interactor binding — one entry per `(plot, brushable interactor)`
/// pair, carrying enough metadata for the UI side to construct a `BrushBinding`
/// (selection name, contributor path, kind, channel columns).
///
/// Built by [`build_brushable_bindings`] during [`analyse_spec`]. Filters out
/// non-brushable interactor kinds (Toggle, Pan*, Highlight, Nearest*, Region).
/// `interactor_bindings` (the v1 surface) is untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrushableBinding {
    /// Component path of the interactor (e.g. `root/plot[0]/interactor[intervalXY]`).
    pub interactor_path: ComponentPath,
    /// Component path of the parent plot (e.g. `root/plot[0]`).
    /// This is the contributor identity used for crossfilter self-exclusion.
    pub parent_plot: ComponentPath,
    /// Selection name the interactor writes to (`as: $brush` → `"brush"`).
    pub selection: String,
    /// Brush kind (mirror enum).
    pub kind: BrushKind,
    /// Resolved x/y channel columns from the parent plot's first child mark.
    pub channels: ChannelColumns,
}

/// Walk the spec tree and build the list of brushable interactor bindings.
/// Each `(Plot, brushable Interactor)` pair produces one entry; the parent
/// plot's first child Mark's `x:` and `y:` options become the channels.
pub fn build_brushable_bindings(spec: &Spec) -> Vec<BrushableBinding> {
    let mut bindings = Vec::new();
    if let Some(root) = &spec.root {
        collect_brushable_bindings(root, "root", &mut bindings);
    }
    bindings
}

fn collect_brushable_bindings(
    component: &Component,
    path: &str,
    bindings: &mut Vec<BrushableBinding>,
) {
    match component {
        Component::Plot(p) => {
            // Resolve channels once per plot from the first child Mark.
            // Channels are shared across all brushable interactors in this
            // plot — they describe the plot's data axes, not the interactor's.
            let channels = first_mark_channels(&p.items);
            // Mirror `collect_interactor_bindings`'s convention: each item
            // position contributes `/plot[i]` to the path, then the matched
            // Interactor branch appends `/interactor[kind]`. The synthetic
            // `plot[i]` segment is the parent-plot identity used elsewhere
            // for crossfilter self-exclusion.
            for (i, item) in p.items.iter().enumerate() {
                let item_path = format!("{path}/plot[{i}]");
                match item {
                    Component::Interactor(intc) => {
                        let Some(kind) = BrushKind::from_interactor_kind(intc.kind) else {
                            continue;
                        };
                        let Some(ValueOrParamRef::Param(pr)) = intc.options.get("as") else {
                            continue;
                        };
                        bindings.push(BrushableBinding {
                            interactor_path: ComponentPath(format!(
                                "{item_path}/interactor[{}]",
                                intc.kind.wire_name()
                            )),
                            // The contributor identity is the plot NODE path
                            // (`path`), not the interactor's item-index path
                            // (`item_path`). Self-exclusion compares this
                            // against the subscriber mark's `plot_node_path`, so
                            // both sides must use the stable plot identity —
                            // otherwise a plot's interactor (`…/plot[1]`) and its
                            // mark (`…/plot[0]`) never match and the plot filters
                            // itself. (card 0006)
                            parent_plot: ComponentPath(path.to_string()),
                            selection: pr.0.clone(),
                            kind,
                            channels: channels.clone(),
                        });
                    }
                    Component::Plot(_) | Component::HConcat(_) | Component::VConcat(_) => {
                        collect_brushable_bindings(item, &item_path, bindings);
                    }
                    _ => {}
                }
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_brushable_bindings(item, &format!("{path}/hconcat[{i}]"), bindings);
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_brushable_bindings(item, &format!("{path}/vconcat[{i}]"), bindings);
            }
        }
        _ => {}
    }
}

/// Inspect the first `Mark` child of a plot's items list and pull its `x:`
/// and `y:` channel options. Skips non-mark items (interactors, legends).
/// Returns empty `ChannelColumns` if no mark or no string-valued channels.
fn first_mark_channels(items: &[Component]) -> ChannelColumns {
    for item in items {
        if let Component::Mark(m) = item {
            return ChannelColumns {
                x: option_string(m, "x"),
                y: option_string(m, "y"),
            };
        }
    }
    ChannelColumns::default()
}

fn option_string(mark: &Mark, key: &str) -> Option<String> {
    match mark.options.get(key)? {
        ValueOrParamRef::Value(SpecValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Legend producer bindings (card 0009, lcf ac-01)
// ---------------------------------------------------------------------------

/// A legend's producer binding to a selection — one entry per standalone
/// `legend: color` node carrying `as: $sel`, resolved to its `for:` plot.
///
/// The legend is a selection PRODUCER (clicking a swatch dispatches
/// `column = 'category'`), so its contributor identity is the `for:` plot's
/// node path: string-equal to `compile_selection`'s per-mark `self_source`,
/// which gives self-exclusion by construction — the legend's own source plot
/// never filters itself, keeping its launch-time colour scale valid.
///
/// Built by [`build_legend_bindings`] during [`analyse_spec`]. Legends
/// without `as:`, non-`color` channels, and unresolvable `for:` targets
/// yield no binding (they stay display-only). The `as:` target must be a
/// DECLARED selection with `crossfilter` resolution — the only resolution
/// under which `compile_selection` self-excludes the contributor's own
/// plot; any other target (undeclared, a value param, or a
/// single/intersect/union selection) is skipped with a `LegendBinding*`
/// warning, because binding it would let the legend filter its own `for:`
/// plot and invalidate the launch-time colour-scale snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegendBinding {
    /// Component path of the legend node (e.g. `root/hconcat[1]`).
    pub legend_path: ComponentPath,
    /// Node path of the `for:` plot (e.g. `root/hconcat[0]`) — the
    /// contributor identity used for crossfilter self-exclusion.
    pub plot_path: ComponentPath,
    /// Selection name the legend writes to (`as: $sel` → `"sel"`).
    pub selection: String,
    /// The colour column category predicates compare against, resolved from
    /// the `for:` plot's first mark's `fill:` (else `stroke:`) channel.
    pub colour_column: String,
}

/// The colour column of a plot's FIRST mark child: its `fill:` channel,
/// falling back to `stroke:` (fill takes precedence, mirroring the app
/// resolver's `colour_scale_of`). String-valued options only — a literal or
/// param-bound colour channel is not a data column, so no binding forms.
fn plot_colour_column(plot: &crate::ast::PlotNode) -> Option<String> {
    for item in &plot.items {
        if let Component::Mark(m) = item {
            return option_string(m, "fill").or_else(|| option_string(m, "stroke"));
        }
    }
    None
}

/// Walk the spec and build the legend producer bindings (card 0009).
///
/// `for:`-resolution mirrors the app's legend resolver (`resolve_legends`):
/// an explicit `for: <name>` must match a colour-encoded plot's `name`
/// attribute (last-wins on a duplicate name, matching the resolver); an
/// absent `for:` falls back to the dashboard's sole colour-encoded plot; a
/// non-literal `for:` (a param) never binds. "Colour-encoded" here means the
/// plot's first mark carries a string `fill:`/`stroke:` column — the static
/// mirror of the resolver's live colour-scale check.
///
/// The `as:` target is validated against `spec.params`: only a declared
/// selection with `crossfilter` resolution binds (the self-exclusion
/// precondition — see [`LegendBinding`]); anything else skips the legend and
/// pushes a `LegendBinding*` warning onto `warnings`, mirroring the
/// interactor-binding warning family.
pub fn build_legend_bindings(spec: &Spec, warnings: &mut Vec<ParseWarning>) -> Vec<LegendBinding> {
    use crate::layout::{collect_legend_nodes, collect_plot_nodes};
    use crate::vocab::LegendChannel;

    // Every colour-encoded plot: (node path, colour column) — plus a
    // name-keyed view for `for:` lookup.
    let plots = collect_plot_nodes(spec);
    let mut colour_plots: Vec<(String, String)> = Vec::new();
    let mut by_name: HashMap<String, (String, String)> = HashMap::new();
    for (path, node) in &plots {
        let Some(column) = plot_colour_column(node) else {
            continue;
        };
        colour_plots.push((path.clone(), column.clone()));
        if let Some(SpecValue::String(name)) = node.attributes.get("name") {
            by_name.insert(name.clone(), (path.clone(), column));
        }
    }
    // The dashboard's sole colour-encoded plot — the `for:`-absent fallback.
    let sole = match colour_plots.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    };

    let mut bindings = Vec::new();
    for (legend_path, node) in collect_legend_nodes(spec) {
        // Hit-testing is scoped to categorical colour legends; opacity and
        // symbol are channel-gated (Unimplemented) and never bind.
        if node.channel != LegendChannel::Color {
            continue;
        }
        let Some(ValueOrParamRef::Param(selection)) = node.options.get("as") else {
            continue; // no `as:` — display-only, exactly as card 0016 shipped
        };
        // Self-exclusion gating: compile_selection self-excludes ONLY under
        // Crossfilter resolution, so a legend bound to any other target would
        // filter its own `for:` plot (select: single) or reference nothing.
        // Skip + warn, mirroring the interactor-binding warning family.
        match spec.params.get(&selection.0) {
            None => {
                warnings.push(ParseWarning::LegendBindingMissing {
                    name: selection.0.clone(),
                });
                continue;
            }
            Some(ParamNode::Value(_)) => {
                warnings.push(ParseWarning::LegendBindingNonSelection {
                    name: selection.0.clone(),
                });
                continue;
            }
            Some(ParamNode::Selection(sel))
                if sel.select != crate::vocab::SelectionResolution::Crossfilter =>
            {
                warnings.push(ParseWarning::LegendBindingNonCrossfilter {
                    name: selection.0.clone(),
                    resolution: sel.select.wire_name().to_string(),
                });
                continue;
            }
            Some(ParamNode::Selection(_)) => {} // crossfilter: valid
        }
        let resolved = match node.options.get("for") {
            Some(ValueOrParamRef::Value(SpecValue::String(name))) => by_name.get(name).cloned(),
            None => sole.clone(),
            Some(_) => None,
        };
        let Some((plot_path, colour_column)) = resolved else {
            continue;
        };
        bindings.push(LegendBinding {
            legend_path: ComponentPath(legend_path),
            plot_path: ComponentPath(plot_path),
            selection: selection.0.clone(),
            colour_column,
        });
    }
    bindings
}

// ---------------------------------------------------------------------------
// Highlight interactor bindings (card 0021, conditional encoding)
// ---------------------------------------------------------------------------

/// Reserved output column carrying a highlight mark's per-row membership in its
/// `by:` selection — the SQL emitter projects `(<pred>) AS __bf_selected` and the
/// renderer reads the boolean back to dim non-matching rows (card 0021). The
/// `__bf_` prefix follows the density/hexbin geometry convention so it can't
/// collide with a user column. Defined here in the shared base crate because
/// both `brightfield-sql` (writer) and `brightfield-render` (reader) reference it.
pub const SELECTED_COLUMN: &str = "__bf_selected";

/// The `otherwise` override surface a `highlight` interactor applies to the
/// NON-matching (deemphasised) rows — the flat declarative fields from Mosaic's
/// `Highlight.ts` (card 0021). Every field is optional; an all-`None` style
/// means "use the default deemphasis" (opacity 0.2). `stroke`/`stroke_opacity`
/// are MODELLED here for corpus fidelity but left unimplemented at the render
/// site (no fill-vs-stroke discriminator, no driving fixture — see the spec's
/// locked decisions).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HighlightStyle {
    /// Element opacity multiplier for deemphasised rows (Mosaic default 0.2).
    pub opacity: Option<f64>,
    /// Literal fill colour replacing the resolved colour (e.g. `#ccc`).
    pub fill: Option<String>,
    /// Fill alpha for deemphasised rows.
    pub fill_opacity: Option<f64>,
    /// Modelled, unimplemented in v1.
    pub stroke: Option<String>,
    /// Modelled, unimplemented in v1.
    pub stroke_opacity: Option<f64>,
}

/// A `highlight` interactor's binding to the selection it CONSUMES — one entry
/// per `(plot, highlight interactor)` pair carrying the selection name (from
/// `by:`, unlike the `as:` PRODUCER bindings), the parent-plot identity, and the
/// resolved `otherwise` style.
///
/// Built by [`build_highlight_bindings`] during [`analyse_spec`]. Unlike a
/// brush/legend producer, a highlight interactor writes nothing — it re-styles
/// its plot's marks per the live membership of `by:`. The membership is a
/// per-row boolean the mark's query PROJECTS (`… AS __bf_selected`) rather than
/// a WHERE that drops rows, so a highlight-bound mark keeps its FULL batch and
/// DIMS (the ce-ac05 classification: highlight-not-filter).
#[derive(Debug, Clone, PartialEq)]
pub struct HighlightBinding {
    /// Component path of the highlight interactor
    /// (e.g. `root/hconcat[0]/plot[3]/interactor[highlight]`).
    pub interactor_path: ComponentPath,
    /// The parent plot's NODE path (e.g. `root/hconcat[0]`) — the stable plot
    /// identity `emit_query` uses as `self_source` for crossfilter
    /// self-exclusion, equal to `plot_node_path` of the plot's marks.
    pub parent_plot: ComponentPath,
    /// The selection name `by:` names (`by: $brush` → `"brush"`) — a CONSUMER
    /// reference, validated against declared + `as:`-bound selection names.
    pub selection: String,
    /// The `otherwise` deemphasis style applied to non-matching rows.
    pub style: HighlightStyle,
}

/// Honouring mark families — the four whose renderer reads the highlight state
/// (`apply_highlight`) and therefore dims. Every one lowers via `SimpleLowerer`
/// (row-level `SELECT * FROM table`), so the `__bf_selected` membership
/// projection over them evaluates against the full source table and is always
/// SQL-safe (the ce-ac05 / ce-ac09 correctness anchor). The other 12 families
/// stay highlight no-ops.
pub fn mark_honours_highlight(kind: MarkKind) -> bool {
    matches!(
        kind,
        MarkKind::Dot
            | MarkKind::BarX
            | MarkKind::BarY
            | MarkKind::Rect
            | MarkKind::RectX
            | MarkKind::RectY
            | MarkKind::Text
    )
}

/// Mark kinds whose lowering AGGREGATES in SQL (GROUP BY / scalar aggregate),
/// so a `SELECT *, (<pred>) AS __bf_selected` wrapper would evaluate the
/// predicate against the grouped output — a non-group-key column reference would
/// SQL-error. None of these is a honouring family, so guarding them out costs no
/// visible dimming; it only prevents a runtime crash (ce-ac09). Kept
/// deliberately conservative (Cell aggregates only under a self-aggregating
/// fill, but is guarded unconditionally — it never dims regardless).
fn mark_kind_aggregates(kind: MarkKind) -> bool {
    matches!(
        kind,
        MarkKind::Density
            | MarkKind::DensityX
            | MarkKind::DensityY
            | MarkKind::Heatmap
            | MarkKind::Contour
            | MarkKind::Raster
            | MarkKind::Cell
            | MarkKind::Hexbin
            | MarkKind::RegressionX
            | MarkKind::RegressionY
    )
}

/// The selection a plot's `highlight` interactor CONSUMES (`by: $sel`), if the
/// plot carries one and `by:` lifted to a `Param` ref. Pure structural scan of a
/// plot's items — no validation (that happens in [`build_highlight_bindings`]).
fn plot_highlight_by(items: &[Component]) -> Option<&ParamRef> {
    for item in items {
        if let Component::Interactor(i) = item {
            if i.kind == InteractorKind::Highlight {
                if let Some(ValueOrParamRef::Param(pr)) = i.options.get("by") {
                    return Some(pr);
                }
            }
        }
    }
    None
}

/// Extract a literal numeric option from an interactor's option bag.
fn interactor_opt_f64(interactor: &Interactor, key: &str) -> Option<f64> {
    match interactor.options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Float(f)) => Some(*f),
        ValueOrParamRef::Value(SpecValue::Integer(i)) => Some(*i as f64),
        _ => None,
    }
}

/// Extract a literal string option from an interactor's option bag.
fn interactor_opt_string(interactor: &Interactor, key: &str) -> Option<String> {
    match interactor.options.get(key)? {
        ValueOrParamRef::Value(SpecValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Resolve a `highlight` interactor's `otherwise` override style from its option
/// bag: `opacity` / `fillOpacity` (numeric), `fill` / `stroke` (literal
/// colour), `strokeOpacity` (numeric). Absent fields stay `None`; the render
/// site applies the Mosaic default (opacity 0.2) when every field is `None`.
fn resolve_highlight_style(interactor: &Interactor) -> HighlightStyle {
    HighlightStyle {
        opacity: interactor_opt_f64(interactor, "opacity"),
        fill: interactor_opt_string(interactor, "fill"),
        fill_opacity: interactor_opt_f64(interactor, "fillOpacity"),
        stroke: interactor_opt_string(interactor, "stroke"),
        stroke_opacity: interactor_opt_f64(interactor, "strokeOpacity"),
    }
}

/// Walk the spec and build the highlight consumer bindings (card 0021).
///
/// One entry per `(Plot, highlight Interactor)` pair whose `by:` resolves to a
/// known selection (declared in `params:` OR created by an `as:` binding),
/// mirroring [`validate_filter_by_refs`]'s known-selection set. An unknown or
/// value-param `by:` pushes a `HighlightBinding*` warning and forms no binding.
/// A highlight on a plot whose data mark AGGREGATES in SQL pushes
/// `HighlightOnAggregate` and forms no binding (ce-ac09 guard) — the row-level
/// honouring families (dot/bar/rect/text) are unaffected.
pub fn build_highlight_bindings(
    spec: &Spec,
    warnings: &mut Vec<ParseWarning>,
) -> Vec<HighlightBinding> {
    // Known selections: declared Selection params + `as:`-bound names — the same
    // set filterBy validation trusts.
    let mut known_selections: HashSet<String> = HashSet::new();
    for (name, node) in &spec.params {
        if matches!(node, ParamNode::Selection(_)) {
            known_selections.insert(name.clone());
        }
    }
    known_selections.extend(collect_as_bound_names(spec));

    let mut bindings = Vec::new();
    if let Some(root) = &spec.root {
        collect_highlight_bindings(root, "root", spec, &known_selections, &mut bindings, warnings);
    }
    bindings
}

fn collect_highlight_bindings(
    component: &Component,
    path: &str,
    spec: &Spec,
    known_selections: &HashSet<String>,
    bindings: &mut Vec<HighlightBinding>,
    warnings: &mut Vec<ParseWarning>,
) {
    match component {
        Component::Plot(p) => {
            // The plot's node path is `path`; its items are `path/plot[i]`.
            for (i, item) in p.items.iter().enumerate() {
                let item_path = format!("{path}/plot[{i}]");
                match item {
                    Component::Interactor(intc)
                        if intc.kind == InteractorKind::Highlight =>
                    {
                        let Some(ValueOrParamRef::Param(pr)) = intc.options.get("by") else {
                            // A highlight with no `by:` selection has nothing to
                            // dim against — inert, no binding (no warning: an
                            // author may be mid-edit).
                            continue;
                        };
                        // Validate the CONSUMER ref like filterBy: declared
                        // selection or `as:`-bound, else warn + skip.
                        if !known_selections.contains(&pr.0) {
                            match spec.params.get(&pr.0) {
                                Some(ParamNode::Value(_)) => {
                                    warnings.push(ParseWarning::HighlightBindingNonSelection {
                                        name: pr.0.clone(),
                                    });
                                }
                                _ => {
                                    warnings.push(ParseWarning::HighlightBindingMissing {
                                        name: pr.0.clone(),
                                    });
                                }
                            }
                            continue;
                        }
                        // Aggregate guard (ce-ac09) — PER MARK, matching emit's
                        // per-plan `plan_aggregates` guard. An aggregating mark
                        // (density/cell/hexbin/…) can't carry the membership
                        // projection (it evaluates against grouped output), so it
                        // is warned about and never dims. But it must NOT veto a
                        // sibling honouring mark's highlight: warn per aggregate
                        // mark, then form the binding IFF the plot has at least one
                        // honouring (dimmable) mark. A honouring family never
                        // aggregates (the two sets are disjoint), so the binding's
                        // style only ever reaches a dimmable mark.
                        for it in &p.items {
                            if let Component::Mark(m) = it {
                                if mark_kind_aggregates(m.kind) {
                                    warnings.push(ParseWarning::HighlightOnAggregate {
                                        selection: pr.0.clone(),
                                        mark: m.kind.wire_name().to_string(),
                                    });
                                }
                            }
                        }
                        let has_honouring = p.items.iter().any(|it| {
                            matches!(it, Component::Mark(m) if mark_honours_highlight(m.kind))
                        });
                        if !has_honouring {
                            // Nothing in this plot can dim → no binding (a bare
                            // aggregate-only highlight is inert, already warned).
                            continue;
                        }
                        bindings.push(HighlightBinding {
                            interactor_path: ComponentPath(format!(
                                "{item_path}/interactor[{}]",
                                intc.kind.wire_name()
                            )),
                            parent_plot: ComponentPath(path.to_string()),
                            selection: pr.0.clone(),
                            style: resolve_highlight_style(intc),
                        });
                    }
                    Component::Plot(_) | Component::HConcat(_) | Component::VConcat(_) => {
                        collect_highlight_bindings(
                            item,
                            &item_path,
                            spec,
                            known_selections,
                            bindings,
                            warnings,
                        );
                    }
                    _ => {}
                }
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_highlight_bindings(
                    item,
                    &format!("{path}/hconcat[{i}]"),
                    spec,
                    known_selections,
                    bindings,
                    warnings,
                );
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_highlight_bindings(
                    item,
                    &format!("{path}/vconcat[{i}]"),
                    spec,
                    known_selections,
                    bindings,
                    warnings,
                );
            }
        }
        _ => {}
    }
}

/// Register each highlight-bound plot's honouring marks as SUBSCRIBERS to the
/// `by:` selection, so a change to that selection re-queries them (with the
/// `__bf_selected` projection, via `emit_query`) — the same
/// `propagate_selection` spine `filterBy` rides. Called by
/// [`build_selection_subscriber_graph`] after the filterBy walk, so a mark that
/// both `filterBy`-s one selection and highlights on another lands in both
/// subscriber sets (ce-ac05: each binding resolved independently).
///
/// Only the honouring, row-level families are registered. A honouring family
/// never aggregates (the two kind sets are disjoint), so an aggregate mark is
/// never registered here — it can share a plot with a honouring mark (which is
/// registered), but is itself guarded out per-mark at emit. Mark paths match the
/// engine's `mark_index_map` keys (`…/plot[i]/mark[kind]`).
fn collect_highlight_subscribers(
    component: &Component,
    path: &str,
    known_selections: &HashSet<String>,
    graph: &mut SelectionSubscriberGraph,
) {
    match component {
        Component::Plot(p) => {
            let by = plot_highlight_by(&p.items)
                .map(|pr| pr.0.clone())
                .filter(|name| known_selections.contains(name));
            for (i, item) in p.items.iter().enumerate() {
                let item_path = format!("{path}/plot[{i}]");
                match item {
                    Component::Mark(m) if by.is_some() && mark_honours_highlight(m.kind) => {
                        let sel = by.clone().expect("guarded by is_some");
                        graph.entry(sel).or_default().push(ComponentPath(format!(
                            "{item_path}/mark[{}]",
                            m.kind.wire_name()
                        )));
                    }
                    Component::Plot(_) | Component::HConcat(_) | Component::VConcat(_) => {
                        collect_highlight_subscribers(item, &item_path, known_selections, graph);
                    }
                    _ => {}
                }
            }
        }
        Component::HConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_highlight_subscribers(
                    item,
                    &format!("{path}/hconcat[{i}]"),
                    known_selections,
                    graph,
                );
            }
        }
        Component::VConcat(c) => {
            for (i, item) in c.items.iter().enumerate() {
                collect_highlight_subscribers(
                    item,
                    &format!("{path}/vconcat[{i}]"),
                    known_selections,
                    graph,
                );
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// SpecAnalysis (ac-09, extended with cfs fields)
// ---------------------------------------------------------------------------

/// Result of static analysis on a parsed Spec.
#[derive(Debug, Clone)]
pub struct SpecAnalysis {
    /// Map from param name to subscriber component paths.
    pub subscriber_graph: SubscriberGraph,
    /// Directed dependency edges between params.
    pub dependency_edges: Vec<ParamEdge>,
    /// Params in topological order (upstream before downstream).
    pub topological_order: Vec<String>,
    /// Map from selection name to components that subscribe via filterBy.
    pub selection_subscribers: SelectionSubscriberGraph,
    /// Interactor-to-selection write bindings.
    pub interactor_bindings: Vec<InteractorBinding>,
    /// Brushable interactor bindings — derived view filtering
    /// `interactor_bindings` to brush-compatible kinds and pairing each with
    /// resolved channel columns. Card 0006 v3 (cfs3) ac-05.
    pub brushable_bindings: Vec<BrushableBinding>,
    /// Legend producer bindings — one per standalone `legend: color` node
    /// bound `as:` a selection, resolved to its `for:` plot. Card 0009
    /// (legend click-to-filter) lcf ac-01.
    pub legend_bindings: Vec<LegendBinding>,
    /// Highlight consumer bindings — one per `highlight, by: $sel` interactor,
    /// carrying its parent plot, the consumed selection, and the `otherwise`
    /// deemphasis style. Card 0021 (conditional encoding) ce-ac02.
    pub highlight_bindings: Vec<HighlightBinding>,
    /// Diagnostics discovered during analysis.
    pub warnings: Vec<ParseWarning>,
}

/// Param names PRODUCED somewhere in the component tree — the raw `as:`
/// option refs of BOTH producer forms: interactors (which also push a
/// pseudo-subscriber into the graph) and legends (which deliberately do
/// not — see the legend arm of [`collect_subscribers`]). Collected from the
/// raw option bag, any channel, unconditionally (no binding validation):
/// a param someone writes to is not "dead" even if the producer is
/// otherwise mis-configured.
fn collect_produced_params(spec: &Spec) -> HashSet<String> {
    fn walk(component: &Component, produced: &mut HashSet<String>) {
        match component {
            Component::Interactor(i) => {
                if let Some(ValueOrParamRef::Param(pr)) = i.options.get("as") {
                    produced.insert(pr.0.clone());
                }
            }
            Component::Legend(l) => {
                if let Some(ValueOrParamRef::Param(pr)) = l.options.get("as") {
                    produced.insert(pr.0.clone());
                }
            }
            Component::Plot(p) => {
                for item in &p.items {
                    walk(item, produced);
                }
            }
            Component::HConcat(c) | Component::VConcat(c) => {
                for item in &c.items {
                    walk(item, produced);
                }
            }
            _ => {}
        }
    }
    let mut produced = HashSet::new();
    if let Some(root) = &spec.root {
        walk(root, &mut produced);
    }
    produced
}

/// Run all static analyses on a parsed Spec.
///
/// Returns `Err` if a cycle is detected in the param dependency graph
/// or if a filterBy reference is invalid (missing or non-selection param).
pub fn analyse_spec(spec: &Spec) -> Result<SpecAnalysis, ParseError> {
    let subscriber_graph = build_subscriber_graph(spec);

    // Dead param warnings (rpw ac-04). A param PRODUCED by an interactor or
    // legend `as:` is not dead even with zero subscribers: interactors
    // suppress the warning via their pseudo-subscriber push in
    // collect_subscribers, and legends (whose `as:` is deliberately NOT a
    // subscription — card 0009) are covered by the produced set, keeping the
    // two producer forms symmetric.
    let produced = collect_produced_params(spec);
    let mut warnings: Vec<ParseWarning> = Vec::new();
    for (name, subscribers) in &subscriber_graph {
        if subscribers.is_empty() && spec.params.contains_key(name) && !produced.contains(name) {
            warnings.push(ParseWarning::DeadParam {
                name: name.clone(),
            });
        }
    }

    // DAG and topological order (rpw ac-05, ac-06).
    let (dependency_edges, topological_order) = build_dependency_dag(spec)?;

    // Type mismatch warnings (rpw ac-07).
    warnings.extend(check_param_type_mismatches(spec));

    // filterBy validation — hard error on missing or non-selection refs (cfs ac-01..ac-04).
    validate_filter_by_refs(spec)?;

    // Interactor binding warnings (cfs ac-05, ac-06).
    warnings.extend(validate_interactor_bindings(spec));

    // Selection subscriber graph (cfs ac-07).
    let selection_subscribers = build_selection_subscriber_graph(spec);

    // Interactor bindings (cfs ac-08).
    let interactor_bindings = build_interactor_bindings(spec);

    // Brushable bindings (cfs3 ac-05): derived from interactor_bindings,
    // filtered to brush-compatible kinds, paired with parent plot channels.
    let brushable_bindings = build_brushable_bindings(spec);

    // Legend producer bindings (card 0009, lcf ac-01) — pushes LegendBinding*
    // warnings for `as:` targets that fail the crossfilter precondition.
    let legend_bindings = build_legend_bindings(spec, &mut warnings);

    // Highlight consumer bindings (card 0021, ce-ac02) — pushes
    // HighlightBinding* / HighlightOnAggregate warnings for invalid `by:` refs
    // and aggregate-mark guards.
    let highlight_bindings = build_highlight_bindings(spec, &mut warnings);

    Ok(SpecAnalysis {
        subscriber_graph,
        dependency_edges,
        topological_order,
        selection_subscribers,
        interactor_bindings,
        brushable_bindings,
        legend_bindings,
        highlight_bindings,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_spec, Format};

    // Helper: build a SpecAnalysis with the given dependency edges, no
    // subscribers, no warnings — for topological_descendants tests.
    fn analysis_with_edges(edges: Vec<(&str, &str)>) -> SpecAnalysis {
        SpecAnalysis {
            subscriber_graph: SubscriberGraph::new(),
            dependency_edges: edges
                .into_iter()
                .map(|(f, t)| ParamEdge {
                    from: f.to_string(),
                    to: t.to_string(),
                })
                .collect(),
            topological_order: Vec::new(),
            selection_subscribers: SelectionSubscriberGraph::new(),
            interactor_bindings: Vec::new(),
            brushable_bindings: Vec::new(),
            legend_bindings: Vec::new(),
            highlight_bindings: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// rpw3 ac-01: topological_descendants on a simple linear chain
    /// A → B → C returns [A, B, C].
    #[test]
    fn rpw3_ac01_topological_descendants_simple_chain() {
        let analysis = analysis_with_edges(vec![("A", "B"), ("B", "C")]);
        let order = topological_descendants(&analysis, "A");
        assert_eq!(order, vec!["A".to_string(), "B".to_string(), "C".to_string()]);
    }

    /// rpw3 ac-02: topological_descendants on the athletes.yaml corpus
    /// chain returns the expected descendant ordering.
    ///
    /// athletes.yaml DAG: category → query → hover (search input writes
    /// $query and reads $category; table input writes $hover and reads
    /// $query).
    #[test]
    fn rpw3_ac02_topological_descendants_athletes_yaml() {
        let yaml = std::fs::read_to_string(
            "vendor/mosaic-specs/yaml/athletes.yaml",
        )
        .expect("athletes.yaml fixture must be readable");
        let parsed = parse_spec(&yaml, Format::Yaml).expect("parse athletes.yaml");
        let analysis = analyse_spec(&parsed.spec).expect("analyse athletes.yaml");

        // Walk descendants from `category`.
        let order = topological_descendants(&analysis, "category");
        assert_eq!(order[0], "category", "root must be first");
        // hover depends on query (transitively on category) — must come after query.
        let pos_query = order
            .iter()
            .position(|n| n == "query")
            .expect("query in descendants");
        let pos_hover = order
            .iter()
            .position(|n| n == "hover")
            .expect("hover in descendants");
        assert!(
            pos_query < pos_hover,
            "query must precede hover; got order: {:?}",
            order
        );
        // Cross-check against analysis.topological_order projection over
        // the descendant set: positions must be consistent.
        let descendant_set: HashSet<&str> = order.iter().map(String::as_str).collect();
        let projected: Vec<&String> = analysis
            .topological_order
            .iter()
            .filter(|n| descendant_set.contains(n.as_str()))
            .collect();
        let projected_strs: Vec<String> =
            projected.into_iter().cloned().collect();
        assert_eq!(
            order, projected_strs,
            "topological_descendants must agree with analysis.topological_order projection"
        );
    }

    /// rpw3 ac-03: topological_descendants on a leaf root (no DAG edges
    /// out, possibly not in DAG at all) returns [root].
    #[test]
    fn rpw3_ac03_topological_descendants_leaf_root() {
        // Root with zero outgoing edges, but present as a target of
        // another edge — its descendants are just [itself].
        let analysis = analysis_with_edges(vec![("upstream", "root")]);
        let order = topological_descendants(&analysis, "root");
        assert_eq!(order, vec!["root".to_string()]);

        // Root not present in the DAG at all (no edges): still returns
        // [root] (the coordinator must still update param_state).
        let empty_analysis = analysis_with_edges(vec![]);
        let order = topological_descendants(&empty_analysis, "isolated");
        assert_eq!(order, vec!["isolated".to_string()]);
    }

    // ----- card 0006 v2 cfs2_ac04: parent_plot helper -----
    #[test]
    fn cfs2_ac04_parent_plot_helper() {
        // mark inside vconcat → plot
        assert_eq!(
            parent_plot("root/vconcat[0]/plot[1]/mark[dot]"),
            "root/vconcat[0]/plot[1]"
        );
        // mark inside plot at root level
        assert_eq!(parent_plot("root/plot[0]/mark[bar]"), "root/plot[0]");
        // interactor under a plot — same parent prefix as the mark
        assert_eq!(
            parent_plot("root/plot[0]/interactor[intervalX]"),
            "root/plot[0]"
        );
        // mark not inside any plot — degenerate fallback returns input unchanged
        assert_eq!(parent_plot("root/mark[dot]"), "root/mark[dot]");
        // root only — no plot in path
        assert_eq!(parent_plot("root"), "root");
        // multi-digit plot index
        assert_eq!(
            parent_plot("root/plot[12]/mark[line]"),
            "root/plot[12]"
        );
        // no /plot[ but contains substring "plot" — must not match
        assert_eq!(parent_plot("root/plotter[0]"), "root/plotter[0]");
        // nested concat hierarchy
        assert_eq!(
            parent_plot("root/hconcat[2]/vconcat[0]/plot[3]/mark[dot]"),
            "root/hconcat[2]/vconcat[0]/plot[3]"
        );
    }

    // ----- card 0006: plot_node_path stable plot identity -----
    #[test]
    fn crossfilter_plot_node_path_is_item_index_stable() {
        // A plot's mark and its brushing interactor have different item
        // indices, but must resolve to the SAME plot identity.
        assert_eq!(
            plot_node_path("root/hconcat[0]/plot[0]/mark[dot]"),
            "root/hconcat[0]"
        );
        assert_eq!(
            plot_node_path("root/hconcat[0]/plot[1]/interactor[intervalX]"),
            "root/hconcat[0]",
            "interactor (item 1) collapses to the same plot node as the mark (item 0)"
        );
        // Root-level plot → the plot node is `root`.
        assert_eq!(plot_node_path("root/plot[2]/mark[bar]"), "root");
        // Distinct plots in a concat stay distinct.
        assert_eq!(plot_node_path("root/vconcat[0]/plot[0]/mark[dot]"), "root/vconcat[0]");
        assert_eq!(plot_node_path("root/vconcat[1]/plot[0]/mark[line]"), "root/vconcat[1]");
        // No /plot[ segment → unchanged. "plotter" must not match.
        assert_eq!(plot_node_path("root/mark[dot]"), "root/mark[dot]");
        assert_eq!(plot_node_path("root/plotter[0]"), "root/plotter[0]");
        // Nested plot-in-plot: strip only the deepest /plot[ segment, so the
        // nested plot's mark and a nested interactor still agree.
        assert_eq!(
            plot_node_path("root/plot[0]/plot[1]/mark[dot]"),
            "root/plot[0]"
        );
    }

    // ac-01: typed fields on Input
    #[test]
    fn rpw_ac01_input_typed_fields_extracted() {
        let yaml = r#"
data:
  athletes: { file: athletes.csv }
params:
  category: Athlete
input: menu
as: $category
from: athletes
column: athlete
filterBy: $category
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let root = out.spec.root.as_ref().expect("has root");
        if let Component::Input(inp) = root {
            assert_eq!(inp.as_param.as_ref().map(|p| p.0.as_str()), Some("category"));
            assert_eq!(inp.from_source.as_deref(), Some("athletes"));
            assert_eq!(inp.filter_by.as_ref().map(|p| p.0.as_str()), Some("category"));
            // `as`, `from`, `filterBy` should NOT be in options
            assert!(!inp.options.contains_key("as"));
            assert!(!inp.options.contains_key("from"));
            assert!(!inp.options.contains_key("filterBy"));
            // `column` should still be in options
            assert!(inp.options.contains_key("column"));
        } else {
            panic!("expected Component::Input, got {root:?}");
        }
    }

    #[test]
    fn rpw_ac01_input_no_typed_fields() {
        let yaml = r#"
input: slider
min: 0
max: 100
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let root = out.spec.root.as_ref().expect("has root");
        if let Component::Input(inp) = root {
            assert!(inp.as_param.is_none());
            assert!(inp.from_source.is_none());
            assert!(inp.filter_by.is_none());
        } else {
            panic!("expected Component::Input");
        }
    }

    // ac-03: subscriber graph
    #[test]
    fn rpw_ac03_subscriber_graph_basic() {
        let yaml = r#"
params:
  threshold: 42
plot:
  - mark: dot
    x: { channel: "delay" }
    filterBy: $threshold
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let graph = build_subscriber_graph(&out.spec);
        let subs = graph.get("threshold").expect("has threshold");
        assert!(!subs.is_empty(), "threshold should have subscribers");
    }

    /// pefr ac-06 (card 0014): a param embedded in a `data.filter` SQL
    /// expression subscribes the mark, so propagate_param re-executes it.
    /// Before card 0014, collect_spec_value_subscribers ignored Expressions.
    #[test]
    fn pefr_ac06_expression_param_subscribes() {
        let yaml = r#"
params:
  k: 0
data:
  t: [{ x: 1 }, { x: 5 }]
plot:
  - mark: dot
    data: { from: t, filter: "x > $k" }
    x: x
    y: x
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let graph = build_subscriber_graph(&out.spec);
        let subs = graph
            .get("k")
            .expect("filter param k should have subscribers");
        assert!(
            subs.iter().any(|p| p.0.contains("mark[dot]")),
            "the data.filter param k should subscribe the dot mark; got {:?}",
            subs
        );
    }

    #[test]
    fn rpw_ac03_subscriber_graph_multiple_subscribers() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
vconcat:
  - plot:
    - mark: dot
      data: { from: t1, filterBy: $brush }
  - plot:
    - mark: line
      data: { from: t2, filterBy: $brush }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let graph = build_subscriber_graph(&out.spec);
        let subs = graph.get("brush").expect("has brush");
        assert!(subs.len() >= 2, "brush should have at least 2 subscribers, got {}", subs.len());
    }

    // ac-04: dead param warning
    #[test]
    fn rpw_ac04_dead_param_warning() {
        let yaml = r#"
params:
  unused: 42
plot:
  - mark: dot
    x: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.warnings.iter().any(|w| matches!(w, ParseWarning::DeadParam { name } if name == "unused")),
            "should warn about dead param 'unused'"
        );
    }

    #[test]
    fn rpw_ac04_no_dead_param_when_referenced() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
plot:
  - mark: dot
    data: { from: t1, filterBy: $brush }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            !analysis.warnings.iter().any(|w| matches!(w, ParseWarning::DeadParam { .. })),
            "should not warn about referenced param"
        );
    }

    // ac-05: topological ordering
    #[test]
    fn rpw_ac05_topological_order_chain() {
        let yaml = r#"
params:
  category: All
  query: ""
vconcat:
  - input: menu
    as: $category
    from: athletes
    column: sport
  - input: search
    filterBy: $category
    as: $query
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let (_, order) = build_dependency_dag(&out.spec).expect("no cycle");
        let cat_pos = order.iter().position(|n| n == "category");
        let query_pos = order.iter().position(|n| n == "query");
        assert!(
            cat_pos.is_some() && query_pos.is_some(),
            "both params should be in topological order"
        );
        assert!(
            cat_pos.unwrap() < query_pos.unwrap(),
            "category should come before query in topological order"
        );
    }

    // ac-06: cycle detection
    #[test]
    fn rpw_ac06_cycle_detected() {
        let yaml = r#"
params:
  a: 1
  b: 2
vconcat:
  - input: menu
    filterBy: $a
    as: $b
  - input: menu
    filterBy: $b
    as: $a
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let result = build_dependency_dag(&out.spec);
        assert!(result.is_err(), "should detect cycle");
        if let Err(ParseError::SchemaViolation { detail, .. }) = result {
            assert!(detail.contains("circular"), "error should mention circular: {detail}");
        }
    }

    // ac-07: type mismatch
    #[test]
    fn rpw_ac07_slider_to_selection_mismatch() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
input: slider
as: $brush
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let warnings = check_param_type_mismatches(&out.spec);
        assert!(
            warnings.iter().any(|w| matches!(w, ParseWarning::ParamTypeMismatch { param, .. } if param == "brush")),
            "slider writing to selection param should produce mismatch"
        );
    }

    #[test]
    fn rpw_ac07_table_to_scalar_mismatch() {
        let yaml = r#"
params:
  count: 42
input: table
as: $count
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let warnings = check_param_type_mismatches(&out.spec);
        assert!(
            warnings.iter().any(|w| matches!(w, ParseWarning::ParamTypeMismatch { param, .. } if param == "count")),
            "table writing to scalar param should produce mismatch"
        );
    }

    #[test]
    fn rpw_ac07_slider_to_numeric_no_mismatch() {
        let yaml = r#"
params:
  threshold: 42
input: slider
as: $threshold
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let warnings = check_param_type_mismatches(&out.spec);
        assert!(
            !warnings.iter().any(|w| matches!(w, ParseWarning::ParamTypeMismatch { .. })),
            "slider writing to numeric param should not produce mismatch"
        );
    }

    // ac-08: type enum constructors
    #[test]
    fn rpw_ac08_widget_output_type_mapping() {
        assert_eq!(WidgetOutputType::from_input_kind(InputKind::Slider), WidgetOutputType::ScalarNumeric);
        assert_eq!(WidgetOutputType::from_input_kind(InputKind::Menu), WidgetOutputType::ScalarString);
        assert_eq!(WidgetOutputType::from_input_kind(InputKind::Search), WidgetOutputType::ScalarString);
        assert_eq!(WidgetOutputType::from_input_kind(InputKind::Table), WidgetOutputType::Selection);
    }

    #[test]
    fn rpw_ac08_param_declared_type_mapping() {
        assert_eq!(
            ParamDeclaredType::from_param_node(&ParamNode::Value(SpecValue::Integer(42))),
            ParamDeclaredType::ScalarNumeric
        );
        assert_eq!(
            ParamDeclaredType::from_param_node(&ParamNode::Value(SpecValue::Float(3.14))),
            ParamDeclaredType::ScalarNumeric
        );
        assert_eq!(
            ParamDeclaredType::from_param_node(&ParamNode::Value(SpecValue::String("hi".into()))),
            ParamDeclaredType::ScalarString
        );
        assert_eq!(
            ParamDeclaredType::from_param_node(&ParamNode::Value(SpecValue::Bool(true))),
            ParamDeclaredType::ScalarBool
        );
        assert_eq!(
            ParamDeclaredType::from_param_node(&ParamNode::Value(SpecValue::Array(vec![]))),
            ParamDeclaredType::Array
        );
        // Test selection param type via parsing
        let yaml = "params:\n  brush: { select: crossfilter }\n";
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let brush = out.spec.params.get("brush").expect("has brush");
        assert_eq!(ParamDeclaredType::from_param_node(brush), ParamDeclaredType::Selection);
    }

    // ac-09: integrated analysis
    #[test]
    fn rpw_ac09_analyse_spec_integration() {
        let yaml = r#"
params:
  category: { select: intersect }
  unused: 99
vconcat:
  - input: menu
    as: $category
    from: athletes
    column: sport
  - plot:
    - mark: dot
      data: { from: t1, filterBy: $category }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        // subscriber graph present
        assert!(analysis.subscriber_graph.contains_key("category"));
        assert!(analysis.subscriber_graph.contains_key("unused"));
        // topological order present
        assert!(!analysis.topological_order.is_empty());
        // dead param warning for 'unused'
        assert!(analysis.warnings.iter().any(|w| matches!(w, ParseWarning::DeadParam { name } if name == "unused")));
    }

    // ac-10: empty spec produces empty analysis
    #[test]
    fn rpw_ac10_empty_spec_no_warnings() {
        let yaml = r#"
data:
  flights: { file: flights.parquet }
plot:
  - mark: dot
    x: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(analysis.subscriber_graph.is_empty());
        assert!(analysis.dependency_edges.is_empty());
        assert!(analysis.topological_order.is_empty());
        assert!(analysis.warnings.is_empty());
    }

    // ac-02: vendored specs with inputs parse correctly
    #[test]
    fn rpw_ac02_vendored_specs_parse_with_typed_fields() {
        use std::path::PathBuf;
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("mosaic-specs")
            .join("yaml");
        let mut tested = 0;
        for entry in std::fs::read_dir(&corpus).expect("corpus dir exists").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read");
            let out = parse_spec(&source, Format::Yaml)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));

            // Walk the component tree looking for Input nodes
            fn check_inputs(c: &Component, path: &str) {
                match c {
                    Component::Input(inp) => {
                        // If the original spec had `as:`, it should be in as_param, not options
                        assert!(
                            !inp.options.contains_key("as"),
                            "{path}: 'as' should be extracted to as_param, not in options"
                        );
                        assert!(
                            !inp.options.contains_key("from"),
                            "{path}: 'from' should be extracted to from_source, not in options"
                        );
                        assert!(
                            !inp.options.contains_key("filterBy"),
                            "{path}: 'filterBy' should be extracted to filter_by, not in options"
                        );
                    }
                    Component::Plot(p) => {
                        for item in &p.items {
                            check_inputs(item, path);
                        }
                    }
                    Component::HConcat(c) | Component::VConcat(c) => {
                        for item in &c.items {
                            check_inputs(item, path);
                        }
                    }
                    _ => {}
                }
            }

            if let Some(root) = &out.spec.root {
                check_inputs(root, &path.display().to_string());
            }
            tested += 1;
        }
        assert!(tested > 0, "no vendored specs found");
    }

    // -----------------------------------------------------------------------
    // cfs (cross-filtered selections) tests
    // -----------------------------------------------------------------------

    // ac-01: filterBy on mark data referencing a missing param → error
    #[test]
    fn cfs_ac01_filterby_mark_missing_param() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
plot:
  - mark: dot
    data: { from: t1, filterBy: $missing }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let err = analyse_spec(&out.spec).unwrap_err();
        match err {
            ParseError::SchemaViolation { detail, .. } => {
                assert!(detail.contains("missing"), "error should name the param: {detail}");
                assert!(detail.contains("undeclared"), "error should say undeclared: {detail}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ac-02: filterBy on mark data referencing a value param → error
    #[test]
    fn cfs_ac02_filterby_mark_value_param() {
        let yaml = r#"
params:
  threshold: 42
plot:
  - mark: dot
    data: { from: t1, filterBy: $threshold }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let err = analyse_spec(&out.spec).unwrap_err();
        match err {
            ParseError::SchemaViolation { detail, .. } => {
                assert!(detail.contains("threshold"), "error should name the param: {detail}");
                assert!(detail.contains("selection"), "error should mention selection: {detail}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ac-03: filterBy on input referencing a missing param → error
    #[test]
    fn cfs_ac03_filterby_input_missing_param() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
input: menu
filterBy: $ghost
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let err = analyse_spec(&out.spec).unwrap_err();
        match err {
            ParseError::SchemaViolation { detail, .. } => {
                assert!(detail.contains("ghost"), "error should name the param: {detail}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ac-04: filterBy on input referencing a value param → error
    #[test]
    fn cfs_ac04_filterby_input_value_param() {
        let yaml = r#"
params:
  x: 1
input: menu
filterBy: $x
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let err = analyse_spec(&out.spec).unwrap_err();
        match err {
            ParseError::SchemaViolation { detail, .. } => {
                assert!(detail.contains("x"), "error should name the param: {detail}");
                assert!(detail.contains("selection"), "error should mention selection: {detail}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ac-05: interactor as: missing param → warning
    #[test]
    fn cfs_ac05_interactor_binding_missing() {
        let yaml = r#"
plot:
  - select: intervalX
    as: $ghost
  - mark: dot
    x: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w, ParseWarning::InteractorBindingMissing { name } if name == "ghost"
            )),
            "should warn about missing interactor binding target"
        );
    }

    // ac-06: interactor as: value param → warning
    #[test]
    fn cfs_ac06_interactor_binding_non_selection() {
        let yaml = r#"
params:
  count: 42
plot:
  - select: intervalX
    as: $count
  - mark: dot
    x: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w, ParseWarning::InteractorBindingNonSelection { name } if name == "count"
            )),
            "should warn about interactor binding to non-selection param"
        );
    }

    // ac-07: selection subscriber graph
    #[test]
    fn cfs_ac07_selection_subscriber_graph() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
vconcat:
  - plot:
    - mark: dot
      data: { from: t1, filterBy: $brush }
  - plot:
    - mark: line
      data: { from: t2, filterBy: $brush }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let graph = build_selection_subscriber_graph(&out.spec);
        let subs = graph.get("brush").expect("has brush");
        assert!(subs.len() >= 2, "brush should have at least 2 subscribers, got {}", subs.len());
    }

    #[test]
    fn cfs_ac07_selection_subscriber_graph_excludes_value_params() {
        let yaml = r#"
params:
  threshold: 42
  brush: { select: crossfilter }
plot:
  - mark: dot
    data: { from: t1, filterBy: $brush }
    x: $threshold
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let graph = build_selection_subscriber_graph(&out.spec);
        // threshold is a value param, should NOT appear in selection subscriber graph
        assert!(!graph.contains_key("threshold"), "value param should not be in selection subscriber graph");
        // brush is a selection, should appear
        assert!(graph.contains_key("brush"));
    }

    // ac-08: interactor bindings
    #[test]
    fn cfs_ac08_interactor_bindings() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
vconcat:
  - plot:
    - select: intervalX
      as: $brush
    - mark: dot
      data: { from: t1, filterBy: $brush }
  - plot:
    - select: intervalX
      as: $brush
    - mark: line
      data: { from: t2, filterBy: $brush }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let bindings = build_interactor_bindings(&out.spec);
        assert_eq!(bindings.len(), 2, "should have 2 interactor bindings");
        assert!(bindings.iter().all(|b| b.selection == "brush"), "all bindings should target brush");
    }

    // ac-09: SpecAnalysis integration with new fields
    #[test]
    fn cfs_ac09_analyse_spec_integration() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
vconcat:
  - plot:
    - select: intervalX
      as: $brush
    - mark: dot
      data: { from: flights, filterBy: $brush }
  - plot:
    - select: intervalX
      as: $brush
    - mark: rectY
      data: { from: flights, filterBy: $brush }
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");

        // selection_subscribers present
        assert!(analysis.selection_subscribers.contains_key("brush"));
        let subs = analysis.selection_subscribers.get("brush").unwrap();
        assert!(subs.len() >= 2, "brush should have at least 2 selection subscribers");

        // interactor_bindings present
        assert_eq!(analysis.interactor_bindings.len(), 2);
        assert!(analysis.interactor_bindings.iter().all(|b| b.selection == "brush"));

        // No errors — valid crossfilter spec
    }

    // ac-10: vendored corpus passes analyse_spec
    #[test]
    fn cfs_ac10_vendored_corpus_passes_analyse() {
        use std::path::PathBuf;
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("mosaic-specs")
            .join("yaml");
        let mut tested = 0;
        for entry in std::fs::read_dir(&corpus).expect("corpus dir exists").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read");
            let out = parse_spec(&source, Format::Yaml)
                .unwrap_or_else(|e| panic!("{}: parse failed: {e}", path.display()));
            analyse_spec(&out.spec)
                .unwrap_or_else(|e| panic!("{}: analyse_spec failed: {e}", path.display()));
            tested += 1;
        }
        assert!(tested > 0, "no vendored specs found");
    }

    // ac-11: round-trip preserved (filterBy on selection still round-trips)
    #[test]
    fn cfs_ac11_round_trip_with_selection_filterby() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
plot:
  - mark: dot
    data: { from: flights, filterBy: $brush }
  - select: intervalX
    as: $brush
"#;
        let a = parse_spec(yaml, Format::Yaml).expect("first parse");
        let serialised = serde_yaml::to_string(&a.spec).expect("serialise");
        let b = parse_spec(&serialised, Format::Yaml).expect("second parse");
        assert_eq!(a.spec, b.spec, "round-trip should produce equal Specs");
    }

    // -----------------------------------------------------------------------
    // cfs3 — brushable_bindings derived view (card 0006 v3)
    // -----------------------------------------------------------------------

    /// cfs3_ac05: build_brushable_bindings filters interactor_bindings to
    /// brush-compatible kinds (IntervalX/Y/XY) and pairs each with the
    /// parent plot's resolved channel columns. Non-brushable interactors
    /// (panZoom etc.) are excluded.
    #[test]
    fn cfs3_ac05_brushable_bindings_built() {
        let yaml = r#"
params:
  brush: { select: crossfilter }
plot:
  - select: intervalXY
    as: $brush
  - select: panZoom
  - mark: dot
    data: { from: flights, filterBy: $brush }
    x: speed
    y: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");

        assert_eq!(
            analysis.brushable_bindings.len(),
            1,
            "panZoom is filtered out — exactly one brushable binding remains"
        );
        let bb = &analysis.brushable_bindings[0];
        assert_eq!(
            bb.interactor_path,
            ComponentPath("root/plot[0]/interactor[intervalXY]".to_string()),
            "path uses kind.wire_name() per analysis convention"
        );
        assert_eq!(
            bb.parent_plot,
            ComponentPath("root".to_string()),
            "contributor identity is the stable plot-node path (here the root \
             plot), NOT the interactor's item-index path — so it matches the \
             subscriber mark's plot_node_path for self-exclusion"
        );
        assert_eq!(bb.selection, "brush");
        assert_eq!(bb.kind, BrushKind::IntervalXY);
        assert_eq!(bb.channels.x.as_deref(), Some("speed"));
        assert_eq!(bb.channels.y.as_deref(), Some("delay"));
    }

    /// cfs3_ac09 (sub-clause c — spec-side property): a plot containing only
    /// non-brushable interactors (panZoom) produces zero brushable bindings.
    /// Asserts the property "non-brushable kinds excluded from
    /// brushable_bindings" without coupling to the UI-side BrushKind::Point
    /// naming.
    #[test]
    fn cfs3_ac09_non_brushable_kinds_excluded() {
        let yaml = r#"
params: {}
plot:
  - select: panZoom
  - mark: dot
    data: { from: flights }
    x: speed
    y: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.brushable_bindings.is_empty(),
            "panZoom-only plot must yield no brushable bindings: {:?}",
            analysis.brushable_bindings
        );
    }

    /// cfs point-selection (card 0006): a `toggleX` interactor becomes a
    /// brushable binding of kind `PointX` carrying the plot's x column; a
    /// `toggleY` becomes `PointY` on the y column. This wires point selections
    /// into the same binding pipeline the interval brushes use.
    #[test]
    fn toggle_x_y_produce_point_bindings() {
        let yaml = r#"
params:
  sel: { select: single }
plot:
  - select: toggleX
    as: $sel
  - select: toggleY
    as: $sel
  - mark: dot
    data: { from: flights, filterBy: $sel }
    x: speed
    y: delay
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");

        let kinds: Vec<(BrushKind, Option<&str>, Option<&str>)> = analysis
            .brushable_bindings
            .iter()
            .map(|b| (b.kind, b.channels.x.as_deref(), b.channels.y.as_deref()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (BrushKind::PointX, Some("speed"), Some("delay")),
                (BrushKind::PointY, Some("speed"), Some("delay")),
            ],
            "toggleX→PointX and toggleY→PointY, each carrying the plot's channels"
        );
    }

    // -----------------------------------------------------------------------
    // Legend producer bindings — card 0009 (legend click-to-filter), lcf ac-01
    // -----------------------------------------------------------------------

    /// Two plots sharing a categorical column, with the legend bound
    /// `as: $sel for: scatter` — the lcf_ac01 fixture.
    const LEGEND_BINDING_SPEC: &str = r#"
params:
  sel: { select: crossfilter }
data:
  t:
    - { x: 1, y: 3, species: adelie }
    - { x: 2, y: 5, species: gentoo }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: species
    name: scatter
  - legend: color
    for: scatter
    as: $sel
  - plot:
    - mark: dot
      data: { from: t, filterBy: $sel }
      x: x
      y: y
"#;

    /// lcf_ac01: analysing a `legend: color as: $sel for: scatter` spec yields
    /// a LegendBinding whose contributor is the `for:` plot's NODE path (the
    /// self-exclusion identity), whose colour column comes from that plot's
    /// first mark's fill channel, and whose legend path locates the node.
    #[test]
    fn lcf_ac01_legend_as_binding_resolves_producer_fields() {
        let out = parse_spec(LEGEND_BINDING_SPEC, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");

        assert_eq!(
            analysis.legend_bindings.len(),
            1,
            "exactly one bound legend: {:?}",
            analysis.legend_bindings
        );
        let b = &analysis.legend_bindings[0];
        assert_eq!(b.selection, "sel", "`as: $sel` names the selection");
        assert_eq!(
            b.plot_path.0, "root/hconcat[0]",
            "contributor = the for:-plot's node path (compile_selection's self_source)"
        );
        assert_eq!(
            b.colour_column, "species",
            "colour column from the for:-plot's first mark's fill channel"
        );
        assert_eq!(b.legend_path.0, "root/hconcat[1]", "the legend node's own path");
    }

    /// lcf_ac01 (the backwards-wiring trap, pinned): the legend's `as:` is a
    /// producer binding, so the legend must NOT appear in `$sel`'s subscriber
    /// graph — only marks subscribe (via filterBy). Before the fix the
    /// analysis arm registered `as: $sel` as a legend subscription.
    #[test]
    fn lcf_ac01_legend_is_not_a_subscriber_of_its_selection() {
        let out = parse_spec(LEGEND_BINDING_SPEC, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");

        let subs = analysis
            .subscriber_graph
            .get("sel")
            .expect("$sel is in the graph (the downstream mark subscribes)");
        assert!(
            !subs.is_empty() && subs.iter().all(|p| p.0.contains("/mark[")),
            "subscribers of $sel are marks only, never the producing legend: {subs:?}"
        );
        assert!(
            subs.iter().all(|p| !p.0.contains("/legend")),
            "regression: `as:` must not wire the legend as a subscriber: {subs:?}"
        );
    }

    /// lcf_ac01 (for:-resolution matches resolve_legends semantics): with
    /// `for:` absent, the legend binds to the dashboard's SOLE colour-encoded
    /// plot; with two colour-encoded plots the fallback is ambiguous and no
    /// binding forms. A legend without `as:` never binds (display-only).
    #[test]
    fn lcf_ac01_sole_colour_plot_fallback_and_display_only() {
        // for:-absent + one colour-encoded plot → binds to it.
        let sole = r#"
params:
  sel: { select: crossfilter }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
  - legend: color
    as: $sel
"#;
        let out = parse_spec(sole, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert_eq!(analysis.legend_bindings.len(), 1, "sole-colour-plot fallback binds");
        assert_eq!(analysis.legend_bindings[0].plot_path.0, "root/hconcat[0]");
        assert_eq!(analysis.legend_bindings[0].colour_column, "grp");

        // for:-absent + TWO colour-encoded plots → ambiguous, no binding.
        let ambiguous = r#"
params:
  sel: { select: crossfilter }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: other
  - legend: color
    as: $sel
"#;
        let out = parse_spec(ambiguous, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.legend_bindings.is_empty(),
            "two colour plots + no for: is ambiguous — no binding: {:?}",
            analysis.legend_bindings
        );

        // No `as:` → display-only, no binding (0016 behaviour preserved).
        let display_only = r#"
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: scatter
"#;
        let out = parse_spec(display_only, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.legend_bindings.is_empty(),
            "a display-only legend (no as:) yields no binding"
        );
    }

    /// lcf F2 (named ≠ sole): with TWO colour-encoded plots, `for: scatter`
    /// must resolve by NAME to the scatter plot — a mutant that falls back to
    /// "the sole colour plot" either binds nothing (two candidates) or the
    /// wrong plot, and fails here. The single-colour-plot fixtures above
    /// cannot distinguish named resolution from the sole fallback.
    #[test]
    fn lcf_f2_named_for_resolves_among_multiple_colour_plots() {
        let yaml = r#"
params:
  sel: { select: crossfilter }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: species
    name: scatter
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: other
  - legend: color
    for: scatter
    as: $sel
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert_eq!(
            analysis.legend_bindings.len(),
            1,
            "exactly one binding: {:?}",
            analysis.legend_bindings
        );
        let b = &analysis.legend_bindings[0];
        assert_eq!(
            b.plot_path.0, "root/hconcat[0]",
            "for: scatter resolves to the NAMED plot, not a sole fallback"
        );
        assert_eq!(b.colour_column, "species", "the named plot's fill column");
    }

    /// lcf F2 (named-but-unmatched): a `for:` that names no colour-encoded
    /// plot binds NOTHING, even when a sole colour plot exists — never
    /// silently borrow another plot's scale (mirrors resolve_legends'
    /// explicit-for:-must-match rule). A param-valued `for:` is equally
    /// unresolvable. Duplicate plot names resolve last-wins, matching the
    /// resolver.
    #[test]
    fn lcf_f2_unmatched_for_never_borrows_the_sole_plot() {
        // for: nosuch + one sole colour plot → EMPTY, not the sole plot.
        let unmatched = r#"
params:
  sel: { select: crossfilter }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: nosuch
    as: $sel
"#;
        let out = parse_spec(unmatched, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.legend_bindings.is_empty(),
            "for: nosuch must not borrow the sole colour plot: {:?}",
            analysis.legend_bindings
        );

        // for: $param → unresolvable, no binding.
        let param_for = r#"
params:
  sel: { select: crossfilter }
  which: scatter
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: $which
    as: $sel
"#;
        let out = parse_spec(param_for, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.legend_bindings.is_empty(),
            "a param-valued for: never binds: {:?}",
            analysis.legend_bindings
        );

        // Duplicate name → last-wins (matching resolve_legends).
        let duplicate = r#"
params:
  sel: { select: crossfilter }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: first_col
    name: scatter
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: second_col
    name: scatter
  - legend: color
    for: scatter
    as: $sel
"#;
        let out = parse_spec(duplicate, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert_eq!(analysis.legend_bindings.len(), 1);
        assert_eq!(
            analysis.legend_bindings[0].plot_path.0, "root/hconcat[1]",
            "duplicate plot names resolve last-wins, matching resolve_legends"
        );
        assert_eq!(analysis.legend_bindings[0].colour_column, "second_col");
    }

    /// lcf F3 (self-exclusion gating): the `as:` target must be a DECLARED
    /// selection with crossfilter resolution — the only resolution
    /// compile_selection self-excludes under. A `select: single` target
    /// would filter the legend's own for:-plot, so it yields NO binding and
    /// a LegendBindingNonCrossfilter warning; an undeclared name and a value
    /// param likewise skip with their own warning kinds.
    #[test]
    fn lcf_f3_non_crossfilter_as_targets_skip_with_warnings() {
        let with_params = |params: &str| {
            format!(
                r#"
params:
{params}
hconcat:
  - plot:
    - mark: dot
      data: {{ from: t }}
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: scatter
    as: $sel
"#
            )
        };

        // select: single — declared selection, wrong resolution.
        let out =
            parse_spec(&with_params("  sel: { select: single }"), Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            analysis.legend_bindings.is_empty(),
            "a select: single target must not bind (it would filter its own plot): {:?}",
            analysis.legend_bindings
        );
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::LegendBindingNonCrossfilter { name, resolution }
                    if name == "sel" && resolution == "single"
            )),
            "warns non-crossfilter: {:?}",
            analysis.warnings
        );

        // Undeclared param name.
        let undeclared = r#"
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: scatter
    as: $sel
"#;
        let out = parse_spec(undeclared, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(analysis.legend_bindings.is_empty());
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::LegendBindingMissing { name } if name == "sel"
            )),
            "warns missing: {:?}",
            analysis.warnings
        );

        // Value param.
        let out = parse_spec(&with_params("  sel: 42"), Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(analysis.legend_bindings.is_empty());
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::LegendBindingNonSelection { name } if name == "sel"
            )),
            "warns non-selection: {:?}",
            analysis.warnings
        );

        // Control: crossfilter binds cleanly, no LegendBinding* warnings.
        let out = parse_spec(&with_params("  sel: { select: crossfilter }"), Format::Yaml)
            .expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert_eq!(analysis.legend_bindings.len(), 1);
        assert!(
            !analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::LegendBindingMissing { .. }
                    | ParseWarning::LegendBindingNonSelection { .. }
                    | ParseWarning::LegendBindingNonCrossfilter { .. }
            )),
            "a crossfilter target binds without warnings: {:?}",
            analysis.warnings
        );
    }

    /// lcf F5 (DeadParam symmetry): a selection produced ONLY by a legend
    /// `as:` (no filterBy subscriber anywhere) must not flag DeadParam —
    /// interactor producers already suppress it via their pseudo-subscriber
    /// push, and the produced-params set extends the same courtesy to legend
    /// producers. A genuinely dead param still warns.
    #[test]
    fn lcf_f5_legend_only_produced_selection_is_not_dead() {
        let yaml = r#"
params:
  sel: { select: crossfilter }
  unused: 7
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: grp
    name: scatter
  - legend: color
    for: scatter
    as: $sel
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis succeeds");
        assert!(
            !analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::DeadParam { name } if name == "sel"
            )),
            "a legend-only-produced selection is not dead: {:?}",
            analysis.warnings
        );
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::DeadParam { name } if name == "unused"
            )),
            "a genuinely dead param still warns: {:?}",
            analysis.warnings
        );
    }

    // --- card 0021: highlight consumer bindings (ce-ac02) ---

    /// ce-ac02: a `highlight, by: $sel, opacity:` builds one binding carrying the
    /// parent-plot identity, the consumed selection, and the resolved style; and
    /// the plot's honouring dot becomes a subscriber to `$sel`.
    #[test]
    fn ce_ac02_highlight_binding_and_subscriber() {
        let yaml = r#"
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
    opacity: 0.1
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis ok");
        assert_eq!(analysis.highlight_bindings.len(), 1);
        let b = &analysis.highlight_bindings[0];
        assert_eq!(b.selection, "brush");
        assert_eq!(b.parent_plot.0, "root");
        assert_eq!(b.style.opacity, Some(0.1));
        // The dot honours highlight → it subscribes to `brush` (so a brush change
        // re-queries it with the __bf_selected projection). ce-ac05.
        let subs = analysis
            .selection_subscribers
            .get("brush")
            .expect("brush selection subscribers");
        assert!(
            subs.iter().any(|p| p.0 == "root/plot[0]/mark[dot]"),
            "highlight-bound dot must subscribe to its `by:` selection, got {subs:?}"
        );
    }

    /// ce-ac02: `by:` a value param → `HighlightBindingNonSelection`, no binding.
    #[test]
    fn ce_ac02_highlight_by_value_param_warns_non_selection() {
        let yaml = r#"
params:
  notasel: 5
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: highlight
    by: $notasel
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis ok");
        assert!(analysis.highlight_bindings.is_empty(), "no binding forms");
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::HighlightBindingNonSelection { name } if name == "notasel"
            )),
            "expected HighlightBindingNonSelection, got {:?}",
            analysis.warnings
        );
    }

    /// ce-ac02: `by:` an undeclared / unbound name → `HighlightBindingMissing`.
    #[test]
    fn ce_ac02_highlight_by_unknown_warns_missing() {
        let yaml = r#"
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: highlight
    by: $ghost
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis ok");
        assert!(analysis.highlight_bindings.is_empty());
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::HighlightBindingMissing { name } if name == "ghost"
            )),
            "expected HighlightBindingMissing, got {:?}",
            analysis.warnings
        );
    }

    /// ce-ac09: a highlight on a plot whose mark AGGREGATES in SQL (a heatmap)
    /// is guarded out — `HighlightOnAggregate`, no binding — so the membership
    /// projection can't reference a dropped column and crash at runtime.
    #[test]
    fn ce_ac09_highlight_on_aggregate_mark_guarded() {
        let yaml = r#"
params:
  brush: { select: single }
plot:
  - mark: heatmap
    data: { from: t }
    x: a
    y: b
  - select: intervalXY
    as: $brush
  - select: highlight
    by: $brush
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis ok");
        assert!(
            analysis.highlight_bindings.is_empty(),
            "aggregate-mark highlight forms no binding"
        );
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::HighlightOnAggregate { mark, .. } if mark == "heatmap"
            )),
            "expected HighlightOnAggregate, got {:?}",
            analysis.warnings
        );
    }

    /// FIX C (ce-ac09): a plot mixing a honouring dot with an aggregating heatmap
    /// keeps the DOT's highlight (per-mark guard, matching emit) — the heatmap is
    /// still warned but does not veto the dot. Only the dot subscribes.
    #[test]
    fn ce_ac09_mixed_plot_binds_honouring_mark_warns_aggregate() {
        let yaml = r#"
params:
  brush: { select: single }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - mark: heatmap
    data: { from: t }
    x: a
    y: b
  - select: intervalXY
    as: $brush
  - select: highlight
    by: $brush
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let analysis = analyse_spec(&out.spec).expect("analysis ok");
        assert_eq!(
            analysis.highlight_bindings.len(),
            1,
            "the honouring dot keeps its highlight despite the heatmap sibling"
        );
        assert!(
            analysis.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::HighlightOnAggregate { mark, .. } if mark == "heatmap"
            )),
            "the heatmap is still flagged, got {:?}",
            analysis.warnings
        );
        let subs = analysis.selection_subscribers.get("brush").expect("subs");
        assert!(
            subs.iter().any(|p| p.0 == "root/plot[0]/mark[dot]"),
            "dot subscribes"
        );
        assert!(
            !subs.iter().any(|p| p.0.contains("heatmap")),
            "the aggregate heatmap is NOT a highlight subscriber"
        );
    }
}
