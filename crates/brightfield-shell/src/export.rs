//! The headless exports: `export-png` and `export-data` (and `export-pdf`,
//! parsed here) run the shipped binary with no window over a Protocol that has
//! run, and write what its panels draw or read.
//!
//! Both read the Parquet snapshot [`crate::protocol_panels::load`] makes and
//! leave the Protocol's database to it, so the attach that holds the file is in
//! one place.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arc::spec::MANIFEST_FILENAME;
use brightfield_spec::layout::Rect;

use crate::capture::{headless_device, render_composed, DEFAULT_SCALE};
use crate::protocol_panels::{
    load, panel_paths, parquet_name, protocol_tables, sql_string, PanelProblem, ProtocolPanels,
};
use crate::sql_ident::quote;

/// One headless export, as the command line asked for it.
#[derive(Clone, Debug, PartialEq)]
pub enum Export {
    /// `export-pdf <dir> [--out PATH]`: every panel on a page of its own.
    Pdf {
        /// The Protocol folder, or its `arcform.yaml`.
        dir: PathBuf,
        /// Where to write; `<dir>/exports/<name>.pdf` when none.
        out: Option<PathBuf>,
    },
    /// `export-png <dir> <panel> [--out PATH] [--scale N]`.
    Png {
        /// The Protocol folder, or its `arcform.yaml`.
        dir: PathBuf,
        /// The panel, as `x`, `x.yaml` or `panels/x.yaml`.
        panel: String,
        /// Where to write; `<dir>/exports/<panel>.png` when none.
        out: Option<PathBuf>,
        /// Device pixels to a logical pixel.
        scale: f32,
    },
    /// `export-data <dir> <panel> [--format csv|parquet] [--out PATH]`.
    Data {
        /// The Protocol folder, or its `arcform.yaml`.
        dir: PathBuf,
        /// The panel, as `x`, `x.yaml` or `panels/x.yaml`.
        panel: String,
        /// The file format.
        format: DataFormat,
        /// A file for one table, a directory for several; `<dir>/exports/` when none.
        out: Option<PathBuf>,
    },
}

/// The formats `export-data` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFormat {
    /// Comma-separated values with a header row.
    Csv,
    /// Parquet.
    Parquet,
}

impl DataFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Parquet => "parquet",
        }
    }

    fn copy_options(self) -> &'static str {
        match self {
            Self::Csv => "FORMAT csv, HEADER",
            Self::Parquet => "FORMAT parquet",
        }
    }
}

/// Whether `name` is one of the export subcommands.
#[must_use]
pub fn is_export_command(name: &str) -> bool {
    matches!(name, "export-pdf" | "export-png" | "export-data")
}

/// Read the arguments after the subcommand `name`.
///
/// # Errors
///
/// A usage message: a missing or extra argument, a flag that is not the
/// subcommand's, or a value that does not parse.
pub fn parse_export(name: &str, args: impl Iterator<Item = String>) -> Result<Export, String> {
    let mut args = args;
    let mut positional: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut scale = DEFAULT_SCALE;
    let mut format = DataFormat::Csv;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => {
                out = Some(PathBuf::from(
                    args.next().ok_or(format!("{name}: --out needs a path"))?,
                ));
            }
            "--scale" if name == "export-png" => {
                let v = args
                    .next()
                    .ok_or(format!("{name}: --scale needs a number"))?;
                scale = v
                    .parse::<f32>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .ok_or(format!("{name}: --scale needs a positive number, not {v}"))?;
            }
            "--format" if name == "export-data" => {
                let v = args
                    .next()
                    .ok_or(format!("{name}: --format needs csv or parquet"))?;
                format = match v.as_str() {
                    "csv" => DataFormat::Csv,
                    "parquet" => DataFormat::Parquet,
                    _ => return Err(format!("{name}: --format is csv or parquet, not {v}")),
                };
            }
            flag if flag.starts_with("--") => {
                return Err(format!("{name}: unknown option {flag}"));
            }
            _ => positional.push(a),
        }
    }
    let want = if name == "export-pdf" { 1 } else { 2 };
    let usage = match name {
        "export-pdf" => "export-pdf <protocol> [--out PATH]",
        "export-png" => "export-png <protocol> <panel> [--out PATH] [--scale N]",
        _ => "export-data <protocol> <panel> [--format csv|parquet] [--out PATH]",
    };
    if positional.len() != want {
        return Err(format!("usage: {usage}"));
    }
    let mut positional = positional.into_iter();
    let dir = PathBuf::from(positional.next().expect("counted above"));
    let panel = positional.next().unwrap_or_default();
    Ok(match name {
        "export-pdf" => Export::Pdf { dir, out },
        "export-png" => Export::Png {
            dir,
            panel,
            out,
            scale,
        },
        _ => Export::Data {
            dir,
            panel,
            format,
            out,
        },
    })
}

