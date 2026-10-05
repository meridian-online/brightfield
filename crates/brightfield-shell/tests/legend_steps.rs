//! **A stepped colour scale's legend draws one flat block for each step, whole
//! points high, the highest at the top, with the boundary values beside them; under
//! the plot it draws blocks of equal width, the lowest at the left, with the
//! boundaries under them.** The stack is as tall as the continuous ramp a number
//! column draws in the same room, to within the rounding.
//!
//! The legend is drawn through `legend::draw_band` and `legend::draw_below` into a
//! headless egui context and read back as the shapes it painted, as
//! `legend_short_chart.rs` and `legend_below.rs` do for the ramp. Each test is
//! about where ink went.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{
    band_width, below_blocks, draw_band, draw_below, kept_labels, step_block_size,
    BELOW_RAMP_MAX_WIDTH, RAMP_HEIGHT,
};
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// The band's left edge in the headless context.
const BAND_LEFT: f32 = 400.0;

/// Where the raster's top sits in the same coordinates, which is the band's top.
const RASTER_TOP: f32 = 20.0;

/// Where the raster's top-left is when the page under a plot is painted alone.
const ORIGIN: (f32, f32) = (10.0, 20.0);

/// The column's name, which the legend names itself by.
const NAME: &str = "reading";

/// Eleven rows from 0 to 100, so the domain's ends are whole numbers and the
/// steps' edges are whole numbers too, for five steps.
const VALUES: [f64; 11] = [
    0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0,
];

fn rows() -> String {
    VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, {NAME}: {v} }}\n", i * 3))
        .collect()
}

/// A chart `height` points tall holding a dot plot coloured by [`NAME`] with the
/// legend item in the plot, and `attrs` after it.
fn right(height: u32, attrs: &str) -> String {
    format!(
        "data:\n  t:\n{}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: {NAME}\n  - legend: color\nwidth: 420\nheight: {height}\n{attrs}",
        rows()
    )
}

