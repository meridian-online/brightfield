//! Axis rendering — tick computation, tick marks, labels, and axis lines.
//!
//! Tick computation is a pure function: `compute_ticks(scale, target_count) ->
//! Vec<Tick>`. The scene builder draws ticks as lines and labels as text.

use kurbo::{Affine, Line, Point};
use vello::Scene;

use crate::ink::ChartInk;
use crate::layout::ChartLayout;
use crate::scale::Scale;
use crate::text::{
    draw_text, draw_text_rotated, measure_width, TextAnchor, LABEL_SIZE, TITLE_SIZE,
};

/// A computed tick mark with its position and label.
#[derive(Debug, Clone)]
pub struct Tick {
    /// The data value this tick represents.
    pub value: f64,
    /// Human-readable label string.
    pub label: String,
    /// Pixel position along the axis.
    pub position: f64,
}

// The tick and axis inks are [`ChartInk::tick`] and [`ChartInk::axis`] — the
// mode's baseline ink. Recessive axes: the domain line and ticks sit back while
// the data ink carries the chart; tick-label TEXT stays legible via the muted
// ink ([`ChartInk::label`]), which is a step closer to the primary.

/// Tick mark length in pixels.
const TICK_LENGTH: f64 = 5.0;

/// Gap (px) between the tick-label band and an axis / plot title baseline.
const TITLE_GAP: f64 = 4.0;

/// The y-axis title baseline's x, measured from the plot's left window edge: it
/// sits in the leftmost grown-margin band, left of the (right-aligned) tick
/// labels. Fixed-band placement — a pathologically wide tick label is the
/// recorded measured-fit deferral, not handled here.
const Y_TITLE_X: f64 = 12.0;

/// Baseline y for the x-axis title — below the tick-label band, inside the
/// (grown) bottom margin. Exposed so the tick-clearance test can pin it.
pub(crate) fn x_title_baseline(layout: &ChartLayout) -> f64 {
    layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE) + f64::from(TITLE_SIZE) + TITLE_GAP
}

/// Baseline y for the plot title — above the frame, inside the (grown) top
/// margin, never above the window top edge.
pub(crate) fn plot_title_baseline(layout: &ChartLayout) -> f64 {
    (layout.plot_y_start() - TITLE_GAP).max(f64::from(TITLE_SIZE))
}

/// Render a per-plot title above the frame, left-aligned at the frame's left
/// edge (Observable Plot parity). Called only when the plot declares a title
/// (so the top margin has grown to make room).
pub fn render_plot_title(scene: &mut Scene, layout: &ChartLayout, title: &str, ink: ChartInk) {
    draw_text(
        scene,
        title,
        layout.plot_x_start(),
        plot_title_baseline(layout),
        TITLE_SIZE,
        ink.title,
        TextAnchor::Start,
    );
}

/// Compute ticks for a scale.
///
/// Returns tick marks with positions and labels appropriate for the scale type.
pub fn compute_ticks(scale: &Scale, target_count: usize) -> Vec<Tick> {
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => compute_linear_ticks(
            *domain_min,
            *domain_max,
            *range_start,
            *range_end,
            target_count,
        ),
        Scale::Band {
            categories,
            range_start,
            range_end,
            padding,
        } => compute_band_ticks(categories, *range_start, *range_end, *padding),
        Scale::Time {
            domain_min_us,
            domain_max_us,
            range_start,
            range_end,
        } => compute_time_ticks(
            *domain_min_us,
            *domain_max_us,
            *range_start,
            *range_end,
            target_count,
        ),
        // Colour ramps (categorical or sequential) have no positional axis ticks.
        Scale::Colour { .. } | Scale::Sequential { .. } => Vec::new(),
    }
}

fn compute_linear_ticks(
    domain_min: f64,
    domain_max: f64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
) -> Vec<Tick> {
    let span = domain_max - domain_min;
    if span.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step = nice_step(span, target_count);
    let first = (domain_min / step).ceil() * step;

    let mut ticks = Vec::new();
    let mut value = first;
    while value <= domain_max + step * 0.001 {
        let t = (value - domain_min) / span;
        let position = range_start + t * (range_end - range_start);
        let label = format_number(value);
        ticks.push(Tick {
            value,
            label,
            position,
        });
        value += step;
    }
    ticks
}

