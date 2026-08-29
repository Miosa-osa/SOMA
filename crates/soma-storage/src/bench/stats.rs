//! Nearest-rank percentiles over raw nanosecond samples.

use serde::{Deserialize, Serialize};

/// Percentiles of one sample set; every value is in nanoseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Percentiles {
    /// Sample count.
    pub n: usize,
    /// Smallest sample.
    pub min: u64,
    /// Integer mean.
    pub mean: u64,
    /// Nearest-rank median.
    pub p50: u64,
    /// Nearest-rank 95th percentile.
    pub p95: u64,
    /// Nearest-rank 99th percentile.
    pub p99: u64,
    /// Largest sample.
    pub max: u64,
}

impl Percentiles {
    /// Computes nearest-rank percentiles: the value at rank `ceil(p * n / 100)` of the sorted
    /// samples, so p99 of 100 samples is the 99th smallest and of 200 samples the 198th.
    ///
    /// An empty slice yields all zeros with `n == 0`.
    #[must_use]
    pub fn of(samples: &[u64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let n = sorted.len();
        let rank = |percent: usize| -> u64 {
            let index = (percent * n).div_ceil(100).max(1) - 1;
            sorted[index.min(n - 1)]
        };
        let sum: u128 = sorted.iter().map(|value| u128::from(*value)).sum();
        let mean = u64::try_from(sum / n as u128).unwrap_or(u64::MAX);
        Self {
            n,
            min: sorted[0],
            mean,
            p50: rank(50),
            p95: rank(95),
            p99: rank(99),
            max: sorted[n - 1],
        }
    }
}

/// Converts a duration to whole nanoseconds, saturating at `u64::MAX`.
#[must_use]
pub fn nanos(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_matches_hand_computed_values() {
        let samples: Vec<u64> = (1..=100).collect();
        let stats = Percentiles::of(&samples);
        assert_eq!(stats.n, 100);
        assert_eq!(stats.min, 1);
        assert_eq!(stats.max, 100);
        assert_eq!(stats.p50, 50);
        assert_eq!(stats.p95, 95);
        assert_eq!(stats.p99, 99);
        assert_eq!(stats.mean, 50);

        let two_hundred: Vec<u64> = (1..=200).rev().collect();
        let stats = Percentiles::of(&two_hundred);
        assert_eq!(stats.p99, 198);
        assert_eq!(stats.p50, 100);
    }

    #[test]
    fn tiny_and_empty_sets_are_handled() {
        assert_eq!(Percentiles::of(&[]), Percentiles::default());
        let one = Percentiles::of(&[7]);
        assert_eq!(
            (one.n, one.min, one.p50, one.p99, one.max, one.mean),
            (1, 7, 7, 7, 7, 7)
        );
        let three = Percentiles::of(&[3, 1, 2]);
        assert_eq!((three.p50, three.p95, three.p99), (2, 3, 3));
    }

    #[test]
    fn nanos_saturates() {
        assert_eq!(nanos(std::time::Duration::from_nanos(5)), 5);
        assert_eq!(nanos(std::time::Duration::MAX), u64::MAX);
    }
}
