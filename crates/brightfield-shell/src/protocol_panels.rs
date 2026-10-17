//! An arc Protocol's panels, loaded headless: one outcome per `panels/*.yaml`.
//!
//! A Protocol an agent wrote draws its charts as panel files, each a Mosaic spec
//! whose marks read the tables the Protocol's run made, by name: `data: { from:
//! T }` is the table `T` in the Protocol's database whenever the panel's own
//! top-level `data:` does not define `T`. That is the rule arc's panel check
//! applies, and [`load`] reads the panels the same way, so a tile that cannot
//! draw says the same thing arc's report says about it.
//!
//! **The database is never held open.** DuckDB lets several processes read a
//! file but refuses a writer while any reader holds it, so a handle kept for as
//! long as the Protocol is open would make the agent's `arc run`, and
//! Brightfield's own Run, fail. Past arc's panel check, which opens and closes
//! it read-only, [`load`] opens the database once: it attaches it
//! read-only to an in-memory DuckDB, copies each table a panel reads into that
//! in-memory database, detaches, and writes the copies to Parquet in a temp
//! directory [`ProtocolPanels`] owns. Each panel is then composed against those
//! files ([`with_protocol_tables`]), so no chart's session touches the
//! `.duckdb` file either.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arc::spec::{PanelFinding, MANIFEST_FILENAME};
use brightfield_model::panel_capture::panel_file;
use brightfield_spec::layout::Rect;
use serde_yaml::{Mapping, Value};

use crate::design::Mode;
use crate::pipeline::{compose_spec_str_in, Composed, LiveDashboard};
use crate::sql_ident::quote;

/// The panels folder of a Protocol, relative to it.
const PANELS_DIR: &str = "panels";

/// The schema name the Protocol's database is attached under while it is copied.
const ATTACHED: &str = "__protocol";

/// DuckDB's wording when another process holds the file: the same two phrases
/// arc's own read-only open recognises.
const LOCK_WORDING: [&str; 2] = ["Could not set lock on file", "Conflicting lock is held"];

/// arc's wording when its read-only open meets that lock.
const ARC_LOCK_WORDING: &str = "is in use by another process";

/// A Protocol folder opened for its panels.
pub struct ProtocolPanels {
    /// The Protocol folder.
    pub dir: PathBuf,
    /// The Protocol's `name:`.
    pub name: String,
    /// Where the Protocol's database is, by arc's rules.
    pub db: PathBuf,
    /// The newest run's id, when arc can name one; a reload is due when it changes.
    pub run_id: Option<String>,
    /// One tile per panel, in file-name order.
    pub tiles: Vec<PanelTile>,
    /// The Parquet snapshot the tiles read, removed when this drops.
    tables: tempfile::TempDir,
}

impl ProtocolPanels {
    /// The directory holding the Parquet snapshot the tiles were composed against.
    #[must_use]
    pub fn tables_dir(&self) -> &Path {
        self.tables.path()
    }

    /// Compose each tile that drew again, into `viewport` and in `mode`'s ink,
    /// from the same snapshot: what a tile does when the box it is drawn in
    /// changes, or the window's mode does. A tile holding problems keeps them,
    /// and a tile that no longer composes holds why.
    pub fn compose_tiles_at(&mut self, viewport: Rect, mode: Mode) {
        let tables = self.tables.path().to_path_buf();
        for tile in &mut self.tiles {
            if tile.outcome.is_ok() {
                tile.outcome = compose_tile(&tile.source, &self.dir, &tables, viewport, mode)
                    .map_err(|p| vec![p]);
            }
        }
    }

    /// The tile at `index` as a live dashboard over the snapshot, with its
    /// first picture: what opening a tile as the chart document draws, so a
    /// brush, a click or an axis edit re-queries as on any chart.
    ///
    /// # Errors
    ///
    /// No tile at `index`, or the panel does not parse or load.
    pub fn live_tile(&self, index: usize) -> Result<(LiveDashboard, Composed), String> {
        let tile = self
            .tiles
            .get(index)
            .ok_or_else(|| format!("the Protocol has no panel {index}"))?;
        let text = with_protocol_tables(&tile.source, self.tables.path())?;
        let parsed = brightfield_spec::parse_spec(&text, brightfield_spec::Format::Yaml)
            .map_err(|e| format!("parse error: {e}"))?;
        let name = tile
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let mut live = LiveDashboard::load_parsed(parsed, name, Some(&self.dir))?;
        let composed = live.present()?;
        Ok((live, composed))
    }
}

