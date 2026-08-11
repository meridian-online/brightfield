//! **The predicate readout** — the chart says what condition selected the rows
//! it is drawing, in the SQL it actually pushed.
//!
//! A chart that reports a count and never a condition is asking to be trusted.
//! This is the answer to "what am I looking at", and the only version of it
//! worth shipping is the one where **shown is executed** — not a re-rendering
//! of the selection, not a tidied-up paraphrase, the string itself. Assert it
//! any other way and the readout becomes a second opinion about the filter.
//!
//! Everything here is TEXT, read through the same `Item::subject` the window
//! rails from, and GPU-free. Five failures are worth a section each:
//!
//! - **The readout paraphrases.** A `temp >= 8.600000000000001` rounded to
//!   `8.6` on the way to the screen reads better and is a different predicate.
//!   Held by comparing the shown string against the store, and by handing the
//!   shown string back to DuckDB.
//! - **The readout goes stale.** A line that renders once and then stops
//!   following the gesture passes every "is it there" assertion ever written.
//!   Held by brushing twice and by clearing in between.
//! - **The readout overclaims.** Under `select: crossfilter` the brushed plot's
//!   own query OMITS its own clause, so "this is your filter" is false for
//!   exactly the plot being looked at; under `highlight` the clause DIMS rows
//!   instead of removing them. Held by asserting what the emitted SQL does with
//!   the shown clause in both shapes.
//! - **The readout will not go away.** A cleared selection that leaves the line
//!   standing is the worst of them — it is a claim about a filter that is no
//!   longer in force.
//! - **The readout has only ever met one clause shape.** An interval brush is
//!   not the only gesture: a click on a category resolves a
//!   [`SqlPredicate::Point`], which renders by cardinality — a bare equality
//!   over a QUOTED string literal for one value, a parenthesised OR chain for
//!   several, `FALSE` for none. Held by clicking real bars and by holding a
//!   multi-value clause.
//! - **The readout has only ever met one PLAN shape.** Every case above filters
//!   a row-level plan, where the clause wraps the whole query. An aggregating
//!   bar is `Order{Aggregation{Filter{Source}}}`, and `apply_selection_filter`
//!   threads the clause onto the aggregation's INPUT instead — a different
//!   place in a different string. Held by reading the rail over a ranked
//!   category bar and locating the shown clause inside its `GROUP BY`.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::{RecordBatch, Session, SqlPredicate};
use brightfield_shell::app::ChartDoc;
use brightfield_shell::chart_item::{ChartItem, PREDICATE_READOUT};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{live_spec, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::{parse_spec, Format, Spec};
use brightfield_sql::emit::{emit_query, emit_rows_query};
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::Item;

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Fixtures and reads
// ---------------------------------------------------------------------------

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The whole window over `path`, live, with a real DuckDB session behind it.
fn live_window(path: &Path) -> MeridianApp {
    let path_str = path.to_str().expect("utf-8 path");
    let (live, composed) = live_spec(path_str).expect("the fixture loads live");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path.to_path_buf());
    MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
}

/// The same spec, parsed independently of the running session — the "executed"
/// side of every identity below. Re-parsed rather than borrowed from the live
/// dashboard on purpose: an emission built from the object under test would be
/// comparing the readout with itself.
fn spec_of(path: &Path) -> Spec {
    let source = std::fs::read_to_string(path).expect("read the fixture");
    parse_spec(&source, Format::Yaml).expect("parse").spec
}

fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// The chart pane's status rail, as `(id, text)` — what a reader is shown,
/// read through the same `Item::subject` the window renders from.
fn chart_rail(doc: &ChartDoc) -> Vec<(&'static str, String)> {
    ChartItem::new()
        .subject(doc)
        .status
        .iter()
        .map(|e| (e.id, e.text.clone()))
        .collect()
}

/// The predicate readout's line, if the chart is showing one.
fn readout(doc: &ChartDoc) -> Option<String> {
    chart_rail(doc)
        .into_iter()
        .find(|(id, _)| *id == PREDICATE_READOUT)
        .map(|(_, text)| text)
}

/// The SQL fragment out of a `$name = clause` readout line — the part that has
/// to be executable, byte for byte.
fn clause_of(line: &str) -> String {
    let (name, clause) = line
        .split_once(" = ")
        .unwrap_or_else(|| panic!("the readout line names its selection: {line:?}"));
    assert!(
        name.starts_with('$'),
        "the readout names the selection as the spec spells it: {line:?}"
    );
    clause.to_string()
}

/// **A real pointer drag across a plot**, pressed and released on the raster
/// the last frame actually presented — the only way to get the bounds a brush
/// really produces, which are an inverted PIXEL and therefore carry the float
/// noise a hand-written predicate never would.
///
/// `from` and `to` are fractions of the plot's width; the sweep is horizontal
/// across the middle of the plot, which is what an `intervalX` interactor reads.
fn brush_x(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, from: f32, to: f32) {
    let raster = app
        .chart_doc()
        .raster_rect
        .expect("a settled frame presented the raster");
    let rect = app.chart_doc().composed.plots[plot].rect;
    let at = |fraction: f32| {
        egui::pos2(
            raster.min.x + rect.x as f32 + rect.width as f32 * fraction,
            raster.min.y + rect.y as f32 + rect.height as f32 * 0.5,
        )
    };
    let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(at(from)), button(at(from), true)],
    );
    frame(app, ctx, vec![egui::Event::PointerMoved(at(to))]);
    frame(app, ctx, vec![button(at(to), false)]);
    frame(app, ctx, Vec::new());
}

