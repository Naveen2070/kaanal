//! **Private.** The per-node time series: one extra pass over the records that cuts the
//! reporting window into [`WINDOWS`] slices and reports, per node, the busy fraction, the longest
//! queue seen at a join and a histogram of its own hop durations. The records only hold requests
//! that arrived at or after warmup, so a run with a warmup under-counts early windows (their
//! pre-warmup requests are missing); with warmup 0 the sums match the engine's figures.

use kaanal_core::{ChainResult, HopKind, Topology};
use serde::Serialize;

use super::{BUCKETS, bucket_of};
use crate::{SimConfig, SimReport};

/// Windows over the reporting window, whatever its length.
pub(super) const WINDOWS: usize = 60;
/// Columns of one latency histogram: the bounded buckets plus the overflow bucket.
const COLS: usize = BUCKETS + 1;

/// One node over time: [`WINDOWS`] windows of `window_s` seconds from warmup.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Series {
    /// Width of one window in seconds (reporting window ÷ 60).
    pub window_s: f64,
    /// Busy fraction of the node's slots (all replicas) per window, at most 1; empty for a node
    /// without `concurrency` or `pool`.
    pub busy: Vec<f64>,
    /// Longest queue seen when a request joined it in the window (the engine's `max_queue`
    /// measured per window; a join after the run's end lands in the last window).
    pub queue: Vec<u32>,
    /// Per window, counts of the node's own hop durations in the same buckets as the summary's
    /// latency histogram (19 bounded plus the overflow bucket), by the window the hop started in
    /// (one that starts after the end of the run counts in the last).
    pub latency: Vec<Vec<u32>>,
}

/// One node's running figures.
struct Work {
    /// Slot-time (s) per window; empty without slots.
    busy: Vec<f64>,
    /// `WINDOWS × COLS` counts, flat.
    lat: Vec<u32>,
    /// Per replica: `(time ns, 0 = leaves the queue | 1 = joins)` for every hop that waited.
    queue: Vec<Vec<(u64, u8)>>,
}

/// **Private.** Series per node, by node id.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "window indices are clamped to WINDOWS; times and counts are far below 2^52"
)]
pub(super) fn series(topo: &Topology, cfg: &SimConfig, report: &SimReport) -> Vec<Series> {
    let warm = cfg.warmup.as_secs_f64();
    let w = cfg.duration.saturating_sub(cfg.warmup).as_secs_f64() / WINDOWS as f64;
    let win = |t: f64| (((t - warm) / w).max(0.0) as usize).min(WINDOWS - 1);
    let mut work: Vec<Work> = topo
        .nodes
        .iter()
        .map(|n| {
            let slotted = n.admission.is_some();
            Work {
                busy: vec![0.0; if slotted { WINDOWS } else { 0 }],
                lat: vec![0; WINDOWS * COLS],
                queue: vec![
                    Vec::new();
                    if slotted {
                        usize::from(n.replicas.max(1))
                    } else {
                        0
                    }
                ],
            }
        })
        .collect();
    for r in &report.requests {
        let arrival = r.arrival.as_secs_f64();
        for (hop, wait) in &r.hops {
            if !matches!(
                hop.kind,
                HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss
            ) {
                continue;
            }
            let wk = &mut work[hop.node];
            let (start, dur) = (arrival + hop.start.as_secs_f64(), hop.dur.as_secs_f64());
            wk.lat[win(start) * COLS + bucket_of(dur * 1000.0)] += 1;
            if wk.busy.is_empty() {
                continue;
            }
            let end = start + dur;
            let mut i = win(start);
            loop {
                let lo = warm + i as f64 * w;
                let hi = lo + w;
                wk.busy[i] += (end.min(hi) - start.max(lo)).max(0.0);
                i += 1;
                if end <= hi || i == WINDOWS {
                    break;
                }
            }
            if !wait.is_zero() {
                // the queue it joined: from the join instant to the instant it got its slot
                let at = r.arrival + hop.start;
                let ns = |d: std::time::Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
                let q = &mut wk.queue[usize::from(hop.replica)];
                q.push((ns(at.saturating_sub(*wait)), 1));
                q.push((ns(at), 0));
            }
        }
    }
    work.into_iter()
        .zip(&topo.nodes)
        .map(|(mut wk, n)| {
            let slots = n.admission.as_ref().map_or(0.0, |a| f64::from(a.slots))
                * f64::from(n.replicas.max(1));
            let mut queue = vec![0u32; WINDOWS];
            for q in &mut wk.queue {
                // a leave sorts before a join at the same instant: the head goes first
                q.sort_unstable();
                let mut len = 0u32;
                for &(t, join) in q.iter() {
                    if join == 0 {
                        len -= 1;
                    } else {
                        len += 1;
                        let i = win(t as f64 / 1e9);
                        queue[i] = queue[i].max(len);
                    }
                }
            }
            Series {
                window_s: w,
                busy: wk
                    .busy
                    .iter()
                    .map(|b| (b / (w * slots)).clamp(0.0, 1.0))
                    .collect(),
                queue,
                latency: wk.lat.chunks(COLS).map(<[u32]>::to_vec).collect(),
            }
        })
        .collect()
}

