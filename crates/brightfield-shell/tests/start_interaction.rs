//! Gate: a shipped start's declared interaction is driven by a real pointer,
//! and a start that declares one nobody drives is named here rather than
//! shipped dead.
//!
//! # What went out of the door
//!
//! The bundled crosswalk chart declares `x: {bin: match_text_chars}` under an
//! `intervalX` interactor bound `as: $band`, and its own header told the reader
//! the drag committed into `$band`. It could not. The analysis layer resolved a
//! positional channel's column from a plain string channel only, so a binned
//! axis named no column, the gesture resolved to nothing, and a hand that swept
//! the histogram got a rectangle that painted and then did nothing at all.
//!
//! Nothing reddened, and the reason is worth writing down because it is a
//! property of the tests rather than of the bug. The starts were gated for
//! *rendering* — `front_door.rs` loads each one and asserts it composes, and
//! the thumbnail gate re-renders it — and the cross-filter was gated by handing
//! `Interaction::Select` or a `Predicate::Interval` straight to the seam that
//! consumes one. Every test in that arrangement is downstream of the producer,
//! so a producer that produced nothing was invisible to all of them.
//!
//! # What is held here
//!
//! - **[`a_pointer_sweep_on_every_shipped_producer_commits_a_clause`]** drives
//!   pointer down, move and up across the shipped plot's data area, through
//!   `ChartItem::drive_gestures` → `resolve_gesture` → `interval_predicate`,
//!   and asserts a clause lands in the selection the spec names and that the
//!   consumer's numbers move under it. This is the assertion the defect above
//!   fails.
//! - **[`a_producer_whose_channel_names_no_column_commits_nothing`]** is its
//!   negative control, and it exists because the assertion above is worth
//!   exactly what its counterpart is worth. It sweeps the same fixture with
//!   the one token changed that puts the brush on an axis holding an
//!   aggregate, where there is no row-level column to compare against — and
//!   the sweep commits nothing. Same harness, same data, opposite answer.
//! - **[`every_selection_a_shipped_start_produces_is_driven_here`]** is the
//!   durable half. It enumerates the selection producers of every shipped
//!   start off the shipped bytes and holds that set equal to [`DRIVEN`], so a
//!   start added with a `select:` nobody drives fails here instead of shipping.
//! - **[`what_a_shipped_starts_prose_claims_resolves`]** reads the comment
//!   lines of each shipped spec and decides the claims that can be decided:
//!   the paths they cite, the `$name`s they name, the `data.<name>`s they
//!   reference, the start ids they mention and the tests they cite. The header
//!   sentence that was false above was of exactly this family.
//!
//! # Why one of the fixtures is not the shipped bytes verbatim
//!
//! The one start that declares a producer is the one start whose data is
//! fetched, and a suite that opened an https source would be green or red for
//! reasons outside this repo. So its rows — and only its rows — are swapped
//! for a locally generated table of the same three columns.
//! [`the_twin_differs_from_the_shipped_chart_only_in_where_its_rows_come_from`]
//! is the strut: the parsed composition and the parsed params of the twin are
//! equal to the shipped ones, and the producer the pointer drives is the
//! shipped producer value-for-value. Nothing about the interaction is a local
//! invention.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use brightfield_engine::{Session, SqlPredicate};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::starts::{self, Start};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::{
    build_brushable_bindings, build_interval_sliders, build_legend_bindings, BrushKind,
};
use brightfield_spec::ast::Spec;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_sql::collect_plot_groups;
use brightfield_sql::ir::ScalarValue;

// ---------------------------------------------------------------------------
// The fixtures: the shipped bytes, and the one substitution
// ---------------------------------------------------------------------------

/// The line the one remote start declares its rows on, written out so the
/// substitution below is a named edit rather than a regex over a spec.
const REMOTE_SOURCE: &str = "    file: \"https://openlake.meridian.online/edgar_gleif.parquet\"";

