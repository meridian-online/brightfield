//! The ranked category bars module — the chart a categorical column gets.
//!
//! One device, assembled from parts that already shipped, and the assembly is
//! the whole of what lives here. There is no new mechanism in this file: the
//! ranking and the row cap are `brightfield-sql`'s `BarLowerer` reading a
//! lifted `sort:`; the total behind the subset is the per-group selected count
//! a `highlight` interactor projects, drawn by `brightfield-render`'s
//! `BarRenderer`; the cross-filter is the crossfilter selection every other
//! interactive example in `examples/` already drives.
//!
//! # The shape, and the one thing it is easy to get wrong
//!
//! A module's mark is **never filtered by the selection**. It reads the table
//! straight, groups on its own column, ranks by the aggregate and caps the
//! rows — and the selection reaches it only through `select: highlight`, which
//! puts the predicate inside a conditional `SUM` beside the aggregate instead
//! of in a `WHERE`.
//!
//! Writing `filterBy: $sel` on the mark instead would look right and draw a
//! perfectly good bar chart. What it would cost is everything the device is
//! for:
//!
//! * the bars would be the subset, so the total would leave the page and there
//!   would be nothing for the subset to read as a fraction of;
//! * the `GROUP BY` would see the filter, so the ranking would be a ranking of
//!   the subset and the bars would reorder under the pointer;
//! * a category with no selected rows would produce no group, so **selecting a
//!   category would be able to remove a different one from the axis** — and
//!   selecting against one would remove the very bar the reader is asking
//!   about.
//!
//! The axis holding still is therefore not a rule enforced somewhere; it is a
//! consequence of the predicate's position. That is the reference design's
//! trick and it is why it is copied rather than improved on.
//!
//! # What a limit means here
//!
//! [`DEFAULT_LIMIT`] rows, configurable per module through
//! [`RankedCategoryBars::limit`]. Because the cap is applied to the unfiltered
//! ranking, which categories survive it is a property of the data and not of
//! the selection — the cut does not move when the pointer does. What the cap
//! does NOT do is bound the query: the aggregation still visits every row, and
//! the `LIMIT` sits above the `ORDER BY`. It is a legibility control, not a
//! performance one.

use std::fmt::Write as _;

use brightfield_render::channel::LabelForm;
use brightfield_workbench::registry::{
    Bound, ChartKind, ChartKindId, FieldSlot, FieldType, ModuleOptions,
};
use brightfield_workbench::Icon;

/// How many bars a module draws when its author says nothing.
///
/// Ten. The number is a legibility judgement rather than a measured one: a
/// generated dashboard puts several of these modules side by side, and a
/// category axis longer than this stops being scannable at a glance in the
/// height a module gets. It is the same cap the reference corpus's own sorted
/// bar chart writes by hand (`sort: { y: -x, limit: 10 }`).
pub const DEFAULT_LIMIT: usize = 10;

/// The chart kind's stable id.
pub const KIND_ID: ChartKindId = ChartKindId::new("ranked-category-bars");

/// The slot a module fills: one categorical column, and nothing else.
///
/// One required slot and no optional ones, which is what makes
/// [`ChartKind::accepts`] answer "yes" for **every** categorical column and
/// "no" for everything else — the applicability rule a dashboard generator
/// needs, expressed as data rather than as a branch in the generator.
///
/// The value axis takes no slot because the module does not offer one: it
/// counts rows. A module measuring a second column would be a different kind
/// with a second slot, not an option on this one.
const SLOTS: &[FieldSlot] = &[FieldSlot::required("category", &[FieldType::Categorical])];

/// One ranked category bars module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedCategoryBars {
    /// The categorical column this module counts over.
    pub column: String,
    /// How many bars survive the ranking.
    pub limit: usize,
    /// The number each bar prints on itself, or `None` for no label.
    pub label: Option<LabelForm>,
    /// The plot's width in logical pixels.
    pub width: u32,
    /// The plot's height in logical pixels.
    pub height: u32,
}

