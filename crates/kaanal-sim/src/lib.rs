//! Discrete-event simulator: the second driver of the core cost model (rule R2.5).
//!
//! [`run`] feeds open Poisson arrival streams through a [`Topology`] on a virtual
//! clock. Every service time comes from [`kaanal_core::execute`], exactly as in live
//! mode; this crate only adds FIFO queueing at nodes that set `concurrency`. It has
//! no timing model of its own, so live mode and the simulator cannot drift apart.

#![warn(missing_docs)]

mod engine;

use std::time::Duration;

use kaanal_core::{ChainResult, ChaosState, Dist, Hop, NodeId, NodeKind, Op, Topology};

/// **Private.** Most expected arrivals per run. Every record is kept in memory (a few
/// hundred bytes plus ~90 bytes per hop), so this bounds a typical run (5 hops) to about
/// 1 GB. It bounds arrivals, not memory: a service with 20 databases has ~43 hops per
/// request and would need several GB at the cap (M1: bound arrivals × hops).
const MAX_ARRIVALS: f64 = 2_000_000.0;
/// **Private.** Largest single service/net time a topology may sample (the config's 1 h
/// cap times the 10× exponential/lognormal tail cap).
const MAX_SERVICE: Duration = Duration::from_hours(10);
/// **Private.** Slowest stream: one arrival per ~11.6 days. Bounds the mean gap.
const MIN_RATE_PER_S: f64 = 1e-6;
/// **Private.** Fastest stream: a 1 µs mean gap. Above ~1e9 the gap rounds to 0 ns.
const MAX_RATE_PER_S: f64 = 1e6;
/// **Private.** Longest simulated time; keeps the virtual clock far inside `u64` nanoseconds.
const MAX_SIM_DURATION: Duration = Duration::from_hours(365 * 24);

/// One open Poisson arrival stream: one endpoint receiving `rate_per_s` requests per second.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// The service that owns the route (the live router's `route.owner`); passed to
    /// `execute` as `Some(entry)`.
    pub entry: NodeId,
    /// The database operation this endpoint performs.
    pub op: Op,
    /// Live format `"METHOD /path-template"`, e.g. `"GET /users/{id}"`: part of the seed.
    pub endpoint_key: String,
    /// Mean arrivals per second (exponential gaps).
    pub rate_per_s: f64,
}

/// What to simulate.
#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// The arrival streams; each `endpoint_key` must be unique.
    pub streams: Vec<Stream>,
    /// Virtual time simulated. Arrivals stop here; requests in flight finish.
    pub duration: Duration,
    /// Requests arriving before this are simulated but not reported.
    pub warmup: Duration,
    /// The config's `seed:` (a `Topology` does not carry it). Same value live mode uses.
    pub seed: u64,
}

/// Everything one run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct SimReport {
    /// Requests arriving at or after `warmup`, in arrival order.
    pub requests: Vec<RequestRecord>,
    /// Per-node statistics, indexed by [`NodeId`].
    pub nodes: Vec<NodeStats>,
}

/// One simulated request.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestRecord {
    /// Index into [`SimConfig::streams`].
    pub stream: usize,
    /// The stream's request counter, as live mode's per-endpoint counter.
    pub n: u64,
    /// Arrival time on the virtual clock.
    pub arrival: Duration,
    /// Completion minus arrival: service times plus queue waits.
    pub latency: Duration,
    /// Ok, or where the chain failed (from `execute`).
    pub result: ChainResult,
    /// `execute`'s hops in order, each with the queue wait before it started.
    pub hops: Vec<(Hop, Duration)>,
}

/// Per-node statistics over the reported requests.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeStats {
    /// Own hops (proc/op) of reported requests at this node.
    pub completed: u64,
    /// Mean queue wait before those hops.
    pub mean_wait: Duration,
    /// Busy slot-time inside the reporting window `[warmup, duration]` ÷ (slots × window), so
    /// it never exceeds 1; `0.0` for unlimited nodes.
    pub utilization: f64,
    /// Longest queue seen when a request joined at or after warmup (so it can include
    /// requests that arrived earlier; a queue that only drains after warmup is not counted).
    pub max_queue: usize,
}

