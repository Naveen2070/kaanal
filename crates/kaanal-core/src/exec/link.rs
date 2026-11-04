//! Payload and chaos on a link (an edge) in the walk: transfer hops, jitter and extra latency on
//! the `Net` hop, and dropped or partitioned calls. Split out of `walk.rs` (R3.8).

use std::time::Duration;

use rand::Rng;

use super::walk::{Walk, draw_rng};
use super::{Hop, HopKind, Pipe, PlanItem, Xfer, hang_cap};
use crate::chaos::{ErrorKind, FailKind, Resolved};
use crate::topology::{EdgeId, NodeId};

/// **Private.** Longest single payload cost (a transfer or a `per_kb` surcharge): 10 h, like a
/// sampled hop, so a chain's sum stays far from `Duration`'s limit.
pub(super) const MAX_HOP: Duration = Duration::from_hours(10);

impl Walk<'_> {
    /// **Private.** A payload crossing `edge` from `from` to `to` (each a node and replica) as
    /// one transfer, pipelined: `bytes / rate` at the narrowest of the sender's send pipe, the
    /// wire and the receiver's receive pipe, capped at 10 h. The hop is owned by that
    /// narrowest pipe (the wire on a tie, then the sender); `xfer` names every pipe, and the
    /// engine shares each fairly between the transfers on it (RULES 62). Nothing (zero, no hop) when no pipe
    /// has a rate or there are no bytes this way. A link chaos `bandwidth_cap` (`cap`) lowers the
    /// wire's rate.
    pub(super) fn link(
        &mut self,
        edge: Option<EdgeId>,
        (from, to): ((NodeId, u16), (NodeId, u16)),
        resp: bool,
        start: Duration,
        cap: Option<u64>,
    ) -> Duration {
        let bytes = if resp { self.bytes.1 } else { self.bytes.0 };
        if bytes == 0 {
            return Duration::ZERO;
        }
        let edge_rate = edge.and_then(|e| self.topo.edges[e].bandwidth);
        let wire = edge_rate.map(|bw| cap.map_or(bw, |c| c.min(bw)).max(1));
        let (send, recv) = (
            self.topo.nodes[from.0].bandwidth.0,
            self.topo.nodes[to.0].bandwidth.1,
        );
        // (rate, the hop's owner and kind, the pipe); the first narrowest wins a tie.
        let mut pipes = Vec::with_capacity(3);
        if let (Some(e), Some(bw)) = (edge, wire) {
            let edge = u16::try_from(e).expect("a topology has at most u16::MAX edges");
            // the wire belongs to the edge's target (the callee), whichever way the body goes
            let callee = if resp { from } else { to };
            pipes.push((
                bw,
                callee,
                HopKind::Link { edge, resp },
                Pipe::Wire { edge },
            ));
        }
        if let Some(bw) = send {
            let pipe = Pipe::Send {
                node: from.0,
                replica: from.1,
            };
            pipes.push((bw, from, HopKind::Nic { send: true, resp }, pipe));
        }
        if let Some(bw) = recv {
            let pipe = Pipe::Recv {
                node: to.0,
                replica: to.1,
            };
            pipes.push((bw, to, HopKind::Nic { send: false, resp }, pipe));
        }
        let Some(&(bw, (node, replica), kind, _)) = pipes.iter().min_by_key(|p| p.0) else {
            return Duration::ZERO;
        };
        let ns =
            (u128::from(bytes) * 1_000_000_000 / u128::from(bw.max(1))).min(MAX_HOP.as_nanos());
        let dur = Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX));
        let mut used = [None; 3];
        for (slot, p) in used.iter_mut().zip(&pipes) {
            *slot = Some(p.3);
        }
        self.xfers.push((
            self.items.len(),
            Xfer {
                bytes,
                pipes: used,
                // the chaos cap lowers the wire's share of this transfer only
                limit: edge_rate.and(cap).map(|c| c.max(1)),
            },
        ));
        self.items.push(PlanItem::Hop(Hop {
            node,
            replica,
            kind,
            start,
            dur,
        }));
        dur
    }

    /// **Private.** `dur` with a link's chaos on it: its `latency_add`, then jitter, uniform in
    /// plus or minus `jitter` from its own draw (none when there is no jitter), never below zero.
    pub(super) fn shake(
        &self,
        dur: Duration,
        lc: Option<&Resolved>,
        path: u64,
        to: NodeId,
    ) -> Duration {
        let Some(l) = lc else { return dur };
        let mut d = dur + l.latency_add;
        if !l.jitter.is_zero() {
            let u: f64 = draw_rng(path, &self.topo.nodes[to].name, b"jitter").random();
            let shift = l.jitter.as_secs_f64() * (2.0 * u - 1.0);
            d = Duration::from_secs_f64((d.as_secs_f64() + shift).max(0.0));
        }
        d
    }

    /// **Private.** Whether the link drops this call: a partition (`down`) is a hang; otherwise
    /// one roll from its own stream (only when the link has a failure rate) picks the band.
    /// `Reset` fails fast, `Hang` waits; a `status` or `malformed` band cannot be set on a link
    /// but would count as a reset.
    pub(super) fn link_drop(&self, l: &Resolved, to: NodeId, path: u64) -> Option<ErrorKind> {
        if l.down {
            return Some(ErrorKind::Hang);
        }
        if l.rate() <= 0.0 {
            return None;
        }
        let roll: f64 = draw_rng(path, &self.topo.nodes[to].name, b"link").random();
        l.hit(roll).map(|(k, _)| k)
    }

    /// **Private.** A call the link dropped, as it ends at `now`: a reset is a 502 `Fail` at once;
    /// a hang (or partition) is a `Net` hop of the callee's `timeout` (else [`HANG_CAP`]) and
    /// then a 504 `Fail` at its end, the same shape as a node hang (Z6). A `Net` hop, not a
    /// `Link` hop: a dead link carries nothing, so it must not take a share of the link.
    pub(super) fn link_fail(
        &mut self,
        kind: ErrorKind,
        to: NodeId,
        replica: u16,
        now: Duration,
    ) -> super::walk::Flow {
        let mut at = now;
        let (status, fk) = if kind == ErrorKind::Hang {
            let cap = hang_cap(self.topo.nodes[to].timeout);
            self.items.push(PlanItem::Hop(Hop {
                node: to,
                replica,
                kind: HopKind::Net,
                start: now,
                dur: cap,
            }));
            at += cap;
            (504, FailKind::Timeout)
        } else {
            (502, FailKind::Reset)
        };
        let idx = self.items.len();
        self.items.push(PlanItem::Fail {
            node: to,
            status,
            kind: fk,
            at,
        });
        super::walk::Flow::Fail { at, idx }
    }
}

impl Walk<'_> {
    /// **Private.** A cache whose link dropped the lookup is a miss (Z17: a cache never fails the
    /// request): at once for a reset, after the node's `timeout` (else [`HANG_CAP`]) for a hang or
    /// a partition. Records the miss hop and returns how long it took.
    pub(super) fn cache_miss_on_drop(
        &mut self,
        kind: ErrorKind,
        node: NodeId,
        (start, replica): (Duration, u16),
    ) -> Duration {
        let dur = if kind == ErrorKind::Hang {
            hang_cap(self.topo.nodes[node].timeout)
        } else {
            Duration::ZERO
        };
        self.items.push(PlanItem::Hop(Hop {
            node,
            replica,
            kind: HopKind::CacheMiss,
            start,
            dur,
        }));
        dur
    }
}
