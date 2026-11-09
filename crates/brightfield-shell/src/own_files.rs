//! The Protocol folder an analyst's own files are written as: one arc load step per table, the
//! raw tables those steps make, a FineType profile of them, and one Frictionless data package per
//! source file.
//!
//! The load steps read the files where they are, by absolute path, so nothing the analyst gave is
//! copied or moved. The Protocol is made in the Protocols folder ([`crate::protocols_dir::root`]),
//! in a folder of its own per open, named after the first source.
//!
//! The profile is a Parquet snapshot of each raw table (lineage columns left out), read through
//! the engine's profiler as any opened file is, and it is kept in a `.profile` folder inside the
//! Protocol so nothing later mistakes it for data the analyst gave. The data packages are the
//! boundary file the model is built from; FineType will write them once it can.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use brightfield_engine::sources::{self, Reader, SourceSet, SourceTable};
use brightfield_engine::{ColumnProfile, Engine, LoadOptions, ProfileOutcome, SourceProfile};
// The lineage columns every raw table carries describe where a row came from, not what it holds,
// so neither the profile nor the data packages list them.
use brightfield_ossie::{Model, LINEAGE};
use brightfield_protocol::silver::{cast_expr, failed_sql, silver_sql, FinetypeTransforms};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use serde_json::{json, Value};

use crate::discovery::MODEL_FILE;
use crate::protocols_dir;
use crate::run::{Finished, Run, Runner};
use crate::starts;

/// The folder under `<datasets>/protocols/` that holds the Protocols made from an analyst's own
/// files.
pub const OWN_DIR: &str = "own";

/// Where the raw tables' Parquet snapshots are kept inside the Protocol folder.
const PROFILE_DIR: &str = ".profile";

/// The file in the Protocol folder recording the tables the open found and the files it passed
/// over, so the folder can be opened again ([`reopen`]) without finding them afresh.
pub(crate) const SOURCES_FILE: &str = ".sources.json";

/// The name prefixes of the silver steps and the files holding their SQL.
const SILVER_STEP: &str = "silver_";
const FAILED_STEP: &str = "failed_";
const SILVER_FILE: &str = "02_silver_";
const FAILED_FILE: &str = "02_failed_";

/// An analyst's own files written as a Protocol.
#[derive(Debug, Clone)]
pub struct OwnProtocol {
    /// The Protocol folder.
    pub dir: PathBuf,
    /// The tables found in what was opened.
    pub sources: SourceSet,
    /// One profile per table, in table order; empty until [`profile`] has run.
    pub profiles: Vec<SourceProfile>,
}

/// Make the Protocol folder for `opened` (a folder, workbook or database file): its
/// `arcform.yaml` and one load step per table, reading the files where they are.
///
/// # Errors
///
/// The reason when nothing in `opened` can be read, or the folder cannot be written.
pub fn create(opened: &Path) -> Result<OwnProtocol, String> {
    create_in(&protocols_dir::root(), opened)
}

/// [`create`], making the Protocol folder in `parent` rather than in the Protocols folder.
///
/// # Errors
///
/// The reason when nothing in `opened` can be read, or the folder cannot be written.
pub fn create_in(parent: &Path, opened: &Path) -> Result<OwnProtocol, String> {
    create_of(parent, opened, sources::discover(opened)?)
}

/// Make one Protocol folder in the Protocols folder for the paths in `opened` together, named
/// after the first: what one gesture handed over at once, as [`create`] does for one path. The
/// paths `refused` before discovery are recorded as skipped with their reasons.
///
/// # Errors
///
/// The reason when `opened` is empty, nothing in it can be read, or the folder cannot be written.
pub fn create_all(
    opened: &[std::path::PathBuf],
    refused: Vec<(std::path::PathBuf, String)>,
) -> Result<OwnProtocol, String> {
    let first = opened.first().ok_or("nothing was given to open")?;
    let mut found = sources::discover_all(opened)?;
    found.skipped.extend(refused);
    create_of(&protocols_dir::root(), first, found)
}

/// The Protocol folder for `sources`, in `parent`, named after `named`.
fn create_of(parent: &Path, named: &Path, sources: SourceSet) -> Result<OwnProtocol, String> {
    if sources.tables.is_empty() {
        return Err(format!(
            "{}: no tables Brightfield can read{}",
            named.display(),
            skipped_note(&sources)
        ));
    }
    let (dir, name) = protocols_dir::claim(parent, &slug_of(named)).map_err(|e| e.to_string())?;

    let models = dir.join("models");
    std::fs::create_dir(&models).map_err(|e| format!("{}: {e}", models.display()))?;
    let mut manifest = format!("name: {name}\nengine: duckdb\nsteps:\n");
    for table in &sources.tables {
        let step = load_step(&table.name);
        let file = format!("models/01_{step}.sql");
        let sql = format!("{}\n", sources::load_sql(table, &dir));
        std::fs::write(dir.join(&file), sql).map_err(|e| format!("{file}: {e}"))?;
        let _ = writeln!(manifest, "  - name: {step}\n    sql: {file}");
        let _ = writeln!(manifest, "    produces:\n      - raw_{}", table.name);
    }
    let path = dir.join("arcform.yaml");
    std::fs::write(&path, manifest).map_err(|e| format!("{}: {e}", path.display()))?;
    write_sources(&dir, &sources)?;
    Ok(OwnProtocol {
        dir,
        sources,
        profiles: Vec::new(),
    })
}

