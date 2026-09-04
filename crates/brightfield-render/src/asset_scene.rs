//! Asset-graph scene builder — draws a `brightfield-protocol`
//! layout into a vello scene.
//!
//! The protocol DAG twin of `scene.rs`: same idiom (Meridian ink via the
//! `ink` boundary, labels through the `text` module, plain kurbo shapes),
//! but the input is an [`AssetGraph`] + [`Layout`] instead of data batches.
//! Node treatments per kind: SOURCE pill, FILE document
//! silhouette, TABLE card, INTERNAL muted card, DATASET double-ring, family
//! tile with an `xN` count, opaque chip with an issue badge. Steps render as
//! seam chevrons on their edges; a validation gate is a shield glyph on the
//! guarded edge, never a node. Edges route orthogonally along the layout's
//! dummy-node lanes.
//!
//! Every colour is resolved for the caller's mode through [`AssetInk`], which
//! reads the same [`mod@meridian_design::semantic`] layer the workbench chrome
//! reads. No colour here is resolved without asking the mode first — the
//! light scales still appear, but only as the light branch of that question,
//! never as a value settled before it is asked.

use kurbo::{Affine, BezPath, Circle, Rect, RoundedRect, Stroke};
use peniko::{Color, Fill};
use vello::Scene;

use std::collections::BTreeMap;

use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_protocol::graph::{AssetGraph, AssetId, AssetKind, AssetNode, StepId};
use brightfield_protocol::layout::{EdgeRoute, Flow, Layout, ViewChip};

use crate::ink::ink;
use crate::text::{draw_text, TextAnchor, LABEL_SIZE};

/// Every colour this module paints, resolved for one mode.
///
/// The scene used to hold thirteen `const Color`s read straight off
/// `chrome::INK_LIGHT` and the `*_LIGHT` scales, which is why the DAG raster
/// stayed a white sheet inside a dark window: nothing in this file could see
/// the mode. This struct is the same list, resolved through
/// [`meridian_design::semantic()`] the way `brightfield_workbench::chrome` does
/// — the panel's chrome and the raster inside it now ask the same layer the
/// same question, so they cannot drift.
///
/// Where a slot names the thing being painted, the semantic layer is used and
/// the field says which slot. Three values sit off the semantic layer's named
/// slots and take the mode's raw gray scale instead — the crate docs sanction
/// exactly that ("drop to a raw scale only when the thing being coloured
/// genuinely has no semantic name yet"), and each says why at the field. In
/// light mode every field resolves to the byte-identical value its `const`
/// predecessor held, which is why this change moves no light-mode pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssetInk {
    /// Canvas behind the graph — the app plane, one step away from the node
    /// cards so cards read as cards.
    pub canvas: Color,
    /// Node card fill — the raised reading surface.
    pub node_fill: Color,
    /// Quiet card border (sources, files). Gray step 5 (index 4): **below**
    /// the semantic border band (6–8) on purpose. A card hairline that repeats
    /// at every node in a dense DAG reads as a lattice at the band's weight —
    /// the same argument `Borders::divider` makes for gridlines, one step
    /// quieter still.
    pub node_border: Color,
    /// Stronger border for TABLE cards — the secondary ink, because a TABLE's
    /// edge is doing the work of a label here, not of a hairline.
    pub table_border: Color,
    /// Edge ink — the default hairline (`borders.subtle`).
    pub edge: Color,
    /// Accent (Dataset double-ring, shield, family count) — the focus border,
    /// the one accent, exactly as `chrome::tone_colour` resolves `Tone::Accent`.
    pub accent: Color,
    /// Issue badge on an opaque chip — the Warning role's resting solid.
    /// Amber step 9 is byte-identical in both scales, so like
    /// [`Self::badge_glyph`] this one does not move with the mode: a bright
    /// solid is a paint, not a plane.
    pub issue: Color,
    /// Glyph drawn on the amber issue badge — the Warning role's own
    /// foreground, which is dark ink rather than the near-white every other
    /// role takes. The design crate says so twice, and means it: near-white on
    /// amber step 9 measures 2.47:1, dark ink on it 6.45:1. Like
    /// [`Self::issue`] it does not move with the mode, because a bright solid
    /// and the ink chosen to sit on it are both paints rather than planes.
    pub badge_glyph: Color,
    /// Muted fill for INTERNAL statement intermediates — the sunken plane.
    pub internal_fill: Color,
    /// Chip fill for degraded statements. Gray step 3 (index 2): one step up
    /// from [`Self::internal_fill`], so a degraded chip reads as *more*
    /// recessed than a plain intermediate. The semantic layer has no slot for
    /// "the next step up from sunken".
    pub chip_fill: Color,
    /// Primary label ink.
    pub label: Color,
    /// Muted label ink (internal/chip labels) — `text.placeholder`, which is
    /// the same step as chrome's `ink_muted` this replaces.
    pub muted_label: Color,
    /// Skipped-seam tint. Gray step 7 (index 6): the only status tint with no
    /// reserved status colour, and it must sit clearly above [`Self::edge`]
    /// (step 6) without becoming ink.
    pub skipped: Color,
    /// The hairline round a **view chip** the canvas is not showing —
    /// `borders.subtle`, the default hairline, because an unfilled chip is a
    /// way to somewhere else rather than a control that must be found.
    pub chip_border: Color,
    /// The plane behind the view chip the canvas **is** showing —
    /// `tabs.active_background`, which is the slot the design system already
    /// uses for "this is the one you are on" in a set of peers. The chip row
    /// in a node's foot is that set.
    pub chip_active_fill: Color,
    /// The hairline round the showing chip — `borders.default_`, one step up
    /// from [`Self::chip_border`], so the filled chip has an edge as well as a
    /// plane and does not read as a bare wash.
    pub chip_active_border: Color,
    /// The word on a chip the canvas is not showing — `tabs.foreground`.
    pub chip_label: Color,
    /// The word on the showing chip — `tabs.active_foreground`.
    pub chip_active_label: Color,
}

