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

/// The vertical room, in pixels, a rotated label's run has below the tick line
/// before it reaches whichever floor this axis actually reserves: the x-title
/// text's own top edge when `titled`, the tile's bottom edge otherwise.
///
/// Read off `layout` rather than assumed, because the floor is not a fixed
/// distance from the tile's bottom: `x_title_baseline` sits a FIXED offset
/// below `plot_y_end()` regardless of how large the margin is, so a titled
/// axis's room does not grow with the margin the way an untitled one's does —
/// see `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
/// (this module), which measures that floor rather than restating it.
fn rotated_label_room(layout: &ChartLayout, titled: bool) -> f64 {
    let near_end = layout.plot_y_end() + TICK_LENGTH + ROTATED_LABEL_GAP;
    let floor = if titled {
        x_title_baseline(layout) - f64::from(TITLE_SIZE) - TITLE_GAP
    } else {
        layout.height
    };
    (floor - near_end).max(0.0)
}

/// The horizontal centre a `width`-wide label drawn with `TextAnchor::Middle`
/// at `position` is nudged to so both its drawn edges stay inside the tile's
/// own `[0, tile_width]` span — [`ChartLayout::width`], the tile's full
/// extent, not the inset-adjusted x-range a mark's own scale places its ticks
/// in. A label draws in the tile's margin band below the plot area, so the
/// plot's inner range is the wrong bound to clamp against: a label near the
/// plot's own edge can already read as inside THAT narrower bound while its
/// glyphs still run past the tile the frame actually clips to — a real date
/// sliced at the tile's right edge at a real window width is what this
/// closes.
///
/// `None` when `width` on its own exceeds `tile_width`: no centre keeps both
/// edges inside a span narrower than the label itself, so the caller drops
/// the label rather than draw one that overflows regardless of where it is
/// placed — a single date wider than the whole tile, at the narrowest
/// widths the live layout still resolves a column tile to, is reachable and
/// is exactly this case. A dropped tick still draws its tick MARK
/// (`render_x_axis` draws marks independently of labels); only the text is
/// withheld.
///
/// A rotated label's footprint is [`LABEL_SIZE`] wide regardless of its text
/// length (its own glyph-height run turns crosswise on rotation), so
/// `render_x_axis`'s rotated branch calls this with that constant rather than
/// with a measured text width — same function, a different `width`.
fn contained_centre(position: f64, width: f64, tile_width: f64) -> Option<f64> {
    if width > tile_width {
        return None;
    }
    let half = width / 2.0;
    Some(position.clamp(half, tile_width - half))
}

/// Whether the labels in `ticks` — each centred (`TextAnchor::Middle`) at its
/// own tick's `position` at `size`, nudged by [`contained_centre`] to
/// `tile_width` the way `render_x_axis` actually draws them — clear
/// [`LABEL_CLEARANCE`] of their drawn neighbour's. A label that
/// [`contained_centre`] drops draws no pixel, so it clears trivially — it
/// cannot collide with a neighbour it shares no pixel with.
///
/// Sorted by drawn span start rather than read pairwise off `ticks`' own
/// order: the clamp can pull an end label in far enough that its nearest
/// drawn neighbour is no longer the tick next to it in `ticks`.
fn labels_clear_horizontally(ticks: &[&Tick], size: f32, tile_width: f64) -> bool {
    let mut spans: Vec<(f64, f64)> = ticks
        .iter()
        .filter_map(|t| {
            let width = measure_width(&t.label, size);
            let centre = contained_centre(t.position, width, tile_width)?;
            Some((centre - width / 2.0, centre + width / 2.0))
        })
        .collect();
    if spans.len() < 2 {
        return true;
    }
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    spans
        .windows(2)
        .all(|pair| pair[1].0 - pair[0].1 >= LABEL_CLEARANCE)
}

