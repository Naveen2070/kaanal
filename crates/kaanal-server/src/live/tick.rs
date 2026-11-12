//! What the live view reads (step 8L, RULES 105): every 250 ms the aggregator turns the requests
//! that finished in the tick into counts the page can sum over any window, a latency histogram of
//! the requests that completed, and a few sampled journeys the page draws as dots. All figures are
//! integers or sums, so a window of ticks is added up on the page and its p50 and p99 are read from
//! the summed histograms; the server keeps no history.
//!
//! The figures come from the finished requests (the generated traffic) and a snapshot of the
//! engine's queues at the instant of the tick. A node's `arrived` counts the own hops that ran at
//! it; a wire's `carried` counts the calls that crossed it (an own hop on one node after an own hop
//! on another with an edge between them); both are attributed to the tick the request finished in.

use std::time::Duration;

use kaanal_core::replay::{FailCause, Finished};
use kaanal_core::{ChainResult, HopKind, Topology};
use serde::Serialize;

use super::hist::LatencyHist;

mod probe;
pub(crate) use probe::{BreakerTick, Funnel, Readings, readings};

/// Most journeys one tick carries (40 a second at four ticks a second).
pub(crate) const MAX_JOURNEYS: usize = 10;

/// Most real (mock) requests' journeys one tick carries, apart from the generated ones.
pub(crate) const MAX_REAL_JOURNEYS: usize = 10;

/// Requests dropped at a node in the tick, by cause.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Dropped {
    pub overflow: u32,
    pub rate_limit: u32,
    pub deadline: u32,
    pub breaker: u32,
    pub oom: u32,
    pub chaos: u32,
    /// Requests to a route whose pin answers an error: the chain ran, the answer is the pin's.
    #[serde(skip_serializing_if = "is_zero")]
    pub pinned: u32,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands a reference"
)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A generated request the engine served but whose route is pinned to an error: it counts as
/// failed at its entry component, with cause `pinned`.
fn pinned_ok(f: &Finished) -> bool {
    matches!(f.result, ChainResult::Ok) && crate::actor::is_pinned_error(f)
}

/// **Private.** The CPU each node really used between two readings of its busy area: busy
/// millicore-ns gained over `millicores x copies x span`, at most 1; `None` for a node with no `cpu`
/// or a span of zero.
#[expect(
    clippy::cast_precision_loss,
    reason = "a ratio of nanosecond totals; f64 precision is ample"
)]
pub(crate) fn cpu_use(
    topo: &Topology,
    before: &[u128],
    after: &[u128],
    span_ns: u64,
) -> Vec<Option<f64>> {
    topo.nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mc = n.cpu?;
            if span_ns == 0 {
                return None;
            }
            let gained = after
                .get(i)?
                .saturating_sub(before.get(i).copied().unwrap_or(0));
            let capacity = f64::from(mc) * f64::from(n.replicas.max(1)) * span_ns as f64;
            Some((gained as f64 / capacity).min(1.0))
        })
        .collect()
}

/// One component in the tick.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct NodeTick {
    /// Own hops that ran here, in requests that finished in this tick.
    pub arrived: u32,
    pub dropped: Dropped,
    /// Waiting for a slot at the instant of the tick, all copies.
    pub queue: usize,
    /// Slots in use at that instant, all copies.
    pub busy_slots: u32,
    /// Running hops over the cores of a node with `cpu`, at that instant; `None` without `cpu`.
    pub cpu_load: Option<f64>,
    /// The CPU the running work really used over the cores of a node with `cpu`, across this tick's
    /// span, at most 1 (step 8U); `None` without `cpu`, or before there are two readings.
    pub cpu_utilization: Option<f64>,
    /// How long the own hops that ran here worked (waiting excluded), as occupied `(bucket, count)`
    /// pairs of [`LatencyHist`]; the same buckets as the tick's `hist`. Empty in a real share.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hist: Vec<(u16, u32)>,
    /// Own hops that ran at each copy (index = copy), only for a node that ran on more than one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replicas: Vec<u32>,
    /// `(running, waiting)` of each copy at the instant of the tick, only for a node with several.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copy_load: Vec<(u32, usize)>,
}

