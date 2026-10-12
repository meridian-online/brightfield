//! **`colorDomain: Fixed` holds a colour ramp's ends from the plot's first draw.**
//!
//! These run the whole composition on a brushed dashboard: the first tile is
//! brushed on `temp`, the second draws the rows the brush leaves, filled by a
//! number. Under `Fixed` the fill scale keeps the ends of the first draw when the
//! brush removes the rows with the largest values; without it the ends re-fit to
//! the rows left, which is the default the pin is read against.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::pipeline::{Composed, LiveDashboard};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;

/// The brushed dashboard; `attrs` is the filtered tile's attributes.
fn brushed_spec(attrs: &str) -> String {
    format!(
        "params:
  brush: {{ select: crossfilter }}
data:
  readings:
    - {{ temp: 1, v: 1 }}
    - {{ temp: 2, v: 2 }}
    - {{ temp: 3, v: 4 }}
    - {{ temp: 4, v: 6 }}
    - {{ temp: 5, v: 8 }}
    - {{ temp: 6, v: 12 }}
    - {{ temp: 7, v: 15 }}
hconcat:
  - plot:
      - mark: dot
        data: {{ from: readings }}
        x: temp
        y: v
      - select: intervalX
        as: $brush
    width: 320
    height: 240
  - plot:
      - mark: dot
        data: {{ from: readings, filterBy: $brush }}
        x: temp
        y: v
        fill: v
    width: 320
    height: 240
    {attrs}
"
    )
}

/// The ends of the filtered tile's fill ramp.
fn fill_ends(composed: &Composed) -> (f64, f64) {
    match composed.plots[1].scales.get(Channel::Fill) {
        Some(Scale::Sequential {
            domain_min,
            domain_max,
            ..
        }) => (*domain_min, *domain_max),
        other => panic!("the fill is not a number ramp: {other:?}"),
    }
}

/// Present, brush `temp` to `0..=4.5` (which takes the rows at 8, 12 and 15 away),
/// and present again: the fill's ends before and after.
fn ends_across_a_sweep(attrs: &str) -> ((f64, f64), (f64, f64)) {
    let mut live =
        LiveDashboard::load_str(&brushed_spec(attrs), None).expect("the spec loads live");
    let before = live.present().expect("first composite");
    let swept = live
        .apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath(before.plots[0].path.clone()),
            predicate: SqlPredicate::Interval {
                column: "temp".to_string(),
                lo: ScalarValue::Float(0.0),
                hi: ScalarValue::Float(4.5),
                meta: None,
            },
        })
        .expect("the sweep re-composites");
    (fill_ends(&before), fill_ends(&swept))
}

#[test]
fn a_fixed_colour_domain_keeps_the_first_draws_ends_across_a_sweep() {
    let (before, after) = ends_across_a_sweep("colorDomain: Fixed");
    assert_eq!(
        before,
        (1.0, 15.0),
        "fixture check: the first draw spans every row"
    );
    assert_eq!(after, before, "the ends of the first draw hold");
}

#[test]
fn without_fixed_the_ends_re_fit_to_the_rows_left() {
    let (before, after) = ends_across_a_sweep("");
    assert_eq!(before, (1.0, 15.0));
    assert_eq!(
        after,
        (1.0, 6.0),
        "the ends shrink to the rows the brush leaves"
    );
}

/// A number column bound to `stroke` builds no ramp under the stroke channel,
/// which is why the colour pin reads the fill alone.
#[test]
fn a_number_stroke_builds_no_ramp_under_the_stroke_channel() {
    let spec = brushed_spec("colorDomain: Fixed").replace("fill: v", "stroke: v");
    let mut live = LiveDashboard::load_str(&spec, None).expect("the spec loads live");
    let composed = live.present().expect("first composite");
    assert!(!matches!(
        composed.plots[1].scales.get(Channel::Stroke),
        Some(Scale::Sequential { .. } | Scale::Diverging { .. } | Scale::Quantized { .. })
    ));
}