/// The widest evenly-strided subset of `ticks` whose labels clear each other
/// horizontally ([`labels_clear_horizontally`], itself bounded to
/// `tile_width`) at `size` — Observable Plot's own answer to a crowded axis:
/// try drawing each label, then try dropping alternating ones, then try a
/// wider stride still, and so on until a stride clears or the search has
/// narrowed to the two end ticks. `render_x_axis` rotates the full set
/// instead of drawing this candidate when even that pair collides AND there
/// is room to rotate into; it degrades past this candidate instead when there
/// is not — see [`rotated_label_room`].
fn thinned_x_ticks(ticks: &[Tick], size: f32, tile_width: f64) -> Vec<&Tick> {
    if ticks.len() < 2 {
        return ticks.iter().collect();
    }
    for stride in 1..ticks.len() {
        let subset: Vec<&Tick> = ticks.iter().step_by(stride).collect();
        if labels_clear_horizontally(&subset, size, tile_width) {
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
/// draws thinned first (`thinned_x_ticks`, private to this module); when even
/// the sparsest horizontal set still collides, it rotates a quarter turn
/// ([`draw_text_rotated`]) IF `rotated_label_room` (private to this module)
/// says the run fits below the tick line, and degrades past the two end
/// labels to a single one
/// otherwise — a rotated run that does not fit reads as clipped digits under
/// the title rather than as an axis, which is worse than one label. This
/// module's tests pin these branches:
/// `thinning_keeps_labels_from_touching_at_various_widths`,
/// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`
/// and
/// `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`.
///
/// `contained_centre` (private to this module) additionally nudges a label
/// whose own drawn footprint would run past the tile's `[0, layout.width]`
/// span back inside it, or drops it when even nudging cannot fit it — applied
/// to the thinned candidate, the rotated band and the degraded single label
/// alike, so a branch above cannot hand back a rect the tile does not hold. A
/// real composed tile's own width is exercised one crate up, in the
/// brightfield-shell crate's dashboard_baseline.rs test suite, since this
/// crate carries no dependency on that composition;
/// `label_clearance_rejects_a_gap_narrower_than_the_minimum` (this module)
/// pins the neighbour-clearance floor the same nudge must not erase.
pub fn render_x_axis(
    scene: &mut Scene,
    layout: &ChartLayout,
    ticks: &[Tick],
    title: Option<&str>,
    ink: ChartInk,
) {
    let y = layout.plot_y_end();
    let tile_width = layout.width;
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
    let thinned = thinned_x_ticks(ticks, LABEL_SIZE, tile_width);
    if labels_clear_horizontally(&thinned, LABEL_SIZE, tile_width) {
        for tick in thinned {
            let width = measure_width(&tick.label, LABEL_SIZE);
            let Some(centre) = contained_centre(tick.position, width, tile_width) else {
                // Wider than the tile on its own: no centre keeps both
                // edges inside it, so this label is dropped rather than
                // drawn overflowing. The tick mark above already drew.
                continue;
            };
            draw_text(
                scene,
                &tick.label,
                centre,
                y + TICK_LENGTH + f64::from(LABEL_SIZE),
                LABEL_SIZE,
                ink.label,
                TextAnchor::Middle,
            );
        }
    } else {
        // Even the two end labels collide horizontally. Rotating helps when
        // the widest label's run actually fits the room below the tick line
        // — `rotated_label_room` reads that off `layout`, and a titled
        // axis's room is a small, FIXED distance (the title sits a constant
        // offset below the tick line no matter how large the margin is), so
        // this is not a check that more margin alone can satisfy.
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        if widest <= rotated_label_room(layout, title.is_some()) {
            // Rotate the full set a quarter turn so each label's OWN
            // footprint is its font size rather than its text width,
            // anchored (`TextAnchor::End`) so the label's last character
            // sits nearest the tick and the rest reaches down into the
            // margin instead of up into the plot. The rotated footprint is
            // `LABEL_SIZE` wide regardless of the text it carries, so it is
            // the pivot itself — not a measured text width — that
            // `contained_centre` nudges here; `LABEL_SIZE` fits comfortably
            // inside any tile this axis draws into in practice, so the drop
            // branch is defensive rather than reachable today.
            for tick in ticks {
                let Some(pivot) =
                    contained_centre(tick.position, f64::from(LABEL_SIZE), tile_width)
                else {
                    continue;
                };
                draw_text_rotated(
                    scene,
                    &tick.label,
                    pivot,
                    y + TICK_LENGTH + ROTATED_LABEL_GAP,
                    LABEL_SIZE,
                    ink.label,
                    TextAnchor::End,
                );
            }
        } else {
            // No room to rotate into without running past the tile's own
            // bottom edge or under the title: degrade past the two end
            // labels `thinned_x_ticks` stopped at to the single label
            // nearest the domain's start, on the SAME horizontal baseline
            // the thinned case draws at. One label cannot collide with
            // itself, and that baseline is already proven clear of a title —
            // `axis_titles_render_and_clear_tick_labels`, this module.
            let solo = &ticks[0];
            let width = measure_width(&solo.label, LABEL_SIZE);
            if let Some(centre) = contained_centre(solo.position, width, tile_width) {
                draw_text(
                    scene,
                    &solo.label,
                    centre,
                    y + TICK_LENGTH + f64::from(LABEL_SIZE),
                    LABEL_SIZE,
                    ink.label,
                    TextAnchor::Middle,
                );
            }
            // Wider than the tile on its own: dropped, same as the thinned
            // branch above — the tick marks and (when present) the title
            // still draw.
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
    use crate::layout::{ChartLayout, Insets, Margins};
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
    /// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`
    /// and `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
    /// for widths where it cannot.
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

    /// The rotation fallback, isolated, over a margin with room to rotate
    /// into — the same six real dates crowded past what dropping labels can
    /// fix, at a width picked to force it (132 points: even the two end
    /// dates' labels do not clear each other there), with a bottom margin
    /// grown past [`rotated_label_room`]'s floor for one date's own width.
    /// `render_x_axis` should still fall back to `draw_text_rotated` and draw
    /// a label for each tick rather than a thinned subset, and the rotated
    /// labels themselves should not collide either — 132 was chosen so the
    /// band between ticks still clears one rotated label's own width even
    /// though it cannot clear the unrotated text. See
    /// `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
    /// for the same crowding with no such room.
    #[test]
    fn rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate()
    {
        let margins = Margins {
            bottom: 100.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins_and_insets(132.0, 300.0, margins, Insets::default());
        let scale = fixture_day_scale(&layout);
        let ticks = compute_ticks(&scale, 5);
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            widest <= rotated_label_room(&layout, false),
            "fixture check: the margin this test grew ({margins:?}) is meant \
             to leave room to rotate a real date ({widest} points wide) into"
        );

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        assert!(
            scene_has_quarter_turn(&scene),
            "thinning cannot clear real dates at 132 points wide and there is \
             room to rotate into, so the axis should have rotated its labels \
             instead of drawing them horizontal"
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

    /// **When there is no room to rotate a real date past a title, the axis
    /// degrades to a single label rather than clip one under it.**
    /// [`FIXTURE_DAYS`] crowded at 130 points — narrow enough that
    /// `thinned_x_ticks` cannot clear even its two end ticks — with an
    /// x title present. [`rotated_label_room`] measures the floor a title
    /// leaves as a small, FIXED distance below the tick line regardless of
    /// how large the margin is (`x_title_baseline` is a constant offset from
    /// `ChartLayout::plot_y_end`, not from the margin's own size), so growing
    /// the margin further would not have bought a 65-point date any more
    /// room here — unlike
    /// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`,
    /// which has no title and grows past its floor instead.
    #[test]
    fn the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title() {
        let unfitted = ChartLayout::new(130.0, 300.0);
        let scale = fixture_day_scale(&unfitted);
        let ticks = compute_ticks(&scale, 5);
        let thinned = thinned_x_ticks(&ticks, LABEL_SIZE, unfitted.width);
        assert!(
            !labels_clear_horizontally(&thinned, LABEL_SIZE, unfitted.width),
            "fixture check: 130 points is meant to be a width where even the \
             two end dates collide, which is what forces the choice this \
             test is about"
        );

        // A bottom margin generous enough that an UNTITLED axis at this same
        // width would have room to rotate a real date into — grown by hand
        // rather than through `grow_margins`, because the point of what
        // follows is to hold the width and the margin FIXED and change just
        // whether a title is present. Finding: a ten-character date used to
        // overrun BOTH floors at once at the margin `grow_margins` actually
        // produces for a titled axis here (bottom 50), which left the titled
        // and untitled floors indistinguishable — this margin (bottom 100)
        // is chosen so the fixture check below can tell them apart.
        let margins = Margins {
            bottom: 100.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins(130.0, 300.0, margins);
        let scale = fixture_day_scale(&layout);
        let ticks = compute_ticks(&scale, 5);
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            widest <= rotated_label_room(&layout, false),
            "fixture check: at this margin an UNTITLED axis is meant to have \
             room to rotate a real date ({widest} points wide) into — the \
             control this test needs so the degrade below reads as the \
             title's doing rather than the layout being too narrow outright"
        );
        assert!(
            widest > rotated_label_room(&layout, true),
            "fixture check: a real date ({widest} points wide) is meant to \
             overrun the room a title leaves, at the SAME margin the line \
             above just proved has room without one"
        );

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, Some("day"), ChartInk::LIGHT);

        assert!(
            !scene_has_quarter_turn(&scene),
            "there is no room below the title to rotate a real date into, so \
             the axis should have degraded to a single label rather than \
             clip a rotated one past the title"
        );
        let glyph_runs = &scene.encoding().resources.glyph_runs;
        assert_eq!(
            glyph_runs.len(),
            2,
            "a degraded titled axis draws one tick label plus the title, not \
             {} runs",
            glyph_runs.len(),
        );

        // The one label drawn sits at the ordinary horizontal tick-label
        // baseline (`axis_titles_render_and_clear_tick_labels` pins that row
        // clear of the title already) and the other run is the title itself
        // — nothing draws anywhere else, in particular not past either.
        let label_y = layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE);
        let title_y = x_title_baseline(&layout);
        let mut saw_label_row = false;
        for run in glyph_runs {
            let y = f64::from(run.transform.translation[1]);
            assert!(
                (y - label_y).abs() < 0.5 || (y - title_y).abs() < 0.5,
                "a glyph run drew at y={y}, neither the tick-label baseline \
                 {label_y} nor the title baseline {title_y} — it drew \
                 somewhere a rotated run would have, clipped or not"
            );
            if (y - label_y).abs() < 0.5 {
                saw_label_row = true;
            }
        }
        assert!(
            saw_label_row,
            "no run drew at the ordinary tick-label baseline {label_y}"
        );

        // The control the fixture checks above set up: the SAME width and
        // margin, with no title, rotates. If this degraded too, the title
        // would not be what forced the degrade above, and the two fixture
        // checks at the top of this test would not actually have
        // distinguished a titled floor from an untitled one.
        let mut untitled_scene = Scene::new();
        render_x_axis(&mut untitled_scene, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            scene_has_quarter_turn(&untitled_scene),
            "the same width and margin, with no title, should have rotated \
             — a degrade here would mean the title above was not what forced \
             the degrade, and this test would be pinning sheer narrowness \
             rather than the claim its name makes"
        );
    }

    /// **A gap inside [`LABEL_CLEARANCE`] reads as NOT clear, even though the
    /// two labels do not yet overlap.** Containment alone cannot pin this —
    /// a tile wide enough leaves both labels contained whatever the gap
    /// between them — so this reads [`labels_clear_horizontally`] directly,
    /// on two synthetic ticks placed so their reach (half of each label's
    /// width) leaves exactly half of [`LABEL_CLEARANCE`] between them:
    /// comfortably short of overlapping, and just as comfortably short of
    /// the minimum. A mutation that zeroed `LABEL_CLEARANCE` would read this
    /// pair as clear, which is the gap this test closes.
    #[test]
    fn label_clearance_rejects_a_gap_narrower_than_the_minimum() {
        let label = "22";
        let width = measure_width(label, LABEL_SIZE);
        let reach = width; // two equal-width labels: half + half = the full width
        let tile_width = 1000.0; // wide enough that containment moves neither label

        let inside_minimum = LABEL_CLEARANCE / 2.0;
        let too_close = [
            Tick {
                value: 0.0,
                label: label.to_string(),
                position: 100.0,
            },
            Tick {
                value: 1.0,
                label: label.to_string(),
                position: 100.0 + reach + inside_minimum,
            },
        ];
        let refs: Vec<&Tick> = too_close.iter().collect();
        assert!(
            !labels_clear_horizontally(&refs, LABEL_SIZE, tile_width),
            "a {inside_minimum}px gap is half of LABEL_CLEARANCE and should \
             not read as clear — a mutation that zeroed LABEL_CLEARANCE \
             would accept this pair, which is what this test pins"
        );

        let at_minimum = [
            Tick {
                value: 0.0,
                label: label.to_string(),
                position: 100.0,
            },
            Tick {
                value: 1.0,
                label: label.to_string(),
                position: 100.0 + reach + LABEL_CLEARANCE,
            },
        ];
        let refs_ok: Vec<&Tick> = at_minimum.iter().collect();
        assert!(
            labels_clear_horizontally(&refs_ok, LABEL_SIZE, tile_width),
            "a gap exactly at LABEL_CLEARANCE should read as clear"
        );
    }
}