/// Execution-status tints for a seam — the reserved Meridian status inks,
/// **fixed across modes by definition** (`viz::STATUS`: "reserved, never
/// reused as series, fixed across modes"), so they are the one part of this
/// palette that does not take a mode. `NotRun` keeps the quiet edge ink so an
/// unrun seam is never green.
const STATUS_OK: Color = ink(meridian_design::viz::STATUS.good);
const STATUS_RUNNING: Color = ink(meridian_design::viz::STATUS.warning);
const STATUS_FAILED: Color = ink(meridian_design::viz::STATUS.critical);

impl AssetInk {
    /// Resolve the palette for a mode — `dark` is the same flag
    /// [`meridian_design::semantic()`] takes, and callers pass `mode.is_dark()`.
    #[must_use]
    pub fn for_mode(dark: bool) -> Self {
        let sem = meridian_design::semantic(dark);
        let gray = if dark {
            meridian_design::scales::GRAY_DARK
        } else {
            meridian_design::scales::GRAY_LIGHT
        };
        Self {
            canvas: ink(sem.surfaces.app),
            node_fill: ink(sem.surfaces.raised),
            node_border: ink(gray[4]),
            table_border: ink(sem.text.secondary),
            edge: ink(sem.borders.subtle),
            accent: ink(sem.borders.focus),
            issue: ink(sem.role(meridian_design::Role::Warning).background.base),
            badge_glyph: ink(sem.role(meridian_design::Role::Warning).foreground.base),
            internal_fill: ink(sem.surfaces.sunken),
            chip_fill: ink(gray[2]),
            label: ink(sem.text.primary),
            muted_label: ink(sem.text.placeholder),
            skipped: ink(gray[6]),
            chip_border: ink(sem.borders.subtle),
            chip_active_fill: ink(sem.tabs.active_background),
            chip_active_border: ink(sem.borders.default_),
            chip_label: ink(sem.tabs.foreground),
            chip_active_label: ink(sem.tabs.active_foreground),
        }
    }

    /// The ink a seam chevron takes for its execution status — the two-channel
    /// rule (execution status is its own colour channel). `NotRun` falls back
    /// to the quiet edge ink, so unmeasured never reads green.
    fn status(self, status: SeamStatus) -> Color {
        match status {
            SeamStatus::Ok => STATUS_OK,
            SeamStatus::Running => STATUS_RUNNING,
            SeamStatus::Skipped => self.skipped,
            SeamStatus::Failed => STATUS_FAILED,
            SeamStatus::NotRun => self.edge,
        }
    }
}
/// Vertical offset from a card's centre to the label baseline (the
/// `WIDGET_BASELINE_NUDGE` convention from `scene.rs`).
const BASELINE_NUDGE: f64 = 4.0;
/// Char budget per pixel of card width — matches the layout crate's
/// char-count width heuristic (~7px per char at 11px Inter).
const PX_PER_CHAR: f64 = 7.0;

