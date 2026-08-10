//! The dashboard a table gets when nobody wrote a spec for it: **one tile per
//! column, each chosen from what that column means**, laid out, and handed over
//! as a spec the reader can open.
//!
//! # Why per column, and not one picture for the table
//!
//! [`crate::chart_kinds::registry`] can answer *"which of my kinds does this
//! whole table fill"*, and the open-a-data-file route used to take the first
//! answer: one chart, over whichever columns that kind's slots swallowed, and
//! silence about every other column in the file. A table of eleven columns
//! opened as a picture of one of them.
//!
//! So the question asked here is the other one: for **each** column, which
//! kinds' *required* slots can that single column fill? That is not the same
//! question and the difference is not cosmetic — `count-grid` requires two
//! categorical slots, and a chooser that reused whole-table applicability would
//! answer "count-grid" for a table of names and then have nothing to say about
//! the third column onwards. [`Dashboard::of`] therefore selects over the kinds
//! a lone column can fill ([`single_column_kinds`]), in the registry's own
//! declaration order, which is where the preference between two applicable
//! kinds is stated.
//!
//! # What decides the tile
//!
//! **The column's semantic type first, its storage type second.** A
//! [`ColumnProfile`] carries a [`SemanticType`] — what the column *means*, as
//! opposed to what DuckDB stored it as — and that is the input that makes this
//! generator different from one switching on a database type. A `BIGINT` of
//! central index keys and a `BIGINT` of currency amounts are the same column to
//! DuckDB and are not the same column to a reader: binning the first produces a
//! histogram of an accession sequence, which is a true picture of nothing.
//!
//! The rule is [`role_of_label`] — one `match` over the label's namespace and
//! family, with the leaf exceptions written out. It answers one of three
//! things, and a label it has no rule for answers nothing at all and leaves the
//! storage type in charge:
//!
//! | role | what it means | what the column gets |
//! |---|---|---|
//! | [`ColumnRole::Measure`] | a measured quantity | binned, if the storage type can be binned |
//! | [`ColumnRole::Category`] | a member of a set | ranked, if it is narrow enough to read |
//! | [`ColumnRole::Neither`] | a value that *names* a thing | no tile at all |
//!
//! [`SemanticType::Unusable`] — a label whose own values contradict it — is
//! deliberately **not** trusted: it falls back to the storage type, because a
//! column labelled an email address whose values are not email addresses is a
//! column nobody has classified.
//!
//! # What is skipped, and why that is a feature
//!
//! A tile drawn badly is worse than a tile not drawn. Four things earn an
//! omission, each recorded with its reason on [`Dashboard::omitted`] and
//! written into the emitted spec as a comment: a column holding one distinct
//! value (its histogram is one bar and its ranking one row), a column holding
//! no values at all, a column whose meaning is to identify rather than to
//! describe, and a column no kind in this build can draw — which is where a
//! free-text column too wide to read as a category lands, by way of
//! [`crate::chart_kinds::fields_of`]'s own ceiling.
//!
//! # One selection, every tile
//!
//! Every tile drives and reads one `select: crossfilter` param named
//! [`SELECTION`], so a brush on any tile reaches every other tile's query. How
//! it reaches them is the receiving kind's business rather than this module's:
//! a histogram consumes through `filterBy:` and narrows, while a ranked-bars
//! module consumes through `select: highlight` and keeps its total behind the
//! subset — see [`crate::ranked_bars`], whose header says what a `filterBy:`
//! there would cost.
//!
//! # Why the tile forms live here and not on the kinds
//!
//! A [`ChartKind`]'s builder emits a **self-contained top-level document
//! fragment** over one table — that is the contract [`crate::chart_kinds`]
//! states, and it is what the chart pane's module route rebuilds. A tile is a
//! different thing: an entry in a concat list, at an indent, sharing one
//! declared selection with its siblings. The two forms coincide for a dashboard
//! of one tile and not otherwise.
//!
//! [`crate::ranked_bars::RankedCategoryBars::plot_yaml`] is the tile form of
//! its kind and is used here verbatim. The other kinds have no such method, so
//! their tile form is the private `tile_form` below — keyed by kind id, and
//! `every_kind_one_column_can_fill_has_a_tile_form` is what stops a kind added
//! later from being silently skipped.

use std::fmt::Write as _;
use std::path::Path;

use brightfield_engine::{ColumnProfile, SemanticType};
use brightfield_workbench::registry::{ChartKind, ChartKindId, Field, FieldType};

use crate::chart_kinds;
use crate::data_file::{file_label, source_spec, SOURCE};
use crate::ranked_bars::RankedCategoryBars;

/// One tile's width in logical points.
///
/// The plots flex — [`brightfield_spec::layout`] distributes a concat's offered
/// box across its items in proportion to their declared sizes — so this is a
/// weight and an aspect ratio rather than a pixel count the reader is stuck
/// with. It is [`RankedCategoryBars`]'s own default, so a dashboard of one
/// ranking is the size that module already ships at.
pub const TILE_WIDTH: u32 = 360;
/// One tile's height in logical points. See [`TILE_WIDTH`].
pub const TILE_HEIGHT: u32 = 300;

/// How many tiles stand in a row before the next row starts.
///
/// A legibility judgement, not a measured one: three 360-point tiles is a
/// 1080-point row, which is the width a laptop panel gives a docked pane
/// without the reflow squeezing each tile below the width its axis labels
/// need.
pub const TILES_PER_ROW: usize = 3;

