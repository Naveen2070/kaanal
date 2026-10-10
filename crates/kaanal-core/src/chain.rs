//! Endpoint chains: compiles `endpoints:`, node `chains:` and the topology into resolved
//! call trees (spec §5.1–§5.2).
//!
//! [`compile`] runs once at load. It resolves every node name, edge, `via:` and `with:`,
//! gives routes without a custom chain the default chain, checks every limit and returns
//! [`Endpoints`]: one [`Endpoint`] per route, each a tree of [`Step`]s the executor walks
//! with no lookups. Default bodies are built once per node, callees first, and shared
//! through `Arc`. They fix no op: a database step runs the op of whoever called it (E10),
//! so one body serves reads and writes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use crate::config::{
    CallStep, Config, EndpointConfig, EndpointsConfig, ErrorMapping, NodeKind, Override, StepConfig,
};
use crate::diagnostics::{Located, PathSeg};
use crate::dist::Dist;
use crate::exec::Op;
use crate::topology::{Edge, NodeId, Topology};

/// Most steps in one chain list.
pub const MAX_STEPS: usize = 256;
/// Most branches in one `parallel` step.
pub const MAX_BRANCHES: usize = 32;
/// Deepest nesting of `parallel` and inline chains inside one endpoint.
pub const MAX_NESTING: usize = 16;
/// Largest `repeat` (and edge `calls`) a chain may resolve.
pub const MAX_REPEAT: u32 = 1000;
/// Most hops an endpoint may resolve to after expanding defaults, `via` and `with`.
pub const MAX_HOPS: u64 = 100_000;
/// Deepest resolved call tree (D9).
pub const MAX_DEPTH: u32 = 64;

/// **Private.** Methods an endpoint key may use.
const METHODS: [&str; 7] = ["GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS"];

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
    /// The service or monolith that owns the route; `None` for a static endpoint.
    pub owner: Option<NodeId>,
    /// The op database steps inherit.
    pub op: Op,
    /// The whole chain: the gateway's work, then the owner's body.
    pub chain: Arc<Seq>,
    /// The owner's body alone (what `via:` reuses).
    pub body: Arc<Seq>,
    /// Failure → response, keys normalized (node name, status, `4xx`/`5xx`).
    pub error_map: IndexMap<String, ErrorMapping>,
    /// A fixed response served instead of generated data.
    pub response: Option<Override>,
    /// A `HEAD` copy made from a `GET` (not listed, R6.8).
    pub synthesized: bool,
    /// **Private.** Where the `endpoints:` entry sits (path and file), if there is one.
    entry: Option<(Vec<PathSeg>, Option<PathBuf>)>,
}

impl Endpoint {
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

/// Every compiled endpoint, by key.
#[derive(Debug, Clone, Default)]
pub struct Endpoints {
    /// **Private.** Sorted by key, so iteration is deterministic.
    by_key: BTreeMap<String, Arc<Endpoint>>,
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

/// The response for a failure (E5, D11): the map entry for the failing node, else for the
/// exact status, else for its class (`5xx`); without one the failure's own status and no body.
pub fn resolve_error(
    map: &IndexMap<String, ErrorMapping>,
    node: &str,
    status: u16,
) -> (u16, Option<Value>) {
    map.get(node)
        .or_else(|| map.get(&status.to_string()))
        .or_else(|| map.get(&format!("{}xx", status / 100)))
        .map_or((status, None), |m| (m.status, m.body.clone()))
}

/// **Private.** Where a step or entry sits: message prefix, path, file.
#[derive(Clone)]
struct Place {
    ctx: String,
    path: Vec<PathSeg>,
    file: Option<PathBuf>,
}

impl Place {
    fn key(&self, k: &str) -> Place {
        let mut p = self.clone();
        p.path.push(PathSeg::Key(k.to_string()));
        p
    }

    fn index(&self, i: usize) -> Place {
        let mut p = self.clone();
        p.path.push(PathSeg::Index(i));
        p
    }

