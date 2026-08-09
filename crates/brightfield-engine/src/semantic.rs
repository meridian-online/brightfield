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
//! in this crate's tests is a column of eight strings of which one is an email
//! address, and it comes back labelled as an email column with most of the
//! values failing that label's own pattern. A label nothing checked is a label
//! that can send a chart down the wrong road silently, so
//! [`SemanticType::Unusable`] exists and is reported separately from
//! [`SemanticType::Unlabelled`]: "the values contradict this" and "there is
//! nothing to say" are different answers and a caller must be able to tell them
//! apart.
//!
//! No confidence figure is quoted anywhere in this module's prose. It is a
//! property of whichever model a bundle happens to carry, this repository
//! vendors no bundle, and a number no test can assert is a number that goes
//! quietly wrong.
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
///
/// Three variants because there are three situations, and two of them are ways
/// of NOT having checked. Collapsing them loses the difference between a
/// classifier that named a type this bundle has never heard of — which is what
/// a model and a schema catalogue from different FineType versions looks like,
/// one column at a time — and a type whose definition simply carries nothing a
/// value could fail.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueCheck {
    /// The catalogue holds no entry for this label at all.
    ///
    /// The classifier emitted a type the schema catalogue does not describe.
    /// Per column that is unremarkable; across many columns it is the visible
    /// edge of a bundle whose model and catalogue disagree, which
    /// [`FinetypeBundle::open`] refuses in the aggregate.
    NoEntry,
    /// The catalogue's entry for this label constrains nothing a value could
    /// fail — no pattern, no length bound, no enumeration.
    ///
    /// Every value reaches the validator through `CAST(… AS VARCHAR)`, so a
    /// schema of `{"type": "string"}` would pass by construction and report a
    /// check that tested nothing.
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

impl ValueCheck {
    /// Whether any value was actually put through a validator.
    ///
    /// `false` for both [`ValueCheck::NoEntry`] and [`ValueCheck::NoConstraint`]
    /// — the two ways of arriving at a label nothing tested. A caller deciding
    /// how much to trust a label wants this, not a match on one variant.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        matches!(self, ValueCheck::Checked { checked, .. } if *checked > 0)
    }
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

/// How a session acquires its [`TypeSource`].
///
/// A [`TypeSource`] cannot be built before the session exists — it has to load
/// itself into that session's own DuckDB connection — so what a caller hands
/// [`LoadOptions`] is this, not a source.
///
/// The two variants are the swap point. Nothing downstream of
/// [`ColumnProfile::semantic`] can tell which one produced a label, which is
/// what "swappable without touching any caller" has to mean to be worth
/// anything.
///
/// [`LoadOptions`]: crate::LoadOptions
/// [`ColumnProfile::semantic`]: crate::ColumnProfile::semantic
#[derive(Clone)]
pub enum TypeSourceSpec {
    /// A FineType bundle directory: `finetype.duckdb_extension`, `model/`,
    /// `taxonomy-schemas.json`. What a packaged build ships.
    Bundle(PathBuf),
    /// Any other implementation of the seam.
    Other(std::sync::Arc<dyn OpenTypeSource>),
}

impl std::fmt::Debug for TypeSourceSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeSourceSpec::Bundle(dir) => f.debug_tuple("Bundle").field(dir).finish(),
            TypeSourceSpec::Other(_) => f.write_str("Other(..)"),
        }
    }
}

impl TypeSourceSpec {
    /// Bring the source up against `conn`.
    ///
    /// # Errors
    ///
    /// Whatever stopped it, in words naming the source. The caller reports
    /// this and carries on with no type source; it is never fatal to a load.
    pub fn open(&self, conn: &Connection) -> Result<Box<dyn TypeSource>, String> {
        match self {
            TypeSourceSpec::Bundle(dir) => {
                FinetypeBundle::open(dir, conn).map(|b| Box::new(b) as Box<dyn TypeSource>)
            }
            TypeSourceSpec::Other(open) => open.open(conn),
        }
    }