/// **A real pointer click on a plot** — pressed and released on the same pixel,
/// which is what the gesture router's click slop separates from a one-pixel
/// sweep, and the only way to get the value a `toggleX` interactor really
/// resolves out of the band scale it was drawn with.
///
/// `at` is a fraction of the plot's width; the click lands halfway down the
/// plot, inside the clicked category's bar.
fn click_x(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, at: f32) {
    let raster = app
        .chart_doc()
        .raster_rect
        .expect("a settled frame presented the raster");
    let rect = app.chart_doc().composed.plots[plot].rect;
    let pos = egui::pos2(
        raster.min.x + rect.x as f32 + rect.width as f32 * at,
        raster.min.y + rect.y as f32 + rect.height as f32 * 0.5,
    );
    let button = |pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(app, ctx, vec![egui::Event::PointerMoved(pos), button(true)]);
    frame(app, ctx, vec![button(false)]);
    frame(app, ctx, Vec::new());
}

/// The row-level SQL the engine emits for `mark` under the session's CURRENT
/// selection state — the grid's read path, which shares its one
/// `compile_selection` with the chart's `emit_query`, so the WHERE in it is the
/// WHERE the chart ran.
fn emitted_rows_sql(session: &Session, spec: &Spec, mark: usize) -> String {
    emit_rows_query(spec, mark, None, Some(&live_selections(session)))
        .expect("the mark emits")
        .sql
}

/// The chart SQL the engine emits for `mark` under the session's CURRENT
/// selection state — the one that carries a `highlight` membership projection.
fn emitted_chart_sql(session: &Session, spec: &Spec, mark: usize) -> String {
    emit_query(spec, mark, None, Some(&live_selections(session)))
        .expect("the mark emits")
        .sql
}

/// The live selection store in the shape the emitters consume.
fn live_selections(session: &Session) -> Vec<(String, Vec<(String, SqlPredicate)>)> {
    session
        .current_selections()
        .iter()
        .map(|(name, contributors)| {
            (
                name.clone(),
                contributors
                    .iter()
                    .map(|(path, predicate)| (path.0.clone(), predicate.clone()))
                    .collect(),
            )
        })
        .collect()
}

fn mark_rows(doc: &mut ChartDoc, mark: usize) -> usize {
    doc.live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries")
        .iter()
        .map(RecordBatch::num_rows)
        .sum()
}

/// **Hand the shown string back to DuckDB on its own**, and count what it
/// selects: `clause` becomes the static `data.filter` of a spec built from the
/// fixture's own text — every `filterBy: $selection` in it — over the fixture's
/// own rows, and `mark` is executed for real.
///
/// The point is that nothing here goes near a selection. If the readout has
/// been rewritten on its way to the rail — rounded, re-spaced, re-parenthesised,
/// re-quoted — this is a second, independent engine run that says so in rows.
///
/// The clause goes in as a **single-quoted** YAML scalar. A clause names its
/// column as a SQL identifier, so it carries double quotes of its own, and the
/// only escape a single-quoted YAML scalar has is a doubled `'` — no
/// backslashes, nothing else interpreted — which is what lets the clause reach
/// DuckDB byte for byte. The one thing that form cannot carry is a line break,
/// which is asserted rather than assumed.
fn rows_selected_by(fixture: &Path, clause: &str, selection: &str, mark: usize) -> usize {
    assert!(
        !clause.contains('\n'),
        "a YAML single-quoted scalar cannot carry the clause's line break: {clause}"
    );
    let source = std::fs::read_to_string(fixture).expect("read the fixture");
    let probed = source.replace(
        &format!("filterBy: ${selection}"),
        &format!("filter: '{}'", clause.replace('\'', "''")),
    );
    assert_ne!(probed, source, "the probe spec substituted nothing");
    let mut live = LiveDashboard::load_str(&probed, None).expect("the probe spec loads");
    live.present().expect("the probe spec draws");
    live.coordinator()
        .chart_rows(mark)
        .expect("the probe mark queries")
        .iter()
        .map(RecordBatch::num_rows)
        .sum()
}

// ---------------------------------------------------------------------------
// 1. Shown is executed
// ---------------------------------------------------------------------------

