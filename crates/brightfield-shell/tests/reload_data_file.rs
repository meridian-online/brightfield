//! The data-changed banner's reload action: it re-reads the file, falls back
//! to the view exactly as a fresh open would once the file no longer fits the
//! copy threshold, and refuses to race a remote fetch that is still landing.
//!
//! `MeridianApp::reload_data_file` is deliberately a re-run of the ordinary
//! open on the same path rather than a second, patched-in-place mechanism —
//! see its doc comment in `window.rs` for why a materialised source cannot be
//! safely re-copied onto itself. So the tests here compare what reload
//! produces against what [`data_file::open_traced`] produces on the same
//! file, the same way `open_materialise.rs` compares its own two branches,
//! rather than asserting on any one tile's exact shape.

use std::fs;
use std::path::{Path, PathBuf};

use arrow::util::pretty::pretty_format_batches;
use brightfield_engine::coordinator::Coordinator;
use brightfield_shell::data_file::{self, OpenOptions, MATERIALISE_UNDER_BYTES};
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

/// A window under test, and one `egui::Context` for its whole life — the
/// shape `status_rail.rs` and `remote_start.rs` both use.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open(boot: Boot) -> Self {
        Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    fn settle(&mut self) {
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }
}

/// Every mark's result set, pretty-printed and concatenated in mark order —
/// the same device `open_materialise.rs::drawn_rows` reads a picture through,
/// because a difference has to be readable rather than a diff of two vectors
/// of Arrow batches.
fn marks_text(coordinator: &mut Coordinator) -> String {
    let marks = coordinator.session().mark_count();
    let mut out = String::new();
    for index in 0..marks {
        let batches = coordinator
            .session_mut()
            .execute_mark(index)
            .unwrap_or_else(|e| panic!("mark {index}: {e}"));
        out.push_str(
            &pretty_format_batches(&batches)
                .unwrap_or_else(|e| panic!("mark {index}: {e}"))
                .to_string(),
        );
    }
    out
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("bf-reload-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// AC1 — the reload reads the edit
// ---------------------------------------------------------------------------

/// A small fixture with a bounded measure, a wide measure, a label category
/// and a timestamp — the same shape `open_materialise.rs::fixture` uses,
/// which is committed there as producing a histogram, ranked bars and time
/// bars. `marker`, when given, is one extra row with that label, standing in
/// for an edit made beside the running app.
fn write_dashboard_fixture(path: &Path, rows: u64, marker: Option<&str>) {
    let mut buf = String::from("bounded,wide,label,at\n");
    for i in 0..rows {
        let bounded = (i * 11 + 7) % 12;
        let wide = f64::from(u32::try_from((i * 17 + 3) % 1000).expect("bounded above")) / 10.0;
        let label = ["alpha", "beta", "gamma"][(i % 3) as usize];
        let secs = i % 60;
        buf.push_str(&format!(
            "{bounded}.0,{wide},{label},2020-01-01 00:00:{secs:02}\n"
        ));
    }
    if let Some(marker) = marker {
        buf.push_str(&format!("5.0,42.0,{marker},2020-01-01 00:05:00\n"));
    }
    fs::write(path, buf).expect("write fixture");
}

/// **Taking the reload action re-reads the file, and lands exactly where a
/// fresh open of the edited file would.**
///
/// The fixture is rewritten between the first open and the reload — the
/// stand-in for an analyst editing the CSV beside the app — and the test
/// reads the picture through a real mark both times, comparing three points:
/// the reload changed what marks draw at all (it is not silently serving the
/// same rows), and what it lands on is identical to a brand new
/// `data_file::open` of the same, now-edited, file.
#[test]
fn reloading_reads_the_edit_on_disk_like_a_fresh_open_would() {
    let dir = scratch("ac1");
    let path = dir.join("rows.csv");
    write_dashboard_fixture(&path, 40, None);
    let chosen = path.to_string_lossy().into_owned();

    let mut w = Window::open(Boot::data_file(&chosen).expect("the fixture opens"));
    w.settle();
    let before = marks_text(
        w.app
            .chart_doc_mut()
            .live_coordinator()
            .expect("a data-file open leaves a live session"),
    );

    // The edit: rewritten with one row this fixture never had before.
    write_dashboard_fixture(&path, 40, Some("zzz_marker_row"));

    let ctx = w.ctx.clone();
    w.app.reload_data_file(&ctx);
    w.settle();
    let after_reload = marks_text(
        w.app
            .chart_doc_mut()
            .live_coordinator()
            .expect("the reload leaves a live session too"),
    );

    assert_ne!(
        before, after_reload,
        "every mark drew the same rows after the reload as before the edit — \
         the reload did not read the file again"
    );

    let (mut fresh, _trace) = data_file::open_traced(&chosen, &OpenOptions::default())
        .expect("a fresh open of the edited file");
    let via_fresh_open = marks_text(fresh.live.coordinator());

    assert_eq!(
        after_reload, via_fresh_open,
        "reload's marks differ from a fresh open of the same edited file — \
         reload did not land where an ordinary open of this file lands"
    );
}

// ---------------------------------------------------------------------------
// AC2 — an edit that crosses the copy threshold falls back to the view
// ---------------------------------------------------------------------------

/// A 200-character label padded with `y`, cycling through seven values — the
/// same shape `open_materialise.rs::sized_csv` writes, so a row is a fixed
/// 205 bytes and a row count converts to a byte count by arithmetic rather
/// than a stat.
fn padded_label(i: u64) -> String {
    let core = format!("x{}", i % 7);
    format!("{}{core}", "y".repeat(200 - core.len()))
}

fn write_sized_csv(path: &Path, rows: u64) {
    let mut buf = String::with_capacity(12 + 205 * usize::try_from(rows).unwrap_or(usize::MAX));
    buf.push_str("label,value\n");
    for i in 0..rows {
        buf.push_str(&format!("{},{:03}\n", padded_label(i), i % 97));
    }
    fs::write(path, buf).expect("write fixture");
}

/// **An edit that pushes the file over the copy threshold falls back to the
/// view on reload, exactly as an ordinary open of that file would.**
///
/// The file opens small enough to be copied, is then rewritten larger than
/// [`MATERIALISE_UNDER_BYTES`] on disk — over the line `data_file::open`
/// declines to even attempt a copy past — and reload is taken. The test
/// reads what reload landed on against a direct `data_file::open_traced` of
/// the same grown file for equality, and — the vacuity guard — confirms that
/// direct open really did take the un-materialised branch, so the equality
/// above is evidence about the fallback and not about two routes agreeing on
/// nothing in particular.
#[test]
fn an_edit_over_the_copy_threshold_falls_back_to_the_view_on_reload() {
    let dir = scratch("ac2");
    let path = dir.join("data.csv");
    let over_rows = MATERIALISE_UNDER_BYTES.div_ceil(205) + 2_000;
    let under_rows = over_rows / 20;
    write_sized_csv(&path, under_rows);
    let chosen = path.to_string_lossy().into_owned();

    let mut w = Window::open(Boot::data_file(&chosen).expect("the small fixture opens"));
    w.settle();
    assert!(
        !w.app.chart_doc().composed.plots.is_empty(),
        "the small fixture drew nothing, so this test has no picture to lose \
         once the file grows"
    );

    // The edit: rewritten large enough to cross the on-disk threshold.
    write_sized_csv(&path, over_rows);
    let grown = fs::metadata(&path).expect("stat the grown fixture").len();
    assert!(
        grown > MATERIALISE_UNDER_BYTES,
        "the grown fixture is {grown} bytes, not over the \
         {MATERIALISE_UNDER_BYTES}-byte threshold this test means to cross"
    );

    let ctx = w.ctx.clone();
    w.app.reload_data_file(&ctx);
    w.settle();
    assert!(
        !w.app.chart_doc().composed.plots.is_empty(),
        "the reload of the grown file drew no picture — it did not fall \
         back to the view, it failed"
    );
    assert!(
        w.app.chart_doc().composed.mark_faults.is_empty(),
        "the reload of the grown file reported mark faults: {:?}",
        w.app.chart_doc().composed.mark_faults
    );
    let via_reload = marks_text(
        w.app
            .chart_doc_mut()
            .live_coordinator()
            .expect("the reload leaves a live session"),
    );

    let (mut fresh, trace) = data_file::open_traced(&chosen, &OpenOptions::default())
        .expect("a fresh open of the grown file");
    assert!(
        !trace.materialised,
        "a fresh open of the grown file was still materialised, so it is not \
         over the threshold this test relies on and proves nothing about the \
         fallback branch"
    );
    let via_fresh_open = marks_text(fresh.live.coordinator());

    assert_eq!(
        via_reload, via_fresh_open,
        "reload's marks differ from a fresh open of the same grown file — \
         reload did not fall back to the view the way a fresh open does"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a reload does not race an outstanding remote fetch
// ---------------------------------------------------------------------------

/// A spec naming one remote `file:` source — `.invalid` per RFC 2606, so it
/// can never resolve. `MeridianApp::open_remote_start` only reads this string
/// to decide whether a fetch is fetchable and to latch the pending state; it
/// does not connect until a second frame is drawn, which this test never
/// draws — see its doc comment on `window.rs`.
fn spec_naming_a_remote_source(url: &str) -> String {
    format!(
        "data:\n  t:\n    file: \"{url}\"\nplot:\n  - mark: rectY\n    \
         data: {{ from: t }}\n    x: {{ bin: v }}\n    y: {{ count: }}\n\
         width: 640\nheight: 400\n"
    )
}

/// **A reload does not run while a remote start's own fetch is still
/// outstanding.**
///
/// `open_remote_start` latches [`MeridianApp::fetching_start`] in the same
/// frame, before any worker exists, which is what lets this test drive the
/// overlap without a network connection: the fetch is outstanding and
/// nothing has happened yet, exactly the window `reload_data_file`'s guard
/// exists for. It does not touch the chart document at all, so a real
/// data-file document is opened first and its marks are read back
/// unchanged after the refused reload — a window with nothing open would
/// let a *missing* guard hide behind reload's separate "nothing to reload"
/// no-op instead of behind the fetch guard this test means to pin.
#[test]
fn reload_refuses_while_a_remote_fetch_is_outstanding() {
    let dir = scratch("ac3");
    let path = dir.join("rows.csv");
    write_dashboard_fixture(&path, 40, None);
    let chosen = path.to_string_lossy().into_owned();

    let mut w = Window::open(Boot::data_file(&chosen).expect("the fixture opens"));
    w.settle();
    let before = marks_text(
        w.app
            .chart_doc_mut()
            .live_coordinator()
            .expect("a data-file open leaves a live session"),
    );

    let ctx = w.ctx.clone();
    let latched = w.app.open_remote_start(
        &ctx,
        "reload-guard-probe",
        &spec_naming_a_remote_source("https://example.invalid/data.csv"),
    );
    assert!(
        latched,
        "the spec named no fetchable source, so no fetch was latched and \
         this test is about nothing"
    );
    assert!(
        w.app.fetching_start().is_some(),
        "the fetch is not outstanding, so there is nothing here for reload \
         to refuse"
    );

    w.app.reload_data_file(&ctx);

    assert!(
        w.app.fetching_start().is_some(),
        "reload cleared the outstanding fetch instead of refusing to race it"
    );
    let after = marks_text(
        w.app
            .chart_doc_mut()
            .live_coordinator()
            .expect("reload must not have replaced the document"),
    );
    assert_eq!(
        before, after,
        "reload changed the open document's marks while a remote fetch was \
         still outstanding — it raced the fetch instead of refusing"
    );
}