/// Held by the own-files flow's writes to a Protocol's database, [`run_loads`] and
/// [`drop_unmade_silver`], for their whole run, and by a discovery pass while it copies the
/// database for its child, so the copy is not taken part way through one of those writes. The
/// window's own runs start `arc` without it: they run the workbench's Protocols, and an own-files
/// Protocol is run through this module. The lock is this process's, so another process writing the
/// same folder is not held off. A writer that panicked left the database as an interrupted run
/// would, so the lock is taken again rather than refused.
pub fn database_lock() -> std::sync::MutexGuard<'static, ()> {
    static DATABASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DATABASE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Run the Protocol's steps with `runner`, which makes the raw tables, and wait for it.
///
/// # Errors
///
/// What `arc` printed, when the run did not succeed.
pub fn run_loads(p: &OwnProtocol, runner: &Runner) -> Result<(), String> {
    let _writing = database_lock();
    let finished: Finished = Run::begin(runner, &p.dir, || {}).wait();
    if finished.succeeded {
        Ok(())
    } else {
        Err(finished.log)
    }
}

/// The datasets that may have a silver table: the source tables, then each dataset of `model` with
/// a combination, in model order and without repeats. A combined dataset has no source table of
/// its own, because it is built from its parts' raw tables by a query.
/// The name of the step that loads the source table `table`.
fn load_step(table: &str) -> String {
    format!("load_{table}")
}

/// The names of `p`'s load steps, one per source table. A step is a load by its exact name, not
/// by its prefix: a grid view recorded on a table named `load_profile` is named
/// `load_profile_filtered_by_…`, and is not a load.
fn load_steps(p: &OwnProtocol) -> HashSet<String> {
    p.sources
        .tables
        .iter()
        .map(|t| load_step(&t.name))
        .collect()
}

/// Whether the manifest step `step` is one of `loads`.
fn is_load(step: &serde_yaml::Value, loads: &HashSet<String>) -> bool {
    step.get("name")
        .and_then(serde_yaml::Value::as_str)
        .is_some_and(|n| loads.contains(n))
}

fn silver_datasets(model: &Model, sources: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = sources.iter().map(|s| (*s).to_string()).collect();
    for d in &model.datasets {
        if d.meta.combination.is_some() && !names.contains(&d.name) {
            names.push(d.name.clone());
        }
    }
    names
}

