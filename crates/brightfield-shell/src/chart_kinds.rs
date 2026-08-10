//! The shell's chart vocabulary, as data — **the registry the running binary
//! reads**, not a fixture.
//!
//! [`brightfield_workbench::registry::ChartKind`] makes a chart kind a value:
//! an icon, a gloss, the slots it takes and a builder that turns bound columns
//! into a spec. [`registry`] is the instance this **process** reads, as opposed
//! to one a test stands up: [`crate::data_file`] chooses a first look out of it
//! and emits that kind's spec, and [`crate::app::ChartDoc`] hands it to the
//! chart pane through [`brightfield_workbench::item::ModuleHost`]. Both routes
//! are held by tests that take a kind away and watch the outcome change.
//!
//! # What a kind's spec *is* here
//!
//! Spec **source** — the body of a Brightfield YAML document, ready to have a
//! `meta:` and a `data:` block written above it. `String` is the spec type for
//! the reason [`crate::ranked_bars::chart_kind`] already chose it: the thing a
//! chart kind produces in this shell is a document the composer parses, and a
//! structured intermediate would be a second spec language to keep in step with
//! the first.
//!
//! Three consequences, and each is a contract a new kind has to keep:
//!
//! - the source is a **self-contained top-level fragment**: the picture's
//!   `plot:` or `hconcat:` key, plus whatever else that picture's instructions
//!   need declared beside them at the top level — a `params:` entry for a
//!   selection its interactors bind, say. So the caller can concatenate it
//!   under a `data:` block without knowing which kind built it;
//! - it **loads clean**: the composed document's diagnostics carry nothing
//!   advisory, because the window raises those as a banner over the picture. A
//!   block whose interactor binds an undeclared param draws a chart and tells
//!   the reader, in the same frame, that one of its instructions had no effect.
//!   `no_kinds_block_asks_for_something_the_load_says_had_no_effect` is what
//!   says so;
//! - it reads the source named [`crate::data_file::SOURCE`], because this
//!   registry's kinds are the ones the shell offers over the **one** table it
//!   synthesises a document for. A kind wanting a different table takes its
//!   own emitter, the way [`crate::ranked_bars::RankedCategoryBars::plot_yaml`]
//!   does.
//!
//! # Declaration order is the preference order
//!
//! [`ChartKindRegistry::applicable`] answers in declaration order, and a caller
//! with no opinion takes the first. So the order below is the product
//! judgement about what a table nobody has described should open as, stated
//! once, where the kinds are.

use std::fmt::Write as _;
use std::sync::OnceLock;

use brightfield_engine::ColumnProfile;
use brightfield_workbench::registry::{
    ChartKind, ChartKindId, ChartKindRegistry, Field, FieldSlot, FieldType,
};
use brightfield_workbench::Icon;

use crate::data_file::SOURCE;

/// A numeric column's distribution: `rectY` over its bin edges, counted.
pub const BINNED_HISTOGRAM: ChartKindId = ChartKindId::new("binned-histogram");
/// Two categories crossed and counted: `cell` over a pair of band axes.
pub const COUNT_GRID: ChartKindId = ChartKindId::new("count-grid");

/// The widest axis this registry will cross: a `distinct × distinct` grid past
/// this on either side is a wall of cells rather than a picture.
///
/// A property of the **field list**, not of a slot — a slot declares types, and
/// "too many categories to read" is a cardinality. So it is applied by
/// [`fields_of`], which is where the column profiles are.
const GRID_MAX_DISTINCT: u64 = 60;

/// The one column a histogram bins.
///
/// A single required slot, so [`ChartKind::accepts`] answers yes for any table
/// carrying a measure and no for a table of names — the applicability rule the
/// first-look chooser needs, as data rather than as a branch.
const HISTOGRAM_SLOTS: &[FieldSlot] = &[FieldSlot::required("x", &[FieldType::Quantitative])];