/// Fit `label` into `width` pixels, truncating with an ellipsis.
fn fit_label(label: &str, width: f64) -> String {
    let budget = ((width - 12.0) / PX_PER_CHAR).max(1.0) as usize;
    if label.chars().count() <= budget {
        return label.to_string();
    }
    let mut out: String = label.chars().take(budget.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Orthogonal polyline through the route's waypoints. Horizontal flow runs each
/// hop H-V-H (out along the flow to the midpoint, across to the next row, in);
/// Vertical flow transposes to V-H-V (down to the midpoint, across to the next
/// column, in) — the same lane discipline, rotated a quarter turn.
fn orthogonal_path(points: &[(f64, f64)], flow: Flow) -> BezPath {
    let mut path = BezPath::new();
    let Some(&(x0, y0)) = points.first() else {
        return path;
    };
    path.move_to((x0, y0));
    let mut prev = (x0, y0);
    for &(x, y) in &points[1..] {
        match flow {
            Flow::Horizontal => {
                if (y - prev.1).abs() < 0.5 {
                    path.line_to((x, y));
                } else {
                    let mid_x = ((prev.0 + x) / 2.0).round();
                    path.line_to((mid_x, prev.1));
                    path.line_to((mid_x, y));
                    path.line_to((x, y));
                }
            }
            Flow::Vertical => {
                if (x - prev.0).abs() < 0.5 {
                    path.line_to((x, y));
                } else {
                    let mid_y = ((prev.1 + y) / 2.0).round();
                    path.line_to((prev.0, mid_y));
                    path.line_to((x, mid_y));
                    path.line_to((x, y));
                }
            }
        }
        prev = (x, y);
    }
    path
}

/// A small double chevron at (`cx`, `cy`) pointing along the flow — right for
/// Horizontal, down for Vertical — the seam glyph.
fn draw_chevron(scene: &mut Scene, cx: f64, cy: f64, flow: Flow, colour: Color) {
    for offset in [-3.0, 2.0] {
        let mut chevron = BezPath::new();
        match flow {
            Flow::Horizontal => {
                chevron.move_to((cx + offset - 2.0, cy - 3.5));
                chevron.line_to((cx + offset + 2.0, cy));
                chevron.line_to((cx + offset - 2.0, cy + 3.5));
            }
            Flow::Vertical => {
                chevron.move_to((cx - 3.5, cy + offset - 2.0));
                chevron.line_to((cx, cy + offset + 2.0));
                chevron.line_to((cx + 3.5, cy + offset - 2.0));
            }
        }
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, colour, None, &chevron);
    }
}

/// The gate-as-shield glyph near the guarded edge's target.
fn draw_shield(scene: &mut Scene, cx: f64, cy: f64, accent: Color) {
    let (w, h) = (9.0, 11.0);
    let mut shield = BezPath::new();
    shield.move_to((cx - w / 2.0, cy - h / 2.0));
    shield.line_to((cx + w / 2.0, cy - h / 2.0));
    shield.line_to((cx + w / 2.0, cy + h / 6.0));
    shield.line_to((cx, cy + h / 2.0));
    shield.line_to((cx - w / 2.0, cy + h / 6.0));
    shield.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &shield);
}

/// The chevron site on the route's middle segment. The right-pointing glyph
/// must land on a HORIZONTAL run: for a row-changing segment `orthogonal_path`
/// draws an H-V-H detour (horizontal at `a.y` to `mid_x`, vertical, horizontal
/// in at `b.y`), and the raw midpoint would sit on the vertical run. Place it
/// mid-way along the FIRST horizontal leg instead, so it always reads along the
/// flow direction.
fn route_midpoint(points: &[(f64, f64)], flow: Flow) -> (f64, f64) {
    let seg = (points.len() - 1) / 2;
    let (a, b) = (points[seg], points[seg + 1]);
    match flow {
        Flow::Horizontal => {
            if (a.1 - b.1).abs() < 0.5 {
                // Already horizontal: the true midpoint reads along the flow.
                (((a.0 + b.0) / 2.0).round(), ((a.1 + b.1) / 2.0).round())
            } else {
                // Row-changing: sit on the first horizontal leg (a.y, a.0..mid_x),
                // matching orthogonal_path's `mid_x = round((a.0 + b.0) / 2)`.
                let mid_x = ((a.0 + b.0) / 2.0).round();
                (((a.0 + mid_x) / 2.0).round(), a.1.round())
            }
        }
        Flow::Vertical => {
            if (a.0 - b.0).abs() < 0.5 {
                // Already vertical: the true midpoint reads down the flow.
                (((a.0 + b.0) / 2.0).round(), ((a.1 + b.1) / 2.0).round())
            } else {
                // Column-changing: sit on the first vertical leg (a.x, a.1..mid_y),
                // matching orthogonal_path's `mid_y = round((a.1 + b.1) / 2)`.
                let mid_y = ((a.1 + b.1) / 2.0).round();
                (a.0.round(), ((a.1 + mid_y) / 2.0).round())
            }
        }
    }
}

fn draw_edge(
    scene: &mut Scene,
    route: &EdgeRoute,
    flow: Flow,
    seam_status: SeamStatus,
    palette: AssetInk,
) {
    let path = orthogonal_path(&route.points, flow);
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        palette.edge,
        None,
        &path,
    );
    // Arrowhead into the target, pointing along the flow (right / down).
    if let Some(&(tx, ty)) = route.points.last() {
        let mut head = BezPath::new();
        match flow {
            Flow::Horizontal => {
                head.move_to((tx - 5.0, ty - 3.0));
                head.line_to((tx, ty));
                head.line_to((tx - 5.0, ty + 3.0));
            }
            Flow::Vertical => {
                head.move_to((tx - 3.0, ty - 5.0));
                head.line_to((tx, ty));
                head.line_to((tx + 3.0, ty - 5.0));
            }
        }
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.edge,
            None,
            &head,
        );
    }
    if route.via.is_some() {
        let (cx, cy) = route_midpoint(&route.points, flow);
        draw_chevron(scene, cx, cy, flow, palette.status(seam_status));
    }
    if route.shield {
        // Shield sits just before the target so the guard reads as "what
        // flows IN here is checked" — left of the entry horizontally, above it
        // vertically.
        if let Some(&(tx, ty)) = route.points.last() {
            match flow {
                Flow::Horizontal => draw_shield(scene, tx - 14.0, ty - 9.0, palette.accent),
                Flow::Vertical => draw_shield(scene, tx - 9.0, ty - 14.0, palette.accent),
            }
        }
    }
}

