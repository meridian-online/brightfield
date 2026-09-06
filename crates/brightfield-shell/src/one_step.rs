//! The Protocol a data file opens as: one SQL step, one table, and the
//! columns the rails draw.
//!
//! Opening a CSV or a Parquet used to fill one of the window's two documents
//! and leave the other empty, so the navigator, Steps and inspector rails
//! reported that nothing existed while a dashboard of the file's own columns
//! was drawn beside them. This module is the missing half: the **spec** that
//! says what was opened, in the shape `arc` already accepts for a local read —
//! a `sql:` step whose model creates the table, `depends_on:` naming the file
//! and `produces:` naming the table.
//!
//! # What is written and what is not
//!
//! Brightfield writes the spec and no run record. The step is *not run* and
//! says so; the table the charts read is materialised by the chart document's
//! own engine exactly as it was before this module existed. This module writes
//! text: it opens no database, executes no SQL and asks `arc` to run nothing.
//! The model file is a declaration, and what reads it back is
//! [`crate::protocol::load_protocol_str`], which derives the lineage graph from
//! its text.
//!
//! # Where the Protocol's directory is, and why
//!
//! **The data file's own directory**, with the file named relative to it.
//!
//! The other option was open: the pinned loader takes a `depends_on:` entry as
//! an opaque string — `arc::spec::Manifest::from_yaml_str` does not touch the
//! filesystem — and the runner resolves it with `dir.join(entry)`, which
//! honours an absolute path by discarding the base. Measured against `arc
//! 0.1.0` (the pinned rev): a protocol directory holding only `arcform.yaml`
//! and `models/load.sql`, with `depends_on:` naming a CSV two directories
//! away by absolute path, runs and reports that file as an external source
//! feeding the step. So the Protocol's directory is not *forced* to be the
//! file's.
//!
//! It is the file's anyway, for two reasons that are about the product rather
//! than the loader. The navigator rail draws the file asset's label, and that
//! label is whatever string the spec spells the path as: an absolute path is
//! unreadable in a 240-point rail and is different on every machine, which
//! makes it unphotographable — a committed baseline of the rails would encode
//! the checkout it was recorded in. And a Protocol saved beside its data is
//! one `arc run` away from being real, where one saved elsewhere carries an
//! absolute path that breaks the moment either end moves.
//!
//! # Why the file is spelled `./name`
//!
//! [`brightfield_protocol::graph`] reads a `depends_on:` entry with no path
//! separator as a *relation* name, not a file — so a bare `readings.csv` would
//! raise a second, spurious table node beside the file the SQL already
//! consumes. The `./` prefix is what makes the entry path-like, and the SQL
//! spells the file the same way so both resolve to one node.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use brightfield_engine::ColumnProfile;
use brightfield_protocol::graph::AssetId;

use crate::dashboard::{ChosenBy, Dashboard};
use crate::data_file::OPENABLE_EXTENSIONS;
use crate::protocol::{load_protocol_str, ProtocolInputs};

/// The one step's name — what the Steps pane lists and what `produced by`
/// names in the inspector.
pub const STEP_NAME: &str = "load";

/// Where the step's model is written, relative to the Protocol's directory.
///
/// `models/` is `arc`'s own convention for a `sql:` step's file. The other
/// half of the pair — the manifest's own filename — is not spelled here:
/// [`brightfield_protocol::MANIFEST_FILENAME`] re-exports the loader's
/// constant, so a bump that renamed it cannot leave this writing to a name the
/// loader has stopped looking for.
pub const MODEL_PATH: &str = "models/load.sql";