    /// **Private.** `ctx: msg` at the key or list item.
    fn err(&self, msg: impl std::fmt::Display) -> Located {
        self.place(Located::new(
            self.path.clone(),
            format!("{}: {msg}", self.ctx),
        ))
    }

    /// **Private.** `ctx: msg` at the value.
    fn bad(&self, msg: impl std::fmt::Display) -> Located {
        self.place(Located::at_value(
            self.path.clone(),
            format!("{}: {msg}", self.ctx),
        ))
    }

    fn place(&self, l: Located) -> Located {
        match &self.file {
            Some(f) => l.in_file(f.clone()),
            None => l,
        }
    }
}

/// **Private.** Occurrence counter per target within one sequence (D8).
struct Keys(Vec<u32>);

impl Keys {
    fn new(nodes: usize) -> Keys {
        Keys(vec![0; nodes])
    }

    /// **Private.** A counter that continues after `steps` (the gateway part of a chain).
    fn after(steps: &[Step], nodes: usize) -> Keys {
        let mut k = Keys::new(nodes);
        for s in steps {
            match s {
                Step::Call(c) => {
                    k.next(c.node);
                }
                Step::Cache(c) => {
                    k.next(c.node);
                }
                _ => {}
            }
        }
        k
    }

    fn next(&mut self, target: NodeId) -> StepKey {
        let occ = self.0[target];
        self.0[target] += 1;
        StepKey { target, occ }
    }
}

/// **Private.** Nodes callees-first (reverse topological order, ties by id). Iterative
/// (R6.12): Kahn's algorithm on out-degree.
fn callees_first(topo: &Topology) -> Vec<NodeId> {
    let n = topo.nodes.len();
    let (mut left, mut callers) = (vec![0usize; n], vec![Vec::new(); n]);
    for e in &topo.edges {
        left[e.from] += 1;
        callers[e.to].push(e.from);
    }
    let mut ready: VecDeque<NodeId> = (0..n).filter(|&i| left[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(x) = ready.pop_front() {
        order.push(x);
        for &c in &callers[x] {
            left[c] -= 1;
            if left[c] == 0 {
                ready.push_back(c);
            }
        }
    }
    order
}

/// **Private.** Compile state.
struct Compiler<'a> {
    topo: &'a Topology,
    /// Default body per node (empty until built; callees are built first).
    default: Vec<Arc<Seq>>,
    /// Named chains by (node, name).
    named: BTreeMap<(NodeId, String), Arc<Seq>>,
    /// Endpoints compiled so far, by normalized key (also the `via:` lookup).
    done: BTreeMap<String, Arc<Endpoint>>,
    warnings: Vec<String>,
}

impl Compiler<'_> {
    /// **Private.** The node's own work: proc, plus the inline op of a monolith with ops;
    /// a database's body is its op.
    fn prefix(&self, n: NodeId) -> Vec<Step> {
        let node = &self.topo.nodes[n];
        match node.kind {
            NodeKind::Database => vec![Step::Op { node: n, op: None }],
            NodeKind::Cache => Vec::new(),
            NodeKind::Monolith if node.ops.is_some() => {
                vec![Step::Proc { node: n }, Step::Op { node: n, op: None }]
            }
            _ => vec![Step::Proc { node: n }],
        }
    }

    fn default_call(&self, e: &Edge, keys: &mut Keys) -> Step {
        Step::Call(Call {
            node: e.to,
            repeat: e.calls.get(),
            net: e.net.clone(),
            body: Arc::clone(&self.default[e.to]),
            op: None,
            key: keys.next(e.to),
        })
    }

    /// **Private.** §5.2: service/monolith/external-api/auth callees in edge order, then
    /// caches (reads and scans only), then databases.
    fn walk(&self, n: NodeId, keys: &mut Keys) -> Vec<Step> {
        let kind = |e: &&Edge| self.topo.nodes[e.to].kind;
        let mut out: Vec<Step> = Vec::new();
        for e in self.topo.out_edges(n).filter(|e| {
            matches!(
                kind(e),
                NodeKind::Service | NodeKind::Monolith | NodeKind::ExternalApi | NodeKind::Auth
            )
        }) {
            out.push(self.default_call(e, keys));
        }
        for e in self
            .topo
            .out_edges(n)
            .filter(|e| kind(e) == NodeKind::Cache)
        {
            out.push(Step::Cache(CacheStep {
                node: e.to,
                net: e.net.clone(),
                reads_only: true,
                repeat: e.calls.get(),
                key: keys.next(e.to),
            }));
        }
        for e in self
            .topo
            .out_edges(n)
            .filter(|e| kind(e) == NodeKind::Database)
        {
            out.push(self.default_call(e, keys));
        }
        out
    }

    /// **Private.** Rejects edges whose `calls` exceeds the repeat cap, then builds every
    /// default body callees-first.
    fn defaults(&mut self, order: &[NodeId]) -> Result<(), Located> {
        for (i, e) in self.topo.edges.iter().enumerate() {
            if e.calls.get() > MAX_REPEAT {
                return Err(Located::at_value(
                    vec![
                        PathSeg::Key("edges".into()),
                        PathSeg::Index(i),
                        PathSeg::Key("calls".into()),
                    ],
                    format!(
                        "edge #{i}: calls must be at most {MAX_REPEAT} when a chain repeats it (got {})",
                        e.calls
                    ),
                ));
            }
        }
        for &n in order {
            let mut keys = Keys::new(self.topo.nodes.len());
            let mut steps = self.prefix(n);
            match self.topo.nodes[n].kind {
                NodeKind::Gateway => {
                    for e in self
                        .topo
                        .out_edges(n)
                        .filter(|e| self.topo.nodes[e.to].kind == NodeKind::Auth)
                    {
                        steps.push(self.default_call(e, &mut keys));
                    }
                }
                NodeKind::Service | NodeKind::Monolith => steps.extend(self.walk(n, &mut keys)),
                _ => {}
            }
            self.default[n] = Arc::new(Seq::new(steps));
        }
        Ok(())
    }
}

/// **Private.** One endpoint to compile.
struct Job<'e> {
    key: String,
    owner: Option<NodeId>,
    op: Op,
    cfg: Option<&'e EndpointConfig>,
    error_map: IndexMap<String, ErrorMapping>,
    place: Place,
}

/// **Private.** A valid `error_map` key, normalized: a node name, `400..=599`, `4xx`/`5xx`.
fn error_key(k: &str, topo: &Topology) -> Option<String> {
    if topo.id(k).is_some() {
        return Some(k.to_string());
    }
    let lower = k.to_ascii_lowercase();
    if lower == "4xx" || lower == "5xx" {
        return Some(lower);
    }
    // Three plain digits: `u16::parse` would also take `+503` and `0503`.
    (k.len() == 3 && k.bytes().all(|b| b.is_ascii_digit()))
        .then(|| k.parse::<u16>().ok())
        .flatten()
        .filter(|s| (400..=599).contains(s))
        .map(|s| s.to_string())
}

/// **Private.** An `endpoints:` entry whose key and values were checked.
struct Entry<'e> {
    written: String,
    method: String,
    path: String,
    cfg: &'e EndpointConfig,
    error_map: IndexMap<String, ErrorMapping>,
    place: Place,
}

