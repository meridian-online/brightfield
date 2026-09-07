//! **No two texts land in the same pixels**, read off the frames the real
//! shell paints.
//!
//! The defect this file exists for is the one a reader meets first: two labels
//! drawn into one place, so that `updated` and `TIMESTAMP WITH TIME ZONE`
//! arrive as `updateTIMESTAMP WITH TIME…`. Before this file nothing in this
//! repository could fail because of it — each occurrence was found by opening
//! a file and looking, which does not reach the panes nobody thought to open
//! and does not cover the pane written next.
//!
//! # Why the check is here and not at each drawing site
//!
//! A per-site assertion is written by whoever already knows their site
//! collides. These tests boot the window, let it draw whatever it draws, and
//! ask [`brightfield_shell::text_ink`] whether any two galleys share pixels —
//! so a pane added tomorrow is covered by a file nobody edits.
//!
//! # What is exempt
//!
//! Not here. A reason two galleys may share a box is a row of
//! `text_ink::EXEMPTIONS` with a sentence saying why, and adding one is an
//! edit to that table — `every_exemption_excuses_a_case_and_no_other` is what
//! keeps a row in it that decides nothing. These tests pass the whole window in and assert the
//! list comes back empty.

use brightfield_shell::design::Mode;
use brightfield_shell::protocol::NodeView;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp};

/// The window the shell's contracts are measured in.
const SCREEN: (f32, f32) = (1440.0, 900.0);

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click resolves against the widget id a previous frame registered.
///
/// The same shape as `tests/column_header_band.rs`'s harness, with one thing
/// added: [`Self::survey`] reads the pass **from inside the frame closure**.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

/// What one pass painted, and what collided in it.
struct Survey {
    /// Every galley, read layer by layer while the pass was open.
    texts: Vec<text_ink::DrawnText>,
    /// The failure message, or `None` where no pair collided.
    report: Option<String>,
    /// How many galleys the flattened `FullOutput` carried — the same pass,
    /// counted the other way. See `the_check_reads_every_galley_the_pass_painted`.
    flattened: usize,
}

impl Live {
    fn open(name: &str) -> Self {
        let path = fixture(name);
        let chosen = path.to_str().expect("utf-8 fixture path");
        let boot =
            Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let mut live = Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SCREEN.0, SCREEN.1)),
        };
        live.run(vec![Vec::new(), Vec::new(), Vec::new()]);
        live
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    /// One more frame, read while it is still open.
    ///
    /// **`text_ink::frame_text` is called inside the closure and that is not a
    /// detail.** egui flattens its paint lists when the pass ends and the
    /// layer each shape came from is not in the flattened list, so a caller
    /// reading `FullOutput::shapes` cannot tell a tooltip's text from the text
    /// underneath it — it would have to excuse by name what a layer already
    /// says.
    fn survey(&mut self, what: &str) -> Survey {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let inside = self.ctx.clone();
        let mut texts = Vec::new();
        let mut report = None;
        let out = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(&inside);
            report = text_ink::collision_report(&inside, what);
        });
        Survey {
            texts,
            report,
            flattened: text_ink::flattened_text(&out.shapes).len(),
        }
    }

    /// Click where the last frame drew the rail's row labelled `label` — the
    /// gesture that puts a node's view on the canvas.
    fn click_row(&mut self, label: &str) {
        let rows = self.app.spine_rows().to_vec();
        let row = rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| {
                let drawn: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
                panic!("the rail drew no row labelled {label:?}; it drew {drawn:?}")
            });
        let at = row.rect.center();
        let mut events = vec![egui::Event::PointerMoved(at)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        self.run(vec![events, Vec::new(), Vec::new()]);
    }
}

/// Every window state this check is driven over, named for the failure
/// message.
///
/// Two fixtures and two densities. `site_readings_sample.csv` is the narrow
/// case: a timestamp column, so the rail draws a long type name beside a
/// column name, and readings whose bounds are wide against a narrow column.
fn states() -> Vec<(&'static str, fn() -> Live)> {
    vec![
        ("the housing table, as opened", || {
            Live::open("california_housing_sample.csv")
        }),
        ("the housing table, grid on the canvas", || {
            let mut live = Live::open("california_housing_sample.csv");
            live.click_row("grid");
            live
        }),
        ("the site readings, as opened", || {
            Live::open("site_readings_sample.csv")
        }),
        ("the site readings, grid on the canvas", || {
            let mut live = Live::open("site_readings_sample.csv");
            live.click_row("grid");
            live
        }),
        ("the wide-label table, as opened", || {
            Live::open("wide_labels_sample.csv")
        }),
        ("the wide-label table, grid on the canvas", || {
            let mut live = Live::open("wide_labels_sample.csv");
            live.click_row("grid");
            live
        }),
    ]
}

/// **No two texts the shell paints land in the same pixels.**
///
/// The card's AC1, over each window state above, each layer and each galley.
/// The judgement about what is not a defect lives in `text_ink::EXEMPTIONS`
/// and `every_exemption_excuses_a_case_and_no_other` drives it, so there is
/// no tolerance here to widen and no predicate here to soften.
#[test]
fn no_two_texts_are_drawn_into_one_place() {
    let mut failures = Vec::new();
    for (what, open) in states() {
        let mut live = open();
        if let Some(report) = live.survey(what).report {
            failures.push(report);
        }
    }
    assert!(
        failures.is_empty(),
        "{}\n\nEach line is two galleys and the box they share. A pair that is \
         not a defect belongs in text_ink::EXEMPTIONS with a sentence saying \
         why, not in a tolerance.",
        failures.join("\n")
    );
}

