//! **Private.** The event loop behind [`crate::run`]: a binary-heap event queue on a
//! virtual clock in integer nanoseconds. Every request's `Plan` comes from `execute()`;
//! this module walks it with tokens (fork/join), adds FIFO queueing at nodes with
//! `concurrency`, and ends a request at its first failure (D17).

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosState, Dist, Endpoint, Hop, HopKind, NodeId,
    PlanItem, Topology, execute, stable_hash,
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
    /// Token `.1` of the request in table slot `.0` finished its hop.
    HopDone(usize, usize),
}

/// **Private.** `Req::starts` value of a hop that never started.
const NOT_STARTED: u64 = u64::MAX;

/// **Private.** A walker over one plan range: the whole plan, or one `parallel` branch.
struct Token {
    /// Next plan index.
    at: usize,
    /// End of the range (exclusive).
    end: usize,
    /// The join it reports to at its end; `None` for the root token.
    join: Option<usize>,
    /// The node queue it waits in, and since when (ns).
    queued_at: Option<(NodeId, u64)>,
}

/// **Private.** A `parallel` block waiting for its branches.
struct Join {
    /// Branch tokens not at their end yet.
    left: usize,
    /// The token that forked; it resumes after the block.
    parent: usize,
}

/// **Private.** One request in flight.
struct Req {
    /// Index into `SimConfig::streams`.
    stream: usize,
    /// The stream's request counter.
    n: u64,
    /// Arrival order, for the report.
    order: u64,
    /// Arrival time (ns).
    arrival: u64,
    /// `execute()`'s result: the plan to replay and how the chain ended.
    outcome: ChainOutcome,
    /// Actual start (ns) per plan index, `NOT_STARTED` otherwise.
    starts: Vec<u64>,
    /// Queue wait (ns) per plan index.
    waits: Vec<u64>,
    tokens: Vec<Token>,
    joins: Vec<Join>,
    /// Tokens holding a hop or waiting in a queue.
    busy: usize,
    /// The winning failure: (instant ns, plan index), D17.
    fail: Option<(u64, usize)>,
    /// When the client got its answer (ns).
    done: Option<u64>,
}

/// **Private.** Queueing state of one node. `slots == None`: unlimited, never queues.
struct NodeQ {
    /// Capacity from `Node::concurrency`.
    slots: Option<u32>,
    /// Slots in use.
    busy: u32,
    /// Tokens waiting for a slot (request slot, token), oldest first.
    queue: VecDeque<(usize, usize)>,
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
    /// The compiled endpoint of each stream.
    endpoints: &'a [Arc<Endpoint>],
    chaos: &'a ChaosState,
    cfg: &'a SimConfig,
    /// Current virtual time (ns).
    now: u64,
    /// Tie-breaker: events at the same time pop in the order they were pushed.
    seq: u64,
    heap: BinaryHeap<Reverse<(u64, u64, Event)>>,
    /// Requests in flight; a finished request's slot is reused (R6.13 b).
    reqs: Vec<Option<Req>>,
    /// Free slots of `reqs`.
    free: Vec<usize>,
    /// Next arrival order number.
    order: u64,
    nodes: Vec<NodeQ>,
    arrivals: Vec<Arrivals>,
    /// Reported records with their arrival order.
    records: Vec<(u64, RequestRecord)>,
    /// Own hops per node over reported requests.
    completed: Vec<u64>,
    /// Summed queue waits (ns) per node over reported requests.
    wait: Vec<u128>,
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
    /// Validation bounds hop records but not arrivals × service time: 600k requests of 9 h
    /// through one slot passes `validate` and overflows here.
    fn after(&self, d: u64) -> u64 {
        self.now.checked_add(d).unwrap_or_else(|| {
            self.overflow.set(true);
            u64::MAX
        })
    }
}

