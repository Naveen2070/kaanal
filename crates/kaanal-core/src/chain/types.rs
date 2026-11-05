//! The compiled endpoint types: steps, `Endpoint`, `Endpoints`, collection operations, route specs and error resolution.

use crate::balance::Balance;

use super::{
    Arc, BTreeMap, Config, Dist, EndpointConfig, EndpointsConfig, ErrorMapping, FailKind, IndexMap,
    Located, NodeId, Op, Override, Path, PathBuf, PathSeg, StateOverrides, Topology, Value,
};

/// What answers a built-in route once its chain succeeds. Plain data: the server builds the
/// body (a token or the JWKS); an `override` on the route still wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Responder {
    /// `POST /login`: checks the credentials and issues a signed token.
    Login,
    /// `GET /.well-known/jwks.json`: the public signing key.
    Jwks,
}

/// **Private.** Methods an endpoint key may use.
pub(super) const METHODS: [&str; 7] = ["GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Which call of which target a step is: the n-th (`occ`) call or cache lookup of `target`
/// in its sequence (D8). Draws are seeded from it chained with the parent step's path, so
/// inserting a step leaves the draws of steps with other targets alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepKey {
    /// The called node.
    pub target: NodeId,
    /// How many calls to `target` came before this one in the same sequence (a body, an
    /// inline or named chain, or the gateway part); a shared body repeats its own counts.
    pub occ: u32,
}

/// One resolved step.
#[derive(Debug, Clone)]
pub enum Step {
    /// The node's own processing time.
    Proc {
        /// The node.
        node: NodeId,
    },
    /// The node's own op (a database, or a monolith with inline ops); `None` runs the op
    /// of the endpoint or of the calling step.
    Op {
        /// The node.
        node: NodeId,
        /// A fixed op, or `None` to inherit.
        op: Option<Op>,
    },
    /// A call: network hop, then the callee's body, `repeat` times in sequence.
    Call(Call),
    /// A cache lookup; a hit ends the enclosing sequence.
    Cache(CacheStep),
    /// Branches that start together; the block ends with the slowest.
    Parallel(Vec<Seq>),
    /// A load balancer's pick: one of its callees takes the call.
    Route(Route),
}

/// A load balancer's body after its own work: one call per wire out of it, of which one runs.
#[derive(Debug, Clone)]
pub struct Route {
    /// The balancer.
    pub node: NodeId,
    /// One resolved call per wire out of the balancer, in `edges:` order (weights follow it).
    pub calls: Vec<Call>,
    /// How the balancer picks (`spread:`).
    pub spread: Balance,
    /// Which pick this is, for seeding.
    pub key: StepKey,
}

impl Route {
    /// The share of calls each wire out of the balancer takes, in `calls` order, when every
    /// callee is up: equal for `round-robin` and `random`, by weight for `weighted`.
    pub fn shares(&self) -> Vec<f64> {
        let n = self.calls.len();
        let weights: Vec<f64> = match &self.spread {
            Balance::Weighted(w) if w.iter().any(|x| *x > 0) => (0..n)
                .map(|i| f64::from(w.get(i).copied().unwrap_or(0)))
                .collect(),
            _ => vec![1.0; n],
        };
        let total: f64 = weights.iter().sum();
        weights.into_iter().map(|w| w / total).collect()
    }
}

/// A resolved `call:`.
#[derive(Debug, Clone)]
pub struct Call {
    /// The callee.
    pub node: NodeId,
    /// Sequential invocations.
    pub repeat: u32,
    /// The edge's network latency.
    pub net: Dist,
    /// What the callee runs; shared between all callers of a default body.
    pub body: Arc<Seq>,
    /// A fixed op for this step (database or inline-ops monolith callee), else inherited.
    pub op: Option<Op>,
    /// Share of requests that take this call; 1.0 always takes it, with no draw.
    pub prob: f64,
    /// The edge's `balance`, which overrides the callee's own for this call.
    pub balance: Option<Balance>,
    /// Request body size this call carries (the step's, else the edge's); `None` passes the
    /// incoming size on.
    pub sends: Option<u64>,
    /// Response body size this call brings back, likewise.
    pub returns: Option<u64>,
    /// Whether the response streams (the step's, else the edge's); `None` = the callee's default.
    pub stream: Option<crate::config::StreamMode>,
    /// What a failure of this call does about it (the step's, else the edge's); `None` = no retry.
    pub retry: Option<std::sync::Arc<crate::config::RetryConfig>>,
    /// Whether the work goes on after the caller's deadline passes (the edge's `abandon`).
    pub orphan: bool,
    /// Which call this is, for seeding.
    pub key: StepKey,
}

/// A resolved `cache:` step (or a default-chain cache lookup).
#[derive(Debug, Clone)]
pub struct CacheStep {
    /// The cache node.
    pub node: NodeId,
    /// The edge's network latency.
    pub net: Dist,
    /// Default-chain lookups run for reads and scans only; written `cache:` steps always run.
    pub reads_only: bool,
    /// The edge's `balance`, which overrides the cache node's own for this lookup.
    pub balance: Option<Balance>,
    /// Lookups in sequence (an edge's `calls`); a hit stops the rest. A stored count, not
    /// that many steps, so a large `calls` costs nothing at compile time.
    pub repeat: u32,
    /// Which lookup this is, for seeding.
    pub key: StepKey,
}

