//! Marching squares — iso-lines over a row-major scalar grid.
//!
//! The contour mark's geometry pass (density marks): given the
//! KDE-smoothed grid the density lowerer's batch reconstructs (see
//! `mark::build_kde_grid`), extract the iso-lines at N levels as data-space
//! polylines. Pure geometry, no scene or scale dependencies, so the topology
//! is testable headlessly.
//!
//! Grid convention matches `KdeGrid`: `values[row * cols + col]` is the field
//! sampled at `(xs[col], ys[row])`. Cells are the rectangles between four
//! adjacent grid points; each crossing cell contributes one or two segments
//! (the two ambiguous saddle cases are resolved by the cell-centre average),
//! and segments are chained into polylines by their shared endpoints. Shared
//! cell edges interpolate from identical inputs, so endpoint equality is
//! exact (bitwise) — chaining needs no tolerance.

/// Evenly-spaced iso-levels strictly inside `(0, max)`: `max · i / (n + 1)`
/// for `i in 1..=n`. Both endpoints are excluded — a level at 0 traces the
/// (degenerate) empty background and a level at `max` collapses to the peak.
pub fn iso_levels(max: f64, n: usize) -> Vec<f64> {
    (1..=n).map(|i| max * i as f64 / (n as f64 + 1.0)).collect()
}

/// One endpoint of a marching-squares segment, on a cell edge.
type Point = (f64, f64);

/// Extract the iso-lines of `values` at `level` as polylines in data space.
///
/// `values` is row-major with `rows * cols` entries sampled at the
/// `(xs[col], ys[row])` lattice. Returns each connected iso-line as a vertex
/// list; a CLOSED loop repeats its first vertex at the end. Lines that meet
/// the grid boundary stay open (the field is only known inside the lattice).
/// Degenerate inputs (fewer than 2 rows/cols, mismatched lengths) return
/// nothing.
pub fn contour_polylines(
    values: &[f64],
    rows: usize,
    cols: usize,
    xs: &[f64],
    ys: &[f64],
    level: f64,
) -> Vec<Vec<Point>> {
    if rows < 2 || cols < 2 || values.len() != rows * cols || xs.len() != cols || ys.len() != rows {
        return Vec::new();
    }

    // Interpolate the crossing point on the edge between two grid points.
    // Only called when the corners straddle `level`, so `vb != va`.
    let cross = |pa: Point, va: f64, pb: Point, vb: f64| -> Point {
        let t = (level - va) / (vb - va);
        (pa.0 + t * (pb.0 - pa.0), pa.1 + t * (pb.1 - pa.1))
    };

    let mut segments: Vec<(Point, Point)> = Vec::new();
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            // Corners: BL/BR on grid row r, TL/TR on row r+1 (in index space —
            // pixel orientation is the scales' concern, not the geometry's).
            let v_bl = values[r * cols + c];
            let v_br = values[r * cols + c + 1];
            let v_tr = values[(r + 1) * cols + c + 1];
            let v_tl = values[(r + 1) * cols + c];
            let p_bl = (xs[c], ys[r]);
            let p_br = (xs[c + 1], ys[r]);
            let p_tr = (xs[c + 1], ys[r + 1]);
            let p_tl = (xs[c], ys[r + 1]);

            let case = usize::from(v_bl >= level)
                | usize::from(v_br >= level) << 1
                | usize::from(v_tr >= level) << 2
                | usize::from(v_tl >= level) << 3;
            if case == 0 || case == 15 {
                continue;
            }

            // Edge crossings, computed with the LOWER-INDEX corner first so an
            // edge shared by two neighbouring cells interpolates from identical
            // inputs and lands on the identical f64 point.
            let bottom = || cross(p_bl, v_bl, p_br, v_br);
            let right = || cross(p_br, v_br, p_tr, v_tr);
            let top = || cross(p_tl, v_tl, p_tr, v_tr);
            let left = || cross(p_bl, v_bl, p_tl, v_tl);

            match case {
                1 | 14 => segments.push((left(), bottom())),
                2 | 13 => segments.push((bottom(), right())),
                3 | 12 => segments.push((left(), right())),
                4 | 11 => segments.push((right(), top())),
                6 | 9 => segments.push((bottom(), top())),
                7 | 8 => segments.push((left(), top())),
                // Saddles: disambiguate by the cell-centre average.
                5 | 10 => {
                    let centre = (v_bl + v_br + v_tr + v_tl) / 4.0;
                    let centre_above = centre >= level;
                    let join_bl_tr = (case == 5) == centre_above;
                    if join_bl_tr {
                        // The two above-level corners (BL, TR for case 5)
                        // connect through the centre.
                        segments.push((left(), top()));
                        segments.push((bottom(), right()));
                    } else {
                        segments.push((left(), bottom()));
                        segments.push((right(), top()));
                    }
                }
                _ => unreachable!("cases 0 and 15 are filtered above"),
            }
        }
    }

    chain_segments(&segments)
}

