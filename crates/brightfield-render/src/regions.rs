//! Dense regions of a two-column plot, found on its density grid.
//!
//! The method is the density-map clustering of "A Scalable Approach to
//! Clustering Embedding Projections" (arXiv 2504.07285, Section 2 and its
//! Algorithm 1), in four steps:
//!
//! 1. Every cell climbs to its highest neighbour (8-neighbourhood), and the
//!    cells that reach the same local maximum form an initial cluster.
//! 2. Neighbouring clusters are joined in a graph whose edges carry the
//!    highest density on their shared boundary, the saddle between them.
//! 3. Clusters are merged greedily, closest first, while a cluster's peak
//!    stands barely above the saddle it shares with a neighbour.
//! 4. Each cluster is truncated to the cells at or above a tenth of its peak,
//!    and keeps only the connected piece that holds the peak.
//!
//! Clusters left with fewer than [`MIN_REGION_CELLS`] cells are then dropped.
//!
//! The output must be identical run to run, since a baseline that pins regions
//! would otherwise pin noise. Every tie therefore breaks by the lower
//! row-major cell index, and nothing iterates a hash map.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

/// A density grid over a plot's view.
///
/// `density` is row-major with `cols * rows` cells, row `r` and column `c` at
/// `r * cols + c`; rows run along y and columns along x. `x` and `y` are the
/// grid's outer edges in data units, low edge first. Densities are finite and
/// non-negative.
#[derive(Debug, Clone, PartialEq)]
pub struct DensityGrid {
    pub cols: usize,
    pub rows: usize,
    pub x: (f64, f64),
    pub y: (f64, f64),
    pub density: Vec<f64>,
}

/// One dense region of a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// The cell of the region's highest density.
    pub peak: usize,
    /// The density at `peak`.
    pub peak_density: f64,
    /// The region's cells, as row-major indices in ascending order.
    pub cells: Vec<usize>,
    /// The region's density mass (the sum over `cells`) over the grid's total.
    pub share: f64,
}

/// The fewest cells a region may have once truncated.
///
/// On the 100 × 100 grid a plot uses, six cells is a patch about 2 × 3 cells,
/// under a thousandth of the view. Anything smaller is a speck: too small to
/// read as an outline or to hold a name, and the size of the bumps a density
/// estimate makes around a handful of stray rows.
pub const MIN_REGION_CELLS: usize = 6;

/// A cluster is truncated to the cells at or above this fraction of its own
/// peak. The paper's value (Section 2, "Truncate Clusters").
const TRUNCATE_FRACTION: f64 = 0.1;

/// Two neighbouring clusters merge when the saddle between them is at least
/// this fraction of the lower of their two peaks.
///
/// The paper (Section 2, "Union Clusters") unions a cluster with its
/// neighbour "if the location of the local maximum within a cluster is close
/// to its boundary", because such a maximum "is merely slightly higher than
/// the boundary and thus may not stand out much on its own", and it unions the
/// pairs that fit the criterion best first. It states the criterion in words
/// and gives no constant for it. This module measures the closeness in
/// density rather than distance, on purpose: the paper's stated reason is the
/// height of the peak over the boundary, and a distance in cells would change
/// with the grid's resolution. The ratio is saddle / lower peak, and the
/// constant is this module's own: a peak at most a quarter above its saddle
/// does not stand out. Two equal Gaussians three standard deviations apart,
/// which read as two lobes, have a ratio near 0.64 and stay apart.
const MERGE_SADDLE_RATIO: f64 = 0.8;

