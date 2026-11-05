//! Packet loss on a link (RULES 74, step 6d): what a lossy transfer costs beyond its lossless time.
//! Closed-form and integer-light; every formula names its source and has a theory test.
//!
//! A body of `n` segments (MSS 1460 B) loses `L ~ Binomial(n, p)` of them. A loss with three or more
//! segments after it triggers **fast retransmit** (RFC 5681 section 3.2): about one round trip. A
//! loss with fewer (the tail of the body, or a body of four segments or fewer) has no duplicate
//! ACKs and waits for the **retransmission timer** (RFC 6298): `RTO = max(200 ms, 3 x RTT)`, the
//! Linux floor and RFC 6298's first-sample `SRTT + 4 x RTTVAR = 3 R`; a lost SYN waits the initial
//! RTO of 1 s (RFC 6298 section 2.1). The chance a loss is a timeout is `min(1, 3 / w)` for a flight
//! of `w` segments (Padhye, Firoiu, Towsley and Kurose, "Modeling TCP Throughput", SIGCOMM 1998).
//! A flow with more than 32 expected losses is long: its rate is the steady-state **TFRC** rate of
//! RFC 5348 section 3.1 (the PFTK equation) instead of discrete events. Not modelled: a
//! retransmission lost again (`p^2`), window halving after a recovered loss inside a short flow,
//! SACK details, ECN, reordering.

use std::time::Duration;

/// Linux's minimum retransmission timeout (`TCP_RTO_MIN`).
const RTO_MIN: Duration = Duration::from_millis(200);
/// The initial retransmission timeout, for a lost SYN (RFC 6298 section 2.1).
pub(super) const SYN_RTO: Duration = Duration::from_secs(1);
/// Segment size the rate formula uses (see `conn.rs`).
const MSS_F: f64 = 1460.0;
/// Most loss events one transfer draws; beyond it the flow is long and its rate formula governs.
const MAX_EVENTS: u32 = 64;
/// Expected losses above which a flow is long (its rate, not discrete events, is the model).
const LONG_FLOW_LOSSES: f64 = 32.0;

/// A segment count as an `f64` (exact below 2^32 segments, far past any body).
fn f(n: u64) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The retransmission timeout for a path with round trip `rtt`: `max(200 ms, 3 x RTT)`.
pub(super) fn rto(rtt: Duration) -> Duration {
    RTO_MIN.max(rtt.saturating_mul(3))
}

/// Segments lost out of `n` when each is lost with probability `p`: the `Binomial(n, p)` quantile
/// at `u` in `[0, 1)`, by inversion with the exact recurrence
/// `P(k+1) = P(k) (n - k) / (k + 1) * p / (1 - p)` from `P(0) = (1 - p)^n`, at most
/// [`MAX_EVENTS`]. A few iterations while `n p` is small, which is the regime the discrete model
/// serves (see [`long_flow`]).
pub(super) fn losses(n: u64, p: f64, u: f64) -> u32 {
    if n == 0 || p <= 0.0 {
        return 0;
    }
    if p >= 1.0 {
        return u32::try_from(n.min(u64::from(MAX_EVENTS))).unwrap_or(MAX_EVENTS);
    }
    let odds = p / (1.0 - p);
    let mut pmf = (f(n) * (1.0 - p).ln()).exp();
    let mut cum = pmf;
    let mut k: u32 = 0;
    while u >= cum && u64::from(k) < n && k < MAX_EVENTS {
        pmf *= (f(n) - f64::from(k)) / (f64::from(k) + 1.0) * odds;
        cum += pmf;
        k += 1;
    }
    k
}

/// The chance a loss within a flight of `window` segments ends in a timeout rather than a fast
/// retransmit: `min(1, 3 / w)` (Padhye et al. 1998), one for a window below three.
pub(super) fn timeout_chance(window: u64) -> f64 {
    (3.0 / f(window.max(1))).min(1.0)
}

/// Whether a body of `n` segments is a long flow at loss `p`: more than 32 expected losses.
pub(super) fn long_flow(n: u64, p: f64) -> bool {
    f(n) * p > LONG_FLOW_LOSSES
}

