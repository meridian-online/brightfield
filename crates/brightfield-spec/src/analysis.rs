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
use crate::vocab::InputKind;

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

    // Mark data filterBy
    if let Some(ref data) = mark.data {
        if let crate::ast::MarkData::From { filter_by, .. } = data {
            if let Some(pr) = filter_by {
                graph
                    .entry(pr.0.clone())
                    .or_default()
                    .push(ComponentPath(mark_path.clone()));
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

    Ok(SpecAnalysis {
        subscriber_graph,
        dependency_edges,
        topological_order,
        selection_subscribers,
        interactor_bindings,
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
}