/// **The check reads every galley the pass painted.**
///
/// The self-test the check needs and cannot do for itself. `frame_text` walks
/// the layers it can name, and it names them through `Memory::layer_ids` —
/// a layer missing from that list is a pane this check would silently pass
/// over, and silence is what a passing check looks like too.
///
/// So the same pass is counted the other way: `FullOutput::shapes` is egui's
/// own flattening, drained by the framework rather than enumerated by us. The
/// two counts have no common cause beyond the pass itself.
#[test]
fn the_check_reads_every_galley_the_pass_painted() {
    for (what, open) in states() {
        let mut live = open();
        let survey = live.survey(what);
        assert!(
            survey.flattened > 0,
            "{what}: the pass painted no text at all, so this test would pass \
             over a window that draws nothing"
        );
        assert_eq!(
            survey.texts.len(),
            survey.flattened,
            "{what}: walking the layers found {} galleys and egui's own \
             flattening carried {} — a layer this check never looked at",
            survey.texts.len(),
            survey.flattened,
        );
    }
}

// ---------------------------------------------------------------------------
// The two live defects, read off the rects the painter returned.
// ---------------------------------------------------------------------------

/// Every row of the rail that carries a name and a type, as the frame drew
/// them.
fn rail_rows(live: &Live) -> Vec<(String, egui::Rect, egui::Rect)> {
    live.app
        .spine_rows()
        .iter()
        .filter_map(|row| {
            row.kind_rect
                .map(|kind| (row.label.clone(), row.name_rect, kind))
        })
        .collect()
}

/// **The rail's name and the type beside it do not touch, at the width this
/// rail has.**
///
/// The card's AC2, half of it. Read off `SpineRowDrawn::name_rect` and
/// `kind_rect` — the boxes the painter handed back — rather than off a
/// screenshot or off the layout constants the drawing used, because a rect
/// recomputed from the constants agrees with a wrong drawing.
///
/// The fixture is the point: `updated` is a `TIMESTAMP WITH TIME ZONE`, which
/// is the longest type name DuckDB hands this rail and the one the two
/// budgeted-character calls this replaces could not fit.
#[test]
fn the_rails_name_and_type_stay_apart_at_this_width() {
    let live = Live::open("wide_labels_sample.csv");
    let rows = rail_rows(&live);
    assert!(
        rows.iter().any(|(label, ..)| label == "updated"),
        "the rail drew no row for the timestamp column, so this test is not \
         driving the case it was written for: {:?}",
        rows.iter().map(|(l, ..)| l).collect::<Vec<_>>()
    );
    for (label, name, kind) in &rows {
        assert!(
            !name.is_negative(),
            "{label}: the rail had no room for a name at all"
        );
        assert!(
            name.right() <= kind.left(),
            "{label}: the name ends at {} and the type begins at {} — {} \
             points of the two are in the same place",
            name.right(),
            kind.left(),
            name.right() - kind.left(),
        );
    }
}

/// **The band's lower and upper bound do not touch, at the width the compact
/// density gives them.**
///
/// The card's AC2, the other half, read off `ColumnBandDrawn::range_rects`.
/// Driven over both fixtures because the two produce different shapes of
/// collision: dates as wide as the column on one, signed seven-figure decimals
/// on the other.
#[test]
fn the_bands_two_bounds_stay_apart_at_this_width() {
    for fixture in ["site_readings_sample.csv", "wide_labels_sample.csv"] {
        for grid in [false, true] {
            let mut live = Live::open(fixture);
            if grid {
                live.click_row("grid");
            }
            let drawn = live
                .app
                .chart_doc()
                .grid_drawn
                .clone()
                .expect("the grid pane laid a table out");
            let mut ranges = 0;
            for cell in &drawn.band {
                let Some((lo, hi)) = cell.range_rects else {
                    continue;
                };
                ranges += 1;
                assert!(
                    !lo.is_negative(),
                    "{fixture} {}: the cell had no room for a lower bound at all",
                    cell.name
                );
                assert!(
                    lo.right() <= hi.left(),
                    "{fixture} {} (grid on canvas: {grid}): the lower bound \
                     ends at {} and the upper begins at {} — {} points of the \
                     two are in the same place",
                    cell.name,
                    lo.right(),
                    hi.left(),
                    lo.right() - hi.left(),
                );
            }
            assert!(
                ranges > 0,
                "{fixture} (grid on canvas: {grid}): no cell drew a range, so \
                 this test passed over a band with nothing in it"
            );
        }
    }
}

/// **The rows stack to the height the frame claims.**
///
/// `ColumnHeaderFrame::extent` states the sum; `ColumnBandDrawn::stacked` is
/// what the drawing reached. They are two derivations of one number, and the
/// distinct row painting without advancing past itself is the shape that puts
/// them out of step — a block that draws a row and leaves the cursor where it
/// was, so the row after it lands on top.
#[test]
fn the_rows_stack_to_the_extent_the_frame_claims() {
    for grid in [false, true] {
        let mut live = Live::open("wide_labels_sample.csv");
        if grid {
            live.click_row("grid");
        }
        let drawn = live
            .app
            .chart_doc()
            .grid_drawn
            .clone()
            .expect("the grid pane laid a table out");
        assert!(!drawn.band.is_empty(), "the pane drew no band");
        for cell in &drawn.band {
            // The extent carries the two insets; `stacked` is measured from
            // the top of the content box, so it is the extent less both.
            let want = cell.extent - 2.0 * brightfield_shell::column_header::INSET_Y;
            assert!(
                (cell.stacked - want).abs() < 0.01,
                "{} (grid on canvas: {grid}): the rows stacked to {} and the \
                 frame claims {want} — a block painted a row and left the \
                 cursor where it was",
                cell.name,
                cell.stacked,
            );
        }
    }
}
