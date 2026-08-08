//! What a column MEANS, as distinct from what DuckDB stored it as.
//!
//! DESCRIBE answers "VARCHAR". That is a storage fact, and no amount of it
//! tells a chart whether the column holds email addresses, IBANs or free text.
//! This module is the one place Brightfield asks a different question — and the
//! one place the answer is checked against the column's own values before
//! anything is allowed to draw from it.
//!
//! ## The seam
//!
//! [`TypeSource`] is the seam. Exactly one implementation ships,
//! [`FinetypeBundle`], which answers in SQL through the FineType DuckDB
//! extension. Everything downstream — `profile_sources`, the Data sidebar,
//! anything that later wants to pick a mark from a column's meaning — sees only
//! [`SemanticType`] on [`ColumnProfile::semantic`], never the extension, never
//! its SQL and never its model. Replacing what is behind the seam is a change
//! to this module and to the one line in [`Engine::load_spec_with`] that
//! constructs it.
//!
//! What the seam does NOT abstract over: a type source that cannot answer in
//! SQL over the session's own connection. [`TypeSource::typing_expr`] hands
//! back a SQL aggregate expression precisely so the typing rides the profiling
//! pass that already exists rather than adding a scan beside it, and that shape
//! is baked into the trait.
//!
//! ## Why the values are checked
//!
//! The classifier is a model. It returns a label and a confidence for any
//! column you give it, including one whose values it has misread — the fixture
//! in this crate's tests is a column of eight strings, one of them an email
//! address, that comes back `identity.person.email` at 0.79 confidence with
//! seven of the eight failing the label's own pattern. A label nothing checked
//! is a label that can send a chart down the wrong road silently, so
//! [`SemanticType::Unusable`] exists and is reported separately from
//! [`SemanticType::Unlabelled`]: "the values contradict this" and "there is
//! nothing to say" are different answers and a caller must be able to tell them
//! apart.
//!
//! The check is deliberately not the classifier's own opinion. It is the
//! label's declared JSON Schema constraint, run by the extension's
//! `ft_validate_text` — a regex/length/enum test, not a neural one. Two
//! judgements drawn through one routine cannot disagree; these two can.
//!
//! [`ColumnProfile::semantic`]: crate::ColumnProfile::semantic
//! [`Engine::load_spec_with`]: crate::Engine::load_spec_with

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use duckdb::arrow::array::{Array, Float64Array, StringArray, StructArray};
use duckdb::Connection;

// ═══════════════════════════════════════════════════════════════════════════
// THE ANSWER
// ═══════════════════════════════════════════════════════════════════════════

/// What a [`TypeSource`] concluded about one column.
///
/// Five states, and the difference between them is the point: a caller that
/// collapses `Unusable` into `Unlabelled` has thrown away the only signal that
/// says *this column is lying about itself*.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticType {
    /// No type source was configured for this session. Nobody looked, and
    /// nothing here is evidence about the column.
    NotAsked,
    /// A type source was configured and could not answer for this column —
    /// it failed to come up, or the column is a shape it will not read
    /// (a STRUCT/LIST/MAP, whose `CAST(… AS VARCHAR)` is display text rather
    /// than data). `reason` says which.
    Unanswered {
        /// Why no answer, in the source's own words.
        reason: String,
    },
    /// It looked and had nothing to say: the classifier's own `unknown`.
    Unlabelled,
    /// A label, and the column's values do not contradict it.
    Labelled {
        /// The semantic label, e.g. `identity.person.email`.
        label: String,
        /// The classifier's confidence, verbatim — not rounded here.
        confidence: f64,
        /// What checking the values against the label found.
        check: ValueCheck,
    },
    /// A label, and the column's values contradict it. Do not draw from this
    /// column as that type: more than [`FinetypeBundle::MAX_FAILURE_RATE`] of
    /// the values checked failed the label's own declared constraint.
    Unusable {
        /// The label the classifier returned.
        label: String,
        /// Its confidence — high confidence and an unusable column is the
        /// combination worth surfacing.
        confidence: f64,
        /// How many non-null values went through the validator.
        checked: u64,
        /// How many of them failed.
        failed: u64,
        /// The lexicographically first failure message, so the reason is
        /// reproducible rather than whichever row happened to arrive first.
        first_failure: Option<String>,
    },
}

