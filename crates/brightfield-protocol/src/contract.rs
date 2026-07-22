//! Deserialize model for the arcform Protocol+Run JSON contract
//! (`contract_version` `"b4/1"`) — the stable, emitted shape brightfield
//! consumes instead of re-parsing a manifest and its `models/*.sql`.
//!
//! `arc run` writes `build/.arcform/runs/<run_id>.json` (this document) plus a
//! live `<run_id>.jsonl` stream ([`crate::stream`]). The emitter fills some
//! fields eagerly and leaves the *measured*/*resolved* ones (`bytes`,
//! `content_hash`, `path`, `duration_sec`, `skip_reason`, `resolved_with`,
//! `ingress_meta`, `io.*`, `narrative.*`) `null`/absent in early phases, so
//! every such field is an `Option<T>` or a `#[serde(default)]` collection.
//! Deserialization must never fail merely because a not-yet-populated field is
//! absent — unknown keys are ignored (serde's default).
//!
//! The enum-typed tags (`outcome`, asset `kind`, step `kind`, step `state`)
//! each carry an `Unknown` catch-all so a forward-compatible emitter that adds a
//! variant does not break an older reader.
//!
//! The coarse asset `kind` here (`source|file|table|model|…`) is deliberately
//! NOT the crate's finer [`graph::AssetKind`](crate::graph::AssetKind); the
//! [`contract_graph`](crate::contract_graph) builder re-derives the finer view
//! kind from run signals. This module names its coarse enum
//! [`ContractAssetKind`] to keep the two apart.

use serde::Deserialize;

/// The `contract_version` this reader is written against. Treated as an opaque
/// scheme tag; the loader accepts any version under the `b4/` family.
pub const SUPPORTED_CONTRACT_VERSION: &str = "b4/1";

/// Accepted `contract_version` family prefix.
pub const SUPPORTED_CONTRACT_FAMILY: &str = "b4/";

/// Top-level Protocol+Run contract document.
#[derive(Debug, Clone, Deserialize)]
pub struct Contract {
    /// Opaque scheme tag, e.g. `"b4/1"`.
    pub contract_version: String,
    /// Run-level metadata.
    pub run: Run,
    /// Assets produced or consumed by the run — the graph's data states.
    #[serde(default)]
    pub assets: Vec<Asset>,
    /// Steps executed by the run — the seams between data states.
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Contract {
    /// `true` when `contract_version` is under the supported `b4/` family.
    #[must_use]
    pub fn is_supported_version(&self) -> bool {
        self.contract_version.starts_with(SUPPORTED_CONTRACT_FAMILY)
    }

    /// Look up an asset by its flat contract id (`"table:widgets"`).
    #[must_use]
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Look up a step by name.
    #[must_use]
    pub fn step(&self, name: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.name == name)
    }
}

/// Outcome of the run as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Every step succeeded.
    Success,
    /// The run failed.
    Error,
    /// The run completed with a mix of succeeded/failed/skipped steps.
    Partial,
    /// A variant not known to this reader.
    #[serde(other)]
    Unknown,
}

/// Run-level metadata block.
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    /// Stable id for the run; also the basename of the live `.jsonl` stream.
    pub run_id: String,
    /// Id of this attempt within the run.
    #[serde(default)]
    pub attempt_id: Option<String>,
    /// The protocol (pipeline definition) that was run.
    pub protocol: Protocol,
    /// Engine versions the run executed against.
    #[serde(default)]
    pub engine: Option<Engine>,
    /// Effective run parameters.
    #[serde(default)]
    pub params: Vec<Param>,
    /// RFC3339 start timestamp, when recorded (second resolution).
    #[serde(default)]
    pub started_at: Option<String>,
    /// RFC3339 finish timestamp, when recorded (second resolution).
    #[serde(default)]
    pub finished_at: Option<String>,
    /// Overall outcome.
    pub outcome: Outcome,
}

