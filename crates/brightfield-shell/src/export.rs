//! The headless exports: `export-pdf`, `export-png` and `export-data` run the
//! shipped binary with no window over a Protocol that has run, and write its
//! report with the charts in place, what one panel draws, or what it reads.
//!
//! Each reads the Parquet snapshot [`crate::protocol_panels::load`] makes and
//! leaves the Protocol's database to it, so the attach that holds the file is
//! in one place.

use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use arc::spec::MANIFEST_FILENAME;
use brightfield_report::{compile_pdf, listed_charts, reported_run, ChartImage, ReportInput};
use brightfield_spec::layout::Rect;

use crate::capture::{headless_device, render_export, DEFAULT_SCALE};
use crate::design::Mode;
use crate::protocol_panels::{
    load, panel_paths, parquet_name, protocol_tables, sql_string, PanelProblem, ProtocolPanels,
};
use crate::sql_ident::quote;

/// One headless export, as the command line asked for it.
#[derive(Clone, Debug, PartialEq)]
pub enum Export {
    /// `export-pdf <dir> [--out PATH]`: the Protocol's `report.md`, with each
    /// panel its Charts section lists drawn in its place.
    Pdf {
        /// The Protocol folder, or its `arcform.yaml`.
        dir: PathBuf,
        /// Where to write; `<dir>/report.pdf` when none.
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
        Export::Pdf { dir, out } => export_pdf(dir, out.as_deref()),
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

/// Say on stderr, as a problem is said, that the panel's colours repeat. A
/// warning, not a failure: the export still draws and exits 0.
fn warn_colours_repeat(file: &Path, composed: &crate::pipeline::Composed) {
    if let Some(note) = composed.diagnostics.colours_repeat() {
        eprintln!("{}: {note}", file.display());
    }
}

/// The problem when there is no GPU to draw with.
fn require_gpu() -> Result<(), Vec<String>> {
    headless_device().map(|_| ()).map_err(|_| {
        vec!["no GPU is available; headless export draws with the GPU and needs one".to_string()]
    })
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
    require_gpu()?;
    let path = out.map_or_else(
        || {
            dir.join("exports")
                .join(format!("{}.png", panel_stem(&file)))
        },
        Path::to_path_buf,
    );
    warn_colours_repeat(&file, &composed);
    let image = render_export(composed, scale).map_err(|e| vec![e])?;
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

/// The name of the report arc writes in a Protocol's folder.
const REPORT_FILE: &str = "report.md";

/// The name of the read-me a Protocol's folder may hold.
const README_FILE: &str = "README.md";

/// Device pixels to a logical pixel for a chart in the PDF: a 760-pixel-wide
/// panel set to the text width is then drawn at about 340 dots to the inch.
const PDF_CHART_SCALE: f32 = 3.0;

/// A report ready to set: its Markdown, each chart it lists drawn, the notice
/// when tables changed since the run it describes, and its heading.
pub struct PreparedReport {
    /// The text of `report.md`.
    pub markdown: String,
    /// The pictures of the charts the report lists, in its order.
    pub charts: Vec<ChartImage>,
    /// The stale-table notice, as Markdown, when there is one.
    pub notice: Option<String>,
    /// The heading the report opens with: the Protocol's name.
    pub title: String,
    /// The line under it: when the run the report describes finished.
    pub subtitle: Option<String>,
}

impl PreparedReport {
    /// The input `brightfield-report` sets.
    #[must_use]
    pub fn input(&self) -> ReportInput<'_> {
        ReportInput {
            markdown: &self.markdown,
            charts: &self.charts,
            notice: self.notice.as_deref(),
            title: Some(&self.title),
            subtitle: self.subtitle.as_deref(),
        }
    }
}

/// The name the report opens with: the first heading of the Protocol's
/// `README.md` when it has one, else the name its manifest gives it.
fn protocol_title(dir: &Path, name: &str) -> String {
    let readme = std::fs::read_to_string(dir.join(README_FILE)).unwrap_or_default();
    let mut fenced = false;
    for line in readme.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let heading = trimmed[hashes..].trim().trim_end_matches('#').trim();
            if !heading.is_empty() {
                return heading.to_string();
            }
        }
    }
    name.to_string()
}

/// `2026-10-16T21:15:10Z` as `16 October 2026`; the text as it is when it does
/// not start with a date.
fn run_date(finished: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let date = finished.get(..10).unwrap_or(finished);
    let parts: Vec<&str> = date.split('-').collect();
    match parts.as_slice() {
        [year, month, day] if year.len() == 4 => {
            let month = month
                .parse::<usize>()
                .ok()
                .and_then(|m| MONTHS.get(m.checked_sub(1)?));
            let day = day.parse::<u32>().ok();
            match (month, day) {
                (Some(month), Some(day)) => format!("{day} {month} {year}"),
                _ => finished.to_string(),
            }
        }
        _ => finished.to_string(),
    }
}