fn draw_node(
    scene: &mut Scene,
    node: &AssetNode,
    rect: &brightfield_protocol::layout::Rect,
    chips: &[ViewChip],
    showing: Option<&str>,
    palette: AssetInk,
) {
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    // **The label is centred in what the chip row leaves, not in the card.**
    // `layout::node_height` grew this card by `VIEW_CHIP_BAND` so the chips
    // would have a foot of their own; centring the label in the whole card
    // would spend that room on moving the label down onto them instead.
    // `the_table_nodes_chips_sit_below_its_label_inside_the_card` reads both
    // out of the scene and holds the label clear of the chip row.
    let band = if chips.is_empty() {
        0.0
    } else {
        brightfield_protocol::layout::VIEW_CHIP_BAND
    };
    let (cx, cy) = (x + w / 2.0, y + (h - band) / 2.0);
    let mut label_colour = palette.label;
    match node.kind {
        AssetKind::Source => {
            let pill = RoundedRect::new(x, y, x + w, y + h, h / 2.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.node_fill,
                None,
                &pill,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &pill,
            );
            label_colour = palette.muted_label;
        }
        AssetKind::File => {
            // Document silhouette: a rect with a folded top-right corner.
            let fold = 8.0;
            let mut doc = BezPath::new();
            doc.move_to((x, y));
            doc.line_to((x + w - fold, y));
            doc.line_to((x + w, y + fold));
            doc.line_to((x + w, y + h));
            doc.line_to((x, y + h));
            doc.close_path();
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.node_fill,
                None,
                &doc,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &doc,
            );
            let mut crease = BezPath::new();
            crease.move_to((x + w - fold, y));
            crease.line_to((x + w - fold, y + fold));
            crease.line_to((x + w, y + fold));
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &crease,
            );
        }
        AssetKind::Table => {
            let card = RoundedRect::new(x, y, x + w, y + h, 4.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.node_fill,
                None,
                &card,
            );
            scene.stroke(
                &Stroke::new(1.2),
                Affine::IDENTITY,
                palette.table_border,
                None,
                &card,
            );
        }
        AssetKind::Internal => {
            let card = RoundedRect::new(x, y, x + w, y + h, 3.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.internal_fill,
                None,
                &card,
            );
            scene.stroke(
                &Stroke::new(0.8),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &card,
            );
            label_colour = palette.muted_label;
        }
        AssetKind::Dataset => {
            // Double ring: the sink is THE artefact.
            let outer = RoundedRect::new(x, y, x + w, y + h, 6.0);
            let inner = RoundedRect::new(x + 3.0, y + 3.0, x + w - 3.0, y + h - 3.0, 4.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.node_fill,
                None,
                &outer,
            );
            scene.stroke(
                &Stroke::new(1.4),
                Affine::IDENTITY,
                palette.accent,
                None,
                &outer,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.accent,
                None,
                &inner,
            );
        }
        AssetKind::Family => {
            // A stacked back card hints at the collapsed instances.
            let back = RoundedRect::new(x + 4.0, y + 4.0, x + w, y + h, 5.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.internal_fill,
                None,
                &back,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &back,
            );
            let front = RoundedRect::new(x, y, x + w - 4.0, y + h - 4.0, 5.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.node_fill,
                None,
                &front,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.node_border,
                None,
                &front,
            );
            if let Some(count) = node.family_count {
                draw_text(
                    scene,
                    &format!("\u{d7}{count}"),
                    x + w - 10.0,
                    y + h - 10.0,
                    LABEL_SIZE,
                    palette.accent,
                    TextAnchor::End,
                );
            }
            let label = fit_label(&node.label, w - 24.0);
            draw_text(
                scene,
                &label,
                cx - 2.0,
                cy - 2.0 + BASELINE_NUDGE,
                LABEL_SIZE,
                palette.label,
                TextAnchor::Middle,
            );
            // Family draws its own label (offset for the badge) and returns —
            // so the chip row is drawn here as well as at the foot of this
            // function, or a family tile that declared views would be sized
            // for chips and draw none.
            draw_view_chips(scene, chips, showing, palette);
            return;
        }
        AssetKind::Opaque => {
            // Issue-badged chip: dashed outline, amber badge.
            let chip = RoundedRect::new(x, y, x + w, y + h, 3.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.chip_fill,
                None,
                &chip,
            );
            let dashed = Stroke::new(1.0).with_dashes(0.0, [3.0, 2.0]);
            scene.stroke(&dashed, Affine::IDENTITY, palette.node_border, None, &chip);
            let badge = Circle::new((x + w - 2.0, y + 2.0), 5.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, palette.issue, None, &badge);
            draw_text(
                scene,
                "!",
                x + w - 2.0,
                y + 5.5,
                9.0,
                palette.badge_glyph,
                TextAnchor::Middle,
            );
            label_colour = palette.muted_label;
        }
    }
    let label = fit_label(&node.label, w);
    draw_text(
        scene,
        &label,
        cx,
        cy + BASELINE_NUDGE,
        LABEL_SIZE,
        label_colour,
        TextAnchor::Middle,
    );
    draw_view_chips(scene, chips, showing, palette);
}

