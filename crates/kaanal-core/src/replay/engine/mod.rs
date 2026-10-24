//! The step-driven replay engine: walks sampled [`Plan`]s against per-node [`Gate`]s on a
//! caller-supplied clock in integer nanoseconds. See `docs/source/files/kaanal-core/replay.md`.
//!
//! File size (R3.8): the data types are in `state.rs`; `impl Engine` stays in this one file (about 650
//! lines) on purpose. It is one type's hot-path impl whose methods call each other, and keeping
//! it in one module keeps the optimizer's view of it exactly as before the split.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use super::bucket::Bucket;
use super::gate::{Admit, Gate};
use crate::chaos::FailKind;
use crate::exec::{ChainResult, Hop, HopKind, Plan, PlanItem};
use crate::topology::{NodeId, Topology};
mod state;

use state::{
    DEADLINE, Failure, Join, NOT_STARTED, NodeState, Req, Scratch, SpanState, Token, live, nanos,
};
pub use state::{EngineError, FailCause, Finished, FinishedHop, NodeCounters};

/// The replay engine. Drive it with [`Engine::submit`] and [`Engine::step`] /
/// [`Engine::advance_to`]; read answers with [`Engine::drain_finished`].
pub struct Engine {
    nodes: Vec<NodeState>,
    counters: Vec<NodeCounters>,
    /// Hop-end events ordered by `(time, seq)`: (time, seq, request slot, token).
    heap: BinaryHeap<Reverse<(u64, u64, usize, usize)>>,
    seq: u64,
    now: u64,
    reqs: Vec<Option<Req>>,
    free: Vec<usize>,
    spare: Vec<Scratch>,
    /// Requests whose answer is decided but not emitted yet (Q3).
    answering: Vec<usize>,
    finished: Vec<Finished>,
    order: u64,
    warmup: u64,
    end: u64,
    overflow: bool,
}

impl Engine {
    /// An engine with every gate free. The window is `[0, u64::MAX]` until [`Engine::set_window`].
    pub fn new(topo: &Topology) -> Engine {
        Engine {
            nodes: topo
                .nodes
                .iter()
                .map(|n| NodeState {
                    gate: n.admission.as_ref().map(Gate::new),
                    bucket: n.rate_limit.as_ref().map(Bucket::new),
                    since: 0,
                })
                .collect(),
            counters: vec![NodeCounters::default(); topo.nodes.len()],
            heap: BinaryHeap::new(),
            seq: 0,
            now: 0,
            reqs: Vec::new(),
            free: Vec::new(),
            spare: Vec::new(),
            answering: Vec::new(),
            finished: Vec::new(),
            order: 0,
            warmup: 0,
            end: u64::MAX,
            overflow: false,
        }
    }

    /// Refills every rate-limit bucket (full, as at startup). Safe at any moment: a bucket is
    /// read only when a hop starts, and holds no per-request state. Gates (slots, queues) and
    /// pools are **not** touched: they hold in-flight requests, and each frees its own as it
    /// finishes, so after the last in-flight request ends they are idle again.
    pub fn refill_buckets(&mut self) {
        for n in &mut self.nodes {
            if let Some(b) = &mut n.bucket {
                b.refill();
            }
        }
    }

    /// Reporting window: counters and [`Engine::drain_finished`] cover requests arriving at or
    /// after `warmup`; utilisation integrates busy time inside `[warmup, end]`.
    pub fn set_window(&mut self, warmup: u64, end: u64) {
        self.warmup = warmup;
        self.end = end;
    }

    /// The next sequence number. Callers that keep their own event queue (the simulator's
    /// arrivals) stamp each event so it orders against the engine's by `(time, seq)`.
    pub fn stamp(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// `(time, seq)` of the next scheduled event.
    pub fn peek(&self) -> Option<(u64, u64)> {
        self.heap.peek().map(|Reverse((t, s, ..))| (*t, *s))
    }

    /// Time (ns) of the next scheduled event.
    pub fn next_due(&self) -> Option<u64> {
        self.peek().map(|(t, _)| t)
    }

    /// Length of the request table (never shrinks): the most requests held at once.
    pub fn table_len(&self) -> usize {
        self.reqs.len()
    }

    /// Per-node totals, indexed by [`NodeId`].
    pub fn node_counters(&self) -> &[NodeCounters] {
        &self.counters
    }

    /// Takes the answers produced so far, in the order they were decided.
    pub fn drain_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }

