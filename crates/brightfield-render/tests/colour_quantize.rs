//! **A stepped colour scale cuts its domain into equal steps, each one flat colour
//! of the ramp it was cut from; a value takes the step it falls in, and a step
//! holds its low edge and not its high one.**
//!
//! These read the scale directly: the cut, the edges the legend labels, the
//! override that makes one from a dot's ramp, and the reversal. The shell's
//! `colour_vocabulary_steps.rs` reads what a whole plot paints.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{apply_colour_override, ColourOverride, Scale, ScaleSet};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn grey(level: f32) -> [f32; 4] {
    [level, level, level, 1.0]
}

fn ramp(domain_min: f64, domain_max: f64) -> Scale {
    Scale::Sequential {
        domain_min,
        domain_max,
        stops: vec![BLACK, WHITE],
    }
}

fn set_with(scale: Scale) -> ScaleSet {
    let mut set = ScaleSet::new();
    set.insert(Channel::Fill, scale);
    set
}

fn fill(set: &ScaleSet) -> &Scale {
    set.get(Channel::Fill).expect("a fill scale")
}

/// The first step wears the ramp's low end, the last its high end, and the ones
/// between are at even spacing along it.
#[test]
fn the_steps_sample_the_ramp_at_even_spacing_from_end_to_end() {
    let scale = Scale::quantized(0.0, 100.0, &[BLACK, WHITE], 5);
    let Scale::Quantized { colours, .. } = &scale else {
        panic!("not stepped: {scale:?}");
    };
    assert_eq!(
        colours,
        &[grey(0.0), grey(0.25), grey(0.5), grey(0.75), grey(1.0)]
    );
    // A ramp of more than two stops is sampled along its length, not stop by stop.
    let three = Scale::quantized(0.0, 1.0, &[RED, GREEN, BLUE], 5);
    let Scale::Quantized { colours, .. } = &three else {
        panic!("not stepped: {three:?}");
    };
    assert_eq!(
        colours,
        &[RED, [0.5, 0.5, 0.0, 1.0], GREEN, [0.0, 0.5, 0.5, 1.0], BLUE]
    );
    // Two steps are the ramp's two ends; one step is its low end.
    let Scale::Quantized { colours, .. } = Scale::quantized(0.0, 1.0, &[BLACK, WHITE], 2) else {
        panic!("not stepped");
    };
    assert_eq!(colours, [BLACK, WHITE]);
    let Scale::Quantized { colours, .. } = Scale::quantized(0.0, 1.0, &[BLACK, WHITE], 1) else {
        panic!("not stepped");
    };
    assert_eq!(colours, [BLACK]);
}

/// A step holds its low edge and not its high one; the last holds its high edge
/// too; a value outside the domain is in the nearest end step.
#[test]
fn a_value_takes_the_step_it_falls_in_and_a_step_holds_its_low_edge() {
    let scale = Scale::quantized(0.0, 100.0, &[BLACK, WHITE], 5);
    for (value, step) in [
        (-5.0, 0),
        (0.0, 0),
        (19.999, 0),
        (20.0, 1),
        (39.999, 1),
        (40.0, 2),
        (59.999, 2),
        (60.0, 3),
        (79.999, 3),
        (80.0, 4),
        (100.0, 4),
        (1.0e9, 4),
        (f64::NAN, 0),
    ] {
        assert_eq!(scale.step_of(value), Some(step), "{value}");
    }
    assert_eq!(ramp(0.0, 1.0).step_of(0.5), None, "a ramp has no steps");
    // The colour a value maps to is its step's.
    assert_eq!(scale.map_continuous(19.999), grey(0.0));
    assert_eq!(scale.map_continuous(20.0), grey(0.25));
    assert_eq!(scale.map_continuous(100.0), grey(1.0));
}

/// The edges the legend labels are the ones that cut the points: one more than
/// there are steps, the first and last the domain's ends exactly, and a value
/// at an edge is in the step above it.
#[test]
fn the_edges_are_equal_apart_and_are_the_ones_the_cut_uses() {
    let scale = Scale::quantized(0.0, 500_001.0, &[BLACK, WHITE], 5);
    let edges = scale.step_edges().expect("a stepped scale has edges");
    assert_eq!(edges.len(), 6);
    assert_eq!(edges[0], 0.0);
    assert_eq!(
        edges[5], 500_001.0,
        "the last edge is the domain's end exactly"
    );
    for pair in edges.windows(2) {
        assert!(
            (pair[1] - pair[0] - 100_000.2).abs() < 1e-6,
            "equal widths: {edges:?}"
        );
    }
    for (k, edge) in edges.iter().enumerate().take(5) {
        assert_eq!(scale.step_of(*edge), Some(k), "edge {k} is in step {k}");
    }
    // An awkward domain: the last edge is still the end, and the edges rise.
    let awkward = Scale::quantized(0.15, 5.0, &[BLACK, WHITE], 5);
    let edges = awkward.step_edges().unwrap();
    assert_eq!(edges[0], 0.15);
    assert_eq!(edges[5], 5.0);
    assert!(edges.windows(2).all(|p| p[0] < p[1]), "{edges:?}");
    // A domain whose width is not exact in floating point: the span times the step
    // count over the step count is 0.9000000000000001 here, and the last edge is
    // still the domain's end to the bit.
    let inexact = Scale::quantized(0.3, 0.9, &[BLACK, WHITE], 5);
    assert_eq!(inexact.step_edges().unwrap()[5], 0.9);
    assert_eq!(ramp(0.0, 1.0).step_edges(), None);
}