/// The regions of `grid`, highest peak first, ties by the lower peak cell.
///
/// A grid with no cells, a density vector of the wrong length, or no
/// variation at all (all zero, or constant) has no regions.
pub fn find_regions(grid: &DensityGrid) -> Vec<Region> {
    let n = grid.cols * grid.rows;
    let d = &grid.density;
    if n == 0 || d.len() != n {
        return Vec::new();
    }
    let lo = d.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = d.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi.partial_cmp(&lo) != Some(Ordering::Greater) {
        return Vec::new();
    }
    let total: f64 = d.iter().sum();

    // Step 1: each cell points uphill to its highest neighbour; following the
    // pointers ends at the cell's local maximum, which names its cluster.
    let mut uphill: Vec<usize> = (0..n).map(|cell| highest_neighbour(grid, cell)).collect();
    let peak_of: Vec<usize> = (0..n).map(|cell| find(&mut uphill, cell)).collect();

    // Step 2: the saddle between two clusters is the highest boundary between
    // them, a boundary between adjacent cells being the lower of the two.
    let mut graph: BTreeMap<usize, BTreeMap<usize, f64>> = BTreeMap::new();
    for cell in 0..n {
        for other in neighbours(grid, cell).filter(|&o| o > cell) {
            let (a, b) = (peak_of[cell], peak_of[other]);
            if a != b {
                let saddle = d[cell].min(d[other]);
                raise_edge(&mut graph, a, b, saddle);
                raise_edge(&mut graph, b, a, saddle);
            }
        }
    }

    // Step 3: merge greedily, the pair whose saddle is closest to its lower
    // peak first. The heap holds candidates lazily: a merge pushes the merged
    // cluster's edges afresh, and a popped candidate whose clusters or ratio
    // have changed since is stale and skipped.
    let mut merged_into: Vec<usize> = (0..n).collect();
    let mut heap = BinaryHeap::new();
    for (&a, edges) in &graph {
        for (&b, &saddle) in edges.range(a + 1..) {
            push_candidate(&mut heap, d, a, b, saddle);
        }
    }
    while let Some(candidate) = heap.pop() {
        let Candidate { ratio, a, b } = candidate;
        if merged_into[a] != a || merged_into[b] != b {
            continue;
        }
        let saddle = graph[&a][&b];
        if merge_ratio(d, a, b, saddle).map(f64::to_bits) != Some(ratio.to_bits()) {
            continue;
        }
        let (winner, loser) = if ranks_above(d, a, b) { (a, b) } else { (b, a) };
        merged_into[loser] = winner;
        let loser_edges = graph.remove(&loser).unwrap_or_default();
        if let Some(edges) = graph.get_mut(&winner) {
            edges.remove(&loser);
        }
        for (other, saddle) in loser_edges {
            if other == winner {
                continue;
            }
            if let Some(edges) = graph.get_mut(&other) {
                edges.remove(&loser);
            }
            raise_edge(&mut graph, winner, other, saddle);
            raise_edge(&mut graph, other, winner, saddle);
        }
        for (&other, &saddle) in graph.get(&winner).into_iter().flatten() {
            push_candidate(&mut heap, d, winner.min(other), winner.max(other), saddle);
        }
    }

    // Step 4: truncate each cluster at a tenth of its peak, keep the piece
    // that holds the peak, then drop specks. A merged cluster's peak is its
    // winner, the highest cell it holds. A single basin stays connected when
    // truncated, since each kept cell climbs to its peak through cells at
    // least as dense, but a merged cluster joined over a saddle below the cut
    // falls apart; its cut-off pieces belong to no region.
    let owner: Vec<Option<usize>> = (0..n)
        .map(|cell| {
            let peak = find(&mut merged_into, peak_of[cell]);
            (d[cell] >= TRUNCATE_FRACTION * d[peak]).then_some(peak)
        })
        .collect();
    let mut regions: Vec<Region> = (0..n)
        .filter(|&peak| owner[peak] == Some(peak) && d[peak] > 0.0)
        .filter_map(|peak| {
            let cells = connected_piece(grid, &owner, peak);
            (cells.len() >= MIN_REGION_CELLS).then(|| {
                let mass: f64 = cells.iter().map(|&c| d[c]).sum();
                Region {
                    peak,
                    peak_density: d[peak],
                    cells,
                    share: mass / total,
                }
            })
        })
        .collect();
    regions.sort_by(|a, b| {
        b.peak_density
            .total_cmp(&a.peak_density)
            .then(a.peak.cmp(&b.peak))
    });
    regions
}

/// The cells owned by `peak` that connect to it through the 8-neighbourhood,
/// in ascending index order.
fn connected_piece(grid: &DensityGrid, owner: &[Option<usize>], peak: usize) -> Vec<usize> {
    let mut seen = BTreeSet::from([peak]);
    let mut stack = vec![peak];
    while let Some(cell) = stack.pop() {
        for other in neighbours(grid, cell) {
            if owner[other] == Some(peak) && seen.insert(other) {
                stack.push(other);
            }
        }
    }
    seen.into_iter().collect()
}

/// The cell holding the point `(x, y)`, or `None` outside the grid's edges.
///
/// Each axis is cut into equal bins from its low edge; a point on the high
/// edge belongs to the last row or column.
pub fn cell_of(grid: &DensityGrid, x: f64, y: f64) -> Option<usize> {
    let col = axis_bin(x, grid.x, grid.cols)?;
    let row = axis_bin(y, grid.y, grid.rows)?;
    Some(row * grid.cols + col)
}

/// The `(x, y)` data ranges a cell covers, each low edge first.
///
/// `cell` must be a cell of the grid.
pub fn cell_bounds(grid: &DensityGrid, cell: usize) -> ((f64, f64), (f64, f64)) {
    let (col, row) = (cell % grid.cols, cell / grid.cols);
    (
        axis_span(grid.x, grid.cols, col),
        axis_span(grid.y, grid.rows, row),
    )
}

