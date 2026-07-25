//! CTE lineage: the joins INSIDE a SQL step, in graph form — no pixels.
//!
//! Five claims, one per test:
//!
//! 1. the two CTEs of the vendored `sec_entities.sql` become nodes under the
//!    same ids whichever input path built the graph — the offline manifest, or
//!    the emitted Protocol+Run contract;
//! 2. a reference to a CTE is an edge INSIDE the step while a reference to a
//!    real relation crosses assets, and a name declared in more than one scope
//!    resolves to its innermost declaration;
//! 3. a CTE whose body fails to parse degrades alone — its sibling CTEs and its
//!    sibling statements still explode.
//!
//! The last two are asserted against the FOLDED graph — the one the builders
//! return and the shell renders today — because that is where a lost degrade
//! signal would actually be seen:
//!
//! 4. a statement recovered from its `WITH`-stripped form draws its relation
//!    BADGED, never as a clean healthy node;
//! 5. a statement that produces no relation is never promoted out of its chip.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use brightfield_protocol::graph::{build_graph, load_model_sources, AssetGraph, AssetKind};
use brightfield_protocol::{
    contract_sql, cte_id, explode_ctes, load_contract, manifest_sql, parse_manifest_str, AssetNode,
    Manifest,
};

const PROTOCOL: &str = "edgar_gleif";
const SQL_STEP: &str = "build_sec_entities";
const RELATION: &str = "sec_entities";

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

/// The exploded graph from the OFFLINE MANIFEST path: `arcform.yaml` plus its
/// `models/*.sql`, re-parsed.
fn exploded_from_manifest() -> AssetGraph {
    let path = repo_path("examples/protocol/edgar_gleif/arcform.yaml");
    let text = std::fs::read_to_string(&path).expect("the vendored manifest is present");
    let manifest = parse_manifest_str(&text).expect("the vendored manifest parses");
    let sources = load_model_sources(&manifest, path.parent().expect("manifest has a parent"));
    let graph = build_graph(&manifest, &sources);
    explode_ctes(&graph, &manifest_sql(&sources))
}

/// The exploded graph from the EMITTED-CONTRACT path: the hand-authored minimal
/// `b4/1` contract for the same protocol slice, whose sql step carries the same
/// model text.
fn exploded_from_contract() -> AssetGraph {
    let view = load_contract(&fixture("edgar_gleif.contract.json")).expect("the contract loads");
    explode_ctes(&view.graph, &contract_sql(&view))
}

/// Every CTE node of a graph, by id.
fn cte_nodes(graph: &AssetGraph) -> BTreeMap<String, AssetNode> {
    graph
        .nodes
        .iter()
        .filter(|(id, _)| id.starts_with("cte."))
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect()
}

/// Every edge with a CTE node at either end, as `(from, to, via)`.
fn cte_edges(graph: &AssetGraph) -> BTreeSet<(String, String, Option<String>)> {
    graph
        .edges
        .iter()
        .filter(|e| e.from.starts_with("cte.") || e.to.starts_with("cte."))
        .map(|e| (e.from.clone(), e.to.clone(), e.via.clone()))
        .collect()
}

fn has_edge(graph: &AssetGraph, from: &str, to: &str) -> bool {
    graph.edges.iter().any(|e| e.from == from && e.to == to)
}

/// A one-step protocol over in-memory SQL: its parsed manifest and its sources.
fn one_step(name: &str, sql: &str) -> (Manifest, BTreeMap<String, Result<String, String>>) {
    let manifest_text = format!(
        "name: {name}\nengine: duckdb\nsteps:\n  - name: shape\n    sql: models/shape.sql\n"
    );
    let manifest = parse_manifest_str(&manifest_text).expect("the manifest parses");
    let mut sources: BTreeMap<String, Result<String, String>> = BTreeMap::new();
    sources.insert("shape".to_string(), Ok(sql.to_string()));
    (manifest, sources)
}

/// The FOLDED graph of a one-step protocol — exactly what `build_graph` returns
/// and what the shell renders today, with nothing exploded.
fn folded_from_sql(name: &str, sql: &str) -> AssetGraph {
    let (manifest, sources) = one_step(name, sql);
    build_graph(&manifest, &sources)
}

/// Build and explode a one-step protocol from in-memory SQL.
fn exploded_from_sql(name: &str, sql: &str) -> AssetGraph {
    let (manifest, sources) = one_step(name, sql);
    let graph = build_graph(&manifest, &sources);
    explode_ctes(&graph, &manifest_sql(&sources))
}

