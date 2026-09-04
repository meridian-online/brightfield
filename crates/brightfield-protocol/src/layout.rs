//! Deterministic Sugiyama layout.
//!
//! Hand-ported, dependency-free: longest-path layering left -> right, dummy
//! nodes lane edges that span more than one layer, crossing reduction is
//! exactly **4 median sweeps** (fixed — never iterate-to-convergence), and
//! every tie breaks by node id (lexicographic) — never by insertion or hash
//! order. `BTreeMap`/`BTreeSet`/`Vec` end-to-end and coordinates quantised
//! to whole pixels, so the same graph always yields the identical `Layout`.
//!
//! No text measurement here: card widths come from char-count heuristics
//! (geometry before pixels) so this crate stays vello-free.

use std::collections::{BTreeMap, BTreeSet};

use meridian_design::spacing;

use crate::graph::{AssetGraph, AssetId, AssetKind, StepId};

/// A node's placed rectangle, whole pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// One routed edge: polyline waypoints from the source's right edge through
/// each dummy lane point to the target's left edge (the renderer draws the
/// orthogonal path along them).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRoute {
    /// Upstream node id.
    pub from: AssetId,
    /// Downstream node id.
    pub to: AssetId,
    /// The seam the flow passes through (chevron site), if any.
    pub via: Option<StepId>,
    /// Gate shield on this edge.
    pub shield: bool,
    /// Waypoints, whole pixels.
    pub points: Vec<(f64, f64)>,
}

/// The layout result.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Canvas width in pixels.
    pub width: f64,
    /// Canvas height in pixels.
    pub height: f64,
    /// Placed rectangles for every REAL node (dummies stay internal).
    pub positions: BTreeMap<AssetId, Rect>,
    /// Routed edges along the dummy-node lanes.
    pub lanes: Vec<EdgeRoute>,
    /// The direction the DAG was laid out — the renderer routes edges (and
    /// points seam chevrons) along this axis.
    pub flow: Flow,
    /// The view chips placed in each node's foot, for the nodes
    /// [`LayoutConfig::view_chips`] named. Empty for every node that declared
    /// no views, which is every node of a Protocol read from a manifest.
    pub view_chips: BTreeMap<AssetId, Vec<ViewChip>>,
}

/// Which way the layers progress — the reading axis of the DAG.
///
/// The layering + crossing-reduction passes are orientation-agnostic and run
/// verbatim; only the final coordinate assignment (and the renderer's edge
/// routing) transposes. `Horizontal` is the wide overview/export render;
/// `Vertical` puts the long axis on natural scroll — the right reading inside a
/// dock pane and the shape web's `/protocols` spine reads top-to-bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flow {
    /// Layers → columns, left → right; nodes stack vertically within a layer.
    #[default]
    Horizontal,
    /// Layers → rows, top → bottom; nodes spread horizontally within a layer.
    Vertical,
}

/// A dummy lane slot's extent, the same measure both ways.
///
/// **Across** the flow it is the lane's width — the channel one routed polyline
/// occupies inside a rank, and what the crossing reduction packs sibling lanes
/// at. That is what it always was.
///
/// **Along** the flow it used to be zero, with a blanket `48.0` floor under
/// every rank's thickness standing in for it (*"floored so a sparse layer still
/// reserves a lane"*). That floor was taller than every card this model draws —
/// the tallest, a `Family` tile, is 46 — so it applied to **every** rank rather
/// than to the sparse ones it was written for: on the collapsed crosswalk it
/// reserved 118 points, 10.5% of the canvas, for nothing at all.
///
/// So the reservation moved onto the thing that needs it. A rank's thickness is
/// now a plain max over what its slots declare, and a rank holding only dummy
/// lanes declares this.
///
/// **Derived, not chosen.** A lane declares exactly one measure — its width —
/// and taking the same along the flow makes a lane-only rank exactly as thick as
/// the lanes crossing it are wide. Checked against the ink the renderer puts on
/// a lane hop: `asset_scene::orthogonal_path` draws each hop as a dogleg out to
/// the hop's midpoint and back, so the run either side of the crossbar is half
/// the lane-point separation — `(LANE_EXTENT + col_gap) / 2` = 21 points between
/// two lane-only ranks — against the 9 points the seam chevron occupies along
/// the flow (`draw_chevron` puts two arrows at offsets −3 and +2, each ±2 about
/// its offset: −5…+4). Twice the glyph, so the hop still reads as a step.
///
/// **It does not bind on any graph this crate can build today, and that is a
/// property of the layering rather than of this number.** Longest-path layering
/// gives every node at layer `L > 0` a predecessor at `L − 1`, so every rank
/// from 0 to the deepest holds at least one real card, and the shortest card
/// (`Internal`/`Opaque`, 26) is taller than a lane. The value is here so that a
/// layering which *can* produce a lane-only rank still reserves the lane's own
/// channel rather than collapsing it to a line.
const LANE_EXTENT: f64 = 10.0;