/// What it is swapped for: a generated table carrying the three columns the
/// shipped `links` view reads (`tier`, `method`, and the `corpus` whose length
/// becomes the brushed axis), at a size that draws in a moment.
///
/// `tier` and `method` are cut on different strides so they vary independently
/// — the grid the brush filters has sixteen cells rather than four, which is
/// what makes "the counts moved" a claim about the predicate rather than about
/// one group vanishing.
///
/// Written with no `\` continuation after the opening quote, deliberately: a
/// Rust line continuation eats the following line's leading whitespace, and
/// the whitespace here is the YAML nesting this text is substituted into.
const LOCAL_SOURCE: &str = "    query: |
      SELECT
        (['ambiguous', 'authoritative', 'candidate', 'confirmed'])[(i % 4) + 1] AS tier,
        (['exact_name', 'jaro_winkler', 'sec-ncen', 'sec-registration'])[(i // 4 % 4) + 1] AS method,
        repeat('x', 20 + CAST(hash(i) % 270 AS BIGINT)) AS corpus
      FROM range(4000) AS t(i)";

/// The interactor token the negative control replaces, and what it becomes.
///
/// `intervalX` sweeps the binned axis, which inverts to values of the column
/// being binned. `intervalY` on the same plot sweeps `y: {count:}` — an axis
/// whose units are a group total, which no row carries — so the resolver has
/// no column to compare against and refuses. That refusal is deliberate and
/// documented at `positional_column` in `brightfield-spec`; here it is the
/// only thing standing between a passing sweep assertion and a vacuous one.
const SWEPT_AXIS: &str = "select: intervalX";
const AGGREGATE_AXIS: &str = "select: intervalY";

/// The repo root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the crate sits inside the checkout")
}

fn parse(yaml: &str) -> Spec {
    parse_spec(yaml, Format::Yaml)
        .expect("a shipped spec parses")
        .spec
}

/// The shipped bytes with the remote rows replaced by generated ones, and
/// nothing else touched.
fn hermetic_twin(shipped: &str) -> String {
    assert_eq!(
        shipped.matches(REMOTE_SOURCE).count(),
        1,
        "the remote start no longer declares its source on the line this file \
         substitutes, so the twin below would be the shipped spec unchanged \
         and every assertion over it would reach the network"
    );
    shipped.replace(REMOTE_SOURCE, LOCAL_SOURCE)
}

/// The spec text this file drives a pointer through for `start`: the shipped
/// bytes, or the hermetic twin of them for the start whose rows are fetched.
/// `None` for a start that opens no chart.
fn driven_spec(start: &Start) -> Option<String> {
    let shipped = start.spec?;
    Some(if start.remote {
        hermetic_twin(shipped)
    } else {
        shipped.to_string()
    })
}

// ---------------------------------------------------------------------------
// The producers a start declares
// ---------------------------------------------------------------------------

/// One selection producer a shipped start declares: something a hand acts on
/// that writes into a named selection.
///
/// Identity is `(start, class, from, into)` and not the binding value, because
/// this is what [`DRIVEN`] has to spell out and a reader has to be able to
/// write one by hand from a failure message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Producer {
    /// The id of the start that ships it.
    start: &'static str,
    /// Which resolve path a hand reaches it by: `brush`, `legend` or `slider`.
    /// Each is a different set of pointer events and a different resolver, so
    /// a class this file has no harness for must be visible as a class.
    class: &'static str,
    /// The component that dispatches — the contributor identity.
    from: String,
    /// The selection it writes into.
    into: String,
}

/// Every selection producer `start` declares, read off its shipped bytes with
/// no engine, no data and no network.
///
/// All three producer classes the spec grammar carries are enumerated, so a
/// start that declared a bound legend or an interval slider instead of a brush
/// would still be reported by [`every_selection_a_shipped_start_produces_is_driven_here`]
/// rather than falling through a brush-shaped filter.
fn producers_of(start: &'static Start) -> Vec<Producer> {
    let Some(text) = start.spec else {
        return Vec::new();
    };
    let spec = parse(text);
    let mut warnings = Vec::new();
    let mut out: Vec<Producer> = Vec::new();
    for b in build_brushable_bindings(&spec) {
        out.push(Producer {
            start: start.id,
            class: "brush",
            from: b.parent_plot.0.clone(),
            into: b.selection.clone(),
        });
    }
    for l in build_legend_bindings(&spec, &mut warnings) {
        out.push(Producer {
            start: start.id,
            class: "legend",
            from: l.legend_path.0.clone(),
            into: l.selection.clone(),
        });
    }
    for s in build_interval_sliders(&spec, &mut warnings) {
        out.push(Producer {
            start: start.id,
            class: "slider",
            from: s.path.0.clone(),
            into: s.selection.clone(),
        });
    }
    out.sort();
    out
}