/// The region (an index into `regions`) whose cell holds the point `(x, y)`,
/// or `None` in a gap between regions or outside the grid.
///
/// Regions never share a cell, so the answer is unique.
pub fn region_at(grid: &DensityGrid, regions: &[Region], x: f64, y: f64) -> Option<usize> {
    let cell = cell_of(grid, x, y)?;
    regions
        .iter()
        .position(|r| r.cells.binary_search(&cell).is_ok())
}

/// The boundary of `region` as closed rings of `(x, y)` data coordinates.
///
/// Each ring follows cell edges, so it is axis-aligned, and lists its corners
/// only: the first corner is not repeated at the end, and a drawer closes the
/// ring itself. Outer rings come first, then holes. Within each group rings are
/// ordered by their lowest-then-leftmost corner, and each ring starts at that
/// corner and runs anticlockwise (outer) or clockwise (hole) in data space, so
/// the output is the same on every run.
///
/// Two cells that touch only at a corner count as joined, as they do when
/// regions are found. Where the boundary meets itself at such a corner, the
/// trace turns toward the region's own cell, so the two cells stay in one ring
/// that touches itself at that corner rather than splitting into two rings or
/// crossing. The empty space is therefore read as 4-connected: a hole touching
/// the outside only at a corner is still a hole.
pub fn region_outline(grid: &DensityGrid, region: &Region) -> Vec<Vec<(f64, f64)>> {
    let (cols, rows) = (grid.cols, grid.rows);
    let n = cols * rows;
    let mut mask = vec![false; n];
    for &cell in region.cells.iter().filter(|&&c| c < n) {
        mask[cell] = true;
    }
    let has = |c: isize, r: isize| {
        (0..cols as isize).contains(&c)
            && (0..rows as isize).contains(&r)
            && mask[r as usize * cols + c as usize]
    };

    // Directed boundary edges with the region on their left, keyed by start
    // vertex and heading. Vertices are cell corners, `(col, row)` in 0..=cols
    // and 0..=rows; headings are 0 east, 1 north, 2 west, 3 south.
    let mut edges: BTreeSet<(usize, usize, u8)> = BTreeSet::new();
    for cell in (0..n).filter(|&c| mask[c]) {
        let (c, r) = ((cell % cols) as isize, (cell / cols) as isize);
        let (u, v) = (c as usize, r as usize);
        if !has(c, r - 1) {
            edges.insert((u, v, 0));
        }
        if !has(c + 1, r) {
            edges.insert((u + 1, v, 1));
        }
        if !has(c, r + 1) {
            edges.insert((u + 1, v + 1, 2));
        }
        if !has(c - 1, r) {
            edges.insert((u, v + 1, 3));
        }
    }

    let step = |(u, v, h): (usize, usize, u8)| match h {
        0 => (u + 1, v),
        1 => (u, v + 1),
        2 => (u - 1, v),
        _ => (u, v - 1),
    };
    let mut used: BTreeSet<(usize, usize, u8)> = BTreeSet::new();
    // Each ring with its sort key: whether it is a hole, then its start corner
    // as (row, col).
    type Keyed = ((bool, (usize, usize)), Vec<(usize, usize)>);
    let mut rings: Vec<Keyed> = Vec::new();
    for &start in &edges {
        if used.contains(&start) {
            continue;
        }
        let mut corners: Vec<(usize, usize)> = Vec::new();
        let mut edge = start;
        loop {
            used.insert(edge);
            let (u, v) = step(edge);
            // Prefer a right turn, then straight on, then a left turn. At a
            // corner touch two edges leave the vertex, and the right turn is
            // the one that keeps the two region cells together.
            let heading = edge.2;
            let next = [(heading + 3) % 4, heading, (heading + 1) % 4]
                .into_iter()
                .map(|h| (u, v, h))
                .find(|e| edges.contains(e));
            let Some(next) = next else { break };
            if next.2 != heading {
                corners.push((u, v));
            }
            if next == start {
                break;
            }
            edge = next;
        }
        // Start the ring at its lowest-leftmost corner.
        let first = corners
            .iter()
            .enumerate()
            .min_by_key(|&(_, &(u, v))| (v, u))
            .map(|(i, _)| i)
            .unwrap_or(0);
        corners.rotate_left(first);
        let twice_area: i64 = corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|(&(u0, v0), &(u1, v1))| u0 as i64 * v1 as i64 - u1 as i64 * v0 as i64)
            .sum();
        let key = corners.first().map_or((0, 0), |&(u, v)| (v, u));
        rings.push(((twice_area < 0, key), corners));
    }
    rings.sort_by_key(|(key, _)| *key);

    let edge =
        |(lo, hi): (f64, f64), count: usize, i: usize| lo + (hi - lo) * (i as f64 / count as f64);
    rings
        .into_iter()
        .map(|(_, corners)| {
            corners
                .into_iter()
                .map(|(u, v)| (edge(grid.x, cols, u), edge(grid.y, rows, v)))
                .collect()
        })
        .collect()
}

