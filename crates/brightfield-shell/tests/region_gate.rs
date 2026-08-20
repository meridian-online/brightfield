//! The region-completeness gate: nothing the shell draws as a region of the
//! window may be missing from the arrangement that declares them.
//!
//! Two layers, and they fail for different reasons on purpose. The sibling
//! `arrangement.rs` asks whether the regions that *are* declared drew at the
//! extents they declare; this file asks whether the declaration is the whole
//! list.
//!
//! - **By source scan**, in the shape of `gallery_gate.rs`'s completeness
//!   grep: every edge panel this crate constructs is addressed by
//!   `window::panel_id`, whose argument is a `Region` — so a panel drawn for
//!   something the arrangement has never heard of has no id to draw under.
//!   The scan also refuses the constructions it cannot read, rather than
//!   passing over them: an older panel spelling, an aliased import, or a
//!   hand-typed id literal.
//! - **By running the window**, which is the layer that does not depend on
//!   how the source happens to be written: the regions the arrangement
//!   declares are laid out and their drawn rects are measured against the
//!   window they were laid out in. They tile it exactly — no gap and no
//!   overlap — so a panel added anywhere, by any means, under any id, takes
//!   its extent out of that sum and this reddens.
//!
//! **What neither layer covers, and why that is not a hole.** A
//! `CentralPanel` carries no id, so the scan has nothing to read from it; and
//! it is the remainder by construction, so a second one does not steal space
//! and the runtime layer would not see it either. That case is covered
//! upstream instead: `audit_arrangement` refuses an arrangement whose count
//! of `Extent::Remainder` regions is anything but one, and
//! `the_shipped_arrangement_declares_one_canvas_and_no_duplicate_region`
//! counts it. A `CentralPanel` cannot be a region the arrangement lacks —
//! only the one it already has.

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::{self, RegionId};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Completeness by source scan
// ---------------------------------------------------------------------------

/// The four edge-panel constructors. Each takes an id, and that id is what
/// says which region is being drawn.
const EDGE_PANELS: &[&str] = &[
    "Panel::top(",
    "Panel::bottom(",
    "Panel::left(",
    "Panel::right(",
];

/// The one call that may supply an edge panel's id.
const ID_CALL: &str = "panel_id(";

/// Panel spellings this scan cannot read. egui carries them as the older
/// names for the same containers; a region drawn through one would be a
/// region drawn past this gate, so they are refused by name rather than
/// passed over in silence.
const UNREADABLE_PANELS: &[&str] = &["SidePanel", "TopBottomPanel"];

