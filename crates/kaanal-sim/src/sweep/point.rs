//! One point of the sweep: the run at a load factor `k`, folded into a small [`Point`].

use std::collections::BTreeMap;
use std::time::Duration;

use kaanal_core::{EffectiveChaos, Endpoints, Topology};

use super::{NodePoint, Over, Point, PointFail};
use crate::{Limits, SimConfig, Stream, run_with_limits, summarize};

/// Seconds past the engine's 365-day cap: a rate that is zero, negative or NaN gets a window this
/// long, so the engine's own check names it instead of `Duration` panicking here.
const TOO_LONG_S: f64 = 4e7;

/// The shortest window a point may have: ten times the largest mean `proc` of any node, so a queue
/// near saturation has time to build. A node without `proc` has a zero `Fixed` one.
pub(crate) fn floor(topo: &Topology) -> Duration {
    topo.nodes
        .iter()
        .map(|n| n.proc.mean())
        .max()
        .unwrap_or_default()
        * 10
}

/// `base` at `k` times every stream's rate, sized for about `target_requests` reported requests:
/// window = target / (total rate x k), at least `floor` (a longer window only adds requests; the
/// caps still apply); warmup = window / 9, so it is a tenth of the run.
#[expect(
    clippy::cast_precision_loss,
    reason = "request targets are far below 2^52"
)]
pub(crate) fn point_cfg(
    base: &SimConfig,
    k: f64,
    target_requests: u64,
    floor: Duration,
) -> SimConfig {
    let streams: Vec<Stream> = base
        .streams
        .iter()
        .map(|s| Stream {
            rate_per_s: s.rate_per_s * k,
            ..s.clone()
        })
        .collect();
    let rate: f64 = streams.iter().map(|s| s.rate_per_s).sum();
    // `min` maps NaN to the cap; a negative value fails the conversion
    let secs = (target_requests as f64 / rate).min(TOO_LONG_S);
    let window = Duration::try_from_secs_f64(secs)
        .unwrap_or(Duration::from_secs_f64(TOO_LONG_S))
        .max(floor);
    let warmup = window / 9;
    SimConfig {
        streams,
        duration: warmup + window,
        warmup,
        seed: base.seed,
    }
}

/// Runs `cfg` (built by [`point_cfg`] at `k`) and keeps the figures a sweep needs; the report is
/// dropped. A refusal by the hop caps is `TooLarge`, any other error `Other`.
#[expect(
    clippy::cast_precision_loss,
    reason = "request counts are far below 2^52"
)]
pub(crate) fn run_point(
    topo: &Topology,
    endpoints: &Endpoints,
    chaos: &EffectiveChaos,
    cfg: &SimConfig,
    k: f64,
    limits: &Limits,
) -> Result<Point, PointFail> {
    let report = run_with_limits(topo, endpoints, chaos, cfg, limits).map_err(|e| {
        // the two cap messages of `validate`: "workload too large", "simulated work too large"
        if e.contains("too large") {
            PointFail::TooLarge(e)
        } else {
            PointFail::Other(e)
        }
    })?;
    let s = summarize(topo, endpoints, cfg, &report);
    let mut causes: BTreeMap<&'static str, u64> = BTreeMap::new();
    for c in report.requests.iter().filter_map(|r| r.cause) {
        *causes.entry(c.as_str()).or_default() += 1;
    }
    drop(report);
    // the most common; on a tie the name first in alphabetical order
    let cause = causes
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(c, _)| c);
    let error_rate = if s.requests == 0 {
        0.0
    } else {
        s.failed as f64 / s.requests as f64
    };
    Ok(Point {
        k,
        req_s: cfg.streams.iter().map(|s| s.rate_per_s).sum(),
        requests: s.requests,
        goodput_req_s: (s.requests - s.failed) as f64 / s.window_s,
        error_rate,
        p50_ms: s.latency.p50_ms,
        p95_ms: s.latency.p95_ms,
        p99_ms: s.latency.p99_ms,
        cause,
        nodes: s
            .nodes
            .into_iter()
            .map(|n| NodePoint {
                name: n.name,
                utilization: n.utilization,
                saturated: n.saturated,
                saturates_at_req_s: n.saturates_at_req_s,
            })
            .collect(),
        over: Over::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(key: &str, rate: f64) -> Stream {
        Stream {
            endpoint_key: key.into(),
            rate_per_s: rate,
            req_bytes: 100,
            resp_bytes: 2000,
        }
    }

    #[test]
    fn scaling_multiplies_every_stream_by_k() {
        let base = SimConfig {
            streams: vec![stream("GET /a", 10.0), stream("GET /b", 30.0)],
            duration: Duration::from_secs(1),
            warmup: Duration::ZERO,
            seed: 42,
        };
        let c = point_cfg(&base, 2.0, 20_000, Duration::ZERO);
        let rates: Vec<f64> = c.streams.iter().map(|s| s.rate_per_s).collect();
        assert_eq!(rates, [20.0, 60.0]);
        for (a, b) in c.streams.iter().zip(&base.streams) {
            assert_eq!(
                (&a.endpoint_key, a.req_bytes, a.resp_bytes),
                (&b.endpoint_key, b.req_bytes, b.resp_bytes)
            );
        }
        assert_eq!(c.seed, 42);
        let reported = c.duration.saturating_sub(c.warmup).as_secs_f64() * 80.0;
        assert!((reported - 20_000.0).abs() / 20_000.0 < 0.01, "{reported}");

        // a window under the floor is lengthened to it
        let floor = Duration::from_secs(1000);
        let c = point_cfg(&base, 2.0, 20_000, floor);
        assert_eq!(c.duration.saturating_sub(c.warmup), floor);
        assert_eq!(c.warmup, floor / 9);
    }

    #[test]
    fn a_degenerate_rate_gives_a_long_window_not_a_panic() {
        let base = SimConfig {
            streams: vec![stream("GET /a", 0.0)],
            duration: Duration::from_secs(1),
            warmup: Duration::ZERO,
            seed: 1,
        };
        for k in [1.0, -1.0, f64::NAN] {
            let c = point_cfg(&base, k, 20_000, Duration::ZERO);
            assert!(c.duration > Duration::from_hours(365 * 24), "{k}");
        }
    }
}