/// One panel file and what came of drawing it.
pub struct PanelTile {
    /// The panel file, relative to the Protocol: `panels/x.yaml`.
    pub file: PathBuf,
    /// The panel's own `title:`, when it declares one.
    pub title: Option<String>,
    /// The file's text as written.
    pub source: String,
    /// The composed chart, or every problem that kept it from drawing: all of
    /// arc's findings for the panel, else what Brightfield refused. Never an
    /// empty list.
    pub outcome: Result<Composed, Vec<PanelProblem>>,
}

/// Why one panel did not draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelProblem {
    /// The file is not YAML, or holds no `plot`.
    Unreadable(String),
    /// A mark reads a table that is neither in the panel's data nor in the database.
    MissingTable {
        /// The table the mark names.
        from: String,
    },
    /// A mark reads a column its table does not have.
    MissingColumn {
        /// The table the mark reads.
        from: String,
        /// The column it names.
        column: String,
    },
    /// The panel uses components Brightfield does not draw, by their wire names.
    Unsupported(Vec<String>),
    /// Anything else the composition refused, in its own words.
    Failed(String),
}

impl PanelProblem {
    /// The problem in plain words. The three arc's panel check finds read as
    /// arc's report prints them.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unreadable(reason) => PanelFinding::Unreadable {
                reason: reason.clone(),
            }
            .to_string(),
            Self::MissingTable { from } => {
                PanelFinding::MissingTable { from: from.clone() }.to_string()
            }
            Self::MissingColumn { from, column } => PanelFinding::MissingColumn {
                from: from.clone(),
                column: column.clone(),
            }
            .to_string(),
            Self::Unsupported(names) => {
                format!("Brightfield does not draw {} yet", names.join(", "))
            }
            Self::Failed(reason) => reason.clone(),
        }
    }

    fn from_finding(finding: &PanelFinding) -> Self {
        match finding {
            PanelFinding::MissingTable { from } => Self::MissingTable { from: from.clone() },
            PanelFinding::MissingColumn { from, column } => Self::MissingColumn {
                from: from.clone(),
                column: column.clone(),
            },
            PanelFinding::Unreadable { reason } => Self::Unreadable(reason.clone()),
        }
    }
}

/// Why a Protocol folder did not open for its panels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenProblem {
    /// The folder has no `arcform.yaml`.
    NotAProtocol(PathBuf),
    /// The Protocol has no database yet: it has never run.
    NotRun {
        /// The Protocol folder.
        dir: PathBuf,
    },
    /// The Protocol has no `panels/*.yaml`.
    NoPanels {
        /// The Protocol folder.
        dir: PathBuf,
    },
    /// Another process holds the database for writing: a run is in progress.
    Locked {
        /// The database file.
        db: PathBuf,
    },
    /// The database was written by a DuckDB whose storage this one cannot read.
    EngineMismatch {
        /// The database file.
        db: PathBuf,
        /// DuckDB's own error.
        detail: String,
    },
    /// arc refused, or the database could not be read for another reason.
    Arc(String),
}

impl OpenProblem {
    /// The problem in plain words, naming the folder or file it is about.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NotAProtocol(dir) => {
                format!("{} is not a Protocol: it has no {MANIFEST_FILENAME}", dir.display())
            }
            Self::NotRun { dir } => format!(
                "the Protocol at {} has not run yet, so there are no tables to draw; run it to make them",
                dir.display()
            ),
            Self::NoPanels { dir } => format!(
                "the Protocol at {} has no panels to draw: {PANELS_DIR}/ holds no .yaml file",
                dir.display()
            ),
            Self::Locked { db } => format!(
                "the Protocol is being run: its database {} is held by the run; open it again when the run has finished",
                db.display()
            ),
            Self::EngineMismatch { db, detail } => {
                let versions = storage_versions(detail).map_or_else(
                    || first_line(detail).to_string(),
                    |(file, readable)| {
                        format!("the file's storage version is {file}, and this Brightfield reads {readable}")
                    },
                );
                format!(
                    "the Protocol's database {} was written by a DuckDB this Brightfield cannot read: {versions}",
                    db.display()
                )
            }
            Self::Arc(message) => message.clone(),
        }
    }
}

