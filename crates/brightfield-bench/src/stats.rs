//! Summary statistics over a sample of millisecond timings.

use serde::Serialize;

/// Distribution summary of one timed quantity, in milliseconds.
///
/// Percentiles are nearest-rank over the sorted sample — no interpolation, so
/// every reported number is a value that was actually observed.
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    /// Sample size.
    pub n: usize,
    /// Minimum, ms.
    pub min_ms: f64,
    /// Arithmetic mean, ms.
    pub mean_ms: f64,
    /// Median (nearest-rank), ms.
    pub p50_ms: f64,
    /// 95th percentile (nearest-rank), ms.
    pub p95_ms: f64,
    /// Maximum, ms.
    pub max_ms: f64,
}

impl Stats {
    /// Summarise `samples` (milliseconds). Returns `None` on an empty sample —
    /// a benchmark that produced no observations must not report numbers.
    pub fn from_ms(mut samples: Vec<f64>) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).expect("timings are finite"));
        let n = samples.len();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let rank = |p: f64| -> f64 {
            // Nearest-rank: ceil(p * n), 1-based, clamped.
            let r = (p * n as f64).ceil() as usize;
            samples[r.clamp(1, n) - 1]
        };
        Some(Self {
            n,
            min_ms: round3(samples[0]),
            mean_ms: round3(mean),
            p50_ms: round3(rank(0.50)),
            p95_ms: round3(rank(0.95)),
            max_ms: round3(samples[n - 1]),
        })
    }

    /// Summarise a sample of [`std::time::Duration`]s.
    pub fn from_durations(samples: &[std::time::Duration]) -> Option<Self> {
        Self::from_ms(samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect())
    }
}

/// Round to 3 decimal places (microsecond resolution in ms) so the committed
/// JSON stays readable and platform-stable in its textual form.
fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sample_reports_nothing() {
        assert!(Stats::from_ms(vec![]).is_none());
    }

    #[test]
    fn nearest_rank_percentiles_are_observed_values() {
        let s = Stats::from_ms((1..=100).map(f64::from).collect()).expect("non-empty");
        assert_eq!(s.n, 100);
        assert_eq!(s.min_ms, 1.0);
        assert_eq!(s.p50_ms, 50.0);
        assert_eq!(s.p95_ms, 95.0);
        assert_eq!(s.max_ms, 100.0);
        assert_eq!(s.mean_ms, 50.5);
    }

    #[test]
    fn single_sample_is_its_own_summary() {
        let s = Stats::from_ms(vec![7.0]).expect("non-empty");
        assert_eq!(s.n, 1);
        assert_eq!(s.min_ms, 7.0);
        assert_eq!(s.p50_ms, 7.0);
        assert_eq!(s.p95_ms, 7.0);
        assert_eq!(s.max_ms, 7.0);
    }
}