/// What the rails know about one column of the opened table.
///
/// One value per column of the file, in the file's own order — including the
/// columns the dashboard declined, because a column with no picture is still a
/// column of the table and the navigator rail lists the table's columns rather
/// than the dashboard's tiles.
//
// No `Eq`: [`ColumnFacts::moments`] carries `f64` fields, which have no total
// equality. Nothing under `crates/` keys a set or a map on this type — checked
// by grep over `HashSet`, `BTreeSet` and the map constructors — so dropping it
// costs no call site.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnFacts {
    /// The column's name, as the table spells it.
    pub column: String,
    /// The whole semantic label the tile was chosen from, when a trusted label
    /// decided it — `representation.numeric.decimal_number`. `None` when the
    /// DuckDB type decided instead, which is what a session with no FineType
    /// bundle behind it gets.
    pub label: Option<String>,
    /// What the 240-point navigator rail draws beside the column name: the
    /// **leaf** of [`Self::label`] (`decimal_number`), or the storage type when
    /// there is no label. The whole of it is the inspector's to show.
    pub leaf: String,
    /// The DuckDB type name the profile carried.
    pub storage: String,
    /// The chart kind the generator gave this column, or `None` for a column
    /// it declined.
    pub tile: Option<String>,
    /// Why that kind, or — for a declined column — why none.
    pub because: String,
    /// The other half of a coordinate pair, when this column is drawn as one:
    /// a point map is a single tile over two columns, and each of the two names
    /// the other here. `None` for a column drawn on its own.
    pub paired: Option<String>,
    /// Rows in the table (null and non-null alike), measured by the engine
    /// when the file was profiled.
    pub rows: u64,
    /// How many of them are null.
    pub nulls: u64,
    /// The column's minimum, rendered by DuckDB. `None` for a type it does not
    /// carry one for.
    pub min: Option<String>,
    /// The column's maximum. See [`Self::min`].
    pub max: Option<String>,
    /// The mean, the median, the deviation, the exact distinct count and the
    /// counted shape of a numeric column — what the grid pane's column header
    /// band states and draws. `None` for a column the engine defines no moment
    /// over: see [`ColumnMoments`](brightfield_engine::ColumnMoments).
    pub moments: Option<brightfield_engine::ColumnMoments>,
}

impl ColumnFacts {
    /// The label if there is one, else the storage type — what the inspector
    /// shows on its `finetype` row.
    #[must_use]
    pub fn full_type(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.storage)
    }
}

/// The spec a data file opened as: its text, its model, where it belongs on
/// disk, and the columns of the table it produces.
#[derive(Clone, Debug)]
pub struct OneStepProtocol {
    /// The Protocol's name, which is also the table's — the file's stem, as a
    /// SQL identifier.
    pub name: String,
    /// The data file, as the caller named it.
    pub data: PathBuf,
    /// The directory the spec belongs in: the data file's own.
    pub dir: PathBuf,
    /// How the spec spells the file — `./california_housing.parquet`. The one
    /// string both `depends_on:` and the model use, so the graph resolves them
    /// to one node.
    pub spelled: String,
    /// The `arcform.yaml` text.
    pub manifest: String,
    /// The `models/load.sql` text.
    pub model: String,
    /// One entry per column of the table, in the table's own order — what the
    /// navigator rail lists.
    pub columns: Vec<ColumnFacts>,
    /// One entry per **tile**, in the order the dashboard laid them out and
    /// therefore the order the composition places its plots — what a click on
    /// plot *n* names. Not [`Self::columns`] filtered: see the private
    /// `tiles_in_plot_order` for the point map that separates them.
    pub tiles: Vec<ColumnFacts>,
}

impl OneStepProtocol {
    /// The Protocol for `path`, over the columns the engine profiled and the
    /// dashboard the generator chose from them.
    #[must_use]
    pub fn of(path: &Path, columns: &[ColumnProfile], dashboard: &Dashboard) -> Self {
        let name = table_name(path);
        let file_name = path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let spelled = format!("./{file_name}");
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self {
            model: model_sql(&name, &spelled, reader_for(path)),
            manifest: manifest_yaml(&name, &spelled),
            columns: facts(columns, dashboard),
            tiles: tiles_in_plot_order(columns, dashboard),
            name,
            data: path.to_path_buf(),
            dir,
            spelled,
        }
    }

