//! The typed asset graph: data states are nodes, steps shrink to
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
//! - A `parquet_export` dest re-kinds to **Dataset** (the sink) when no other
//!   step reads it via `depends_on`/SQL/`with.input` AND it does not feed —
//!   through any edge, including annotator-style `with:` op consumption —
//!   another export dest. That second clause is what keeps a mid-pipeline
//!   export consumed by a later op via a named `with:` key (splink's `edgar:`)
//!   from being mistaken for the terminal, while a describe/validate read of
//!   the terminal (a leaf sidecar) leaves it the sink.
//! - `finetype_validate` steps produce NO node: `gate: true` on the seam and
//!   `shield: true` on the edge into the asset named by `with.parquet`
//!   — a shield on the lineage, not a box in it.
//!
//! Node ids: `asset.<protocol>.<relation>` / `file.<protocol>.<path>` /
//! `source.<protocol>.<url>` / `stmt.<protocol>.<step>#<n>` (INTERNAL
//! statement intermediates and opaque chips) / `stmt.<protocol>.<step>#<n>!partial`
//! (the chip drawn BESIDE a statement recovered from its `WITH`-stripped form,
//! whose lineage is real but incomplete). Everything is
//! `BTreeMap`/`BTreeSet`/`Vec` — deterministic end-to-end.
//!
//! **A degraded graph says so, and says which kind.** A step-level degrade's
//! chip is labelled `<class>: <model>` — the class first, because the renderer
//! fits a label to the chip's width and elides the tail. [`Degradation`] is
//! the class set and [`degrades`] enumerates them off a built graph. That
//! enumeration is what a run with nobody watching reads instead of diffing
//! pictures: `brightfield-shell`'s `degrade_report` turns it into stderr lines
//! and the boot summary's degrade count, on the path
//! `crates/brightfield-shell/src/bin/brightfield-shot.rs` takes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::manifest::{Manifest, Step, StepExt};
use crate::sql_assets::{extract_statement_assets, StatementAssets};

/// What a node IS — the visual class the renderer draws.
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

/// Why a node stands in for something the derivation could not produce.
///
/// The distinction the first three draw is the point of the type. *Absent* and
/// *unreadable* are one shape to this code — a `sql:` step with no source — and
/// two different problems for whoever is looking at the chart. One is a
/// protocol that was authored naming a model that is not there; the other is a
/// permission, an ACL or a sandbox grant, and the render comes back whole once
/// it is widened. A chip that cannot tell them apart sends someone to fix the
/// wrong thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Degradation {
    /// A `sql:` step's model file is not on disk, and the directory holding it
    /// could be read to establish that. The protocol's author's to fix.
    ModelAbsent,
    /// A `sql:` step's model file is there and could not be opened — a
    /// permission, an ACL, a parent directory with no search bit, or a sandbox
    /// grant that does not reach it. The reader's to fix.
    ModelUnreadable,
    /// A `sql:` step's model source did not arrive and no filesystem verdict
    /// says why: a caller-built source map that omits it, or an I/O failure of
    /// neither class above.
    ModelUnavailable,
    /// A degrade of another kind — a statement that did not parse, a step that
    /// declares no output asset, a lineage recovered only in part.
    Other,
}

impl Degradation {
    /// The word this class writes at the HEAD of a chip label and of the
    /// degrade text.
    ///
    /// At the head, not the tail, for a measured reason: the renderer sizes a
    /// card from its label's char count and then fits the label to that width
    /// (`node_width` here, `fit_label` in
    /// `crates/brightfield-render/src/asset_scene.rs`), and the count is capped
    /// — so a marker after a model path longer than the cap is elided before it
    /// reaches a pixel.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::ModelAbsent => "absent",
            Self::ModelUnreadable => "unreadable",
            Self::ModelUnavailable => "unavailable",
            Self::Other => "degraded",
        }
    }

    /// What the class means for the reader, appended to the degrade text the
    /// inspector shows. The chip says which class; this says whose problem it
    /// is and what makes it go away.
    #[must_use]
    pub const fn guidance(self) -> &'static str {
        match self {
            Self::ModelAbsent => {
                " — the protocol names a model file that is not there, so this step is drawn as \
                 one chip in place of the statements inside it."
            }
            Self::ModelUnreadable => {
                " — the file is there and could not be opened. Access, not authorship: a \
                 permission, an ACL, a parent directory with no search bit, or a sandbox grant \
                 that does not reach it. The statements are drawn again once the read succeeds."
            }
            Self::ModelUnavailable => {
                " — no source arrived for this model and no filesystem verdict says why, so this \
                 step is drawn as one chip in place of the statements inside it."
            }
            Self::Other => "",
        }
    }
}

/// Read back the class a tagged degrade text opens with, plus the rest of it.
/// `None` when the text carries no class tag, which is how a degrade raised
/// somewhere other than the model read reaches [`Degradation::Other`].
///
/// The class travels as a leading token because the source map's error type is
/// a plain `String` shared with callers that build the map themselves (a start
/// whose models are compiled in, a test with sources in memory), and widening
/// that type would reach across crates for a distinction only this file draws.
fn tagged(text: &str) -> Option<(Degradation, &str)> {
    let (head, rest) = text.split_once(": ")?;
    let class = match head {
        "absent" => Degradation::ModelAbsent,
        "unreadable" => Degradation::ModelUnreadable,
        "unavailable" => Degradation::ModelUnavailable,
        "degraded" => Degradation::Other,
        _ => return None,
    };
    Some((class, rest))
}

