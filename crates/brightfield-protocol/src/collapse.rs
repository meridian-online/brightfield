//! Parameterised family collapse (card 0025, pds-ac03).
//!
//! Detection: maximal runs of >=2 step *pair* (or longer) cycles whose names
//! share a prefix and differ only in a trailing `_`-separated token —
//! `fetch_ncen_2026q2`/`extract_ncen_2026q2`, `fetch_ncen_2026q1`/... Strip
//! the last token from each step name; consecutive steps whose stripped names
//! form a repeating cycle over the same op sequence, where every step inside
//! one instance shares the SAME trailing token and instances carry DISTINCT
//! tokens, are one parameterised family. Each family folds to one
//! `Family` tile node (label + xN count); member nodes/edges are removed and
//! external edges re-target the tile (the deepest-visible-ancestor rule).
//!
//! The fold is a pure `AssetGraph -> AssetGraph` function — unit-testable
//! without pixels, idempotent (a second application is a no-op).

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{AssetGraph, AssetKind, AssetNode, Edge, Seam, SeamKind, StepId};

/// The cycle-matching signature of a seam: op steps must repeat the same
/// operator; sql/command/opaque steps match their own class.
fn seam_signature(seam: &Seam) -> (u8, &str) {
    match &seam.kind {
        SeamKind::Op { name, .. } => (0, name.as_str()),
        SeamKind::Sql { .. } => (1, ""),
        SeamKind::Command => (2, ""),
        SeamKind::Opaque => (3, ""),
    }
}

/// `fetch_ncen_2026q2` -> `("fetch_ncen", "2026q2")`; `None` for names
/// without a `_`-separated trailing token.
fn strip_tail(name: &str) -> Option<(&str, &str)> {
    name.rsplit_once('_')
}

/// One detected family: the member steps (in manifest order), the cycle's
/// stripped names, and the instance count.
struct Family {
    members: Vec<StepId>,
    cycle_names: Vec<String>,
    count: usize,
}

/// Detect parameterised families over the seams in manifest order.
fn detect_families(ordered: &[&Seam]) -> Vec<Family> {
    let n = ordered.len();
    let mut families = Vec::new();
    let mut i = 0;
    while i < n {
        let mut found: Option<(usize, usize)> = None; // (cycle len k, repeats r)
        // Smallest viable cycle wins; k starts at 2 (a single repeated step
        // is not a *pair* family — `fetch_edgar`/`fetch_gleif` must survive).
        let max_k = (n - i) / 2;
        'k: for k in 2..=max_k {
            // Every step of an instance must strip and share one tail.
            let Some(instance_tail) = common_tail(&ordered[i..i + k]) else {
                continue 'k;
            };
            let pattern: Vec<(String, (u8, String))> = ordered[i..i + k]
                .iter()
                .map(|s| {
                    let (stripped, _) = strip_tail(&s.step).expect("common_tail checked");
                    let (class, op) = seam_signature(s);
                    (stripped.to_string(), (class, op.to_string()))
                })
                .collect();
            let mut tails = vec![instance_tail.to_string()];
            let mut r = 1;
            while i + (r + 1) * k <= n {
                let block = &ordered[i + r * k..i + (r + 1) * k];
                let Some(tail) = common_tail(block) else { break };
                let matches = block.iter().zip(pattern.iter()).all(|(s, (stripped, sig))| {
                    strip_tail(&s.step).is_some_and(|(st, _)| st == stripped) && {
                        let (class, op) = seam_signature(s);
                        (class, op.to_string()) == *sig
                    }
                });
                if !matches {
                    break;
                }
                tails.push(tail.to_string());
                r += 1;
            }
            let distinct: BTreeSet<&String> = tails.iter().collect();
            if r >= 2 && distinct.len() == tails.len() {
                found = Some((k, r));
                break 'k;
            }
        }
        if let Some((k, r)) = found {
            families.push(Family {
                members: ordered[i..i + k * r].iter().map(|s| s.step.clone()).collect(),
                cycle_names: ordered[i..i + k]
                    .iter()
                    .map(|s| strip_tail(&s.step).expect("checked").0.to_string())
                    .collect(),
                count: r,
            });
            i += k * r;
        } else {
            i += 1;
        }
    }
    families
}

