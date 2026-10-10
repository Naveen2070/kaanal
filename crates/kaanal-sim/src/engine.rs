//! **Private.** The event loop behind [`crate::run`]: a binary-heap event queue on a
//! virtual clock in integer nanoseconds. Service times come from `execute()`; this
//! module only adds FIFO queueing at nodes with `concurrency`.

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::{
    ChainOutcome, ChainRequest, ChaosState, Dist, Endpoint, HopKind, Topology, execute, stable_hash,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::{NodeStats, RequestRecord, SimConfig, SimReport};

/// **Private.** A scheduled event. The heap orders by `(time, seq)`; `seq` is unique, so
/// this derive only satisfies the type and never decides an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Event {
    /// The next arrival of stream `.0`.
    Arrival(usize),
    /// Request `.0` finished its current hop.
    HopDone(usize),
}

/// **Private.** One request in flight (or finished).
struct Req {
    /// Index into `SimConfig::streams`.
    stream: usize,
    /// The stream's request counter.
    n: u64,
    /// Arrival time (ns).
    arrival: u64,
    /// `execute()`'s result: the hops to replay and how the chain ended.
    outcome: ChainOutcome,
    /// Queue wait before each hop (ns), parallel to `outcome.hops`.
    waits: Vec<u64>,
    /// Index of the hop in progress; `hops.len()` once done.
    next: usize,
    /// When the request joined its current node's queue (ns).
    queued_at: u64,
    /// Completion time (ns), set when the last hop ends.
    done: Option<u64>,
}

/// **Private.** Queueing state of one node. `slots == None`: unlimited, never queues.
struct NodeQ {
    /// Capacity from `Node::concurrency`.
    slots: Option<u32>,
    /// Slots in use.
    busy: u32,
    /// Requests waiting for a slot, oldest first.
    queue: VecDeque<usize>,
    /// Longest queue seen when a request joined at or after warmup. It includes requests that
    /// arrived before warmup and are still waiting; a queue that only drains after warmup
    /// is not counted.
    max_queue: usize,
    /// Busy slot-nanoseconds inside the reporting window `[warmup, end]`, up to `since`.
    area: u128,
    /// Last time `busy` changed (ns); `area` is integrated up to here.
    since: u64,
}

impl NodeQ {
    /// **Private.** Adds `busy × (time since the last change, clipped to the window)` to `area`.
    /// Call just before `busy` changes, so utilization counts only busy time in the window.
    fn mark(&mut self, now: u64, warmup: u64, end: u64) {
        let (from, to) = (self.since.max(warmup), now.min(end));
        self.area += u128::from(self.busy) * u128::from(to.saturating_sub(from));
        self.since = now;
    }
}

/// **Private.** One stream's arrival RNG, gap distribution and request counter.
struct Arrivals {
    /// Seeded from `(seed, "arrivals", endpoint_key)`.
    rng: ChaCha8Rng,
    /// Exponential with mean `1 / rate_per_s`.
    gap: Dist,
    /// Next request counter.
    n: u64,
}

/// **Private.** The whole simulation state.
struct Sim<'a> {
    topo: &'a Topology,
    /// The compiled endpoint of each stream (A2 shim: the owner's default chain).
    endpoints: &'a [Arc<Endpoint>],
    chaos: &'a ChaosState,
    cfg: &'a SimConfig,
    /// Current virtual time (ns).
    now: u64,
    /// Tie-breaker: events at the same time pop in the order they were pushed.
    seq: u64,
    heap: BinaryHeap<Reverse<(u64, u64, Event)>>,
    reqs: Vec<Req>,
    nodes: Vec<NodeQ>,
    arrivals: Vec<Arrivals>,
    /// Reporting starts here (ns).
    warmup: u64,
    /// Arrivals stop here (ns).
    end: u64,
    /// Set when an event time would pass `u64::MAX` ns; the run is then abandoned.
    overflow: Cell<bool>,
}

