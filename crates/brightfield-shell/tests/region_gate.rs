//! The region gate: what the window draws has to be what the arrangement
//! declares — which regions exist, and what extent each takes.
//!
//! Three layers. Two of them run the window; the third reads the source, and
//! the difference between what those two kinds of layer can hold is the whole
//! shape of this file.
//!
//! - **Every region drew at the extent it declares.**
//!   `every_regions_drawn_extent_is_the_one_it_declares` lays out every window
//!   this build can open and compares each drawn rect with the number the
//!   declaration names. This is what holds the extents.
//! - **The declared regions cover the window exactly.**
//!   `the_declared_regions_account_for_the_whole_window` measures the same
//!   rects against the window they were laid out in — no gap, no overlap — so
//!   a panel added anywhere, by any means, under any id takes its extent out
//!   of that sum. This is what holds the list of regions.
//! - **An edge panel's id comes from `window::panel_id`.**
//!   `every_panel_this_shell_draws_is_addressed_by_a_declared_region` reads
//!   the source for that. It is a fast convenience that names the offending
//!   line, and the paragraph below says exactly what it is and is not.
//!
//! # What a source scan can hold here, and what it cannot
//!
//! **It can hold the shape of a call, because that is a fact about the text.**
//! `Panel::top(panel_id(region, false))` either is or is not spelled that way,
//! and reading the text answers it. That is the one rule the scan below keeps.
//!
//! **It cannot hold what a value comes to, because that is not in the text.**
//! This file used to refuse a numeric literal inside a builder chain and claim
//! that a measure therefore had to have a name. That claim was false, and
//! four spellings walked past it: a `let` above the chain, the same literal
//! rebound twice, a local const, and the chain moved wholesale into a
//! correctly-shaped helper with the literal at its call site. None of them is
//! a gap a better pattern closes — a scan over Rust cannot follow an
//! identifier to what it resolves to, so no version of that rule could have
//! held the property. The rule is gone rather than qualified.
//!
//! **What replaced it asks the window instead of the source.** A literal
//! cannot survive being drawn, however it is spelled, because every one of
//! those four spellings changes the rect that reaches the screen — which is
//! what `every_regions_drawn_extent_is_the_one_it_declares` reads. All four
//! were applied and watched fail there while the scan reported clean.
//!
//! **The two remaining scan rules are backed, and that is why they may stay.**
//! Defeat the id rule — `let id = "bf-x"; Panel::top(id)` — and the panel
//! still draws, still takes space, and the cover still reddens. The extent
//! rule had no such backing: the cover sums areas and the canvas is the
//! remainder, so a band drawn at the wrong height is absorbed by the canvas
//! and the sum comes out right. That asymmetry is the reason one rule was
//! deleted and two were kept.
//!
//! # Neither running layer may enumerate its own coverage
//!
//! A completeness gate with a hand-kept list inside it is the defect it exists
//! to refuse, one level up. The corpus is therefore `Boot::empty` plus one
//! boot per `starts::STARTS` entry — the shell's own answer to *what can a
//! user open*, and the single declaration the gallery cards, the empty-state
//! buttons and the boot path read. A start added there is laid out here with
//! no edit to this file. Before that, the corpus was two hand-picked windows
//! and both were past the front door, so a panel added to the door branch
//! alone drew to a user and was seen by nothing.
//!
//! # What no layer covers
//!
//! A `CentralPanel` carries no id, so the scan has nothing to read from it,
//! and it is the remainder by construction, so a second one steals no space
//! and neither running layer would see it. That case is covered upstream:
//! `audit_arrangement` refuses an arrangement whose count of
//! `Extent::Remainder` regions is anything but one, and
//! `the_shipped_arrangement_declares_one_canvas_and_no_duplicate_region`
//! counts it. A `CentralPanel` cannot be a region the arrangement lacks — it
//! can be the one the arrangement already has.
//!
//! A rail the user has collapsed or dragged is not laid out here: every window
//! in the corpus is freshly booted, so each rail is at its default. Nor is a
//! rail's *floor*, which is a rail's other declared number and binds only
//! under a drag — an `egui::Panel` lays out at its default whatever room is
//! left, so no window here presses one down to it. All three live in the
//! sibling `arrangement.rs`:
//! `each_rail_collapses_to_the_measure_it_declares`,
//! `a_rail_reopens_at_the_extent_it_was_dragged_to`, and
//! `a_rail_dragged_past_its_floor_stops_at_the_floor_it_declares`.

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::starts;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::{self, Edge, Extent, RegionId};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The id rule, by source scan
// ---------------------------------------------------------------------------

