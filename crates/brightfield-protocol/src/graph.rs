//! The typed asset graph (card 0025): data states are nodes, steps shrink to
//! seams.
//!
//! Derivation rules (interim, from manifest + sql_assets — they swap with the
//! arcform Protocol+Run contract like `manifest.rs` does):
//! - `with.url` => SOURCE node (label = host); `with.out`/`with.dest` => FILE
//!   node (label = path); `with.input` => edge from that relation's node;
//!   `depends_on` entries => edges from the FILE/TABLE node they name
//!   (path-looking => FILE, bare ident => TABLE).
//! - Other string `with:` values that resolve to a produced file (exact path,
//!   ancestor directory, or glob) wire as consumption — this is how
//!   `archive:`/`parquet:`/`edgar:`-style operator args connect without the
//!   parser knowing every operator's vocabulary.
//! - A consumed path resolves to the DEEPEST produced file/directory covering
//!   it (`build/ncen/2026q2/registrant.tsv` resolves to the extract step's
//!   `build/ncen/2026q2` dest; the glob `build/ncen/*/REGISTRANT.tsv`
//!   resolves to every covering dest).
//! - `sql:` steps contribute per-statement produced/consumed sets; the last
//!   producing statement of a relation that is consumed later in the same
//!   file and never referenced by any other step is INTERNAL, otherwise
//!   TABLE (statement-level only — CTE parsing is a later card).
//! - The terminal `parquet_export` whose dest no other step reads (via
//!   `depends_on`/SQL/`with.input` — annotator-style `with:` path references
//!   do not count) re-kinds that FILE node **Dataset**.
//! - `finetype_validate` steps produce NO node: `gate: true` on the seam and
//!   `shield: true` on the edge into the asset named by `with.parquet`
//!   (pds-ac05) — a shield on the lineage, not a box in it.
//!
//! Node ids: `asset.<protocol>.<relation>` / `file.<protocol>.<path>` /
//! `source.<protocol>.<url>` / `stmt.<protocol>.<step>#<n>` (INTERNAL
//! statement intermediates and opaque chips). Everything is
//! `BTreeMap`/`BTreeSet`/`Vec` — deterministic end-to-end (pds-ac06).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::manifest::{Manifest, Step};
use crate::sql_assets::{extract_statement_assets, StatementAssets};

/// What a node IS — the visual class the renderer draws (pds-ac05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssetKind {
    /// External origin (a fetched URL) — outside the build.
    Source,
    /// A file on disk produced or read by a step.
    File,
    /// A durable DuckDB relation other steps read.
    Table,
    /// A statement intermediate consumed only inside its own model file.
    Internal,
    /// The terminal exported artefact — the Protocol's sink.
    Dataset,
    /// A collapsed parameterised step family (one tile for xN instances).
    Family,
    /// A degraded statement (or unreadable model) — an issue-badged chip.
    Opaque,
}

/// Stable node identity (dotted, namespaced by protocol name).
pub type AssetId = String;
/// Step identity — the manifest step name.
pub type StepId = String;

/// One asset node.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetNode {
    /// Stable dotted id (see module docs).
    pub id: AssetId,
    /// Visual class.
    pub kind: AssetKind,
    /// Display label (host / path / relation name).
    pub label: String,
    /// The step that produces this asset (`None` for external inputs).
    pub step: Option<StepId>,
    /// `Some(n)` on a Family tile: the number of collapsed instances.
    pub family_count: Option<usize>,
    /// `Some(error)` on an Opaque chip: the degrade reason (issue badge).
    pub issue: Option<String>,
}

/// What a step's transform IS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeamKind {
    /// A typed catalog operator (`op: name@version`).
    Op {
        /// Operator name.
        name: String,
        /// Operator version (`"?"` when the manifest omits `@version`).
        version: String,
    },
    /// A SQL model step.
    Sql {
        /// Model path as written in the manifest.
        model: String,
    },
    /// An opaque-by-design shell step.
    Command,
    /// A step with none of op/sql/command (renders like command).
    Opaque,
}

/// One step, shrunk to a seam between data states.
#[derive(Debug, Clone, PartialEq)]
pub struct Seam {
    /// The step name.
    pub step: StepId,
    /// Manifest position — collapse detection needs the original order.
    pub index: usize,
    /// Transform class.
    pub kind: SeamKind,
    /// `true` for `finetype_validate` — a gate renders as a shield on its
    /// guarded edge, never as a lineage node.
    pub gate: bool,
}