/// **Private.** Duration → virtual nanoseconds. Validation (duration ≤ 365 days, every
/// service/net/chaos time ≤ 10 h) keeps each value far below `u64::MAX`.
fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("validated durations fit in u64 nanoseconds")
}

impl Sim<'_> {
    /// **Private.** `now + d`. On overflow it flags the run as failed (and returns `u64::MAX`)
    /// instead of letting the clock pin, which would silently collapse `(time, seq)` ordering.
    /// Validation bounds arrivals but not arrivals × service time: 2M requests of 9 h through
    /// one slot passes `validate` and overflows here.
    fn after(&self, d: u64) -> u64 {
        self.now.checked_add(d).unwrap_or_else(|| {
            self.overflow.set(true);
            u64::MAX
        })
    }
}

/// **Private.** Runs a validated config until the event queue drains.
pub(crate) fn simulate(
    topo: &Topology,
    endpoints: &[Arc<Endpoint>],
    chaos: &ChaosState,
    cfg: &SimConfig,
) -> Result<SimReport, String> {
    let mut sim = Sim {
        topo,
        endpoints,
        chaos,
        cfg,
        now: 0,
        seq: 0,
        heap: BinaryHeap::new(),
        reqs: Vec::new(),
        nodes: topo
            .nodes
            .iter()
            .map(|n| NodeQ {
                slots: n.concurrency.map(NonZeroU32::get),
                busy: 0,
                queue: VecDeque::new(),
                max_queue: 0,
                area: 0,
                since: 0,
            })
            .collect(),
        arrivals: cfg
            .streams
            .iter()
            .map(|s| Arrivals {
                rng: ChaCha8Rng::seed_from_u64(stable_hash(&[
                    &cfg.seed.to_le_bytes(),
                    b"arrivals",
                    s.endpoint_key.as_bytes(),
                ])),
                gap: Dist::Exp {
                    mean: Duration::from_secs_f64(1.0 / s.rate_per_s),
                },
                n: 0,
            })
            .collect(),
        warmup: nanos(cfg.warmup),
        end: nanos(cfg.duration),
        overflow: Cell::new(false),
    };
    for s in 0..cfg.streams.len() {
        sim.schedule_arrival(s);
    }
    while let Some(Reverse((at, _, event))) = sim.heap.pop() {
        sim.now = at;
        match event {
            Event::Arrival(s) => sim.arrive(s),
            Event::HopDone(r) => sim.hop_done(r),
        }
        if sim.overflow.get() {
            return Err("simulated clock overflow (about 584 years): the workload queues for too long, lower the rate or the service times".into());
        }
    }
    Ok(sim.report())
}