/// Identity of the protocol (pipeline definition) that produced the run.
#[derive(Debug, Clone, Deserialize)]
pub struct Protocol {
    /// Human name of the protocol — the namespace for every derived asset id.
    pub name: String,
    /// Content hash of the protocol manifest, when computed.
    #[serde(default)]
    pub manifest_sha256: Option<String>,
    /// Source directory of the protocol, when recorded.
    #[serde(default)]
    pub dir: Option<String>,
}

/// Engine version fingerprint.
#[derive(Debug, Clone, Deserialize)]
pub struct Engine {
    /// Runner/engine version.
    #[serde(default)]
    pub arc: Option<String>,
    /// Embedded DuckDB version.
    #[serde(default)]
    pub duckdb: Option<String>,
}

/// One effective run parameter.
#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    /// Parameter name.
    #[serde(default)]
    pub key: Option<String>,
    /// Parameter value (any JSON scalar/structure).
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Where the value came from (default / cli / env / …).
    #[serde(default)]
    pub source: Option<String>,
}

/// Coarse kind of an asset as recorded in the contract. NOT the crate's finer
/// [`graph::AssetKind`](crate::graph::AssetKind) — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractAssetKind {
    /// External input the run reads but does not produce.
    Source,
    /// A file on disk.
    File,
    /// A base table.
    Table,
    /// A derived (modelled) table.
    Model,
    /// A visualisation spec.
    ChartSpec,
    /// A metrics artifact.
    Metrics,
    /// A dashboard artifact.
    Dashboard,
    /// A variant not known to this reader.
    #[serde(other)]
    Unknown,
}

/// An asset produced or consumed by the run — a data state (graph node).
#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    /// Flat contract id, e.g. `"table:widgets"` / `"file:build/x.parquet"`.
    pub id: String,
    /// Coarse kind.
    pub kind: ContractAssetKind,
    /// Human name / short label (a relation name for tables).
    pub name: String,
    /// Path on disk, when materialised (deferred → `null`).
    #[serde(default)]
    pub path: Option<String>,
    /// Size in bytes, when measured (deferred → `null`).
    #[serde(default)]
    pub bytes: Option<u64>,
    /// Row count, when measured. `null` is overloaded (a file has none; a
    /// not-materialised relation has none) — disambiguated by kind + status.
    #[serde(default)]
    pub row_count: Option<u64>,
    /// Content hash, when computed (deferred → `null`).
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Name of the step that produced this asset, if any (sources have none).
    /// May name a step whose `status.state` is `skipped`/`failed` — cross-
    /// reference the step before treating the asset as materialised.
    #[serde(default)]
    pub produced_by: Option<String>,
    /// Names of steps that consume this asset.
    #[serde(default)]
    pub consumed_by: Vec<String>,
}

/// Kind of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    /// A SQL model step.
    Sql,
    /// A named operator invocation.
    Op,
    /// A shell command.
    Command,
    /// A lifecycle hook.
    Hook,
    /// A variant not known to this reader.
    #[serde(other)]
    Unknown,
}

/// Terminal state of a step as recorded in the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    /// Step completed successfully.
    Success,
    /// Step failed.
    Failed,
    /// Step was skipped.
    Skipped,
    /// A variant not known to this reader.
    #[serde(other)]
    Unknown,
}

/// One executed step — the transform that shrinks to a seam.
#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    /// Step name (unique within a run; matches the `.jsonl` `step` field).
    pub name: String,
    /// What kind of step this is.
    pub kind: StepKind,
    /// Operator reference, for `op` steps.
    #[serde(default)]
    pub op_ref: Option<OpRef>,
    /// Free-form resolution record (deferred → `null`).
    #[serde(default)]
    pub resolved_with: Option<serde_json::Value>,
    /// SQL detail, for `sql` steps.
    #[serde(default)]
    pub sql: Option<SqlBlock>,
    /// Execution status.
    pub status: StepStatus,
    /// Number of attempts made.
    #[serde(default)]
    pub attempts: Option<u32>,
    /// Wall-clock duration in seconds — deferred (`null`); do not rely on it.
    #[serde(default)]
    pub duration_sec: Option<f64>,
    /// Retry policy that was in force.
    #[serde(default)]
    pub retry: Option<Retry>,
    /// Timeout in seconds, when configured.
    #[serde(default)]
    pub timeout_sec: Option<f64>,
    /// Captured stdout/stderr locations (deferred → `null`).
    #[serde(default)]
    pub io: Option<Io>,
    /// Free-form ingress metadata (deferred → `null`).
    #[serde(default)]
    pub ingress_meta: Option<serde_json::Value>,
    /// Narrative annotations for the step-detail panel (deferred → `null`).
    #[serde(default)]
    pub narrative: Option<Narrative>,
}

