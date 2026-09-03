//! The correctness oracle for the nearest-point read.
//!
//! A SQL-shape assertion cannot tell a correct nearest-point query from a
//! plausible-but-wrong one — the sibling unit tests in
//! `brightfield_engine::nearest` pin what the string *says*, and that is a
//! different question from what DuckDB *does* with it. So every case here
//! executes against a real session over a real spec and compares the row that
//! comes back with a brute-force scan written independently, in Rust, over the
//! same literal values the spec declares.
//!
//! # Why the fixture is deliberately anisotropic
//!
//! The plot below is 200 pixels across for an x domain of 0–100, and 100
//! pixels down for a y domain of 0–1000. One pixel is therefore half an x unit
//! and ten y units, a twentyfold difference — so a read that measured distance
//! in *data* units rather than in pixels answers a different row from the one
//! the reader is pointing at, and every ordering case here says which. A
//! square fixture would let both readings agree and pin neither.

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::nearest::{NearestAxis, NearestCell, NearestProbe, NearestRead};
use brightfield_engine::{Engine, Session, SqlPredicate};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::{parse_spec, Format};
use brightfield_sql::ir::ScalarValue;

/// The plot's pixel geometry, stated once so the probes below and the
/// brute-force scan cannot disagree about the frame they are in.
const X_DOMAIN: (f64, f64) = (0.0, 100.0);
const Y_DOMAIN: (f64, f64) = (0.0, 1000.0);
const X_RANGE: f64 = 200.0;
const Y_RANGE: f64 = 100.0;

/// Data units per logical pixel, per axis — what a plot's scales hand the
/// probe. `X_UNITS` is 0.5 and `Y_UNITS` is 10.0; see the module note.
const X_UNITS: f64 = (X_DOMAIN.1 - X_DOMAIN.0) / X_RANGE;
const Y_UNITS: f64 = (Y_DOMAIN.1 - Y_DOMAIN.0) / Y_RANGE;

/// One fixture row: `(x, y, label)`.
type Row = (f64, f64, &'static str);

/// The fixture's rows, in the order the spec declares them.
///
/// Chosen so the two readings of "nearest" rank them differently, and
/// **deliberately with no row at the aim**: a row under the pointer is at zero
/// distance under either metric and would let a wrong one pass.
///
/// From [`AT_ORIGIN`] — data `(50, 500)` — `near-in-x` is 4 x units and 0 y
/// units away, which is 8 pixels; `near-in-y` is 0 x units and 30 y units
/// away, which is 3 pixels. So in data units the first is nearest and in
/// pixels the second is, and `the_nearest_read_measures_distance_on_screen`
/// says which one the engine hands back.
const ROWS: &[Row] = &[
    (54.0, 500.0, "near-in-x"),
    (50.0, 530.0, "near-in-y"),
    (90.0, 900.0, "far"),
];

/// The plot-local pixel at data `(50, 500)` — where the two metrics disagree.
const AT_ORIGIN: (f64, f64) = (100.0, 50.0);

/// A dot plot over `ROWS`, optionally narrowed by a selection.
///
/// `filter_by` is what makes the mark the *subset* layer of a generated tile —
/// the layer a hover reads — so the brushed case below runs against the same
/// shape the shipped dashboard draws.
fn spec_yaml(filter_by: bool) -> String {
    let rows: Vec<String> = ROWS
        .iter()
        .map(|(x, y, label)| format!("    - {{ x: {x}, y: {y}, label: {label} }}"))
        .collect();
    let data = if filter_by {
        "{ from: t, filterBy: $brush }"
    } else {
        "{ from: t }"
    };
    format!(
        "params:\n  brush:\n    select: intersect\ndata:\n  t:\n{}\nplot:\n  \
         - mark: dot\n    data: {data}\n    x: x\n    y: y\n",
        rows.join("\n")
    )
}

/// A live session over [`spec_yaml`], wrapped so a selection can be pushed.
fn coordinator(filter_by: bool) -> Coordinator {
    let source = spec_yaml(filter_by);
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the fixture parses")
        .spec;
    let analysis = analyse_spec(&spec).expect("the fixture analyses");
    Coordinator::from_session(session(spec, analysis))
}

fn session(
    spec: brightfield_spec::ast::Spec,
    analysis: brightfield_spec::analysis::SpecAnalysis,
) -> Session {
    Engine::new()
        .load_spec(spec, analysis, None)
        .expect("the fixture loads")
        .session
}

/// The probe a pointer resting at `(px, py)` **plot-local pixels** means,
/// reading the two positional columns and a label column beside them.
fn probe_at(px: f64, py: f64, radius: f64) -> NearestProbe {
    NearestProbe {
        x: NearestAxis {
            column: "x".to_string(),
            at: X_DOMAIN.0 + px * X_UNITS,
            per_pixel: X_UNITS,
        },
        y: NearestAxis {
            column: "y".to_string(),
            at: Y_DOMAIN.0 + py * Y_UNITS,
            per_pixel: Y_UNITS,
        },
        read: vec!["x".to_string(), "y".to_string(), "label".to_string()],
        radius,
    }
}

/// The label the read named, or `None` when it found no row.
fn label(read: &NearestRead) -> Option<&str> {
    read.cells
        .iter()
        .find(|c| c.column == "label")
        .map(|c| c.value.as_str())
}

/// **The brute-force answer**, written independently of the query: the row
/// minimising pixel distance from `(px, py)`, or `None` when the nearest is
/// outside `radius`.
///
/// It scans [`ROWS`] — the literal values the spec declares — rather than what
/// the engine returned, so agreeing with it is a claim about the SQL and not a
/// comparison of the implementation with itself.
fn brute_force(rows: &[Row], px: f64, py: f64, radius: f64) -> Option<&'static str> {
    let mut best: Option<(f64, &'static str)> = None;
    for (x, y, l) in rows {
        let dx = (x - (X_DOMAIN.0 + px * X_UNITS)) / X_UNITS;
        let dy = (y - (Y_DOMAIN.0 + py * Y_UNITS)) / Y_UNITS;
        let d = dx.hypot(dy);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, l));
        }
    }
    best.filter(|(d, _)| *d <= radius).map(|(_, l)| l)
}

