//! Fixture-level assertions against the vendored edgar_gleif
//! Protocol and the degrade fixture — the acceptance criteria in graph form,
//! no pixels involved.

use std::collections::BTreeMap;
use std::path::PathBuf;

use brightfield_protocol::graph::{build_graph, load_model_sources, AssetGraph, AssetKind};
use brightfield_protocol::layout::{layout, LayoutConfig};
use brightfield_protocol::{collapse_families, parse_manifest_str};

fn fixture_dir(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/protocol").join(rel)
}

fn fixture_graph(manifest_rel: &str) -> AssetGraph {
    let path = fixture_dir(manifest_rel);
    let text = std::fs::read_to_string(&path).expect("fixture manifest present");
    let manifest = parse_manifest_str(&text).expect("fixture manifest parses");
    let dir = path.parent().expect("manifest has a parent dir").to_path_buf();
    let sources = load_model_sources(&manifest, &dir);
    build_graph(&manifest, &sources)
}

/// Find the node for a relation label produced by `step`.
fn rel_node<'g>(g: &'g AssetGraph, step: &str, label: &str) -> &'g brightfield_protocol::AssetNode {
    g.nodes
        .values()
        .find(|n| n.label == label && n.step.as_deref() == Some(step))
        .unwrap_or_else(|| panic!("node {label} produced by {step} exists"))
}

#[test]
fn pds_ac02_tier_sql_explodes_into_a_statement_chain() {
    let g = fixture_graph("edgar_gleif/arcform.yaml");
    // tier.sql's four statements: authoritative -> authoritative_d ->
    // probabilistic -> crosswalk_edges. Intermediates are INTERNAL (consumed
    // later in the same file, referenced by no other step); the terminal
    // statement's relation is TABLE.
    let auth = rel_node(&g, "tier", "authoritative");
    let auth_d = rel_node(&g, "tier", "authoritative_d");
    let prob = rel_node(&g, "tier", "probabilistic");
    let edges_out = rel_node(&g, "tier", "crosswalk_edges");
    assert_eq!(auth.kind, AssetKind::Internal);
    assert_eq!(auth_d.kind, AssetKind::Internal);
    assert_eq!(prob.kind, AssetKind::Internal);
    assert_eq!(edges_out.kind, AssetKind::Table);
    assert_eq!(edges_out.id, "asset.edgar_gleif.crosswalk_edges");
    // The chain is connected in order.
    let has_edge = |from: &str, to: &str| g.edges.iter().any(|e| e.from == from && e.to == to);
    assert!(has_edge(&auth.id, &auth_d.id), "authoritative -> authoritative_d");
    assert!(has_edge(&auth_d.id, &prob.id), "authoritative_d -> probabilistic (NOT EXISTS ref)");
    assert!(has_edge(&auth_d.id, &edges_out.id), "authoritative_d -> crosswalk_edges");
    assert!(has_edge(&prob.id, &edges_out.id), "probabilistic -> crosswalk_edges");
}

#[test]
fn pds_ac03_ncen_family_collapses_to_one_tile() {
    let g = collapse_families(&fixture_graph("edgar_gleif/arcform.yaml"));
    // The four fetch+extract pairs (8 steps) fold to one partitioned tile.
    let tile = &g.nodes["family.edgar_gleif.fetch_ncen+extract_ncen"];
    assert_eq!(tile.kind, AssetKind::Family);
    assert_eq!(tile.family_count, Some(4));
    // Member-owned nodes are gone (zips, quarter dirs, their sec.gov sources).
    assert!(!g.nodes.keys().any(|k| k.contains("ncen_zips")));
    assert!(!g.nodes.keys().any(|k| k.contains("build/ncen/20")));
    assert!(!g.seams.contains_key("fetch_ncen_2026q2"));
    assert!(!g.seams.contains_key("extract_ncen_2025q3"));
    // External edges re-target the tile: the load step's N-CEN tables now
    // read from the tile, deduplicated across the four quarters.
    let reg = rel_node(&g, "load", "ncen_registrant");
    let ser = rel_node(&g, "load", "ncen_series");
    let tile_to_reg: Vec<_> =
        g.edges.iter().filter(|e| e.from == tile.id && e.to == reg.id).collect();
    let tile_to_ser: Vec<_> =
        g.edges.iter().filter(|e| e.from == tile.id && e.to == ser.id).collect();
    assert_eq!(tile_to_reg.len(), 1, "4 quarter dirs fold to ONE tile edge");
    assert_eq!(tile_to_ser.len(), 1);
    // The un-parameterised fetches survive untouched.
    assert!(g.nodes.contains_key("file.edgar_gleif.build/edgar.parquet"));
    assert!(g.nodes.contains_key("file.edgar_gleif.build/gleif.parquet"));
    assert!(g.seams.contains_key("fetch_edgar"));
}

