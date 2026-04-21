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

fn collect_dag_edges(component: &Component, edges: &mut Vec<ParamEdge>) {
    match component {
        Component::Input(inp) => {
            // An input that has as_param (writes to) and also consumes params
            // via filter_by or from creates a dependency edge.
            if let Some(ref target) = inp.as_param {
                if let Some(ref source) = inp.filter_by {
                    edges.push(ParamEdge {
                        from: source.0.clone(),
                        to: target.0.clone(),
                    });
                }
                // Also check options for param refs that create dependencies.
                for v in inp.options.values() {
                    if let ValueOrParamRef::Param(pr) = v {
                        edges.push(ParamEdge {
                            from: pr.0.clone(),
                            to: target.0.clone(),
                        });
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
// SpecAnalysis (ac-09)
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
    /// Diagnostics discovered during analysis.
    pub warnings: Vec<ParseWarning>,
}

/// Run all static analyses on a parsed Spec.
///
/// Returns `Err` if a cycle is detected in the param dependency graph
/// (this is a hard error, not a warning).
pub fn analyse_spec(spec: &Spec) -> Result<SpecAnalysis, ParseError> {
    let subscriber_graph = build_subscriber_graph(spec);

    // Dead param warnings (ac-04).
    let mut warnings: Vec<ParseWarning> = Vec::new();
    for (name, subscribers) in &subscriber_graph {
        if subscribers.is_empty() && spec.params.contains_key(name) {
            warnings.push(ParseWarning::DeadParam {
                name: name.clone(),
            });
        }
    }

    // DAG and topological order (ac-05, ac-06).
    let (dependency_edges, topological_order) = build_dependency_dag(spec)?;

    // Type mismatch warnings (ac-07).
    warnings.extend(check_param_type_mismatches(spec));

    Ok(SpecAnalysis {
        subscriber_graph,
        dependency_edges,
        topological_order,
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
  threshold: 42
plot:
  - mark: dot
    filterBy: $threshold
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
  category: All
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
}
