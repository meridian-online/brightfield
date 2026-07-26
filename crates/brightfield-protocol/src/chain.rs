//! Chain contraction: a run of assets that only ever hands off to one another,
//! drawn as the one asset it ends at.
//!
//! # The criterion, and the textbook one it is not
//!
//! [`contract_chains`] merges the edge `u -> v` whenever `u` has exactly one
//! consumer and `v` exactly one producer. That is **not** the strict linear
//! chain — "every interior node has in-degree 1 and out-degree 1" — which is
//! what a first reading of a Sugiyama paper hands you and which removes
//! precisely nothing here: the crosswalk has eight nodes with in-degree 1 and
//! out-degree 1, and no two of them are adjacent, so no strict chain of length
//! two exists to contract. The edge-local criterion finds seven chains absorbing
//! eight nodes on the same graph.
//!
//! The two differ at the ends. A source host feeding one downloaded file is
//! `outdeg(host) == 1 && indeg(file) == 1`, and it is exactly the hand-off the
//! picture gains nothing from drawing twice — but the host has in-degree 0, so
//! the strict rule refuses it.
//!
//! # It absorbs nothing by default, and the default is the point
//!
//! The eight nodes this takes on the crosswalk are the intermediate build
//! artefacts and the hosts they came from — which is precisely the provenance
//! the protocol view exists to show. The geometry is right and the default would
//! be wrong, so this is a transform the *user opens*, exactly like
//! [`explode_ctes`](crate::explode_ctes) and unlike nothing else in this crate:
//! the graph a builder returns is untouched.
//!
//! # The merged node is the tail
//!
//! `u` is absorbed into `v` — the node keeps `v`'s id, label, kind and step
//! whole. The alternative, a joined label (`edgar.parquet → sec_entities`), was
//! measured at **+294 points of canvas width** on the crosswalk, 941 → 1235:
//! past the pane it has to fit, and so a solved vertical scroll traded for a new
//! horizontal one. A chain reads downstream, so the tail is what the run
//! *produced*, and the run's intermediate names stay reachable in the outline,
//! the nav and the inspector, which all walk the uncontracted graph.
//!
//! # Order: this runs LAST
//!
//! The composition is explode → collapse → contract, and the last position is
//! not a preference. [`explode_ctes`](crate::explode_ctes) resolves what a CTE
//! body reads against the *relation-shaped nodes of the graph it is handed*; a
//! contraction that had
//! already absorbed one of those relations leaves the explode nothing to wire
//! from, and the canvas draws a CTE box fed by nothing — the same failure shape
//! as collapsing before exploding, one pass further along. Held by
//! `contracting_before_the_explode_orphans_the_ctes` in the shell's protocol
//! tests, which runs the wrong order and asserts the orphan.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{AssetGraph, AssetId, Edge};

/// One contraction round's result: which node each id was absorbed into.
///
/// A node absent from the map was not absorbed. A node present maps to the
/// **tail** of its chain, transitively resolved — so a three-node chain
/// `a -> b -> c` records `a -> c` and `b -> c`, never `a -> b`.
#[must_use]
pub fn chain_tails(graph: &AssetGraph) -> BTreeMap<AssetId, AssetId> {
    let mut resolved: BTreeMap<AssetId, AssetId> = BTreeMap::new();
    let mut current = graph.clone();
    // Each round strictly removes at least one node, so the node count bounds
    // the iteration; the loop exists because one round is not a fixed point.
    // Contracting two sibling chains into one tail can leave their shared
    // producer with a single deduplicated consumer, which is a chain the round
    // that created it could not see.
    for _ in 0..=graph.nodes.len() {
        let round = contract_once(&current);
        if round.is_empty() {
            break;
        }
        for (absorbed, tail) in &round {
            // Anything already pointing at `absorbed` now points past it.
            for target in resolved.values_mut() {
                if target == absorbed {
                    target.clone_from(tail);
                }
            }
            resolved.insert(absorbed.clone(), tail.clone());
        }
        current = apply(&current, &round);
    }
    resolved
}

/// Contract every chain in `graph`: pure, idempotent, and a no-op on a graph
/// with no chain in it.
///
/// The result is a plain [`AssetGraph`] — it lays out, collapses and renders
/// like any other. Absorbed nodes are gone from `nodes`; every edge that touched
/// one is re-targeted onto that chain's tail, self-edges are dropped and
/// duplicates fold away. `seams` are untouched: a step is not an asset, and the
/// steps sheet reads the same rows whether or not the canvas is contracted.
#[must_use]
pub fn contract_chains(graph: &AssetGraph) -> AssetGraph {
    let tails = chain_tails(graph);
    if tails.is_empty() {
        return graph.clone();
    }
    apply(graph, &tails)
}

