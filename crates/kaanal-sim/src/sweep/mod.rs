//! Load sweep (Step 9a): find the load factor at which a topology first breaks its limit.
//!
//! [`sweep`] runs the topology at a ladder of load factors (`point`), finds the first crossing of
//! a [`Limit`] (`search`, pure) and reports the knee, the first node to saturate and the main cause.

pub(crate) mod point;
pub mod search;

use std::cmp::Ordering;
use std::time::Duration;

use kaanal_core::{EffectiveChaos, Endpoints, Topology};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::{Limits, MAX_RATE_PER_S, SimConfig};
use point::{floor, point_cfg, run_point};
use search::{Ladder, search};

/// Fewer reported requests than this make a point's p99 noise (the summary's own floor).
const TAIL_SAMPLES: u64 = 1000;

/// How to sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The pass line.
    pub limit: Limit,
    /// Highest load factor on the ladder (default 8).
    pub max_factor: f64,
    /// Reported requests each point aims for (default 20 000, at least 1000).
    pub target_requests: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            limit: Limit::default(),
            max_factor: 8.0,
            target_requests: 20_000,
        }
    }
}

/// The node saturated at the lowest load factor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FirstSaturated {
    /// Node name.
    pub node: String,
    /// Lowest load factor at which it was saturated.
    pub k: f64,
    /// Offered requests per second at that factor.
    pub req_s: f64,
    /// Its analytic `saturates_at_req_s` at the lowest point, when known.
    pub analytic_req_s: Option<f64>,
}

/// What a sweep found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SweepResult {
    /// Always the summary's label (R6.5).
    pub label: &'static str,
    /// The pass line used.
    pub limit: Limit,
    /// Every point run, sorted by `k`.
    pub points: Vec<Point>,
    /// The finding.
    pub verdict: Verdict,
    /// First ladder `k` above the first crossing that passes with margin.
    pub recovers_at: Option<f64>,
    /// `k` of the point refused as too large, when the ladder was cut.
    pub stopped_at: Option<f64>,
    /// The node saturated at the lowest `k` (a report, not a cause).
    pub first_to_saturate: Option<FirstSaturated>,
    /// Main failure cause at the lowest point over the limit.
    pub dominant_cause: Option<&'static str>,
    /// Things a reader should know.
    pub warnings: Vec<String>,
}