/// Run the export. Answers the paths written, or one line per problem for
/// stderr.
///
/// # Errors
///
/// The lines to print: the Protocol did not open, the panel is unknown or did
/// not draw (`<panel file>: <message>`, one per problem), there is no GPU, or a
/// file could not be written.
pub fn run_export(export: &Export) -> Result<Vec<PathBuf>, Vec<String>> {
    match export {
        Export::Pdf { .. } => Err(vec!["export-pdf is not built yet".to_string()]),
        Export::Png {
            dir,
            panel,
            out,
            scale,
        } => export_png(dir, panel, out.as_deref(), *scale),
        Export::Data {
            dir,
            panel,
            format,
            out,
        } => export_data(dir, panel, *format, out.as_deref()),
    }
}

/// Run the export for the command line: each written path on stdout, each
/// problem on stderr. The process's exit code.
#[must_use]
pub fn run_cli(export: &Export) -> i32 {
    match run_export(export) {
        Ok(paths) => {
            for path in paths {
                println!("{}", path.display());
            }
            0
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            1
        }
    }
}

/// The panel file `panel` names in the Protocol at `dir`, as `panels/x.yaml`.
/// `panel` is `x`, `x.yaml` or `panels/x.yaml`.
///
/// # Errors
///
/// No such panel; the message names the panels that exist.
pub fn resolve_panel(dir: &Path, panel: &str) -> Result<PathBuf, String> {
    let dir = protocol_dir(dir);
    let existing: Vec<String> = panel_paths(&dir)
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let wanted = panel.strip_prefix("panels/").unwrap_or(panel);
    let wanted = wanted.strip_suffix(".yaml").unwrap_or(wanted);
    let file = format!("{wanted}.yaml");
    if existing.contains(&file) {
        return Ok(Path::new("panels").join(file));
    }
    let names: Vec<&str> = existing
        .iter()
        .map(|f| f.strip_suffix(".yaml").unwrap_or(f))
        .collect();
    Err(if names.is_empty() {
        format!("no panel named {panel}: {} has no panels", dir.display())
    } else {
        format!(
            "no panel named {panel}; the panels are {}",
            names.join(", ")
        )
    })
}

/// The Protocol folder `dir` names: itself, or the folder of its `arcform.yaml`.
fn protocol_dir(dir: &Path) -> PathBuf {
    if dir.is_file() && dir.file_name().is_some_and(|n| n == MANIFEST_FILENAME) {
        return dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    }
    dir.to_path_buf()
}

/// Open the Protocol and find the tile for `panel`; its problems are the
/// export's lines when it did not draw.
fn open_tile(dir: &Path, panel: &str) -> Result<(ProtocolPanels, usize, PathBuf), Vec<String>> {
    let panels = load(dir, Rect::zero()).map_err(|p| vec![p.message()])?;
    let file = resolve_panel(dir, panel).map_err(|m| vec![m])?;
    let index = panels
        .tiles
        .iter()
        .position(|t| t.file == file)
        .ok_or_else(|| vec![format!("no panel named {panel}")])?;
    if let Err(problems) = &panels.tiles[index].outcome {
        return Err(lines(&file, problems));
    }
    Ok((panels, index, file))
}

