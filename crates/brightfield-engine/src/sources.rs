//! Finding the tables in what an analyst opened (a folder, a workbook, a database file) and the
//! SQL that loads each one with row lineage.
//!
//! Discovery asks DuckDB what is readable: each candidate file is probed through the reader that
//! will later load it, a database's tables come from `duckdb_tables()` after a read-only `ATTACH`,
//! and a workbook's sheets are probed with `read_xlsx`. The SQL [`load_sql`] returns is
//! self-contained, because a Protocol step runs it in a DuckDB of its own.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use duckdb::Connection;

/// What a file holds: each table's schema (unset for the main schema), its sheet or table name
/// (unset for a flat file) and the reader that loads it.
type Parts = Vec<(Option<String>, Option<String>, Reader)>;

/// How a table's rows are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Csv,
    Tsv,
    Parquet,
    XlsxSheet,
    SqliteTable,
    DuckdbTable,
}

/// One table found in what was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTable {
    /// The table's name, without the `raw_` prefix its load step gives it.
    pub name: String,
    /// The file the rows come from (absolute).
    pub file: PathBuf,
    /// The sheet or database table inside `file`; `None` for a flat file.
    pub part: Option<String>,
    /// The schema holding a DuckDB table when it is not `main`; `None` otherwise. Kept apart from
    /// `part` because a schema or table name may itself contain a `.`.
    pub schema: Option<String>,
    pub reader: Reader,
}

/// The tables found, and what was passed over with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceSet {
    pub tables: Vec<SourceTable>,
    pub skipped: Vec<(PathBuf, String)>,
}

/// The tables in `path`: a folder's top level (no recursion), or a single workbook or database
/// file. A file that cannot be opened is skipped with the reason, never fatal.
pub fn discover(path: &Path) -> Result<SourceSet, String> {
    let path = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let files = if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        entries.sort();
        entries
    } else if path.is_file() {
        vec![path]
    } else {
        return Err(format!("{}: no such file or folder", path.display()));
    };

    let mut probe = Probe::new();
    let mut set = SourceSet::default();
    let mut found: Vec<(PathBuf, Parts)> = Vec::new();
    for file in files {
        if file.is_dir() {
            set.skipped
                .push((file, "folder (subfolders are not read)".into()));
            continue;
        }
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let parts = match ext.as_str() {
            "csv" => probe.flat(&file, Reader::Csv),
            "tsv" => probe.flat(&file, Reader::Tsv),
            "parquet" => probe.flat(&file, Reader::Parquet),
            "xlsx" => probe.workbook(&file, &mut set.skipped),
            "duckdb" | "db" | "sqlite" => probe.database(&file),
            _ => Err("not a kind of file Brightfield reads".to_string()),
        };
        match parts {
            Ok(parts) if parts.is_empty() => set.skipped.push((file, "no tables".into())),
            Ok(parts) => found.push((file, parts)),
            Err(why) => set.skipped.push((file, why)),
        }
    }

    let mut taken = HashSet::new();
    for (file, parts) in found {
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (schema, part, reader) in parts {
            let base = match (&schema, &part) {
                (Some(schema), Some(part)) => format!("{stem}_{schema}_{part}"),
                (None, Some(part)) => format!("{stem}_{part}"),
                _ => stem.clone(),
            };
            set.tables.push(SourceTable {
                name: unique_name(&base, &mut taken),
                file: file.clone(),
                part,
                schema,
                reader,
            });
        }
    }
    Ok(set)
}

/// The DuckDB extensions loading `set` needs, in a fixed order.
pub fn extensions_needed(set: &SourceSet) -> Vec<&'static str> {
    let mut needed = Vec::new();
    for (reader, ext) in [
        (Reader::XlsxSheet, "excel"),
        (Reader::SqliteTable, "sqlite"),
    ] {
        if set.tables.iter().any(|t| t.reader == reader) {
            needed.push(ext);
        }
    }
    needed
}