/// Write the silver steps for `model`: for each dataset that has a silver table, a step that casts
/// the raw table through FineType's transformation contracts and one that keeps the values that did
/// not cast, as `models/02_silver_<name>.sql` and `models/02_failed_<name>.sql`.
///
/// Every call rewrites them: the new files and manifest are written first and the earlier files
/// the manifest no longer names are removed after, so a failed write leaves a usable folder. A
/// dataset that has since been excluded or folded into another loses its silver table's steps. The
/// load steps are left as they are, and so are steps recorded after them, such as a grid view arc
/// recorded on a silver table: the silver steps are put back straight after the loads, ahead of
/// what reads them. Run the Protocol afterwards to make the tables.
///
/// # Errors
///
/// The reason when the manifest cannot be read or written, or a step file cannot be written.
pub fn write_silver(p: &OwnProtocol, model: &Model) -> Result<(), String> {
    let transforms = FinetypeTransforms::vendored();
    let manifest_path = p.dir.join("arcform.yaml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let mut manifest: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let steps = manifest
        .get_mut("steps")
        .and_then(serde_yaml::Value::as_sequence_mut)
        .ok_or_else(|| format!("{}: no steps", manifest_path.display()))?;
    steps.retain(|s| {
        let name = s
            .get("name")
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or("");
        !(name.starts_with(SILVER_STEP) || name.starts_with(FAILED_STEP))
    });
    let loads = load_steps(p);
    let mut at = steps
        .iter()
        .rposition(|s| is_load(s, &loads))
        .map_or(0, |i| i + 1);

    let mut written: HashSet<String> = HashSet::new();
    let sources: Vec<&str> = p.sources.tables.iter().map(|t| t.name.as_str()).collect();
    for name in silver_datasets(model, &sources) {
        let silver = silver_sql(model, &name, &transforms);
        if silver.is_empty() {
            continue;
        }
        let failed = failed_sql(model, &name, &transforms);
        for (step, file, sql, produces) in [
            (
                format!("{SILVER_STEP}{}", name),
                format!("models/{SILVER_FILE}{}.sql", name),
                silver,
                name.clone(),
            ),
            (
                format!("{FAILED_STEP}{}", name),
                format!("models/{FAILED_FILE}{}.sql", name),
                failed,
                format!("{}__failed", name),
            ),
        ] {
            std::fs::write(p.dir.join(&file), format!("{sql};\n"))
                .map_err(|e| format!("{file}: {e}"))?;
            written.insert(file.clone());
            steps.insert(
                at,
                serde_yaml::from_str(&format!(
                    "name: {}\nsql: {}\nproduces:\n  - {}\n",
                    quoted_yaml(&step),
                    quoted_yaml(&file),
                    quoted_yaml(&produces)
                ))
                .map_err(|e| e.to_string())?,
            );
            at += 1;
        }
    }
    let out = serde_yaml::to_string(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&manifest_path, out).map_err(|e| format!("{}: {e}", manifest_path.display()))?;

    // Only now, with the manifest naming the new files, are the old ones that nothing names removed.
    let models = p.dir.join("models");
    let entries = std::fs::read_dir(&models).map_err(|e| format!("{}: {e}", models.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = (name.starts_with(SILVER_FILE) || name.starts_with(FAILED_FILE))
            && !written.contains(&format!("models/{name}"));
        if stale {
            std::fs::remove_file(entry.path())
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Drop the silver and failed-value tables of each table `model` gives none, from the Protocol's
/// database: a table left out or folded into another keeps no table of its own.
///
/// arc leaves standing what a step it no longer runs once made, so [`write_silver`] taking a
/// table's steps out of the manifest would leave its tables in the database
/// (`left_out_table_is_not_left_materialised` in `tests/model_reshape.rs`). The record arc keeps
/// of each of those steps' last success goes too, so a table brought back has its steps run again
/// rather than skipped as unchanged. A Protocol that has not been run has no database, and nothing to
/// drop.
///
/// # Errors
///
/// The reason when the database cannot be opened or a table cannot be dropped.
pub fn drop_unmade_silver(p: &OwnProtocol, model: &Model) -> Result<(), String> {
    let _writing = database_lock();
    let db = arc::spec::protocol_db_path(&p.dir).map_err(|e| e.to_string())?;
    if !db.is_file() {
        return Ok(());
    }
    let transforms = FinetypeTransforms::vendored();
    let conn = duckdb::Connection::open(&db).map_err(|e| format!("{}: {e}", db.display()))?;
    let state: i64 = conn
        .query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = '_arcform_state'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("{}: {e}", db.display()))?;
    // A part of a combination is a source table that has no silver table; a combined dataset or a
    // part left out of the model keeps none either, so what an earlier model made of them goes.
    let mut candidates: Vec<String> = p.sources.tables.iter().map(|t| t.name.clone()).collect();
    for d in &model.datasets {
        let combined = d.meta.combination.is_some() || d.meta.combined_into.is_some();
        if combined && !candidates.contains(&d.name) {
            candidates.push(d.name.clone());
        }
    }
    // A dataset the model no longer has, such as a combination a later pass stopped making, is
    // known only by the silver step arc recorded for it.
    if state > 0 {
        let mut stmt = conn
            .prepare("SELECT step_name FROM _arcform_state WHERE step_name LIKE 'silver\\_%' ESCAPE '\\'")
            .map_err(|e| format!("{}: {e}", db.display()))?;
        let steps = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("{}: {e}", db.display()))?;
        for step in steps.flatten() {
            let name = step[SILVER_STEP.len()..].to_string();
            if !candidates.contains(&name) {
                candidates.push(name);
            }
        }
    }
    let unmade: Vec<String> = candidates
        .into_iter()
        .filter(|name| silver_sql(model, name, &transforms).is_empty())
        .collect();
    if unmade.is_empty() {
        return Ok(());
    }
    let mut sql = String::new();
    for name in &unmade {
        let _ = write!(
            sql,
            "DROP TABLE IF EXISTS {}; DROP TABLE IF EXISTS {};",
            quoted(name),
            quoted(&format!("{name}__failed"))
        );
        if state > 0 {
            let _ = write!(
                sql,
                "DELETE FROM _arcform_state WHERE step_name IN ({}, {});",
                literal(&format!("{SILVER_STEP}{name}")),
                literal(&format!("{FAILED_STEP}{name}"))
            );
        }
    }
    conn.execute_batch(&sql)
        .map_err(|e| format!("{}: {e}", db.display()))
}

/// Profile the raw tables with FineType, one [`SourceProfile`] per table in `p.profiles`.
///
/// Each table is copied to a Parquet snapshot first, with the database held only for the copy, as
/// the panels do, so the run's database is free for the next arc run. Without a FineType bundle
/// beside the binary the columns come back unlabelled and the profile still has their DuckDB
/// types.
///
/// # Errors
///
/// The reason when a table cannot be snapshotted or the engine cannot load the snapshots.
pub fn profile(p: &mut OwnProtocol) -> Result<(), String> {
    let snapshots = p.dir.join(PROFILE_DIR);
    std::fs::create_dir_all(&snapshots).map_err(|e| format!("{}: {e}", snapshots.display()))?;
    let db = arc::spec::protocol_db_path(&p.dir).map_err(|e| e.to_string())?;
    let conn = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch(&format!(
        "ATTACH {} AS __run (READ_ONLY)",
        literal(&db.to_string_lossy())
    ))
    .map_err(|e| format!("{}: {e}", db.display()))?;
    let mut spec = String::from("data:\n");
    let copied = (|| -> duckdb::Result<()> {
        for table in &p.sources.tables {
            let file = snapshots.join(format!("{}.parquet", table.name));
            conn.execute_batch(&format!(
                "COPY (SELECT * EXCLUDE ({}) FROM __run.main.{}) TO {} (FORMAT parquet)",
                LINEAGE.join(", "),
                quoted(&format!("raw_{}", table.name)),
                literal(&file.to_string_lossy())
            ))?;
            let _ = writeln!(
                spec,
                "  {}:\n    file: {}",
                quoted_yaml(&table.name),
                quoted_yaml(&file.to_string_lossy())
            );
        }
        Ok(())
    })();
    let _ = conn.execute_batch("DETACH __run");
    copied.map_err(|e| format!("could not snapshot the raw tables: {e}"))?;

    let parsed = parse_spec(&spec, Format::Yaml).map_err(|e| format!("profile spec: {e}"))?;
    let analysis = analyse_spec(&parsed.spec).map_err(|e| format!("profile spec: {e}"))?;
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
        .map_err(|e| format!("could not profile the raw tables: {e}"))?;
    p.profiles = load.session.profile_sources();
    Ok(())
}

/// Write `packages/<source>/datapackage.json` for each source file, one resource per table of it,
/// and answer the paths written.
///
/// # Errors
///
/// The reason when [`profile`] has not covered a table, a table's profile failed, or a file
/// cannot be written.
pub fn write_packages(p: &OwnProtocol) -> Result<Vec<PathBuf>, String> {
    let mut by_file: Vec<(&Path, Vec<&SourceTable>)> = Vec::new();
    for table in &p.sources.tables {
        match by_file.iter_mut().find(|(f, _)| *f == table.file) {
            Some((_, tables)) => tables.push(table),
            None => by_file.push((&table.file, vec![table])),
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    let mut written = Vec::new();
    let mut raw = RawTables::default();
    for (file, tables) in by_file {
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // Package folders are compared lower-cased, as `Sales.csv` and `sales.parquet` are one
        // folder on a case-insensitive file system. A stem already taken gets its extension
        // (`a_xlsx`), then a number.
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mut folder = stem.clone();
        if taken.contains(&folder) {
            folder = format!("{stem}_{ext}");
        }
        let mut n = 2;
        while taken.contains(&folder) {
            folder = format!("{stem}_{ext}_{n}");
            n += 1;
        }
        taken.insert(folder.clone());
        let package_dir = p.dir.join("packages").join(&folder);
        let mut resources = Vec::new();
        for table in tables {
            resources.push(resource(p, table, &package_dir)?);
        }
        let package = json!({
            "$schema": "https://datapackage.org/profiles/2.0/datapackage.json",
            "name": folder.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.', "-"),
            "resources": resources,
        });
        std::fs::create_dir_all(&package_dir)
            .map_err(|e| format!("{}: {e}", package_dir.display()))?;
        let path = package_dir.join("datapackage.json");
        // What Brightfield recorded on these fields before (`x-brightfield`) is not the profile's
        // to rewrite, so it is carried into the package this writes; how each column was written
        // is read again from the values.
        let mut package = crate::column_record::carry_over(package, &path);
        record_written(p, &mut raw, &mut package, &path);
        crate::column_record::write_package(&path, &package)?;
        written.push(path);
    }
    Ok(written)
}

/// A connection to the Protocol's database, attached read-only as `__run` once something asks for
/// it, for [`record_written`]'s aggregates over the raw tables.
#[derive(Default)]
struct RawTables(Option<duckdb::Connection>);

impl RawTables {
    /// One row of `terms`, aggregates over the raw table of `table`, each answer as text.
    fn aggregate(
        &mut self,
        p: &OwnProtocol,
        table: &str,
        terms: &[String],
    ) -> Result<Vec<Option<String>>, String> {
        if self.0.is_none() {
            let db = arc::spec::protocol_db_path(&p.dir).map_err(|e| e.to_string())?;
            let conn = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
            conn.execute_batch(&format!(
                "ATTACH {} AS __run (READ_ONLY)",
                literal(&db.to_string_lossy())
            ))
            .map_err(|e| format!("{}: {e}", db.display()))?;
            self.0 = Some(conn);
        }
        let conn = self.0.as_ref().expect("connected above");
        let sql = format!(
            "SELECT {} FROM __run.main.{}",
            terms.join(", "),
            quoted(&format!("raw_{table}"))
        );
        conn.query_row(&sql, [], |row| {
            (0..terms.len())
                .map(|i| row.get::<_, Option<String>>(i))
                .collect()
        })
        .map_err(|e| format!("raw_{table}: {e}"))
    }
}

impl Drop for RawTables {
    fn drop(&mut self) {
        if let Some(conn) = &self.0 {
            let _ = conn.execute_batch("DETACH __run");
        }
    }
}

/// Put on each currency field of `package` that the silver cast casts how its source wrote it
/// (`x-brightfield.written`): the symbol its values share and how its negatives are written, read
/// with one aggregate per table from the raw table the cast reads, whose text still holds them.
///
/// The cast is the one [`silver_sql`] makes from the field's label as the package now records it,
/// an analyst's nominated label included. Only a column the raw table holds as text is read: a
/// number stored as a number has no symbol, and its text is DuckDB's, not the source's. A table
/// whose aggregate fails is reported and its fields keep the `written` the package at `existing`
/// recorded: the open does not depend on it.
fn record_written(p: &OwnProtocol, raw: &mut RawTables, package: &mut Value, existing: &Path) {
    let transforms = FinetypeTransforms::vendored();
    for res in package["resources"].as_array_mut().into_iter().flatten() {
        let Some(table) = res["name"].as_str().map(str::to_string) else {
            continue;
        };
        let text = text_columns(p, &table);
        let Some(fields) = res["schema"]["fields"].as_array_mut() else {
            continue;
        };
        let cast: Vec<usize> = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let (Some(name), Some(label)) =
                    (f["name"].as_str(), f["x-finetype-label"].as_str())
                else {
                    return false;
                };
                text.contains(name)
                    && label.starts_with("finance.currency.")
                    && cast_expr(name, Some(label), &transforms).is_some()
            })
            .map(|(at, _)| at)
            .collect();
        if cast.is_empty() {
            continue;
        }
        let terms: Vec<String> = cast
            .iter()
            .flat_map(|&at| {
                crate::flat_cast::written_terms(&quoted(fields[at]["name"].as_str().unwrap_or("")))
            })
            .collect();
        match raw.aggregate(p, &table, &terms) {
            Ok(answers) => {
                let mut answers = answers.into_iter();
                for &at in &cast {
                    let how = crate::flat_cast::written_from(
                        answers.next().flatten(),
                        answers.next().flatten(),
                    );
                    crate::column_record::put_written(&mut fields[at], &how);
                }
            }
            Err(e) => {
                eprintln!(
                    "warning: {table}: how its currency columns were written was not read, so \
                     what was recorded before is kept: {e}"
                );
                let names: Vec<String> = cast
                    .iter()
                    .filter_map(|&at| fields[at]["name"].as_str().map(str::to_string))
                    .collect();
                crate::column_record::keep_old_written(res, existing, &names);
            }
        }
    }
}

/// The columns of `table` its profile found stored as text (VARCHAR).
fn text_columns(p: &OwnProtocol, table: &str) -> HashSet<String> {
    let Some(ProfileOutcome::Profiled { columns, .. }) = p
        .profiles
        .iter()
        .find(|s| s.name == table)
        .map(|s| &s.outcome)
    else {
        return HashSet::new();
    };
    columns
        .iter()
        .filter(|c| c.type_name.trim().eq_ignore_ascii_case("VARCHAR"))
        .map(|c| c.name.clone())
        .collect()
}

/// One Frictionless resource for `table`, its fields typed from the profile.
fn resource(p: &OwnProtocol, table: &SourceTable, package_dir: &Path) -> Result<Value, String> {
    let profile = p
        .profiles
        .iter()
        .find(|s| s.name == table.name)
        .ok_or_else(|| format!("{}: not profiled yet", table.name))?;
    let columns = match &profile.outcome {
        ProfileOutcome::Profiled { columns, .. } => columns,
        ProfileOutcome::Failed(why) => {
            return Err(format!("{}: profiling failed: {why}", table.name))
        }
        ProfileOutcome::Unsupported => {
            return Err(format!("{}: profiling is not supported for it", table.name))
        }
    };
    let fields: Vec<Value> = columns
        .iter()
        .filter(|c| !LINEAGE.contains(&c.name.as_str()))
        .map(field)
        .collect();
    let mut resource = json!({
        "name": table.name,
        "path": relative_to(&table.file, package_dir)?,
        "schema": { "fields": fields },
    });
    if let Some(part) = &table.part {
        resource["x-part"] = Value::String(part.clone());
    }
    Ok(resource)
}

/// One field: its name, the Frictionless type, and FineType's label when it gave one.
fn field(column: &ColumnProfile) -> Value {
    // A label whose values the column's own data contradicts is not trusted for the type or kept.
    let label = column
        .semantic
        .is_usable()
        .then(|| column.semantic.label())
        .flatten();
    let mut field = json!({
        "name": column.name,
        "type": frictionless_type(label, &column.type_name),
    });
    if let Some(label) = label {
        field["x-finetype-label"] = Value::String(label.to_string());
    }
    field
}

/// The Frictionless field type for a column.
///
/// A column DuckDB already typed keeps the type that implies. A VARCHAR column, which is every
/// column of a workbook and any CSV column DuckDB could not read as something narrower, takes
/// its type from its FineType label when the label is one whose values Frictionless reads as
/// that type unaided, so `datetime.date.iso` is a `date`. No label, or any other, is a `string`.
fn frictionless_type(label: Option<&str>, duckdb_type: &str) -> &'static str {
    let upper = duckdb_type.trim().to_uppercase();
    if upper.ends_with(']') || upper.starts_with("LIST") {
        return "array";
    }
    if upper.starts_with("STRUCT") || upper.starts_with("MAP") {
        return "object";
    }
    let by_storage = match upper.split('(').next().unwrap_or("").trim() {
        "BOOLEAN" => Some("boolean"),
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT"
        | "UINTEGER" | "UBIGINT" | "UHUGEINT" => Some("integer"),
        "FLOAT" | "DOUBLE" | "DECIMAL" => Some("number"),
        "DATE" => Some("date"),
        "TIME" | "TIME WITH TIME ZONE" | "TIMETZ" => Some("time"),
        "TIMESTAMP"
        | "TIMESTAMP_NS"
        | "TIMESTAMP_MS"
        | "TIMESTAMP_S"
        | "TIMESTAMP WITH TIME ZONE"
        | "TIMESTAMPTZ" => Some("datetime"),
        _ => None,
    };
    if let Some(t) = by_storage {
        return t;
    }
    // FineType labels whose pattern admits nothing outside what Frictionless reads by default for
    // the type, from the `pattern` of each label in the taxonomy catalogue of FineType v0.6.67
    // (`taxonomy-schemas.json` in the bundle). A label not listed is a string, as
    // `only_default_format_labels_are_typed` shows: `datetime.time.hms_24h` admits a one-digit
    // hour and fractional seconds, `datetime.timestamp.iso_8601` admits fractional seconds, and
    // the boolean labels (`binary`, `initials`, `terms`) all carry words besides true and false.
    match label {
        Some("datetime.date.iso") => "date",
        Some("representation.numeric.integer_number") => "integer",
        Some("representation.numeric.decimal_number") => "number",
        _ => "string",
    }
}

/// The path from `base` (a directory) to `target`, both absolute.
pub(crate) fn relative_to(target_path: &Path, base: &Path) -> Result<String, String> {
    let target: Vec<Component> = target_path.components().collect();
    let base: Vec<Component> = base.components().collect();
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return Err(format!(
            "{}: shares no root with the Protocol folder, so a package cannot point at it",
            target_path.display()
        ));
    }
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for c in &target[common..] {
        out.push(c);
    }
    Ok(out.to_string_lossy().replace('\\', "/"))
}

/// The own-files Protocol folder `path` names, when it names one: the folder itself or its
/// `arcform.yaml`, with a model file beside the manifest.
#[must_use]
pub fn own_protocol_dir(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() {
        path
    } else if path.file_name().is_some_and(|n| n == "arcform.yaml") {
        path.parent()?
    } else {
        return None;
    };
    (dir.join("arcform.yaml").is_file() && dir.join(MODEL_FILE).is_file())
        .then(|| std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()))
}