fn lines(file: &Path, problems: &[PanelProblem]) -> Vec<String> {
    problems
        .iter()
        .map(|p| format!("{}: {}", file.display(), p.message()))
        .collect()
}

fn panel_stem(file: &Path) -> String {
    file.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

fn write_problem(path: &Path, e: &dyn std::fmt::Display) -> Vec<String> {
    vec![format!("could not write {}: {e}", path.display())]
}

fn make_parent(path: &Path) -> Result<(), Vec<String>> {
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => std::fs::create_dir_all(parent).map_err(|e| write_problem(parent, &e)),
        None => Ok(()),
    }
}

fn export_png(
    dir: &Path,
    panel: &str,
    out: Option<&Path>,
    scale: f32,
) -> Result<Vec<PathBuf>, Vec<String>> {
    let dir = protocol_dir(dir);
    let (mut panels, index, file) = open_tile(&dir, panel)?;
    let Ok(composed) = panels.tiles.remove(index).outcome else {
        unreachable!("open_tile answers a tile that drew");
    };
    if headless_device().is_err() {
        return Err(vec![
            "no GPU is available; headless export draws with the GPU and needs one".to_string(),
        ]);
    }
    let path = out.map_or_else(
        || {
            dir.join("exports")
                .join(format!("{}.png", panel_stem(&file)))
        },
        Path::to_path_buf,
    );
    let image = render_composed(composed, scale).map_err(|e| vec![e])?;
    make_parent(&path)?;
    image.save(&path).map_err(|e| write_problem(&path, &e))?;
    Ok(vec![path])
}

fn export_data(
    dir: &Path,
    panel: &str,
    format: DataFormat,
    out: Option<&Path>,
) -> Result<Vec<PathBuf>, Vec<String>> {
    let dir = protocol_dir(dir);
    let (panels, index, file) = open_tile(&dir, panel)?;
    let mut seen = BTreeSet::new();
    let tables: Vec<String> = protocol_tables(&panels.tiles[index].source)
        .into_iter()
        .filter(|t| seen.insert(t.to_lowercase()))
        .collect();
    if tables.is_empty() {
        return Err(vec![format!(
            "{}: the panel reads no table from the database, so there are no rows to export",
            file.display()
        )]);
    }
    let into_dir = match out {
        Some(out) => tables.len() > 1 || out.is_dir(),
        None => true,
    };
    if tables.len() > 1 && out.is_some_and(|o| o.exists() && !o.is_dir()) {
        return Err(vec![format!(
            "{}: the panel reads {} tables ({}); --out must be a directory",
            file.display(),
            tables.len(),
            tables.join(", ")
        )]);
    }
    let folder = out.map_or_else(|| dir.join("exports"), Path::to_path_buf);
    let conn = duckdb::Connection::open_in_memory()
        .map_err(|e| vec![format!("could not start DuckDB: {e}")])?;
    let mut written = Vec::new();
    for table in &tables {
        let snapshot = panels
            .tables_dir()
            .join(parquet_name(&table.to_lowercase()));
        let path = if into_dir {
            folder.join(format!(
                "{}.{}",
                parquet_name(table).trim_end_matches(".parquet"),
                format.extension()
            ))
        } else {
            folder.clone()
        };
        make_parent(&path)?;
        conn.execute_batch(&format!(
            "COPY (SELECT * FROM read_parquet({})) TO {} ({})",
            sql_string(&snapshot.to_string_lossy()),
            sql_string(&path.to_string_lossy()),
            format.copy_options()
        ))
        .map_err(|e| {
            vec![format!(
                "{}: could not write the rows of {}: {e}",
                file.display(),
                quote(table)
            )]
        })?;
        written.push(path);
    }
    Ok(written)
}