/// Every producer in the shipped set, sorted.
fn declared_producers() -> Vec<Producer> {
    let mut all: Vec<Producer> = starts::STARTS.iter().flat_map(producers_of).collect();
    all.sort();
    all
}

/// The producers this file drives a pointer through, as
/// `(start id, class, contributor, selection)`.
///
/// Hand-written on purpose: this is the list a new start has to be added to,
/// and the failure that makes someone add it is
/// [`every_selection_a_shipped_start_produces_is_driven_here`].
const DRIVEN: &[(&str, &str, &str, &str)] =
    &[(starts::CROSSWALK_CHART, "brush", "root/hconcat[0]", "band")];

// ---------------------------------------------------------------------------
// The window, and the pointer
// ---------------------------------------------------------------------------

/// A window over one spec, with a single `egui::Context` for its whole life —
/// the arrangement `data_file.rs` and `front_door.rs` use, and for the reason
/// they record: egui resolves a pointer event against widget ids registered on
/// a previous frame.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    /// Load `spec` live, boot a window onto it and settle one frame, so the
    /// raster rect the pointer positions are measured against exists.
    fn open(spec: &str) -> Self {
        let mut live = LiveDashboard::load_str(spec, None).expect("the fixture loads live");
        let composed = live.present().expect("the fixture composes");
        let mut boot = Boot::charts(composed);
        boot.live = Some(live);
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        };
        win.settle();
        win
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
    }

    /// A pointer position at `fraction` across plot `index`'s **data area**,
    /// halfway down it, in the window coordinates the raster was presented at.
    ///
    /// The data area and not the plot rect: the margins carry the axis and its
    /// labels, and a pixel out there inverts to a value off the end of the
    /// domain — so a sweep measured over the rect would commit bounds the
    /// column never reaches.
    fn at(&self, index: usize, fraction: f64) -> egui::Pos2 {
        let doc = self.app.chart_doc();
        let raster = doc
            .raster_rect
            .expect("a settled frame presented the raster");
        let plot = &doc.composed.plots[index];
        let l = &plot.layout;
        let x = plot.rect.x + l.plot_x_start() + (l.plot_x_end() - l.plot_x_start()) * fraction;
        let y = plot.rect.y + (l.plot_y_start() + l.plot_y_end()) / 2.0;
        egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32)
    }

    /// Press at `from`, move to `to`, release — the frames a real sweep across
    /// plot `index` occupies.
    ///
    /// Three frames and not one: the gesture machine is edge-triggered on the
    /// button, so a press and a release inside one frame is a click, and the
    /// move between them is what makes this a sweep.
    fn sweep(&mut self, index: usize, from: f64, to: f64) {
        let start = self.at(index, from);
        self.run(vec![vec![
            egui::Event::PointerMoved(start),
            button_at(start, true),
        ]]);
        let end = self.at(index, to);
        self.run(vec![vec![egui::Event::PointerMoved(end)]]);
        self.run(vec![vec![button_at(end, false)]]);
        self.settle();
    }
}

/// A primary-button press or release at `pos`.
fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The channel columns the interval resolver reads for a binding of `kind`,
/// in the order it reads them.
///
/// This mirrors `interval_predicate`'s own per-axis rule, restated once so the
/// fixture and its control are judged by the same one — a control asserted
/// against the `y` channel while the resolver reads `x` is not a control,
/// which is the mistake this function exists to make unavailable. A `None`
/// entry is an axis the resolver has no column for, which is enough on its own
/// for the gesture to resolve to no predicate;
/// [`a_producer_whose_channel_names_no_column_commits_nothing`] is the test
/// that drives a sweep into that case.
fn swept_columns(kind: BrushKind, x: Option<&str>, y: Option<&str>) -> Vec<Option<String>> {
    let (x, y) = (x.map(str::to_string), y.map(str::to_string));
    match kind {
        BrushKind::IntervalX | BrushKind::PointX => vec![x],
        BrushKind::IntervalY | BrushKind::PointY => vec![y],
        BrushKind::IntervalXY | BrushKind::Point => vec![x, y],
    }
}