/// The SQL that loads `table` as `"raw_<name>"` with the three lineage columns: the file (relative
/// to `protocol_dir` when it lies inside, else absolute), the sheet or table, and the 1-based row.
///
/// A step that needs an extension starts with its `INSTALL` and `LOAD`, so the Protocol records it.
pub fn load_sql(table: &SourceTable, protocol_dir: &Path) -> String {
    let shown = table.file.strip_prefix(protocol_dir).unwrap_or(&table.file);
    let file = literal(&table.file.to_string_lossy());
    let lineage_file = literal(&shown.to_string_lossy());
    // A table outside `main` is recorded as `schema.table` so two schemas' tables stay apart.
    let part = match (&table.schema, &table.part) {
        (Some(schema), Some(part)) => literal(&format!("{schema}.{part}")),
        (None, Some(part)) => literal(part),
        _ => "NULL::VARCHAR".to_string(),
    };
    let target = format!(
        "CREATE OR REPLACE TABLE {} AS",
        ident(&format!("raw_{}", table.name))
    );
    let lead = format!("{lineage_file} AS __source_file, {part} AS __source_part");
    let row_number = "row_number() OVER () AS __source_row";
    let part_name = table.part.as_deref().unwrap_or_default();
    match table.reader {
        Reader::Csv | Reader::Tsv => {
            let delim = if table.reader == Reader::Tsv { ", delim = '\\t'" } else { "" };
            format!(
                "SET preserve_insertion_order = true; \
                 {target} SELECT {lead}, {row_number}, * FROM read_csv({file}{delim});"
            )
        }
        Reader::Parquet => format!(
            "{target} SELECT {lead}, file_row_number + 1 AS __source_row, * EXCLUDE (file_row_number) \
             FROM read_parquet({file}, file_row_number = true);"
        ),
        Reader::XlsxSheet => format!(
            "INSTALL excel; LOAD excel; SET preserve_insertion_order = true; \
             {target} SELECT {lead}, {row_number}, * \
             FROM read_xlsx({file}, sheet = {}, header = true, all_varchar = true);",
            literal(part_name)
        ),
        Reader::SqliteTable => format!(
            "INSTALL sqlite; LOAD sqlite; SET preserve_insertion_order = true; \
             {target} SELECT {lead}, {row_number}, * FROM sqlite_scan({file}, {});",
            literal(part_name)
        ),
        Reader::DuckdbTable => format!(
            "SET preserve_insertion_order = true; \
             DETACH DATABASE IF EXISTS __source; ATTACH {file} AS __source (READ_ONLY); \
             {target} SELECT {lead}, {row_number}, * FROM {}; \
             DETACH DATABASE __source;",
            qualified("__source", table.schema.as_deref().unwrap_or("main"), part_name)
        ),
    }
}

/// A SQL string literal.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A quoted SQL identifier.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `db.schema.table`, each part quoted.
fn qualified(db: &str, schema: &str, table: &str) -> String {
    format!("{db}.{}.{}", ident(schema), ident(table))
}

/// Lower-cased, with each non-alphanumeric turned to `_`; a name already taken gets `_2`, `_3`, in
/// the order the tables are discovered.
fn unique_name(base: &str, taken: &mut HashSet<String>) -> String {
    let clean: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut name = clean.clone();
    let mut n = 2;
    while !taken.insert(name.clone()) {
        name = format!("{clean}_{n}");
        n += 1;
    }
    name
}

/// A DuckDB connection that discovery asks its questions of, loading extensions on first need.
struct Probe {
    conn: Result<Connection, String>,
    loaded: HashSet<&'static str>,
}

impl Probe {
    fn new() -> Self {
        Probe {
            conn: Connection::open_in_memory().map_err(|e| e.to_string()),
            loaded: HashSet::new(),
        }
    }

    fn conn(&self) -> Result<&Connection, String> {
        self.conn.as_ref().map_err(Clone::clone)
    }

    fn extension(&mut self, ext: &'static str) -> Result<(), String> {
        if self.loaded.contains(ext) {
            return Ok(());
        }
        self.conn()?
            .execute_batch(&format!("INSTALL {ext}; LOAD {ext};"))
            .map_err(|e| format!("the {ext} extension is unavailable: {e}"))?;
        self.loaded.insert(ext);
        Ok(())
    }

    /// Reads the first row through the reader that will load the file; whatever DuckDB refuses
    /// (an unreadable encoding, a truncated Parquet) is the reason it is skipped.
    fn flat(&self, file: &Path, reader: Reader) -> Result<Parts, String> {
        let f = literal(&file.to_string_lossy());
        let from = match reader {
            Reader::Tsv => format!("read_csv({f}, delim = '\\t')"),
            Reader::Parquet => format!("read_parquet({f})"),
            _ => format!("read_csv({f})"),
        };
        self.ask(&format!("SELECT * FROM {from} LIMIT 1"))?;
        Ok(vec![(None, None, reader)])
    }

    fn workbook(
        &mut self,
        file: &Path,
        skipped: &mut Vec<(PathBuf, String)>,
    ) -> Result<Parts, String> {
        self.extension("excel")?;
        let f = literal(&file.to_string_lossy());
        let mut parts = Vec::new();
        for sheet in workbook_sheets(file)? {
            let sql = format!(
                "SELECT * FROM read_xlsx({f}, sheet = {}, all_varchar = true) LIMIT 1",
                literal(&sheet)
            );
            match self.ask(&sql) {
                Ok(()) => parts.push((None, Some(sheet), Reader::XlsxSheet)),
                // A sheet with no cells at all is the one failure DuckDB names this way.
                Err(e) if e.contains("No rows found") => {
                    skipped.push((file.to_path_buf(), format!("empty sheet ({sheet})")));
                }
                Err(e) => skipped.push((file.to_path_buf(), format!("sheet {sheet}: {e}"))),
            }
        }
        Ok(parts)
    }

