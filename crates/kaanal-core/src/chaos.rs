//! Live chaos state: the failure knobs flipped at runtime via the control API.
//!
//! Nothing here comes from `kaanal.yaml`; this is runtime state only. Written by
//! `PUT/DELETE /__kaanal/chaos...`, read by the chain executor on every request.
//!
//! Chaos is stored as sparse [`ChaosLayer`]s (only the fields a caller set) under a
//! [`ChaosKey`] (a node, an endpoint, or a node at one endpoint). The executor resolves the
//! layers into an [`EffectiveChaos`] (see [`compile_layers`]) once, so a visit is one lookup.

use std::collections::{BTreeSet, HashMap};
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
            replace
        );
    }
}

/// Dense view of a single stored layer, for the control API's GET (defaults filled in). Not
/// the executor's rule: that is [`compile_layers`], where kind and status belong to their own
/// rate. With several layers this folds rates as independent chances and takes the last kind.
#[must_use]
pub fn merge(layers: &[&ChaosLayer]) -> NodeChaos {
    let mut out = NodeChaos::default();
    let mut rate: Option<f64> = None;
    for l in layers.iter().copied() {
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

    /// One message per stored layer whose kind or status has no effect for lack of an
    /// `error_rate`, in [`ChaosState::entries`] order. Not an error (S7).
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter(|(_, l)| l.has_orphan_fault_fields())
            .map(|(k, _)| {
                let at = match k {
                    ChaosKey::Node(id) => format!("node {id}"),
                    ChaosKey::NodeAt(id, ep) => format!("node {id} at endpoint '{ep}'"),
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

    /// The endpoint-wide layer for `endpoint`, if any.
    #[must_use]
    pub fn endpoint(&self, endpoint: &str) -> Option<&ChaosLayer> {
        self.endpoints.get(endpoint)
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
                ChaosKey::Node(id) => format!("chaos on node {id}: {e}"),
                ChaosKey::NodeAt(id, ep) => format!("chaos on node {id} at endpoint '{ep}': {e}"),
                ChaosKey::Endpoint(ep) => format!("chaos on endpoint '{ep}': {e}"),
            })
        })
    }
}

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
    faults: [Fault; MAX_FAULTS],
    len: u8,
}

impl Resolved {
    fn empty() -> Self {
        Resolved {
            latency_add: Duration::ZERO,
            down: false,
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

fn resolve_entries(mut entries: Vec<Entry<'_>>) -> Resolved {
    entries.sort_by_key(|e| std::cmp::Reverse((e.0, e.1))); // highest layer first, then most specific
    let mut out = Resolved::empty();
    for (_, _, l) in entries {
        if let Some(d) = l.latency_add {
            out.latency_add = out.latency_add.saturating_add(d);
        }
        if l.down == Some(true) {
            out.down = true;
        }
        if let Some(r) = l.error_rate
            && r > 0.0
        {
            out.push(
                r,
                l.error_kind.unwrap_or(ErrorKind::Status),
                l.error_status.unwrap_or(503),
            );
        }
    }
    out.latency_add = out.latency_add.min(MAX_LATENCY_ADD);
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
}

impl EffectiveChaos {
    /// True when nothing is set (the executor's fast path).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.node_at.is_empty() && self.endpoints.is_empty()
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
        nodes.chain(eps).chain(at).collect()
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
    for s in layers {
        node_ids.extend(s.nodes.keys().copied());
        eps.extend(s.endpoints.keys().cloned());
        for (n, m) in &s.node_at {
            pairs.extend(m.keys().map(|e| (*n, e.clone())));
        }
    }
    let mut out = EffectiveChaos::default();
    for id in node_ids {
        let e = entries_of(layers, 0, |s| s.nodes.get(&id));
        out.nodes.insert(id, resolve_entries(e));
    }
    for ep in eps {
        let e = entries_of(layers, 0, |s| s.endpoints.get(&ep));
        out.endpoints.insert(ep, resolve_entries(e));
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
            .insert(ep, resolve_entries(e));
    }
    out
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