    /// The lineage graph, the steps sheet and the columns, as the protocol
    /// document's inputs.
    ///
    /// Derived from the spec's own text through the same loader a manifest read
    /// off disk goes through, so the document a freshly opened file produces
    /// and the document its saved spec would produce are the same by
    /// construction rather than by two spellings agreeing.
    ///
    /// # Errors
    ///
    /// Whatever the loader refuses — which for a spec this module wrote is a
    /// build-time defect rather than a user's circumstance, so the caller
    /// surfaces it rather than swallowing it.
    pub fn inputs(&self) -> Result<ProtocolInputs, String> {
        let mut inputs = load_protocol_str(&self.manifest, &[(MODEL_PATH, &self.model)])?;
        inputs.table = Some(self.table_id());
        inputs.columns.clone_from(&self.columns);
        inputs.tiles.clone_from(&self.tiles);
        inputs.source = Some(self.clone());
        Ok(inputs)
    }

    /// The asset id of the table this Protocol produces — the outline row the
    /// columns are listed under.
    ///
    /// Spelled the way [`brightfield_protocol::graph::build_graph`] spells a
    /// relation node, because that is the id the outline carries and a second
    /// derivation of it here would be a second thing to keep in step.
    #[must_use]
    pub fn table_id(&self) -> AssetId {
        format!("asset.{}.{}", self.name, self.name)
    }

    /// Where [`Self::save_to`] would write the manifest for `dir`.
    #[must_use]
    pub fn manifest_path_in(dir: &Path) -> PathBuf {
        dir.join(brightfield_protocol::MANIFEST_FILENAME)
    }

    /// Write the spec into `dir`: the manifest and its one model.
    ///
    /// **Both files are gated before either is written**, and they need two
    /// gates because the pinned loader reads one of them. The manifest's bytes
    /// go through `arc`'s own validator — the gate `arc run` loads with — which
    /// does not open the model: `Manifest::from_yaml_str` touches no
    /// filesystem, so a `sql:` step naming a model that is malformed, or
    /// absent, is a manifest it accepts. The model is gated by [`Self::inputs`] instead,
    /// which parses it exactly as the rails do and degrades a statement it
    /// cannot read to an issue-badged chip; a spec that would draw one is
    /// refused here rather than written. Either refusal leaves the directory
    /// untouched.
    ///
    /// # Errors
    ///
    /// If the loader refuses the manifest, if the model does not parse into a
    /// clean graph, or if the directory or either file cannot be written. Each
    /// carries the reason.
    pub fn save_to(&self, dir: &Path) -> Result<PathBuf, String> {
        brightfield_protocol::parse_manifest_str(&self.manifest)
            .map_err(|e| format!("the spec brightfield wrote will not load: {e}"))?;
        let degrades = self.inputs()?.degrade_report();
        if !degrades.is_empty() {
            return Err(format!(
                "the model brightfield wrote does not derive a clean graph: {}",
                degrades.join("; ")
            ));
        }
        let manifest = Self::manifest_path_in(dir);
        let model = dir.join(MODEL_PATH);
        let models_dir = model
            .parent()
            .ok_or_else(|| format!("{}: has no parent directory", model.display()))?;
        std::fs::create_dir_all(models_dir)
            .map_err(|e| format!("{}: {e}", models_dir.display()))?;
        std::fs::write(&model, &self.model).map_err(|e| format!("{}: {e}", model.display()))?;
        std::fs::write(&manifest, &self.manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        Ok(manifest)
    }
}

/// The data file a **one-step Protocol** reads, resolved against `dir` — or
/// `None` when `text` is not one.
///
/// The predicate is a shape, not a marker: exactly one step, that step is a
/// `sql:` step, exactly one `depends_on:` entry, and that entry resolves
/// against `dir` to a file on disk whose extension this build opens. A marker
/// comment would be narrower and would also be a lie about what is being
/// recognised — the shape is what makes the file openable, and a hand-written
/// spec of the same shape is as openable as one brightfield wrote.
///
/// What the caller does with the answer is open **the data file**, not the
/// manifest: the graph the rails then draw is the one brightfield derives from
/// the file it just profiled, not a lineage graph recovered from a declaration.
#[must_use]
pub fn data_file_named_by(text: &str, dir: &Path) -> Option<PathBuf> {
    if !brightfield_protocol::is_protocol_manifest(text) {
        return None;
    }
    let manifest = brightfield_protocol::parse_manifest_str(text).ok()?;
    let [step] = manifest.steps.as_slice() else {
        return None;
    };
    step.sql.as_ref()?;
    let [dep] = step.depends_on.as_slice() else {
        return None;
    };
    let path = dir.join(dep);
    let openable = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| OPENABLE_EXTENSIONS.contains(&e.as_str()));
    (openable && path.is_file()).then_some(path)
}

