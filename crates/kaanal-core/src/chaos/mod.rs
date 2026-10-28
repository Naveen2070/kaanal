//! Live chaos state: the failure knobs flipped at runtime via the control API.
//!
//! Nothing here comes from `kaanal.yaml`; this is runtime state only. Written by
//! `PUT/DELETE /__kaanal/chaos...`, read by the chain executor on every request.
//!
//! Chaos is stored as sparse [`ChaosLayer`]s (only the fields a caller set) under a
//! [`ChaosKey`] (a node, an endpoint, or a node at one endpoint). The executor resolves the
//! layers into an [`EffectiveChaos`] (see [`compile_layers`]) once, so a visit is one lookup.

use std::collections::HashMap;
use std::time::Duration;

use crate::topology::{EdgeId, NodeId};

/// Longest extra latency chaos may add to a node (5 min). A typo like 1e12 ms would
/// otherwise hang every request (live) or overflow the virtual clock (simulator).
pub const MAX_LATENCY_ADD: Duration = Duration::from_secs(300);

/// Longest a `hang` failure holds a visit before the caller gives up (1 min).
pub const HANG_CAP: Duration = Duration::from_secs(60);

/// How an injected error shows up on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorKind {
    /// An HTTP error response with `error_status`.
    #[default]
    Status,
    /// The connection is reset.
    Reset,
    /// The response never arrives (capped by [`HANG_CAP`] or a deadline).
    Hang,
    /// A garbled response.
    Malformed,
}

impl ErrorKind {
    /// Parses `status|reset|hang|malformed`; anything else is `None`.
    #[must_use]
    pub fn parse(s: &str) -> Option<ErrorKind> {
        Some(match s {
            "status" => ErrorKind::Status,
            "reset" => ErrorKind::Reset,
            "hang" => ErrorKind::Hang,
            "malformed" => ErrorKind::Malformed,
            _ => return None,
        })
    }

    /// The name [`ErrorKind::parse`] accepts.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Status => "status",
            ErrorKind::Reset => "reset",
            ErrorKind::Hang => "hang",
            ErrorKind::Malformed => "malformed",
        }
    }
}

/// What a failed visit looked like to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// An HTTP error status.
    Status,
    /// A reset connection.
    Reset,
    /// A garbled response.
    Malformed,
    /// The caller gave up waiting.
    Timeout,
}

impl FailKind {
    /// `status|reset|malformed|timeout`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FailKind::Status => "status",
            FailKind::Reset => "reset",
            FailKind::Malformed => "malformed",
            FailKind::Timeout => "timeout",
        }
    }
}

/// Resolved chaos settings for one visit.
///
/// Every driver checks validation before anything lands here (rule R6.9);
/// the executor trusts the values.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeChaos {
    /// Extra latency added to every visit of the node.
    pub latency_add: Duration,
    /// Probability of failing a visit, 0.0..=1.0.
    pub error_rate: f64,
    /// How a failed visit shows up.
    pub error_kind: ErrorKind,
    /// HTTP status a failed `status` visit returns.
    pub error_status: u16,
    /// Hard outage: every visit fails with 503, regardless of `error_rate`.
    pub down: bool,
}

impl Default for NodeChaos {
    /// All-off defaults. An all-default entry is inert: it changes no timings and
    /// no outcomes, which the executor's determinism tests rely on.
    fn default() -> Self {
        NodeChaos {
            latency_add: Duration::ZERO,
            error_rate: 0.0,
            error_kind: ErrorKind::Status,
            error_status: 503,
            down: false,
        }
    }
}

impl NodeChaos {
    /// The one set of chaos limits, shared by the control API and the simulator (rule R6.9):
    /// `latency_add` at most [`MAX_LATENCY_ADD`], `error_rate` within 0..=1, and an
    /// `error_status` (whatever the kind) within 500–504 (4xx is for pins and cycle states, not
    /// random chaos).
    ///
    /// # Errors
    /// A message naming the first field out of range.
    pub fn validate(&self) -> Result<(), String> {
        if self.latency_add > MAX_LATENCY_ADD {
            return Err(format!(
                "latency_add must be at most {} ms (5 min)",
                MAX_LATENCY_ADD.as_millis()
            ));
        }
        if !(0.0..=1.0).contains(&self.error_rate) {
            return Err("error_rate must be between 0 and 1".into());
        }
        if !(500..=504).contains(&self.error_status) {
            return Err("error_status must be 500-504".into());
        }
        Ok(())
    }
}