/// The own-files Protocol of one source `path` names, when it names one: the folder itself or its
/// `arcform.yaml`, holding the record of the tables its open found, that record naming one table,
/// and no model file, since one table has nothing to relate.
#[must_use]
pub fn one_source_protocol_dir(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() {
        path
    } else if path.file_name().is_some_and(|n| n == "arcform.yaml") {
        path.parent()?
    } else {
        return None;
    };
    if !dir.join("arcform.yaml").is_file() || dir.join(MODEL_FILE).exists() {
        return None;
    }
    let sources = read_sources(dir).ok()?;
    (sources.tables.len() == 1)
        .then(|| std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()))
}

/// The names of the steps the own-files Protocol of one source in `dir` was made with, its load
/// and its silver and failed steps, when `dir` holds one: every other step of its manifest was
/// recorded after, from the grid or from typed SQL. Known by exact name, as `load_steps` knows
/// a load, so a step recorded on a table whose name starts like one of these is not taken for it.
#[must_use]
pub fn one_source_steps(dir: &Path) -> Option<(String, HashSet<String>)> {
    one_source_protocol_dir(dir)?;
    let sources = read_sources(dir).ok()?;
    let [table] = sources.tables.as_slice() else {
        return None;
    };
    let name = &table.name;
    let steps = [
        load_step(name),
        format!("{SILVER_STEP}{name}"),
        format!("{FAILED_STEP}{name}"),
    ];
    Some((name.clone(), steps.into_iter().collect()))
}