/// `sec_entities.sql` declares `ck` and `tk`. Both become INTERNAL nodes under
/// `cte.edgar_gleif.sec_entities#…`, and the two input paths agree on the ids,
/// the nodes and the edges those nodes carry.
#[test]
fn sec_entities_ctes_are_internal_nodes_on_both_input_paths() {
    let from_manifest = exploded_from_manifest();
    let from_contract = exploded_from_contract();

    let ck = cte_id(PROTOCOL, RELATION, "ck");
    let tk = cte_id(PROTOCOL, RELATION, "tk");
    assert_eq!(ck, "cte.edgar_gleif.sec_entities#ck");
    assert_eq!(tk, "cte.edgar_gleif.sec_entities#tk");

    for (path, graph) in [("manifest", &from_manifest), ("contract", &from_contract)] {
        for id in [&ck, &tk] {
            let node = graph
                .nodes
                .get(id)
                .unwrap_or_else(|| panic!("{path} path draws {id}"));
            assert_eq!(node.kind, AssetKind::Internal, "{path} path: {id}");
            assert_eq!(node.step.as_deref(), Some(SQL_STEP), "{path} path: {id}");
            assert!(node.issue.is_none(), "{path} path: {id} parsed cleanly");
        }
        assert_eq!(graph.nodes[&ck].label, "ck");
        assert_eq!(graph.nodes[&tk].label, "tk");
    }

    // Identical, not merely both present: same ids, same nodes, same edges.
    assert_eq!(
        cte_nodes(&from_manifest),
        cte_nodes(&from_contract),
        "the two input paths derive the same CTE nodes"
    );
    assert_eq!(
        cte_edges(&from_manifest),
        cte_edges(&from_contract),
        "the two input paths wire the same CTE edges"
    );

    // The lineage the two CTEs exist to show: a file into each CTE, each CTE
    // into the relation the statement produces.
    let target = format!("asset.{PROTOCOL}.{RELATION}");
    for graph in [&from_manifest, &from_contract] {
        assert!(has_edge(
            graph,
            "file.edgar_gleif.build/cik_lookup.txt",
            &ck
        ));
        assert!(has_edge(graph, "file.edgar_gleif.build/edgar.parquet", &tk));
        assert!(has_edge(graph, &ck, &target));
        assert!(has_edge(graph, &tk, &target));
        // The direct file->relation edges are gone: the lineage now runs
        // through the CTEs rather than beside them.
        assert!(!has_edge(
            graph,
            "file.edgar_gleif.build/cik_lookup.txt",
            &target
        ));
        assert!(!has_edge(
            graph,
            "file.edgar_gleif.build/edgar.parquet",
            &target
        ));
    }

    // Deterministic: exploding twice yields the same graph.
    assert_eq!(exploded_from_manifest(), from_manifest);
    assert_eq!(exploded_from_contract(), from_contract);
}

/// A CTE reference is an edge inside the step; a relation reference crosses
/// assets; and a name declared in a nested scope resolves there, so the
/// top-level CTE of the same name gains no edge it did not earn.
#[test]
fn cte_reference_is_intra_step_relation_reference_is_cross_asset_and_inner_scope_wins() {
    let graph = exploded_from_sql(
        "scopes",
        "CREATE OR REPLACE TABLE scoped AS\n\
           WITH ck AS (\n\
             SELECT * FROM source_table\n\
           ),\n\
           tk AS (\n\
             WITH ck AS (SELECT * FROM inner_source)\n\
             SELECT * FROM ck\n\
           )\n\
           SELECT * FROM ck JOIN tk USING (id) JOIN other_table USING (id);\n",
    );

    let ck = cte_id("scopes", "scoped", "ck");
    let tk = cte_id("scopes", "scoped", "tk");
    let target = "asset.scopes.scoped";
    assert_eq!(graph.nodes[&ck].kind, AssetKind::Internal);
    assert_eq!(graph.nodes[&tk].kind, AssetKind::Internal);

    // Intra-step: the main body's references to ck and tk.
    assert!(has_edge(&graph, &ck, target), "ck feeds the statement");
    assert!(has_edge(&graph, &tk, target), "tk feeds the statement");

    // Cross-asset: the relations the CTE bodies read.
    assert!(has_edge(&graph, "asset.scopes.source_table", &ck));
    assert!(has_edge(&graph, "asset.scopes.inner_source", &tk));
    // ...and they no longer short-circuit into the statement's product.
    assert!(!has_edge(&graph, "asset.scopes.source_table", target));
    assert!(!has_edge(&graph, "asset.scopes.inner_source", target));

    // The main body's own non-CTE read keeps its direct edge.
    assert!(has_edge(&graph, "asset.scopes.other_table", target));

    // The innermost declaration wins: `tk` reads ITS OWN `ck`, so there is no
    // edge from the top-level `ck` into `tk`...
    assert!(
        !has_edge(&graph, &ck, &tk),
        "a shadowed name must not draw lineage it does not have: {:?}",
        graph.edges
    );
    // ...and the shadowed name is not fabricated as a relation either.
    assert!(
        !graph.nodes.contains_key("asset.scopes.ck"),
        "a CTE name is never a cross-asset relation"
    );
}

