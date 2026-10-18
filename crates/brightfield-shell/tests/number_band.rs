//! **A bar or a cell over a number column draws bands, and a mark that draws
//! nothing says so.**
//!
//! Observable Plot puts a bar's band axis and both of a cell's axes on a band
//! scale, so a number column there (a month, an hour of the day) reads as
//! categories in ascending order. Brightfield inferred a linear scale for it,
//! and the bar and cell renderers, which size each shape to a band, drew
//! nothing: a valid picture with axes and no marks, and no word about why.
//!
//! These run the whole composition over inline data, rasterise it through the
//! Vello renderer the window and the exports share, and read the diagnostics
//! the page is handed.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_render::VelloRenderer;
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use image::RgbaImage;

/// Twelve months, written out of order, each with a value of its own.
fn months() -> String {
    [7, 1, 12, 3, 10, 5, 2, 11, 4, 9, 6, 8]
        .iter()
        .map(|m| format!("    - {{ month: {m}, v: {} }}\n", 10 + m))
        .collect()
}

fn bar_spec(attrs: &str) -> String {
    format!(
        "data:\n  t:\n{}plot:\n  - mark: barY\n    data: {{ from: t }}\n    x: month\n    y: v\n\
         width: 640\nheight: 400\n{attrs}",
        months()
    )
}

/// Hours 0 to 3 on x, two places on y, one cell for each pair, in a colour
/// of their own so the cells can be counted in the picture.
const CELL_SPEC: &str = "data:
  t:
    - { hour: 3, place: north, v: 1 }
    - { hour: 0, place: north, v: 2 }
    - { hour: 2, place: north, v: 3 }
    - { hour: 1, place: north, v: 4 }
    - { hour: 3, place: south, v: 5 }
    - { hour: 0, place: south, v: 6 }
    - { hour: 2, place: south, v: 7 }
    - { hour: 1, place: south, v: 8 }
plot:
  - mark: cell
    data: { from: t }
    x: hour
    y: place
    fill: \"#d62728\"
width: 640
height: 400
";

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

fn raster(composed: &Composed) -> RgbaImage {
    let (w, h) = (composed.width, composed.height);
    let pixels = VelloRenderer::new()
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&composed.scene, w, h);
    RgbaImage::from_raw(w, h, pixels).expect("vello pixel buffer size mismatch")
}

fn ink_of(colour: [u8; 3]) -> impl Fn(&image::Rgba<u8>) -> bool {
    move |p| (0..3).all(|c| (i32::from(p.0[c]) - i32::from(colour[c])).abs() <= 20)
}

/// The default mark ink, read from the token layer.
fn mark_ink() -> [u8; 3] {
    let c = meridian_design::viz::MARK_DEFAULT_LIGHT;
    [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    ]
}

/// How many inked pixels each image column holds, left to right.
fn column_heights(img: &RgbaImage, inked: &impl Fn(&image::Rgba<u8>) -> bool) -> Vec<u32> {
    let (w, h) = img.dimensions();
    (0..w)
        .map(|x| (0..h).filter(|&y| inked(img.get_pixel(x, y))).count() as u32)
        .collect()
}

/// Maximal runs of inked columns at least three pixels wide: the bars, left to
/// right, each as the tallest column in it.
fn bars(img: &RgbaImage, inked: &impl Fn(&image::Rgba<u8>) -> bool) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut run, mut tallest) = (0usize, 0u32);
    for height in column_heights(img, inked).into_iter().chain([0]) {
        if height > 0 {
            run += 1;
            tallest = tallest.max(height);
        } else {
            if run >= 3 {
                out.push(tallest);
            }
            run = 0;
            tallest = 0;
        }
    }
    out
}

fn categories(composed: &Composed, channel: Channel) -> Vec<String> {
    match composed.plots[0].scales.get(channel) {
        Some(Scale::Band { categories, .. }) => categories.clone(),
        other => panic!("{channel:?} is not a band: {other:?}"),
    }
}