fn compute_band_ticks(
    categories: &[String],
    range_start: f64,
    range_end: f64,
    padding: f64,
) -> Vec<Tick> {
    let n = categories.len() as f64;
    if n == 0.0 {
        return vec![];
    }
    let total = range_end - range_start;
    let band = total / n;

    categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let centre = range_start
                + band * (padding / 2.0)
                + band * i as f64
                + band * (1.0 - padding) / 2.0;
            Tick {
                value: i as f64,
                label: cat.clone(),
                position: centre,
            }
        })
        .collect()
}

fn compute_time_ticks(
    domain_min_us: i64,
    domain_max_us: i64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
) -> Vec<Tick> {
    let span_us = (domain_max_us - domain_min_us) as f64;
    if span_us.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step_us = nice_step(span_us, target_count);
    let first = ((domain_min_us as f64 / step_us).ceil() * step_us) as i64;

    let mut ticks = Vec::new();
    let mut value_us = first;
    while value_us <= domain_max_us {
        let t = (value_us - domain_min_us) as f64 / span_us;
        let position = range_start + t * (range_end - range_start);
        // Format as seconds for simplicity in v1.
        let seconds = value_us as f64 / 1_000_000.0;
        let label = format!("{seconds:.1}s");
        ticks.push(Tick {
            value: value_us as f64,
            label,
            position,
        });
        value_us += step_us as i64;
    }
    ticks
}

/// Compute a "nice" step size for tick spacing.
fn nice_step(span: f64, target_count: usize) -> f64 {
    let raw_step = span / target_count as f64;
    let magnitude = 10_f64.powf(raw_step.log10().floor());
    let residual = raw_step / magnitude;

    let nice = if residual <= 1.5 {
        1.0
    } else if residual <= 3.5 {
        2.0
    } else if residual <= 7.5 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}

/// Format a number for tick labels.
pub(crate) fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Horizontal clearance a drawn tick label must keep from its neighbour's, in
/// pixels — [`labels_clear_horizontally`]'s threshold. Small on purpose: it is
/// not a design gap, it is the minimum that keeps two adjacent digits from
/// reading as one run of glyphs.
const LABEL_CLEARANCE: f64 = 4.0;

/// The vertical gap between a tick mark and a rotated label's near end, in
/// pixels — the rotation fallback in `render_x_axis` anchors the label's last
/// character here rather than at the horizontal band's baseline, since a
/// rotated run's own length is what needs the room a fixed baseline assumes a
/// horizontal one has.
const ROTATED_LABEL_GAP: f64 = 3.0;

/// Whether consecutive labels in `ticks` — each centred under its own tick's
/// `position` at `size`, the anchor `render_x_axis` draws with — clear
/// [`LABEL_CLEARANCE`] of their neighbour's.
///
/// Reads `position` as a distance, not a direction — `compute_ticks` hands
/// `ticks` back walking the domain low to high, which is ascending screen
/// position on an x axis and descending on a y axis reused through this same
/// path, and a neighbour gap is a collision risk either way.
fn labels_clear_horizontally(ticks: &[&Tick], size: f32) -> bool {
    ticks.windows(2).all(|pair| {
        let gap = (pair[1].position - pair[0].position).abs();
        let reach =
            measure_width(&pair[0].label, size) / 2.0 + measure_width(&pair[1].label, size) / 2.0;
        gap - reach >= LABEL_CLEARANCE
    })
}

/// The widest evenly-strided subset of `ticks` whose labels clear each other
/// horizontally ([`labels_clear_horizontally`]) at `size` — Observable Plot's
/// own answer to a crowded axis: try drawing each label, then try dropping
/// alternating ones, then try a wider stride still, and so on until a stride
/// clears or the search has narrowed to the two end ticks. `render_x_axis`
/// rotates the full set instead of drawing this candidate when even that pair
/// collides.
fn thinned_x_ticks(ticks: &[Tick], size: f32) -> Vec<&Tick> {
    if ticks.len() < 2 {
        return ticks.iter().collect();
    }
    for stride in 1..ticks.len() {
        let subset: Vec<&Tick> = ticks.iter().step_by(stride).collect();
        if labels_clear_horizontally(&subset, size) {
            return subset;
        }
    }
    vec![&ticks[0], &ticks[ticks.len() - 1]]
}

