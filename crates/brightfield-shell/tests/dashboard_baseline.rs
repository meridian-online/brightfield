//! **What the generator chose for a table it had never met** — held twice over
//! one committed file: as the kind each column was given, by name, and as the
//! picture those choices compose into.
//!
//! # Why both, and why the structural half leads
//!
//! The picture is the product: it is what a reader is shown when they hand
//! Brightfield a data file. But an image diff reddens on a font bump and a
//! colour-token change exactly as loudly as on a moved tile choice, so a
//! reviewer holding one red baseline cannot tell which of those happened — and
//! the cheapest response to an image that "just moved" is to re-record it. The
//! choice table below is what makes the other failure legible: it names the
//! column, the kind and what decided the kind, so a moved choice fails with a
//! sentence instead of with pixels.
//!
//! [`assert_choices`] therefore runs **before** `image_snapshot` inside the
//! pixel test as well as in its own. Under `UPDATE_SNAPSHOTS=1` a snapshot call
//! writes whatever it was handed, so a guard sitting behind one would author a
//! baseline of a dashboard whose choices had already moved and complain about
//! it afterwards — the ordering `tests/surfaces.rs` records for its scripted
//! captures, here for the same reason.
//!
//! # How the dashboard reaches a capture: no second path
//!
//! `brightfield-shot` takes `--spec`, and this dashboard arrives by opening a
//! **data file**. The two were already reconciled: `Boot::open_sampled`
//! classifies a path naming a data file first and hands it to
//! [`Boot::data_file`], so `brightfield-shot --spec table.csv --out out.png`
//! renders this same picture through this same code today — the classification
//! step being what `a_path_on_the_command_line_opens_as_the_generated_dashboard`
//! holds, in `tests/scripted_open.rs`. What is photographed
//! here is that boot, run through [`brightfield_shell::capture::capture_png`] —
//! the crate's own headless path, which is what the shot binary runs and what
//! the live window runs — with the resulting image handed to
//! `egui_kittest::image_snapshot`. So the comparison, the `kittest.toml`
//! thresholds and the regeneration workflow are the sheet tier's, and the
//! diff is perceptual rather than byte-exact: a byte-exact dashboard baseline
//! would fail on text antialiasing and be switched off inside a week.
//!
//! Regenerate the baseline with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p
//! brightfield-shell --test dashboard_baseline`.
//!
//! Thresholds come from `kittest.toml` at the workspace root — read the policy
//! comment there before reaching for a per-test override. This baseline was
//! recorded at the repo floor and needs none.
//!
//! # Why the DuckDB type is what decides here
//!
//! `LoadOptions::packaged` looks for a FineType bundle beside the running
//! executable, and a `cargo test` binary has none beside it, so each column of
//! the fixture arrives carrying `SemanticType::NotAsked` and its storage type
//! takes the decision. That is asserted rather than assumed: [`EXPECTED`]
//! carries what decided each tile, so a run in which a bundle *is* present
//! fails naming the label it found instead of quietly photographing a
//! different dashboard.
//!
//! # This tier needs a GPU
//!
//! The capture rasterises through a real wgpu adapter, like `tests/snapshot.rs`
//! and `tests/surfaces.rs`, and there is deliberately no skip switch here
//! either — an env-var opt-out would render "no GPU here" as a passing test.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_shell::capture::{capture_png, capture_vello_only};
use brightfield_shell::dashboard::{self, ChosenBy, Dashboard, Omission};
use brightfield_shell::design::Mode;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_shell::{chart_kinds, data_file, ranked_bars};
use brightfield_workbench::registry::ChartKindId;
use brightfield_workbench::{GridLayout, GridSpot, RunState};

/// Device pixels per logical point for this baseline.
///
/// `tests/surfaces.rs`'s scale, for the reason recorded there: the perceptual
/// gate is a per-pixel delta rather than a per-image one, so a lower raster
/// buys no slack — it just stores less of it.
const SCALE: f32 = 1.0;

/// The committed table the dashboard under test is generated from.
///
/// Four columns picked so the walk has a different answer for each: a `DATE`, a
/// `VARCHAR` of four regions, a `BIGINT` of readings, and a sensor id holding
/// one distinct value. Addressed from the crate root so the test does not
/// depend on the shell's working directory.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/dashboard_baseline.csv")
}

/// **The table the committed picture is of**: nine numeric columns named and
/// ordered as California Housing's are, two of them a coordinate pair.
///
/// A committed **sample**, not the dataset — the real Parquet is 16,640 rows
/// and belongs in `open-analytics` rather than in this repo's test data. What
/// it shares with the real file is everything the picture depends on: the nine
/// columns in file order, a coordinate pair among them so the generator draws
/// a map, and a tile for every one of the nine — the pair's two included, each
/// beside the joint map.
///
/// The choice table above stays on [`fixture`], whose four columns are four
/// different shapes and answer a different question — which kind each *type*
/// earns. This one answers what the first screen looks like, which is a
/// question about a file with a map in it.
fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

/// **[`housing`] again, as a Parquet whose fractional columns are `DECIMAL`**:
/// `median_income` at scale 4, `median_house_value` at scale 3, and the other
/// five at scale 2, each cast from the CSV's own text rather than from the
/// doubles DuckDB parses that text to. `house_age` and `population` stay
/// `BIGINT`.
///
/// Built with DuckDB from the committed CSV:
///
/// ```sql
/// COPY (SELECT CAST(median_income AS DECIMAL(9,4)) AS median_income, …
///       FROM read_csv('california_housing_sample.csv', all_varchar = true))
/// TO 'california_housing_decimal.parquet' (FORMAT parquet, COMPRESSION zstd);
/// ```
fn housing_decimal() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_decimal.parquet")
}

/// The columns of [`housing_decimal`] DuckDB types `DECIMAL`, sorted by name.
const HOUSING_DECIMAL_COLUMNS: &[&str] = &[
    "avg_bedrooms",
    "avg_occupancy",
    "avg_rooms",
    "latitude",
    "longitude",
    "median_house_value",
    "median_income",
];

/// The tiles [`housing`] earns, in the order the composition places them: the
/// pair's joint map first, then every one of the file's nine columns in the
/// file's own order — the two coordinates among them, each with the histogram
/// every other column gets, which
/// [`the_generated_dashboard_light_baseline`] reads back through
/// [`assert_housing`] before it photographs anything.
///
/// `longitude` appears twice on purpose. The first is the map, which the
/// generator names for the pair's longitude column; the second is that
/// column's own histogram, standing where the file declares it.
const HOUSING_PLOTS: &[&str] = &[
    "longitude",
    "median_income",
    "house_age",
    "avg_rooms",
    "avg_bedrooms",
    "population",
    "avg_occupancy",
    "latitude",
    "longitude",
    "median_house_value",
];

/// **The structural half of the picture below**: the map is the hero, its pair
/// is the two coordinate columns, and every column of the file — the two
/// coordinates included — stacks beside it in file order.
///
/// Runs ahead of `image_snapshot` for the reason [`assert_choices`] does, and
/// it is the assertion that makes a red baseline legible: a photograph of a
/// dashboard whose hero had moved and one of a dashboard whose font had moved
/// differ by the same kind of pixel diff.
fn assert_housing(dash: &Dashboard) {
    let drawn: Vec<&str> = dash.plot_order().iter().map(|t| t.column()).collect();
    assert_eq!(
        drawn,
        HOUSING_PLOTS.to_vec(),
        "the tiles this picture is of, or the order the composition places \
         them in, have moved. The first is the hero the map pane holds and \
         the rest are the column beside it."
    );
    let hero = &dash.plot_order()[0];
    assert_eq!(
        hero.kind(),
        chart_kinds::POINT_MAP,
        "the hero is not the point map, so the map pane is holding something \
         else and the picture below is not the first screen"
    );
    assert_eq!(
        hero.paired_column(),
        Some("latitude"),
        "the map's paired column moved"
    );
    assert_eq!(
        dash.column_tiles().len(),
        HOUSING_PLOTS.len() - 1,
        "the column holds {} tiles rather than one per column of the file, \
         the pair's joint map being the hero and standing outside it",
        dash.column_tiles().len()
    );
    assert!(
        dash.omitted().is_empty(),
        "a column was left out of this dashboard: {:?}",
        dash.omitted()
    );
}

/// **A table small like [`housing`], with a coordinate pair AND a date
/// column** — the shape [`fixture`] does not have.
///
/// [`fixture`]'s `day` earns no picture of a real column tile: with no
/// coordinate pair in that file, [`dashboard::Dashboard::hero_index`] falls
/// back to the first tile, so `day` draws in the map pane at the hero's own
/// (much wider) share of the page rather than at a column tile's width. This
/// table gives the coordinate pair its own hero (`longitude`/`latitude`, one
/// of three repeated sites) so `day` stacks beside it as an ordinary column
/// tile instead — which is the width the counts_over_time axis actually
/// draws at whenever a file's map takes the hero's place, the shape the
/// time-axis collision was recorded against. `reading` earns the column's
/// second tile so the width
/// [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`] reads
/// is a real dashboard's, not a single-tile one.
fn site_readings() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/site_readings_sample.csv")
}

/// The tiles [`site_readings`] earns, in the order the composition places
/// them: the map first (named for its longitude column, [`assert_housing`]'s
/// convention), then every one of the file's four columns beside it in file
/// order — `day`, the pair's own two, and `reading`.
const SITE_READINGS_PLOTS: &[&str] = &["longitude", "day", "longitude", "latitude", "reading"];

/// **The structural half of the picture [`the_site_readings_dashboard_light_baseline`]
/// and its dark twin capture**, [`assert_housing`]'s pattern read against
/// [`site_readings`] instead: the map is the hero, the file's four columns
/// stack beside it, and nothing was left out.
fn assert_site_readings(dash: &Dashboard) {
    let drawn: Vec<&str> = dash.plot_order().iter().map(|t| t.column()).collect();
    assert_eq!(
        drawn,
        SITE_READINGS_PLOTS.to_vec(),
        "the tiles this picture is of, or the order the composition places \
         them in, have moved. The first is the hero the map pane holds and \
         the rest are the column beside it."
    );
    let hero = &dash.plot_order()[0];
    assert_eq!(
        hero.kind(),
        chart_kinds::POINT_MAP,
        "the hero is not the point map, so the map pane is holding something \
         else and day no longer draws as a column tile"
    );
    assert_eq!(
        hero.paired_column(),
        Some("latitude"),
        "the map's paired column moved"
    );
    assert_eq!(
        dash.column_tiles().len(),
        SITE_READINGS_PLOTS.len() - 1,
        "the column holds {} tiles rather than one per column of the file \
         (day, longitude, latitude, reading)",
        dash.column_tiles().len()
    );
    assert!(
        dash.omitted().is_empty(),
        "a column was left out of this dashboard: {:?}",
        dash.omitted()
    );
}