/// The names of the steps the own-files Protocol of several tables in `dir` was made with, each
/// table's load and its silver and failed steps, when `dir` holds one and `table` is one of its
/// tables: what a step recorded from that table's grid comes after.
#[must_use]
pub fn folder_table_steps(dir: &Path, table: &str) -> Option<HashSet<String>> {
    own_protocol_dir(dir)?;
    let sources = read_sources(dir).ok()?;
    if !sources
        .tables
        .iter()
        .any(|t| t.name.eq_ignore_ascii_case(table))
    {
        return None;
    }
    Some(
        sources
            .tables
            .iter()
            .flat_map(|t| {
                [
                    load_step(&t.name),
                    format!("{SILVER_STEP}{}", t.name),
                    format!("{FAILED_STEP}{}", t.name),
                ]
            })
            .collect(),
    )
}

/// Open the own-files Protocol of one source in `dir` again: the table its open recorded, the
/// Protocol run, which reads the source file afresh where it changed, and the raw table profiled.
/// No folder is made.
///
/// # Errors
///
/// The reason when the folder holds no record of its tables, the run fails, or the raw table
/// cannot be profiled.
pub fn reopen_one_source(dir: &Path, runner: &Runner) -> Result<OwnProtocol, String> {
    let mut p = OwnProtocol {
        dir: dir.to_path_buf(),
        sources: read_sources(dir)?,
        profiles: Vec::new(),
    };
    run_loads(&p, runner)?;
    profile(&mut p)?;
    Ok(p)
}