/// Render the x-axis into the scene. `title`, when `Some`, is drawn centred
/// below the tick-label band (the bottom margin has grown to make room).
///
/// A tick mark draws for each tick regardless of what its label does —
/// dropping one would misstate which values the axis carries. A tick label
/// draws thinned first ([`thinned_x_ticks`]) and rotated a quarter turn
/// ([`draw_text_rotated`]) when even the sparsest horizontal set still
/// collides. `thinning_keeps_labels_from_touching` and
/// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels` (this
/// module's tests) pin the two branches.
pub fn render_x_axis(
    scene: &mut Scene,
    layout: &ChartLayout,
    ticks: &[Tick],
    title: Option<&str>,
    ink: ChartInk,
) {
    let y = layout.plot_y_end();
    let stroke = kurbo::Stroke::new(1.0);

    // Axis line.
    let axis_line = Line::new(
        Point::new(layout.plot_x_start(), y),
        Point::new(layout.plot_x_end(), y),
    );
    scene.stroke(&stroke, Affine::IDENTITY, ink.axis, None, &axis_line);

    // Tick marks, independent of which labels draw.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(tick.position, y),
            Point::new(tick.position, y + TICK_LENGTH),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.tick, None, &tick_line);
    }

    // Tick labels: thin before rotating.
    let thinned = thinned_x_ticks(ticks, LABEL_SIZE);
    if labels_clear_horizontally(&thinned, LABEL_SIZE) {
        for tick in thinned {
            draw_text(
                scene,
                &tick.label,
                tick.position,
                y + TICK_LENGTH + f64::from(LABEL_SIZE),
                LABEL_SIZE,
                ink.label,
                TextAnchor::Middle,
            );
        }
    } else {
        // Even the two end labels collide horizontally — rotate the full set
        // a quarter turn so each label's OWN footprint is its font size
        // rather than its text width, anchored (`TextAnchor::End`) so the
        // label's last character sits nearest the tick and the rest reaches
        // down into the margin instead of up into the plot.
        for tick in ticks {
            draw_text_rotated(
                scene,
                &tick.label,
                tick.position,
                y + TICK_LENGTH + ROTATED_LABEL_GAP,
                LABEL_SIZE,
                ink.label,
                TextAnchor::End,
            );
        }
    }

    // Axis title, centred below the tick-label band.
    if let Some(title) = title {
        draw_text(
            scene,
            title,
            (layout.plot_x_start() + layout.plot_x_end()) / 2.0,
            x_title_baseline(layout),
            TITLE_SIZE,
            ink.title,
            TextAnchor::Middle,
        );
    }
}

