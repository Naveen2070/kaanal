//! Holding a slot across a call (`hold: call`): a node with a thread or worker per request keeps its
//! slot while it waits on what it calls, so a slow dependency fills the slots above it.
//!
//! A [`PlanItem::Hold`] marker opens the scope. The token that reaches it takes a slot of the node
//! (queueing like a hop when none is free, or failing on a full queue or the rate limit) and keeps it
//! until it walks past the scope's last item. The node's own hops inside the scope take no slot of
//! their own: they run in the held one. A request that fails while holding lets go of what it holds
//! the moment it fails, as a caller's worker is freed when its call errors, except where the node still
//! has a hop of that request running: that slot is freed when the hop ends (`release_idle_holds`).
//! `drop_holds` is the last net, when the request's table slot is freed.

use super::state::{Failure, HoldState, NOT_STARTED, live};
use super::{Admit, Bucket, Engine, FailCause, FailKind, Gate, NodeId, PlanItem};
use std::time::Duration;

impl Engine {
    /// **Private.** Token `k` of request `r` reaches a `Hold` for `(node, rep)` covering `size` more
    /// items: takes the node's rate-limit token and a slot. `true` when the slot is held now and the
    /// token stands past the marker; `false` when it queued or the request failed here.
    pub(super) fn enter_hold(
        &mut self,
        r: usize,
        k: usize,
        node: NodeId,
        rep: u16,
        size: usize,
    ) -> bool {
        let now = self.now;
        let (idx, counted) = {
            let req = live(&mut self.reqs, r);
            (req.tokens[k].at, req.arrival >= self.warmup)
        };
        if let Some(b) = self.units.state(node, rep).bucket.as_mut()
            && let Err(wait) = b.take(now)
        {
            if counted {
                self.count(node, rep, |c| c.limited += 1);
            }
            let status = self
                .units
                .state(node, rep)
                .bucket
                .as_ref()
                .map_or(429, Bucket::status);
            self.fail_or_retry(
                r,
                Failure {
                    at: now,
                    idx,
                    node,
                    status,
                    kind: FailKind::Status,
                    retry_after: Some(Duration::from_nanos(wait)),
                    refused: counted.then_some((node, rep, true)),
                    cause: FailCause::RateLimit,
                },
            );
            return false;
        }
        if self.units.state(node, rep).gate.is_none() {
            // nothing to hold (the walk only plans a hold for a node with slots)
            live(&mut self.reqs, r).tokens[k].at += 1;
            return true;
        }
        self.mark(node, now);
        let gate = self.units.state(node, rep).gate.as_mut().expect("checked");
        match gate.arrive((r, k)) {
            Admit::Run => {
                self.take_hold(r, k, node, rep, size);
                true
            }
            Admit::Queued => {
                let len = gate.queued();
                if now >= self.warmup {
                    self.count(node, rep, |c| c.max_queue = c.max_queue.max(len));
                }
                let req = live(&mut self.reqs, r);
                req.busy += 1;
                req.tokens[k].queued_at = Some((node, rep, now));
                false
            }
            Admit::Overflow => {
                let status = gate.overflow_status();
                if counted {
                    self.count(node, rep, |c| c.overflowed += 1);
                }
                self.fail_or_retry(
                    r,
                    Failure {
                        at: now,
                        idx,
                        node,
                        status,
                        kind: FailKind::Status,
                        retry_after: None,
                        refused: counted.then_some((node, rep, false)),
                        cause: FailCause::Overflow,
                    },
                );
                false
            }
        }
    }

    /// **Private.** The slot of `(node, rep)` is now held by token `k`: record the scope and stand
    /// the token past its marker.
    fn take_hold(&mut self, r: usize, k: usize, node: NodeId, rep: u16, size: usize) {
        let now = self.now;
        let req = live(&mut self.reqs, r);
        let at = req.tokens[k].at;
        let id = req.holds.len();
        req.holds.push(HoldState {
            parent: req.tokens[k].hold_top,
            owner: k,
            end: at + 1 + size,
            node,
            replica: rep,
            open: true,
            since: now,
            until: now,
        });
        req.tokens[k].hold_top = Some(id);
        req.tokens[k].at = at + 1;
        req.starts[at] = now;
    }

