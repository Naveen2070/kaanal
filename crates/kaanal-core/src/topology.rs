//! The topology graph: nodes, edges, and all semantic validation of the config.
//!
//! [`Topology::build`] turns a parsed [`Config`] into a validated graph the
//! executor can walk. Anything the executor would silently ignore or misinterpret
//! is rejected at load with a named error (rules R2.7, R6.1).

use std::collections::HashMap;
use std::fmt;

use crate::config::{Config, NodeKind};
use crate::dist::Dist;

/// Index of a node in [`Topology::nodes`].
pub type NodeId = usize;

/// A runtime node with its costs resolved to distributions.
#[derive(Debug, Clone)]
pub struct Node {
    /// Node name from `kaanal.yaml`.
    pub name: String,
    /// Gateway, service or database.
    pub kind: NodeKind,
    /// Cosmetic flavor label (`postgres`, `nginx`, …).
    pub flavor: Option<String>,
    /// `OpenAPI` spec file (service nodes only).
    pub spec: Option<String>,
    /// Processing-time distribution (zero when unconfigured).
    pub proc: Dist,
    /// Database op costs, indexed by `Op as usize`: read, write, scan.
    pub ops: Option<[Dist; 3]>,
}

/// A dependency between two nodes, with its network latency.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Upstream node.
    pub from: NodeId,
    /// Downstream node.
    pub to: NodeId,
    /// Network latency distribution on this edge (zero when unconfigured).
    pub net: Dist,
}

/// A validated topology graph.
#[derive(Debug, Clone)]
pub struct Topology {
    /// All nodes; a [`NodeId`] indexes this vector.
    pub nodes: Vec<Node>,
    /// All edges.
    pub edges: Vec<Edge>,
    /// The one gateway node.
    pub gateway: NodeId,
    /// **Private.** name → id lookup.
    index: HashMap<String, NodeId>,
}