    /// Like [`Engine::drain_finished`], but hands the answers out without giving up the
    /// buffer's capacity: a driver that drains after every event allocates nothing per answer.
    pub fn drain_finished_iter(&mut self) -> std::vec::Drain<'_, Finished> {
        self.finished.drain(..)
    }

    /// Starts a request at `now` (ns). Events due strictly before `now` are run first, each at
    /// its own time, so a late caller gets the same result as one that wakes on time; events due
    /// exactly at `now` stay queued (arrival-before-release ties keep their `(time, seq)` order).
    /// `plan` must come from `sample_plan` or `execute`: a hand-built plan with mismatched
    /// `Fork`/`Branch` sizes can panic or never finish.
    ///
    /// # Panics
    /// If `now` is earlier than the engine's clock (the last submit or event): time runs forward.
    ///
    /// # Errors
    /// [`EngineError::ClockOverflow`] if a hop would end past `u64::MAX` ns.
    pub fn submit(&mut self, now: u64, plan: Arc<Plan>, tag: u64) -> Result<(), EngineError> {
        self.check()?;
        while let Some(t) = self.next_due()
            && t < now
        {
            self.step()?;
        }
        assert!(
            now >= self.now,
            "submit at {now} ns is earlier than the engine clock ({} ns)",
            self.now
        );
        self.now = now;
        let len = plan.items.len();
        let mut sc = self.spare.pop().unwrap_or_default();
        sc.starts.clear();
        sc.starts.resize(len, NOT_STARTED);
        sc.waits.clear();
        sc.waits.resize(len, 0);
        sc.tokens.clear();
        sc.tokens.push(Token {
            at: 0,
            end: len,
            join: None,
            queued_at: None,
            last_own: None,
            open_top: None,
        });
        sc.joins.clear();
        sc.spans.clear();
        let req = Req {
            tag,
            order: self.order,
            arrival: now,
            plan,
            starts: sc.starts,
            waits: sc.waits,
            tokens: sc.tokens,
            joins: sc.joins,
            spans: sc.spans,
            busy: 0,
            fail: None,
            done: None,
            emitted: false,
        };
        self.order += 1;
        let r = if let Some(r) = self.free.pop() {
            self.reqs[r] = Some(req);
            r
        } else {
            self.reqs.push(Some(req));
            self.reqs.len() - 1
        };
        self.advance(r, 0);
        self.settle(r);
        self.check()
    }

    /// Runs exactly one event (the earliest by `(time, seq)`). `Ok(false)`: none is scheduled.
    ///
    /// # Errors
    /// [`EngineError::ClockOverflow`] as for [`Engine::submit`].
    pub fn step(&mut self) -> Result<bool, EngineError> {
        self.check()?;
        let Some(Reverse((at, seq, r, k))) = self.heap.pop() else {
            return Ok(false);
        };
        self.now = at;
        if seq & DEADLINE != 0 {
            self.deadline(r, k, seq);
        } else {
            self.hop_done(r, k);
        }
        self.check()?;
        Ok(true)
    }

    /// Runs every event due at or before `now`, each at its own scheduled time: a caller that
    /// wakes late (a coarse OS timer) gets the same hop starts and ends as one that wakes on time.
    ///
    /// # Errors
    /// [`EngineError::ClockOverflow`] as for [`Engine::submit`].
    pub fn advance_to(&mut self, now: u64) -> Result<(), EngineError> {
        while let Some(t) = self.next_due()
            && t <= now
        {
            self.step()?;
        }
        self.check()
    }

    /// **Private.** `Err` once an event time passed `u64::MAX`.
    fn check(&self) -> Result<(), EngineError> {
        if self.overflow {
            Err(EngineError::ClockOverflow)
        } else {
            Ok(())
        }
    }

    /// **Private.** `now + d`. On overflow it flags the engine (and returns `u64::MAX`) instead of
    /// letting the clock pin, which would silently collapse `(time, seq)` ordering.
    fn after(&mut self, d: u64) -> u64 {
        self.now.checked_add(d).unwrap_or_else(|| {
            self.overflow = true;
            u64::MAX
        })
    }

    /// **Private.** Schedules the end of token `k`'s hop in request `r` at `at`.
    fn push(&mut self, at: u64, r: usize, k: usize) {
        let seq = self.stamp();
        self.heap.push(Reverse((at, seq, r, k)));
    }

    /// **Private.** Adds `busy × (time since the last change, clipped to the window)` to the
    /// node's `area`. Call just before `busy` changes; extra calls change nothing (additive).
    fn mark(&mut self, node: NodeId, now: u64) {
        let (warmup, end) = (self.warmup, self.end);
        let ns = &mut self.nodes[node];
        let busy = ns.gate.as_ref().map_or(0, Gate::busy);
        let (from, to) = (ns.since.max(warmup), now.min(end));
        self.counters[node].area += u128::from(busy) * u128::from(to.saturating_sub(from));
        ns.since = now;
    }

    /// **Private.** Runs token `k` from its position until it starts a hop, parks at a fork,
    /// fails, or ends. A request that has failed starts nothing new; only a `Fail` reached at
    /// the same instant with a lower plan index can still replace the winner (D17).
    fn advance(&mut self, r: usize, mut k: usize) {
        loop {
            let now = self.now;
            let req = live(&mut self.reqs, r);
            let (at, end) = (req.tokens[k].at, req.tokens[k].end);
            // A span closes when its opener walks on past it. A fork parks the opener at the block
            // end at once, but it only loops again at the join, so the span stays open meanwhile.
            while let Some(i) = req.tokens[k].open_top
                && req.spans[i].end <= at
            {
                req.spans[i].open = false;
                req.tokens[k].open_top = req.spans[i].parent;
            }
            if at == end {
                let Some(j) = req.tokens[k].join else {
                    if req.fail.is_none() && req.done.is_none() {
                        req.done = Some(now);
                        self.answering.push(r);
                    }
                    return;
                };
                req.joins[j].left -= 1;
                if req.joins[j].left > 0 || req.fail.is_some() {
                    return;
                }
                let parent = req.joins[j].parent;
                // The branches ran edge hops: the next own hop of the parent starts a new visit (Y6).
                req.tokens[parent].last_own = None;
                k = parent; // the last branch resumes the block's parent
                continue;
            }
            let item = req.plan.items[at].clone();
            if let Some(f) = &req.fail {
                let tie = f.at == now && at < f.idx;
                // A sibling cancelled at this instant still reaches a lower-index `Fail` through
                // zero-duration hops that need no slot or token, as `execute` does (D17); the
                // skipped hop never starts, so it is not recorded.
                let skip = match &item {
                    PlanItem::Span { .. } => true,
                    PlanItem::Hop(h) => {
                        h.dur.is_zero()
                            && (h.kind == HopKind::Net
                                || h.kind == HopKind::Delay
                                || (self.nodes[h.node].gate.is_none()
                                    && self.nodes[h.node].bucket.is_none()))
                    }
                    _ => false,
                };
                if tie && skip {
                    req.tokens[k].at += 1;
                    continue;
                }
                if !(tie && matches!(item, PlanItem::Fail { .. })) {
                    return;
                }
            }
            match item {
                PlanItem::Hop(h) => {
                    self.start_hop(r, k, &h);
                    return;
                }
                PlanItem::Fail {
                    node, status, kind, ..
                } => {
                    self.fail(
                        r,
                        Failure {
                            at: now,
                            idx: at,
                            node,
                            status,
                            kind,
                            retry_after: None,
                            refused: None,
                            cause: FailCause::Chaos,
                        },
                    );
                    return;
                }
                PlanItem::Fork { branches, size } => {
                    self.fork(r, k, branches, size);
                    return;
                }
                PlanItem::Span {
                    node,
                    timeout,
                    size,
                    ..
                } => self.open_span(r, k, node, nanos(timeout), size),
                PlanItem::Branch { .. } => unreachable!("tokens start after their Branch marker"),
            }
        }
    }

    /// **Private.** Token `k` reaches a `Fork`: park it at the block end, start one token per branch.
    fn fork(&mut self, r: usize, k: usize, branches: usize, size: usize) {
        let req = live(&mut self.reqs, r);
        let at = req.tokens[k].at;
        let j = req.joins.len();
        req.joins.push(Join {
            left: branches,
            parent: k,
        });
        req.tokens[k].at = at + 1 + size;
        let inherited = req.tokens[k].last_own;
        let first = req.tokens.len();
        let mut b = at + 1;
        while b < at + 1 + size {
            let PlanItem::Branch { size: bs } = req.plan.items[b] else {
                break;
            };
            req.tokens.push(Token {
                at: b + 1,
                end: b + 1 + bs,
                join: Some(j),
                queued_at: None,
                last_own: inherited,
                open_top: None,
            });
            b += 1 + bs;
        }
        let last = req.tokens.len();
        // Branch order: a contested slot goes to the lower branch first.
        for c in first..last {
            self.advance(r, c);
        }
    }

    /// **Private.** Token `k` enters the `Span` at its position: record it and schedule its deadline.
    fn open_span(&mut self, r: usize, k: usize, node: NodeId, timeout: u64, size: usize) {
        let seq = self.stamp() | DEADLINE;
        let due = self.after(timeout);
        let req = live(&mut self.reqs, r);
        let at = req.tokens[k].at;
        let si = req.spans.len();
        req.spans.push(SpanState {
            parent: req.tokens[k].open_top,
            end: at + 1 + size,
            idx: at,
            node,
            seq,
            due,
            open: true,
        });
        req.tokens[k].open_top = Some(si);
        req.tokens[k].at = at + 1;
        // ponytail: a closed span's deadline stays in the heap until its time (at most
        // `timeout`); drop it on close if heap size under heavy timeouts ever shows up.
        self.heap.push(Reverse((due, seq, r, si)));
    }

    /// **Private.** Starts token `k`'s hop now. Order (Y7): a rate-limit token if this hop starts
    /// a visit (Y6), then the node's gate. A refusal fails the request at this hop's start; the
    /// refused hop never starts and takes neither a token's slot nor a queue place.
    fn start_hop(&mut self, r: usize, k: usize, h: &Hop) {
        let now = self.now;
        let (idx, counted) = {
            let req = live(&mut self.reqs, r);
            (req.tokens[k].at, req.arrival >= self.warmup)
        };
        if h.kind == HopKind::Net || h.kind == HopKind::Delay {
            let req = live(&mut self.reqs, r);
            req.tokens[k].last_own = None;
            req.busy += 1;
        } else {
            let node = h.node;
            let tok = &mut live(&mut self.reqs, r).tokens[k];
            let new_visit = tok.last_own != Some(node);
            tok.last_own = Some(node);
            if new_visit
                && let Some(b) = self.nodes[node].bucket.as_mut()
                && let Err(wait) = b.take(now)
            {
                if counted {
                    self.counters[node].limited += 1;
                }
                let retry_after = Some(Duration::from_nanos(wait));
                self.fail(
                    r,
                    Failure {
                        at: now,
                        idx,
                        node,
                        status: 429,
                        kind: FailKind::Status,
                        retry_after,
                        refused: counted.then_some((node, true)),
                        cause: FailCause::RateLimit,
                    },
                );
                return;
            }
            if self.nodes[node].gate.is_some() {
                self.mark(node, now);
                let gate = self.nodes[node].gate.as_mut().expect("checked");
                match gate.arrive((r, k)) {
                    Admit::Run => {}
                    Admit::Queued => {
                        let len = gate.queued();
                        if now >= self.warmup {
                            let c = &mut self.counters[node];
                            c.max_queue = c.max_queue.max(len);
                        }
                        let req = live(&mut self.reqs, r);
                        req.busy += 1;
                        req.tokens[k].queued_at = Some((node, now));
                        return;
                    }
                    Admit::Overflow => {
                        let status = gate.overflow_status();
                        if counted {
                            self.counters[node].overflowed += 1;
                        }
                        self.fail(
                            r,
                            Failure {
                                at: now,
                                idx,
                                node,
                                status,
                                kind: FailKind::Status,
                                retry_after: None,
                                refused: counted.then_some((node, false)),
                                cause: FailCause::Overflow,
                            },
                        );
                        return;
                    }
                }
            }
            live(&mut self.reqs, r).busy += 1;
            if counted {
                self.counters[node].completed += 1;
            }
        }
        live(&mut self.reqs, r).starts[idx] = now;
        let done = self.after(nanos(h.dur));
        self.push(done, r, k);
    }

    /// **Private.** Token `k`'s hop ended: hand its slot to the head of the queue (or free it),
    /// then move the token on.
    fn hop_done(&mut self, r: usize, k: usize) {
        let now = self.now;
        let (node, kind) = {
            let req = live(&mut self.reqs, r);
            let at = req.tokens[k].at;
            let PlanItem::Hop(h) = &req.plan.items[at] else {
                unreachable!("HopDone names a hop")
            };
            (h.node, h.kind)
        };
        if kind != HopKind::Net && kind != HopKind::Delay && self.nodes[node].gate.is_some() {
            self.mark(node, now);
            // FIFO hand-over at this instant: the slot never looks free to a later arrival.
            let next = self.nodes[node].gate.as_mut().expect("checked").release();
            if let Some((qr, qk)) = next {
                self.start_queued(qr, qk);
            }
        }
        let req = live(&mut self.reqs, r);
        req.busy -= 1;
        req.tokens[k].at += 1;
        self.advance(r, k);
        self.settle(r);
    }

    /// **Private.** The queued token `(qr, qk)` got a slot now: record its wait and schedule its hop.
    fn start_queued(&mut self, qr: usize, qk: usize) {
        let (now, warmup) = (self.now, self.warmup);
        let waiter = live(&mut self.reqs, qr);
        let i = waiter.tokens[qk].at;
        let (node, since) = waiter.tokens[qk]
            .queued_at
            .take()
            .expect("a token in a node queue has queued_at set");
        let wait = now - since;
        waiter.waits[i] = wait;
        waiter.starts[i] = now;
        let counted = waiter.arrival >= warmup;
        let PlanItem::Hop(qh) = &waiter.plan.items[i] else {
            unreachable!("queued at a hop")
        };
        let dur = nanos(qh.dur);
        if counted {
            let c = &mut self.counters[node];
            c.completed += 1;
            c.wait += u128::from(wait);
        }
        let done = self.after(dur);
        self.push(done, qr, qk);
    }

    /// **Private.** A span's deadline passed (Z3, Z4, Z14). Ignored unless the request is still
    /// unanswered and the span still open; then the whole request fails as a timeout through
    /// `fail()`: queued tokens leave their queues, running hops keep their slot until they end.
    fn deadline(&mut self, r: usize, si: usize, seq: u64) {
        let (now, warmup) = (self.now, self.warmup);
        let ready = self.reqs[r].as_ref().and_then(|q| {
            let s = q.spans.get(si)?;
            if !(s.seq == seq && s.open && q.done.is_none() && q.fail.is_none()) {
                return None;
            }
            // Z14: of this request's open spans due now, the lowest plan index (the outer) wins,
            // as in `execute`; the others then find the request answered.
            let w = q
                .spans
                .iter()
                .filter(|x| x.open && x.due == now)
                .min_by_key(|x| x.idx)?;
            Some((w.idx, w.node, q.arrival >= warmup))
        });
        let Some((idx, node, counted)) = ready else {
            self.flush();
            return;
        };
        if counted {
            self.counters[node].timed_out += 1;
        }
        self.fail(
            r,
            Failure {
                at: now,
                idx,
                node,
                kind: FailKind::Timeout,
                status: 504,
                retry_after: None,
                refused: None,
                cause: FailCause::Deadline,
            },
        );
        self.settle(r);
    }

    /// **Private.** `f` decided request `r` (E14: at the failing hop's start). It wins (first, or
    /// a lower index at the same instant, D17); siblings still queued leave their queues; running
    /// ones keep their slot until their hop ends.
    fn fail(&mut self, r: usize, f: Failure) {
        let now = self.now;
        let req = live(&mut self.reqs, r);
        let first = req.done.is_none();
        let old = req.fail.replace(f);
        req.done = Some(now);
        if let Some(Failure {
            refused: Some((node, limited)),
            ..
        }) = old
        {
            let c = &mut self.counters[node];
            if limited {
                c.limited -= 1;
            } else {
                c.overflowed -= 1;
            }
        }
        let queued: Vec<(usize, NodeId)> = req
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(k, t)| t.queued_at.map(|(n, _)| (k, n)))
            .collect();
        for (k, node) in queued {
            // ponytail: linear scan of that node's queue; keep a per-token handle if long
            // queues of failing requests ever show up in a profile.
            self.nodes[node]
                .gate
                .as_mut()
                .expect("queued implies a gate")
                .cancel((r, k));
            let req = live(&mut self.reqs, r);
            req.tokens[k].queued_at = None;
            req.busy -= 1;
        }
        if first {
            self.answering.push(r);
        }
    }

    /// **Private.** After an event on request `r`: free it if answered and idle, and emit the
    /// answers of the instant that just ended.
    fn settle(&mut self, r: usize) {
        let req = live(&mut self.reqs, r);
        if req.busy == 0 && req.emitted {
            self.release_slot(r);
        }
        self.flush();
    }

    /// **Private.** Emits every pending answer once no other event is scheduled at this
    /// instant (a same-instant tie may still replace a failure, D17).
    fn flush(&mut self) {
        if self.answering.is_empty() || self.peek().is_some_and(|(t, _)| t <= self.now) {
            return;
        }
        // Index loop and `clear`, not `mem::take`: the list keeps its capacity (no allocation per answer).
        for i in 0..self.answering.len() {
            let r = self.answering[i];
            let req = self.reqs[r].as_mut().expect("answered requests are live");
            req.emitted = true;
            if req.arrival >= self.warmup {
                self.finished.push(record(req));
            }
            if req.busy == 0 {
                self.release_slot(r);
            }
        }
        self.answering.clear();
    }

    /// **Private.** Frees request slot `r` and keeps its buffers for the next request (R6.13).
    fn release_slot(&mut self, r: usize) {
        let req = self.reqs[r].take().expect("released once");
        self.free.push(r);
        self.spare.push(Scratch {
            starts: req.starts,
            waits: req.waits,
            tokens: req.tokens,
            joins: req.joins,
            spans: req.spans,
        });
    }
}