// ---------------------------------------------------------------------------
// The bytes
// ---------------------------------------------------------------------------

/// The DuckDB reader for `path`'s extension — the same dispatch
/// [`OPENABLE_EXTENSIONS`] enumerates, with `tsv` riding the CSV reader.
fn reader_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("parquet") => "read_parquet",
        _ => "read_csv",
    }
}

/// `path`'s stem as a SQL identifier that needs no quoting: a character
/// outside `[A-Za-z0-9_]` becomes `_`, and a name that would start with a digit
/// (or is empty) is prefixed. `a_file_stem_becomes_an_unquoted_identifier`
/// holds both halves.
///
/// Unquoted on purpose. The name is written into the model's `CREATE OR
/// REPLACE TABLE` target, into `produces:`, and read back out of the SQL by
/// [`brightfield_protocol::graph`] — so it crosses a parser twice, and a name
/// that survives both without quoting is a name neither can spell differently.
fn table_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut out: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "t_");
    }
    out
}

/// The `arcform.yaml` a data file opens as.
///
/// Single-quoted scalars for the path, for the reason
/// [`crate::data_file`]'s emitter gives: a single-quoted YAML scalar
/// interprets a doubled quote and leaves the rest of its bytes alone, so a
/// path full of punctuation survives verbatim. `accept` has already refused a
/// path carrying a control character, which is the byte single-quoting cannot
/// carry — YAML folds a line break inside a quoted scalar to a space.
fn manifest_yaml(name: &str, spelled: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "name: {name}");
    let _ = writeln!(out, "engine: duckdb");
    let _ = writeln!(out, "steps:");
    let _ = writeln!(out, "  - name: {STEP_NAME}");
    let _ = writeln!(out, "    sql: {MODEL_PATH}");
    let _ = writeln!(out, "    depends_on:");
    let _ = writeln!(out, "      - {}", yaml_quoted(spelled));
    let _ = writeln!(out, "    produces:");
    let _ = writeln!(out, "      - {name}");
    out
}

/// The one model: the table, created from the file.
///
/// The path is a **SQL** single-quoted literal here and a **YAML** one in the
/// manifest, and the two languages are escaped separately because they are two
/// languages that happen to agree on the doubling rule. `data_file::accept`
/// admits an apostrophe in a file name — it is not glob syntax and not a
/// control character — so `it's.csv` reaches here, and an unescaped literal
/// would be `read_csv('./it's.csv')`: an unterminated string that the graph
/// derivation cannot parse and DuckDB would refuse. It is escaped rather than
/// refused at `accept` because an apostrophe in a file name is ordinary and
/// the escape is exact, where the glob characters `accept` does refuse have no
/// escape this build is willing to rely on.
fn model_sql(name: &str, spelled: &str, reader: &str) -> String {
    format!(
        "CREATE OR REPLACE TABLE {name} AS SELECT * FROM {reader}({});\n",
        sql_quoted(spelled)
    )
}

/// `value` as a SQL single-quoted string literal, with an embedded quote
/// doubled — the one escape a standard SQL string has, and the one DuckDB
/// reads.
fn sql_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `value` as a YAML single-quoted scalar.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// The columns
// ---------------------------------------------------------------------------

