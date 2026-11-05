//! What one call's wire costs, in one place (RULES 68, 71, 74): the connection set-up of a new
//! connection (6b), the body transfer (6a, 6e), and the retransmits a lossy link adds (6d). A
//! [`Wire`] is made once per call from the edge, the sampled round trip and the call's path; its
//! methods push the hops in order, so `call_inner` and the cache step stay readable.

use std::time::Duration;

use rand::Rng;

use super::conn::{self, MSS};
use super::link::MAX_HOP;
use super::loss;
use super::walk::{Walk, draw_rng};
use super::{Hop, HopKind, PlanItem};
use crate::config::Conn;
use crate::rng::HopRng;
use crate::topology::{EdgeId, NodeId};

/// **Private.** One call's wire: who talks to whom over which edge, the round trip of this call
/// (`net`), whether it opens a new connection, and the call's own loss stream (only on a lossy edge
/// with a round trip).
pub(super) struct Wire {
    pub(super) edge: Option<EdgeId>,
    pub(super) cap: Option<u64>,
    pub(super) net: Duration,
    fresh: Option<Conn>,
    loss: Option<HopRng>,
    /// The callee and its copy.
    pub(super) at: (NodeId, u16),
    /// The caller and its copy.
    pub(super) run: (NodeId, u16),
}

impl Walk<'_> {
    /// **Private.** The wire of one call: its connection decision (one `conn` draw in `keepalive`)
    /// and its loss stream (tag `loss`), both skipped when there is no round trip to pay in.
    pub(super) fn wire(
        &self,
        (edge, cap): (Option<EdgeId>, Option<u64>),
        net: Duration,
        (at, run): ((NodeId, u16), (NodeId, u16)),
        path: u64,
    ) -> Wire {
        let fresh = self.opens_connection(edge, net, at.0, path);
        let lossy = !net.is_zero() && edge.is_some_and(|e| self.topo.edges[e].loss > 0.0);
        Wire {
            edge,
            cap,
            net,
            fresh,
            loss: lossy.then(|| draw_rng(path, &self.topo.nodes[at.0].name, b"loss")),
            at,
            run,
        }
    }

    /// **Private.** The loss probability of the wire's edge (0 when off).
    fn loss_p(&self, w: &Wire) -> f64 {
        w.edge.map_or(0.0, |e| self.topo.edges[e].loss)
    }

    /// **Private.** Before the request: the new connection's `Connect` hop (handshake round trips,
    /// lost SYNs, the request's slow-start stalls); zero for a call on a warm connection.
    pub(super) fn wire_connect(&mut self, w: &mut Wire, start: Duration) -> Duration {
        let Some(conn) = w.fresh else {
            return Duration::ZERO;
        };
        // each handshake packet (two a round trip) is lost with p; a lost SYN waits 1 s
        let packets = u64::from(conn::handshake_rtts(conn.tls)) * 2;
        let lost = self.loss_p(w);
        let syns = w
            .loss
            .as_mut()
            .map_or(0, |r| loss::losses(packets, lost, r.random()));
        let syn_wait = loss::SYN_RTO.saturating_mul(syns);
        // the handshake round trips, plus the slow-start stalls of the request body
        let bytes = self.wire_bytes(w.edge, false);
        let rate = self.path_rate(w.edge, (w.run.0, w.at.0), w.cap);
        let rtts = conn::handshake_rtts(conn.tls) + conn::stalls(bytes, rate, w.net);
        self.push_connect(false, w.net.saturating_mul(rtts) + syn_wait, w.at, start)
    }

    /// **Private.** The request's body crossing the wire, then the retransmits a lossy link adds.
    pub(super) fn wire_send(&mut self, w: &mut Wire, start: Duration) -> Duration {
        let moved = self.link(w.edge, (w.run, w.at), false, start, w.cap);
        moved + self.retransmits(w, false, start + moved)
    }

    /// **Private.** The response's body crossing the wire back, then the retransmits.
    pub(super) fn wire_back(&mut self, w: &mut Wire, start: Duration) -> Duration {
        let moved = self.link(w.edge, (w.at, w.run), true, start, w.cap);
        moved + self.retransmits(w, true, start + moved)
    }

    /// **Private.** After the response: its slow-start stalls on a new connection.
    pub(super) fn wire_stalls(&mut self, w: &mut Wire, start: Duration) -> Duration {
        if w.fresh.is_none() {
            return Duration::ZERO;
        }
        self.connect_response(w.net, w.edge, (w.run.0, w.at), start, w.cap)
    }

    /// **Private.** What loss adds to one transfer, as a `Retransmit` hop (nothing when the edge is
    /// lossless, there is no round trip, or no bytes move). A short flow draws its lost segments
    /// (`Binomial(n, p)`): each costs one round trip (fast retransmit) or, with the chance a small
    /// flight gives it, the retransmission timeout. A long flow (over 32 expected losses) runs at the
    /// RFC 5348 rate instead, and the hop is its surplus over the lossless time (RULES 74).
    fn retransmits(&mut self, w: &mut Wire, resp: bool, start: Duration) -> Duration {
        // the common case, a lossless wire: nothing to compute (the hot path)
        if w.loss.is_none() {
            return Duration::ZERO;
        }
        let p = self.loss_p(w);
        let bytes = self.wire_bytes(w.edge, resp);
        let Some(rng) = w.loss.as_mut().filter(|_| bytes > 0) else {
            return Duration::ZERO;
        };
        let segments = bytes.div_ceil(MSS);
        let ends = if resp {
            (w.at.0, w.run.0)
        } else {
            (w.run.0, w.at.0)
        };
        let rate = self.path_rate(w.edge, ends, w.cap);
        let extra = if loss::long_flow(segments, p) {
            let x = loss::tfrc_rate(w.net, p);
            match rate {
                Some(r) if r <= x => Duration::ZERO,
                _ => {
                    let at = |rate: u64| ns_for(bytes, rate);
                    at(x).saturating_sub(rate.map_or(Duration::ZERO, at))
                }
            }
        } else {
            let lost = loss::losses(segments, p, rng.random());
            // the flight a loss happens in: a body of a few segments is all window; a long one is
            // limited by the bandwidth-delay product (at least the first flight of 10)
            let bdp = rate.map_or(segments, |r| {
                u64::try_from(u128::from(r) * w.net.as_nanos() / 1_000_000_000 / u128::from(MSS))
                    .unwrap_or(u64::MAX)
            });
            let window = segments.min(bdp.max(10));
            let timeout = loss::timeout_chance(window);
            let mut total = Duration::ZERO;
            for _ in 0..lost {
                total += if rng.random::<f64>() < timeout {
                    loss::rto(w.net)
                } else {
                    w.net
                };
            }
            total
        }
        .min(MAX_HOP);
        if extra.is_zero() {
            return extra;
        }
        self.items.push(PlanItem::Hop(Hop {
            node: w.at.0,
            replica: w.at.1,
            kind: HopKind::Retransmit { resp },
            start,
            dur: extra,
        }));
        extra
    }
}

/// **Private.** The time `bytes` take at `rate` bytes a second, in whole nanoseconds, capped.
fn ns_for(bytes: u64, rate: u64) -> Duration {
    let ns = (u128::from(bytes) * 1_000_000_000 / u128::from(rate.max(1))).min(MAX_HOP.as_nanos());
    Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
}
