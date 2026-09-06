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
//! **A temporal column is not one of them.** That ceiling bounds how many
//! categories a reader can tell apart, and applying it to a lone column refused
//! a tile to any date past two months of daily readings — the commonest column
//! in a data file, told there was no chart in this build that fits it. A `DATE`
//! now takes [`crate::chart_kinds::COUNTS_OVER_TIME`] at any width, and the
//! ceiling is applied where it was argued for: to categories.
//!
//! A `TIMESTAMP` takes the same kind by a longer road, because it has no band
//! to stand on and a `barY` bound to one draws nothing — the test that measures
//! that is `a_timestamp_band_puts_no_ink_on_the_page`, in
//! [`crate::chart_kinds`]. [`crate::resample`] chooses the calendar step its
//! own span asks for, [`Dashboard::to_spec`] declares a bucket column at that
//! step beside the file's own, and the tile is drawn over that. The step is
//! written into the tile's comment, so the reader holding the spec can see it
//! was counted by the day rather than the hour.
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
//! declared selection with its siblings.
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
use crate::data_file::{file_label, source_spec_deriving, SOURCE};
use crate::ranked_bars::RankedCategoryBars;
use crate::resample::{self, Step};

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
///
/// The one-tile fallback in [`Dashboard::to_spec`] is what still reads it,
/// now that a dashboard of two tiles or more is a hero beside a column — see
/// [`HERO_WIDTH`], and `a_single_tile_dashboard_is_not_a_pane_group` for the
/// case that still comes through here.
pub const TILES_PER_ROW: usize = 3;

// ---------------------------------------------------------------------------
// The hero and the column beside it
// ---------------------------------------------------------------------------

/// The share of the canvas the hero takes, as a fraction of the width.
///
/// The declared widths below are this ratio written as two weights, because
/// [`brightfield_spec::layout`] shares a constrained `hconcat`'s residual out
/// in proportion to its items' intrinsic sizes. Stated as a number as well
/// because the shell splits the canvas into panes with it, and the two have to
/// be one declaration: `the_hero_takes_the_larger_share_of_the_page` lays the
/// emitted spec out and reads the hero's width back as a fraction of what the
/// gutter leaves, so a weight that stopped agreeing with this number reddens
/// there.
pub const HERO_SHARE: f32 = 0.62;

/// The share of its own **column** the map pane takes, as a fraction of the
/// height — the rows pane beneath it takes what is left.
///
/// [`HERO_SHARE`] again, and the tie is the design rather than a coincidence:
/// the map is the same fraction of the canvas across and of its column down,
/// so the two panes beside and below it read as one proportion instead of two.
/// `the_rows_pane_sits_under_the_map_and_takes_the_rest_of_its_column` reads
/// the drawn rects back against it.
pub const MAP_COLUMN_SHARE: f32 = HERO_SHARE;

/// The hero plot's declared width in logical points — [`HERO_SHARE`] of a
/// thousand, so the pair of weights below reads as the ratio it is.
pub const HERO_WIDTH: u32 = 620;

/// The hero plot's declared height. Only the *unconstrained* layout reads it —
/// the size the window is first derived from — because a constrained `hconcat`
/// stretches its items to the box it was offered.
pub const HERO_HEIGHT: u32 = 620;

/// One stacked tile's declared width — the remainder of [`HERO_WIDTH`]'s
/// thousand.
pub const COLUMN_TILE_WIDTH: u32 = 380;

/// One stacked tile's declared height, unconstrained.
///
/// Small deliberately: it is the *weight* the column's tiles share their box
/// out by (all equal, so the split is even) and it is what a window derived
/// from an unconstrained layout is sized around. At [`TILE_HEIGHT`] a file of
/// nine columns would ask for a two-thousand-point window.
pub const COLUMN_TILE_HEIGHT: u32 = 100;

/// The floor a stacked tile's drawn height is held at, in logical points.
///
/// Below this a histogram is a smear rather than a distribution. The column
/// scrolls rather than compressing past it — see [`stack_extent`].
pub const MIN_COLUMN_TILE_HEIGHT: f32 = 96.0;

/// The gutter between the hero and the column, in logical points, written into
/// the emitted spec as an `hspace`.
///
/// **It is a chrome measurement, and that is deliberate.** The two panes the
/// shell draws this dashboard in each take
/// [`brightfield_workbench::chrome::pane_content_inset`] out of their own rect
/// and stand a pane gap apart, so the page the raster is composed on has to
/// carry that gutter or the hero's right edge and the column's left edge land
/// inside the other pane's frame. Its value is what makes the map pane's
/// drawn rect [`HERO_SHARE`] of the canvas exactly:
///
/// ```text
/// gutter = (2 · HERO_SHARE · pane_content_inset + pane_gap) / (1 − HERO_SHARE)
/// ```
///
/// `the_gutter_is_what_puts_the_map_pane_at_its_declared_share` derives it
/// from the chrome and reddens if either moves.
pub const HERO_GUTTER: u32 = 42;

/// The spacer under the hero, as the emitted spec declares it: **zero**.
///
/// The page is as tall as the *column* needs — [`stack_extent`] — and an
/// `hconcat` offers each item that flexes its whole height, so a hero standing
/// directly in the row is composed at the page's height rather than at the map
/// pane's. That is what put its x-axis below the pane's content rect at 1440 by
/// 900. It stands under a `vspace` instead, which does not flex: the hero takes
/// the page height less the spacer, so setting the spacer to what the page
/// overflows the pane by bounds the hero at the pane's own content height.
///
/// Zero here because the emitted source is the artefact a reader opens and the
/// overflow is not a property of the dashboard — it is a property of the window
/// it is laid out in, which is why [`crate::app::ChartDoc::reflow_to`] writes
/// the live value on each layout and [`crate::pipeline::LiveDashboard::set_hero_bound`]
/// is what receives it. A window with room for the whole column overflows by
/// nothing and leaves this at the value written here.
pub const HERO_BOUND: u32 = 0;

/// The height the composed page needs for a column of `tiles` stacked tiles,
/// given the `offered` height of the pane they stand in.
///
/// [`MIN_COLUMN_TILE_HEIGHT`] each, or the offered box shared out evenly when
/// that is the larger — the "or 96 points whichever is greater, scrolling past
/// that" rule, as one number the caller can scroll a page of.
///
/// What the page grows by is what the hero must *not* grow by: see
/// [`HERO_BOUND`].
#[must_use]
pub fn stack_extent(offered: f32, tiles: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let floor = MIN_COLUMN_TILE_HEIGHT * tiles as f32;
    offered.max(floor)
}

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
/// segments — with the leaves that disagree with their family written out.
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
/// - **`datetime`** is categorical: a period is a member of a set, and the bin
///   arithmetic cannot take a logarithm of an interval. Where the column is
///   *stored* as a date, `field_as` keeps the temporal field its type offers
///   instead, so the set's order survives — see that function.
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
    /// Two columns identified together as a coordinate pair — see this
    /// module's private `coordinate_pair`. This tile's own [`Tile::column`]
    /// is the longitude; `latitude` names the column beside it.
    CoordinatePair {
        /// The paired latitude column's name.
        latitude: String,
        /// Which test found the pair: `"label"`, `"name"`, or `"range"` — see
        /// this module's private `coordinate_pair`.
        rule: &'static str,
    },
}

/// One tile of a generated dashboard: a column, the kind drawn over it, and why
/// that kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    kind: ChartKindId,
    column: String,
    field: Field,
    block: String,
    chosen_by: ChosenBy,
    resampled: Option<Step>,
    /// The second column and field of a two-column tile — the latitude beside
    /// [`Tile::column`]'s longitude, for [`chart_kinds::POINT_MAP`]. `None`
    /// for the rest of the kinds this dashboard draws, each one column.
    paired: Option<(String, Field)>,
}

impl Tile {
    /// Which chart kind draws this tile.
    #[must_use]
    pub const fn kind(&self) -> ChartKindId {
        self.kind
    }