/// Where the capture's intermediate PNG goes. Under the target dir, already
/// git-ignored, so a concurrent test cannot race this one on a path.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// **The tile choice, as a table**: which kind each column of the fixture was
/// given, by name, and the DuckDB type that decided it — in the table's own
/// column order, which is the order the dashboard reads in.
///
/// This is the assertion the committed image cannot make. Two kinds can ink
/// similarly at one tile size, and a reader comparing photographs would not
/// see the swap; a reader of a failure naming `region: ranked-category-bars`
/// against `region: binned-histogram` cannot miss it.
const EXPECTED: &[(&str, ChartKindId, &str)] = &[
    ("day", chart_kinds::COUNTS_OVER_TIME, "DATE"),
    ("region", ranked_bars::KIND_ID, "VARCHAR"),
    ("reading", chart_kinds::BINNED_HISTOGRAM, "BIGINT"),
];

/// The column of the fixture that earns no tile, and why: one distinct value,
/// whose histogram is one bar and whose ranking is one row.
///
/// Pinned beside the tiles because a column vanishing from a generated
/// analysis is indistinguishable from a bug in the generator, and the picture
/// alone cannot tell the reader which — the omission is written into the
/// emitted spec's comment block, not drawn.
const OMITTED_COLUMN: &str = "sensor";

/// The declaration order of the kinds a lone column can fill.
///
/// [`dashboard::single_column_kinds`] answers in the registry's own order and
/// the chooser takes the first kind that accepts the field, so this list *is*
/// the preference between two applicable kinds. The three kinds declare
/// disjoint slot types today, so reordering them moves no tile in this fixture
/// — which is why the order is pinned here rather than left to the choice
/// table: a reorder is a change to the tiebreak, and this assertion is what
/// reports it.
const PREFERENCE: &[ChartKindId] = &[
    chart_kinds::BINNED_HISTOGRAM,
    chart_kinds::COUNTS_OVER_TIME,
    ranked_bars::KIND_ID,
];

/// [`EXPECTED`] as the lines [`chosen_lines`] produces, so a failure prints two
/// readable lists rather than two debug dumps.
fn expected_lines() -> Vec<String> {
    EXPECTED
        .iter()
        .map(|(column, kind, type_name)| format!("{column}: {kind} (from {type_name})"))
        .collect()
}

/// What the generator actually chose, one line per tile: the column, the kind,
/// and what decided the kind.
fn chosen_lines(dash: &Dashboard) -> Vec<String> {
    dash.tiles()
        .iter()
        .map(|tile| {
            let because = match tile.chosen_by() {
                ChosenBy::Storage { type_name } => format!("from {type_name}"),
                ChosenBy::Meaning { label, role } => {
                    format!("from the label {label}, read as {role:?}")
                }
                ChosenBy::CoordinatePair { latitude, rule } => {
                    format!("paired with {latitude} by its {rule}")
                }
            };
            format!("{}: {} ({because})", tile.column(), tile.kind())
        })
        .collect()
}

/// **The structural half of this baseline**: the kind each column was given, by
/// name, and the column that was given none.
///
/// Called by the pixel test before it photographs anything, and asserted on its
/// own below.
fn assert_choices(dash: &Dashboard) {
    assert_eq!(
        chosen_lines(dash),
        expected_lines(),
        "the generator's tile choices for {} have moved. Left is what it chose \
         on this run; right is what the committed baseline image was recorded \
         against. A different kind for a column is a different dashboard, \
         however similar the two ink at this size — re-recording the image \
         without reading this line is the failure this assertion exists to \
         stop.",
        fixture().display()
    );

    let omitted: Vec<&str> = dash.omitted().iter().map(|o| o.column.as_str()).collect();
    assert_eq!(
        omitted,
        vec![OMITTED_COLUMN],
        "a different set of columns was left out of the dashboard for {}, and \
         an omission is invisible in the picture — it is written into the \
         emitted spec's comment block and nowhere else",
        fixture().display()
    );
    let left = &dash.omitted()[0];
    assert!(
        matches!(left.because, Omission::OneValue),
        "{OMITTED_COLUMN} was left out for {:?} rather than for holding one \
         distinct value, so the fixture no longer exercises the rule it was \
         written to exercise",
        left.because
    );
}

/// **Which kind each column of a table this build has never met is given** —
/// the choice, per column, by name.
///
/// The claim the committed image cannot carry on its own, and the one that has
/// to fail legibly: a semantic-type change, a new kind whose required slot a
/// lone column happens to fill, or a registry reordering each move these
/// answers, and this is the assertion that says which column moved and to what.
///
/// A column of each of three kinds, so the fixture says something about the
/// choosing rather than about one type: a date counted over time, a category
/// ranked, a measure binned — and a fourth column given nothing.
#[test]
fn each_column_of_the_table_gets_the_tile_its_type_earns() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_choices(&opened.dashboard);
}

/// **The preference between two applicable kinds is the registry's declaration
/// order**, and that order is here rather than only in the registry.
///
/// [`dashboard::single_column_kinds`] filters the registry to the kinds one
/// column can fill and keeps the registry's order; the chooser takes the first
/// of them that accepts the field. Swapping two declarations is therefore a
/// change to which picture a column gets — and, with the slot types the three
/// kinds declare today, a change that moves no pixel and no tile, so the choice
/// table above would stay green through it.
#[test]
fn the_preference_between_applicable_kinds_is_the_registrys_declaration_order() {
    let declared: Vec<&str> = dashboard::single_column_kinds()
        .iter()
        .map(|kind| kind.id.as_str())
        .collect();
    let pinned: Vec<&str> = PREFERENCE.iter().map(|id| id.as_str()).collect();
    assert_eq!(
        declared, pinned,
        "the kinds a lone column can fill are declared in a different order \
         than the dashboard baseline was recorded against. That order is the \
         preference — the chooser takes the first kind whose slots accept the \
         column's field — so a swap here changes which picture a column gets \
         wherever two kinds accept one field type."
    );
}

/// **The dashboard generated for a data file, as pixels** — the composed
/// picture a reader is shown for a table nobody wrote a spec for.
///
/// The boot is [`Boot::data_file`]'s, which is the boot the front door's picker
/// builds and the boot `brightfield-shot --spec table.csv` builds; the capture
/// is the crate's own headless path. The table is [`housing`] — the first
/// screen is a picture of a file with a map in it. The dashboard that chose the tiles is
/// asked for separately, because a `Boot` carries the composed document rather
/// than the walk that produced it.
///
/// The tile CHOICES are mode-independent — they are read off column types, not
/// off ink — so [`assert_housing`] runs here and the dark twin below inherits
/// its verdict rather than restating it. What the pair holds that neither half
/// can alone is that the ink moves and nothing else does.
#[test]
fn the_generated_dashboard_light_baseline() {
    assert_one_grid_per_frame(&[]);
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");

    // The structural guard, ahead of the photograph, for the reason in this
    // file's header: `UPDATE_SNAPSHOTS=1` writes whatever `image_snapshot` is
    // handed.
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_housing(&opened.dashboard);
    assert_eq!(
        opened.composed.plots.len(),
        HOUSING_PLOTS.len(),
        "the walk chose {} tiles and the composition placed {} plots, so the \
         image below is not a picture of those choices",
        HOUSING_PLOTS.len(),
        opened.composed.plots.len()
    );
    drop(opened);

    // Hermetic capture: keep `BRIGHTFIELD_DEVTOOLS` from baking the top bar's
    // renderer string into a regenerated golden, as `tests/surfaces.rs` does.
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("dashboard_light");
    let (w, h) = capture_png(boot, Mode::Light, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture dashboard_light: {e}"));
    assert!(w > 0 && h > 0, "dashboard_light: empty capture");

    // PNG is lossless, so reading the capture back is pixel-exact; the file on
    // disk is the way `capture_png` hands its result over.
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "dashboard_light");
}

/// Device-pixel count of exactly `token` in `image`.
///
/// Exact, not perceptual: an interior pixel of a filled rect is the fill
/// colour, and the two surface tokens under test differ by far more than any
/// rounding in the Rgba8Unorm round-trip. A tolerance here would let the defect
/// through under the name of robustness.
fn pixels_of(image: &image::RgbaImage, token: meridian_design::colour::Rgba) -> usize {
    let want = [
        (token.r * 255.0).round() as u8,
        (token.g * 255.0).round() as u8,
        (token.b * 255.0).round() as u8,
    ];
    image
        .pixels()
        .filter(|p| p.0[0] == want[0] && p.0[1] == want[1] && p.0[2] == want[2])
        .count()
}