/// The `Interval` clauses in a predicate tree, as `(column, lo, hi)`.
///
/// Walked rather than matched at the root: how many contributors a selection's
/// clause wraps, and in what, is the selection compiler's business rather than
/// this file's.
fn intervals(predicate: &SqlPredicate) -> Vec<(String, f64, f64)> {
    match predicate {
        SqlPredicate::Interval { column, lo, hi, .. } => match (lo, hi) {
            (ScalarValue::Float(lo), ScalarValue::Float(hi)) => vec![(column.clone(), *lo, *hi)],
            _ => Vec::new(),
        },
        SqlPredicate::And(parts) | SqlPredicate::Or(parts) => {
            parts.iter().flat_map(intervals).collect()
        }
        _ => Vec::new(),
    }
}

/// The clauses the window's selections are holding, as
/// `(selection name, column, lo, hi)`.
fn held(win: &Window) -> Vec<(String, String, f64, f64)> {
    win.app
        .chart_doc()
        .live_dashboard()
        .expect("a live window")
        .selection_clauses()
        .iter()
        .flat_map(|(name, p)| {
            intervals(p)
                .into_iter()
                .map(|(c, lo, hi)| (name.clone(), c, lo, hi))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Sum of the `__bf_count` column over a mark's returned batches — the number
/// of source rows that reached the screen inside a group — or `None` for a
/// mark that emits no such column.
///
/// Read off the engine rather than off the document's record of what it asked
/// for, for the reason `ChartDoc::live_coordinator` is public: a gate that
/// asserted this from a field the code under test writes would be asserting
/// against itself.
fn counted_rows(session: &mut Session, mark: usize) -> Option<f64> {
    use arrow::array::Float64Array;
    let batches = session.execute_mark(mark).expect("the mark executes");
    let mut total = 0.0_f64;
    let mut seen = false;
    for batch in &batches {
        let Ok(idx) = batch.schema().index_of("__bf_count") else {
            continue;
        };
        seen = true;
        let column = batch
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("__bf_count is the DOUBLE the count lowerers alias");
        for i in 0..column.len() {
            total += column.value(i);
        }
    }
    seen.then_some(total)
}

/// Each mark's counted rows right now, indexed as `collect_plot_groups` indexes
/// them.
fn counts(win: &mut Window, marks: usize) -> Vec<Option<f64>> {
    let session = win
        .app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live window")
        .session_mut();
    (0..marks).map(|m| counted_rows(session, m)).collect()
}

// ---------------------------------------------------------------------------
// The strut: the twin is the shipped chart bar its rows
// ---------------------------------------------------------------------------

/// The hermetic twin is the shipped crosswalk chart with its rows generated
/// locally, and is that chart in every other respect.
///
/// Without this the sweep below would be a statement about a fixture that
/// resembles the product. With it, the composition, the params and the
/// producer the pointer drives are the shipped values, compared rather than
/// described.
#[test]
fn the_twin_differs_from_the_shipped_chart_only_in_where_its_rows_come_from() {
    let start = starts::find(starts::CROSSWALK_CHART).expect("the chart start ships");
    let shipped_text = start.spec.expect("the chart start carries a spec");
    let twin_text = hermetic_twin(shipped_text);
    assert_ne!(
        twin_text, shipped_text,
        "the substitution changed nothing, so this fixture reaches the network"
    );

    let shipped = parse(shipped_text);
    let twin = parse(&twin_text);

    assert_eq!(
        twin.root, shipped.root,
        "the twin's composition is not the shipped one, so a gesture over it \
         is a gesture over a different chart"
    );
    assert_eq!(
        twin.params, shipped.params,
        "the twin declares different params, so the selection a sweep writes \
         into is not the shipped selection"
    );
    assert_eq!(
        build_brushable_bindings(&twin),
        build_brushable_bindings(&shipped),
        "the producer the pointer drives is not the shipped producer"
    );

    let changed: Vec<&String> = shipped
        .data
        .keys()
        .filter(|k| shipped.data.get(*k) != twin.data.get(*k))
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "the substitution moved {} data sources; it may move exactly the one \
         whose rows are fetched",
        changed.len()
    );
    assert_eq!(
        shipped.data.keys().collect::<Vec<_>>(),
        twin.data.keys().collect::<Vec<_>>(),
        "the twin declares a different set of data sources"
    );
}

// ---------------------------------------------------------------------------
// The pointer, and its counterpart
// ---------------------------------------------------------------------------

/// **A hand sweeping a shipped start's plot commits a clause into the
/// selection that start's spec names, and the numbers on the other plot move.**
///
/// The gesture is real pointer input on the presented raster: press, move,
/// release, through `ChartItem::drive_gestures` → `resolve_gesture` →
/// `interval_predicate`. That path is what shipped dead, and no test in this
/// tree drove it through a bundled start until this one — the cross-filter
/// gates all begin at the seam that *consumes* a selection, which a producer
/// resolving to nothing sails straight past.
///
/// Three things are asserted, and the third is what makes the first two mean
/// something. A clause is held, over the column the binding names, with an
/// order-independent `lo < hi`; the consumer's counted rows fall, and stay
/// above zero, so the predicate reached the rows rather than emptying the
/// query; and the producing plot's own count does not move, which is the
/// crossfilter convention its header states.
#[test]
fn a_pointer_sweep_on_every_shipped_producer_commits_a_clause() {
    let with_producers: Vec<&Start> = starts::STARTS
        .iter()
        .filter(|s| !producers_of(s).is_empty())
        .collect();
    assert!(
        !with_producers.is_empty(),
        "no shipped start declares a selection producer, so this test asserts \
         nothing; if that is now true, this file and its enumeration gate are \
         what should have been deleted"
    );

    for start in with_producers {
        let text = driven_spec(start).expect("a start with a producer opens a chart");
        let spec = parse(&text);
        let groups = collect_plot_groups(&spec);
        let marks: usize = groups.iter().map(|g| g.mark_indices.len()).sum();

        let mut win = Window::open(&text);
        assert!(
            win.app.chart_doc().selection_sql().is_none(),
            "{}: a chart nobody has touched is already holding {:?}",
            start.id,
            win.app.chart_doc().selection_sql()
        );

        let producing = win
            .app
            .chart_doc()
            .composed
            .plots
            .iter()
            .position(|p| p.gesture.is_some())
            .unwrap_or_else(|| panic!("{}: no composed plot carries a gesture", start.id));
        let binding = win.app.chart_doc().composed.plots[producing]
            .gesture
            .clone()
            .expect("just found");
        let producing_path = win.app.chart_doc().composed.plots[producing].path.clone();
        let own_marks: BTreeSet<usize> = groups
            .iter()
            .filter(|g| g.plot_path == producing_path)
            .flat_map(|g| g.mark_indices.clone())
            .collect();

        let before = counts(&mut win, marks);

        // Middling fractions on purpose: far enough inside the axis that a
        // bound cannot be right by clamping to an end of the domain.
        win.sweep(producing, 0.25, 0.7);

        // What the resolver reads for this binding's kind, and therefore what
        // the committed clauses have to constrain. Taken from the composed
        // binding rather than written here, so this stays a statement about
        // the shipped spec on the day it changes.
        let declared: Vec<String> = swept_columns(
            binding.kind,
            binding.x_column.as_deref(),
            binding.y_column.as_deref(),
        )
        .into_iter()
        .collect::<Option<Vec<String>>>()
        .unwrap_or_else(|| {
            panic!(
                "{}: the plot at {producing_path} carries a live gesture whose \
                 swept axis resolves to no column at all, so nothing it \
                 dispatches could ever reach a WHERE",
                start.id
            )
        });

        let clauses = held(&win);
        assert_eq!(
            clauses.len(),
            declared.len(),
            "{}: a released sweep across the plot at {producing_path} was \
             expected to commit {} interval clause(s), one per swept axis; the \
             document holds {clauses:?}. A producer that resolves to no column \
             dispatches nothing and lands here.",
            start.id,
            declared.len()
        );
        let mut constrained: Vec<String> = clauses
            .iter()
            .map(|(_, column, _, _)| column.trim_matches('"').to_string())
            .collect();
        constrained.sort();
        let mut want = declared.clone();
        want.sort();
        assert_eq!(
            constrained, want,
            "{}: the sweep constrained {constrained:?}, and the plot's swept \
             channels resolve to {want:?}",
            start.id
        );
        for (name, column, lo, hi) in &clauses {
            assert_eq!(
                name, &binding.selection,
                "{}: the sweep wrote into ${name}, and the spec binds the \
                 interactor to ${}",
                start.id, binding.selection
            );
            assert!(
                lo < hi,
                "{}: the sweep committed the degenerate interval [{lo}, {hi}] \
                 over {column}",
                start.id
            );
        }

        let after = counts(&mut win, marks);
        let narrowed: Vec<usize> = (0..marks)
            .filter(|m| !own_marks.contains(m))
            .filter(|m| match (before[*m], after[*m]) {
                (Some(b), Some(a)) => a < b,
                _ => false,
            })
            .collect();
        assert!(
            !narrowed.is_empty(),
            "{}: {clauses:?} is held, and no mark outside the producing plot \
             returned fewer rows for it — before {before:?}, after {after:?}. \
             A committed predicate nothing re-queries is a readout, not a \
             cross-filter.",
            start.id
        );
        for m in &narrowed {
            assert!(
                after[*m].unwrap_or(0.0) > 0.0,
                "{}: mark {m} came back empty under the brush, which proves \
                 the query ran and nothing about what it means",
                start.id
            );
        }
        for m in &own_marks {
            assert_eq!(
                after[*m], before[*m],
                "{}: the producing plot's own mark {m} narrowed under its own \
                 brush — a plot does not filter itself under crossfilter, and \
                 this start's header says so",
                start.id
            );
        }
    }
}

/// **The counterpart: the same sweep, on a producer whose axis names no
/// row-level column, commits nothing.**
///
/// This is what the test above is worth. The fixture is the same twin with one
/// token changed — the interactor moved from the binned x axis to the `count:`
/// y axis — so the data, the marks, the plot geometry and the harness are
/// identical and the only difference is whether the swept channel resolves to
/// a column. It does not, `interval_predicate` returns nothing, and no
/// selection is dispatched.
///
/// That is precisely the state the shipped chart was in for as long as it
/// shipped: a live interactor over an axis the resolver could not name a
/// column for. A sweep assertion that passed in both states would be testing
/// the harness rather than the product.
#[test]
fn a_producer_whose_channel_names_no_column_commits_nothing() {
    let start = starts::find(starts::CROSSWALK_CHART).expect("the chart start ships");
    let twin = driven_spec(start).expect("the chart start opens a chart");
    assert_eq!(
        twin.matches(SWEPT_AXIS).count(),
        1,
        "the fixture no longer declares its interactor on the token this \
         control substitutes"
    );
    let control = twin.replace(SWEPT_AXIS, AGGREGATE_AXIS);

    // The two fixtures side by side, judged by the resolver's own per-kind
    // rule: the one the sweep test drives resolves every axis it sweeps, and
    // this one resolves none of them. Asserted rather than described, because
    // a "control" that still resolves a column is a second working brush and
    // would pass everything below by doing the opposite of what it claims.
    let swept_by = |yaml: &str| {
        let bindings = build_brushable_bindings(&parse(yaml));
        let [b] = bindings.as_slice() else {
            panic!("the fixture declares {} producers, not one", bindings.len())
        };
        swept_columns(b.kind, b.channels.x.as_deref(), b.channels.y.as_deref())
    };
    let working = swept_by(&twin);
    let dead = swept_by(&control);
    assert!(
        working.iter().all(Option::is_some),
        "the shipped fixture's swept axes resolve to {working:?}, so the \
         contrast below is not between a working producer and a dead one"
    );
    assert!(
        dead.iter().all(Option::is_none),
        "the control's swept axes resolve to {dead:?}, so it is not a control \
         at all — it is a second working brush"
    );

    let mut win = Window::open(&control);
    let producing = win
        .app
        .chart_doc()
        .composed
        .plots
        .iter()
        .position(|p| p.gesture.is_some())
        .expect("the control still carries a live gesture binding");
    win.sweep(producing, 0.25, 0.7);

    assert!(
        win.app.chart_doc().selection_sql().is_none(),
        "a sweep over an axis with no row-level column committed {:?}",
        win.app.chart_doc().selection_sql()
    );
    assert!(
        held(&win).is_empty(),
        "the control is holding {:?}",
        held(&win)
    );
}

// ---------------------------------------------------------------------------
// The enumeration
// ---------------------------------------------------------------------------

/// **Every selection a shipped start produces is driven by a pointer in this
/// file.**
///
/// The durable half of this file. The test above fixes the starts that ship
/// today; this is what stops the next one shipping dead — a `select:` bound
/// `as:` a selection is a producer whether or not anything drives it, and the
/// spec bytes say so without an engine or a network.
///
/// A failure here is not a bug in the product. It means a start now ships an
/// interaction nobody drives: add it to [`DRIVEN`] and give
/// [`a_pointer_sweep_on_every_shipped_producer_commits_a_clause`] whatever it
/// needs to reach it. A producer of a class this file has no harness for —
/// a bound legend, an interval slider — surfaces here as a row whose `class`
/// is not `brush`, which is a harness to write rather than a line to add.
#[test]
fn every_selection_a_shipped_start_produces_is_driven_here() {
    let declared = declared_producers();
    let mut driven: Vec<Producer> = DRIVEN
        .iter()
        .map(|(start, class, from, into)| Producer {
            start,
            class,
            from: (*from).to_string(),
            into: (*into).to_string(),
        })
        .collect();
    driven.sort();

    assert_eq!(
        declared, driven,
        "the shipped starts declare selection producers this file does not \
         drive, or drive producers they no longer declare.\n  declared: \
         {declared:#?}\n  driven:   {driven:#?}"
    );
}

// ---------------------------------------------------------------------------
// The prose
// ---------------------------------------------------------------------------

/// One decidable claim pulled out of a shipped spec's comments.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Claim {
    /// A backticked repo-relative path: `examples/remote/README.md`.
    Path(String),
    /// A `$name` the prose refers to.
    Param(String),
    /// A `data.<name>` reference.
    Source(String),
    /// A backticked token equal to a shipped start's id.
    StartId(String),
    /// A backticked token of four or more snake_case segments, read as the
    /// name of a test the prose is citing.
    Test(String),
}

/// The claims in `spec_text`'s comment lines that this file can decide.
///
/// Comment lines rather than the leading header block alone: a claim two
/// thirds of the way down a spec is a claim, and the extraction is the same.
///
/// The shape rules are drawn where they are because of what the shipped corpus
/// actually contains, and each is narrow in the direction that reports
/// nothing rather than reporting prose:
///
/// - A backticked token with a `/` and no scheme is a path. `https://` carries
///   a scheme and is skipped; so is anything with whitespace in it.
/// - A `::` path is out of scope, exactly as it is for the comment-citation
///   gate over this repo's Rust: resolving one means teaching this modules,
///   re-exports and glob imports.
/// - Four snake_case segments is the line between a test name and a column
///   name. The longest backticked column name in this corpus is three
///   (`match_text_chars`); the tests these files cite are sentences.
///   `every_shipped_starts_prose_makes_at_least_one_claim_of_each_kind` is
///   what keeps a rule that stopped matching from reading as a clean tree.
fn claims_in(spec_text: &str) -> Vec<Claim> {
    let mut out = Vec::new();
    for line in spec_text.lines() {
        let trimmed = line.trim_start();
        let Some(comment) = trimmed.strip_prefix('#') else {
            continue;
        };
        for token in backticked(comment) {
            if token.contains("://") || token.contains(char::is_whitespace) {
                continue;
            }
            if let Some(name) = token.strip_prefix("data.") {
                out.push(Claim::Source(name.to_string()));
            } else if token.contains('/') {
                out.push(Claim::Path(token.to_string()));
            } else if starts::find(&token).is_some() {
                out.push(Claim::StartId(token.to_string()));
            } else if !token.contains("::")
                && token.split('_').count() >= 4
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                out.push(Claim::Test(token.to_string()));
            }
        }
        let mut rest = comment;
        while let Some(at) = rest.find('$') {
            rest = &rest[at + 1..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(Claim::Param(name));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The backticked spans of one line, without their backticks.
fn backticked(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('`') else { break };
        out.push(rest[..close].to_string());
        rest = &rest[close + 1..];
    }
    out
}

/// Every `#[test]` function name in this crate's integration tests, read off
/// the source so the prose can cite one and be held to it.
fn test_names() -> BTreeSet<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut names = BTreeSet::new();
    collect_test_names(&dir, &mut names);
    assert!(
        !names.is_empty(),
        "no test functions were found under {}, so the citation rule below \
         would report every cited test as missing",
        dir.display()
    );
    names
}

fn collect_test_names(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_test_names(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                if let Some(after) = line.trim_start().strip_prefix("fn ") {
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        out.insert(name);
                    }
                }
            }
        }
    }
}