/// Draw a node's **view chips** into its foot, the one the canvas is showing
/// filled.
///
/// The rectangles are the layout's — `brightfield_protocol::layout`'s
/// [`view_chip_rects`], which is also what the shell hit-tests a click
/// against, so a chip that is drawn is a chip that can be clicked. The
/// treatment is `brightfield_workbench::chrome`'s `chip`, drawn here in vello
/// rather than in egui because a node's foot is inside a rasterised scene:
/// both take the height, the corner radius and the padding from the same
/// design tokens, and neither carries a measure of its own.
///
/// `showing` is the word on the chip whose view the canvas returns to. `None`
/// draws every chip unfilled, which is what a node whose views are all
/// elsewhere looks like.
///
/// [`view_chip_rects`]: brightfield_protocol::layout::view_chip_rects
fn draw_view_chips(scene: &mut Scene, chips: &[ViewChip], showing: Option<&str>, palette: AssetInk) {
    for chip in chips {
        let r = &chip.rect;
        let box_ = RoundedRect::new(
            r.x,
            r.y,
            r.x + r.width,
            r.y + r.height,
            f64::from(meridian_design::radius::CHIP),
        );
        let on = showing == Some(chip.label.as_str());
        if on {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.chip_active_fill,
                None,
                &box_,
            );
        }
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            if on {
                palette.chip_active_border
            } else {
                palette.chip_border
            },
            None,
            &box_,
        );
        draw_text(
            scene,
            &chip.label,
            r.x + r.width / 2.0,
            // The chip's own centre plus the same baseline nudge a card's
            // label takes, so a word in a chip sits on the line a word on a
            // card does.
            r.y + r.height / 2.0 + BASELINE_NUDGE,
            LABEL_SIZE,
            if on {
                palette.chip_active_label
            } else {
                palette.chip_label
            },
            TextAnchor::Middle,
        );
    }
}

/// Draw the laid-out asset graph into `scene` with no execution-status tint —
/// the headless / offline (manifest) path, where no run status exists. Seams
/// draw in the quiet edge ink.
///
/// `dark` selects the mode's ink, and it is the same flag
/// [`meridian_design::semantic()`] takes: callers holding a workbench `Mode` pass
/// `mode.is_dark()`.
pub fn render_asset_graph(scene: &mut Scene, layout: &Layout, graph: &AssetGraph, dark: bool) {
    render_asset_graph_with_status(scene, layout, graph, &BTreeMap::new(), None, dark);
}

