//! Compiling chaos layers into [`EffectiveChaos`]: the per-key resolution a visit reads.
//! Moved out of `mod.rs` unchanged (R3.8).

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use super::{ChaosKey, ChaosLayer, ChaosState, ErrorKind, MAX_LATENCY_ADD, reps_sorted};
use crate::topology::{EdgeId, NodeId};

/// Most failure bands one visit can carry; further faults fold into the last band.
pub const MAX_FAULTS: usize = 8;

/// One failure band: a roll below `upto` (cumulative) fails this way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fault {
    /// Cumulative probability bound of this band.
    pub upto: f64,
    /// How the failure shows up.
    pub kind: ErrorKind,
    /// HTTP status for a `status` failure.
    pub status: u16,
}

/// Chaos compiled for one visit (or one endpoint front door): no allocation to read.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// Extra latency, at most [`MAX_LATENCY_ADD`].
    pub latency_add: Duration,
    /// Hard outage: every visit fails with 503.
    pub down: bool,
    /// Link only: up to this much random extra latency either way on the `net` hop.
    pub jitter: Duration,
    /// Link only: caps the edge's bandwidth (bytes per second).
    pub bandwidth_cap: Option<u64>,
    faults: [Fault; MAX_FAULTS],
    len: u8,
}

impl Resolved {
    fn empty() -> Self {
        Resolved {
            latency_add: Duration::ZERO,
            down: false,
            jitter: Duration::ZERO,
            bandwidth_cap: None,
            faults: [Fault {
                upto: 0.0,
                kind: ErrorKind::Status,
                status: 503,
            }; MAX_FAULTS],
            len: 0,
        }
    }

    /// The failure bands, top layer first.
    #[must_use]
    pub fn faults(&self) -> &[Fault] {
        &self.faults[..self.len as usize]
    }

    /// Total failure probability (the last band's bound), 0.0 when none.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.faults().last().map_or(0.0, |f| f.upto)
    }

    /// The failure a roll in 0..1 lands on; `down` always fails with 503.
    #[must_use]
    pub fn hit(&self, roll: f64) -> Option<(ErrorKind, u16)> {
        if self.down {
            return Some((ErrorKind::Status, 503));
        }
        self.faults()
            .iter()
            .find(|f| roll < f.upto)
            .map(|f| (f.kind, f.status))
    }

    fn push(&mut self, r: f64, kind: ErrorKind, status: u16) {
        let n = self.len as usize;
        let upto = if n == 0 {
            r
        } else {
            let u = self.faults[n - 1].upto;
            u + r - u * r
        };
        if n < MAX_FAULTS {
            self.faults[n] = Fault { upto, kind, status };
            self.len += 1;
        } else {
            self.faults[n - 1].upto = upto; // fold the tail into the last band
        }
    }
}

/// (rank, specificity, layer)
type Entry<'a> = (usize, u8, &'a ChaosLayer);

/// `link`: the entries are of an edge, whose failures default to `reset` instead of a status.
fn resolve_entries(mut entries: Vec<Entry<'_>>, link: bool) -> Resolved {
    entries.sort_by_key(|e| std::cmp::Reverse((e.0, e.1))); // highest layer first, then most specific
    let mut out = Resolved::empty();
    for (_, _, l) in entries {
        if let Some(d) = l.latency_add {
            out.latency_add = out.latency_add.saturating_add(d);
        }
        if let Some(j) = l.jitter {
            out.jitter = out.jitter.max(j);
        }
        if let Some(c) = l.bandwidth_cap {
            out.bandwidth_cap = Some(out.bandwidth_cap.map_or(c, |o| o.min(c)));
        }
        if l.down == Some(true) {
            out.down = true;
        }
        if let Some(r) = l.error_rate
            && r > 0.0
        {
            out.push(
                r,
                l.error_kind.unwrap_or(if link {
                    ErrorKind::Reset
                } else {
                    ErrorKind::Status
                }),
                l.error_status.unwrap_or(503),
            );
        }
    }
    out.latency_add = out.latency_add.min(MAX_LATENCY_ADD);
    out.jitter = out.jitter.min(MAX_LATENCY_ADD);
    out
}