/// The tail shared by every step in `block`, if they all strip to one.
fn common_tail<'a>(block: &[&'a Seam]) -> Option<&'a str> {
    let mut tail: Option<&str> = None;
    for seam in block {
        let (_, t) = strip_tail(&seam.step)?;
        match tail {
            None => tail = Some(t),
            Some(prev) if prev == t => {}
            Some(_) => return None,
        }
    }
    tail
}

/// Fold every detected parameterised family to one `Family` tile (pds-ac03).
/// Pure and idempotent: collapsing an already-collapsed graph is a no-op.
#[must_use]
pub fn collapse_families(graph: &AssetGraph) -> AssetGraph {
    let mut ordered: Vec<&Seam> = graph.seams.values().collect();
    ordered.sort_by_key(|s| s.index);
    let families = detect_families(&ordered);
    if families.is_empty() {
        return graph.clone();
    }

    let mut nodes = graph.nodes.clone();
    let mut seams = graph.seams.clone();
    // member step -> its family's tile id.
    let mut member_tile: BTreeMap<StepId, String> = BTreeMap::new();
    for family in &families {
        let tile_id = format!(
            "family.{}.{}",
            graph.protocol,
            family.cycle_names.join("+")
        );
        let anchor_index = seams[&family.members[0]].index;
        for member in &family.members {
            member_tile.insert(member.clone(), tile_id.clone());
            seams.remove(member);
        }
        nodes.retain(|_, n| {
            n.step.as_ref().is_none_or(|s| !member_tile.contains_key(s))
        });
        nodes.insert(
            tile_id.clone(),
            AssetNode {
                id: tile_id.clone(),
                kind: AssetKind::Family,
                label: family.cycle_names.join(" \u{b7} "),
                step: None,
                family_count: Some(family.count),
                issue: None,
            },
        );
        // The tile keeps the family's place in manifest order for any later
        // collapse pass over the folded graph.
        seams.insert(
            tile_id.clone(),
            Seam {
                step: tile_id.clone(),
                index: anchor_index,
                kind: SeamKind::Opaque,
                gate: false,
            },
        );
    }

    // Re-target edges: an endpoint owned by a member step moves to its tile;
    // edges wholly inside a family vanish; duplicates fold away.
    let removed: BTreeSet<&String> = graph
        .nodes
        .iter()
        .filter(|(_, n)| n.step.as_ref().is_some_and(|s| member_tile.contains_key(s)))
        .map(|(id, _)| id)
        .collect();
    let mut edges = Vec::new();
    let mut seen: BTreeSet<(String, String, Option<String>, bool)> = BTreeSet::new();
    for edge in &graph.edges {
        let retarget = |id: &String| -> String {
            if removed.contains(id) {
                let node = &graph.nodes[id];
                let step = node.step.as_ref().expect("removed nodes have steps");
                member_tile[step].clone()
            } else {
                id.clone()
            }
        };
        let from = retarget(&edge.from);
        let to = retarget(&edge.to);
        if from == to {
            continue; // wholly inside the family
        }
        let via = edge.via.clone().filter(|v| !member_tile.contains_key(v));
        let key = (from.clone(), to.clone(), via.clone(), edge.shield);
        if seen.insert(key) {
            edges.push(Edge { from, to, via, shield: edge.shield });
        }
    }

    AssetGraph { protocol: graph.protocol.clone(), nodes, seams, edges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph;
    use crate::manifest::parse_manifest_str;
    use std::collections::BTreeMap;

    /// Two fetch+extract pairs differing only in the trailing quarter token,
    /// followed by a loader that reads both extract dests.
    const FAMILY: &str = r"
name: fam
steps:
  - name: fetch_x_q1
    op: http_fetch@1
    with: { url: 'https://h.example/q1.zip', out: build/z/q1.zip }
  - name: extract_x_q1
    op: archive_extract@1
    with: { archive: build/z/q1.zip, dest: build/x/q1 }
  - name: fetch_x_q2
    op: http_fetch@1
    with: { url: 'https://h.example/q2.zip', out: build/z/q2.zip }
  - name: extract_x_q2
    op: archive_extract@1
    with: { archive: build/z/q2.zip, dest: build/x/q2 }
  - name: load
    sql: models/load.sql
    depends_on: [build/x/q1/p.tsv, build/x/q2/p.tsv]
";

    fn family_graph() -> AssetGraph {
        let manifest = parse_manifest_str(FAMILY).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok("CREATE TABLE loaded AS SELECT * FROM read_csv('build/x/*/p.tsv');".to_string()),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn pds_ac03_pairs_collapse_to_one_tile() {
        let g = collapse_families(&family_graph());
        let tile = &g.nodes["family.fam.fetch_x+extract_x"];
        assert_eq!(tile.kind, AssetKind::Family);
        assert_eq!(tile.family_count, Some(2));
        assert_eq!(tile.label, "fetch_x \u{b7} extract_x");
        // Member-owned nodes (sources, zips, dests) are gone.
        assert!(!g.nodes.contains_key("file.fam.build/z/q1.zip"));
        assert!(!g.nodes.contains_key("file.fam.build/x/q2"));
        assert!(!g.nodes.keys().any(|k| k.starts_with("source.fam.")));
        // External edges re-target the tile, deduplicated: one tile->loaded.
        let tile_out: Vec<&Edge> =
            g.edges.iter().filter(|e| e.from == tile.id).collect();
        assert_eq!(tile_out.len(), 1);
        assert_eq!(tile_out[0].to, "asset.fam.loaded");
        // Member seams folded away.
        assert!(!g.seams.contains_key("fetch_x_q1"));
        assert!(g.seams.contains_key("load"));
    }

    #[test]
    fn pds_ac03_collapse_is_pure_and_idempotent() {
        let g = family_graph();
        let once = collapse_families(&g);
        assert_eq!(once, collapse_families(&g), "pure: same input, same output");
        assert_eq!(once, collapse_families(&once), "idempotent");
    }

    #[test]
    fn pds_ac03_distinct_singles_do_not_collapse() {
        // fetch_edgar/fetch_gleif strip to the same prefix with distinct
        // tails but form no >=2-step cycle — they must survive.
        let yaml = r"
name: nofam
steps:
  - name: fetch_edgar
    op: http_fetch@1
    with: { url: 'https://h.example/e.parquet', out: build/e.parquet }
  - name: fetch_gleif
    op: http_fetch@1
    with: { url: 'https://h.example/g.parquet', out: build/g.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        let collapsed = collapse_families(&g);
        assert_eq!(collapsed, g, "no family, no fold");
    }

    #[test]
    fn pds_ac03_repeated_tails_do_not_collapse() {
        // Instances must carry DISTINCT parameter tokens; a repeated tail is
        // not a parameterised family.
        let yaml = r"
name: rep
steps:
  - name: fetch_x_q1
    op: http_fetch@1
    with: { url: 'https://h.example/1', out: build/a1 }
  - name: extract_x_q1
    op: archive_extract@1
    with: { dest: build/b1 }
  - name: fetch_y_q1
    op: http_fetch@1
    with: { url: 'https://h.example/2', out: build/a2 }
  - name: extract_y_q1
    op: archive_extract@1
    with: { dest: build/b2 }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        // fetch_x/extract_x vs fetch_y/extract_y: stripped names differ, so
        // the cycle does not repeat; tails are all q1 anyway.
        assert_eq!(collapse_families(&g), g);
    }
}