/// The ink the ghost layer is drawn in — the warm-gray border step of the
/// design system's generated gray scale,
/// [`meridian_design::scales::GRAY_LIGHT`].
///
/// A token rather than a hex constant, so a palette regeneration moves the
/// ghost with the rest of the chart's ink instead of leaving it behind. The
/// emitter spells it out with [`meridian_design::colour::Rgba::hex`], which
/// round-trips the scale's own 8-bit channels exactly — so the colour reaching
/// the canvas is the token's, not an approximation of it.
///
/// **This step rather than a lighter one**, and a pixel test is what decides
/// it: the reading in `crates/brightfield-shell/tests/ghosted_histogram.rs`
/// tells ghost ink from chart chrome by per-channel distance, and the plot
/// frame's own baseline is drawn from a step of this same scale. A ghost close
/// enough to that step to sit inside the tolerance would turn the reading into
/// a reading of the gridlines, so
/// `the_registrys_ghost_ink_is_not_the_charts_own_chrome` holds the separation
/// rather than this comment asserting it.
const GHOST_INK: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[7];

/// The crossfilter selection this registry's blocks drive and read.
///
/// **One name for every kind**, and that is the point of putting it here rather
/// than in each builder: two blocks composed into one document cross-filter
/// each other only while they name the same selection. Two private names would
/// compose into a dashboard whose tiles each filtered nothing but themselves —
/// and self-exclusion means that is a dashboard where brushing does nothing at
/// all.
///
/// The name is arbitrary and the **declaration** is not: a block writing `as:
/// $sel` on an interactor has to declare `sel` under `params:`, because an
/// interactor binding a name no `params:` entry declares raises
/// [`brightfield_spec::ParseWarning::InteractorBindingMissing`] — which the
/// window puts on screen as a *"had no effect"* banner over the picture it has
/// just drawn. [`crate::ranked_bars::Dashboard::to_spec`] declares the same
/// entry for the same reason.
const SELECTION: &str = "sel";

/// The two axes a count grid crosses. Both required: one category is a bar
/// chart, not a grid.
const GRID_SLOTS: &[FieldSlot] = &[
    FieldSlot::required("x", &[FieldType::Categorical]),
    FieldSlot::required("y", &[FieldType::Categorical]),
];

/// The shell's chart kinds, in preference order.
///
/// Built once per process. A `OnceLock` rather than a `const`: a
/// [`ChartKind`]'s `controls` is a function and its description a `&'static
/// str`, but [`ChartKindRegistry::new`] takes a `Vec` and asserts ids are
/// unique — which is a run-time check, deliberately (see its docs), so the
/// registry is a run-time value.
#[must_use]
pub fn registry() -> &'static ChartKindRegistry<String> {
    static KINDS: OnceLock<ChartKindRegistry<String>> = OnceLock::new();
    KINDS.get_or_init(|| {
        ChartKindRegistry::new(vec![
            binned_histogram(),
            count_grid(),
            ranked_category_bars(),
        ])
    })
}

/// The kind registered for `id`, if this build has one.
#[must_use]
pub fn find(id: ChartKindId) -> Option<&'static ChartKind<String>> {
    registry().find(id)
}