/// The unique `(from, to)` pairs a layout would draw, with self-edges and edges
/// naming a missing endpoint dropped — the same filter
/// [`layout`](crate::layout::layout) applies, so "one consumer" here means one
/// consumer *on the canvas*.
fn unique_pairs(graph: &AssetGraph) -> BTreeSet<(AssetId, AssetId)> {
    graph
        .edges
        .iter()
        .filter(|e| {
            e.from != e.to && graph.nodes.contains_key(&e.from) && graph.nodes.contains_key(&e.to)
        })
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect()
}

/// One round: the absorbed → tail map for the chains visible in `graph`.
///
/// `outdeg(u) == 1 && indeg(v) == 1` is a partial matching — `u` has at most one
/// outgoing contractible edge and `v` at most one incoming — so the contractible
/// edges form disjoint simple paths and the walk to each path's tail terminates.
fn contract_once(graph: &AssetGraph) -> BTreeMap<AssetId, AssetId> {
    let pairs = unique_pairs(graph);
    let mut outdeg: BTreeMap<&AssetId, usize> = BTreeMap::new();
    let mut indeg: BTreeMap<&AssetId, usize> = BTreeMap::new();
    for (from, to) in &pairs {
        *outdeg.entry(from).or_default() += 1;
        *indeg.entry(to).or_default() += 1;
    }
    let next: BTreeMap<&AssetId, &AssetId> = pairs
        .iter()
        .filter(|(from, to)| {
            outdeg.get(from).copied().unwrap_or(0) == 1 && indeg.get(to).copied().unwrap_or(0) == 1
        })
        .map(|(from, to)| (from, to))
        .collect();

    let mut tails = BTreeMap::new();
    for start in next.keys() {
        let mut at: &AssetId = start;
        // Bounded by the path's length, which is bounded by the node count.
        for _ in 0..=graph.nodes.len() {
            match next.get(at) {
                Some(step) => at = step,
                None => break,
            }
        }
        tails.insert((*start).clone(), at.clone());
    }
    tails
}

