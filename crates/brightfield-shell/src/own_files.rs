//! The Protocol folder an analyst's own files are written as: one arc load step per table, the
//! raw tables those steps make, a FineType profile of them, and one Frictionless data package per
//! source file.
//!
//! The load steps read the files where they are, by absolute path, so nothing the analyst gave is
//! copied or moved. The Protocol is made under [`crate::starts::datasets_dir`], beside the
//! Protocols that ship with the app, in a folder of its own per open.
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
use brightfield_protocol::silver::{failed_sql, silver_sql, FinetypeTransforms};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use serde_json::{json, Value};

use crate::discovery::MODEL_FILE;
use crate::run::{Finished, Run, Runner};
use crate::starts;

/// The folder under `<datasets>/protocols/` that holds the Protocols made from an analyst's own
/// files.
pub const OWN_DIR: &str = "own";

/// Where the raw tables' Parquet snapshots are kept inside the Protocol folder.
const PROFILE_DIR: &str = ".profile";

/// The file in the Protocol folder recording the tables the open found and the files it passed
/// over, so the folder can be opened again ([`reopen`]) without finding them afresh.
const SOURCES_FILE: &str = ".sources.json";

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
    let sources = sources::discover(opened)?;
    if sources.tables.is_empty() {
        return Err(format!(
            "{}: no tables Brightfield can read{}",
            opened.display(),
            skipped_note(&sources)
        ));
    }
    let slug = slug_of(opened);
    let parent = starts::datasets_dir().join("protocols").join(OWN_DIR);
    std::fs::create_dir_all(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    // `create_dir` is the claim on a number: it fails if the folder exists, so two opens at once
    // cannot both take it.
    let (dir, name) = (1..)
        .find_map(|n| {
            let name = format!("{slug}-{n}");
            let dir = parent.join(&name);
            match std::fs::create_dir(&dir) {
                Ok(()) => Some(Ok((dir, name))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(format!("{}: {e}", dir.display()))),
            }
        })
        .expect("an unbounded range")?;

    let models = dir.join("models");
    std::fs::create_dir(&models).map_err(|e| format!("{}: {e}", models.display()))?;
    let mut manifest = format!("name: {name}\nengine: duckdb\nsteps:\n");
    for table in &sources.tables {
        let step = format!("load_{}", table.name);
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

/// Run the Protocol's steps with `runner`, which makes the raw tables, and wait for it.
///
/// # Errors
///
/// What `arc` printed, when the run did not succeed.
pub fn run_loads(p: &OwnProtocol, runner: &Runner) -> Result<(), String> {
    let finished: Finished = Run::begin(runner, &p.dir, || {}).wait();
    if finished.succeeded {
        Ok(())
    } else {
        Err(finished.log)
    }
}

/// Write the silver steps for `model`: for each dataset that has a silver table, a step that casts
/// the raw table through FineType's transformation contracts and one that keeps the values that did
/// not cast, as `models/02_silver_<name>.sql` and `models/02_failed_<name>.sql`.
///
/// Every call rewrites them: the new files and manifest are written first and the earlier files
/// the manifest no longer names are removed after, so a failed write leaves a usable folder. A
/// dataset that has since been excluded or folded into another loses its silver table's steps. The
/// load steps are left as they are. Run the Protocol afterwards to make the tables.
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

    let mut written: HashSet<String> = HashSet::new();
    for table in &p.sources.tables {
        let silver = silver_sql(model, &table.name, &transforms);
        if silver.is_empty() {
            continue;
        }
        let failed = failed_sql(model, &table.name, &transforms);
        for (step, file, sql, produces) in [
            (
                format!("{SILVER_STEP}{}", table.name),
                format!("models/{SILVER_FILE}{}.sql", table.name),
                silver,
                table.name.clone(),
            ),
            (
                format!("{FAILED_STEP}{}", table.name),
                format!("models/{FAILED_FILE}{}.sql", table.name),
                failed,
                format!("{}__failed", table.name),
            ),
        ] {
            std::fs::write(p.dir.join(&file), format!("{sql};\n"))
                .map_err(|e| format!("{file}: {e}"))?;
            written.insert(file.clone());
            steps.push(
                serde_yaml::from_str(&format!(
                    "name: {}\nsql: {}\nproduces:\n  - {}\n",
                    quoted_yaml(&step),
                    quoted_yaml(&file),
                    quoted_yaml(&produces)
                ))
                .map_err(|e| e.to_string())?,
            );
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
    let db = arc::spec::protocol_db_path(&p.dir).map_err(|e| e.to_string())?;
    if !db.is_file() {
        return Ok(());
    }
    let transforms = FinetypeTransforms::vendored();
    let unmade: Vec<&str> = p
        .sources
        .tables
        .iter()
        .filter(|t| silver_sql(model, &t.name, &transforms).is_empty())
        .map(|t| t.name.as_str())
        .collect();
    if unmade.is_empty() {
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
    let mut sql = String::new();
    for name in unmade {
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
        let text = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
        std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
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
    // the type, from the `pattern` of each label in the taxonomy catalogue of FineType v0.6.59
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
fn relative_to(target_path: &Path, base: &Path) -> Result<String, String> {
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

/// The own-files Protocol folders under `<datasets>/protocols/own/`, the most recently changed
/// model first.
#[must_use]
pub fn listed() -> Vec<PathBuf> {
    let parent = starts::datasets_dir().join("protocols").join(OWN_DIR);
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| own_protocol_dir(&e.path()))
        .map(|dir| {
            let changed = std::fs::metadata(dir.join(MODEL_FILE))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (changed, dir)
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
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

/// Record `set` in the Protocol folder `dir`, for [`reopen`].
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
    let text = serde_json::to_string_pretty(&json!({ "tables": tables, "skipped": skipped }))
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