/// Entries of one key across the layers, truncated at the last `replace`.
fn entries_of<'a>(
    layers: &[&'a ChaosState],
    spec: u8,
    get: impl Fn(&'a ChaosState) -> Option<&'a ChaosLayer>,
) -> Vec<Entry<'a>> {
    let mut v: Vec<Entry<'a>> = layers
        .iter()
        .enumerate()
        .filter_map(|(i, s)| get(s).map(|l| (i, spec, l)))
        .collect();
    if let Some(p) = v.iter().rposition(|e| e.2.replace == Some(true)) {
        v.drain(..p);
    }
    v
}

/// Every key's chaos, compiled from the layer stack. Read-only on the hot path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectiveChaos {
    nodes: HashMap<NodeId, Resolved>,
    node_at: HashMap<NodeId, HashMap<String, Resolved>>,
    endpoints: HashMap<String, Resolved>,
    /// A replica entry: the node's own entry folded with the replica's.
    replicas: HashMap<(NodeId, u16), Resolved>,
    /// The same through one endpoint: also folds in the node-at-endpoint entry.
    replica_at: HashMap<(NodeId, u16), HashMap<String, Resolved>>,
    /// One edge's entry (link chaos).
    edges: HashMap<EdgeId, Resolved>,
}

impl EffectiveChaos {
    /// True when nothing is set (the executor's fast path).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.node_at.is_empty()
            && self.endpoints.is_empty()
            && self.replicas.is_empty()
            && self.edges.is_empty()
    }

    /// The chaos for a visit of `node` through `endpoint`: the node-at-endpoint entry (which
    /// already folds in the node's own), else the node entry. The endpoint-wide entry is not
    /// here: it applies once at the front door.
    #[must_use]
    pub fn visit(&self, node: NodeId, endpoint: &str) -> Option<&Resolved> {
        self.node_at
            .get(&node)
            .and_then(|m| m.get(endpoint))
            .or_else(|| self.nodes.get(&node))
    }

    /// [`EffectiveChaos::visit`] for one `replica` of `node`: its replica entry (which folds in the
    /// node's own entry and, through `endpoint`, the node-at-endpoint entry), else the plain visit.
    #[must_use]
    pub fn visit_replica(&self, node: NodeId, endpoint: &str, replica: u16) -> Option<&Resolved> {
        self.replica_at
            .get(&(node, replica))
            .and_then(|m| m.get(endpoint))
            .or_else(|| self.replicas.get(&(node, replica)))
            .or_else(|| self.visit(node, endpoint))
    }

    /// Whether `replica` of `node` is down for a visit through `endpoint`: its own entry, or the
    /// node's (a node that is down takes every replica with it).
    #[must_use]
    pub fn replica_down(&self, node: NodeId, endpoint: &str, replica: u16) -> bool {
        self.visit_replica(node, endpoint, replica)
            .is_some_and(|r| r.down)
    }

    /// The chaos on the link `edge`, if any.
    #[must_use]
    pub fn link(&self, edge: EdgeId) -> Option<&Resolved> {
        self.edges.get(&edge)
    }

    /// The endpoint-wide entry, if any.
    #[must_use]
    pub fn endpoint(&self, endpoint: &str) -> Option<&Resolved> {
        self.endpoints.get(endpoint)
    }

    /// Every entry with its key, sorted like [`ChaosState::entries`].
    #[must_use]
    pub fn entries(&self) -> Vec<(ChaosKey, &Resolved)> {
        let mut nodes: Vec<_> = self.nodes.iter().collect();
        nodes.sort_by_key(|(n, _)| **n);
        let mut eps: Vec<_> = self.endpoints.iter().collect();
        eps.sort_by(|a, b| a.0.cmp(b.0));
        let mut at: Vec<_> = self
            .node_at
            .iter()
            .flat_map(|(n, m)| m.iter().map(move |(e, l)| ((*n, e), l)))
            .collect();
        at.sort_by(|a, b| a.0.cmp(&b.0));
        let nodes = nodes.into_iter().map(|(n, l)| (ChaosKey::Node(*n), l));
        let eps = eps
            .into_iter()
            .map(|(e, l)| (ChaosKey::Endpoint(e.clone()), l));
        let at = at
            .into_iter()
            .map(|((n, e), l)| (ChaosKey::NodeAt(n, e.clone()), l));
        let reps = reps_sorted(&self.replicas)
            .into_iter()
            .map(|((n, r), l)| (ChaosKey::Replica(n, r), l));
        let mut edges: Vec<_> = self.edges.iter().collect();
        edges.sort_by_key(|(e, _)| **e);
        let edges = edges.into_iter().map(|(e, l)| (ChaosKey::Edge(*e), l));
        nodes
            .chain(eps)
            .chain(at)
            .chain(reps)
            .chain(edges)
            .collect()
    }
}