/// A sparse set of chaos fields: `None` means "not set by this layer".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChaosLayer {
    /// Extra latency, summed across layers.
    pub latency_add: Option<Duration>,
    /// Failure probability, combined across layers as independent chances.
    pub error_rate: Option<f64>,
    /// Failure kind; one unit with this layer's own `error_rate`: it adds a fault only when
    /// this layer also sets a rate (S7).
    pub error_kind: Option<ErrorKind>,
    /// Failure status; same unit as `error_kind`, defaults to 503 with a rate.
    pub error_status: Option<u16>,
    /// Hard outage; any layer setting `true` wins.
    pub down: Option<bool>,
    /// `true` drops every lower layer's contribution to this key (an override).
    pub replace: Option<bool>,
    /// Link only: random extra latency of up to this much either way on the edge's `net` hop;
    /// the largest across layers.
    pub jitter: Option<Duration>,
    /// Link only: caps the edge's `bandwidth` (bytes per second, at most the edge's own); the
    /// smallest across layers.
    pub bandwidth_cap: Option<u64>,
}

impl From<NodeChaos> for ChaosLayer {
    fn from(c: NodeChaos) -> Self {
        ChaosLayer {
            latency_add: Some(c.latency_add),
            error_rate: Some(c.error_rate),
            error_kind: Some(c.error_kind),
            error_status: Some(c.error_status),
            down: Some(c.down),
            replace: None,
            jitter: None,
            bandwidth_cap: None,
        }
    }
}

impl ChaosLayer {
    /// The limits of [`NodeChaos::validate`], applied to only the fields that are set.
    ///
    /// # Errors
    /// A message naming the first field out of range.
    pub fn validate(&self) -> Result<(), String> {
        self.check().map_err(|(_, m)| m)
    }

    /// Like [`validate`](Self::validate), but also names the failing field
    /// (`latency_add`, `error_rate` or `error_status`).
    ///
    /// # Errors
    /// The first field out of range and a message about it.
    pub fn check(&self) -> Result<(), (&'static str, String)> {
        if self.latency_add.is_some_and(|d| d > MAX_LATENCY_ADD) {
            return Err((
                "latency_add",
                format!(
                    "latency_add must be at most {} ms (5 min)",
                    MAX_LATENCY_ADD.as_millis()
                ),
            ));
        }
        if self.jitter.is_some_and(|d| d > MAX_LATENCY_ADD) {
            return Err(("jitter", "jitter must be at most 300000 ms (5 min)".into()));
        }
        if self.bandwidth_cap == Some(0) {
            return Err(("bandwidth_cap", "bandwidth_cap must be at least 1".into()));
        }
        if self.error_rate.is_some_and(|r| !(0.0..=1.0).contains(&r)) {
            return Err(("error_rate", "error_rate must be between 0 and 1".into()));
        }
        // Range-checked whatever the kind: a status set under `reset` is still a status somebody typed.
        if self.error_status.is_some_and(|s| !(500..=504).contains(&s)) {
            return Err(("error_status", "error_status must be 500-504".into()));
        }
        Ok(())
    }

    /// True when a kind or status is set without a rate: it has no effect (S7).
    #[must_use]
    pub fn has_orphan_fault_fields(&self) -> bool {
        (self.error_kind.is_some() || self.error_status.is_some()) && self.error_rate.is_none()
    }

    /// This one layer with the unset fields filled in with their defaults, for the control
    /// API's GET. Not the executor's rule (that is [`compile_layers`], where a kind and status
    /// belong to their own rate); there is deliberately no multi-layer version of this.
    #[must_use]
    pub fn dense(&self) -> NodeChaos {
        NodeChaos {
            latency_add: self.latency_add.unwrap_or_default(),
            error_rate: self.error_rate.unwrap_or(0.0),
            error_kind: self.error_kind.unwrap_or_default(),
            error_status: self
                .error_status
                .unwrap_or(NodeChaos::default().error_status),
            down: self.down == Some(true),
        }
    }

