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

use brightfield_protocol::graph::{AssetGraph, AssetKind, AssetNode};
use brightfield_protocol::layout::{EdgeRoute, Layout};

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
/// Muted fill for INTERNAL statement intermediates.
const INTERNAL_FILL: Color = ink(meridian_design::scales::GRAY_LIGHT[1]);
/// Chip fill for degraded statements.
const CHIP_FILL: Color = ink(meridian_design::scales::GRAY_LIGHT[2]);
/// Primary label ink.
const LABEL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_primary);
/// Muted label ink (internal/chip labels).
const MUTED_LABEL_COLOUR: Color = ink(meridian_design::chrome::INK_LIGHT.ink_muted);
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

/// Orthogonal polyline through the route's waypoints: each hop runs
/// horizontally to the midpoint x, vertically to the next row, then
/// horizontally in — the lane discipline that keeps bundles readable.
fn orthogonal_path(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    let Some(&(x0, y0)) = points.first() else {
        return path;
    };
    path.move_to((x0, y0));
    let mut prev = (x0, y0);
    for &(x, y) in &points[1..] {
        if (y - prev.1).abs() < 0.5 {
            path.line_to((x, y));
        } else {
            let mid_x = ((prev.0 + x) / 2.0).round();
            path.line_to((mid_x, prev.1));
            path.line_to((mid_x, y));
            path.line_to((x, y));
        }
        prev = (x, y);
    }
    path
}

/// A small right-pointing double chevron at (`cx`, `cy`) — the seam glyph.
fn draw_chevron(scene: &mut Scene, cx: f64, cy: f64) {
    for offset in [-3.0, 2.0] {
        let mut chevron = BezPath::new();
        chevron.move_to((cx + offset - 2.0, cy - 3.5));
        chevron.line_to((cx + offset + 2.0, cy));
        chevron.line_to((cx + offset - 2.0, cy + 3.5));
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, EDGE_COLOUR, None, &chevron);
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

/// Midpoint of the route's middle segment — the chevron site.
fn route_midpoint(points: &[(f64, f64)]) -> (f64, f64) {
    let seg = (points.len() - 1) / 2;
    let (a, b) = (points[seg], points[seg + 1]);
    (((a.0 + b.0) / 2.0).round(), ((a.1 + b.1) / 2.0).round())
}

fn draw_edge(scene: &mut Scene, route: &EdgeRoute) {
    let path = orthogonal_path(&route.points);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, EDGE_COLOUR, None, &path);
    // Arrowhead into the target.
    if let Some(&(tx, ty)) = route.points.last() {
        let mut head = BezPath::new();
        head.move_to((tx - 5.0, ty - 3.0));
        head.line_to((tx, ty));
        head.line_to((tx - 5.0, ty + 3.0));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, EDGE_COLOUR, None, &head);
    }
    if route.via.is_some() {
        let (cx, cy) = route_midpoint(&route.points);
        draw_chevron(scene, cx, cy);
    }
    if route.shield {
        // Shield sits just before the target so the guard reads as "what
        // flows IN here is checked".
        if let Some(&(tx, ty)) = route.points.last() {
            draw_shield(scene, tx - 14.0, ty - 9.0);
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
                Color::WHITE,
                TextAnchor::Middle,
            );
            label_colour = MUTED_LABEL_COLOUR;
        }
    }
    let label = fit_label(&node.label, w);
    draw_text(scene, &label, cx, cy + BASELINE_NUDGE, LABEL_SIZE, label_colour, TextAnchor::Middle);
}

/// Draw the laid-out asset graph into `scene`: canvas, edges (with seam
/// chevrons and gate shields), then node cards on top.
pub fn render_asset_graph(scene: &mut Scene, layout: &Layout, graph: &AssetGraph) {
    let canvas = Rect::new(0.0, 0.0, layout.width, layout.height);
    scene.fill(Fill::NonZero, Affine::IDENTITY, CANVAS_COLOUR, None, &canvas);
    for route in &layout.lanes {
        draw_edge(scene, route);
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
    fn pds_fit_label_truncates_with_ellipsis() {
        assert_eq!(fit_label("short", 224.0), "short");
        let long = "a_very_long_relation_name_that_cannot_fit_in_a_card";
        let fitted = fit_label(long, 100.0);
        assert!(fitted.chars().count() < long.chars().count());
        assert!(fitted.ends_with('\u{2026}'));
    }
}
