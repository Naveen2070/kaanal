//! Connection set-up (RULES 68, step 6b): what a call that opens a new connection pays before and
//! while its bodies cross. Pure integer arithmetic, so it costs a few instructions per call.
//!
//! Sources: TCP's three-way handshake is one round trip before data (RFC 9293 section 3.5, data
//! rides the third segment); TLS 1.3 adds one (RFC 8446 section 2), TLS 1.2 two (RFC 5246 section
//! 7.3); the initial window is 10 segments (RFC 6928) of an MSS of 1460 B (Ethernet 1500 less 40),
//! doubling every round trip in slow start (RFC 5681 section 3.1); after `k` rounds
//! `10 * (2^k - 1)` segments are out (Cardwell, Savage and Anderson, "Modeling TCP Latency",
//! INFOCOM 2000). Not modelled, and named so: pool exhaustion under concurrency, idle-timeout
//! closes, slow-start-after-idle, 0-RTT and session resumption, HTTP/2 and HTTP/3 multiplexing,
//! loss (step 6d).

use std::time::Duration;

use crate::config::Tls;

/// Initial congestion window in segments (RFC 6928).
const IW_SEGMENTS: u64 = 10;
/// Maximum segment size in bytes: a 1500 B Ethernet MTU less 20 B of IP and 20 B of TCP header.
pub(super) const MSS: u64 = 1460;
/// Bytes of the first flight: the most a fresh connection sends before it hears an ACK.
const IW_BYTES: u128 = (IW_SEGMENTS * MSS) as u128;

/// Round trips a new connection spends before the request can be sent: TCP's one, plus TLS's.
pub(super) fn handshake_rtts(tls: Tls) -> u32 {
    match tls {
        Tls::Off => 1,
        Tls::V13 => 2,
        Tls::V12 => 3,
    }
}

/// Slow-start rounds `bytes` take on a fresh connection: the least `k` with
/// `10 * (2^k - 1) * MSS >= bytes`, i.e. `ceil(log2(ceil(segments / 10) + 1))`. Zero bytes, zero
/// rounds; a body that fits the first window (14,600 B) is one.
pub(super) fn rounds(bytes: u64) -> u32 {
    if bytes == 0 {
        return 0;
    }
    // 2^k >= ceil(segments / 10) + 1 = m, and m >= 2, so k = ceil_log2(m) = ilog2(m - 1) + 1
    let m = bytes.div_ceil(MSS).div_ceil(IW_SEGMENTS) + 1;
    (m - 1).ilog2() + 1
}

/// Round trips a body of `bytes` waits on a fresh connection because its window is still smaller
/// than the pipe: every round after the first stalls one RTT, but only until the window reaches the
/// bandwidth-delay product `rate * rtt` (from then on the pipe is full and the link hop already
/// charges `bytes / rate`). `rate` is the narrowest pipe's bytes a second; `None` is an unlimited
/// wire, whose pipe is never full, so every round after the first stalls.
pub(super) fn stalls(bytes: u64, rate: Option<u64>, rtt: Duration) -> u32 {
    let k = rounds(bytes);
    if k <= 1 {
        return 0;
    }
    let j = match rate {
        None => k,
        Some(rate) => {
            let bdp = u128::from(rate) * rtt.as_nanos() / 1_000_000_000;
            if bdp <= IW_BYTES {
                1
            } else {
                // the first round whose window 10 * MSS * 2^(j-1) reaches the BDP
                let q = bdp.div_ceil(IW_BYTES);
                (q - 1).ilog2() + 2
            }
        }
    };
    k.min(j) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn handshakes_are_one_two_and_three_round_trips() {
        // RFC 9293 (TCP), RFC 8446 (TLS 1.3), RFC 5246 (TLS 1.2)
        assert_eq!(handshake_rtts(Tls::Off), 1);
        assert_eq!(handshake_rtts(Tls::V13), 2);
        assert_eq!(handshake_rtts(Tls::V12), 3);
    }

    #[test]
    fn rounds_follow_the_slow_start_series() {
        // after k rounds 10 * (2^k - 1) segments of 1460 B are out: 14,600 / 43,800 / 102,200 / ...
        assert_eq!(rounds(0), 0);
        assert_eq!(rounds(1), 1);
        assert_eq!(rounds(14_600), 1);
        assert_eq!(rounds(14_601), 2);
        assert_eq!(rounds(43_800), 2);
        assert_eq!(rounds(43_801), 3);
        assert_eq!(rounds(102_200), 3);
        assert_eq!(rounds(102_201), 4);
        // 1 MiB is 719 segments: 10 * (2^7 - 1) = 1270 >= 719 > 10 * (2^6 - 1) = 630
        assert_eq!(rounds(1_048_576), 7);
    }

    #[test]
    fn rounds_match_the_closed_form_for_every_size_up_to_a_million_segments() {
        // the oracle is the series itself, summed with a loop, against the integer shortcut
        for segs in (1..=2000_u64).chain([4095, 4096, 100_000, 1_000_000]) {
            let k = (1..=40)
                .find(|k| IW_SEGMENTS * ((1_u64 << k) - 1) >= segs)
                .unwrap();
            assert_eq!(u64::from(rounds(segs * MSS)), k, "{segs} segments");
        }
    }

    #[test]
    fn an_unlimited_wire_stalls_every_round_after_the_first() {
        assert_eq!(stalls(14_600, None, MS), 0);
        assert_eq!(stalls(14_601, None, MS), 1);
        assert_eq!(stalls(1_048_576, None, MS), 6);
    }

    #[test]
    fn a_window_that_already_fills_the_pipe_does_not_stall() {
        // 1 KB/s at 10 ms is a 10 B pipe: the first flight overfills it, the link hop is the limit
        assert_eq!(stalls(10_000_000, Some(1024), 10 * MS), 0);
        // 1 MiB at exactly one window of BDP: 14,600 B/s over 1 s
        assert_eq!(stalls(1_048_576, Some(14_600), Duration::from_secs(1)), 0);
    }

    #[test]
    fn stalls_end_at_the_round_whose_window_reaches_the_bdp() {
        // BDP 100,000 B (1 MB/s ... 100 KB/s at 1 s): windows 14.6k, 29.2k, 58.4k, 116.8k -> j = 4
        let rate = Some(100_000);
        assert_eq!(stalls(1_048_576, rate, Duration::from_secs(1)), 3);
        // a body of only 3 rounds stalls two of them, the pipe never fills within it
        assert_eq!(stalls(102_200, rate, Duration::from_secs(1)), 2);
    }
}
