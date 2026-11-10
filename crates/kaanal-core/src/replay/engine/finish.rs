//! How a request ends: a span's deadline, a failure cutting the request short, settling the table
//! slot, releasing a slot, and the record the client sees. Split out of `mod.rs` (R3.8: that file was
//! at 796 of 800 lines); moved unchanged, only `pub(super)` where `mod.rs` calls them.

use std::time::Duration;

use super::state::{Failure, FinishedAttempt, NOT_STARTED, Req, RetryState, Scratch, live};
use super::{
    ChainResult, Engine, FailCause, FailKind, Finished, FinishedHold, FinishedHop, NodeId, PlanItem,
};

impl Engine {
    /// **Private.** The gate of copy `(node, rep)` refused request `r`'s hop at `idx` (every slot
    /// busy, the queue full): the node's overflow status, or, on a node that dies of memory, the
    /// kill of the copy (step 8K).
    pub(super) fn refuse_overflow(
        &mut self,
        r: usize,
        idx: usize,
        (node, rep): (NodeId, u16),
        counted: bool,
    ) {
        if self.oom_overflow(r, idx, node, rep) {
            return;
        }
        let at = self.now;
        let gate = self.units.state(node, rep).gate.as_ref().expect("checked");
        let status = gate.overflow_status();
        if counted {
            self.count(node, rep, |c| c.overflowed += 1);
        }
        self.fail_or_retry(
            r,
            Failure {
                at,
                idx,
                node,
                status,
                kind: FailKind::Status,
                retry_after: None,
                refused: counted.then_some((node, rep, false)),
                cause: FailCause::Overflow,
            },
        );
    }