/// A sequence of steps with its size, computed once.
#[derive(Debug, Clone)]
pub struct Seq {
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// Hops this sequence resolves to (saturating).
    pub hops: u64,
    /// Deepest nesting of calls and parallels (saturating).
    pub depth: u32,
}

impl Seq {
    /// A sequence with its hop count and call depth computed from its steps (saturating).
    pub fn new(steps: Vec<Step>) -> Seq {
        let (mut hops, mut depth) = (0u64, 0u32);
        for s in &steps {
            let (h, d) = match s {
                Step::Proc { .. } | Step::Op { .. } => (1, 0),
                Step::Cache(c) => (u64::from(c.repeat).saturating_mul(2), 0),
                Step::Call(c) => (
                    u64::from(c.repeat).saturating_mul(c.body.hops.saturating_add(1)),
                    c.body.depth.saturating_add(1),
                ),
                Step::Route(r) => (
                    r.calls
                        .iter()
                        .map(|c| u64::from(c.repeat).saturating_mul(c.body.hops.saturating_add(1)))
                        .max()
                        .unwrap_or(0),
                    r.calls
                        .iter()
                        .map(|c| c.body.depth.saturating_add(1))
                        .max()
                        .unwrap_or(0),
                ),
                Step::Parallel(bs) => (
                    bs.iter().fold(0u64, |a, b| a.saturating_add(b.hops)),
                    bs.iter()
                        .map(|b| b.depth)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1),
                ),
            };
            hops = hops.saturating_add(h);
            depth = depth.max(d);
        }
        Seq { steps, hops, depth }
    }
}

impl Drop for Seq {
    /// Frees nested bodies with a work list: a call tree can be thousands of calls deep
    /// (R6.12), and the default recursive drop would overflow the stack on it.
    fn drop(&mut self) {
        let mut work = vec![std::mem::take(&mut self.steps)];
        while let Some(steps) = work.pop() {
            for step in steps {
                match step {
                    Step::Call(c) => {
                        if let Ok(mut body) = Arc::try_unwrap(c.body) {
                            work.push(std::mem::take(&mut body.steps));
                        }
                    }
                    Step::Route(r) => {
                        for c in r.calls {
                            if let Ok(mut body) = Arc::try_unwrap(c.body) {
                                work.push(std::mem::take(&mut body.steps));
                            }
                        }
                    }
                    Step::Parallel(branches) => {
                        for mut b in branches {
                            work.push(std::mem::take(&mut b.steps));
                        }
                    }
                    Step::Proc { .. } | Step::Op { .. } | Step::Cache(_) => {}
                }
            }
        }
    }
}

/// One compiled endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Normalized `"METHOD /path"`.
    pub key: String,
    /// The service, monolith or auth node that owns the route; `None` for a static endpoint.
    pub owner: Option<NodeId>,
    /// Where a request enters: the gateway if there is one, else the owner (B2a Z7).
    pub entry_node: Option<NodeId>,
    /// The op database steps inherit.
    pub op: Op,
    /// The whole chain: the gateway's work, then the owner's body.
    pub chain: Arc<Seq>,
    /// The owner's body alone (what `via:` reuses).
    pub body: Arc<Seq>,
    /// Failure → response, keys normalized (node name, failure kind, status, `4xx`/`5xx`).
    pub error_map: IndexMap<String, ErrorMapping>,
    /// A fixed response served instead of generated data.
    pub response: Option<Override>,
    /// A `HEAD` copy made from a `GET` (not listed, R6.8).
    pub synthesized: bool,
    /// The `states:` overrides of this entry (what a pin to `loading`, `empty`, `error` or
    /// `slow` answers by default).
    pub states: StateOverrides,
    /// The built-in body producer of an auth node's routes; `None` for spec and static routes.
    pub responder: Option<Responder>,
    /// The in-memory collection this route reads or writes; see [`collection_op`].
    pub collection: Option<String>,
    /// Records a paged list route reports in total (0 to 10000); `None` = seeded by the route.
    pub total: Option<u32>,
    /// **Private.** Where the `endpoints:` entry sits (path and file), if there is one.
    pub(super) entry: Option<(Vec<PathSeg>, Option<PathBuf>)>,
}

impl Endpoint {
    /// Whether `node` answers on the wire for this endpoint: the gateway or the owner.
    pub fn is_wire_node(&self, topo: &Topology, node: NodeId) -> bool {
        Some(node) == topo.gateway || Some(node) == self.owner
    }
    /// An error placed at this endpoint's `endpoints:` entry (no place for a plain spec route).
    pub fn located(&self, msg: impl Into<String>) -> Located {
        let Some((path, file)) = &self.entry else {
            return Located::new(Vec::new(), msg);
        };
        let l = Located::new(path.clone(), msg);
        match file {
            Some(f) => l.in_file(f.clone()),
            None => l,
        }
    }
}