/// **Private.** The client's view of an answered request: hops that started, cut at the
/// failure when their branch was cancelled.
fn record(req: &Req) -> Finished {
    let plan = &req.plan;
    let (cut, fail_at) = match &req.fail {
        Some(f) => (plan.cancelled_by(f.idx), f.at),
        None => (Vec::new(), u64::MAX),
    };
    // A timeout cuts every hop at the failure instant, not only cancelled sibling ranges.
    let timeout = matches!(&req.fail, Some(f) if f.kind == FailKind::Timeout);
    let mut hops = Vec::new();
    for (i, item) in plan.items.iter().enumerate() {
        let PlanItem::Hop(h) = item else { continue };
        let s = req.starts[i];
        if s == NOT_STARTED {
            continue;
        }
        let mut h = h.clone();
        h.start = Duration::from_nanos(s - req.arrival);
        if timeout || cut.iter().any(|c| c.contains(&i)) {
            if s >= fail_at {
                continue;
            }
            h.dur = h.dur.min(Duration::from_nanos(fail_at - s));
        }
        hops.push(FinishedHop {
            idx: i,
            hop: h,
            wait: Duration::from_nanos(req.waits[i]),
        });
    }
    let result = req
        .fail
        .as_ref()
        .map_or(ChainResult::Ok, |f| ChainResult::Err {
            node: f.node,
            status: f.status,
            kind: f.kind,
            retry_after: f.retry_after,
        });
    Finished {
        tag: req.tag,
        order: req.order,
        arrival: req.arrival,
        done: req.done.expect("answered requests are done"),
        result,
        cause: req.fail.as_ref().map(|f| f.cause),
        hops,
    }
}
