//! The topology graph: nodes, edges, and all semantic validation of the config.
//!
//! [`Topology::build`] turns a parsed [`Config`] into a validated graph the executor can
//! walk: at most one gateway (R1.23), the caller → dependency edge table (R1.24), no
//! unknown, self or duplicate edges, no cycles (R2.7), and op/cache costs completed from
//! flavor presets (R1.25). Anything the executor would silently ignore or misinterpret is
//! rejected at load with a named error (R6.1).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::num::NonZeroU32;

use crate::config::{Config, NodeConfig, NodeKind};
use crate::diagnostics::{Located, PathSeg, keys};
use crate::dist::Dist;
use crate::presets;

/// Index of a node in [`Topology::nodes`].
pub type NodeId = usize;

/// A runtime node with its costs resolved to distributions.
#[derive(Debug, Clone)]
pub struct Node {
    /// Node name from `kaanal.yaml`.
    pub name: String,
    /// The node's kind.
    pub kind: NodeKind,
    /// Flavor label (`nginx`, `postgres`, `redis`, …).
    pub flavor: Option<String>,
    /// `OpenAPI` spec file (service and monolith nodes).
    pub spec: Option<String>,
    /// Processing-time distribution (zero when unconfigured).
    pub proc: Dist,
    /// Op costs, indexed by `Op as usize`: read, write, scan. Databases always; monoliths
    /// with `ops` or a `flavor` (inline ops, walked by chains from A2).
    pub ops: Option<[Dist; 3]>,
    /// Hit ratio and hit/miss costs of a cache node, presets applied (walked from A2).
    pub cache: Option<CacheModel>,
    /// Capacity for this node's own hops (see `NodeConfig::concurrency`); `None` = unlimited.
    pub concurrency: Option<NonZeroU32>,
}

/// A cache node's behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheModel {
    /// Probability that a lookup hits, 0..=1.
    pub hit_ratio: f64,
    /// Cost of a hit.
    pub hit: Dist,
    /// Cost of finding out it is a miss.
    pub miss: Dist,
}

/// A dependency between two nodes, caller → dependency.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Calling node.
    pub from: NodeId,
    /// Called node.
    pub to: NodeId,
    /// Network latency distribution on this edge (zero when unconfigured).
    pub net: Dist,
    /// Calls per request along this edge (FR-TOP-02); used by chains from A2.
    pub calls: NonZeroU32,
}

/// A validated topology graph.
#[derive(Debug, Clone)]
pub struct Topology {
    /// All nodes; a [`NodeId`] indexes this vector.
    pub nodes: Vec<Node>,
    /// All edges, in config order.
    pub edges: Vec<Edge>,
    /// The gateway, if the topology has one (at most one, R1.23).
    pub gateway: Option<NodeId>,
    /// **Private.** name → id lookup.
    index: HashMap<String, NodeId>,
}

/// Why a topology was rejected. Every variant renders as human prose via `Display`.
#[derive(Debug, Clone, PartialEq)]
pub enum TopologyError {
    /// An edge references a node that does not exist.
    UnknownNode {
        /// 0-based index of the offending edge in `edges:`.
        edge: usize,
        /// The unknown node name.
        name: String,
    },
    /// A second gateway; carries its name.
    SecondGateway(String),
    /// No gateway and no service or monolith: requests have nowhere to enter.
    NoEntry,
    /// An edge from a node to itself.
    SelfLoop {
        /// 0-based index of the offending edge.
        edge: usize,
        /// The node's name.
        name: String,
    },
    /// The same caller → dependency pair twice.
    DuplicateEdge {
        /// 0-based index of the repeated edge.
        edge: usize,
        /// Caller name.
        from: String,
        /// Dependency name.
        to: String,
    },
    /// The graph is not a DAG; carries the cycle as node names (`a -> b -> a`).
    Cycle(Vec<String>),
    /// A caller → dependency pair outside the R1.24 table.
    ForbiddenEdge {
        /// 0-based index of the offending edge.
        edge: usize,
        /// Caller name.
        from: String,
        /// Dependency name.
        to: String,
        /// Caller kind.
        from_kind: NodeKind,
        /// Dependency kind.
        to_kind: NodeKind,
    },
    /// A database (or monolith with inline ops) missing an op and no flavor to fill it.
    MissingOps(String),
    /// A cache missing `hit_ratio`, `hit` or `miss` and no flavor to fill it.
    MissingCache(String),
    /// With a gateway, a node with an `OpenAPI` spec but no edge from the gateway.
    MissingGatewayEdge(String),
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopologyError::UnknownNode { edge, name } => {
                write!(f, "edge #{edge}: unknown node '{name}'")
            }
            TopologyError::SecondGateway(n) => {
                write!(f, "node '{n}': a topology has at most one gateway")
            }
            TopologyError::NoEntry => f.write_str(
                "a topology needs a gateway, a service or a monolith (requests enter there)",
            ),
            TopologyError::SelfLoop { edge, name } => {
                write!(f, "edge #{edge}: '{name}' cannot call itself")
            }
            TopologyError::DuplicateEdge { edge, from, to } => {
                write!(f, "edge #{edge}: duplicate edge '{from}' -> '{to}'")
            }
            TopologyError::Cycle(path) => write!(f, "cycle: {}", path.join(" -> ")),
            TopologyError::ForbiddenEdge {
                edge,
                from,
                to,
                from_kind,
                to_kind,
            } => {
                let allowed: Vec<&str> = callees(*from_kind)
                    .iter()
                    .copied()
                    .map(NodeKind::as_str)
                    .collect();
                let allowed = if allowed.is_empty() {
                    "nothing".to_string()
                } else {
                    allowed.join(", ")
                };
                write!(
                    f,
                    "edge #{edge}: {from_kind} '{from}' cannot call {to_kind} '{to}'; {from_kind} may call: {allowed}"
                )
            }
            TopologyError::MissingOps(n) => write!(
                f,
                "node '{n}' needs ops: read, write and scan (or a flavor: postgres, mysql or mongodb)"
            ),
            TopologyError::MissingCache(n) => write!(
                f,
                "cache node '{n}' needs hit_ratio, hit and miss (or a flavor: redis or memcached)"
            ),
            TopologyError::MissingGatewayEdge(n) => {
                write!(
                    f,
                    "node '{n}' has an OpenAPI spec but no edge from the gateway"
                )
            }
        }
    }
}

