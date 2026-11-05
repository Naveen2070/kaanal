//! Streamed responses (RULES 71, step 6c): a response whose bytes cross the wire while the callee is
//! still producing them. The callee's work and the transfer become the two branches of an ordinary
//! [`PlanItem::Fork`] (branch A the callee, branch B a [`HopKind::FirstByte`] wait and then the
//! response link hop), so the engine needs nothing new: the fork ends with its slower branch, which
//! is the pipeline limit `first byte + size / slowest stage` (Kleinrock, Queueing Systems vol. 2,
//! on cut-through; the bottleneck law of a pipeline), and 6a's fair sharing sees the transfer while
//! it is really on the wire. A failing callee cancels the transfer like any failing sibling.

use std::time::Duration;

use super::walk::{Flow, Walk};
use super::wire::Wire;
use super::{Hop, HopKind, Op, PlanItem};
use crate::chain::Call;
use crate::config::{NodeKind, StreamMode};
use crate::topology::{EdgeId, NodeId};

/// **Private.** What the walk remembers of the last own hop it emitted: whose it was, when it ended
/// and how much of its length was the size-dependent part (`per_kb`, a store's `throughput`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Tail {
    pub(super) node: NodeId,
    pub(super) start: Duration,
    pub(super) dur: Duration,
    /// The fixed part of `dur`; the rest is the size-dependent part. Kept raw: only a streamed call
    /// reads it, so the hot path does no arithmetic on it.
    pub(super) fixed: Duration,
}

/// **Private.** An upload in progress: its fork and branch B in the plan, and when the request
/// finishes crossing the wire.
pub(super) struct Upload {
    fork: usize,
    branch: usize,
    pub(super) sent: Duration,
}

impl Walk<'_> {
    /// **Private.** Which bodies of `call` stream: its own or its edge's `stream`, else the default
    /// of the callee: a storage read streams its response and a storage write its request, as S3,
    /// GCS and Azure send a GET as it is read and take a PUT as it arrives.
    #[inline]
    pub(super) fn stream_mode(&self, call: &Call, op: Op) -> StreamMode {
        call.stream.unwrap_or_else(|| {
            match (self.topo.nodes[call.node].kind == NodeKind::Storage, op) {
                (true, Op::Read) => StreamMode::Response,
                (true, Op::Write) => StreamMode::Request,
                _ => StreamMode::None,
            }
        })
    }

    /// **Private.** Whether a body that `mode` says streams has anything to overlap: it has bytes on
    /// the wire and a pipe with a rate between `ends` (`resp` picks the direction). Nothing streams
    /// without either, so a topology without bandwidth makes the same plans as before.
    #[inline]
    pub(super) fn can_stream(
        &self,
        edge: Option<EdgeId>,
        ends: (NodeId, NodeId),
        resp: bool,
        cap: Option<u64>,
    ) -> bool {
        self.wire_bytes(edge, resp) > 0 && self.path_rate(edge, ends, cap).is_some()
    }

    /// **Private.** Opens an upload's fork and its branch A, the request crossing the wire (with
    /// its retransmits), starting at `start`; branch B, the callee, is opened and left for the walk
    /// to fill. The callee consumes the bytes as they arrive, so both start together (RULES 76).
    pub(super) fn begin_upload(&mut self, wire: &mut Wire, start: Duration) -> Upload {
        let fork = self.items.len();
        self.items.push(PlanItem::Fork {
            branches: 2,
            size: 0,
        });
        let a = self.items.len();
        self.items.push(PlanItem::Branch { size: 0 });
        let sent = start + self.wire_send(wire, start);
        self.items[a] = PlanItem::Branch {
            size: self.items.len() - a - 1,
        };
        let branch = self.items.len();
        self.items.push(PlanItem::Branch { size: 0 });
        Upload { fork, branch, sent }
    }

    /// **Private.** Closes branch B and the upload's fork; the callee's call ends at the later of
    /// its own end and `up.sent` (the caller of this, who knows the flow, takes the maximum).
    pub(super) fn end_upload(&mut self, up: &Upload) {
        self.items[up.branch] = PlanItem::Branch {
            size: self.items.len() - up.branch - 1,
        };
        self.items[up.fork] = PlanItem::Fork {
            branches: 2,
            size: self.items.len() - up.fork - 1,
        };
    }

    /// **Private.** Opens the fork and branch A (the callee's work); returns their plan indices.
    pub(super) fn begin_stream(&mut self) -> (usize, usize) {
        let fi = self.items.len();
        self.items.push(PlanItem::Fork {
            branches: 2,
            size: 0,
        });
        let bi = self.items.len();
        self.items.push(PlanItem::Branch { size: 0 });
        (fi, bi)
    }

    /// **Private.** Closes branch A, and for a callee that answered adds branch B: the wait for the
    /// first byte, then the response transfer, started at that instant. The first byte is when the
    /// callee's final own hop has done its fixed part (the end of the callee's sequence less that
    /// hop's size-dependent part); a callee whose sequence does not end in its own hop (it calls
    /// something after) cannot send early, so the transfer starts at its end, as buffered. Returns
    /// when the fork ends, `None` for a failed callee (which has no response: the fork keeps only
    /// branch A and the failure goes up as usual).
    pub(super) fn end_stream(
        &mut self,
        (fi, bi): (usize, usize),
        flow: &Flow,
        wire: &mut Wire,
        t0: Duration,
    ) -> Option<Duration> {
        let at = wire.at;
        self.items[bi] = PlanItem::Branch {
            size: self.items.len() - bi - 1,
        };
        let (Flow::Done(end) | Flow::Hit(end)) = *flow else {
            self.items[fi] = PlanItem::Fork {
                branches: 1,
                size: self.items.len() - fi - 1,
            };
            return None;
        };
        let first = match self.tail {
            Some(t) if t.node == at.0 && t.start + t.dur == end => {
                end.saturating_sub(t.dur.saturating_sub(t.fixed))
            }
            _ => end,
        };
        let b = self.items.len();
        self.items.push(PlanItem::Branch { size: 0 });
        let wait = first.saturating_sub(t0);
        if !wait.is_zero() {
            self.items.push(PlanItem::Hop(Hop {
                node: at.0,
                replica: at.1,
                kind: HopKind::FirstByte,
                start: t0,
                dur: wait,
            }));
        }
        let moved = self.wire_back(wire, first);
        self.items[b] = PlanItem::Branch {
            size: self.items.len() - b - 1,
        };
        self.items[fi] = PlanItem::Fork {
            branches: 2,
            size: self.items.len() - fi - 1,
        };
        Some(end.max(first + moved))
    }
}
