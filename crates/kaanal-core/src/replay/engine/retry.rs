//! Retries in the engine (RULES 82-84, step 7a): a failure inside an attempt of a [`PlanItem::Retry`]
//! scope ends that attempt, not the request, when the failure is transient and an attempt is left.
//!
//! The plan holds every attempt, one after another. A token walks the first; a transient failure
//! anywhere inside the attempt (a planned fault, a refused hop, a fired deadline) **cuts** the
//! attempt's plan range: nothing in it starts again, queued tokens leave their queues, running hops
//! finish as zombies (as after any failure), and the slots it held across calls are let go. A timer
//! then starts the next attempt after its backoff (at least the failure's `retry_after`, RFC 9110
//! 10.2.3) on a fresh token that takes the place of the one that entered the scope. A failure that is
//! not transient, or one in the last attempt, goes on outward: to an enclosing attempt, or it fails the
//! request as before.
//!
//! A plan without retries never touches this file: every check is on an empty vector.

use std::cmp::Reverse;
use std::time::Duration;

use super::state::{Cut, FailCause, Failure, NOT_STARTED, RETRY, RetryState, Token, live, nanos};
use super::{Engine, NodeId, PlanItem};
use crate::chaos::FailKind;
use crate::exec::{BreakerRef, RetryBudget, retryable};

use super::breaker::{Breaker, Decision};

/// **Private.** What a token does next, see [`Engine::retry_next`].
pub(super) enum Next {
    /// The token is a zombie: it goes no further.
    Stop,
    /// The token moved past a finished scope: look again from there.
    Again,
    /// Nothing to do: take the next item.
    Go,
}

impl Engine {
    /// **Private.** `f` ended something: the innermost enclosing attempt that can try again is cut and
    /// retried; failing that, the request fails.
    pub(super) fn fail_or_retry(&mut self, r: usize, f: Failure) {
        let req = live(&mut self.reqs, r);
        // abandoned work that fails ends only itself: nobody is waiting for it
        if req.orphan_at(f.idx) {
            return;
        }
        if !req.retries.is_empty() && req.fail.is_none() && req.done.is_none() {
            let transient = retryable(f.status, f.kind);
            // the scopes whose running attempt holds the failing item, innermost first (scopes are
            // pushed as they are entered, so later ones are deeper)
            let around: Vec<usize> = req
                .retries
                .iter()
                .enumerate()
                .rev()
                // `>=`: a breaker rejection is reported at the attempt's own marker, so it also reaches an
                // attempt with no items; no other failure sits on a marker
                .filter(|(_, s)| s.open && !s.waiting && s.cur_start <= f.idx && f.idx < s.cur_end)
                .map(|(i, _)| i)
                .collect();
            for si in around {
                let (budget, more, breaker) = {
                    let s = &live(&mut self.reqs, r).retries[si];
                    (s.budget, s.cur_end < s.end, s.breaker)
                };
                // a breaker's own rejection sent no call: it is neither a breaker outcome nor a budget cost
                let sent = f.cause != FailCause::Breaker;
                if let Some(b) = breaker
                    && sent
                    && (f.status >= 500 || matches!(f.kind, FailKind::Timeout | FailKind::Reset))
                {
                    self.breaker_record(b, false);
                }
                // every transient failure costs a token, also one that is not retried (A6)
                let afforded = match budget {
                    Some(b) if transient && sent => self.budget_failed(&b),
                    _ => true,
                };
                if transient && more && afforded {
                    self.retry_attempt(r, si, &f);
                    return;
                }
            }
        }
        self.fail(r, f);
    }

    /// **Private.** gRPC A6 retry throttling ("gRPC Retry Design"): a failed attempt takes a token (the
    /// count never goes below 0); `true` while more than half of `maxTokens` is left, the rest of the
    /// time no retry is made.
    fn budget_failed(&mut self, b: &RetryBudget) -> bool {
        let t = self
            .budgets
            .entry(u32::from(b.edge) << 16 | u32::from(b.replica))
            .or_insert(b.max_milli);
        *t = t.saturating_sub(1000);
        *t > b.max_milli / 2
    }

    /// **Private.** A successful attempt gives `tokenRatio` back, up to `maxTokens`.
    fn budget_succeeded(&mut self, b: &RetryBudget) {
        let t = self
            .budgets
            .entry(u32::from(b.edge) << 16 | u32::from(b.replica))
            .or_insert(b.max_milli);
        *t = (*t + u32::from(b.ratio_milli)).min(b.max_milli);
    }