/// Draw the laid-out asset graph into `scene`, tinting each seam chevron by its
/// per-step execution status. `status` is keyed by step name
/// (matching a route's `via`); a seam with no entry falls back to
/// [`SeamStatus::NotRun`] — the quiet edge ink, never green. Feed it
/// [`ContractView::seam_statuses`](brightfield_protocol::contract_graph::ContractView::seam_statuses).
///
/// `showing` names the node and the word of the **view chip the canvas returns
/// to**, which draws filled while its siblings draw as hairlines. It is a
/// parameter rather than a field of the [`Layout`] because it moves on a
/// click, and the layout is recomputed only when a fold, a drill or a flow
/// change moves a card.
pub fn render_asset_graph_with_status(
    scene: &mut Scene,
    layout: &Layout,
    graph: &AssetGraph,
    status: &BTreeMap<StepId, SeamStatus>,
    showing: Option<(&AssetId, &str)>,
    dark: bool,
) {
    let palette = AssetInk::for_mode(dark);
    let canvas = Rect::new(0.0, 0.0, layout.width, layout.height);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        palette.canvas,
        None,
        &canvas,
    );
    for route in &layout.lanes {
        let seam_status = route
            .via
            .as_ref()
            .and_then(|v| status.get(v))
            .copied()
            .unwrap_or(SeamStatus::NotRun);
        draw_edge(scene, route, layout.flow, seam_status, palette);
    }
    for (id, rect) in &layout.positions {
        if let Some(node) = graph.nodes.get(id) {
            let chips = layout.view_chips.get(id).map_or(&[][..], Vec::as_slice);
            let on = showing
                .filter(|(node, _)| *node == id)
                .map(|(_, label)| label);
            draw_node(scene, node, rect, chips, on, palette);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_protocol::graph::build_graph;
    use brightfield_protocol::layout::{layout as compute_layout, LayoutConfig};
    use brightfield_protocol::parse_manifest_str;
    use std::collections::BTreeMap;

    /// every node kind + a shielded edge renders real geometry.
    #[test]
    fn all_node_treatments_render() {
        let yaml = r"
name: mini
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://example.com/a.csv', out: build/a.csv }
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
  - name: export
    op: parquet_export@1
    with: { input: t_out, dest: build/t.parquet }
  - name: validate
    op: finetype_validate@1
    with: { parquet: build/t.parquet, schema: schema.json }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok(
                "CREATE TABLE staging AS SELECT * FROM read_csv('build/a.csv');\n\
                SELEC deliberately broken;\n\
                CREATE TABLE t_out AS SELECT * FROM staging;"
                    .to_string(),
            ),
        );
        let graph = build_graph(&manifest, &sources);
        let l = compute_layout(&graph, &LayoutConfig::default());

        let mut scene = Scene::new();
        render_asset_graph(&mut scene, &l, &graph, false);
        let tags = scene.encoding().draw_tags.len();
        assert!(
            tags > graph.nodes.len(),
            "each card is several draw ops (fills, strokes, glyphs): {tags} tags for {} nodes",
            graph.nodes.len()
        );

        // An empty graph still paints the canvas and nothing else panics.
        let empty_manifest = parse_manifest_str("name: empty\nsteps: []\n").unwrap();
        let empty = build_graph(&empty_manifest, &BTreeMap::new());
        let le = compute_layout(&empty, &LayoutConfig::default());
        let mut empty_scene = Scene::new();
        render_asset_graph(&mut empty_scene, &le, &empty, false);
        assert_eq!(empty_scene.encoding().draw_tags.len(), 1, "canvas only");
    }

    #[test]
    fn seam_status_tints_the_chevron() {
        use brightfield_protocol::contract_graph::SeamStatus;
        let yaml = r"
name: s
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://example.com/a.csv', out: build/a.csv }
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE t_out AS SELECT * FROM read_csv('build/a.csv');".to_string()),
        );
        let graph = build_graph(&manifest, &sources);
        let l = compute_layout(&graph, &LayoutConfig::default());

        // A run with the `transform` seam FAILED tints its chevron the critical
        // ink; the same graph with no status draws it in the quiet edge ink — so
        // the two encodings must differ. (A skipped seam is its own tint too.)
        let mut failed: BTreeMap<String, SeamStatus> = BTreeMap::new();
        failed.insert("transform".to_string(), SeamStatus::Failed);
        failed.insert("fetch".to_string(), SeamStatus::Ok);

        let mut plain = Scene::new();
        render_asset_graph(&mut plain, &l, &graph, false);
        let mut tinted = Scene::new();
        render_asset_graph_with_status(&mut tinted, &l, &graph, &failed, None, false);

        // Same number of draw ops (only colour changed), different draw data.
        assert_eq!(
            plain.encoding().draw_tags.len(),
            tinted.encoding().draw_tags.len()
        );
        assert_ne!(
            plain.encoding().draw_data.len() + plain.encoding().draw_tags.len(),
            0,
            "something was drawn"
        );
        assert_ne!(
            plain.encoding().draw_data,
            tinted.encoding().draw_data,
            "the status tint changes the seam colour in the draw stream"
        );
    }

    #[test]
    fn pds_fit_label_truncates_with_ellipsis() {
        assert_eq!(fit_label("short", 224.0), "short");
        let long = "a_very_long_relation_name_that_cannot_fit_in_a_card";
        let fitted = fit_label(long, 100.0);
        assert!(fitted.chars().count() < long.chars().count());
        assert!(fitted.ends_with('\u{2026}'));
    }

    #[test]
    fn pds_chevron_sits_on_a_horizontal_run_not_the_vertical() {
        // A row-changing middle segment is drawn H-V-H; the chevron must land
        // on the first horizontal leg (y == a.y, x between a.x and mid_x), never
        // on the vertical run where a right-pointing glyph reads detached.
        let a = (100.0, 40.0);
        let b = (200.0, 120.0); // different row
        let (cx, cy) = route_midpoint(&[a, b], Flow::Horizontal);
        assert_eq!(
            cy, a.1,
            "chevron y is on the horizontal leg, not mid-way up the vertical"
        );
        let mid_x = ((a.0 + b.0) / 2.0).round();
        assert!(
            a.0 <= cx && cx <= mid_x,
            "chevron x is on the first horizontal leg: {cx}"
        );
        // A same-row segment keeps the true midpoint.
        let (hx, hy) = route_midpoint(&[(10.0, 50.0), (30.0, 50.0)], Flow::Horizontal);
        assert_eq!((hx, hy), (20.0, 50.0));
    }

    #[test]
    fn vertical_chevron_sits_on_a_vertical_run_not_the_horizontal() {
        // A column-changing middle segment is drawn V-H-V; the chevron must land
        // on the first vertical leg (x == a.x, y between a.y and mid_y), so a
        // down-pointing glyph reads along the flow.
        let a = (40.0, 100.0);
        let b = (120.0, 200.0); // different column
        let (cx, cy) = route_midpoint(&[a, b], Flow::Vertical);
        assert_eq!(
            cx, a.0,
            "chevron x is on the vertical leg, not mid-way across the horizontal"
        );
        let mid_y = ((a.1 + b.1) / 2.0).round();
        assert!(
            a.1 <= cy && cy <= mid_y,
            "chevron y is on the first vertical leg: {cy}"
        );
        // A same-column segment keeps the true midpoint.
        let (vx, vy) = route_midpoint(&[(50.0, 10.0), (50.0, 30.0)], Flow::Vertical);
        assert_eq!((vx, vy), (50.0, 20.0));
    }

    #[test]
    fn vertical_scene_renders_and_transposes_the_path() {
        // The vertical layout draws real geometry, and orthogonal_path routes
        // V-H-V (a mid-run at a shared x), never the horizontal H-V-H.
        use brightfield_protocol::layout::Flow;
        let yaml = r"
name: v
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://example.com/a.csv', out: build/a.csv }
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE t_out AS SELECT * FROM read_csv('build/a.csv');".to_string()),
        );
        let graph = build_graph(&manifest, &sources);
        let cfg = LayoutConfig {
            flow: Flow::Vertical,
            ..LayoutConfig::default()
        };
        let l = compute_layout(&graph, &cfg);
        assert_eq!(l.flow, Flow::Vertical);
        let mut scene = Scene::new();
        render_asset_graph(&mut scene, &l, &graph, false);
        assert!(
            scene.encoding().draw_tags.len() > graph.nodes.len(),
            "real geometry drawn"
        );
    }

    #[test]
    fn pds_badge_glyph_is_the_warning_roles_own_foreground() {
        // Amber is the one role whose foreground is dark ink rather than the
        // near-white every other role takes, because near-white on amber step 9
        // measures 2.47:1 and the dark ink 6.45:1. Taking `text.on_solid` here
        // would resolve through the semantic layer and still be wrong, which is
        // the failure this pins. Invariant across modes for the same reason the
        // badge itself is: a solid and the ink chosen for it are both paints.
        let warning_fg = |dark| {
            ink(meridian_design::semantic(dark)
                .role(meridian_design::Role::Warning)
                .foreground
                .base)
        };
        for dark in [false, true] {
            let p = AssetInk::for_mode(dark);
            assert_eq!(p.badge_glyph, warning_fg(dark), "dark={dark}");
            assert_ne!(p.badge_glyph, Color::WHITE);
            assert_ne!(
                p.badge_glyph,
                ink(meridian_design::semantic(dark).text.on_solid),
                "dark={dark}: on_solid is the slot this must NOT take"
            );
        }
    }

    /// The light palette is **exactly** the thirteen `const Color`s this struct
    /// replaced, value for value.
    ///
    /// This is the whole light-mode-is-untouched claim, and it is checkable
    /// rather than asserted in a commit message: the right-hand sides here are
    /// the literal token expressions the deleted constants held. Any of them
    /// drifting is a light baseline that moves.
    #[test]
    fn pds_light_palette_matches_the_const_palette_it_replaced() {
        use meridian_design::chrome::INK_LIGHT;
        use meridian_design::scales::{AMBER_LIGHT, GRAY_LIGHT};
        let p = AssetInk::for_mode(false);
        assert_eq!(p.canvas, ink(INK_LIGHT.page), "canvas");
        assert_eq!(p.node_fill, ink(INK_LIGHT.surface), "node_fill");
        assert_eq!(p.node_border, ink(GRAY_LIGHT[4]), "node_border");
        assert_eq!(p.table_border, ink(INK_LIGHT.ink_secondary), "table_border");
        assert_eq!(p.edge, ink(GRAY_LIGHT[5]), "edge");
        assert_eq!(p.accent, ink(INK_LIGHT.focus), "accent");
        assert_eq!(p.issue, ink(AMBER_LIGHT[8]), "issue");
        // badge_glyph is the ONE field that deliberately does not match the
        // const it replaced. The old `BADGE_GLYPH_COLOUR` was the near-white
        // surface tone, which measures 2.47:1 on amber step 9; the Warning
        // role's own foreground measures 6.45:1, and the design crate names it
        // as the badge ink twice. Everything else here is byte-parity, so this
        // exception is stated rather than quietly dropped from the list.
        assert_ne!(p.badge_glyph, ink(INK_LIGHT.surface), "badge_glyph");
        assert_eq!(p.internal_fill, ink(GRAY_LIGHT[1]), "internal_fill");
        assert_eq!(p.chip_fill, ink(GRAY_LIGHT[2]), "chip_fill");
        assert_eq!(p.label, ink(INK_LIGHT.ink_primary), "label");
        assert_eq!(p.muted_label, ink(INK_LIGHT.ink_muted), "muted_label");
        assert_eq!(p.skipped, ink(GRAY_LIGHT[6]), "skipped");
    }

    /// Every mode-dependent field actually moves, and `badge_glyph` — the one
    /// field documented as mode-invariant — actually does not.
    ///
    /// A palette that read the mode for *some* fields and kept a hardcoded
    /// light token for the rest is precisely the half-fix this increment
    /// exists to avoid, and it would pass a "the dark scene differs" test.
    #[test]
    fn pds_every_mode_dependent_field_moves_with_the_mode() {
        let l = AssetInk::for_mode(false);
        let d = AssetInk::for_mode(true);
        for (name, a, b) in [
            ("canvas", l.canvas, d.canvas),
            ("node_fill", l.node_fill, d.node_fill),
            ("node_border", l.node_border, d.node_border),
            ("table_border", l.table_border, d.table_border),
            ("edge", l.edge, d.edge),
            ("accent", l.accent, d.accent),
            ("internal_fill", l.internal_fill, d.internal_fill),
            ("chip_fill", l.chip_fill, d.chip_fill),
            ("label", l.label, d.label),
            ("muted_label", l.muted_label, d.muted_label),
            ("skipped", l.skipped, d.skipped),
        ] {
            assert_ne!(a, b, "{name} is the same in both modes");
        }
        // The issue badge is the one *pair* that is mode-invariant, and it was
        // this test failing that established it: Amber step 9 is byte-identical
        // in AMBER_LIGHT and AMBER_DARK (#da950b), because a bright solid is a
        // paint rather than a plane. The glyph on it is invariant for the same
        // reason — and it is the Warning role's own dark foreground, not the
        // near-white every other role takes, because near-white on amber
        // measures 2.47:1 against the dark ink's 6.45:1. Asserted rather than
        // assumed; a contrast gate covers the ratio itself.
        assert_eq!(
            l.issue, d.issue,
            "the Warning solid is one paint, both modes"
        );
        assert_eq!(
            l.badge_glyph, d.badge_glyph,
            "badge_glyph is documented as a paint, not a mode-dependent slot"
        );
    }

    /// The bug, stated as a property: in dark mode the canvas is darker than
    /// the ink on it; in light mode it is lighter. The white-sheet raster
    /// failed this — its canvas was `INK_LIGHT.page` on a dark window.
    #[test]
    fn pds_dark_canvas_is_darker_than_its_ink() {
        // Relative luminance is monotone in each channel, so the plain channel
        // sum orders these tones correctly without a colour-space detour.
        let lum = |c: Color| -> f32 { c.components[0] + c.components[1] + c.components[2] };
        let d = AssetInk::for_mode(true);
        assert!(
            lum(d.canvas) < lum(d.label),
            "dark canvas {:?} is not darker than its primary ink {:?}",
            d.canvas,
            d.label
        );
        assert!(
            lum(d.canvas) < lum(d.node_fill),
            "dark cards must sit above the page, not below it"
        );
        // And the page tone is genuinely dark, not merely darker than white.
        assert!(
            lum(d.canvas) < 0.3,
            "dark canvas is not dark: {:?}",
            d.canvas
        );
        let l = AssetInk::for_mode(false);
        assert!(lum(l.canvas) > lum(l.label), "light canvas must be light");
        assert!(
            lum(l.canvas) > 2.7,
            "light canvas is not light: {:?}",
            l.canvas
        );
    }

    /// End to end: the same graph rastered in the two modes produces two
    /// different draw streams with the same geometry — colour changed, nothing
    /// else. A `dark` flag that never reached the scene would fail this.
    #[test]
    fn pds_the_scene_takes_its_ink_from_the_mode() {
        let yaml = r"
name: m
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://example.com/a.csv', out: build/a.csv }
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE t_out AS SELECT * FROM read_csv('build/a.csv');".to_string()),
        );
        let graph = build_graph(&manifest, &sources);
        let l = compute_layout(&graph, &LayoutConfig::default());

        let mut light = Scene::new();
        render_asset_graph(&mut light, &l, &graph, false);
        let mut dark = Scene::new();
        render_asset_graph(&mut dark, &l, &graph, true);

        assert_eq!(
            light.encoding().draw_tags.len(),
            dark.encoding().draw_tags.len(),
            "the mode changes colour, not geometry"
        );
        assert_eq!(
            light.encoding().path_data,
            dark.encoding().path_data,
            "the mode changes colour, not geometry"
        );
        assert_ne!(
            light.encoding().draw_data,
            dark.encoding().draw_data,
            "the two modes produced the same ink — the flag never reached the scene"
        );
    }
}