/// Where to write `region`'s name: the centre, in data units, of the largest
/// square block of the region's own cells that lies nearest the peak.
///
/// The centre of a region's bounding box or its mass can fall in a hollow of a
/// crescent or over a neighbour, but a block of the region's own cells cannot.
/// Ties between equally large blocks go to the one whose centre is nearest the
/// peak cell's centre, measured in cells, then to the one whose top-left (lowest
/// row, then column) cell has the lowest index.
pub fn label_anchor(grid: &DensityGrid, region: &Region) -> (f64, f64) {
    let (cols, rows) = (grid.cols, grid.rows);
    let n = cols * rows;
    let mut mask = vec![false; n];
    for &cell in region.cells.iter().filter(|&&c| c < n) {
        mask[cell] = true;
    }
    // side[i]: the side of the largest all-region square whose far corner
    // (highest row and column) is cell i.
    let mut side = vec![0usize; n];
    for cell in (0..n).filter(|&c| mask[c]) {
        let (c, r) = (cell % cols, cell / cols);
        side[cell] = if c == 0 || r == 0 {
            1
        } else {
            1 + side[cell - 1]
                .min(side[cell - cols])
                .min(side[cell - cols - 1])
        };
    }
    let k = side.iter().copied().max().unwrap_or(0);
    let (peak_col, peak_row) = (region.peak % cols.max(1), region.peak / cols.max(1));
    // Everything below is in half cells, so equal distances compare exactly.
    let best = (0..n)
        .filter(|&cell| side[cell] == k && k > 0)
        .map(|cell| {
            let (c0, r0) = (cell % cols + 1 - k, cell / cols + 1 - k);
            let (cx, cy) = (2 * c0 + k, 2 * r0 + k);
            let (dx, dy) = (
                cx as i64 - (2 * peak_col + 1) as i64,
                cy as i64 - (2 * peak_row + 1) as i64,
            );
            (dx * dx + dy * dy, r0 * cols + c0, cx, cy)
        })
        .min();
    let (cx, cy) = match best {
        Some((_, _, cx, cy)) => (cx, cy),
        None => (2 * peak_col + 1, 2 * peak_row + 1),
    };
    let at = |(lo, hi): (f64, f64), count: usize, half: usize| {
        lo + (hi - lo) * (half as f64 / (2 * count) as f64)
    };
    (at(grid.x, cols, cx), at(grid.y, rows, cy))
}

fn axis_bin(v: f64, (lo, hi): (f64, f64), bins: usize) -> Option<usize> {
    if bins == 0 || lo.partial_cmp(&hi) != Some(Ordering::Less) || !(lo..=hi).contains(&v) {
        return None;
    }
    let bin = ((v - lo) / (hi - lo) * bins as f64).floor() as usize;
    Some(bin.min(bins - 1))
}

fn axis_span((lo, hi): (f64, f64), bins: usize, bin: usize) -> (f64, f64) {
    let edge = |i: usize| lo + (hi - lo) * (i as f64 / bins as f64);
    (edge(bin), edge(bin + 1))
}

/// The cells around `cell` in its 8-neighbourhood, in ascending index order.
fn neighbours(grid: &DensityGrid, cell: usize) -> impl Iterator<Item = usize> + '_ {
    let (col, row) = (cell % grid.cols, cell / grid.cols);
    (-1isize..=1).flat_map(move |dr| {
        (-1isize..=1).filter_map(move |dc| {
            let (c, r) = (col as isize + dc, row as isize + dr);
            let inside = (dc, dr) != (0, 0)
                && (0..grid.cols as isize).contains(&c)
                && (0..grid.rows as isize).contains(&r);
            inside.then(|| r as usize * grid.cols + c as usize)
        })
    })
}

/// Whether cell `a` ranks above cell `b`: denser, or as dense and earlier.
/// A strict total order, so climbing it never cycles, even on a plateau.
fn ranks_above(d: &[f64], a: usize, b: usize) -> bool {
    d[a].total_cmp(&d[b]).then(b.cmp(&a)) == Ordering::Greater
}

