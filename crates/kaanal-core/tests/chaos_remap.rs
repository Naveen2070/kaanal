//! Moving chaos from one topology to another by name: `ChaosKey::remap` and `ChaosState::remap`.

use kaanal_core::{ChaosKey, ChaosLayer, ChaosState, EdgeId, KeyMap, NodeId};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// A hand-made old-to-new mapping.
#[derive(Default)]
struct Map {
    nodes: HashMap<NodeId, NodeId>,
    edges: HashMap<EdgeId, EdgeId>,
    /// replica counts of the new nodes
    replicas: HashMap<NodeId, u16>,
    endpoints: HashSet<&'static str>,
}

impl KeyMap for Map {
    fn node(&self, old: NodeId) -> Option<NodeId> {
        self.nodes.get(&old).copied()
    }
    fn edge(&self, old: EdgeId) -> Option<EdgeId> {
        self.edges.get(&old).copied()
    }
    fn replica_exists(&self, new_node: NodeId, replica: u16) -> bool {
        replica < self.replicas.get(&new_node).copied().unwrap_or(1)
    }
    fn endpoint_exists(&self, endpoint: &str) -> bool {
        self.endpoints.contains(endpoint)
    }
}

fn lag(ms: u64) -> ChaosLayer {
    ChaosLayer {
        latency_add: Some(Duration::from_millis(ms)),
        ..Default::default()
    }
}

/// Old node 0 becomes 2, node 1 is gone; edge 3 becomes 0; `GET /a` survives, `GET /gone` not.
fn map() -> Map {
    Map {
        nodes: HashMap::from([(0, 2)]),
        edges: HashMap::from([(3, 0)]),
        replicas: HashMap::from([(2, 2)]),
        endpoints: HashSet::from(["GET /a"]),
    }
}

#[test]
fn keys_move_to_their_new_ids_and_carry_their_layer() {
    let mut s = ChaosState::default();
    s.set(ChaosKey::Node(0), lag(10));
    s.set(ChaosKey::Node(1), lag(11));
    s.set(ChaosKey::Edge(3), lag(12));
    s.set(ChaosKey::Endpoint("GET /a".into()), lag(13));
    s.set(ChaosKey::NodeAt(0, "GET /a".into()), lag(14));
    let (out, dropped) = s.remap(&map());
    assert_eq!(out.layer(&ChaosKey::Node(2)), Some(&lag(10)));
    assert_eq!(out.layer(&ChaosKey::Edge(0)), Some(&lag(12)));
    assert_eq!(
        out.layer(&ChaosKey::Endpoint("GET /a".into())),
        Some(&lag(13))
    );
    assert_eq!(
        out.layer(&ChaosKey::NodeAt(2, "GET /a".into())),
        Some(&lag(14))
    );
    assert_eq!(
        out.layer(&ChaosKey::Node(0)),
        None,
        "the old id is not left behind"
    );
    assert_eq!(
        dropped,
        [ChaosKey::Node(1)],
        "only the removed node is reported"
    );
}

#[test]
fn a_key_whose_node_or_endpoint_is_gone_is_dropped_and_reported() {
    let mut s = ChaosState::default();
    s.set(ChaosKey::Node(1), lag(1));
    s.set(ChaosKey::Edge(9), lag(2));
    s.set(ChaosKey::Endpoint("GET /gone".into()), lag(3));
    s.set(ChaosKey::NodeAt(0, "GET /gone".into()), lag(4));
    s.set(ChaosKey::NodeAt(1, "GET /a".into()), lag(5));
    let (out, mut dropped) = s.remap(&map());
    assert!(out.is_empty(), "nothing survives");
    dropped.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(dropped.len(), 5);
}

#[test]
fn a_replica_beyond_the_new_count_is_dropped_and_one_within_is_kept() {
    let mut s = ChaosState::default();
    s.set(ChaosKey::Replica(0, 1), lag(1));
    s.set(ChaosKey::Replica(0, 2), lag(2));
    let (out, dropped) = s.remap(&map());
    assert_eq!(
        out.layer(&ChaosKey::Replica(2, 1)),
        Some(&lag(1)),
        "node 2 has 2 replicas now"
    );
    assert_eq!(out.layer(&ChaosKey::Replica(2, 2)), None);
    assert_eq!(dropped, [ChaosKey::Replica(0, 2)]);
}

#[test]
fn an_identity_map_changes_nothing() {
    struct Same;
    impl KeyMap for Same {
        fn node(&self, old: NodeId) -> Option<NodeId> {
            Some(old)
        }
        fn edge(&self, old: EdgeId) -> Option<EdgeId> {
            Some(old)
        }
        fn replica_exists(&self, _: NodeId, _: u16) -> bool {
            true
        }
        fn endpoint_exists(&self, _: &str) -> bool {
            true
        }
    }
    let mut s = ChaosState::default();
    s.set(ChaosKey::Node(4), lag(1));
    s.set(ChaosKey::Edge(2), lag(2));
    s.set(ChaosKey::Endpoint("GET /x".into()), lag(3));
    s.set(ChaosKey::NodeAt(1, "GET /x".into()), lag(4));
    s.set(ChaosKey::Replica(1, 3), lag(5));
    let (out, dropped) = s.remap(&Same);
    assert_eq!(out, s);
    assert_eq!(dropped, Vec::<ChaosKey>::new());
}