    /// Whether opening this source needs signature checking off on the
    /// connection.
    ///
    /// True for [`TypeSourceSpec::Bundle`]: a FineType extension built anywhere
    /// but DuckDB's own community pipeline carries 256 zero bytes where a
    /// signature would go, so `LOAD` refuses it otherwise. A
    /// [`TypeSourceSpec::Other`] answers for itself and says no by default,
    /// which is why a source that loads nothing native — the common case — is
    /// usable on any session.
    #[must_use]
    pub fn needs_unsigned_extensions(&self) -> bool {
        match self {
            TypeSourceSpec::Bundle(_) => true,
            TypeSourceSpec::Other(open) => open.needs_unsigned_extensions(),
        }
    }
}

/// Something that can bring a [`TypeSource`] up against a live connection.
///
/// Implement this beside a [`TypeSource`] to put it behind the seam without
/// touching the engine.
pub trait OpenTypeSource: Send + Sync {
    /// Load whatever this source needs into `conn` and return it ready to
    /// answer.
    ///
    /// # Errors
    ///
    /// Whatever stopped it, in words that name the source.
    fn open(&self, conn: &Connection) -> Result<Box<dyn TypeSource>, String>;

    /// Whether this source needs DuckDB's extension signature checking turned
    /// off on the connection it is opened against.
    ///
    /// `false` by default, and say so honestly: the flag is not per-extension,
    /// so a source that asks for it takes signature checking off for
    /// EVERYTHING that connection loads. Answering `true` therefore shuts the
    /// acquisition routes the engine itself travels — see
    /// [`LoadOptions::type_source`], which records what that covers and the
    /// measured form of `INSTALL` it does not.
    ///
    /// [`LoadOptions::type_source`]: crate::LoadOptions::type_source
    fn needs_unsigned_extensions(&self) -> bool {
        false
    }
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
    /// What the bundle's schema catalogue says about each label.
    schemas: SchemaCatalogue,
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

        // ── Everything readable off disk is decided HERE, before the LOAD. ──
        //
        // Ordering, not taste: `LOAD` maps a shared library into this process
        // and runs its entry point, so every question that can be answered
        // without doing that should be. It also keeps this whole stretch
        // reachable by a test holding a synthesised bundle and no extension at
        // all, which is the difference between these checks being exercised on
        // every CI run and only when somebody has a 30 MB artefact to hand.
        verify_against_manifest(dir, conn).map_err(|e| at(MANIFEST_FILE, e))?;

        let model = dir.join(MODEL_DIR);
        if !model.join("model.safetensors").exists() {
            return Err(at(
                MODEL_DIR,
                "no model.safetensors — the extension would fall back to downloading one".into(),
            ));
        }

        let catalogue = dir.join(SCHEMA_CATALOGUE);
        let text =
            std::fs::read_to_string(&catalogue).map_err(|e| at(SCHEMA_CATALOGUE, e.to_string()))?;
        let schemas = parse_schema_catalogue(&text).map_err(|e| at(SCHEMA_CATALOGUE, e))?;

        // The model and the catalogue are two artefacts, bundled independently,
        // and nothing before this compares them.
        let labels = std::fs::read_to_string(model.join(LABEL_MAP_FILE))
            .map_err(|e| at(LABEL_MAP_FILE, e.to_string()))?;
        let coverage = catalogue_coverage(&labels, &schemas).map_err(|e| at(LABEL_MAP_FILE, e))?;
        coverage.accept().map_err(|e| at("coverage", e))?;