/// One wire in the tick.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct WireTick {
    /// Calls that crossed it, in requests that finished in this tick.
    pub carried: u32,
    /// Time requests spent waiting to get onto it (sharing the wire), summed over the hops that
    /// crossed it in requests that finished in this tick (ns).
    pub wait_ns: u64,
    /// Time those hops spent on it, waiting excluded (ns): `wire_ns` over `carried` is the mean
    /// crossing.
    pub wire_ns: u64,
    /// Crossings that were a retry (attempt 2 or later): the calls are `carried - retries`.
    pub retries: u32,
    /// Calls that made at least one retry: the second attempts.
    pub retried: u32,
    /// What the engine counted for the wire's retry scopes in the span: the funnel from calls to
    /// recovered. Absent for a wire with no retry scope or in a real share.
    #[serde(skip_serializing_if = "Funnel::is_empty")]
    pub funnel: Funnel,
    /// How busy the wire was over the span (1 = full the whole time): bytes carried over its rate.
    /// Absent for a wire with no `bandwidth`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<f64>,
}

/// One route of the offered mix in the tick, by index into the session's `streams`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct EndpointTick {
    /// Generated requests of this route that finished.
    pub requests: u32,
    /// Of those, dropped.
    pub failed: u32,
    /// Latency of the served ones, as occupied `(bucket, count)` pairs of [`LatencyHist`].
    pub hist: Vec<(u16, u32)>,
}

/// One hop of a sampled journey, offsets from the request's arrival.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct JourneyHop {
    pub node: usize,
    pub replica: u16,
    pub kind: &'static str,
    pub start_ns: u64,
    pub end_ns: u64,
    pub wait_ns: u64,
    /// Which try of a retried call (1 outside retries).
    pub attempt: u8,
    /// The wire a `net` or `link` hop crosses, or the wire an own hop was entered through.
    pub edge: Option<usize>,
}

/// A finished request the page draws as a dot along its hops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Journey {
    pub hops: Vec<JourneyHop>,
    /// The index in `hops` where it was dropped; `None` when it was served.
    pub dropped_at: Option<usize>,
    pub total_ns: u64,
    /// The route of a generated request, as an index into the session's `streams`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<u16>,
    /// Why a dropped request was dropped: `overflow`, `rate_limit`, `deadline`, `breaker`, `oom` or
    /// `chaos`, the keys of a node's `dropped`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<&'static str>,
    /// A real mock request (step 8A), drawn apart from the generated ones.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub real: bool,
}

/// What the generator is doing, repeated in every tick so the strip needs no other call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct TickStatus {
    pub rate: f64,
    pub paused: bool,
    pub overloaded: bool,
    pub in_flight: u64,
}

/// The real mock requests' share of a tick (step 8A, RULES 108): the same counts as the tick, for
/// requests a frontend made. The tick's own figures include them; a page shows the generated
/// share as the tick minus this. Queue, busy slots and CPU load are engine snapshots, so they
/// stay on the tick and are zero here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RealTick {
    pub requests: u32,
    pub failed: u32,
    pub nodes: Vec<NodeTick>,
    pub wires: Vec<WireTick>,
    pub hist: Vec<(u16, u32)>,
}

/// One tick.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Tick {
    /// Milliseconds since the session started, at the end of the tick.
    pub t_ms: u64,
    /// Milliseconds since the running system started: the clock a `from_ms` outage is set on.
    pub system_ms: u64,
    /// Requests that finished (served or dropped).
    pub requests: u32,
    /// Of those, dropped.
    pub failed: u32,
    pub nodes: Vec<NodeTick>,
    pub wires: Vec<WireTick>,
    /// Latency of the served requests, as occupied `(bucket, count)` pairs of [`LatencyHist`].
    pub hist: Vec<(u16, u32)>,
    /// Per route of the offered mix (generated requests only); empty when none finished.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<EndpointTick>,
    /// What each circuit breaker is doing at the instant of the tick; empty when there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breakers: Vec<BreakerTick>,
    pub journeys: Vec<Journey>,
    pub status: TickStatus,
    /// Present only in a tick that had a real request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real: Option<RealTick>,
}

/// A label for the page: what kind of time a hop is.
fn kind_label(k: HopKind) -> &'static str {
    match k {
        HopKind::Net => "net",
        HopKind::Proc => "proc",
        HopKind::Op(_) => "op",
        HopKind::CacheHit => "cache_hit",
        HopKind::CacheMiss => "cache_miss",
        HopKind::Delay => "delay",
        HopKind::Connect { .. } => "connect",
        HopKind::Retransmit { .. } => "retransmit",
        HopKind::FirstByte => "first_byte",
        HopKind::Link { .. } => "link",
        HopKind::Nic { .. } => "nic",
    }
}

