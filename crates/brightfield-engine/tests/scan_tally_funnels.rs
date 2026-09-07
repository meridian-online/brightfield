//! Each funnel the `record_scan` rustdoc enumerates is driven here, and its
//! contribution to the tally is read back.
//!
//! **The tally is only as complete as the set of places that call into it**,
//! and a funnel that stopped counting would report a smaller number rather
//! than an error. The open-scan bound `COMPOSITION_FILE_READS` is stated over
//! that number, so a composition reaching the file through an uncounted funnel
//! would read the file and be recorded as reading it no times — the bound met
//! by not counting.
//!
//! That was measurable before this file existed: `record_scan` removed from
//! four of the six funnels its own rustdoc enumerates left the whole suite
//! green, because the fixtures in it reach the mark-execute funnel and
//! `query_arrow_raw` and no other.
//!
//! One test per funnel, each asserting that the statement that funnel issues
//! is present in the tally, matched on a fragment of its own SQL rather than
//! on the total. A total moves when any funnel moves and so pins none of them.
//!
//! # What this does not cover
//!
//! A funnel added after this file was written. The set below is the set named
//! in the rustdoc, checked against it by hand, and an eighth place that
//! handed a statement to the connection would go unnoticed here.
//!
//! `FinetypeBundle` reads a column per label from inside the profile pass and
//! is deliberately outside `record_scan` — that gap is stated on the rustdoc.

use brightfield_engine::{Engine, ScanTally, Session};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::{Predicate, SampleRate, ScalarValue};

/// A categorical `dot` over a small generated table: a VARCHAR on `x` (a band
/// scale, so the band-order funnel fires), a VARCHAR on `fill` (a colour
/// scale, so the category funnel fires) and a number on `y`.
const CATEGORICAL: &str = r#"
data:
  t: { query: "SELECT ('cat_' || (i % 4))::VARCHAR AS label, ('grp_' || (i % 3))::VARCHAR AS kind, (i % 50) AS value FROM range(400) t(i)" }
plot:
  - mark: dot
    data: { from: t }
    x: label
    y: value
    fill: kind
"#;

/// A brushable pair whose subscriber aggregates, so the pre-aggregation layer
/// builds a cube and serves the re-query off it. The shape is the one
/// `crates/brightfield-engine/tests/crossfilter_column_validation.rs` uses for
/// the same reason.
const CUBED: &str = r#"
params:
  brush: { select: crossfilter }
data:
  t: { query: "SELECT (i % 50) AS a, (i % 30) AS b FROM range(2000) t(i)" }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: b
      y: a
    - select: intervalX
      as: $brush
  - plot:
    - mark: densityX
      data: { from: t, filterBy: $brush }
      x: a
"#;