        // ── From here the extension is running. ──
        let quoted = ext.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("LOAD '{quoted}';"))
            .map_err(|e| at("LOAD", e.to_string()))?;

        let name = conn
            .query_row("SELECT ft_version()", [], |r| r.get::<_, String>(0))
            .map_err(|e| at("ft_version()", e.to_string()))?;

        // Immediately before the first call that needs a model, not earlier: a
        // bundle whose extension will not even load must not leave a pointer to
        // its model behind for the next bundle opened in this process.
        seal_model_dir_to(&model).map_err(|e| at(MODEL_DIR_ENV, e))?;

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
        self.schemas.schema_for(label)
    }

    /// Whether the bundle's catalogue carries an entry for this label at all.
    #[must_use]
    pub fn catalogue_mentions(&self, label: &str) -> bool {
        self.schemas.mentions(label)
    }

    /// The validation query for one column: how many of its first
    /// [`Self::CHECK_ROWS`] non-null values fail the schema bound as `?`.
    ///
    /// Split out from the private `count_failures` that runs it, so the row
    /// cap is visible to a test that needs no extension, no model and no
    /// bundle. The cap only means anything if it reaches the SQL.
    #[must_use]
    pub fn check_sql(view: &str, column: &str) -> String {
        let v = crate::escape_ident(view);
        let c = crate::escape_ident(column);
        format!(
            "SELECT CAST(count(*) AS BIGINT), \
             CAST(count(*) FILTER (WHERE NOT v.valid) AS BIGINT), \
             min(v.message) FILTER (WHERE NOT v.valid) \
             FROM (SELECT ft_validate_text(CAST(\"{c}\" AS VARCHAR), ?::VARCHAR) AS v \
             FROM \"{v}\" WHERE \"{c}\" IS NOT NULL LIMIT {}) s",
            Self::CHECK_ROWS
        )
    }

    /// Count how many of a column's first [`Self::CHECK_ROWS`] non-null values
    /// fail `schema`.
    fn count_failures(
        conn: &Connection,
        view: &str,
        column: &str,
        schema: &str,
    ) -> Result<(u64, u64, Option<String>), String> {
        let sql = Self::check_sql(view, column);
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

    /// Turn a label and a count of failures into the verdict.
    ///
    /// Pure, and public within the crate, because this is where
    /// [`Self::MAX_FAILURE_RATE`] is actually applied — a test can pin the
    /// threshold's BEHAVIOUR here with no extension, no model and no bundle,
    /// which is the only way a constant nothing in CI exercises stops drifting.
    #[must_use]
    pub fn verdict(
        label: String,
        confidence: f64,
        checked: u64,
        failed: u64,
        first_failure: Option<String>,
    ) -> SemanticType {
        // `checked == 0` is a column of nothing but NULLs: there is no evidence
        // either way, so it cannot be a contradiction.
        #[allow(clippy::cast_precision_loss)]
        let over = checked > 0 && (failed as f64) > (checked as f64) * Self::MAX_FAILURE_RATE;
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

/// The file `scripts/package.sh` writes recording what it staged.
///
/// `<sha256>  <path relative to the bundle>` per line — the format
/// `shasum -a 256` prints and `shasum -c` reads, so it is checkable by hand.
pub const MANIFEST_FILE: &str = "bundle-manifest.sha256";

/// The model's label set, inside [`MODEL_DIR`]. A JSON array of every label the
/// classifier can emit.
pub const LABEL_MAP_FILE: &str = "label_map.json";

/// Check every file a bundle manifest names against its recorded hash.
///
/// Hashing goes through DuckDB — `sha256(read_blob(…))` — rather than a Rust
/// digest crate, because the connection is already open and it is the same
/// library that will shortly be asked to `LOAD` the file.
///
/// WHAT THIS IS FOR, precisely. A bundle is assembled by one machine, tarred,
/// and unpacked somewhere else. It catches a truncated unpack, a partial copy,
/// a stale extension left behind by an earlier build, and a model swapped for a
/// different version's. It does NOT make loading an unsigned extension safe
/// against an adversary: the manifest sits in the directory it describes, so
/// anyone able to replace the extension can rewrite the manifest — and could
/// equally replace the executable that reads either. The control that bounds
/// the unsigned-extension relaxation is not this; it is that the relaxation is
/// granted only to a connection on which the engine's own acquisition routes
/// are shut (see [`LoadOptions::type_source`], which also records the form of
/// `INSTALL` that is outside that bound).
///
/// A bundle with no manifest passes. That is the locally assembled bundle a
/// contributor points `BRIGHTFIELD_FINETYPE_BUNDLE` at, which no packaging run
/// has ever recorded hashes for. What it must not do is pass a manifest whose
/// hashes disagree, or one naming a file that is gone.
///
/// # Errors
///
/// A file the manifest names that is missing or has different bytes, or a
/// manifest that is not in the format above.
///
/// [`LoadOptions::type_source`]: crate::LoadOptions::type_source
pub fn verify_against_manifest(dir: &Path, conn: &Connection) -> Result<(), String> {
    let manifest = dir.join(MANIFEST_FILE);
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return Ok(());
    };
    let mut checked = 0usize;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (want, rel) = line
            .split_once("  ")
            .ok_or_else(|| format!("line {}: expected '<sha256>  <path>', got {line:?}", n + 1))?;
        if want.len() != 64 || !want.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("line {}: {want:?} is not a sha256", n + 1));
        }
        // A manifest path that climbs out of the bundle would have this verify
        // some other file and report the bundle clean.
        if rel.contains("..") || rel.starts_with('/') {
            return Err(format!("line {}: {rel:?} is not inside the bundle", n + 1));
        }
        let path = dir.join(rel);
        let got: String = conn
            .query_row(
                "SELECT lower(sha256(content)) FROM read_blob(?)",
                duckdb::params![path.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .map_err(|e| format!("{rel}: cannot be read to hash it: {e}"))?;
        if got != want.to_ascii_lowercase() {
            return Err(format!(
                "{rel} is not the file that was packaged (recorded {want}, found {got})"
            ));
        }
        checked += 1;
    }
    if checked == 0 {
        return Err("the manifest names no files, so it verified nothing".to_string());
    }
    Ok(())
}