/// **What a shipped start's own prose claims is what its spec does.**
///
/// The header of the crosswalk chart told the reader the drag committed into
/// `$band` for as long as it could not, which is the sentence this file exists
/// because of. A gate cannot judge whether a sentence is true — most of these
/// headers explain *why*, and no check reads that. What it can do is refuse
/// the claims that name something: a path that has moved, a `$name` the spec
/// does not declare, a `data.<name>` that is not a source, a start id no build
/// ships, a test that has been renamed away.
///
/// The narrowness is the point. A rule that argued about prose would be turned
/// off inside a month.
#[test]
fn what_a_shipped_starts_prose_claims_resolves() {
    let root = repo_root();
    let tests = test_names();
    for start in starts::STARTS {
        let Some(text) = start.spec else {
            continue;
        };
        let spec = parse(text);
        for claim in claims_in(text) {
            match &claim {
                Claim::Path(p) => assert!(
                    root.join(p).exists(),
                    "{}: its prose cites {p}, which is not in the checkout",
                    start.id
                ),
                Claim::Param(name) => assert!(
                    spec.params.contains_key(name),
                    "{}: its prose names ${name}, which the spec does not \
                     declare under params:",
                    start.id
                ),
                Claim::Source(name) => assert!(
                    spec.data.contains_key(name),
                    "{}: its prose names data.{name}, which the spec does not \
                     declare under data:",
                    start.id
                ),
                Claim::StartId(id) => assert!(
                    starts::find(id).is_some(),
                    "{}: its prose names the start {id}, which this build does \
                     not ship",
                    start.id
                ),
                Claim::Test(name) => assert!(
                    tests.contains(name),
                    "{}: its prose cites the test {name}, which no test in \
                     this crate is called",
                    start.id
                ),
            }
        }
    }
}