/// Tunable spacing; the default is the dump arm's configuration.
///
/// # The three gaps are design-system spacing, not layout-local numbers
///
/// [`margin`](LayoutConfig::margin), [`col_gap`](LayoutConfig::col_gap) and
/// [`row_gap`](LayoutConfig::row_gap) shipped as bare floats — `32.0`, `64.0`,
/// `20.0` — and two of the three were off the Meridian spacing ladder
/// altogether: the ladder tops out at [`spacing::SPACE_9`] (48), so a 64-point
/// gap was a fourth spacing vocabulary in a product that has one. They are drawn
/// from the ladder now, and `pds_the_spacing_defaults_are_ladder_steps` holds
/// them there.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    /// Canvas margin on all sides.
    pub margin: f64,
    /// Gap between layers along the flow axis (columns horizontally, rows
    /// vertically).
    pub col_gap: f64,
    /// Gap between sibling nodes across the flow axis (down a column
    /// horizontally, along a row vertically).
    pub row_gap: f64,
    /// The direction the DAG flows.
    pub flow: Flow,
    /// Which nodes carry view chips in their foot, and the word on each chip,
    /// in the order they are drawn.
    ///
    /// The node's ways of being looked at are the *shell's* fact — a manifest
    /// declares relations, not views — so they arrive as words rather than as
    /// a type this crate would have to know. A node named here is sized for its
    /// chips; a node absent from the map keeps the card it always had, which is
    /// what leaves a manifest Protocol's layout where it was.
    pub view_chips: BTreeMap<AssetId, Vec<String>>,
}

impl Default for LayoutConfig {
    /// The shipped pitch: page gutter [`spacing::SPACE_8`] (32), rank pitch
    /// [`spacing::SPACE_8`] (32), sibling pitch [`spacing::SPACE_6`] (16).
    ///
    /// # Why these rungs and not the tighter pair below them
    ///
    /// The canvas was 90.3% whitespace at the old `(32, 64, 20)`, so the
    /// question was how far down the ladder to go before a multi-rank edge stops
    /// being followable. Both terms bottom out on the ink the renderer draws:
    ///
    /// - `col_gap` sets the separation between consecutive lane points, and
    ///   `asset_scene::orthogonal_path` puts half of it either side of each
    ///   dogleg's crossbar. At 32 that run is at least 16 points — comfortably
    ///   past the 9-point seam chevron and the 5-point arrowhead that sit on it.
    ///   At [`spacing::SPACE_7`] (24) it would be 12, three points of clearance,
    ///   which is where a chevron starts touching the corner it is meant to sit
    ///   inside.
    /// - `row_gap` separates parallel lanes across the flow. A lane channel is
    ///   `LANE_EXTENT` (10) wide, so 16 leaves more page between two lanes than
    ///   either lane occupies; [`spacing::SPACE_5`] (12) would leave less.
    ///
    /// Measured on the collapsed crosswalk, with the dead thickness floor also
    /// gone: 1034×1120 → 1018×714, a 36.3% cut down the reading axis. (An
    /// earlier revision of this line said 962×786; no combination of ladder
    /// rungs produces that height, and the figure the rest of the tree carries
    /// is this one.)
    fn default() -> Self {
        Self {
            margin: f64::from(spacing::SPACE_8),
            col_gap: f64::from(spacing::SPACE_8),
            row_gap: f64::from(spacing::SPACE_6),
            flow: Flow::Horizontal,
            view_chips: BTreeMap::new(),
        }
    }
}

/// Pixels per character at the render module's 11px Inter — the Dagster trick,
/// geometry before pixels, so this crate can size a card without measuring
/// text and stays free of a font stack.
///
/// It sizes a node's label and the word on a view chip, and the two have to
/// read the same figure or a chip laid out here would be drawn at a width the
/// raster disagrees with.
const PX_PER_CHAR: f64 = 7.0;

/// Card width from the label's char count, widened where the node's foot has
/// to hold a chip row.
fn node_width(kind: AssetKind, label: &str, chips: &[String]) -> f64 {
    let chars = label.chars().count().min(28) as f64;
    let base = (24.0 + chars * PX_PER_CHAR).clamp(64.0, 224.0);
    let base = match kind {
        AssetKind::Family => base + 28.0, // room for the xN badge
        _ => base,
    };
    base.max(view_chip_row_width(chips))
}