/// Every column of the profiled table, paired with what the generator did with
/// it.
///
/// The walk is over the **profile**, not over the dashboard's tiles, so a
/// column the generator declined is still listed — with the reason in
/// [`ColumnFacts::because`] and no tile.
///
/// A column can be drawn by a tile that is not *of* it: a point map is one
/// tile over two columns, whose [`Tile::column`](crate::dashboard::Tile::column)
/// is the longitude and whose
/// [`Tile::paired_column`](crate::dashboard::Tile::paired_column) is the
/// latitude. Both are looked up here, so the latitude row reads as drawn rather
/// than declined — and both carry the other half in [`ColumnFacts::paired`].
/// That is also why the tile list the chart document is handed is built
/// separately, by `tiles_in_plot_order`: two column rows can share one plot.
fn facts(columns: &[ColumnProfile], dashboard: &Dashboard) -> Vec<ColumnFacts> {
    columns.iter().map(|p| facts_for(p, dashboard)).collect()
}

/// One column's facts.
fn facts_for(profile: &ColumnProfile, dashboard: &Dashboard) -> ColumnFacts {
    let tile = dashboard
        .tiles()
        .iter()
        .find(|t| t.column() == profile.name || t.paired_column() == Some(profile.name.as_str()));
    let mut paired = None;
    let (label, because) = match tile.map(crate::dashboard::Tile::chosen_by) {
        Some(ChosenBy::Meaning { label, role }) => (
            Some(label.clone()),
            format!("the semantic type {label}, read as {role:?}"),
        ),
        Some(ChosenBy::Storage { type_name }) => (None, format!("the storage type {type_name}")),
        Some(ChosenBy::CoordinatePair { latitude, rule }) => {
            // The tile is of the longitude and paired with the latitude, so
            // which of the two this row is decides which name it reports as
            // the other half.
            let tile = tile.expect("the arm matched on this tile's own chosen_by");
            let other = if profile.name == *latitude {
                tile.column().to_string()
            } else {
                latitude.clone()
            };
            let because = format!("a coordinate pair with {other}, found by its {rule}");
            paired = Some(other);
            // The label is this column's own, not the pair's: a pair found by
            // its labels has `geography.coordinate.longitude` on one side and
            // `…latitude` on the other, and the rail draws each row its own.
            (profile.semantic.label().map(str::to_string), because)
        }
        None => (
            profile.semantic.label().map(str::to_string),
            dashboard
                .omitted()
                .iter()
                .find(|o| o.column == profile.name)
                .map_or_else(
                    || "no tile".to_string(),
                    |o| format!("no tile — {}", o.because),
                ),
        ),
    };
    let leaf = label.as_deref().map_or_else(
        || profile.type_name.clone(),
        |l| l.rsplit('.').next().unwrap_or(l).to_string(),
    );
    ColumnFacts {
        column: profile.name.clone(),
        label,
        leaf,
        storage: profile.type_name.clone(),
        tile: tile.map(|t| t.kind().as_str().to_string()),
        because,
        paired,
        rows: profile.non_null.saturating_add(profile.nulls),
        nulls: profile.nulls,
        min: profile.min.clone(),
        max: profile.max.clone(),
        moments: profile.moments.clone(),
    }
}

