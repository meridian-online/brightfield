//! The region-completeness gate: what the shell draws as a region of the
//! window has to be in the arrangement that declares them.
//!
//! Two layers, and they fail for different reasons on purpose. The sibling
//! `arrangement.rs` asks whether the regions that *are* declared drew at the
//! extents they declare; this file asks whether the declaration is the whole
//! list.
//!
//! - **By source scan**, in the shape of `gallery_gate.rs`'s completeness
//!   grep. An edge panel's id comes from `window::panel_id`, whose argument is
//!   a `Region`, so a panel drawn for something the arrangement has never
//!   heard of has no id to draw under; and a builder chain that reaches a
//!   `Panel` or a `Frame` may carry no numeric literal, so a measure has to
//!   have a name. `every_panel_this_shell_draws_is_addressed_by_a_declared_region`
//!   is that scan.
//! - **By running the window**, which is the layer that does not depend on
//!   how the source happens to be written: the regions the arrangement
//!   declares are laid out and their drawn rects are measured against the
//!   window they were laid out in. They tile it exactly — no gap and no
//!   overlap — so a panel added anywhere, by any means, under any id, takes
//!   its extent out of that sum and
//!   `the_declared_regions_account_for_the_whole_window` reddens.
//!
//! # Neither layer may enumerate its own coverage
//!
//! This file is a completeness gate, so a hand-kept list *inside* it is the
//! defect it exists to refuse, one level up. Both lists it started with had a
//! gap, and both gaps were real:
//!
//! - it laid out two windows, both of them past the front door, so a panel
//!   added to the door branch alone was drawn to a user and seen by nothing.
//!   The corpus is now `Boot::empty` plus one boot per `starts::STARTS` entry
//!   — the shell's own answer to *what can a user open* — so a window shape
//!   this gate does not reach is a window a user cannot open.
//! - it matched a list of extent setters by name, and egui has more of them
//!   than the list carried: `size_range` bounds the same measure `min_size`
//!   does, so a rail could be given a floor of `160.0` with no test pinning
//!   it, and the initial undragged layout still rendered at its default so
//!   the running window saw no difference either. There is no setter list
//!   now. `every_panel_this_shell_draws_is_addressed_by_a_declared_region`
//!   reads the whole builder chain instead, so a setter added to egui
//!   tomorrow is covered by a rule that was written without knowing the old
//!   ones' names.
//!
//! # What neither layer covers, and why that is not a hole
//!
//! A `CentralPanel` carries no id, so the scan has nothing to read from it;
//! and it is the remainder by construction, so a second one does not steal
//! space and the runtime layer would not see it either. That case is covered
//! upstream instead: `audit_arrangement` refuses an arrangement whose count of
//! `Extent::Remainder` regions is anything but one, and
//! `the_shipped_arrangement_declares_one_canvas_and_no_duplicate_region`
//! counts it. A `CentralPanel` cannot be a region the arrangement lacks — it
//! can be the one the arrangement already has.

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::starts;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::{self, RegionId};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Completeness by source scan
// ---------------------------------------------------------------------------

/// Where a builder chain that ends at a `Panel` or a `Frame` begins.
///
/// Three patterns rather than a list of names, and that is the point.
/// `Panel::` matches every edge constructor and, as a substring,
/// `CentralPanel::` too; `Frame::` matches egui's frame constructors;
/// `_frame(` matches the chrome token set's own frame functions by the
/// convention they are named under. An edge, a constructor or a frame
/// function added to any of the three is matched without this file being
/// edited.
const CHAIN_HEADS: &[&str] = &["Panel::", "Frame::", "_frame("];

/// The one call that may supply an edge panel's id.
const ID_CALL: &str = "panel_id(";

/// Panel spellings this scan cannot read. egui carries them as the older
/// names for the same containers, and an alias hides the type outright; a
/// region drawn through either would be a region drawn past this gate, so
/// they are refused by name rather than passed over in silence.
const UNREADABLE: &[&str] = &["SidePanel", "TopBottomPanel", "Panel as "];

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

/// `src` with every comment body and every string body blanked to spaces,
/// keeping length and line breaks so an offset still names its own line.
///
/// Comments are blanked because prose that *names* a constructor is
/// documentation rather than a draw call. String bodies are blanked because a
/// digit inside one is text, not a measure — and because blanking the body
/// while keeping the quotes leaves a hand-typed panel id still visible as a
/// literal where an id call belongs.
fn masked(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out[i] = b' ';
                        i += 1;
                    }
                    out[i] = b' ';
                    i += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("blanking bytes keeps the text UTF-8")
}