/// The pixel a probe is aimed at in most cases below: two pixels right and two
/// down of `probe-point`, so that no row sits exactly under it and the
/// ordering has something to decide.
const AIM: (f64, f64) = (102.0, 52.0);

// ---------------------------------------------------------------------------
// AC2 — the read agrees with a brute-force scan, and refuses outside the radius
// ---------------------------------------------------------------------------

/// The read and an independent scan name the same row, at several aims.
///
/// Every aim is checked against `brute_force` rather than against a row named
/// here, so this stays honest if the fixture moves. The named expectation is
/// carried alongside for the aims where the two readings of "nearest" differ —
/// see the module note.
#[test]
fn the_nearest_read_agrees_with_a_brute_force_scan() {
    let mut c = coordinator(false);
    for (px, py) in [(102.0, 52.0), (100.0, 50.0), (108.0, 50.0), (100.0, 53.0)] {
        let read = c
            .session_mut()
            .nearest_row(0, &probe_at(px, py, 40.0))
            .expect("the read runs");
        assert_eq!(
            label(&read),
            brute_force(ROWS, px, py, 40.0),
            "at ({px}, {py}) the engine and an independent scan disagree"
        );
    }
}

/// **Distance is measured in pixels, not in data units.**
///
/// The aim is [`AT_ORIGIN`], where no row sits: `near-in-x` is 8 pixels away
/// and 4 data units, `near-in-y` is 3 pixels away and 30 data units. The two
/// metrics rank them in opposite orders, so which label comes back says which
/// one ran — and the assertion is stated as the pixel answer by name rather
/// than as agreement with a scan that could be scaled the same wrong way.
#[test]
fn the_nearest_read_measures_distance_on_screen() {
    let mut c = coordinator(false);
    let read = c
        .session_mut()
        .nearest_row(0, &probe_at(AT_ORIGIN.0, AT_ORIGIN.1, 40.0))
        .expect("the read runs");
    assert_eq!(
        label(&read),
        Some("near-in-y"),
        "the row 30 DATA units away is nearer on screen than the row 4 data \
         units away, and the read named the other one — it is measuring in \
         data units"
    );
}

/// Nothing inside the radius is nothing found — not the nearest row anyway.
///
/// Asked at a radius small enough to exclude every row, and paired with a
/// radius that includes one, so a read that always found nothing could not
/// pass this.
#[test]
fn a_rest_outside_the_hit_radius_reads_no_row() {
    let mut c = coordinator(false);
    let far = (10.0, 10.0);
    let tight = c
        .session_mut()
        .nearest_row(0, &probe_at(far.0, far.1, 4.0))
        .expect("the read runs");
    assert!(
        !tight.found() && tight.cells.is_empty(),
        "a rest with no mark inside the radius named {:?}",
        label(&tight)
    );
    assert_eq!(
        brute_force(ROWS, far.0, far.1, 4.0),
        None,
        "the fixture must have nothing within 4 pixels of this aim, or the \
         assertion above passes for the wrong reason"
    );

    let wide = c
        .session_mut()
        .nearest_row(0, &probe_at(far.0, far.1, 400.0))
        .expect("the read runs");
    assert!(
        wide.found(),
        "the same aim at a wide radius found nothing either — the radius is \
         not what refused the read above"
    );
}