/// How much of the model's label set the schema catalogue describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueCoverage {
    /// Labels the model can emit.
    pub model_labels: usize,
    /// How many of those the catalogue has an entry for.
    pub described: usize,
}

impl CatalogueCoverage {
    /// The share of the model's labels the catalogue can say anything about,
    /// below which the two artefacts are treated as coming from different
    /// FineType versions.
    ///
    /// Not 100%: a catalogue legitimately omits labels whose definition carries
    /// nothing checkable, and [`parse_schema_catalogue`] drops those on the way
    /// in, so perfect coverage is not achievable by construction. Half is well
    /// under any plausible honest gap and well over what version skew leaves —
    /// a catalogue from a different taxonomy generation shares only the labels
    /// that never got renamed.
    pub const MIN_DESCRIBED: f64 = 0.5;

    /// Whether this coverage is consistent with one FineType version.
    ///
    /// # Errors
    ///
    /// Names both counts, because "the model and the catalogue disagree" is
    /// only actionable with the size of the disagreement.
    pub fn accept(&self) -> Result<(), String> {
        #[allow(clippy::cast_precision_loss)]
        let share = if self.model_labels == 0 {
            0.0
        } else {
            self.described as f64 / self.model_labels as f64
        };
        if share < Self::MIN_DESCRIBED {
            return Err(format!(
                "the schema catalogue describes {} of the model's {} labels — the two were \
                 built from different FineType versions, and a column typed with a label the \
                 catalogue has never heard of gets no value check at all",
                self.described, self.model_labels
            ));
        }
        Ok(())
    }
}

/// Compare the model's label set against the catalogue's.
///
/// # Errors
///
/// `label_map.json` is not the JSON array of label strings the model ships.
pub fn catalogue_coverage(
    label_map: &str,
    schemas: &SchemaCatalogue,
) -> Result<CatalogueCoverage, String> {
    let parsed: serde_json::Value = serde_json::from_str(label_map).map_err(|e| e.to_string())?;
    let labels = parsed
        .as_array()
        .ok_or_else(|| "expected a JSON array of label strings".to_string())?;
    let mut model_labels = 0usize;
    let mut described = 0usize;
    for label in labels {
        let Some(label) = label.as_str() else {
            return Err("expected a JSON array of label strings".to_string());
        };
        model_labels += 1;
        if schemas.mentions(label) {
            described += 1;
        }
    }
    if model_labels == 0 {
        return Err("the model declares no labels".to_string());
    }
    Ok(CatalogueCoverage {
        model_labels,
        described,
    })
}

