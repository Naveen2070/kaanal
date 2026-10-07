use std::collections::HashMap;
use std::fmt;

use crate::config::{Config, NodeKind};
use crate::dist::Dist;

pub type NodeId = usize;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub flavor: Option<String>,
    pub spec: Option<String>,
    pub proc: Dist,
    /// Database op costs, indexed by `Op as usize`: read, write, scan.
    pub ops: Option<[Dist; 3]>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub net: Dist,
}

#[derive(Debug, Clone)]
pub struct Topology {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub gateway: NodeId,
    index: HashMap<String, NodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopologyError {
    UnknownNode { edge: usize, name: String },
    GatewayCount(usize),
    Cycle(Vec<String>),
    MissingOps(String),
    MissingGatewayEdge(String),
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

    pub fn id(&self, name: &str) -> Option<NodeId> {
        self.index.get(name).copied()
    }

    pub fn out_edges(&self, id: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.from == id)
    }

    pub fn edge(&self, from: NodeId, to: NodeId) -> Option<&Edge> {
        self.edges.iter().find(|e| e.from == from && e.to == to)
    }

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