/// The whole system over time, in the same [`WINDOWS`] windows as [`Series`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemSeries {
    /// Width of one window in seconds (reporting window ÷ 60).
    pub window_s: f64,
    /// Requests that finished OK, by the window they finished in.
    pub done: Vec<u32>,
    /// Requests that finished failed, by the window they finished in.
    pub failed: Vec<u32>,
    /// End-to-end p99 in ms of the requests finished in the window, estimated inside its latency
    /// bucket; 0 for a window with none.
    pub p99_ms: Vec<f64>,
}

/// Estimate in ms of the nearest-rank percentile `p` of `counts` (the summary's buckets, overflow
/// last); `None` when `counts` has no samples. The samples are taken as spread evenly in log
/// scale across the bucket `(lo, hi]` that holds the rank, so the estimate sits between the
/// bucket's edges instead of on its upper edge (which can be twice the truth). The first bucket
/// starts at 0, so it is spread evenly in linear scale. The overflow bucket has no upper edge, so
/// it reports its lower edge: a floor, which keeps the figure finite.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the rank is clamped to the total; counts are far below 2^52"
)]
pub(super) fn percentile_est(counts: &[u32], p: f64) -> Option<f64> {
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total == 0 {
        return None;
    }
    let rank = ((p * total as f64).ceil() as u64).clamp(1, total);
    let mut before = 0u64;
    for (i, &c) in counts.iter().enumerate() {
        let c = u64::from(c);
        if before + c < rank {
            before += c;
            continue;
        }
        let hi = 0.25 * (1u64 << i) as f64;
        if i == COLS - 1 {
            return Some(hi / 2.0); // overflow: the lower edge, a floor
        }
        let frac = (rank - before) as f64 / c as f64;
        return Some(if i == 0 {
            hi * frac
        } else {
            let lo = hi / 2.0;
            lo * (hi / lo).powf(frac) // log-linear between the edges
        });
    }
    None
}

/// **Private.** The system series: requests finished per window, split by outcome, and each
/// window's end-to-end p99. Same windows and same records as [`series`], so the same warmup
/// caveat holds; a request finishing after the run's end counts in the last window.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "window indices are clamped to WINDOWS; counts are far below 2^52"
)]
pub(super) fn system_series(cfg: &SimConfig, report: &SimReport) -> SystemSeries {
    let warm = cfg.warmup.as_secs_f64();
    let w = cfg.duration.saturating_sub(cfg.warmup).as_secs_f64() / WINDOWS as f64;
    let mut done = vec![0u32; WINDOWS];
    let mut failed = vec![0u32; WINDOWS];
    let mut lat = vec![0u32; WINDOWS * COLS];
    for r in &report.requests {
        let end = (r.arrival + r.latency).as_secs_f64();
        // w is 0 for an empty window: the division gives NaN or inf, which the cast sends to 0
        // or the clamp to the last window
        let i = (((end - warm) / w).max(0.0) as usize).min(WINDOWS - 1);
        if matches!(r.result, ChainResult::Err { .. }) {
            failed[i] += 1;
        } else {
            done[i] += 1;
        }
        lat[i * COLS + bucket_of(r.latency.as_secs_f64() * 1000.0)] += 1;
    }
    SystemSeries {
        window_s: w,
        done,
        failed,
        p99_ms: lat
            .chunks(COLS)
            .map(|c| percentile_est(c, 0.99).unwrap_or(0.0))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{COLS, percentile_est};

    fn counts(at: usize, n: u32) -> Vec<u32> {
        let mut c = vec![0; COLS];
        c[at] = n;
        c
    }

    #[test]
    fn estimate_sits_inside_the_bucket_not_on_its_upper_edge() {
        // 100 samples in (1, 2] ms: the p99 is near the top of the bucket, but below it
        let p99 = percentile_est(&counts(3, 100), 0.99).unwrap();
        assert!(p99 > 1.9 && p99 < 2.0, "{p99}");
        // and the median is nearer the lower edge: log-linear, 1 * 2^0.5
        let p50 = percentile_est(&counts(3, 100), 0.50).unwrap();
        assert!((p50 - 2f64.sqrt()).abs() < 0.02, "{p50}");
    }

    #[test]
    fn first_bucket_is_linear_and_overflow_is_a_floor_and_empty_is_none() {
        assert!((percentile_est(&counts(0, 10), 0.5).unwrap() - 0.125).abs() < 1e-9);
        assert_eq!(percentile_est(&counts(COLS - 1, 5), 0.99), Some(65536.0));
        assert_eq!(percentile_est(&[0; COLS], 0.99), None);
    }
}