/// The neighbour of `cell` that ranks highest, or `cell` itself when it ranks
/// above all its neighbours (a local maximum).
fn highest_neighbour(grid: &DensityGrid, cell: usize) -> usize {
    neighbours(grid, cell).fold(cell, |best, other| {
        if ranks_above(&grid.density, other, best) {
            other
        } else {
            best
        }
    })
}

/// The root of `cell` in a forest of parent pointers, compressing the path.
fn find(parent: &mut [usize], cell: usize) -> usize {
    let mut root = cell;
    while parent[root] != root {
        root = parent[root];
    }
    let mut at = cell;
    while parent[at] != root {
        let next = parent[at];
        parent[at] = root;
        at = next;
    }
    root
}

fn raise_edge(graph: &mut BTreeMap<usize, BTreeMap<usize, f64>>, a: usize, b: usize, s: f64) {
    let saddle = graph.entry(a).or_default().entry(b).or_insert(s);
    *saddle = saddle.max(s);
}

/// Saddle over the lower of the two clusters' peaks, or `None` when that peak
/// has no density to compare against.
fn merge_ratio(d: &[f64], a: usize, b: usize, saddle: f64) -> Option<f64> {
    let lower = d[a].min(d[b]);
    (lower > 0.0).then(|| saddle / lower)
}

fn push_candidate(heap: &mut BinaryHeap<Candidate>, d: &[f64], a: usize, b: usize, s: f64) {
    if let Some(ratio) = merge_ratio(d, a, b, s) {
        if ratio >= MERGE_SADDLE_RATIO {
            heap.push(Candidate { ratio, a, b });
        }
    }
}