/// Read the report of the Protocol at `dir` and draw each chart it lists.
/// A panel whose colours repeat is said on stderr, still drawn, and its note
/// set under its caption.
///
/// # Errors
///
/// One line per problem: no `report.md`, the Protocol did not open (it has not
/// run, say), a listed panel is missing, cannot draw or carries a `Check:`
/// line, there is no GPU, or a chart could not be drawn.
pub fn prepare_report(dir: &Path) -> Result<PreparedReport, Vec<String>> {
    let dir = protocol_dir(dir);
    let markdown = std::fs::read_to_string(dir.join(REPORT_FILE)).map_err(|_| {
        vec![format!(
            "no {REPORT_FILE} in {}; write one with arc report write",
            dir.display()
        )]
    })?;
    let panels = load(&dir, Rect::zero()).map_err(|p| vec![p.message()])?;

    let listed = listed_charts(&markdown);
    let mut problems = Vec::new();
    let mut drawn = Vec::new();
    for chart in &listed {
        let file = Path::new(&chart.path);
        for check in &chart.checks {
            problems.push(format!(
                "{}: {REPORT_FILE} says Check: {check}; fix the panel and write the report again",
                file.display()
            ));
        }
        match panels.tiles.iter().position(|t| t.file == file) {
            None => problems.push(format!(
                "{}: {REPORT_FILE} lists this panel, and the Protocol has no such file",
                file.display()
            )),
            Some(index) => match &panels.tiles[index].outcome {
                Err(found) => problems.extend(lines(file, found)),
                Ok(_) => drawn.push((index, chart)),
            },
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    if !drawn.is_empty() {
        require_gpu()?;
    }

    let mut charts: Vec<ChartImage> = Vec::new();
    for (index, chart) in drawn {
        // A panel the report lists twice is drawn once; both items find it.
        if charts.iter().any(|c| c.path == chart.path) {
            continue;
        }
        let tile = &panels.tiles[index];
        // The chart is drawn from the panel as it is now, so its caption is
        // the panel's title now, not the one the report copied when it was
        // written; a panel with no title keeps the report's.
        let title = tile
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| chart.title.clone());
        // The caption is text a reader can search and copy, so the picture is
        // drawn without the heading the panel's `title:` would put in it: the
        // title once, not twice. Only the listed panels are composed again.
        let composed = panels
            .compose_source_at(
                index,
                &without_title(&tile.source),
                Rect::zero(),
                Mode::Light,
            )
            .map_err(|p| lines(&tile.file, &[p]))?;
        warn_colours_repeat(&tile.file, &composed);
        let note = composed.diagnostics.colours_repeat().map(|n| n.to_string());
        let image = render_export(composed, PDF_CHART_SCALE)
            .map_err(|e| vec![format!("{}: {e}", tile.file.display())])?;
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| {
                vec![format!(
                    "{}: the chart could not be encoded: {e}",
                    tile.file.display()
                )]
            })?;
        charts.push(ChartImage {
            path: chart.path.clone(),
            title,
            png,
            width_px: image.width(),
            height_px: image.height(),
            note,
        });
    }

    let run = reported_run(&markdown);
    let notice = match stale_tables(&dir) {
        Ok(stale) if stale.is_empty() => None,
        Ok(stale) => {
            let names: Vec<String> = stale.iter().map(|t| format!("`{t}`")).collect();
            Some(format!(
                "These charts read the Protocol's tables as they are now. {} changed since the run this report describes (`{}`).",
                names.join(", "),
                run.run_id.as_deref().unwrap_or_default()
            ))
        }
        Err(e) => {
            eprintln!("{REPORT_FILE}: {e}");
            None
        }
    };
    Ok(PreparedReport {
        markdown,
        charts,
        notice,
        title: protocol_title(&dir, &panels.name),
        subtitle: run
            .finished
            .as_deref()
            .map(|at| format!("Run finished {}", run_date(at))),
    })
}

/// The panel `source` with its top-level `title:` taken out; the text as it
/// was when it is not a YAML mapping.
fn without_title(source: &str) -> String {
    let Ok(mut panel) = serde_yaml::from_str::<serde_yaml::Value>(source) else {
        return source.to_string();
    };
    let removed = panel
        .as_mapping_mut()
        .is_some_and(|top| top.remove("title").is_some());
    if removed {
        serde_yaml::to_string(&panel).unwrap_or_else(|_| source.to_string())
    } else {
        source.to_string()
    }
}

