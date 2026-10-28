//! Moving chaos from one topology to another, by name. Chaos keys hold topology ids (a node
//! index, an edge index), which a changed topology renumbers; a [`KeyMap`] says where each old
//! id went (or that it is gone), so the server can keep what still applies after an Apply.

use super::{ChaosKey, ChaosState};
use crate::topology::{EdgeId, NodeId};

/// How the ids of an old topology map into a new one.
pub trait KeyMap {
    /// The new id of an old node, `None` when the node is gone (a rename is a removal).
    fn node(&self, old: NodeId) -> Option<NodeId>;
    /// The new id of an old edge, `None` when either end or the edge is gone.
    fn edge(&self, old: EdgeId) -> Option<EdgeId>;
    /// Whether `replica` exists on the (new) node `new_node`.
    fn replica_exists(&self, new_node: NodeId, replica: u16) -> bool;
    /// Whether the new system still has this endpoint key (`"GET /orders"`).
    fn endpoint_exists(&self, endpoint: &str) -> bool;
}

impl ChaosKey {
    /// The same key in the new topology, or `None` when what it names is gone.
    #[must_use]
    pub fn remap(&self, m: &dyn KeyMap) -> Option<ChaosKey> {
        Some(match self {
            ChaosKey::Node(n) => ChaosKey::Node(m.node(*n)?),
            ChaosKey::Edge(e) => ChaosKey::Edge(m.edge(*e)?),
            ChaosKey::Endpoint(ep) => m
                .endpoint_exists(ep)
                .then(|| ChaosKey::Endpoint(ep.clone()))?,
            ChaosKey::NodeAt(n, ep) => {
                let n = m.node(*n)?;
                m.endpoint_exists(ep)
                    .then(|| ChaosKey::NodeAt(n, ep.clone()))?
            }
            ChaosKey::Replica(n, r) => {
                let n = m.node(*n)?;
                m.replica_exists(n, *r)
                    .then_some(ChaosKey::Replica(n, *r))?
            }
        })
    }
}

impl ChaosState {
    /// The layers that still apply, under their new keys, and the old keys that did not survive
    /// (in [`ChaosState::entries`] order). A layer keeps its values; only its key changes.
    #[must_use]
    pub fn remap(&self, m: &dyn KeyMap) -> (ChaosState, Vec<ChaosKey>) {
        let mut out = ChaosState::default();
        let mut dropped = Vec::new();
        for (key, layer) in self.entries() {
            match key.remap(m) {
                Some(new) => out.set(new, layer.clone()),
                None => dropped.push(key),
            }
        }
        (out, dropped)
    }
}
