//! Retry arithmetic (RULES 82-84, step 7a): when a failed call is tried again and how long it waits.
//! Pure functions; the plan sampler and the engine call them (tasks 3 and 4).
//!
//! * **Backoff** is exponential, `min(cap, base * mult^k)` for the wait before retry `k + 1`, with
//!   the AWS SDK defaults (50 ms, x2, 20 s cap): AWS SDK "standard" retry mode.
//! * **Jitter** spreads simultaneous retriers: **full** `U(0, d)` is the default, **equal**
//!   `d/2 + U(0, d/2)`, or none (Brooker, "Exponential Backoff And Jitter", AWS Architecture Blog,
//!   2015: full jitter does the least total work under contention).
//! * **Retryable** is the transient set the real clients share: HTTP 500, 502, 503, 504 and 429,
//!   a timeout, a reset connection (AWS SDK standard mode, gRPC `retryableStatusCodes`). A 501 or
//!   any other 4xx is a verdict, not a blip, and a garbled response is not retried.

use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::chaos::FailKind;

/// Wait before retry number `attempt + 1` (`attempt` counts from 0): `min(cap, base * mult^attempt)`.
/// Saturating, so a large `attempt` or `mult` cannot overflow.
pub(super) fn backoff(base: Duration, mult: u32, cap: Duration, attempt: u32) -> Duration {
    base.saturating_mul(mult.saturating_pow(attempt)).min(cap)
}

/// How a wait is randomised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Jitter {
    /// The exact backoff.
    None,
    /// `U(0, d)`.
    #[default]
    Full,
    /// `d/2 + U(0, d/2)`.
    Equal,
}

/// `delay` randomised by `kind`, where `u` is a draw in `[0, 1)`.
pub(super) fn jitter(delay: Duration, kind: Jitter, u: f64) -> Duration {
    match kind {
        Jitter::None => delay,
        Jitter::Full => delay.mul_f64(u),
        Jitter::Equal => delay / 2 + (delay / 2).mul_f64(u),
    }
}

/// Is this failure transient, so a retry has a chance? `status` is the HTTP status (ignored unless
/// `kind` is [`FailKind::Status`]).
pub(crate) fn retryable(status: u16, kind: FailKind) -> bool {
    match kind {
        FailKind::Status => matches!(status, 500 | 502 | 503 | 504 | 429),
        FailKind::Timeout | FailKind::Reset => true,
        FailKind::Malformed => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn backoff_doubles_from_the_base_and_saturates_at_the_cap() {
        let got: Vec<_> = (0..4).map(|k| backoff(MS(50), 2, MS(20_000), k)).collect();
        assert_eq!(got, [MS(50), MS(100), MS(200), MS(400)]);
        // 50 ms * 2^9 = 25.6 s > 20 s cap.
        assert_eq!(backoff(MS(50), 2, MS(20_000), 9), MS(20_000));
        // Absurd inputs saturate instead of overflowing.
        assert_eq!(backoff(MS(50), 2, MS(20_000), 4_000), MS(20_000));
        assert_eq!(backoff(Duration::MAX, u32::MAX, MS(7), 3), MS(7));
        // mult 1 is a constant wait.
        assert_eq!(backoff(MS(50), 1, MS(20_000), 5), MS(50));
    }

    /// Mean of `jitter` over an even grid of `u` in `[0, 1)` (midpoints), in nanoseconds.
    fn mean(d: Duration, kind: Jitter) -> f64 {
        const N: u32 = 10_000;
        let sum: f64 = (0..N)
            .map(|i| jitter(d, kind, (f64::from(i) + 0.5) / f64::from(N)).as_secs_f64())
            .sum();
        sum / f64::from(N)
    }

    #[test]
    fn jitter_means_match_brookers_definitions() {
        let d = MS(800);
        let dm = d.as_secs_f64();
        // Full: U(0, d) has mean d/2. Equal: d/2 + U(0, d/2) has mean 3d/4. None: exactly d.
        assert!((mean(d, Jitter::Full) - dm / 2.0).abs() < dm * 1e-4);
        assert!((mean(d, Jitter::Equal) - dm * 0.75).abs() < dm * 1e-4);
        assert_eq!(jitter(d, Jitter::None, 0.37), d);
    }

    #[test]
    fn jitter_stays_inside_its_range() {
        let d = MS(800);
        for i in 0..1000 {
            let u = f64::from(i) / 1000.0;
            assert!(jitter(d, Jitter::Full, u) <= d);
            let e = jitter(d, Jitter::Equal, u);
            assert!(e >= d / 2 && e <= d);
        }
        assert_eq!(jitter(Duration::ZERO, Jitter::Full, 0.9), Duration::ZERO);
        assert_eq!(Jitter::default(), Jitter::Full);
    }

    #[test]
    fn the_retryable_set_is_the_transient_one() {
        for s in [500, 502, 503, 504, 429] {
            assert!(retryable(s, FailKind::Status), "{s} retries");
        }
        // 501 is not transient; the rest of 4xx and a 3xx/2xx are verdicts.
        for s in [501, 505, 400, 401, 403, 404, 408, 409, 422, 301, 200] {
            assert!(!retryable(s, FailKind::Status), "{s} does not retry");
        }
        // The status is ignored for non-status failures.
        assert!(retryable(0, FailKind::Timeout));
        assert!(retryable(404, FailKind::Reset));
        assert!(!retryable(503, FailKind::Malformed));
    }
}