    /// Overwrites every field `patch` sets, leaving the rest untouched.
    pub fn apply(&mut self, patch: &ChaosLayer) {
        macro_rules! take {
            ($($f:ident),*) => { $( if patch.$f.is_some() { self.$f = patch.$f; } )* }
        }
        take!(
            latency_add,
            error_rate,
            error_kind,
            error_status,
            down,
            replace,
            jitter,
            bandwidth_cap
        );
    }
}

/// Where a chaos layer applies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChaosKey {
    /// Every visit of a node.
    Node(NodeId),
    /// Once, at the front door of the endpoint (`"GET /orders"`), before its chain; a failure
    /// belongs to the entry node. It does not reach the nodes behind it.
    Endpoint(String),
    /// A node, only when visited through the endpoint.
    NodeAt(NodeId, String),
    /// One replica of a node, whatever the endpoint. It folds in the node's own entry and its
    /// node-at-endpoint entries; a replica that is `down` is skipped by the balancer.
    Replica(NodeId, u16),
    /// One edge (a link between two nodes), by its index in the topology.
    Edge(EdgeId),
}

/// Every chaos layer in force, by key. An absent key has no chaos.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChaosState {
    nodes: HashMap<NodeId, ChaosLayer>,
    endpoints: HashMap<String, ChaosLayer>,
    node_at: HashMap<NodeId, HashMap<String, ChaosLayer>>,
    replicas: HashMap<(NodeId, u16), ChaosLayer>,
    edges: HashMap<EdgeId, ChaosLayer>,
}

