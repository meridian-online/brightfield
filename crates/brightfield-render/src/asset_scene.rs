//! Asset-graph scene builder (card 0025) — draws a `brightfield-protocol`
//! layout into a vello scene.
//!
//! The protocol DAG twin of `scene.rs`: same idiom (Meridian ink via the
//! `ink` boundary, labels through the `text` module, plain kurbo shapes),
//! but the input is an [`AssetGraph`] + [`Layout`] instead of data batches.
//! Node treatments per kind (pds-ac05): SOURCE pill, FILE document
//! silhouette, TABLE card, INTERNAL muted card, DATASET double-ring, family
//! tile with an `xN` count, opaque chip with an issue badge. Steps render as
//! seam chevrons on their edges; a validation gate is a shield glyph on the
//! guarded edge, never a node. Edges route orthogonally along the layout's
//! dummy-node lanes.

use kurbo::{Affine, BezPath, Circle, Rect, RoundedRect, Stroke};
use peniko::{Color, Fill};
use vello::Scene;

use std::collections::BTreeMap;

use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_protocol::graph::{AssetGraph, AssetKind, AssetNode, StepId};
use brightfield_protocol::layout::{EdgeRoute, Flow, Layout};

use crate::ink::ink;
use crate::text::{draw_text, TextAnchor, LABEL_SIZE};

/// Canvas behind the graph — the Meridian page tone, one step warmer than
/// the node cards so cards read as cards.
const CANVAS_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.page);
/// Node card fill — the chart surface.
const NODE_FILL: Color = ink(meridian_design::chrome::INK_LIGHT.surface);
/// Quiet card border (sources, files).
const NODE_BORDER: Color = ink(meridian_design::scales::GRAY_LIGHT[4]);
/// Stronger border for TABLE cards.
const TABLE_BORDER: Color = ink(meridian_design::chrome::INK_LIGHT.ink_secondary);
/// Edge ink.
const EDGE_COLOUR: Color = ink(meridian_design::scales::GRAY_LIGHT[5]);
/// Accent (Dataset double-ring, shield, family count) — Maritime focus ink.
const ACCENT_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.focus);
/// Issue badge on an opaque chip.
const ISSUE_COLOUR: Color = ink(meridian_design::scales::AMBER_LIGHT[8]);
/// Glyph drawn on the amber issue badge — the light surface tone through the
/// ink() boundary (not an ad-hoc `Color::WHITE`), so a future dark-ink theme
/// pass tracks it with every other chrome constant.
const BADGE_GLYPH_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.surface);
/// Muted fill for INTERNAL statement intermediates.
const INTERNAL_FILL: Color = ink(meridian_design::scales::GRAY_LIGHT[1]);
/// Chip fill for degraded statements.
const CHIP_FILL: Color = ink(meridian_design::scales::GRAY_LIGHT[2]);
/// Primary label ink.
const LABEL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_primary);
/// Muted label ink (internal/chip labels).
const MUTED_LABEL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_muted);
/// Execution-status tints for a seam (card 0029) — the reserved Meridian status
/// inks, never reused as series colour. `NotRun` keeps the quiet edge ink so an
/// unrun seam is never green.
const STATUS_OK: Color = ink(meridian_design::viz::STATUS.good);
const STATUS_RUNNING: Color = ink(meridian_design::viz::STATUS.warning);
const STATUS_SKIPPED: Color = ink(meridian_design::scales::GRAY_LIGHT[6]);
const STATUS_FAILED: Color = ink(meridian_design::viz::STATUS.critical);

/// The ink a seam chevron takes for its execution status — the two-channel rule
/// (execution status is its own colour channel). `NotRun` falls back to the
/// quiet edge ink, so unmeasured never reads green.
fn status_ink(status: SeamStatus) -> Color {
    match status {
        SeamStatus::Ok => STATUS_OK,
        SeamStatus::Running => STATUS_RUNNING,
        SeamStatus::Skipped => STATUS_SKIPPED,
        SeamStatus::Failed => STATUS_FAILED,
        SeamStatus::NotRun => EDGE_COLOUR,
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

/// The gate-as-shield glyph near the guarded edge's target (pds-ac05).
fn draw_shield(scene: &mut Scene, cx: f64, cy: f64) {
    let (w, h) = (9.0, 11.0);
    let mut shield = BezPath::new();
    shield.move_to((cx - w / 2.0, cy - h / 2.0));
    shield.line_to((cx + w / 2.0, cy - h / 2.0));
    shield.line_to((cx + w / 2.0, cy + h / 6.0));
    shield.line_to((cx, cy + h / 2.0));
    shield.line_to((cx - w / 2.0, cy + h / 6.0));
    shield.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, ACCENT_COLOUR, None, &shield);
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

fn draw_edge(scene: &mut Scene, route: &EdgeRoute, flow: Flow, seam_status: SeamStatus) {
    let path = orthogonal_path(&route.points, flow);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, EDGE_COLOUR, None, &path);
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
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, EDGE_COLOUR, None, &head);
    }
    if route.via.is_some() {
        let (cx, cy) = route_midpoint(&route.points, flow);
        draw_chevron(scene, cx, cy, flow, status_ink(seam_status));
    }
    if route.shield {
        // Shield sits just before the target so the guard reads as "what
        // flows IN here is checked" — left of the entry horizontally, above it
        // vertically.
        if let Some(&(tx, ty)) = route.points.last() {
            match flow {
                Flow::Horizontal => draw_shield(scene, tx - 14.0, ty - 9.0),
                Flow::Vertical => draw_shield(scene, tx - 9.0, ty - 14.0),
            }
        }
    }
}