/// Sweeps `base` from 0.25x to `opts.max_factor` times its stream rates and finds the knee.
///
/// `base.streams` carry the rates and sizes, `base.seed` is the seed at every point (common random
/// numbers); `base.duration` and `base.warmup` are ignored, as each point sizes its own window for
/// `opts.target_requests`. `on_point` is called after each point, in run order.
///
/// # Errors
/// A `max_factor` that is not finite or is under 0.25, a `target_requests` under 1000, a lowest
/// point too large for `limits` (naming it), or any other run error.
pub fn sweep(
    topo: &Topology,
    endpoints: &Endpoints,
    chaos: &EffectiveChaos,
    base: &SimConfig,
    opts: &Options,
    limits: &Limits,
    on_point: &mut dyn FnMut(&Point),
) -> Result<SweepResult, String> {
    let mut ladder = Ladder {
        top: opts.max_factor,
        ..Ladder::default()
    };
    if !opts.max_factor.is_finite() || opts.max_factor < ladder.lowest {
        return Err(format!(
            "max_factor must be a number of at least {} (got {})",
            ladder.lowest, opts.max_factor
        ));
    }
    if opts.target_requests < TAIL_SAMPLES {
        return Err(format!(
            "target_requests must be at least {TAIL_SAMPLES} for a p99 (got {})",
            opts.target_requests
        ));
    }
    // The engine refuses a stream over MAX_RATE_PER_S before its hop caps. A point keeps about
    // `target_requests`, so it seldom nears the hop caps, but a slow node's window floor can make
    // one long and then they refuse first (`stopped_at`). End the ladder below the rate cap.
    let fastest = base
        .streams
        .iter()
        .map(|s| s.rate_per_s)
        .fold(0.0, f64::max);
    let cap_k = MAX_RATE_PER_S / fastest; // infinite with no positive rate: the engine names that
    if cap_k < ladder.lowest {
        return Err(format!(
            "the lowest point k = {} is too large to run: a stream at {fastest} req/s x {} passes the engine's cap of {MAX_RATE_PER_S} req/s per stream",
            ladder.lowest, ladder.lowest
        ));
    }
    let cut = ladder.ks().into_iter().find(|&k| k > cap_k);
    if cut.is_some() {
        // a millionth under the cap, so float rounding in `ks` and `point_cfg` stays inside it
        ladder.top = cap_k * (1.0 - 1e-6);
    }
    let floor = floor(topo);
    let s = search(&ladder, &opts.limit, &mut |k| {
        let cfg = point_cfg(base, k, opts.target_requests, floor);
        let mut p = run_point(topo, endpoints, chaos, &cfg, k, limits)?;
        p.over = opts.limit.over(&p);
        on_point(&p);
        Ok(p)
    })?;

    let lim = &opts.limit;
    let crossing = s.points.iter().find(|p| !lim.passes(p));
    let first_to_saturate = s.points.iter().find_map(|p| {
        let n = p.nodes.iter().find(|n| n.saturated)?;
        Some(FirstSaturated {
            node: n.name.clone(),
            k: p.k,
            req_s: p.req_s,
            analytic_req_s: s.points[0]
                .nodes
                .iter()
                .find(|m| m.name == n.name)
                .and_then(|m| m.saturates_at_req_s),
        })
    });
    let dominant_cause = crossing.and_then(|p| p.cause);

    let mut warnings = Vec::new();
    if let Some(c) = crossing {
        let saturated_by_then = first_to_saturate.as_ref().is_some_and(|f| f.k <= c.k);
        if !saturated_by_then {
            let by = if c.over.errors {
                c.cause.unwrap_or("errors")
            } else {
                "p99 latency"
            };
            warnings.push(format!(
                "no node saturated before the knee: the limit was crossed by {by}"
            ));
        }
    }
    if let Some(k) = s.recovers_at {
        warnings.push(format!(
            "the curve recovers: it passes again at {k:.3}x, above the knee (a retry storm or a breaker can do this)"
        ));
    }
    // a hop-cap refusal inside the cut ladder comes first, as in `stopped_at` below
    if let Some(k) = s.stopped_at.or(cut) {
        let req_s: f64 = base.streams.iter().map(|s| s.rate_per_s * k).sum();
        warnings.push(if s.stopped_at.is_some() {
            format!("ladder stopped at {k:.3}x ({req_s:.0} req/s): run too large")
        } else {
            format!("ladder cut at {k:.3}x ({req_s:.0} req/s): a stream would pass the engine's rate cap of {MAX_RATE_PER_S} req/s per stream")
        });
    }
    for p in s.points.iter().filter(|p| p.requests < TAIL_SAMPLES) {
        warnings.push(format!(
            "at {:.3}x the p99 rests on {} requests",
            p.k, p.requests
        ));
    }

    Ok(SweepResult {
        label: crate::summary::LABEL,
        limit: opts.limit.clone(),
        points: s.points,
        verdict: s.verdict,
        recovers_at: s.recovers_at,
        // a hop-cap refusal inside the cut ladder comes first
        stopped_at: s.stopped_at.or(cut),
        first_to_saturate,
        dominant_cause,
        warnings,
    })
}

/// The pass line for one point: p99 and error rate both at or under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    /// Highest p99 latency that still passes.
    pub p99: Duration,
    /// Highest error rate (0 to 1) that still passes.
    pub error_rate: f64,
}

impl Default for Limit {
    fn default() -> Self {
        Limit {
            p99: Duration::from_secs(1),
            error_rate: 0.01,
        }
    }
}

impl Limit {
    fn p99_ms(&self) -> f64 {
        self.p99.as_secs_f64() * 1000.0
    }

    /// True when the point is at or under both limits.
    #[must_use]
    pub fn passes(&self, p: &Point) -> bool {
        let o = self.over(p);
        !o.p99 && !o.errors
    }

    /// Which limits the point is over. A NaN p99 counts as over, so it never passes.
    #[must_use]
    pub fn over(&self, p: &Point) -> Over {
        Over {
            // incomparable (NaN) is over: `is_none_or`
            p99: p
                .p99_ms
                .partial_cmp(&self.p99_ms())
                .is_none_or(Ordering::is_gt),
            errors: p
                .error_rate
                .partial_cmp(&self.error_rate)
                .is_none_or(Ordering::is_gt),
        }
    }

    /// True when the point is clear of the line: p99 at most 0.9 of the limit and error rate at
    /// most half of it. A pass without this margin is a graze, not a recovery.
    #[must_use]
    pub fn passes_with_margin(&self, p: &Point) -> bool {
        p.p99_ms <= 0.9 * self.p99_ms() && p.error_rate <= 0.5 * self.error_rate
    }
}

