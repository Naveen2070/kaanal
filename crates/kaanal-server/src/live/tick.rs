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

/// Most journeys one tick carries (40 a second at four ticks a second).
pub(crate) const MAX_JOURNEYS: usize = 10;

/// Requests dropped at a node in the tick, by cause.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Dropped {
    pub overflow: u32,
    pub rate_limit: u32,
    pub deadline: u32,
    pub breaker: u32,
    pub chaos: u32,
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
}

/// One wire in the tick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct WireTick {
    /// Calls that crossed it, in requests that finished in this tick.
    pub carried: u32,
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
}

/// What the generator is doing, repeated in every tick so the strip needs no other call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct TickStatus {
    pub rate: f64,
    pub paused: bool,
    pub overloaded: bool,
    pub in_flight: u64,
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
    pub journeys: Vec<Journey>,
    pub status: TickStatus,
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

fn is_own(k: HopKind) -> bool {
    matches!(
        k,
        HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss
    )
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Sums finished requests into ticks.
pub(crate) struct Aggregator {
    nodes: Vec<NodeTick>,
    wires: Vec<WireTick>,
    /// `edge_of[from][to]`, the index of the wire between two nodes.
    edge_of: Vec<Vec<Option<usize>>>,
    hist: LatencyHist,
    requests: u32,
    failed: u32,
    journeys: Vec<Journey>,
    /// Requests seen since the session began (the sampling stride counts them).
    seen: u64,
    /// One request in `stride` is sampled, adapted to the last tick's rate.
    stride: u64,
}

impl Aggregator {
    pub(crate) fn new(topo: &Topology) -> Aggregator {
        let n = topo.nodes.len();
        let mut edge_of = vec![vec![None; n]; n];
        for (i, e) in topo.edges.iter().enumerate() {
            edge_of[e.from][e.to] = Some(i);
        }
        Aggregator {
            nodes: vec![NodeTick::default(); n],
            wires: vec![WireTick::default(); topo.edges.len()],
            edge_of,
            hist: LatencyHist::new(),
            requests: 0,
            failed: 0,
            journeys: Vec::new(),
            seen: 0,
            stride: 1,
        }
    }

    /// Adds one finished request to the tick being built.
    pub(crate) fn add(&mut self, f: &Finished) {
        // a request of another system (the old engine's, while an Apply swaps) names nodes this
        // aggregator does not have: it belongs to no tick here
        let n = self.nodes.len();
        let foreign = f.hops.iter().any(|h| h.hop.node >= n)
            || matches!(&f.result, ChainResult::Err { node, .. } if *node >= n);
        if foreign {
            return;
        }
        self.requests += 1;
        self.seen += 1;
        let total = f.done.saturating_sub(f.arrival);
        let mut prev_own: Option<usize> = None;
        let mut hops = Vec::new();
        for h in &f.hops {
            let kind = h.hop.kind;
            let mut edge = None;
            if is_own(kind) {
                self.nodes[h.hop.node].arrived += 1;
                if let Some(p) = prev_own.filter(|p| *p != h.hop.node)
                    && let Some(e) = self.edge_of[p][h.hop.node]
                {
                    self.wires[e].carried += 1;
                    // the wire this hop was entered through, so a wire with no latency hop of its
                    // own still has something to draw a dot along
                    edge = Some(e);
                }
                prev_own = Some(h.hop.node);
            } else if let HopKind::Link { edge: e, .. } = kind {
                edge = Some(usize::from(e));
            } else if matches!(kind, HopKind::Net) {
                edge = prev_own.and_then(|p| self.edge_of[p][h.hop.node]);
            }
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
        let mut dropped_at = None;
        match &f.result {
            ChainResult::Ok => self.hist.record(total),
            ChainResult::Err { node, .. } => {
                self.failed += 1;
                let d = &mut self.nodes[*node].dropped;
                match f.cause {
                    Some(FailCause::Overflow) => d.overflow += 1,
                    Some(FailCause::RateLimit) => d.rate_limit += 1,
                    Some(FailCause::Deadline) => d.deadline += 1,
                    Some(FailCause::Breaker) => d.breaker += 1,
                    Some(FailCause::Chaos) | None => d.chaos += 1,
                }
                dropped_at = Some(
                    hops.iter()
                        .position(|h| h.node == *node)
                        .unwrap_or_else(|| hops.len().saturating_sub(1)),
                );
            }
        }
        // one request in `stride` is a journey; a dropped one is always worth drawing
        let sample = self.seen.is_multiple_of(self.stride) || dropped_at.is_some();
        if sample && self.journeys.len() < MAX_JOURNEYS && !hops.is_empty() {
            self.journeys.push(Journey {
                hops,
                dropped_at,
                total_ns: total,
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
        let mut nodes =
            std::mem::replace(&mut self.nodes, vec![NodeTick::default(); topo.nodes.len()]);
        for (i, n) in nodes.iter_mut().enumerate() {
            let (busy, queued) = loads.get(i).copied().unwrap_or((0, 0));
            n.busy_slots = busy;
            n.queue = queued;
            n.cpu_load = topo.nodes[i].cpu.map(|mc| {
                let cores = f64::from(mc) / 1000.0 * f64::from(topo.nodes[i].replicas.max(1));
                f64::from(busy) / cores
            });
        }
        let wires = std::mem::replace(&mut self.wires, vec![WireTick::default(); topo.edges.len()]);
        let hist = std::mem::take(&mut self.hist).sparse();
        let tick = Tick {
            t_ms,
            system_ms,
            requests: self.requests,
            failed: self.failed,
            nodes,
            wires,
            hist,
            journeys: std::mem::take(&mut self.journeys),
            status,
        };
        // aim for about MAX_JOURNEYS journeys a tick at the rate just seen
        self.stride = (u64::from(self.requests) / MAX_JOURNEYS as u64).max(1);
        self.requests = 0;
        self.failed = 0;
        tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
    use kaanal_core::replay::Engine;
    use kaanal_core::{ChainRequest, Config, EffectiveChaos, sample_plan};
    use std::sync::Arc;

    const STATUS: TickStatus = TickStatus {
        rate: 100.0,
        paused: false,
        overloaded: false,
        in_flight: 0,
    };

    /// Runs `n` requests through `yaml`'s gw -> app -> db chain at one instant and returns the
    /// topology and what finished.
    fn finished(yaml: &str, n: u64, spacing_ms: u64) -> (Topology, Vec<Finished>) {
        let t = Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap();
        let eps = compile(
            &t,
            &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
            EndpointsInput::default(),
        )
        .unwrap();
        let ep = eps.get("GET /x/{id}").unwrap();
        let mut e = Engine::live(&t);
        let mut out = Vec::new();
        for i in 0..n {
            let (plan, _) = sample_plan(
                &t,
                &EffectiveChaos::default(),
                &ChainRequest {
                    endpoint: ep,
                    at: Duration::ZERO,
                    n: i,
                    seed: 1,
                    pin_delay: Duration::ZERO,
                    req_bytes: 0,
                    resp_bytes: 0,
                },
            );
            e.submit(i * spacing_ms * 1_000_000, Arc::new(plan), i)
                .unwrap();
            out.extend(e.drain_finished());
        }
        e.advance_to(u64::MAX / 4).unwrap();
        out.extend(e.drain_finished());
        (t, out)
    }

    const CHAIN: &str = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 2ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n";

    /// Review finding 3: requests finished by an old, larger system must not panic an aggregator
    /// built for the new, smaller one (an Apply swaps the generation before the old feed stops).
    #[test]
    fn requests_from_a_different_topology_are_ignored_not_indexed_out_of_range() {
        let (_, fin) = finished(CHAIN, 10, 5);
        let small = Topology::build(
            &Config::from_yaml(
                "version: 1
nodes:
  gw: { type: gateway, proc: 1ms }
  app: { type: service, proc: 2ms }
edges:
  - { from: gw, to: app, net: 1ms }
",
            )
            .unwrap(),
        )
        .unwrap();
        let mut a = Aggregator::new(&small);
        for f in &fin {
            a.add(f);
        }
        let tick = a.take((250, 250), &[(0, 0), (0, 0)], &small, STATUS);
        assert_eq!(tick.nodes.len(), 2);
    }

    #[test]
    fn a_tick_counts_sum_to_the_requests_the_engine_finished() {
        let (t, fin) = finished(CHAIN, 40, 5);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f);
        }
        let tick = a.take((250, 250), &[(0, 0), (0, 0), (0, 0)], &t, STATUS);
        assert_eq!(tick.requests, 40);
        assert_eq!(tick.failed, 0);
        let hist: u32 = tick.hist.iter().map(|(_, c)| c).sum();
        assert_eq!(hist, 40, "every served request is in the histogram");
        // every request ran its own hop once at each node, and crossed each wire once
        assert!(
            tick.nodes.iter().all(|n| n.arrived == 40),
            "{:?}",
            tick.nodes
        );
        assert_eq!(
            tick.wires,
            vec![WireTick { carried: 40 }, WireTick { carried: 40 }]
        );
        // the next tick starts empty
        let next = a.take((500, 500), &[], &t, STATUS);
        assert_eq!((next.requests, next.hist.len()), (0, 0));
    }

    #[test]
    fn drops_are_counted_by_cause_at_the_node_that_dropped() {
        // one slot and no queue at the app: simultaneous requests overflow there
        let yaml = CHAIN.replace(
            "app: { type: service, proc: 2ms }",
            "app: { type: service, proc: 20ms, concurrency: { limit: 1, queue: 0 } }",
        );
        let (t, fin) = finished(&yaml, 10, 0);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        let app = t.id("app").unwrap();
        assert!(tick.failed > 0 && tick.requests == 10);
        assert_eq!(tick.nodes[app].dropped.overflow, tick.failed);
        assert_eq!(tick.nodes[app].dropped.rate_limit, 0);
        let served: u32 = tick.hist.iter().map(|(_, c)| c).sum();
        assert_eq!(
            served + tick.failed,
            10,
            "a dropped request is not in the latency histogram"
        );
        assert!(
            tick.journeys.iter().any(|j| j.dropped_at.is_some()),
            "a dropped request is drawn"
        );
    }

    #[test]
    fn journeys_are_capped_per_tick_and_carry_their_hops() {
        let (t, fin) = finished(CHAIN, 400, 1);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        assert!(
            tick.journeys.len() <= MAX_JOURNEYS,
            "{}",
            tick.journeys.len()
        );
        let j = &tick.journeys[0];
        assert!(j.hops.iter().any(|h| h.kind == "proc"));
        assert!(
            j.hops.iter().any(|h| h.kind == "net" && h.edge.is_some()),
            "{:?}",
            j.hops
        );
        assert!(
            j.hops.iter().any(|h| h.kind == "proc" && h.edge.is_some()),
            "an own hop entered over a wire names it: {:?}",
            j.hops
        );
        assert!(j.hops.windows(2).all(|w| w[0].start_ns <= w[1].start_ns));
        // the stride adapts: the next tick at the same rate samples about the cap, not every request
        for f in &fin {
            a.add(f);
        }
        let again = a.take((500, 500), &[(0, 0); 3], &t, STATUS);
        assert!(again.journeys.len() <= MAX_JOURNEYS);
    }

    #[test]
    fn queues_and_cpu_load_come_from_the_engine_snapshot() {
        let yaml = CHAIN.replace(
            "app: { type: service, proc: 2ms }",
            "app: { type: service, proc: 2ms, cpu: { cores: 2 }, replicas: 2 }",
        );
        let (t, _) = finished(&yaml, 1, 0);
        let mut a = Aggregator::new(&t);
        let app = t.id("app").unwrap();
        let mut loads = vec![(0, 0); 3];
        loads[app] = (8, 3);
        let tick = a.take((250, 250), &loads, &t, STATUS);
        assert_eq!((tick.nodes[app].busy_slots, tick.nodes[app].queue), (8, 3));
        // 8 running over 2 cores x 2 copies
        assert_eq!(tick.nodes[app].cpu_load, Some(2.0));
        assert_eq!(tick.nodes[t.id("gw").unwrap()].cpu_load, None);
    }
}