/// Whether `panels/` in `dir` holds a panel other than the one chart file
/// Brightfield itself writes for the Protocol, `panel_file(dir, name)`: the
/// sign of panels someone else wrote.
#[must_use]
pub fn agent_panels_in(dir: &Path) -> bool {
    let own = arc::spec::Manifest::load(dir)
        .ok()
        .map(|manifest| panel_file(dir, &manifest.name));
    panel_paths(dir)
        .into_iter()
        .any(|path| Some(&path) != own.as_ref())
}

/// How many panels the Protocol in `dir` holds: the tiles [`load`] makes for it.
#[must_use]
pub fn panel_count(dir: &Path) -> usize {
    panel_paths(dir).len()
}

/// Open the Protocol in `dir` and compose each of its panels into `viewport`.
///
/// A panel that cannot draw is a tile holding its problem; the others draw.
///
/// # Errors
///
/// An [`OpenProblem`] when the folder does not open for its panels.
pub fn load(dir: &Path, viewport: Rect) -> Result<ProtocolPanels, OpenProblem> {
    if !dir.join(MANIFEST_FILENAME).is_file() {
        return Err(OpenProblem::NotAProtocol(dir.to_path_buf()));
    }
    let manifest = arc::spec::Manifest::load(dir).map_err(|e| OpenProblem::Arc(e.to_string()))?;
    let db = arc::spec::protocol_db_path(dir).map_err(|e| OpenProblem::Arc(e.to_string()))?;
    let paths = panel_paths(dir);
    if paths.is_empty() {
        return Err(OpenProblem::NoPanels {
            dir: dir.to_path_buf(),
        });
    }
    if !db.is_file() {
        return Err(OpenProblem::NotRun {
            dir: dir.to_path_buf(),
        });
    }

    // Every table any panel reads from the database. A panel that does not
    // parse reads none; one that names a table the database lacks gets arc's
    // finding below, and nothing is copied for it.
    let mut wanted = BTreeSet::new();
    for path in &paths {
        if let Some(panel) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_yaml::from_str::<Value>(&text).ok())
        {
            wanted.extend(database_tables(&panel));
        }
    }
    // arc's check opens the database read-only and closes it, and it goes
    // first: a POSIX record lock is the process's, so closing any handle on the
    // file releases every lock this process holds on it, and a check after the
    // snapshot would hide a snapshot that failed to let go
    // (`arc_can_run_while_the_protocol_is_open`).
    let (_, checked) =
        arc::spec::check_panels(dir).map_err(|e| check_problem(dir, &db, &e.to_string()))?;
    let tables = tempfile::Builder::new()
        .prefix("brightfield-protocol-tables-")
        .tempdir()
        .map_err(|e| {
            OpenProblem::Arc(format!(
                "could not make a folder for the panels' tables: {e}"
            ))
        })?;
    snapshot_tables(&db, &wanted, tables.path())?;
    let run_id = arc::spec::run_tables(dir, None).ok().map(|run| run.run_id);

    let tiles = checked
        .into_iter()
        .map(|(file, findings)| {
            let source = std::fs::read_to_string(dir.join(&file)).unwrap_or_default();
            let title = serde_yaml::from_str::<Value>(&source)
                .ok()
                .and_then(|panel| panel.get("title")?.as_str().map(str::to_string));
            let outcome = if findings.is_empty() {
                compose_tile(&source, dir, tables.path(), viewport, Mode::Light)
                    .map_err(|p| vec![p])
            } else {
                Err(findings.iter().map(PanelProblem::from_finding).collect())
            };
            PanelTile {
                file,
                title,
                source,
                outcome,
            }
        })
        .collect();

    Ok(ProtocolPanels {
        dir: dir.to_path_buf(),
        name: manifest.name,
        db,
        run_id,
        tiles,
        tables,
    })
}