impl Sim<'_> {
    /// **Private.** Schedules `event` at `at`, after everything already scheduled for `at`.
    fn push(&mut self, at: u64, event: Event) {
        self.seq += 1;
        self.heap.push(Reverse((at, self.seq, event)));
    }

    /// **Private.** Schedules stream `s`'s next arrival, unless it falls at or after `end`.
    fn schedule_arrival(&mut self, s: usize) {
        let a = &mut self.arrivals[s];
        let gap = nanos(a.gap.sample(&mut a.rng));
        let at = self.after(gap);
        if at < self.end {
            self.push(at, Event::Arrival(s));
        }
    }

    /// **Private.** A request of stream `s` arrives: compute its chain with `execute()`
    /// (the same call live mode makes), then start its first hop.
    fn arrive(&mut self, s: usize) {
        let n = self.arrivals[s].n;
        self.arrivals[s].n += 1;
        let outcome = execute(
            self.topo,
            self.chaos,
            &ChainRequest {
                endpoint: &self.endpoints[s],
                n,
                seed: self.cfg.seed,
            },
        );
        let r = self.reqs.len();
        self.reqs.push(Req {
            stream: s,
            n,
            arrival: self.now,
            waits: vec![0; outcome.hops.len()],
            outcome,
            next: 0,
            queued_at: 0,
            done: None,
        });
        self.schedule_arrival(s);
        self.start_hop(r);
    }

    /// **Private.** Starts request `r`'s next hop now: a plain delay, a free slot, or the
    /// node's queue. With no hops left (or none at all, e.g. a down gateway), it completes.
    fn start_hop(&mut self, r: usize) {
        let req = &self.reqs[r];
        let Some(hop) = req.outcome.hops.get(req.next) else {
            self.reqs[r].done = Some(self.now);
            return;
        };
        let (node, dur) = (hop.node, nanos(hop.dur));
        if hop.kind != HopKind::Net {
            let q = &mut self.nodes[node];
            if let Some(slots) = q.slots {
                if q.busy == slots {
                    q.queue.push_back(r);
                    if self.now >= self.warmup {
                        q.max_queue = q.max_queue.max(q.queue.len());
                    }
                    self.reqs[r].queued_at = self.now;
                    return;
                }
                q.mark(self.now, self.warmup, self.end);
                q.busy += 1;
            }
        }
        self.push(self.after(dur), Event::HopDone(r));
    }

    /// **Private.** Request `r`'s hop ended: hand its slot to the head of the queue (or
    /// free it), then start `r`'s next hop.
    fn hop_done(&mut self, r: usize) {
        let hop = &self.reqs[r].outcome.hops[self.reqs[r].next];
        let node = hop.node;
        if hop.kind != HopKind::Net && self.nodes[node].slots.is_some() {
            // FIFO hand-over at this instant: the slot never looks free to a later arrival.
            if let Some(q) = self.nodes[node].queue.pop_front() {
                let i = self.reqs[q].next;
                self.reqs[q].waits[i] = self.now - self.reqs[q].queued_at;
                let dur = nanos(self.reqs[q].outcome.hops[i].dur);
                self.push(self.after(dur), Event::HopDone(q));
            } else {
                let (now, warmup, end) = (self.now, self.warmup, self.end);
                let q = &mut self.nodes[node];
                q.mark(now, warmup, end);
                q.busy -= 1;
            }
        }
        self.reqs[r].next += 1;
        self.start_hop(r);
    }

    /// **Private.** Builds the report from requests arriving at or after warmup.
    #[expect(
        clippy::cast_precision_loss,
        reason = "utilization is a ratio of nanosecond totals; f64 precision is ample"
    )]
    fn report(self) -> SimReport {
        let len = self.topo.nodes.len();
        let (mut completed, mut wait) = (vec![0u64; len], vec![0u128; len]);
        let mut requests = Vec::new();
        for r in self.reqs {
            if r.arrival < self.warmup {
                continue;
            }
            let done = r.done.expect("the heap drains, so every request completes");
            for (h, w) in r.outcome.hops.iter().zip(&r.waits) {
                if h.kind != HopKind::Net {
                    completed[h.node] += 1;
                    wait[h.node] += u128::from(*w);
                }
            }
            requests.push(RequestRecord {
                stream: r.stream,
                n: r.n,
                arrival: Duration::from_nanos(r.arrival),
                latency: Duration::from_nanos(done - r.arrival),
                result: r.outcome.result.clone(),
                hops: r
                    .outcome
                    .hops
                    .into_iter()
                    .zip(r.waits)
                    .map(|(h, w)| (h, Duration::from_nanos(w)))
                    .collect(),
            });
        }
        let window = (self.end - self.warmup) as f64;
        let nodes = (0..len)
            .map(|i| NodeStats {
                completed: completed[i],
                mean_wait: match u128::from(completed[i]) {
                    0 => Duration::ZERO,
                    c => Duration::from_nanos(u64::try_from(wait[i] / c).unwrap_or(u64::MAX)),
                },
                utilization: self.nodes[i]
                    .slots
                    .map_or(0.0, |c| self.nodes[i].area as f64 / (f64::from(c) * window)),
                max_queue: self.nodes[i].max_queue,
            })
            .collect();
        SimReport { requests, nodes }
    }
}