/// The steady-state rate of one TCP flow in bytes a second, RFC 5348 section 3.1 (TFRC, the PFTK
/// equation) with `b = 1`, `s = MSS` and `t_RTO = max(200 ms, 4 R)`:
/// `X = s / (R sqrt(2p/3) + t_RTO (3 sqrt(3p/8)) p (1 + 32 p^2))`. At least 1.
pub(super) fn tfrc_rate(rtt: Duration, p: f64) -> u64 {
    let r = rtt.as_secs_f64();
    let t_rto = 0.2_f64.max(4.0 * r);
    let denom = r * (2.0 * p / 3.0).sqrt()
        + t_rto * 3.0 * (3.0 * p / 8.0).sqrt() * p * (1.0 + 32.0 * p * p);
    if denom <= 0.0 {
        return u64::MAX;
    }
    // a rate in bytes a second never exceeds 2^63
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped"
    )]
    let x = (MSS_F / denom).clamp(1.0, 9.0e18) as u64;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeout_is_three_round_trips_with_a_200_ms_floor() {
        assert_eq!(rto(Duration::from_millis(10)), Duration::from_millis(200));
        assert_eq!(rto(Duration::from_millis(100)), Duration::from_millis(300));
        assert_eq!(rto(Duration::ZERO), Duration::from_millis(200));
    }

    #[test]
    fn losses_invert_the_exact_binomial_cdf() {
        // n = 10, p = 0.1: P(0) = 0.34868, P(1) = 0.38742, P(2) = 0.19371, cumulative 0.34868,
        // 0.73610, 0.92981 (the Binomial table)
        for (u, want) in [
            (0.0, 0),
            (0.34, 0),
            (0.35, 1),
            (0.73, 1),
            (0.74, 2),
            (0.929, 2),
            (0.931, 3),
        ] {
            assert_eq!(losses(10, 0.1, u), want, "u = {u}");
        }
    }

    #[test]
    fn losses_have_the_binomial_mean_and_variance() {
        // the quantile function over an even grid of u integrates to the mean n p; the mean of
        // the squares gives the variance n p (1 - p)
        for (n, p) in [(10_u64, 0.1), (719, 0.01), (50, 0.2)] {
            let grid = 100_000_u32;
            let (mut sum, mut sq) = (0.0, 0.0);
            for i in 0..grid {
                let k = f64::from(losses(n, p, (f64::from(i) + 0.5) / f64::from(grid)));
                sum += k;
                sq += k * k;
            }
            let (mean, var) = (
                sum / f64::from(grid),
                sq / f64::from(grid) - (sum / f64::from(grid)).powi(2),
            );
            let (want_mean, want_var) = (f(n) * p, f(n) * p * (1.0 - p));
            assert!(
                (mean - want_mean).abs() / want_mean < 0.01,
                "n={n} p={p}: mean {mean} vs {want_mean}"
            );
            assert!(
                (var - want_var).abs() / want_var < 0.03,
                "n={n} p={p}: var {var} vs {want_var}"
            );
        }
    }

    #[test]
    fn certain_and_impossible_loss_are_exact() {
        assert_eq!(losses(10, 0.0, 0.5), 0);
        assert_eq!(losses(10, 1.0, 0.5), 10);
        assert_eq!(losses(1000, 1.0, 0.5), 64, "capped");
        assert_eq!(losses(0, 0.5, 0.9), 0);
    }

    #[test]
    fn a_loss_in_a_small_flight_is_a_timeout_and_in_a_big_one_rarely() {
        assert_eq!(timeout_chance(1), 1.0);
        assert_eq!(timeout_chance(2), 1.0);
        assert_eq!(timeout_chance(3), 1.0);
        assert!((timeout_chance(10) - 0.3).abs() < 1e-12);
        assert!((timeout_chance(300) - 0.01).abs() < 1e-12);
    }

    #[test]
    fn the_long_flow_boundary_is_32_expected_losses() {
        assert!(!long_flow(3200, 0.01));
        assert!(long_flow(3201, 0.01));
        assert!(!long_flow(2000, 0.01));
        assert!(long_flow(4000, 0.01));
    }

    #[test]
    fn the_tfrc_rate_is_the_rfc_5348_equation() {
        // computed independently, with powf rather than sqrt, for R = 100 ms, p = 1 %:
        // X = s / (R sqrt(2p/3) + t_RTO (3 sqrt(3p/8)) p (1 + 32 p^2)), t_RTO = max(0.2, 4R) = 0.4
        let (s, r, p, t) = (1460.0_f64, 0.1_f64, 0.01_f64, 0.4_f64);
        let want = s
            / (r * (2.0 * p / 3.0).powf(0.5)
                + t * 3.0 * (3.0 * p / 8.0).powf(0.5) * p * (1.0 + 32.0 * p.powi(2)));
        let got = f(tfrc_rate(Duration::from_millis(100), 0.01));
        assert!((got - want).abs() / want < 1e-3, "{got} vs {want}");
        // about 164 kB/s: the familiar order of magnitude for 1 % loss at 100 ms
        assert!((160_000.0..168_000.0).contains(&got), "{got}");
    }

    #[test]
    fn the_tfrc_rate_falls_with_loss_and_with_round_trip_time() {
        let at = |ms, p| tfrc_rate(Duration::from_millis(ms), p);
        assert!(at(100, 0.001) > at(100, 0.01));
        assert!(at(100, 0.01) > at(100, 0.1));
        assert!(at(10, 0.01) > at(100, 0.01));
        assert!(at(100, 0.01) > at(300, 0.01));
    }
}