impl TopologyError {
    /// This error with its place in `kaanal.yaml`: the edge entry (or the unknown name in
    /// it), the edge that closes a cycle, the node, or `nodes` itself.
    pub fn located(&self, cfg: &Config) -> Located {
        let edge = |i: usize| vec![PathSeg::Key("edges".into()), PathSeg::Index(i)];
        let path = match self {
            TopologyError::UnknownNode { edge: i, name } => {
                let field = if cfg.edges.get(*i).is_some_and(|e| e.from == *name) {
                    "from"
                } else {
                    "to"
                };
                let mut p = edge(*i);
                p.push(PathSeg::Key(field.into()));
                p
            }
            TopologyError::SelfLoop { edge: i, .. }
            | TopologyError::DuplicateEdge { edge: i, .. }
            | TopologyError::ForbiddenEdge { edge: i, .. } => edge(*i),
            TopologyError::Cycle(names) => match names.as_slice() {
                [.., from, to] => cfg
                    .edges
                    .iter()
                    .position(|e| &e.from == from && &e.to == to)
                    .map_or_else(|| keys(&["edges"]), edge),
                _ => keys(&["edges"]),
            },
            TopologyError::SecondGateway(n)
            | TopologyError::MissingOps(n)
            | TopologyError::MissingCache(n)
            | TopologyError::MissingGatewayEdge(n) => keys(&["nodes", n]),
            TopologyError::NoEntry => keys(&["nodes"]),
        };
        Located::new(path, self.to_string())
    }
}

impl std::error::Error for TopologyError {}

/// **Private.** R1.24: the kinds a node of kind `from` may call.
fn callees(from: NodeKind) -> &'static [NodeKind] {
    match from {
        NodeKind::Gateway => &[NodeKind::Service, NodeKind::Monolith, NodeKind::Auth],
        NodeKind::Service | NodeKind::Monolith => &[
            NodeKind::Service,
            NodeKind::Monolith,
            NodeKind::Database,
            NodeKind::Cache,
            NodeKind::ExternalApi,
            NodeKind::Auth,
        ],
        NodeKind::Auth => &[
            NodeKind::Service,
            NodeKind::Monolith,
            NodeKind::Database,
            NodeKind::Cache,
            NodeKind::ExternalApi,
        ],
        NodeKind::Database | NodeKind::Cache | NodeKind::ExternalApi => &[],
    }
}

/// **Private.** Kinds a request can enter at when there is no gateway.
fn is_entry(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Service | NodeKind::Monolith)
}

/// **Private.** A node's op costs: the user's values, gaps filled from the flavor
/// preset (R1.25). Databases always have ops; monoliths when they set `ops` or `flavor`.
fn ops_of(name: &str, n: &NodeConfig) -> Result<Option<[Dist; 3]>, TopologyError> {
    let (flavor, ops) = match n {
        NodeConfig::Database(d) => (d.flavor, d.ops.as_ref()),
        NodeConfig::Monolith(m) if m.ops.is_some() || m.flavor.is_some() => {
            (m.flavor, m.ops.as_ref())
        }
        _ => return Ok(None),
    };
    let ops = presets::resolve_ops(flavor, ops)
        .ok_or_else(|| TopologyError::MissingOps(name.to_string()))?;
    Ok(Some(ops.map(|d| Dist::from_config(Some(&d)))))
}