/// One malformed CTE body degrades to an issue-badged chip. Its sibling CTE,
/// the relation its own statement produces, and the sibling STATEMENT all still
/// explode — never a black-boxed statement, never a black-boxed file.
#[test]
fn a_malformed_cte_body_chips_alone() {
    let graph = exploded_from_sql(
        "degraded_cte",
        "CREATE OR REPLACE TABLE degraded AS\n\
           WITH good AS (SELECT * FROM real_table),\n\
                bad AS (SELEC every FORM here IS deliberately unparseable)\n\
           SELECT * FROM good JOIN bad USING (id);\n\
         \n\
         CREATE OR REPLACE TABLE sibling AS SELECT * FROM other_table;\n",
    );

    let good = cte_id("degraded_cte", "degraded", "good");
    let bad = cte_id("degraded_cte", "degraded", "bad");
    let target = "asset.degraded_cte.degraded";

    // The bad body is a chip, and it carries its parse error.
    assert_eq!(graph.nodes[&bad].kind, AssetKind::Opaque);
    assert!(
        graph.nodes[&bad]
            .issue
            .as_ref()
            .is_some_and(|i| !i.is_empty()),
        "the chip carries its degrade reason"
    );

    // The sibling CTE still explodes, with its own cross-asset read.
    assert_eq!(graph.nodes[&good].kind, AssetKind::Internal);
    assert!(graph.nodes[&good].issue.is_none());
    assert!(has_edge(&graph, "asset.degraded_cte.real_table", &good));

    // Both CTEs sit IN the lineage of the relation the statement produces —
    // which itself survived the bad body.
    assert!(graph.nodes.contains_key(target), "the target still exists");
    assert!(has_edge(&graph, &good, target));
    assert!(has_edge(&graph, &bad, target));

    // The statement did not black-box: no whole-statement chip for it.
    assert!(
        !graph.nodes.contains_key("stmt.degraded_cte.shape#0"),
        "a bad CTE body must not degrade the whole statement: {:?}",
        graph.nodes.keys().collect::<Vec<_>>()
    );

    // The sibling STATEMENT still explodes into its own typed lineage.
    assert!(has_edge(
        &graph,
        "asset.degraded_cte.other_table",
        "asset.degraded_cte.sibling"
    ));

    // Exactly one chip in the whole graph: the bad body, nothing else.
    let chips: Vec<&String> = graph
        .nodes
        .iter()
        .filter(|(_, n)| n.kind == AssetKind::Opaque)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(chips, vec![&bad], "one chip, and it is the bad CTE");
}

/// The SAME malformed statement in the FOLDED graph — the one the builders
/// return and the shell renders today, with no explode. Recovering the relation
/// from the stripped statement must not present it as healthy: the node carries
/// the parse error as its issue, because the bad body's reads are unknowable
/// and its lineage is therefore incomplete.
#[test]
fn a_partly_parsed_statement_badges_the_relation_it_produces() {
    let graph = folded_from_sql(
        "degraded_cte",
        "CREATE OR REPLACE TABLE degraded AS\n\
           WITH good AS (SELECT * FROM real_table),\n\
                bad AS (SELEC every FORM here IS deliberately unparseable)\n\
           SELECT * FROM good JOIN bad USING (id);\n\
         \n\
         CREATE OR REPLACE TABLE sibling AS SELECT * FROM other_table;\n",
    );

    let target = "asset.degraded_cte.degraded";
    let node = graph
        .nodes
        .get(target)
        .unwrap_or_else(|| panic!("the folded graph draws {target}: {:?}", graph.nodes.keys()));

    // The signal survives the fold: not a clean, healthy-looking relation.
    let issue = node
        .issue
        .as_ref()
        .expect("a statement that did not parse in full is badged, never drawn healthy");
    assert!(
        issue.contains("partial lineage"),
        "the badge says the lineage is incomplete: {issue}"
    );

    // What DID parse is still lineage: the good body's read reaches the target.
    assert!(has_edge(&graph, "asset.degraded_cte.real_table", target));

    // The sibling statement is untouched, and carries no badge of its own.
    assert!(has_edge(
        &graph,
        "asset.degraded_cte.other_table",
        "asset.degraded_cte.sibling"
    ));
    assert!(graph.nodes["asset.degraded_cte.sibling"].issue.is_none());
}

/// A statement that produces no relation is NEVER promoted out of its chip.
/// `build_graph` draws no node for a targetless statement, so promoting one
/// would delete it from the picture — no node, no chip, no badge. It stays an
/// issue-badged chip in the FOLDED graph, wired like any other degrade.
#[test]
fn a_targetless_statement_with_a_bad_cte_body_still_chips() {
    for sql in [
        "INSERT INTO sink WITH bad AS (SELEC every FORM here IS unparseable) SELECT * FROM bad;",
        "WITH bad AS () SELECT 1;",
    ] {
        let graph = folded_from_sql("targetless", sql);
        let chip = graph
            .nodes
            .get("stmt.targetless.shape#0")
            .unwrap_or_else(|| {
                panic!(
                    "a targetless statement stays a chip, never a silent skip — {sql}: {:?}",
                    graph.nodes.keys().collect::<Vec<_>>()
                )
            });
        assert_eq!(chip.kind, AssetKind::Opaque, "{sql}");
        assert!(
            chip.issue.as_ref().is_some_and(|i| !i.is_empty()),
            "the chip carries its parse error — {sql}"
        );
    }
}
