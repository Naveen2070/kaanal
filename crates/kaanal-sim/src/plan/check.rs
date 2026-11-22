//! One run at a target load, through the sweep's own point runner.

use kaanal_core::{EffectiveChaos, Endpoints, Topology};

use crate::sweep::point::{floor, point_cfg, run_point};
use crate::sweep::{Limit, Point, PointFail};
use crate::{Limits, SimConfig};

/// Reported requests the check's run aims for (the sweep's default).
const TARGET_REQUESTS: u64 = 20_000;

/// The load a plan must carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// A multiple of the configured arrival rates.
    Multiple(f64),
    /// Total requests per second over all streams.
    PerSecond(f64),
}

impl Target {
    /// The load factor `k` for `base`: a multiple is the factor, a rate divides by the sum of the
    /// stream rates.
    ///
    /// # Errors
    /// A value that is not finite or not above zero, or streams whose rates do not sum above zero.
    pub fn factor(&self, base: &SimConfig) -> Result<f64, String> {
        let (Target::Multiple(v) | Target::PerSecond(v)) = *self;
        if !v.is_finite() || v <= 0.0 {
            return Err(format!("the target must be a number above zero (got {v})"));
        }
        let sum: f64 = base.streams.iter().map(|s| s.rate_per_s).sum();
        if !sum.is_finite() || sum <= 0.0 {
            return Err("the streams need a rate above zero".into());
        }
        Ok(match self {
            Target::Multiple(_) => v,
            Target::PerSecond(_) => v / sum,
        })
    }
}

/// What one run at the target says.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// True when the point is under the limit with margin (`Limit::passes_with_margin`).
    pub holds: bool,
    /// The run itself.
    pub point: Point,
    /// The lowest analytic `saturates_at_req_s` of any node: an estimate (the M/M/c capacity
    /// of the weakest node), not a measurement.
    pub capacity_req_s: Option<f64>,
    /// The node with that lowest value, else the saturated node of highest utilisation.
    pub bottleneck: Option<String>,
}

/// Runs `topo` at load factor `k` with `base`'s seed and checks it against `limit`. The window is
/// sized for the candidate's own `floor`, so a slower node gets a longer one.
///
/// # Errors
/// The point's failure: too large for `limits`, or any other run error.
pub fn check(
    topo: &Topology,
    endpoints: &Endpoints,
    chaos: &EffectiveChaos,
    base: &SimConfig,
    k: f64,
    limit: &Limit,
    limits: &Limits,
) -> Result<Check, PointFail> {
    let cfg = point_cfg(base, k, TARGET_REQUESTS, floor(topo));
    let point = run_point(topo, endpoints, chaos, &cfg, k, limits)?;
    let weakest = point
        .nodes
        .iter()
        .filter_map(|n| Some((n, n.saturates_at_req_s.filter(|c| c.is_finite())?)))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let bottleneck = weakest.map(|(n, _)| n).or_else(|| {
        point.nodes.iter().filter(|n| n.saturated).max_by(|a, b| {
            a.utilization
                .unwrap_or(0.0)
                .total_cmp(&b.utilization.unwrap_or(0.0))
        })
    });
    Ok(Check {
        holds: limit.passes_with_margin(&point),
        capacity_req_s: weakest.map(|(_, c)| c),
        bottleneck: bottleneck.map(|n| n.name.clone()),
        point,
    })
}