impl SemanticType {
    /// The label, for the two states that carry one.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            SemanticType::Labelled { label, .. } | SemanticType::Unusable { label, .. } => {
                Some(label)
            }
            _ => None,
        }
    }

    /// Whether a caller may draw from this column as its label.
    ///
    /// True only for [`SemanticType::Labelled`]. `Unusable` is false because
    /// the values disagree; the other three are false because there is no
    /// label to draw from.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(self, SemanticType::Labelled { .. })
    }
}

/// What checking a column's values against its label found.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueCheck {
    /// The label's catalogue entry constrains nothing a value could fail —
    /// no pattern, no length bound, no enumeration. No value was put through
    /// a validator. This is NOT a check that passed, and the two are separate
    /// variants so no caller can read it as one.
    NoConstraint,
    /// Values went through the label's declared constraint.
    Checked {
        /// How many non-null values were checked (bounded — see
        /// [`FinetypeBundle::CHECK_ROWS`]).
        checked: u64,
        /// How many failed. Within tolerance; past it the column is
        /// [`SemanticType::Unusable`] instead.
        failed: u64,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// THE SEAM
// ═══════════════════════════════════════════════════════════════════════════

/// Something that can say what a column means.
///
/// Two calls per column, in this order, because they cannot be one: the
/// validation constraint is not known until the label is.
pub trait TypeSource: Send + Sync {
    /// A short name for whatever is behind the seam, for diagnostics.
    fn name(&self) -> &str;

    /// A SQL aggregate expression typing `column`, to be folded into the
    /// caller's own single aggregate pass over the view.
    ///
    /// # Errors
    ///
    /// Names why a column of this DuckDB type cannot be typed. The caller
    /// turns that into [`SemanticType::Unanswered`] and leaves the expression
    /// out of its SELECT.
    fn typing_expr(&self, column: &str, duckdb_type: &str) -> Result<String, String>;

    /// Read back one cell produced by [`TypeSource::typing_expr`] and check
    /// the values behind the label it carries.
    ///
    /// `conn` and `view` are for that check, which is a second, much smaller
    /// query — bounded by row count and issued only for a column that got a
    /// label with something to check.
    fn read_and_check(
        &self,
        conn: &Connection,
        view: &str,
        column: &str,
        cell: &dyn Array,
        row: usize,
    ) -> SemanticType;
}

// ═══════════════════════════════════════════════════════════════════════════
// THE DUCKDB EXTENSION ABI STAMP
// ═══════════════════════════════════════════════════════════════════════════

/// The metadata DuckDB appends to a loadable extension, read back off the
/// bundled file.
///
/// The trailer is the last 512 bytes: eight 32-byte NUL-padded ASCII fields
/// followed by 256 bytes of signature space. The fields are written last-first,
/// so field 1 (the magic) lands at offset 224 and the three this reads sit
/// below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionStamp {
    /// DuckDB platform triple, e.g. `osx_arm64`. Must equal the running
    /// engine's `pragma_platform()`.
    pub platform: String,
    /// The DuckDB version the extension was stamped against. Under the stable
    /// C API this is a FLOOR, not an equality: an extension stamped `v1.2.0`
    /// loads on any DuckDB from 1.2.0 up.
    pub duckdb_version: String,
    /// The extension's own version, e.g. `0.6.56`.
    pub extension_version: String,
    /// `C_STRUCT` for the stable C API. Anything else version-locks the
    /// artefact to one exact DuckDB release.
    pub abi_type: String,
}

/// The trailer length DuckDB appends: 8 × 32-byte fields + 256 signature bytes.
const STAMP_LEN: usize = 512;
/// Offset of field 1 (the magic) within the trailer. Fields are written
/// last-first, so this is the highest of the eight.
const STAMP_MAGIC_OFFSET: usize = 224;
/// Field 1's value in a well-formed trailer.
const STAMP_MAGIC: &str = "4";