impl Compiler<'_> {
    /// **Private.** Checks the key and values of every `endpoints:` entry, keyed by the
    /// normalized route, in file order.
    fn entries<'e>(
        &self,
        input: EndpointsInput<'e>,
    ) -> Result<IndexMap<String, Entry<'e>>, Located> {
        let topo = self.topo;
        let prefix = if input.file.is_none() {
            vec![PathSeg::Key("endpoints".into())]
        } else {
            Vec::new()
        };
        let file = input.file.map(Path::to_path_buf);
        let mut entries: IndexMap<String, Entry<'e>> = IndexMap::new();
        let mut written_as: BTreeMap<String, String> = BTreeMap::new();
        for (written, cfg) in input.entries.into_iter().flatten() {
            let mut path = prefix.clone();
            path.push(PathSeg::Key(written.clone()));
            let place = Place {
                ctx: format!("endpoint '{written}'"),
                path,
                file: file.clone(),
            };
            let Some((m, p)) = written.split_once(' ') else {
                return Err(place.err("key must look like 'GET /path'"));
            };
            let m = m.to_ascii_uppercase();
            if !METHODS.contains(&m.as_str()) {
                return Err(place.err(format!("unknown method '{m}'")));
            }
            let p = p.trim();
            if !p.starts_with('/') {
                return Err(place.err("path must start with '/'"));
            }
            let key = format!("{m} {p}");
            if let Some(first) = written_as.get(&key) {
                return Err(place.err(format!("same route as '{first}'")));
            }
            written_as.insert(key.clone(), written.clone());
            if let Some(s) = cfg.response.as_ref().map(|o| o.status)
                && !(200..=599).contains(&s)
            {
                return Err(place
                    .key("override")
                    .key("status")
                    .bad(format!("override: status {s} must be between 200 and 599")));
            }
            let mut error_map = IndexMap::new();
            let mut written_keys: BTreeMap<String, String> = BTreeMap::new();
            for (k, v) in &cfg.error_map {
                let at = place.key("error_map").key(k);
                let Some(norm) = error_key(k, topo) else {
                    return Err(at.err(format!(
                        "error_map: '{k}' is not a node name, a status (400-599) or a class (4xx, 5xx)"
                    )));
                };
                if !(200..=599).contains(&v.status) {
                    return Err(at.bad(format!(
                        "error_map: status {} must be between 200 and 599",
                        v.status
                    )));
                }
                if let Some(first) = written_keys.get(&norm) {
                    return Err(at.err(format!("error_map: '{k}' is the same key as '{first}'")));
                }
                written_keys.insert(norm.clone(), k.clone());
                error_map.insert(norm, v.clone());
            }
            entries.insert(
                key,
                Entry {
                    written: written.clone(),
                    method: m,
                    path: p.to_string(),
                    cfg,
                    error_map,
                    place,
                },
            );
        }
        Ok(entries)
    }

    /// **Private.** Lists the endpoints to compile: spec routes in order (with their entry,
    /// if any), then `HEAD` entries of a `GET` route, then static entries. Stale entries
    /// become warnings.
    fn jobs<'e>(
        &mut self,
        routes: &[RouteSpec],
        input: EndpointsInput<'e>,
    ) -> Result<Vec<Job<'e>>, Located> {
        let topo = self.topo;
        let mut entries = self.entries(input)?;
        let mut jobs = Vec::new();
        for r in routes {
            let job = match entries.shift_remove(&r.key) {
                Some(e) => Job {
                    key: r.key.clone(),
                    owner: Some(r.owner),
                    op: r.op,
                    cfg: Some(e.cfg),
                    error_map: e.error_map,
                    place: e.place,
                },
                None => Job {
                    key: r.key.clone(),
                    owner: Some(r.owner),
                    op: r.op,
                    cfg: None,
                    error_map: IndexMap::new(),
                    // No entry to point at: a limit hit by the default chain is placed at the
                    // owning node, whose edges (`calls`) decide its size.
                    place: Place {
                        ctx: format!("endpoint '{}'", r.key),
                        path: vec![
                            PathSeg::Key("nodes".into()),
                            PathSeg::Key(topo.nodes[r.owner].name.clone()),
                        ],
                        file: None,
                    },
                },
            };
            jobs.push(job);
        }
        // GET entries with an override: a HEAD entry for one is served as a copy of it.
        let static_gets: Vec<String> = entries
            .iter()
            .filter(|(k, e)| k.starts_with("GET ") && e.cfg.response.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for (key, e) in entries {
            let get = (e.method == "HEAD")
                .then(|| routes.iter().find(|r| r.key == format!("GET {}", e.path)))
                .flatten();
            let (owner, op) = if let Some(r) = get {
                (Some(r.owner), r.op)
            } else if e.cfg.response.is_none()
                && static_gets.contains(&format!("GET {}", e.path))
                && e.method == "HEAD"
            {
                self.warnings.push(format!(
                    "endpoint '{}': ignored: a HEAD of a static GET entry is served as a copy of that GET (give it an override to serve it separately)",
                    e.written
                ));
                continue;
            } else if e.cfg.response.is_none() {
                self.warnings.push(format!(
                    "endpoint '{}': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)",
                    e.written
                ));
                continue;
            } else if topo.gateway.is_none() {
                return Err(e
                    .place
                    .err("static route needs a gateway or a service that owns it"));
            } else {
                (None, Op::infer(&e.method, &e.path))
            };
            jobs.push(Job {
                key,
                owner,
                op,
                cfg: Some(e.cfg),
                error_map: e.error_map,
                place: e.place,
            });
        }
        Ok(jobs)
    }

    /// **Private.** `"chains: a, b"` or `"it has no chains"`.
    fn chain_names(&self, n: NodeId) -> String {
        let node = &self.topo.nodes[n];
        if node.chains.is_empty() {
            "it has no chains".to_string()
        } else {
            let names: Vec<&str> = node.chains.keys().map(String::as_str).collect();
            format!("chains: {}", names.join(", "))
        }
    }

    /// **Private.** `"endpoints: K1, K2"` or `"it has none"` (sorted, no HEAD copies).
    fn endpoint_names(&self, n: NodeId) -> String {
        let keys: Vec<&str> = self
            .done
            .values()
            .filter(|e| e.owner == Some(n) && !e.synthesized)
            .map(|e| e.key.as_str())
            .collect();
        if keys.is_empty() {
            "it has none".to_string()
        } else {
            format!("endpoints: {}", keys.join(", "))
        }
    }

    fn steps(
        &mut self,
        list: &[StepConfig],
        runner: NodeId,
        at: &Place,
        depth: usize,
        keys: &mut Keys,
    ) -> Result<Vec<Step>, Located> {
        if list.is_empty() {
            return Err(at.err("a chain needs at least one step"));
        }
        if list.len() > MAX_STEPS {
            return Err(at.err(format!(
                "a chain allows at most {MAX_STEPS} steps (got {})",
                list.len()
            )));
        }
        list.iter()
            .enumerate()
            .map(|(i, s)| self.step(s, runner, &at.index(i), depth, keys))
            .collect()
    }

    fn step(
        &mut self,
        s: &StepConfig,
        runner: NodeId,
        at: &Place,
        depth: usize,
        keys: &mut Keys,
    ) -> Result<Step, Located> {
        if depth > MAX_NESTING {
            return Err(at.err(format!("chains nest at most {MAX_NESTING} levels deep")));
        }
        match s {
            StepConfig::Call(c) => self.call(c, runner, at, depth, keys).map(Step::Call),
            StepConfig::Cache { node } => {
                let at = at.key("cache");
                let to = self
                    .topo
                    .id(node)
                    .ok_or_else(|| at.bad(format!("unknown node '{node}'")))?;
                let kind = self.topo.nodes[to].kind;
                if kind != NodeKind::Cache {
                    return Err(at.bad(format!("cache: '{node}' is a {kind}, not a cache")));
                }
                let net = self.edge(runner, to, &at)?.net.clone();
                Ok(Step::Cache(CacheStep {
                    node: to,
                    net,
                    reads_only: false,
                    repeat: 1,
                    key: keys.next(to),
                }))
            }
            StepConfig::Parallel(branches) => {
                let at = at.key("parallel");
                if branches.len() < 2 {
                    return Err(at.err("parallel needs at least 2 branches"));
                }
                if branches.len() > MAX_BRANCHES {
                    return Err(at.err(format!(
                        "parallel allows at most {MAX_BRANCHES} branches (got {})",
                        branches.len()
                    )));
                }
                let mut seqs = Vec::with_capacity(branches.len());
                for (b, br) in branches.iter().enumerate() {
                    let here = at.index(b);
                    // D8: one counter for the whole block, so it is transparent for occurrences.
                    let steps = if br.list {
                        self.steps(&br.steps, runner, &here, depth + 1, keys)?
                    } else {
                        let mut one = Vec::with_capacity(1);
                        for s in &br.steps {
                            one.push(self.step(s, runner, &here, depth + 1, keys)?);
                        }
                        one
                    };
                    seqs.push(Seq::new(steps));
                }
                Ok(Step::Parallel(seqs))
            }
        }
    }

    /// **Private.** The edge `runner → to`, or the "no edge" error at the target.
    fn edge(&self, runner: NodeId, to: NodeId, at: &Place) -> Result<&Edge, Located> {
        self.topo.edge(runner, to).ok_or_else(|| {
            at.bad(format!(
                "'{}' has no edge to '{}' (add it under edges:)",
                self.topo.nodes[runner].name, self.topo.nodes[to].name
            ))
        })
    }

    fn call(
        &mut self,
        c: &CallStep,
        runner: NodeId,
        at: &Place,
        depth: usize,
        keys: &mut Keys,
    ) -> Result<Call, Located> {
        let target = if c.bare { at.clone() } else { at.key("call") };
        let to = self
            .topo
            .id(&c.node)
            .ok_or_else(|| target.bad(format!("unknown node '{}'", c.node)))?;
        let (name, kind) = (self.topo.nodes[to].name.clone(), self.topo.nodes[to].kind);
        if kind == NodeKind::Cache {
            return Err(target.bad(format!("'{name}' is a cache; use `cache: {name}`")));
        }
        let edge = self.edge(runner, to, &target)?;
        let (net, calls) = (edge.net.clone(), edge.calls.get());
        let bodies = usize::from(c.chain.is_some())
            + usize::from(c.with.is_some())
            + usize::from(c.via.is_some());
        if bodies > 1 {
            return Err(at.err("use only one of chain, with and via"));
        }
        if bodies == 1 && !matches!(kind, NodeKind::Service | NodeKind::Monolith) {
            return Err(at.err(format!(
                "'{name}' ({kind}) has no chains; chain, with and via need a service or monolith"
            )));
        }
        if c.op.is_some() {
            if c.via.is_some() {
                return Err(at
                    .key("op")
                    .bad("op cannot be used with via (the endpoint's own op is used)"));
            }
            let has_ops = kind == NodeKind::Database
                || (kind == NodeKind::Monolith && self.topo.nodes[to].ops.is_some());
            if !has_ops {
                return Err(at.key("op").bad(format!("'{name}' ({kind}) has no ops")));
            }
        }
        let repeat = match c.repeat {
            None => calls,
            Some(r) if (1..=MAX_REPEAT).contains(&r) => r,
            Some(r) => {
                return Err(at.key("repeat").bad(format!(
                    "repeat must be between 1 and {MAX_REPEAT} (got {r})"
                )));
            }
        };
        let (body, op) = if let Some(steps) = &c.chain {
            let mut s = self.prefix(to);
            let mut k = Keys::new(self.topo.nodes.len());
            s.extend(self.steps(steps, to, &at.key("chain"), depth + 1, &mut k)?);
            (Arc::new(Seq::new(s)), c.op)
        } else if let Some(w) = &c.with {
            let Some(b) = self.named.get(&(to, w.clone())) else {
                return Err(at.key("with").bad(format!(
                    "with: '{w}' is not a chain of '{name}' ({})",
                    self.chain_names(to)
                )));
            };
            (Arc::clone(b), c.op)
        } else if let Some(v) = &c.via {
            let t = normalize(v)
                .and_then(|k| self.done.get(&k))
                .filter(|t| t.owner == Some(to) && !t.synthesized)
                .cloned();
            let Some(t) = t else {
                return Err(at.key("via").bad(format!(
                    "via: '{v}' is not an endpoint of '{name}' ({})",
                    self.endpoint_names(to)
                )));
            };
            if !t.error_map.is_empty() {
                self.warnings.push(format!(
                    "{}: via '{v}': that endpoint's error_map is ignored here (each endpoint maps its own errors)",
                    at.ctx
                ));
            }
            if t.response.is_some() {
                self.warnings.push(format!(
                    "{}: via '{v}': that endpoint's override is ignored here (via runs its chain, not its fixed response)",
                    at.ctx
                ));
            }
            (Arc::clone(&t.body), Some(t.op))
        } else {
            (Arc::clone(&self.default[to]), c.op)
        };
        Ok(Call {
            node: to,
            repeat,
            net,
            body,
            op,
            key: keys.next(to),
        })
    }

    fn node_chains(&mut self, n: NodeId) -> Result<(), Located> {
        let topo = self.topo;
        let node = &topo.nodes[n];
        for (name, steps) in &node.chains {
            let place = Place {
                ctx: format!("node '{}': chain '{name}'", node.name),
                path: vec![
                    PathSeg::Key("nodes".into()),
                    PathSeg::Key(node.name.clone()),
                    PathSeg::Key("chains".into()),
                    PathSeg::Key(name.clone()),
                ],
                file: None,
            };
            let mut s = self.prefix(n);
            let mut k = Keys::new(topo.nodes.len());
            s.extend(self.steps(steps, n, &place, 1, &mut k)?);
            let seq = Seq::new(s);
            limits(&seq, &place)?;
            self.named.insert((n, name.clone()), Arc::new(seq));
        }
        Ok(())
    }

    fn endpoint(&mut self, j: &Job<'_>) -> Result<(), Located> {
        let op = j.cfg.and_then(|c| c.op).unwrap_or(j.op);
        let custom = j.cfg.and_then(|c| c.chain.as_ref());
        let n = self.topo.nodes.len();
        let body = match (j.owner, custom) {
            (Some(o), Some(steps)) => {
                let mut s = self.prefix(o);
                s.extend(self.steps(steps, o, &j.place.key("chain"), 1, &mut Keys::new(n))?);
                Arc::new(Seq::new(s))
            }
            (Some(o), None) => Arc::clone(&self.default[o]),
            (None, custom) => {
                let Some(gw) = self.topo.gateway else {
                    return Err(j
                        .place
                        .err("static route needs a gateway or a service that owns it"));
                };
                let mut s = self.default[gw].steps.clone();
                if let Some(steps) = custom {
                    let mut k = Keys::after(&s, n);
                    s.extend(self.steps(steps, gw, &j.place.key("chain"), 1, &mut k)?);
                }
                Arc::new(Seq::new(s))
            }
        };
        let chain = match (self.topo.gateway, j.owner) {
            (Some(gw), Some(o)) => {
                let net = self.edge(gw, o, &j.place)?.net.clone();
                let mut steps = self.default[gw].steps.clone();
                let key = Keys::after(&steps, n).next(o);
                steps.push(Step::Call(Call {
                    node: o,
                    repeat: 1,
                    net,
                    body: Arc::clone(&body),
                    op: None,
                    key,
                }));
                Arc::new(Seq::new(steps))
            }
            _ => Arc::clone(&body),
        };
        limits(&chain, &j.place)?;
        let ep = Endpoint {
            key: j.key.clone(),
            owner: j.owner,
            op,
            chain,
            body,
            error_map: j.error_map.clone(),
            response: j.cfg.and_then(|c| c.response.clone()),
            synthesized: false,
            entry: j.cfg.map(|_| (j.place.path.clone(), j.place.file.clone())),
        };
        self.done.insert(j.key.clone(), Arc::new(ep));
        Ok(())
    }

    /// **Private.** D20: every GET without a HEAD gets a `HEAD` copy with its own key.
    fn heads(&mut self) {
        let gets: Vec<Arc<Endpoint>> = self
            .done
            .values()
            .filter(|e| e.key.starts_with("GET "))
            .cloned()
            .collect();
        for g in gets {
            let key = format!("HEAD {}", &g.key["GET ".len()..]);
            self.done.entry(key.clone()).or_insert_with(|| {
                let mut h = (*g).clone();
                h.key = key;
                h.synthesized = true;
                Arc::new(h)
            });
        }
    }
}