/// **The same generated dashboard in dark** — and, held in the same test, the
/// claim the picture is here to make: **not one pixel of it is the light chart
/// surface.**
///
/// The image half is the baseline; the pixel half is what makes a red baseline
/// legible. A dark window whose chart pane is a white slab differs from this
/// golden in tens of thousands of pixels and a reviewer reading a perceptual
/// diff cannot tell that from a font bump — so the surface count is asserted by
/// name, ahead of the photograph, for the reason [`assert_choices`] runs ahead
/// of it.
///
/// This dashboard reaches dark through a path the light twin does not exercise:
/// [`Boot::data_file`] composes before anything knows the mode, and
/// `ChartDoc::set_mode` re-presents through the live session it left behind on
/// the first frame that names one. So this is also the regression test for that
/// seam — remove it and the capture goes back to photographing a light
/// composition inside a dark window.
#[test]
fn the_generated_dashboard_dark_baseline() {
    assert_one_grid_per_frame(&[]);
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("dashboard_dark");
    let (w, h) = capture_png(boot, Mode::Dark, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture dashboard_dark: {e}"));
    assert!(w > 0 && h > 0, "dashboard_dark: empty capture");

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();

    let light = pixels_of(&image, meridian_design::chrome::INK_LIGHT.surface);
    assert_eq!(
        light, 0,
        "{light} pixels of this dark dashboard are the LIGHT chart surface \
         (#fcfcfb). That colour has one source — the plot background — so this \
         window is drawing a white slab exactly where the analyst is reading."
    );
    let dark = pixels_of(&image, meridian_design::chrome::INK_DARK.surface);
    assert!(
        dark > 0,
        "no pixel of this dark dashboard is the dark chart surface (#161413), \
         so the plot background is neither of the two colours it can be and \
         the assertion above is passing for the wrong reason"
    );

    egui_kittest::image_snapshot(&image, "dashboard_dark");
}

/// **A data file whose columns are `DECIMAL` opens as the dashboard its
/// `DOUBLE` twin opens as**, scale for scale and pixel for pixel.
///
/// [`housing_decimal`] is [`housing`] with seven of its nine columns stored as
/// `DECIMAL`, and the CSV is its `DOUBLE` twin: DuckDB reads each CSV value as
/// the double the `DECIMAL` casts to. So the generator makes the same choices,
/// which [`assert_housing`] holds, and every plot of the composition must carry
/// the scales and draw the pixels the CSV's plot does. The hero map reads its
/// two `DECIMAL` coordinates through brightfield-render's `Decimal128` arms;
/// with those arms gone its scales come back empty and its dots undrawn.
///
/// The composed scene is compared rather than the window, because the window's
/// rails print each column's type and the file's name, which are what differ.
#[test]
fn a_decimal_data_file_draws_the_dashboard_its_double_twin_draws() {
    let open = |path: PathBuf| {
        let chosen = path.to_str().expect("utf-8 fixture path").to_owned();
        data_file::open(&chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
    };
    let decimal = open(housing_decimal());
    let double = open(housing());

    let mut stored_decimal: Vec<&str> = decimal
        .dashboard
        .tiles()
        .iter()
        .filter(|t| {
            matches!(t.chosen_by(), ChosenBy::Storage { type_name } if type_name.starts_with("DECIMAL"))
        })
        .map(|t| t.column())
        .collect();
    stored_decimal.sort_unstable();
    assert_eq!(
        stored_decimal, HOUSING_DECIMAL_COLUMNS,
        "fixture check: these are the columns whose tiles DuckDB's DECIMAL type \
         decided, so the file no longer carries the DECIMAL columns this test \
         is about"
    );
    assert_housing(&decimal.dashboard);
    assert_eq!(
        decimal.composed.plots.len(),
        double.composed.plots.len(),
        "the DECIMAL file composed a different count of plots"
    );

    for (i, (d, f)) in decimal
        .composed
        .plots
        .iter()
        .zip(&double.composed.plots)
        .enumerate()
    {
        let column = decimal.dashboard.plot_order()[i].column();
        for &channel in Channel::all() {
            assert_eq!(
                format!("{:?}", d.scales.get(channel)),
                format!("{:?}", f.scales.get(channel)),
                "plot {i} ({column}): the DECIMAL file's {channel:?} scale must \
                 be its DOUBLE twin's"
            );
        }
        assert_eq!(
            d.scales.geo_extent(),
            f.scales.geo_extent(),
            "plot {i} ({column}): the DECIMAL file's map extent must be its \
             DOUBLE twin's"
        );
    }

    let (decimal_png, double_png) = (scratch("decimal_twin"), scratch("double_twin"));
    capture_vello_only(decimal.composed, SCALE, &decimal_png)
        .unwrap_or_else(|e| panic!("capture the DECIMAL dashboard: {e}"));
    capture_vello_only(double.composed, SCALE, &double_png)
        .unwrap_or_else(|e| panic!("capture the DOUBLE dashboard: {e}"));
    let read = |png: &PathBuf| {
        image::open(png)
            .unwrap_or_else(|e| panic!("read capture {}: {e}", png.display()))
            .to_rgba8()
    };
    let (decimal_img, double_img) = (read(&decimal_png), read(&double_png));
    assert_eq!(decimal_img.dimensions(), double_img.dimensions());
    let differing = decimal_img
        .pixels()
        .zip(double_img.pixels())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing,
        0,
        "the DECIMAL file's dashboard differs from its DOUBLE twin's in \
         {differing} pixels; see {} and {}",
        decimal_png.display(),
        double_png.display()
    );
}

/// **The window a data file opens at does not grow with the count of tiles
/// beside the hero.**
///
/// Two generated dashboards, one from each fixture this file already opens:
/// [`housing`] earns a tile for each of its nine columns beside the joint map,
/// [`site_readings`] four beside its own. The generator's page is as tall as
/// the taller of the hero and that column, so those two compose pages of
/// different heights — asserted here, because a pair of fixtures whose pages
/// happened to agree would let this test pass over the arithmetic it is about.
/// The window each boot asks for is then read back, and it is one size.
///
/// What it pins is the route `Boot::window_size` takes for a generated
/// dashboard: `rows_layout_window_size`, which caps the page at the hero's own
/// height. Read straight through `chart_window_size`, as it was until this
/// test existed, the taller page opens a taller window — a window sized
/// around a column the rows layout composes out of sight and clips away.
#[test]
fn the_window_a_data_file_opens_at_does_not_grow_with_the_tile_count() {
    let tall_path = housing();
    let tall = Boot::data_file(tall_path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", tall_path.display()));
    let short_path = site_readings();
    let short = Boot::data_file(short_path.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("open {}: {e}", short_path.display()));

    assert_ne!(
        tall.stacked_tiles, short.stacked_tiles,
        "both fixtures stand the same number of tiles beside the hero \
         ({:?}), so this test cannot tell a window derived from the column \
         from one that is not",
        tall.stacked_tiles
    );
    assert_ne!(
        tall.composed.height, short.composed.height,
        "the two fixtures compose pages of the same height ({} points), so the \
         tile count has nothing left to leak into the window and this test \
         would stay green with the cap removed",
        tall.composed.height
    );

    let (tall_size, short_size) = (tall.window_size(), short.window_size());
    assert_eq!(
        tall_size, short_size,
        "{:?} tiles beside the hero open a window of {tall_size:?} and {:?} \
         tiles open {short_size:?}, so the size a data file opens at is \
         following the tile column — which the rows layout it opens on \
         composes out of sight and clips away",
        tall.stacked_tiles, short.stacked_tiles
    );
}

/// The window the scrolled capture below is taken in — the size the
/// composition this card is cut from was drawn at, and short enough that nine
/// tiles at their 96-point floor need a page taller than the pane.
const SHORT_WINDOW: (f32, f32) = (1440.0, 900.0);

/// One turn of the wheel in logical points, and how many frames carry one.
///
/// Enough travel to reach the end of the column's scroll, which is where the
/// page is furthest from where it was composed and therefore where a missing
/// clip paints over the most chrome. The offset the frame reached is clamped by
/// the window, so over-turning the wheel is how a test scrolls "to the end"
/// without naming a distance.
const WHEEL_TRAVEL: f32 = 400.0;
/// How many frames of [`WHEEL_TRAVEL`] the scripted capture turns.
const WHEEL_FRAMES: usize = 8;

/// A settled headless window over [`housing`] at `size`, for reading the pane
/// rects the capture below is measured against.
///
/// The layout is the one the capture runs: `MeridianApp::headless` differs from
/// the device path in the raster alone — the canvas pane reserves the same box
/// and paints nothing into it — which is the property `tests/canvas_pane_group.rs`
/// is built on.
fn pane_rects(
    size: (f32, f32),
) -> (
    Vec<brightfield_shell::window::CanvasPane>,
    egui::Pos2,
    egui::Pos2,
) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0, size.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    // The control that reopens the ledger rail, read before it is clicked and
    // returned for the capture to aim at. A data file opens as a one-step
    // Protocol, so the rail opens closed to its strip and hands the canvas its
    // other 124 points, which is room the grid pane's page needs to overrun
    // before a scroll is a scroll rather than a no-op. See
    // `settled_scrollable` in `tests/canvas_pane_group.rs`, which reopens it
    // for the same reason.
    let control = app
        .rail_collapse_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
        .expect("the collapsed ledger drew the control that reopens it")
        .center();
    for events in reopen_the_ledger(control) {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    }

    // …and the layout switch, thrown to the grid's transposed state, for the
    // reason `settled_scrollable` throws it: the grid pane's rows are the page
    // taller than the pane it is drawn in, and a page that fits its pane is a
    // page no clip and no scroll can be asserted about. The rect is read off
    // the frame that drew it and returned, so the capture aims at the same
    // place this window did.
    let at = transposed_state_rect(&app).center();
    for events in throw_the_switch(at) {
        let mut input = raw.clone();
        input.events = events;
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    }
    assert_eq!(
        app.grid_layout(),
        brightfield_shell::app::GridLayout::Columns,
        "the click at {at:?} did not throw the layout switch, so the page \
         below is the one that fits its pane and the scroll claims would pass \
         for want of a page"
    );
    (app.canvas_panes().panes.clone(), control, at)
}

/// Where the grid pane's header band drew the layout switch's transposed
/// state, on the frame `app` last ran.
///
/// Read off the frame rather than typed: a coordinate that missed would leave
/// the grid on its rows and every claim below would be about the wrong page.
fn transposed_state_rect(app: &brightfield_shell::window::MeridianApp) -> egui::Rect {
    app.chart_doc()
        .grid_layout_switch
        .clone()
        .expect("the grid pane's header band drew a layout switch")
        .states
        .iter()
        .find(|(state, _)| *state == brightfield_shell::app::GridLayout::Columns)
        .expect("the switch offers a transposed state")
        .1
}

/// The frames that throw the layout switch by clicking `at` — the same shape
/// as [`reopen_the_ledger`], and settled for the same reason.
fn throw_the_switch(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![egui::Event::PointerMoved(at), button(true)],
        vec![egui::Event::PointerMoved(at), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

/// The frames that reopen a collapsed ledger rail by clicking `control` — one
/// to put the pointer there, one carrying the press and release, and three to
/// settle the panel egui reads back on the frame after.
fn reopen_the_ledger(control: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: control,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(control)],
        vec![button(true), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

/// **The two panes clip their own share of the one page, and a scroll moves
/// one of them** — held over a pair of captures at 1440 by 900 with the grid
/// transposed, where the file's nine rows at their tile floor need a page
/// taller than the pane that draws them.
///
/// The canvas draws two panes: the hero, and the grid beside it at the
/// canvas's full height. Transposed is the layout the scroll claims are read
/// in, for the reason `settled_scrollable` in `tests/canvas_pane_group.rs`
/// gives — on its rows the grid's page fits the pane and a page that fits its
/// pane is one no scroll can be asserted about.
///
/// Three claims, and the pair is what makes the middle one decidable:
///
/// 1. the wheel over the grid pane **moved** it — the grid pane's content rect
///    differs between the two captures, so nothing below is being asserted
///    about a window where the scroll did nothing;
/// 2. it moved **nothing else** — each pixel outside the grid pane's content
///    rect is identical in the two captures, the map's picture and both header
///    bands included. This is [`the_generated_dashboard_light_baseline`]'s
///    claim about a scrolled window, and it is what reddens when the grid
///    pane's view of the page stops clipping: an unclipped copy of the
///    scrolled page paints across the hero pane and over both bands;
/// 3. no pixel of the marks' own ink lands outside a pane's content rect in
///    either capture — the standing containment check, held over both frames.
///
/// Breaking `by: scroll` to `by: 0.0` in the transposed pane group's
/// `PaneViews` reddens the first of those.
///
/// The **untransposed** layout's containment is not asserted here and is not
/// assertable by mutation: on its rows the page is composed as the hero, a
/// gutter and the tile column, and clipped to the hero pane in three places in
/// series — `child.shrink_clip_rect(clip)` here in `draw_chart_body`, the
/// `shrink_clip_rect(views.first)` `chart_item` narrows the paint with, and
/// the module frame's own content clip — each stating the same rect, so
/// removing any one of them leaves the tile column exactly as invisible as
/// before.
#[test]
fn the_pane_group_clips_the_page_to_the_panes_it_is_drawn_in() {
    let (panes, ledger_control, switch_at) = pane_rects(SHORT_WINDOW);
    assert_eq!(
        panes.len(),
        2,
        "the canvas drew {} panes at {SHORT_WINDOW:?}, so the rects below are \
         not the pane group's",
        panes.len()
    );
    let bodies: Vec<egui::Rect> = panes.iter().map(|p| p.body).collect();
    let grid = panes
        .iter()
        .find(|p| p.name == "grid")
        .expect("the grid pane drew")
        .body;

    // The pointer lands in the same place in both captures and the wheel is
    // the only difference between them, so a pixel that differs is one the
    // scroll moved. Two empty frames lead: a resizable panel's reported size
    // is read back on the frame after, so the pointer has to land on a settled
    // layout.
    let mut point = reopen_the_ledger(ledger_control);
    point.extend(throw_the_switch(switch_at));
    point.extend([
        Vec::new(),
        Vec::new(),
        vec![egui::Event::PointerMoved(grid.center())],
    ]);
    let mut turn = point.clone();
    for _ in 0..WHEEL_FRAMES {
        turn.push(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -WHEEL_TRAVEL),
            modifiers: egui::Modifiers::default(),
            phase: egui::TouchPhase::Move,
        }]);
    }
    let still = capture_short(point, "dashboard_pane_group_still");
    let moved = capture_short(turn, "dashboard_pane_group_scrolled");
    assert_eq!(still.dimensions(), moved.dimensions());

    let mut in_grid = 0usize;
    let mut elsewhere: Vec<(u32, u32)> = Vec::new();
    for (x, y, p) in still.enumerate_pixels() {
        if p == moved.get_pixel(x, y) {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let at = egui::pos2(x as f32 / SCALE, y as f32 / SCALE);
        if grid.expand(1.0).contains(at) {
            in_grid += 1;
        } else {
            elsewhere.push((x, y));
        }
    }
    assert!(
        in_grid > 0,
        "the two captures are identical inside the grid pane's content rect \
         {grid:?}, so the wheel scrolled nothing and the comparison below \
         holds over a window this test is not about"
    );
    assert!(
        elsewhere.is_empty(),
        "{} device pixels outside the grid pane's content rect changed when \
         the grid pane scrolled — the first five at {:?}. Scrolling the grid \
         pane moves the grid pane; the map's picture, both header bands and \
         the frames around them are somebody else's.",
        elsewhere.len(),
        &elsewhere[..elsewhere.len().min(5)]
    );

    // …and each pane's own bottom frame survives the page laid out over it.
    // This is the one the mark ink cannot see: what the page paints into a
    // pane's inset is its BACKGROUND, and the chart surface and a pane's fill
    // are the same token, so the visible loss is the hairline the page covers.
    //
    // With the clip in place, each pane's strip carries the border colour
    // across its width on at least one device row below its content rect —
    // its own stroke. Without the clip the page is laid out across the union
    // of the two panes' content rects and painted from the hero pane's origin,
    // so it reaches over the hero pane's bottom frame and the row below the
    // hero's content rect stops being the stroke. The `rows >= 1` below is
    // that as an assertion, and the hero pane is the one it bites on.
    let border = meridian_design::semantic(false).borders.subtle;
    for pane in &panes {
        let strip = pane.rect.bottom() - pane.body.bottom();
        assert!(
            strip > 1.0,
            "the {} pane's content rect ends at its own bottom edge, so there \
             is no frame below it to paint over and this claim is empty",
            pane.name
        );
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = (0..(strip * SCALE).ceil() as u32 + 1)
            .filter(|dy| {
                let y = (pane.body.bottom() * SCALE) as u32 + dy;
                let run = ((pane.body.left() * SCALE) as u32..(pane.body.right() * SCALE) as u32)
                    .filter(|x| {
                        y < still.height()
                            && *x < still.width()
                            && is_token(still.get_pixel(*x, y), border)
                    })
                    .count();
                #[allow(clippy::cast_precision_loss)]
                let across = run as f32 >= 0.9 * pane.body.width() * SCALE;
                across
            })
            .count();
        assert!(
            rows >= 1,
            "the {} pane's bottom frame runs the pane's full width on {rows} \
             device rows below its content rect — the page is painting over \
             the frame of the pane it is drawn in",
            pane.name
        );
    }

    for (name, image) in [("still", &still), ("scrolled", &moved)] {
        let mut ink = 0usize;
        let mut stray: Vec<(u32, u32)> = Vec::new();
        for (x, y, p) in image.enumerate_pixels() {
            if !is_mark_ink(p) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let at = egui::pos2(x as f32 / SCALE, y as f32 / SCALE);
            if bodies.iter().any(|body| body.expand(1.0).contains(at)) {
                ink += 1;
            } else {
                stray.push((x, y));
            }
        }
        assert!(
            ink > 0,
            "not one pixel of the {name} capture is the marks' own ink, so the \
             count below is passing for the wrong reason — the picture did not \
             draw"
        );
        assert!(
            stray.is_empty(),
            "{} device pixels of the marks' own ink landed outside both panes' \
             content rects {bodies:?} in the {name} capture — the first five \
             at {:?}. The page is painting where no picture is drawn.",
            stray.len(),
            &stray[..stray.len().min(5)]
        );
    }
}

/// [`housing`] captured at [`SHORT_WINDOW`] under `script`, read back as
/// pixels.
fn capture_short(script: Vec<Vec<egui::Event>>, name: &str) -> image::RgbaImage {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at(
        boot,
        Mode::Light,
        SCALE,
        SHORT_WINDOW,
        &out,
        script,
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// Whether `p` is the first series colour of the chart palette — the ink every
/// mark of this dashboard is drawn in.
///
/// The right ink to count for a clip: it has exactly one source in the window.
/// The chart *surface* would be the wrong one — a pane's own frame is filled
/// with the same token, so a page painting over a pane's inset would be
/// invisible to it: the surface colour is already down there, below the panes'
/// content rects, with the clip in place. The mark ink is not, and that is the
/// assertion — `stray.is_empty()` in
/// [`the_pane_group_clips_the_page_to_the_panes_it_is_drawn_in`].
///
/// Exact rather than perceptual, for the reason [`pixels_of`] gives: a mark's
/// interior is the flat fill, and antialiasing at its edge produces neighbours
/// this deliberately does not count.
fn is_mark_ink(p: &image::Rgba<u8>) -> bool {
    is_token(p, meridian_design::viz::CATEGORICAL_LIGHT[0])
}

/// Whether `p` is exactly `token`, alpha ignored.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn is_token(p: &image::Rgba<u8>, token: meridian_design::colour::Rgba) -> bool {
    p.0[0] == (token.r * 255.0).round() as u8
        && p.0[1] == (token.g * 255.0).round() as u8
        && p.0[2] == (token.b * 255.0).round() as u8
}

// ---------------------------------------------------------------------------
// A time axis at a dashboard tile's real width
// ---------------------------------------------------------------------------

/// The `day` column's real dates, walked off the composed dashboard's own
/// resolved `Scale::Band` for [`fixture`] rather than typed a second time —
/// a change to the fixture's dates cannot leave this test checking a set the
/// picture no longer draws.
fn fixture_day_categories(composed: &brightfield_shell::pipeline::Composed) -> Vec<String> {
    let day_plot = composed
        .plots
        .iter()
        .find(|p| p.x_column.as_deref() == Some("day"))
        .expect("fixture check: the day column earns a tile with x: day");
    match day_plot.scales.get(brightfield_render::channel::Channel::X) {
        Some(brightfield_render::scale::Scale::Band { categories, .. }) => categories.clone(),
        other => panic!("fixture check: day's x scale is not a band scale: {other:?}"),
    }
}

/// The plot `composed` placed whose x channel is bound to `column`, found by
/// that binding rather than by position — a hero promotion or a column
/// reorder must point this at a different plot, not silently keep reading the
/// same index.
fn plot_for_x_column<'a>(
    composed: &'a brightfield_shell::pipeline::Composed,
    column: &str,
) -> &'a brightfield_shell::pipeline::PlotHandle {
    composed
        .plots
        .iter()
        .find(|p| p.x_column.as_deref() == Some(column))
        .unwrap_or_else(|| panic!("fixture check: no placed plot binds x to {column}"))
}

/// `(x0, y0, x1, y1)` — an axis-aligned rect in the composed PAGE's own
/// coordinate space, the same space [`brightfield_shell::pipeline::PlotHandle::rect`]
/// lives in.
type Rect4 = (f64, f64, f64, f64);

/// Whether `a` and `b` share any interior point.
fn rects_intersect(a: Rect4, b: Rect4) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Whether `inner` lies inside `outer`, both edges included up to float
/// round-trip slop.
fn rect_inside(inner: Rect4, outer: Rect4) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.2 <= outer.2 + 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.3 <= outer.3 + 0.5
}

