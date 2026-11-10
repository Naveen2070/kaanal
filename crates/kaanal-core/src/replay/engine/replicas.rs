//! Replicas in the engine: every replica of a node has its own slots, queue, pool and rate limit.
//!
//! The state lives in one flat table (`Units`), indexed by `first[node] + replica`, so a node with
//! one replica costs what it always did. Counters are kept twice: per node, summed over its
//! replicas (what `NodeStats` reports and R1.48(5) froze), and per replica for the Studio report.
//! A deadline belongs to a call, not a replica, so `timed_out` is per node only.

use super::state::{NodeCounters, NodeState};
use super::{Bucket, Engine, Gate, NodeId, Topology};
use crate::exec::Pipe;
use crate::exec::{Hop, HopKind};

/// **Private.** Most copies a node can have (config limit); a live engine allocates all of them.
pub(super) const MAX_COPIES: u16 = 64;

/// **Private.** Units to allocate for `n`'s copies.
fn copies(n: &crate::topology::Node, headroom: bool) -> u16 {
    if headroom {
        MAX_COPIES
    } else {
        n.replicas.max(1)
    }
}

/// **Private.** The admission of a node with no limit, for a live engine: every node there has a
/// gate (so a limit added later finds the running work counted), this one never makes anyone wait.
const UNLIMITED: crate::topology::Admission = crate::topology::Admission {
    slots: u32::MAX,
    queue: None,
    overflow_status: 503,
};

/// **Private.** The gate of one copy of `n`: its own, or in a live engine an unlimited one.
fn gate_of(n: &crate::topology::Node, headroom: bool) -> Option<Gate> {
    match &n.admission {
        Some(a) => Some(Gate::new(a)),
        None if headroom => Some(Gate::new(&UNLIMITED)),
        None => None,
    }
}

/// **Private.** The admission a live engine applies for `n`: its own, or no limit at all.
pub(super) fn admission_of(n: &crate::topology::Node) -> crate::topology::Admission {
    n.admission.clone().unwrap_or(UNLIMITED)
}

/// **Private.** The queueing state of every replica of every node.
pub(super) struct Units {
    /// Index in `states` of replica 0 of each node.
    pub(super) first: Vec<usize>,
    /// Replicas of each node.
    pub(super) count: Vec<u16>,
    /// Gate, bucket and clock of each replica, node by node.
    pub(super) states: Vec<NodeState>,
    /// Per-replica counters: `counters[node][replica]`.
    pub(super) counters: Vec<Vec<NodeCounters>>,
    /// Number of real nodes: the units after them are one pseudo-node per edge (the link), then
    /// two per node (its send and receive pipe, in that order).
    pub(super) nodes: usize,
    /// Number of edges (links).
    pub(super) edges: usize,
    /// The rate of every unit as a pipe (bytes a second; 1 for a unit that is not a rated pipe),
    /// indexed like `states`: the capacities the fair-sharing solver starts from.
    caps: Vec<u64>,
    /// The pseudo-node and copy that unit `i` of `states` belongs to.
    owners: Vec<(NodeId, u16)>,
}