/// The same plot named `scatter` with a `legend: color` for it in a `vconcat`,
/// and `attrs` as the plot's attributes.
fn under(attrs: &str) -> String {
    let attrs: String = attrs.lines().map(|l| format!("    {l}\n")).collect();
    format!(
        "data:\n  t:\n{}vconcat:\n  - plot:\n      - mark: dot\n        data: {{ from: t }}\n        x: x\n        y: y\n        fill: {NAME}\n    name: scatter\n    width: 420\n    height: 300\n{attrs}  - legend: color\n    for: scatter\n",
        rows()
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// One thing the legend painted.
#[derive(Debug, Clone, PartialEq)]
enum Ink {
    /// Text, with the rect it fills.
    Text(String, egui::Rect),
    /// A solid rect, with its fill.
    Fill(egui::Rect, egui::Color32),
}

fn shapes(run: impl FnOnce(&mut egui::Ui)) -> Vec<Ink> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        ..Default::default()
    };
    let mut run = Some(run);
    let out = ctx.run_ui(raw, |ui| {
        if let Some(run) = run.take() {
            run(ui);
        }
    });
    let mut ink = Vec::new();
    for clipped in &out.shapes {
        match &clipped.shape {
            egui::Shape::Text(t) => ink.push(Ink::Text(
                t.galley.rows.iter().map(|r| r.row.text()).collect(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            egui::Shape::Rect(r) => ink.push(Ink::Fill(r.rect, r.fill)),
            _ => {}
        }
    }
    ink
}

/// What `draw_band` paints for `composed`: the band the chart pane reserves is
/// level with the raster and as tall as the chart.
fn at_right(composed: &Composed) -> Vec<Ink> {
    let band = egui::Rect::from_min_size(
        egui::pos2(BAND_LEFT, RASTER_TOP),
        egui::vec2(band_width(composed), composed.height as f32),
    );
    shapes(|ui| draw_band(ui, band, RASTER_TOP, composed, Mode::Light))
}

/// What `draw_below` paints for `composed`, the raster standing at [`ORIGIN`].
fn below(composed: &Composed) -> Vec<Ink> {
    shapes(|ui| draw_below(ui, egui::pos2(ORIGIN.0, ORIGIN.1), composed, Mode::Light))
}

fn fills(ink: &[Ink]) -> Vec<(egui::Rect, egui::Color32)> {
    ink.iter()
        .filter_map(|i| match i {
            Ink::Fill(r, c) => Some((*r, *c)),
            Ink::Text(..) => None,
        })
        .collect()
}

fn texts(ink: &[Ink]) -> Vec<(String, egui::Rect)> {
    ink.iter()
        .filter_map(|i| match i {
            Ink::Text(t, r) => Some((t.clone(), *r)),
            Ink::Fill(..) => None,
        })
        .collect()
}

/// The labels the legend painted, without the column's name.
fn labels(ink: &[Ink]) -> Vec<(String, egui::Rect)> {
    let all = texts(ink);
    assert_eq!(all[0].0, NAME, "the first text is the column's name");
    all[1..].to_vec()
}

/// The palette colour as the painter quantises it.
fn ink_of(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// The colours of the fill's steps, lowest first, as the scale holds them.
fn step_colours(composed: &Composed) -> Vec<[f32; 4]> {
    match composed.plots[0].scales.get(Channel::Fill) {
        Some(Scale::Quantized { colours, .. }) => colours.clone(),
        other => panic!("the fill is not stepped: {other:?}"),
    }
}

const STEPS: &str = "colorScale: quantize\ncolorN: 5\n";

/// **AC2, at the right.** The legend draws N flat blocks of one whole-point
/// height each, stacked with the lowest step at the foot and the highest at the
/// top, the stack as tall as the continuous ramp to within the rounding, with the
/// N plus one boundary values beside them.
#[test]
fn the_legend_at_the_right_stacks_n_blocks_of_one_whole_height_with_n_plus_one_values() {
    let composed = compose(&right(300, STEPS));
    let colours = step_colours(&composed);
    assert_eq!(colours.len(), 5);
    let ink = at_right(&composed);

    // The blocks, top to bottom: the highest step first.
    let blocks = fills(&ink);
    assert_eq!(blocks.len(), 5, "one block for each step: {blocks:?}");
    let height = blocks[0].0.height();
    assert!(
        height >= 1.0 && height.fract() == 0.0,
        "a whole number of points: {height}"
    );
    for (i, (rect, colour)) in blocks.iter().enumerate() {
        assert_eq!(
            rect.height(),
            height,
            "block {i}: the same height as the first"
        );
        assert_eq!(
            *colour,
            ink_of(colours[4 - i]),
            "block {i} wears step {} — the highest at the top",
            4 - i
        );
        if i > 0 {
            assert_eq!(
                rect.top(),
                blocks[i - 1].0.bottom(),
                "block {i} stands on the one above"
            );
            assert_eq!(rect.left(), blocks[0].0.left(), "in one column");
        }
    }
    let stack = blocks[4].0.bottom() - blocks[0].0.top();
    assert_eq!(stack, 5.0 * height);
    assert!(
        (RAMP_HEIGHT - 5.0..=RAMP_HEIGHT).contains(&stack),
        "as tall as the continuous ramp ({RAMP_HEIGHT}) to within the rounding: {stack}"
    );

    // The boundary values, top to bottom, each level with its line.
    let found = labels(&ink);
    let names: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        names,
        ["100", "80", "60", "40", "20", "0"],
        "N plus one values"
    );
    let top = blocks[0].0.top();
    let foot = blocks[4].0.bottom();
    for (k, (_, rect)) in found.iter().enumerate() {
        let line = top + k as f32 * height;
        let centre = rect.center().y;
        match k {
            0 => assert_eq!(
                rect.top(),
                top,
                "the highest value is level with the stack's top"
            ),
            5 => assert_eq!(
                rect.bottom(),
                foot,
                "the lowest is level with the stack's foot"
            ),
            _ => assert!(
                (centre - line).abs() <= 0.51,
                "value {k} is centred on the line between its blocks: {centre} against {line}"
            ),
        }
        assert!(
            rect.left() >= blocks[0].0.right(),
            "the values stand beside the stack, not over it"
        );
    }
}

/// The stack is the height of the ramp a number column draws, whatever the step
/// count, to within the rounding.
#[test]
fn the_stack_is_as_tall_as_the_ramp_for_each_count() {
    let ramp = {
        let ink = at_right(&compose(&right(300, "")));
        let strips = fills(&ink);
        assert_eq!(
            strips.len(),
            71,
            "fixture check: the continuous ramp's strips"
        );
        strips[70].0.bottom() - strips[0].0.top()
    };
    assert_eq!(ramp, RAMP_HEIGHT);
    for n in [2usize, 3, 4, 5, 6, 7, 10] {
        let composed = compose(&right(300, &format!("colorScale: quantize\ncolorN: {n}\n")));
        let blocks = fills(&at_right(&composed));
        assert_eq!(blocks.len(), n, "{n} steps, {n} blocks");
        let stack = blocks[n - 1].0.bottom() - blocks[0].0.top();
        assert!(
            ramp - (n as f32 - 1.0) <= stack && stack <= ramp,
            "{n} steps: {stack} against the ramp's {ramp}"
        );
    }
}

/// **AC2, under the plot.** N blocks of equal width, the lowest at the left, with
/// the boundaries under them.
#[test]
fn the_legend_under_the_plot_runs_n_equal_blocks_from_the_left_with_the_values_under_them() {
    let composed = compose(&under(STEPS));
    let colours = step_colours(&composed);
    assert_eq!(
        below_blocks(&composed).len(),
        1,
        "the legend is under the plot"
    );
    let ink = below(&composed);

    let blocks = fills(&ink);
    assert_eq!(blocks.len(), 5, "one block for each step: {blocks:?}");
    let width = blocks[0].0.width();
    assert!(
        width >= 1.0 && width.fract() == 0.0,
        "a whole number of points: {width}"
    );
    for (i, (rect, colour)) in blocks.iter().enumerate() {
        assert_eq!(
            rect.width(),
            width,
            "block {i}: the same width as the first"
        );
        assert_eq!(rect.top(), blocks[0].0.top(), "in one row");
        assert_eq!(
            *colour,
            ink_of(colours[i]),
            "block {i} wears step {i} — the lowest at the left"
        );
        if i > 0 {
            assert_eq!(
                rect.left(),
                blocks[i - 1].0.right(),
                "block {i} abuts the one before"
            );
        }
    }
    let row = egui::Rect::from_min_max(blocks[0].0.min, blocks[4].0.max);
    assert!(row.width() <= BELOW_RAMP_MAX_WIDTH);

    let found = labels(&ink);
    let names: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        names,
        ["0", "20", "40", "60", "80", "100"],
        "N plus one values"
    );
    for (k, (_, rect)) in found.iter().enumerate() {
        assert!(
            rect.top() >= row.bottom(),
            "value {k} is under the blocks, not over them"
        );
        let line = row.left() + k as f32 * width;
        match k {
            0 => assert_eq!(
                rect.left(),
                row.left(),
                "the lowest value starts at the row's left"
            ),
            5 => assert_eq!(
                rect.right(),
                row.right(),
                "the highest ends at the row's right"
            ),
            _ => assert!(
                (rect.center().x - line).abs() <= 0.51,
                "value {k} is centred on the line between its blocks: {} against {line}",
                rect.center().x
            ),
        }
    }
}

/// With more steps than the legend has room to label, the values that fit are
/// drawn and none overlaps another; the two at the ends are always drawn.
#[test]
fn many_steps_keep_the_end_values_and_drop_the_ones_that_would_overlap() {
    let composed = compose(&right(300, "colorScale: quantize\ncolorN: 40\n"));
    let ink = at_right(&composed);
    assert_eq!(fills(&ink).len(), 40, "all forty blocks are drawn");
    let found = labels(&ink);
    assert!(found.len() < 41, "not every value fits: {}", found.len());
    assert_eq!(found.first().map(|(t, _)| t.as_str()), Some("100"));
    assert_eq!(found.last().map(|(t, _)| t.as_str()), Some("0"));
    for pair in found.windows(2) {
        assert!(
            pair[0].1.bottom() <= pair[1].1.top(),
            "{:?} overlaps {:?}",
            pair[0],
            pair[1]
        );
    }

    let under = compose(&under("colorScale: quantize\ncolorN: 40\n"));
    let ink = below(&under);
    assert_eq!(fills(&ink).len(), 40);
    let found = labels(&ink);
    assert_eq!(found.first().map(|(t, _)| t.as_str()), Some("0"));
    assert_eq!(found.last().map(|(t, _)| t.as_str()), Some("100"));
    assert!(found.len() < 41);
    for pair in found.windows(2) {
        assert!(
            pair[0].1.right() <= pair[1].1.left(),
            "{:?} overlaps {:?}",
            pair[0],
            pair[1]
        );
    }
}

/// A count of steps the room cannot give a point each to is drawn as the end
/// labels alone, as a ramp too short for its labels is.
#[test]
fn more_steps_than_points_draws_the_end_values_alone() {
    let composed = compose(&right(300, "colorScale: quantize\ncolorN: 200\n"));
    let ink = at_right(&composed);
    assert!(fills(&ink).is_empty(), "no blocks: {:?}", fills(&ink));
    let found = labels(&ink);
    let names: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(names, ["100", "0"]);
}

/// A chart too short for a ramp draws the end values alone for steps too, and the
/// stack comes in whole: no height draws some of the blocks and not the rest.
#[test]
fn a_chart_too_short_for_the_stack_draws_the_end_values_alone() {
    // The margins declared away, as `legend_short_chart.rs` does, so a chart this
    // short composes.
    let attrs = format!("marginTop: 0\nmarginBottom: 0\n{STEPS}");
    let mut labels_alone = Vec::new();
    for height in 20..=200u32 {
        let ink = at_right(&compose(&right(height, &attrs)));
        let blocks = fills(&ink).len();
        assert!(
            blocks == 0 || blocks == 5,
            "a chart {height} tall draws {blocks} blocks"
        );
        if blocks == 0 && texts(&ink).len() > 1 {
            labels_alone.push((height, labels(&ink)));
        }
    }
    assert!(
        !labels_alone.is_empty(),
        "fixture check: some height draws the end values and no stack"
    );
    for (height, found) in &labels_alone {
        assert_eq!(
            found.first().map(|(t, _)| t.as_str()),
            Some("100"),
            "a chart {height} tall puts the maximum first"
        );
    }
    let tall = fills(&at_right(&compose(&right(200, &attrs)))).len();
    assert_eq!(tall, 5, "and a chart 200 tall draws the stack");
}

/// The block size is the whole points each of `steps` blocks gets of `extent`.
#[test]
fn a_block_is_a_whole_number_of_points_and_never_under_one() {
    assert_eq!(step_block_size(142.0, 5), Some(28.0));
    assert_eq!(step_block_size(142.0, 3), Some(47.0));
    assert_eq!(step_block_size(142.0, 142), Some(1.0));
    assert_eq!(step_block_size(142.0, 143), None);
    assert_eq!(step_block_size(0.0, 5), None);
    assert_eq!(step_block_size(142.0, 0), None);
}

/// Which value labels are drawn: the ends always, a middle one when it clears the
/// one drawn before it and the last.
#[test]
fn the_ends_are_always_kept_and_a_middle_label_that_would_touch_is_not() {
    let gap = 2.0;
    assert_eq!(kept_labels(&[], gap), Vec::<usize>::new());
    assert_eq!(kept_labels(&[(0.0, 10.0)], gap), [0]);
    assert_eq!(kept_labels(&[(0.0, 10.0), (20.0, 30.0)], gap), [0, 1]);
    // Three that stand clear of each other are all kept.
    assert_eq!(
        kept_labels(&[(0.0, 10.0), (20.0, 30.0), (40.0, 50.0)], gap),
        [0, 1, 2]
    );
    // The middle one touches the first: dropped, the ends stay.
    assert_eq!(
        kept_labels(&[(0.0, 10.0), (11.0, 21.0), (40.0, 50.0)], gap),
        [0, 2]
    );
    // The middle one touches the last: dropped.
    assert_eq!(
        kept_labels(&[(0.0, 10.0), (30.0, 40.0), (41.0, 50.0)], gap),
        [0, 2]
    );
    // Even two ends that overlap are both kept.
    assert_eq!(kept_labels(&[(0.0, 10.0), (5.0, 15.0)], gap), [0, 1]);
    // A dropped label does not push the next one out: the next is judged against
    // the last one drawn. The second ends at 21, which would push the third out
    // were it judged against the label before it.
    assert_eq!(
        kept_labels(
            &[
                (0.0, 10.0),
                (11.0, 21.0),
                (22.0, 32.0),
                (33.0, 43.0),
                (60.0, 70.0)
            ],
            gap
        ),
        [0, 2, 4]
    );
}