/// A numeric column's distribution, **ghosted**: the unfiltered total behind
/// the filtered subset.
///
/// Two `rectY` layers over one table and one `x: { bin: }` / `y: { count: }`
/// transform — the lift
/// [`brightfield_spec::vocab::MarkKind::bins_positionally`] recognises, so the
/// aggregation happens in SQL and the picture is of the whole table rather than
/// of a sample of it. The first layer reads [`SOURCE`] straight and never
/// narrows; the second reads it through `filterBy:` the crossfilter selection
/// and lands on top in the default mark ink.
///
/// # Why two layers rather than one filtered one
///
/// Both layers share the plot's scales, so the count axis and the pixel mapping
/// are fixed by the total. A brushed tile therefore reads as a fraction of the
/// bars behind it. One filtered layer draws a perfectly good histogram after a
/// brush — right bars, right counts, rescaled axis — and gives the reader no
/// way to see what fraction of the data it is; it reads as a chart that redrew
/// itself. `examples/rect-bin-count-ghost.yaml` is the same device authored by
/// hand, and its header comment is the long form of this paragraph.
///
/// The alternative device is `select: highlight`, which draws the selected part
/// inside a single unfiltered bar (`examples/rect-bin-count-part-of-whole.yaml`).
/// It is not interchangeable with this one: it deemphasises non-matching rows
/// within one layer, so the ghost and the subset cannot be read as two
/// separately-scaled quantities and a bin the selection empties keeps no
/// standing bar of its own.
///
/// # Why the plot also brushes
///
/// `select: intervalX` makes the tile a contributor to [`SELECTION`] and not
/// only a subscriber to it. A sweep resolves to an interval over the binned
/// column: `x: { bin: col }` draws on an axis in `col`'s own units, so a pixel
/// range on it inverts to a `col` range. `brightfield-spec`'s
/// `positional_column` is what reads that column out of the bin transform, and
/// `tests/ghosted_histogram.rs` drives a pointer sweep through the whole path
/// to the committed clause.
///
/// Without the interactor nothing in the document this kind composes can write
/// [`SELECTION`], so the second layer's `filterBy:` never narrows, the two
/// layers stay identical and the ghost is decoration.
///
/// A sweep here does not move this tile's own bars, and that is the design
/// rather than a dead control. Crossfilter self-exclusion drops a plot's own
/// clause from its own query, so the tile keeps its whole distribution while
/// whatever else subscribes to [`SELECTION`] narrows.
fn binned_histogram() -> ChartKind<String> {
    ChartKind {
        id: BINNED_HISTOGRAM,
        icon: Icon("chart-bar"),
        description:
            "Bins a measure and counts the rows in each bin, the total behind the selection",
        slots: HISTOGRAM_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let column = yaml_quoted(bound.name("x").unwrap_or_default());
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            out.push_str("plot:\n");
            // The ghost, first so the subset covers it: the whole table, with
            // no `filterBy:` to narrow it.
            let _ = writeln!(out, "  - mark: rectY");
            let _ = writeln!(out, "    data: {{ from: {SOURCE} }}");
            let _ = writeln!(out, "    x: {{ bin: {column} }}");
            let _ = writeln!(out, "    y: {{ count: }}");
            let _ = writeln!(out, "    fill: \"{}\"", GHOST_INK.hex());
            // The subset: the same transform, through the selection, in the
            // mark ink a layer binding no colour channel takes.
            let _ = writeln!(out, "  - mark: rectY");
            let _ = writeln!(
                out,
                "    data: {{ from: {SOURCE}, filterBy: ${SELECTION} }}"
            );
            let _ = writeln!(out, "    x: {{ bin: {column} }}");
            let _ = writeln!(out, "    y: {{ count: }}");
            let _ = writeln!(out, "  - select: intervalX");
            let _ = writeln!(out, "    as: ${SELECTION}");
            out
        },
    }
}

/// Two categories crossed and counted.
///
/// The answer for a table with no distribution to draw: `cell` over two band
/// axes with a counted fill, which is the shape a table of names admits.
fn count_grid() -> ChartKind<String> {
    ChartKind {
        id: COUNT_GRID,
        icon: Icon("chart-bar"),
        description: "Crosses two categories and counts the rows in each cell",
        slots: GRID_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let x = bound.name("x").unwrap_or_default();
            let y = bound.name("y").unwrap_or_default();
            let mut out = String::from("plot:\n");
            let _ = writeln!(out, "  - mark: cell");
            let _ = writeln!(out, "    data: {{ from: {SOURCE} }}");
            let _ = writeln!(out, "    x: {}", yaml_quoted(x));
            let _ = writeln!(out, "    y: {}", yaml_quoted(y));
            let _ = writeln!(out, "    fill: {{ count: }}");
            out
        },
    }
}

/// [`crate::ranked_bars::chart_kind`] over this registry's source, as a
/// self-contained top-level fragment.
///
/// The declaration — id, icon, gloss, slot — is that module's, taken whole;
/// only the builder differs, and only in the three things this registry's spec
/// contract fixes that a placeholder cannot: the table the module counts over,
/// the `hconcat:` key that makes one module a document, and the `params:` entry
/// declaring the selection its interactors bind (see
/// [`SELECTION`]). Rebuilding the declaration here instead would be
/// a second copy of it, which is what a registry exists to end.
fn ranked_category_bars() -> ChartKind<String> {
    ChartKind {
        build: |bound, _options| {
            let column = bound.name("category").unwrap_or_default();
            let module = crate::ranked_bars::RankedCategoryBars::new(column);
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            let _ = writeln!(out, "hconcat:");
            out.push_str(&module.plot_yaml(SOURCE, SELECTION, 2));
            out
        },
        ..crate::ranked_bars::chart_kind()
    }
}

