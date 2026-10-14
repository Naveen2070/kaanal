//! Discrete-event simulator: the second driver of the core cost model (rule R2.5).
//!
//! [`run`] feeds open Poisson arrival streams through a [`Topology`] on a virtual
//! clock. Every service time comes from `kaanal_core::sample_plan`, and the replay `Engine` in
//! `kaanal-core` runs it with fork/join, FIFO queues, bounded queues and rate limits. Live mode
//! runs the same engine on a real clock and this crate has no timing model of its own, so live
//! mode and the simulator cannot drift apart.

#![warn(missing_docs)]

mod engine;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::{ChainResult, ChaosState, Dist, Endpoint, Endpoints, Hop, Topology};

/// **Private.** Most hop records one run may produce (R6.13): expected reported (post-warmup) arrivals × the most
/// hops each stream's chain can take (`Seq::hops`, repeats included). A record costs about
/// 100 bytes per hop plus a few hundred per request, so the cap bounds a run near 2–3 GB.
/// Start value; tune once measured.
const MAX_HOP_RECORDS: f64 = 20_000_000.0;
/// **Private.** Most hop events one run may simulate, warmup included (a CPU bound; R6.13 bounds
/// memory only). About 0.35 µs per hop in release, so the cap is roughly 20 s. Start value;
/// tune once measured.
const MAX_SIM_HOPS: f64 = 50_000_000.0;
/// **Private.** Largest single service/net time a topology may sample (the config's 1 h
/// cap times the 10× exponential/lognormal tail cap).
const MAX_SERVICE: Duration = Duration::from_hours(10);
/// **Private.** Slowest stream: one arrival per ~11.6 days. Bounds the mean gap.
const MIN_RATE_PER_S: f64 = 1e-6;
/// **Private.** Fastest stream: a 1 µs mean gap. Above ~1e9 the gap rounds to 0 ns.
const MAX_RATE_PER_S: f64 = 1e6;
/// **Private.** Longest simulated time; keeps the virtual clock far inside `u64` nanoseconds.
const MAX_SIM_DURATION: Duration = Duration::from_hours(365 * 24);
/// **Private.** Most streams a run may have (a request tag packs the stream index into 16 bits).
const MAX_STREAMS: usize = 65_535;

/// One open Poisson stream on a compiled endpoint: its key (`"METHOD /path"`, as live mode)
/// and its rate. Any compiled endpoint can be a stream, static ones included.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// The endpoint's key, e.g. `"GET /users/{id}"`: part of the seed.
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

impl SimReport {
    /// Failed reported requests by kind: `status`, `reset`, `malformed` or `timeout`
    /// (kinds with no failures are absent).
    #[must_use]
    pub fn errors_by_kind(&self) -> BTreeMap<&'static str, u64> {
        let mut by = BTreeMap::new();
        for r in &self.requests {
            if let ChainResult::Err { kind, .. } = r.result {
                *by.entry(kind.as_str()).or_insert(0) += 1;
            }
        }
        by
    }
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
    /// Ok, or where and how the request failed: a chaos failure (its `kind`: status, reset,
    /// malformed or hang; a hang is reported as kind `timeout`, 504), a deadline that fired (kind `timeout`, 504), a rate limit (429,
    /// with `retry_after`) or a full queue (the node's overflow status).
    pub result: ChainResult,
    /// The hops that ran, in plan order, each with the queue wait before it: `start` is the
    /// actual offset from arrival; a hop whose branch a failure cancelled is cut at the
    /// failure instant (one that had not started is absent). Equals `execute()`'s hops at
    /// zero load, including when two failing branches tie at the same instant through
    /// zero-duration hops that need no slot or token (D17; the lower branch wins, as in live mode).
    pub hops: Vec<(Hop, Duration)>,
}

/// Per-node statistics over the reported requests.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeStats {
    /// Own hops (not `Net`) of reported requests that started at this node, including a
    /// cancelled hop that began at the failure instant (it held its slot) but is absent from
    /// the record.
    pub completed: u64,
    /// Mean queue wait before those hops.
    pub mean_wait: Duration,
    /// Busy slot-time inside the reporting window `[warmup, duration]` ÷ (slots × window), so
    /// it never exceeds 1; `0.0` for unlimited nodes.
    pub utilization: f64,
    /// Longest queue seen when a request joined at or after warmup (so it can include
    /// requests that arrived earlier; a queue that only drains after warmup is not counted).
    pub max_queue: usize,
    /// Requests refused at this node by its rate limit (429), over reported requests.
    pub limited: u64,
    /// Requests refused at this node by a full queue, over reported requests.
    pub overflowed: u64,
    /// Deadlines (`timeout:` spans) that fired at this node, over reported requests. A `hang` that\n    /// runs to its planned 504 is a timeout in [`SimReport::errors_by_kind`] but not counted here (no deadline fired).
    pub timed_out: u64,
}