/// Simulates `cfg` on `topo` under fixed `chaos`. Deterministic: same inputs, same report.
///
/// # Errors
/// Returns a message for: no streams; a rate that is not finite or outside
/// 1e-6..=1e6 per second; an `entry` that is not a service node; a zero `duration`,
/// `warmup ≥ duration`, or a duration over 365 days; duplicate `endpoint_key`s; or more
/// than 2 000 000 expected arrivals; a chaos entry that [`kaanal_core::NodeChaos::validate`] rejects
/// (e.g. `latency_add` over 5 min); or a node or edge
/// latency that can sample over 10 h (e.g. a config with `p50` above `p99`, which core
/// turns into a fixed `p50` without the 1 h cap, or a hand-edited `Topology`).
/// A run whose virtual clock would pass `u64::MAX` ns (about 584 years, e.g. 2M requests of
/// 9 h each through one slot) is also an error, never a silently wrong report.
/// An overloaded system (ρ ≥ 1) is not an error.
pub fn run(topo: &Topology, chaos: &ChaosState, cfg: &SimConfig) -> Result<SimReport, String> {
    validate(topo, chaos, cfg)?;
    engine::simulate(topo, chaos, cfg)
}

/// **Private.** Largest value `d` can sample, in seconds (NaN if its parameters are NaN).
fn worst_case_secs(d: &Dist) -> f64 {
    match d {
        Dist::Fixed(x) => x.as_secs_f64(),
        Dist::LogNormal { cap_ms, .. } => cap_ms / 1000.0,
        Dist::Exp { mean } => mean.as_secs_f64() * 10.0,
    }
}

/// **Private.** Rejects inputs the engine cannot run safely (see [`run`]).
fn validate(topo: &Topology, chaos: &ChaosState, cfg: &SimConfig) -> Result<(), String> {
    if cfg.streams.is_empty() {
        return Err("no streams".into());
    }
    if cfg.duration.is_zero() || cfg.warmup >= cfg.duration {
        return Err(format!(
            "duration must be > 0 and warmup < duration (got duration {:?}, warmup {:?})",
            cfg.duration, cfg.warmup
        ));
    }
    if cfg.duration > MAX_SIM_DURATION {
        return Err(format!(
            "duration must be at most 365 days (got {:?})",
            cfg.duration
        ));
    }
    // `build()` caps config durations, but not everything: `{ p50: 100h, p99: 1ms }` falls
    // back to `Fixed(100h)`. Name the culprit so the user can fix the YAML.
    let too_long = |d: &Dist| {
        let w = worst_case_secs(d);
        w.is_nan() || w > MAX_SERVICE.as_secs_f64()
    };
    for n in &topo.nodes {
        if too_long(&n.proc) || n.ops.iter().flatten().any(too_long) {
            return Err(format!("node '{}': a latency can exceed 10h", n.name));
        }
    }
    for e in &topo.edges {
        if too_long(&e.net) {
            return Err(format!(
                "edge '{}' -> '{}': latency can exceed 10h",
                topo.nodes[e.from].name, topo.nodes[e.to].name
            ));
        }
    }
    // The same limits live mode enforces (R6.9); the 5 min latency cap, with the 2 M arrival
    // cap, keeps the virtual clock far inside `u64` nanoseconds.
    // min(): a HashMap has no order, and the error must name the same node every run.
    if let Some((id, e)) = chaos
        .iter()
        .filter_map(|(id, c)| c.validate().err().map(|e| (*id, e)))
        .min()
    {
        return Err(format!("chaos on node {id}: {e}"));
    }
    let mut expected = 0.0;
    for (i, s) in cfg.streams.iter().enumerate() {
        if !(MIN_RATE_PER_S..=MAX_RATE_PER_S).contains(&s.rate_per_s) {
            return Err(format!(
                "stream {i}: rate_per_s must be between 0.000001 and 1000000 (got {})",
                s.rate_per_s
            ));
        }
        match topo.nodes.get(s.entry) {
            Some(n) if n.kind == NodeKind::Service => {}
            Some(n) => {
                return Err(format!(
                    "stream {i}: entry '{}' is not a service node",
                    n.name
                ));
            }
            None => return Err(format!("stream {i}: entry {} is not a node", s.entry)),
        }
        if cfg.streams[..i]
            .iter()
            .any(|o| o.endpoint_key == s.endpoint_key)
        {
            return Err(format!(
                "stream {i}: duplicate endpoint_key '{}'",
                s.endpoint_key
            ));
        }
        expected += s.rate_per_s * cfg.duration.as_secs_f64();
    }
    if expected > MAX_ARRIVALS {
        return Err(format!(
            "workload too large: about {expected:.0} arrivals (max {MAX_ARRIVALS:.0})"
        ));
    }
    Ok(())
}