// ---------------------------------------------------------------------------
// Columns → fields
// ---------------------------------------------------------------------------

/// The columns of a profiled table as fields a chart kind can be chosen for,
/// in the order a chooser should offer them.
///
/// Two decisions live here rather than in a slot, because a slot declares
/// *types* and both of these are about the values in a column:
///
/// - **eligibility.** A column is offered when its name survives both the
///   emitted SQL and the synthesised YAML unchanged, when it has a non-null
///   value, and when it has more than one distinct value — a constant column
///   bins to a single bar and crosses to a single row, which is a true picture
///   of nothing. A category is offered only up to the private
///   `GRID_MAX_DISTINCT` ceiling above.
/// - **order.** Measures first, in the table's own order; then categories,
///   fewest distinct values first. [`ChartKind::bind`] is first-fit in slot
///   order, so this is what decides *which* column fills a slot once the kind
///   is chosen. `sort_by_key` is stable, so two categories of equal width keep
///   the table's own order rather than an arbitrary one.
///
/// A column's [`FieldType`] is decided by whether DuckDB stored it as a type
/// the bin arithmetic can subtract and take a logarithm of — the private
/// `is_binnable_type` below. Everything else is a category, dates included: a
/// date is a perfectly good grid axis and has no logarithm.
#[must_use]
pub fn fields_of(columns: &[ColumnProfile]) -> Vec<Field> {
    let usable = || {
        columns
            .iter()
            .filter(|c| nameable(&c.name))
            .filter(|c| c.non_null > 0 && c.distinct > 1)
    };

    let mut fields: Vec<Field> = usable()
        .filter(|c| is_binnable_type(&c.type_name))
        .map(|c| Field::new(&c.name, FieldType::Quantitative))
        .collect();

    let mut categories: Vec<&ColumnProfile> = usable()
        .filter(|c| !is_binnable_type(&c.type_name))
        .filter(|c| c.distinct <= GRID_MAX_DISTINCT)
        .collect();
    categories.sort_by_key(|c| c.distinct);
    fields.extend(
        categories
            .into_iter()
            .map(|c| Field::new(&c.name, FieldType::Categorical)),
    );

    fields
}

/// Whether a column name can be written into both the emitted SQL and the
/// synthesised YAML without changing what it names.
///
/// The emitted SQL quotes identifiers with `"`, and the spec is written as
/// YAML, so a name carrying a `"` or a control character has no faithful
/// spelling in either — and silently drawing a *different* column would be
/// worse than drawing none.
fn nameable(name: &str) -> bool {
    !name.is_empty() && !name.contains('"') && !name.chars().any(char::is_control)
}

/// Whether a DuckDB column type can be **binned** — strictly the numeric
/// types.
///
/// Temporal types are deliberately out, and the exclusion is load-bearing
/// rather than cautious: the bin scheme is arithmetic (`max - min`, then a
/// logarithm of the span), and subtracting two DuckDB `DATE`s yields an
/// `INTERVAL`, which has no logarithm.
fn is_binnable_type(duckdb_type: &str) -> bool {
    let upper = duckdb_type.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    matches!(
        base,
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "FLOAT"
            | "REAL"
            | "DOUBLE"
            | "DECIMAL"
            | "NUMERIC"
    )
}

