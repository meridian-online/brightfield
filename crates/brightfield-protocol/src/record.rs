//! Promoting a grid exploration into a durable protocol step — through arc.
//!
//! A viewer on top of a protocol lets a person *explore*: filter a grid,
//! select a range, narrow to a predicate — transient queries over tables the
//! pipeline already materialised. Exploration is free precisely because it is
//! not durable. The moment a filter should become part of the protocol,
//! something has to translate it into the durable document: a SQL model under
//! `models/` and a step in `arcform.yaml` that names it.
//!
//! That translation is arc's business, not brightfield's. This module compiles
//! a grid predicate to push-down SQL and hands it to arc's published record
//! path ([`arc::spec::record_step`] / [`arc::spec::amend_step_sql`]); arc
//! splices the step onto the manifest (format-preserving, every untargeted byte
//! untouched) and writes the generated model. **brightfield serialises no
//! manifest here** — it never emits YAML for a spec it did not create. The one
//! thing it authors is the model SQL, which carries arc's
//! [`GENERATED_MARKER`](arc::spec::GENERATED_MARKER) so a later amend is
//! licensed to regenerate it.
//!
//! **Recording never runs anything.** [`record_step`]
//! opens no database, executes no SQL, materialises no asset — it writes files
//! and returns. A freshly promoted step is a step that has **never run**, even
//! though equivalent rows were just on screen: what the person saw was a query,
//! what they recorded is a promise, and only `arc run` makes the promise true.
//! A surface must label the promoted step never-run, never fresh.
//!
//! The write is routed through arc's checkpoint-hooked record path: `record_step`
//! fires arc's checkpoint seam before the first durable byte moves, so a caller
//! that brings a local-history net snapshots there — the ordering stays visible.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use arc::spec::{amend_step_sql, record_step, Error as ArcError, RecordedStep};
use brightfield_sql::ir::Predicate;

use crate::contract_graph::downstream_steps;
use crate::graph::{AssetGraph, StepId};

/// A grid filter/selection promoted to a durable step: the upstream relation the
/// exploration read, and the predicate it filtered by.
///
/// The predicate is the machine-readable form a grid selection compiles to
/// (`brightfield_sql::ir::Predicate` — an interval, a point set, a conjunction),
/// and it renders straight into a push-down `WHERE` via its `Display`. `upstream`
/// is the relation the grid was showing, named in the asset namespace — trusted
/// SQL identifier text, the same trust level as the predicate's column
/// expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridFilter {
    /// The upstream relation the grid was showing (a step's output asset, or a
    /// source table). Trusted SQL text.
    pub upstream: String,
    /// The predicate the grid selection compiled to.
    pub predicate: Predicate,
}

impl GridFilter {
    /// Compile the filter to push-down SQL: a create-mode model that filters
    /// `upstream` by the predicate, materialising a table named `output` so the
    /// recorded step produces a queryable asset under `arc run`. The predicate
    /// renders into the `WHERE` exactly as a hand-written model's would.
    #[must_use]
    pub fn to_pushdown_sql(&self, output: &str) -> String {
        format!(
            "CREATE OR REPLACE TABLE {} AS\nSELECT *\nFROM {}\nWHERE {};\n",
            quote_ident(output),
            self.upstream,
            self.predicate
        )
    }

    /// The one-line provenance note recorded in the model's marker header — which
    /// tool wrote the file and what interaction it captured.
    #[must_use]
    pub fn provenance(&self) -> String {
        format!(
            "brightfield grid filter on {}: {}",
            self.upstream, self.predicate
        )
    }
}

/// A successful promotion: the durable step arc just appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promotion {
    /// The generated model's path, relative to the protocol dir (as the manifest
    /// cites it — e.g. `models/01_dover_tides.sql`).
    pub model_path: PathBuf,
    /// The step name now last in the manifest.
    pub step_name: String,
}

/// The result of amending a recorded step's SQL: the model rewritten, and the
/// steps the amend drags stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendOutcome {
    /// The rewritten model's path, relative to the protocol dir.
    pub model_path: PathBuf,
    /// The steps downstream of the amended one — every step whose previewed data
    /// this edit invalidates. Nothing here has run; a surface labels these
    /// stale-upstream and the amended step itself stale-edited, before any run.
    pub downstream_stale: BTreeSet<StepId>,
}