/// One node that stands in for something the derivation could not produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Degrade {
    /// The id of the issue-badged node.
    pub node: AssetId,
    /// The step it belongs to, where the derivation knows one.
    pub step: Option<StepId>,
    /// What class of problem produced it.
    pub class: Degradation,
    /// The full reason, exactly as the inspector shows it.
    pub detail: String,
}

/// Every degrade in `graph`, in node-id order.
///
/// **The channel for a run with nobody watching.** An empty return says the
/// graph is everything the protocol asked for; a non-empty one names each node
/// that stands in for something that could not be derived, with the class that
/// says whether the cause is the protocol or the access to it. A degraded
/// render is a picture that looks finished, so the thing that distinguishes it
/// from a complete one must not itself be the picture.
///
/// That claim is only worth the call site behind it: `brightfield-shell`'s
/// `degrade_report` is the caller, reached on every `--spec` boot of a protocol
/// manifest, and `protocol_degrade_channel` in `brightfield-shell` holds the
/// shipped binary to it. This function existing is not the same as it running,
/// and for one round of this work it did not run anywhere but a test.
#[must_use]
pub fn degrades(graph: &AssetGraph) -> Vec<Degrade> {
    graph
        .nodes
        .values()
        .filter_map(|node| {
            let issue = node.issue.as_ref()?;
            Some(Degrade {
                node: node.id.clone(),
                step: node.step.clone(),
                class: tagged(issue).map_or(Degradation::Other, |(class, _)| class),
                detail: issue.clone(),
            })
        })
        .collect()
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

/// Which class of failure a model read hit.
///
/// Drawn here, at the read, because this is the last place that still holds
/// both the `io::Error` and the path — by the time the failure is a string in
/// the source map the two cases are indistinguishable.
///
/// `NotFound` is not taken at face value. A refusal can surface as `NotFound`
/// rather than `PermissionDenied` when something between the process and the
/// file hides its existence, so absence is claimed only when the entry really
/// is not there AND the directory holding it could be opened to establish
/// that. Everything else falls to `ModelUnreadable`, the verdict that sends a
/// reader to look at access rather than at the protocol.
fn classify_read_failure(path: &Path, error: &std::io::Error) -> Degradation {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => Degradation::ModelUnreadable,
        std::io::ErrorKind::NotFound => {
            // The entry is there after all: the read was refused, not missed.
            if path.symlink_metadata().is_ok() {
                return Degradation::ModelUnreadable;
            }
            // The directory could not be listed, so "not there" is unfounded.
            let parent_refused = path.parent().is_some_and(|dir| {
                !dir.as_os_str().is_empty()
                    && std::fs::read_dir(dir)
                        .err()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
            });
            if parent_refused {
                Degradation::ModelUnreadable
            } else {
                Degradation::ModelAbsent
            }
        }
        _ => Degradation::ModelUnavailable,
    }
}

/// Read every `sql:` step's model file relative to the manifest's parent
/// directory. A missing/unreadable model is an `Err` entry — a step-level
/// degrade (opaque chip for that step's SQL), never a parse failure.
///
/// The `Err` string opens with the [`Degradation`] tag, so the class the read
/// established survives into [`build_graph`], which no longer has the path or
/// the `io::Error` to establish it from.
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
            let source = std::fs::read_to_string(&path).map_err(|e| {
                format!(
                    "{}: {}: {e}",
                    classify_read_failure(&path, &e).tag(),
                    path.display()
                )
            });
            Some((step.name.clone(), source))
        })
        .collect()
}

/// The finetype gate operator — renders as a shield, never a node. Shared with
/// the contract path so both inputs recognise the same gate vocabulary.
pub(crate) const GATE_OP: &str = "finetype_validate";
/// The export operator whose unread dest is the Dataset sink. Shared with the
/// contract path so both inputs derive the terminal sink identically.
pub(crate) const EXPORT_OP: &str = "parquet_export";

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