// ---------------------------------------------------------------------------
// AC1 — at most one row, and only the columns asked for
// ---------------------------------------------------------------------------

/// **One row, out of a cluster of many.**
///
/// Every row of this fixture sits on the same point, so every one of them is
/// inside the radius and tied on distance. `rows` is the count the query
/// returned, not the count of what was kept, so a read that had lost its bound
/// reports four here while still handing back one row's worth of cells — which
/// is exactly the failure a `cells`-only assertion cannot see.
#[test]
fn the_nearest_read_returns_one_row_from_a_cluster() {
    const CLUSTER: &str = r"
data:
  t:
    - { x: 50, y: 500, label: a }
    - { x: 50, y: 500, label: b }
    - { x: 50, y: 500, label: c }
    - { x: 50, y: 500, label: d }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
";
    let spec = parse_spec(CLUSTER, Format::Yaml)
        .expect("the cluster parses")
        .spec;
    let analysis = analyse_spec(&spec).expect("the cluster analyses");
    let mut s = session(spec, analysis);
    let read = s
        .nearest_row(0, &probe_at(100.0, 50.0, 40.0))
        .expect("the read runs");
    assert_eq!(
        read.rows, 1,
        "four coincident rows are all inside the radius and the read returned \
         {} of them — the client is holding more than one row",
        read.rows
    );
}

/// **The read's result type is exactly its two declared fields wide**, so a
/// batch cannot be riding along inside it.
///
/// The sibling above bounds how many rows the query *returns*. This bounds
/// what the crate hands over, and the two are different questions: a
/// `NearestRead` that carried the `RecordBatch` beside its cells would report
/// `rows == 1` and still put the whole materialised result in the caller's
/// hands, which is the client-side copy the seam exists to reject.
///
/// Asserted as a width rather than by reading the struct, because a width is
/// something a test can watch move. A `RecordBatch` field, a `Vec` of them, a
/// `Box` or an `Arc` around one, or a second `Vec` holding the rows that were
/// not returned, each grow this number. The declared shape is a `usize` and a
/// `Vec`, and the cost of the assertion is that a field added for a good
/// reason has to be added here too — which is the point, since the review that
/// costs is the one for the field that was not added for a good reason.
///
/// `NearestCell` is pinned beside it: widening the cell would widen what one
/// row's worth means without moving the outer type.
#[test]
fn the_reads_result_type_is_exactly_its_two_declared_fields_wide() {
    use std::mem::size_of;

    assert_eq!(
        size_of::<NearestRead>(),
        size_of::<usize>() + size_of::<Vec<NearestCell>>(),
        "`NearestRead` is {} bytes against the {} its declared `rows` and \
         `cells` account for — something else is being carried across the \
         crate boundary with the row",
        size_of::<NearestRead>(),
        size_of::<usize>() + size_of::<Vec<NearestCell>>(),
    );
    assert_eq!(
        size_of::<NearestCell>(),
        2 * size_of::<String>(),
        "`NearestCell` is {} bytes against the {} its two declared `String` \
         fields account for",
        size_of::<NearestCell>(),
        2 * size_of::<String>(),
    );
}

/// The read projects the probe's columns and no others.
///
/// The fixture carries a `label` column the probe below does not ask for. A
/// read that handed back the whole row would carry it, and a client holding a
/// column no channel encodes is the client-side scan this seam exists to
/// reject.
#[test]
fn the_read_holds_the_probes_columns_and_no_others() {
    let mut c = coordinator(false);
    let mut probe = probe_at(AIM.0, AIM.1, 40.0);
    probe.read = vec!["x".to_string(), "y".to_string()];
    let read = c
        .session_mut()
        .nearest_row(0, &probe)
        .expect("the read runs");
    let named: Vec<&str> = read.cells.iter().map(|c| c.column.as_str()).collect();
    assert_eq!(
        named,
        vec!["x", "y"],
        "the probe asked for two columns and the read came back with {named:?}"
    );
}

