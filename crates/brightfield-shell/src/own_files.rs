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

use brightfield_engine::sources::{self, SourceSet, SourceTable};
use brightfield_engine::{ColumnProfile, Engine, LoadOptions, ProfileOutcome, SourceProfile};
use brightfield_ossie::Model;
use brightfield_protocol::silver::{failed_sql, silver_sql, FinetypeTransforms};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use serde_json::{json, Value};

use crate::run::{Finished, Run, Runner};
use crate::starts;

/// The folder under `<datasets>/protocols/` that holds the Protocols made from an analyst's own
/// files.
pub const OWN_DIR: &str = "own";

/// Where the raw tables' Parquet snapshots are kept inside the Protocol folder.
const PROFILE_DIR: &str = ".profile";

/// The lineage columns every raw table carries; they describe where a row came from, not what it
/// holds, so neither the profile nor the data packages list them.
const LINEAGE: [&str; 3] = ["__source_file", "__source_part", "__source_row"];

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
/// Every call rewrites them: steps and files from an earlier call are removed first, so a dataset
/// that has since been excluded or folded into another loses its silver table's steps. The load
/// steps are left as they are. Run the Protocol afterwards to make the tables.
///
/// # Errors
///
/// The reason when the manifest cannot be read or written, or a step file cannot be written.
pub fn write_silver(p: &OwnProtocol, model: &Model) -> Result<(), String> {
    let transforms = FinetypeTransforms::vendored();
    let models = p.dir.join("models");
    let entries = std::fs::read_dir(&models).map_err(|e| format!("{}: {e}", models.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(SILVER_FILE) || name.starts_with(FAILED_FILE) {
            std::fs::remove_file(entry.path())
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        }
    }

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
    std::fs::write(&manifest_path, out).map_err(|e| format!("{}: {e}", manifest_path.display()))
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
