//! Calls and the load balancer's pick, split out of `walk.rs`: one `call:` step run as a method,
//! and the choice of a callee for a [`Step::Route`].

use std::time::Duration;

use super::Op;
use super::walk::{Flow, Walk, step_path, takes_branch};
use crate::balance::pick_replica;
use crate::chain::{Call, Route};
use crate::stable_hash;
use crate::topology::NodeId;

impl Walk<'_> {
    /// **Private.** Runs one `call:` step from `*now`: the network hop, the callee's body, the
    /// response, `repeat` times in sequence. Moves `*now` to the end; `Some` is the failure that
    /// ended the request.
    pub(super) fn call_step(
        &mut self,
        call: &Call,
        path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        // The call carries its own sizes when it has them (`sends`, `returns`); the callee's body
        // and its own calls see them, and the caller's go back once the call is over.
        let saved = self.bytes;
        self.bytes = (
            call.sends.unwrap_or(saved.0),
            call.returns.unwrap_or(saved.1),
        );
        let flow = self.call_inner(call, path, op, now, run);
        self.bytes = saved;
        flow
    }

    /// **Private.** [`Walk::call_step`] with the call's sizes already in `bytes`.
    fn call_inner(
        &mut self,
        call: &Call,
        path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        let topo = self.topo;
        let chaos = self.chaos;
        let target = topo.nodes[call.node].name.as_str();
        // One draw per step: a skipped call has no hop, span or repeat.
        if !takes_branch(call.prob, path, target, call.key.occ) {
            return None;
        }
        let op = call.op.unwrap_or(op);
        for rep in 0..call.repeat {
            let rep_path = step_path(path, target, call.key.occ, rep);
            let Some(r) = self.pick(call.node, call.balance.as_ref(), rep_path) else {
                return Some(self.no_replica(call.node, *now));
            };
            let span = self.open_span(call.node, *now);
            let edge = topo.edge_id(run.0, call.node);
            let lc = edge.and_then(|e| chaos.link(e));
            let cap = lc.and_then(|l| l.bandwidth_cap);
            if let Some(k) = lc.and_then(|l| self.link_drop(l, call.node, rep_path)) {
                let fail = self.link_fail(k, call.node, r, *now);
                self.close_span(span);
                return Some(fail);
            }
            let net = self.net_sample(call.node, &call.net, rep_path, lc);
            let mut wire = self.wire((edge, cap), net, ((call.node, r), run), rep_path);
            *now += self.wire_connect(&mut wire, *now);
            // an upload streams into the callee, so the request's transfer runs beside it (RULES 76)
            let mode = self.stream_mode(call, op);
            let upload = mode.request() && self.can_stream(edge, (run.0, call.node), false, cap);
            if !upload {
                *now += self.wire_send(&mut wire, *now);
            }
            *now += self.net_hop(call.node, net, (*now, r));
            let up = upload.then(|| self.begin_upload(&mut wire, *now));
            let stream = (mode.response() && self.can_stream(edge, (call.node, run.0), true, cap))
                .then(|| self.begin_stream());
            let t0 = *now;
            let hold = self.open_hold(call.node, r);
            let flow = self.seq(&call.body, rep_path, op, *now, (call.node, r));
            self.close_hold(hold);
            let done = if let Some(fork) = stream {
                self.end_stream(fork, &flow, &mut wire, t0)
            } else if let Flow::Done(end) | Flow::Hit(end) = flow {
                Some(end + self.wire_back(&mut wire, end))
            } else {
                None
            };
            let sent = up.as_ref().map(|u| u.sent);
            if let Some(u) = &up {
                self.end_upload(u);
            }
            if let Some(end) = done {
                *now = end.max(sent.unwrap_or(Duration::ZERO));
                *now += self.wire_stalls(&mut wire, *now);
            }
            self.close_span(span);
            match flow {
                Flow::Done(_) | Flow::Hit(_) => {}
                fail @ Flow::Fail { .. } => return Some(fail),
            }
        }
        None
    }

    /// **Private.** The wire out of a balancer a call takes now: the pure [`pick_replica`] over
    /// its callees by `spread`, a callee with every copy down skipped. The round-robin counter
    /// is the balancer's own (a balancer with copies keeps its replica picks apart), and the
    /// draw is tagged with its own name so it does not repeat the replica pick. `None` when every
    /// callee is down.
    pub(super) fn pick_callee<'c>(&mut self, r: &'c Route, path: u64) -> Option<&'c Call> {
        let lb = &self.topo.nodes[r.node];
        let k = u64::from(self.routes[r.node]);
        self.routes[r.node] += 1;
        let tag = format!("{}#spread", lb.name);
        let offset = stable_hash(&[self.ep_key.as_bytes(), tag.as_bytes()]).wrapping_add(k);
        let up: Vec<bool> = r
            .calls
            .iter()
            .map(|c| {
                self.chaos.is_empty()
                    || (0..self.topo.nodes[c.node].replicas)
                        .any(|rep| !self.chaos.replica_down(c.node, self.ep_key, rep))
            })
            .collect();
        let count = u16::try_from(r.calls.len()).unwrap_or(u16::MAX);
        // each copy of the balancer round-robins over only the calls it gets, one in `replicas`;
        // counting requests in step with the copy pick would lock a callee to a copy
        let n = self.n / u64::from(lb.replicas.max(1));
        let i = pick_replica(&r.spread, count, &up, n, offset, path, &tag)?;
        r.calls.get(usize::from(i))
    }
}