/// Simulates `cfg` on `topo` under fixed `chaos`. Deterministic: same inputs, same report.
/// Nodes with `concurrency` or a `pool` queue requests (bounded queues overflow with the
/// configured status); nodes with a `rate_limit` refuse with 429; both run in the shared replay
/// engine, exactly as live mode does.
///
/// # Errors
/// Returns a message for: no streams; a rate that is not finite or outside
/// 1e-6..=1e6 per second; an `endpoint_key` that is not a compiled endpoint; a zero `duration`,
/// `warmup ≥ duration`, or a duration over 365 days; duplicate `endpoint_key`s; or more
/// than 20 000 000 expected hop records (arrivals × the most hops of each stream's chain);
/// a chaos entry that [`kaanal_core::NodeChaos::validate`] rejects
/// (e.g. `latency_add` over 5 min); or a node or edge
/// latency that can sample over 10 h (e.g. a config with `p50` above `p99`, which core
/// turns into a fixed `p50` without the 1 h cap, or a hand-edited `Topology`).
/// A run whose virtual clock would pass `u64::MAX` ns (about 584 years, e.g. 600k requests of
/// 9 h each through one slot) is also an error, never a silently wrong report.
/// An overloaded system (ρ ≥ 1) is not an error.
pub fn run(
    topo: &Topology,
    endpoints: &Endpoints,
    chaos: &ChaosState,
    cfg: &SimConfig,
) -> Result<SimReport, String> {
    let streams = validate(topo, endpoints, chaos, cfg)?;
    engine::simulate(topo, &streams, chaos, cfg).map(|(report, _)| report)
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
fn validate(
    topo: &Topology,
    endpoints: &Endpoints,
    chaos: &ChaosState,
    cfg: &SimConfig,
) -> Result<Vec<Arc<Endpoint>>, String> {
    if cfg.streams.len() > MAX_STREAMS {
        return Err(format!(
            "at most {MAX_STREAMS} streams (got {})",
            cfg.streams.len()
        ));
    }
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
        if too_long(&n.proc)
            || n.ops.iter().flatten().any(too_long)
            || n.cache
                .iter()
                .any(|c| too_long(&c.hit) || too_long(&c.miss))
        {
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
    // The same limits live mode enforces (R6.9); the 5 min latency cap, with the hop
    // cap, keeps the virtual clock far inside `u64` nanoseconds.
    // first_error() is ordered, so the error names the same node every run.
    if let Some(e) = chaos.first_error() {
        return Err(e);
    }
    let (mut arrivals, mut records, mut work) = (0.0, 0.0, 0.0);
    let mut out = Vec::with_capacity(cfg.streams.len());
    for (i, s) in cfg.streams.iter().enumerate() {
        if !(MIN_RATE_PER_S..=MAX_RATE_PER_S).contains(&s.rate_per_s) {
            return Err(format!(
                "stream {i}: rate_per_s must be between 0.000001 and 1000000 (got {})",
                s.rate_per_s
            ));
        }
        let Some(ep) = endpoints.get(&s.endpoint_key) else {
            return Err(format!("stream {i}: unknown endpoint '{}'", s.endpoint_key));
        };
        if cfg.streams[..i]
            .iter()
            .any(|o| o.endpoint_key == s.endpoint_key)
        {
            return Err(format!(
                "stream {i}: duplicate endpoint_key '{}'",
                s.endpoint_key
            ));
        }
        // Only requests arriving after warmup are recorded, so only they count.
        let a = s.rate_per_s * cfg.duration.saturating_sub(cfg.warmup).as_secs_f64();
        arrivals += a;
        #[expect(
            clippy::cast_precision_loss,
            reason = "hop counts are capped at 100 000 (E13)"
        )]
        let hops = ep.chain.hops as f64;
        records += a * hops;
        work += s.rate_per_s * cfg.duration.as_secs_f64() * hops;
        out.push(Arc::clone(ep));
    }
    if records > MAX_HOP_RECORDS {
        return Err(format!(
            "workload too large: about {arrivals:.0} arrivals with up to {records:.0} hops in total (max {MAX_HOP_RECORDS:.0}); lower the rate, the duration or the chain length"
        ));
    }
    if work > MAX_SIM_HOPS {
        return Err(format!(
            "simulated work too large: about {work:.0} hops in total, warmup included (max {MAX_SIM_HOPS:.0}); lower the rate, the duration, the warmup or the chain length"
        ));
    }
    Ok(out)
}