/// The crossfilter param every tile drives and reads.
///
/// One name for the whole dashboard, declared once under `params:`. An
/// interactor binding a name no `params:` entry declares raises
/// [`brightfield_spec::ParseWarning::InteractorBindingMissing`], which the
/// window puts on screen as a *"had no effect"* banner over the picture — so
/// the declaration is not decoration.
pub const SELECTION: &str = "sel";

// ---------------------------------------------------------------------------
// What a column is for
// ---------------------------------------------------------------------------

/// What a column is *for*, as far as choosing a picture goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnRole {
    /// A measured quantity: it has a distribution worth binning.
    Measure,
    /// A member of a set: it has a ranking worth counting.
    Category,
    /// Neither — a value whose job is to *identify* a thing (an LEI, a hash, a
    /// URL, an email address) or to hold a document (a JSON blob, a delimited
    /// list). Counting it counts rows, and binning it bins an accession
    /// sequence; both are true pictures of nothing.
    Neither,
}

/// The role a semantic label implies, or `None` for a label this build has no
/// rule for.
///
/// Read on the label's **namespace and family** — its first two dotted
/// segments — with the leaves that disagree with their family written out. That
/// is deliberate: the classifier's taxonomy is versioned outside this
/// repository and gains leaves, so a rule keyed on families degrades to
/// `None` (and therefore to the storage type) on a leaf nobody here has heard
/// of, rather than to a wrong picture.
///
/// The families, and why each falls where it does:
///
/// - **`identity`** identifies people and things, so it draws nothing — except
///   a person's height and weight, which are measurements, and the small closed
///   sets (gender, blood type), which are categories.
/// - **`finance`** splits: a currency `amount` and a `rate` are measures, a
///   currency code is a category, and every security, bank and payment
///   identifier draws nothing.
/// - **`geography`** is categorical — a country, a region, a city, a postal
///   code — except a latitude and a longitude, which are measures, and the
///   packed coordinate encodings (geohash, MGRS, WKT), which identify a place
///   rather than describing one.
/// - **`datetime`** is categorical: a date is a perfectly good band axis, and
///   the bin arithmetic cannot take a logarithm of an interval. A timestamp
///   wide enough to be unreadable as a category is dropped by the cardinality
///   ceiling rather than by a rule here.
/// - **`representation`** is where the numbers live: `numeric` and a file size
///   are measures; booleans, ordinals, file extensions and mime types are
///   categories; an identifier, a colour literal, a chemical or biological
///   sequence and free prose are none of it. A single `word` or an entity name
///   is a category.
/// - **`technology`** identifies: hosts, URLs, hashes, tokens, paths. The three
///   closed sets it carries — an HTTP method, a top-level domain, a locale —
///   are categories.
/// - **`container`** holds a document per cell, so it draws nothing.
#[must_use]
pub fn role_of_label(label: &str) -> Option<ColumnRole> {
    let mut parts = label.split('.');
    let namespace = parts.next()?;
    let family = parts.next()?;
    let leaf = parts.next().unwrap_or_default();
    Some(match (namespace, family, leaf) {
        ("identity", "person", "height" | "weight") => ColumnRole::Measure,
        ("identity", "person", "gender" | "gender_code" | "blood_type") => ColumnRole::Category,
        ("identity", _, _) => ColumnRole::Neither,

        ("finance", "currency", leaf) if leaf.starts_with("amount") => ColumnRole::Measure,
        ("finance", "currency", _) => ColumnRole::Category,
        ("finance", "rate", _) => ColumnRole::Measure,
        ("finance", _, _) => ColumnRole::Neither,

        ("geography", "coordinate", "latitude" | "longitude") => ColumnRole::Measure,
        ("geography", "coordinate" | "format" | "index", _) => ColumnRole::Neither,
        ("geography", _, _) => ColumnRole::Category,

        ("datetime", _, _) => ColumnRole::Category,

        ("representation", "numeric", _) => ColumnRole::Measure,
        ("representation", "file", "file_size") => ColumnRole::Measure,
        ("representation", "file", _) => ColumnRole::Category,
        ("representation", "boolean" | "discrete", _) => ColumnRole::Category,
        ("representation", "scientific", "measurement_unit") => ColumnRole::Category,
        ("representation", "text", "word" | "entity_name") => ColumnRole::Category,
        ("representation", _, _) => ColumnRole::Neither,

        ("technology", "internet", "http_method" | "top_level_domain") => ColumnRole::Category,
        ("technology", "code", "locale_code") => ColumnRole::Category,
        ("technology", _, _) => ColumnRole::Neither,

        ("container", _, _) => ColumnRole::Neither,

        _ => return None,
    })
}