fn draw_node(
    scene: &mut Scene,
    node: &AssetNode,
    rect: &brightfield_protocol::layout::Rect,
) {
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let mut label_colour = LABEL_COLOUR;
    match node.kind {
        AssetKind::Source => {
            let pill = RoundedRect::new(x, y, x + w, y + h, h / 2.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, NODE_FILL, None, &pill);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, NODE_BORDER, None, &pill);
            label_colour = MUTED_LABEL_COLOUR;
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
            scene.fill(Fill::NonZero, Affine::IDENTITY, NODE_FILL, None, &doc);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, NODE_BORDER, None, &doc);
            let mut crease = BezPath::new();
            crease.move_to((x + w - fold, y));
            crease.line_to((x + w - fold, y + fold));
            crease.line_to((x + w, y + fold));
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, NODE_BORDER, None, &crease);
        }
        AssetKind::Table => {
            let card = RoundedRect::new(x, y, x + w, y + h, 4.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, NODE_FILL, None, &card);
            scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, TABLE_BORDER, None, &card);
        }
        AssetKind::Internal => {
            let card = RoundedRect::new(x, y, x + w, y + h, 3.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, INTERNAL_FILL, None, &card);
            scene.stroke(&Stroke::new(0.8), Affine::IDENTITY, NODE_BORDER, None, &card);
            label_colour = MUTED_LABEL_COLOUR;
        }
        AssetKind::Dataset => {
            // Double ring: the sink is THE artefact.
            let outer = RoundedRect::new(x, y, x + w, y + h, 6.0);
            let inner = RoundedRect::new(x + 3.0, y + 3.0, x + w - 3.0, y + h - 3.0, 4.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, NODE_FILL, None, &outer);
            scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, ACCENT_COLOUR, None, &outer);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, ACCENT_COLOUR, None, &inner);
        }
        AssetKind::Family => {
            // A stacked back card hints at the collapsed instances.
            let back = RoundedRect::new(x + 4.0, y + 4.0, x + w, y + h, 5.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, INTERNAL_FILL, None, &back);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, NODE_BORDER, None, &back);
            let front = RoundedRect::new(x, y, x + w - 4.0, y + h - 4.0, 5.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, NODE_FILL, None, &front);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, NODE_BORDER, None, &front);
            if let Some(count) = node.family_count {
                draw_text(
                    scene,
                    &format!("\u{d7}{count}"),
                    x + w - 10.0,
                    y + h - 10.0,
                    LABEL_SIZE,
                    ACCENT_COLOUR,
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
                LABEL_COLOUR,
                TextAnchor::Middle,
            );
            return; // family draws its own label (offset for the badge)
        }
        AssetKind::Opaque => {
            // Issue-badged chip: dashed outline, amber badge (pds-ac04).
            let chip = RoundedRect::new(x, y, x + w, y + h, 3.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, CHIP_FILL, None, &chip);
            let dashed = Stroke::new(1.0).with_dashes(0.0, [3.0, 2.0]);
            scene.stroke(&dashed, Affine::IDENTITY, NODE_BORDER, None, &chip);
            let badge = Circle::new((x + w - 2.0, y + 2.0), 5.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, ISSUE_COLOUR, None, &badge);
            draw_text(
                scene,
                "!",
                x + w - 2.0,
                y + 5.5,
                9.0,
                BADGE_GLYPH_COLOUR,
                TextAnchor::Middle,
            );
            label_colour = MUTED_LABEL_COLOUR;
        }
    }
    let label = fit_label(&node.label, w);
    draw_text(scene, &label, cx, cy + BASELINE_NUDGE, LABEL_SIZE, label_colour, TextAnchor::Middle);
}

/// Draw the laid-out asset graph into `scene` with no execution-status tint —
/// the headless / offline (manifest) path, where no run status exists. Seams
/// draw in the quiet edge ink.
pub fn render_asset_graph(scene: &mut Scene, layout: &Layout, graph: &AssetGraph) {
    render_asset_graph_with_status(scene, layout, graph, &BTreeMap::new());
}