/// **The line on screen is the string DuckDB was handed.** One drag, then two
/// identities that only hold if nothing rewrote the predicate on its way to the
/// rail: the shown clause is a verbatim substring of the emitted query, and the
/// shown clause — cut out of the rail text and handed back to the engine on its
/// own — resolves exactly the rows the filtered mark drew.
///
/// The float noise is the reason this is one test rather than two assertions
/// side by side. A brush bound is an inverted pixel, so it lands on values like
/// `8.600000000000001`; a readout that rounded to `8.6` would still look
/// right, still parse, and still very nearly select the same rows.
#[test]
fn the_predicate_shown_is_the_predicate_executed() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    assert!(
        readout(app.chart_doc()).is_none(),
        "at rest nothing is held, so nothing is claimed"
    );

    // Plot 0 brushes $brush; plot 1 is filtered by it (crossfilter: a plot
    // never filters itself), so mark 1 is where the clause lands.
    brush_x(&mut app, &ctx, 0, 0.3, 0.65);

    let line = readout(app.chart_doc()).expect("a committed brush is shown");
    let clause = clause_of(&line);
    assert!(
        line.starts_with("$brush = "),
        "the readout names the selection the spec declares: {line}"
    );
    assert!(
        clause.contains("\"temp\" >=") && clause.contains("\"temp\" <="),
        "an intervalX brush reads back as bounds on the x column: {clause}"
    );

    let spec = spec_of(&path);
    let drawn = mark_rows(app.chart_doc_mut(), 1);
    assert!(
        drawn > 0 && drawn < 12,
        "fixture check: the brush must narrow the consumer plot to some but \
         not all of the 12 rows, or the identities below prove nothing (drew {drawn})"
    );

    let session = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session();
    let sql = emitted_rows_sql(session, &spec, 1);
    assert!(
        sql.contains(&clause),
        "the shown clause is not in the executed SQL byte for byte — \
         something rewrote it on the way to the rail.\nshown:    {clause}\nexecuted: {sql}"
    );

    // …and it selects, on its own and through a second engine run that never
    // sees a selection, exactly the rows the filtered plot drew.
    assert_eq!(
        rows_selected_by(&path, &clause, "brush", 0),
        drawn,
        "the shown clause selects a different row set than the plot it is \
         supposed to describe: {clause}"
    );
}

/// **The bounds are the brush's own, digits and all.** The readout is compared
/// against the live selection store rather than against a literal, so it
/// cannot pass by being rewritten in both places — and the store's own
/// `Display` is the one the emitters interpolate.
#[test]
fn the_readout_carries_the_stores_bounds_unrounded() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    brush_x(&mut app, &ctx, 0, 0.25, 0.7);

    let line = readout(app.chart_doc()).expect("a committed brush is shown");
    let contributor = app.chart_doc().composed.plots[0].path.clone();
    let held = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session()
        .contributor_predicate("brush", &contributor)
        .expect("the brushed plot contributed to $brush")
        .to_string();

    // The store's clause appears in the rail line untouched. The one thing
    // wrapped around it is `compile_selection`'s own AND-of-one — a
    // crossfilter/intersect resolution conjoins its live contributors even
    // when there is one of them — and that pair of parentheses is in the
    // emitted SQL too, which is why it is shown rather than tidied away.
    assert!(
        line.contains(&held),
        "the rail is not showing the store's own clause.\nrail:  {line}\nstore: {held}"
    );
    assert_eq!(
        line,
        format!("$brush = ({held})"),
        "the readout is not `compile_selection`'s value of the selection"
    );
    // The digits themselves: an inverted pixel is not a round number, and a
    // readout that tidied it would be showing a predicate nothing ran. This
    // asserts the property (full f64 precision) rather than a literal, which
    // would pin the fixture's pixel geometry into an unrelated test.
    for token in held.split_whitespace() {
        if let Ok(value) = token.parse::<f64>() {
            assert_eq!(
                token,
                value.to_string(),
                "a bound was reformatted somewhere between the scale inversion \
                 and the store: {held}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. The readout follows the gesture
// ---------------------------------------------------------------------------

/// **The regression this exists to catch: a line that renders once and then
/// stops.** A readout wired to a value that is computed at boot, cached, or
/// read off a stale mirror passes every "the line is there" assertion. Two
/// different brushes must produce two different lines, and the second must be
/// the one the store now holds.
#[test]
fn a_second_brush_replaces_the_line_the_first_one_left() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    brush_x(&mut app, &ctx, 0, 0.1, 0.4);
    let first = readout(app.chart_doc()).expect("the first brush is shown");

    brush_x(&mut app, &ctx, 0, 0.55, 0.9);
    let second = readout(app.chart_doc()).expect("the second brush is shown");

    assert_ne!(
        first, second,
        "the readout did not follow the second brush — it is showing the \
         first one's clause, which is the failure nobody sees"
    );

    let contributor = app.chart_doc().composed.plots[0].path.clone();
    let held = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session()
        .contributor_predicate("brush", &contributor)
        .expect("the brushed plot contributed")
        .to_string();
    assert_eq!(
        second,
        format!("$brush = ({held})"),
        "the readout moved, but not to what the store now holds"
    );

    // Exactly one line, and it says the new predicate — a readout that appended
    // would grow a rail entry per gesture. Nothing replaces anything by id;
    // the next test says what actually holds this, and says it where it shows.
    let readouts = chart_rail(app.chart_doc())
        .into_iter()
        .filter(|(id, _)| *id == PREDICATE_READOUT)
        .count();
    assert_eq!(readouts, 1, "one readout, never a stack of them");
}

/// **What actually stops the line stacking**, asserted where it would show:
/// the rail the window really drew, over many frames and two gestures.
///
/// Worth its own test because the mechanism is easy to misremember. Nothing
/// dedups status entries by id — `status_rail` draws every entry it is handed,
/// in order — so the readout's id is a handle and not a replacement key. The
/// property is `Item::subject`'s: it takes `&self` and `&ChartDoc`, builds a
/// fresh `Subject`, and adds the readout at most once, so the rail is a
/// function of the document rather than an accumulation over frames. Two calls
/// over one document must agree, and twenty frames over each of two successive
/// selections must draw the line exactly once every time.
#[test]
fn the_readout_is_rebuilt_from_the_document_and_never_accumulates() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    brush_x(&mut app, &ctx, 0, 0.3, 0.65);

    // `subject` is a read, not an update: asking twice cannot change the answer.
    let item = ChartItem::new();
    assert_eq!(
        item.subject(app.chart_doc()).status,
        item.subject(app.chart_doc()).status,
        "a second `subject` over one document disagreed with the first"
    );

    let drawn_readouts = |app: &MeridianApp| {
        app.rail()
            .drawn
            .iter()
            .filter(|id| **id == PREDICATE_READOUT)
            .count()
    };
    for _ in 0..20 {
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(
            drawn_readouts(&app),
            1,
            "the rail drew the readout {} times over one unchanged selection",
            drawn_readouts(&app)
        );
    }
    // …and a second gesture replaces the line rather than adding one beside it.
    brush_x(&mut app, &ctx, 0, 0.55, 0.9);
    for _ in 0..20 {
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(
            drawn_readouts(&app),
            1,
            "a re-brushed chart rails {} readouts",
            drawn_readouts(&app)
        );
    }
}

/// **Clearing the selection clears the readout** (AC2), and brushing again
/// brings it back. A line left standing over a retracted selection is a claim
/// about a filter that is no longer in force.
#[test]
fn clearing_the_selection_clears_the_readout() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());

    brush_x(&mut app, &ctx, 0, 0.3, 0.65);
    assert!(readout(app.chart_doc()).is_some(), "the brush is shown");

    app.chart_doc_mut().clear_selection();
    assert_eq!(
        readout(app.chart_doc()),
        None,
        "the retracted selection is still on the rail"
    );
    assert!(
        !app.chart_doc().selection_active(),
        "fixture check: the clear actually retracted the gesture"
    );

    brush_x(&mut app, &ctx, 0, 0.2, 0.5);
    assert!(
        readout(app.chart_doc()).is_some(),
        "the readout did not come back after a clear — it was torn down \
         rather than emptied"
    );
}