/// A lineage edge between two assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Upstream asset.
    pub from: AssetId,
    /// Downstream asset.
    pub to: AssetId,
    /// The seam (step) the data flows through.
    pub via: Option<StepId>,
    /// `true` when a validation gate guards this edge (shield badge).
    pub shield: bool,
}

/// The typed asset graph.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetGraph {
    /// Protocol name (the id namespace).
    pub protocol: String,
    /// All nodes, keyed by id (BTreeMap: deterministic iteration).
    pub nodes: BTreeMap<AssetId, AssetNode>,
    /// All seams, keyed by step name.
    pub seams: BTreeMap<StepId, Seam>,
    /// All edges, in derivation order (deterministic from the manifest).
    pub edges: Vec<Edge>,
}

/// Read every `sql:` step's model file relative to the manifest's parent
/// directory. A missing/unreadable model is an `Err` entry — a step-level
/// degrade (opaque chip for that step's SQL), never a parse failure.
#[must_use]
pub fn load_model_sources(
    manifest: &Manifest,
    manifest_dir: &Path,
) -> BTreeMap<StepId, Result<String, String>> {
    manifest
        .steps
        .iter()
        .filter_map(|step| {
            let model = step.sql.as_ref()?;
            let path = manifest_dir.join(model);
            Some((
                step.name.clone(),
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display())),
            ))
        })
        .collect()
}

/// The finetype gate operator — renders as a shield, never a node.
const GATE_OP: &str = "finetype_validate";
/// The export operator whose unread dest is the Dataset sink.
const EXPORT_OP: &str = "parquet_export";

/// Host portion of a URL, for SOURCE labels.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// `depends_on` entries with a path separator name files; bare idents name
/// relations.
fn is_path_like(entry: &str) -> bool {
    entry.contains('/')
}

/// Does produced path `p` (a real file or directory, never a glob) cover
/// consumed path `c` (possibly a glob, possibly a deeper path)? Component-wise:
/// `*` in the consumed path matches one component; a consumed path deeper than
/// `p` is covered when its leading components match `p` as a directory.
fn path_covers(produced: &str, consumed: &str) -> bool {
    let p: Vec<&str> = produced.split('/').collect();
    let c: Vec<&str> = consumed.split('/').collect();
    if c.len() < p.len() {
        return false;
    }
    p.iter()
        .zip(c.iter())
        .all(|(pp, cc)| *cc == "*" || pp.eq_ignore_ascii_case(cc))
}

/// Per-step derivation input, decoded once from the manifest step.
struct StepIr {
    name: StepId,
    index: usize,
    op_name: Option<String>,
    url: Option<String>,
    out_files: Vec<String>,
    input_rels: Vec<String>,
    other_paths: Vec<String>,
    depends: Vec<String>,
    stmts: Vec<StatementAssets>,
    sql_error: Option<String>,
    gate_target: Option<String>,
    seam: Seam,
}

fn decode_step(step: &Step, index: usize, sources: &BTreeMap<StepId, Result<String, String>>) -> StepIr {
    let op_parts = step.op_parts();
    let op_name = op_parts.as_ref().map(|(n, _)| n.clone());
    let seam_kind = if let Some((name, version)) = op_parts {
        SeamKind::Op { name, version }
    } else if let Some(model) = &step.sql {
        SeamKind::Sql { model: model.clone() }
    } else if step.command.is_some() {
        SeamKind::Command
    } else {
        SeamKind::Opaque
    };
    let gate = op_name.as_deref() == Some(GATE_OP);

    let mut url = None;
    let mut out_files = Vec::new();
    let mut input_rels = Vec::new();
    let mut other_paths = Vec::new();
    if let Some(with) = &step.with {
        for (key, value) in with {
            let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
                continue; // nested mappings (headers) / sequences (members) / scalars
            };
            match key {
                "url" => url = Some(value.to_string()),
                "out" | "dest" => out_files.push(value.to_string()),
                "input" => input_rels.push(value.to_string()),
                _ => other_paths.push(value.to_string()),
            }
        }
    }

    let (mut stmts, mut sql_error) = (Vec::new(), None);
    if step.sql.is_some() {
        match sources.get(&step.name) {
            Some(Ok(text)) => stmts = extract_statement_assets(text),
            Some(Err(e)) => sql_error = Some(e.clone()),
            None => sql_error = Some("model source not loaded".to_string()),
        }
    }

    StepIr {
        name: step.name.clone(),
        index,
        op_name,
        url,
        out_files,
        input_rels,
        other_paths,
        depends: step.depends_on.clone(),
        stmts,
        sql_error,
        gate_target: step.with_str("parquet").map(str::to_string),
        seam: Seam { step: step.name.clone(), index, kind: seam_kind, gate },
    }
}

