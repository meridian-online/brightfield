//! **A file that holds a log scale and two ends through zero draws no degenerate
//! axis, and says why.**
//!
//! `xScale: log` with `xDomain: [0, 100]` used to draw a few dozen unlabeled `0`
//! ticks with every dot against one edge. The draw now leaves ends through zero,
//! so the axis runs over its rows, and the load warns that the ends were left.
//! The shelf's range row refuses the same ends as they are typed, in
//! `shelf_settings_range.rs`.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::pipeline::{Composed, LiveDashboard};

const DOTS: &str = r"
data:
  pts:
    - { a: 4,   b: 3 }
    - { a: 50,  b: 60 }
    - { a: 96,  b: 97 }
    - { a: 150, b: 60 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
name: probe
width: 600
height: 300
ATTRS
";

fn compose(attrs: &str) -> Composed {
    LiveDashboard::load_str(&DOTS.replace("ATTRS", attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

/// The ends of the log scale x was drawn against.
fn log_ends(composed: &Composed) -> (f64, f64) {
    match composed.plots[0]
        .scales
        .get(Channel::X)
        .expect("the plot drew x")
    {
        Scale::Log {
            domain_min,
            domain_max,
            ..
        } => (*domain_min, *domain_max),
        other => panic!("x was not drawn as a log scale: {other:?}"),
    }
}

/// What the load said, one line per diagnostic: what the warning banner draws.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

/// **AC6.** The draw leaves ends through zero on a log axis, so the axis runs over
/// its rows, and the load warns, naming the key and the ends it was given.
#[test]
fn a_log_axis_over_a_range_through_zero_runs_over_its_rows_and_the_load_warns() {
    let rows = compose("xScale: log");
    assert!(said(&rows).is_empty(), "the control loads clean");
    let (rows_lo, rows_hi) = log_ends(&rows);
    assert!(rows_lo > 0.0, "a log axis over rows reaches above zero");

    let refused = compose("xScale: log\nxDomain: [0, 100]");
    assert_eq!(
        log_ends(&refused),
        (rows_lo, rows_hi),
        "the axis ran over its rows, not over 0 to 100"
    );
    let lines = said(&refused);
    assert!(
        lines.iter().any(|l| l.contains("`xDomain: [0, 100]`")
            && l.contains("a log axis cannot draw")
            && l.contains("the axis runs over its rows")),
        "the load said {lines:?}"
    );
}

/// **AC6.** A pair held under `xyDomain` is judged the same way, and the warning
/// names that key.
#[test]
fn a_pair_under_xydomain_through_zero_on_a_log_axis_is_left_and_named() {
    let rows = compose("xScale: log");
    let refused = compose("xScale: log\nxyDomain: [-5, 100]");
    assert_eq!(log_ends(&refused), log_ends(&rows));
    let lines = said(&refused);
    assert!(
        lines.iter().any(|l| l.contains("`xyDomain: [-5, 100]`")),
        "the load said {lines:?}"
    );
}

/// **AC6.** Ends above zero on a log axis are the analyst's and draw as written,
/// with no warning, so the refusal above is the zero and not the log.
#[test]
fn ends_above_zero_on_a_log_axis_draw_as_written_and_do_not_warn() {
    let pinned = compose("xScale: log\nxDomain: [1, 100]");
    assert_eq!(log_ends(&pinned), (1.0, 100.0));
    assert!(said(&pinned).is_empty(), "{:?}", said(&pinned));
}
