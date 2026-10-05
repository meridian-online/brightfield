//! **`apply_colour_override` sets a diverging scale's ends and its colours and
//! leaves its pivot where it was.**
//!
//! A sequential scale has no pivot, and the override had no arm for a scale that
//! does: a plot that wrote `colorScale: diverging` with a `colorDomain` kept the
//! ends its rows gave. The arm takes the written ends as they stand, so a domain
//! uneven about the pivot gives each arm of the ramp its own span.

use brightfield_render::channel::Channel;
use brightfield_render::ink::ChartInk;
use brightfield_render::scale::{apply_colour_override, ColourOverride, Scale, ScaleSet};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// A diverging fill scale the rows gave: even about 2, reaching 8 either side,
/// red to white to blue.
fn drawn() -> ScaleSet {
    let mut set = ScaleSet::in_ink(ChartInk::LIGHT);
    set.insert(
        Channel::Fill,
        Scale::Diverging {
            domain_min: -6.0,
            domain_max: 10.0,
            pivot: 2.0,
            stops: vec![RED, WHITE, BLUE],
        },
    );
    set
}

fn fill(set: &ScaleSet) -> Scale {
    set.get(Channel::Fill).expect("a fill scale").clone()
}

/// The diverging scale's parts, or a failure naming what the fill was instead.
fn parts(set: &ScaleSet) -> (f64, f64, f64, Vec<[f32; 4]>) {
    match fill(set) {
        Scale::Diverging {
            domain_min,
            domain_max,
            pivot,
            stops,
        } => (domain_min, domain_max, pivot, stops),
        other => panic!("the fill is not diverging: {other:?}"),
    }
}

/// A written domain replaces the ends as written, the pivot stays, and the stops
/// stay when no range was written.
#[test]
fn a_domain_sets_the_ends_and_the_pivot_stays() {
    let mut set = drawn();
    apply_colour_override(
        &mut set,
        &ColourOverride {
            domain: Some((0.0, 10.0)),
            ..ColourOverride::default()
        },
    );
    assert_eq!(parts(&set), (0.0, 10.0, 2.0, vec![RED, WHITE, BLUE]));
    // The arms run over their own spans: 1 is halfway down the lower arm, so a
    // quarter of the way along the ramp.
    let at_one = fill(&set).map_continuous(1.0);
    assert_eq!(
        at_one,
        [1.0, 0.5, 0.5, 1.0],
        "1 is halfway between red and white"
    );
}

/// A range of two or more colours replaces the stops; one colour is no ramp and
/// leaves them; the ends stay when no domain was written.
#[test]
fn a_range_of_two_or_more_colours_sets_the_stops() {
    let mut set = drawn();
    apply_colour_override(
        &mut set,
        &ColourOverride {
            range: Some(vec![RED, GREEN, BLUE, WHITE, RED]),
            ..ColourOverride::default()
        },
    );
    assert_eq!(
        parts(&set),
        (-6.0, 10.0, 2.0, vec![RED, GREEN, BLUE, WHITE, RED]),
        "five colours replace the stops and the rows' ends stay"
    );
    assert_eq!(
        fill(&set).map_continuous(2.0),
        BLUE,
        "the middle of an odd count is at the pivot"
    );

    let mut set = drawn();
    apply_colour_override(
        &mut set,
        &ColourOverride {
            range: Some(vec![GREEN]),
            ..ColourOverride::default()
        },
    );
    assert_eq!(
        parts(&set),
        (-6.0, 10.0, 2.0, vec![RED, WHITE, BLUE]),
        "one colour is no ramp"
    );
}

/// A list of categories does not fit a diverging scale and is ignored; an empty
/// override leaves the scale as it was.
#[test]
fn a_category_list_and_an_empty_override_leave_the_scale() {
    for ov in [
        ColourOverride {
            categories: Some(vec!["a".into(), "b".into()]),
            ..ColourOverride::default()
        },
        ColourOverride::default(),
    ] {
        let mut set = drawn();
        apply_colour_override(&mut set, &ov);
        assert_eq!(parts(&set), (-6.0, 10.0, 2.0, vec![RED, WHITE, BLUE]));
    }
}