/// The own-files Protocol of one source in `dir` with its raw table read afresh from the source
/// file and profiled, for its silver steps to be written again from what the file holds now: a
/// column added, renamed or retyped since the Protocol was made.
///
/// Only the load steps are run. The steps after them were written for the file's earlier columns,
/// and a silver step naming a column the file no longer has would fail the run before the new
/// columns could be profiled. arc runs a whole manifest, so the loads are run from a manifest of
/// their own in a temporary folder, naming the Protocol's database by path (arc's `db:` key) so
/// they write the tables the Protocol reads. The Protocol's own `arcform.yaml` is not touched.
///
/// # Errors
///
/// The reason when the folder holds no record of its tables, its manifest cannot be read, the
/// temporary manifest cannot be written, the load fails, or the raw table cannot be profiled.
pub fn reload_one_source(dir: &Path, runner: &Runner) -> Result<OwnProtocol, String> {
    let mut p = OwnProtocol {
        dir: dir.to_path_buf(),
        sources: read_sources(dir)?,
        profiles: Vec::new(),
    };
    let manifest_path = dir.join("arcform.yaml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let mut manifest: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let loads = load_steps(&p);
    let mut files = Vec::new();
    if let Some(steps) = manifest
        .get_mut("steps")
        .and_then(serde_yaml::Value::as_sequence_mut)
    {
        steps.retain(|s| is_load(s, &loads));
        files.extend(
            steps
                .iter()
                .filter_map(|s| s.get("sql").and_then(serde_yaml::Value::as_str))
                .map(str::to_owned),
        );
    }
    let db = arc::spec::protocol_db_path(dir).map_err(|e| e.to_string())?;
    let db = std::path::absolute(&db).unwrap_or(db);
    if let Some(map) = manifest.as_mapping_mut() {
        map.insert("db".into(), db.to_string_lossy().into_owned().into());
    }
    let scratch = tempfile::tempdir().map_err(|e| format!("a temporary folder: {e}"))?;
    for file in &files {
        let to = scratch.path().join(file);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(dir.join(file), &to).map_err(|e| format!("{file}: {e}"))?;
    }
    let loads_only = serde_yaml::to_string(&manifest).map_err(|e| e.to_string())?;
    let at = scratch.path().join("arcform.yaml");
    std::fs::write(&at, loads_only).map_err(|e| format!("{}: {e}", at.display()))?;
    run_loads(
        &OwnProtocol {
            dir: scratch.path().to_path_buf(),
            sources: p.sources.clone(),
            profiles: Vec::new(),
        },
        runner,
    )?;
    forget_after_loads(&p)?;
    profile(&mut p)?;
    Ok(p)
}

/// Forget arc's record of each step other than a load succeeding, so the next run runs them over
/// the raw tables a loads-only run has just made. Without this, a silver step whose SQL had not
/// changed sat out the next run as unchanged, though the loads-only run had changed its input
/// (`reloading_the_data_rereads_the_analysts_file` in `tests/open_makes_a_protocol.rs`).
fn forget_after_loads(p: &OwnProtocol) -> Result<(), String> {
    let _writing = database_lock();
    let db = arc::spec::protocol_db_path(&p.dir).map_err(|e| e.to_string())?;
    if !db.is_file() {
        return Ok(());
    }
    let conn = duckdb::Connection::open(&db).map_err(|e| format!("{}: {e}", db.display()))?;
    let state: i64 = conn
        .query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = '_arcform_state'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("{}: {e}", db.display()))?;
    if state == 0 {
        return Ok(());
    }
    let mut loads: Vec<String> = load_steps(p).iter().map(|n| literal(n)).collect();
    loads.sort();
    conn.execute_batch(&format!(
        "DELETE FROM _arcform_state WHERE step_name NOT IN ({})",
        loads.join(", ")
    ))
    .map_err(|e| format!("{}: {e}", db.display()))
}