    fn database(&mut self, file: &Path) -> Result<Parts, String> {
        let (reader, attach) = if is_sqlite(file)? {
            self.extension("sqlite")?;
            (Reader::SqliteTable, "(TYPE sqlite, READ_ONLY)")
        } else {
            (Reader::DuckdbTable, "(READ_ONLY)")
        };
        let conn = self.conn()?;
        let f = literal(&file.to_string_lossy());
        conn.execute_batch(&format!(
            "DETACH DATABASE IF EXISTS __probe; ATTACH {f} AS __probe {attach};"
        ))
        .map_err(|e| e.to_string())?;
        let listed = (|| -> duckdb::Result<Parts> {
            let mut stmt = conn.prepare(
                "SELECT schema_name, table_name FROM duckdb_tables() \
                 WHERE database_name = '__probe' ORDER BY schema_name, table_name",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.map(|r| {
                r.map(|(schema, table)| {
                    let schema = (schema != "main").then_some(schema);
                    (schema, Some(table), reader)
                })
            })
            .collect()
        })();
        let _ = conn.execute_batch("DETACH DATABASE __probe;");
        listed.map_err(|e| e.to_string())
    }

    /// Runs a query that may return nothing, as a check that DuckDB can read what it names.
    fn ask(&self, sql: &str) -> Result<(), String> {
        match self.conn()?.query_row(sql, [], |_| Ok(())) {
            Ok(()) | Err(duckdb::Error::QueryReturnedNoRows) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Whether `file` starts with SQLite's magic header; a `.db` file may be either engine's.
fn is_sqlite(file: &Path) -> Result<bool, String> {
    let mut head = [0u8; 16];
    let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let n = f.read(&mut head).map_err(|e| e.to_string())?;
    Ok(&head[..n] == b"SQLite format 3\0")
}

/// The sheet names of a workbook, in workbook order.
///
/// DuckDB's excel extension cannot list a workbook's sheets: `read_xlsx` is the one function that
/// reads a workbook, and it takes a sheet to read rather than naming them. So the names are read
/// here, from `xl/workbook.xml`, and DuckDB reads the data.
fn workbook_sheets(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a workbook: {e}"))?;
    let mut xml = String::new();
    zip.by_name("xl/workbook.xml")
        .map_err(|e| format!("not a workbook: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("not a workbook: {e}"))?;
    Ok(sheet_names(&xml))
}

/// The `name` of each `<sheet>` element in a workbook part.
fn sheet_names(xml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        // The element is `sheet`, optionally behind a namespace prefix (`x:sheet`); `sheets` and
        // `sheetPr` are other elements, and a sheet's tag name is followed by whitespace.
        let tag_name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let tag_name = &rest[..tag_name_end];
        if tag_name.rsplit(':').next() != Some("sheet") {
            continue;
        }
        rest = &rest[tag_name_end..];
        if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        if let Some(name) = attribute(tag, "name") {
            names.push(unescape(name));
        }
        rest = &rest[end..];
    }
    names
}

/// The raw value of `name="..."` (or single-quoted) inside one tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(at) = tag[from..].find(name) {
        let start = from + at;
        let after = &tag[start + name.len()..];
        let preceded = tag[..start].ends_with(|c: char| c.is_ascii_whitespace());
        let after = after.trim_start();
        if preceded && after.starts_with('=') {
            let value = after[1..].trim_start();
            let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let value = &value[1..];
            return value.find(quote).map(|end| &value[..end]);
        }
        from = start + name.len();
    }
    None
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').and_then(|end| {
            let ch = match &rest[1..end] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                n => {
                    let code = match n.strip_prefix("#x").or_else(|| n.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => n.strip_prefix('#')?.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_keep_order_and_decode_entities() {
        let xml = r#"<workbook><sheets><sheet name="Q1 &amp; Q2" sheetId="1" r:id="rId1"/><sheet sheetId="2" name='It&apos;s' r:id="rId2"/></sheets></workbook>"#;
        assert_eq!(sheet_names(xml), ["Q1 & Q2", "It's"]);
        let prefixed =
            r#"<x:workbook><x:sheets><x:sheet name="A" sheetId="1"/></x:sheets></x:workbook>"#;
        assert_eq!(sheet_names(prefixed), ["A"]);
    }

    #[test]
    fn names_are_cleaned_and_collisions_numbered() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("Sales 2026", &mut taken), "sales_2026");
        assert_eq!(unique_name("sales_2026", &mut taken), "sales_2026_2");
        assert_eq!(unique_name("sales-2026", &mut taken), "sales_2026_3");
    }
}