/// Classified identity of a SQL-produced relation.
struct RelInfo {
    id: AssetId,
    kind: AssetKind,
    producer: StepId,
}

/// Classify every SQL-produced relation: INTERNAL when its last producing
/// statement's relation is consumed later within the same file and never
/// referenced by any other step; TABLE otherwise (spec §1.2).
fn classify_relations(proto: &str, irs: &[StepIr]) -> BTreeMap<String, RelInfo> {
    // Last producer wins: (step index, statement index) per relation.
    let mut producers: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for ir in irs {
        for stmt in &ir.stmts {
            if let StatementAssets::Parsed { index, produced: Some(rel), .. } = stmt {
                producers.insert(rel.clone(), (ir.index, *index));
            }
        }
    }

    let mut out = BTreeMap::new();
    for (rel, (step_idx, stmt_idx)) in &producers {
        let consumed_later_same_file = irs[*step_idx].stmts.iter().any(|s| {
            matches!(s, StatementAssets::Parsed { index, consumed_relations, .. }
                if index > stmt_idx && consumed_relations.contains(rel))
        });
        let referenced_elsewhere = irs.iter().filter(|ir| ir.index != *step_idx).any(|ir| {
            ir.input_rels.iter().any(|r| r == rel)
                || ir.depends.iter().any(|d| !is_path_like(d) && d == rel)
                || ir.stmts.iter().any(|s| {
                    matches!(s, StatementAssets::Parsed { consumed_relations, .. }
                        if consumed_relations.contains(rel))
                })
        });
        let internal = consumed_later_same_file && !referenced_elsewhere;
        let step_name = &irs[*step_idx].name;
        let (id, kind) = if internal {
            (format!("stmt.{proto}.{step_name}#{stmt_idx}"), AssetKind::Internal)
        } else {
            (format!("asset.{proto}.{rel}"), AssetKind::Table)
        };
        out.insert(rel.clone(), RelInfo { id, kind, producer: step_name.clone() });
    }
    out
}

/// Mutable state threaded through the node/edge pass.
struct Builder {
    proto: String,
    nodes: BTreeMap<AssetId, AssetNode>,
    edges: Vec<Edge>,
    edge_seen: BTreeSet<(AssetId, AssetId, Option<StepId>)>,
    produced_file: BTreeMap<String, AssetId>,
    /// Asset -> steps that read it via depends_on / SQL / with.input (the
    /// consumption classes that count against Dataset re-kinding).
    strict_consumers: BTreeMap<AssetId, BTreeSet<StepId>>,
}

impl Builder {
    fn ensure_node(&mut self, node: AssetNode) -> AssetId {
        let id = node.id.clone();
        self.nodes.entry(id.clone()).or_insert(node);
        id
    }

    fn push_edge(&mut self, from: &AssetId, to: &AssetId, via: &StepId) {
        if from == to {
            return;
        }
        let key = (from.clone(), to.clone(), Some(via.clone()));
        if self.edge_seen.insert(key) {
            self.edges.push(Edge {
                from: from.clone(),
                to: to.clone(),
                via: Some(via.clone()),
                shield: false,
            });
        }
    }

    /// Resolve a consumed path against produced files: exact match, else ALL
    /// deepest covering produced entries (ancestor dirs / glob matches).
    fn resolve_file_existing(&self, path: &str) -> Vec<AssetId> {
        if let Some(id) = self.produced_file.get(path) {
            return vec![id.clone()];
        }
        let covering: Vec<(&String, &AssetId)> = self
            .produced_file
            .iter()
            .filter(|(p, _)| path_covers(p, path))
            .collect();
        let deepest = covering.iter().map(|(p, _)| p.split('/').count()).max();
        covering
            .into_iter()
            .filter(|(p, _)| Some(p.split('/').count()) == deepest)
            .map(|(_, id)| id.clone())
            .collect()
    }

    /// [`Self::resolve_file_existing`], creating an external FILE node when
    /// nothing produced covers the path (a repo/local input).
    fn resolve_file_or_create(&mut self, path: &str) -> Vec<AssetId> {
        let found = self.resolve_file_existing(path);
        if !found.is_empty() {
            return found;
        }
        let id = self.ensure_node(AssetNode {
            id: format!("file.{}.{path}", self.proto),
            kind: AssetKind::File,
            label: path.to_string(),
            step: None,
            family_count: None,
            issue: None,
        });
        vec![id]
    }