/// The role a profiled column's semantic type implies, and the label it came
/// from.
///
/// `None` for the four states that are not a trusted label: nobody was asked,
/// the source could not answer, it answered *unlabelled* — and
/// [`SemanticType::Unusable`], where a label came back and the column's own
/// values contradict it. The last is the one worth stating: a label the values
/// fail is not weak evidence about the column, it is evidence about the
/// classifier, so the storage type takes the decision back.
#[must_use]
pub fn role_of(semantic: &SemanticType) -> Option<(&str, ColumnRole)> {
    match semantic {
        SemanticType::Labelled { label, .. } => role_of_label(label).map(|role| (&**label, role)),
        SemanticType::NotAsked
        | SemanticType::Unanswered { .. }
        | SemanticType::Unlabelled
        | SemanticType::Unusable { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Columns → tiles
// ---------------------------------------------------------------------------

/// Where a tile's field type came from — what the emitted spec's comment says,
/// so the rule is legible in the artefact and not only in this file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChosenBy {
    /// The column's semantic label, and the role [`role_of_label`] gave it.
    Meaning {
        /// The label the classifier returned.
        label: String,
        /// What this build takes that label to be for.
        role: ColumnRole,
    },
    /// No trusted label, so the DuckDB type decided.
    Storage {
        /// The DuckDB type name the profile carried.
        type_name: String,
    },
}

/// One tile of a generated dashboard: a column, the kind drawn over it, and why
/// that kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    kind: ChartKindId,
    field: Field,
    block: String,
    chosen_by: ChosenBy,
}

impl Tile {
    /// Which chart kind draws this tile.
    #[must_use]
    pub const fn kind(&self) -> ChartKindId {
        self.kind
    }

    /// The column this tile is of, and what the chooser decided it holds.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// The column's name.
    #[must_use]
    pub fn column(&self) -> &str {
        &self.field.name
    }

    /// What decided this tile's field type.
    #[must_use]
    pub const fn chosen_by(&self) -> &ChosenBy {
        &self.chosen_by
    }

    /// The **kind's own** standalone block for this column — the document
    /// fragment [`ChartKind::spec`] builds, which is what the chart pane's
    /// module route rebuilds and compares against.
    ///
    /// Not the tile's YAML: see this module's header for why the two forms
    /// differ. They describe the same picture over the same column.
    #[must_use]
    pub fn block(&self) -> &str {
        &self.block
    }
}

/// Why a column got no tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Omission {
    /// One distinct value: a histogram of one bar, a ranking of one row.
    OneValue,
    /// No non-null value at all.
    NoValues,
    /// The column identifies rather than describes — see
    /// [`ColumnRole::Neither`].
    Identifies {
        /// The label that said so.
        label: String,
    },
    /// Nothing in this build draws it: a category too wide to read, a name that
    /// cannot be written into the emitted SQL, or a type no kind's slot takes.
    NoPicture,
}

impl std::fmt::Display for Omission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OneValue => f.write_str("one distinct value, so every picture of it is one bar"),
            Self::NoValues => f.write_str("no values at all"),
            Self::Identifies { label } => {
                write!(f, "{label} — it identifies rather than describes")
            }
            Self::NoPicture => f.write_str("no chart in this build fits it"),
        }
    }
}

/// A column this dashboard left out, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Omitted {
    /// The column's name, as the table spells it.
    pub column: String,
    /// Why it got no tile.
    pub because: Omission,
}

/// The dashboard a table with no spec opens as.
///
/// Built by [`Dashboard::of`], emitted by [`Dashboard::to_spec`], and carrying
/// the omissions so the reader can be told what was left out rather than
/// wondering.
#[derive(Clone, Debug)]
pub struct Dashboard {
    path: std::path::PathBuf,
    tiles: Vec<Tile>,
    omitted: Vec<Omitted>,
}

impl Dashboard {
    /// Walk `columns` and choose a tile for each, over the file at `path`.
    ///
    /// The walk is in the table's own column order, so the dashboard reads in
    /// the order the file does. Every column ends in exactly one of the two
    /// lists — [`Self::tiles`] or [`Self::omitted`] — which is what
    /// `every_column_is_either_a_tile_or_an_omission` holds.
    #[must_use]
    pub fn of(path: &Path, columns: &[ColumnProfile]) -> Self {
        let mut tiles = Vec::new();
        let mut omitted = Vec::new();
        for column in columns {
            match tile_for(column) {
                Ok(tile) => tiles.push(tile),
                Err(because) => omitted.push(Omitted {
                    column: column.name.clone(),
                    because,
                }),
            }
        }
        Self {
            path: path.to_path_buf(),
            tiles,
            omitted,
        }
    }

    /// The tiles, in the table's own column order.
    #[must_use]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// The columns that got no tile, and why.
    #[must_use]
    pub fn omitted(&self) -> &[Omitted] {
        &self.omitted
    }

    /// The one tile this dashboard has, when it has exactly one.
    ///
    /// The single-tile dashboard is the case where a tile's picture *is* the
    /// document's picture, which is what lets the chart pane host it through
    /// that kind's module. See [`crate::app::Authored`].
    #[must_use]
    pub fn sole_tile(&self) -> Option<&Tile> {
        match self.tiles.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// The whole dashboard as spec source: the header comment, the title, the
    /// shared selection, the file as the one data source, and the tiles laid
    /// out in rows of [`TILES_PER_ROW`].
    ///
    /// **This is the artefact, not a rendering of one.** The picture is
    /// composed from these exact bytes, so a reader who opens the spec is
    /// reading what ran — the same property the filter readout has, one level
    /// up.
    #[must_use]
    pub fn to_spec(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.preamble());
        let _ = writeln!(out, "meta:");
        let _ = writeln!(out, "  title: {}", yaml_string(&file_label(&self.path)));
        let _ = writeln!(out, "params:");
        let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
        out.push_str(&source_spec(&self.path));
        let _ = writeln!(out, "vconcat:");
        for row in self.tiles.chunks(TILES_PER_ROW) {
            let _ = writeln!(out, "  - hconcat:");
            for tile in row {
                let _ = writeln!(out, "    # {}", tile_comment(tile));
                out.push_str(&tile_yaml(tile, 4));
            }
        }
        out
    }