/// The extraction above still finds each kind of claim in the shipped corpus.
///
/// Without this, a rule that quietly stopped matching would read as a tree
/// with nothing to report — the failure mode every hygiene check in this repo
/// carries a self-test for.
#[test]
fn every_shipped_starts_prose_makes_at_least_one_claim_of_each_kind() {
    let found: Vec<Claim> = starts::STARTS
        .iter()
        .filter_map(|s| s.spec)
        .flat_map(claims_in)
        .collect();
    for (kind, matched) in [
        ("path", found.iter().any(|c| matches!(c, Claim::Path(_)))),
        ("param", found.iter().any(|c| matches!(c, Claim::Param(_)))),
        (
            "source",
            found.iter().any(|c| matches!(c, Claim::Source(_))),
        ),
        (
            "start id",
            found.iter().any(|c| matches!(c, Claim::StartId(_))),
        ),
        ("test", found.iter().any(|c| matches!(c, Claim::Test(_)))),
    ] {
        assert!(
            matched,
            "the shipped starts' prose yields no {kind} claim, so that rule in \
             what_a_shipped_starts_prose_claims_resolves is now inert: {found:#?}"
        );
    }
}

/// A start's `spec:` is the bytes its click composes.
///
/// `producers_of` and `claims_in` both read the field rather than the loader,
/// so a field that named some other text would let this whole file pass over a
/// spec the product does not open. Held by loading each start that opens
/// without a network and comparing the composition against one built from the
/// field directly.
#[test]
fn the_spec_a_start_carries_is_the_spec_its_click_opens() {
    for start in starts::STARTS.iter().filter(|s| !s.remote) {
        let Some(text) = start.spec else { continue };
        let opened = starts::load(start.id).unwrap_or_else(|e| panic!("{}: {e}", start.id));
        let starts::Opened::Charts(chart) = opened else {
            panic!(
                "{}: carries a chart spec but loads a protocol document",
                start.id
            )
        };
        let mut direct = LiveDashboard::load_str(text, None)
            .unwrap_or_else(|e| panic!("{}: the carried spec does not load: {e}", start.id));
        let composed = direct
            .present()
            .unwrap_or_else(|e| panic!("{}: the carried spec does not compose: {e}", start.id));
        assert_eq!(
            chart
                .composed
                .plots
                .iter()
                .map(|p| p.path.clone())
                .collect::<Vec<_>>(),
            composed
                .plots
                .iter()
                .map(|p| p.path.clone())
                .collect::<Vec<_>>(),
            "{}: the spec on the entry composes a different set of plots from \
             the one its click opens",
            start.id
        );
        assert_eq!(
            (chart.composed.width, chart.composed.height),
            (composed.width, composed.height),
            "{}: the spec on the entry composes at a different size from the \
             one its click opens",
            start.id
        );
    }
}