impl Units {
    /// Every replica free, every bucket full. With `headroom` every node gets `MAX_COPIES` units
    /// whatever its `replicas`, so a live change of the copy count never moves an index.
    pub(super) fn new(topo: &Topology, headroom: bool) -> Units {
        let mut first = Vec::with_capacity(topo.nodes.len());
        let mut states = Vec::new();
        for n in &topo.nodes {
            first.push(states.len());
            for _ in 0..copies(n, headroom) {
                states.push(NodeState {
                    gate: gate_of(n, headroom),
                    bucket: n
                        .rate_limit
                        .as_ref()
                        .map(|r| Bucket::new(r).with_status(n.kind.rate_limit_status())),
                    cpu: n.cpu.map(|c| Box::new(super::cpu::CpuUnit::new(c))),
                    since: 0,
                    oom: n.oom_restart.map(super::state::nanos),
                    down_until: 0,
                    gc: super::state::Gc::of(n.memory.as_ref()),
                });
            }
        }
        let mut count: Vec<u16> = topo.nodes.iter().map(|n| copies(n, headroom)).collect();
        let mut counters: Vec<Vec<NodeCounters>> = count
            .iter()
            .map(|&c| vec![NodeCounters::default(); usize::from(c)])
            .collect();
        // One pseudo-node per edge for its wire. A pipe has no gate: transfers share its rate
        // fairly (`fluid.rs`, RULES 62) instead of queueing.
        for _ in &topo.edges {
            first.push(states.len());
            states.push(NodeState {
                gate: None,
                bucket: None,
                cpu: None,
                since: 0,
                oom: None,
                down_until: 0,
                gc: None,
            });
            count.push(1);
            counters.push(vec![NodeCounters::default()]);
        }
        // Then a send and a receive pipe per node, one per replica.
        for n in &topo.nodes {
            for _ in 0..2 {
                first.push(states.len());
                for _ in 0..copies(n, headroom) {
                    states.push(NodeState {
                        gate: None,
                        bucket: None,
                        cpu: None,
                        since: 0,
                        oom: None,
                        down_until: 0,
                        gc: None,
                    });
                }
                count.push(copies(n, headroom));
                counters.push(vec![
                    NodeCounters::default();
                    usize::from(copies(n, headroom))
                ]);
            }
        }
        let mut u = Units {
            first,
            nodes: topo.nodes.len(),
            edges: topo.edges.len(),
            count,
            states,
            counters,
            caps: Vec::new(),
            owners: Vec::new(),
        };
        u.caps = u.pipe_caps(topo);
        u.owners = (0..u.states.len())
            .map(|i| {
                let node = u.first.partition_point(|&f| f <= i) - 1;
                let rep = u16::try_from(i - u.first[node]).expect("at most u16::MAX copies");
                (node, rep)
            })
            .collect();
        u
    }

    /// The rate of every unit as a pipe (bytes a second; 1 for a unit that is not a rated pipe),
    /// indexed like `states`: the capacities the fair-sharing solver starts from.
    fn pipe_caps(&self, topo: &Topology) -> Vec<u64> {
        let mut caps = vec![1; self.states.len()];
        for (e, edge) in topo.edges.iter().enumerate() {
            if let Some(bw) = edge.bandwidth {
                caps[self.at(self.nodes + e, 0)] = bw.max(1);
            }
        }
        for (n, node) in topo.nodes.iter().enumerate() {
            for (side, rate) in [node.bandwidth.0, node.bandwidth.1].into_iter().enumerate() {
                for rep in 0..self.count[n] {
                    if let Some(bw) = rate {
                        caps[self.at(self.nodes + self.edges + 2 * n + side, rep)] = bw.max(1);
                    }
                }
            }
        }
        caps
    }

    /// The unit index of a pipe (the solver's pipe id).
    pub(super) fn pipe_unit(&self, p: Pipe) -> usize {
        match p {
            Pipe::Wire { edge } => self.at(self.nodes + usize::from(edge), 0),
            Pipe::Send { node, replica } => self.at(self.nodes + self.edges + 2 * node, replica),
            Pipe::Recv { node, replica } => {
                self.at(self.nodes + self.edges + 2 * node + 1, replica)
            }
        }
    }

    /// The pseudo-node and replica whose counters belong to unit `i` of `states`.
    pub(super) fn owner(&self, i: usize) -> (NodeId, u16) {
        self.owners[i]
    }

    pub(super) fn caps(&self) -> &[u64] {
        &self.caps
    }

    /// The rate of unit `i` as a pipe (bytes a second).
    pub(super) fn cap(&self, i: usize) -> u64 {
        self.caps[i]
    }

    /// Sets the rate of unit `i` (a live parameter edit), the figure `sync_pipes` divides by.
    pub(super) fn set_cap(&mut self, i: usize, bytes_per_s: u64) {
        self.caps[i] = bytes_per_s.max(1);
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