fn session(spec: &str) -> Session {
    let parsed = parse_spec(spec, Format::Yaml).expect("the spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("the spec analyses");
    Engine::new()
        .load_spec(parsed.spec, analysis, None)
        .expect("the spec loads")
        .session
}

/// Assert the tally holds a statement this funnel's `recognise` accepts,
/// naming the funnel and printing what was counted when it does not.
fn counted(tally: &ScanTally, funnel: &str, recognise: impl Fn(&str) -> bool) {
    let seen: Vec<&str> = tally.statements.iter().map(|s| s.sql.as_str()).collect();
    assert!(
        seen.iter().copied().any(&recognise),
        "the {funnel} funnel issued a statement the tally never saw. What was \
         counted:\n{}",
        seen.iter()
            .map(|s| format!("  {}\n", &s[..s.len().min(160)]))
            .collect::<String>()
    );
}

/// **The mark execute counts.** The statement a mark is drawn from is the
/// bulk of a composition's reads.
#[test]
fn the_mark_execute_funnel_counts() {
    let mut live = session(CATEGORICAL);
    live.begin_scan_tally();
    live.execute_mark(0).expect("the mark executes");
    let tally = live.take_scan_tally();
    // The mark's own SQL, and NOT the copy's — `materialise_source` wraps the
    // same text in a `CREATE TEMP TABLE`, so a needle without this second half
    // would be satisfied by the wrong funnel.
    counted(&tally, "mark execute", |sql| {
        sql.contains("SELECT * FROM \"t\"") && !sql.contains("CREATE TEMP TABLE")
    });
}

/// **The copy counts.** `materialise_source` reads the source once, and that
/// read is what an open pays so that its composition need not.
#[test]
fn the_materialise_copy_funnel_counts() {
    let mut live = session(CATEGORICAL);
    live.begin_scan_tally();
    live.materialise_source("t", 512 * 1024 * 1024)
        .expect("the copy fits");
    let tally = live.take_scan_tally();
    counted(&tally, "materialise copy", |sql| {
        sql.contains("__bf_materialised")
    });
}

/// **The profile pass's reads count.** `query_arrow_raw` is the funnel behind
/// `profile_sources`, which is the other term of a file open's wait.
#[test]
fn the_query_arrow_raw_funnel_counts() {
    let live = session(CATEGORICAL);
    let (profiles, tally) = live.profile_sources_counting_scans();
    assert!(!profiles.is_empty(), "the profile pass found no source");
    counted(&tally, "query_arrow_raw", |sql| sql.contains("DESCRIBE"));
}

/// **The unsampled facts and both category reads count** — three funnels, one
/// call, distinguished by the alias each statement carries.
///
/// They are asserted separately rather than through the total because they
/// share an entry point: a total moves when any one of them moves, so it
/// cannot tell which stopped counting.
#[test]
fn the_unsampled_facts_and_both_category_funnels_count() {
    let mut live = session(CATEGORICAL);
    live.set_sample(Some(
        SampleRate::from_exponent(1).expect("a rate of one in two"),
    ));
    live.begin_scan_tally();
    let facts = live
        .unsampled_mark_facts(0)
        .expect("a sampled non-aggregating mark has facts");
    facts.expect("the facts query runs");
    let tally = live.take_scan_tally();
    counted(&tally, "unsampled facts", |sql| sql.contains("__bf_facts"));
    counted(&tally, "unsampled colour categories", |sql| {
        sql.contains("__bf_cats")
    });
    counted(&tally, "unsampled band order", |sql| {
        sql.contains("__bf_band")
    });
}

/// **The cube build and the cube serve count.**
///
/// A brush on the producer builds a cube for the aggregating subscriber and
/// then answers that mark's re-query off it. Both statements read a relation
/// and both are charged; the build is a `CREATE TEMP TABLE` over the base
/// table and the serve is a read of the cube, so the two are told apart by
/// whether the statement creates it.
#[test]
fn the_cube_build_and_cube_serve_funnels_count() {
    let mut live = session(CUBED);
    live.set_preagg_enabled(true);
    live.begin_scan_tally();
    let results = live.propagate_selection(
        "brush",
        ComponentPath("root/hconcat[0]".to_string()),
        Predicate::Interval {
            column: "b".to_string(),
            lo: ScalarValue::Int(5),
            hi: ScalarValue::Int(15),
            meta: None,
        },
    );
    assert!(!results.is_empty(), "the brush dispatched to no mark");
    let stats = live.preagg_stats().clone();
    let tally = live.take_scan_tally();
    assert!(
        stats.cubes_built > 0 && stats.cube_hits > 0,
        "no cube was built ({} built) or none served a re-query ({} hits), so \
         this test drove neither funnel it names",
        stats.cubes_built,
        stats.cube_hits
    );

    counted(&tally, "cube build", |sql| {
        sql.contains("CREATE TEMP TABLE \"__bf_preagg_")
    });
    counted(&tally, "cube serve", |sql| {
        sql.contains("__bf_preagg_") && !sql.contains("CREATE TEMP TABLE")
    });
}