    /// The **drawn** column, and what the chooser decided it holds.
    ///
    /// The same column as [`Self::column`] except where [`Self::resampled`]
    /// answers: a timestamp is drawn through the bucket column derived beside
    /// it, so the field names that one and this tile is still *of* the
    /// timestamp.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// The table's own column this tile is of.
    #[must_use]
    pub fn column(&self) -> &str {
        &self.column
    }

    /// The column the tile's marks name — [`Self::field`]'s.
    #[must_use]
    pub fn drawn_column(&self) -> &str {
        &self.field.name
    }

    /// The calendar step this tile counts at, for a column that had to be
    /// resampled to have a band at all. `None` for a column drawn as it stands.
    #[must_use]
    pub const fn resampled(&self) -> Option<Step> {
        self.resampled
    }

    /// The second column of a two-column tile — the latitude beside
    /// [`Self::column`]'s longitude, for [`chart_kinds::POINT_MAP`]. `None`
    /// for the rest of the kinds this dashboard draws.
    #[must_use]
    pub fn paired_column(&self) -> Option<&str> {
        self.paired.as_ref().map(|(name, _)| name.as_str())
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
    /// Walk `columns` and choose a tile for each, over the file at `path` —
    /// except a coordinate pair, which this module's private
    /// `coordinate_pair` finds and this draws as one joint
    /// [`chart_kinds::POINT_MAP`] tile instead of two.
    ///
    /// The walk is in the table's own column order, so the dashboard reads in
    /// the order the file does — a coordinate pair's joint tile takes the
    /// position of whichever of the two columns the table declares first.
    /// Every column ends in exactly one of the two lists — [`Self::tiles`] or
    /// [`Self::omitted`] — which is what
    /// `every_column_is_either_a_tile_or_an_omission` holds for a table with
    /// no pair; a table with one accounts for both its columns in a single
    /// entry of [`Self::tiles`] instead, which
    /// `a_coordinate_pair_draws_one_tile_for_two_columns` holds.
    #[must_use]
    pub fn of(path: &Path, columns: &[ColumnProfile]) -> Self {
        let mut tiles = Vec::new();
        let mut omitted = Vec::new();
        let taken: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        let pair = coordinate_pair(columns);
        for (i, column) in columns.iter().enumerate() {
            if let Some((lon_i, lat_i, rule)) = pair {
                let anchor = lon_i.min(lat_i);
                let other = lon_i.max(lat_i);
                if i == other {
                    continue;
                }
                if i == anchor {
                    tiles.push(point_map_tile_for(&columns[lon_i], &columns[lat_i], rule));
                    continue;
                }
            }
            match tile_for(column, &taken) {
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

    /// **Which tile takes the map pane** — the hero — as an index into
    /// [`Self::tiles`], or `None` for a dashboard of fewer than two tiles,
    /// which has no column to stand a hero beside.
    ///
    /// The generator names it: the joint tile a coordinate pair earned, if the
    /// table has one, and **otherwise the generator's first tile**. That
    /// fallback is decided here rather than left to the tile order by
    /// accident, because a table with no coordinate pair still opens as a hero
    /// beside a column and something has to be the hero. The first tile is the
    /// first column of the file that earned a picture, which is the one a
    /// reader's eye is already going to.
    #[must_use]
    pub fn hero_index(&self) -> Option<usize> {
        if self.tiles.len() < 2 {
            return None;
        }
        Some(
            self.tiles
                .iter()
                .position(|t| t.kind == chart_kinds::POINT_MAP)
                .unwrap_or(0),
        )
    }

    /// The tiles **in the order the composition places its plots**: the hero
    /// first, then every other tile in the table's own column order.
    ///
    /// Not the same list as [`Self::tiles`], which stays in file order: a
    /// coordinate pair's joint tile sits where its first column does, and the
    /// hero is lifted out of that order to head the page. Anything that maps a
    /// plot index back to a column — the inspector's read of a clicked tile —
    /// has to read this one. `the_plot_order_leads_with_the_hero` holds it.
    #[must_use]
    pub fn plot_order(&self) -> Vec<&Tile> {
        match self.hero_index() {
            None => self.tiles.iter().collect(),
            Some(hero) => std::iter::once(&self.tiles[hero])
                .chain(
                    self.tiles
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != hero)
                        .map(|(_, t)| t),
                )
                .collect(),
        }
    }

    /// The tiles that stack in the column beside the hero, in file order.
    ///
    /// Empty for a dashboard with no hero, which draws no column.
    #[must_use]
    pub fn column_tiles(&self) -> Vec<&Tile> {
        match self.hero_index() {
            None => Vec::new(),
            Some(hero) => self
                .tiles
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != hero)
                .map(|(_, t)| t)
                .collect(),
        }
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
    /// shared selection, the data block — the file, and the bucket column of
    /// every tile that had to be resampled — and the tiles laid out as **the
    /// hero beside the column that holds the rest**. The tests are
    /// `every_tile_reads_the_one_table_however_many_columns_are_derived` over
    /// the derived half and `the_hero_takes_the_larger_share_of_the_page` over
    /// the layout.
    ///
    /// A dashboard of one tile has no column to stand a hero beside and is
    /// written as it always was: one row of one plot.
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
        out.push_str(&source_spec_deriving(&self.path, &self.derived_columns()));
        let Some(hero) = self.hero_index() else {
            let _ = writeln!(out, "vconcat:");
            for row in self.tiles.chunks(TILES_PER_ROW) {
                let _ = writeln!(out, "  - hconcat:");
                for tile in row {
                    let _ = writeln!(out, "    # {}", tile_comment(tile));
                    out.push_str(&tile_yaml(tile, 4, TILE_WIDTH, TILE_HEIGHT));
                }
            }
            return out;
        };
        let _ = writeln!(out, "hconcat:");
        // The hero stands in a column of its own, under a spacer, because an
        // `hconcat` offers each flexing item its whole height and the page is
        // as tall as the *column* needs — see `HERO_BOUND`.
        let _ = writeln!(out, "  - vconcat:");
        let _ = writeln!(out, "    # {}", tile_comment(&self.tiles[hero]));
        out.push_str(&tile_yaml(&self.tiles[hero], 4, HERO_WIDTH, HERO_HEIGHT));
        let _ = writeln!(out, "    - vspace: {HERO_BOUND}");
        // The gutter the two panes' own frames take out of the page between
        // them. See `HERO_GUTTER`.
        let _ = writeln!(out, "  - hspace: {HERO_GUTTER}");
        let _ = writeln!(out, "  - vconcat:");
        for tile in self.column_tiles() {
            let _ = writeln!(out, "    # {}", tile_comment(tile));
            out.push_str(&tile_yaml(tile, 4, COLUMN_TILE_WIDTH, COLUMN_TILE_HEIGHT));
        }
        out
    }

    /// The SQL projections the data block declares beside the file's own
    /// columns: one bucket column per resampled tile, in tile order.
    ///
    /// Empty for a dashboard whose columns all draw as they stand, which is
    /// what keeps the data block a single `file:` entry for a table carrying no
    /// timestamp.
    fn derived_columns(&self) -> Vec<String> {
        self.tiles
            .iter()
            .filter_map(|tile| {
                tile.resampled
                    .map(|step| resample::projection(&tile.column, tile.drawn_column(), step))
            })
            .collect()
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
fn tile_for(column: &ColumnProfile, taken: &[String]) -> Result<Tile, Omission> {
    if column.non_null == 0 {
        return Err(Omission::NoValues);
    }
    if column.distinct <= 1 {
        return Err(Omission::OneValue);
    }
    // The bucket column, when this is a timestamp. It outranks both doors
    // below, for the reason `field_as` keeps a stored date temporal: the label
    // still gets to refuse the column outright, and a label that answers
    // "category" does not take a column's time order away.
    let bucket = bucket_field(column, taken);
    let offered = |own: Option<Field>| bucket.clone().map(|(f, _)| f).or(own);
    let (field, chosen_by) = match role_of(&column.semantic) {
        Some((label, ColumnRole::Neither)) => {
            return Err(Omission::Identifies {
                label: label.to_string(),
            })
        }
        Some((label, role)) => (
            offered(field_as(column, role)).ok_or(Omission::NoPicture)?,
            ChosenBy::Meaning {
                label: label.to_string(),
                role,
            },
        ),
        None => (
            offered(field_of(column)).ok_or(Omission::NoPicture)?,
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
        column: column.name.clone(),
        field,
        block,
        chosen_by,
        resampled: bucket.map(|(_, step)| step),
        paired: None,
    })
}

/// The columns [`Dashboard::of`] draws as one [`chart_kinds::POINT_MAP`] tile
/// instead of two histograms — the longitude's index, the latitude's, and
/// which test found them — or `None` when no pair is there.
///
/// Three tests, in strength order, and the first that answers wins:
///
/// - **a finetype label** naming one column `geography.coordinate.longitude`
///   and another `geography.coordinate.latitude` — read straight off
///   [`role_of`], which already gives both that role;
/// - **a column name**, case-insensitively `longitude`/`lon`/`lng` beside
///   `latitude`/`lat`;
/// - **a measured range that fits degrees**: a column whose profiled `min`/
///   `max` both fall inside `[-90, 90]` (a plausible latitude) beside one
///   whose range needs `[-180, 180]` — falls outside `[-90, 90]` somewhere,
///   which a latitude cannot — and is still inside it. Applied only when each
///   rule matches **exactly one** column, so a table with two candidate
///   latitudes finds no pair rather than guessing which is real.
///
/// Each candidate still has to be a column [`tile_for`] would draw in the
/// first place — nameable, non-null, more than one distinct value, and
/// quantitative — so a constant or all-null `latitude` column does not steal
/// its neighbour into a tile neither of them can hold.
pub(crate) fn coordinate_pair(columns: &[ColumnProfile]) -> Option<(usize, usize, &'static str)> {
    let eligible = |c: &ColumnProfile| {
        chart_kinds::nameable(&c.name)
            && c.non_null > 0
            && c.distinct > 1
            && chart_kinds::fields_of(std::slice::from_ref(c))
                .first()
                .is_some_and(|f| f.ty == FieldType::Quantitative)
    };

    // Tier 1: the finetype label.
    let labelled_as = |leaf: &str| {
        columns.iter().position(|c| {
            eligible(c)
                && matches!(role_of(&c.semantic), Some((label, _)) if label == format!("geography.coordinate.{leaf}"))
        })
    };
    if let (Some(lon_i), Some(lat_i)) = (labelled_as("longitude"), labelled_as("latitude")) {
        return Some((lon_i, lat_i, "label"));
    }

    // Tier 2: the column name.
    let named = |names: &[&str]| {
        columns
            .iter()
            .position(|c| eligible(c) && names.iter().any(|n| c.name.eq_ignore_ascii_case(n)))
    };
    if let (Some(lon_i), Some(lat_i)) = (
        named(&["longitude", "lon", "lng"]),
        named(&["latitude", "lat"]),
    ) {
        if lon_i != lat_i {
            return Some((lon_i, lat_i, "name"));
        }
    }

    // Tier 3: the measured range.
    let range = |c: &ColumnProfile| -> Option<(f64, f64)> {
        let min: f64 = c.min.as_ref()?.parse().ok()?;
        let max: f64 = c.max.as_ref()?.parse().ok()?;
        (min <= max).then_some((min, max))
    };
    let candidates = |test: fn(f64, f64) -> bool| -> Vec<usize> {
        columns
            .iter()
            .enumerate()
            .filter(|(_, c)| eligible(c))
            .filter(|(_, c)| range(c).is_some_and(|(min, max)| test(min, max)))
            .map(|(i, _)| i)
            .collect()
    };
    let lat_candidates =
        candidates(|min, max| (-90.0..=90.0).contains(&min) && (-90.0..=90.0).contains(&max));
    let lon_candidates = candidates(|min, max| {
        (-180.0..=180.0).contains(&min)
            && (-180.0..=180.0).contains(&max)
            && (min < -90.0 || max > 90.0)
    });
    if let ([lat_i], [lon_i]) = (lat_candidates.as_slice(), lon_candidates.as_slice()) {
        return Some((*lon_i, *lat_i, "range"));
    }

    None
}

/// The joint [`chart_kinds::POINT_MAP`] tile over `lon` and `lat`, found by
/// `rule` — [`coordinate_pair`]'s own account of why these two columns and not
/// two histograms.
///
/// # Panics
///
/// Never, in practice: the point map kind's two required quantitative slots
/// are exactly what [`coordinate_pair`]'s own eligibility test already proved
/// of both columns, so the `expect`s below hold for any pair this function is
/// actually called with. They are `expect`s rather than a `Result` because a
/// caller that reaches here has nothing left to hand back — the two columns
/// were already promised a tile the moment [`coordinate_pair`] found them.
fn point_map_tile_for(lon: &ColumnProfile, lat: &ColumnProfile, rule: &'static str) -> Tile {
    let x = Field::new(&lon.name, FieldType::Quantitative);
    let y = Field::new(&lat.name, FieldType::Quantitative);
    let kind =
        chart_kinds::find(chart_kinds::POINT_MAP).expect("this build ships a point map kind");
    let binding = kind
        .bind(&[x.clone(), y.clone()])
        .expect("two measures fill the point map's two required slots");
    let block = kind
        .spec(&binding, &kind.options())
        .expect("the point map kind builds its spec");
    Tile {
        kind: kind.id,
        column: lon.name.clone(),
        field: x,
        block,
        chosen_by: ChosenBy::CoordinatePair {
            latitude: lat.name.clone(),
            rule,
        },
        resampled: None,
        paired: Some((lat.name.clone(), y)),
    }
}

/// The temporal field a timestamp column is drawn through, and the step it is
/// counted at — or `None` for a column that draws as it stands.
///
/// The field names a column the table does not have: [`crate::resample`]
/// derives it, and [`Dashboard::to_spec`] declares it beside the file. That is
/// why the field is built here rather than asked of
/// [`chart_kinds::fields_of`], which maps the columns a profile carries — and
/// why the one eligibility rule a derived name can still break, `nameable`, is
/// asked for explicitly.
///
/// `taken` is the table's own column names, so the derived one cannot collide
/// with a column the file already has.
fn bucket_field(column: &ColumnProfile, taken: &[String]) -> Option<(Field, Step)> {
    if !resample::is_resampled_type(&column.type_name) || !chart_kinds::nameable(&column.name) {
        return None;
    }
    let step = resample::step_for(column);
    let name = resample::derived_name(&column.name, step, taken);
    Some((Field::new(name, FieldType::Temporal), step))
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
    // A column DuckDB stored as a date keeps the temporal field its own type
    // offers, whatever the label says it is for. `datetime.*` labels answer
    // `Category` — a period IS a member of a set — but the set is ordered, and
    // restating the column as text to honour that answer would put it back
    // under the category ceiling and take its time order away. The label is
    // still what refuses the column outright: a date labelled an identifier
    // never reaches here.
    let own = field_of(column);
    if own.as_ref().is_some_and(|f| f.ty == FieldType::Temporal) {
        return own;
    }
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
    } else if kind == chart_kinds::COUNTS_OVER_TIME {
        Some(TileForm::TimeBars)
    } else if kind == crate::ranked_bars::KIND_ID {
        Some(TileForm::RankedBars)
    } else if kind == chart_kinds::POINT_MAP {
        Some(TileForm::PointMap)
    } else {
        None
    }
}

/// The shapes a tile is written in. One per kind that can be one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileForm {
    /// A binned count, brushed with an `intervalX`.
    Histogram,
    /// A count per date in time order, clicked with a `toggleX`.
    TimeBars,
    /// [`RankedCategoryBars`], whose own emitter writes it.
    RankedBars,
    /// [`chart_kinds::point_map_tile`], over the tile's two columns rather
    /// than one — the form this module writes for a [`Tile`] whose
    /// [`Tile::paired_column`] answers.
    PointMap,
}

/// `tile` as one entry of a concat list, indented by `indent` spaces.
///
/// Written over [`Tile::drawn_column`] rather than [`Tile::column`], which are
/// the same name for every tile a resample did not touch. A timestamp's marks
/// name its bucket column, because that is the one with a band — the test that
/// reads the two apart is `a_timestamp_past_the_grid_ceiling_gets_a_tile`, and
/// `the_dates_tile_counts_in_time_order_and_drops_no_date` is the date beside
/// it, whose mark names the file's own column.
fn tile_yaml(tile: &Tile, indent: usize, width: u32, height: u32) -> String {
    match tile_form(tile.kind) {
        Some(TileForm::Histogram) => {
            histogram_tile_sized(tile.drawn_column(), indent, width, height)
        }
        Some(TileForm::TimeBars) => {
            time_bars_tile_sized(tile.drawn_column(), indent, width, height)
        }
        Some(TileForm::RankedBars) => RankedCategoryBars::new(tile.drawn_column())
            .with_size(width, height)
            .plot_yaml(SOURCE, SELECTION, indent),
        Some(TileForm::PointMap) => chart_kinds::point_map_tile_sized(
            tile.drawn_column(),
            tile.paired_column().unwrap_or_default(),
            indent,
            width,
            height,
        ),
        // Unreachable: a tile's kind came from `single_column_kinds` or from
        // `point_map_tile_for`, and a kind reaching here from either one
        // already has a form. Emitting an empty string rather than a
        // half-written plot is the answer that cannot produce a spec which
        // parses and draws something else.
        None => String::new(),
    }
}

/// The ink the ghost layer is drawn in — the warm-grey border step of the
/// design system's generated grey scale,
/// [`meridian_design::scales::GRAY_LIGHT`].
///
/// A token rather than a hex constant, so a palette regeneration moves the ghost
/// with the rest of the chart's ink instead of leaving it behind. Spelled out
/// with [`meridian_design::colour::Rgba::hex`], which round-trips the scale's own
/// 8-bit channels exactly.
///
/// **Read from the scale rather than copied from the histogram kind**, which
/// reads the same step for the same device — see [`histogram_tile`] for why
/// there are two emitters of one device and what closes that.
const GHOST_INK: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[7];

/// A measure's distribution, brushable, with **the unfiltered total kept behind
/// the filtered subset**.
///
/// Two `rectY` layers over one table and one `x: { bin: }` / `y: { count: }`
/// transform. The first reads [`SOURCE`] straight and never narrows — the ghost,
/// drawn in the private `GHOST_INK`; the second reads it through `filterBy:` the shared
/// selection and lands on top in the default mark ink. They share the plot's
/// scales, so the count axis is fixed by the total and a brushed tile reads as a
/// fraction of the bars behind it. One filtered layer draws a perfectly good
/// histogram after a brush and gives the reader no way to see what fraction of
/// the data it is; `examples/rect-bin-count-ghost.yaml` is the same device
/// authored by hand.
///
/// `xDomain: Fixed` does the same job one axis over: without it the bin edges
/// would be re-derived from whatever the *other* tiles left, so the bars would
/// move sideways under the pointer and two frames of one column would not be
/// comparable.
///
/// **Public, and that is the seam.** This device is emitted in two places — here
/// as a tile, and by the `binned-histogram` kind as a standalone document — and
/// prose is all that holds them in step. Publishing the tile form is what lets
/// that end in one call rather than a third copy: a kind's builder can wrap this
/// body under its own `params:` header. Until it does, a change to the device
/// has to be made twice, and this comment is where a reader finds that out.
#[must_use]
pub fn histogram_tile(column: &str, indent: usize) -> String {
    histogram_tile_sized(column, indent, TILE_WIDTH, TILE_HEIGHT)
}

/// [`histogram_tile`] at a declared size — the weight a constrained concat
/// shares its box out by. See [`HERO_WIDTH`].
#[must_use]
pub fn histogram_tile_sized(column: &str, indent: usize, width: u32, height: u32) -> String {
    let pad = " ".repeat(indent);
    let col = yaml_string(column);
    let table = yaml_string(SOURCE);
    let mut out = String::new();
    let _ = writeln!(out, "{pad}- plot:");
    // The ghost, first so the subset covers it: the whole table, with no
    // `filterBy:` to narrow it.
    let _ = writeln!(out, "{pad}  - mark: rectY");
    let _ = writeln!(out, "{pad}    data: {{ from: {table} }}");
    let _ = writeln!(out, "{pad}    x: {{ bin: {col} }}");
    let _ = writeln!(out, "{pad}    y: {{ count: }}");
    let _ = writeln!(out, "{pad}    fill: \"{}\"", GHOST_INK.hex());
    // The subset: the same transform, through the selection, in the mark ink a
    // layer binding no colour channel takes.
    let _ = writeln!(out, "{pad}  - mark: rectY");
    let _ = writeln!(
        out,
        "{pad}    data: {{ from: {table}, filterBy: ${SELECTION} }}"
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
    let _ = writeln!(out, "{pad}  width: {width}");
    let _ = writeln!(out, "{pad}  height: {height}");
    out
}

/// A dated column's rows **counted per date, in time order**, with the
/// unfiltered total kept behind the selected part.
///
/// One `barY` over a band of dates: `x:` names the column and `y: { count: }`
/// aggregates it, which is the pair
/// [`brightfield_spec::vocab::MarkKind::band_aggregate_axes`] lifts into one
/// `GROUP BY` and one `COUNT(*)`.
///
/// # The two things it deliberately does not write
///
/// **No `sort:`.** `brightfield-sql`'s `BarLowerer` orders a band aggregation
/// by its band column when no sort is lifted, so the omission is the
/// instruction: ascending on a column of dates is chronological. A
/// [`RankedCategoryBars`] over the same column would write `sort: { y: -x,
/// limit: 10 }` and hand back the ten busiest dates in descending order of
/// count — each bar true, the series gone.
///
/// **No `limit:`.** A cap is a legibility control over categories nobody can
/// rank; dropping the quiet days out of a time series leaves a picture that
/// reads as continuous and is not. A long series draws thin bars, and a thin
/// bar is still where that day was.
///
/// # The selection
///
/// `toggleX` publishes the clicked date into the shared selection and
/// `highlight` reads it back — the same part-of-whole device
/// [`crate::ranked_bars`] documents, in the other orientation, so the dates and
/// their positions are computed from the unfiltered table and no date can drop
/// off the axis by being selected against. There is deliberately no `filterBy:`
/// on the mark; that module's header says what one would cost.
#[must_use]
pub fn time_bars_tile(column: &str, indent: usize) -> String {
    time_bars_tile_sized(column, indent, TILE_WIDTH, TILE_HEIGHT)
}

/// [`time_bars_tile`] at a declared size. See [`histogram_tile_sized`].
#[must_use]
pub fn time_bars_tile_sized(column: &str, indent: usize, width: u32, height: u32) -> String {
    let pad = " ".repeat(indent);
    let col = yaml_string(column);
    let table = yaml_string(SOURCE);
    let mut out = String::new();
    let _ = writeln!(out, "{pad}- plot:");
    let _ = writeln!(out, "{pad}  - mark: barY");
    let _ = writeln!(out, "{pad}    data: {{ from: {table} }}");
    let _ = writeln!(out, "{pad}    x: {col}");
    let _ = writeln!(out, "{pad}    y: {{ count: }}");
    // The producer: a click on a date's band publishes it into the selection.
    let _ = writeln!(out, "{pad}  - select: toggleX");
    let _ = writeln!(out, "{pad}    as: ${SELECTION}");
    // The consumer: the predicate lands inside the conditional SUM.
    let _ = writeln!(out, "{pad}  - select: highlight");
    let _ = writeln!(out, "{pad}    by: ${SELECTION}");
    // Plot attributes are siblings of `plot:`, so they sit at its indent — one
    // level deeper and they read as more options on the last interactor, which
    // is a spec that parses and does something else.
    let _ = writeln!(out, "{pad}  xLabel: {col}");
    let _ = writeln!(out, "{pad}  width: {width}");
    let _ = writeln!(out, "{pad}  height: {height}");
    out
}

/// The sentence written above a tile in the emitted spec: the column, what
/// decided its type, and the kind that came back.
///
/// **This is where AC-level "the rule is stated somewhere a reader can check
/// it" is discharged in the artefact.** A reader with the spec in front of them
/// can see that `amount` was binned because a label called it a currency
/// amount, and that `region` was ranked because no trusted label came back and
/// DuckDB stored it as text. The four states that count as no trusted label are
/// [`role_of`]'s.
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
            format!("no trusted label, and DuckDB stored it as {type_name}")
        }
        ChosenBy::CoordinatePair { latitude, rule } => {
            format!("a coordinate pair with {latitude}, found by its {rule}")
        }
    };
    // The step a resampled column is counted at is part of what the reader has
    // to be able to check: it is the difference between a count per day and a
    // count per year over the same instants, and it was chosen from the span
    // rather than written by anyone.
    let at = match tile.resampled() {
        Some(step) => format!(", counted by the {}", step.label()),
        None => String::new(),
    };
    format!("{}: {because} → {}{at}", tile.column(), tile.kind())
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
            moments: None,
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