/// Exact-equality key for a segment endpoint (see module doc: shared edges
/// interpolate from identical inputs, so no tolerance is needed).
fn key(p: Point) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

/// Chain loose segments into polylines by walking shared endpoints. Each
/// endpoint joins at most two segments (marching squares is manifold), so the
/// walk is a simple extend-both-ends pass per unvisited segment.
fn chain_segments(segments: &[(Point, Point)]) -> Vec<Vec<Point>> {
    use std::collections::HashMap;

    // endpoint key → indices of the segments touching it.
    let mut touching: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        touching.entry(key(*a)).or_default().push(i);
        touching.entry(key(*b)).or_default().push(i);
    }

    let mut visited = vec![false; segments.len()];
    let mut polylines: Vec<Vec<Point>> = Vec::new();

    // Extend from `point` away from segment `from`, appending to `out`.
    let extend = |start: Point, from: usize, visited: &mut Vec<bool>, out: &mut Vec<Point>| {
        let (mut point, mut prev) = (start, from);
        loop {
            let next = touching
                .get(&key(point))
                .and_then(|idxs| idxs.iter().find(|&&i| i != prev && !visited[i]))
                .copied();
            let Some(i) = next else { break };
            visited[i] = true;
            let (a, b) = segments[i];
            point = if key(a) == key(point) { b } else { a };
            out.push(point);
            prev = i;
        }
    };

    for i in 0..segments.len() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let (a, b) = segments[i];

        // Walk forward from b, then backward from a; the polyline is the
        // reversed backward walk + [a, b] + the forward walk.
        let mut forward: Vec<Point> = vec![a, b];
        extend(b, i, &mut visited, &mut forward);
        let mut backward: Vec<Point> = Vec::new();
        extend(a, i, &mut visited, &mut backward);
        backward.reverse();
        backward.extend(forward);
        polylines.push(backward);
    }

    polylines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bilinear interpolation of the grid field at an arbitrary point —
    /// marching-squares vertices sit on cell edges, where bilinear reduces to
    /// the same linear interpolation that placed them, so a correct vertex
    /// evaluates back to the level (up to float rounding).
    fn bilinear(
        values: &[f64],
        rows: usize,
        cols: usize,
        xs: &[f64],
        ys: &[f64],
        p: (f64, f64),
    ) -> f64 {
        let c = xs.iter().rposition(|x| *x <= p.0).unwrap().min(cols - 2);
        let r = ys.iter().rposition(|y| *y <= p.1).unwrap().min(rows - 2);
        let tx = (p.0 - xs[c]) / (xs[c + 1] - xs[c]);
        let ty = (p.1 - ys[r]) / (ys[r + 1] - ys[r]);
        let v00 = values[r * cols + c];
        let v10 = values[r * cols + c + 1];
        let v01 = values[(r + 1) * cols + c];
        let v11 = values[(r + 1) * cols + c + 1];
        v00 * (1.0 - tx) * (1.0 - ty)
            + v10 * tx * (1.0 - ty)
            + v01 * (1.0 - tx) * ty
            + v11 * tx * ty
    }

    // a single-Gaussian grid yields one CLOSED iso-loop per level —
    // loop count matches the requested threshold count — and every vertex
    // evaluates back to its level through the grid (it brackets the crossing).
    #[test]
    fn single_gaussian_yields_closed_loops_at_each_level() {
        let (rows, cols) = (17, 17);
        let xs: Vec<f64> = (0..cols).map(|c| c as f64).collect();
        let ys: Vec<f64> = (0..rows).map(|r| r as f64).collect();
        let (cx, cy, sigma) = (8.0, 8.0, 3.0);
        let values: Vec<f64> = (0..rows * cols)
            .map(|i| {
                let (r, c) = (i / cols, i % cols);
                let d2 = (c as f64 - cx).powi(2) + (r as f64 - cy).powi(2);
                (-d2 / (2.0 * sigma * sigma)).exp()
            })
            .collect();
        let max = values.iter().cloned().fold(0.0_f64, f64::max);

        let thresholds = 3;
        let levels = iso_levels(max, thresholds);
        assert_eq!(levels.len(), thresholds);
        let mut total_loops = 0;
        for level in &levels {
            let lines = contour_polylines(&values, rows, cols, &xs, &ys, *level);
            assert_eq!(
                lines.len(),
                1,
                "a radial Gaussian has exactly one iso-line at level {level}"
            );
            let line = &lines[0];
            assert!(
                line.len() >= 8,
                "the loop has real extent ({} vertices)",
                line.len()
            );
            assert_eq!(
                line.first(),
                line.last(),
                "the iso-line closes (first vertex repeated at the end)"
            );
            for p in line {
                let v = bilinear(&values, rows, cols, &xs, &ys, *p);
                assert!(
                    (v - level).abs() < 1e-9,
                    "vertex {p:?} evaluates to {v}, expected level {level}"
                );
            }
            total_loops += lines.len();
        }
        assert_eq!(
            total_loops, thresholds,
            "one closed loop per requested level"
        );
    }

    // Iso-levels are evenly spaced strictly inside (0, max).
    #[test]
    fn iso_levels_exclude_endpoints() {
        let levels = iso_levels(8.0, 3);
        assert_eq!(levels, vec![2.0, 4.0, 6.0]);
        assert!(iso_levels(8.0, 0).is_empty());
    }

    // A field crossing the grid boundary yields an OPEN polyline (the field is
    // unknown outside the lattice), still chained into a single line.
    #[test]
    fn boundary_crossing_yields_open_polyline() {
        // 2×3 ramp along x: one vertical iso-line crossing top and bottom.
        let values = vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0];
        let xs = vec![0.0, 1.0, 2.0];
        let ys = vec![0.0, 1.0];
        let lines = contour_polylines(&values, 2, 3, &xs, &ys, 0.5);
        assert_eq!(lines.len(), 1, "one connected iso-line");
        let line = &lines[0];
        assert_ne!(line.first(), line.last(), "boundary lines stay open");
        for p in line {
            assert!((p.0 - 0.5).abs() < 1e-12, "the 0.5 level sits at x = 0.5");
        }
    }

    // Degenerate inputs return no geometry rather than panicking.
    #[test]
    fn degenerate_grids_yield_nothing() {
        assert!(contour_polylines(&[1.0, 2.0], 1, 2, &[0.0, 1.0], &[0.0], 1.5).is_empty());
        assert!(contour_polylines(&[], 0, 0, &[], &[], 0.5).is_empty());
        // Mismatched lengths.
        assert!(contour_polylines(&[1.0; 6], 2, 3, &[0.0, 1.0], &[0.0, 1.0], 0.5).is_empty());
    }
}