/// The builder chain starting at `at`: the text up to the closure it shows
/// into, the statement's end, or the close of the block it sits in —
/// whichever comes first.
///
/// Stopping at `.show(` is what keeps the rule about the *builder* rather
/// than about a rail's hundred-line body, where a numeric literal is
/// ordinary.
///
/// Returned as a span rather than a slice: the rule reads the blanked copy so
/// a comment or a string cannot trip it, and the failure message reads the
/// same span of the real source so the reader sees what was written.
fn chain(src: &str, at: usize) -> (usize, usize) {
    let b = src.as_bytes();
    let mut depth = 0_i32;
    let mut i = at;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return (at, i);
                }
            }
            b';' if depth == 0 => return (at, i),
            b'.' if depth == 0 && src[i..].starts_with(".show(") => return (at, i),
            _ => {}
        }
        i += 1;
    }
    (at, b.len())
}

/// The balanced argument list opening at `open`, as a span, or `None` when
/// the parens do not close.
fn argument(src: &str, open: usize) -> Option<(usize, usize)> {
    let b = src.as_bytes();
    let mut depth = 0_i32;
    for (i, byte) in b.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, i));
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
/// `28` and `160.0..=999.0` do.
fn has_numeric_literal(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|i| {
        b[i].is_ascii_digit()
            && (i == 0
                || !matches!(b[i - 1], b'_' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'))
    })
}

/// The 1-based line `offset` falls on.
fn line_of(src: &str, offset: usize) -> usize {
    src[..offset].matches('\n').count() + 1
}

/// Whether the identifier `at` sits inside is being *declared* rather than
/// called.
///
/// A definition is not a call site, and reading one as a chain runs the walk
/// through a whole function body — which is how `fn request_frame` first
/// arrived here as an extent with a literal in it.
fn is_a_declaration(src: &str, at: usize) -> bool {
    let b = src.as_bytes();
    let mut start = at;
    while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
        start -= 1;
    }
    src[..start].trim_end().ends_with("fn")
}

/// A chain, cut to something a failure message can carry.
fn shown(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 160 {
        one_line
    } else {
        let head: String = one_line.chars().take(160).collect();
        format!("{head}…")
    }
}

