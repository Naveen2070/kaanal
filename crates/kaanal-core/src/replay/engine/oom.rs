//! Killing a copy for lack of memory (step 8K, RULES 112). With `memory: { on_limit: kill }` a
//! request the memory cannot hold does not wait or get a 503: it kills the copy, as the kernel's
//! OOM killer kills a process.
//!
//! - **The kill.** Every request that is serving or queued on the copy fails with 502 (cause
//!   [`FailCause::Oom`]), through the same `fail_or_retry` as any failure, so a retry policy retries
//!   it. The copy is down until `now + restart`.
//! - **While it is down** every hop that reaches it is refused with 503 (the same cause).
//! - **The copy is emptied at the kill**, as a real process's memory is: the gate is reset (every
//!   slot free, the queue gone), the `hold: call` slots on it are closed and what ran on its CPUs
//!   stops. A killed request's hop is marked (`Token::killed`) and, when its time was a timer that
//!   is already in the heap, ends then; it releases nothing, so it can never free a slot a new
//!   request holds. Back up, the copy is empty whatever the length of the hops that were on it.
//! - The other copies of the node carry on. A balancer chooses its copy when the plan is sampled
//!   and does not know the copy died, so it keeps sending some calls there.
//!
//! Cost: one `bool` check on the path of a topology with no `on_limit: kill`; the scan of the
//! request table runs only at a kill.

use super::state::{Failure, NOT_STARTED};
use super::{Engine, FailCause, FailKind, NodeId, PlanItem};

impl Engine {
    /// **Private.** `demand` ns of the hop's own work, stretched for the garbage collection of a copy
    /// that sets a `gc` cost (step 8G): the live requests are the slots busy as the hop starts, this
    /// one included. Unchanged for everything else, at the price of one `bool`.
    pub(super) fn gc_stretch(&mut self, h: &crate::exec::Hop, demand: u64) -> u64 {
        if !self.has.gc
            || !matches!(
                h.kind,
                crate::exec::HopKind::Proc | crate::exec::HopKind::Op(_)
            )
        {
            return demand;
        }
        let ns = self.units.state(h.node, h.replica);
        match (ns.gc, ns.gate.as_ref()) {
            (Some(gc), Some(g)) => gc.stretch(u64::from(g.busy()), demand),
            _ => demand,
        }
    }

    /// **Private.** Whether any node dies of memory: the only cost of the feature on other paths.
    pub(super) fn any_oom(topo: &crate::topology::Topology) -> bool {
        topo.nodes.iter().any(|n| n.oom_restart.is_some())
    }

    /// **Private.** Whether copy `rep` of `node` is down from a kill now.
    pub(super) fn copy_down(&self, node: NodeId, rep: u16) -> bool {
        self.has.oom && self.units.states[self.units.at(node, rep)].down_until > self.now
    }

    /// **Private.** Whether copy `rep` of `node` is down from a kill; if so request `r`'s hop at `idx`
    /// is refused with 503 and the caller stops.
    pub(super) fn oom_down(&mut self, r: usize, idx: usize, node: NodeId, rep: u16) -> bool {
        if !self.has.oom || self.units.state(node, rep).down_until <= self.now {
            return false;
        }
        let at = self.now;
        self.fail_or_retry(
            r,
            Failure {
                at,
                idx,
                node,
                status: 503,
                kind: FailKind::Status,
                retry_after: None,
                refused: None,
                cause: FailCause::Oom,
            },
        );
        true
    }

    /// **Private.** The gate refused request `r`'s hop at `idx`. On a node that dies of memory
    /// that is the kill, and the caller stops; otherwise `false` and the usual refusal follows.
    pub(super) fn oom_overflow(&mut self, r: usize, idx: usize, node: NodeId, rep: u16) -> bool {
        let Some(restart) = self.units.state(node, rep).oom else {
            return false;
        };
        self.oom_kill(r, idx, node, rep, restart);
        true
    }