    fn record_strict(&mut self, id: &AssetId, step: &StepId) {
        self.strict_consumers.entry(id.clone()).or_default().insert(step.clone());
    }
}

/// Build the typed asset graph from a parsed manifest plus its model sources
/// (from [`load_model_sources`], or in-memory in tests).
#[must_use]
pub fn build_graph(
    manifest: &Manifest,
    sources: &BTreeMap<StepId, Result<String, String>>,
) -> AssetGraph {
    let proto = manifest.name.clone();
    let irs: Vec<StepIr> = manifest
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| decode_step(s, i, sources))
        .collect();
    let rels = classify_relations(&proto, &irs);

    let mut b = Builder {
        proto: proto.clone(),
        nodes: BTreeMap::new(),
        edges: Vec::new(),
        edge_seen: BTreeSet::new(),
        produced_file: BTreeMap::new(),
        strict_consumers: BTreeMap::new(),
    };

    // Pre-register every produced file + source so consumers can resolve them
    // regardless of manifest ordering.
    for ir in &irs {
        if ir.seam.gate {
            continue;
        }
        for path in &ir.out_files {
            let id = b.ensure_node(AssetNode {
                id: format!("file.{proto}.{path}"),
                kind: AssetKind::File,
                label: path.clone(),
                step: Some(ir.name.clone()),
                family_count: None,
                issue: None,
            });
            b.produced_file.insert(path.clone(), id);
        }
        if let Some(url) = &ir.url {
            b.ensure_node(AssetNode {
                id: format!("source.{proto}.{url}"),
                kind: AssetKind::Source,
                label: host_of(url),
                step: Some(ir.name.clone()),
                family_count: None,
                issue: None,
            });
        }
    }

    // Helper: relation name -> node id (creating an external TABLE node for
    // relations nothing in this manifest produces).
    let resolve_rel = |b: &mut Builder, rel: &str| -> AssetId {
        match rels.get(rel) {
            Some(info) => b.ensure_node(AssetNode {
                id: info.id.clone(),
                kind: info.kind,
                label: rel.to_string(),
                step: Some(info.producer.clone()),
                family_count: None,
                issue: None,
            }),
            None => b.ensure_node(AssetNode {
                id: format!("asset.{proto}.{rel}"),
                kind: AssetKind::Table,
                label: rel.to_string(),
                step: None,
                family_count: None,
                issue: None,
            }),
        }
    };

    // Nodes + edges, step by step in manifest order.
    for ir in &irs {
        if ir.seam.gate {
            continue; // a gate is a shield on an edge, never a node (below)
        }
        if ir.stmts.is_empty() && ir.sql_error.is_none() {
            // op / command / opaque step: consumed set -> produced files.
            let mut consumed: BTreeSet<AssetId> = BTreeSet::new();
            if let Some(url) = &ir.url {
                consumed.insert(format!("source.{proto}.{url}"));
            }
            for rel in &ir.input_rels {
                let id = resolve_rel(&mut b, rel);
                b.record_strict(&id, &ir.name);
                consumed.insert(id);
            }
            for dep in &ir.depends {
                let ids = if is_path_like(dep) {
                    b.resolve_file_or_create(dep)
                } else {
                    vec![resolve_rel(&mut b, dep)]
                };
                for id in ids {
                    b.record_strict(&id, &ir.name);
                    consumed.insert(id);
                }
            }
            for path in &ir.other_paths {
                // Annotator-style operator args: wire only when they resolve
                // to something produced (no node fabrication from config
                // strings) and do NOT count against Dataset re-kinding.
                for id in b.resolve_file_existing(path) {
                    consumed.insert(id);
                }
            }
            for path in &ir.out_files {
                let to = format!("file.{proto}.{path}");
                for from in &consumed {
                    b.push_edge(from, &to, &ir.name);
                }
            }
        } else if let Some(error) = &ir.sql_error {
            // Step-level degrade: the whole model is one opaque chip wired
            // from its depends_on (pds-ac04's file-read sibling).
            let model = match &ir.seam.kind {
                SeamKind::Sql { model } => model.clone(),
                _ => String::new(),
            };
            let chip = b.ensure_node(AssetNode {
                id: format!("stmt.{proto}.{}#0", ir.name),
                kind: AssetKind::Opaque,
                label: format!("{model} (unreadable)"),
                step: Some(ir.name.clone()),
                family_count: None,
                issue: Some(error.clone()),
            });
            for dep in &ir.depends {
                let ids = if is_path_like(dep) {
                    b.resolve_file_or_create(dep)
                } else {
                    vec![resolve_rel(&mut b, dep)]
                };
                for id in ids {
                    b.record_strict(&id, &ir.name);
                    b.push_edge(&id, &chip, &ir.name);
                }
            }
        } else {
            // SQL step: per-statement nodes + edges.
            let mut covered: BTreeSet<AssetId> = BTreeSet::new();
            let mut first_target: Option<AssetId> = None;
            for stmt in &ir.stmts {
                match stmt {
                    StatementAssets::Opaque { index, error, .. } => {
                        // Statement-level degrade: ONLY this statement becomes
                        // an issue-badged chip; siblings still explode.
                        b.ensure_node(AssetNode {
                            id: format!("stmt.{proto}.{}#{index}", ir.name),
                            kind: AssetKind::Opaque,
                            label: format!("statement {}", index + 1),
                            step: Some(ir.name.clone()),
                            family_count: None,
                            issue: Some(error.clone()),
                        });
                    }
                    StatementAssets::Parsed {
                        produced, consumed_relations, consumed_files, ..
                    } => {
                        let target = produced.as_ref().map(|rel| resolve_rel(&mut b, rel));
                        let mut consumed: BTreeSet<AssetId> = BTreeSet::new();
                        for rel in consumed_relations {
                            let id = resolve_rel(&mut b, rel);
                            b.record_strict(&id, &ir.name);
                            consumed.insert(id);
                        }
                        for path in consumed_files {
                            for id in b.resolve_file_or_create(path) {
                                b.record_strict(&id, &ir.name);
                                consumed.insert(id);
                            }
                        }
                        if let Some(target) = &target {
                            for from in &consumed {
                                b.push_edge(from, target, &ir.name);
                            }
                            if first_target.is_none() {
                                first_target = Some(target.clone());
                            }
                        }
                        covered.extend(consumed);
                    }
                }
            }
            // depends_on entries the statements didn't already consume wire
            // into the step's first produced node.
            for dep in &ir.depends {
                let ids = if is_path_like(dep) {
                    b.resolve_file_or_create(dep)
                } else {
                    vec![resolve_rel(&mut b, dep)]
                };
                for id in ids {
                    b.record_strict(&id, &ir.name);
                    if !covered.contains(&id) {
                        if let Some(target) = &first_target {
                            b.push_edge(&id, target, &ir.name);
                        }
                    }
                }
            }
        }
    }

    // Gates: shield the edge into the guarded asset (pds-ac05).
    for ir in &irs {
        if !ir.seam.gate {
            continue;
        }
        let target = ir
            .gate_target
            .as_deref()
            .map(|p| b.resolve_file_existing(p))
            .into_iter()
            .flatten()
            .next();
        if let Some(target) = target {
            for edge in &mut b.edges {
                if edge.to == target {
                    edge.shield = true;
                }
            }
        }
    }

    // Dataset sink: a parquet_export dest no other step reads (strictly).
    for ir in &irs {
        if ir.op_name.as_deref() != Some(EXPORT_OP) {
            continue;
        }
        for path in &ir.out_files {
            let id = format!("file.{proto}.{path}");
            let read_elsewhere = b
                .strict_consumers
                .get(&id)
                .is_some_and(|steps| steps.iter().any(|s| *s != ir.name));
            if !read_elsewhere {
                if let Some(node) = b.nodes.get_mut(&id) {
                    node.kind = AssetKind::Dataset;
                }
            }
        }
    }

    AssetGraph {
        protocol: proto,
        nodes: b.nodes,
        seams: irs.into_iter().map(|ir| (ir.name.clone(), ir.seam)).collect(),
        edges: b.edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest_str;

    /// A minimal fetch -> sql -> export -> gate manifest exercising every
    /// derivation rule without touching the filesystem.
    const MINI: &str = r"
name: mini
steps:
  - name: fetch
    op: http_fetch@1
    with:
      url: https://example.com/data/a.csv
      out: build/a.csv
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
  - name: export
    op: parquet_export@1
    with:
      input: t_out
      dest: build/t.parquet
  - name: validate
    op: finetype_validate@1
    with:
      parquet: build/t.parquet
      schema: schema.json
";

    fn mini_graph() -> AssetGraph {
        let manifest = parse_manifest_str(MINI).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE staging AS SELECT * FROM read_csv('build/a.csv', header=true);\n\
                CREATE TABLE t_out AS SELECT * FROM staging;"
                .to_string()),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn pds_ac05_derivation_covers_every_kind() {
        let g = mini_graph();
        let kind = |id: &str| g.nodes.get(id).map(|n| n.kind);
        assert_eq!(kind("source.mini.https://example.com/data/a.csv"), Some(AssetKind::Source));
        assert_eq!(kind("file.mini.build/a.csv"), Some(AssetKind::File));
        // staging is consumed later in the same file and referenced by no
        // other step -> INTERNAL under a stmt id; t_out is exported -> TABLE.
        assert_eq!(kind("stmt.mini.transform#0"), Some(AssetKind::Internal));
        assert_eq!(kind("asset.mini.t_out"), Some(AssetKind::Table));
        // The export dest is read by nothing -> Dataset sink.
        assert_eq!(kind("file.mini.build/t.parquet"), Some(AssetKind::Dataset));
        // The SOURCE label is the host.
        assert_eq!(g.nodes["source.mini.https://example.com/data/a.csv"].label, "example.com");
    }

    #[test]
    fn pds_ac05_gate_is_a_shield_not_a_node() {
        let g = mini_graph();
        // The gate seam exists and is marked.
        assert!(g.seams["validate"].gate);
        // No node belongs to the validate step.
        assert!(g.nodes.values().all(|n| n.step.as_deref() != Some("validate")));
        // The edge INTO the guarded parquet carries the shield.
        let shielded: Vec<&Edge> =
            g.edges.iter().filter(|e| e.to == "file.mini.build/t.parquet").collect();
        assert!(!shielded.is_empty());
        assert!(shielded.iter().all(|e| e.shield), "guarded edge carries shield: {shielded:?}");
        // No other edge is shielded.
        assert!(g
            .edges
            .iter()
            .filter(|e| e.to != "file.mini.build/t.parquet")
            .all(|e| !e.shield));
    }

    #[test]
    fn pds_lineage_chain_source_to_dataset() {
        let g = mini_graph();
        let has_edge = |from: &str, to: &str| g.edges.iter().any(|e| e.from == from && e.to == to);
        assert!(has_edge("source.mini.https://example.com/data/a.csv", "file.mini.build/a.csv"));
        assert!(has_edge("file.mini.build/a.csv", "stmt.mini.transform#0"));
        assert!(has_edge("stmt.mini.transform#0", "asset.mini.t_out"));
        assert!(has_edge("asset.mini.t_out", "file.mini.build/t.parquet"));
    }

    #[test]
    fn pds_depends_ancestor_resolution_reuses_the_dest_dir_node() {
        let yaml = r"
name: anc
steps:
  - name: extract
    op: archive_extract@1
    with:
      archive: build/z.zip
      dest: build/out/q1
  - name: load
    sql: models/load.sql
    depends_on: [build/out/q1/part.tsv]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok("CREATE TABLE loaded AS SELECT * FROM read_csv('build/out/*/part.tsv');".to_string()),
        );
        let g = build_graph(&manifest, &sources);
        // Neither the deep depends_on path nor the glob fabricates a node —
        // both resolve to the extract step's dest directory.
        assert!(g.nodes.contains_key("file.anc.build/out/q1"));
        assert!(!g.nodes.keys().any(|k| k.contains("part.tsv")));
        assert!(g
            .edges
            .iter()
            .any(|e| e.from == "file.anc.build/out/q1" && e.to == "asset.anc.loaded"));
        // The archive is an annotator-style `with:` path with no producer —
        // no node is fabricated from an unresolvable config string.
        assert!(!g.nodes.contains_key("file.anc.build/z.zip"));
    }

    #[test]
    fn pds_step_level_degrade_is_an_opaque_chip() {
        let yaml = "name: deg\nsteps:\n  - name: broken\n    sql: models/missing.sql\n    depends_on: [in.csv]\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert("broken".to_string(), Err("models/missing.sql: not found".to_string()));
        let g = build_graph(&manifest, &sources);
        let chip = &g.nodes["stmt.deg.broken#0"];
        assert_eq!(chip.kind, AssetKind::Opaque);
        assert!(chip.issue.as_deref().is_some_and(|e| e.contains("not found")));
        assert!(g.edges.iter().any(|e| e.to == "stmt.deg.broken#0"), "depends_on wires the chip");
    }

    #[test]
    fn pds_build_graph_is_deterministic() {
        assert_eq!(mini_graph(), mini_graph());
    }
}