/// Every edge panel this shell draws is addressed by a declared region, and
/// no measure reaching a panel or a frame is a literal.
///
/// The scan is over the whole of `src/`, not over the one file that draws the
/// window today: a panel moved into a helper in another module is the same
/// defect and would walk past a scan pointed at `window.rs`.
#[test]
fn every_panel_this_shell_draws_is_addressed_by_a_declared_region() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src_root = Path::new(manifest).join("src");
    let mut files = Vec::new();
    rs_files(&src_root, &mut files);
    assert!(
        !files.is_empty(),
        "found no src/ files to scan under {manifest}"
    );

    let mut violations: Vec<String> = Vec::new();
    let mut edge_panels = 0_usize;

    for file in &files {
        let rel = file
            .strip_prefix(manifest)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let raw = fs::read_to_string(file).expect("source file is UTF-8");
        let src = masked(&raw);

        for spelling in UNREADABLE {
            for (idx, _) in src.match_indices(spelling) {
                violations.push(format!(
                    "{rel}:{}: `{spelling}` — the window draws its regions through \
                     `Panel::top/bottom/left/right` with an id from `panel_id`, which \
                     is the shape this gate can read",
                    line_of(&src, idx)
                ));
            }
        }

        for head in CHAIN_HEADS {
            for (idx, _) in src.match_indices(head) {
                if is_a_declaration(&src, idx) {
                    continue;
                }
                let loc = format!("{rel}:{}", line_of(&src, idx));
                let (from, to) = chain(&src, idx);

                if has_numeric_literal(&src[from..to]) {
                    violations.push(format!(
                        "{loc}: a numeric literal in the builder chain at `{}` — an \
                         extent, a floor, a bound or an inset is a named measure on \
                         the arrangement or the chrome token set, never a number here",
                        shown(&raw[from..to])
                    ));
                }

                // The id rule is the edge panel's alone. A `CentralPanel` and
                // a frame constructor take no id, which shows up here as an
                // empty argument rather than as a name written into this test.
                if *head != "Panel::" {
                    continue;
                }
                let Some(open) = src[idx..].find('(').map(|o| idx + o) else {
                    continue;
                };
                // The paren has to belong to this constructor rather than to
                // something later in the statement.
                if src[idx + head.len()..open].contains(|c: char| !c.is_alphanumeric() && c != '_')
                {
                    continue;
                }
                let Some((from, to)) = argument(&src, open) else {
                    violations.push(format!(
                        "{loc}: a panel constructor whose argument list does not close, \
                         so this gate cannot read which region it draws"
                    ));
                    continue;
                };
                let arg = &src[from..to];
                if arg.trim().is_empty() {
                    continue;
                }
                edge_panels += 1;
                if !arg.trim_start().starts_with(ID_CALL) {
                    violations.push(format!(
                        "{loc}: `{}({})` — a panel's id comes from `{ID_CALL}region)`, so \
                         it is the region's own name. A literal here draws a region the \
                         arrangement has never heard of",
                        &raw[idx..open],
                        shown(&raw[from..to])
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
    // regions at all, which is the failure this count refuses. Seven edge
    // panels is the shipped window: two bands drawn always, one drawn on the
    // surface with a key grammar, and each of the three rails twice — once
    // open and once collapsed.
    assert!(
        edge_panels >= 7,
        "the scan read {edge_panels} edge panels; it is not reading the draw path"
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

/// A boot carrying a protocol **and** a chart, so each rail has real content
/// rather than an empty state. Not a window a user reaches by one click, which
/// is why it is named beside the corpus below rather than being part of it.
fn a_protocol_and_a_chart() -> Boot {
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

/// Every window this build can open, **derived rather than listed**.
///
/// This is the part of the gate that was wrong before it was written down.
/// A cover that lays out a hand-picked pair of windows is complete as far as
/// somebody remembered to extend the pair, and no further — which is the same
/// defect this whole file exists to refuse, one level up in the checker. The
/// corpus is therefore the shell's own answer to *what can a user open*:
/// `Boot::empty`, which is the window with no document and therefore the
/// front door, and one boot per entry of `starts::STARTS`, which is the
/// single declaration the gallery cards, the empty-state buttons and the boot
/// path read. A start added there is laid out here with no edit to this file,
/// and a window shape this gate does not reach is a window a user cannot
/// open. `the_declared_regions_account_for_the_whole_window` is what consumes
/// this corpus, and its coverage loop reddens when an arm of the draw path
/// goes unlaid-out.
///
/// The both-documents boot rides along because it is the one window where
/// each rail draws content instead of an empty state.
fn every_window_this_build_can_open() -> Vec<(String, Boot)> {
    let mut boots = vec![
        ("no document — the front door".to_owned(), Boot::empty()),
        (
            "a protocol and a chart".to_owned(),
            a_protocol_and_a_chart(),
        ),
    ];
    for start in starts::STARTS {
        let boot = Boot::start(start.id, Flow::Vertical)
            .unwrap_or_else(|e| panic!("the {} start ships with this build: {e}", start.id));
        boots.push((format!("the {} start", start.id), boot));
    }
    boots
}

/// Which arm of the draw path a settled window is in.
///
/// Read off the window's own two predicates rather than declared here: the
/// door replaces every region below the title band, and the canvas holds one
/// document or the other. Those are the branches `MeridianApp::draw` takes,
/// and each draws a different set of regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Composition {
    /// The door stands where the canvas would.
    FrontDoor,
    /// The asset graph is on the canvas, which is the surface with a key
    /// grammar and therefore the one that draws the hint band.
    GraphOnCanvas,
    /// A chart is on the canvas, and there is no hint band.
    ChartOnCanvas,
}

impl Composition {
    fn of(app: &MeridianApp) -> Self {
        match (app.front_door_is_live(), app.graph_on_canvas()) {
            (true, _) => Self::FrontDoor,
            (false, true) => Self::GraphOnCanvas,
            (false, false) => Self::ChartOnCanvas,
        }
    }
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
/// an edge — as neighbouring regions do.
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
    let mut compositions: Vec<Composition> = Vec::new();

    for (name, boot) in every_window_this_build_can_open() {
        let (app, screen) = settled(boot);
        compositions.push(Composition::of(&app));
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

    // The corpus reaches every arm of the draw path. Derived from the windows
    // above rather than asserted about a list: each arm draws a different set
    // of regions, so a corpus that missed one would be a cover with a branch
    // in it nobody laid out — which is how the front door went uncovered
    // while this file reported clean.
    for arm in [
        Composition::FrontDoor,
        Composition::GraphOnCanvas,
        Composition::ChartOnCanvas,
    ] {
        assert!(
            compositions.contains(&arm),
            "no window in the corpus settled into {arm:?}, so the cover never \
             laid that arm of the draw path out"
        );
    }

    // The other direction: a region declared and drawn by nothing is a
    // declaration nobody reads.
    for region in plan.regions {
        if !region.extent.takes_space() {
            continue;
        }
        assert!(
            drawn_somewhere.contains(&region.id),
            "{} is declared and no window in the corpus drew it",
            region.id
        );
    }
}