/// Draw the laid-out asset graph into `scene`, tinting each seam chevron by its
/// per-step execution status (card 0029). `status` is keyed by step name
/// (matching a route's `via`); a seam with no entry falls back to
/// [`SeamStatus::NotRun`] — the quiet edge ink, never green. Feed it
/// [`ContractView::seam_statuses`](brightfield_protocol::contract_graph::ContractView::seam_statuses).
pub fn render_asset_graph_with_status(
    scene: &mut Scene,
    layout: &Layout,
    graph: &AssetGraph,
    status: &BTreeMap<StepId, SeamStatus>,
) {
    let canvas = Rect::new(0.0, 0.0, layout.width, layout.height);
    scene.fill(Fill::NonZero, Affine::IDENTITY, CANVAS_COLOUR, None, &canvas);
    for route in &layout.lanes {
        let seam_status = route
            .via
            .as_ref()
            .and_then(|v| status.get(v))
            .copied()
            .unwrap_or(SeamStatus::NotRun);
        draw_edge(scene, route, layout.flow, seam_status);
    }
    for (id, rect) in &layout.positions {
        if let Some(node) = graph.nodes.get(id) {
            draw_node(scene, node, rect);
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

    /// pds-ac05: every node kind + a shielded edge renders real geometry.
    #[test]
    fn pds_ac05_all_node_treatments_render() {
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
    with: { parquet: build/t.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE staging AS SELECT * FROM read_csv('build/a.csv');\n\
                SELEC deliberately broken;\n\
                CREATE TABLE t_out AS SELECT * FROM staging;"
                .to_string()),
        );
        let graph = build_graph(&manifest, &sources);
        let l = compute_layout(&graph, &LayoutConfig::default());

        let mut scene = Scene::new();
        render_asset_graph(&mut scene, &l, &graph);
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
        render_asset_graph(&mut empty_scene, &le, &empty);
        assert_eq!(empty_scene.encoding().draw_tags.len(), 1, "canvas only");
    }

    #[test]
    fn t29_seam_status_tints_the_chevron() {
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
        render_asset_graph(&mut plain, &l, &graph);
        let mut tinted = Scene::new();
        render_asset_graph_with_status(&mut tinted, &l, &graph, &failed);

        // Same number of draw ops (only colour changed), different draw data.
        assert_eq!(plain.encoding().draw_tags.len(), tinted.encoding().draw_tags.len());
        assert_ne!(
            plain.encoding().draw_data.len() + plain.encoding().draw_tags.len(),
            0,
            "something was drawn"
        );
        assert_ne!(
            plain.encoding().draw_data, tinted.encoding().draw_data,
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
        assert_eq!(cy, a.1, "chevron y is on the horizontal leg, not mid-way up the vertical");
        let mid_x = ((a.0 + b.0) / 2.0).round();
        assert!(a.0 <= cx && cx <= mid_x, "chevron x is on the first horizontal leg: {cx}");
        // A same-row segment keeps the true midpoint.
        let (hx, hy) = route_midpoint(&[(10.0, 50.0), (30.0, 50.0)], Flow::Horizontal);
        assert_eq!((hx, hy), (20.0, 50.0));
    }

    #[test]
    fn t29_vertical_chevron_sits_on_a_vertical_run_not_the_horizontal() {
        // A column-changing middle segment is drawn V-H-V; the chevron must land
        // on the first vertical leg (x == a.x, y between a.y and mid_y), so a
        // down-pointing glyph reads along the flow.
        let a = (40.0, 100.0);
        let b = (120.0, 200.0); // different column
        let (cx, cy) = route_midpoint(&[a, b], Flow::Vertical);
        assert_eq!(cx, a.0, "chevron x is on the vertical leg, not mid-way across the horizontal");
        let mid_y = ((a.1 + b.1) / 2.0).round();
        assert!(a.1 <= cy && cy <= mid_y, "chevron y is on the first vertical leg: {cy}");
        // A same-column segment keeps the true midpoint.
        let (vx, vy) = route_midpoint(&[(50.0, 10.0), (50.0, 30.0)], Flow::Vertical);
        assert_eq!((vx, vy), (50.0, 20.0));
    }

    #[test]
    fn t29_vertical_scene_renders_and_transposes_the_path() {
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
        let cfg = LayoutConfig { flow: Flow::Vertical, ..LayoutConfig::default() };
        let l = compute_layout(&graph, &cfg);
        assert_eq!(l.flow, Flow::Vertical);
        let mut scene = Scene::new();
        render_asset_graph(&mut scene, &l, &graph);
        assert!(scene.encoding().draw_tags.len() > graph.nodes.len(), "real geometry drawn");
    }

    #[test]
    fn pds_badge_glyph_is_an_ink_token_not_raw_white() {
        // The issue-badge glyph colour goes through the meridian-design ink()
        // boundary, not an ad-hoc peniko Color::WHITE.
        assert_eq!(BADGE_GLYPH_COLOUR, ink(meridian_design::chrome::INK_LIGHT.surface));
        assert_ne!(BADGE_GLYPH_COLOUR, Color::WHITE);
    }
}