    /// The comment block above the spec: what chose this dashboard, and what it
    /// left out.
    ///
    /// The omissions are here rather than nowhere because a column silently
    /// missing from a generated analysis is indistinguishable from a bug in the
    /// generator, and the reader is the only one who can tell which.
    fn preamble(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# Brightfield wrote this dashboard by walking {}'s own columns: one",
            file_label(&self.path)
        );
        let _ = writeln!(
            out,
            "# tile per column, each chosen from what that column holds, and every"
        );
        let _ = writeln!(
            out,
            "# tile brushing into one shared crossfilter selection. Nobody authored"
        );
        let _ = writeln!(
            out,
            "# it — and it is an ordinary spec, so change anything in it."
        );
        if !self.omitted.is_empty() {
            let _ = writeln!(out, "#");
            let _ = writeln!(out, "# Columns with no tile, and why:");
            for left in &self.omitted {
                let _ = writeln!(out, "#   {}: {}", left.column, left.because);
            }
        }
        out
    }
}

/// The tile `column` gets, or why it gets none.
///
/// Three steps. The column becomes a field, which is where the eligibility
/// rules and the DuckDB-type mapping are applied — through
/// [`chart_kinds::fields_of`], so this module holds no second copy of either.
/// The semantic label then gets to overrule the field's type, or to refuse the
/// column outright. Finally the registry is asked which of the kinds a lone
/// column can fill takes that field, and the first one builds.
fn tile_for(column: &ColumnProfile) -> Result<Tile, Omission> {
    if column.non_null == 0 {
        return Err(Omission::NoValues);
    }
    if column.distinct <= 1 {
        return Err(Omission::OneValue);
    }
    let (field, chosen_by) = match role_of(&column.semantic) {
        Some((label, ColumnRole::Neither)) => {
            return Err(Omission::Identifies {
                label: label.to_string(),
            })
        }
        Some((label, role)) => (
            field_as(column, role).ok_or(Omission::NoPicture)?,
            ChosenBy::Meaning {
                label: label.to_string(),
                role,
            },
        ),
        None => (
            field_of(column).ok_or(Omission::NoPicture)?,
            ChosenBy::Storage {
                type_name: column.type_name.clone(),
            },
        ),
    };
    let kind = kind_for(&field).ok_or(Omission::NoPicture)?;
    let binding = kind.bind(std::slice::from_ref(&field)).ok();
    let block = binding
        .and_then(|b| kind.spec(&b, &kind.options()).ok())
        .ok_or(Omission::NoPicture)?;
    Ok(Tile {
        kind: kind.id,
        field,
        block,
        chosen_by,
    })
}

/// The field `column` offers on its DuckDB type alone.
///
/// One column in, at most one field out: [`chart_kinds::fields_of`] answers
/// with an empty list for a column it will not offer to any kind, and that is
/// the whole of the eligibility rule — a name that cannot be written into the
/// emitted SQL, or a category too wide to read.
fn field_of(column: &ColumnProfile) -> Option<Field> {
    chart_kinds::fields_of(std::slice::from_ref(column))
        .into_iter()
        .next()
}

/// The field `column` offers when its **meaning** says it is a `role`.
///
/// The eligibility rules still apply, and they are still
/// [`chart_kinds::fields_of`]'s: the column is re-offered under the DuckDB type
/// that carries the role's field type, and the answer comes back through the
/// same door. That is why a `VARCHAR` labelled a measure stays a category —
/// `fields_of` will not call a string binnable, and it is right not to, since
/// the bin scheme is arithmetic — and why a numeric column labelled a category
/// is still refused when it holds more distinct values than a band axis can
/// show. The ceiling that decides *that* is private to `chart_kinds`, and
/// asking through this door is how this module honours it without holding a
/// second copy of the number.
fn field_as(column: &ColumnProfile, role: ColumnRole) -> Option<Field> {
    let wanted = match role {
        ColumnRole::Measure => FieldType::Quantitative,
        ColumnRole::Category => FieldType::Categorical,
        // `Neither` never reaches here: `tile_for` refuses the column first.
        ColumnRole::Neither => return None,
    };
    let restated = ColumnProfile {
        type_name: match wanted {
            FieldType::Quantitative => "DOUBLE".to_string(),
            _ => "VARCHAR".to_string(),
        },
        ..column.clone()
    };
    let offered = chart_kinds::fields_of(std::slice::from_ref(&restated))
        .into_iter()
        .next()?;
    // The role only gets its way where the storage type agrees it could: a
    // measure DuckDB stored as text has no bin arithmetic, so it falls back to
    // what the column's own type offers.
    if offered.ty == wanted && (wanted != FieldType::Quantitative || binnable(column)) {
        Some(offered)
    } else {
        field_of(column)
    }
}

/// Whether the column's own DuckDB type is one `chart_kinds` will bin.
///
/// Asked by offering the column as itself and reading back the field type,
/// rather than by restating the type list — the list is private to
/// `chart_kinds` and is the arithmetic's own business.
fn binnable(column: &ColumnProfile) -> bool {
    field_of(column).is_some_and(|f| f.ty == FieldType::Quantitative)
}

/// The registry kinds a **single** column can fill, in declaration order.
///
/// "Fill" means the required slots: exactly one of them, since a lone column
/// fills one slot. `count-grid` declares two required categorical slots and a
/// scatter would declare two quantitative ones, so neither is ever a per-column
/// tile — which is the whole reason this is not
/// [`brightfield_workbench::registry::ChartKindRegistry::applicable`] over the
/// table's fields.
#[must_use]
pub fn single_column_kinds() -> Vec<&'static ChartKind<String>> {
    chart_kinds::registry()
        .kinds()
        .iter()
        .filter(|kind| kind.slots.iter().filter(|slot| slot.required).count() == 1)
        .filter(|kind| tile_form(kind.id).is_some())
        .collect()
}

/// The kind that draws `field`: the first of [`single_column_kinds`] whose
/// slots take it. Declaration order in the registry is therefore the
/// preference, stated there rather than here.
fn kind_for(field: &Field) -> Option<&'static ChartKind<String>> {
    single_column_kinds()
        .into_iter()
        .find(|kind| kind.accepts(std::slice::from_ref(field)))
}