/// Skeleton `endpoints:` lines for spec routes with no entry yet (E3, D22): one
/// `"KEY": {} # owner NODE, op OP` line per route, sorted. `existing` keys match after
/// normalizing the method's case and the path's spaces; HEAD copies are never listed.
pub fn skeleton(topo: &Topology, routes: &[RouteSpec], existing: &[String]) -> Vec<String> {
    let have: BTreeSet<String> = existing.iter().filter_map(|k| normalize(k)).collect();
    let mut out: Vec<String> = routes
        .iter()
        .filter(|r| !have.contains(&r.key))
        .map(|r| {
            // A JSON string is a valid YAML double-quoted key, escapes included.
            let key = serde_json::Value::String(r.key.clone());
            format!(
                "{key}: {{}} # owner {}, op {}",
                topo.nodes[r.owner].name,
                r.op.name()
            )
        })
        .collect();
    out.sort();
    out
}

/// **Private.** `"get /x"` → `"GET /x"`; `None` if it is not `METHOD /path`.
fn normalize(key: &str) -> Option<String> {
    let (m, p) = key.split_once(' ')?;
    Some(format!("{} {}", m.to_uppercase(), p.trim()))
}

/// **Private.** The resolved-size and depth limits (E13, D9) for a chain at `place`.
fn limits(s: &Seq, place: &Place) -> Result<(), Located> {
    if s.hops > MAX_HOPS {
        return Err(place.err(format!(
            "resolves to {} hops (max {MAX_HOPS}); lower repeat or calls",
            s.hops
        )));
    }
    if s.depth > MAX_DEPTH {
        return Err(place.err(format!(
            "resolves to calls nested {} deep (max {MAX_DEPTH})",
            s.depth
        )));
    }
    Ok(())
}