/// The own-files Protocol folders in the Protocols folder and in the older
/// `<datasets>/protocols/own/` (where earlier builds made them), the most recently changed model
/// first.
#[must_use]
pub fn listed() -> Vec<PathBuf> {
    let old = starts::datasets_dir().join("protocols").join(OWN_DIR);
    let mut entries: Vec<PathBuf> = Vec::new();
    for parent in [protocols_dir::root(), old] {
        if let Ok(read) = std::fs::read_dir(&parent) {
            entries.extend(read.flatten().map(|e| e.path()));
        }
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .iter()
        .filter_map(|e| own_protocol_dir(e))
        .map(|dir| {
            let changed = std::fs::metadata(dir.join(MODEL_FILE))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (changed, dir)
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|(_, dir)| dir).collect()
}

/// Open the own-files Protocol in `dir` again, as it was left: the tables its open recorded,
/// the silver steps written for the model its folder holds, the Protocol run, and the raw tables
/// profiled. The run makes again only what has changed or is missing, since arc skips a step
/// whose SQL and inputs are as they were after its last success.
///
/// # Errors
///
/// The reason when the folder holds no record of its tables or no model Brightfield reads, a
/// step cannot be written, the run fails, or the raw tables cannot be profiled.
pub fn reopen(dir: &Path, runner: &Runner) -> Result<OwnProtocol, String> {
    let sources = read_sources(dir)?;
    let model_path = dir.join(MODEL_FILE);
    let text = std::fs::read_to_string(&model_path)
        .map_err(|e| format!("{}: {e}", model_path.display()))?;
    let model =
        brightfield_ossie::read(&text).map_err(|e| format!("{}: {e}", model_path.display()))?;
    let mut p = OwnProtocol {
        dir: dir.to_path_buf(),
        sources,
        profiles: Vec::new(),
    };
    write_silver(&p, &model)?;
    drop_unmade_silver(&p, &model)?;
    run_loads(&p, runner)?;
    profile(&mut p)?;
    Ok(p)
}

/// Copy the silver table `name` to a Parquet file in the Protocol folder, its lineage columns
/// left out, and answer the file: the rows of a single sheet or database table, for the grid.
///
/// # Errors
///
/// The reason when the Protocol has not been run or the copy fails.
pub fn grid_copy(p: &OwnProtocol, name: &str) -> Result<PathBuf, String> {
    crate::model_inspector::snapshot(
        &p.dir,
        name,
        &format!(
            "SELECT * EXCLUDE ({}) FROM {}",
            LINEAGE.join(", "),
            quoted(name)
        ),
    )
}

/// Record `set` in the Protocol folder `dir`, for [`reopen`], with the folder's canonical path as
/// `dir`, which is how an open notices the folder was moved since ([`crate::rename::moved_since`]).
fn write_sources(dir: &Path, set: &SourceSet) -> Result<(), String> {
    let tables: Vec<Value> = set
        .tables
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "file": t.file.to_string_lossy(),
                "part": t.part,
                "schema": t.schema,
                "reader": reader_name(t.reader),
            })
        })
        .collect();
    let skipped: Vec<Value> = set
        .skipped
        .iter()
        .map(|(path, reason)| json!({ "path": path.to_string_lossy(), "reason": reason }))
        .collect();
    let place = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let text = serde_json::to_string_pretty(
        &json!({ "dir": place, "tables": tables, "skipped": skipped }),
    )
    .map_err(|e| e.to_string())?;
    let path = dir.join(SOURCES_FILE);
    std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// The tables and skipped files [`write_sources`] recorded in `dir`.
fn read_sources(dir: &Path) -> Result<SourceSet, String> {
    let path = dir.join(SOURCES_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e}; this folder records no tables, so open the files it was made from instead",
            path.display()
        )
    })?;
    let bad = |what: &str| format!("{}: {what}", path.display());
    let value: Value = serde_json::from_str(&text).map_err(|e| bad(&e.to_string()))?;
    let text_of = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let mut set = SourceSet::default();
    for t in value["tables"].as_array().ok_or_else(|| bad("no tables"))? {
        let reader = text_of(t, "reader")
            .as_deref()
            .and_then(reader_named)
            .ok_or_else(|| bad("a table with no reader Brightfield knows"))?;
        set.tables.push(SourceTable {
            name: text_of(t, "name").ok_or_else(|| bad("a table with no name"))?,
            file: PathBuf::from(text_of(t, "file").ok_or_else(|| bad("a table with no file"))?),
            part: text_of(t, "part"),
            schema: text_of(t, "schema"),
            reader,
        });
    }
    for s in value["skipped"].as_array().into_iter().flatten() {
        if let (Some(path), Some(reason)) = (text_of(s, "path"), text_of(s, "reason")) {
            set.skipped.push((PathBuf::from(path), reason));
        }
    }
    Ok(set)
}

