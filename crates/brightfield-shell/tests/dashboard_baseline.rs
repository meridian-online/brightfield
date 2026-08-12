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
//! renders this same picture through this same code today. What is photographed
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

use brightfield_shell::capture::capture_png;
use brightfield_shell::dashboard::{self, ChosenBy, Dashboard, Omission};
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_shell::{chart_kinds, data_file, ranked_bars};
use brightfield_workbench::registry::ChartKindId;

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
    let opened =
        data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
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
/// is the crate's own headless path. The dashboard that chose the tiles is
/// asked for separately, because a `Boot` carries the composed document rather
/// than the walk that produced it.
///
/// Light only, deliberately. The choices are mode-independent — they are read
/// off column types, not off ink — so a dark twin would cost a second GPU
/// capture to re-photograph the same decision. Covered here: the composed
/// picture in light. Not covered: this dashboard in dark.
#[test]
fn the_generated_dashboard_light_baseline() {
    let path = fixture();
    let chosen = path.to_str().expect("utf-8 fixture path");

    // The structural guard, ahead of the photograph, for the reason in this
    // file's header: `UPDATE_SNAPSHOTS=1` writes whatever `image_snapshot` is
    // handed.
    let opened =
        data_file::open(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_choices(&opened.dashboard);
    assert_eq!(
        opened.composed.plots.len(),
        EXPECTED.len(),
        "the walk chose {} tiles and the composition placed {} plots, so the \
         image below is not a picture of those choices",
        EXPECTED.len(),
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

    // PNG is lossless, so reading the capture back is pixel-exact; the file is
    // only how `capture_png` hands its result over.
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, "dashboard_light");
}