/// Make `FINETYPE_MODEL_DIR` name THIS bundle's model, or say why it cannot.
///
/// The extension reads that variable and, finding nothing there, downloads a
/// model from HuggingFace — which is precisely what an air-gapped build must
/// not do. There is no SQL setting for it, so the environment is the only lever
/// the extension offers.
///
/// # Why a value already there is refused
///
/// It used to be left alone, on the reasoning that an existing value was the
/// operator's to choose. The consequence was that a bundle whose weights were
/// eighteen bytes of rubbish passed [`FinetypeBundle::open`]'s canary whenever
/// the variable happened to be exported: the extension classified beautifully,
/// using somebody else's model, and every claim this module then made about
/// the bundle was a claim about a file the bundle does not contain.
///
/// So an inherited value pointing anywhere else is an error. There is no
/// legitimate reading of it — `open` exists to answer "does THIS bundle work",
/// and it cannot be answered by a model the bundle did not supply. A value that
/// already names this model is fine and common: it is what the previous
/// successful open in this process wrote.
///
/// # Errors
///
/// The variable names a different directory, with both paths in the message.
fn seal_model_dir_to(dir: &Path) -> Result<(), String> {
    match std::env::var_os(MODEL_DIR_ENV) {
        None => {
            std::env::set_var(MODEL_DIR_ENV, dir);
            Ok(())
        }
        Some(existing) if Path::new(&existing) == dir => Ok(()),
        Some(existing) => Err(format!(
            "{MODEL_DIR_ENV} already names {:?}, so the extension would classify with that \
             model and nothing here would be about this bundle's own ({})",
            existing.to_string_lossy(),
            dir.display()
        )),
    }
}

/// `finetype taxonomy '*' -o json-schema` output, indexed by label.
///
/// Two sets, because "the catalogue says nothing testable about this label" and
/// "the catalogue has never heard of this label" are different facts and the
/// second one is how version skew between the model and the catalogue shows up.
#[derive(Debug, Clone, Default)]
pub struct SchemaCatalogue {
    /// Label → its schema, verbatim, for labels whose schema constrains
    /// something a value could fail.
    checkable: HashMap<String, String>,
    /// Every label the catalogue carries an entry for, checkable or not.
    mentioned: std::collections::HashSet<String>,
}

impl SchemaCatalogue {
    /// The label's schema, for labels with something checkable.
    #[must_use]
    pub fn schema_for(&self, label: &str) -> Option<&str> {
        self.checkable.get(label).map(String::as_str)
    }

    /// Whether the catalogue carries an entry for this label at all.
    #[must_use]
    pub fn mentions(&self, label: &str) -> bool {
        self.mentioned.contains(label)
    }

    /// How many labels the catalogue describes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.mentioned.len()
    }

    /// Whether the catalogue describes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mentioned.is_empty()
    }
}

