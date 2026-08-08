//! Gate: the bundled crosswalk chart draws the WHOLE published table, and says
//! out loud that it fetches it.
//!
//! Two claims live here, and they fail for different reasons.
//!
//! # 1. Nothing in this chart is sampled away
//!
//! The published crosswalk is past [`MEASURED_INKED_MAX`], the largest
//! row-level primitive count measured to ink a frame. A dot scatter over it is
//! therefore sampled by `pipeline::automatic_sample` with nothing on the
//! command line — a correct picture of a fraction of the table, drawn without
//! anyone asking for a fraction.
//!
//! Both marks in the shipped spec aggregate instead. That is not a stylistic
//! preference and it is not enforced by a comment: the emitter's sample clause
//! is guarded out of an aggregating plan (`emit::plan_aggregates`), and
//! `Session::drawn_primitive_estimate` asks the emitter that same question by
//! emitting each mark's query twice, with a rate and without, and calling the
//! mark row-level exactly when the two strings differ. So the property "this
//! chart cannot be sampled" is decidable from the spec alone — no data, no
//! network, no adapter — and
//! [`every_mark_in_the_shipped_crosswalk_chart_is_out_of_the_samplers_reach`]
//! decides it. Swap either mark for `dot` and it reddens.
//!
//! The counterpart matters as much as the claim. A test that only asserts "the
//! aggregate is not sampled" passes just as well against an emitter that never
//! samples anything, so
//! [`at_the_crosswalks_row_count_the_scatter_is_sampled_and_the_aggregate_is_not`]
//! builds BOTH marks over one local table of the crosswalk's magnitude and
//! shows the two answers differing: the scatter's estimate is every row and the
//! policy halves it; the aggregate's estimate is zero and the policy leaves it
//! alone, with the executed bins summing back to the row count.
//!
//! # 2. It needs the network, and that is stated rather than discovered
//!
//! Every other shipped start opens with no connection. This one does not, and
//! the difference is disclosed on the button it is opened from
//! ([`starts::REMOTE_MARK`]) and asserted here against the flag that makes it a
//! property rather than a promise about a string.
//!
//! **The offline behaviour is tested, not assumed**, and hermetically: the
//! engine is given [`NetworkPolicy::Disabled`] and an empty extension directory,
//! which is the same seam the engine's own air-gap tests use. `httpfs` can then
//! neither be installed nor loaded, so the remote source cannot be read by any
//! path, and the load fails as a structured error naming the network and the
//! URL. No jail, no unplugging, and no dependence on how this machine is
//! connected — the failure is arranged, so it is the same on every machine.
//!
//! # What is here and what is `--ignored`
//!
//! Everything above runs in CI. What cannot is the live source itself: whether
//! the published Parquet still answers, still has the columns this spec names,
//! and is still past the ceiling that makes the whole argument necessary. Those
//! are the `#[ignore]`d tests at the foot of this file — the same shape and the
//! same reason as `brightfield-engine`'s own httpfs tests. Run them with
//! `cargo +1.95.0 test -p brightfield-shell --test crosswalk_chart -- --ignored`.

use std::path::PathBuf;

use brightfield_engine::error::EngineError;
use brightfield_engine::{Engine, LoadOptions, NetworkPolicy, Session};
use brightfield_render::sample_policy::{renders_complete, sample_exponent, MEASURED_INKED_MAX};
use brightfield_shell::starts;
use brightfield_spec::analysis::{analyse_spec, ComponentPath, SpecAnalysis};
use brightfield_spec::ast::Spec;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_sql::emit::{collect_marks, emit_query_sampled};
use brightfield_sql::ir::{Predicate, SampleRate, ScalarValue, SelectionPredicate};
use brightfield_workbench::ViewKind;

/// The published crosswalk's row count, read live on 2026-08-06.
///
/// It is used here as a FIXTURE SIZE — how big to make the local table the two
/// mark families are compared over — and not as a claim about what the live
/// table holds today. The only property the argument in this file needs is the
/// one the `const` assertion below pins: that the crosswalk is past the
/// drawn-primitive ceiling, so a row-level mark over it would be sampled.
/// [`the_published_crosswalk_is_still_past_the_drawn_primitive_ceiling`] is
/// what re-reads the live number and fails if that stops being true.
const CROSSWALK_ROWS: u64 = 207_099;