fn stamp_field(trailer: &[u8], offset: usize) -> String {
    String::from_utf8_lossy(&trailer[offset..offset + 32])
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

/// Read the DuckDB metadata trailer off the tail of an extension file.
///
/// # Errors
///
/// The file is shorter than a trailer, or field 1 does not carry the magic —
/// i.e. this is a plain shared library that was never stamped, which DuckDB
/// would refuse to `LOAD` with a message about a missing footer.
pub fn read_stamp(file_bytes: &[u8]) -> Result<ExtensionStamp, String> {
    if file_bytes.len() < STAMP_LEN {
        return Err(format!(
            "not a DuckDB extension: {} bytes is shorter than the {STAMP_LEN}-byte metadata trailer",
            file_bytes.len()
        ));
    }
    let trailer = &file_bytes[file_bytes.len() - STAMP_LEN..];
    let magic = stamp_field(trailer, STAMP_MAGIC_OFFSET);
    if magic != STAMP_MAGIC {
        return Err(format!(
            "not a DuckDB extension: metadata magic is {magic:?}, expected {STAMP_MAGIC:?} \
             (an unstamped shared library will not LOAD)"
        ));
    }
    Ok(ExtensionStamp {
        abi_type: stamp_field(trailer, 96),
        extension_version: stamp_field(trailer, 128),
        duckdb_version: stamp_field(trailer, 160),
        platform: stamp_field(trailer, 192),
    })
}

/// `v1.5.2` → `(1, 5, 2)`. Trailing components may be absent.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().unwrap_or("0").parse().ok()?;
    let patch = it
        .next()
        .unwrap_or("0")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

/// Decide whether a stamped extension can be loaded by the DuckDB this build
/// links.
///
/// Three conditions, and each one reddens on a real change rather than on a
/// comment going stale:
///
/// 1. **The ABI must be `C_STRUCT`.** That is DuckDB's stable C API, and it is
///    the only reason one extension artefact survives a DuckDB patch bump at
///    all. An artefact rebuilt against the unstable API stamps something else
///    and is refused here rather than at a segfault.
/// 2. **The platform must match** the running engine's `pragma_platform()`.
///    Cross-packaging an `osx_arm64` extension into a `linux_amd64` artefact
///    fails here, in the packaging run, not on a stranger's machine.
/// 3. **The stamped DuckDB version is a floor** and must not exceed the
///    running one. Bump the extension's floor past this build's DuckDB and
///    this turns red; bump this build's DuckDB and it stays green, which is
///    correct — that is the compatibility the stable ABI buys.
///
/// What it does NOT stop: a DuckDB bump that keeps the floor below and still
/// breaks something at runtime. Nothing static can see that, which is why
/// [`FinetypeBundle::open`] also loads the extension and makes it classify a
/// known column before the bundle is accepted.
///
/// # Errors
///
/// One message per failed condition, naming both sides of the comparison.
pub fn check_abi(
    stamp: &ExtensionStamp,
    running_platform: &str,
    running_version: &str,
) -> Result<(), String> {
    if stamp.abi_type != "C_STRUCT" {
        return Err(format!(
            "extension ABI is {:?}, not \"C_STRUCT\" — only the stable C API survives a \
             DuckDB version bump, so this artefact is pinned to one exact release and \
             cannot be bundled",
            stamp.abi_type
        ));
    }
    if stamp.platform != running_platform {
        return Err(format!(
            "extension is built for {:?} and this DuckDB is {:?}",
            stamp.platform, running_platform
        ));
    }
    let (Some(stamped), Some(running)) = (
        parse_version(&stamp.duckdb_version),
        parse_version(running_version),
    ) else {
        return Err(format!(
            "cannot compare DuckDB versions: extension stamped {:?}, engine reports {:?}",
            stamp.duckdb_version, running_version
        ));
    };
    if stamped > running {
        return Err(format!(
            "extension needs DuckDB {} or newer and this build links {}",
            stamp.duckdb_version, running_version
        ));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// THE FINETYPE BUNDLE
// ═══════════════════════════════════════════════════════════════════════════

/// The three things a bundle directory must contain.
///
/// Named here rather than spelled inline so `scripts/package.sh` and this
/// module cannot drift apart silently — the packaging script stages exactly
/// these paths and this is what refuses a bundle missing any of them.
pub const EXTENSION_FILE: &str = "finetype.duckdb_extension";
/// The model directory inside a bundle. `FINETYPE_MODEL_DIR` is pointed here.
pub const MODEL_DIR: &str = "model";
/// The per-label JSON Schema catalogue inside a bundle, as emitted by
/// `finetype taxonomy '*' -o json-schema`.
pub const SCHEMA_CATALOGUE: &str = "taxonomy-schemas.json";

/// The environment variable the FineType extension reads to find its model.
const MODEL_DIR_ENV: &str = "FINETYPE_MODEL_DIR";

/// A FineType extension + model + schema catalogue, loaded into one DuckDB
/// connection and proved to work before it is handed out.
pub struct FinetypeBundle {
    name: String,
    /// Label → the label's JSON Schema, verbatim, for every label whose schema
    /// constrains something a value could fail.
    schemas: HashMap<String, String>,
}

impl FinetypeBundle {
    /// How many non-null values of a column go through the validator.
    ///
    /// The first rows in scan order, not a random sample: reproducibility is
    /// worth more here than statistical purity, because a column that flips
    /// between usable and unusable across two runs of the same file is worse
    /// than one judged on a fixed prefix. Bounded because the check is a regex
    /// per value and a column can be arbitrarily long.
    pub const CHECK_ROWS: u64 = 500;

    /// The share of checked values that may fail the label's own constraint
    /// before the column is [`SemanticType::Unusable`].
    ///
    /// Not zero: one malformed address in a column of addresses is a data
    /// quality note, not a reason to refuse the column's meaning. A tenth is
    /// the point at which the label stops describing the column — past it,
    /// whatever the classifier saw is not what is actually in there, and
    /// anything drawn on that basis is wrong for a tenth of the rows.
    pub const MAX_FAILURE_RATE: f64 = 0.10;

    /// Open a bundle against `conn`: check the ABI stamp, point the extension
    /// at the bundled model, `LOAD` it, and make it classify a column whose
    /// answer is known.
    ///
    /// The last step is not ceremony. With no model reachable, the extension's
    /// classifier panics inside its own `catch_unwind` and `ft_profile` returns
    /// `unknown` at confidence 0 — which is indistinguishable, at the SQL
    /// surface, from an honest "I have nothing to say about this column". A
    /// bundle that cannot classify email addresses as email addresses is
    /// refused here rather than reporting every column of every source as
    /// unlabelled.
    ///
    /// # Errors
    ///
    /// A missing file, an ABI the running DuckDB cannot take, a `LOAD` DuckDB
    /// refuses, or a canary that comes back wrong. Every one names the bundle
    /// directory.
    pub fn open(dir: &Path, conn: &Connection) -> Result<Self, String> {
        let at = |what: &str, e: String| format!("finetype bundle {}: {what}: {e}", dir.display());

        let ext = dir.join(EXTENSION_FILE);
        let bytes = std::fs::read(&ext).map_err(|e| at(EXTENSION_FILE, e.to_string()))?;
        let stamp = read_stamp(&bytes).map_err(|e| at(EXTENSION_FILE, e))?;

        let platform: String = conn
            .query_row("SELECT * FROM pragma_platform()", [], |r| r.get(0))
            .map_err(|e| at("pragma_platform()", e.to_string()))?;
        let version: String = conn
            .query_row("SELECT version()", [], |r| r.get(0))
            .map_err(|e| at("version()", e.to_string()))?;
        check_abi(&stamp, &platform, &version).map_err(|e| at(EXTENSION_FILE, e))?;

        let model = dir.join(MODEL_DIR);
        if !model.join("model.safetensors").exists() {
            return Err(at(
                MODEL_DIR,
                "no model.safetensors — the extension would fall back to downloading one".into(),
            ));
        }
        point_model_dir_at(&model);

        let quoted = ext.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("LOAD '{quoted}';"))
            .map_err(|e| at("LOAD", e.to_string()))?;

        let catalogue = dir.join(SCHEMA_CATALOGUE);
        let text =
            std::fs::read_to_string(&catalogue).map_err(|e| at(SCHEMA_CATALOGUE, e.to_string()))?;
        let schemas = parse_schema_catalogue(&text).map_err(|e| at(SCHEMA_CATALOGUE, e))?;

        let name = conn
            .query_row("SELECT ft_version()", [], |r| r.get::<_, String>(0))
            .map_err(|e| at("ft_version()", e.to_string()))?;

        let bundle = FinetypeBundle { name, schemas };
        bundle.canary(conn).map_err(|e| at("canary", e))?;
        Ok(bundle)
    }

    /// Classify three email addresses and insist on the answer.
    fn canary(&self, conn: &Connection) -> Result<(), String> {
        let label: String = conn
            .query_row(
                "SELECT ft_profile(v, 'email').\"type\" FROM (VALUES \
                 ('alice@example.com'), ('bob@example.org'), ('carol@example.net')) t(v)",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if label != "identity.person.email" {
            return Err(format!(
                "the extension loaded but classified three email addresses as {label:?} — \
                 its model is not reachable, and every column would come back unlabelled"
            ));
        }
        Ok(())
    }

    /// The label's schema, for labels with something checkable.
    #[must_use]
    pub fn schema_for(&self, label: &str) -> Option<&str> {
        self.schemas.get(label).map(String::as_str)
    }

    /// Count how many of a column's first [`Self::CHECK_ROWS`] non-null values
    /// fail `schema`.
    fn count_failures(
        conn: &Connection,
        view: &str,
        column: &str,
        schema: &str,
    ) -> Result<(u64, u64, Option<String>), String> {
        let v = crate::escape_ident(view);
        let c = crate::escape_ident(column);
        let sql = format!(
            "SELECT CAST(count(*) AS BIGINT), \
             CAST(count(*) FILTER (WHERE NOT v.valid) AS BIGINT), \
             min(v.message) FILTER (WHERE NOT v.valid) \
             FROM (SELECT ft_validate_text(CAST(\"{c}\" AS VARCHAR), ?::VARCHAR) AS v \
             FROM \"{v}\" WHERE \"{c}\" IS NOT NULL LIMIT {}) s",
            Self::CHECK_ROWS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let batches: Vec<_> = stmt
            .query_arrow(duckdb::params![schema])
            .map_err(|e| e.to_string())?
            .collect();
        let batch = batches
            .into_iter()
            .find(|b: &duckdb::arrow::record_batch::RecordBatch| b.num_rows() > 0)
            .ok_or_else(|| "the validation pass returned no rows".to_string())?;
        Ok((
            crate::profile::read_count(&batch, 0),
            crate::profile::read_count(&batch, 1),
            crate::profile::read_text(&batch, 2),
        ))
    }
}

/// Point the FineType extension's model resolution at `dir`, once per process.
///
/// The extension reads `FINETYPE_MODEL_DIR` from the environment and, finding
/// nothing there, downloads a model from HuggingFace — which is precisely what
/// an air-gapped build must not do. There is no SQL setting for it, so the
/// environment is the only lever the extension offers.
///
/// Written once and never overwritten: a value already in the environment is
/// the operator's, and a session opened later on another thread must not have
/// the variable rewritten underneath it. The extension resolves its model into
/// a `OnceLock` on first use, so a later write would be ignored anyway while
/// still being a data race against any thread reading the environment.
fn point_model_dir_at(dir: &Path) {
    if std::env::var_os(MODEL_DIR_ENV).is_none() {
        std::env::set_var(MODEL_DIR_ENV, dir);
    }
}

/// Index `finetype taxonomy '*' -o json-schema` output by label, keeping only
/// the entries that constrain something a value could actually fail.
///
/// A schema of `{"type": "string"}` is dropped: every value reaches the
/// validator through `CAST(… AS VARCHAR)`, so it would pass by construction and
/// report a check that tested nothing. What survives is a schema carrying at
/// least one of the keywords below.
///
/// # Errors
///
/// The catalogue is not JSON, or is not the array of schema objects the
/// `finetype taxonomy` exporter emits.
pub fn parse_schema_catalogue(text: &str) -> Result<HashMap<String, String>, String> {
    /// Keywords a VARCHAR value can fail. `type` is absent on purpose.
    const CHECKABLE: [&str; 6] = [
        "pattern",
        "enum",
        "minLength",
        "maxLength",
        "minimum",
        "maximum",
    ];

    let parsed: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let entries = parsed
        .as_array()
        .ok_or_else(|| "expected a JSON array of schema objects".to_string())?;
    let mut out = HashMap::new();
    for entry in entries {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(label) = obj.get("x-finetype-label").and_then(|v| v.as_str()) else {
            continue;
        };
        if !CHECKABLE.iter().any(|k| obj.contains_key(*k)) {
            continue;
        }
        out.insert(label.to_string(), entry.to_string());
    }
    if out.is_empty() {
        return Err("no entry carries both a label and a checkable constraint".to_string());
    }
    Ok(out)
}

/// Whether `CAST(col AS VARCHAR)` on this DuckDB type would produce data or
/// display text.
///
/// A STRUCT/LIST/MAP/UNION casts to DuckDB's own rendering (`{'city': London}`
/// — inner quotes lost, not valid JSON), so both classifying and validating it
/// would be reading the renderer rather than the data. FineType's own table
/// macros refuse nested columns for the same reason.
fn is_nested_type(duckdb_type: &str) -> bool {
    let upper = duckdb_type.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    upper.ends_with("[]")
        || base.starts_with("STRUCT")
        || base.starts_with("MAP")
        || base.starts_with("UNION")
        || base.starts_with("LIST")
}

impl TypeSource for FinetypeBundle {
    fn name(&self) -> &str {
        &self.name
    }

    fn typing_expr(&self, column: &str, duckdb_type: &str) -> Result<String, String> {
        if is_nested_type(duckdb_type) {
            return Err(format!(
                "nested {duckdb_type} column — unnest, extract or to_json it before asking \
                 what it means"
            ));
        }
        let c = crate::escape_ident(column);
        // The header hint feeds the model's header branch: profiling a column
        // called `email` is a different question from profiling the same
        // values anonymously.
        let header = column.replace('\'', "''");
        Ok(format!(
            "ft_profile(CAST(\"{c}\" AS VARCHAR), '{header}')"
        ))
    }

    fn read_and_check(
        &self,
        conn: &Connection,
        view: &str,
        column: &str,
        cell: &dyn Array,
        row: usize,
    ) -> SemanticType {
        let Some(st) = cell.as_any().downcast_ref::<StructArray>() else {
            return SemanticType::Unanswered {
                reason: format!(
                    "ft_profile returned {:?}, not the STRUCT it documents",
                    cell.data_type()
                ),
            };
        };
        let label = st
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .filter(|a| !a.is_null(row))
            .map(|a| a.value(row).to_string());
        let confidence = st
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .filter(|a| !a.is_null(row))
            .map_or(0.0, |a| a.value(row));
        let Some(label) = label else {
            return SemanticType::Unanswered {
                reason: "ft_profile returned no type for this column".to_string(),
            };
        };
        if label == "unknown" {
            return SemanticType::Unlabelled;
        }
        let Some(schema) = self.schema_for(&label) else {
            return SemanticType::Labelled {
                label,
                confidence,
                check: ValueCheck::NoConstraint,
            };
        };
        match FinetypeBundle::count_failures(conn, view, column, schema) {
            Err(reason) => SemanticType::Unanswered { reason },
            Ok((checked, failed, first_failure)) => {
                let over = checked > 0
                    && (failed as f64) > (checked as f64) * FinetypeBundle::MAX_FAILURE_RATE;
                if over {
                    SemanticType::Unusable {
                        label,
                        confidence,
                        checked,
                        failed,
                        first_failure,
                    }
                } else {
                    SemanticType::Labelled {
                        label,
                        confidence,
                        check: ValueCheck::Checked { checked, failed },
                    }
                }
            }
        }
    }
}

/// Where a packaged Brightfield looks for its bundle, relative to the running
/// executable.
///
/// Two layouts ship, and both are checked because both are produced by
/// `scripts/package.sh`: the tarball puts `finetype/` beside the binary, and
/// the macOS app bundle puts it under `Contents/Resources/` while the binary
/// sits in `Contents/MacOS/`. Returns `None` when neither exists — a build
/// packaged without a bundle runs with no type source rather than failing.
#[must_use]
pub fn bundle_beside(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    for candidate in [dir.join("finetype"), dir.join("../Resources/finetype")] {
        if candidate.join(EXTENSION_FILE).is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed trailer, built the way `append-duckdb-metadata` builds
    /// one: eight 32-byte fields written last-first, then 256 signature bytes.
    fn trailer(platform: &str, duckdb_version: &str, ext_version: &str, abi: &str) -> Vec<u8> {
        let pad = |s: &str| {
            let mut f = [0u8; 32];
            f[..s.len()].copy_from_slice(s.as_bytes());
            f
        };
        let mut v = vec![0u8; 96];
        v.extend_from_slice(&pad(abi));
        v.extend_from_slice(&pad(ext_version));
        v.extend_from_slice(&pad(duckdb_version));
        v.extend_from_slice(&pad(platform));
        v.extend_from_slice(&pad("4"));
        v.extend_from_slice(&[0u8; 256]);
        assert_eq!(v.len(), STAMP_LEN);
        v
    }

    #[test]
    fn stamp_reads_the_four_fields_off_the_trailer() {
        let mut file = b"a shared library's worth of bytes".to_vec();
        file.extend_from_slice(&trailer("osx_arm64", "v1.2.0", "0.6.56", "C_STRUCT"));
        let stamp = read_stamp(&file).expect("well-formed trailer");
        assert_eq!(stamp.platform, "osx_arm64");
        assert_eq!(stamp.duckdb_version, "v1.2.0");
        assert_eq!(stamp.extension_version, "0.6.56");
        assert_eq!(stamp.abi_type, "C_STRUCT");
    }

    #[test]
    fn stamp_refuses_an_unstamped_library() {
        let file = vec![0u8; 4096];
        let err = read_stamp(&file).expect_err("all-zero tail carries no magic");
        assert!(err.contains("magic"), "{err}");
        let err = read_stamp(b"short").expect_err("shorter than a trailer");
        assert!(err.contains("shorter"), "{err}");
    }

    #[test]
    fn abi_accepts_a_stable_extension_older_than_the_engine() {
        let stamp = ExtensionStamp {
            platform: "osx_arm64".into(),
            duckdb_version: "v1.2.0".into(),
            extension_version: "0.6.56".into(),
            abi_type: "C_STRUCT".into(),
        };
        assert_eq!(check_abi(&stamp, "osx_arm64", "v1.5.2"), Ok(()));
        // The compatibility the stable ABI buys: bumping the ENGINE forward
        // must not turn this red.
        assert_eq!(check_abi(&stamp, "osx_arm64", "v1.9.0"), Ok(()));
    }

    #[test]
    fn abi_reddens_on_a_floor_above_the_engine() {
        let stamp = ExtensionStamp {
            platform: "osx_arm64".into(),
            duckdb_version: "v1.6.0".into(),
            extension_version: "0.6.56".into(),
            abi_type: "C_STRUCT".into(),
        };
        let err = check_abi(&stamp, "osx_arm64", "v1.5.2").expect_err("floor above the engine");
        assert!(err.contains("v1.6.0") && err.contains("v1.5.2"), "{err}");
    }

    #[test]
    fn abi_reddens_on_a_platform_or_abi_mismatch() {
        let mut stamp = ExtensionStamp {
            platform: "linux_amd64".into(),
            duckdb_version: "v1.2.0".into(),
            extension_version: "0.6.56".into(),
            abi_type: "C_STRUCT".into(),
        };
        let err = check_abi(&stamp, "osx_arm64", "v1.5.2").expect_err("wrong platform");
        assert!(err.contains("linux_amd64") && err.contains("osx_arm64"), "{err}");

        stamp.platform = "osx_arm64".into();
        stamp.abi_type = "CPP".into();
        let err = check_abi(&stamp, "osx_arm64", "v1.5.2").expect_err("unstable ABI");
        assert!(err.contains("C_STRUCT"), "{err}");
    }

    #[test]
    fn catalogue_keeps_constrained_labels_and_drops_bare_ones() {
        let text = r#"[
          {"x-finetype-label": "identity.person.email", "type": "string", "pattern": "^.+@.+$"},
          {"x-finetype-label": "representation.text.plain_text", "type": "string"},
          {"x-finetype-label": "geography.address.country_code", "type": "string",
           "enum": ["GB", "DE"]},
          {"type": "string", "pattern": "^x$"}
        ]"#;
        let map = parse_schema_catalogue(text).expect("a well-formed catalogue");
        assert!(map.contains_key("identity.person.email"));
        assert!(map.contains_key("geography.address.country_code"));
        assert!(
            !map.contains_key("representation.text.plain_text"),
            "a bare string schema constrains nothing a CAST-to-VARCHAR value could fail"
        );
        assert_eq!(map.len(), 2, "the label-less entry is not indexable");
    }

    #[test]
    fn catalogue_refuses_what_is_not_a_catalogue() {
        assert!(parse_schema_catalogue("not json").is_err());
        assert!(parse_schema_catalogue(r#"{"a": 1}"#).is_err());
        assert!(parse_schema_catalogue("[]").is_err());
    }

    #[test]
    fn nested_types_are_refused_and_scalar_ones_are_not() {
        for t in ["STRUCT(city VARCHAR)", "VARCHAR[]", "MAP(VARCHAR, INTEGER)"] {
            assert!(is_nested_type(t), "{t} should be refused");
        }
        for t in ["VARCHAR", "BIGINT", "DECIMAL(18,3)", "TIMESTAMP", "BLOB"] {
            assert!(!is_nested_type(t), "{t} should be typeable");
        }
    }

    #[test]
    fn semantic_type_only_calls_labelled_usable() {
        let labelled = SemanticType::Labelled {
            label: "identity.person.email".into(),
            confidence: 0.9,
            check: ValueCheck::Checked {
                checked: 8,
                failed: 0,
            },
        };
        let unusable = SemanticType::Unusable {
            label: "identity.person.email".into(),
            confidence: 0.79,
            checked: 8,
            failed: 7,
            first_failure: None,
        };
        assert!(labelled.is_usable());
        assert!(!unusable.is_usable());
        assert!(!SemanticType::Unlabelled.is_usable());
        assert!(!SemanticType::NotAsked.is_usable());
        assert!(!SemanticType::Unanswered { reason: "x".into() }.is_usable());
        // Both carry the label; only one of them may be drawn from.
        assert_eq!(labelled.label(), Some("identity.person.email"));
        assert_eq!(unusable.label(), Some("identity.person.email"));
        assert_eq!(SemanticType::Unlabelled.label(), None);
    }

    #[test]
    fn bundle_is_found_beside_a_binary_and_under_app_resources() {
        let tmp = std::env::temp_dir().join(format!("bf-bundle-{}", std::process::id()));
        let tar = tmp.join("tarball");
        let app = tmp.join("Brightfield.app");
        std::fs::create_dir_all(tar.join("finetype")).unwrap();
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources/finetype")).unwrap();
        std::fs::write(tar.join("finetype").join(EXTENSION_FILE), b"x").unwrap();
        std::fs::write(
            app.join("Contents/Resources/finetype").join(EXTENSION_FILE),
            b"x",
        )
        .unwrap();

        assert_eq!(
            bundle_beside(&tar.join("brightfield")),
            Some(tar.join("finetype"))
        );
        assert_eq!(
            bundle_beside(&app.join("Contents/MacOS/brightfield")),
            Some(app.join("Contents/MacOS/../Resources/finetype"))
        );
        assert_eq!(bundle_beside(&tmp.join("nowhere/brightfield")), None);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