// ---------------------------------------------------------------------------
// 3. The readout does not overclaim
// ---------------------------------------------------------------------------

/// **Under crossfilter there is no single "your filter".** The brushed plot's
/// own query OMITS its own clause — that is the whole point of `crossfilter` —
/// so a line saying "this is what you are filtered to" is wrong for exactly
/// the plot the reader is looking at.
///
/// This test pins the fact the wording answers to: the shown clause is in the
/// OTHER plot's SQL and NOT in the brushed plot's. `$brush = …` stays true of
/// both, because it reports what the selection holds and not what a query did
/// with it.
#[test]
fn the_brushed_plots_own_query_omits_the_clause_the_readout_shows() {
    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    brush_x(&mut app, &ctx, 0, 0.3, 0.65);

    let clause = clause_of(&readout(app.chart_doc()).expect("the brush is shown"));
    let spec = spec_of(&path);
    let session = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session();

    assert!(
        emitted_rows_sql(session, &spec, 1).contains(&clause),
        "the consumer plot does not carry the shown clause — the fixture is \
         not cross-filtering at all"
    );
    assert!(
        !emitted_rows_sql(session, &spec, 0).contains(&clause),
        "the brushed plot's own query carries its own clause; either \
         self-exclusion broke, or the readout may now say \"your filter\""
    );
}