/// Promote a grid filter into the protocol at `dir` as a **new** step, through
/// arc's published write path.
///
/// The step name doubles as the created table's name. brightfield writes no
/// manifest YAML: [`record_step`] appends the step (a
/// format-preserving splice) and writes the generated model, marker header and
/// all. The promotion runs nothing — the step has never run when this returns.
///
/// # Errors
///
/// arc's own [`Error`](arc::spec::Error): a hostile step name, an already-occupied
/// model path, a manifest with no `steps` to record against, or a spliced result
/// that will not reload (a duplicate name, most often). A refusal leaves the
/// protocol directory untouched, byte for byte.
pub fn record_grid_filter(
    dir: &Path,
    step_name: &str,
    filter: &GridFilter,
) -> Result<Promotion, ArcError> {
    let step = RecordedStep {
        name: step_name.to_string(),
        sql: filter.to_pushdown_sql(step_name),
        provenance: filter.provenance(),
    };
    let (model_path, _validated) = record_step(dir, &step)?;
    Ok(Promotion {
        model_path,
        step_name: step_name.to_string(),
    })
}

/// Amend a previously-recorded step's SQL in place, then report the steps it
/// drags stale.
///
/// The rewrite is permitted only when the model on disk carries arc's generated
/// marker — the license to regenerate. A hand-authored model is refused
/// ([`Error::HandAuthoredSql`](arc::spec::Error::HandAuthoredSql)) with the file
/// untouched and a record-a-new-step-downstream remedy in the message: bytes
/// this tool did not author are never rewritten.
///
/// The stale set is the existing lineage walk ([`downstream_steps`]) — the
/// representation-side mirror of the runner's own staleness propagation. There is
/// **no** second staleness computation here: this only names which previews the
/// amend invalidates, so a surface can label them (the amended step stale-edited,
/// its downstream stale-upstream) before any run happens.
///
/// # Errors
///
/// arc's [`Error`](arc::spec::Error): no step of that name, a step with no
/// `sql:` file to amend, or the hand-authored-SQL refusal.
pub fn amend_recorded_filter(
    dir: &Path,
    graph: &AssetGraph,
    step_name: &str,
    filter: &GridFilter,
) -> Result<AmendOutcome, ArcError> {
    let sql = filter.to_pushdown_sql(step_name);
    let model_path = amend_step_sql(dir, step_name, &sql, &filter.provenance())?;
    let mut seeds = BTreeSet::new();
    seeds.insert(step_name.to_string());
    Ok(AmendOutcome {
        model_path,
        downstream_stale: downstream_steps(graph, &seeds),
    })
}

/// Double-quote a SQL identifier, doubling any embedded quote, so any name arc
/// accepts for a step (which may carry spaces or hyphens) is a valid table name.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_sql::ir::ScalarValue;

    fn point_filter() -> GridFilter {
        GridFilter {
            upstream: "tides".to_string(),
            predicate: Predicate::Point {
                column: "port".to_string(),
                values: vec![ScalarValue::Text("dover".to_string())],
                meta: None,
            },
        }
    }

    #[test]
    fn pushdown_sql_pushes_the_predicate_into_a_where() {
        let sql = point_filter().to_pushdown_sql("dover_tides");
        assert_eq!(
            sql,
            "CREATE OR REPLACE TABLE \"dover_tides\" AS\n\
             SELECT *\n\
             FROM tides\n\
             WHERE port = 'dover';\n"
        );
    }

    #[test]
    fn pushdown_sql_renders_an_interval_predicate() {
        let filter = GridFilter {
            upstream: "readings".to_string(),
            predicate: Predicate::Interval {
                column: "tide_m".to_string(),
                lo: ScalarValue::Float(5.0),
                hi: ScalarValue::Float(6.0),
                meta: None,
            },
        };
        let sql = filter.to_pushdown_sql("high_tides");
        assert!(
            sql.contains("WHERE (tide_m >= 5 AND tide_m <= 6);"),
            "interval pushed into WHERE: {sql}"
        );
    }

    #[test]
    fn provenance_is_one_line_and_names_the_source() {
        let p = point_filter().provenance();
        assert!(!p.contains('\n'), "the marker header is one line");
        assert!(p.contains("tides"));
        assert!(p.contains("port = 'dover'"));
    }

    #[test]
    fn quoted_identifiers_tolerate_spaces_and_quotes() {
        assert_eq!(quote_ident("plain"), "\"plain\"");
        assert_eq!(quote_ident("top-10 ports"), "\"top-10 ports\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    }
}
