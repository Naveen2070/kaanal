//! How a request ends: a span's deadline, a failure cutting the request short, settling the table
//! slot, releasing a slot, and the record the client sees. Split out of `mod.rs` (R3.8: that file was
//! at 796 of 800 lines); moved unchanged, only `pub(super)` where `mod.rs` calls them.

use std::time::Duration;

use super::state::{Failure, NOT_STARTED, Req, Scratch, live};
use super::{
    ChainResult, Engine, FailCause, FailKind, Finished, FinishedHold, FinishedHop, NodeId, PlanItem,
};

impl Engine {
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
            Some((w.idx, w.node, q.arrival >= warmup))
        });
        let Some((idx, node, counted)) = ready else {
            self.flush();
            return;
        };
        if counted {
            self.counters[node].timed_out += 1;
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
            .filter_map(|(k, t)| t.queued_at.map(|(n, rep, _)| (k, n, rep)))
            .collect();
        for (k, node, rep) in queued {
            // ponytail: linear scan of that node's queue; keep a per-token handle if long
            // queues of failing requests ever show up in a profile.
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
    }
}