/// **Private.** A cache node's model: the user's values, gaps filled from its flavor.
fn cache_of(name: &str, n: &NodeConfig) -> Result<Option<CacheModel>, TopologyError> {
    let NodeConfig::Cache(c) = n else {
        return Ok(None);
    };
    let p =
        presets::resolve_cache(c).ok_or_else(|| TopologyError::MissingCache(name.to_string()))?;
    Ok(Some(CacheModel {
        hit_ratio: p.hit_ratio,
        hit: Dist::from_config(Some(&p.hit)),
        miss: Dist::from_config(Some(&p.miss)),
    }))
}

impl Topology {
    /// Builds and validates the graph.
    ///
    /// Checks, in order: per node, op and cache costs complete (presets fill gaps) and at
    /// most one gateway; without a gateway, at least one service or monolith; per edge,
    /// both ends exist, no self edge, no duplicate; the graph is a DAG; every edge is in
    /// the R1.24 table; with a gateway, every service/monolith with a spec has an edge
    /// from it. The cycle check runs before the table so a cycle is reported as one.
    ///
    /// # Errors
    /// Returns the first [`TopologyError`] found; nothing is half-built.
    pub fn build(cfg: &Config) -> Result<Topology, TopologyError> {
        let mut nodes = Vec::new();
        let mut index = HashMap::new();
        let mut gateway = None;
        for (name, n) in &cfg.nodes {
            let id = nodes.len();
            if n.kind() == NodeKind::Gateway {
                if gateway.is_some() {
                    return Err(TopologyError::SecondGateway(name.clone()));
                }
                gateway = Some(id);
            }
            let ops = ops_of(name, n)?;
            let cache = cache_of(name, n)?;
            index.insert(name.clone(), id);
            nodes.push(Node {
                name: name.clone(),
                kind: n.kind(),
                flavor: n.flavor().map(str::to_string),
                spec: n.spec().map(str::to_string),
                proc: Dist::from_config(n.proc()),
                ops,
                cache,
                concurrency: n.concurrency(),
            });
        }
        if gateway.is_none() && !nodes.iter().any(|n| is_entry(n.kind)) {
            return Err(TopologyError::NoEntry);
        }

        let mut edges: Vec<Edge> = Vec::new();
        let mut seen = HashSet::new();
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
            let (from, to) = (look(&e.from)?, look(&e.to)?);
            if from == to {
                return Err(TopologyError::SelfLoop {
                    edge: i,
                    name: e.from.clone(),
                });
            }
            if !seen.insert((from, to)) {
                return Err(TopologyError::DuplicateEdge {
                    edge: i,
                    from: e.from.clone(),
                    to: e.to.clone(),
                });
            }
            edges.push(Edge {
                from,
                to,
                net: Dist::from_config(e.net.as_ref()),
                calls: e.calls,
            });
        }

        let topo = Topology {
            nodes,
            edges,
            gateway,
            index,
        };
        topo.check_cycles()?;

        for (i, e) in topo.edges.iter().enumerate() {
            let (from, to) = (&topo.nodes[e.from], &topo.nodes[e.to]);
            if !callees(from.kind).contains(&to.kind) {
                return Err(TopologyError::ForbiddenEdge {
                    edge: i,
                    from: from.name.clone(),
                    to: to.name.clone(),
                    from_kind: from.kind,
                    to_kind: to.kind,
                });
            }
        }

        if let Some(gw) = topo.gateway {
            for (id, n) in topo.nodes.iter().enumerate() {
                if is_entry(n.kind) && n.spec.is_some() && topo.edge(gw, id).is_none() {
                    return Err(TopologyError::MissingGatewayEdge(n.name.clone()));
                }
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

    /// **Private.** Iterative DFS cycle check with 3-colour marking (R6.12: an explicit
    /// stack, so a long chain of services cannot overflow the call stack). Edges are
    /// visited in config order, so the reported cycle is deterministic.
    fn check_cycles(&self) -> Result<(), TopologyError> {
        let mut out: Vec<Vec<NodeId>> = vec![Vec::new(); self.nodes.len()];
        for e in &self.edges {
            out[e.from].push(e.to);
        }
        // state: 0 = unvisited, 1 = on the current path, 2 = done
        let mut state = vec![0u8; self.nodes.len()];
        for start in 0..self.nodes.len() {
            if state[start] != 0 {
                continue;
            }
            // (node, index of its next edge to try); the stack is the current path.
            let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
            state[start] = 1;
            while let Some(&(n, next)) = stack.last() {
                let Some(&to) = out[n].get(next) else {
                    state[n] = 2;
                    stack.pop();
                    continue;
                };
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                match state[to] {
                    2 => {}
                    1 => {
                        let from = stack.iter().position(|&(p, _)| p == to).unwrap_or(0);
                        let mut names: Vec<String> = stack[from..]
                            .iter()
                            .map(|&(p, _)| self.nodes[p].name.clone())
                            .collect();
                        names.push(self.nodes[to].name.clone());
                        return Err(TopologyError::Cycle(names));
                    }
                    _ => {
                        state[to] = 1;
                        stack.push((to, 0));
                    }
                }
            }
        }
        Ok(())
    }
}