impl Step {
    /// The operator name, for `op` steps that carry an [`OpRef`].
    #[must_use]
    pub fn op_name(&self) -> Option<&str> {
        self.op_ref.as_ref().map(|o| o.name.as_str())
    }

    /// The preferred display label — `narrative.label` when present, else the
    /// step name.
    #[must_use]
    pub fn label(&self) -> String {
        self.narrative
            .as_ref()
            .and_then(|n| n.label.clone())
            .unwrap_or_else(|| self.name.clone())
    }
}

/// Reference to a named operator plus how its version was pinned/resolved.
#[derive(Debug, Clone, Deserialize)]
pub struct OpRef {
    /// Operator name.
    pub name: String,
    /// The requested version constraint, when present.
    #[serde(default)]
    pub constraint: Option<String>,
    /// The concrete version that was resolved, when present.
    #[serde(default)]
    pub version_resolved: Option<String>,
}

impl OpRef {
    /// The best available version string: resolved, else the constraint, else
    /// `"?"` — mirroring the manifest path's `op@?` fallback.
    #[must_use]
    pub fn version(&self) -> String {
        self.version_resolved
            .clone()
            .or_else(|| self.constraint.clone())
            .unwrap_or_else(|| "?".to_string())
    }
}

/// SQL detail for a `sql` step.
#[derive(Debug, Clone, Deserialize)]
pub struct SqlBlock {
    /// Path to the model file, when known.
    #[serde(default)]
    pub model_path: Option<String>,
    /// The SQL text (shown in the step detail).
    #[serde(default)]
    pub sql_text: Option<String>,
    /// Content hash of the SQL text.
    #[serde(default)]
    pub sql_hash: Option<String>,
    /// Per-statement produces/reads — the finer intra-step lineage.
    #[serde(default)]
    pub statements: Vec<SqlStatement>,
}

/// A single SQL statement's produced/read relations.
#[derive(Debug, Clone, Deserialize)]
pub struct SqlStatement {
    /// Relations produced by this statement.
    #[serde(default)]
    pub produces: Vec<String>,
    /// Relations read by this statement.
    #[serde(default)]
    pub reads: Vec<String>,
}

/// Execution status of a step.
#[derive(Debug, Clone, Deserialize)]
pub struct StepStatus {
    /// The terminal state.
    pub state: StepState,
    /// Reason a step was skipped, when applicable (deferred → `null`).
    #[serde(default)]
    pub skip_reason: Option<String>,
}

impl StepStatus {
    /// The typed form of [`StepStatus::skip_reason`] — see [`SkipReason`].
    #[must_use]
    pub fn skip_reason_kind(&self) -> Option<SkipReason> {
        self.skip_reason.as_deref().map(SkipReason::parse)
    }
}

/// The typed skip reasons the runner records on a step it left un-executed
/// because the materialised data is already current: `hash_clean` (the step's
/// SQL/config fingerprint matches the prior run) and the precondition pair
/// (`precondition_fresh` / `precondition_modified_after` — the step's declared
/// precondition held). The runner computes these; this reader only names them,
/// so a surface can render a skip's meaning without re-deriving freshness.
///
/// A reason outside that set parses as [`SkipReason::Other`], which proves
/// nothing — the forward-compatibility stance the enum-tagged contract fields
/// already take, applied to this string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The step's SQL/config fingerprint matches the prior successful run.
    HashClean,
    /// The step's declared precondition reported fresh.
    PreconditionFresh,
    /// The step's modified-after precondition reported fresh.
    PreconditionModifiedAfter,
    /// A reason this reader does not know. Proves nothing.
    Other,
}