impl RankedCategoryBars {
    /// A module over `column`, at [`DEFAULT_LIMIT`], labelled with its counts.
    #[must_use]
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            limit: DEFAULT_LIMIT,
            label: Some(LabelForm::Count),
            width: 360,
            height: 300,
        }
    }

    /// The same module with a different row cap.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// The same module with a different label form, or none.
    #[must_use]
    pub fn with_label(mut self, label: Option<LabelForm>) -> Self {
        self.label = label;
        self
    }

    /// The same module at a different size.
    #[must_use]
    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// This module as one entry of an `hconcat:` / `vconcat:` list, indented
    /// by `indent` spaces.
    ///
    /// `table` is the data source it counts over and `selection` the
    /// crossfilter param name it both drives and reads. Both are emitted as
    /// JSON strings, which are valid YAML double-quoted scalars — so a column
    /// named `y`, `on` or `12:00` cannot turn into a boolean, a number or a
    /// sexagesimal by being written bare.
    #[must_use]
    pub fn plot_yaml(&self, table: &str, selection: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let col = yaml_string(&self.column);
        let mut out = String::new();
        let _ = writeln!(out, "{pad}- plot:");
        let _ = writeln!(out, "{pad}  - mark: barX");
        // No `filterBy:`. See this module's header for what writing one costs.
        let _ = writeln!(out, "{pad}    data: {{ from: {} }}", yaml_string(table));
        let _ = writeln!(out, "{pad}    x: {{ count: }}");
        let _ = writeln!(out, "{pad}    y: {col}");
        let _ = writeln!(
            out,
            "{pad}    sort: {{ y: -x, limit: {limit} }}",
            limit = self.limit
        );
        if let Some(label) = self.label {
            let _ = writeln!(out, "{pad}    label: {}", label.wire_name());
        }
        // The producer: a click on a band publishes that category into the
        // shared selection.
        let _ = writeln!(out, "{pad}  - select: toggleY");
        let _ = writeln!(out, "{pad}    as: ${selection}");
        // The consumer: the predicate lands inside the conditional SUM.
        let _ = writeln!(out, "{pad}  - select: highlight");
        let _ = writeln!(out, "{pad}    by: ${selection}");
        // The plot's own attributes are siblings of `plot:`, so they sit at
        // the key's indent and NOT at the mark's — one level deeper and they
        // would be read as more options on the last interactor, which is a
        // spec that parses and does something else.
        let _ = writeln!(out, "{pad}  yLabel: {col}");
        let _ = writeln!(out, "{pad}  width: {}", self.width);
        let _ = writeln!(out, "{pad}  height: {}", self.height);
        // Category names run along the y axis and are as long as the data
        // makes them; the default margin is sized for tick numbers.
        let _ = writeln!(out, "{pad}  marginLeft: 90");
        out
    }
}

/// A generated dashboard: one [`RankedCategoryBars`] per categorical column,
/// every module driving and reading one shared crossfilter selection.
///
/// The `data:` block is the caller's, verbatim — a `file:` reference or inline
/// rows — because which table a dashboard is of is not this module's question.
#[derive(Clone, Debug)]
pub struct Dashboard {
    /// The body of the spec's `data:` block, already indented two spaces.
    pub data: String,
    /// The table every module counts over.
    pub table: String,
    /// The crossfilter param every module drives and reads.
    pub selection: String,
    /// The modules, in the order they appear across the page.
    pub modules: Vec<RankedCategoryBars>,
}

impl Dashboard {
    /// A dashboard over `table`, one module per column, all at the defaults.
    #[must_use]
    pub fn of_columns<S: AsRef<str>>(data: impl Into<String>, table: &str, columns: &[S]) -> Self {
        Self {
            data: data.into(),
            table: table.to_string(),
            selection: "sel".to_string(),
            modules: columns
                .iter()
                .map(|c| RankedCategoryBars::new(c.as_ref()))
                .collect(),
        }
    }

    /// The whole spec, as YAML source ready for the composer.
    #[must_use]
    pub fn to_spec(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "params:");
        // Unquoted, unlike a column name: a param is referred to as `$name`
        // everywhere else it appears, which admits no quoting, so quoting the
        // declaration would be the inconsistency rather than the safety.
        let _ = writeln!(out, "  {}: {{ select: crossfilter }}", self.selection);
        let _ = writeln!(out, "data:");
        out.push_str(&self.data);
        if !self.data.ends_with('\n') {
            out.push('\n');
        }
        let _ = writeln!(out, "hconcat:");
        for module in &self.modules {
            out.push_str(&module.plot_yaml(&self.table, &self.selection, 2));
        }
        out
    }
}