/// The rects `render_x_axis` (`brightfield-render`'s `axis` module) drew for
/// `plot`'s x axis, read straight out of the real composed `scene` —
/// [`Composed::scene`]'s glyph runs, not a second render — rather than
/// assumed from which branch it is expected to have taken: the tick labels
/// (whichever family `render_x_axis` chose — thinned, degraded to one, or
/// rotated) as the first element, and the axis title's rect, when the tile
/// draws one, as the second.
///
/// A run's own [`vello_encoding::GlyphRun::font_size`] is what tells a title
/// apart from a tick label — the SAME field `render_x_axis` sets when it
/// draws each (`TITLE_SIZE` for the title, `size` for a tick label,
/// whichever way it drew them), rather than the two being told apart by
/// where they landed. A ROTATED run's rect runs the other way from a
/// horizontal one: its OWN glyph-height footprint (`size` wide) is centred on
/// its pivot along x, and its measured text length runs DOWN from the pivot
/// along y — the near end is nearest the tick
/// (`draw_text_rotated`'s `TextAnchor::End`) — so a run that has clipped past
/// the tile or under the title is exactly one whose far end has run past
/// `plot.rect.height` or into the title's own rect, which
/// `rect_inside`/`rects_intersect` read directly off the rects this returns.
fn drawn_x_axis_rects(
    scene: &vello::Scene,
    plot: &brightfield_shell::pipeline::PlotHandle,
    ticks: &[brightfield_render::axis::Tick],
    title_text: &str,
    size: f32,
) -> (Vec<Rect4>, Option<Rect4>) {
    let title_size = brightfield_render::text::TITLE_SIZE;
    let title_width = brightfield_render::text::measure_width(title_text, title_size);

    // Wide enough to catch a label that has overflowed the tile — the
    // defect this file's sweep test exists to catch — but derived from the
    // labels themselves rather than a flat pad: a pad generous enough at
    // one window width reached past the gutter into a NEIGHBOUR plot's own
    // labels at a narrower one, measured on this build at an 888-point
    // window — a flat 200-point pad swept up the map pane's own latitude
    // row label, roughly 200 points left of day's own plot, and mismatched
    // it against day's nearest tick candidate by position alone. `render_x_axis`
    // centres a horizontal label on a tick position already inside the
    // plot's own range, so an unclamped label can overflow by at most half
    // its own width on either side, and a rotated one by at most half
    // `size` — the widest label this axis actually draws already bounds
    // both, with room to spare short of the next plot over.
    let slack = ticks
        .iter()
        .map(|t| brightfield_render::text::measure_width(&t.label, size))
        .fold(f64::from(size), f64::max);
    let x_lo = plot.rect.x - slack;
    let x_hi = plot.rect.x + plot.rect.width + slack;
    // Half a label's cap height below the axis line, not the line itself:
    // the y-axis's own occasional stray label (a y-tick landing exactly on
    // `plot_y_end` draws its row-label `LABEL_SIZE / 3` below the line) sits
    // short of this floor, while the x-axis's tick-label row (`LABEL_SIZE`
    // below the line, or the rotated pivots' `ROTATED_LABEL_GAP` short of
    // that) clears it.
    let y_lo = plot.rect.y + plot.layout.plot_y_end() + f64::from(size) / 2.0;
    let y_hi = plot.rect.y + plot.rect.height;

    // A horizontal draw (`TextAnchor::Middle`) anchors its LEFT edge at
    // `position - width / 2`; a rotated one (`TextAnchor::End`) anchors its
    // PIVOT exactly at `position` — two different predicted x0s for the same
    // tick, so two candidate lists, matched against whichever family a given
    // run turns out to belong to.
    let horiz_candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| {
            (
                plot.rect.x + t.position
                    - brightfield_render::text::measure_width(&t.label, size) / 2.0,
                t.label.as_str(),
            )
        })
        .collect();
    let rotated_candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| (plot.rect.x + t.position, t.label.as_str()))
        .collect();

    let mut label_rects = Vec::new();
    let mut title_rect = None;
    for run in &scene.encoding().resources.glyph_runs {
        let x0 = f64::from(run.transform.translation[0]);
        let y0 = f64::from(run.transform.translation[1]);
        if x0 < x_lo || x0 > x_hi || y0 < y_lo || y0 > y_hi {
            continue;
        }
        if (run.font_size - title_size).abs() < 0.01 {
            // The axis title itself: `TextAnchor::Middle`, so x0 is already
            // its own left edge.
            title_rect = Some((x0, y0 - f64::from(title_size), x0 + title_width, y0));
            continue;
        }
        let m = run.transform.matrix;
        let rotated = m[0].abs() < 1e-3 && m[3].abs() < 1e-3;
        if rotated {
            let (_, label) = rotated_candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            let run_len = brightfield_render::text::measure_width(label, size);
            label_rects.push((
                x0 - f64::from(size) / 2.0,
                y0,
                x0 + f64::from(size) / 2.0,
                y0 + run_len,
            ));
        } else {
            let (_, label) = horiz_candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            let width = brightfield_render::text::measure_width(label, size);
            label_rects.push((x0, y0 - f64::from(size), x0 + width, y0));
        }
    }
    (label_rects, title_rect)
}

