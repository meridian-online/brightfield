//! Kernel density estimation — pure Rust, no new deps.
//!
//! Implements 1D and 2D Gaussian KDE via discretised convolution over a
//! pre-binned histogram. The 2D Gaussian is separable, so 2D convolution is
//! row-wise then column-wise 1D passes — both run on flat `Vec` buffers with
//! row-stride indexing.
//!
//! Bandwidth selection follows Silverman's rule of thumb when the spec omits
//! `bandwidth`. All bandwidths are in **data units** (matching Mosaic web
//! defaults), not pixel units.
//!
//! The kernel is truncated at ±3σ — values beyond contribute < 0.3% mass.

/// 1D Gaussian KDE over a histogram.
///
/// `bins[i]` is the count of samples falling into bucket `i`, where bucket `i`
/// covers data range `[origin + i*bin_size, origin + (i+1)*bin_size)`.
///
/// Returns a density estimate of the same length, normalised so that
/// `sum(out) * bin_size == sum(bins)`. (i.e. integrated density equals total
/// sample count when integrating over the data domain.)
///
/// `bandwidth` is in data units (the same units as the original samples /
/// `bin_size`). It is *not* a pixel quantity.
pub fn kde_1d(bins: &[u32], bandwidth: f64, bin_size: f64) -> Vec<f64> {
    let n = bins.len();
    if n == 0 || bandwidth <= 0.0 || bin_size <= 0.0 {
        return vec![0.0; n];
    }

    // Build truncated kernel weights at ±3σ in data-unit terms,
    // then convert offset to bin steps via bin_size.
    let radius_data = 3.0 * bandwidth;
    let radius_bins = (radius_data / bin_size).ceil() as isize;

    // kernel[k] for k in -radius..=radius, gaussian density at offset k*bin_size.
    let two_sigma_sq = 2.0 * bandwidth * bandwidth;
    let norm = 1.0 / (bandwidth * (2.0 * std::f64::consts::PI).sqrt());
    let kernel: Vec<f64> = (-radius_bins..=radius_bins)
        .map(|k| {
            let x = (k as f64) * bin_size;
            norm * (-(x * x) / two_sigma_sq).exp()
        })
        .collect();

    // Convolve.
    let mut out = vec![0.0; n];
    for i in 0..n {
        let mut acc = 0.0;
        for (k_idx, &kw) in kernel.iter().enumerate() {
            let k = k_idx as isize - radius_bins;
            let j = i as isize + k;
            if j < 0 || j >= n as isize {
                continue;
            }
            // Each unit of count at bin j contributes kw * bin_size mass per bin
            // in the convolved output (kernel was evaluated as a density).
            // We multiply by bin_size so that out has units of "samples per data unit".
            acc += (bins[j as usize] as f64) * kw * bin_size;
        }
        out[i] = acc;
    }
    out
}

/// 2D Gaussian KDE over a flat row-major histogram.
///
/// `bins` has length `shape.0 * shape.1` where `shape.0` is the number of rows
/// (y-bins) and `shape.1` is the number of columns (x-bins). Index of cell
/// `(row, col)` is `row * shape.1 + col`.
///
/// `bandwidth = (h_x, h_y)` and `bin_size = (dx, dy)` are both `(x, y)` pairs
/// in data units.
///
/// The 2D Gaussian is separable — convolves along x (within each row) then
/// along y (within each column), both via the 1D pipeline above.
///
/// Returns a flat row-major `Vec<f64>` of the same shape and stride.
pub fn kde_2d(
    bins: &[u32],
    shape: (usize, usize),
    bandwidth: (f64, f64),
    bin_size: (f64, f64),
) -> Vec<f64> {
    let (rows, cols) = shape;
    let (h_x, h_y) = bandwidth;
    let (dx, dy) = bin_size;

    if bins.len() != rows * cols {
        return vec![0.0; rows * cols];
    }
    if rows == 0 || cols == 0 {
        return Vec::new();
    }

    // Pass 1: convolve x within each row. Convert u32 -> f64 row by row.
    let mut intermediate = vec![0.0_f64; rows * cols];
    for r in 0..rows {
        let row_slice = &bins[r * cols..(r + 1) * cols];
        let row_density = kde_1d(row_slice, h_x, dx);
        for c in 0..cols {
            intermediate[r * cols + c] = row_density[c];
        }
    }

    // Pass 2: convolve y within each column. We need an f64 1D KDE; reuse the
    // primitive on a temporary u32 buffer wouldn't work because kde_1d only
    // accepts u32 counts. Inline a Gaussian convolution over f64 here.
    let radius_data = 3.0 * h_y;
    let radius_bins = (radius_data / dy).ceil() as isize;
    let two_sigma_sq = 2.0 * h_y * h_y;
    let norm = 1.0 / (h_y * (2.0 * std::f64::consts::PI).sqrt());
    let kernel: Vec<f64> = (-radius_bins..=radius_bins)
        .map(|k| {
            let y = (k as f64) * dy;
            norm * (-(y * y) / two_sigma_sq).exp()
        })
        .collect();

    let mut out = vec![0.0_f64; rows * cols];
    for c in 0..cols {
        for r in 0..rows {
            let mut acc = 0.0;
            for (k_idx, &kw) in kernel.iter().enumerate() {
                let k = k_idx as isize - radius_bins;
                let rj = r as isize + k;
                if rj < 0 || rj >= rows as isize {
                    continue;
                }
                acc += intermediate[(rj as usize) * cols + c] * kw * dy;
            }
            out[r * cols + c] = acc;
        }
    }
    out
}

