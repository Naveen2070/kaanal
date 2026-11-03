//! Replicas in the engine: every replica of a node has its own slots, queue, pool and rate limit.
//!
//! The state lives in one flat table (`Units`), indexed by `first[node] + replica`, so a node with
//! one replica costs what it always did. Counters are kept twice: per node, summed over its
//! replicas (what `NodeStats` reports and R1.48(5) froze), and per replica for the Studio report.
//! A deadline belongs to a call, not a replica, so `timed_out` is per node only.

use super::state::{NodeCounters, NodeState};
use super::{Bucket, Engine, Gate, NodeId, Topology};
use crate::exec::{Hop, HopKind};
use crate::topology::Admission;

/// **Private.** The queueing state of every replica of every node.
pub(super) struct Units {
    /// Index in `states` of replica 0 of each node.
    first: Vec<usize>,
    /// Replicas of each node.
    count: Vec<u16>,
    /// Gate, bucket and clock of each replica, node by node.
    pub(super) states: Vec<NodeState>,
    /// Per-replica counters: `counters[node][replica]`.
    pub(super) counters: Vec<Vec<NodeCounters>>,
    /// Number of real nodes: the units after them are one pseudo-node per edge (the link), then
    /// two per node (its send and receive pipe, in that order).
    pub(super) nodes: usize,
    /// Number of edges (links).
    pub(super) edges: usize,
}

impl Units {
    /// Every replica free, every bucket full.
    pub(super) fn new(topo: &Topology) -> Units {
        let mut first = Vec::with_capacity(topo.nodes.len());
        let mut states = Vec::new();
        for n in &topo.nodes {
            first.push(states.len());
            for _ in 0..n.replicas.max(1) {
                states.push(NodeState {
                    gate: n.admission.as_ref().map(Gate::new),
                    bucket: n.rate_limit.as_ref().map(Bucket::new),
                    since: 0,
                });
            }
        }
        let mut count: Vec<u16> = topo.nodes.iter().map(|n| n.replicas.max(1)).collect();
        let mut counters: Vec<Vec<NodeCounters>> = count
            .iter()
            .map(|&c| vec![NodeCounters::default(); usize::from(c)])
            .collect();
        // One FIFO single-slot unit per edge, with a gate only when the edge has a bandwidth.
        let link = Admission {
            slots: 1,
            queue: None,
            overflow_status: 503,
        };
        for e in &topo.edges {
            first.push(states.len());
            states.push(NodeState {
                gate: e.bandwidth.map(|_| Gate::new(&link)),
                bucket: None,
                since: 0,
            });
            count.push(1);
            counters.push(vec![NodeCounters::default()]);
        }
        // Then a send and a receive pipe per node, one per replica, gated only when the node has
        // a bandwidth that way.
        for n in &topo.nodes {
            for rate in [n.bandwidth.0, n.bandwidth.1] {
                first.push(states.len());
                for _ in 0..n.replicas.max(1) {
                    states.push(NodeState {
                        gate: rate.map(|_| Gate::new(&link)),
                        bucket: None,
                        since: 0,
                    });
                }
                count.push(n.replicas.max(1));
                counters.push(vec![
                    NodeCounters::default();
                    usize::from(n.replicas.max(1))
                ]);
            }
        }
        Units {
            first,
            nodes: topo.nodes.len(),
            edges: topo.edges.len(),
            count,
            states,
            counters,
        }
    }

    /// Index in `states` of `replica` of `node`.
    pub(super) fn at(&self, node: NodeId, replica: u16) -> usize {
        debug_assert!(
            replica < self.count[node],
            "replica {replica} of a node with {} replicas",
            self.count[node]
        );
        self.first[node] + usize::from(replica)
    }

    /// The replica of `node` and its state.
    pub(super) fn state(&mut self, node: NodeId, replica: u16) -> &mut NodeState {
        let i = self.at(node, replica);
        &mut self.states[i]
    }
}

impl Engine {
    /// Counters of each replica of each node, `[node][replica]`; the node's own totals are
    /// [`Engine::node_counters`] (their sum, `max_queue` the longest queue of any replica).
    pub fn replica_counters(&self) -> &[Vec<NodeCounters>] {
        &self.units.counters[..self.units.nodes]
    }

    /// Counters of each bandwidth-limited link, indexed by [`crate::EdgeId`]: `completed` is
    /// transfers started, `wait` their summed queueing, `area` the busy time. An edge without a
    /// `bandwidth` stays zero.
    pub fn link_counters(&self) -> &[NodeCounters] {
        &self.counters[self.units.nodes..self.units.nodes + self.units.edges]
    }

    /// Counters of each node's network pipes, `[2 * node]` the send pipe and `[2 * node + 1]`
    /// the receive pipe (summed over its replicas). A node without a bandwidth that way stays
    /// zero.
    pub fn nic_counters(&self) -> &[NodeCounters] {
        &self.counters[self.units.nodes + self.units.edges..]
    }

    /// **Private.** Applies `f` to the node's totals and to the counters of its `replica`.
    pub(super) fn count(&mut self, node: NodeId, replica: u16, f: impl Fn(&mut NodeCounters)) {
        f(&mut self.counters[node]);
        f(&mut self.units.counters[node][usize::from(replica)]);
    }

    /// **Private.** Adds `busy x (time since the last change, clipped to the window)` to the
    /// `area` of every replica of `node`, and to the node's total. Call just before `busy`
    /// changes; extra calls change nothing (additive).
    pub(super) fn mark(&mut self, node: NodeId, now: u64) {
        let (warmup, end) = (self.warmup, self.end);
        let first = self.units.first[node];
        for r in 0..usize::from(self.units.count[node]) {
            let ns = &mut self.units.states[first + r];
            let busy = ns.gate.as_ref().map_or(0, Gate::busy);
            let (from, to) = (ns.since.max(warmup), now.min(end));
            let area = u128::from(busy) * u128::from(to.saturating_sub(from));
            ns.since = now;
            self.units.counters[node][r].area += area;
            self.counters[node].area += area;
        }
    }
}

impl Units {
    /// The unit whose gate, bucket and counters hop `h` uses: its node's replica, or for a link
    /// hop the edge's pseudo-node (index `nodes + edge`, replica 0).
    pub(super) fn target(&self, h: &Hop) -> (NodeId, u16) {
        match h.kind {
            HopKind::Link { edge, .. } => (self.nodes + usize::from(edge), 0),
            HopKind::Nic { send, .. } => (
                self.nodes + self.edges + 2 * h.node + usize::from(!send),
                h.replica,
            ),
            _ => (h.node, h.replica),
        }
    }

    /// Whether the unit of `h` has neither a gate nor a rate limit (its hops start at once).
    pub(super) fn unconstrained(&self, h: &Hop) -> bool {
        let (node, replica) = self.target(h);
        let s = &self.states[self.at(node, replica)];
        s.gate.is_none() && s.bucket.is_none()
    }
}