/// Compiles every endpoint: spec `routes` (from the server, which reads the spec files),
/// `endpoints:` entries and node `chains:` (spec §5.1).
///
/// Order (D21): default bodies callees-first; then per node, callees-first, its named
/// chains and the endpoints it owns; then static endpoints; then HEAD copies (D20). A
/// `via:` target is therefore always compiled before its caller, so `via:` cannot cycle.
///
/// # Errors
/// The first problem as a [`Located`]: a malformed endpoint key, a bad status or
/// `error_map` key, an unknown node, a missing edge, a wrong target kind, a misplaced
/// `op`, `repeat` or `with`/`via`, an unknown `with`/`via` target, an empty chain, a static
/// route with no gateway, or a limit (steps, branches, nesting, `repeat`, resolved hops,
/// resolved depth).
pub fn compile(
    topo: &Topology,
    routes: &[RouteSpec],
    input: EndpointsInput<'_>,
) -> Result<Endpoints, Located> {
    debug_assert!(
        routes
            .iter()
            .enumerate()
            .all(|(i, r)| routes[..i].iter().all(|p| p.key != r.key)),
        "compile: route keys must be unique (the server rejects duplicates first)"
    );
    let order = callees_first(topo);
    let mut c = Compiler {
        topo,
        default: std::iter::repeat_with(|| Arc::new(Seq::new(Vec::new())))
            .take(topo.nodes.len())
            .collect(),
        named: BTreeMap::new(),
        done: BTreeMap::new(),
        warnings: Vec::new(),
    };
    c.defaults(&order)?;
    let jobs = c.jobs(routes, input)?;
    for &n in &order {
        if matches!(topo.nodes[n].kind, NodeKind::Service | NodeKind::Monolith) {
            c.node_chains(n)?;
        }
        for j in jobs.iter().filter(|j| j.owner == Some(n)) {
            c.endpoint(j)?;
        }
    }
    for j in jobs.iter().filter(|j| j.owner.is_none()) {
        c.endpoint(j)?;
    }
    c.heads();
    Ok(Endpoints {
        by_key: c.done,
        warnings: c.warnings,
    })
}