/// A row with no position cannot be the nearest to a position.
///
/// The null-coordinate row sits at the aim on its non-null axis, so a read
/// that let SQL's three-valued logic through — or that coalesced a NULL to
/// zero — would put it first.
#[test]
fn a_row_with_a_null_coordinate_is_never_the_nearest() {
    const NULLED: &str = r"
data:
  t:
    - { x: 50, y: null, label: nulled }
    - { x: 90, y: 900, label: real }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
";
    let spec = parse_spec(NULLED, Format::Yaml)
        .expect("the null fixture parses")
        .spec;
    let analysis = analyse_spec(&spec).expect("the null fixture analyses");
    let mut s = session(spec, analysis);
    let read = s
        .nearest_row(0, &probe_at(100.0, 50.0, 400.0))
        .expect("the read runs");
    assert_eq!(
        label(&read),
        Some("real"),
        "the row with a NULL y is nearer in x than anything else and was \
         returned anyway"
    );
}

// ---------------------------------------------------------------------------
// AC3 — the row is one the mark is currently drawing
// ---------------------------------------------------------------------------

/// **A committed brush narrows what a hover can find.**
///
/// The selection admits only the far corner of the fixture. A rest over the
/// cluster the brush excluded finds nothing, and the same rest before the
/// brush finds a row — so the refusal is the predicate and not the aim. Then a
/// rest over the corner the brush *kept* finds it, which is what stops this
/// passing on a read that had simply stopped working.
#[test]
fn a_committed_brush_narrows_what_a_hover_can_find() {
    let mut c = coordinator(true);
    let over_cluster = probe_at(AIM.0, AIM.1, 40.0);
    let before = c
        .session_mut()
        .nearest_row(0, &over_cluster)
        .expect("the read runs");
    assert!(before.found(), "the unbrushed read found nothing to lose");

    c.apply(Interaction::Select {
        name: "brush".to_string(),
        contributor: ComponentPath("root".to_string()),
        predicate: SqlPredicate::Interval {
            column: "\"x\"".to_string(),
            lo: ScalarValue::Float(80.0),
            hi: ScalarValue::Float(100.0),
            meta: None,
        },
    });

    let outside = c
        .session_mut()
        .nearest_row(0, &over_cluster)
        .expect("the read runs");
    assert!(
        !outside.found(),
        "a rest over rows the brush excluded named {:?} — the hover is reading \
         rows the chart is no longer drawing",
        label(&outside)
    );

    // 90 x units is pixel 180; 900 y units is pixel 90.
    let inside = c
        .session_mut()
        .nearest_row(0, &probe_at(180.0, 90.0, 40.0))
        .expect("the read runs");
    assert_eq!(
        label(&inside),
        Some("far"),
        "the brush kept this row and the hover cannot find it either"
    );
}

// ---------------------------------------------------------------------------
// AC4 — a production, non-caching read that still counts its execute
// ---------------------------------------------------------------------------

/// The read raises the execute count and leaves the SQL cache where it was.
///
/// Both halves in one test because either alone reads as the property and is
/// not: a read that never ran raises neither, and a read that went through the
/// caching path raises both.
#[test]
fn a_hover_read_raises_the_execute_count_without_touching_the_cache() {
    let mut c = coordinator(false);
    // The chart's own query first, so the cache is not empty and a read that
    // inserted into it would be growing a real cache rather than seeding one.
    c.chart_rows(0).expect("the mark queries");

    let executes = c.session().duckdb_execute_count();
    let cached = c.session().sql_cache_len();
    assert!(
        cached > 0,
        "the chart's query did not cache, so this proves nothing"
    );

    let read = c
        .session_mut()
        .nearest_row(0, &probe_at(AIM.0, AIM.1, 40.0))
        .expect("the read runs");
    assert!(
        read.found(),
        "the read found nothing, so it may not have run"
    );

    assert_eq!(
        c.session().duckdb_execute_count(),
        executes + 1,
        "a hover read is one DuckDB execute and the counter did not move by one"
    );
    assert_eq!(
        c.session().sql_cache_len(),
        cached,
        "the hover read inserted into the renderer-side cache — a stream of \
         one-shot pointer positions will evict the chart's own results"
    );
}

/// A second read at a *different* pixel is a second execute, not a cache hit.
///
/// The pair above proves the read does not write the cache. This proves it
/// does not read it either, which is the other way a hover could quietly stop
/// being a query.
#[test]
fn each_rest_at_a_new_pixel_is_its_own_execute() {
    let mut c = coordinator(false);
    let before = c.session().duckdb_execute_count();
    for px in [100.0, 101.0, 102.0] {
        c.session_mut()
            .nearest_row(0, &probe_at(px, 50.0, 40.0))
            .expect("the read runs");
    }
    assert_eq!(
        c.session().duckdb_execute_count(),
        before + 3,
        "three rests at three pixels are three queries"
    );
}
