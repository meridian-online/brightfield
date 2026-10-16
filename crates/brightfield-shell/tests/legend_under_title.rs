//! **A legend at a plot's right starts under the plot's title, so a long title
//! is not drawn over.** A plot's title runs from the frame's left edge with no
//! width limit, and the band a legend at the plot's right stands in comes out of
//! the plot's own box. A band that began at the box's top shared the title's row,
//! and the legend's name and ramp covered the end of a title long enough to reach
//! it.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_render::text::{measure_width, TITLE_SIZE};
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// A title long enough to run past the narrowed plot and across the band.
const LONG_TITLE: &str =
    "Readings from every station along the coast, coloured by how high each one read";

/// A 420-point-wide dot plot coloured by a number column, with its legend item at
/// its right, and `title` when there is one.
fn compose(title: Option<&str>) -> Composed {
    let title = title.map(|t| format!("title: {t}\n")).unwrap_or_default();
    let source = format!(
        "data:\n  t:\n    - {{ x: 0, y: 0, reading: 1.5 }}\n    - {{ x: 1, y: 3, reading: 9 }}\n    - {{ x: 2, y: 6, reading: 4 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: reading\n  - legend: color\n{title}width: 420\nheight: 300\n"
    );
    compose_spec_str(&source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

#[test]
fn a_right_legend_and_a_long_title_do_not_overlap() {
    let composed = compose(Some(LONG_TITLE));
    let plot = &composed.plots[0];
    let band = plot
        .legend_right
        .expect("the plot gave up a band for its legend");

    // The title's row, on the page plane: from the box's top to the frame's top,
    // and from the frame's left edge for as far as the title's text runs.
    let title_left = plot.rect.x + plot.layout.plot_x_start();
    let title_right = title_left + measure_width(LONG_TITLE, TITLE_SIZE);
    let title_foot = plot.rect.y + plot.layout.plot_y_start();
    assert!(
        title_right > band.x,
        "the title must reach into the band's columns for this test to mean anything: \
         it ends at {title_right}, the band starts at {}",
        band.x
    );

    assert!(
        band.y >= title_foot,
        "the band starts at {} and the title's row runs to {title_foot}: the legend \
         would be drawn over the title",
        band.y
    );
    assert!(
        (band.y + band.height - (plot.rect.y + plot.rect.height)).abs() < 1e-9,
        "the band still ends at the plot's foot"
    );
}

#[test]
fn an_untitled_plot_keeps_its_band_level_with_its_box() {
    let composed = compose(None);
    let plot = &composed.plots[0];
    let band = plot
        .legend_right
        .expect("the plot gave up a band for its legend");
    assert_eq!(
        band.y, plot.rect.y,
        "an untitled plot's band starts at its top"
    );
    assert_eq!(band.height, plot.rect.height, "and is as tall as it");
}