/// A `day`-axis rendered in isolation at `width`, over `categories` — the same
/// [`brightfield_render::axis::compute_ticks`] / `render_x_axis` path a
/// counts_over_time tile's own scene draws its x axis through, at
/// [`brightfield_render::layout::ChartLayout`]'s own inset-adjusted x range.
fn day_axis_scene(
    categories: &[String],
    width: f64,
) -> (vello::Scene, Vec<brightfield_render::axis::Tick>) {
    let layout = brightfield_render::layout::ChartLayout::new(width, 300.0);
    let (range_start, range_end) = layout.x_range();
    let scale = brightfield_render::scale::Scale::Band {
        categories: categories.to_vec(),
        range_start,
        range_end,
        padding: 0.1,
    };
    let ticks = brightfield_render::axis::compute_ticks(&scale, 5);
    let mut scene = vello::Scene::new();
    brightfield_render::axis::render_x_axis(
        &mut scene,
        &layout,
        &ticks,
        None,
        brightfield_render::ink::ChartInk::LIGHT,
    );
    (scene, ticks)
}

/// Matches each horizontal glyph run in `scene` back to whichever `ticks`
/// entry its draw position (`TextAnchor::Middle`, `render_x_axis`'s own
/// anchor) is nearest, then asserts no two runs' `[x, x + width]` intervals
/// intersect — the width read with `measure_width`, the shaping
/// `render_x_axis` measured it with, rather than estimated from the run's raw
/// glyph count. A run whose transform carries a quarter turn is skipped: a
/// rotated axis is a different claim, made in `brightfield-render`'s own
/// `axis` tests.
fn assert_no_tick_label_overlap(
    scene: &vello::Scene,
    ticks: &[brightfield_render::axis::Tick],
    size: f32,
) {
    let candidates: Vec<(f64, &str)> = ticks
        .iter()
        .map(|t| {
            (
                t.position - brightfield_render::text::measure_width(&t.label, size) / 2.0,
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
        spans.push((
            x0,
            x0 + brightfield_render::text::measure_width(label, size),
        ));
    }
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for pair in spans.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "two drawn tick labels overlap: {pair:?} (all spans: {spans:?})"
        );
    }
}

/// The logical window [`site_readings`]'s own baseline captures
/// ([`the_site_readings_dashboard_light_baseline`] and its dark twin) open
/// at — [`Boot::window_size`], the natural size their `capture_png` call
/// derives with no explicit size of its own.
fn site_readings_baseline_window() -> (f32, f32) {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    boot.window_size()
}