    /// A timestamp column spanning `min` to `max`, rendered the way
    /// `Session::profile_sources` renders a temporal minimum and maximum:
    /// `CAST(… AS VARCHAR)`.
    fn stamped(name: &str, min: &str, max: &str, distinct: u64) -> ColumnProfile {
        ColumnProfile {
            min: Some(min.to_string()),
            max: Some(max.to_string()),
            ..column(name, "TIMESTAMP", distinct)
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

    // -----------------------------------------------------------------
    // AC2 — a coordinate pair draws one point-map tile, not two histograms
    // -----------------------------------------------------------------

    /// A profiled column carrying a measured `min`/`max`, for
    /// [`coordinate_pair`]'s range tier — `DuckDB`'s own `CAST(min/max AS
    /// VARCHAR)` rendering, which is plain decimal text for these values.
    fn ranged(name: &str, distinct: u64, min: f64, max: f64) -> ColumnProfile {
        ColumnProfile {
            min: Some(min.to_string()),
            max: Some(max.to_string()),
            ..column(name, "DOUBLE", distinct)
        }
    }

    /// **The card's own scenario**: `longitude` and `latitude` beside an
    /// ordinary measure, none of them carrying a semantic label — a `cargo
    /// test` binary carries no FineType bundle (see [`role_of`]'s callers),
    /// so the pair is found by column name, the tier the label defers to.
    #[test]
    fn a_coordinate_pair_draws_one_tile_for_two_columns() {
        let dash = of(&[
            column("longitude", "DOUBLE", 900),
            column("latitude", "DOUBLE", 900),
            column("housing_median_age", "BIGINT", 52),
        ]);
        let kinds: Vec<(&str, ChartKindId)> = dash
            .tiles()
            .iter()
            .map(|t| (t.column(), t.kind()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("longitude", chart_kinds::POINT_MAP),
                ("housing_median_age", chart_kinds::BINNED_HISTOGRAM),
            ],
            "{kinds:?}"
        );
        assert_eq!(dash.tiles()[0].paired_column(), Some("latitude"));
        assert!(
            !dash
                .tiles()
                .iter()
                .any(|t| t.kind() == chart_kinds::BINNED_HISTOGRAM
                    && (t.column() == "longitude" || t.column() == "latitude")),
            "a coordinate column reached a histogram tile: {kinds:?}"
        );
        assert!(dash.omitted().is_empty(), "{:?}", dash.omitted());
    }

    /// **The finetype label wins over a name that would answer differently.**
    ///
    /// `lon`/`lat` are named so the name tier alone would find them too, but
    /// this asks whether the label tier is consulted FIRST and reads its
    /// rule off [`Tile::chosen_by`] rather than its outcome alone.
    #[test]
    fn a_coordinate_pair_found_by_its_finetype_label_names_the_label_tier() {
        let dash = of(&[
            labelled("lon", "DOUBLE", 900, "geography.coordinate.longitude"),
            labelled("lat", "DOUBLE", 900, "geography.coordinate.latitude"),
        ]);
        assert_eq!(dash.tiles().len(), 1, "{dash:?}");
        match dash.tiles()[0].chosen_by() {
            ChosenBy::CoordinatePair { latitude, rule } => {
                assert_eq!(latitude, "lat");
                assert_eq!(
                    *rule, "label",
                    "a labelled pair should not fall to the name tier"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// **The joint tile takes the position of whichever column the table
    /// declares first** — here the latitude, so the pair's tile sits second
    /// and `weight` keeps its own histogram after it rather than before.
    #[test]
    fn a_coordinate_pairs_tile_takes_the_position_of_whichever_column_comes_first() {
        let dash = of(&[
            column("region", "VARCHAR", 4),
            column("latitude", "DOUBLE", 900),
            column("longitude", "DOUBLE", 900),
            column("weight", "BIGINT", 400),
        ]);
        let kinds: Vec<(&str, ChartKindId)> = dash
            .tiles()
            .iter()
            .map(|t| (t.column(), t.kind()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("region", crate::ranked_bars::KIND_ID),
                ("longitude", chart_kinds::POINT_MAP),
                ("weight", chart_kinds::BINNED_HISTOGRAM),
            ],
            "{kinds:?}"
        );
        assert_eq!(dash.tiles()[1].paired_column(), Some("latitude"));
    }

    /// **A lone coordinate column, with no partner, gets its own histogram —
    /// the fallback per-column path is unchanged.**
    #[test]
    fn a_lone_coordinate_column_still_gets_its_own_histogram() {
        let dash = of(&[column("longitude", "DOUBLE", 900)]);
        assert_eq!(dash.tiles().len(), 1);
        assert_eq!(dash.tiles()[0].kind(), chart_kinds::BINNED_HISTOGRAM);
        assert_eq!(dash.tiles()[0].paired_column(), None);
    }

    /// **The measured-range tier**, when neither a label nor a name helps: two
    /// numeric columns named nothing like coordinates, one whose range is a
    /// plausible latitude and one whose range escapes it but still fits a
    /// longitude.
    #[test]
    fn a_coordinate_pair_found_by_its_measured_range_names_the_range_tier() {
        let dash = of(&[
            ranged("y_coord", 900, 32.5, 41.9),
            ranged("x_coord", 900, -124.3, -114.3),
        ]);
        assert_eq!(dash.tiles().len(), 1, "{dash:?}");
        let tile = &dash.tiles()[0];
        assert_eq!(tile.column(), "x_coord", "x is the longitude candidate");
        assert_eq!(tile.paired_column(), Some("y_coord"));
        match tile.chosen_by() {
            ChosenBy::CoordinatePair { rule, .. } => assert_eq!(*rule, "range"),
            other => panic!("{other:?}"),
        }
    }

    /// **An ambiguous range finds no pair rather than guessing.** Two columns
    /// both fit inside a plausible latitude span, so the range tier — which
    /// only answers when exactly one column matches each side — declines, and
    /// each column keeps its own histogram.
    #[test]
    fn an_ambiguous_measured_range_is_not_guessed_into_a_pair() {
        let dash = of(&[
            ranged("metric_a", 900, 10.0, 40.0),
            ranged("metric_b", 900, 5.0, 60.0),
        ]);
        let kinds: Vec<ChartKindId> = dash.tiles().iter().map(Tile::kind).collect();
        assert_eq!(
            kinds,
            vec![chart_kinds::BINNED_HISTOGRAM, chart_kinds::BINNED_HISTOGRAM],
            "two ordinary measures were paired into a map: {kinds:?}"
        );
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

    /// **A daily series over more than two months gets a tile.**
    ///
    /// The case this rule was written for, end to end: ninety days is past the
    /// category ceiling, and the ceiling used to be applied to every
    /// non-binnable column — so the commonest column in a data file was handed
    /// back with *"no chart in this build fits it"*. It is now a tile, and the
    /// kind is the one that follows from the column holding dates.
    #[test]
    fn a_daily_series_over_more_than_two_months_gets_a_tile() {
        let dash = of(&[column("day", "DATE", 90)]);
        assert!(
            dash.omitted().is_empty(),
            "the date was left out: {:?}",
            dash.omitted()
        );
        let tile = dash.sole_tile().expect("one usable column is one tile");
        assert_eq!(tile.column(), "day");
        assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME);
        assert_eq!(tile.field().ty, FieldType::Temporal);
        // The device follows from the column being dated, not from its width:
        // a decade of days answers the same as a fortnight.
        for width in [14, 3_650] {
            assert_eq!(
                of(&[column("day", "DATE", width)]).tiles()[0].kind(),
                chart_kinds::COUNTS_OVER_TIME,
                "{width} distinct days"
            );
        }
    }

    /// **The tile the date gets is a count per date in time order, uncapped**,
    /// and the spec says which rule chose it.
    ///
    /// The two absences are the device: a `sort:` would rank the dates by count
    /// and a `limit:` would drop the quiet ones, and either turns a series into
    /// a leaderboard. Asserted on the emitted source, because that is the
    /// artefact and it is where the mistake would be made.
    #[test]
    fn the_dates_tile_counts_in_time_order_and_drops_no_date() {
        let source = of(&[column("day", "DATE", 90)]).to_spec();
        assert!(source.contains("mark: barY"), "{source}");
        assert!(source.contains("x: \"day\""), "{source}");
        assert!(source.contains("y: { count: }"), "{source}");
        assert!(
            !source.contains("sort:"),
            "a sort would rank the dates by count and lose the series:\n{source}"
        );
        assert!(
            !source.contains("limit:"),
            "a limit would drop the quiet dates out of a series that reads as \
             continuous:\n{source}"
        );
        // It drives and reads the one shared selection, like every other tile.
        assert!(source.contains("select: toggleX"), "{source}");
        assert!(source.contains(&format!("by: ${SELECTION}")), "{source}");
        assert!(
            !source.contains("filterBy:"),
            "the selection reaches a band aggregation through `highlight`, \
             inside the conditional sum:\n{source}"
        );
        // And the reader holding the spec can see why this tile is this tile.
        assert!(
            source.contains(
                "# day: no trusted label, and DuckDB stored it as DATE → counts-over-time"
            ),
            "{source}"
        );
    }

    /// **A stored date keeps its time order even when a label calls it a
    /// category.** `datetime.*` answers [`ColumnRole::Category`], and restating
    /// the column as text to honour that would put a year of days back under
    /// the category ceiling — which is the omission this rule exists to end.
    #[test]
    fn a_dated_column_labelled_a_category_is_still_drawn_in_time_order() {
        let dash = of(&[labelled("day", "DATE", 365, "datetime.date.iso")]);
        assert!(dash.omitted().is_empty(), "{:?}", dash.omitted());
        let tile = dash.sole_tile().expect("a tile");
        assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME);
        assert!(matches!(
            tile.chosen_by(),
            ChosenBy::Meaning {
                role: ColumnRole::Category,
                ..
            }
        ));
        // The label still gets to refuse the column outright — being stored as
        // a date does not make an identifier drawable.
        let refused = of(&[labelled("issued", "DATE", 900, "identity.government.ssn")]);
        assert!(refused.tiles().is_empty(), "{refused:?}");
    }

    /// **A timestamp past the ceiling gets a tile too**, which is the half of
    /// this rule that was excluded rather than solved: a `TIMESTAMP` reaches no
    /// kind of its own, so the column that draws is the bucket one
    /// [`crate::resample`] derives, and the tile is still *of* the timestamp.
    #[test]
    fn a_timestamp_past_the_grid_ceiling_gets_a_tile() {
        let dash = of(&[stamped(
            "observed",
            "2026-01-01 00:00:00",
            "2026-04-01 00:00:00",
            9_000,
        )]);
        assert!(
            dash.omitted().is_empty(),
            "the timestamp was left out: {:?}",
            dash.omitted()
        );
        let tile = dash.sole_tile().expect("one usable column is one tile");
        assert_eq!(tile.column(), "observed", "the tile is of the column");
        assert_eq!(tile.drawn_column(), "observed by day");
        assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME);
        assert_eq!(tile.field().ty, FieldType::Temporal);
        assert_eq!(tile.resampled(), Some(Step::Day));
        // The device follows from the column being temporal, and the step from
        // its span — neither from how many distinct instants it holds.
        for distinct in [61, 9_000, 4_000_000] {
            let one = of(&[stamped(
                "observed",
                "2026-01-01 00:00:00",
                "2026-04-01 00:00:00",
                distinct,
            )]);
            let tile = one.sole_tile().expect("a tile");
            assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME, "{distinct}");
            assert_eq!(tile.resampled(), Some(Step::Day), "{distinct}");
        }
    }

    /// **The timestamp's tile is the date's tile, over the bucket column** —
    /// the same two absences, the same interactors, and a `data:` block that
    /// declares where the bucket column comes from.
    #[test]
    fn the_timestamps_tile_is_the_dates_tile_over_the_bucket_column() {
        let source = of(&[stamped(
            "observed",
            "2026-01-01 00:00:00",
            "2026-04-01 00:00:00",
            9_000,
        )])
        .to_spec();
        assert!(source.contains("mark: barY"), "{source}");
        assert!(source.contains("x: \"observed by day\""), "{source}");
        assert!(source.contains("y: { count: }"), "{source}");
        assert!(
            !source.contains("sort:") && !source.contains("limit:"),
            "a sort would rank the days by count and a limit would drop the \
             quiet ones:\n{source}"
        );
        assert!(source.contains("select: toggleX"), "{source}");
        assert!(source.contains(&format!("by: ${SELECTION}")), "{source}");
        // The bucket column, declared: one strftime over the instant, projected
        // beside the file's own columns.
        assert!(
            source.contains(
                "SELECT *, strftime(CAST(\"observed\" AS TIMESTAMP), \
                 ''%Y-%m-%d'') AS \"observed by day\" FROM \"opened_rows\""
            ),
            "{source}"
        );
        // And the reader holding the spec is told the step it was counted at,
        // because a count per day and a count per year are different pictures
        // of the same instants.
        assert!(
            source.contains(
                "# observed: no trusted label, and DuckDB stored it as \
                 TIMESTAMP → counts-over-time, counted by the day"
            ),
            "{source}"
        );
        let parsed = parse_spec(&source, Format::Yaml)
            .unwrap_or_else(|e| panic!("the generated spec does not parse: {e}\n{source}"));
        assert!(
            parsed.warnings.is_empty(),
            "the generated spec warns: {:?}\n{source}",
            parsed.warnings
        );
    }

    /// **Every tile reads one table, however many columns the dashboard
    /// derived.**
    ///
    /// This is what the crossfilter rests on. A clause published by one tile is
    /// applied to every other, so a tile reading a table its siblings' lacks a
    /// column of would take their queries down with it — and the bucket column
    /// exists in exactly one place, which is why the file is read under a
    /// second name rather than the tiles being pointed at a second table.
    #[test]
    fn every_tile_reads_the_one_table_however_many_columns_are_derived() {
        let source = of(&[
            stamped(
                "observed",
                "2026-01-01 00:00:00",
                "2026-04-01 00:00:00",
                900,
            ),
            stamped("settled", "2026-01-01 00:00:00", "2030-01-01 00:00:00", 900),
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
        ])
        .to_spec();
        let one = format!("from: {}", yaml_string(SOURCE));
        let reads = source.match_indices("from: ").count();
        assert!(reads > 0, "no mark reads anything:\n{source}");
        assert_eq!(
            source.matches(one.as_str()).count(),
            reads,
            "a tile reading anything but the one table breaks the \
             crossfilter:\n{source}"
        );
        // Two timestamps, two bucket columns, each at the step its own span
        // asked for — and both in the one projection list.
        assert!(
            source.contains("AS \"observed by day\"") && source.contains("AS \"settled by month\""),
            "{source}"
        );
        assert_eq!(source.matches("SELECT *,").count(), 1, "{source}");
    }

    /// A table with nothing to derive declares the file and nothing else, which
    /// is the shape every dashboard of measures, categories and dates keeps.
    #[test]
    fn a_table_with_nothing_to_derive_declares_the_file_and_nothing_else() {
        let source = of(&[column("amount", "DOUBLE", 900), column("day", "DATE", 90)]).to_spec();
        assert!(
            source.contains("data:\n  opened:\n    file: '/tmp/readings.csv'\n"),
            "{source}"
        );
        assert!(!source.contains("opened_rows"), "{source}");
    }

    /// The bucket column takes a name the table does not already carry, so the
    /// tile's mark binds the projection rather than the file's own column of
    /// that name.
    #[test]
    fn a_bucket_column_steps_aside_for_a_column_that_already_has_its_name() {
        let dash = of(&[
            stamped("t", "2026-01-01 00:00:00", "2026-04-01 00:00:00", 900),
            column("t by day", "VARCHAR", 4),
        ]);
        assert_eq!(dash.tiles()[0].drawn_column(), "t by day 2");
        let source = dash.to_spec();
        assert!(source.contains("AS \"t by day 2\""), "{source}");
    }

    /// A label still decides what a timestamp is *for*, and still cannot take
    /// its time order away — the rule `a_dated_column_labelled_a_category…`
    /// holds for a stored date, reaching a timestamp through the bucket column.
    #[test]
    fn a_timestamp_labelled_a_category_is_still_counted_in_time_order() {
        let mut profile = stamped("seen", "2026-01-01 00:00:00", "2026-04-01 00:00:00", 900);
        profile.semantic = labelled("seen", "TIMESTAMP", 900, "datetime.timestamp.iso").semantic;
        let dash = of(&[profile.clone()]);
        let tile = dash.sole_tile().expect("a tile");
        assert_eq!(tile.kind(), chart_kinds::COUNTS_OVER_TIME);
        assert_eq!(tile.resampled(), Some(Step::Day));
        // And a label that refuses the column outright still refuses it.
        profile.semantic = labelled("seen", "TIMESTAMP", 900, "technology.internet.url").semantic;
        let refused = of(&[profile]);
        assert!(refused.tiles().is_empty(), "{refused:?}");
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
            // A resampled column is named by the table's spelling here, not by
            // the bucket column's: the ledger accounts for the file's columns.
            stamped(
                "observed",
                "2026-01-01 00:00:00",
                "2026-04-01 00:00:00",
                900,
            ),
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
            stamped(
                "observed",
                "2026-01-01 00:00:00",
                "2026-04-01 00:00:00",
                900,
            ),
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

    /// Every plot rect the emitted spec lays out into a box of `width` by
    /// `height`, in the order the composition places them.
    ///
    /// The spec's own layout engine rather than a second copy of its rules,
    /// which is the whole point: a claim about where the hero lands is a claim
    /// about what `brightfield_spec::layout` does with these weights.
    fn plot_rects(source: &str, width: f64, height: f64) -> Vec<brightfield_spec::layout::Rect> {
        use brightfield_spec::layout::{compute_layout, LayoutNode, Rect};
        let parsed = parse_spec(source, Format::Yaml)
            .unwrap_or_else(|e| panic!("the generated spec does not parse: {e}\n{source}"));
        let tree = compute_layout(&parsed.spec, Rect::new(0.0, 0.0, width, height));
        fn walk(node: &LayoutNode, out: &mut Vec<Rect>) {
            match node {
                LayoutNode::Plot { rect, .. } => out.push(*rect),
                LayoutNode::HConcat { children, .. } | LayoutNode::VConcat { children, .. } => {
                    for child in children {
                        walk(child, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        if let Some(root) = &tree {
            walk(root, &mut out);
        }
        out
    }

    /// **The hero takes the larger share of the page and the rest stack beside
    /// it** — read off the layout the emitted spec produces, not off the
    /// weights it declares.
    ///
    /// The two claims a picture cannot make, and this test walks both: the
    /// hero's width is [`HERO_SHARE`] of what the gutter leaves, and each
    /// stacked tile stands at one width and one height in one column to its
    /// right.
    #[test]
    fn the_hero_takes_the_larger_share_of_the_page() {
        let many: Vec<ColumnProfile> = (0..8)
            .map(|i| column(&format!("m{i}"), "DOUBLE", 900))
            .collect();
        let source = of(&many).to_spec();
        let (w, h) = (1000.0_f64, 700.0_f64);
        let rects = plot_rects(&source, w, h);
        assert_eq!(rects.len(), 8, "eight columns, eight plots:\n{source}");

        let gutter = f64::from(HERO_GUTTER);
        let residual = w - gutter;
        let hero = rects[0];
        assert!(
            (hero.width - f64::from(HERO_SHARE) * residual).abs() < 0.5,
            "the hero is {} wide of a {residual} residual, which is {:.4} of it \
             rather than the declared {HERO_SHARE}:\n{source}",
            hero.width,
            hero.width / residual
        );
        assert!(
            (hero.x - 0.0).abs() < 0.5 && (hero.height - h).abs() < 0.5,
            "the hero does not fill its column: {hero:?}"
        );

        let stacked = &rects[1..];
        let left = hero.x + hero.width + gutter;
        for (i, tile) in stacked.iter().enumerate() {
            assert!(
                (tile.x - left).abs() < 0.5,
                "tile {i} starts at {} rather than beside the hero at {left}",
                tile.x
            );
            assert!(
                (tile.width - stacked[0].width).abs() < 0.5
                    && (tile.height - stacked[0].height).abs() < 0.5,
                "tile {i} is {:?} where the first is {:?} — the column's tiles \
                 stand at one size",
                tile,
                stacked[0]
            );
        }
        let spanned: f64 = stacked.iter().map(|r| r.height).sum();
        assert!(
            (spanned - h).abs() < 0.5,
            "the seven tiles span {spanned} of the {h} their column offers"
        );
    }

    /// **The map's tile is the hero, wherever the file puts its coordinates**,
    /// and the plot order leads with it while the tile list stays in file
    /// order.
    #[test]
    fn the_plot_order_leads_with_the_hero() {
        let dash = of(&[
            column("amount", "DOUBLE", 900),
            column("weight", "DOUBLE", 900),
            column("latitude", "DOUBLE", 900),
            column("longitude", "DOUBLE", 900),
        ]);
        let filed: Vec<&str> = dash.tiles().iter().map(Tile::column).collect();
        assert_eq!(
            filed,
            vec!["amount", "weight", "longitude"],
            "the tile list stays in the file's own order, the pair's joint tile \
             where its first column sits"
        );
        assert_eq!(dash.hero_index(), Some(2));
        let drawn: Vec<&str> = dash.plot_order().iter().map(|t| t.column()).collect();
        assert_eq!(
            drawn,
            vec!["longitude", "amount", "weight"],
            "the composition places the hero first"
        );
        let stacked: Vec<&str> = dash.column_tiles().iter().map(|t| t.column()).collect();
        assert_eq!(stacked, vec!["amount", "weight"]);
    }

    /// **A table with no coordinate pair still opens as a hero beside a
    /// column**, and the hero is the generator's first tile.
    ///
    /// The fallback is decided rather than inherited from the tile order by
    /// accident, so it is asserted rather than left to
    /// `the_plot_order_leads_with_the_hero` to imply.
    #[test]
    fn a_table_with_no_coordinate_pair_makes_its_first_tile_the_hero() {
        let dash = of(&[
            column("sensor", "VARCHAR", 1),
            column("amount", "DOUBLE", 900),
            column("region", "VARCHAR", 4),
        ]);
        // `sensor` earns no tile at all, so the first *tile* and the first
        // column are deliberately different here.
        assert_eq!(dash.hero_index(), Some(0));
        assert_eq!(dash.plot_order()[0].column(), "amount");
    }

    /// **A dashboard of one tile has no column to stand a hero beside**, and
    /// is written the way it always was.
    #[test]
    fn a_single_tile_dashboard_is_not_a_pane_group() {
        let dash = of(&[column("amount", "DOUBLE", 900)]);
        assert_eq!(dash.hero_index(), None);
        assert!(dash.column_tiles().is_empty());
        let source = dash.to_spec();
        assert!(
            source.contains("vconcat:") && !source.contains("hspace"),
            "one tile is one row, with no gutter to leave:\n{source}"
        );
    }

    /// **[`HERO_GUTTER`] is what puts the map pane's drawn rect at
    /// [`HERO_SHARE`] of the canvas**, and it is derived from the chrome
    /// rather than tuned by eye.
    ///
    /// The shell splits the canvas into two panes, each of which takes
    /// `pane_content_inset` out of its own rect at the top, at the bottom and
    /// at both sides, with a pane gap between them; the page the raster is composed on spans the two content
    /// rects and the gutter, and its hero takes `HERO_SHARE` of what the
    /// gutter leaves. Setting the map pane's outer width equal to
    /// `HERO_SHARE · canvas` and solving for the gutter gives the line below.
    /// Move either chrome measurement and this reddens rather than the map
    /// pane quietly drifting off its share.
    #[test]
    fn the_gutter_is_what_puts_the_map_pane_at_its_declared_share() {
        let inset = brightfield_workbench::chrome::pane_content_inset();
        let gap = brightfield_workbench::behavior::TILE_GAP;
        let derived = (2.0 * HERO_SHARE * inset + gap) / (1.0 - HERO_SHARE);
        #[allow(clippy::cast_precision_loss)]
        let declared = HERO_GUTTER as f32;
        assert!(
            (derived - declared).abs() < 0.5,
            "the declared gutter is {declared} where the chrome asks for \
             {derived:.2}: a pane inset of {inset} and a pane gap of {gap}"
        );
    }

    /// **The column's tiles hold at their floor and the page grows instead** —
    /// the "or 96 points whichever is greater, scrolling past that" rule, as
    /// the one number the canvas scrolls a page of.
    #[test]
    fn the_stacked_tiles_hold_at_their_height_floor() {
        // Room to spare: the offered box is what the tiles share.
        assert!((stack_extent(700.0, 7) - 700.0).abs() < f32::EPSILON);
        // Too short: the page grows to the floor, and the difference is what
        // there is to scroll.
        assert!((stack_extent(500.0, 7) - 7.0 * MIN_COLUMN_TILE_HEIGHT).abs() < f32::EPSILON);
        assert!(stack_extent(500.0, 7) > 500.0);
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

    /// **The denominator stays on the page, and the frame of reference with
    /// it.** A histogram tile is two layers: a ghost that never narrows and a
    /// subset read through the selection, over a pinned bin domain.
    ///
    /// Asserted on the emitted source because that is where the mistake is
    /// made — a `filterBy:` on the ghost, or a missing `xDomain`, both draw a
    /// perfectly good chart that has quietly rescaled itself. The raster
    /// arithmetic for the same device is
    /// `a_two_layer_spec_draws_the_filtered_subset_over_the_unfiltered_total`
    /// in `tests/ghosted_histogram.rs`.
    #[test]
    fn a_histogram_tile_keeps_the_total_behind_the_subset() {
        let source = of(&[column("amount", "DOUBLE", 900)]).to_spec();
        assert_eq!(
            source.matches("mark: rectY").count(),
            2,
            "one layer cannot show a subset AS a fraction of a total:\n{source}"
        );
        assert_eq!(
            source.matches("filterBy:").count(),
            1,
            "exactly one of the two layers narrows — the ghost is the whole \
             table or it is not a denominator:\n{source}"
        );
        // The ghost is the FIRST layer, so the subset is drawn over it.
        let ghost = source
            .find("mark: rectY")
            .expect("a layer to look at in the source above");
        let filtered = source.find("filterBy:").expect("the subset layer");
        assert!(ghost < filtered, "the subset must be drawn last:\n{source}");
        // Its ink is the design system's, not a literal — a palette
        // regeneration has to move it.
        assert!(
            source.contains(&format!("fill: \"{}\"", GHOST_INK.hex())),
            "{source}"
        );
        assert_eq!(
            GHOST_INK,
            meridian_design::scales::GRAY_LIGHT[7],
            "the ghost's ink is a scale step, so it cannot be typed by hand"
        );
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
            source.contains("# region: no trusted label, and DuckDB stored it as VARCHAR → ranked-category-bars"),
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

    /// **A timestamp's tile draws bars, counted in pixels** — and the same bars
    /// the dates draw.
    ///
    /// The reason this is a raster reading and not a structural one is the
    /// whole card: a `TIMESTAMP` bound to a band axis produced a spec that
    /// parsed, lowered, executed and exited 0 with **no** bars on it. Every
    /// check short of the pixels passed on it.
    ///
    /// The comparison is against a `DATE` column over the same six days rather
    /// than against a number. Both dashboards draw six bands of the same
    /// counts, so the mark ink is the same ink — and a figure written into this
    /// comment would go stale the first time a bar's corner radius moved.
    #[test]
    fn a_resampled_timestamp_draws_the_bars_a_date_draws() {
        let dir = std::env::temp_dir().join(format!("bf-dashboard-ink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory to write the fixtures into");
        let days = [
            "2020-01-01",
            "2020-01-01",
            "2020-01-02",
            "2020-01-03",
            "2020-01-03",
            "2020-01-06",
        ];
        let dates = dashboard_mark_ink(&dir, "DATE", &days.map(str::to_string));
        assert!(
            dates > 1_000,
            "the harness draws nothing at all, so nothing below is evidence"
        );
        let instants = days.map(|d| format!("{d} 00:00:00"));
        let stamps = dashboard_mark_ink(&dir, "TIMESTAMP", &instants);
        assert_eq!(
            stamps, dates,
            "a resampled timestamp draws bars a date does not, or none at all"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pixels of default mark ink in the dashboard a one-column table of
    /// `values` opens as, with the column cast to `type_name`.
    ///
    /// Written out as a CSV and read back by the engine, so what is measured is
    /// the artefact [`Dashboard::to_spec`] emits — its `data:` block included,
    /// which is where the bucket column is declared and therefore where a
    /// resampled tile would fail to find its band.
    fn dashboard_mark_ink(dir: &Path, type_name: &str, values: &[String]) -> u64 {
        let csv = dir.join(format!("{type_name}.csv"));
        std::fs::write(&csv, format!("t\n{}\n", values.join("\n"))).expect("write the fixture");
        let profile = ColumnProfile {
            name: "t".to_string(),
            type_name: type_name.to_string(),
            non_null: values.len() as u64,
            nulls: 0,
            distinct: values.len() as u64,
            min: values.first().cloned(),
            max: values.last().cloned(),
            semantic: SemanticType::NotAsked,
            moments: None,
        };
        let source = Dashboard::of(&csv, std::slice::from_ref(&profile)).to_spec();
        let composed = crate::pipeline::compose_spec_str(&source, None)
            .unwrap_or_else(|e| panic!("{type_name}: {e}\n{source}"));
        let png = dir.join(format!("{type_name}.png"));
        crate::capture::capture_vello_only(composed, 1.0, &png).expect("export");
        let want = meridian_design::viz::MARK_DEFAULT_LIGHT;
        let want = [
            (want.r * 255.0).round() as i32,
            (want.g * 255.0).round() as i32,
            (want.b * 255.0).round() as i32,
        ];
        let img = image::open(&png).expect("open png").to_rgba8();
        img.pixels()
            .filter(|p| (0..3).all(|c| (i32::from(p.0[c]) - want[c]).abs() <= 20))
            .count() as u64
    }
}