/// **Under `highlight` the clause does not filter anything.** The same string
/// is PROJECTED as a per-row boolean and dims the rest, so the word "filter"
/// would be wrong on this spec. The readout still shows the clause — `$range`
/// does hold it — and the emitted SQL proves what it is used for.
///
/// `highlight.yaml`'s `$range` is also the as-bound-only shape: born from
/// `intervalXY as: $range`, never declared in `params:`. A readout that only
/// understood declared selections would be silent on this whole spec.
#[test]
fn a_highlight_selection_is_shown_as_held_not_as_filtering() {
    let path = example("highlight.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    brush_x(&mut app, &ctx, 0, 0.25, 0.7);

    let line = readout(app.chart_doc()).expect("an as-bound selection is shown too");
    assert!(
        line.starts_with("$range = "),
        "the readout names the as-bound selection: {line}"
    );
    let clause = clause_of(&line);

    let spec = spec_of(&path);
    let session = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session();
    let sql = emitted_chart_sql(session, &spec, 0);
    assert!(
        sql.contains(&clause),
        "the shown clause is not the projected membership.\nshown: {clause}\nsql:   {sql}"
    );
    assert!(
        sql.contains(&format!("({clause}) AS __bf_selected")),
        "the clause is used to DIM rows, not to drop them — if this stops \
         holding, the readout's wording has to be revisited.\nsql: {sql}"
    );
    assert!(
        !sql.contains(&format!("WHERE {clause}")),
        "a highlight selection reached a WHERE: {sql}"
    );
}

// ---------------------------------------------------------------------------
// 4. The empty and multi-contributor states
// ---------------------------------------------------------------------------

/// A headless live document over `yaml` — the pane's read path with no window
/// and no GPU, for the states a pointer cannot reach in one drag.
fn live_doc(yaml: &str) -> ChartDoc {
    let mut live = LiveDashboard::load_str(yaml, None).expect("load live");
    let composed = live.present().expect("present");
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    doc
}

/// Two selections whose declaration order is the REVERSE of their alphabetical
/// order — so a readout that reported the store's iteration order rather than
/// a sorted one has somewhere to go wrong.
const TWO_SELECTIONS: &str = r#"
params:
  zulu: { select: intersect }
  alpha: { select: intersect }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
hconcat:
  - plot:
      - mark: dot
        data: { from: t, filterBy: $zulu }
        x: x
        y: y
  - plot:
      - mark: dot
        data: { from: t, filterBy: $alpha }
        x: x
        y: y
"#;

/// **Nothing held, nothing said.** Not "no filter" — that is a claim about the
/// query — and not `WHERE TRUE`, which is a clause nobody drew. A one-shot
/// composition with no live session behind it says nothing either.
#[test]
fn a_chart_with_nothing_held_shows_no_readout() {
    let doc = live_doc(include_str!("../../../examples/crossfilter.yaml"));
    assert_eq!(doc.selection_sql(), None, "a rest state claims nothing");
    assert!(readout(&doc).is_none());
}

/// **Two selections, one line, sorted.** The store is a `HashMap`, so a readout
/// that reported its iteration order would reorder itself between frames and
/// read as flicker. Declaration order here is `zulu` then `alpha`; the line
/// leads with `$alpha`.
#[test]
fn several_selections_read_back_sorted_and_stable() {
    let mut doc = live_doc(TWO_SELECTIONS);
    for (name, clause) in [("zulu", "y > 15"), ("alpha", "x > 1")] {
        doc.apply_interaction(Interaction::Select {
            name: name.to_string(),
            contributor: ComponentPath("root/hconcat[99]".to_string()),
            predicate: SqlPredicate::Expr(clause.to_string()),
        });
    }

    let line = doc.selection_sql().expect("two selections are held");
    assert_eq!(
        line, "$alpha = (x > 1) · $zulu = (y > 15)",
        "two selections are said once each, sorted by name and joined the way \
         the shell joins every other multi-part status line"
    );
    for _ in 0..8 {
        assert_eq!(
            doc.selection_sql().as_deref(),
            Some(line.as_str()),
            "the order of the store's iteration reached the rail"
        );
    }
}

/// **Two contributors to one selection combine under the spec's own
/// resolution** — `compile_selection`'s work, never a second joining rule
/// invented at the readout. Under `select: crossfilter` nothing is excluded
/// here, because the value a selection holds is not any one consumer's WHERE.
#[test]
fn two_contributors_to_one_selection_read_back_combined() {
    let mut doc = live_doc(include_str!("../../../examples/crossfilter.yaml"));
    for (plot, clause) in [
        ("root/hconcat[0]", "temp > 5"),
        ("root/hconcat[1]", "temp < 30"),
    ] {
        doc.apply_interaction(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath(plot.to_string()),
            predicate: SqlPredicate::Expr(clause.to_string()),
        });
    }

    assert_eq!(
        doc.selection_sql().as_deref(),
        Some("$brush = (temp > 5 AND temp < 30)"),
        "the contributors are conjoined by the fixture's `select: crossfilter`"
    );
}