/// [`site_readings`] driven through the REAL pane group at a window `size` —
/// the same `MeridianApp::headless` + `ctx.run_ui` settle loop [`pane_rects`]
/// drives, three frames so a resizable panel's reported size is read back
/// on the frame after (the same reason [`pane_rects`] settles before reading
/// anything back). Returns the settled app so its `chart_doc().composed` can
/// be read by reference: `day`'s tile at whatever width THIS window's
/// constrained `hconcat` resolved for it, not [`data_file::open`]'s own
/// one-shot, UNCONSTRAINED composition — which is what this card's round 2
/// read, and which sits at `crate::dashboard::COLUMN_TILE_WIDTH` (380
/// points) regardless of window, because an unconstrained `hconcat` sizes
/// each item to its OWN declared weight rather than sharing a box out. The
/// live app does not lay this dashboard out unconstrained — `ChartDoc::reflow_to`
/// hands `LiveDashboard::set_viewport` a real box first, the box the window's
/// own arrangement gave the chart pane.
fn site_readings_app_at(size: (f32, f32)) -> brightfield_shell::window::MeridianApp {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0, size.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// Sweep granularity, in logical points, for
/// [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`] — this
/// is small enough to land inside a violation band as narrow as 48 points,
/// which is what round 3's own hand sweep found (window widths 1064 to
/// 1016, four points at a time: a real capture at 1024 sliced a date's last
/// digit against the tile's own right edge, a width round 3's four FIXED
/// samples missed). Doubled to eight rather than kept at
/// four because this suite already carries a 60-minute CI ceiling other
/// tiers spend more slowly than this one does; eight still lands several
/// samples inside a band that width.
const SWEEP_STEP: f32 = 8.0;

/// The widest window [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`]'s
/// sweep starts from — chosen to sit above both
/// [`site_readings_baseline_window`] and round 2/3's fixed samples.
const SWEEP_START_WIDTH: f32 = 1600.0;

/// A backstop on how far the sweep is allowed to descend before the test
/// gives up looking for the app's own layout floor and fails outright,
/// rather than looping toward zero. The sweep is expected to stop well
/// above this — see
/// [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`].
const SWEEP_MIN_WIDTH: f32 = 100.0;

/// One width's worth of the sweep in
/// [`the_time_axis_never_overlaps_or_clips_across_real_window_widths`]: drive
/// the real pane group at `window_width`, read `day`'s own placed tile and
/// its drawn axis rects back out of the composed scene, and assert
/// containment in the tile, clearance from the axis title, and pairwise
/// clearance between labels. Returns the tile width actually resolved, so
/// the caller can tell a genuinely narrower sample from the app's layout
/// floor repeating the last one.
fn assert_day_axis_at_window_width(window_height: f32, window_width: f32) -> f64 {
    let app = site_readings_app_at((window_width, window_height));
    let composed = &app.chart_doc().composed;
    let day_plot = plot_for_x_column(composed, "day");
    let tile_width = day_plot.rect.width;
    assert!(
        tile_width > 0.0,
        "fixture check: day's tile drew at a non-positive width at window \
         width {window_width}"
    );

    let x_scale = day_plot
        .scales
        .get(brightfield_render::channel::Channel::X)
        .expect("fixture check: day's placed plot carries an x scale");
    let ticks = brightfield_render::axis::compute_ticks(x_scale, 5);
    assert_eq!(
        ticks.len(),
        6,
        "fixture check: site_readings_sample.csv's day column no longer \
         carries six distinct dates"
    );
    let (label_rects, title_rect) = drawn_x_axis_rects(
        &composed.scene,
        day_plot,
        &ticks,
        "day",
        brightfield_render::text::LABEL_SIZE,
    );

    // The narrowest of day's own six dates, on its own, at the size
    // `render_x_axis` draws a horizontal label at — the width below which no
    // centre keeps EVEN the narrowest label inside this tile, so the axis is
    // expected to drop each one of them rather than draw one it cannot
    // contain (tick marks and the title, when present, still draw). This is
    // reachable at the live layout's narrowest resolved widths: measured on
    // this build, a real date is roughly 65 points wide and the column tile
    // shrinks past that.
    let narrowest_label = ticks
        .iter()
        .map(|t| {
            brightfield_render::text::measure_width(&t.label, brightfield_render::text::LABEL_SIZE)
        })
        .fold(f64::MAX, f64::min);
    if narrowest_label > tile_width {
        assert!(
            label_rects.is_empty(),
            "day's tile ({tile_width} points wide) cannot hold even its \
             narrowest date label ({narrowest_label} points), so no \
             tick-label rect should have drawn at window width \
             {window_width} — {} drew instead",
            label_rects.len()
        );
        return tile_width;
    }
    assert!(
        !label_rects.is_empty(),
        "no tick-label rect was drawn for day's x axis at window width \
         {window_width} (its tile drew at {tile_width} points wide, wide \
         enough to hold its narrowest date label at {narrowest_label} points)"
    );

    let tile: Rect4 = (
        day_plot.rect.x,
        day_plot.rect.y,
        day_plot.rect.x + day_plot.rect.width,
        day_plot.rect.y + day_plot.rect.height,
    );
    for rect in &label_rects {
        assert!(
            rect_inside(*rect, tile),
            "a tick-label rect {rect:?} does not lie inside day's own \
             tile rect {tile:?} at window width {window_width} (tile \
             {tile_width} points wide)"
        );
        if let Some(title_rect) = title_rect {
            assert!(
                !rects_intersect(*rect, title_rect),
                "a tick-label rect {rect:?} intersects the axis title's \
                 rect {title_rect:?} at window width {window_width}"
            );
        }
    }
    for i in 0..label_rects.len() {
        for other in &label_rects[i + 1..] {
            assert!(
                !rects_intersect(label_rects[i], *other),
                "two of day's drawn tick-label rects intersect at window \
                 width {window_width}: {:?} and {other:?}",
                label_rects[i],
            );
        }
    }

    tile_width
}

/// **The counts_over_time tile's time axis never overlaps or clips at the
/// widths the live app actually resolves for it** — read off the REAL pane
/// group, swept across the whole range the live layout resolves for a
/// column tile rather than sampled at a handful of fixed widths.
///
/// [`site_readings`] gives the coordinate pair its own hero, so `day` earns
/// an ordinary tile in the STACKED column ([`assert_site_readings`] pins
/// that shape) rather than falling back into the map pane the way
/// [`fixture`]'s `day` does. [`assert_day_axis_at_window_width`] is one
/// width's worth of the sweep: it drives the pane group at that width, reads
/// `day`'s own placed tile and [`drawn_x_axis_rects`]'s rects straight out
/// of the composed scene, and asserts containment in the tile, clearance
/// from the axis title, and pairwise clearance between the drawn labels —
/// whichever way `render_x_axis` chose to draw them
/// ([`site_readings_app_at`]'s own doc names why round 2's version of this
/// test read the wrong quantity in the first place).
///
/// A fixed four-sample version of this test (the baseline window, 1400,
/// 1200, 1000) passed while the property failed BETWEEN the samples: round
/// 3's own hand sweep found a real capture at a 1024-point window slicing a
/// date's last digit against the tile's own right edge, a width none of
/// those four landed on. What follows sweeps [`SWEEP_STEP`]-point steps from
/// [`SWEEP_START_WIDTH`] down to wherever the app's own layout stops moving
/// `day`'s tile width — `brightfield_shell::window::canvas_pane_rects`'s own
/// floor on the grid pane, detected at runtime (two consecutive
/// samples resolving the identical tile width) rather than restated as a
/// constant, because a restated one would rot the moment that floor moved.
#[test]
fn the_time_axis_never_overlaps_or_clips_across_real_window_widths() {
    // Fixture check, once: the shape the whole sweep below assumes.
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_site_readings(&opened.dashboard);
    drop(opened);

    let baseline = site_readings_baseline_window();

    // AC1's own width: the dashboard baseline's window, checked on its own
    // so a failure here reads as "the baseline capture itself is wrong"
    // rather than being buried inside the sweep below.
    assert_day_axis_at_window_width(baseline.1, baseline.0);

    let mut width = SWEEP_START_WIDTH;
    let mut last_tile_width: Option<f64> = None;
    let mut collapsed = false;
    while width >= SWEEP_MIN_WIDTH {
        let tile_width = assert_day_axis_at_window_width(baseline.1, width);
        if let Some(prev) = last_tile_width {
            if (tile_width - prev).abs() < 1e-6 {
                // Two consecutive samples resolved the same tile width: the
                // app's own layout has hit its floor on the grid pane, and
                // `width` was the narrowest sample still on the live side of
                // it (the FIRST floored sample was tested at the step
                // above — its own assertions already ran). Descending
                // further would repeat this identical assertion for however
                // many more steps SWEEP_MIN_WIDTH allowed, for no benefit.
                collapsed = true;
                break;
            }
        }
        last_tile_width = Some(tile_width);
        width -= SWEEP_STEP;
    }
    assert!(
        collapsed,
        "fixture check: the sweep reached {SWEEP_MIN_WIDTH} points wide \
         without the app's own layout ever resolving the same tile width \
         twice in a row for day's column tile — either the grid pane has \
         no floor at this width any more, or SWEEP_MIN_WIDTH needs lowering \
         to reach it"
    );
}

/// **The same axis, at 240 and at 720 points wide** — so the claim above is a
/// rule about the render path rather than a fact about today's one measured
/// column width.
#[test]
fn the_time_axis_does_not_collide_at_240_or_720_points() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let categories = fixture_day_categories(&opened.composed);

    for width in [240.0_f64, 720.0_f64] {
        let (scene, ticks) = day_axis_scene(&categories, width);
        assert_no_tick_label_overlap(&scene, &ticks, brightfield_render::text::LABEL_SIZE);
    }
}

/// **[`site_readings`]'s own generated dashboard, as pixels** — the pair
/// [`the_generated_dashboard_light_baseline`] / `_dark_baseline` draw for
/// [`housing`], drawn instead for the table whose `day` column is what this
/// card's regression is about. Unlike the four-shapes fixture's own baseline
/// pair (which this replaces — see `git log` for that pair's history), `day`
/// draws in the STACKED COLUMN here, at a column tile's real width, rather
/// than falling back into the map pane: a baseline that cannot redden on the
/// time-axis defect is not a pin of it. [`assert_site_readings`] runs ahead of
/// any capture, for the reason this file's header gives.
#[test]
fn the_site_readings_dashboard_light_baseline() {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");

    let opened = data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_site_readings(&opened.dashboard);
    drop(opened);

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("site_readings_dashboard_light");
    let (w, h) = capture_png(boot, Mode::Light, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture site_readings_dashboard_light: {e}"));
    assert!(
        w > 0 && h > 0,
        "site_readings_dashboard_light: empty capture"
    );

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "site_readings_dashboard_light");
}

/// The most pixels of a dark [`site_readings`] capture allowed to land
/// exactly on the light chart surface's bytes before this test reads it as
/// the white-slab regression `the_generated_dashboard_dark_baseline` guards
/// against on [`housing`], rather than as antialiasing.
///
/// [`site_readings`]'s `reading` tile is a binned histogram, whose two
/// `rectY` layers (the unfiltered ghost behind its filtered subset — see
/// `histogram_tile`) meet at an antialiased edge inside the tile. Measured on
/// this build, zero device pixels blend to the light surface's precise bytes
/// even so — this fixture does not reproduce the coincidence the four-shapes
/// fixture once measured (one pixel inside the `region` tile's ranked bars,
/// beside its `"10"` value label, where THAT fixture's ranked-bars tile drew
/// its own highlight-over-total pair; this fixture has no ranked-bars tile).
/// The budget stays at what is actually measured here rather than carrying
/// that figure over, so it still catches the defect this check exists for: a
/// pane painted the light surface wholesale runs to thousands of pixels, nowhere
/// near this floor.
const DARK_CAPTURE_LIGHT_PIXEL_BUDGET: usize = 0;