// ---------------------------------------------------------------------------
// Tiles → YAML
// ---------------------------------------------------------------------------

/// How a kind is written as a tile — an entry in a concat list, at an indent,
/// over [`SOURCE`] and [`SELECTION`].
///
/// `None` for a kind with no tile form, which is how a kind that is not a
/// per-column picture stays out of [`single_column_kinds`]. Adding a one-column
/// kind and no arm here reddens
/// `every_kind_one_column_can_fill_has_a_tile_form`.
fn tile_form(kind: ChartKindId) -> Option<TileForm> {
    if kind == chart_kinds::BINNED_HISTOGRAM {
        Some(TileForm::Histogram)
    } else if kind == crate::ranked_bars::KIND_ID {
        Some(TileForm::RankedBars)
    } else {
        None
    }
}

/// The shapes a tile is written in. One per kind that can be one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileForm {
    /// A binned count, brushed with an `intervalX`.
    Histogram,
    /// [`RankedCategoryBars`], whose own emitter writes it.
    RankedBars,
}

/// `tile` as one entry of a concat list, indented by `indent` spaces.
fn tile_yaml(tile: &Tile, indent: usize) -> String {
    match tile_form(tile.kind) {
        Some(TileForm::Histogram) => histogram_tile(tile.column(), indent),
        Some(TileForm::RankedBars) => RankedCategoryBars::new(tile.column())
            .with_size(TILE_WIDTH, TILE_HEIGHT)
            .plot_yaml(SOURCE, SELECTION, indent),
        // Unreachable: a tile's kind came from `single_column_kinds`, which
        // admits only kinds with a form. Emitting nothing rather than a
        // half-written plot is the answer that cannot produce a spec which
        // parses and draws something else.
        None => String::new(),
    }
}

/// A measure's distribution, brushable.
///
/// `xDomain: Fixed` is not decoration: under a crossfilter the bin edges would
/// otherwise be re-derived from whatever the *other* tiles left, so the bars
/// would move sideways under the pointer and two frames of the same column
/// would not be comparable. It pins the frame of reference the reader is
/// measuring against, which is the same job the ghost layer does on the count
/// axis.
fn histogram_tile(column: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let col = yaml_string(column);
    let mut out = String::new();
    let _ = writeln!(out, "{pad}- plot:");
    let _ = writeln!(out, "{pad}  - mark: rectY");
    let _ = writeln!(
        out,
        "{pad}    data: {{ from: {}, filterBy: ${SELECTION} }}",
        yaml_string(SOURCE)
    );
    let _ = writeln!(out, "{pad}    x: {{ bin: {col} }}");
    let _ = writeln!(out, "{pad}    y: {{ count: }}");
    // The producer: dragging an x-range publishes it into the shared selection.
    let _ = writeln!(out, "{pad}  - select: intervalX");
    let _ = writeln!(out, "{pad}    as: ${SELECTION}");
    // Plot attributes are siblings of `plot:`, so they sit at its indent — one
    // level deeper and they read as more options on the last interactor, which
    // is a spec that parses and does something else.
    let _ = writeln!(out, "{pad}  xDomain: Fixed");
    let _ = writeln!(out, "{pad}  xLabel: {col}");
    let _ = writeln!(out, "{pad}  width: {TILE_WIDTH}");
    let _ = writeln!(out, "{pad}  height: {TILE_HEIGHT}");
    out
}

/// The sentence written above a tile in the emitted spec: the column, what
/// decided its type, and the kind that came back.
///
/// **This is where AC-level "the rule is stated somewhere a reader can check
/// it" is discharged in the artefact.** A reader with the spec in front of them
/// can see that `amount` was binned because a label called it a currency
/// amount, and that `region` was ranked because nothing labelled it and DuckDB
/// stored it as text.
fn tile_comment(tile: &Tile) -> String {
    let because = match tile.chosen_by() {
        ChosenBy::Meaning { label, role } => {
            format!(
                "{label} is {}",
                match role {
                    ColumnRole::Measure => "a measure",
                    ColumnRole::Category => "a category",
                    ColumnRole::Neither => "not drawable",
                }
            )
        }
        ChosenBy::Storage { type_name } => {
            format!("no semantic label, and DuckDB stored it as {type_name}")
        }
    };
    format!("{}: {because} → {}", tile.column(), tile.kind())
}