/// A YAML scalar that cannot be read as anything but a string.
///
/// JSON string syntax is a subset of YAML's double-quoted scalar syntax, so
/// `serde_json` is a correct quoter here and saves this crate a YAML
/// serialiser it needs for nothing else.
fn yaml_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

/// This module as a [`ChartKind`] — the registry entry a picker offers and a
/// generator chooses.
///
/// The spec type is `String`: what a module hands its host here is YAML source,
/// which is what the composer takes. The builder is a plain function of the
/// bound column, which it can be because [`ChartKind::spec`] has already
/// checked the id and re-checked the column against this kind's slots before it
/// makes the [`Bound`] the builder takes. (Those are the private `SLOTS` const
/// above — named rather than linked, since a doc link does not get to widen an
/// API.)
///
/// No [`ModuleControl`](brightfield_workbench::registry::ModuleControl): a
/// control must carry a registered keyboard verb, and inventing one is a
/// keymap change rather than a chart change. The row cap is configurable
/// through [`RankedCategoryBars::limit`] and through the `sort: { limit: }` in
/// the emitted spec; it is not yet configurable from inside a drawn module.
///
/// The kind builds a spec over a placeholder table, because a bound column is
/// all [`Bound`] carries — it names a column, not the table it came from. A
/// host with a document in hand emits through [`RankedCategoryBars::plot_yaml`]
/// with the real table instead.
#[must_use]
pub fn chart_kind() -> ChartKind<String> {
    ChartKind {
        id: KIND_ID,
        icon: Icon("chart-bar"),
        description: "Counts per category, ranked, with the total behind the selection",
        slots: SLOTS,
        controls: Vec::new,
        build: |bound: &Bound<'_>, _options: &ModuleOptions| {
            let column = bound.name("category").unwrap_or_default();
            RankedCategoryBars::new(column).plot_yaml(PLACEHOLDER_TABLE, "sel", 0)
        },
    }
}

