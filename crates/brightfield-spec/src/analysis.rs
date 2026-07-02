//! Static analysis of param dependencies and subscriber relationships.
//!
//! Pure functions of a parsed [`Spec`] — no I/O, no DuckDB.
//! Produces a [`SpecAnalysis`] containing the subscriber graph, dependency DAG,
//! topological order, and diagnostic warnings.

use std::collections::{HashMap, HashSet, VecDeque};

use indexmap::IndexMap;

use crate::ast::{
    Component, Input, Mark, ParamNode, Spec, SpecValue,
    ValueOrParamRef,
};
use crate::error::ParseError;
use crate::parse::ParseWarning;
use crate::vocab::{InputKind, InteractorKind};

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
                | SpecValue::Param(_) | SpecValue::Expression(_) => ParamDeclaredType::ScalarString,
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
            for v in l.options.values() {
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

    // Mark data filterBy + any params embedded in data extras (e.g. a
    // `filter: "x > $k"` expression, which lowers to a WHERE — card 0014).
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
            for v in extras.values() {
                collect_spec_value_subscribers(v, &mark_path, graph);
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
}

impl BrushKind {
    /// Map an `InteractorKind` to a brushable variant. Returns `None` for
    /// non-brushable kinds (Toggle, Highlight, Nearest*, Pan*, Region, etc.).
    #[must_use]
    pub fn from_interactor_kind(kind: InteractorKind) -> Option<Self> {
        match kind {
            InteractorKind::IntervalX => Some(Self::IntervalX),
            InteractorKind::IntervalY => Some(Self::IntervalY),
            InteractorKind::IntervalXY => Some(Self::IntervalXY),
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
    /// Diagnostics discovered during analysis.
    pub warnings: Vec<ParseWarning>,
}

/// Run all static analyses on a parsed Spec.
///
/// Returns `Err` if a cycle is detected in the param dependency graph
/// or if a filterBy reference is invalid (missing or non-selection param).
pub fn analyse_spec(spec: &Spec) -> Result<SpecAnalysis, ParseError> {
    let subscriber_graph = build_subscriber_graph(spec);

    // Dead param warnings (rpw ac-04).
    let mut warnings: Vec<ParseWarning> = Vec::new();
    for (name, subscribers) in &subscriber_graph {
        if subscribers.is_empty() && spec.params.contains_key(name) {
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

    Ok(SpecAnalysis {
        subscriber_graph,
        dependency_edges,
        topological_order,
        selection_subscribers,
        interactor_bindings,
        brushable_bindings,
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
}