#[test]
fn pds_ac04_degrade_fixture_chips_only_the_bad_statement() {
    let g = fixture_graph("degrade.yaml");
    // The middle statement is an opaque, issue-badged chip...
    let chip = &g.nodes["stmt.degrade.transform#1"];
    assert_eq!(chip.kind, AssetKind::Opaque);
    assert!(chip.issue.is_some(), "the chip carries its parse error");
    // ...while both siblings still explode: staged (internal intermediate)
    // and cleaned (exported -> table), still connected.
    let staged = rel_node(&g, "transform", "staged");
    let cleaned = rel_node(&g, "transform", "cleaned");
    assert_eq!(staged.kind, AssetKind::Internal);
    assert_eq!(cleaned.kind, AssetKind::Table);
    assert!(g.edges.iter().any(|e| e.from == staged.id && e.to == cleaned.id));
    // Never a black-boxed file: exactly one chip for the whole model.
    let chips = g.nodes.values().filter(|n| n.kind == AssetKind::Opaque).count();
    assert_eq!(chips, 1);
}

#[test]
fn pds_ac05_fixture_kinds_gate_and_sink() {
    let g = fixture_graph("edgar_gleif/arcform.yaml");
    // SOURCE nodes carry host labels.
    assert!(g
        .nodes
        .values()
        .any(|n| n.kind == AssetKind::Source && n.label == "www.sec.gov"));
    // Intermediate exports stay FILE (read downstream)...
    assert_eq!(
        g.nodes["file.edgar_gleif.build/sec_entities.parquet"].kind,
        AssetKind::File
    );
    // ...the terminal export is the one Dataset sink.
    let sink = &g.nodes["file.edgar_gleif.build/edgar_gleif.parquet"];
    assert_eq!(sink.kind, AssetKind::Dataset);
    let datasets = g.nodes.values().filter(|n| n.kind == AssetKind::Dataset).count();
    assert_eq!(datasets, 1, "exactly one Dataset sink");
    // The finetype gate is a shield on the edge into the sink, not a node.
    assert!(g.seams["validate"].gate);
    assert!(g.nodes.values().all(|n| n.step.as_deref() != Some("validate")));
    let into_sink: Vec<_> = g.edges.iter().filter(|e| e.to == sink.id).collect();
    assert!(!into_sink.is_empty());
    assert!(into_sink.iter().all(|e| e.shield), "the guarded edge carries the shield");
    // The long-running resolve op wires from both exported parquets.
    assert!(g.edges.iter().any(|e| e.from == "file.edgar_gleif.build/sec_entities.parquet"
        && e.to == "file.edgar_gleif.build/resolved.parquet"));
    assert!(g.edges.iter().any(|e| e.from == "file.edgar_gleif.build/gleif.parquet"
        && e.to == "file.edgar_gleif.build/resolved.parquet"));
}

#[test]
fn pds_ac06_fixture_layout_is_deterministic() {
    // Parse -> graph -> collapse -> layout TWICE from scratch: identical
    // coordinates (BTreeMap ordering end-to-end, fixed median sweeps).
    let cfg = LayoutConfig::default();
    let a = layout(&collapse_families(&fixture_graph("edgar_gleif/arcform.yaml")), &cfg);
    let b = layout(&collapse_families(&fixture_graph("edgar_gleif/arcform.yaml")), &cfg);
    assert_eq!(a, b);
    assert!(a.positions.len() > 10, "the fixture lays out a real graph");
}

#[test]
fn pds_fixture_parses_all_21_steps_with_no_step_degrades() {
    let path = fixture_dir("edgar_gleif/arcform.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    let manifest = parse_manifest_str(&text).unwrap();
    assert_eq!(manifest.steps.len(), 21, "the vendored fixture's step count");
    let sources = load_model_sources(&manifest, path.parent().unwrap());
    assert_eq!(sources.len(), 4, "four SQL models");
    assert!(sources.values().all(Result::is_ok), "all models readable");
    let g = build_graph(&manifest, &sources);
    // Every statement in every model parses — no opaque chips in the real
    // fixture (the degrade fixture owns that path).
    assert_eq!(g.nodes.values().filter(|n| n.kind == AssetKind::Opaque).count(), 0);
    let mut sources2 = BTreeMap::new();
    sources2.extend(sources);
    assert_eq!(g, build_graph(&manifest, &sources2), "graph build is deterministic");
}