/// Card height per node class (the distinguishable treatments), plus the foot
/// a chip row needs.
fn node_height(kind: AssetKind, chips: bool) -> f64 {
    let base = match kind {
        AssetKind::Source => 30.0,
        AssetKind::File => 34.0,
        AssetKind::Table => 36.0,
        AssetKind::Internal | AssetKind::Opaque => 26.0,
        AssetKind::Dataset => 42.0,
        AssetKind::Family => 46.0,
    };
    if chips {
        base + VIEW_CHIP_BAND
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// View chips: a node's ways of being looked at, drawn in its foot
// ---------------------------------------------------------------------------

/// A **view chip**: one way of looking at the table a node names, drawn as a
/// small box in that node's foot.
///
/// Placed here rather than in the renderer because three readers need the same
/// rectangle and a second derivation of it is how they would come to disagree:
/// the raster draws it, the shell hit-tests a pointer against it, and this
/// module sizes the node around it.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewChip {
    /// The word on the chip.
    pub label: String,
    /// Where it sits, in the same canvas coordinates a node's [`Rect`] is in.
    pub rect: Rect,
}

/// A view chip's height — the design system's extra-small control rung.
pub const VIEW_CHIP_HEIGHT: f64 = meridian_design::control::HEIGHT_XS as f64;

/// How far a chip row sits from the node's leading edge, and from its bottom.
pub const VIEW_CHIP_INSET: f64 = spacing::SPACE_4 as f64;

/// The gap between two chips in one row.
pub const VIEW_CHIP_GAP: f64 = spacing::SPACE_3 as f64;

/// The room each side of a chip's word, inside its box.
pub const VIEW_CHIP_PADDING_X: f64 = spacing::CHIP_PADDING_X as f64;

/// What a chip row costs a node's height: the chip itself plus the inset that
/// holds it off the bottom edge.
///
/// The node grows by exactly this so the chips sit **under** the node's lines
/// rather than over them — the label is centred in what is left above the
/// band, which is why the band is a term of the height rather than a place the
/// renderer finds room in.
pub const VIEW_CHIP_BAND: f64 = VIEW_CHIP_HEIGHT + VIEW_CHIP_INSET;

/// The width of a chip carrying `label`.
#[must_use]
pub fn view_chip_width(label: &str) -> f64 {
    2.0 * VIEW_CHIP_PADDING_X + label.chars().count() as f64 * PX_PER_CHAR
}

/// The room a row of `labels` needs across a node, insets included. Zero for a
/// node with no views, which is what leaves every manifest Protocol's cards the
/// width they were.
fn view_chip_row_width(labels: &[String]) -> f64 {
    if labels.is_empty() {
        return 0.0;
    }
    let words: f64 = labels.iter().map(|label| view_chip_width(label)).sum();
    let gaps = VIEW_CHIP_GAP * (labels.len() - 1) as f64;
    2.0 * VIEW_CHIP_INSET + words + gaps
}

/// Where each of `labels` sits in the foot of the node at `rect` — the one
/// piece of chip arithmetic, so the raster and the pointer read one answer.
///
/// The row is laid from the node's leading edge in by [`VIEW_CHIP_INSET`], and
/// sits [`VIEW_CHIP_INSET`] above the node's bottom. `rect` is expected to be a
/// rectangle this module sized for chips — [`node_height`] adds
/// [`VIEW_CHIP_BAND`] to it — and the caller that hands one that was not gets a
/// row overlapping the node's label, which is a sizing bug rather than a case
/// to handle here.
#[must_use]
pub fn view_chip_rects(rect: &Rect, labels: &[String]) -> Vec<ViewChip> {
    let y = rect.y + rect.height - VIEW_CHIP_INSET - VIEW_CHIP_HEIGHT;
    let mut x = rect.x + VIEW_CHIP_INSET;
    labels
        .iter()
        .map(|label| {
            let width = view_chip_width(label);
            let chip = ViewChip {
                label: label.clone(),
                rect: Rect {
                    x,
                    y,
                    width,
                    height: VIEW_CHIP_HEIGHT,
                },
            };
            x += width + VIEW_CHIP_GAP;
            chip
        })
        .collect()
}

/// Compute the deterministic layout for `graph`.
#[must_use]
pub fn layout(graph: &AssetGraph, config: &LayoutConfig) -> Layout {
    // Unique (from, to) pairs drive layering + routing; parallel edges merge
    // into one lane that keeps the FIRST seam via and ORs any shield. A None
    // via never shadows a Some (collapse re-targeting can emit a via-less edge
    // alongside a seam-bearing one), so the merged lane always draws its
    // chevron when any parallel edge carries a seam.
    let mut unique: BTreeMap<(AssetId, AssetId), (Option<StepId>, bool)> = BTreeMap::new();
    for edge in &graph.edges {
        if edge.from == edge.to
            || !graph.nodes.contains_key(&edge.from)
            || !graph.nodes.contains_key(&edge.to)
        {
            continue;
        }
        let entry = unique
            .entry((edge.from.clone(), edge.to.clone()))
            .or_insert((edge.via.clone(), edge.shield));
        if entry.0.is_none() {
            entry.0 = edge.via.clone();
        }
        entry.1 = entry.1 || edge.shield;
    }

    let mut succs: BTreeMap<&AssetId, BTreeSet<&AssetId>> = BTreeMap::new();
    let mut preds: BTreeMap<&AssetId, BTreeSet<&AssetId>> = BTreeMap::new();
    for (from, to) in unique.keys() {
        succs.entry(from).or_default().insert(to);
        preds.entry(to).or_default().insert(from);
    }

    // Longest-path layering via Kahn's algorithm in id order. Nodes caught in
    // a cycle (malformed input) fall deterministically into layer 0.
    let mut layer: BTreeMap<&AssetId, usize> = graph.nodes.keys().map(|id| (id, 0)).collect();
    let mut indegree: BTreeMap<&AssetId, usize> = graph
        .nodes
        .keys()
        .map(|id| (id, preds.get(id).map_or(0, BTreeSet::len)))
        .collect();
    let mut ready: BTreeSet<&AssetId> = indegree
        .iter()
        .filter_map(|(id, d)| (*d == 0).then_some(*id))
        .collect();
    while let Some(&id) = ready.iter().next() {
        ready.remove(&id);
        let l = layer[&id];
        for &succ in succs.get(&id).into_iter().flatten() {
            if layer[&succ] < l + 1 {
                *layer.get_mut(&succ).expect("all nodes layered") = l + 1;
            }
            let d = indegree.get_mut(&succ).expect("all nodes counted");
            *d = d.saturating_sub(1);
            if *d == 0 {
                ready.insert(succ);
            }
        }
    }
    let n_layers = layer.values().copied().max().unwrap_or(0) + 1;

    // Ordering slots: real nodes plus dummy chains for edges spanning >1
    // layer. Dummy ids derive from the (from, to) pair — deterministic.
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Slot {
        Real(AssetId),
        Dummy {
            edge: (AssetId, AssetId),
            hop: usize,
        },
    }
    let slot_id = |slot: &Slot| -> String {
        match slot {
            Slot::Real(id) => id.clone(),
            Slot::Dummy { edge, hop } => format!("dummy.{}->{}#{hop}", edge.0, edge.1),
        }
    };

    let mut layers: Vec<Vec<Slot>> = vec![Vec::new(); n_layers];
    for (id, l) in &layer {
        layers[*l].push(Slot::Real((*id).clone()));
    }
    // Dummy chains + the slot-level adjacency used by the median sweeps.
    let mut slot_succs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut slot_preds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let link = |a: &Slot,
                b: &Slot,
                succs: &mut BTreeMap<String, BTreeSet<String>>,
                preds: &mut BTreeMap<String, BTreeSet<String>>| {
        succs.entry(slot_id(a)).or_default().insert(slot_id(b));
        preds.entry(slot_id(b)).or_default().insert(slot_id(a));
    };
    let mut routes_chain: BTreeMap<(AssetId, AssetId), Vec<Slot>> = BTreeMap::new();
    for (from, to) in unique.keys() {
        let (lf, lt) = (layer[from], layer[to]);
        let mut chain = Vec::new();
        if lt > lf + 1 {
            for (hop, l) in (lf + 1..lt).enumerate() {
                let dummy = Slot::Dummy {
                    edge: (from.clone(), to.clone()),
                    hop,
                };
                layers[l].push(dummy.clone());
                chain.push(dummy);
            }
        }
        let mut prev = Slot::Real(from.clone());
        for d in &chain {
            link(&prev, d, &mut slot_succs, &mut slot_preds);
            prev = d.clone();
        }
        link(
            &prev,
            &Slot::Real(to.clone()),
            &mut slot_succs,
            &mut slot_preds,
        );
        routes_chain.insert((from.clone(), to.clone()), chain);
    }
    for l in &mut layers {
        l.sort_by_key(|s| slot_id(s));
    }

    // Crossing reduction: exactly 4 median sweeps (down, up, down, up), ties
    // by node id — a fixed-iteration rule.
    for sweep in 0..4 {
        let downward = sweep % 2 == 0;
        let range: Vec<usize> = if downward {
            (1..n_layers).collect()
        } else {
            (0..n_layers.saturating_sub(1)).rev().collect()
        };
        for l in range {
            let neighbour_layer = if downward { l - 1 } else { l + 1 };
            let neighbour_index: BTreeMap<String, usize> = layers[neighbour_layer]
                .iter()
                .enumerate()
                .map(|(i, s)| (slot_id(s), i))
                .collect();
            let lookup = if downward { &slot_preds } else { &slot_succs };
            // 2x the median of neighbour indices keeps the key integral.
            let keys: Vec<(i64, String)> = layers[l]
                .iter()
                .enumerate()
                .map(|(current, slot)| {
                    let id = slot_id(slot);
                    let mut idx: Vec<i64> = lookup
                        .get(&id)
                        .into_iter()
                        .flatten()
                        .filter_map(|n| neighbour_index.get(n).map(|i| *i as i64))
                        .collect();
                    idx.sort_unstable();
                    let median2 = if idx.is_empty() {
                        2 * current as i64 // no neighbours: hold position
                    } else if idx.len() % 2 == 1 {
                        2 * idx[idx.len() / 2]
                    } else {
                        idx[idx.len() / 2 - 1] + idx[idx.len() / 2]
                    };
                    (median2, id)
                })
                .collect();
            let mut order: Vec<usize> = (0..layers[l].len()).collect();
            order.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
            let reordered: Vec<Slot> = order.into_iter().map(|i| layers[l][i].clone()).collect();
            layers[l] = reordered;
        }
    }

    // Coordinates. Two axes, named orientation-neutrally: the **along** axis
    // separates layers (the flow direction), the **cross** axis stacks the
    // nodes within a layer. Horizontal maps along→x, cross→y; Vertical
    // transposes to along→y, cross→x. The layering + crossing passes above are
    // untouched — only this assignment (and the renderer's routing) flips.
    let vertical = matches!(config.flow, Flow::Vertical);
    // A node's chip words, or nothing where it declares no views. Looked up
    // once per call site rather than cloned into the closure, so a node the
    // config does not name costs an empty slice and no allocation.
    let chips_of = |id: &AssetId| -> &[String] {
        config.view_chips.get(id).map_or(&[], Vec::as_slice)
    };
    let size_of = |slot: &Slot| -> (f64, f64) {
        match slot {
            Slot::Real(id) => {
                let node = &graph.nodes[id];
                let chips = chips_of(id);
                (
                    node_width(node.kind, &node.label, chips),
                    node_height(node.kind, !chips.is_empty()),
                )
            }
            Slot::Dummy { .. } => (LANE_EXTENT, LANE_EXTENT),
        }
    };
    // (along, cross) extent of a slot. A dummy is a square lane cell in BOTH
    // flows — see `LANE_EXTENT` — so transposing must not swap a card's two
    // sides into it, hence the explicit form rather than reusing `size_of`.
    let extents = |slot: &Slot| -> (f64, f64) {
        match slot {
            Slot::Real(_) => {
                let (w, h) = size_of(slot);
                if vertical {
                    (h, w)
                } else {
                    (w, h)
                }
            }
            Slot::Dummy { .. } => (LANE_EXTENT, LANE_EXTENT),
        }
    };
    // Layer "thickness" along the flow: the tallest/widest thing the layer
    // actually holds, card or lane. **Content-driven, with no constant under
    // it** — the 48.0 floor that used to sit here was taller than every card in
    // the model, so it was not a floor at all, it was 118 points of reserved
    // nothing on the collapsed crosswalk. `LANE_EXTENT` carries what the floor
    // was written for. Layer "length" is the extent across it.
    let thickness: Vec<f64> = layers
        .iter()
        .map(|l| l.iter().map(|s| extents(s).0).fold(0.0, f64::max))
        .collect();
    let lengths: Vec<f64> = layers
        .iter()
        .map(|l| {
            let sum: f64 = l.iter().map(|s| extents(s).1).sum();
            sum + config.row_gap * (l.len().saturating_sub(1)) as f64
        })
        .collect();
    let longest = lengths.iter().copied().fold(0.0, f64::max);

    let mut positions: BTreeMap<AssetId, Rect> = BTreeMap::new();
    let mut lane_points: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut a = config.margin; // along coordinate — advances per layer
    for (l, slots) in layers.iter().enumerate() {
        let thick = thickness[l];
        let mut c = config.margin + ((longest - lengths[l]) / 2.0).max(0.0);
        for slot in slots {
            let (a_ext, c_ext) = extents(slot);
            match slot {
                Slot::Real(id) => {
                    let (w, h) = size_of(slot);
                    // Centre the card within the layer thickness (along), place
                    // it at the running cross coordinate.
                    let centred = a + (thick - a_ext) / 2.0;
                    let (x, y) = if vertical { (c, centred) } else { (centred, c) };
                    positions.insert(
                        id.clone(),
                        Rect {
                            x: x.round(),
                            y: y.round(),
                            width: w.round(),
                            height: h.round(),
                        },
                    );
                }
                Slot::Dummy { .. } => {
                    // Lane point: along-centre of the layer, cross-centre of the
                    // slot.
                    let along_c = a + thick / 2.0;
                    let cross_c = c + c_ext / 2.0;
                    let (px, py) = if vertical {
                        (cross_c, along_c)
                    } else {
                        (along_c, cross_c)
                    };
                    lane_points.insert(slot_id(slot), (px.round(), py.round()));
                }
            }
            c += c_ext + config.row_gap;
        }
        a += thick + config.col_gap;
    }
    let along_total = (a - config.col_gap + config.margin).round();
    let cross_total = (longest + 2.0 * config.margin).round();
    let (width, height) = if vertical {
        (cross_total, along_total)
    } else {
        (along_total, cross_total)
    };

    // Routes: exit the producer's downstream edge -> dummy lane points -> enter
    // the consumer's upstream edge. Horizontal exits the right edge / enters the
    // left; Vertical exits the bottom / enters the top (downstream sits below).
    let lanes: Vec<EdgeRoute> = unique
        .iter()
        .map(|((from, to), (via, shield))| {
            let f = &positions[from];
            let t = &positions[to];
            let start = if vertical {
                ((f.x + f.width / 2.0).round(), f.y + f.height)
            } else {
                (f.x + f.width, (f.y + f.height / 2.0).round())
            };
            let end = if vertical {
                ((t.x + t.width / 2.0).round(), t.y)
            } else {
                (t.x, (t.y + t.height / 2.0).round())
            };
            let mut points = vec![start];
            for d in &routes_chain[&(from.clone(), to.clone())] {
                points.push(lane_points[&slot_id(d)]);
            }
            points.push(end);
            EdgeRoute {
                from: from.clone(),
                to: to.clone(),
                via: via.clone(),
                shield: *shield,
                points,
            }
        })
        .collect();

    // The chips, placed once the cards are. Driven off `positions` rather than
    // off the config's keys, so a node the config names and the graph does not
    // hold contributes nothing rather than a rectangle at no node.
    let view_chips: BTreeMap<AssetId, Vec<ViewChip>> = positions
        .iter()
        .filter_map(|(id, rect)| {
            let labels = config.view_chips.get(id)?;
            (!labels.is_empty()).then(|| (id.clone(), view_chip_rects(rect, labels)))
        })
        .collect();

    Layout {
        width,
        height,
        positions,
        lanes,
        flow: config.flow,
        view_chips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{build_graph, AssetGraph, AssetNode, Edge};
    use crate::manifest::parse_manifest_str;

    /// A two-node graph reached by parallel edges: a via-less one (as collapse
    /// re-targeting emits) FIRST, then a seam-bearing one.
    fn parallel_edges(via_first: Option<&str>, via_second: Option<&str>) -> AssetGraph {
        let node = |id: &str| AssetNode {
            id: id.to_string(),
            kind: AssetKind::File,
            label: id.to_string(),
            step: None,
            family_count: None,
            issue: None,
        };
        let edge = |via: Option<&str>| Edge {
            from: "file.p.a".to_string(),
            to: "file.p.b".to_string(),
            via: via.map(str::to_string),
            shield: false,
        };
        AssetGraph {
            protocol: "p".to_string(),
            nodes: [
                ("file.p.a".to_string(), node("file.p.a")),
                ("file.p.b".to_string(), node("file.p.b")),
            ]
            .into_iter()
            .collect(),
            seams: BTreeMap::new(),
            edges: vec![edge(via_first), edge(via_second)],
        }
    }

    #[test]
    fn pds_parallel_edge_merge_keeps_the_seam_via() {
        // A None-via edge listed FIRST must not shadow the Some-via sibling —
        // the merged lane keeps the chevron.
        let g = parallel_edges(None, Some("transform"));
        let l = layout(&g, &LayoutConfig::default());
        assert_eq!(l.lanes.len(), 1);
        assert_eq!(
            l.lanes[0].via.as_deref(),
            Some("transform"),
            "the seam via survives the merge"
        );
    }

    fn diamond() -> AssetGraph {
        // fetch feeds two extracts which both feed a loader — plus one edge
        // spanning two layers (fetch's file read directly by the loader), so
        // a dummy lane exists.
        let yaml = r"
name: d
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://h.example/a', out: build/a }
  - name: left
    op: archive_extract@1
    with: { archive: build/a, dest: build/l, members: [p.tsv] }
  - name: right
    op: archive_extract@1
    with: { archive: build/a, dest: build/r, members: [p.tsv] }
  - name: join
    sql: models/j.sql
    depends_on: [build/l, build/r, build/a]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = std::collections::BTreeMap::new();
        sources.insert(
            "join".to_string(),
            Ok("CREATE TABLE j AS SELECT * FROM read_csv('build/l/x.csv') \
                UNION ALL SELECT * FROM read_csv('build/r/x.csv') \
                UNION ALL SELECT * FROM read_csv('build/a');"
                .to_string()),
        );
        build_graph(&manifest, &sources)
    }

    /// The three spacing defaults are steps of the Meridian ladder, not numbers
    /// this file invented.
    ///
    /// Two of the three were not, before this: `col_gap` was 64, past the top of
    /// the ladder ([`spacing::SPACE_9`] = 48), and `row_gap` was 20, which is a
    /// *row height* rung and not a gap at all. A fourth spacing vocabulary in a
    /// product with one design system is a bug, and it is the kind that spreads
    /// silently, so it is mechanically checked rather than remembered.
    #[test]
    fn pds_the_spacing_defaults_are_ladder_steps() {
        let cfg = LayoutConfig::default();
        for (name, value) in [
            ("margin", cfg.margin),
            ("col_gap", cfg.col_gap),
            ("row_gap", cfg.row_gap),
        ] {
            #[allow(clippy::cast_possible_truncation)]
            let v = value as f32;
            assert!(
                spacing::SPACE.contains(&v),
                "{name} = {value} is not a step of the Meridian spacing ladder \
                 {:?} — it is a fourth spacing vocabulary",
                spacing::SPACE
            );
        }
    }

    /// The ranks of a laid-out canvas, recovered from the outside: each rank's
    /// centre along the flow and the extent of the deepest card on it.
    ///
    /// Every card is centred within its rank's thickness, so `start + extent / 2`
    /// is the rank's own centre — but the coordinates are quantised to whole
    /// pixels and a card whose extent differs from the rank's thickness by an odd
    /// number rounds half a point off that centre. Hence the clustering rather
    /// than an exact key: ranks are `col_gap` apart, so anything within a point
    /// is the same rank and nothing else can be.
    fn ranks_of(l: &Layout, flow: Flow) -> Vec<f64> {
        let along = |r: &Rect| {
            if matches!(flow, Flow::Vertical) {
                (r.y, r.height)
            } else {
                (r.x, r.width)
            }
        };
        let mut cards: Vec<(f64, f64)> = l
            .positions
            .values()
            .map(|r| {
                let (start, extent) = along(r);
                (start + extent / 2.0, extent)
            })
            .collect();
        cards.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut ranks: Vec<f64> = Vec::new();
        let mut centre = f64::NEG_INFINITY;
        for (c, extent) in cards {
            if c - centre > 1.0 {
                ranks.push(extent);
                centre = c;
            } else if let Some(deepest) = ranks.last_mut() {
                *deepest = deepest.max(extent);
            }
        }
        ranks
    }

    /// **A rank is exactly as thick as the deepest card on it.** No constant
    /// sits under that.
    ///
    /// Asserted as an equation over the canvas the caller is handed, not over an
    /// internal: the canvas's along-flow extent must be exactly its two margins,
    /// its per-rank maxima, and one `col_gap` between each neighbouring pair.
    ///
    /// Watched redden, one mutation: putting the old `fold(48.0, f64::max)` back
    /// under `thickness` fails here with *"58 points are reserved for nothing"*
    /// on this fixture — the same defect that cost the collapsed crosswalk 118.
    #[test]
    fn pds_rank_thickness_is_content_driven_with_no_floor_under_it() {
        let g = diamond();
        for flow in [Flow::Vertical, Flow::Horizontal] {
            let cfg = LayoutConfig {
                flow,
                ..LayoutConfig::default()
            };
            let l = layout(&g, &cfg);
            let ranks = ranks_of(&l, flow);
            assert!(ranks.len() > 1, "{flow:?}: one rank proves nothing");
            let sum: f64 = ranks.iter().sum();
            let expected = 2.0 * cfg.margin + sum + cfg.col_gap * (ranks.len() - 1) as f64;
            let actual = if matches!(flow, Flow::Vertical) {
                l.height
            } else {
                l.width
            };
            assert!(
                (actual - expected).abs() < f64::EPSILON,
                "{flow:?}: the canvas is {actual} along the flow but its \
                 {} ranks, its margins and its gaps account for {expected} — \
                 {} points are reserved for nothing",
                ranks.len(),
                actual - expected
            );
        }
    }

    /// A lane declares the same extent both ways, so a rank that held only lanes
    /// would be as thick as the lanes crossing it are wide — and the hop through
    /// it would still carry the seam glyph the renderer draws on it.
    ///
    /// The clearance is the point of the number, and it is checked
    /// arithmetically rather than photographed: `orthogonal_path` puts half the
    /// lane-point separation either side of each dogleg's crossbar, and
    /// `draw_chevron` occupies 9 points along the flow.
    #[test]
    fn pds_a_lane_reserves_its_own_channel_along_the_flow() {
        /// `draw_chevron`'s along-flow extent: two arrows at offsets −3 and +2,
        /// each ±2 about its offset, so −5…+4.
        const CHEVRON_EXTENT: f64 = 9.0;
        let cfg = LayoutConfig::default();
        let run = LANE_EXTENT + cfg.col_gap;
        assert!(
            run / 2.0 >= 2.0 * CHEVRON_EXTENT,
            "a hop between two lane-only ranks gives each side of its crossbar \
             {}pt, which does not clear the {CHEVRON_EXTENT}pt seam chevron with \
             room to spare",
            run / 2.0
        );
        // And the layering this crate ships cannot actually produce a lane-only
        // rank: longest-path layering gives every node above layer 0 a
        // predecessor one layer down, so every rank holds at least one real card
        // and the shortest card is deeper than a lane. Stated here so the number
        // above reads as a guard on a layering rule rather than as something the
        // shipped graphs exercise.
        let l = layout(&diamond(), &cfg);
        for (rank, deepest) in ranks_of(&l, Flow::Vertical).into_iter().enumerate() {
            assert!(
                deepest >= node_height(AssetKind::Internal, false),
                "rank {rank} is {deepest}pt deep — shallower than the shortest \
                 card, so a lane-only rank is reachable after all and \
                 LANE_EXTENT is load-bearing rather than defensive"
            );
        }
    }

    #[test]
    fn layout_repeated_call_equality() {
        let g = diamond();
        let cfg = LayoutConfig::default();
        assert_eq!(layout(&g, &cfg), layout(&g, &cfg));
    }

    #[test]
    fn whole_pixel_quantisation() {
        let g = diamond();
        let l = layout(&g, &LayoutConfig::default());
        assert_eq!(l.width.fract(), 0.0);
        assert_eq!(l.height.fract(), 0.0);
        for r in l.positions.values() {
            for v in [r.x, r.y, r.width, r.height] {
                assert_eq!(v.fract(), 0.0, "whole pixels only: {r:?}");
            }
        }
        for lane in &l.lanes {
            for (px, py) in &lane.points {
                assert_eq!(px.fract(), 0.0);
                assert_eq!(py.fract(), 0.0);
            }
        }
    }

    #[test]
    fn pds_layout_flows_left_to_right() {
        let g = diamond();
        let l = layout(&g, &LayoutConfig::default());
        assert_eq!(l.flow, Flow::Horizontal);
        for lane in &l.lanes {
            let f = &l.positions[&lane.from];
            let t = &l.positions[&lane.to];
            assert!(
                f.x + f.width <= t.x,
                "downstream sits strictly right: {} -> {}",
                lane.from,
                lane.to
            );
        }
        // Every real node is placed; the multi-layer span routed through a
        // dummy lane (3+ waypoints).
        assert_eq!(l.positions.len(), g.nodes.len());
        assert!(
            l.lanes.iter().any(|lane| lane.points.len() > 2),
            "the layer-spanning edge routes through a dummy lane"
        );
    }

    #[test]
    fn vertical_flow_reads_top_to_bottom() {
        // Vertical transposes the SAME graph: every downstream node sits strictly
        // BELOW its producer, edges exit the bottom edge and enter the top.
        let g = diamond();
        let cfg = LayoutConfig {
            flow: Flow::Vertical,
            ..LayoutConfig::default()
        };
        let l = layout(&g, &cfg);
        assert_eq!(l.flow, Flow::Vertical);
        for lane in &l.lanes {
            let f = &l.positions[&lane.from];
            let t = &l.positions[&lane.to];
            assert!(
                f.y + f.height <= t.y,
                "downstream sits strictly below: {} -> {}",
                lane.from,
                lane.to
            );
            // Edge exits the producer's bottom edge, enters the consumer's top.
            let (_, sy) = *lane.points.first().unwrap();
            let (_, ey) = *lane.points.last().unwrap();
            assert_eq!(sy, f.y + f.height, "exits the bottom edge");
            assert_eq!(ey, t.y, "enters the top edge");
        }
        assert_eq!(l.positions.len(), g.nodes.len());
    }

    #[test]
    fn vertical_bounds_width_to_the_widest_layer() {
        // The motivating win: horizontal's long axis is width (sideways scroll);
        // vertical puts the long axis on height, so the canvas is TALLER than it
        // is WIDE and strictly narrower than the horizontal render's width.
        let g = diamond();
        let h = layout(&g, &LayoutConfig::default());
        let v = layout(
            &g,
            &LayoutConfig {
                flow: Flow::Vertical,
                ..LayoutConfig::default()
            },
        );
        assert!(
            v.height > v.width,
            "vertical is taller than wide: {}x{}",
            v.width,
            v.height
        );
        assert!(
            v.width < h.width,
            "vertical bounds width below the horizontal render"
        );
        // The transpose swaps which axis is long: horizontal is wider than tall,
        // vertical is taller than wide (the layouts are near-mirror dimensions,
        // not byte-identical — per-node width != height shifts the packing).
        assert!(h.width > h.height, "horizontal is wider than tall");
    }

    #[test]
    fn vertical_layout_is_deterministic_and_whole_pixel() {
        let g = diamond();
        let cfg = LayoutConfig {
            flow: Flow::Vertical,
            ..LayoutConfig::default()
        };
        assert_eq!(layout(&g, &cfg), layout(&g, &cfg));
        let l = layout(&g, &cfg);
        for r in l.positions.values() {
            for val in [r.x, r.y, r.width, r.height] {
                assert_eq!(val.fract(), 0.0, "whole pixels: {r:?}");
            }
        }
    }
}