/// A pair of clusters, named by their peaks with `a < b`, that meets the merge
/// criterion. The heap pops the highest ratio first, then the lower pair.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    ratio: f64,
    a: usize,
    b: usize,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ratio
            .total_cmp(&other.ratio)
            .then((other.a, other.b).cmp(&(self.a, self.b)))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDE: usize = 100;

    /// A `SIDE` × `SIDE` grid over `[0, 100]²` whose density is a sum of
    /// isotropic Gaussians, each `(col, row, height, sigma)` in cell units.
    fn gaussians(peaks: &[(f64, f64, f64, f64)]) -> DensityGrid {
        let mut density = vec![0.0; SIDE * SIDE];
        for row in 0..SIDE {
            for col in 0..SIDE {
                density[row * SIDE + col] = peaks
                    .iter()
                    .map(|&(pc, pr, h, s)| {
                        let d2 = (col as f64 - pc).powi(2) + (row as f64 - pr).powi(2);
                        h * (-d2 / (2.0 * s * s)).exp()
                    })
                    .sum();
            }
        }
        DensityGrid {
            cols: SIDE,
            rows: SIDE,
            x: (0.0, 100.0),
            y: (0.0, 100.0),
            density,
        }
    }

    fn index(col: usize, row: usize) -> usize {
        row * SIDE + col
    }

    #[test]
    fn two_separated_peaks_are_two_regions() {
        let grid = gaussians(&[(20.0, 20.0, 1.0, 5.0), (80.0, 80.0, 2.0, 5.0)]);
        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].peak, index(80, 80));
        assert_eq!(regions[1].peak, index(20, 20));
        assert!(regions[0].peak_density > regions[1].peak_density);
        for region in &regions {
            assert!(region.cells.windows(2).all(|w| w[0] < w[1]));
            assert!(region.cells.contains(&region.peak));
        }
        // The taller peak of equal width holds twice the mass.
        assert!((regions[0].share / regions[1].share - 2.0).abs() < 0.01);
        assert!(regions[0].share + regions[1].share < 1.0);
    }

    #[test]
    fn a_bump_on_a_mountain_merges_into_it() {
        // A small bump on the flank of a wide mountain: it is its own local
        // maximum, but the saddle to the mountain is barely below it.
        let grid = gaussians(&[(50.0, 50.0, 1.0, 12.0), (74.0, 50.0, 0.15, 2.0)]);
        let d = &grid.density;
        let bump = (71..78)
            .map(|col| index(col, 50))
            .max_by(|&a, &b| d[a].total_cmp(&d[b]))
            .unwrap();
        let is_local_max = [-1isize, 1]
            .iter()
            .flat_map(|&dc| [-1isize, 0, 1].map(move |dr| (dc, dr)))
            .chain([(0, -1), (0, 1)])
            .all(|(dc, dr)| d[(bump as isize + dc + dr * SIDE as isize) as usize] < d[bump]);
        assert!(is_local_max, "the bump must be a local maximum of its own");

        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].peak, index(50, 50));
        assert!(regions[0].cells.contains(&bump));
    }

    /// Whether `cells` form one piece under the 8-neighbourhood.
    fn is_connected(cells: &[usize]) -> bool {
        let Some(&first) = cells.first() else {
            return true;
        };
        let mut seen = vec![first];
        let mut stack = vec![first];
        while let Some(cell) = stack.pop() {
            let (col, row) = (cell % SIDE, cell / SIDE);
            for &other in cells {
                let near = (other % SIDE).abs_diff(col) <= 1 && (other / SIDE).abs_diff(row) <= 1;
                if near && !seen.contains(&other) {
                    seen.push(other);
                    stack.push(other);
                }
            }
        }
        seen.len() == cells.len()
    }

    #[test]
    fn a_bump_cut_off_by_truncation_leaves_the_region_in_one_piece() {
        // A bump far down a mountain's flank: it merges, since its peak
        // (about 0.109) stands barely above its saddle (about 0.099), but
        // that saddle lies below a tenth of the mountain's peak, so truncation
        // cuts the bump's top off from the mountain.
        let grid = gaussians(&[(30.0, 50.0, 1.0, 12.0), (61.0, 50.0, 0.07, 2.5)]);
        let d = &grid.density;
        let bump = index(60, 50);
        let is_local_max = (59..=61)
            .flat_map(|col| (49..=51).map(move |row| index(col, row)))
            .all(|c| c == bump || d[c] < d[bump]);
        assert!(is_local_max, "the bump must be a local maximum of its own");
        assert!(d[bump] >= 0.1 * d[index(30, 50)]);

        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 1);
        let region = &regions[0];
        assert_eq!(region.peak, index(30, 50));
        assert!(is_connected(&region.cells));
        assert!(!region.cells.contains(&bump));
        let mass: f64 = region.cells.iter().map(|&c| d[c]).sum();
        let total: f64 = d.iter().sum();
        assert_eq!(region.share, mass / total);
    }

    #[test]
    fn a_chain_merges_through_the_cluster_between() {
        // Three peaks in a row, A > B > C. B merges into A first (saddle over
        // B's peak about 0.91). A and C share almost no boundary of their own,
        // so C then merges only over the saddle it shared with B, which B's
        // merge hands to A (about 0.83 of C's peak).
        let grid = gaussians(&[
            (20.0, 50.0, 1.0, 5.0),
            (31.0, 50.0, 0.85, 4.0),
            (42.0, 50.0, 0.7, 4.0),
        ]);
        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].peak, index(20, 50));
        assert!(regions[0].cells.contains(&index(30, 50)));
        assert!(regions[0].cells.contains(&index(42, 50)));
    }

    #[test]
    fn the_closest_pair_merges_first() {
        // A > C > B, with B low between them, and both of B's saddles meet the
        // criterion: about 0.92 of B's peak towards A, 0.83 towards C. Taking
        // the closer pair first merges B into A, after which C stands well
        // above its saddle and stays apart. The other order would have put B
        // with C.
        let grid = gaussians(&[
            (20.0, 50.0, 1.0, 5.0),
            (31.0, 50.0, 0.5, 3.0),
            (41.0, 50.0, 0.95, 4.0),
        ]);
        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].peak, index(20, 50));
        assert_eq!(regions[1].peak, index(41, 50));
        assert!(regions[0].cells.contains(&index(31, 50)));
        assert!(!regions[1].cells.contains(&index(31, 50)));
    }

    #[test]
    fn regions_are_truncated_at_a_tenth_of_their_peak() {
        let grid = gaussians(&[
            (25.0, 30.0, 1.0, 6.0),
            (75.0, 30.0, 0.5, 8.0),
            (50.0, 75.0, 0.8, 4.0),
        ]);
        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 3);
        for region in &regions {
            let floor = 0.1 * region.peak_density;
            for &cell in &region.cells {
                assert!(grid.density[cell] >= floor);
            }
        }
        // Truncation leaves the low tails out of every region.
        let covered: usize = regions.iter().map(|r| r.cells.len()).sum();
        assert!(covered < SIDE * SIDE / 2);
    }

    #[test]
    fn a_speck_is_not_a_region() {
        // A peak so narrow that only its own cell clears a tenth of its height.
        let grid = gaussians(&[(30.0, 30.0, 1.0, 6.0), (80.0, 80.0, 1.0, 0.4)]);
        let speck_cells = (0..SIDE * SIDE)
            .filter(|&c| {
                let (col, row) = (c % SIDE, c / SIDE);
                col.abs_diff(80) <= 3 && row.abs_diff(80) <= 3 && grid.density[c] >= 0.1
            })
            .count();
        assert!(speck_cells < MIN_REGION_CELLS);

        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].peak, index(30, 30));
    }

    #[test]
    fn empty_and_flat_grids_have_no_regions() {
        let mut grid = gaussians(&[]);
        assert_eq!(find_regions(&grid), vec![]);
        grid.density.fill(0.37);
        assert_eq!(find_regions(&grid), vec![]);
    }

    #[test]
    fn a_single_cloud_is_one_region() {
        let grid = gaussians(&[(40.0, 60.0, 3.0, 10.0)]);
        let regions = find_regions(&grid);
        assert_eq!(regions.len(), 1);
        let region = &regions[0];
        assert_eq!(region.peak, index(40, 60));
        assert_eq!(region.peak_density, grid.density[index(40, 60)]);
        // The 0.1 level of a Gaussian lies at sigma * sqrt(2 ln 10) ≈ 21.5
        // cells, so the region is roughly that disc.
        let disc = std::f64::consts::PI * 21.46_f64.powi(2);
        let n = region.cells.len() as f64;
        assert!((n - disc).abs() / disc < 0.05, "{n} cells vs disc {disc}");
        // Inside that disc is ~90% of the cloud's mass.
        assert!((region.share - 0.9).abs() < 0.02, "share {}", region.share);
    }

    #[test]
    fn equal_neighbours_break_ties_by_cell_index() {
        // A flat-topped block: every cell of the top has equal density.
        let mut grid = gaussians(&[]);
        for row in 40..50 {
            for col in 30..45 {
                grid.density[index(col, row)] = 1.0;
            }
        }
        for row in 38..52 {
            for col in 28..47 {
                if grid.density[index(col, row)] == 0.0 {
                    grid.density[index(col, row)] = 0.5;
                }
            }
        }
        let first = find_regions(&grid);
        for _ in 0..20 {
            assert_eq!(find_regions(&grid), first);
        }
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].peak, index(30, 40));
        assert_eq!(first[0].cells.len(), 14 * 19);
    }

    #[test]
    fn cell_of_and_cell_bounds_agree() {
        let grid = DensityGrid {
            cols: 7,
            rows: 5,
            x: (-3.0, 11.0),
            y: (100.0, 102.5),
            density: vec![0.0; 35],
        };
        for cell in 0..35 {
            let ((x0, x1), (y0, y1)) = cell_bounds(&grid, cell);
            assert!(x0 < x1 && y0 < y1);
            let centre = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            assert_eq!(cell_of(&grid, centre.0, centre.1), Some(cell));
        }
        assert_eq!(cell_bounds(&grid, 0), ((-3.0, -1.0), (100.0, 100.5)));
        // The top edges belong to the last column and the last row.
        assert_eq!(cell_of(&grid, 11.0, 102.5), Some(34));
        assert_eq!(cell_of(&grid, 11.0, 100.0), Some(6));
        assert_eq!(cell_of(&grid, -3.0, 102.5), Some(28));
        assert_eq!(cell_of(&grid, -3.0, 100.0), Some(0));
        // Outside the grid, or not a number, is no cell.
        assert_eq!(cell_of(&grid, -3.1, 101.0), None);
        assert_eq!(cell_of(&grid, 0.0, 102.6), None);
        assert_eq!(cell_of(&grid, f64::NAN, 101.0), None);
    }

    /// A 10 × 10 grid over `[0, 10]²`, so a cell's data range is its own
    /// column and row, and a region built from the given `(col, row)` cells.
    fn small(cells: &[(usize, usize)], peak: (usize, usize)) -> (DensityGrid, Region) {
        let grid = DensityGrid {
            cols: 10,
            rows: 10,
            x: (0.0, 10.0),
            y: (0.0, 10.0),
            density: vec![1.0; 100],
        };
        let mut cells: Vec<usize> = cells.iter().map(|&(c, r)| r * 10 + c).collect();
        cells.sort_unstable();
        let region = Region {
            peak: peak.1 * 10 + peak.0,
            peak_density: 1.0,
            cells,
            share: 1.0,
        };
        (grid, region)
    }

    fn block(cols: std::ops::Range<usize>, rows: std::ops::Range<usize>) -> Vec<(usize, usize)> {
        rows.flat_map(|r| cols.clone().map(move |c| (c, r)))
            .collect()
    }

    #[test]
    fn an_l_shaped_region_has_one_ring_of_six_corners() {
        // Three cells along the bottom and two more up the left side.
        let (grid, region) = small(&[(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)], (0, 0));
        let rings = region_outline(&grid, &region);
        assert_eq!(
            rings,
            vec![vec![
                (0.0, 0.0),
                (3.0, 0.0),
                (3.0, 1.0),
                (1.0, 1.0),
                (1.0, 3.0),
                (0.0, 3.0),
            ]]
        );
    }

    #[test]
    fn a_region_with_a_hole_has_two_rings() {
        let mut cells = block(2..5, 2..5);
        cells.retain(|&c| c != (3, 3));
        let (grid, region) = small(&cells, (2, 2));
        let rings = region_outline(&grid, &region);
        assert_eq!(rings.len(), 2);
        // The outer ring first, anticlockwise from its lowest-left corner.
        assert_eq!(
            rings[0],
            vec![(2.0, 2.0), (5.0, 2.0), (5.0, 5.0), (2.0, 5.0)]
        );
        // The hole runs the other way round.
        assert_eq!(
            rings[1],
            vec![(3.0, 3.0), (3.0, 4.0), (4.0, 4.0), (4.0, 3.0)]
        );
    }

    #[test]
    fn cells_that_touch_at_a_corner_share_one_ring_that_touches_itself() {
        let (grid, region) = small(&[(1, 1), (2, 2)], (1, 1));
        let rings = region_outline(&grid, &region);
        assert_eq!(rings.len(), 1);
        // Eight corners: the pinch at (2, 2) is visited twice, and the walk
        // goes round the lower cell, then the upper one.
        assert_eq!(
            rings[0],
            vec![
                (1.0, 1.0),
                (2.0, 1.0),
                (2.0, 2.0),
                (3.0, 2.0),
                (3.0, 3.0),
                (2.0, 3.0),
                (2.0, 2.0),
                (1.0, 2.0),
            ]
        );
    }

    #[test]
    fn a_hole_that_touches_the_outside_at_a_corner_is_still_a_hole() {
        // A 3 x 3 block missing its centre and its top-right corner: the
        // centre cell is closed in on its four sides and meets the gap in the
        // corner only at a point.
        let mut cells = block(1..4, 1..4);
        cells.retain(|&c| c != (2, 2) && c != (3, 3));
        let (grid, region) = small(&cells, (1, 1));
        let rings = region_outline(&grid, &region);
        assert_eq!(rings.len(), 2, "{rings:?}");
        assert_eq!(
            rings[1],
            vec![(2.0, 2.0), (2.0, 3.0), (3.0, 3.0), (3.0, 2.0)]
        );
    }

    #[test]
    fn outlines_are_the_same_on_every_run() {
        let (grid, region) = small(&block(2..6, 3..7), (3, 4));
        assert_eq!(
            region_outline(&grid, &region),
            region_outline(&grid, &region)
        );
        assert_eq!(label_anchor(&grid, &region), label_anchor(&grid, &region));
    }

    #[test]
    fn the_anchor_sits_inside_its_region() {
        // A thick C: a square annulus with its right side cut away, so the
        // bounding box's centre lies in the hollow.
        let mut cells = block(1..9, 1..9);
        cells.retain(|&(c, r)| !((3..7).contains(&c) && (3..7).contains(&r)));
        cells.retain(|&(c, _)| c < 7);
        let (grid, region) = small(&cells, (1, 4));
        let (x, y) = label_anchor(&grid, &region);
        let cell = cell_of(&grid, x, y).expect("the anchor is on the grid");
        assert!(
            region.cells.contains(&cell),
            "anchor ({x}, {y}) is in the hollow"
        );
    }

    #[test]
    fn the_anchor_is_near_the_peak() {
        let (grid, region) = small(&block(0..10, 0..10), (5, 5));
        let (x, y) = label_anchor(&grid, &region);
        let peak_centre = (5.5, 5.5);
        assert!((x - peak_centre.0).abs() <= 3.0 && (y - peak_centre.1).abs() <= 3.0);
    }

    #[test]
    fn among_equal_squares_the_anchor_takes_the_one_nearest_the_peak() {
        // A bar three cells high and eight long: every 3 x 3 block is as big,
        // and the peak sits at the right-hand end.
        let (grid, region) = small(&block(1..9, 2..5), (8, 3));
        assert_eq!(label_anchor(&grid, &region), (7.5, 3.5));
        // With the peak at the left end, the leftmost block wins.
        let (grid, region) = small(&block(1..9, 2..5), (1, 3));
        assert_eq!(label_anchor(&grid, &region), (2.5, 3.5));
    }

    #[test]
    fn region_at_finds_the_region_under_a_point_and_none_in_the_gap() {
        let (grid, left) = small(&block(1..3, 1..3), (1, 1));
        let (_, right) = small(&block(6..9, 6..9), (6, 6));
        let regions = [left, right];
        assert_eq!(region_at(&grid, &regions, 1.5, 1.5), Some(0));
        assert_eq!(region_at(&grid, &regions, 7.0, 8.9), Some(1));
        assert_eq!(region_at(&grid, &regions, 4.5, 4.5), None);
        assert_eq!(region_at(&grid, &regions, -1.0, 1.5), None);
    }
}