/// Compiles layers (bottom to top) into one [`EffectiveChaos`]. Per key, the higher layer's
/// fault takes the first band (layer order beats specificity; within a layer the more specific
/// key goes first), latencies add (then clamp to [`MAX_LATENCY_ADD`]), `down` is an OR, and
/// a layer with `replace` drops the lower layers' contribution to that key. A node override
/// also drops the lower layers' entries of that node's node-at-endpoint pairs.
#[must_use]
pub fn compile_layers(layers: &[&ChaosState]) -> EffectiveChaos {
    let mut node_ids = BTreeSet::new();
    let mut eps = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    let mut reps = BTreeSet::new();
    let mut edge_ids = BTreeSet::new();
    for s in layers {
        node_ids.extend(s.nodes.keys().copied());
        eps.extend(s.endpoints.keys().cloned());
        reps.extend(s.replicas.keys().copied());
        edge_ids.extend(s.edges.keys().copied());
        for (n, m) in &s.node_at {
            pairs.extend(m.keys().map(|e| (*n, e.clone())));
        }
    }
    let mut out = EffectiveChaos::default();
    for id in node_ids {
        let e = entries_of(layers, 0, |s| s.nodes.get(&id));
        out.nodes.insert(id, resolve_entries(e, false));
    }
    for id in edge_ids {
        let e = entries_of(layers, 0, |s| s.edges.get(&id));
        out.edges.insert(id, resolve_entries(e, true));
    }
    for ep in eps {
        let e = entries_of(layers, 0, |s| s.endpoints.get(&ep));
        out.endpoints.insert(ep, resolve_entries(e, false));
    }
    // A replica folds in its node's own entry (and, per endpoint, the node-at-endpoint entry),
    // so one lookup serves a visit; a node override calms the replica entries of lower layers.
    for (n, r) in reps {
        let floor = layers
            .iter()
            .rposition(|s| s.nodes.get(&n).is_some_and(|l| l.replace == Some(true)))
            .unwrap_or(0);
        let mut own = entries_of(layers, 2, |s| s.replicas.get(&(n, r)));
        own.retain(|x| x.0 >= floor);
        let mut e = entries_of(layers, 0, |s| s.nodes.get(&n));
        e.extend(own.iter().copied());
        out.replicas.insert((n, r), resolve_entries(e, false));
        for (_, ep) in pairs.iter().filter(|(pn, _)| *pn == n) {
            let mut e = entries_of(layers, 0, |s| s.nodes.get(&n));
            let mut at = entries_of(layers, 1, |s| s.node_at.get(&n).and_then(|m| m.get(ep)));
            at.retain(|x| x.0 >= floor);
            e.extend(at);
            e.extend(own.iter().copied());
            out.replica_at
                .entry((n, r))
                .or_default()
                .insert(ep.clone(), resolve_entries(e, false));
        }
    }
    for (n, ep) in pairs {
        let mut e = entries_of(layers, 0, |s| s.nodes.get(&n));
        // A node override also calms the node's pairs from lower layers; a pair override
        // leaves the node-level entries alone.
        let floor = layers
            .iter()
            .rposition(|s| s.nodes.get(&n).is_some_and(|l| l.replace == Some(true)))
            .unwrap_or(0);
        let mut at = entries_of(layers, 1, |s| s.node_at.get(&n).and_then(|m| m.get(&ep)));
        at.retain(|x| x.0 >= floor);
        e.extend(at);
        out.node_at
            .entry(n)
            .or_default()
            .insert(ep, resolve_entries(e, false));
    }
    out
}