/// `value` as a YAML single-quoted scalar.
///
/// Single-quoted rather than double-quoted because the only escape a
/// single-quoted YAML scalar has is a doubled quote — no backslashes, no
/// interpretation — so a column name full of punctuation survives verbatim.
/// What single-quoting cannot carry is a **line break**, which [`nameable`]
/// keeps out of the field list above.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};
    use brightfield_workbench::registry::audit_chart_kinds;

    fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
        ColumnProfile {
            name: name.to_string(),
            type_name: type_name.to_string(),
            non_null: 100,
            nulls: 0,
            distinct,
            min: None,
            max: None,
            semantic: brightfield_engine::SemanticType::NotAsked,
        }
    }

    /// The registry the running binary reads passes the workbench's own
    /// conformance gate — every kind's icon, gloss, slots and builder.
    #[test]
    fn the_shipped_registry_passes_the_audit() {
        audit_chart_kinds(registry()).expect("the shell's chart kinds are well-formed");
    }

    /// **The kinds this build ships, named.** A registry is a list, and a list
    /// nobody enumerates grows an entry that nothing downstream expects — so
    /// the set is pinned here and adding one is a deliberate edit of this line.
    #[test]
    fn the_registry_ships_these_kinds_in_this_order() {
        assert_eq!(
            registry().ids(),
            vec![BINNED_HISTOGRAM, COUNT_GRID, crate::ranked_bars::KIND_ID],
            "declaration order is the preference order a chooser reads"
        );
    }

    /// Every kind declares **no** control.
    ///
    /// Not decoration: the chart pane rebuilds its
    /// [`brightfield_workbench::item::ChartModule`] from the document each
    /// frame, which is only sound while a module's own state is a function of
    /// the document. A [`brightfield_workbench::registry::ModuleControl`] is
    /// state a *user* sets, so the first kind to declare one has to be met by
    /// the pane holding its module across frames — and this is the test that
    /// says so at the moment it happens.
    #[test]
    fn no_kind_declares_a_control_that_the_pane_would_have_to_remember() {
        for kind in registry().kinds() {
            assert!(
                (kind.controls)().is_empty(),
                "{}: declares a control, so the chart pane can no longer rebuild \
                 its module every frame — hold the module instead",
                kind.id
            );
        }
    }

    /// Every kind builds **a document**: the source it emits parses on its
    /// own once a `data:` block is written above it. That is the spec contract
    /// this registry states, and it is what lets a caller concatenate a block
    /// without knowing which kind built it.
    #[test]
    fn every_kind_builds_a_block_that_parses_under_a_data_header() {
        for kind in registry().kinds() {
            let source = document_of(kind, FILE_HEADER);
            let parsed = parse_spec(&source, Format::Yaml);
            assert!(parsed.is_ok(), "{}: {parsed:?}\n{source}", kind.id);
        }
    }

    /// **No kind's block asks for something the load then says had no
    /// effect.** A picture the reader is shown must not arrive under a sentence
    /// saying part of it did nothing.
    ///
    /// Held on the composed document's [`LoadDiagnostics`], because that is the
    /// object the window turns into a banner: `MeridianApp::say_load_diagnostics`
    /// raises one `Severity::Warning` over the advisories and one
    /// `Severity::Error` over the blocking ones, so an advisory a kind's own
    /// builder earns is a sentence a user reads about a file they merely
    /// opened, with no spec of theirs to correct.
    ///
    /// **Composed, not parsed** — the binding checks that produce these live in
    /// `analyse_spec`, which `parse_spec` does not run. A version of this test
    /// written on `parse_spec`'s warnings stayed green on the very block that
    /// prompted it: the ranked-bars block's `toggleY` and `highlight` bind
    /// `$sel`, and until the builder declared `sel` under `params:` a
    /// one-category CSV opened under *"1 instruction … had no effect"*.
    #[test]
    fn no_kinds_block_asks_for_something_the_load_says_had_no_effect() {
        for kind in registry().kinds() {
            let source = document_of(kind, INLINE_ROWS);
            let composed = crate::pipeline::compose_spec_str(&source, None)
                .unwrap_or_else(|e| panic!("{}: {e}\n{source}", kind.id));
            let found: Vec<String> = composed
                .diagnostics
                .advisory()
                .iter()
                .map(ToString::to_string)
                .collect();
            assert!(
                found.is_empty(),
                "{}: the block it builds earns an advisory, and the window puts \
                 every one of these over the picture: {found:?}\n{source}",
                kind.id
            );
        }
    }

    /// Rows carrying the columns [`document_of`] binds, under the name every
    /// kind reads. `c0` is numeric so it fills a quantitative slot; both
    /// columns serve as band axes.
    const INLINE_ROWS: &str = "\
data:
  opened:
    - { c0: 1, c1: north }
    - { c0: 4, c1: north }
    - { c0: 9, c1: south }
    - { c0: 16, c1: east }