/// Where an edge panel is constructed.
///
/// One pattern rather than a list of edges: `Panel::` matches every edge
/// constructor egui has, and, as a substring, `CentralPanel::` too — which
/// takes no id and shows up below as an empty argument rather than as a name
/// written into this test.
const PANEL: &str = "Panel::";

/// The one call that may supply an edge panel's id.
const ID_CALL: &str = "panel_id(";

/// Panel spellings this scan cannot read. egui carries the first two as the
/// older names for the same containers, and an alias hides the type outright.
/// A region drawn through any of them would be a region drawn past this rule,
/// so they are refused by name rather than passed over in silence.
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
/// documentation rather than a draw call, and both spellings are blanked:
/// reading `//` alone left a `/* */` block being read as code. String bodies
/// are blanked while their quotes are kept, which leaves a hand-typed panel
/// id visible as a literal exactly where an id call belongs.
///
/// Block comments nest in Rust, so the depth is counted rather than the first
/// `*/` being taken as the end.
fn masked(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 0_u32;
            while i < b.len() {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
        } else if b[i] == b'"' {
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
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking bytes keeps the text UTF-8")
}

/// The balanced argument list opening at `open`, as a span, or `None` when
/// the parens do not close.
///
/// A span rather than a slice: the rule reads the blanked copy so a comment
/// or a string cannot trip it, and the failure message reads the same span of
/// the real source so a reader sees what was written.
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

/// The 1-based line `offset` falls on.
fn line_of(src: &str, offset: usize) -> usize {
    src[..offset].matches('\n').count() + 1
}

/// A span, cut to something a failure message can carry.
fn shown(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 160 {
        one_line
    } else {
        let head: String = one_line.chars().take(160).collect();
        format!("{head}…")
    }
}

/// Every edge panel this shell draws is addressed through `window::panel_id`,
/// whose argument is a `Region` — so a panel drawn for something the
/// arrangement has never heard of has no id to draw under. What holds that
/// once the spelling is evaded is
/// `the_declared_regions_account_for_the_whole_window`, not this rule.
///
/// **This rule is a convenience, not the gate.** It fails in a second and
/// names the line, which is worth having; but it is a claim about how a call
/// is spelled, and `let id = "bf-x"; Panel::top(id)` satisfies it. What
/// catches that is `the_declared_regions_account_for_the_whole_window`, which
/// reads the rect the panel actually took. The module docs say why this rule
/// survived a round that deleted its sibling.
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
                     is the shape this rule can read",
                    line_of(&src, idx)
                ));
            }
        }

        for (idx, _) in src.match_indices(PANEL) {
            let loc = format!("{rel}:{}", line_of(&src, idx));
            let Some(open) = src[idx..].find('(').map(|o| idx + o) else {
                continue;
            };
            // The paren has to belong to this constructor rather than to
            // something later in the statement.
            if src[idx + PANEL.len()..open].contains(|c: char| !c.is_alphanumeric() && c != '_') {
                continue;
            }
            let Some((from, to)) = argument(&src, open) else {
                violations.push(format!(
                    "{loc}: a panel constructor whose argument list does not close, so \
                     this rule cannot read which region it draws"
                ));
                continue;
            };
            let arg = &src[from..to];
            if arg.trim().is_empty() {
                continue; // a `CentralPanel`, which takes no id
            }
            edge_panels += 1;
            if !arg.trim_start().starts_with(ID_CALL) {
                violations.push(format!(
                    "{loc}: `{}({})` — a panel's id comes from `{ID_CALL}region)`, so it \
                     is the region's own name. A literal here draws a region the \
                     arrangement has never heard of",
                    &raw[idx..open],
                    shown(&raw[from..to])
                ));
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
        stacked_tiles: None,
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

// ---------------------------------------------------------------------------
// Every extent on the screen is an extent the declaration names
// ---------------------------------------------------------------------------

/// The extent `rect` takes on the axis `edge` runs across.
///
/// # Panics
///
/// On [`Edge::Centre`], which has no axis of its own — the canvas is the
/// remainder and is compared against the remainder arithmetic instead.
fn extent_across(edge: Edge, rect: egui::Rect) -> f32 {
    match edge {
        Edge::Left | Edge::Right => rect.width(),
        Edge::Top | Edge::Bottom => rect.height(),
        Edge::Centre => panic!("a region that is the remainder has no extent of its own"),
    }
}

/// Every region drew at the extent its declaration names, over every window
/// this build can open.
///
/// **This is the assertion that holds the extents, and the source scan above
/// is not.** A scan can ask whether a digit sits inside a builder chain; it
/// cannot ask what an identifier resolves to, so `let h = 40.0; …
/// .exact_size(h)`, a rebinding, a local const, a helper with the literal at
/// its call site and a macro parameter all read as clean text. Each of them
/// changes the rect that reaches the screen, which is what this reads. The
/// rule stopped being about how the measure is spelled and became about what
/// it comes to.
///
/// The sibling `arrangement.rs` carries the single-window form of this and is
/// where the collapse and drag cases live. Two things it leaves out are here:
///
/// - **the canvas**, which it skips because `Extent::Remainder` names no
///   number. It has one all the same — whatever the bands and rails leave —
///   and that is arithmetic over the same declarations every other line here
///   reads, so it is computed and compared. Without it, a band drawn one
///   point taller than it declares is a point the canvas silently absorbs.
/// - **the status band**, which it skips because an `Extent::Overlay` takes
///   no space from a sibling. Taking no space is not the same as having no
///   size, and its declared height was drawn by a draw path calling the
///   measure itself rather than reading the region until this round.
///
/// Both over the corpus rather than one window, which is what the derived
/// corpus was built for.
///
/// What this does not cover: a rail the user has collapsed or dragged. Every
/// window here is freshly booted, so each rail is at its default;
/// `a_rail_reopens_at_the_extent_it_was_dragged_to` and
/// `each_rail_collapses_to_the_measure_it_declares` are where the other two
/// states are held.
#[test]
fn every_regions_drawn_extent_is_the_one_it_declares() {
    let plan = arrangement::default_arrangement();
    let mut fixed = 0_usize;
    let mut canvases = 0_usize;
    let mut overlays = 0_usize;

    for (name, boot) in every_window_this_build_can_open() {
        let (app, screen) = settled(boot);

        // What the declarations predict is left for the canvas, accumulated
        // off the same numbers the per-region assertions below compare
        // against — so a band that drew wide would have to have been declared
        // wide for the canvas to still come out right.
        let mut across = screen.width();
        let mut down = screen.height();

        for region in plan.regions {
            let Some(rect) = app.region_rect(region.id) else {
                continue; // a region this window does not draw
            };
            let declared = match region.extent {
                Extent::Band(size) | Extent::Rail { default: size, .. } => size,
                Extent::Overlay(size) => {
                    let drawn = extent_across(region.edge, rect);
                    assert!(
                        (drawn - size).abs() < 1e-3,
                        "{name}: {} floats at {drawn}pt against the {size}pt the \
                         arrangement declares — the layer is drawing at a measure \
                         the declaration does not name",
                        region.id
                    );
                    overlays += 1;
                    continue;
                }
                Extent::Remainder => continue, // compared below, once the rest is known
            };

            let drawn = extent_across(region.edge, rect);
            assert!(
                (drawn - declared).abs() < 1e-3,
                "{name}: {} drew at {drawn}pt against the {declared}pt the \
                 arrangement declares — the extent that reached the screen is not \
                 the extent the declaration names, however it was spelled",
                region.id
            );
            match region.edge {
                Edge::Left | Edge::Right => across -= declared,
                _ => down -= declared,
            }
            fixed += 1;
        }

        if let Some(rect) = app.region_rect(arrangement::CANVAS) {
            assert!(
                (rect.width() - across).abs() < 1e-3 && (rect.height() - down).abs() < 1e-3,
                "{name}: the canvas drew {}x{} against the {across}x{down} the \
                 declarations leave it — a region took room the arrangement does \
                 not account for, or took more of it than it declares",
                rect.width(),
                rect.height()
            );
            canvases += 1;
        }
    }

    // A sweep that compared almost nothing would satisfy every assertion
    // above. The canvas is drawn by every window in the corpus — the door
    // stands in it when there is no document — and the status band floats on
    // at least the one that has something to say.
    assert!(
        fixed >= 20 && canvases >= 6 && overlays >= 1,
        "the sweep compared {fixed} fixed extents, {canvases} canvases and \
         {overlays} floating bands; it is not reading the corpus"
    );
}