/// A YAML scalar that cannot be read as anything but a string.
///
/// JSON string syntax is a subset of YAML's double-quoted scalar syntax, so
/// `serde_json` is a correct quoter here — and it is what keeps a column named
/// `y`, `on` or `12:00` from arriving as a boolean, a number or a sexagesimal.
fn yaml_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
        ColumnProfile {
            name: name.to_string(),
            type_name: type_name.to_string(),
            non_null: 100,
            nulls: 0,
            distinct,
            min: None,
            max: None,
            semantic: SemanticType::NotAsked,
        }
    }

    fn labelled(name: &str, type_name: &str, distinct: u64, label: &str) -> ColumnProfile {
        ColumnProfile {
            semantic: SemanticType::Labelled {
                label: label.to_string(),
                confidence: 0.99,
                check: brightfield_engine::ValueCheck::Checked {
                    checked: 100,
                    failed: 0,
                },
            },
            ..column(name, type_name, distinct)
        }
    }

    fn of(columns: &[ColumnProfile]) -> Dashboard {
        Dashboard::of(Path::new("/tmp/readings.csv"), columns)
    }

    /// **A tile per column** — the property this module exists for, over a
    /// table whose columns are of three different shapes.
    #[test]
    fn every_usable_column_gets_its_own_tile() {
        let dash = of(&[
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
            column("day", "DATE", 30),
            column("weight", "BIGINT", 400),
        ]);
        let columns: Vec<&str> = dash.tiles().iter().map(Tile::column).collect();
        assert_eq!(columns, vec!["amount", "region", "day", "weight"]);
        assert!(dash.omitted().is_empty(), "{:?}", dash.omitted());
    }

    /// **The count grid can never be a per-column tile**, and the reason is its
    /// slot declaration rather than a name checked here: two required slots
    /// cannot be filled by one column.
    ///
    /// The table below is exactly the one that would go wrong if this chooser
    /// asked whether a kind accepts the *table*: two categorical columns admit
    /// `count-grid`, which would then swallow both and leave the third column
    /// undrawn.
    #[test]
    fn a_kind_needing_two_columns_is_never_a_tile() {
        let dash = of(&[
            column("tier", "VARCHAR", 3),
            column("method", "VARCHAR", 4),
            column("region", "VARCHAR", 6),
        ]);
        assert_eq!(dash.tiles().len(), 3, "{dash:?}");
        for tile in dash.tiles() {
            assert_ne!(
                tile.kind(),
                chart_kinds::COUNT_GRID,
                "{}: a two-slot kind was chosen for one column",
                tile.column()
            );
        }
        assert!(!single_column_kinds()
            .iter()
            .any(|kind| kind.id == chart_kinds::COUNT_GRID));
    }

    /// **Every kind a lone column can fill has a tile form.** A kind added to
    /// the registry with one required slot and no arm in [`tile_form`] would be
    /// chosen for no column and nothing else would say so.
    #[test]
    fn every_kind_one_column_can_fill_has_a_tile_form() {
        for kind in chart_kinds::registry().kinds() {
            let required = kind.slots.iter().filter(|s| s.required).count();
            if required != 1 {
                continue;
            }
            assert!(
                tile_form(kind.id).is_some(),
                "{}: one column fills this kind's required slot, so it can be a \
                 per-column tile — but no tile form is written for it, so the \
                 generator will never choose it",
                kind.id
            );
        }
    }

    /// **The semantic type decides, not the storage type.** Two `BIGINT`
    /// columns, identical to DuckDB: one labelled a currency amount is binned,
    /// one labelled a securities identifier is left out.
    #[test]
    fn two_identical_bigints_get_different_answers_from_their_meaning() {
        let dash = of(&[
            labelled("amount", "BIGINT", 900, "finance.currency.amount"),
            labelled("lei", "BIGINT", 900, "finance.securities.lei"),
        ]);
        assert_eq!(dash.tiles().len(), 1, "{dash:?}");
        let tile = &dash.tiles()[0];
        assert_eq!(tile.column(), "amount");
        assert_eq!(tile.kind(), chart_kinds::BINNED_HISTOGRAM);
        assert_eq!(
            dash.omitted(),
            &[Omitted {
                column: "lei".to_string(),
                because: Omission::Identifies {
                    label: "finance.securities.lei".to_string()
                },
            }]
        );
    }

    /// A numeric column labelled a **category** is ranked rather than binned —
    /// a year, a quarter, a gender code. The storage type would have binned it.
    #[test]
    fn a_number_labelled_a_category_is_ranked_not_binned() {
        let dash = of(&[labelled("year", "BIGINT", 12, "datetime.component.year")]);
        let tile = &dash.tiles()[0];
        assert_eq!(tile.kind(), crate::ranked_bars::KIND_ID);
        assert_eq!(tile.field().ty, FieldType::Categorical);
        // …and without the label the same column is a measure.
        let bare = of(&[column("year", "BIGINT", 12)]);
        assert_eq!(bare.tiles()[0].kind(), chart_kinds::BINNED_HISTOGRAM);
    }

    /// A measure the storage cannot bin keeps the answer its type gives: the
    /// bin scheme is arithmetic and a `VARCHAR` has none, so `'$1,200.00'` is
    /// ranked rather than binned.
    #[test]
    fn a_measure_stored_as_text_is_not_binned() {
        let dash = of(&[labelled(
            "price",
            "VARCHAR",
            9,
            "finance.currency.amount_comma",
        )]);
        assert_eq!(dash.tiles()[0].kind(), crate::ranked_bars::KIND_ID);
    }

    /// **A label the column's own values contradict is not believed.** An
    /// `Unusable` verdict falls back to the storage type rather than acting on
    /// the label, so a `DOUBLE` mislabelled an email address is still binned.
    #[test]
    fn a_label_the_values_contradict_falls_back_to_the_storage_type() {
        let mut mislabelled = column("amount", "DOUBLE", 900);
        mislabelled.semantic = SemanticType::Unusable {
            label: "identity.person.email".to_string(),
            confidence: 0.98,
            checked: 100,
            failed: 100,
            first_failure: None,
        };
        let dash = of(&[mislabelled]);
        assert_eq!(dash.tiles()[0].kind(), chart_kinds::BINNED_HISTOGRAM);
        assert!(matches!(
            dash.tiles()[0].chosen_by(),
            ChosenBy::Storage { .. }
        ));
    }

    /// **The rule, arm by arm.** One representative label per branch of
    /// [`role_of_label`], so a re-write of the match that moved a family
    /// reddens here.
    #[test]
    fn the_rule_answers_these_labels_these_ways() {
        for (label, role) in [
            ("identity.person.height", ColumnRole::Measure),
            ("identity.person.gender", ColumnRole::Category),
            ("identity.person.email", ColumnRole::Neither),
            ("identity.government.ssn", ColumnRole::Neither),
            ("finance.currency.amount", ColumnRole::Measure),
            ("finance.currency.amount_comma", ColumnRole::Measure),
            ("finance.currency.currency_code", ColumnRole::Category),
            ("finance.rate.yield", ColumnRole::Measure),
            ("finance.securities.isin", ColumnRole::Neither),
            ("finance.banking.iban", ColumnRole::Neither),
            ("geography.coordinate.latitude", ColumnRole::Measure),
            ("geography.coordinate.geohash", ColumnRole::Neither),
            ("geography.location.country", ColumnRole::Category),
            ("geography.address.postal_code", ColumnRole::Category),
            ("datetime.date.iso", ColumnRole::Category),
            ("datetime.period.quarter", ColumnRole::Category),
            ("representation.numeric.percentage", ColumnRole::Measure),
            ("representation.file.file_size", ColumnRole::Measure),
            ("representation.file.mime_type", ColumnRole::Category),
            ("representation.boolean.terms", ColumnRole::Category),
            ("representation.discrete.ordinal", ColumnRole::Category),
            ("representation.text.word", ColumnRole::Category),
            ("representation.text.plain_text", ColumnRole::Neither),
            ("representation.identifier.uuid", ColumnRole::Neither),
            ("technology.internet.http_method", ColumnRole::Category),
            ("technology.internet.url", ColumnRole::Neither),
            ("technology.cryptographic.hash", ColumnRole::Neither),
            ("container.object.json", ColumnRole::Neither),
        ] {
            assert_eq!(role_of_label(label), Some(role), "{label}");
        }
        // A namespace this build has no rule for answers nothing, which leaves
        // the storage type in charge rather than guessing.
        assert_eq!(role_of_label("astronomy.star.magnitude"), None);
        assert_eq!(role_of_label("unqualified"), None);
    }

    /// **A column that would draw badly is left out, with its reason.** One
    /// distinct value, no values, a name that cannot be written, and a
    /// free-text column too wide to read as a category.
    #[test]
    fn a_column_that_would_draw_badly_is_omitted_with_a_reason() {
        let mut empty = column("spare", "VARCHAR", 4);
        empty.non_null = 0;
        empty.nulls = 100;
        let dash = of(&[
            column("version", "BIGINT", 1),
            empty,
            column("we\"ird", "DOUBLE", 900),
            column("notes", "VARCHAR", 900),
            column("amount", "DOUBLE", 900),
        ]);
        assert_eq!(
            dash.tiles().iter().map(Tile::column).collect::<Vec<_>>(),
            vec!["amount"]
        );
        let left: Vec<(&str, &Omission)> = dash
            .omitted()
            .iter()
            .map(|o| (o.column.as_str(), &o.because))
            .collect();
        assert_eq!(
            left,
            vec![
                ("version", &Omission::OneValue),
                ("spare", &Omission::NoValues),
                ("we\"ird", &Omission::NoPicture),
                ("notes", &Omission::NoPicture),
            ]
        );
    }

    /// Every column ends in exactly one of the two lists, so a reader counting
    /// tiles and omissions accounts for the whole table.
    #[test]
    fn every_column_is_either_a_tile_or_an_omission() {
        let columns = [
            column("amount", "DOUBLE", 900),
            column("version", "BIGINT", 1),
            column("region", "VARCHAR", 4),
            labelled("id", "VARCHAR", 900, "representation.identifier.uuid"),
        ];
        let dash = of(&columns);
        assert_eq!(dash.tiles().len() + dash.omitted().len(), columns.len());
        let named: Vec<&str> = dash
            .tiles()
            .iter()
            .map(Tile::column)
            .chain(dash.omitted().iter().map(|o| o.column.as_str()))
            .collect();
        for c in &columns {
            assert!(named.contains(&c.name.as_str()), "{} vanished", c.name);
        }
    }

    /// **The generated spec parses, and every key it writes has a reader.**
    ///
    /// Not "it parses" — a spec full of options nothing reads also parses.
    /// `Unconsumed…` names a key that reaches no lowerer and no renderer, and a
    /// generator emitting one ships a chart that silently ignores half its own
    /// instructions.
    #[test]
    fn every_key_the_generated_spec_writes_has_a_reader() {
        let dash = of(&[
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
            column("day", "DATE", 30),
            column("tier", "VARCHAR", 3),
        ]);
        let source = dash.to_spec();
        let parsed = parse_spec(&source, Format::Yaml)
            .unwrap_or_else(|e| panic!("the generated spec does not parse: {e}\n{source}"));
        let warnings: Vec<String> = parsed.warnings.iter().map(|w| format!("{w:?}")).collect();
        assert!(
            warnings.is_empty(),
            "the generated spec warns: {warnings:?}\n{source}"
        );
    }

    /// The tiles are laid out in rows of [`TILES_PER_ROW`], and the layout is
    /// the spec's own `vconcat`/`hconcat` rather than something the pane does
    /// afterwards.
    #[test]
    fn the_tiles_are_laid_out_in_rows() {
        let many: Vec<ColumnProfile> = (0..7)
            .map(|i| column(&format!("m{i}"), "DOUBLE", 900))
            .collect();
        let source = of(&many).to_spec();
        let rows = source.matches("- hconcat:").count();
        assert_eq!(
            rows, 3,
            "seven tiles at three per row is three rows:\n{source}"
        );
        assert_eq!(source.matches("- plot:").count(), 7);
        let parsed = parse_spec(&source, Format::Yaml).expect("parses");
        let root = parsed.spec.root.expect("the dashboard has a root");
        match root {
            brightfield_spec::ast::Component::VConcat(node) => {
                assert_eq!(node.items.len(), 3, "{node:?}");
            }
            other => panic!("the root of a laid-out dashboard is a vconcat: {other:?}"),
        }
    }

    /// **One selection, declared once, driven and read by every tile.** This is
    /// the crossfilter wiring, asserted on the emitted source because that is
    /// where it would be missing.
    #[test]
    fn every_tile_drives_and_reads_the_one_shared_selection() {
        let dash = of(&[
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
        ]);
        let source = dash.to_spec();
        assert_eq!(
            source
                .matches(&format!("{SELECTION}: {{ select: crossfilter }}"))
                .count(),
            1,
            "the selection is declared exactly once:\n{source}"
        );
        // Each tile publishes into it…
        assert_eq!(
            source.matches(&format!("as: ${SELECTION}")).count(),
            2,
            "every tile has to drive the selection:\n{source}"
        );
        // …and each consumes it, in the form its own kind consumes it in: the
        // histogram narrows, the ranking highlights (see `ranked_bars`).
        assert!(
            source.contains(&format!("filterBy: ${SELECTION}")),
            "{source}"
        );
        assert!(source.contains(&format!("by: ${SELECTION}")), "{source}");
    }

    /// The bin domain is pinned, so a filtered histogram is measured against
    /// the frame it had before the brush.
    #[test]
    fn a_histogram_tile_pins_its_bin_domain() {
        let source = of(&[column("amount", "DOUBLE", 900)]).to_spec();
        assert!(source.contains("xDomain: Fixed"), "{source}");
    }

    /// **The spec says why each tile is the tile it is**, and what it left out.
    /// The rule is checkable by a reader holding the artefact, not only by one
    /// holding this file.
    #[test]
    fn the_spec_states_the_rule_it_was_generated_by() {
        let dash = of(&[
            labelled("amount", "BIGINT", 900, "finance.currency.amount"),
            labelled("lei", "VARCHAR", 900, "finance.securities.lei"),
            column("region", "VARCHAR", 4),
        ]);
        let source = dash.to_spec();
        assert!(
            source.contains("# amount: finance.currency.amount is a measure → binned-histogram"),
            "{source}"
        );
        assert!(
            source.contains("# region: no semantic label, and DuckDB stored it as VARCHAR → ranked-category-bars"),
            "{source}"
        );
        assert!(
            source.contains("lei: finance.securities.lei — it identifies rather than describes"),
            "the omission and its reason belong in the spec the reader opens:\n{source}"
        );
    }

    /// A column name YAML would read as something other than a string survives
    /// into the spec, and the spec still parses.
    #[test]
    fn a_column_name_yaml_would_misread_survives() {
        for name in ["y", "on", "no", "12:00", "1.5", "null"] {
            let source = of(&[column(name, "DOUBLE", 900), column("tag", "VARCHAR", 4)]).to_spec();
            assert!(
                source.contains(&format!("bin: \"{name}\"")),
                "{name} was written bare:\n{source}"
            );
            assert!(
                parse_spec(&source, Format::Yaml).is_ok(),
                "{name} does not parse:\n{source}"
            );
        }
    }

    /// A path or a title carrying an apostrophe survives into the spec, and
    /// round-trips to the bytes it started as.
    #[test]
    fn an_apostrophe_in_the_path_survives_into_the_spec() {
        let dash = Dashboard::of(
            Path::new("/Users/hugh/Hugh's data.csv"),
            &[column("it's", "DOUBLE", 900)],
        );
        let source = dash.to_spec();
        let parsed = parse_spec(&source, Format::Yaml).expect("the generated spec parses");
        let declared = parsed
            .spec
            .data
            .get(SOURCE)
            .expect("the file is declared under the fixed key");
        assert_eq!(
            declared.kind,
            brightfield_spec::ast::DataSourceKind::File("/Users/hugh/Hugh's data.csv".to_string())
        );
        assert_eq!(
            parsed.spec.meta.and_then(|m| m.title),
            Some("Hugh's data.csv".to_string())
        );
    }

    /// A table with nothing drawable in it is a dashboard with no tiles — and
    /// it still knows why, which is what the caller turns into a sentence.
    #[test]
    fn a_table_with_nothing_drawable_has_no_tiles_and_says_why() {
        let dash = of(&[column("version", "BIGINT", 1), column("id", "VARCHAR", 900)]);
        assert!(dash.tiles().is_empty());
        assert_eq!(dash.omitted().len(), 2);
        assert!(dash.sole_tile().is_none());
        assert!(Dashboard::of(Path::new("/tmp/t.csv"), &[])
            .tiles()
            .is_empty());
    }

    /// The single-tile dashboard is the one the chart pane can host through a
    /// kind's module, and the block it carries is the kind's own — what that
    /// module rebuilds and compares against.
    #[test]
    fn a_one_column_table_carries_the_kinds_own_block() {
        let dash = of(&[column("amount", "DOUBLE", 900)]);
        let tile = dash.sole_tile().expect("one usable column is one tile");
        let kind = chart_kinds::find(tile.kind()).expect("the kind is in the registry");
        let binding = kind
            .bind(std::slice::from_ref(tile.field()))
            .expect("the field binds");
        assert_eq!(
            tile.block(),
            kind.spec(&binding, &kind.options()).expect("builds"),
            "the recorded block has to be the one the module rebuilds, or the \
             pane draws nothing"
        );
        // Two tiles is not one picture, so there is no sole tile to host.
        assert!(of(&[
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4)
        ])
        .sole_tile()
        .is_none());
    }
}
