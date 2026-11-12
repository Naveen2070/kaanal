//! A latency histogram with log-spaced buckets (step 8L, RULES 105): the page sums the histograms
//! of the ticks in its window and reads p50 and p99 from the sum, so the figures need no list of
//! every request and the wire carries only the buckets that hold something.
//!
//! Buckets are geometric with ratio [`RATIO`], so any value is within about 1% of the middle of its
//! bucket (two significant digits, the precision of `HdrHistogram` at 2 digits): a quantile read from
//! the buckets is within 1% of the exact one. The range is [`MIN_NS`] to [`MAX_NS`]; a smaller
//! value shares the first bucket, a larger one the last. About a thousand buckets cover the range;
//! a tick sends only the occupied ones.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "latencies and bucket indexes cross between integers and f64 by design; the range is far below 2^52"
)]

/// Smallest value with its own bucket, nanoseconds (10 microseconds).
const MIN_NS: f64 = 10_000.0;
/// Largest value with its own bucket, nanoseconds (1,000 seconds).
const MAX_NS: f64 = 1_000_000_000_000.0;
/// Width of a bucket as a ratio: half of it is the worst relative error, about 1%.
const RATIO: f64 = 1.0201;

/// Number of buckets: one for everything under [`MIN_NS`], then one per [`RATIO`] step to [`MAX_NS`].
pub(crate) fn bucket_count() -> usize {
    index(MAX_NS as u64) + 1
}

/// The bucket of `ns`.
fn index(ns: u64) -> usize {
    let v = ns as f64;
    if v < MIN_NS {
        return 0;
    }
    let steps = (v.min(MAX_NS) / MIN_NS).ln() / RATIO.ln();
    1 + steps as usize
}

/// The value a bucket stands for: the geometric middle of its range.
#[cfg(test)]
fn middle(i: usize) -> u64 {
    if i == 0 {
        return (MIN_NS / 2.0) as u64;
    }
    (MIN_NS * RATIO.powf(i as f64 - 0.5)) as u64
}

/// Counts of latencies by bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatencyHist {
    counts: Vec<u32>,
    total: u64,
}

impl Default for LatencyHist {
    fn default() -> Self {
        LatencyHist::new()
    }
}

impl LatencyHist {
    pub(crate) fn new() -> LatencyHist {
        LatencyHist {
            counts: vec![0; bucket_count()],
            total: 0,
        }
    }

    /// Records one latency, nanoseconds.
    pub(crate) fn record(&mut self, ns: u64) {
        self.record_in(Self::bucket(ns));
    }

    /// The bucket of `ns`, for a latency that goes into several histograms (one `ln`, not one each).
    pub(crate) fn bucket(ns: u64) -> usize {
        index(ns)
    }

    /// Records one latency whose bucket [`LatencyHist::bucket`] already found.
    pub(crate) fn record_in(&mut self, bucket: usize) {
        self.counts[bucket] = self.counts[bucket].saturating_add(1);
        self.total += 1;
    }

    /// Adds `other`'s counts to this one (the page does this with the ticks of its window).
    #[cfg(test)]
    pub(crate) fn merge(&mut self, other: &LatencyHist) {
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a = a.saturating_add(*b);
        }
        self.total += other.total;
    }

    /// Latencies recorded.
    #[cfg(test)]
    pub(crate) fn count(&self) -> u64 {
        self.total
    }

    #[cfg(test)]
    /// The latency below which a share `q` (0 to 1) of the recorded values fall, read from the
    /// buckets (within 1%); 0 when nothing was recorded.
    pub(crate) fn quantile(&self, q: f64) -> u64 {
        if self.total == 0 {
            return 0;
        }
        // the rank of the value: the smallest count such that at least q of them are at or below it
        let rank = ((q.clamp(0.0, 1.0) * self.total as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (i, &c) in self.counts.iter().enumerate() {
            seen += u64::from(c);
            if seen >= rank {
                return middle(i);
            }
        }
        middle(self.counts.len() - 1)
    }

    /// The occupied buckets as `(bucket, count)`, for the wire.
    pub(crate) fn sparse(&self) -> Vec<(u16, u32)> {
        self.counts
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, c)| (u16::try_from(i).unwrap_or(u16::MAX), *c))
            .collect()
    }

    /// A histogram from [`LatencyHist::sparse`] (buckets past the range are ignored).
    #[cfg(test)]
    pub(crate) fn from_sparse(s: &[(u16, u32)]) -> LatencyHist {
        let mut h = LatencyHist::new();
        for &(i, c) in s {
            if let Some(slot) = h.counts.get_mut(usize::from(i)) {
                *slot = c;
                h.total += u64::from(c);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64* (the tests need no `rand` stream of their own).
    struct Rng(u64);
    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
            ((v >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        }
        /// A lognormal draw with median `m` and log-sigma `s` (Box-Muller).
        fn lognormal(&mut self, m: f64, s: f64) -> f64 {
            let z =
                (-2.0 * self.unit().ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos();
            m * (s * z).exp()
        }
    }

    fn samples(n: usize) -> Vec<u64> {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        (0..n)
            .map(|_| r.lognormal(20_000_000.0, 1.0) as u64)
            .collect()
    }

    #[test]
    fn quantiles_match_the_exact_ones_within_1_percent() {
        let mut v = samples(100_000);
        let mut h = LatencyHist::new();
        for &x in &v {
            h.record(x);
        }
        v.sort_unstable();
        for q in [0.5, 0.9, 0.99, 0.999] {
            let exact = v[((q * v.len() as f64).ceil() as usize).max(1) - 1] as f64;
            let got = h.quantile(q) as f64;
            assert!(
                (got - exact).abs() / exact < 0.011,
                "q {q}: histogram {got} exact {exact}"
            );
        }
        assert_eq!(h.count(), 100_000);
    }

    #[test]
    fn merging_equals_recording_into_one() {
        let v = samples(20_000);
        let (mut a, mut b, mut whole) =
            (LatencyHist::new(), LatencyHist::new(), LatencyHist::new());
        for (i, &x) in v.iter().enumerate() {
            whole.record(x);
            if i % 3 == 0 { a.record(x) } else { b.record(x) }
        }
        a.merge(&b);
        assert_eq!(a, whole);
    }

    #[test]
    fn sparse_round_trips_and_holds_only_occupied_buckets() {
        let mut h = LatencyHist::new();
        for x in [5_000_000u64, 5_000_000, 90_000_000] {
            h.record(x);
        }
        let s = h.sparse();
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(LatencyHist::from_sparse(&s), h);
    }

    #[test]
    fn the_range_has_about_a_thousand_buckets_and_the_edges_clamp() {
        let n = bucket_count();
        assert!((900..1200).contains(&n), "{n} buckets");
        let mut h = LatencyHist::new();
        h.record(0);
        h.record(u64::MAX);
        assert_eq!(h.count(), 2);
        assert!(h.quantile(0.0) < 10_000);
        assert!(h.quantile(1.0) > 900_000_000_000);
        assert_eq!(LatencyHist::new().quantile(0.99), 0, "empty reads 0");
    }
}