";

    /// A file-backed `data:` header — enough to parse against, and it opens no
    /// file because parsing does not read one.
    const FILE_HEADER: &str = "data:\n  opened:\n    file: 'rows.csv'\n";

    /// The document `kind` builds over its own required slots, under `data`.
    fn document_of(kind: &ChartKind<String>, data: &str) -> String {
        let fields: Vec<Field> = kind
            .slots
            .iter()
            .filter(|s| s.required)
            .enumerate()
            .map(|(i, s)| Field::new(format!("c{i}"), s.accepts[0]))
            .collect();
        let binding = kind.bind(&fields).expect("its own required slots bind");
        let block = kind
            .spec(&binding, &kind.options())
            .expect("its own builder runs");
        format!("{data}{block}")
    }

    /// A measure beats a cross-tabulation, and the field order decides which
    /// column fills the slot.
    #[test]
    fn a_table_with_a_measure_opens_on_its_distribution() {
        let fields = fields_of(&[
            column("region", "VARCHAR", 4),
            column("amount", "DOUBLE", 900),
        ]);
        assert_eq!(
            registry().applicable(&fields).first().copied(),
            Some(BINNED_HISTOGRAM)
        );
        let kind = find(BINNED_HISTOGRAM).expect("shipped");
        let block = kind
            .spec(&kind.bind(&fields).expect("binds"), &kind.options())
            .expect("builds");
        assert!(block.contains("x: { bin: 'amount' }"), "{block}");
    }

    /// A table of names with no distribution crosses its two narrowest
    /// categories — narrowest first, which is what the field order carries.
    #[test]
    fn a_table_of_names_crosses_its_two_narrowest_categories() {
        let fields = fields_of(&[
            column("city", "VARCHAR", 40),
            column("region", "VARCHAR", 4),
            column("day", "DATE", 12),
        ]);
        assert_eq!(
            registry().applicable(&fields).first().copied(),
            Some(COUNT_GRID)
        );
        let kind = find(COUNT_GRID).expect("shipped");
        let block = kind
            .spec(&kind.bind(&fields).expect("binds"), &kind.options())
            .expect("builds");
        assert!(block.contains("x: 'region'"), "{block}");
        assert!(block.contains("y: 'day'"), "{block}");
    }

    /// One category and nothing else is the ranked bars' case — a table that
    /// used to admit no first look at all.
    #[test]
    fn one_category_opens_on_ranked_bars() {
        let fields = fields_of(&[column("tag", "VARCHAR", 9)]);
        assert_eq!(
            registry().applicable(&fields),
            vec![crate::ranked_bars::KIND_ID]
        );
    }

    /// The eligibility rules, each on its own column: a constant, an all-null,
    /// a name that cannot be written, and a category with too many values are
    /// each offered to no kind.
    #[test]
    fn a_column_that_cannot_be_drawn_is_not_offered() {
        let constant = column("flat", "DOUBLE", 1);
        let all_null = ColumnProfile {
            non_null: 0,
            ..column("blank", "DOUBLE", 900)
        };
        let unwritable = column("we\"ird", "DOUBLE", 900);
        let too_many = column("id", "VARCHAR", GRID_MAX_DISTINCT + 1);
        for profile in [constant, all_null, unwritable, too_many] {
            let name = profile.name.clone();
            assert!(
                fields_of(&[profile]).is_empty(),
                "{name} was offered to a chart kind"
            );
        }
        // …and the boundary is inclusive: one fewer distinct value is offered.
        assert_eq!(
            fields_of(&[column("id", "VARCHAR", GRID_MAX_DISTINCT)]).len(),
            1
        );
    }

    /// A numeric type is a measure and a temporal one is a category — the
    /// split the bin arithmetic forces, stated over the types themselves.
    #[test]
    fn a_columns_field_type_follows_what_can_be_binned() {
        for numeric in ["BIGINT", "DOUBLE", "DECIMAL(10,2)", " integer "] {
            assert_eq!(
                fields_of(&[column("v", numeric, 900)])
                    .first()
                    .map(|f| f.ty),
                Some(FieldType::Quantitative),
                "{numeric}"
            );
        }
        for other in ["VARCHAR", "DATE", "TIMESTAMP", "BOOLEAN"] {
            assert_eq!(
                fields_of(&[column("v", other, 9)]).first().map(|f| f.ty),
                Some(FieldType::Categorical),
                "{other}"
            );
        }
    }
}
