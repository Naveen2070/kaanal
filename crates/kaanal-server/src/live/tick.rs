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
    wires: Vec<WireTick>,
    hist: LatencyHist,
    requests: u32,
    failed: u32,
}

/// A request's journey, built once: its hops, where it was dropped, its total time.
type Built = (Vec<JourneyHop>, Option<usize>, u64);

impl Counts {
    fn new(nodes: usize, wires: usize) -> Counts {
        Counts {
            nodes: vec![NodeTick::default(); nodes],
            wires: vec![WireTick::default(); wires],
            hist: LatencyHist::new(),
            requests: 0,
            failed: 0,
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
        let mut hops = Vec::new();
        for h in &f.hops {
            let kind = h.hop.kind;
            let mut edge = None;
            if is_own(kind) {
                self.nodes[h.hop.node].arrived += 1;
                if let Some(p) = prev_own.filter(|p| *p != h.hop.node)
                    && let Some(e) = edge_of[p][h.hop.node]
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
                edge = prev_own.and_then(|p| edge_of[p][h.hop.node]);
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
                    Some(FailCause::Oom) => d.oom += 1,
                    Some(FailCause::Chaos) | None => d.chaos += 1,
                }
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
}

impl Aggregator {
    pub(crate) fn new(topo: &Topology) -> Aggregator {
        let n = topo.nodes.len();
        let mut edge_of = vec![vec![None; n]; n];
        for (i, e) in topo.edges.iter().enumerate() {
            edge_of[e.from][e.to] = Some(i);
        }
        Aggregator {
            total: Counts::new(n, topo.edges.len()),
            real: Counts::new(n, topo.edges.len()),
            edge_of,
            journeys: Vec::new(),
            real_journeys: 0,
            seen: 0,
            stride: 1,
            cpu_use: Vec::new(),
        }
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
        let mut total = std::mem::replace(&mut self.total, Counts::new(n, w));
        let real = std::mem::replace(&mut self.real, Counts::new(n, w));
        for (i, node) in total.nodes.iter_mut().enumerate() {
            let (busy, queued) = loads.get(i).copied().unwrap_or((0, 0));
            node.busy_slots = busy;
            node.queue = queued;
            node.cpu_utilization = self.cpu_use.get(i).copied().flatten();
            node.cpu_load = topo.nodes[i].cpu.map(|mc| {
                let cores = f64::from(mc) / 1000.0 * f64::from(topo.nodes[i].replicas.max(1));
                f64::from(busy) / cores
            });
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
            a.add(f, false);
        }
        let tick = a.take((250, 250), &[(0, 0), (0, 0)], &small, STATUS);
        assert_eq!(tick.nodes.len(), 2);
    }

    /// Deferred from 8A: after an Apply the old engine still finishes requests; with a new system
    /// of the same size they would be summed into nodes they never touched.
    #[test]
    fn what_the_old_engine_finishes_after_an_apply_is_dropped_not_counted() {
        let (t, fin) = finished(CHAIN, 6, 5);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for f in &fin {
            tx.send(f.clone()).unwrap();
        }
        let mut a = Aggregator::new(&t);
        crate::live::drain_finished(&mut a, &mut rx, false);
        assert!(rx.try_recv().is_err(), "read and dropped");
        let none = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        assert_eq!(none.requests, 0);
        for f in &fin {
            tx.send(f.clone()).unwrap();
        }
        crate::live::drain_finished(&mut a, &mut rx, true);
        let some = a.take((500, 500), &[(0, 0); 3], &t, STATUS);
        assert_eq!(some.requests, 6);
    }

    #[test]
    fn measured_cpu_use_is_the_gained_busy_area_over_cores_copies_and_span() {
        let t = Topology::build(&Config::from_yaml(CHAIN).unwrap()).unwrap();
        let t2 = Topology::build(
            &Config::from_yaml(&CHAIN.replace(
                "app: { type: service, proc: 2ms }",
                "app: { type: service, proc: 2ms, cpu: { cores: 2 }, replicas: 2 }",
            ))
            .unwrap(),
        )
        .unwrap();
        let app = t2.id("app").unwrap();
        // 4 cores in all, a quarter second: 1 core-second of busy area is 25% use
        let mut after = vec![0u128; 3];
        after[app] = 1_000_000_000_000; // 1 core-second in millicore-ns
        let u = cpu_use(&t2, &[0, 0, 0], &after, 1_000_000_000);
        assert!((u[app].unwrap() - 0.25).abs() < 1e-12);
        assert_eq!(u[0], None, "no cpu, no figure");
        // more than the cores could do in the span reads as 1, a span of zero as nothing
        after[app] = u128::MAX / 4;
        assert_eq!(cpu_use(&t2, &[0; 3], &after, 1_000_000_000)[app], Some(1.0));
        assert_eq!(cpu_use(&t2, &[0; 3], &after, 0)[app], None);
        assert!(cpu_use(&t, &[0; 3], &[0; 3], 1)[1].is_none());
    }

    #[test]
    fn a_tick_counts_sum_to_the_requests_the_engine_finished() {
        let (t, fin) = finished(CHAIN, 40, 5);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f, false);
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
            a.add(f, false);
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
            a.add(f, false);
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
            a.add(f, false);
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

    #[test]
    fn real_requests_are_in_the_total_and_in_real() {
        let (t, fin) = finished(CHAIN, 8, 5);
        let mut a = Aggregator::new(&t);
        for (i, f) in fin.iter().enumerate() {
            a.add(f, i < 3);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        assert_eq!(tick.requests, 8);
        assert_eq!(tick.real.as_ref().unwrap().requests, 3);
    }

    #[test]
    fn real_and_generated_add_up_to_the_same_total_node_by_node_and_wire_by_wire() {
        let (t, fin) = finished(CHAIN, 40, 5);
        let mut all = Aggregator::new(&t);
        for f in &fin {
            all.add(f, false);
        }
        let all = all.take((250, 250), &[(0, 0); 3], &t, STATUS);
        let mut mixed = Aggregator::new(&t);
        for (i, f) in fin.iter().enumerate() {
            mixed.add(f, i % 4 == 0);
        }
        let m = mixed.take((250, 250), &[(0, 0); 3], &t, STATUS);
        let arrived = |t: &Tick| t.nodes.iter().map(|n| n.arrived).collect::<Vec<_>>();
        assert_eq!(arrived(&m), arrived(&all), "the total ignores the source");
        assert_eq!(m.wires, all.wires);
        let real = m.real.expect("10 real requests");
        assert_eq!(real.requests, 10);
        // each request ran one own hop at each of the 3 nodes and crossed both wires once
        assert!(
            real.nodes.iter().all(|n| n.arrived == 10),
            "{:?}",
            real.nodes
        );
        assert_eq!(real.wires, vec![WireTick { carried: 10 }; 2]);
        let served: u32 = real.hist.iter().map(|(_, c)| c).sum();
        assert_eq!(served, 10, "a served real request is in the real histogram");
    }

    #[test]
    fn a_dropped_real_request_is_counted_by_cause_in_real_too() {
        let yaml = CHAIN.replace(
            "app: { type: service, proc: 2ms }",
            "app: { type: service, proc: 20ms, concurrency: { limit: 1, queue: 0 } }",
        );
        let (t, fin) = finished(&yaml, 10, 0);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f, true);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        let real = tick.real.unwrap();
        let app = t.id("app").unwrap();
        assert!(real.failed > 0);
        assert_eq!(real.nodes[app].dropped.overflow, real.failed);
        assert_eq!((real.requests, real.failed), (tick.requests, tick.failed));
    }

    #[test]
    fn with_no_real_request_real_is_none_and_the_json_has_no_real_key() {
        let (t, fin) = finished(CHAIN, 5, 5);
        let mut a = Aggregator::new(&t);
        for f in &fin {
            a.add(f, false);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        assert!(tick.real.is_none());
        let v = serde_json::to_value(&tick).unwrap();
        assert!(v.get("real").is_none());
        assert!(
            v["journeys"]
                .as_array()
                .unwrap()
                .iter()
                .all(|j| j.get("real").is_none()),
            "a generated journey has no real flag"
        );
    }

    #[test]
    fn real_journeys_are_flagged_and_capped_apart_from_generated() {
        let (t, fin) = finished(CHAIN, 1000, 1);
        let mut a = Aggregator::new(&t);
        for (i, f) in fin.iter().enumerate() {
            a.add(f, i < 30);
        }
        let tick = a.take((250, 250), &[(0, 0); 3], &t, STATUS);
        let real = tick.journeys.iter().filter(|j| j.real).count();
        let generated = tick.journeys.len() - real;
        assert_eq!(real, MAX_REAL_JOURNEYS);
        assert!(generated <= MAX_JOURNEYS && generated > 0, "{generated}");
    }

    #[test]
    fn a_real_request_of_another_topology_is_ignored_in_both_sets() {
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
            a.add(f, true);
        }
        let tick = a.take((250, 250), &[(0, 0); 2], &small, STATUS);
        assert_eq!(tick.requests, 0);
        assert!(tick.real.is_none());
    }
}