    /// **Private.** The queued token `(qr, qk)` got the slot of its `Hold` now: record its wait (shown
    /// on the node's first own hop in the scope), take the hold and let the token go on.
    pub(super) fn grant_hold(
        &mut self,
        qr: usize,
        qk: usize,
        (node, rep): (NodeId, u16),
        wait: u64,
    ) {
        let counted = {
            let req = live(&mut self.reqs, qr);
            let i = req.tokens[qk].at;
            if matches!(req.plan.items.get(i + 1), Some(PlanItem::Hop(_))) {
                req.waits[i + 1] = wait;
            }
            req.busy -= 1;
            req.arrival >= self.warmup
        };
        if counted {
            self.count(node, rep, |c| c.wait += u128::from(wait));
        }
        let size = {
            let req = live(&mut self.reqs, qr);
            let i = req.tokens[qk].at;
            let PlanItem::Hold { size, .. } = req.plan.items[i] else {
                unreachable!("queued at a hold")
            };
            size
        };
        self.take_hold(qr, qk, node, rep, size);
        self.advance(qr, qk);
    }

    /// **Private.** Whether token `k` of request `r` holds the slot of `(node, rep)` now, so a hop of
    /// that node runs inside it instead of taking one.
    pub(super) fn holds_unit(&mut self, r: usize, k: usize, node: NodeId, rep: u16) -> bool {
        let req = live(&mut self.reqs, r);
        let mut cur = req.tokens[k].hold_top;
        while let Some(i) = cur {
            let h = &req.holds[i];
            if h.open && h.node == node && h.replica == rep {
                return true;
            }
            cur = h.parent;
        }
        false
    }

    /// **Private.** Lets go of every hold token `k` has walked past (innermost first), handing each
    /// slot to the head of its queue.
    pub(super) fn close_holds(&mut self, r: usize, k: usize) {
        loop {
            let freed = {
                let req = live(&mut self.reqs, r);
                let at = req.tokens[k].at;
                let Some(i) = req.tokens[k].hold_top else {
                    return;
                };
                let h = &req.holds[i];
                if h.owner != k || h.end > at {
                    return;
                }
                let (open, unit, parent) = (h.open, (h.node, h.replica), h.parent);
                req.holds[i].open = false;
                req.holds[i].until = self.now;
                req.tokens[k].hold_top = parent;
                open.then_some(unit)
            };
            if let Some((node, rep)) = freed {
                self.free_unit(node, rep);
            }
        }
    }

    /// **Private.** Request `r` failed: frees every slot it holds except those of a unit where a hop
    /// of it is still running (that one frees its slot when it ends, as a zombie hop always has).
    /// Called when the request fails and again as each hop of a failed request ends.
    pub(super) fn release_idle_holds(&mut self, r: usize) {
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
            let orphans = &req.orphans;
            req.holds
                .iter_mut()
                .filter(|x| {
                    x.open
                        && !running.contains(&(x.node, x.replica))
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

    /// **Private.** Frees whatever request `r` still holds: its table slot is being freed.
    pub(super) fn drop_holds(&mut self, r: usize) {
        let now = self.now;
        let open: Vec<(NodeId, u16)> = {
            let req = live(&mut self.reqs, r);
            req.holds
                .iter_mut()
                .filter(|h| h.open)
                .map(|h| {
                    h.open = false;
                    h.until = now;
                    (h.node, h.replica)
                })
                .collect()
        };
        for (node, rep) in open {
            self.free_unit(node, rep);
        }
    }

    /// **Private.** One slot of `(node, rep)` is free: the oldest waiter, if any, gets it at once (no
    /// barging, as in `hop_done`); otherwise it is idle.
    pub(super) fn free_unit(&mut self, node: NodeId, rep: u16) {
        let now = self.now;
        self.mark(node, now);
        let next = self
            .units
            .state(node, rep)
            .gate
            .as_mut()
            .and_then(Gate::release);
        if let Some((qr, qk)) = next {
            self.start_queued(qr, qk);
        }
    }
}