/// Why a topology was rejected. Every variant renders as human prose via `Display`.
#[derive(Debug, Clone, PartialEq)]
pub enum TopologyError {
    /// An edge references a node that does not exist; carries the edge number and the name.
    UnknownNode {
        /// 0-based index of the offending edge in `edges:`.
        edge: usize,
        /// The unknown node name.
        name: String,
    },
    /// There must be exactly one gateway; carries how many were found (0 or more).
    GatewayCount(usize),
    /// The graph is not a DAG; carries the cycle as node names (`a -> b -> a`).
    Cycle(Vec<String>),
    /// A database node without `ops`; carries the node name.
    MissingOps(String),
    /// A service with an `OpenAPI` spec but no edge from the gateway; carries the node name.
    MissingGatewayEdge(String),
    /// Anything valid YAML but outside phase-one semantics (edge shapes, field placement).
    Unsupported(String),
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopologyError::UnknownNode { edge, name } => {
                write!(f, "edge #{edge}: unknown node '{name}'")
            }
            TopologyError::GatewayCount(n) => {
                write!(f, "expected exactly 1 gateway node, found {n}")
            }
            TopologyError::Cycle(path) => write!(f, "cycle: {}", path.join(" -> ")),
            TopologyError::MissingOps(n) => {
                write!(f, "database node '{n}' needs ops: read, write, scan")
            }
            TopologyError::MissingGatewayEdge(n) => {
                write!(
                    f,
                    "service '{n}' has an OpenAPI spec but no edge from the gateway"
                )
            }
            TopologyError::Unsupported(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for TopologyError {}

impl Topology {
    /// Builds and validates the graph.
    ///
    /// Checks, in order: field placement (`ops` on databases, `spec` on services
    /// only), database ops present, exactly one gateway, edge endpoints exist, the
    /// graph is a DAG, phase-one edge shapes (`gateway → service`, `service →
    /// database`, no duplicates), and every service with a spec is reachable from
    /// the gateway.
    ///
    /// # Errors
    /// Returns the first [`TopologyError`] found; nothing is half-built.
    pub fn build(cfg: &Config) -> Result<Topology, TopologyError> {
        let mut nodes = Vec::new();
        let mut index = HashMap::new();
        for (name, n) in &cfg.nodes {
            if n.ops.is_some() && n.kind != NodeKind::Database {
                return Err(TopologyError::Unsupported(format!(
                    "node '{name}': ops is only valid on database nodes"
                )));
            }
            if n.spec.is_some() && n.kind != NodeKind::Service {
                return Err(TopologyError::Unsupported(format!(
                    "node '{name}': spec is only valid on service nodes"
                )));
            }
            let ops = match (n.kind, &n.ops) {
                (NodeKind::Database, None) => return Err(TopologyError::MissingOps(name.clone())),
                (_, Some(o)) => Some([
                    Dist::from_config(Some(&o.read)),
                    Dist::from_config(Some(&o.write)),
                    Dist::from_config(Some(&o.scan)),
                ]),
                (_, None) => None,
            };
            index.insert(name.clone(), nodes.len());
            nodes.push(Node {
                name: name.clone(),
                kind: n.kind,
                flavor: n.flavor.clone(),
                spec: n.spec.clone(),
                proc: Dist::from_config(n.proc.as_ref()),
                ops,
            });
        }

        let gateways: Vec<NodeId> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Gateway)
            .map(|(i, _)| i)
            .collect();
        if gateways.len() != 1 {
            return Err(TopologyError::GatewayCount(gateways.len()));
        }

        let mut edges = Vec::new();
        for (i, e) in cfg.edges.iter().enumerate() {
            let look = |name: &String| {
                index
                    .get(name)
                    .copied()
                    .ok_or_else(|| TopologyError::UnknownNode {
                        edge: i,
                        name: name.clone(),
                    })
            };
            edges.push(Edge {
                from: look(&e.from)?,
                to: look(&e.to)?,
                net: Dist::from_config(e.net.as_ref()),
            });
        }

        let topo = Topology {
            nodes,
            edges,
            gateway: gateways[0],
            index,
        };
        topo.check_cycles()?;

        // Phase one only walks gateway -> service -> database; anything else would be silently ignored.
        for (i, e) in topo.edges.iter().enumerate() {
            let (from, to) = (&topo.nodes[e.from], &topo.nodes[e.to]);
            if !matches!(
                (from.kind, to.kind),
                (NodeKind::Gateway, NodeKind::Service) | (NodeKind::Service, NodeKind::Database)
            ) {
                return Err(TopologyError::Unsupported(format!(
                    "phase one: edge '{}' -> '{}' must be gateway -> service or service -> database",
                    from.name, to.name
                )));
            }
            if topo.edges[..i]
                .iter()
                .any(|p| p.from == e.from && p.to == e.to)
            {
                return Err(TopologyError::Unsupported(format!(
                    "duplicate edge '{}' -> '{}'",
                    from.name, to.name
                )));
            }
        }

        for (id, n) in topo.nodes.iter().enumerate() {
            if n.kind == NodeKind::Service
                && n.spec.is_some()
                && topo.edge(topo.gateway, id).is_none()
            {
                return Err(TopologyError::MissingGatewayEdge(n.name.clone()));
            }
        }
        Ok(topo)
    }

    /// Looks up a node by name.
    pub fn id(&self, name: &str) -> Option<NodeId> {
        self.index.get(name).copied()
    }

    /// Iterates over the edges leaving a node, in config order.
    pub fn out_edges(&self, id: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.from == id)
    }

    /// Finds the edge between two specific nodes, if any.
    pub fn edge(&self, from: NodeId, to: NodeId) -> Option<&Edge> {
        self.edges.iter().find(|e| e.from == from && e.to == to)
    }

    /// **Private.** DFS cycle check with 3-colour marking.
    fn check_cycles(&self) -> Result<(), TopologyError> {
        // state: 0 = unvisited, 1 = on the current path, 2 = done
        fn visit(
            t: &Topology,
            n: NodeId,
            state: &mut [u8],
            path: &mut Vec<NodeId>,
        ) -> Result<(), TopologyError> {
            match state[n] {
                2 => return Ok(()),
                1 => {
                    let start = path.iter().position(|&p| p == n).expect("node on path");
                    let mut names: Vec<String> = path[start..]
                        .iter()
                        .map(|&p| t.nodes[p].name.clone())
                        .collect();
                    names.push(t.nodes[n].name.clone());
                    return Err(TopologyError::Cycle(names));
                }
                _ => {}
            }
            state[n] = 1;
            path.push(n);
            for e in t.out_edges(n) {
                visit(t, e.to, state, path)?;
            }
            path.pop();
            state[n] = 2;
            Ok(())
        }
        let mut state = vec![0u8; self.nodes.len()];
        for n in 0..self.nodes.len() {
            visit(self, n, &mut state, &mut Vec::new())?;
        }
        Ok(())
    }
}