/// Calls whose argument is an extent or an inset. A numeric literal in one is
/// a measure that the arrangement and the token set do not know about.
///
/// Matched only where the call is reached through a `.`, which is what
/// separates a builder's `.exact_size(` from `Ui::allocate_exact_size(` — the
/// second is a widget reserving a box of its own and has no region behind it.
const MEASURE_CALLS: &[&str] = &[
    "exact_size(",
    "default_size(",
    "min_size(",
    "max_size(",
    "inner_margin(",
    "outer_margin(",
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/ is readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The argument list at `open` — the text between the paren at `open` and its
/// match — or `None` if the parens do not close on this line.
///
/// Line-scoped deliberately: a call whose arguments wrap is a call this scan
/// reports rather than guesses at, and every rule below treats "could not
/// read" as a finding.
fn argument(line: &str, open: usize) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut depth = 0_i32;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&line[open + 1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether `s` holds a numeric literal that is not part of an identifier.
///
/// `SPACE_4`, `region.id` and `panel_id(x, false)` do not count. `32.0`,
/// `28`, and `2.0 * BAR` do.
fn has_numeric_literal(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|i| {
        b[i].is_ascii_digit()
            && (i == 0
                || !matches!(b[i - 1], b'_' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'))
    })
}

/// Every edge panel this crate draws is addressed by a declared region, and
/// every measure it is given has a name.
///
/// The scan is over the whole of `src/`, not over the one file that draws the
/// window today: a panel moved into a helper in another module is the same
/// defect and would walk past a scan pointed at `window.rs`.
#[test]
fn every_panel_this_shell_draws_is_addressed_by_a_declared_region() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    assert!(
        !files.is_empty(),
        "found no src/ files to scan under {manifest}"
    );

    let mut violations: Vec<String> = Vec::new();
    let mut panels = 0_usize;

    for file in &files {
        let rel = file
            .strip_prefix(manifest)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(file).expect("source file is UTF-8");
        for (n, line) in text.lines().enumerate() {
            // Prose that *names* a panel constructor — a doc comment linking
            // `Panel::right`, say — is documentation, not a draw call.
            if line.trim_start().starts_with("//") {
                continue;
            }
            let loc = format!("{rel}:{}", n + 1);

            for spelling in UNREADABLE_PANELS {
                if line.contains(spelling) {
                    violations.push(format!(
                        "{loc}: `{spelling}` — the window draws its regions through \
                         `Panel::top/bottom/left/right` with an id from `panel_id`, \
                         which is what this gate can read"
                    ));
                }
            }
            if line.trim_start().starts_with("use ")
                && line.contains("Panel")
                && line.contains(" as ")
            {
                violations.push(format!(
                    "{loc}: a panel type imported under another name — this gate reads \
                     the call, so an alias is a region drawn past it"
                ));
            }

            for call in EDGE_PANELS {
                let Some(idx) = line.find(call) else { continue };
                panels += 1;
                let open = idx + call.len() - 1;
                let Some(arg) = argument(line, open) else {
                    violations.push(format!(
                        "{loc}: `{call}` argument does not close on its own line, so \
                         this gate cannot read which region it draws"
                    ));
                    continue;
                };
                if !arg.trim_start().starts_with(ID_CALL) {
                    violations.push(format!(
                        "{loc}: `{call}{arg})` — a panel's id comes from `{ID_CALL}region)`, \
                         so it is the region's own name. A literal here draws a region the \
                         arrangement has never heard of"
                    ));
                }
            }

            for call in MEASURE_CALLS {
                let Some(idx) = line
                    .match_indices(call)
                    .map(|(i, _)| i)
                    .find(|i| line.as_bytes().get(i.wrapping_sub(1)) == Some(&b'.'))
                else {
                    continue;
                };
                let open = idx + call.len() - 1;
                let Some(arg) = argument(line, open) else {
                    continue;
                };
                if has_numeric_literal(arg) {
                    violations.push(format!(
                        "{loc}: `{call}{arg})` — an extent or an inset is a named measure \
                         on the arrangement or the chrome token set, never a literal here"
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "region-completeness violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
    // A scan that matched nothing would report clean on a window with no
    // regions at all, which is the failure the count refuses. Seven edge
    // panels is the shipped window: two bands drawn always, one drawn on the
    // surface with a key grammar, and each of the three rails twice — once
    // open and once collapsed.
    assert!(
        panels >= 7,
        "the scan found {panels} edge panels; it is not reading the draw path"
    );
}

// ---------------------------------------------------------------------------
// Completeness by running the window
// ---------------------------------------------------------------------------

/// A path relative to the workspace root.
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const DASHBOARD: &str = "../../examples/dashboard.yaml";

/// A boot carrying a protocol **and** a chart: the chart takes the canvas, so
/// this is the surface with no key grammar and therefore no hint band.
fn chart_on_canvas() -> Boot {
    let spec = fixture("examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    Boot {
        composed: compose_spec(DASHBOARD).expect("compose the dashboard"),
        live: None,
        spec_path: Some(DASHBOARD.into()),
        authored: None,
        protocol: inputs,
        flow: Flow::Vertical,
        focus: None,
    }
}

/// A boot carrying a protocol and no chart: the graph takes the canvas, which
/// is the surface that draws the hint band. Both shapes are laid out below,
/// because a region that draws on only one of them is invisible to a gate
/// that lays out only the other.
fn graph_on_canvas() -> Boot {
    Boot::start(brightfield_shell::starts::CROSSWALK, Flow::Vertical)
        .expect("the crosswalk start ships with this build")
}

/// A settled window over `boot`, and the screen it was laid out in.
fn settled(boot: Boot) -> (MeridianApp, egui::Rect) {
    let (w, h) = boot.window_size();
    let mut app = MeridianApp::headless(boot, Mode::Light);
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
    let raw = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    // Three frames, as the sibling `arrangement.rs` runs: egui stores a
    // resizable panel's reported size and reads it back on the frame after.
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    (app, screen)
}

/// The area two rects share, which is zero for rects that merely touch along
/// an edge — as every pair of neighbouring regions does.
fn overlap(a: egui::Rect, b: egui::Rect) -> f32 {
    let w = (a.max.x.min(b.max.x) - a.min.x.max(b.min.x)).max(0.0);
    let h = (a.max.y.min(b.max.y) - a.min.y.max(b.min.y)).max(0.0);
    w * h
}

/// The declared regions cover the window exactly, so a panel the arrangement
/// does not know about has nowhere to come from.
///
/// This is the layer that does not care how the source is written. A band
/// added to the draw path takes its height out of the canvas, and the canvas
/// is the remainder — so the sum of the drawn rects falls short of the window
/// by exactly the extent of the thing nobody declared, whatever id it was
/// given and whichever file it was written in.
///
/// Three claims together, because any one of them alone can be satisfied by a
/// window that is wrong: every drawn region is inside the screen, no two
/// overlap, and the areas sum to the screen's. Exact cover follows from the
/// three and from nothing less.
#[test]
fn the_declared_regions_account_for_the_whole_window() {
    let plan = arrangement::default_arrangement();
    let mut drawn_somewhere: Vec<RegionId> = Vec::new();

    for (name, boot) in [
        ("the chart on the canvas", chart_on_canvas()),
        ("the graph on the canvas", graph_on_canvas()),
    ] {
        let (app, screen) = settled(boot);
        let mut covering: Vec<(RegionId, egui::Rect)> = Vec::new();

        for region in plan.regions {
            let Some(rect) = app.region_rect(region.id) else {
                continue; // a region this window does not draw
            };
            drawn_somewhere.push(region.id);
            // Excluded by the declaration rather than by a name written here:
            // an `Extent::Overlay` floats over the window's edge instead of
            // taking room from a sibling, so it is not part of the cover.
            if !region.extent.takes_space() {
                continue;
            }
            assert!(
                screen.contains_rect(rect),
                "{name}: {} drew at {rect:?}, outside the {screen:?} it was laid out in",
                region.id
            );
            covering.push((region.id, rect));
        }

        for (i, (a_id, a)) in covering.iter().enumerate() {
            for (b_id, b) in covering.iter().skip(i + 1) {
                let shared = overlap(*a, *b);
                assert!(
                    shared <= 1e-3,
                    "{name}: {a_id} and {b_id} overlap over {shared} square points; \
                     two regions are drawing over each other"
                );
            }
        }

        let covered: f32 = covering.iter().map(|(_, r)| r.area()).sum();
        let short = screen.area() - covered;
        assert!(
            short.abs() <= 1.0,
            "{name}: the {} declared regions drawn cover {covered} of the window's \
             {}, leaving {short} square points to something the arrangement does \
             not declare — a panel drawn here is a region, and a region is \
             declared in `brightfield_workbench::arrangement`",
            covering.len(),
            screen.area()
        );
    }

    // The other direction: a region declared and drawn by nothing is a
    // declaration nobody reads. Asked across both window shapes, because the
    // hint band belongs to the surface with a key grammar and only that one.
    for region in plan.regions {
        if !region.extent.takes_space() {
            continue;
        }
        assert!(
            drawn_somewhere.contains(&region.id),
            "{} is declared and neither window drew it",
            region.id
        );
    }
}