/// `panel` with a `T: { file: <tables_dir>/t.parquet }` entry added to its
/// top-level `data:` for each table `T` it reads from the database, so the
/// existing Parquet source loads the snapshot [`load`] wrote. The file is
/// named for the table's name in lower case, the one copy [`load`] makes
/// however many spellings the panels use: DuckDB matches names without regard
/// to case, as arc's check does. Entries the panel defines itself are left as
/// they are. Nothing is written to disk.
///
/// # Errors
///
/// The panel is not YAML, or is not a mapping.
pub fn with_protocol_tables(panel: &str, tables_dir: &Path) -> Result<String, String> {
    let mut doc: Value =
        serde_yaml::from_str(panel).map_err(|e| format!("the panel is not valid YAML: {e}"))?;
    let reads = database_tables(&doc);
    let Some(top) = doc.as_mapping_mut() else {
        return Err("the panel is not a YAML mapping".to_string());
    };
    if !reads.is_empty() {
        let data = top
            .entry(Value::from("data"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        // A `data:` that is not a mapping defines no table (arc's check reads
        // it the same way), so it is replaced rather than added to.
        if !data.is_mapping() {
            *data = Value::Mapping(Mapping::new());
        }
        let data = data.as_mapping_mut().expect("made a mapping above");
        for table in reads {
            let file = tables_dir.join(parquet_name(&table.to_lowercase()));
            let mut source = Mapping::new();
            source.insert(
                Value::from("file"),
                Value::from(file.to_string_lossy().into_owned()),
            );
            data.insert(Value::from(table), Value::Mapping(source));
        }
    }
    serde_yaml::to_string(&doc).map_err(|e| format!("the panel could not be written back: {e}"))
}

/// The tables [`with_protocol_tables`] adds an entry for in `panel`: those its
/// marks read that its own top-level `data:` does not define, as written.
/// None when the panel is not YAML.
#[must_use]
pub fn protocol_tables(panel: &str) -> Vec<String> {
    serde_yaml::from_str::<Value>(panel)
        .map(|doc| database_tables(&doc).into_iter().collect())
        .unwrap_or_default()
}

/// `text` with the `data:` entries [`with_protocol_tables`] added for the
/// tables named in `injected` taken out again, so a panel opened against the
/// snapshot is saved as the panel it was.
///
/// An entry is taken out when it is one [`with_protocol_tables`] writes:
/// a table named in `injected` whose source is a mapping holding a `file:`
/// naming a `.parquet` file as its one key. A `data:` left empty by that is
/// taken out too, since the panel read those tables from the database and
/// declared no `data:` of its own. A text holding no such entry is handed back
/// byte for byte; one that does is written back through YAML, as
/// [`with_protocol_tables`] wrote it.
#[must_use]
pub fn strip_protocol_tables(text: &str, injected: &[String]) -> String {
    let Ok(mut doc) = serde_yaml::from_str::<Value>(text) else {
        return text.to_string();
    };
    let Some(top) = doc.as_mapping_mut() else {
        return text.to_string();
    };
    let Some(data) = top.get_mut("data").and_then(Value::as_mapping_mut) else {
        return text.to_string();
    };
    let ours = |source: &Value| {
        source.as_mapping().is_some_and(|m| {
            m.len() == 1
                && m.get("file")
                    .and_then(Value::as_str)
                    .is_some_and(|f| f.ends_with(".parquet"))
        })
    };
    let before = data.len();
    data.retain(|name, source| {
        !(name
            .as_str()
            .is_some_and(|n| injected.iter().any(|i| i == n))
            && ours(source))
    });
    if data.len() == before {
        return text.to_string();
    }
    if data.is_empty() {
        top.remove("data");
    }
    serde_yaml::to_string(&doc).unwrap_or_else(|_| text.to_string())
}

/// Compose one panel that arc's check found nothing wrong with.
fn compose_tile(
    source: &str,
    dir: &Path,
    tables_dir: &Path,
    viewport: Rect,
    mode: Mode,
) -> Result<Composed, PanelProblem> {
    let text = with_protocol_tables(source, tables_dir).map_err(PanelProblem::Failed)?;
    let parsed = brightfield_spec::parse_spec(&text, brightfield_spec::Format::Yaml)
        .map_err(|e| PanelProblem::Failed(format!("parse error: {e}")))?;
    let mut blocking: Vec<String> = Vec::new();
    for entry in brightfield_conformance::preflight(&parsed.spec).blocking() {
        let name = entry.identity.wire_name().to_string();
        if !blocking.contains(&name) {
            blocking.push(name);
        }
    }
    if !blocking.is_empty() {
        return Err(PanelProblem::Unsupported(blocking));
    }
    compose_spec_str_in(&text, Some(dir), viewport, mode).map_err(PanelProblem::Failed)
}

/// Every `panels/*.yaml` file of the Protocol in `dir`, sorted: the files arc's
/// panel check reads, with the same test.
fn panel_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir.join(PANELS_DIR)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    paths.sort();
    paths
}

/// The tables `panel`'s marks read that its own top-level `data:` does not
/// define, as written. A `catalog.schema.table` name is left out, as arc's
/// check leaves it.
fn database_tables(panel: &Value) -> BTreeSet<String> {
    let inline: BTreeSet<&str> = panel
        .get("data")
        .and_then(Value::as_mapping)
        .map(|m| m.keys().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut marks = Vec::new();
    collect_marks(panel, &mut marks);
    marks
        .into_iter()
        .filter_map(|mark| mark.get("data")?.get("from")?.as_str())
        .filter(|from| !inline.contains(from) && from.matches('.').count() < 2)
        .map(str::to_string)
        .collect()
}

/// The marks under `plot`, and under the `plot` of each `hconcat`/`vconcat`
/// child: where arc's check looks.
fn collect_marks<'a>(node: &'a Value, into: &mut Vec<&'a Value>) {
    if let Some(list) = node.get("plot").and_then(Value::as_sequence) {
        into.extend(list);
    }
    for key in ["hconcat", "vconcat"] {
        for child in node
            .get(key)
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            collect_marks(child, into);
        }
    }
}