/// The columns the composition's **plots** draw, in the composition's own plot
/// order — one entry per tile.
///
/// Not the column list filtered to the ones that earned a tile, and the
/// difference is a point map: it is one tile over two columns, so filtering the
/// columns yields two entries where the composition places one plot and every
/// click after it names the column next door. Walking the tiles cannot drift
/// that way, because the tiles are what `Dashboard::to_spec` lays out and
/// therefore what the composition places.
///
/// Each entry is the facts of the tile's own
/// [`Tile::column`](crate::dashboard::Tile::column) — for a point map the
/// longitude, with the latitude in [`ColumnFacts::paired`].
fn tiles_in_plot_order(columns: &[ColumnProfile], dashboard: &Dashboard) -> Vec<ColumnFacts> {
    dashboard
        .plot_order()
        .into_iter()
        .filter_map(|tile| {
            let profile = columns.iter().find(|p| p.name == tile.column())?;
            Some(facts_for(profile, dashboard))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stem that is already an identifier survives; one that is not is made
    /// into one, and a leading digit is prefixed rather than dropped.
    #[test]
    fn a_file_stem_becomes_an_unquoted_identifier() {
        assert_eq!(
            table_name(Path::new("/x/california_housing.parquet")),
            "california_housing"
        );
        assert_eq!(
            table_name(Path::new("/x/2026 readings-final.csv")),
            "t_2026_readings_final"
        );
        // A dotfile has no extension as far as `Path` is concerned, so the
        // whole name is the stem — `.csv` sanitises to `_csv`, which is
        // already an identifier and needs no prefix.
        assert_eq!(table_name(Path::new("/x/.csv")), "_csv");
        // A name that sanitises to nothing at all, and one that starts with a
        // digit, are the two cases the prefix exists for.
        assert_eq!(table_name(Path::new("/x/2026.csv")), "t_2026");
        assert_eq!(table_name(Path::new("/x/x")), "x");
    }

    /// The reader dispatches on the extension the same way the engine's source
    /// emitter does, and a `.tsv` rides the CSV reader.
    #[test]
    fn the_reader_follows_the_extension() {
        assert_eq!(reader_for(Path::new("a.parquet")), "read_parquet");
        assert_eq!(reader_for(Path::new("a.PARQUET")), "read_parquet");
        assert_eq!(reader_for(Path::new("a.csv")), "read_csv");
        assert_eq!(reader_for(Path::new("a.tsv")), "read_csv");
    }

    /// The manifest is what the pinned loader accepts, and it declares exactly
    /// the three things the shape needs: the model, the file it reads and the
    /// table it produces.
    #[test]
    fn the_written_manifest_loads_through_the_pinned_gate() {
        let text = manifest_yaml("readings", "./readings.csv");
        let manifest = brightfield_protocol::parse_manifest_str(&text)
            .expect("the spec this module writes has to load through arc's own gate");
        assert_eq!(manifest.name, "readings");
        assert_eq!(manifest.steps.len(), 1);
        assert_eq!(manifest.steps[0].sql.as_deref(), Some(MODEL_PATH));
        assert_eq!(
            manifest.steps[0].depends_on,
            vec!["./readings.csv".to_string()]
        );
        assert_eq!(manifest.steps[0].produces, vec!["readings".to_string()]);
    }

    /// The file `depends_on:` names and the file the model reads are the SAME
    /// string — the property that keeps the graph from raising two nodes for
    /// one file.
    #[test]
    fn the_manifest_and_the_model_spell_the_file_identically() {
        let spec = OneStepProtocol::of(
            Path::new("/data/readings.csv"),
            &[],
            &Dashboard::of(Path::new("/data/readings.csv"), &[]),
        );
        assert_eq!(spec.spelled, "./readings.csv");
        assert!(
            spec.manifest.contains("'./readings.csv'"),
            "the manifest has to name the file: {}",
            spec.manifest
        );
        assert!(
            spec.model.contains("read_csv('./readings.csv')"),
            "the model has to read the same spelling: {}",
            spec.model
        );
    }

    /// A one-step read of a file on disk is recognised; a manifest of any other
    /// shape is not.
    #[test]
    fn the_shape_predicate_takes_one_step_reads_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("bf-one-step-shape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
        std::fs::write(dir.join("readings.csv"), "a\n1\n").expect("the fixture writes");

        let one = manifest_yaml("readings", "./readings.csv");
        assert_eq!(
            data_file_named_by(&one, &dir),
            Some(dir.join("./readings.csv")),
            "a one-step read of a file that is there names that file"
        );

        // Two steps is not this shape.
        let two = format!("{one}  - name: second\n    sql: models/second.sql\n");
        assert_eq!(data_file_named_by(&two, &dir), None);
        // A step with no sql: arm is not this shape.
        let op = "name: m\nsteps:\n  - name: fetch\n    op: http_fetch@1\n    with:\n      url: https://example.com/a.csv\n      out: build/a.csv\n";
        assert_eq!(data_file_named_by(op, &dir), None);
        // A file that is not there is not openable, so it is not this shape.
        let missing = manifest_yaml("gone", "./gone.csv");
        assert_eq!(data_file_named_by(&missing, &dir), None);
        // Nor is a Mosaic spec.
        assert_eq!(
            data_file_named_by("data:\n  a:\n    file: x.csv\nplot:\n  - mark: dot\n", &dir),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