/// The key of a node's `dropped` that a failure counts under.
fn cause_key(c: FailCause) -> &'static str {
    match c {
        FailCause::Overflow => "overflow",
        FailCause::RateLimit => "rate_limit",
        FailCause::Deadline => "deadline",
        FailCause::Breaker => "breaker",
        FailCause::Oom => "oom",
        FailCause::Chaos => "chaos",
    }
}

fn is_own(k: HopKind) -> bool {
    matches!(
        k,
        HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss
    )
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// What a set of finished requests adds up to: counts per node and wire and a latency histogram.
struct Counts {
    nodes: Vec<NodeTick>,
    /// The work time of each node's own hops; empty where it is not wanted (the real share).
    node_hist: Vec<LatencyHist>,
    wires: Vec<WireTick>,
    hist: LatencyHist,
    /// Per route (only where `node_hist` is wanted): requests, failures and served latencies.
    streams: Vec<(EndpointTick, LatencyHist)>,
    requests: u32,
    failed: u32,
}

/// Most routes the per-route figures follow (the mix is small; a tag that claims more is ignored).
const MAX_STREAMS: usize = 256;

/// A request's journey, built once: its hops, where it was dropped, its total time.
type Built = (Vec<JourneyHop>, Option<usize>, u64);

impl Counts {
    fn new(nodes: usize, wires: usize) -> Counts {
        Counts {
            nodes: vec![NodeTick::default(); nodes],
            node_hist: Vec::new(),
            wires: vec![WireTick::default(); wires],
            hist: LatencyHist::new(),
            streams: Vec::new(),
            requests: 0,
            failed: 0,
        }
    }

    /// Keeps each node's own-hop work times too (the total does; the real share does not).
    fn with_node_hist(mut self) -> Counts {
        self.node_hist = vec![LatencyHist::new(); self.nodes.len()];
        self
    }

    /// Counts a request against its route when per-route figures are wanted (the total, not the
    /// real share) and the route is a generated one; the route's index in `streams`.
    fn count_route(&mut self, f: &Finished) -> Option<usize> {
        let s = (!self.node_hist.is_empty())
            .then(|| crate::actor::stream_of(f))
            .flatten()
            .map(usize::from)
            .filter(|s| *s < MAX_STREAMS)?;
        if self.streams.len() <= s {
            self.streams
                .resize_with(s + 1, || (EndpointTick::default(), LatencyHist::new()));
        }
        self.streams[s].0.requests += 1;
        Some(s)
    }

    /// Counts a drop at `node` under its cause.
    fn count_drop(&mut self, node: usize, cause: Option<FailCause>) {
        let d = &mut self.nodes[node].dropped;
        match cause {
            Some(FailCause::Overflow) => d.overflow += 1,
            Some(FailCause::RateLimit) => d.rate_limit += 1,
            Some(FailCause::Deadline) => d.deadline += 1,
            Some(FailCause::Breaker) => d.breaker += 1,
            Some(FailCause::Oom) => d.oom += 1,
            Some(FailCause::Chaos) | None => d.chaos += 1,
        }
    }

    /// Adds one finished request; with `build`, also returns its journey. `None` for a request of
    /// another system (the old engine's, while an Apply swaps): it names nodes this set does not
    /// have and belongs to no tick here.
    fn add(&mut self, f: &Finished, edge_of: &[Vec<Option<usize>>], build: bool) -> Option<Built> {
        let n = self.nodes.len();
        let foreign = f.hops.iter().any(|h| h.hop.node >= n)
            || matches!(&f.result, ChainResult::Err { node, .. } if *node >= n);
        if foreign {
            return None;
        }
        self.requests += 1;
        let total = f.done.saturating_sub(f.arrival);
        let mut prev_own: Option<usize> = None;
        // the node and wire of the last own hop that crossed a wire
        let mut last_in: Option<(usize, usize)> = None;
        let mut hops = Vec::new();
        for h in &f.hops {
            let kind = h.hop.kind;
            let mut edge = None;
            if is_own(kind) {
                self.nodes[h.hop.node].arrived += 1;
                if !self.node_hist.is_empty() {
                    self.node_hist[h.hop.node].record(nanos(h.hop.dur));
                }
                let r = usize::from(h.hop.replica);
                let copies = &mut self.nodes[h.hop.node].replicas;
                if copies.len() <= r {
                    copies.resize(r + 1, 0);
                }
                copies[r] += 1;
                if let Some(p) = prev_own.filter(|p| *p != h.hop.node)
                    && let Some(e) = edge_of[p][h.hop.node]
                {
                    self.wires[e].carried += 1;
                    last_in = Some((h.hop.node, e));
                    // the wire this hop was entered through, so a wire with no latency hop of its
                    // own still has something to draw a dot along
                    edge = Some(e);
                }
                // a retry runs at the node its first try ran at, so it crosses no wire again: it
                // counts on the wire that first try came in through
                if h.attempt > 1
                    && let Some((_, e)) = last_in.filter(|(n, _)| *n == h.hop.node)
                {
                    self.wires[e].retries += 1;
                    if h.attempt == 2 {
                        self.wires[e].retried += 1;
                    }
                }
                prev_own = Some(h.hop.node);
            } else if let HopKind::Link { edge: e, .. } = kind {
                edge = Some(usize::from(e));
            } else if matches!(kind, HopKind::Net) {
                edge = prev_own.and_then(|p| edge_of[p][h.hop.node]);
            }
            if !is_own(kind)
                && let Some(e) = edge
            {
                self.wires[e].wait_ns += nanos(h.wait);
                self.wires[e].wire_ns += nanos(h.hop.dur);
            }
            if build {
                hops.push(JourneyHop {
                    node: h.hop.node,
                    replica: h.hop.replica,
                    kind: kind_label(kind),
                    start_ns: nanos(h.hop.start),
                    end_ns: nanos(h.hop.start + h.hop.dur),
                    wait_ns: nanos(h.wait),
                    attempt: h.attempt,
                    edge,
                });
            }
        }
        let mut dropped_at = None;
        let stream = self.count_route(f);
        match &f.result {
            ChainResult::Ok if pinned_ok(f) => {
                self.failed += 1;
                if let Some(s) = stream {
                    self.streams[s].0.failed += 1;
                }
                // the entry component is where the chain began
                let entry = f.hops.first().map_or(0, |h| h.hop.node);
                self.nodes[entry].dropped.pinned += 1;
                dropped_at = (build && !hops.is_empty()).then_some(0);
            }
            ChainResult::Ok => {
                // one `ln` for the tick's histogram and the route's
                let b = LatencyHist::bucket(total);
                self.hist.record_in(b);
                if let Some(s) = stream {
                    self.streams[s].1.record_in(b);
                }
            }
            ChainResult::Err { node, .. } => {
                self.failed += 1;
                if let Some(s) = stream {
                    self.streams[s].0.failed += 1;
                }
                self.count_drop(*node, f.cause);
                if build {
                    dropped_at = Some(
                        hops.iter()
                            .position(|h| h.node == *node)
                            .unwrap_or_else(|| hops.len().saturating_sub(1)),
                    );
                }
            }
        }
        Some((hops, dropped_at, total))
    }
}

/// Sums finished requests into ticks: every request into the tick's own counts, and a real one
/// (a mock request, step 8A) into a second set as well, so the page can tell the two apart.
pub(crate) struct Aggregator {
    total: Counts,
    real: Counts,
    /// `edge_of[from][to]`, the index of the wire between two nodes.
    edge_of: Vec<Vec<Option<usize>>>,
    /// The tick's journeys, generated and real together; `real_journeys` of them are real.
    journeys: Vec<Journey>,
    real_journeys: usize,
    /// Generated requests seen since the session began (the sampling stride counts them).
    seen: u64,
    /// One generated request in `stride` is sampled, adapted to the last tick's rate.
    stride: u64,
    /// The measured CPU use of each node over the coming tick, set by [`Aggregator::set_cpu_use`].
    cpu_use: Vec<Option<f64>>,
    /// The engine's own figures for the coming tick, set by [`Aggregator::set_readings`].
    readings: Readings,
}

impl Aggregator {
    pub(crate) fn new(topo: &Topology) -> Aggregator {
        let n = topo.nodes.len();
        let mut edge_of = vec![vec![None; n]; n];
        for (i, e) in topo.edges.iter().enumerate() {
            edge_of[e.from][e.to] = Some(i);
        }
        Aggregator {
            total: Counts::new(n, topo.edges.len()).with_node_hist(),
            real: Counts::new(n, topo.edges.len()),
            edge_of,
            journeys: Vec::new(),
            real_journeys: 0,
            seen: 0,
            stride: 1,
            cpu_use: Vec::new(),
            readings: Readings::default(),
        }
    }

    /// Sets what the engine says about the tick about to be taken (see [`readings`]).
    pub(crate) fn set_readings(&mut self, r: Readings) {
        self.readings = r;
    }

    /// Sets the measured CPU use of each node over the tick about to be taken (see [`cpu_use`]).
    pub(crate) fn set_cpu_use(&mut self, use_: Vec<Option<f64>>) {
        self.cpu_use = use_;
    }

    /// Adds one finished request to the tick being built; `real` for a mock request.
    pub(crate) fn add(&mut self, f: &Finished, real: bool) {
        let Some((hops, dropped_at, total_ns)) = self.total.add(f, &self.edge_of, true) else {
            return;
        };
        if real {
            // the same counts again, for the real share; the journey is already built
            self.real.add(f, &self.edge_of, false);
        } else {
            self.seen += 1;
        }
        // a real request is always worth drawing (up to its own cap); of the generated ones one
        // in `stride` is, and a dropped one always
        let sample = if real {
            self.real_journeys < MAX_REAL_JOURNEYS
        } else {
            (self.seen.is_multiple_of(self.stride) || dropped_at.is_some())
                && self.journeys.len() - self.real_journeys < MAX_JOURNEYS
        };
        if sample && !hops.is_empty() {
            self.real_journeys += usize::from(real);
            self.journeys.push(Journey {
                hops,
                dropped_at,
                total_ns,
                stream: if real {
                    None
                } else {
                    crate::actor::stream_of(f)
                },
                cause: dropped_at.and(if pinned_ok(f) {
                    Some("pinned")
                } else {
                    f.cause.map(cause_key)
                }),
                real,
            });
        }
    }

    /// Closes the tick at `t_ms`: the counts so far, the queues of `loads` (one `(running,
    /// waiting)` per node, from the engine) and `status`; the next tick starts empty.
    pub(crate) fn take(
        &mut self,
        (t_ms, system_ms): (u64, u64),

        loads: &[(u32, usize)],
        topo: &Topology,
        status: TickStatus,
    ) -> Tick {
        let (n, w) = (topo.nodes.len(), topo.edges.len());
        let mut total = std::mem::replace(&mut self.total, Counts::new(n, w).with_node_hist());
        let mut real = std::mem::replace(&mut self.real, Counts::new(n, w));
        for (i, node) in total.nodes.iter_mut().enumerate() {
            node.hist = total
                .node_hist
                .get(i)
                .map(LatencyHist::sparse)
                .unwrap_or_default();
            let (busy, queued) = loads.get(i).copied().unwrap_or((0, 0));
            node.busy_slots = busy;
            node.queue = queued;
            node.cpu_utilization = self.cpu_use.get(i).copied().flatten();
            node.cpu_load = topo.nodes[i].cpu.map(|mc| {
                let cores = f64::from(mc) / 1000.0 * f64::from(topo.nodes[i].replicas.max(1));
                f64::from(busy) / cores
            });
        }
        let engine = std::mem::take(&mut self.readings);
        for (i, node) in total.nodes.iter_mut().enumerate() {
            if let Some(c) = engine.copies.get(i).filter(|c| c.len() > 1) {
                node.copy_load.clone_from(c);
            }
        }
        for (i, wire) in total.wires.iter_mut().enumerate() {
            wire.funnel = engine.funnel.get(i).copied().unwrap_or_default();
            wire.load = engine.link_load.get(i).copied().flatten();
        }
        // a node that ran on one copy has nothing to split
        for n in total.nodes.iter_mut().chain(real.nodes.iter_mut()) {
            if n.replicas.len() <= 1 {
                n.replicas.clear();
            }
        }
        let generated = total.requests - real.requests;
        let real_tick = (real.requests > 0).then(|| RealTick {
            requests: real.requests,
            failed: real.failed,
            nodes: real.nodes,
            wires: real.wires,
            hist: real.hist.sparse(),
        });
        let tick = Tick {
            t_ms,
            system_ms,
            requests: total.requests,
            failed: total.failed,
            nodes: total.nodes,
            wires: total.wires,
            hist: total.hist.sparse(),
            endpoints: total
                .streams
                .into_iter()
                .map(|(mut e, h)| {
                    e.hist = h.sparse();
                    e
                })
                .collect(),
            breakers: engine.breakers,
            journeys: std::mem::take(&mut self.journeys),
            status,
            real: real_tick,
        };
        // aim for about MAX_JOURNEYS generated journeys a tick at the rate just seen
        self.stride = (u64::from(generated) / MAX_JOURNEYS as u64).max(1);
        self.real_journeys = 0;
        tick
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod extra_tests;