/// The Parquet file name the table `table` is written to: the name itself when
/// it is a plain identifier, and every other byte as `%XX`, so no name reaches
/// outside the snapshot folder. Callers pass the name in lower case, so two
/// names differing only in case, which are one table to DuckDB, are one file,
/// and a case-insensitive file system cannot fold two copies together.
fn parquet_name(table: &str) -> String {
    let mut name = String::with_capacity(table.len() + 8);
    for byte in table.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
            name.push(char::from(byte));
        } else {
            name.push_str(&format!("%{byte:02X}"));
        }
    }
    name.push_str(".parquet");
    name
}

/// `text` as a SQL string literal.
fn sql_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Copy each table of `wanted` the database at `db` holds into Parquet under
/// `out`, holding the database only between the attach and the detach.
fn snapshot_tables(db: &Path, wanted: &BTreeSet<String>, out: &Path) -> Result<(), OpenProblem> {
    let failed = |what: &str, e: duckdb::Error| {
        OpenProblem::Arc(format!(
            "could not read the Protocol's database {}: {what}: {e}",
            db.display()
        ))
    };
    let conn =
        duckdb::Connection::open_in_memory().map_err(|e| failed("an in-memory database", e))?;
    conn.execute_batch(&format!(
        "ATTACH {} AS {} (READ_ONLY)",
        sql_string(&db.to_string_lossy()),
        quote(ATTACHED)
    ))
    .map_err(|e| database_problem(db, &e.to_string()))?;

    // Copy while attached, then let go of the file before anything is written.
    let copied = copy_attached(&conn, wanted).map_err(|e| failed("copying its tables", e));
    let detached = conn
        .execute_batch(&format!("DETACH {}", quote(ATTACHED)))
        .map_err(|e| failed("detaching it", e));
    let copied = copied?;
    detached?;

    for table in copied {
        let file = out.join(parquet_name(&table));
        conn.execute_batch(&format!(
            "COPY {} TO {} (FORMAT parquet)",
            quote(&table),
            sql_string(&file.to_string_lossy())
        ))
        .map_err(|e| failed("writing its tables to Parquet", e))?;
    }
    Ok(())
}