/// A domain with no width has every edge at one value: a value at or past it is in
/// the top step, as a ramp's degenerate domain maps to its top.
#[test]
fn a_domain_with_no_width_puts_a_value_at_it_in_the_top_step() {
    let scale = Scale::quantized(5.0, 5.0, &[BLACK, WHITE], 4);
    assert_eq!(scale.step_of(5.0), Some(3));
    assert_eq!(scale.step_of(9.0), Some(3));
    assert_eq!(scale.step_of(1.0), Some(0));
    assert_eq!(scale.domain_min(), Some(5.0));
    assert_eq!(scale.domain_max(), Some(5.0));
}

fn stepped(steps: usize) -> ColourOverride {
    ColourOverride {
        steps: Some(steps),
        ..ColourOverride::default()
    }
}

/// The override cuts the dot's ramp after the domain is fixed, so the steps divide
/// the domain drawn.
#[test]
fn the_override_cuts_the_ramp_over_the_domain_it_was_given() {
    let mut set = set_with(ramp(0.0, 80.0));
    apply_colour_override(&mut set, &stepped(5));
    assert_eq!(
        fill(&set).step_edges().unwrap(),
        [0.0, 16.0, 32.0, 48.0, 64.0, 80.0]
    );

    let mut set = set_with(ramp(0.0, 80.0));
    let ov = ColourOverride {
        domain: Some((0.0, 50.0)),
        steps: Some(5),
        ..ColourOverride::default()
    };
    apply_colour_override(&mut set, &ov);
    assert_eq!(
        fill(&set).step_edges().unwrap(),
        [0.0, 10.0, 20.0, 30.0, 40.0, 50.0],
        "the fixed domain is cut, not the rows'"
    );
}

/// A `colorRange` of two or more colours gives the steps their colours and their
/// count, and `colorN` does not change either.
#[test]
fn a_range_gives_the_steps_their_colours_and_their_count() {
    for steps in [2, 4, 9] {
        let mut set = set_with(ramp(0.0, 100.0));
        let ov = ColourOverride {
            range: Some(vec![RED, GREEN, BLUE, WHITE]),
            steps: Some(steps),
            ..ColourOverride::default()
        };
        apply_colour_override(&mut set, &ov);
        let Scale::Quantized { colours, .. } = fill(&set) else {
            panic!("not stepped: {:?}", fill(&set));
        };
        assert_eq!(colours, &[RED, GREEN, BLUE, WHITE], "colorN {steps}");
    }
    // A range of one colour is no ramp: the steps come from the ramp, as before.
    let mut set = set_with(ramp(0.0, 100.0));
    let ov = ColourOverride {
        range: Some(vec![RED]),
        steps: Some(3),
        ..ColourOverride::default()
    };
    apply_colour_override(&mut set, &ov);
    let Scale::Quantized { colours, .. } = fill(&set) else {
        panic!("not stepped");
    };
    assert_eq!(colours, &[BLACK, grey(0.5), WHITE]);
}

/// With no steps asked for the ramp is untouched, and a string column's colour
/// scale and a diverging ramp are not cut.
#[test]
fn only_a_sequential_ramp_is_cut() {
    let mut set = set_with(ramp(0.0, 100.0));
    apply_colour_override(&mut set, &ColourOverride::default());
    assert_eq!(
        format!("{:?}", fill(&set)),
        format!("{:?}", ramp(0.0, 100.0))
    );

    let categories = Scale::Colour {
        categories: vec!["a".into(), "b".into()],
        palette: vec![RED, GREEN],
    };
    let mut set = set_with(categories.clone());
    apply_colour_override(&mut set, &stepped(5));
    assert_eq!(
        format!("{:?}", fill(&set)),
        format!("{categories:?}"),
        "a string fill stays categorical"
    );

    let diverging = Scale::Diverging {
        domain_min: -1.0,
        domain_max: 1.0,
        pivot: 0.0,
        stops: vec![RED, WHITE, BLUE],
    };
    let mut set = set_with(diverging.clone());
    apply_colour_override(&mut set, &stepped(5));
    assert_eq!(
        format!("{:?}", fill(&set)),
        format!("{diverging:?}"),
        "a diverging ramp is not stepped"
    );
}

/// The override that asks for steps is not empty, so the shell applies it.
#[test]
fn an_override_that_asks_for_steps_is_not_empty() {
    assert!(ColourOverride::default().is_empty());
    assert!(!stepped(5).is_empty());
}

/// A reversal turns the colours and keeps the edges.
#[test]
fn reversing_turns_the_colours_and_keeps_the_edges() {
    let scale = Scale::quantized(0.0, 100.0, &[BLACK, WHITE], 5);
    let turned = scale.colour_reversed();
    let (Scale::Quantized { colours: a, .. }, Scale::Quantized { colours: b, .. }) =
        (&scale, &turned)
    else {
        panic!("not stepped");
    };
    let mut flipped = a.clone();
    flipped.reverse();
    assert_eq!(b, &flipped);
    assert_eq!(turned.step_edges(), scale.step_edges());
    assert_eq!(
        turned.map_continuous(5.0),
        grey(1.0),
        "the first step wears the last colour"
    );
}