impl SkipReason {
    /// Parse a recorded skip-reason string into its typed form.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "hash_clean" => SkipReason::HashClean,
            "precondition_fresh" => SkipReason::PreconditionFresh,
            "precondition_modified_after" => SkipReason::PreconditionModifiedAfter,
            _ => SkipReason::Other,
        }
    }

    /// Whether this reason is the runner's own proof that the materialised
    /// data is current — a skip recorded *because* the data is fresh, as
    /// opposed to a reason this reader cannot vouch for. The honesty rule
    /// rides on this: only a proven skip may be presented as fresh.
    #[must_use]
    pub const fn proves_fresh(self) -> bool {
        !matches!(self, SkipReason::Other)
    }
}

/// Retry policy for a step.
#[derive(Debug, Clone, Deserialize)]
pub struct Retry {
    /// Maximum attempts allowed.
    #[serde(default)]
    pub max_attempts: Option<u32>,
    /// Backoff between attempts, in seconds.
    #[serde(default)]
    pub backoff_sec: Option<f64>,
}

/// Captured stdout/stderr paths for a step (deferred → `null`).
#[derive(Debug, Clone, Deserialize)]
pub struct Io {
    /// Path to captured stdout, when captured.
    #[serde(default)]
    pub stdout_path: Option<String>,
    /// Path to captured stderr, when captured.
    #[serde(default)]
    pub stderr_path: Option<String>,
}

/// Narrative annotations attached to a step for the detail panel.
#[derive(Debug, Clone, Deserialize)]
pub struct Narrative {
    /// Short human label overriding the raw step name.
    #[serde(default)]
    pub label: Option<String>,
    /// Pipeline stage grouping.
    #[serde(default)]
    pub stage: Option<String>,
    /// Longer prose / documentation string.
    #[serde(default)]
    pub doc: Option<String>,
}

/// Parse a Protocol+Run contract from JSON bytes.
///
/// # Errors
/// Returns [`serde_json::Error`] if the bytes are not a structurally valid
/// contract document.
pub fn parse_contract(bytes: &[u8]) -> Result<Contract, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three recorded reasons parse to their typed forms and all three
    /// prove freshness; anything else parses to `Other` and proves nothing.
    #[test]
    fn skip_reasons_parse_typed_and_only_the_known_three_prove_fresh() {
        assert_eq!(SkipReason::parse("hash_clean"), SkipReason::HashClean);
        assert_eq!(
            SkipReason::parse("precondition_fresh"),
            SkipReason::PreconditionFresh
        );
        assert_eq!(
            SkipReason::parse("precondition_modified_after"),
            SkipReason::PreconditionModifiedAfter
        );
        assert!(SkipReason::HashClean.proves_fresh());
        assert!(SkipReason::PreconditionFresh.proves_fresh());
        assert!(SkipReason::PreconditionModifiedAfter.proves_fresh());
        // A reason from a future emitter is named, not trusted.
        assert_eq!(SkipReason::parse("gated_off"), SkipReason::Other);
        assert!(!SkipReason::Other.proves_fresh());
    }

    /// The accessor reads the recorded string off a step's status verbatim.
    #[test]
    fn step_status_exposes_its_typed_skip_reason() {
        let skipped: StepStatus =
            serde_json::from_str(r#"{ "state": "skipped", "skip_reason": "hash_clean" }"#).unwrap();
        assert_eq!(skipped.skip_reason_kind(), Some(SkipReason::HashClean));
        let ran: StepStatus = serde_json::from_str(r#"{ "state": "success" }"#).unwrap();
        assert_eq!(ran.skip_reason_kind(), None);
    }
}