// If the crosswalk ever falls under the ceiling, the aggregating mark is still
// correct but this file's whole argument is moot — and a moot test that still
// passes is worse than a red one. The build says so rather than a test run.
const _: () = assert!(CROSSWALK_ROWS > MEASURED_INKED_MAX);

/// The URL the shipped spec reads. Written out so the assertions can hold the
/// error messages to it by name; the spec is the source of truth and
/// [`the_shipped_spec_reads_exactly_this_url`] is the strut between them.
const CROSSWALK_URL: &str = "https://openlake.meridian.online/edgar_gleif.parquet";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The shipped spec, parsed and analysed — the bytes the button opens.
fn shipped() -> (Spec, SpecAnalysis) {
    parse_and_analyse(starts::CROSSWALK_CHART_SPEC)
}

fn parse_and_analyse(yaml: &str) -> (Spec, SpecAnalysis) {
    let spec = parse_spec(yaml, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let analysis = analyse_spec(&spec).expect("the spec analyses");
    (spec, analysis)
}

/// A LOCAL table of `rows` rows carrying the three columns the shipped spec's
/// marks read, generated in SQL so the fixture is the size of the crosswalk
/// without being the crosswalk.
///
/// The two mark bodies below are lifted from the shipped spec and differ in
/// exactly one thing — the mark family — which is the difference this file
/// exists to measure.
///
/// `tier` and `method` are cut on different strides (`i` against `i / 4`) so
/// they vary independently. A shared stride would collapse the grid to one cell
/// per tier, and "the cells account for every row" is a weaker claim when there
/// are four cells than when there are sixteen.
fn local_table(rows: u64) -> String {
    format!(
        "data:
  links:
    query: |
      SELECT
        (['ambiguous', 'authoritative', 'candidate', 'confirmed'])[(i % 4) + 1] AS tier,
        (['exact_name', 'jaro_winkler', 'sec-ncen', 'sec-registration'])[(i // 4 % 4) + 1] AS method,
        20 + CAST(hash(i) % 270 AS BIGINT) AS match_text_chars
      FROM range({rows}) AS t(i)
"
    )
}

/// The shipped chart's mark grammar over the local table: both marks aggregate.
fn local_aggregating(rows: u64) -> String {
    format!(
        "{}plot:
  - mark: rectY
    data: {{ from: links }}
    x: {{ bin: match_text_chars }}
    y: {{ count: }}
  - mark: cell
    data: {{ from: links }}
    x: tier
    y: method
    fill: {{ count: }}
width: 640
height: 400
",
        local_table(rows)
    )
}

/// The substitution this file exists to refuse: the same rows, drawn one
/// primitive each.
fn local_scatter(rows: u64) -> String {
    format!(
        "{}plot:
  - mark: dot
    data: {{ from: links }}
    x: match_text_chars
    y: match_text_chars
width: 640
height: 400
",
        local_table(rows)
    )
}

fn session(yaml: &str) -> Session {
    let (spec, analysis) = parse_and_analyse(yaml);
    Engine::new()
        .load_spec(spec, analysis, None)
        .expect("the local fixture loads")
        .session
}

/// Sum of the `__bf_count` column across every batch of a mark — the number of
/// source rows that reached the screen inside a group.
///
/// `f64` because that is what the count lowerers emit (`CAST(COUNT(*) AS
/// DOUBLE)`, aliased `__bf_count` by both the rect-bin and cell paths) and
/// because narrowing it back to an integer here would be a cast this workspace
/// warns on for good reason. Compared with `==` against an exact row count,
/// which is safe: every value in the sum is a whole number well inside the 2^53
/// f64 integers, so the arithmetic is exact rather than approximately right.
fn counted_rows(session: &mut Session, mark: usize) -> f64 {
    use arrow::array::Float64Array;
    let batches = session.execute_mark(mark).expect("the mark executes");
    let mut total = 0.0_f64;
    let mut seen_column = false;
    for batch in &batches {
        let Ok(idx) = batch.schema().index_of("__bf_count") else {
            continue;
        };
        seen_column = true;
        let column = batch
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("__bf_count is the DOUBLE the count lowerers alias");
        for i in 0..column.len() {
            total += column.value(i);
        }
    }
    assert!(
        seen_column,
        "mark {mark} returned no __bf_count column, so this reads nothing — \
         the mark is not aggregating"
    );
    total
}

/// An extension directory with nothing in it, so `LOAD httpfs` has nowhere to
/// find the extension and [`NetworkPolicy::Disabled`] leaves it nowhere to get
/// it from either. Under `CARGO_TARGET_TMPDIR`, per-name, so concurrent runs
/// do not share one.
fn empty_extension_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("crosswalk-ext-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create an empty extension dir");
    dir
}

/// Emit `mark`'s query with a sample rate and without, and report whether the
/// rate reached it.
///
/// This is the emitter's own definition of "row-level", re-asked rather than
/// re-implemented — the identical comparison
/// `Session::drawn_primitive_estimate` makes to decide which marks the sampling
/// policy counts. A guard that re-derived it would be one refactor away from
/// disagreeing with the thing it guards.
fn sample_clause_reaches(spec: &Spec, mark: usize) -> bool {
    let probe = SampleRate::from_exponent(1).expect("1 is a representable exponent");
    let unsampled = emit_query_sampled(spec, mark, None, None, &[], None)
        .expect("the mark emits")
        .sql;
    let sampled = emit_query_sampled(spec, mark, None, None, &[], Some(probe))
        .expect("the mark emits under a rate")
        .sql;
    unsampled != sampled
}

// ---------------------------------------------------------------------------
// The mark family — decided from the spec, with no data and no network
// ---------------------------------------------------------------------------

/// No mark in the shipped chart can be sampled, and the check has teeth.
///
/// Asked of the shipped bytes, so this is about the artifact rather than about
/// a fixture that resembles it. The third assertion is the one that keeps the
/// first two from passing vacuously: the SAME question, asked of a `dot` mark
/// over the SAME data source, answers the other way.
#[test]
fn every_mark_in_the_shipped_crosswalk_chart_is_out_of_the_samplers_reach() {
    let (spec, _) = shipped();
    let marks = collect_marks(&spec).len();
    assert!(
        marks >= 2,
        "the shipped crosswalk chart is down to {marks} mark(s) — this gate is \
         holding almost nothing"
    );
    for mark in 0..marks {
        assert!(
            !sample_clause_reaches(&spec, mark),
            "mark {mark} of the shipped crosswalk chart is ROW-LEVEL: the \
             emitter's sample clause reaches it, so at this table's size the \
             automatic policy will draw one row in two. The mark family is the \
             point of this spec — see its header."
        );
    }

    let (scatter, _) = parse_and_analyse(&local_scatter(64));
    assert!(
        sample_clause_reaches(&scatter, 0),
        "a dot mark over the same shape is NOT reported row-level, so the \
         assertions above say nothing about the shipped marks"
    );
}

/// At the crosswalk's magnitude the two mark families get different answers
/// from the policy, and the aggregate's answer accounts for every row.
///
/// This is the claim in full, over real rows: the scatter's estimate is one
/// primitive per row and the policy halves it; the aggregate's estimate is
/// zero, the policy passes, and the counts the bins and cells come back with
/// sum to the row count — so nothing was dropped on the way to the screen, as
/// opposed to nothing being *known* to have been dropped.
#[test]
fn at_the_crosswalks_row_count_the_scatter_is_sampled_and_the_aggregate_is_not() {
    // The row-level twin: every row is a primitive, and the count is past the
    // ceiling, so the policy picks a modulus.
    let scatter = session(&local_scatter(CROSSWALK_ROWS));
    let scatter_estimate = scatter
        .drawn_primitive_estimate()
        .expect("the scatter can be counted");
    assert_eq!(
        scatter_estimate, CROSSWALK_ROWS,
        "the scatter draws one primitive per row"
    );
    assert!(
        !renders_complete(scatter_estimate),
        "{scatter_estimate} primitives is under the ceiling, so this fixture no \
         longer demonstrates anything — see CROSSWALK_ROWS"
    );
    let modulus = sample_exponent(scatter_estimate)
        .map(|e| 1_u64 << e)
        .expect("a scatter this size needs a sample");
    assert_eq!(
        modulus, 2,
        "at this row count the policy halves the scatter; it drew one row in \
         {modulus}"
    );

    // The shipped mark family over the same rows: the sample clause never
    // reaches either mark, so there is nothing to count and nothing to halve.
    let mut aggregate = session(&local_aggregating(CROSSWALK_ROWS));
    let aggregate_estimate = aggregate
        .drawn_primitive_estimate()
        .expect("the aggregate can be counted");
    assert_eq!(
        aggregate_estimate, 0,
        "an aggregating mark contributes no row-level primitives; this spec \
         contributed {aggregate_estimate}, which means one of its marks is \
         row-level"
    );
    assert!(
        sample_exponent(aggregate_estimate).is_none(),
        "the policy chose a sample rate for a spec that draws complete"
    );

    // And the reduction accounts for every row, in both marks.
    let want = CROSSWALK_ROWS as f64;
    for mark in 0..2 {
        let counted = counted_rows(&mut aggregate, mark);
        assert_eq!(
            counted, want,
            "mark {mark}'s groups account for {counted} of {CROSSWALK_ROWS} rows"
        );
    }
}

// ---------------------------------------------------------------------------
// What the grid is grouped by
// ---------------------------------------------------------------------------

/// The grid's second category is the METHOD that made the link, not the type of
/// the SEC key.
///
/// This is an editorial claim about the chart, so it is worth saying why a test
/// holds it at all. `key_type` and `method` are interchangeable to every other
/// gate in this file: both are categories, both group, both keep the mark out
/// of the sampler's reach. Swapping one for the other is invisible to the
/// machine and changes what the chart says, which is exactly the kind of edit
/// that needs an assertion rather than a comment.
///
/// What separates them is legible in
/// `examples/protocol/edgar_gleif/models/tier.sql`, the model that writes both
/// columns. Its probabilistic branch hardcodes `key_type` to `cik`, so a
/// `series` edge reaches the table only through the authoritative branch: that
/// value reports how the row was BUILT and can only ever land opposite the tier
/// the other axis already draws. `method` reports what asserted the edge — a
/// filing, or a fuzzy string comparison — which is the fact a consumer of a
/// crosswalk needs in order to decide what a row is worth.
///
/// Decided from the shipped bytes, with no data and no network: the grid's
/// emitted SQL groups by both category columns, so naming them is enough.
#[test]
fn the_grid_groups_by_the_method_that_made_the_link() {
    let (spec, _) = shipped();
    let groups = brightfield_sql::collect_plot_groups(&spec);
    let grid_mark = *groups
        .last()
        .expect("the shipped chart has plots")
        .mark_indices
        .first()
        .expect("the last plot has a mark");
    let sql = emit_query_sampled(&spec, grid_mark, None, None, &[], None)
        .expect("the grid emits")
        .sql;

    let group_by = sql
        .split_once("GROUP BY")
        .expect("the grid aggregates, so it has a GROUP BY")
        .1;
    for column in ["method", "tier"] {
        assert!(
            group_by.contains(&format!("\"{column}\"")),
            "the grid does not group by {column}:\n{sql}"
        );
    }
    assert!(
        !sql.contains("key_type"),
        "the grid still reads key_type, which the header of \
         examples/remote/edgar-gleif-crosswalk.yaml says it does not:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// The filter is a re-query
// ---------------------------------------------------------------------------

/// A brush on the histogram rewrites the grid's SQL, and rewrites it BELOW the
/// GROUP BY.
///
/// The distinction the criterion is about: a filter applied above the
/// aggregate would drop whole groups from a result set already computed, which
/// is client-side filtering wearing SQL's clothes. Threaded below it, DuckDB
/// re-aggregates the source under a new `WHERE` and the counts themselves
/// change — which is the only reading that is correct for a crosswalk this
/// size, because the rows the brush excluded were never in a client result set
/// to begin with.
#[test]
fn a_brush_on_the_histogram_re_queries_the_grid_below_its_group_by() {
    let (spec, _) = shipped();
    let groups = brightfield_sql::collect_plot_groups(&spec);
    assert_eq!(
        groups.len(),
        2,
        "the shipped chart is no longer two plots, so the brush and the grid \
         may no longer be in the plots this test names"
    );
    let brushing_plot = groups[0].plot_path.clone();
    let grid_mark = *groups[1]
        .mark_indices
        .first()
        .expect("the second plot has a mark");

    let resting = emit_query_sampled(&spec, grid_mark, None, None, &[], None)
        .expect("the grid emits at rest")
        .sql;

    let selection: Vec<SelectionPredicate> = vec![(
        "band".to_string(),
        vec![(
            brushing_plot,
            Predicate::Interval {
                column: "\"match_text_chars\"".to_string(),
                lo: ScalarValue::Float(40.0),
                hi: ScalarValue::Float(60.0),
                meta: None,
            },
        )],
    )];
    let brushed = emit_query_sampled(&spec, grid_mark, None, Some(&selection), &[], None)
        .expect("the grid emits under a brush")
        .sql;

    assert_ne!(
        resting, brushed,
        "the brush did not change the grid's SQL, so whatever the drag does it \
         is not re-querying"
    );
    assert!(
        brushed.contains("\"match_text_chars\" >= 40")
            && brushed.contains("\"match_text_chars\" <= 60"),
        "the brushed SQL does not carry the brushed bounds:\n{brushed}"
    );

    let where_at = brushed
        .find("\"match_text_chars\" >= 40")
        .expect("just asserted present");
    let group_at = brushed
        .find("GROUP BY")
        .expect("the grid aggregates, so it has a GROUP BY");
    assert!(
        where_at < group_at,
        "the brush predicate lands AFTER the GROUP BY, which filters groups \
         rather than rows — the counts would stay at their unbrushed values:\n\
         {brushed}"
    );
}

/// The same drag, executed: it costs new DuckDB statements and the numbers
/// come back different.
///
/// The SQL assertion above is about what is emitted; this is about what
/// happens. Held over the local twin rather than the live source so it runs in
/// CI — the emitted plan is the shipped one either way, because it is the same
/// mark grammar over the same column names.
#[test]
fn a_brush_over_the_local_twin_re_executes_and_the_counts_move() {
    // A modest fixture: this test is about statements and totals, not size.
    let yaml = format!(
        "params:
  band: {{ select: crossfilter }}
{}hconcat:
  - plot:
      - mark: rectY
        data: {{ from: links }}
        x: {{ bin: match_text_chars }}
        y: {{ count: }}
      - select: intervalX
        as: $band
    width: 460
    height: 340
  - plot:
      - mark: cell
        data: {{ from: links, filterBy: $band }}
        x: tier
        y: method
        fill: {{ count: }}
    width: 380
    height: 340
",
        local_table(20_000)
    );
    let mut session = session(&yaml);

    let at_rest = counted_rows(&mut session, 1);
    assert_eq!(
        at_rest, 20_000.0,
        "the unbrushed grid should account for every fixture row"
    );

    let before = session.duckdb_execute_count();
    let results = session.propagate_selection(
        "band",
        ComponentPath("root/hconcat[0]/plot[0]".to_string()),
        Predicate::Interval {
            column: "\"match_text_chars\"".to_string(),
            lo: ScalarValue::Float(20.0),
            hi: ScalarValue::Float(60.0),
            meta: None,
        },
    );
    assert!(
        !results.is_empty(),
        "the brush reached no subscriber mark, so nothing re-queried"
    );
    for (mark, result) in &results {
        assert!(result.is_ok(), "mark {mark} failed under the brush");
    }
    assert!(
        session.duckdb_execute_count() > before,
        "the brush issued no new DuckDB statement — it was resolved without \
         going back to the engine"
    );

    let brushed = counted_rows(&mut session, 1);
    assert!(
        brushed < at_rest,
        "the brushed grid still accounts for {brushed} of {at_rest} rows, so \
         the predicate did not reach the rows"
    );
    assert!(
        brushed > 0.0,
        "the brushed grid is empty, so this proves the query ran and nothing \
         about what it means"
    );
}

// ---------------------------------------------------------------------------
// The start, and what it discloses
// ---------------------------------------------------------------------------

/// The chart is reachable from the front door, opens on the chart view, and is
/// not a run-less declaration.
#[test]
fn the_crosswalk_chart_is_a_start_that_opens_on_a_rendered_chart() {
    let start = starts::find(starts::CROSSWALK_CHART)
        .expect("the crosswalk chart is a shipped starting point");
    assert_eq!(
        start.view,
        ViewKind::Charts,
        "the crosswalk chart is offered for the {:?} view, so the click lands \
         on a lineage canvas rather than on a drawn chart",
        start.view
    );
    assert!(
        !start.run_less,
        "the crosswalk CHART is a result, not a declaration — run_less belongs \
         to the manifest start beside it"
    );
    assert!(
        start.remote,
        "the crosswalk chart reads an https source; a start that does that and \
         does not declare it is exempt from the hermetic front-door gates by \
         accident rather than by decision"
    );
    assert!(
        starts::find(starts::CROSSWALK).is_some_and(|s| !s.remote),
        "the manifest start must stay local — it is the one the protocol \
         view's empty canvas offers"
    );
}

/// A start that reaches the network says so on its own button.
///
/// The same strut `a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button`
/// holds for [`starts::RUN_LESS_MARK`], for the other property a click cannot
/// take back. The flag is what the hermetic gates read; the label is the only
/// thing a user reads. Neither may move without the other.
#[test]
fn a_start_that_reaches_the_network_says_so_on_its_own_button() {
    for start in starts::STARTS {
        assert_eq!(
            start.remote,
            start.label.contains(starts::REMOTE_MARK),
            "{}'s label {:?} and its remote flag ({}) disagree — the flag \
             exempts it from the hermetic gates, and the label is the only \
             warning anyone gets before the click",
            start.id,
            start.label,
            start.remote
        );
    }
    assert_eq!(
        starts::STARTS.iter().filter(|s| s.remote).count(),
        1,
        "the shipped set is meant to hold exactly one start that needs a \
         connection; if that changed deliberately, change this number and the \
         front door's skip-count assertions with it"
    );
}

/// The URL the assertions in this file name is the URL the shipped spec reads.
#[test]
fn the_shipped_spec_reads_exactly_this_url() {
    assert!(
        starts::CROSSWALK_CHART_SPEC.contains(CROSSWALK_URL),
        "the shipped spec no longer names {CROSSWALK_URL}, so every message \
         assertion below is checking a string nothing produces"
    );
}

// ---------------------------------------------------------------------------
// With no network
// ---------------------------------------------------------------------------

/// Air-gapped, the crosswalk chart refuses — by name, naming the network and
/// the URL.
///
/// Hermetic and arranged rather than observed: [`NetworkPolicy::Disabled`] plus
/// an empty extension directory means `httpfs` can be neither installed nor
/// loaded, so there is no path by which the https source could be read on any
/// machine, however this one happens to be connected. That is the same seam
/// `brightfield-engine`'s own air-gap tests use.
///
/// What is asserted is the SHAPE of the failure, because the shape is the
/// product decision: a structured error carrying the source and the location,
/// rendering to a message that names the network. Not a bare SQL error that
/// reads like a local-data problem, not an empty chart, and not a chart drawn
/// from whatever happened to be cached.
#[test]
fn with_no_network_the_crosswalk_chart_refuses_naming_the_network_and_the_url() {
    let ext_dir = empty_extension_dir("crosswalk-chart");
    let (spec, analysis) = shipped();
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: Some(ext_dir.clone()),
    };
    let err = Engine::new()
        .load_spec_with(spec, analysis, None, &options)
        .expect_err("the crosswalk chart cannot load with no way to reach the lake");

    match &err {
        EngineError::RemoteDisabled { location, .. }
        | EngineError::RemoteSourceFailed { location, .. } => {
            assert_eq!(
                location, CROSSWALK_URL,
                "the error names a location, but not the one the spec reads"
            );
        }
        other => panic!(
            "expected the remote-source error the shell raises as a banner, \
             got: {other:?}"
        ),
    }
    let msg = format!("{err}");
    assert!(
        msg.contains("network"),
        "the message a user sees does not name the network: {msg}"
    );
    assert!(
        msg.contains(CROSSWALK_URL),
        "the message a user sees does not name what it could not reach: {msg}"
    );

    std::fs::remove_dir_all(&ext_dir).ok();
}

/// The same denial does not touch the rest of the gallery: every start this
/// binary does not mark [`starts::Start::remote`] still opens.
///
/// The other half of the air-gapped promise. `front_door.rs` loads these too,
/// but it loads them without saying why they are allowed to be in a hermetic
/// test at all — here the filter is `!remote`, so a start that grows a fetched
/// source and forgets to declare it lands in this loop and fails.
#[test]
fn no_other_shipped_start_needs_a_connection_to_open() {
    let local: Vec<&starts::Start> = starts::STARTS.iter().filter(|s| !s.remote).collect();
    assert!(
        local.len() >= 3,
        "only {} start(s) claim to open with no connection — the promise this \
         asserts has almost nothing left to hold",
        local.len()
    );
    for start in local {
        starts::load(start.id).unwrap_or_else(|e| {
            panic!(
                "{} is not marked remote but does not open: {e}. Either its \
                 spec grew a fetched source (declare it) or it broke.",
                start.id
            )
        });
    }
}

// ---------------------------------------------------------------------------
// Network-gated: the live source
// ---------------------------------------------------------------------------

/// The published crosswalk still answers, and is still past the ceiling that
/// makes an aggregating mark necessary.
///
/// The row count is READ rather than asserted equal to anything: a published
/// dataset is refreshed, and a test that pins its cardinality reddens on a
/// successful publish. What is asserted is the property the file's argument
/// rests on.
#[test]
#[ignore = "network: reads the published crosswalk over https"]
fn the_published_crosswalk_is_still_past_the_drawn_primitive_ceiling() {
    let session = session(&format!(
        "data:
  crosswalk:
    file: \"{CROSSWALK_URL}\"
plot:
  - mark: dot
    data: {{ from: crosswalk }}
    x: confidence
    y: confidence
width: 640
height: 400
"
    ));
    let rows = session
        .drawn_primitive_estimate()
        .expect("the published crosswalk can be counted");
    println!("published crosswalk: {rows} rows");
    assert!(
        !renders_complete(rows),
        "the published crosswalk is down to {rows} rows, under the {MEASURED_INKED_MAX} \
         ceiling — a row-level mark over it would no longer be sampled, so the \
         reason this chart aggregates has changed"
    );
}

/// The shipped start opens over the network onto a drawn chart, and the drawn
/// chart accounts for every row in the published table.
#[test]
#[ignore = "network: opens the bundled crosswalk chart over https"]
fn the_crosswalk_chart_start_opens_over_the_network_drawing_every_row() {
    let opened = starts::load(starts::CROSSWALK_CHART)
        .expect("the crosswalk chart start composes with a connection");
    match opened {
        starts::Opened::Charts(chart) => assert!(
            chart.composed.width > 0 && chart.composed.height > 0,
            "the start composed no plots"
        ),
        starts::Opened::Protocol(_) => panic!("the crosswalk CHART composed a protocol document"),
    }

    let (spec, analysis) = shipped();
    let mut session = Engine::new()
        .load_spec(spec, analysis, None)
        .expect("the shipped spec loads over httpfs")
        .session;
    assert_eq!(
        session
            .drawn_primitive_estimate()
            .expect("the live session can be counted"),
        0,
        "the live session reports row-level primitives, so the sampler is in \
         play over the published table"
    );

    let binned = counted_rows(&mut session, 0);
    let celled = counted_rows(&mut session, 1);
    println!("live crosswalk: histogram {binned} rows, grid {celled} rows");
    assert_eq!(
        binned, celled,
        "the two marks disagree about how many rows the table has, so at least \
         one of them is dropping some"
    );
    // The same property the sibling test reads off the estimate, read here off
    // the drawn result instead: if the live table has fallen under the
    // ceiling, the reason both marks aggregate has gone with it.
    assert!(
        binned > ceiling_as_rows(),
        "the live table drew {binned} rows, at or under the {MEASURED_INKED_MAX} \
         ceiling — see the sibling test"
    );
}

/// [`MEASURED_INKED_MAX`] in the units [`counted_rows`] answers in.
///
/// A widening conversion in one place rather than a narrowing one at each
/// comparison: `u32::from`/`f64::from` do not reach `u64`, and turning a
/// summed `f64` back into an integer to compare is the cast this workspace
/// warns on. The ceiling is six figures, so nothing is lost going this way.
#[allow(
    clippy::cast_precision_loss,
    reason = "MEASURED_INKED_MAX is far inside f64's exact-integer range"
)]
fn ceiling_as_rows() -> f64 {
    MEASURED_INKED_MAX as f64
}