/// **The dark twin of [`the_site_readings_dashboard_light_baseline`]** — the
/// same white-slab regression check `the_generated_dashboard_dark_baseline`
/// runs for [`housing`], run here for [`site_readings`] instead, with the
/// tolerance [`DARK_CAPTURE_LIGHT_PIXEL_BUDGET`] documents.
#[test]
fn the_site_readings_dashboard_dark_baseline() {
    let path = site_readings();
    let chosen = path.to_str().expect("utf-8 fixture path");

    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch("site_readings_dashboard_dark");
    let (w, h) = capture_png(boot, Mode::Dark, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture site_readings_dashboard_dark: {e}"));
    assert!(
        w > 0 && h > 0,
        "site_readings_dashboard_dark: empty capture"
    );

    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();

    let light = pixels_of(&image, meridian_design::chrome::INK_LIGHT.surface);
    assert_eq!(
        light, DARK_CAPTURE_LIGHT_PIXEL_BUDGET,
        "{light} pixels of this dark dashboard are the LIGHT chart surface \
         (#fcfcfb), past the {DARK_CAPTURE_LIGHT_PIXEL_BUDGET}-pixel budget \
         measured for this fixture — a pane is painting the light surface"
    );
    let dark = pixels_of(&image, meridian_design::chrome::INK_DARK.surface);
    assert!(
        dark > 0,
        "no pixel of this dark dashboard is the dark chart surface (#161413)"
    );

    egui_kittest::image_snapshot(&image, "site_readings_dashboard_dark");
}

// ---------------------------------------------------------------------------
// The grid as the canvas's view of the node — the full column header band.
// ---------------------------------------------------------------------------

/// Where the navigator rail drew its `grid` row at [`SHORT_WINDOW`] — the
/// point the captures below click.
///
/// Read off a settled headless window rather than typed, for the reason
/// [`pane_rects`] reads the ledger control off one: a click at a coordinate
/// that missed would photograph the dashboard again and the pair below would
/// pin the wrong picture. Read in light mode and used for both captures,
/// because the rail's layout does not depend on the mode — the assertion in
/// each test that the grid took the canvas is what says the click landed.
fn grid_row_centre() -> egui::Pos2 {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    let rows = app.spine_rows().to_vec();
    rows.iter()
        .find(|row| row.label == "grid")
        .unwrap_or_else(|| {
            let drawn: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
            panic!("the navigator rail drew no `grid` row; it drew {drawn:?}")
        })
        .rect
        .center()
}

/// The frames that put the grid on the canvas: the pointer onto the rail's
/// `grid` row, the press and release, then three to settle.
fn open_the_grid_view(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![button(true), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

/// The structural guard the two captures below run first.
///
/// `UPDATE_SNAPSHOTS=1` writes whatever `image_snapshot` is handed, so a
/// regeneration of a window whose click missed the grid row would commit a
/// photograph of the dashboard under the grid view's name, and each later run
/// would agree with it. This settles the same window under the same script and
/// says what the frame holds: the grid on the canvas as one pane, and a full
/// band over the fixture's nine columns.
fn assert_grid_view_is_what_is_being_photographed(at: egui::Pos2) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = brightfield_shell::window::MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let screen =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1));
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
    for events in open_the_grid_view(at) {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
    let panes: Vec<&str> = app.canvas_panes().panes.iter().map(|p| p.name).collect();
    assert_eq!(
        panes,
        vec!["grid"],
        "the click at {at:?} did not put the grid on the canvas — the capture \
         below would photograph whatever is there instead"
    );
    let drawn = app
        .chart_doc()
        .grid_drawn()
        .cloned()
        .expect("the grid pane laid a table out");
    assert_eq!(
        drawn.columns, HOUSING_COLUMN_COUNT,
        "the grid is of a table with {} columns, not the {HOUSING_COLUMN_COUNT} \
         this pair is photographed over",
        drawn.columns
    );
    assert!(
        drawn.band.iter().all(|cell| cell.density
            == brightfield_shell::column_header::GridDensity::Full
            && cell.stats.is_some()
            && !cell.bars.is_empty()),
        "the band in the frame being photographed is not the full one: {:?}",
        drawn.band
    );
}

/// How many columns the housing fixture has.
const HOUSING_COLUMN_COUNT: usize = 9;

/// **The grid as the canvas's view of the table, as pixels** — the full column
/// header band over the file's nine columns, at 1440 by 900.
///
/// The companion to [`the_generated_dashboard_light_baseline`], which
/// photographs the same file with the grid as a quarter of the canvas and the
/// band at its compact density. Between them the pair pins both densities, and
/// each reddens when the rows its own density draws go missing.
#[test]
fn the_grid_view_light_baseline() {
    let at = grid_row_centre();
    assert_grid_view_is_what_is_being_photographed(at);
    assert_one_grid_per_frame(&open_the_grid_view(at));
    let image = capture_grid_view(Mode::Light, at, "grid_view_light");
    egui_kittest::image_snapshot(&image, "grid_view_light");
}

/// **The dark twin of [`the_grid_view_light_baseline`]** — the same frame, the
/// same script, the ink moved.
#[test]
fn the_grid_view_dark_baseline() {
    let at = grid_row_centre();
    assert_grid_view_is_what_is_being_photographed(at);
    assert_one_grid_per_frame(&open_the_grid_view(at));
    let image = capture_grid_view(Mode::Dark, at, "grid_view_dark");
    egui_kittest::image_snapshot(&image, "grid_view_dark");
}

/// [`housing`] at [`SHORT_WINDOW`] with the grid put on the canvas, read back
/// as pixels.
fn capture_grid_view(mode: Mode, at: egui::Pos2, name: &str) -> image::RgbaImage {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at(
        boot,
        mode,
        SCALE,
        SHORT_WINDOW,
        &out,
        open_the_grid_view(at),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

// ---------------------------------------------------------------------------
// The grid transposed
// ---------------------------------------------------------------------------

/// **The window the transposed pair is photographed in** — wide as the
/// untransposed baselines and tall enough for the nine rows [`housing`] has,
/// one per column of the file.
///
/// A transposed row does not compress past `MIN_ROW_HEIGHT`, so in a shorter
/// window the last rows stand below the pane's foot and the painter clips them
/// away. That is correct behaviour and the scroll exists for it, but a
/// baseline photographed there would be a picture of fewer rows offered as a
/// picture of the layout — and the rows that go missing are the ones whose
/// labels a regression would take out first, since they are the ones nothing
/// else is drawn beside. `canvas_pane_group.rs`'s `TRANSPOSED_SCREEN` is the
/// same window, so the labels the pair photographs are the labels that file
/// reads back as text.
///
/// 1344 and not the 1088 this held while the coordinate pair's two columns
/// earned no rows of their own: nine rows at `MIN_ROW_HEIGHT` need 1152 points
/// of pane content and the pane's content is the window less 164 points of
/// chrome.
const TRANSPOSED_WINDOW: (f32, f32) = (1440.0, 1344.0);

/// How many rows the transposed grid draws for [`housing`]: one per tile past
/// the hero, which is one per column of the file.
const TRANSPOSED_ROWS: usize = HOUSING_PLOTS.len() - 1;

/// **The script that throws the grid pane's layout switch**, aimed at the rect
/// a headless frame of the same window drew the control at — and asserted, in
/// that headless window, to have landed.
///
/// The aim cannot be typed: the switch stands at the trailing end of the grid
/// pane's header band, which moves with the pane, which moves with the split.
/// So it is read off a frame. The headless window lays out identically to the
/// captured one — everything but the raster is a pure function of the loaded
/// documents, which is the premise `tests/canvas_pane_group.rs` is built on —
/// so a rect read there is a rect the capture drew the control at.
///
/// The guard runs **here**, before the caller reaches `image_snapshot`, for
/// the reason this file's header gives: under `UPDATE_SNAPSHOTS=1` the
/// snapshot writes whatever it is handed, so a click that missed would commit
/// a golden of the untransposed grid under a transposed name and each later
/// run would agree with it. Three things are checked — the switch is in its
/// columns state, the pane drew one row per tile past the hero, and every one
/// of those rows is inside the clip rather than below the fold.
fn transposed_script(mode: Mode) -> Vec<Vec<egui::Event>> {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, mode);
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(TRANSPOSED_WINDOW.0, TRANSPOSED_WINDOW.1),
    );
    let frame = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    };
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }
    let at = app
        .chart_doc()
        .grid_layout_switch
        .as_ref()
        .expect("the grid pane's header band drew a layout switch")
        .states
        .iter()
        .find(|(state, _)| *state == brightfield_shell::app::GridLayout::Columns)
        .expect("the switch offers a columns state")
        .1
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    let script = vec![
        vec![egui::Event::PointerMoved(at), press(true)],
        vec![press(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    for events in script.clone() {
        frame(&mut app, events);
    }

    assert_eq!(
        app.grid_layout(),
        brightfield_shell::app::GridLayout::Columns,
        "the scripted click at {at:?} did not throw the layout switch, so the \
         capture below is a photograph of the untransposed grid"
    );
    let rows = &app.chart_doc().transposed_rows;
    assert_eq!(
        rows.len(),
        TRANSPOSED_ROWS,
        "the transposed grid drew {} rows where {} of {}'s tiles stand past the \
         hero",
        rows.len(),
        TRANSPOSED_ROWS,
        path.display()
    );
    for row in rows {
        assert!(
            row.clip.contains_rect(row.cell),
            "the row for {} was clipped to {:?} from a cell of {:?} — it is \
             below the fold at {TRANSPOSED_WINDOW:?}, so the picture is of \
             fewer rows than the layout has",
            row.name,
            row.clip,
            row.cell
        );
    }
    script
}

/// [`housing`] with the grid transposed, captured at [`TRANSPOSED_WINDOW`].
fn capture_transposed(mode: Mode, name: &str) -> image::RgbaImage {
    let script = transposed_script(mode);
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at(
        boot,
        mode,
        SCALE,
        TRANSPOSED_WINDOW,
        &out,
        script,
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// **The grid in its transposed layout, as pixels** — the hero pane, and
/// beside it one row per tiled column carrying that column's histogram, its
/// finetype leaf and its storage type.
///
/// The arrangement had no photograph at all: the committed
/// `dashboard_{light,dark}` pair pins the grid drawing the table's rows, and
/// `grid_view_{light,dark}` photograph the grid as the canvas's whole view,
/// which does not change with the switch. So a row's leaf and its storage
/// label could both go blank and every image in this repository would still
/// match. `canvas_pane_group.rs`'s
/// `every_transposed_row_states_its_leaf_and_its_storage_type` reads those two
/// strings back as text; this is the picture of them, and of everything beside
/// them that no assertion names.
#[test]
fn the_transposed_dashboard_light_baseline() {
    let image = capture_transposed(Mode::Light, "dashboard_transposed_light");
    egui_kittest::image_snapshot(&image, "dashboard_transposed_light");
}

/// **The same transposed grid in dark**, and the white-slab check its
/// untransposed twin makes.
///
/// The transposed layout draws the page through a second pane view, so it
/// reaches the dark composition by a path `the_generated_dashboard_dark_baseline`
/// does not: the tiles are re-homed into the grid pane at a row's height after
/// the mode is known. A light chart surface arriving through that seam is tens
/// of thousands of pixels of difference in a perceptual diff and unreadable as
/// a cause, so it is counted by name ahead of the photograph.
#[test]
fn the_transposed_dashboard_dark_baseline() {
    let image = capture_transposed(Mode::Dark, "dashboard_transposed_dark");

    let light = pixels_of(&image, meridian_design::chrome::INK_LIGHT.surface);
    assert_eq!(
        light, 0,
        "{light} pixels of this dark transposed grid are the LIGHT chart \
         surface (#fcfcfb). That colour has one source — the plot background — \
         so this window is drawing a white slab exactly where the analyst is \
         reading."
    );
    let dark = pixels_of(&image, meridian_design::chrome::INK_DARK.surface);
    assert!(
        dark > 0,
        "no pixel of this dark transposed grid is the dark chart surface \
         (#161413), so the plot background is neither of the two colours it can \
         be and the assertion above is passing for the wrong reason"
    );

    egui_kittest::image_snapshot(&image, "dashboard_transposed_dark");
}

// ---------------------------------------------------------------------------
// One grid, in either of its spots
// ---------------------------------------------------------------------------

/// **One table filed per frame, across both of the grid's spots** — the guard
/// the dashboard, grid-view and ledger baselines run ahead of their
/// photographs.
///
/// A photograph cannot see a second grid in a rail it shows collapsed, and the
/// dashboard and grid-view frames show the ledger collapsed. So the guard
/// settles [`housing`] under the photograph's own `script`, counts the tables
/// that frame filed, then moves the grid between the canvas and the ledger and
/// back — from whichever spot the script left it in, so the ledger pair's
/// first move takes the grid out rather than in — and replays the script,
/// counting at each stop. A frame that draws the grid in both spots files two
/// tables and fails here, before any picture is taken.
///
/// Watched redden, two mutations: the ledger drawing the grid whatever the
/// canvas draws, and the canvas drawing it whatever the grid's spot says.
fn assert_one_grid_per_frame(script: &[Vec<egui::Event>]) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let screen =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1));
    let run = |app: &mut MeridianApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    };
    let settle = |app: &mut MeridianApp, script: &[Vec<egui::Event>]| {
        for events in script
            .iter()
            .cloned()
            .chain(std::iter::repeat_n(Vec::new(), 3))
        {
            run(app, events);
        }
    };
    let filed = |app: &MeridianApp, when: &str| {
        let n = app.chart_doc().tables_filed();
        assert_eq!(
            n, 1,
            "{when}, the frame filed {n} tables — the grid drew in more than one spot"
        );
    };
    settle(&mut app, script);
    filed(&app, "at the photographed state");
    app.move_grid();
    settle(&mut app, &[]);
    filed(
        &app,
        "with the grid moved between the canvas and the ledger, from whichever \
         spot the script left it in",
    );
    app.move_grid();
    settle(&mut app, script);
    filed(
        &app,
        "with the grid moved back to the spot the script left it in",
    );
}