/// What a route does to its collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionOp {
    /// `GET /x`: every item.
    List,
    /// `GET /x/{p}`: one item.
    Get,
    /// `POST /x`: add an item.
    Add,
    /// `DELETE /x/{p}`: remove an item.
    Remove,
}

/// The collection operation of an uppercase `method` and a `path`, `None` for any other shape.
pub fn collection_op(method: &str, path: &str) -> Option<CollectionOp> {
    let mut segs = path.strip_prefix('/')?.split('/');
    let (first, last) = (segs.next()?, segs.next());
    // Exactly one parameter: `{name}` with no further braces inside.
    let param = |s: &str| {
        s.strip_prefix('{')
            .and_then(|r| r.strip_suffix('}'))
            .is_some_and(|n| !n.is_empty() && !n.contains(['{', '}']))
    };
    if first.is_empty() || param(first) || segs.next().is_some() {
        return None;
    }
    match (method, last) {
        ("GET", None) => Some(CollectionOp::List),
        ("POST", None) => Some(CollectionOp::Add),
        ("GET", Some(p)) if param(p) => Some(CollectionOp::Get),
        ("DELETE", Some(p)) if param(p) => Some(CollectionOp::Remove),
        _ => None,
    }
}

/// **Private.** `^[a-z][a-z0-9_-]{0,31}$`.
pub(super) fn collection_name_ok(n: &str) -> bool {
    n.len() <= 32
        && n.starts_with(|c: char| c.is_ascii_lowercase())
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Every compiled endpoint, by key.
#[derive(Debug, Clone, Default)]
pub struct Endpoints {
    /// **Private.** Sorted by key, so iteration is deterministic.
    pub(super) by_key: BTreeMap<String, Arc<Endpoint>>,
    /// Problems that do not stop the load (stale entries, ignored `via` error maps).
    pub warnings: Vec<String>,
}

impl Endpoints {
    /// The endpoint with this normalized key.
    pub fn get(&self, key: &str) -> Option<&Arc<Endpoint>> {
        self.by_key.get(key)
    }

    /// All endpoints, sorted by key.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Endpoint>> {
        self.by_key.values()
    }
}

/// A route a spec defines, as the server reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteSpec {
    /// Normalized `"METHOD /path"`.
    pub key: String,
    /// The service or monolith that serves it.
    pub owner: NodeId,
    /// The op inferred from method and path.
    pub op: Op,
}

impl RouteSpec {
    /// A route with its op inferred from the key (`GET /x/{id}` reads, `GET /x` scans).
    pub fn inferred(key: &str, owner: NodeId) -> RouteSpec {
        let (m, p) = key.split_once(' ').unwrap_or((key, ""));
        RouteSpec {
            key: key.to_string(),
            owner,
            op: Op::infer(m, p),
        }
    }
}

/// Where the `endpoints:` entries come from.
#[derive(Debug, Clone, Copy, Default)]
pub struct EndpointsInput<'a> {
    /// The entries, in file order.
    pub entries: Option<&'a IndexMap<String, EndpointConfig>>,
    /// The external endpoints file they were read from; `None` = inline in `kaanal.yaml`.
    pub file: Option<&'a Path>,
}

impl<'a> EndpointsInput<'a> {
    /// The inline `endpoints:` map of `cfg`, if it has one.
    pub fn inline(cfg: &'a Config) -> EndpointsInput<'a> {
        match &cfg.endpoints {
            Some(EndpointsConfig::Inline(m)) => EndpointsInput {
                entries: Some(m),
                file: None,
            },
            _ => EndpointsInput::default(),
        }
    }
}

/// The status a failure answers with when no `error_map` entry matches: the failure's own for
/// [`FailKind::Status`], 502 for a reset or malformed reply, 504 for a timeout.
pub fn default_status(kind: FailKind, status: u16) -> u16 {
    match kind {
        FailKind::Status => status,
        FailKind::Reset | FailKind::Malformed => 502,
        FailKind::Timeout => 504,
    }
}

/// The response for a failure (E5, D11, B2a Z8): the map entry for the failing node, else for
/// the failure kind (`reset`, `malformed`, `timeout`), else for the exact status, else for its
/// class (`5xx`); without one the kind's default status and no body. A node named like a kind
/// (`reset`, `malformed`, `timeout`) is found by the node lookup first (Z8), so for that node's own failure an entry
/// under that name is the node's. The same key still acts as the kind key for every other node's
/// failure of that kind (the lookup is by string, not by what the key was meant for).
pub fn resolve_error(
    map: &IndexMap<String, ErrorMapping>,
    node: &str,
    kind: FailKind,
    status: u16,
) -> (u16, Option<Value>) {
    let status = default_status(kind, status);
    map.get(node)
        .or_else(|| {
            (kind != FailKind::Status)
                .then(|| map.get(kind.as_str()))
                .flatten()
        })
        .or_else(|| map.get(&status.to_string()))
        .or_else(|| map.get(&format!("{}xx", status / 100)))
        .map_or((status, None), |m| (m.status, m.body.clone()))
}