impl ChaosState {
    /// True when no layer is set (the executor's fast path).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.endpoints.is_empty()
            && self.node_at.is_empty()
            && self.replicas.is_empty()
            && self.edges.is_empty()
    }

    /// Sets a whole-node entry from dense settings.
    pub fn insert(&mut self, node: NodeId, c: NodeChaos) {
        self.set(ChaosKey::Node(node), c.into());
    }

    /// Replaces the layer under `key`.
    pub fn set(&mut self, key: ChaosKey, layer: ChaosLayer) {
        match key {
            ChaosKey::Node(n) => {
                self.nodes.insert(n, layer);
            }
            ChaosKey::Endpoint(e) => {
                self.endpoints.insert(e, layer);
            }
            ChaosKey::NodeAt(n, e) => {
                self.node_at.entry(n).or_default().insert(e, layer);
            }
            ChaosKey::Replica(n, r) => {
                self.replicas.insert((n, r), layer);
            }
            ChaosKey::Edge(e) => {
                self.edges.insert(e, layer);
            }
        }
    }

    /// The layer under `key`, if any.
    #[must_use]
    pub fn layer(&self, key: &ChaosKey) -> Option<&ChaosLayer> {
        match key {
            ChaosKey::Node(n) => self.nodes.get(n),
            ChaosKey::Endpoint(e) => self.endpoints.get(e),
            ChaosKey::NodeAt(n, e) => self.node_at.get(n).and_then(|m| m.get(e)),
            ChaosKey::Replica(n, r) => self.replicas.get(&(*n, *r)),
            ChaosKey::Edge(e) => self.edges.get(e),
        }
    }

    /// One message per stored layer whose kind or status has no effect for lack of an
    /// `error_rate`, in [`ChaosState::entries`] order. Not an error (S7). Names nodes by id, as
    /// core knows no names; the server words its own messages from the topology. Meant for
    /// tests of core itself.
    #[doc(hidden)]
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter(|(_, l)| l.has_orphan_fault_fields())
            .map(|(k, _)| {
                let at = match k {
                    ChaosKey::Node(id) => format!("node {id}"),
                    ChaosKey::NodeAt(id, ep) => format!("node {id} at endpoint '{ep}'"),
                    ChaosKey::Replica(id, r) => format!("node {id} replica {r}"),
                    ChaosKey::Edge(e) => format!("edge {e}"),
                    ChaosKey::Endpoint(ep) => format!("endpoint '{ep}'"),
                };
                format!("chaos on {at}: error_kind/error_status have no effect without error_rate")
            })
            .collect()
    }

    /// Deletes exactly the layer under `key`; true when there was one.
    pub fn remove(&mut self, key: &ChaosKey) -> bool {
        match key {
            ChaosKey::Node(n) => self.nodes.remove(n).is_some(),
            ChaosKey::Endpoint(e) => self.endpoints.remove(e).is_some(),
            ChaosKey::Replica(n, r) => self.replicas.remove(&(*n, *r)).is_some(),
            ChaosKey::Edge(e) => self.edges.remove(e).is_some(),
            ChaosKey::NodeAt(n, e) => {
                let Some(m) = self.node_at.get_mut(n) else {
                    return false;
                };
                let had = m.remove(e).is_some();
                if m.is_empty() {
                    self.node_at.remove(n);
                }
                had
            }
        }
    }

    /// Drops every layer.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.endpoints.clear();
        self.node_at.clear();
        self.replicas.clear();
        self.edges.clear();
    }

    /// Compiles this single layer set; same as `compile_layers(&[self])`.
    #[must_use]
    pub fn compile(&self) -> EffectiveChaos {
        compile_layers(&[self])
    }

    /// How many layers are set: nodes, endpoints and (node, endpoint) pairs.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.nodes.len()
            + self.endpoints.len()
            + self.node_at.values().map(HashMap::len).sum::<usize>()
            + self.replicas.len()
            + self.edges.len()
    }

    /// Every layer with its key, sorted: nodes by id, then endpoints by name, then
    /// node-at-endpoint by (id, name).
    #[must_use]
    pub fn entries(&self) -> Vec<(ChaosKey, &ChaosLayer)> {
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

    /// The first validation failure, by [`ChaosState::entries`] order, so the message is
    /// deterministic.
    #[must_use]
    pub fn first_error(&self) -> Option<String> {
        self.entries().into_iter().find_map(|(k, l)| {
            l.validate().err().map(|e| match k {
                ChaosKey::Node(id) => format!("chaos on node {id}: {e}"),
                ChaosKey::NodeAt(id, ep) => format!("chaos on node {id} at endpoint '{ep}': {e}"),
                ChaosKey::Endpoint(ep) => format!("chaos on endpoint '{ep}': {e}"),
                ChaosKey::Replica(id, r) => format!("chaos on node {id} replica {r}: {e}"),
                ChaosKey::Edge(id) => format!("chaos on edge {id}: {e}"),
            })
        })
    }
}

/// The replica entries of `m` sorted by (node id, replica), for a stable listing.
fn reps_sorted<V>(m: &HashMap<(NodeId, u16), V>) -> Vec<((NodeId, u16), &V)> {
    let mut v: Vec<_> = m.iter().map(|(k, l)| (*k, l)).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}

mod link;
mod resolve;
pub use resolve::{EffectiveChaos, Fault, MAX_FAULTS, Resolved, compile_layers};

impl<const N: usize> From<[(NodeId, NodeChaos); N]> for ChaosState {
    fn from(a: [(NodeId, NodeChaos); N]) -> Self {
        a.into_iter().collect()
    }
}

impl FromIterator<(NodeId, NodeChaos)> for ChaosState {
    fn from_iter<T: IntoIterator<Item = (NodeId, NodeChaos)>>(iter: T) -> Self {
        let mut s = ChaosState::default();
        for (n, c) in iter {
            s.insert(n, c);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_status_is_range_checked_for_every_kind() {
        let bad = ChaosLayer {
            error_kind: Some(ErrorKind::Reset),
            error_status: Some(200),
            ..ChaosLayer::default()
        };
        assert!(bad.validate().is_err());
        let kind_only = ChaosLayer {
            error_kind: Some(ErrorKind::Reset),
            ..ChaosLayer::default()
        };
        assert!(kind_only.validate().is_ok());
        let dense = NodeChaos {
            error_kind: ErrorKind::Hang,
            error_status: 200,
            ..NodeChaos::default()
        };
        assert!(dense.validate().is_err());
    }
}
