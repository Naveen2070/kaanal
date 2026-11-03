//! The copy a request is sent to when the balance is `least-conn` or `p2c`: chosen when the token
//! reaches the plan's [`PlanItem::Pick`], from the live gates, the way a balancer decides at the
//! moment it forwards. The plan names the candidates; the later hops carry the pick's code and are
//! bound to the chosen copy here, so every other part of the engine sees an ordinary copy.

use std::sync::Arc;

use super::state::live;
use super::{Engine, NodeId, PickKind, PlanItem};

impl Engine {
    /// **Private.** Requests in copy `rep` of `node` now: busy slots plus those waiting for one. A
    /// node with no slot limit has no gate and counts 0, so a choice among such copies is the tie.
    fn copy_load(&self, node: NodeId, rep: u16) -> usize {
        self.units.states[self.units.at(node, rep)]
            .gate
            .as_ref()
            .map_or(0, |g| g.busy() as usize + g.queued())
    }

    /// **Private.** Token `k` of request `r` reaches the `Pick` at its position: choose the copy,
    /// bind the later hops to it, step past the marker. `least-conn` takes the least loaded
    /// candidate, the tied ones taking turns by the offset; `p2c` the less loaded of its two, the
    /// first on a tie.
    pub(super) fn dispatch(&mut self, r: usize, k: usize) {
        let at = live(&mut self.reqs, r).tokens[k].at;
        let PlanItem::Pick {
            node,
            kind,
            cands,
            offset,
            ..
        } = live(&mut self.reqs, r).plan.items[at]
        else {
            unreachable!("dispatch is called at a Pick")
        };
        let ranked: Vec<(usize, u16)> = (0..64u16)
            .filter(|i| cands >> i & 1 == 1)
            .map(|i| (self.copy_load(node, i), i))
            .collect();
        let least = ranked.iter().map(|(l, _)| *l).min().unwrap_or(0);
        let ties: Vec<u16> = ranked
            .iter()
            .filter(|(l, _)| *l == least)
            .map(|(_, i)| *i)
            .collect();
        let copy = match kind {
            PickKind::LeastConn => ties[usize::from(offset) % ties.len()],
            PickKind::P2c => ties[0],
        };
        let req = live(&mut self.reqs, r);
        // a plan shared by several requests is copied once, for this one
        Arc::make_mut(&mut req.plan).bind_pick(at, copy);
        req.tokens[k].at = at + 1;
    }
}
