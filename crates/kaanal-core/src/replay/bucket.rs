//! A token bucket as the generic cell rate algorithm (GCRA) in integer nanoseconds.

use crate::topology::RateLimit;

/// A token bucket that starts full. One `u128` of state (`tat`, the theoretical arrival time
/// of the next cell); `u128` cannot overflow for `now < 2^64`, `interval < 2^64` and
/// `burst < 2^32`, so [`Bucket::take`] needs no checked arithmetic.
#[derive(Debug, Clone)]
pub struct Bucket {
    interval: u64,
    burst: u32,
    tat: u128,
}

impl Bucket {
    /// A full bucket for `limit` (interval and burst at least 1, as `Topology::build` makes them).
    pub fn new(limit: &RateLimit) -> Bucket {
        debug_assert!(limit.interval_ns >= 1 && limit.burst >= 1);
        Bucket {
            interval: limit.interval_ns,
            burst: limit.burst,
            tat: 0,
        }
    }

    /// Back to full, as if never used.
    pub fn refill(&mut self) {
        self.tat = 0;
    }

    /// Takes a token at `now` (ns). `Err(wait)` says how many nanoseconds until one is
    /// available; a refusal spends nothing.
    ///
    /// # Errors
    /// `Err(ns)` when the bucket is empty.
    pub fn take(&mut self, now: u64) -> Result<(), u64> {
        let now = u128::from(now);
        let window = u128::from(self.burst - 1) * u128::from(self.interval);
        if self.tat > now + window {
            return Err(u64::try_from(self.tat - now - window).unwrap_or(u64::MAX));
        }
        self.tat = self.tat.max(now) + u128::from(self.interval);
        Ok(())
    }
}