/// Whether `error` is arc's refusal of a run id its history does not hold:
/// the wording `load_contract` in arc's `contract.rs` gives, at the arcform rev
/// the workspace pins, as an `InvalidInput` I/O error.
/// `a_report_from_another_machine_falls_back_on_its_own_hashes` in
/// `tests/export_pdf.rs` holds the match against arc.
fn is_no_such_run(error: &arc::spec::Error) -> bool {
    matches!(error, arc::spec::Error::Io(e)
        if e.kind() == std::io::ErrorKind::InvalidInput
            && e.to_string().starts_with("there is no run "))
}

/// The tables the Protocol's panels read whose content hash in the newest run
/// differs from the hash the run its report describes recorded, by the names
/// the run gives them. Run ids differ after any run; only a changed hash
/// counts.
///
/// The report's run's hashes are arc's record of that run. When arc's history
/// holds no such run, which is what a Protocol copied from another machine
/// carries, they are the hashes the report's own Run section lists. A table
/// either side did not measure is not counted.
///
/// # Errors
///
/// The Protocol has not run, its report names no run, or arc could not read
/// the run it names for any reason but its not being there.
pub fn stale_tables(dir: &Path) -> Result<Vec<String>, String> {
    let dir = protocol_dir(dir);
    let newest = arc::spec::run_tables(&dir, None).map_err(|e| e.to_string())?;
    let report = std::fs::read_to_string(dir.join(REPORT_FILE)).map_err(|e| e.to_string())?;
    let reported = reported_run(&report);
    let Some(run) = reported.run_id else {
        return Err(
            "it names no run, so whether the tables changed since it was written is not known"
                .to_string(),
        );
    };
    let then = match arc::spec::run_tables(&dir, Some(&run)) {
        Ok(tables) => tables.tables,
        Err(e) if is_no_such_run(&e) => reported.tables,
        Err(e) => return Err(e.to_string()),
    };
    let mut read = BTreeSet::new();
    for path in panel_paths(&dir) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            read.extend(protocol_tables(&text).into_iter().map(|t| t.to_lowercase()));
        }
    }
    let lower = |m: &std::collections::BTreeMap<String, Option<String>>| {
        m.iter()
            .filter_map(|(name, hash)| Some((name.to_lowercase(), (name.clone(), hash.clone()?))))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let (newest, then) = (lower(&newest.tables), lower(&then));
    Ok(read
        .iter()
        .filter_map(|table| {
            let (name, now) = newest.get(table)?;
            let (_, before) = then.get(table)?;
            (now != before).then(|| name.clone())
        })
        .collect())
}

fn export_pdf(dir: &Path, out: Option<&Path>) -> Result<Vec<PathBuf>, Vec<String>> {
    let dir = protocol_dir(dir);
    let report = prepare_report(&dir)?;
    let pdf = compile_pdf(&report.input())
        .map_err(|e| e.lines().map(str::to_string).collect::<Vec<_>>())?;
    let path = out.map_or_else(|| dir.join("report.pdf"), Path::to_path_buf);
    make_parent(&path)?;
    std::fs::write(&path, pdf).map_err(|e| write_problem(&path, &e))?;
    Ok(vec![path])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_drawn_for_the_report_loses_only_its_title() {
        let panel =
            "title: Change in CO2\nplot:\n  - mark: barX\n    x: change_pct\nxLabel: change\n";
        let stripped: serde_yaml::Value =
            serde_yaml::from_str(&without_title(panel)).expect("still YAML");
        assert!(stripped.get("title").is_none());
        assert_eq!(stripped["xLabel"], serde_yaml::Value::from("change"));
        assert!(stripped.get("plot").is_some());
        assert_eq!(without_title("plot: []\n"), "plot: []\n");
    }

    #[test]
    fn the_run_date_reads_as_words() {
        assert_eq!(run_date("2026-10-16T21:15:10Z"), "16 October 2026");
        assert_eq!(run_date("not recorded"), "not recorded");
    }

    #[test]
    fn the_title_is_the_readme_heading_or_the_name() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert_eq!(protocol_title(dir.path(), "co2"), "co2");
        std::fs::write(
            dir.path().join(README_FILE),
            "```\n# not this\n```\n\n## CO2 per capita ##\n\nText.\n",
        )
        .expect("writes");
        assert_eq!(protocol_title(dir.path(), "co2"), "CO2 per capita");
    }
}