    /// **Private.** Request `r`'s hop at `idx` could not be held by the copy's memory: the copy is
    /// killed. `r` and every request serving or queued on it fail with 502; the copy is down for
    /// its `restart`.
    pub(super) fn oom_kill(&mut self, r: usize, idx: usize, node: NodeId, rep: u16, restart: u64) {
        let now = self.now;
        let counted = self.reqs[r]
            .as_ref()
            .is_some_and(|q| q.arrival >= self.warmup);
        self.units.state(node, rep).down_until = now.saturating_add(restart);
        if counted {
            self.count(node, rep, |c| c.oom_kills += 1);
        }
        let victims: Vec<(usize, usize)> = (0..self.reqs.len())
            .filter(|&v| v != r)
            .filter_map(|v| self.touching(v, node, rep).map(|i| (v, i)))
            .collect();
        // empty the copy before anyone fails, so no failure hands a slot on or releases one
        self.mark(node, now);
        for &(v, _) in &victims {
            self.cut_running(v, node, rep);
        }
        let running_on_cpu = self.empty_copy(node, rep);
        for (v, i) in victims {
            self.fail_or_retry(v, self.killed(i, node));
            self.settle(v);
        }
        // hops that were sharing the CPUs end now, having lost their slot with the copy (before
        // the arriving request fails: ending a hop can answer and free other requests)
        for (qr, qk) in running_on_cpu {
            self.transfer_done(qr, qk);
        }
        let f = self.killed(idx, node);
        self.fail_or_retry(r, f);
    }

    /// **Private.** Marks request `v`'s running hops on the copy as cut, and closes its open
    /// `hold: call` slots there (the gate is emptied as a whole, so nothing is handed on).
    fn cut_running(&mut self, v: usize, node: NodeId, rep: u16) {
        let now = self.now;
        let Some(req) = self.reqs[v].as_mut() else {
            return;
        };
        for k in 0..req.tokens.len() {
            let t = &req.tokens[k];
            let running = t.at < t.end
                && t.queued_at.is_none()
                && req.starts[t.at] != NOT_STARTED
                && matches!(&req.plan.items[t.at], PlanItem::Hop(h) if !h.kind.is_wait()
                    && self.units.target(h) == (node, rep));
            if running {
                req.tokens[k].killed = true;
            }
        }
        for h in &mut req.holds {
            if h.open && h.node == node && h.replica == rep {
                h.open = false;
                h.until = now;
            }
        }
    }

    /// **Private.** Every slot of the copy free, nobody waiting, nothing running on its CPUs.
    /// Returns the hops that were sharing the CPUs.
    fn empty_copy(&mut self, node: NodeId, rep: u16) -> Vec<(usize, usize)> {
        let now = self.now;
        let i = self.units.at(node, rep);
        let ns = &mut self.units.states[i];
        if let Some(g) = ns.gate.as_mut() {
            g.reset();
        }
        let who = ns.cpu.as_deref_mut().map_or_else(Vec::new, |c| c.kill(now));
        self.cpu_due.remove(&i);
        who
    }

    /// **Private.** The failure of a request the kill cut short, at plan index `idx`.
    fn killed(&self, idx: usize, node: NodeId) -> Failure {
        Failure {
            at: self.now,
            idx,
            node,
            status: 502,
            kind: FailKind::Status,
            retry_after: None,
            refused: None,
            cause: FailCause::Oom,
        }
    }

    /// **Private.** The plan index at which live request `v` is serving or queued on copy `rep` of
    /// `node` (a running hop, a queue place or an open `hold: call` slot), or `None`. A request
    /// already failed or answered is not touched again.
    fn touching(&self, v: usize, node: NodeId, rep: u16) -> Option<usize> {
        let req = self.reqs[v].as_ref()?;
        if req.fail.is_some() || req.done.is_some() {
            return None;
        }
        for t in &req.tokens {
            if t.queued_at.is_some_and(|(n, p, _)| n == node && p == rep) {
                return Some(t.at);
            }
            if t.at < t.end
                && let PlanItem::Hop(h) = &req.plan.items[t.at]
                && !h.kind.is_wait()
                && req.starts[t.at] != NOT_STARTED
                && self.units.target(h) == (node, rep)
            {
                return Some(t.at);
            }
        }
        req.holds
            .iter()
            .find(|h| h.open && h.node == node && h.replica == rep)
            .map(|h| req.tokens[h.owner].at)
    }
}