// By hand so the reply reads `{ "p99_ms", "error_rate" }`, not serde's `Duration` form.
impl Serialize for Limit {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Limit", 2)?;
        st.serialize_field("p99_ms", &self.p99_ms())?;
        st.serialize_field("error_rate", &self.error_rate)?;
        st.end()
    }
}

/// Which of the two limits a point is over (the reader of a record does not redo the comparison).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Over {
    /// p99 latency is over the limit.
    pub p99: bool,
    /// Error rate is over the limit.
    pub errors: bool,
}

/// One node's load at one point.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodePoint {
    /// Node name.
    pub name: String,
    /// Busy fraction of its slots, when it has a bounded pool.
    pub utilization: Option<f64>,
    /// True when the node was saturated at this point.
    pub saturated: bool,
    /// Offered request rate at which the node saturates, when known.
    pub saturates_at_req_s: Option<f64>,
}

/// The result of running the topology at one load factor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    /// Load factor, the multiple of the configured arrival rate.
    pub k: f64,
    /// Offered requests per second at this factor.
    pub req_s: f64,
    /// Requests reported (after warmup).
    pub requests: u64,
    /// Successful requests per second.
    pub goodput_req_s: f64,
    /// Fraction of requests that failed (0 to 1).
    pub error_rate: f64,
    /// Median latency, ms.
    pub p50_ms: f64,
    /// 95th percentile latency, ms.
    pub p95_ms: f64,
    /// 99th percentile latency, ms.
    pub p99_ms: f64,
    /// Main cause of failures, when any.
    pub cause: Option<&'static str>,
    /// Per-node load.
    pub nodes: Vec<NodePoint>,
    /// Which limits this point is over, filled by `sweep()` from the limit in force.
    pub over: Over,
}

/// What the sweep found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    /// Last passing point before the first crossing, to the search resolution.
    Knee {
        /// Load factor.
        k: f64,
        /// Offered requests per second.
        req_s: f64,
    },
    /// Already over the limit at the lowest point.
    BrokenAtLowest {
        /// Load factor.
        k: f64,
        /// Offered requests per second.
        req_s: f64,
    },
    /// No failure seen: held to the last point that ran (see `stopped_at` for a cut ladder).
    HoldsTo {
        /// Load factor.
        k: f64,
        /// Offered requests per second.
        req_s: f64,
    },
}

/// Why one point could not be run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PointFail {
    /// The run would exceed the size caps.
    TooLarge(String),
    /// Any other error.
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(p99_ms: f64, error_rate: f64) -> Point {
        Point {
            k: 1.0,
            req_s: 1.0,
            requests: 0,
            goodput_req_s: 0.0,
            error_rate,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms,
            cause: None,
            nodes: Vec::new(),
            over: Over::default(),
        }
    }

    #[test]
    fn over_says_which_limit_and_matches_passes() {
        let l = Limit::default(); // 1000 ms, 0.01
        assert_eq!(
            l.over(&pt(1000.0, 0.01)),
            Over {
                p99: false,
                errors: false
            }
        );
        assert_eq!(
            l.over(&pt(1001.0, 0.0)),
            Over {
                p99: true,
                errors: false
            }
        );
        assert_eq!(
            l.over(&pt(0.0, 0.02)),
            Over {
                p99: false,
                errors: true
            }
        );
        assert_eq!(
            l.over(&pt(1001.0, 0.02)),
            Over {
                p99: true,
                errors: true
            }
        );
        // a NaN p99 is over, as `passes` was before
        assert!(l.over(&pt(f64::NAN, 0.0)).p99);
        assert!(!l.passes(&pt(f64::NAN, 0.0)));
    }

    #[test]
    fn the_limit_passes_at_its_exact_boundaries() {
        let l = Limit::default(); // 1000 ms, 0.01
        assert!(l.passes(&pt(1000.0, 0.01)));
        assert!(!l.passes(&pt(1000.0 + 1e-6, 0.0)));
        assert!(!l.passes(&pt(0.0, 0.01 + 1e-9)));
        assert!(l.passes_with_margin(&pt(900.0, 0.005)));
        assert!(!l.passes_with_margin(&pt(900.0 + 1e-6, 0.0)));
        assert!(!l.passes_with_margin(&pt(0.0, 0.005 + 1e-9)));
    }

    #[test]
    fn a_limit_serializes_as_ms_and_a_share() {
        let l = Limit {
            p99: Duration::from_millis(250),
            error_rate: 0.02,
        };
        assert_eq!(
            serde_json::to_value(&l).unwrap(),
            serde_json::json!({ "p99_ms": 250.0, "error_rate": 0.02 })
        );
    }
}