/// Copy each wanted table the attached database holds into the in-memory one,
/// once per name in lower case, and answer the lower-cased names copied.
///
/// A name resolves as arc's check resolves it, ignoring case: `schema.table`
/// exactly, and a bare name as `main.name`, else the first schema holding it.
/// `CO2` and `co2` are one table to DuckDB, so a second spelling is not copied
/// again: a second `CREATE TABLE` of it would fail as already existing.
fn copy_attached(
    conn: &duckdb::Connection,
    wanted: &BTreeSet<String>,
) -> Result<Vec<String>, duckdb::Error> {
    let mut stmt = conn.prepare(
        "SELECT table_schema, table_name FROM information_schema.tables \
         WHERE table_catalog = ? ORDER BY table_schema, table_name",
    )?;
    let present: Vec<(String, String)> = stmt
        .query_map([ATTACHED], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut copied: Vec<String> = Vec::new();
    for name in wanted {
        let lowered = name.to_lowercase();
        if copied.contains(&lowered) {
            continue;
        }
        let found = match lowered.split_once('.') {
            Some((schema, table)) => present
                .iter()
                .find(|(s, t)| s.to_lowercase() == schema && t.to_lowercase() == table),
            None => present
                .iter()
                .find(|(s, t)| s == "main" && t.to_lowercase() == lowered)
                .or_else(|| present.iter().find(|(_, t)| t.to_lowercase() == lowered)),
        };
        let Some((schema, table)) = found else {
            continue;
        };
        conn.execute_batch(&format!(
            "CREATE TABLE {} AS SELECT * FROM {}.{}.{}",
            quote(&lowered),
            quote(ATTACHED),
            quote(schema),
            quote(table)
        ))?;
        copied.push(lowered);
    }
    Ok(copied)
}

/// The problem arc's panel check of the Protocol in `dir` failing with `error`
/// is: a database problem when the error came from opening `db`, which arc
/// words as a lock refusal or prefixes with `database:`, else arc's own.
fn check_problem(dir: &Path, db: &Path, error: &str) -> OpenProblem {
    let from_database = error.starts_with("database:")
        || error.contains(ARC_LOCK_WORDING)
        || LOCK_WORDING.iter().any(|w| error.contains(w));
    if from_database {
        database_problem(db, error)
    } else {
        OpenProblem::Arc(format!(
            "arc could not check the panels of the Protocol at {}: {error}",
            dir.display()
        ))
    }
}

/// The problem an open of `db` that failed with `error` is, whether the error
/// is DuckDB's own or arc's wording of it.
fn database_problem(db: &Path, error: &str) -> OpenProblem {
    if error.contains(ARC_LOCK_WORDING) || LOCK_WORDING.iter().any(|w| error.contains(w)) {
        OpenProblem::Locked {
            db: db.to_path_buf(),
        }
    } else if storage_versions(error).is_some() || error.contains("database file with version") {
        OpenProblem::EngineMismatch {
            db: db.to_path_buf(),
            detail: error.to_string(),
        }
    } else {
        OpenProblem::Arc(format!(
            "could not read the Protocol's database {}: {}",
            db.display(),
            first_line(error)
        ))
    }
}

/// The file's storage version and the versions this DuckDB reads, from
/// DuckDB's "Trying to read a database file with version number N, but we can
/// only read version M" (or "versions between A and B").
fn storage_versions(error: &str) -> Option<(String, String)> {
    let rest = error.split_once("version number ")?.1;
    let (file, rest) = rest.split_once(',')?;
    let readable = rest.split_once("can only read ")?.1;
    let readable = readable.split(['.', '\n']).next()?.trim();
    let readable = readable
        .strip_prefix("versions between ")
        .map(|span| span.replacen(" and ", " to ", 1))
        .or_else(|| readable.strip_prefix("version ").map(str::to_string))?;
    Some((file.trim().to_string(), readable))
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DuckDB 1.5's wording for a file from a later DuckDB reads as both versions.
    #[test]
    fn a_storage_version_error_names_both_versions() {
        let error = "IO Error: Trying to read a database file with version number 69, but we \
                     can only read versions between 64 and 68.\nThe database file was created \
                     with a newer version of DuckDB.";
        assert_eq!(
            storage_versions(error),
            Some(("69".to_string(), "64 to 68".to_string()))
        );
        let older = "Trying to read a database file with version number 51, but we can only read version 64.";
        assert_eq!(
            storage_versions(older),
            Some(("51".to_string(), "64".to_string()))
        );
    }

    /// A plain identifier is its own file name, and nothing else can climb out.
    #[test]
    fn a_table_name_becomes_a_file_name_inside_the_folder() {
        assert_eq!(
            parquet_name("co2_per_capita_change"),
            "co2_per_capita_change.parquet"
        );
        assert_eq!(parquet_name("../x"), "%2E%2E%2Fx.parquet");
        assert_eq!(parquet_name("s.t"), "s%2Et.parquet");
    }
}