/// Rewrite `graph` with every key of `tails` absorbed into its value.
fn apply(graph: &AssetGraph, tails: &BTreeMap<AssetId, AssetId>) -> AssetGraph {
    let rep = |id: &AssetId| -> AssetId { tails.get(id).unwrap_or(id).clone() };

    let nodes = graph
        .nodes
        .iter()
        .filter(|(id, _)| !tails.contains_key(*id))
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect();

    let mut edges = Vec::new();
    let mut seen: BTreeSet<(AssetId, AssetId, Option<String>, bool)> = BTreeSet::new();
    for edge in &graph.edges {
        let from = rep(&edge.from);
        let to = rep(&edge.to);
        if from == to {
            continue; // wholly inside one chain
        }
        let key = (from.clone(), to.clone(), edge.via.clone(), edge.shield);
        if seen.insert(key) {
            edges.push(Edge {
                from,
                to,
                via: edge.via.clone(),
                shield: edge.shield,
            });
        }
    }

    AssetGraph {
        protocol: graph.protocol.clone(),
        nodes,
        seams: graph.seams.clone(),
        edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{build_graph, AssetKind};
    use crate::manifest::parse_manifest_str;

    /// fetch -> extract -> load: two hand-offs with nowhere else to go.
    const LINE: &str = r"
name: line
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://h.example/a.zip', out: build/a.zip }
  - name: extract
    op: archive_extract@1
    with: { archive: build/a.zip, dest: build/a, members: [p.tsv] }
  - name: load
    sql: models/load.sql
    depends_on: [build/a/p.tsv]
";

    fn line_graph() -> AssetGraph {
        let manifest = parse_manifest_str(LINE).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok("CREATE TABLE loaded AS SELECT * FROM read_csv('build/a/p.tsv');".to_string()),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn a_run_of_single_hand_offs_contracts_to_its_tail() {
        let g = line_graph();
        let c = contract_chains(&g);
        assert!(
            c.nodes.len() < g.nodes.len(),
            "nothing contracted: {} nodes in, {} out",
            g.nodes.len(),
            c.nodes.len()
        );
        // The survivor is the node the run ENDS at, keeping its own identity —
        // no joined label, no synthesised id.
        let sink = "asset.line.loaded";
        assert!(c.nodes.contains_key(sink), "the tail survives");
        assert_eq!(c.nodes[sink].label, g.nodes[sink].label, "label untouched");
        for id in c.nodes.keys() {
            assert!(
                g.nodes.contains_key(id),
                "{id} is a node the contraction invented"
            );
        }
    }

    #[test]
    fn contraction_is_pure_and_idempotent() {
        let g = line_graph();
        let once = contract_chains(&g);
        assert_eq!(once, contract_chains(&g), "pure: same input, same output");
        assert_eq!(once, contract_chains(&once), "idempotent");
    }

    /// A fan-in is not a chain: a node two producers reach keeps both of them.
    #[test]
    fn pds_a_fan_in_is_never_absorbed() {
        let yaml = r"
name: fan
steps:
  - name: fetch_a
    op: http_fetch@1
    with: { url: 'https://h.example/a.parquet', out: build/a.parquet }
  - name: fetch_b
    op: http_fetch@1
    with: { url: 'https://h.example/b.parquet', out: build/b.parquet }
  - name: join
    sql: models/join.sql
    depends_on: [build/a.parquet, build/b.parquet]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "join".to_string(),
            Ok(
                "CREATE TABLE joined AS SELECT * FROM read_parquet('build/a.parquet') \
                JOIN read_parquet('build/b.parquet') USING (k);"
                    .to_string(),
            ),
        );
        let g = build_graph(&manifest, &sources);
        let c = contract_chains(&g);
        // Both inputs still reach the join: the two-producer node is untouched.
        let joined = "asset.fan.joined";
        let feeders: Vec<&AssetId> = c
            .edges
            .iter()
            .filter(|e| e.to == joined)
            .map(|e| &e.from)
            .collect();
        assert_eq!(
            feeders.len(),
            2,
            "a fan-in lost a producer to the contraction: {feeders:?}"
        );
    }

    /// The strict linear chain — the criterion this pass deliberately is not —
    /// removes nothing on a graph this one contracts. Stated as a test so the
    /// next author cannot "simplify" the criterion back to the textbook one and
    /// find every test still green.
    #[test]
    fn pds_the_strict_linear_criterion_would_remove_nothing() {
        // A source host that feeds exactly one downloaded file. The hand-off is
        // as single as a hand-off gets, and the strict rule refuses it purely
        // because the host has no producer of its own.
        let yaml = r"
name: ends
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://h.example/a.parquet', out: build/a.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        let pairs = unique_pairs(&g);
        let mut outdeg: BTreeMap<&AssetId, usize> = BTreeMap::new();
        let mut indeg: BTreeMap<&AssetId, usize> = BTreeMap::new();
        for (from, to) in &pairs {
            *outdeg.entry(from).or_default() += 1;
            *indeg.entry(to).or_default() += 1;
        }
        let interior = |id: &AssetId| {
            indeg.get(id).copied().unwrap_or(0) == 1 && outdeg.get(id).copied().unwrap_or(0) == 1
        };
        let strict = pairs
            .iter()
            .filter(|(from, to)| interior(from) && interior(to))
            .count();
        assert_eq!(
            strict, 0,
            "the strict rule found a chain on this fixture, so the fixture no \
             longer distinguishes the two criteria"
        );
        assert!(
            !chain_tails(&g).is_empty(),
            "...while the shipped criterion finds one"
        );
    }

    /// Nothing to contract, nothing changes — and the graph comes back
    /// unmodified rather than rebuilt into an equal-but-reordered twin.
    #[test]
    fn a_graph_with_no_chain_comes_back_untouched() {
        let yaml = r"
name: none
steps:
  - name: fetch_a
    op: http_fetch@1
    with: { url: 'https://h.example/a.parquet', out: build/a.parquet }
  - name: fetch_b
    op: http_fetch@1
    with: { url: 'https://h.example/b.parquet', out: build/b.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        // Each fetch IS a source -> file hand-off, so this fixture does contract;
        // the assertion is that what survives is only ever the tail.
        let c = contract_chains(&g);
        assert!(
            !c.nodes.values().any(|n| n.kind == AssetKind::Source),
            "a source that feeds exactly one file is absorbed into it"
        );
        // ...and a graph with no edges at all is returned as itself.
        let bare = AssetGraph {
            protocol: "bare".to_string(),
            nodes: g.nodes.clone(),
            seams: g.seams.clone(),
            edges: Vec::new(),
        };
        assert_eq!(contract_chains(&bare), bare);
    }
}