/// Render the y-axis into the scene. `title`, when `Some`, is drawn rotated a
/// quarter-turn up the (grown) left margin, left of the tick labels.
pub fn render_y_axis(
    scene: &mut Scene,
    layout: &ChartLayout,
    ticks: &[Tick],
    title: Option<&str>,
    ink: ChartInk,
) {
    let x = layout.plot_x_start();
    let stroke = kurbo::Stroke::new(1.0);

    // Axis line.
    let axis_line = Line::new(
        Point::new(x, layout.plot_y_start()),
        Point::new(x, layout.plot_y_end()),
    );
    scene.stroke(&stroke, Affine::IDENTITY, ink.axis, None, &axis_line);

    // Tick marks and labels.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(x - TICK_LENGTH, tick.position),
            Point::new(x, tick.position),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.tick, None, &tick_line);

        // Label, right-aligned in the left margin and vertically centred on the tick.
        draw_text(
            scene,
            &tick.label,
            x - TICK_LENGTH - 3.0,
            tick.position + f64::from(LABEL_SIZE) / 3.0,
            LABEL_SIZE,
            ink.label,
            TextAnchor::End,
        );
    }

    // Axis title, rotated bottom-to-top and centred on the plot height.
    if let Some(title) = title {
        draw_text_rotated(
            scene,
            title,
            Y_TITLE_X,
            (layout.plot_y_start() + layout.plot_y_end()) / 2.0,
            TITLE_SIZE,
            ink.title,
            TextAnchor::Middle,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ChartLayout;
    use crate::scale::Scale;

    #[test]
    fn linear_scale_ticks_stay_within_range_and_are_labelled() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce ticks");
        // All tick positions should be within the range.
        for tick in &ticks {
            assert!(
                tick.position >= 40.0 - 0.1,
                "tick at {:.1} below range start",
                tick.position
            );
            assert!(
                tick.position <= 600.0 + 0.1,
                "tick at {:.1} above range end",
                tick.position
            );
        }
        // Labels should be numeric strings.
        for tick in &ticks {
            assert!(!tick.label.is_empty(), "tick should have a label");
        }
    }

    #[test]
    fn band_scale_yields_one_ordered_tick_per_category() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            range_start: 40.0,
            range_end: 600.0,
            padding: 0.1,
        };
        let ticks = compute_ticks(&scale, 3);
        assert_eq!(ticks.len(), 3, "should produce one tick per category");
        assert_eq!(ticks[0].label, "a");
        assert_eq!(ticks[1].label, "b");
        assert_eq!(ticks[2].label, "c");
        // Positions should be in order.
        assert!(ticks[0].position < ticks[1].position);
        assert!(ticks[1].position < ticks[2].position);
    }

    #[test]
    fn time_scale_ticks_stay_within_range() {
        let scale = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce time ticks");
        for tick in &ticks {
            assert!(tick.position >= 40.0 - 0.1);
            assert!(tick.position <= 600.0 + 0.1);
            assert!(
                tick.label.contains('s'),
                "time tick label should contain 's': {}",
                tick.label
            );
        }
    }

    /// True if any glyph run in the scene carries a quarter-turn (±90°) rotation
    /// — a rotation has a ~zero diagonal and ~±1 off-diagonal, whereas
    /// horizontal text (tick labels, x-title, plot title) uses an
    /// identity/translate run transform ([1,0,0,1]). Reads the public
    /// `vello_encoding` glyph-run transform matrices, no GPU.
    fn scene_has_quarter_turn(scene: &Scene) -> bool {
        scene.encoding().resources.glyph_runs.iter().any(|r| {
            let m = r.transform.matrix;
            m[0].abs() < 1e-3 && m[3].abs() < 1e-3 && m[1].abs() > 0.5 && m[2].abs() > 0.5
        })
    }

    #[test]
    fn render_y_axis_rotates_its_title_but_x_does_not() {
        // Pinned AT THE RENDER SITE: render_y_axis must draw its title
        // rotated (a draw_text_rotated → draw_text refactor would ship a
        // horizontal, tick-colliding y-title otherwise). render_x_axis's title
        // is horizontal; neither axis rotates without a title.
        let layout = ChartLayout::new(400.0, 300.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut y_titled = Scene::new();
        render_y_axis(
            &mut y_titled,
            &layout,
            &ticks,
            Some("Travelers"),
            ChartInk::LIGHT,
        );
        assert!(
            scene_has_quarter_turn(&y_titled),
            "render_y_axis must rotate its title (bottom-to-top)"
        );

        let mut y_plain = Scene::new();
        render_y_axis(&mut y_plain, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            !scene_has_quarter_turn(&y_plain),
            "no rotation without a y-title"
        );

        let mut x_titled = Scene::new();
        render_x_axis(
            &mut x_titled,
            &layout,
            &ticks,
            Some("weight"),
            ChartInk::LIGHT,
        );
        assert!(
            !scene_has_quarter_turn(&x_titled),
            "the x-axis title is horizontal, not rotated"
        );
    }

    #[test]
    fn axis_titles_render_and_clear_tick_labels() {
        use crate::layout::{Insets, Margins};
        use crate::text::measure_width;

        // Grown margins (left +band for a y-title, bottom +band for an x-title).
        let margins = Margins {
            left: 60.0,
            right: 20.0,
            bottom: 50.0,
            top: 20.0,
        };
        let layout = ChartLayout::with_margins_and_insets(400.0, 300.0, margins, Insets::default());
        let ticks = vec![
            Tick {
                value: 10.0,
                label: "10".into(),
                position: layout.plot_y_end(),
            },
            Tick {
                value: 20.0,
                label: "20".into(),
                position: layout.plot_y_start(),
            },
        ];

        // Drawing WITH a title adds ink over drawing without.
        let mut with_t = Scene::new();
        render_x_axis(
            &mut with_t,
            &layout,
            &ticks,
            Some("Arrival Delay"),
            ChartInk::LIGHT,
        );
        let mut no_t = Scene::new();
        render_x_axis(&mut no_t, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            with_t.encoding().draw_tags.len() > no_t.encoding().draw_tags.len(),
            "an x-axis title adds ink"
        );

        // x-title top edge sits BELOW the x tick-label baseline (no overlap).
        let tick_label_baseline = layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE);
        assert!(
            x_title_baseline(&layout) - f64::from(TITLE_SIZE) > tick_label_baseline,
            "x-title top edge is below the tick labels"
        );

        // y-title right edge sits LEFT of the widest y tick label's left edge.
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        let label_left = layout.plot_x_start() - TICK_LENGTH - 3.0 - widest;
        assert!(
            Y_TITLE_X + f64::from(TITLE_SIZE) < label_left,
            "y-title right edge {} must be left of the widest y label at {label_left}",
            Y_TITLE_X + f64::from(TITLE_SIZE),
        );

        let mut yt = Scene::new();
        render_y_axis(&mut yt, &layout, &ticks, Some("Travelers"), ChartInk::LIGHT);
        let mut yn = Scene::new();
        render_y_axis(&mut yn, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            yt.encoding().draw_tags.len() > yn.encoding().draw_tags.len(),
            "a y-axis title adds ink"
        );
    }

    #[test]
    fn render_x_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_x_start(),
            range_end: layout.plot_x_end(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "x-axis should produce scene content"
        );
    }

    #[test]
    fn render_y_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_y_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "y-axis should produce scene content"
        );
    }

    #[test]
    fn nice_step_produces_human_readable_intervals() {
        // 0-100 with ~5 ticks should give step=20
        let step = nice_step(100.0, 5);
        assert!(
            (step - 20.0).abs() < f64::EPSILON,
            "expected step 20, got {step}"
        );

        // 0-1000 with ~5 ticks should give step=200
        let step = nice_step(1000.0, 5);
        assert!(
            (step - 200.0).abs() < f64::EPSILON,
            "expected step 200, got {step}"
        );
    }

    // -----------------------------------------------------------------
    // Thin before you rotate — a time axis at a dashboard tile's width
    // -----------------------------------------------------------------

    /// The six real dates `crates/brightfield-shell/tests/data/dashboard_baseline.csv`'s
    /// `day` column carries, in file order. Restated rather than read off the
    /// CSV, because this crate carries no CSV reader and no dependency on
    /// `brightfield-shell`; the six strings are what ties the test below to
    /// that fixture rather than to an invented one.
    const FIXTURE_DAYS: &[&str] = &[
        "2026-01-05",
        "2026-01-06",
        "2026-01-07",
        "2026-01-08",
        "2026-01-09",
        "2026-01-10",
    ];

    /// A [`Scale::Band`] over [`FIXTURE_DAYS`], ranged across `layout`'s own
    /// (inset-adjusted) x range — the same range [`crate::scale::infer_scales_in`]
    /// would resolve a `day` column's scale against.
    fn fixture_day_scale(layout: &ChartLayout) -> Scale {
        let (range_start, range_end) = layout.x_range();
        Scale::Band {
            categories: FIXTURE_DAYS.iter().map(|s| (*s).to_string()).collect(),
            range_start,
            range_end,
            padding: 0.1,
        }
    }

    /// Matches each horizontal glyph run in `scene` back to whichever `ticks`
    /// entry its draw position ([`TextAnchor::Middle`], `render_x_axis`'s own
    /// anchor) is nearest, then asserts no two runs' `[x, x + width]`
    /// intervals intersect — the width read with [`measure_width`], the same
    /// shaping `render_x_axis` measured it with, rather than estimated from
    /// the run's raw glyph count. A rotated run (its transform carries a
    /// quarter turn) is skipped: this checks the horizontal branch.
    fn assert_no_tick_label_overlap(scene: &Scene, ticks: &[Tick], size: f32) {
        let candidates: Vec<(f64, &str)> = ticks
            .iter()
            .map(|t| {
                (
                    t.position - measure_width(&t.label, size) / 2.0,
                    t.label.as_str(),
                )
            })
            .collect();
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for run in &scene.encoding().resources.glyph_runs {
            let m = run.transform.matrix;
            let rotated = m[0].abs() < 1e-3 && m[3].abs() < 1e-3;
            if rotated {
                continue;
            }
            let x0 = f64::from(run.transform.translation[0]);
            let (_, label) = candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            spans.push((x0, x0 + measure_width(label, size)));
        }
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "two drawn tick labels overlap: {pair:?} (all spans: {spans:?})"
            );
        }
    }

    /// The counts_over_time tile's real dates stay legible at a narrow tile
    /// (240 points) and a wide one (720), so the thinning rule is a rule
    /// rather than a constant tuned to one width. At both, `thinned_x_ticks`
    /// finds a stride that clears, so the labels stay horizontal — see
    /// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels` for a
    /// width where it cannot.
    #[test]
    fn thinning_keeps_labels_from_touching_at_various_widths() {
        for width in [240.0_f64, 720.0_f64] {
            let layout = ChartLayout::new(width, 300.0);
            let scale = fixture_day_scale(&layout);
            let ticks = compute_ticks(&scale, 5);
            assert_eq!(
                ticks.len(),
                FIXTURE_DAYS.len(),
                "fixture check: one tick per date"
            );

            let mut scene = Scene::new();
            render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);
            assert!(
                !scene_has_quarter_turn(&scene),
                "at {width} points wide thinning should have kept the labels \
                 horizontal rather than rotating them"
            );
            assert_no_tick_label_overlap(&scene, &ticks, LABEL_SIZE);
        }
    }

    /// The rotation fallback, isolated — the same six real dates crowded past
    /// what dropping labels can fix, at a width picked to force it (132
    /// points: even the two end dates' labels do not clear each other there).
    /// `render_x_axis` should fall back to `draw_text_rotated` and draw a
    /// label for each tick rather than a thinned subset, and the rotated
    /// labels themselves should not collide either — 132 was chosen so the
    /// band between ticks still clears one rotated label's own width even
    /// though it cannot clear the unrotated text.
    #[test]
    fn rotation_is_the_fallback_when_thinning_cannot_clear_the_labels() {
        let layout = ChartLayout::new(132.0, 300.0);
        let scale = fixture_day_scale(&layout);
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        assert!(
            scene_has_quarter_turn(&scene),
            "thinning cannot clear real dates at 132 points wide, so the axis \
             should have rotated its labels instead of drawing them horizontal"
        );
        let glyph_runs = scene.encoding().resources.glyph_runs.len();
        assert_eq!(
            glyph_runs,
            ticks.len(),
            "a rotated axis draws a label for each tick ({} ticks) rather than \
             the thinned subset a horizontal axis would ({glyph_runs} runs drawn)",
            ticks.len(),
        );

        // The rotated labels themselves must not collide: consecutive runs'
        // pivots (`TextAnchor::End`, so translation is each label's END, the
        // point nearest its tick) sit at least `LABEL_SIZE` apart along the x
        // axis, which is a rotated label's own footprint.
        let mut xs: Vec<f64> = scene
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .map(|r| f64::from(r.transform.translation[0]))
            .collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.windows(2) {
            assert!(
                pair[1] - pair[0] >= f64::from(LABEL_SIZE) - 0.01,
                "two rotated tick labels sit closer than one label's own \
                 width apart: {pair:?}"
            );
        }
    }
}