/// The pane declares the readout on the LEADING end — what the reader came
/// for, not a notice about the frame — and offers `clear-selection` as the way
/// out, which is the verb that actually retracts it. A `WithRail` dismissal
/// would hide the line while the selection went on filtering the picture.
#[test]
fn the_readout_is_leading_and_dismissed_by_the_verb_that_retracts_it() {
    use brightfield_workbench::subject::{HideAffordance, StatusSide, Tone};

    let path = example("crossfilter.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    brush_x(&mut app, &ctx, 0, 0.3, 0.65);

    let subject = ChartItem::new().subject(app.chart_doc());
    let entry = subject
        .status
        .iter()
        .find(|e| e.id == PREDICATE_READOUT)
        .expect("the readout is declared");
    assert_eq!(entry.side, StatusSide::Leading);
    assert_eq!(entry.tone, Tone::Neutral, "a condition is not an alarm");
    assert_eq!(
        entry.hide,
        HideAffordance::Verb(brightfield_workbench::Verb::new("clear-selection"))
    );

    // And the window really rails it — the pane declaring an entry nothing
    // draws is the other way this can look finished.
    frame(&mut app, &ctx, Vec::new());
    assert!(
        app.rail().drawn.contains(&PREDICATE_READOUT),
        "the window drew {:?}",
        app.rail().drawn
    );
}

// ---------------------------------------------------------------------------
// 5. The other gesture: a point selection
// ---------------------------------------------------------------------------

/// **A category click reads back as its own equality, quotes and all.** The
/// gesture the interval brush is not: `toggleX` over a band scale resolves the
/// clicked category and dispatches a [`SqlPredicate::Point`], which renders a
/// single value as `"region" = 'North'` — a quoted identifier against a QUOTED
/// string literal, which is the part an interval brush's bare numbers never
/// exercised.
///
/// The same two identities as the interval case, because they are the ones
/// that matter: the shown clause is a verbatim substring of the consumer's
/// emitted SQL, and handed back to a second engine on its own it selects the
/// rows the consumer plot drew. A readout that re-quoted the literal — or
/// dropped the quotes, or escaped them — fails both.
#[test]
fn a_category_click_reads_back_as_the_clicked_values_own_equality() {
    let path = example("point-select-categorical.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    assert!(
        readout(app.chart_doc()).is_none(),
        "at rest nothing is held, so nothing is claimed"
    );

    // The three bars sit at a quarter, the middle and five sixths of the plot's
    // width; this one is the first of them.
    click_x(&mut app, &ctx, 0, 0.25);

    let line = readout(app.chart_doc()).expect("a committed click is shown");
    assert_eq!(
        line, "$pick = (\"region\" = 'North')",
        "the readout is not the clicked category's own equality"
    );
    let clause = clause_of(&line);

    // The store's own clause, verbatim — a single-value `Point` is the bare
    // equality and NOT the OR chain several values would render as. The one
    // thing wrapped around it is `compile_selection`'s AND-of-one, exactly as
    // in the interval case.
    let contributor = app.chart_doc().composed.plots[0].path.clone();
    let held = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session()
        .contributor_predicate("pick", &contributor)
        .expect("the clicked plot contributed to $pick")
        .to_string();
    assert_eq!(held, "\"region\" = 'North'");
    assert_eq!(line, format!("$pick = ({held})"));

    let spec = spec_of(&path);
    let drawn = mark_rows(app.chart_doc_mut(), 1);
    assert_eq!(
        drawn, 2,
        "fixture check: the clicked region has two rows in the detail plot"
    );
    let session = app
        .chart_doc_mut()
        .live_coordinator()
        .expect("a live document")
        .session();
    let sql = emitted_rows_sql(session, &spec, 1);
    assert!(
        sql.contains(&clause),
        "the shown clause is not in the executed SQL byte for byte — \
         something re-quoted it on the way to the rail.\nshown:    {clause}\nexecuted: {sql}"
    );
    assert_eq!(
        rows_selected_by(&path, &clause, "pick", 1),
        drawn,
        "the shown clause selects a different row set than the plot it is \
         supposed to describe: {clause}"
    );

    // And the line follows the next click, as it follows the next brush.
    click_x(&mut app, &ctx, 0, 0.85);
    assert_eq!(
        readout(app.chart_doc()).as_deref(),
        Some("$pick = (\"region\" = 'East')"),
        "the readout did not follow the second click"
    );
}

/// **Several values in one point clause read back as the OR chain.** A
/// `Point`'s rendering is cardinality-dependent, and the readout shows whatever
/// the store holds because that is the string the emitters interpolate — so
/// both shapes have to reach the rail intact, not just the single-value one a
/// click produces today.
///
/// Held rather than clicked: no gesture accumulates values into one clause yet
/// (see `a_point_gesture_can_never_hold_an_empty_clause`), so the multi-value
/// shape is pushed through the same `apply_interaction` seam the gestures use.
/// It is then executed, which is what makes this more than a `Display` test —
/// the OR chain has to be a legal WHERE over the fixture's own rows.
#[test]
fn a_multi_value_point_reads_back_as_the_or_chain_it_emits() {
    let path = example("point-select.yaml");
    let mut doc = live_doc(include_str!("../../../examples/point-select.yaml"));
    let point = |values: Vec<&str>| SqlPredicate::Point {
        column: "g".to_string(),
        values: values
            .into_iter()
            .map(|v| ScalarValue::Text(v.to_string()))
            .collect(),
        meta: None,
    };
    let select = |doc: &mut ChartDoc, predicate: SqlPredicate| {
        doc.apply_interaction(Interaction::Select {
            name: "pick".to_string(),
            contributor: ComponentPath("root/hconcat[0]".to_string()),
            predicate,
        });
    };

    select(&mut doc, point(vec!["a", "b"]));
    let line = doc.selection_sql().expect("a point selection is held");
    assert_eq!(
        line, "$pick = ((g = 'a' OR g = 'b'))",
        "several values render as the parenthesised OR chain; the outer \
         parentheses are `compile_selection`'s AND-of-one, and both are in the \
         emitted SQL"
    );

    let clause = clause_of(&line);
    let spec = spec_of(&path);
    let drawn = mark_rows(&mut doc, 1);
    assert_eq!(
        drawn, 6,
        "fixture check: three rows are `a` and three are `b`"
    );
    let session = doc.live_coordinator().expect("a live document").session();
    assert!(
        emitted_rows_sql(session, &spec, 1).contains(&clause),
        "the shown OR chain is not the executed one: {clause}"
    );
    assert_eq!(
        rows_selected_by(&path, &clause, "pick", 1),
        drawn,
        "the shown OR chain selects a different row set than the plot drew"
    );

    // The cardinality is read off the value, not fixed at the first render: one
    // value collapses the same clause back to the bare equality.
    select(&mut doc, point(vec!["c"]));
    assert_eq!(
        doc.selection_sql().as_deref(),
        Some("$pick = (g = 'c')"),
        "the readout kept the OR chain after the clause narrowed to one value"
    );
}

/// **`$name = FALSE` is not reachable from a gesture** — the fact the
/// `TRUE`-only guard in `LiveDashboard::selection_clauses` rests on, and the
/// reason an empty `Point` is not filtered out there.
///
/// A `Point` with no values renders as `FALSE`, so a readout could in principle
/// rail `$pick = FALSE`. It cannot today, and this pins why: every point a
/// click produces carries exactly one value, and a click that resolves nothing
/// — past the last band, or before the first — dispatches no interaction at all
/// rather than an empty clause, leaving the previous selection standing.
///
/// Only a pixel OUTSIDE the axis range resolves nothing. Band slots tile the
/// range contiguously and `band_category` floors into them, ignoring the
/// scale's `padding` — so a click in the visible gap between two bars still
/// lands in the slot it falls in. That is why the click below is at 0.99.
/// If a producer ever does hold an empty clause, this test goes red and the
/// comment on the guard has to be revisited rather than quietly outlived.
#[test]
fn a_point_gesture_can_never_hold_an_empty_clause() {
    let path = example("point-select-categorical.yaml");
    let mut app = live_window(&path);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    let contributor = app.chart_doc().composed.plots[0].path.clone();

    for (at, region) in [(0.25, "North"), (0.55, "South"), (0.85, "East")] {
        click_x(&mut app, &ctx, 0, at);
        let held = app
            .chart_doc_mut()
            .live_coordinator()
            .expect("a live document")
            .session()
            .contributor_predicate("pick", &contributor)
            .expect("the clicked plot contributed to $pick")
            .clone();
        match &held {
            SqlPredicate::Point { values, .. } => assert_eq!(
                values.len(),
                1,
                "a click produced a point clause of {} values: {held}",
                values.len()
            ),
            other => panic!("a category click produced {other:?}, not a point clause"),
        }
        assert_eq!(
            readout(app.chart_doc()).as_deref(),
            Some(format!("$pick = (\"region\" = '{region}')").as_str())
        );
    }

    // A click on no band at all — past the right-hand end of the last bar — is
    // not a selection of nothing. It is not a selection.
    click_x(&mut app, &ctx, 0, 0.99);
    assert_eq!(
        readout(app.chart_doc()).as_deref(),
        Some("$pick = (\"region\" = 'East')"),
        "a click that resolved no category disturbed the held selection"
    );
}

// ---------------------------------------------------------------------------
// 6. The other plan shape: a clause pushed inside a GROUP BY
// ---------------------------------------------------------------------------

/// A ranked category bar and the plot that filters it.
///
/// Mark 0 is the contributor; mark 1 is a `barX` binding `x: {sum: revenue}`
/// over `y: region`, which lowers to `Order{Aggregation{Filter{Source}}}` —
/// the plan shape no other case in this file reaches.
///
/// The six rows are built so a clause landing on the WRONG side of the
/// grouping cannot pass by coincidence: `quarter = 'Q1'` moves every one of the
/// three totals (30→12, 24→9, 18→7), so filtering the aggregated output —
/// where the totals all exceed any bound the clause could carry, and `quarter`
/// is not even a column — is a different picture in every bar.
const AGGREGATING_BAR: &str = r#"
params:
  pick: { select: crossfilter }
data:
  sales:
    - { region: North, quarter: Q1, revenue: 12 }
    - { region: South, quarter: Q1, revenue: 9 }
    - { region: East,  quarter: Q1, revenue: 7 }
    - { region: North, quarter: Q2, revenue: 18 }
    - { region: South, quarter: Q2, revenue: 15 }
    - { region: East,  quarter: Q2, revenue: 11 }
hconcat:
  - plot:
      - mark: dot
        data: { from: sales }
        x: quarter
        y: revenue
  - plot:
      - mark: barX
        data: { from: sales, filterBy: $pick }
        x: { sum: revenue }
        y: region
"#;

/// The same source, parsed independently of the running session — the
/// "executed" side of the identity, for a fixture that lives in this file
/// rather than in `examples/`. Same discipline as [`spec_of`]: an emission
/// built from the object under test would be comparing the readout with
/// itself.
fn spec_of_str(source: &str) -> Spec {
    parse_spec(source, Format::Yaml).expect("parse").spec
}

/// The bars a mark is drawing, as `(band, aggregate)` in row order — read off
/// the Arrow batches the coordinator returned, so this is what the picture was
/// built from rather than what the SQL was expected to produce.
///
/// The band is cast to `Utf8` and the value to `Float64` rather than downcast
/// directly, because which Arrow string layout DuckDB hands back is its
/// choice, not this test's.
fn bars_drawn(doc: &mut ChartDoc, mark: usize, band: &str, value: &str) -> Vec<(String, f64)> {
    use arrow::array::{Float64Array, StringArray};
    use arrow::compute::cast;
    use arrow::datatypes::DataType;
    let batches = doc
        .live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries");
    let mut bars = Vec::new();
    for batch in &batches {
        let bi = batch
            .schema()
            .index_of(band)
            .unwrap_or_else(|_| panic!("the emitted columns carry the band `{band}`"));
        let vi = batch
            .schema()
            .index_of(value)
            .unwrap_or_else(|_| panic!("the emitted columns carry the aggregate `{value}`"));
        let bands = cast(batch.column(bi), &DataType::Utf8).expect("a text band");
        let bands = bands
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("utf-8 band");
        let values = cast(batch.column(vi), &DataType::Float64).expect("a numeric aggregate");
        let values = values
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("f64 aggregate");
        for i in 0..batch.num_rows() {
            bars.push((bands.value(i).to_string(), values.value(i)));
        }
    }
    bars
}

/// **The clause a ranked category bar grouped under is the clause on the
/// rail.** AC3 for the shape the lift introduced.
///
/// Every other case in this file filters a row-level plan, where the selection
/// wraps the whole query and the readout's identity with the emitted SQL is a
/// substring test over one WHERE. An aggregating bar has two candidate places
/// for the same string — around the aggregation, or beneath it — and only one
/// of them is the chart the author asked for. So three things are held at once:
///
/// 1. the rail shows the clause the store holds, as it does for a brush;
/// 2. that string is in the mark's emitted SQL **byte for byte**, and it is
///    positioned BEFORE the `GROUP BY`, which is what "the filter is inside the
///    grouping" means in the emitted text; and
/// 3. the bars the mark actually drew are the totals of the rows that clause
///    selects — hand-computed here, so a plausible-but-wrong grouping fails on
///    the numbers rather than passing on the text.
///
/// Assertion 3 is what makes 2 more than a string search. A clause threaded
/// beneath the aggregation and a clause wrapped around it can both appear
/// before a `GROUP BY` in some emitted string; only one of them recomputes the
/// totals.
#[test]
fn an_aggregating_bars_readout_is_the_clause_it_grouped_under() {
    let mut doc = live_doc(AGGREGATING_BAR);

    // At rest: the totals over all six rows, in the ascending band order the
    // lowerer emits. A fixture check — if these are wrong the identities below
    // are being read off the wrong chart.
    assert_eq!(
        bars_drawn(&mut doc, 1, "region", "revenue"),
        vec![
            ("East".to_string(), 18.0),
            ("North".to_string(), 30.0),
            ("South".to_string(), 24.0),
        ],
        "fixture check: unfiltered, the bar is each region's whole total"
    );
    assert_eq!(doc.selection_sql(), None, "a rest state claims nothing");
    assert!(readout(&doc).is_none());

    doc.apply_interaction(Interaction::Select {
        name: "pick".to_string(),
        contributor: ComponentPath("root/hconcat[0]".to_string()),
        predicate: SqlPredicate::Point {
            column: "quarter".to_string(),
            values: vec![ScalarValue::Text("Q1".to_string())],
            meta: None,
        },
    });

    // 1. The rail.
    let line = readout(&doc).expect("a held selection is shown");
    assert_eq!(
        line, "$pick = (quarter = 'Q1')",
        "the readout over an aggregating bar is not the clause the store holds"
    );
    let clause = clause_of(&line);

    // 3. The picture, first — so that if the push-down is wrong the failure
    // names the arithmetic rather than a missing substring.
    assert_eq!(
        bars_drawn(&mut doc, 1, "region", "revenue"),
        vec![
            ("East".to_string(), 7.0),
            ("North".to_string(), 12.0),
            ("South".to_string(), 9.0),
        ],
        "the bars are not the totals of the rows `{clause}` selects — the \
         clause did not reach the rows that were grouped"
    );

    // 2. The emitted SQL, and where in it the shown string sits.
    let spec = spec_of_str(AGGREGATING_BAR);
    let session = doc.live_coordinator().expect("a live document").session();
    let sql = emitted_chart_sql(session, &spec, 1);
    let at = sql.find(&clause).unwrap_or_else(|| {
        panic!(
            "the shown clause is not in the executed SQL byte for byte — \
             something rewrote it on the way to the rail.\nshown:    {clause}\nexecuted: {sql}"
        )
    });
    let group_by = sql
        .find("GROUP BY")
        .unwrap_or_else(|| panic!("an aggregating bar emits a GROUP BY: {sql}"));
    assert!(
        at < group_by,
        "the shown clause is in the SQL but OUTSIDE the grouping — it is \
         filtering the totals rather than the rows they are computed \
         from.\nshown:    {clause}\nexecuted: {sql}"
    );

    // And it follows the next selection, as it does over a row-level plan.
    doc.apply_interaction(Interaction::Select {
        name: "pick".to_string(),
        contributor: ComponentPath("root/hconcat[0]".to_string()),
        predicate: SqlPredicate::Point {
            column: "quarter".to_string(),
            values: vec![ScalarValue::Text("Q2".to_string())],
            meta: None,
        },
    });
    assert_eq!(
        readout(&doc).as_deref(),
        Some("$pick = (quarter = 'Q2')"),
        "the readout did not follow the second selection"
    );
    assert_eq!(
        bars_drawn(&mut doc, 1, "region", "revenue"),
        vec![
            ("East".to_string(), 11.0),
            ("North".to_string(), 18.0),
            ("South".to_string(), 15.0),
        ],
        "the bars did not follow the second selection"
    );
}