/// **Private.** Runs a validated config until the event queue drains. The second value is
/// the most requests held at once (the request table's length), for the release test.
pub(crate) fn simulate(
    topo: &Topology,
    endpoints: &[Arc<Endpoint>],
    chaos: &ChaosState,
    cfg: &SimConfig,
) -> Result<(SimReport, usize), String> {
    let len = topo.nodes.len();
    let mut sim = Sim {
        topo,
        endpoints,
        chaos,
        cfg,
        now: 0,
        seq: 0,
        heap: BinaryHeap::new(),
        reqs: Vec::new(),
        free: Vec::new(),
        order: 0,
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
        records: Vec::new(),
        completed: vec![0; len],
        wait: vec![0; len],
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
            Event::HopDone(r, k) => sim.hop_done(r, k),
        }
        if sim.overflow.get() {
            return Err("simulated clock overflow (about 584 years): the workload queues for too long, lower the rate or the service times".into());
        }
    }
    let peak = sim.reqs.len();
    Ok((sim.report(), peak))
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

    /// **Private.** The live request in slot `r`.
    fn req(&mut self, r: usize) -> &mut Req {
        self.reqs[r]
            .as_mut()
            .expect("events only name live requests")
    }

    /// **Private.** A request of stream `s` arrives: sample its plan with `execute()` (the
    /// same call live mode makes) and start its root token.
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
        let len = outcome.plan.items.len();
        let req = Req {
            stream: s,
            n,
            order: self.order,
            arrival: self.now,
            starts: vec![NOT_STARTED; len],
            waits: vec![0; len],
            tokens: vec![Token {
                at: 0,
                end: len,
                join: None,
                queued_at: None,
            }],
            joins: Vec::new(),
            busy: 0,
            fail: None,
            done: None,
            outcome,
        };
        self.order += 1;
        let r = if let Some(r) = self.free.pop() {
            self.reqs[r] = Some(req);
            r
        } else {
            self.reqs.push(Some(req));
            self.reqs.len() - 1
        };
        self.schedule_arrival(s);
        self.advance(r, 0);
        self.finish_if_done(r);
    }

    /// **Private.** Runs token `k` from its position until it starts a hop, parks at a fork,
    /// fails, or ends. A request that has failed starts nothing new; only a `Fail` reached
    /// at the same instant with a lower plan index can still replace the winner (D17).
    fn advance(&mut self, r: usize, mut k: usize) {
        loop {
            let now = self.now;
            let req = self.req(r);
            let (at, end) = (req.tokens[k].at, req.tokens[k].end);
            if at == end {
                let Some(j) = req.tokens[k].join else {
                    if req.fail.is_none() {
                        req.done = Some(now);
                    }
                    return;
                };
                req.joins[j].left -= 1;
                if req.joins[j].left > 0 || req.fail.is_some() {
                    return;
                }
                k = req.joins[j].parent; // the last branch resumes the block's parent
                continue;
            }
            let item = req.outcome.plan.items[at].clone();
            if let Some((t, i)) = req.fail
                && !(matches!(item, PlanItem::Fail { .. }) && t == now && at < i)
            {
                return;
            }
            match item {
                PlanItem::Hop(h) => {
                    self.start_hop(r, k, &h);
                    return;
                }
                PlanItem::Fail { .. } => {
                    self.fail(r, at);
                    return;
                }
                PlanItem::Fork { branches, size } => {
                    let j = req.joins.len();
                    req.joins.push(Join {
                        left: branches,
                        parent: k,
                    });
                    req.tokens[k].at = at + 1 + size;
                    let first = req.tokens.len();
                    let mut b = at + 1;
                    while b < at + 1 + size {
                        let PlanItem::Branch { size: bs } = req.outcome.plan.items[b] else {
                            break;
                        };
                        req.tokens.push(Token {
                            at: b + 1,
                            end: b + 1 + bs,
                            join: Some(j),
                            queued_at: None,
                        });
                        b += 1 + bs;
                    }
                    let last = req.tokens.len();
                    // Branch order: a contested slot goes to the lower branch first.
                    for c in first..last {
                        self.advance(r, c);
                    }
                    return;
                }
                PlanItem::Branch { .. } => unreachable!("tokens start after their Branch marker"),
            }
        }
    }

    /// **Private.** Starts token `k`'s hop now: a plain delay, a free slot, or the node's queue.
    fn start_hop(&mut self, r: usize, k: usize, h: &Hop) {
        let (now, warmup, end) = (self.now, self.warmup, self.end);
        let at = self.req(r).tokens[k].at;
        self.req(r).busy += 1;
        if h.kind != HopKind::Net {
            let q = &mut self.nodes[h.node];
            if let Some(slots) = q.slots {
                if q.busy == slots {
                    q.queue.push_back((r, k));
                    if now >= warmup {
                        q.max_queue = q.max_queue.max(q.queue.len());
                    }
                    self.req(r).tokens[k].queued_at = Some((h.node, now));
                    return;
                }
                q.mark(now, warmup, end);
                q.busy += 1;
            }
        }
        self.req(r).starts[at] = now;
        let done = self.after(nanos(h.dur));
        self.push(done, Event::HopDone(r, k));
    }

    /// **Private.** Token `k`'s hop ended: hand its slot to the head of the queue (or free
    /// it), then move the token on.
    fn hop_done(&mut self, r: usize, k: usize) {
        let now = self.now;
        let at = self.req(r).tokens[k].at;
        let PlanItem::Hop(hop) = &self.req(r).outcome.plan.items[at] else {
            unreachable!("HopDone names a hop")
        };
        let (node, kind) = (hop.node, hop.kind);
        if kind != HopKind::Net && self.nodes[node].slots.is_some() {
            // FIFO hand-over at this instant: the slot never looks free to a later arrival.
            if let Some((qr, qk)) = self.nodes[node].queue.pop_front() {
                let waiter = self.req(qr);
                let i = waiter.tokens[qk].at;
                let (_, since) = waiter.tokens[qk]
                    .queued_at
                    .take()
                    .expect("a token in a node queue has queued_at set");
                waiter.waits[i] = now - since;
                waiter.starts[i] = now;
                let PlanItem::Hop(qh) = &waiter.outcome.plan.items[i] else {
                    unreachable!("queued at a hop")
                };
                let dur = nanos(qh.dur);
                let done = self.after(dur);
                self.push(done, Event::HopDone(qr, qk));
            } else {
                let (warmup, end) = (self.warmup, self.end);
                let slotq = &mut self.nodes[node];
                slotq.mark(now, warmup, end);
                slotq.busy -= 1;
            }
        }
        let req = self.req(r);
        req.busy -= 1;
        req.tokens[k].at += 1;
        self.advance(r, k);
        self.finish_if_done(r);
    }

    /// **Private.** A token reached the `Fail` at plan index `idx` (E14: at the failing hop's
    /// start). It wins (first, or a lower index at the same instant, D17); siblings still
    /// queued leave their queues; running ones keep their slot until their hop ends.
    fn fail(&mut self, r: usize, idx: usize) {
        let now = self.now;
        let req = self.req(r);
        req.fail = Some((now, idx));
        req.done = Some(now);
        let queued: Vec<(usize, NodeId)> = req
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(k, t)| t.queued_at.map(|(n, _)| (k, n)))
            .collect();
        for (k, node) in queued {
            // ponytail: linear scan of that node's queue; keep a per-token handle if long
            // queues of failing requests ever show up in a profile.
            self.nodes[node].queue.retain(|&e| e != (r, k));
            let req = self.req(r);
            req.tokens[k].queued_at = None;
            req.busy -= 1;
        }
    }

    /// **Private.** Once a request has its answer and holds no slot, record it (if it
    /// arrived at or after warmup) and free its table slot (R6.13 b).
    fn finish_if_done(&mut self, r: usize) {
        let req = self.req(r);
        if req.busy > 0 || req.done.is_none() {
            return;
        }
        let req = self.reqs[r].take().expect("checked above");
        self.free.push(r);
        if req.arrival >= self.warmup {
            self.record(&req);
        }
    }

    /// **Private.** The client's view of a finished request (see [`RequestRecord::hops`]):
    /// hops that started, cut at the failure when their branch was cancelled.
    fn record(&mut self, req: &Req) {
        let plan = &req.outcome.plan;
        let (cut, fail_at) = match req.fail {
            Some((t, i)) => (plan.cancelled_by(i), t),
            None => (Vec::new(), u64::MAX),
        };
        let mut hops = Vec::new();
        for (i, item) in plan.items.iter().enumerate() {
            let PlanItem::Hop(h) = item else { continue };
            let s = req.starts[i];
            if s == NOT_STARTED {
                continue;
            }
            // Every hop that started held its slot, so it counts for the node even when
            // the record drops it (a cancelled hop that began at the failure instant).
            if h.kind != HopKind::Net {
                self.completed[h.node] += 1;
                self.wait[h.node] += u128::from(req.waits[i]);
            }
            let mut h = h.clone();
            h.start = Duration::from_nanos(s - req.arrival);
            if cut.iter().any(|c| c.contains(&i)) {
                if s >= fail_at {
                    continue;
                }
                h.dur = h.dur.min(Duration::from_nanos(fail_at - s));
            }
            hops.push((h, Duration::from_nanos(req.waits[i])));
        }
        let result = match req.fail.map(|(_, i)| &plan.items[i]) {
            Some(PlanItem::Fail { node, status, .. }) => ChainResult::Err {
                node: *node,
                status: *status,
            },
            _ => ChainResult::Ok,
        };
        let done = req.done.expect("recorded requests are done");
        self.records.push((
            req.order,
            RequestRecord {
                stream: req.stream,
                n: req.n,
                arrival: Duration::from_nanos(req.arrival),
                latency: Duration::from_nanos(done - req.arrival),
                result,
                hops,
            },
        ));
    }

    /// **Private.** Builds the report from requests arriving at or after warmup.
    #[expect(
        clippy::cast_precision_loss,
        reason = "utilization is a ratio of nanosecond totals; f64 precision is ample"
    )]
    fn report(mut self) -> SimReport {
        self.records.sort_by_key(|(order, _)| *order);
        let requests = self.records.into_iter().map(|(_, r)| r).collect();
        let window = (self.end - self.warmup) as f64;
        let nodes = (0..self.nodes.len())
            .map(|i| NodeStats {
                completed: self.completed[i],
                mean_wait: match u128::from(self.completed[i]) {
                    0 => Duration::ZERO,
                    c => Duration::from_nanos(u64::try_from(self.wait[i] / c).unwrap_or(u64::MAX)),
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

#[cfg(test)]
mod tests {
    use super::simulate;
    use crate::{SimConfig, Stream};
    use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
    use kaanal_core::{ChaosState, Config, Topology};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn pre_warmup_requests_release_their_plans() {
        // R6.13 b: ~10 000 requests run, but at ρ = 0.5 only a handful are ever in flight,
        // so the request table stays small instead of holding every request.
        let t = Topology::build(
            &Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n").unwrap(),
        )
        .unwrap();
        let eps = compile(
            &t,
            &[RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())],
            EndpointsInput::default(),
        )
        .unwrap();
        let cfg = SimConfig {
            streams: vec![Stream {
                endpoint_key: "GET /x/{id}".into(),
                rate_per_s: 50.0,
            }],
            duration: Duration::from_secs(200),
            warmup: Duration::from_secs(190),
            seed: 7,
        };
        let one = [Arc::clone(eps.get("GET /x/{id}").unwrap())];
        let (rep, peak) = simulate(&t, &one, &ChaosState::default(), &cfg).unwrap();
        assert!(rep.requests.len() > 300, "{}", rep.requests.len());
        assert!(peak < 64, "requests held at once: {peak}");
    }
}