fn drew_nothing(composed: &Composed) -> Vec<String> {
    composed
        .diagnostics
        .notes()
        .into_iter()
        .filter(|n| n.contains("drew nothing"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_bar_over_a_whole_number_column_draws_one_band_per_value_in_order() {
    let composed = compose(&bar_spec(""));
    let wanted: Vec<String> = (1..=12).map(|m| m.to_string()).collect();
    assert_eq!(
        categories(&composed, Channel::X),
        wanted,
        "the bands are the months in ascending order, each written as the number reads"
    );

    let heights = bars(&raster(&composed), &ink_of(mark_ink()));
    assert_eq!(heights.len(), 12, "one bar per month: {heights:?}");
    assert!(
        heights.windows(2).all(|w| w[0] < w[1]),
        "each month's value is larger than the last, so each bar is taller: {heights:?}"
    );
    assert!(
        drew_nothing(&composed).is_empty(),
        "{:?}",
        composed.diagnostics.notes()
    );
}

#[test]
fn a_cell_over_a_number_column_and_a_string_column_draws_every_cell() {
    let composed = compose(CELL_SPEC);
    assert_eq!(categories(&composed, Channel::X), ["0", "1", "2", "3"]);
    assert_eq!(categories(&composed, Channel::Y), ["north", "south"]);

    let img = raster(&composed);
    let red = ink_of([0xd6, 0x27, 0x28]);
    assert_eq!(
        bars(&img, &red).len(),
        4,
        "four columns of cells, one per hour"
    );
    let rows = {
        let (w, h) = img.dimensions();
        let heights: Vec<u32> = (0..h)
            .map(|y| (0..w).filter(|&x| red(img.get_pixel(x, y))).count() as u32)
            .collect();
        let mut runs = 0;
        let mut run = 0;
        for n in heights.into_iter().chain([0]) {
            if n > 0 {
                run += 1;
            } else {
                if run >= 3 {
                    runs += 1;
                }
                run = 0;
            }
        }
        runs
    };
    assert_eq!(rows, 2, "two rows of cells, one per place");
    assert!(
        drew_nothing(&composed).is_empty(),
        "{:?}",
        composed.diagnostics.notes()
    );
}

#[test]
fn a_line_beside_the_bars_shares_their_band_and_is_drawn_through_its_centres() {
    let source = format!(
        "{}  - mark: lineY\n    data: {{ from: t }}\n    x: month\n    y: v\n",
        bar_spec("").replace("width: 640\nheight: 400\n", "")
    ) + "width: 640\nheight: 400\n";
    let composed = compose(&source);
    let wanted: Vec<String> = (1..=12).map(|m| m.to_string()).collect();
    assert_eq!(categories(&composed, Channel::X), wanted);
    assert!(
        drew_nothing(&composed).is_empty(),
        "{:?}",
        composed.diagnostics.notes()
    );
}

#[test]
fn a_written_linear_scale_is_kept_and_the_bars_that_cannot_draw_on_it_say_so() {
    let composed = compose(&bar_spec("xScale: linear\n"));
    assert!(
        matches!(
            composed.plots[0].scales.get(Channel::X),
            Some(Scale::Linear { .. })
        ),
        "the file wrote a linear x, and it is kept: {:?}",
        composed.plots[0].scales.get(Channel::X)
    );
    assert_eq!(
        bars(&raster(&composed), &ink_of(mark_ink())).len(),
        0,
        "a bar has no band to stand in on a linear x"
    );
    assert_eq!(
        drew_nothing(&composed),
        [
            "`barY` drew nothing: x reads a number column on a linear scale, and a barY \
             draws x as bands"
        ],
    );
}

#[test]
fn a_written_band_scale_on_a_bar_over_numbers_is_the_band_it_draws() {
    let composed = compose(&bar_spec("xScale: band\n"));
    let wanted: Vec<String> = (1..=12).map(|m| m.to_string()).collect();
    assert_eq!(categories(&composed, Channel::X), wanted);
    assert_eq!(bars(&raster(&composed), &ink_of(mark_ink())).len(), 12);
    assert!(
        composed.diagnostics.notes().is_empty(),
        "a band is a scale this build draws on a bar's band axis: {:?}",
        composed.diagnostics.notes()
    );
}

#[test]
fn a_layer_the_projection_cannot_draw_is_named_once_and_not_as_drawing_nothing() {
    let composed = compose(
        "data:\n  t:\n    - { lon: 150, lat: -30 }\n    - { lon: 151, lat: -31 }\n\
         plot:\n  - mark: dot\n    data: { from: t }\n    x: lon\n    y: lat\n  \
         - mark: lineY\n    data: { from: t }\n    x: lon\n    y: lat\n\
         projectionType: mercator\nwidth: 400\nheight: 300\n",
    );
    assert!(
        drew_nothing(&composed).is_empty(),
        "{:?}",
        composed.diagnostics.notes()
    );
    assert!(
        composed
            .diagnostics
            .notes()
            .iter()
            .any(|n| n.contains("lineY") && n.contains("project")),
        "the parser's own note names the line: {:?}",
        composed.diagnostics.notes()
    );
}

#[test]
fn a_line_of_one_point_says_a_line_needs_two() {
    let composed = compose(
        "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - mark: lineY\n    data: { from: t }\n    \
         x: x\n    y: y\n",
    );
    assert_eq!(
        drew_nothing(&composed),
        [
            "`lineY` drew nothing: a lineY needs at least two points in a series, and no series \
             had two it could place"
        ]
    );
}