fn reader_name(reader: Reader) -> &'static str {
    match reader {
        Reader::Csv => "csv",
        Reader::Tsv => "tsv",
        Reader::Parquet => "parquet",
        Reader::XlsxSheet => "xlsx_sheet",
        Reader::SqliteTable => "sqlite_table",
        Reader::DuckdbTable => "duckdb_table",
    }
}

fn reader_named(name: &str) -> Option<Reader> {
    [
        Reader::Csv,
        Reader::Tsv,
        Reader::Parquet,
        Reader::XlsxSheet,
        Reader::SqliteTable,
        Reader::DuckdbTable,
    ]
    .into_iter()
    .find(|r| reader_name(*r) == name)
}

/// The opened path's stem, kept to characters a folder name and an arc Protocol name are safe
/// with.
fn slug_of(opened: &Path) -> String {
    let stem = opened
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if slug.is_empty() {
        "files".to_string()
    } else {
        slug
    }
}

fn skipped_note(set: &SourceSet) -> String {
    if set.skipped.is_empty() {
        return String::new();
    }
    let list: Vec<String> = set
        .skipped
        .iter()
        .map(|(f, why)| format!("{} ({why})", f.display()))
        .collect();
    format!("; skipped: {}", list.join(", "))
}

fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn quoted_yaml(s: &str) -> String {
    literal(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_types_win_and_labels_type_varchar() {
        assert_eq!(frictionless_type(None, "BIGINT"), "integer");
        assert_eq!(frictionless_type(None, "DECIMAL(18,3)"), "number");
        assert_eq!(frictionless_type(None, "TIMESTAMP"), "datetime");
        assert_eq!(frictionless_type(None, "VARCHAR"), "string");
        assert_eq!(
            frictionless_type(Some("identity.person.email"), "VARCHAR"),
            "string"
        );
    }

    #[test]
    fn relative_paths_climb_out_of_the_package() {
        assert_eq!(
            relative_to(Path::new("/a/b/c.csv"), Path::new("/a/p/packages/c")).unwrap(),
            "../../../b/c.csv"
        );
    }

    #[test]
    fn nested_and_zoned_storage_types() {
        assert_eq!(frictionless_type(None, "INTEGER[]"), "array");
        assert_eq!(frictionless_type(None, "INTEGER[3]"), "array");
        assert_eq!(frictionless_type(None, "VARCHAR[][]"), "array");
        assert_eq!(frictionless_type(None, "STRUCT(a INTEGER)"), "object");
        assert_eq!(frictionless_type(None, "MAP(VARCHAR, INTEGER)"), "object");
        for t in [
            "TIMESTAMP_NS",
            "TIMESTAMP_MS",
            "TIMESTAMP_S",
            "TIMESTAMP WITH TIME ZONE",
            "TIMESTAMPTZ",
        ] {
            assert_eq!(frictionless_type(None, t), "datetime", "{t}");
        }
        assert_eq!(frictionless_type(None, "TIME WITH TIME ZONE"), "time");
    }

    #[test]
    fn only_default_format_labels_are_typed() {
        let typed = |label| frictionless_type(Some(label), "VARCHAR");
        assert_eq!(typed("datetime.date.iso"), "date");
        assert_eq!(typed("datetime.date.iso_week"), "string");
        assert_eq!(typed("datetime.date.mdy_slash"), "string");
        assert_eq!(typed("datetime.timestamp.iso_8601"), "string");
        assert_eq!(typed("datetime.timestamp.iso_8601_offset"), "string");
        assert_eq!(typed("datetime.timestamp.iso_8601_compact"), "string");
        assert_eq!(typed("datetime.timestamp.iso_space_zulu"), "string");
        assert_eq!(typed("datetime.time.hms_24h"), "string");
        assert_eq!(typed("datetime.time.hms_12h"), "string");
        assert_eq!(typed("representation.numeric.integer_number"), "integer");
        assert_eq!(typed("representation.numeric.decimal_number"), "number");
        assert_eq!(
            typed("representation.numeric.decimal_number_comma"),
            "string"
        );
        assert_eq!(typed("representation.numeric.percentage"), "string");
        assert_eq!(typed("representation.numeric.si_number"), "string");
        assert_eq!(typed("representation.boolean.terms"), "string");
        assert_eq!(typed("representation.boolean.binary"), "string");
    }

    #[test]
    fn an_unusable_label_is_neither_trusted_nor_written() {
        use brightfield_engine::SemanticType;
        let mut column = ColumnProfile {
            name: "d".into(),
            type_name: "VARCHAR".into(),
            non_null: 1,
            nulls: 0,
            distinct: 1,
            min: None,
            max: None,
            semantic: SemanticType::Unusable {
                label: "datetime.date.iso".into(),
                confidence: 0.9,
                checked: 10,
                failed: 9,
                first_failure: None,
            },
            moments: None,
        };
        let f = field(&column);
        assert_eq!(f["type"], "string");
        assert!(f.get("x-finetype-label").is_none());
        column.semantic = SemanticType::Labelled {
            label: "datetime.date.iso".into(),
            confidence: 0.9,
            check: brightfield_engine::ValueCheck::Checked {
                checked: 10,
                failed: 0,
            },
        };
        let f = field(&column);
        assert_eq!(f["type"], "date");
        assert_eq!(f["x-finetype-label"], "datetime.date.iso");
    }
}
