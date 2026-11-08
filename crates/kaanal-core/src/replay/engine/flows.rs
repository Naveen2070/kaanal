//! Transfers sharing pipes fairly (RULES 62, step 6a): a transfer hop with an [`Xfer`] joins the
//! solver in `fluid.rs` instead of queueing on a pipe. The engine keeps one pending event, for the
//! flow that ends first; every join or leave can move it, so each pending event carries a
//! generation and a stale one is skipped. A transfer's end shifts as others join and leave, so the
//! time it lost to sharing is recorded as the hop's `wait`.

use std::cmp::Reverse;

use super::fluid::Fluid;
use super::state::live;
use super::{Engine, PlanItem, nanos};
use crate::exec::Xfer;

/// **Private.** The solver and who is waiting on each flow.
pub(super) struct Flows {
    fluid: Fluid,
    /// `(request slot, token)` of each flow, by the flow's slab index.
    owner: Vec<Option<(usize, usize)>>,
    /// When the live pending event fires (ns). An event for a later time than a newer one is left
    /// in the heap and ignored; one that fires with nothing due just re-arms.
    due: Option<u64>,
    /// The tokens whose transfers just ended (kept so a finishing event allocates nothing).
    done: Vec<(usize, usize)>,
    /// Which pending event is the current one.
    epoch: usize,
}

impl Flows {
    pub(super) fn new(caps: Vec<u64>) -> Flows {
        Flows {
            fluid: Fluid::new(caps),
            owner: Vec::new(),
            done: Vec::new(),
            due: None,
            epoch: 0,
        }
    }

    pub(super) fn set_window(&mut self, lo: u64, hi: u64) {
        self.fluid.set_window(lo, hi);
    }
}

impl Engine {
    /// **Private.** Token `k` of request `r` starts the transfer `x` now: it joins the solver and
    /// the pending event is re-armed.
    pub(super) fn join_flow(&mut self, r: usize, k: usize, x: &Xfer) {
        let mut pipes = [0usize; 3];
        let mut used = 0;
        for pipe in x.pipes() {
            pipes[used] = self.units.pipe_unit(pipe);
            used += 1;
        }
        let now = self.now;
        let id = self.flows.fluid.add(now, x.bytes, &pipes[..used], x.limit);
        let i = Fluid::index(id);
        if self.flows.owner.len() <= i {
            self.flows.owner.resize(i + 1, None);
        }
        self.flows.owner[i] = Some((r, k));
        self.rearm();
    }

    /// **Private.** The pending event fired: if it is the current one, every flow due now ends and
    /// its token goes on, as at the end of any hop.
    pub(super) fn flow_event(&mut self, epoch: usize) {
        if epoch != self.flows.epoch {
            return;
        }
        let now = self.now;
        self.flows.due = None;
        let mut done = std::mem::take(&mut self.flows.done);
        while let Some((t, id)) = self.flows.fluid.next_finish()
            && t <= now
        {
            let (pipes, n) = self.flows.fluid.finish(now, id);
            self.sync_pipes(&pipes[..n]);
            if let Some(who) = self.flows.owner[Fluid::index(id)].take() {
                done.push(who);
            }
        }
        for (r, k) in done.drain(..) {
            self.transfer_done(r, k);
        }
        self.flows.done = done;
        self.rearm();
    }

    /// **Private.** A transfer (or a hop sharing CPUs, `cpu.rs`) ended: record the time sharing
    /// cost it, then end the hop.
    pub(super) fn transfer_done(&mut self, r: usize, k: usize) {
        let now = self.now;
        let warmup = self.warmup;
        let req = live(&mut self.reqs, r);
        let i = req.tokens[k].at;
        let PlanItem::Hop(h) = &req.plan.items[i] else {
            unreachable!("a flow belongs to a hop")
        };
        let lost = (now - req.starts[i]).saturating_sub(nanos(h.dur));
        let counted = req.arrival >= warmup;
        let (node, rep) = self.units.target(h);
        req.waits[i] = lost;
        if counted {
            self.count(node, rep, |c| c.wait += u128::from(lost));
        }
        self.hop_done(r, k);
    }

    /// **Private.** Makes sure an event is pending at or before the time the next flow ends. A
    /// pending event that is earlier is kept: it fires with nothing due and re-arms.
    fn rearm(&mut self) {
        let Some((t, _)) = self.flows.fluid.next_finish() else {
            return;
        };
        let t = t.max(self.now);
        if self.flows.due.is_some_and(|d| d <= t) {
            return;
        }
        self.flows.due = Some(t);
        self.flows.epoch += 1;
        let seq = self.stamp();
        self.heap
            .push(Reverse((t, seq, usize::MAX, self.flows.epoch)));
    }

    /// **Private.** Sets the busy area of the given pipes to what they carried: the nano-bytes
    /// moved over the rate is the time a pipe would have been busy at full rate. Done for the
    /// pipes of a flow that ended, so once a pipe goes idle its counter is complete.
    fn sync_pipes(&mut self, pipes: &[usize]) {
        for &p in pipes {
            let (node, rep) = self.units.owner(p);
            let (moved, cap) = (self.flows.fluid.moved(p), self.units.cap(p));
            // nano-bytes fit 64 bits for any run under 18 GB through the pipe, and that division is
            // several times quicker
            let area =
                u64::try_from(moved).map_or(moved / u128::from(cap), |m| u128::from(m / cap));
            let old =
                std::mem::replace(&mut self.units.counters[node][usize::from(rep)].area, area);
            self.counters[node].area = (self.counters[node].area + area).saturating_sub(old);
        }
    }
}