    /// **Private.** Before token `k` of request `r` takes its next item: a token in an attempt that was
    /// cut is a zombie and stops; one standing at the end of an attempt has succeeded, so the scope is
    /// over and the remaining attempts are skipped.
    pub(super) fn retry_next(&mut self, r: usize, k: usize) -> Next {
        let at = live(&mut self.reqs, r).tokens[k].at;
        if self.is_cut(r, at) {
            return Next::Stop;
        }
        let to = self.leave_retry(r, at);
        if to == at {
            return Next::Go;
        }
        live(&mut self.reqs, r).tokens[k].at = to;
        Next::Again
    }

    /// **Private.** Token `k` reaches a `Retry` marker: record the scope and stand the token in the
    /// first attempt, past its marker.
    pub(super) fn open_retry(&mut self, r: usize, k: usize, size: usize) {
        let req = live(&mut self.reqs, r);
        let at = req.tokens[k].at;
        let PlanItem::Retry {
            budget,
            breaker,
            attempts,
            ..
        } = req.plan.items[at]
        else {
            unreachable!("open_retry is called at a Retry marker")
        };
        let PlanItem::Attempt { size: first, .. } = req.plan.items[at + 1] else {
            unreachable!("a Retry is followed by its first Attempt")
        };
        let t = &req.tokens[k];
        let base = Token {
            at: 0,
            end: t.end,
            join: t.join,
            queued_at: None,
            last_own: None,
            open_top: t.open_top,
            hold_top: t.hold_top,
        };
        req.retries.push(RetryState {
            end: at + 1 + size,
            cur_start: at + 1,
            cur_end: at + 2 + first,
            base,
            budget,
            breaker,
            open: true,
            waiting: false,
            timer: 0,
            attempts,
            first: at + 1,
            starts: Vec::new(),
        });
        req.tokens[k].at = at + 2;
        // the first attempt asks the breaker; a rejection fails it and leaves this token a zombie
        let si = req.retries.len() - 1;
        self.breaker_admits(r, si);
    }

    /// **Private.** A token stands at plan index `at` (its own position): if that is the end of an
    /// attempt, the attempt succeeded and the scope is over. Returns the position to go on from (past
    /// the remaining attempts), or `at` when no scope ended here.
    pub(super) fn leave_retry(&mut self, r: usize, at: usize) -> usize {
        let req = live(&mut self.reqs, r);
        let mut to = at;
        let mut paid: Vec<RetryBudget> = Vec::new();
        let mut ok: Vec<BreakerRef> = Vec::new();
        for s in &mut req.retries {
            if s.open && !s.waiting && s.cur_end == at {
                s.open = false;
                to = to.max(s.end);
                paid.extend(s.budget);
                ok.extend(s.breaker);
            }
        }
        for b in paid {
            self.budget_succeeded(&b);
        }
        for b in ok {
            self.breaker_record(b, true);
        }
        to
    }

    /// **Private.** Whether plan index `at` lies in an attempt that was cut: a token standing there is
    /// a zombie and goes no further. The end counts, for a zombie that just finished its last hop.
    pub(super) fn is_cut(&mut self, r: usize, at: usize) -> bool {
        live(&mut self.reqs, r)
            .cuts
            .iter()
            .any(|c| c.lo < at && at <= c.hi)
    }

    /// **Private.** Cuts the current attempt of scope `si` because of `f` and schedules the next one.
    fn retry_attempt(&mut self, r: usize, si: usize, f: &Failure) {
        let now = self.now;
        let seq = self.stamp() | RETRY;
        let req = live(&mut self.reqs, r);
        let (lo, hi) = (req.retries[si].cur_start, req.retries[si].cur_end);
        // the next attempt's marker sits at `hi`; its backoff is the sampled one, at least `retry_after`
        let PlanItem::Attempt { backoff, .. } = req.plan.items[hi] else {
            unreachable!("an attempt is followed by the next Attempt marker")
        };
        let wait = backoff.max(f.retry_after.unwrap_or(Duration::ZERO));
        req.cuts.push(Cut { lo, hi, at: now });
        // deadlines of spans opened in the attempt no longer count
        for sp in &mut req.spans {
            if lo < sp.idx && sp.idx < hi {
                sp.open = false;
            }
        }
        // a scope inside the cut attempt is over, with any backoff it was waiting out
        for s in &mut req.retries {
            if s.open && lo < s.cur_start && s.end <= hi {
                s.open = false;
                s.waiting = false;
                s.timer = 0;
            }
        }
        req.retries[si].waiting = true;
        req.retries[si].timer = seq;
        // queued tokens in the attempt leave their queues, as when a request fails
        let queued: Vec<(usize, NodeId, u16)> = req
            .tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| lo < t.at && t.at <= hi && !req.orphan_at(t.at))
            .filter_map(|(k, t)| t.queued_at.map(|(n, rep, _)| (k, n, rep)))
            .collect();
        for (k, node, rep) in queued {
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
        self.release_cut_holds(r);
        let at = self.after(nanos(wait));
        self.heap.push(Reverse((at, seq, r, si)));
    }