    /// **Private.** A span's deadline passed (Z3, Z4, Z14). Ignored unless the request is still
    /// unanswered and the span still open; then the whole request fails as a timeout through
    /// `fail()`: queued tokens leave their queues, running hops keep their slot until they end.
    pub(super) fn deadline(&mut self, r: usize, si: usize, seq: u64) {
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
            Some((
                w.idx,
                w.node,
                q.arrival >= warmup,
                w.orphan.then_some(w.end),
            ))
        });
        let Some((idx, node, counted, goes_on)) = ready else {
            self.flush();
            return;
        };
        if counted {
            self.counters[node].timed_out += 1;
        }
        if let Some(end) = goes_on {
            live(&mut self.reqs, r).orphans.push((idx, end));
        }
        self.fail_or_retry(
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
    pub(super) fn fail(&mut self, r: usize, f: Failure) {
        let now = self.now;
        let req = live(&mut self.reqs, r);
        let first = req.done.is_none();
        let old = req.fail.replace(f);
        req.done = Some(now);
        if let Some(Failure {
            refused: Some((node, rep, limited)),
            ..
        }) = old
        {
            for c in [
                &mut self.counters[node],
                &mut self.units.counters[node][usize::from(rep)],
            ] {
                if limited {
                    c.limited -= 1;
                } else {
                    c.overflowed -= 1;
                }
            }
        }
        let queued: Vec<(usize, NodeId, u16)> = req
            .tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| !req.orphan_at(t.at))
            .filter_map(|(k, t)| t.queued_at.map(|(n, rep, _)| (k, n, rep)))
            .collect();
        for (k, node, rep) in queued {
            // A linear scan of that node's queue; it runs only for a request that failed while it
            // was waiting there.
            self.units
                .state(node, rep)
                .gate
                .as_mut()
                .expect("queued implies a gate")
                .cancel((r, k));
            let req = live(&mut self.reqs, r);
            req.tokens[k].queued_at = None;
            req.busy -= 1;
        }
        // callers' workers are freed as their calls fail, but not under a hop still running
        self.release_idle_holds(r);
        if first {
            self.answering.push(r);
        }
    }

    /// **Private.** After an event on request `r`: free it if answered and idle, and emit the
    /// answers of the instant that just ended.
    pub(super) fn settle(&mut self, r: usize) {
        let req = live(&mut self.reqs, r);
        if req.busy == 0 && req.emitted {
            self.release_slot(r);
        }
        self.flush();
    }

    /// **Private.** Emits every pending answer once no other event is scheduled at this
    /// instant (a same-instant tie may still replace a failure, D17).
    pub(super) fn flush(&mut self) {
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
    pub(super) fn release_slot(&mut self, r: usize) {
        // a request that failed while holding slots lets them go now that nothing of it runs
        self.drop_holds(r);
        let req = self.reqs[r].take().expect("released once");
        self.free.push(r);
        self.spare.push(Scratch {
            starts: req.starts,
            waits: req.waits,
            tokens: req.tokens,
            joins: req.joins,
            spans: req.spans,
            holds: req.holds,
            retries: req.retries,
            cuts: req.cuts,
            orphans: req.orphans,
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
        // an attempt that was cut stops at its failure, like a hop a failure cancelled
        if let Some(c) = req.cuts.iter().find(|c| c.lo < i && i <= c.hi) {
            if s >= c.at {
                continue;
            }
            h.dur = h.dur.min(Duration::from_nanos(c.at - s));
        }
        hops.push(FinishedHop {
            idx: i,
            hop: h,
            wait: Duration::from_nanos(req.waits[i]),
            attempt: attempt_of_hop(req, i),
        });
    }
    // a hold still open at the answer is cut there, as a hop a failure cancelled is
    let answered = req.done.expect("answered requests are done");
    let holds = req
        .holds
        .iter()
        .map(|h| {
            let until = if h.open {
                answered.max(h.since)
            } else {
                h.until
            };
            FinishedHold {
                node: h.node,
                replica: h.replica,
                start: Duration::from_nanos(h.since - req.arrival),
                dur: Duration::from_nanos(until.saturating_sub(h.since)),
            }
        })
        .collect();
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
        holds,
        // most requests retry nothing: no call, no allocation
        attempts: if req.retries.is_empty() {
            Vec::new()
        } else {
            attempts_of(req)
        },
    }
}

/// **Private.** The retries a request made, with the component each went to: the first hop, failure or
/// span of the attempt names it (an attempt a breaker refused has none of its own, so the failure's node).
#[inline(never)]
fn attempts_of(req: &Req) -> Vec<FinishedAttempt> {
    let mut out = Vec::new();
    for s in &req.retries {
        for (k, &at) in s.starts.iter().enumerate() {
            // the attempt that started at the k-th retry is attempt k + 2; its marker is found by counting
            let marker = attempt_marker(&req.plan, s, k + 1);
            let node = req.plan.items[marker + 1..]
                .iter()
                .find_map(|i| match i {
                    PlanItem::Hop(h) => Some(h.node),
                    PlanItem::Fail { node, .. } | PlanItem::Span { node, .. } => Some(*node),
                    _ => None,
                })
                .or_else(|| req.fail.as_ref().map(|f| f.node))
                .unwrap_or_default();
            out.push(FinishedAttempt {
                at: Duration::from_nanos(at.saturating_sub(req.arrival)),
                n: u8::try_from(k + 2).unwrap_or(u8::MAX),
                of: s.attempts,
                node,
            });
        }
    }
    out.sort_by_key(|a| a.at);
    out
}

/// **Private.** The try of its call that plan item `i` ran in (1 for the first, and outside any retry): the
/// deepest of the retries that started and whose attempt holds the item.
fn attempt_of_hop(req: &Req, i: usize) -> u8 {
    let mut best = 1;
    for s in &req.retries {
        for k in 1..=s.starts.len() {
            let m = attempt_marker(&req.plan, s, k);
            if let PlanItem::Attempt { size, .. } = req.plan.items[m]
                && i > m
                && i <= m + size
            {
                best = best.max(u8::try_from(k + 1).unwrap_or(u8::MAX));
            }
        }
    }
    best
}

/// **Private.** Plan index of the marker of attempt `k` (0-based) of scope `s`: the scope's attempts
/// follow its `Retry` item one after another, each `Attempt` marker saying how many items belong to it.
fn attempt_marker(plan: &crate::exec::Plan, s: &RetryState, k: usize) -> usize {
    // `end` is just past the scope and `cur_start` is the marker of its latest attempt; walk back from the
    // scope's first attempt (the item after its `Retry`) by sizes
    let mut at = s.first;
    for _ in 0..k {
        let PlanItem::Attempt { size, .. } = plan.items[at] else {
            break;
        };
        at += 1 + size;
    }
    at
}