/// The table name [`chart_kind`]'s builder writes, having none of its own.
const PLACEHOLDER_TABLE: &str = "table";

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format, ParseWarning};
    use brightfield_workbench::registry::{audit_chart_kinds, ChartKindRegistry, Field};

    /// The fixture every test here and the shipped example share.
    ///
    /// **The list-valued case, written out.** A `tag` column whose values are
    /// lists — one reading carrying several tags — is a real shape in the
    /// reference corpus, and the form that reaches a `GROUP BY` is the
    /// exploded one: one row per (reading, tag) pair. Reading 3 carries two
    /// tags and so contributes to two categories; reading 5 carries three.
    /// A count is therefore a count of CONTRIBUTIONS, not of readings, which
    /// is what a list-valued category always means and the reason the two
    /// numbers differ (8 readings, 12 rows).
    ///
    /// Unnesting a genuine list column inside the query is NOT covered — the
    /// lowerer groups by a column, not by an `UNNEST`. The fixture is the
    /// exploded table such a query would produce.
    pub(super) fn fixture_data() -> String {
        [
            "  observations:",
            "    - { reading: 1, tag: drift, band: low }",
            "    - { reading: 2, tag: drift, band: low }",
            "    - { reading: 3, tag: drift, band: high }",
            "    - { reading: 3, tag: spike, band: high }",
            "    - { reading: 4, tag: spike, band: mid }",
            "    - { reading: 5, tag: drift, band: mid }",
            "    - { reading: 5, tag: spike, band: mid }",
            "    - { reading: 5, tag: gap, band: mid }",
            "    - { reading: 6, tag: drift, band: low }",
            "    - { reading: 7, tag: gap, band: high }",
            "    - { reading: 8, tag: drift, band: low }",
            "    - { reading: 8, tag: spike, band: low }",
            "",
        ]
        .join("\n")
    }

    pub(super) fn fixture_dashboard() -> Dashboard {
        Dashboard::of_columns(fixture_data(), "observations", &["tag", "band"])
    }

    /// The parse warnings a source raises, as `variant`-ish strings.
    fn warnings(source: &str) -> Vec<String> {
        parse_spec(source, Format::Yaml)
            .expect("the generated spec parses")
            .warnings
            .iter()
            .map(|w| format!("{w:?}"))
            .collect()
    }

    /// **The generated spec is one brightfield understands whole.**
    ///
    /// Not "it parses" — a spec full of options nothing reads also parses.
    /// Every key the generator writes must have a reader, and the parser says
    /// so itself: `UnconsumedMarkOption` names a key that reaches no lowerer
    /// and no renderer, and `UnconsumedSort` names a `sort:` shape no lowerer
    /// orders by. A generator emitting either would be shipping a chart that
    /// silently ignores half its own instructions.
    #[test]
    fn every_key_the_generator_writes_has_a_reader() {
        let source = fixture_dashboard().to_spec();
        let found = warnings(&source);
        let unconsumed: Vec<&String> = found
            .iter()
            .filter(|w| w.starts_with("Unconsumed"))
            .collect();
        assert!(
            unconsumed.is_empty(),
            "the generated spec asks for something nothing reads: {unconsumed:?}\n{source}"
        );
    }

    /// The parse is clean of *every* warning, not only the unconsumed ones.
    #[test]
    fn the_generated_spec_parses_without_warnings() {
        let source = fixture_dashboard().to_spec();
        let found = warnings(&source);
        assert!(
            found.is_empty(),
            "the generated spec warns: {found:?}\n{source}"
        );
    }

    /// **The mark is not filtered by the selection**, which is the whole
    /// device — see this module's header for what a `filterBy:` here would
    /// cost. Asserted on the emitted source because that is where the mistake
    /// would be made.
    #[test]
    fn a_module_never_filters_its_own_mark_by_the_selection() {
        let source = fixture_dashboard().to_spec();
        assert!(
            !source.contains("filterBy"),
            "a module's mark must read the table straight — the selection \
             reaches it through `highlight`, inside the conditional sum:\n{source}"
        );
        assert!(
            source.contains("select: highlight"),
            "…and it must actually consume the selection:\n{source}"
        );
        assert!(
            source.contains("select: toggleY"),
            "…and drive it:\n{source}"
        );
    }

    /// One module per column, and each over its own column.
    #[test]
    fn a_dashboard_gets_one_module_per_categorical_column() {
        let dash = fixture_dashboard();
        let source = dash.to_spec();
        assert_eq!(dash.modules.len(), 2);
        assert_eq!(source.matches("mark: barX").count(), 2);
        assert!(source.contains("y: \"tag\""));
        assert!(source.contains("y: \"band\""));
    }

    /// **The default limit is a default, and the limit is a knob.** Both
    /// halves of AC1's second clause, on the emitted source; the picture half
    /// is `tests/ranked_category_bars.rs`.
    #[test]
    fn the_row_cap_defaults_and_is_configurable() {
        let default = RankedCategoryBars::new("tag").plot_yaml("t", "sel", 0);
        assert!(
            default.contains(&format!("limit: {DEFAULT_LIMIT}")),
            "a module says nothing about a cap and gets the default: {default}"
        );
        let wide = RankedCategoryBars::new("tag")
            .with_limit(3)
            .plot_yaml("t", "sel", 0);
        assert!(
            wide.contains("limit: 3"),
            "and a module that asks for another gets it: {wide}"
        );
        assert!(
            default.contains("sort: { y: -x, limit:"),
            "the cap sits inside the sort, above the ranking, so it keeps the \
             TOP rows rather than the first ones: {default}"
        );
    }

    /// The label form reaches the spec, and `None` writes no key at all —
    /// an unlabelled module has to be indistinguishable from every bar mark
    /// that predates the option.
    #[test]
    fn the_label_form_is_the_modules_to_choose() {
        let counted = RankedCategoryBars::new("tag").plot_yaml("t", "sel", 0);
        assert!(counted.contains("label: count"), "{counted}");
        let pct = RankedCategoryBars::new("tag")
            .with_label(Some(LabelForm::Percent))
            .plot_yaml("t", "sel", 0);
        assert!(pct.contains("label: percent"), "{pct}");
        let bare = RankedCategoryBars::new("tag")
            .with_label(None)
            .plot_yaml("t", "sel", 0);
        assert!(!bare.contains("label:"), "{bare}");
    }

    /// A column name YAML would read as something other than a string is
    /// quoted, so the spec names the column the caller meant.
    #[test]
    fn a_column_name_yaml_would_misread_survives() {
        for name in ["y", "on", "no", "12:00", "1.5", "null"] {
            let plot = RankedCategoryBars::new(name).plot_yaml("t", "sel", 2);
            let quoted = format!("y: \"{name}\"");
            assert!(plot.contains(&quoted), "{name} was written bare: {plot}");
            let source = format!("data:\n  t:\n    - {{ \"{name}\": a }}\nhconcat:\n{plot}");
            let parsed = parse_spec(&source, Format::Yaml);
            assert!(parsed.is_ok(), "{name}: {parsed:?}\n{source}");
        }
    }

    /// The kind passes the workbench's own conformance gate — icon, gloss in
    /// house style, a required slot, and a builder that survives its own
    /// declaration.
    #[test]
    fn the_chart_kind_passes_the_registry_audit() {
        let reg = ChartKindRegistry::new(vec![chart_kind()]);
        audit_chart_kinds(&reg).expect("the ranked category bars kind is well-formed");
    }

    /// **Applicability is the slot declaration**, which is what lets a
    /// generator ask a list of kinds rather than carry a branch per kind.
    #[test]
    fn the_kind_applies_to_a_categorical_column_and_to_nothing_else() {
        let kind = chart_kind();
        assert!(kind.accepts(&[Field::new("tag", FieldType::Categorical)]));
        for wrong in [
            FieldType::Quantitative,
            FieldType::Temporal,
            FieldType::Boolean,
        ] {
            assert!(
                !kind.accepts(&[Field::new("v", wrong)]),
                "a {wrong:?} column is not what this module counts"
            );
        }
    }

    /// The kind's builder produces the same plot the direct constructor does,
    /// so the registry route and the generator route cannot draw two
    /// different charts.
    #[test]
    fn the_kinds_builder_and_the_constructor_agree() {
        let kind = chart_kind();
        let binding = kind
            .bind(&[Field::new("tag", FieldType::Categorical)])
            .expect("a categorical column binds");
        let built = kind
            .spec(&binding, &kind.options())
            .expect("the builder runs");
        assert_eq!(
            built,
            RankedCategoryBars::new("tag").plot_yaml(PLACEHOLDER_TABLE, "sel", 0)
        );
    }

    /// The shipped example is what the generator makes.
    ///
    /// A committed file and a generator are two statements of one thing, and
    /// this is the assertion that stops them being two different things. The
    /// example is a demo a reader opens; if it drifts from what a dashboard
    /// actually gets, the demo is a lie about the product.
    #[test]
    fn the_shipped_example_is_the_generators_own_output() {
        let shipped = include_str!("../../../examples/ranked-category-bars.yaml");
        let generated = fixture_dashboard().to_spec();
        let body = shipped
            .split_once("params:")
            .map(|(_, rest)| format!("params:{rest}"))
            .expect("the example carries a params block");
        assert_eq!(
            body, generated,
            "examples/ranked-category-bars.yaml has drifted from the generator"
        );
    }

    /// The example's header is prose, not spec — so the assertion above is
    /// comparing the spec and not the comment.
    #[test]
    fn the_shipped_example_carries_its_own_explanation() {
        let shipped = include_str!("../../../examples/ranked-category-bars.yaml");
        assert!(
            shipped.starts_with('#'),
            "the example opens with the comment block every example carries"
        );
    }

    /// Nothing in the generated corpus is a parse error rather than a warning.
    #[test]
    fn a_module_over_every_fixture_column_composes() {
        for column in ["tag", "band", "reading"] {
            let dash = Dashboard::of_columns(fixture_data(), "observations", &[column]);
            let out = parse_spec(&dash.to_spec(), Format::Yaml);
            assert!(out.is_ok(), "{column}: {out:?}");
            let warnings: Vec<&ParseWarning> = out
                .as_ref()
                .map(|o| o.warnings.iter().collect())
                .unwrap_or_default();
            assert!(warnings.is_empty(), "{column}: {warnings:?}");
        }
    }
}