    /// **Private.** The backoff of scope `si` of request `r` is over (`seq` names the timer, so a
    /// recycled request slot cannot match): start the next attempt on a fresh token, unless the request
    /// has been answered meanwhile.
    pub(super) fn retry_timer(&mut self, r: usize, si: usize, seq: u64) {
        let ready = self.reqs[r].as_ref().is_some_and(|q| {
            q.done.is_none()
                && q.fail.is_none()
                && q.retries.get(si).is_some_and(|s| s.timer == seq)
        });
        if !ready {
            self.flush();
            return;
        }
        let req = live(&mut self.reqs, r);
        let s = &mut req.retries[si];
        let next = s.cur_end;
        let PlanItem::Attempt { size, .. } = req.plan.items[next] else {
            unreachable!("the next attempt starts at its marker")
        };
        s.cur_start = next;
        s.cur_end = next + 1 + size;
        s.waiting = false;
        s.starts.push(self.now);
        // an open breaker fails this attempt at once; no token starts
        if !self.breaker_admits(r, si) {
            self.settle(r);
            return;
        }
        let req = live(&mut self.reqs, r);
        let s = &mut req.retries[si];
        let mut t = Token {
            at: next + 1,
            ..s.base
        };
        t.queued_at = None;
        let k = req.tokens.len();
        req.tokens.push(t);
        self.advance(r, k);
        self.settle(r);
    }

    /// **Private.** The attempt now starting in scope `si` asks the scope's circuit breaker (step 7c):
    /// `false` when it was rejected, in which case the attempt has been failed as a 503 with no work
    /// done (`short_circuited` counts it at the callee) and the scope retries it or fails the request.
    fn breaker_admits(&mut self, r: usize, si: usize) -> bool {
        let req = live(&mut self.reqs, r);
        // reported at the attempt's marker: an attempt may hold no item to point at
        let (idx, counted) = (req.retries[si].cur_start, req.arrival >= self.warmup);
        let Some(b) = req.retries[si].breaker else {
            return true;
        };
        let Some(&Some((cfg, callee))) = self.breaker_cfg.get(usize::from(b.edge)) else {
            return true;
        };
        let now = self.now;
        let d = self
            .breakers
            .entry(u32::from(b.edge) << 16 | u32::from(b.replica))
            .or_insert_with(Breaker::new)
            .allow(&cfg, now);
        if d != Decision::Reject {
            return true;
        }
        if counted {
            self.counters[callee].short_circuited += 1;
        }
        self.fail_or_retry(
            r,
            Failure {
                at: now,
                idx,
                node: callee,
                status: 503,
                kind: FailKind::Status,
                retry_after: None,
                refused: None,
                cause: FailCause::Breaker,
            },
        );
        false
    }

    /// **Private.** An attempt that went through a breaker ended: `ok` false is a counted failure.
    fn breaker_record(&mut self, b: BreakerRef, ok: bool) {
        let Some(&Some((cfg, _))) = self.breaker_cfg.get(usize::from(b.edge)) else {
            return;
        };
        let now = self.now;
        self.breakers
            .entry(u32::from(b.edge) << 16 | u32::from(b.replica))
            .or_insert_with(Breaker::new)
            .record(&cfg, now, ok);
    }

    /// **Private.** Lets go of the slots a cut attempt held across calls, except where a hop of the
    /// request is still running on the unit (it frees its slot when it ends). Called when the attempt is
    /// cut and as each hop of a cut attempt ends.
    pub(super) fn release_cut_holds(&mut self, r: usize) {
        let now = self.now;
        let idle: Vec<(NodeId, u16)> = {
            let req = live(&mut self.reqs, r);
            let mut running: Vec<(NodeId, u16)> = Vec::new();
            for t in &req.tokens {
                if t.queued_at.is_some() || t.at >= t.end {
                    continue;
                }
                if let PlanItem::Hop(h) = &req.plan.items[t.at]
                    && req.starts[t.at] != NOT_STARTED
                {
                    running.push(self.units.target(h));
                }
            }
            let (cuts, orphans) = (&req.cuts, &req.orphans);
            req.holds
                .iter_mut()
                .filter(|x| {
                    x.open
                        && !running.contains(&(x.node, x.replica))
                        && cuts.iter().any(|c| c.lo < x.end && x.end <= c.hi)
                        && !orphans.iter().any(|&(lo, hi)| lo < x.end && x.end <= hi)
                })
                .map(|x| {
                    x.open = false;
                    x.until = now;
                    (x.node, x.replica)
                })
                .collect()
        };
        for (node, rep) in idle {
            self.free_unit(node, rep);
        }
    }
}
