//! Streamed responses (RULES 71, step 6c): a response whose bytes cross the wire while the callee is
//! still producing them. The callee's work and the transfer become the two branches of an ordinary
//! [`PlanItem::Fork`] (branch A the callee, branch B a [`HopKind::FirstByte`] wait and then the
//! response link hop), so the engine needs nothing new: the fork ends with its slower branch, which
//! is the pipeline limit `first byte + size / slowest stage` (Kleinrock, Queueing Systems vol. 2,
//! on cut-through; the bottleneck law of a pipeline), and 6a's fair sharing sees the transfer while
//! it is really on the wire. A failing callee cancels the transfer like any failing sibling.

use std::time::Duration;

use super::walk::{Flow, Walk};
use super::{Hop, HopKind, Op, PlanItem};
use crate::chain::Call;
use crate::config::NodeKind;
use crate::topology::{EdgeId, NodeId};

/// **Private.** What the walk remembers of the last own hop it emitted: whose it was, when it ended
/// and how much of its length was the size-dependent part (`per_kb`, a store's `throughput`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Tail {
    pub(super) node: NodeId,
    pub(super) end: Duration,
    pub(super) size: Duration,
}

impl Walk<'_> {
    /// **Private.** Whether the response of `call` streams: its own or its edge's `stream`, else the
    /// default of the callee (a read of a storage node streams, as S3, GCS and Azure send a GET as
    /// it is read). Nothing streams with no response bytes or no pipe with a rate (nothing to
    /// overlap), so a topology without bandwidth makes the same plans as before.
    pub(super) fn streams(
        &self,
        call: &Call,
        op: Op,
        edge: Option<EdgeId>,
        run: NodeId,
        cap: Option<u64>,
    ) -> bool {
        let on = call.stream.unwrap_or_else(|| {
            self.topo.nodes[call.node].kind == NodeKind::Storage && op == Op::Read
        });
        on && self.bytes.1.saturating_add(self.topo.header_bytes.1) > 0
            && self.path_rate(edge, (call.node, run), cap).is_some()
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
    #[expect(
        clippy::too_many_arguments,
        reason = "one call's context; a struct would only rename them"
    )]
    pub(super) fn end_stream(
        &mut self,
        (fi, bi): (usize, usize),
        flow: &Flow,
        at: (NodeId, u16),
        run: (NodeId, u16),
        edge: Option<EdgeId>,
        cap: Option<u64>,
        t0: Duration,
    ) -> Option<Duration> {
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
            Some(t) if t.node == at.0 && t.end == end => end.saturating_sub(t.size),
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
        let moved = self.link(edge, (at, run), true, first, cap);
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
