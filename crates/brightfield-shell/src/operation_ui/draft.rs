//! arc's model for a draft: the SQL arc would write for an operation, asked
//! for before the operation is recorded.
//!
//! Brightfield writes no operation's SQL, so a preview has to run the model
//! arc writes, and recording a step is how arc writes one. So the draft
//! is recorded into a Protocol that is thrown away. [`draft_model`] makes a
//! folder under the system's temporary folder, writes a manifest there with a
//! stub step for each table the draft reads, asks arc to record the draft in
//! it, reads back the model arc wrote, and removes the folder.
//!
//! A stub step's model makes a table of the name the draft reads and one
//! column of no meaning. That is enough, because arc finds the tables a
//! Protocol makes by reading its models, and runs no step when
//! it records. The text arc writes there is the text it writes for the same
//! request in the real Protocol
//! (`a_drafts_model_is_what_arc_records_in_the_protocol` in
//! `tests/operation_reads.rs`).
//!
//! arc keeps snapshots of a Protocol's manifest in a history folder outside
//! the Protocol, keyed by its path. A draft is recorded at each edit, so its
//! snapshots are sent to a folder inside the one thrown away, and the
//! analyst's history holds no entry for a folder that no longer exists.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::operation::{record_step_with_history, StepRequest};
use crate::run::Runner;

/// The model arc writes for a draft, and what arc noted as it wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftModel {
    /// The model, as arc wrote it.
    pub model: String,
    /// arc's notes, in arc's order.
    pub notes: Vec<String>,
}

/// A folder under the system's temporary folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "brightfield-draft-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("models"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// **The model arc writes for `request` recorded as the step `name`**, with
/// arc's notes. No file of the analyst's is read or written: the recording is
/// into a folder made for it and removed after.
///
/// # Errors
///
/// arc's refusal of the request, in arc's words, or why the folder could not
/// be made or the model read back.
pub fn draft_model(
    runner: &Runner,
    name: &str,
    request: &StepRequest,
) -> Result<DraftModel, String> {
    let scratch = Scratch::new()?;
    let dir = &scratch.0;
    let mut manifest = String::from("name: draft\nsteps:");
    let tables = crate::typed_sql::tables_read(request);
    if tables.is_empty() {
        manifest.push_str(" []\n");
    } else {
        manifest.push('\n');
    }
    for (i, table) in tables.iter().enumerate() {
        let stub = format!("bf_stub_{}", i + 1);
        let model = format!("models/{stub}.sql");
        write(
            &dir.join(&model),
            &format!(
                "CREATE OR REPLACE TABLE \"{}\" AS SELECT 1 AS x;\n",
                table.replace('"', "\"\"")
            ),
        )?;
        manifest.push_str(&format!("  - name: {stub}\n    sql: {model}\n"));
    }
    write(&dir.join("arcform.yaml"), &manifest)?;
    let recorded =
        record_step_with_history(runner, dir, name, request, Some(&dir.join("history")))?;
    let text = std::fs::read_to_string(dir.join("arcform.yaml"))
        .map_err(|e| format!("the draft's manifest did not read back: {e}"))?;
    let parsed = brightfield_protocol::parse_manifest_str(&text)
        .map_err(|e| format!("the draft's manifest did not read back: {e}"))?;
    let path = parsed
        .steps
        .iter()
        .find(|step| step.name == name)
        .and_then(|step| step.sql.as_ref())
        .ok_or_else(|| format!("arc recorded no model for {name}"))?;
    let model = std::fs::read_to_string(dir.join(path))
        .map_err(|e| format!("arc's model for {name} did not read back: {e}"))?;
    Ok(DraftModel {
        model,
        notes: recorded.notes,
    })
}