fn decode_step(
    step: &Step,
    index: usize,
    sources: &BTreeMap<StepId, Result<String, String>>,
) -> StepIr {
    let op_parts = step.op_parts();
    let op_name = op_parts.as_ref().map(|(n, _)| n.clone());
    let seam_kind = if let Some((name, version)) = op_parts {
        SeamKind::Op { name, version }
    } else if let Some(model) = &step.sql {
        SeamKind::Sql {
            model: model.clone(),
        }
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
    if let Some(with) = step.with.as_ref().and_then(serde_yaml::Value::as_mapping) {
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
        seam: Seam {
            step: step.name.clone(),
            index,
            kind: seam_kind,
            gate,
        },
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
            if let StatementAssets::Parsed {
                index,
                produced: Some(rel),
                ..
            } = stmt
            {
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
            (
                format!("stmt.{proto}.{step_name}#{stmt_idx}"),
                AssetKind::Internal,
            )
        } else {
            (format!("asset.{proto}.{rel}"), AssetKind::Table)
        };
        out.insert(
            rel.clone(),
            RelInfo {
                id,
                kind,
                producer: step_name.clone(),
            },
        );
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
        // (a) Produced entries that COVER the consumed path (a producer
        //     dir/glob of a deeper consumed path) — keep the deepest producer.
        let covering: Vec<(&String, &AssetId)> = self
            .produced_file
            .iter()
            .filter(|(p, _)| path_covers(p, path))
            .collect();
        if !covering.is_empty() {
            let deepest = covering.iter().map(|(p, _)| p.split('/').count()).max();
            return covering
                .into_iter()
                .filter(|(p, _)| Some(p.split('/').count()) == deepest)
                .map(|(_, id)| id.clone())
                .collect();
        }
        // (b) The consumed path is an ANCESTOR directory of produced dests: it
        //     names every producer beneath it (a `depends_on: [build/ncen]`
        //     over extract steps that each dest `build/ncen/<q>`). Wire EVERY
        //     producer beneath it that is not itself nested under a SHALLOWER
        //     producer already in the set (per-subtree "shallowest") — never
        //     fabricate a bare dir node. A single global min-depth filter would
        //     silently drop deeper INDEPENDENT producers (`build/ncen/2023/q2`
        //     alongside `build/ncen/q1`), erasing their lineage edge.
        let under: Vec<(&String, &AssetId)> = self
            .produced_file
            .iter()
            .filter(|(p, _)| path_covers(path, p))
            .collect();
        under
            .iter()
            .filter(|(p, _)| {
                !under
                    .iter()
                    .any(|(q, _)| q.as_str() != p.as_str() && path_covers(q, p))
            })
            .map(|&(_, id)| id.clone())
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
        self.strict_consumers
            .entry(id.clone())
            .or_default()
            .insert(step.clone());
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
            if ir.out_files.is_empty() {
                // An op/command/opaque step that declares no output asset
                // (out/dest) has no data-state node to hang its seam on — the
                // interim contract can't type its effect. Surface it as an
                // issue-badged chip wired from its inputs, never a silent drop
                // from the DAG.
                let chip = b.ensure_node(AssetNode {
                    id: format!("stmt.{proto}.{}#effect", ir.name),
                    kind: AssetKind::Opaque,
                    label: ir.name.clone(),
                    step: Some(ir.name.clone()),
                    family_count: None,
                    issue: Some("step declares no output asset".to_string()),
                });
                for from in &consumed {
                    b.push_edge(from, &chip, &ir.name);
                }
            }
        } else if let Some(error) = &ir.sql_error {
            // Step-level degrade: the whole model is one opaque chip wired
            // from its depends_on (the file-read sibling).
            //
            // The chip LEADS with the fault class, so a reader learns from the
            // canvas alone whether the model was never there or was refused —
            // the second is theirs to fix and the first is not. An untagged
            // error is a source map a caller built without touching a
            // filesystem, so no verdict on the file is available to state.
            let (class, detail) =
                tagged(error).unwrap_or((Degradation::ModelUnavailable, error.as_str()));
            let model = match &ir.seam.kind {
                SeamKind::Sql { model } => model.clone(),
                _ => String::new(),
            };
            let chip = b.ensure_node(AssetNode {
                id: format!("stmt.{proto}.{}#0", ir.name),
                kind: AssetKind::Opaque,
                label: format!("{}: {model}", class.tag()),
                step: Some(ir.name.clone()),
                family_count: None,
                issue: Some(format!("{}: {detail}{}", class.tag(), class.guidance())),
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
                        let chip = b.ensure_node(AssetNode {
                            id: format!("stmt.{proto}.{}#{index}", ir.name),
                            kind: AssetKind::Opaque,
                            label: format!("statement {}", index + 1),
                            step: Some(ir.name.clone()),
                            family_count: None,
                            issue: Some(error.clone()),
                        });
                        // Wire the chip from the step's depends_on (like the
                        // step-level degrade) so a failed MIDDLE statement sits
                        // inside its lineage, not adrift in the input column.
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
                    }
                    StatementAssets::Parsed {
                        index,
                        produced,
                        consumed_relations,
                        consumed_files,
                        degraded,
                        ..
                    } => {
                        // A targetless statement (INSERT/COPY/UPDATE — no
                        // CREATE TABLE/VIEW) produces no relation node in the
                        // interim contract; do NOT fabricate orphan consumed
                        // nodes with no edge to hang them on (DML lineage is a
                        // later card).
                        let Some(target) = produced.as_ref().map(|rel| resolve_rel(&mut b, rel))
                        else {
                            continue;
                        };
                        // A statement recovered from its WITH-stripped form is
                        // real but INCOMPLETE — whatever its malformed body
                        // read is unknowable and has no edge here. Two things
                        // carry that, because they are read in different
                        // places:
                        //
                        // - the produced node gets the reason as its `issue`,
                        //   which is what an inspector reads. First badge wins,
                        //   so the reason is stable when several statements
                        //   share a relation;
                        // - an issue-badged CHIP is drawn feeding that node,
                        //   because a renderer keys a tile's treatment on
                        //   `kind` — a badged Table still paints as an ordinary
                        //   healthy table. Without the chip the only on-canvas
                        //   trace of the defect is gone, which is LESS than the
                        //   whole-statement chip this recovery replaced. The
                        //   incomplete lineage must be visible without
                        //   selecting anything.
                        if let Some(reason) = degraded {
                            let detail = format!(
                                "partial lineage: this statement did not parse in full, so \
                                 anything its malformed CTE body reads is missing from the \
                                 graph. {reason}"
                            );
                            if let Some(node) = b.nodes.get_mut(&target) {
                                if node.issue.is_none() {
                                    node.issue = Some(detail.clone());
                                }
                            }
                            let chip = b.ensure_node(AssetNode {
                                id: format!("stmt.{proto}.{}#{index}!partial", ir.name),
                                kind: AssetKind::Opaque,
                                label: format!("statement {} (partial)", index + 1),
                                step: Some(ir.name.clone()),
                                family_count: None,
                                issue: Some(detail),
                            });
                            b.push_edge(&chip, &target, &ir.name);
                        }
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
                        for from in &consumed {
                            b.push_edge(from, &target, &ir.name);
                        }
                        if first_target.is_none() {
                            first_target = Some(target.clone());
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

    // Gates: shield the edge into the guarded asset.
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

    // Dataset sink: a parquet_export dest that (a) no OTHER step reads via
    // depends_on/SQL/with.input AND (b) does not feed — through any edge,
    // including annotator-style op consumption — another parquet_export dest.
    // (b) is what stops a mid-pipeline export consumed by a later op through a
    // named `with:` key (splink's `edgar:`) from being mistaken for the
    // terminal even when nothing depends_on it; a describe/validate read of the
    // terminal produces only a leaf sidecar, so the terminal still qualifies.
    let export_dests: BTreeSet<AssetId> = irs
        .iter()
        .filter(|ir| ir.op_name.as_deref() == Some(EXPORT_OP))
        .flat_map(|ir| ir.out_files.iter().map(|p| format!("file.{proto}.{p}")))
        .collect();
    let mut forward: BTreeMap<AssetId, BTreeSet<AssetId>> = BTreeMap::new();
    for edge in &b.edges {
        forward
            .entry(edge.from.clone())
            .or_default()
            .insert(edge.to.clone());
    }
    let feeds_another_export = |start: &AssetId| -> bool {
        let mut stack = vec![start.clone()];
        let mut seen: BTreeSet<AssetId> = BTreeSet::new();
        while let Some(node) = stack.pop() {
            for succ in forward.get(&node).into_iter().flatten() {
                if succ != start && export_dests.contains(succ) {
                    return true;
                }
                if seen.insert(succ.clone()) {
                    stack.push(succ.clone());
                }
            }
        }
        false
    };
    for ir in &irs {
        if ir.op_name.as_deref() != Some(EXPORT_OP) {
            continue;
        }
        for path in &ir.out_files {
            let id = format!("file.{proto}.{path}");
            let read_strictly = b
                .strict_consumers
                .get(&id)
                .is_some_and(|steps| steps.iter().any(|s| *s != ir.name));
            if !read_strictly && !feeds_another_export(&id) {
                if let Some(node) = b.nodes.get_mut(&id) {
                    node.kind = AssetKind::Dataset;
                }
            }
        }
    }

    AssetGraph {
        protocol: proto,
        nodes: b.nodes,
        seams: irs
            .into_iter()
            .map(|ir| (ir.name.clone(), ir.seam))
            .collect(),
        edges: b.edges,
    }
}

/// The **local neighbourhood** of `focus` in `graph`: the focused node plus its
/// direct producers and consumers (one hop each way). The drill scope — `Enter`
/// focuses the canvas on this induced slice, `Esc` pops back out. An id not in
/// the graph yields an empty set.
#[must_use]
pub fn neighbourhood(graph: &AssetGraph, focus: &AssetId) -> BTreeSet<AssetId> {
    let mut ids = BTreeSet::new();
    if !graph.nodes.contains_key(focus) {
        return ids;
    }
    ids.insert(focus.clone());
    for edge in &graph.edges {
        if &edge.to == focus {
            ids.insert(edge.from.clone());
        }
        if &edge.from == focus {
            ids.insert(edge.to.clone());
        }
    }
    ids
}

/// The **full transitive lineage** of `focus` in `graph`: the focused node,
/// every ancestor that feeds it (all upstream, transitively) and every
/// descendant it feeds (all downstream, transitively). Where [`neighbourhood`]
/// stops one hop each way, this walks the whole chain — the drill scope shows
/// the entire family tree that touches the selection. An id not in the graph
/// yields an empty set.
#[must_use]
pub fn lineage(graph: &AssetGraph, focus: &AssetId) -> BTreeSet<AssetId> {
    let mut ids = BTreeSet::new();
    if !graph.nodes.contains_key(focus) {
        return ids;
    }
    ids.insert(focus.clone());
    // Upstream: everything that (transitively) produces the focus.
    let mut frontier = vec![focus.clone()];
    while let Some(node) = frontier.pop() {
        for edge in &graph.edges {
            if edge.to == node && ids.insert(edge.from.clone()) {
                frontier.push(edge.from.clone());
            }
        }
    }
    // Downstream: everything the focus (transitively) feeds — seeded from the
    // focus alone, so an ancestor's *other* branches are not pulled in. This
    // walk keeps its OWN visited set: a node that is both an ancestor and a
    // descendant of the focus (a cycle through the focus) is already in `ids`
    // from the upstream pass, so sharing that set would stop the downstream
    // walk from expanding it and silently drop its exclusive descendants.
    let mut seen_down = BTreeSet::new();
    let mut frontier = vec![focus.clone()];
    while let Some(node) = frontier.pop() {
        for edge in &graph.edges {
            if edge.from == node && seen_down.insert(edge.to.clone()) {
                ids.insert(edge.to.clone());
                frontier.push(edge.to.clone());
            }
        }
    }
    ids
}

/// The subgraph of `graph` induced on `keep`: the kept nodes plus every edge
/// (and the seam it flows through) with both endpoints kept. A pure,
/// deterministic `AssetGraph -> AssetGraph` slice — the drill scope's displayed
/// graph, laid out and rastered like any other.
#[must_use]
pub fn induced_subgraph(graph: &AssetGraph, keep: &BTreeSet<AssetId>) -> AssetGraph {
    let nodes: BTreeMap<AssetId, AssetNode> = graph
        .nodes
        .iter()
        .filter(|(id, _)| keep.contains(*id))
        .map(|(id, n)| (id.clone(), n.clone()))
        .collect();
    let edges: Vec<Edge> = graph
        .edges
        .iter()
        .filter(|e| keep.contains(&e.from) && keep.contains(&e.to))
        .cloned()
        .collect();
    let kept_steps: BTreeSet<&StepId> = edges.iter().filter_map(|e| e.via.as_ref()).collect();
    let seams: BTreeMap<StepId, Seam> = graph
        .seams
        .iter()
        .filter(|(step, _)| kept_steps.contains(step))
        .map(|(s, seam)| (s.clone(), seam.clone()))
        .collect();
    AssetGraph {
        protocol: graph.protocol.clone(),
        nodes,
        seams,
        edges,
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
            Ok(
                "CREATE TABLE staging AS SELECT * FROM read_csv('build/a.csv', header=true);\n\
                CREATE TABLE t_out AS SELECT * FROM staging;"
                    .to_string(),
            ),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn derivation_covers_every_kind() {
        let g = mini_graph();
        let kind = |id: &str| g.nodes.get(id).map(|n| n.kind);
        assert_eq!(
            kind("source.mini.https://example.com/data/a.csv"),
            Some(AssetKind::Source)
        );
        assert_eq!(kind("file.mini.build/a.csv"), Some(AssetKind::File));
        // staging is consumed later in the same file and referenced by no
        // other step -> INTERNAL under a stmt id; t_out is exported -> TABLE.
        assert_eq!(kind("stmt.mini.transform#0"), Some(AssetKind::Internal));
        assert_eq!(kind("asset.mini.t_out"), Some(AssetKind::Table));
        // The export dest is read by nothing -> Dataset sink.
        assert_eq!(kind("file.mini.build/t.parquet"), Some(AssetKind::Dataset));
        // The SOURCE label is the host.
        assert_eq!(
            g.nodes["source.mini.https://example.com/data/a.csv"].label,
            "example.com"
        );
    }

    #[test]
    fn gate_is_a_shield_not_a_node() {
        let g = mini_graph();
        // The gate seam exists and is marked.
        assert!(g.seams["validate"].gate);
        // No node belongs to the validate step.
        assert!(g
            .nodes
            .values()
            .all(|n| n.step.as_deref() != Some("validate")));
        // The edge INTO the guarded parquet carries the shield.
        let shielded: Vec<&Edge> = g
            .edges
            .iter()
            .filter(|e| e.to == "file.mini.build/t.parquet")
            .collect();
        assert!(!shielded.is_empty());
        assert!(
            shielded.iter().all(|e| e.shield),
            "guarded edge carries shield: {shielded:?}"
        );
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
        assert!(has_edge(
            "source.mini.https://example.com/data/a.csv",
            "file.mini.build/a.csv"
        ));
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
      members: [part.tsv]
  - name: load
    sql: models/load.sql
    depends_on: [build/out/q1/part.tsv]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok(
                "CREATE TABLE loaded AS SELECT * FROM read_csv('build/out/*/part.tsv');"
                    .to_string(),
            ),
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
    fn pds_depends_ancestor_wires_every_producer_at_mixed_depths() {
        // A `depends_on: [build/ncen]` names EVERY producer beneath the
        // ancestor dir. Two producers sit at depth 3 (build/ncen/q1, /q2) and
        // one INDEPENDENT producer sits deeper at depth 4 (build/ncen/2023/q3).
        // A single global min-depth filter keeps only the depth-3 producers and
        // silently drops the deeper one's lineage edge; per-subtree "shallowest"
        // wires all three (none is nested under another). This fails pre-fix:
        // the build/ncen/2023/q3 edge is never drawn.
        let yaml = r"
name: mixdepth
steps:
  - name: extract_q1
    op: archive_extract@1
    with:
      archive: build/q1.zip
      dest: build/ncen/q1
      members: [p.tsv]
  - name: extract_q2
    op: archive_extract@1
    with:
      archive: build/q2.zip
      dest: build/ncen/q2
      members: [p.tsv]
  - name: extract_q3
    op: archive_extract@1
    with:
      archive: build/q3.zip
      dest: build/ncen/2023/q3
      members: [p.tsv]
  - name: load
    op: parquet_export@1
    with:
      input: loaded
      dest: build/loaded.parquet
    depends_on: [build/ncen]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        let wired = |from: &str| {
            g.edges
                .iter()
                .any(|e| e.from == from && e.to == "file.mixdepth.build/loaded.parquet")
        };
        // Equal-depth siblings both wire (the invariant the fix must preserve).
        assert!(
            wired("file.mixdepth.build/ncen/q1"),
            "shallow sibling q1 wired"
        );
        assert!(
            wired("file.mixdepth.build/ncen/q2"),
            "shallow sibling q2 wired"
        );
        // The DEEPER independent producer wires too (the regression this guards).
        assert!(
            wired("file.mixdepth.build/ncen/2023/q3"),
            "deeper independent producer under the same ancestor must still wire"
        );
        // The consumed ancestor dir itself is never fabricated — branch (b)
        // wires existing producers, it does not invent a directory node.
        assert!(!g.nodes.contains_key("file.mixdepth.build/ncen"));
    }

    #[test]
    fn pds_step_level_degrade_is_an_opaque_chip() {
        let yaml = "name: deg\nsteps:\n  - name: broken\n    sql: models/missing.sql\n    depends_on: [in.csv]\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "broken".to_string(),
            Err("models/missing.sql: not found".to_string()),
        );
        let g = build_graph(&manifest, &sources);
        let chip = &g.nodes["stmt.deg.broken#0"];
        assert_eq!(chip.kind, AssetKind::Opaque);
        assert!(chip
            .issue
            .as_deref()
            .is_some_and(|e| e.contains("not found")));
        // An untagged error is a caller-built source map, which touched no
        // filesystem — so no verdict on the file is claimed.
        assert_eq!(chip.label, "unavailable: models/missing.sql");
        assert!(
            g.edges.iter().any(|e| e.to == "stmt.deg.broken#0"),
            "depends_on wires the chip"
        );
    }

    /// A `sql:` step whose model is one of `absent` / `unreadable`, plus its
    /// readable form. `models/` under a manifest directory, one `sql:` step.
    fn degraded_graph(model_source: Result<String, String>) -> AssetGraph {
        let yaml = "name: acc\nsteps:\n  - name: fetch\n    op: http_fetch@1\n    with: { url: 'https://h/a', out: build/a.csv }\n  - name: tier\n    sql: models/entity_tiering_rules.sql\n    depends_on: [build/a.csv]\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert("tier".to_string(), model_source);
        build_graph(&manifest, &sources)
    }

    /// AC2 at the derivation layer: a model that is not there and a model that
    /// is there and refused are two different chips, and the difference is in
    /// the part of the label a narrow chip still shows.
    #[test]
    fn pds_absent_and_unreadable_models_do_not_draw_the_same_chip() {
        let absent = degraded_graph(Err(
            "absent: models/entity_tiering_rules.sql: No such file or directory (os error 2)"
                .to_string(),
        ));
        let unreadable = degraded_graph(Err(
            "unreadable: models/entity_tiering_rules.sql: Permission denied (os error 13)"
                .to_string(),
        ));
        let chip = |g: &AssetGraph| g.nodes["stmt.acc.tier#0"].clone();
        let (a, u) = (chip(&absent), chip(&unreadable));
        assert_ne!(a.label, u.label, "the two causes are not one label");
        // The renderer fits a label to the card's width and elides the tail, so
        // a difference late in a long label never reaches a pixel — and the
        // width is capped, so a long enough model path elides any marker after
        // it. Asserting the divergence is EARLY is what stops the marker
        // migrating back to the end of the label.
        let diverges_at = a
            .label
            .chars()
            .zip(u.label.chars())
            .position(|(x, y)| x != y)
            .unwrap_or(usize::MAX);
        assert!(
            diverges_at < 4,
            "the cause is legible before any elision (diverges at char {diverges_at}): \
             {:?} vs {:?}",
            a.label,
            u.label
        );
        // And the badge text names whose problem each is.
        assert!(a.issue.as_deref().is_some_and(|i| i.contains("not there")));
        assert!(u
            .issue
            .as_deref()
            .is_some_and(|i| i.contains("Access, not authorship")));
    }

    /// AC4 at the derivation layer: the graph answers "did this draw
    /// everything it was asked for" without anyone looking at it.
    #[test]
    fn pds_degrades_reports_complete_and_classifies_each_degrade() {
        let complete = degraded_graph(Ok(
            "CREATE TABLE tiered AS SELECT * FROM read_csv('build/a.csv');".to_string(),
        ));
        assert_eq!(
            degrades(&complete),
            Vec::new(),
            "a graph with nothing missing reports nothing"
        );

        for (source, expected) in [
            (
                "absent: models/entity_tiering_rules.sql: No such file or directory (os error 2)",
                Degradation::ModelAbsent,
            ),
            (
                "unreadable: models/entity_tiering_rules.sql: Permission denied (os error 13)",
                Degradation::ModelUnreadable,
            ),
            (
                "models/entity_tiering_rules.sql: not embedded",
                Degradation::ModelUnavailable,
            ),
        ] {
            let g = degraded_graph(Err(source.to_string()));
            let found = degrades(&g);
            assert_eq!(found.len(), 1, "one degrade for {source}");
            assert_eq!(found[0].class, expected, "class of {source}");
            assert_eq!(found[0].node, "stmt.acc.tier#0");
            assert_eq!(found[0].step.as_deref(), Some("tier"));
            assert_eq!(
                Some(found[0].detail.as_str()),
                g.nodes["stmt.acc.tier#0"].issue.as_deref(),
                "the report carries exactly what the inspector shows"
            );
        }

        // A degrade raised somewhere other than the model read still counts as
        // "this is not a complete render" — it is only unclassed.
        let yaml = "name: c\nsteps:\n  - name: scrub\n    command: ./scrub.sh\n";
        let g = build_graph(&parse_manifest_str(yaml).unwrap(), &BTreeMap::new());
        let found = degrades(&g);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].class, Degradation::Other);
    }

    /// The read-site classifier is the only place that can tell the two apart,
    /// so it is held to the `io::ErrorKind` mapping directly. The filesystem
    /// half — a real refused read — is
    /// `crates/brightfield-protocol/tests/model_source_faults.rs`.
    #[test]
    fn pds_read_failures_classify_by_error_kind() {
        use std::io::{Error, ErrorKind};
        let nowhere = Path::new("/nonexistent-brightfield-fixture/models/t.sql");
        assert_eq!(
            classify_read_failure(nowhere, &Error::from(ErrorKind::PermissionDenied)),
            Degradation::ModelUnreadable
        );
        // Absence is claimed only where the entry really is not there and its
        // directory could be opened to establish that. Here neither holds
        // (the parent does not exist), which is not a refusal either.
        assert_eq!(
            classify_read_failure(nowhere, &Error::from(ErrorKind::NotFound)),
            Degradation::ModelAbsent
        );
        // A path that DOES exist reported as NotFound is a refusal wearing the
        // wrong error: the entry is there, so absence is refused.
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert_eq!(
            classify_read_failure(&here, &Error::from(ErrorKind::NotFound)),
            Degradation::ModelUnreadable
        );
        assert_eq!(
            classify_read_failure(nowhere, &Error::from(ErrorKind::InvalidData)),
            Degradation::ModelUnavailable
        );
    }

    #[test]
    fn pds_build_graph_is_deterministic() {
        assert_eq!(mini_graph(), mini_graph());
    }

    #[test]
    fn pds_dataset_sink_ignores_annotator_but_not_pipeline_consumption() {
        // A mid-pipeline parquet_export dest consumed by a LATER op through a
        // named `with:` key (splink-style `edgar:`) must NOT be re-kinded the
        // terminal sink — even when nothing depends_on it. This is the
        // fixture's own splink_resolve shape MINUS load.sql's depends_on
        // backstop: the under-sampled input that hid the two-Dataset bug.
        let yaml = r"
name: f
steps:
  - name: build_sec
    sql: models/sec.sql
  - name: export_sec
    op: parquet_export@1
    with: { input: sec_entities, dest: build/sec_entities.parquet }
  - name: resolve
    op: splink_resolve@1
    with: { edgar: build/sec_entities.parquet, gleif: gleif.parquet, out: build/resolved.parquet }
  - name: tier
    sql: models/tier.sql
    depends_on: [build/resolved.parquet]
  - name: export_final
    op: parquet_export@1
    with: { input: edges_out, dest: build/final.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "build_sec".to_string(),
            Ok("CREATE TABLE sec_entities AS SELECT 1 AS cik;".to_string()),
        );
        sources.insert(
            "tier".to_string(),
            Ok(
                "CREATE TABLE edges_out AS SELECT * FROM read_parquet('build/resolved.parquet');"
                    .to_string(),
            ),
        );
        let g = build_graph(&manifest, &sources);
        let datasets: Vec<&str> = g
            .nodes
            .values()
            .filter(|n| n.kind == AssetKind::Dataset)
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(
            datasets,
            vec!["build/final.parquet"],
            "exactly the terminal export is the sink"
        );
        assert_eq!(
            g.nodes["file.f.build/sec_entities.parquet"].kind,
            AssetKind::File,
            "the mid-pipeline export stays FILE (it has an outgoing lineage edge)"
        );
        assert!(
            g.edges
                .iter()
                .any(|e| e.from == "file.f.build/sec_entities.parquet"
                    && e.to == "file.f.build/resolved.parquet"),
            "the named `with:` consumption still draws the lineage edge"
        );
    }

    #[test]
    fn pds_depends_on_ancestor_dir_wires_producers_not_a_fabricated_node() {
        // depends_on names the PARENT directory of produced dests; it must
        // resolve to every producer beneath it, never fabricate a disconnected
        // external `file.<proto>.build/ncen` node (path_covers only handled the
        // consumed-deeper direction before).
        let yaml = r"
name: x
steps:
  - name: extract_q1
    op: archive_extract@1
    with: { archive: a.zip, dest: build/ncen/q1, members: [p.tsv] }
  - name: extract_q2
    op: archive_extract@1
    with: { archive: b.zip, dest: build/ncen/q2, members: [p.tsv] }
  - name: load
    sql: models/load.sql
    depends_on: [build/ncen]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok("CREATE TABLE loaded AS SELECT 1;".to_string()),
        );
        let g = build_graph(&manifest, &sources);
        assert!(
            !g.nodes.contains_key("file.x.build/ncen"),
            "no fabricated ancestor node"
        );
        assert!(g
            .edges
            .iter()
            .any(|e| e.from == "file.x.build/ncen/q1" && e.to == "asset.x.loaded"));
        assert!(g
            .edges
            .iter()
            .any(|e| e.from == "file.x.build/ncen/q2" && e.to == "asset.x.loaded"));
    }

    #[test]
    fn pds_targetless_statement_fabricates_no_orphan_node() {
        // An INSERT (no CREATE target) must not fabricate a floating
        // consumed-relation node with no incident edges.
        let yaml = "name: d\nsteps:\n  - name: t\n    sql: models/t.sql\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "t".to_string(),
            Ok(
                "CREATE TABLE agg AS SELECT 1 AS n;\nINSERT INTO agg SELECT * FROM extra_src;"
                    .to_string(),
            ),
        );
        let g = build_graph(&manifest, &sources);
        assert!(
            !g.nodes.keys().any(|k| k.contains("extra_src")),
            "the INSERT's consumed relation is not fabricated as an orphan node"
        );
        assert!(
            g.nodes.contains_key("asset.d.agg"),
            "the CREATE target still exists"
        );
    }

    #[test]
    fn pds_statement_level_chip_wires_from_depends_on() {
        // A failed MIDDLE statement chip must inherit the step's depends_on so
        // it isn't a layer-0 orphan detached from the flow it interrupts.
        let yaml = "name: d\nsteps:\n  - name: fetch\n    op: http_fetch@1\n    with: { url: 'https://h/x', out: build/raw.csv }\n  - name: t\n    sql: models/t.sql\n    depends_on: [build/raw.csv]\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "t".to_string(),
            Ok(
                "CREATE TABLE a AS SELECT * FROM read_csv('build/raw.csv');\n\
                SELEC broken here;\n\
                CREATE TABLE b AS SELECT * FROM a;"
                    .to_string(),
            ),
        );
        let g = build_graph(&manifest, &sources);
        assert_eq!(g.nodes["stmt.d.t#1"].kind, AssetKind::Opaque);
        assert!(
            g.edges
                .iter()
                .any(|e| e.to == "stmt.d.t#1" && e.from == "file.d.build/raw.csv"),
            "the chip is wired from the step's depends_on, not adrift in layer 0"
        );
    }

    #[test]
    fn pds_output_less_step_surfaces_a_chip_not_a_silent_drop() {
        // A command step that consumes inputs but declares no out/dest must
        // still appear in the DAG (an issue-badged chip wired from its inputs),
        // never vanish with zero signal.
        let yaml = "name: c\nsteps:\n  - name: fetch\n    op: http_fetch@1\n    with: { url: 'https://h/a', out: build/a.csv }\n  - name: scrub\n    command: ./scrub.sh\n    depends_on: [build/a.csv]\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        assert_eq!(
            g.nodes["stmt.c.scrub#effect"].kind,
            AssetKind::Opaque,
            "the output-less step is a visible chip"
        );
        assert!(
            g.edges
                .iter()
                .any(|e| e.to == "stmt.c.scrub#effect" && e.from == "file.c.build/a.csv"),
            "wired from its input, so it never vanishes"
        );
    }

    /// A drill scope is the focused node plus its one-hop producers/consumers,
    /// and the induced slice keeps only edges internal to that set.
    #[test]
    fn neighbourhood_and_induced_subgraph_slice_the_local_scope() {
        let yaml = "name: d\nsteps:\n  - name: fetch\n    op: http_fetch@1\n    with: { url: 'https://h/a', out: build/a.csv }\n  - name: transform\n    sql: models/t.sql\n    depends_on: [build/a.csv]\n  - name: export\n    op: parquet_export@1\n    with: { input: t_out, dest: build/t.parquet }\n";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE t_out AS SELECT * FROM read_csv('build/a.csv');".to_string()),
        );
        let g = build_graph(&manifest, &sources);
        let focus = "file.d.build/a.csv".to_string();
        let hood = neighbourhood(&g, &focus);
        assert!(
            hood.contains(&focus),
            "the focus is in its own neighbourhood"
        );
        // Every node adjacent to the focus is included; nothing two hops out.
        for edge in &g.edges {
            if edge.from == focus {
                assert!(hood.contains(&edge.to), "a direct consumer is kept");
            }
            if edge.to == focus {
                assert!(hood.contains(&edge.from), "a direct producer is kept");
            }
        }
        let sub = induced_subgraph(&g, &hood);
        assert_eq!(
            sub.nodes.len(),
            hood.len(),
            "the slice has exactly the kept nodes"
        );
        assert!(
            sub.edges
                .iter()
                .all(|e| hood.contains(&e.from) && hood.contains(&e.to)),
            "only internal edges survive the slice"
        );
        // Unknown focus yields an empty scope.
        assert!(neighbourhood(&g, &"file.d.nope".to_string()).is_empty());
    }

    /// `lineage` is the full transitive closure — every upstream ancestor and
    /// downstream descendant of the focus — strictly wider than the one-hop
    /// neighbourhood on a chain longer than two.
    #[test]
    fn lineage_is_the_full_transitive_closure_not_one_hop() {
        // A linear chain: source -> file -> stmt -> table -> dataset. Focusing
        // the middle statement, its lineage is the whole chain (two hops each
        // way), where the neighbourhood is only the three adjacent nodes.
        let g = mini_graph();
        let focus = "stmt.mini.transform#0".to_string();
        let full = lineage(&g, &focus);
        for id in [
            "source.mini.https://example.com/data/a.csv",
            "file.mini.build/a.csv",
            "stmt.mini.transform#0",
            "asset.mini.t_out",
            "file.mini.build/t.parquet",
        ] {
            assert!(full.contains(id), "the transitive lineage keeps {id}");
        }
        let hood = neighbourhood(&g, &focus);
        assert!(
            full.len() > hood.len(),
            "lineage ({}) is wider than the one-hop neighbourhood ({})",
            full.len(),
            hood.len()
        );
        // Focusing the sink pulls in every ancestor but adds no descendant.
        let sink = lineage(&g, &"file.mini.build/t.parquet".to_string());
        assert_eq!(
            sink.len(),
            g.nodes.len(),
            "the sink's lineage is the whole chain"
        );
        // An unknown focus yields an empty lineage.
        assert!(lineage(&g, &"file.mini.nope".to_string()).is_empty());
    }

    /// A cycle through the focus must not swallow descendants. `x` is BOTH an
    /// ancestor (`x -> focus`) and a descendant (`focus -> x`) of the focus.
    /// Sharing one visited set between the upstream and downstream walks marks
    /// `x` seen upstream, so the downstream walk never expands it and silently
    /// drops `y` — a descendant reachable only through the cycle node. Separate
    /// visited sets keep every descendant.
    #[test]
    fn lineage_keeps_descendants_reachable_only_through_a_cycle_node() {
        let node = |id: &str| AssetNode {
            id: id.to_string(),
            kind: AssetKind::Table,
            label: id.to_string(),
            step: None,
            family_count: None,
            issue: None,
        };
        let edge = |from: &str, to: &str| Edge {
            from: from.to_string(),
            to: to.to_string(),
            via: None,
            shield: false,
        };
        let mut nodes = BTreeMap::new();
        for id in ["a", "focus", "x", "y"] {
            nodes.insert(id.to_string(), node(id));
        }
        let g = AssetGraph {
            protocol: "cyc".to_string(),
            nodes,
            seams: BTreeMap::new(),
            edges: vec![
                edge("a", "focus"),
                edge("focus", "x"),
                edge("x", "focus"),
                edge("x", "y"),
            ],
        };
        let full = lineage(&g, &"focus".to_string());
        // `a` upstream, `x` on the cycle, and `y` — the descendant reachable
        // only through the cycle node — are all in the focus's lineage.
        for id in ["a", "focus", "x", "y"] {
            assert!(
                full.contains(id),
                "lineage keeps {id} across the focus cycle"
            );
        }
    }
}
