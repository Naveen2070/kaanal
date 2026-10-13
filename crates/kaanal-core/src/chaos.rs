//! Live chaos state: the failure knobs flipped at runtime via the control API.
//!
//! Nothing here comes from `kaanal.yaml`; this is runtime state only. Written by
//! `PUT/DELETE /__kaanal/chaos...`, read by the chain executor on every request.
//!
//! Chaos is stored as sparse [`ChaosLayer`]s (only the fields a caller set) under a
//! [`ChaosKey`] (a node, an endpoint, or a node at one endpoint). The executor resolves the
//! layers that apply to a visit into one dense [`NodeChaos`] with [`merge`].

use std::collections::HashMap;
use std::time::Duration;

use crate::topology::NodeId;

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
    /// `latency_add` at most [`MAX_LATENCY_ADD`], `error_rate` within 0..=1, and for kind
    /// `status` an `error_status` within 500–504 (4xx is for pins and cycle states, not
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
        if self.error_kind == ErrorKind::Status && !(500..=504).contains(&self.error_status) {
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
    /// Failure kind; the last layer that sets it wins.
    pub error_kind: Option<ErrorKind>,
    /// Failure status; the last layer that sets it wins.
    pub error_status: Option<u16>,
    /// Hard outage; any layer setting `true` wins.
    pub down: Option<bool>,
}

impl From<NodeChaos> for ChaosLayer {
    fn from(c: NodeChaos) -> Self {
        ChaosLayer {
            latency_add: Some(c.latency_add),
            error_rate: Some(c.error_rate),
            error_kind: Some(c.error_kind),
            error_status: Some(c.error_status),
            down: Some(c.down),
        }
    }
}

impl ChaosLayer {
    /// The limits of [`NodeChaos::validate`], applied to only the fields that are set.
    ///
    /// # Errors
    /// A message naming the first field out of range.
    pub fn validate(&self) -> Result<(), String> {
        if self.latency_add.is_some_and(|d| d > MAX_LATENCY_ADD) {
            return Err(format!(
                "latency_add must be at most {} ms (5 min)",
                MAX_LATENCY_ADD.as_millis()
            ));
        }
        if self.error_rate.is_some_and(|r| !(0.0..=1.0).contains(&r)) {
            return Err("error_rate must be between 0 and 1".into());
        }
        // The status only matters for kind `status`; a layer that sets a status without a kind is
        // checked as `status` because the kind it merges into may well be `status`.
        let kind = self.error_kind.unwrap_or(ErrorKind::Status);
        if kind == ErrorKind::Status && self.error_status.is_some_and(|s| !(500..=504).contains(&s))
        {
            return Err("error_status must be 500-504".into());
        }
        Ok(())
    }

    /// Overwrites every field `patch` sets, leaving the rest untouched.
    pub fn apply(&mut self, patch: &ChaosLayer) {
        macro_rules! take {
            ($($f:ident),*) => { $( if patch.$f.is_some() { self.$f = patch.$f; } )* }
        }
        take!(latency_add, error_rate, error_kind, error_status, down);
    }
}

/// Folds layers (least specific first) into one [`NodeChaos`]: latencies add, rates combine as
/// independent chances (`p + r - p*r`; one set rate comes back bit-for-bit), `down` is an OR,
/// and the last kind and status set win.
#[must_use]
pub fn merge(layers: &[&ChaosLayer]) -> NodeChaos {
    merge_iter(layers.iter().copied())
}

/// **Private.** [`merge`] over any iterator, so [`ChaosState::resolve`] needs no heap.
fn merge_iter<'a>(layers: impl Iterator<Item = &'a ChaosLayer>) -> NodeChaos {
    let mut out = NodeChaos::default();
    let mut rate: Option<f64> = None;
    for l in layers {
        if let Some(d) = l.latency_add {
            out.latency_add = out.latency_add.saturating_add(d);
        }
        if let Some(r) = l.error_rate {
            rate = Some(rate.map_or(r, |p| p + r - p * r));
        }
        if let Some(k) = l.error_kind {
            out.error_kind = k;
        }
        if let Some(s) = l.error_status {
            out.error_status = s;
        }
        if l.down == Some(true) {
            out.down = true;
        }
    }
    out.error_rate = rate.unwrap_or(0.0);
    out
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
}

/// Every chaos layer in force, by key. An absent key has no chaos.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChaosState {
    nodes: HashMap<NodeId, ChaosLayer>,
    endpoints: HashMap<String, ChaosLayer>,
    node_at: HashMap<NodeId, HashMap<String, ChaosLayer>>,
}

impl ChaosState {
    /// True when no layer is set (the executor's fast path).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.endpoints.is_empty() && self.node_at.is_empty()
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
        }
    }

    /// The layer under `key`, if any.
    #[must_use]
    pub fn layer(&self, key: &ChaosKey) -> Option<&ChaosLayer> {
        match key {
            ChaosKey::Node(n) => self.nodes.get(n),
            ChaosKey::Endpoint(e) => self.endpoints.get(e),
            ChaosKey::NodeAt(n, e) => self.node_at.get(n).and_then(|m| m.get(e)),
        }
    }

    /// Deletes exactly the layer under `key`; true when there was one.
    pub fn remove(&mut self, key: &ChaosKey) -> bool {
        match key {
            ChaosKey::Node(n) => self.nodes.remove(n).is_some(),
            ChaosKey::Endpoint(e) => self.endpoints.remove(e).is_some(),
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
    }

    /// The chaos in force for a visit of `node` through `endpoint`: the node layer, then the
    /// node-at-endpoint layer, merged with the most specific last. The endpoint-wide layer is not
    /// here: it applies once at the front door, not per node (Z10); see [`ChaosState::endpoint`].
    #[must_use]
    pub fn resolve(&self, node: NodeId, endpoint: &str) -> NodeChaos {
        let n = self.nodes.get(&node);
        let at = self.node_at.get(&node).and_then(|m| m.get(endpoint)); // borrows: no allocation
        merge_iter([n, at].into_iter().flatten())
    }

    /// The endpoint-wide layer for `endpoint`, if any.
    #[must_use]
    pub fn endpoint(&self, endpoint: &str) -> Option<&ChaosLayer> {
        self.endpoints.get(endpoint)
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
        nodes.chain(eps).chain(at).collect()
    }

    /// The first validation failure, by [`ChaosState::entries`] order, so the message is
    /// deterministic.
    #[must_use]
    pub fn first_error(&self) -> Option<String> {
        self.entries().into_iter().find_map(|(k, l)| {
            l.validate().err().map(|e| match k {
                ChaosKey::Node(id) | ChaosKey::NodeAt(id, _) => {
                    format!("chaos on node {id}: {e}")
                }
                ChaosKey::Endpoint(ep) => format!("chaos on endpoint '{ep}': {e}"),
            })
        })
    }
}

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