/// Where the ledger strip draws its Rows name in [`housing`] at
/// [`SHORT_WINDOW`] — the name a reader clicks to send the grid there.
fn rows_name_centre() -> egui::Pos2 {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    // Log, Quality, Rows, Editor.
    app.rail_name_rect(brightfield_workbench::arrangement::LEDGER_RAIL, 2)
        .expect("the ledger strip drew its Rows name")
        .center()
}

/// The frames that send the grid to the ledger: a click on the Rows name, then
/// three to settle — the same click [`open_the_grid_view`] makes, aimed at the
/// strip.
fn send_the_grid_to_the_ledger(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    open_the_grid_view(at)
}

/// The structural guard the ledger pair runs first: the frame being
/// photographed holds the grid in the ledger — its table's header inside the
/// rail — and the hero alone on the canvas.
fn assert_grid_in_ledger_is_what_is_being_photographed(at: egui::Pos2) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let screen =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1));
    for events in std::iter::repeat_n(Vec::new(), 3).chain(send_the_grid_to_the_ledger(at)) {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
    let panes: Vec<&str> = app.canvas_panes().panes.iter().map(|p| p.name).collect();
    assert_eq!(
        panes,
        vec!["map"],
        "the click at {at:?} did not leave the hero alone on the canvas"
    );
    let ledger = app
        .region_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
        .expect("the ledger drew");
    let head = app
        .chart_doc()
        .grid_drawn()
        .and_then(|drawn| drawn.header_cells.first().map(|cell| cell.1))
        .expect("the grid laid a table out");
    assert!(
        ledger.contains_rect(head.shrink(1.0)),
        "the grid's header drew at {head:?}, outside the ledger {ledger:?}"
    );
}

/// [`housing`] at [`SHORT_WINDOW`] with the grid sent to the ledger, read back
/// as pixels.
fn capture_grid_in_ledger(mode: Mode, at: egui::Pos2, name: &str) -> image::RgbaImage {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at(
        boot,
        mode,
        SCALE,
        SHORT_WINDOW,
        &out,
        send_the_grid_to_the_ledger(at),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// **The grid in the ledger, as pixels** — the hero alone on the canvas, and
/// the table under the ledger strip's Rows name with its own header band and
/// the spot switch reading *ledger*.
///
/// The companion to [`the_generated_dashboard_light_baseline`], which
/// photographs the same file with the grid beside the hero: between them the
/// pair pins both of the grid's spots.
#[test]
fn the_grid_in_ledger_light_baseline() {
    let at = rows_name_centre();
    assert_grid_in_ledger_is_what_is_being_photographed(at);
    assert_one_grid_per_frame(&send_the_grid_to_the_ledger(at));
    let image = capture_grid_in_ledger(Mode::Light, at, "grid_in_ledger_light");
    egui_kittest::image_snapshot(&image, "grid_in_ledger_light");
}

/// **The dark twin of [`the_grid_in_ledger_light_baseline`]** — the same frame,
/// the same script, the ink moved.
#[test]
fn the_grid_in_ledger_dark_baseline() {
    let at = rows_name_centre();
    assert_grid_in_ledger_is_what_is_being_photographed(at);
    assert_one_grid_per_frame(&send_the_grid_to_the_ledger(at));
    let image = capture_grid_in_ledger(Mode::Dark, at, "grid_in_ledger_dark");
    egui_kittest::image_snapshot(&image, "grid_in_ledger_dark");
}

// ---------------------------------------------------------------------------
// The grid in the ledger, transposed
// ---------------------------------------------------------------------------

/// [`housing`] opened by the args route with `grid_layout` and `grid_spot`
/// already saved for it, the way a relaunch over a document last left in the
/// ledger transposed opens — no click script needed, unlike
/// [`send_the_grid_to_the_ledger`], because the boot itself carries the state.
fn boot_over_saved_layout(
    grid_layout: GridLayout,
    grid_spot: GridSpot,
) -> (Boot, brightfield_workbench::SavedLayout) {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path").to_string();
    let mut layout = brightfield_shell::startup::default_layout();
    layout.remember(
        &chosen,
        "Housing",
        RunState::NeverRun,
        grid_layout,
        grid_spot,
        1_000,
    );
    let boot = Boot::open_sampled(&chosen, Flow::Vertical, None, None)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    (boot, layout)
}

/// The structural guard the columns-in-ledger pair runs first: the frame
/// being photographed holds the hero alone on the canvas and, in the ledger,
/// one row per tiled column rather than the table [`assert_grid_in_ledger_is_what_is_being_photographed`]
/// pins for a document saved on its rows.
fn assert_grid_in_ledger_columns_is_what_is_being_photographed() {
    let (boot, layout) = boot_over_saved_layout(GridLayout::Columns, GridSpot::Ledger);
    let mut app = MeridianApp::headless_with_layout(boot, layout, Mode::Light);
    let ctx = egui::Context::default();
    let screen =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SHORT_WINDOW.0, SHORT_WINDOW.1));
    // More than the three frames a click settles over: the ledger draws
    // before the canvas each frame and reads the canvas's own `pane_views`
    // from the frame before, so the first frame this boot's saved state is
    // live the numbers still lag the picture by one settle.
    for _ in 0..6 {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
    let panes: Vec<&str> = app.canvas_panes().panes.iter().map(|p| p.name).collect();
    assert_eq!(
        panes,
        vec!["map"],
        "a document saved with the grid in the ledger did not leave the hero \
         alone on the canvas"
    );
    let rows = &app.chart_doc().transposed_rows;
    assert_eq!(
        rows.len(),
        TRANSPOSED_ROWS,
        "the ledger's grid pane drew {} rows where {TRANSPOSED_ROWS} of \
         housing's tiles stand past the hero — a document saved with its \
         layout columns left the untransposed table drawn in the ledger \
         instead, or drew nothing",
        rows.len()
    );
    assert_eq!(
        app.chart_doc().tables_filed(),
        0,
        "the frame filed a table — the ledger drew the rows of a document saved \
         with its layout columns"
    );
    let ledger = app
        .region_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
        .expect("the ledger drew");
    // Every row is painted under the ledger's own clip, and the first stands
    // inside it. The rest are not asserted to fit: the page is composed at
    // the hero's height, so the tiles share that height rather than standing
    // at the canvas layout's `MIN_ROW_HEIGHT` floor, the ledger opens at its
    // default height, and no scroll reaches the rows below its foot. The
    // picture below is of the rows that height holds.
    for row in rows {
        assert!(
            ledger.contains_rect(row.clip),
            "the row for {} drew under a clip of {:?}, outside the ledger {ledger:?}",
            row.name,
            row.clip
        );
    }
    let first = rows.first().expect("a row");
    assert!(
        ledger.contains_rect(first.cell),
        "the first row, for {}, drew at {:?}, outside the ledger {ledger:?}",
        first.name,
        first.cell
    );
}

/// [`housing`] with the grid in the ledger and its saved layout columns, read
/// back as pixels.
fn capture_grid_in_ledger_columns(mode: Mode, name: &str) -> image::RgbaImage {
    let (boot, layout) = boot_over_saved_layout(GridLayout::Columns, GridSpot::Ledger);
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = scratch(name);
    let (w, h) = brightfield_shell::capture::capture_png_at_with_layout(
        boot,
        layout,
        mode,
        SCALE,
        SHORT_WINDOW,
        &out,
        Vec::new(),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// **The grid in the ledger, transposed, as pixels** — the hero alone on the
/// canvas, and beside it in the ledger one row per tiled column carrying its
/// histogram and its summaries: [`the_grid_in_ledger_light_baseline`]'s
/// columns twin.
///
/// The companion to [`the_transposed_dashboard_light_baseline`], which
/// photographs the same saved layout on the canvas: between them the pair
/// pins the columns layout in both of the grid's spots, the way
/// [`the_grid_in_ledger_light_baseline`] and
/// [`the_generated_dashboard_light_baseline`] pin the rows layout in both.
#[test]
fn the_grid_in_ledger_columns_light_baseline() {
    assert_grid_in_ledger_columns_is_what_is_being_photographed();
    let image = capture_grid_in_ledger_columns(Mode::Light, "grid_in_ledger_columns_light");
    egui_kittest::image_snapshot(&image, "grid_in_ledger_columns_light");
}

/// **The dark twin of [`the_grid_in_ledger_columns_light_baseline`]** — the
/// same frame, the same saved layout, the ink moved.
#[test]
fn the_grid_in_ledger_columns_dark_baseline() {
    assert_grid_in_ledger_columns_is_what_is_being_photographed();
    let image = capture_grid_in_ledger_columns(Mode::Dark, "grid_in_ledger_columns_dark");
    egui_kittest::image_snapshot(&image, "grid_in_ledger_columns_dark");
}