/// Read `finetype taxonomy '*' -o json-schema` output.
///
/// An entry whose schema constrains nothing is REMEMBERED but not made
/// checkable: every value reaches the validator through `CAST(… AS VARCHAR)`,
/// so a schema of `{"type": "string"}` would pass by construction and report a
/// check that tested nothing. Only a schema carrying one of the keywords below
/// can be failed.
///
/// # Errors
///
/// The catalogue is not JSON, is not the array of schema objects the
/// `finetype taxonomy` exporter emits, or carries no checkable entry at all.
pub fn parse_schema_catalogue(text: &str) -> Result<SchemaCatalogue, String> {
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
    let mut out = SchemaCatalogue::default();
    for entry in entries {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(label) = obj.get("x-finetype-label").and_then(|v| v.as_str()) else {
            continue;
        };
        out.mentioned.insert(label.to_string());
        if CHECKABLE.iter().any(|k| obj.contains_key(*k)) {
            out.checkable.insert(label.to_string(), entry.to_string());
        }
    }
    if out.checkable.is_empty() {
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
        Ok(format!("ft_profile(CAST(\"{c}\" AS VARCHAR), '{header}')"))
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
        // Two ways to arrive at a label nothing tested, and they are different
        // facts about the bundle: no entry at all is the model naming a type
        // this catalogue does not describe; an entry that constrains nothing is
        // a type whose definition has nothing a VARCHAR could fail.
        let Some(schema) = self.schema_for(&label) else {
            let check = if self.catalogue_mentions(&label) {
                ValueCheck::NoConstraint
            } else {
                ValueCheck::NoEntry
            };
            return SemanticType::Labelled {
                label,
                confidence,
                check,
            };
        };
        match FinetypeBundle::count_failures(conn, view, column, schema) {
            Err(reason) => SemanticType::Unanswered { reason },
            Ok((checked, failed, first_failure)) => {
                FinetypeBundle::verdict(label, confidence, checked, failed, first_failure)
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
    [dir.join("finetype"), dir.join("../Resources/finetype")]
        .into_iter()
        .find(|candidate| candidate.join(EXTENSION_FILE).is_file())
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
        assert!(
            err.contains("linux_amd64") && err.contains("osx_arm64"),
            "{err}"
        );

        stamp.platform = "osx_arm64".into();
        stamp.abi_type = "CPP".into();
        let err = check_abi(&stamp, "osx_arm64", "v1.5.2").expect_err("unstable ABI");
        assert!(err.contains("C_STRUCT"), "{err}");
    }

    const CATALOGUE: &str = r#"[
      {"x-finetype-label": "identity.person.email", "type": "string", "pattern": "^.+@.+$"},
      {"x-finetype-label": "representation.text.plain_text", "type": "string"},
      {"x-finetype-label": "geography.address.country_code", "type": "string",
       "enum": ["GB", "DE"]},
      {"type": "string", "pattern": "^x$"}
    ]"#;

    #[test]
    fn catalogue_keeps_constrained_labels_and_remembers_the_bare_ones() {
        let cat = parse_schema_catalogue(CATALOGUE).expect("a well-formed catalogue");
        assert!(cat.schema_for("identity.person.email").is_some());
        assert!(cat.schema_for("geography.address.country_code").is_some());
        assert!(
            cat.schema_for("representation.text.plain_text").is_none(),
            "a bare string schema constrains nothing a CAST-to-VARCHAR value could fail"
        );
        // …but it is still MENTIONED, and that difference is the whole reason
        // ValueCheck has two not-checked variants.
        assert!(cat.mentions("representation.text.plain_text"));
        assert!(!cat.mentions("finance.banking.iban"));
        assert_eq!(cat.len(), 3, "the label-less entry is not indexable");
    }

    #[test]
    fn catalogue_refuses_what_is_not_a_catalogue() {
        assert!(parse_schema_catalogue("not json").is_err());
        assert!(parse_schema_catalogue(r#"{"a": 1}"#).is_err());
        assert!(parse_schema_catalogue("[]").is_err());
    }

    /// A catalogue and a model from different FineType versions is refused,
    /// and one from the same version is not.
    ///
    /// The failure this closes: nothing else compares the two artefacts, and a
    /// mismatched pair passes every other check in this module — the extension
    /// loads, the canary classifies, and columns come back `Labelled` with
    /// `NoEntry` because the classifier is naming types the catalogue has never
    /// heard of. Every label would then be unverified while still reading as a
    /// label.
    #[test]
    fn a_catalogue_that_does_not_describe_the_model_is_refused() {
        let cat = parse_schema_catalogue(CATALOGUE).expect("a well-formed catalogue");

        let agreed = r#"["identity.person.email", "representation.text.plain_text",
                         "geography.address.country_code", "finance.banking.iban"]"#;
        let coverage = catalogue_coverage(agreed, &cat).expect("a label map");
        assert_eq!(coverage.model_labels, 4);
        assert_eq!(coverage.described, 3);
        assert_eq!(coverage.accept(), Ok(()));

        let skewed = r#"["a.b.c", "d.e.f", "g.h.i", "identity.person.email"]"#;
        let coverage = catalogue_coverage(skewed, &cat).expect("a label map");
        assert_eq!(coverage.described, 1);
        let err = coverage.accept().expect_err("1 of 4 is version skew");
        assert!(
            err.contains("different FineType versions"),
            "the refusal does not name the cause: {err}"
        );

        assert!(catalogue_coverage("[]", &cat).is_err());
        assert!(catalogue_coverage(r#"[1, 2]"#, &cat).is_err());
        assert!(catalogue_coverage("not json", &cat).is_err());
    }

    /// The failure tolerance, pinned at its boundary.
    ///
    /// Runs with no extension, no model and no bundle, because
    /// [`FinetypeBundle::MAX_FAILURE_RATE`] is a number this project chose and
    /// a number nothing exercises is a number that drifts. Widening it to
    /// anything above a tenth turns the 11-in-100 case green; narrowing it
    /// below turns the 10-in-100 case red.
    #[test]
    fn the_failure_tolerance_decides_labelled_from_unusable_at_its_boundary() {
        let v = |checked, failed| {
            FinetypeBundle::verdict("identity.person.email".into(), 0.9, checked, failed, None)
        };
        assert!(
            v(100, 10).is_usable(),
            "a tenth failing is a data-quality note, not a wrong label"
        );
        assert!(!v(100, 11).is_usable(), "past a tenth the label is wrong");
        assert!(v(100, 0).is_usable());
        assert!(!v(100, 100).is_usable());

        // A column of nothing but NULLs contradicts nothing.
        assert_eq!(
            v(0, 0),
            SemanticType::Labelled {
                label: "identity.person.email".into(),
                confidence: 0.9,
                check: ValueCheck::Checked {
                    checked: 0,
                    failed: 0
                },
            }
        );
        match v(0, 0) {
            SemanticType::Labelled { check, .. } => assert!(
                !check.is_verified(),
                "nothing was checked, so nothing is verified"
            ),
            other => panic!("expected a label, got {other:?}"),
        }
    }

    /// The row cap reaches the SQL, and is big enough for the tolerance to mean
    /// anything.
    ///
    /// The second half is the one that catches a cap shrunk to a handful:
    /// with a cap so small that a single failure already exceeds
    /// [`FinetypeBundle::MAX_FAILURE_RATE`], the tolerance is off and every
    /// column with one bad value in its first rows becomes `Unusable`.
    #[test]
    fn the_row_cap_reaches_the_query_and_leaves_the_tolerance_expressible() {
        let sql = FinetypeBundle::check_sql("people", "email");
        assert!(
            sql.contains(&format!("LIMIT {}", FinetypeBundle::CHECK_ROWS)),
            "the cap never reaches the query: {sql}"
        );
        assert!(sql.contains("ft_validate_text"));
        assert!(sql.contains("IS NOT NULL"));

        #[allow(clippy::cast_precision_loss)]
        let tolerated = FinetypeBundle::CHECK_ROWS as f64 * FinetypeBundle::MAX_FAILURE_RATE;
        assert!(
            tolerated >= 1.0,
            "a cap of {} tolerates {tolerated} failures, so a single bad value in the first \
             rows makes every column unusable and the tolerance is off",
            FinetypeBundle::CHECK_ROWS
        );
    }

    /// Identifiers reach the query quoted, so a column named with a quote
    /// cannot end the identifier early.
    #[test]
    fn the_check_query_escapes_the_identifiers_it_is_given() {
        let sql = FinetypeBundle::check_sql("we\"ird", "col\"umn");
        assert!(sql.contains("\"we\"\"ird\""), "{sql}");
        assert!(sql.contains("\"col\"\"umn\""), "{sql}");
    }

    /// The model directory is sealed to the bundle's own, and an inherited
    /// value naming somewhere else is refused.
    ///
    /// The failure this closes, measured before it was: with a value already
    /// exported, a bundle whose weights were eighteen bytes of rubbish passed
    /// the canary — the extension classified with somebody else's model and
    /// every claim about the bundle was about a file it does not contain.
    ///
    /// Restores whatever the variable was, because it is process-global and
    /// this is one test in a binary full of others.
    #[test]
    fn the_model_directory_is_sealed_to_the_bundle_that_asked_for_it() {
        let prior = std::env::var_os(MODEL_DIR_ENV);
        std::env::remove_var(MODEL_DIR_ENV);

        let mine = Path::new("/tmp/bf-seal-mine/model");
        assert_eq!(
            seal_model_dir_to(mine),
            Ok(()),
            "an unset variable is ours to set"
        );
        assert_eq!(
            std::env::var_os(MODEL_DIR_ENV).map(PathBuf::from),
            Some(mine.to_path_buf())
        );

        // The same bundle again — what a second open in one process sees.
        assert_eq!(
            seal_model_dir_to(mine),
            Ok(()),
            "our own value is not a conflict"
        );

        // Somebody else's model.
        let theirs = Path::new("/tmp/bf-seal-theirs/model");
        let err = seal_model_dir_to(theirs).expect_err("a foreign model must be refused");
        assert!(
            err.contains("bf-seal-mine") && err.contains("bf-seal-theirs"),
            "the refusal names neither model: {err}"
        );

        match prior {
            Some(v) => std::env::set_var(MODEL_DIR_ENV, v),
            None => std::env::remove_var(MODEL_DIR_ENV),
        }
    }

    #[test]
    fn a_manifest_catches_a_bundle_whose_bytes_changed_since_packaging() {
        let conn = Connection::open_in_memory().expect("in-memory duckdb");
        let dir = std::env::temp_dir().join(format!("bf-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"the packaged bytes").unwrap();

        // No manifest: a locally assembled bundle, nothing recorded, nothing
        // to contradict.
        assert_eq!(verify_against_manifest(&dir, &conn), Ok(()));

        let good: String = conn
            .query_row(
                "SELECT lower(sha256(content)) FROM read_blob(?)",
                duckdb::params![dir.join("a.bin").to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), format!("{good}  a.bin\n")).unwrap();
        assert_eq!(verify_against_manifest(&dir, &conn), Ok(()));

        std::fs::write(dir.join("a.bin"), b"the bytes somebody else put there").unwrap();
        let err = verify_against_manifest(&dir, &conn).expect_err("the bytes changed");
        assert!(err.contains("not the file that was packaged"), "{err}");

        // A manifest that verifies nothing is not a pass.
        std::fs::write(dir.join(MANIFEST_FILE), "# only a comment\n").unwrap();
        assert!(verify_against_manifest(&dir, &conn).is_err());

        // A path climbing out of the bundle would have this hash some other
        // file and report the bundle clean.
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!("{good}  ../elsewhere/a.bin\n"),
        )
        .unwrap();
        let err = verify_against_manifest(&dir, &conn).expect_err("escaping path");
        assert!(err.contains("not inside the bundle"), "{err}");

        std::fs::write(dir.join(MANIFEST_FILE), "not-a-hash  a.bin\n").unwrap();
        assert!(verify_against_manifest(&dir, &conn).is_err());
        std::fs::write(dir.join(MANIFEST_FILE), "nonsense\n").unwrap();
        assert!(verify_against_manifest(&dir, &conn).is_err());

        std::fs::remove_dir_all(&dir).ok();
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
        // Confidences here are arbitrary constructor arguments, not measured
        // ones — nothing in this test depends on their values, and no figure
        // in this module claims to be what a model returned.
        let unusable = SemanticType::Unusable {
            label: "identity.person.email".into(),
            confidence: 0.5,
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
