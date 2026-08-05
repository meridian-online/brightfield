//! Gate: a navigation move over a sampled plot re-composites without asking
//! DuckDB to restore the positional domain again.
//!
//! **What a sampled plot owes the reader, and what it costs.** A sampled plot
//! draws its axes from the domain the UNSAMPLED rows span, because a domain
//! inferred from a sample shrinks toward the interior and moves the ticks.
//! That measurement is an aggregate over the positional columns of the rows
//! the sample left unread, and `LiveDashboard::present()` asks for it on each
//! repaint — including the many a single pan produces before the gesture
//! settles.
//!
//! **Why this is asserted against the executed-SQL record rather than against
//! a clock.** A timing assertion is a claim about the machine it runs on; the
//! interval slider's gate declines one for that reason and this declines one
//! for the same. What is asserted here is not that the repaint is fast, but
//! that the statement is not issued. The record answers that, and the same
//! reading proves the record can SEE this class of statement — the settled
//! gesture at the end of each test puts one there — so an empty window is the
//! cache's doing rather than the log's silence.

use brightfield_render::sample_policy::MEASURED_INKED_MAX;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::navigation;
use brightfield_shell::pipeline::LiveDashboard;

/// A row-level dot scatter big enough that the policy samples it with nothing
/// on the command line, which is what puts the facts path in the repaint.
///
/// The row count is taken from the ceiling constant rather than written out,
/// so moving the ceiling moves this fixture with it instead of leaving a test
/// exercising a path the product no longer takes.
fn sampling_scatter() -> String {
    let rows = MEASURED_INKED_MAX + 1;
    format!(
        "data:
  points:
    query: |
      SELECT
        (i * 2654435761 % 100003) / 1000.0            AS spread,
        ((i * 40503 + 12345) % 100019) / 1000.0       AS depth
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    x: spread
    y: depth
width: 640
height: 480
"
    )
}

/// The same shape with a CATEGORICAL y, so the band-order statement the
/// sampling policy's merge added to this path is in the repaint too.
fn sampling_categorical_scatter() -> String {
    let rows = MEASURED_INKED_MAX + 1;
    format!(
        "data:
  points:
    query: |
      SELECT
        (i * 2654435761 % 100003) / 1000.0                AS spread,
        'band-' || (7 - i % 8)::VARCHAR                   AS band
      FROM range({rows}) AS t(i)
plot:
  - mark: dot
    data: {{ from: points }}
    x: spread
    y: band
width: 640
height: 480
"
    )
}

/// A live document over `source`, sampled by the policy, past its first paint.
fn sampled_doc(source: &str) -> ChartDoc {
    let mut live = LiveDashboard::load_str(source, None).expect("the fixture loads live");
    let composed = live.present().expect("first paint");
    assert!(
        composed.plots[0].sample.is_some(),
        "fixture check: this plot must sample itself, or the facts path this \
         test is about never runs"
    );
    let mut doc = ChartDoc::headless(composed);
    doc.attach_live(live);
    doc
}

fn executed(doc: &mut ChartDoc) -> Vec<String> {
    doc.live_coordinator()
        .expect("a live document")
        .session()
        .executed_sql()
}

fn scope_the_window(doc: &mut ChartDoc) {
    doc.live_coordinator()
        .expect("a live document")
        .session_mut()
        .clear_executed_sql();
}

/// One step of a sustained pan: the frame closes in a little and nothing
/// settles.
fn pan_step(doc: &mut ChartDoc, hi: f64) {
    let outcome = navigation::NavOutcome {
        extent: brightfield_render::scale::ViewExtent {
            x: Some((0.0, hi)),
            y: None,
        },
        refused: Vec::new(),
    };
    assert!(doc.note_navigation(0, &outcome), "the frame did not move");
    assert!(
        !doc.pump_navigation(),
        "a mid-gesture step issued the settled re-query"
    );
}

/// **A sustained pan issues no domain-restoration statement.**
///
/// Eight steps of one gesture, each of which re-composites the whole picture.
/// The window is scoped after the first paint, so what the record holds at the
/// end is what the gesture itself sent to DuckDB — and it is nothing.
///
/// The settle at the bottom is the positive control: it moves the session's
/// own extent, which moves the statement, which is a DIFFERENT unsampled
/// picture and has to be measured. Without it an implementation that stopped
/// logging these statements at all would pass the assertion above.
#[test]
fn a_sustained_pan_issues_no_domain_restoration_statement() {
    let mut doc = sampled_doc(&sampling_scatter());
    scope_the_window(&mut doc);

    for step in 1..=8 {
        pan_step(&mut doc, 100.0 - f64::from(step) * 5.0);
    }

    let during = executed(&mut doc);
    assert!(
        during.is_empty(),
        "the pan ran SQL — a repaint at a frame the session's extent has not \
         reached yet re-measured what it already knew: {during:#?}"
    );

    doc.settle_navigation();
    assert!(
        doc.pump_navigation(),
        "the settled gesture never re-queried"
    );
    let after = executed(&mut doc);
    assert!(
        after.iter().any(|sql| sql.contains("__bf_facts")),
        "the settled extent is a different unsampled picture and was not \
         measured, so the empty record above proves nothing: {after:#?}"
    );
}

/// **The same holds for the band-order statement.**
///
/// A categorical positional axis adds a second statement to this path, and it
/// is the one whose answer decides where the marks are laid out — a band
/// scale gives each category a slot by its index in that list. It rides the
/// same fact set and therefore the same key.
#[test]
fn a_sustained_pan_issues_no_band_order_statement() {
    let mut doc = sampled_doc(&sampling_categorical_scatter());
    scope_the_window(&mut doc);

    for step in 1..=8 {
        pan_step(&mut doc, 100.0 - f64::from(step) * 5.0);
    }

    let during = executed(&mut doc);
    assert!(
        during.is_empty(),
        "the pan ran SQL over a categorical axis: {during:#?}"
    );

    doc.settle_navigation();
    assert!(
        doc.pump_navigation(),
        "the settled gesture never re-queried"
    );
    let after = executed(&mut doc);
    assert!(
        after.iter().any(|sql| sql.contains("__bf_band")),
        "the band order was not re-measured at the settled extent, so the \
         empty record above proves nothing: {after:#?}"
    );
}

/// **What the reader sees does not move under the gesture.**
///
/// The record says no statement ran. This says the axes are still drawn from
/// the unsampled domain while it does not — the outcome the statement exists
/// to produce, read off the composed plot rather than off the cache.
#[test]
fn the_restored_domain_survives_a_pan_unchanged() {
    let mut doc = sampled_doc(&sampling_scatter());
    let before = doc
        .composed
        .plots
        .first()
        .and_then(|p| p.sample)
        .expect("fixture check: the plot draws a sample notice");

    for step in 1..=8 {
        pan_step(&mut doc, 100.0 - f64::from(step) * 5.0);
    }

    let after = doc
        .composed
        .plots
        .first()
        .and_then(|p| p.sample)
        .expect("the notice survived the gesture");
    assert_eq!(
        (after.drawn, after.of),
        (before.drawn, before.of),
        "the notice moved under a gesture that queried nothing"
    );
}