/// Silverman's rule of thumb for 1D bandwidth.
///
/// `h = 1.06 · σ̂ · n^(-1/5)`
///
/// `samples` is the raw 1D data. Returns `0.0` for `n < 2` or zero variance
/// — caller is expected to fall back (e.g. avoid rendering an empty mark).
pub fn silverman_1d(samples: &[f64]) -> f64 {
    let n = samples.len();
    if n < 2 {
        return 0.0;
    }
    let n_f = n as f64;
    let mean = samples.iter().sum::<f64>() / n_f;
    let var = samples
        .iter()
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum::<f64>()
        / (n_f - 1.0);
    let sigma = var.sqrt();
    if sigma == 0.0 {
        return 0.0;
    }
    1.06 * sigma * n_f.powf(-1.0 / 5.0)
}

/// Silverman's rule for 2D, applied per-axis.
///
/// `h_axis = 0.9 · min(σ̂, IQR̂ / 1.34) · n^(-1/6)`
///
/// IQR is computed via order-statistic estimation on a sorted copy
/// (Q1 = 25th percentile, Q3 = 75th percentile, linear interpolation).
///
/// `xs` and `ys` must have the same length. Returns `(h_x, h_y)` —
/// `0.0` for either axis when degenerate.
pub fn silverman_2d_per_axis(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len();
    debug_assert_eq!(n, ys.len(), "silverman_2d_per_axis: x/y length mismatch");
    if n < 2 {
        return (0.0, 0.0);
    }
    (silverman_axis(xs), silverman_axis(ys))
}

fn silverman_axis(samples: &[f64]) -> f64 {
    let n = samples.len();
    if n < 2 {
        return 0.0;
    }
    let n_f = n as f64;
    let mean = samples.iter().sum::<f64>() / n_f;
    let var = samples
        .iter()
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum::<f64>()
        / (n_f - 1.0);
    let sigma = var.sqrt();

    let mut sorted: Vec<f64> = samples.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q1 = quantile(&sorted, 0.25);
    let q3 = quantile(&sorted, 0.75);
    let iqr = q3 - q1;

    let scale = if sigma == 0.0 && iqr == 0.0 {
        return 0.0;
    } else if sigma == 0.0 {
        iqr / 1.34
    } else if iqr == 0.0 {
        sigma
    } else {
        sigma.min(iqr / 1.34)
    };

    0.9 * scale * n_f.powf(-1.0 / 6.0)
}

/// Linear-interpolation quantile on a pre-sorted ascending slice.
fn quantile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted[0];
    }
    let pos = p * (n as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gomb_ac02_kde_1d_uniform_density() {
        // Single sample at bin 5 — peak should be at bin 5, symmetric.
        let bins: Vec<u32> = (0..11)
            .map(|i| if i == 5 { 1 } else { 0 })
            .collect();
        let density = kde_1d(&bins, 1.0, 1.0);
        assert_eq!(density.len(), 11);
        // Peak at bin 5.
        let peak = density[5];
        for i in 0..11 {
            assert!(
                density[i] <= peak + 1e-12,
                "bin {i} density {} > peak {peak}",
                density[i]
            );
        }
        // Symmetry: bin 4 ≈ bin 6, bin 3 ≈ bin 7.
        assert!((density[4] - density[6]).abs() < 1e-9);
        assert!((density[3] - density[7]).abs() < 1e-9);
    }

    #[test]
    fn gomb_ac02_kde_1d_zero_bandwidth_safe() {
        let bins = vec![0u32, 1, 0];
        let density = kde_1d(&bins, 0.0, 1.0);
        assert_eq!(density, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn gomb_ac02_kde_2d_separable_peak() {
        // 5x5 grid, single sample at (2, 2); peak should be at (2, 2).
        let mut bins = vec![0u32; 25];
        bins[2 * 5 + 2] = 1;
        let density = kde_2d(&bins, (5, 5), (1.0, 1.0), (1.0, 1.0));
        assert_eq!(density.len(), 25);
        let peak = density[2 * 5 + 2];
        for r in 0..5 {
            for c in 0..5 {
                assert!(
                    density[r * 5 + c] <= peak + 1e-12,
                    "({r},{c}) > peak"
                );
            }
        }
        // Symmetry across both axes.
        assert!((density[1 * 5 + 2] - density[3 * 5 + 2]).abs() < 1e-9);
        assert!((density[2 * 5 + 1] - density[2 * 5 + 3]).abs() < 1e-9);
    }

    #[test]
    fn gomb_ac02_silverman_1d_known_value() {
        // n=100 samples from N(0,1)-like uniform sequence — sigma ≈ 1.
        // Silverman = 1.06 * sigma * n^(-1/5).
        let samples: Vec<f64> = (0..100).map(|i| (i as f64) / 99.0 - 0.5).collect();
        let h = silverman_1d(&samples);
        // Variance of uniform on [-0.5, 0.5] ≈ 1/12 ⇒ σ ≈ 0.289.
        // Expected ≈ 1.06 * 0.289 * 100^(-0.2) ≈ 0.122.
        assert!(
            h > 0.05 && h < 0.30,
            "silverman_1d unexpected: {h}"
        );
    }

    #[test]
    fn gomb_ac02_silverman_1d_degenerate_returns_zero() {
        assert_eq!(silverman_1d(&[]), 0.0);
        assert_eq!(silverman_1d(&[1.0]), 0.0);
        assert_eq!(silverman_1d(&[5.0, 5.0, 5.0, 5.0]), 0.0);
    }

    #[test]
    fn gomb_ac02_silverman_2d_per_axis_independent() {
        // x with wider spread than y → h_x > h_y.
        let xs: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let ys: Vec<f64> = (0..50).map(|i| (i as f64) * 0.1).collect();
        let (h_x, h_y) = silverman_2d_per_axis(&xs, &ys);
        assert!(h_x > h_y, "h_x={h_x} h_y={h_y}");
        assert!(h_y > 0.0);
    }
}
