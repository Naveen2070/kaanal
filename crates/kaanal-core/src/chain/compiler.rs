//! The chain compiler entry points: `compile`, `skeleton`, the limits and the `Compiler` state.

use super::{
    Arc, BTreeMap, BTreeSet, CacheStep, Call, Edge, Endpoint, Endpoints, EndpointsInput, Keys,
    Located, MAX_DEPTH, MAX_HOPS, MAX_REPEAT, NodeId, NodeKind, PathSeg, Place, Route, RouteSpec,
    Seq, Step, Topology, callees_first,
};

/// **Private.** Compile state.
pub(super) struct Compiler<'a> {
    pub(super) topo: &'a Topology,
    /// Default body per node (empty until built; callees are built first).
    pub(super) default: Vec<Arc<Seq>>,
    /// Named chains by (node, name).
    pub(super) named: BTreeMap<(NodeId, String), Arc<Seq>>,
    /// Endpoints compiled so far, by normalized key (also the `via:` lookup).
    pub(super) done: BTreeMap<String, Arc<Endpoint>>,
    pub(super) warnings: Vec<String>,
}

impl Compiler<'_> {
    /// **Private.** The node's own work: proc, plus the inline op of a monolith with ops;
    /// a database's body is its op.
    pub(super) fn prefix(&self, n: NodeId) -> Vec<Step> {
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

    pub(super) fn default_call(&self, e: &Edge, keys: &mut Keys) -> Step {
        Step::Call(self.call_of(e, keys))
    }

    /// **Private.** The call along edge `e` with the callee's default body.
    fn call_of(&self, e: &Edge, keys: &mut Keys) -> Call {
        Call {
            node: e.to,
            repeat: e.calls.get(),
            net: e.net.clone(),
            body: Arc::clone(&self.default[e.to]),
            op: None,
            prob: e.prob,
            balance: e.balance.clone(),
            sends: e.sends,
            returns: e.returns,
            key: keys.next(e.to),
        }
    }

    /// **Private.** Section 5.2: service/monolith/external-api/auth callees in edge order, then
    /// caches (reads and scans only), then databases and storage.
    pub(super) fn walk(&self, n: NodeId, keys: &mut Keys) -> Vec<Step> {
        let kind = |e: &&Edge| self.topo.nodes[e.to].kind;
        let mut out: Vec<Step> = Vec::new();
        for e in self.topo.out_edges(n).filter(|e| {
            matches!(
                kind(e),
                NodeKind::Service
                    | NodeKind::Monolith
                    | NodeKind::ExternalApi
                    | NodeKind::Auth
                    | NodeKind::LoadBalancer
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
                balance: e.balance.clone(),
                repeat: e.calls.get(),
                key: keys.next(e.to),
            }));
        }
        for e in self
            .topo
            .out_edges(n)
            .filter(|e| matches!(kind(e), NodeKind::Database | NodeKind::Storage))
        {
            out.push(self.default_call(e, keys));
        }
        out
    }

    /// **Private.** Rejects edges whose `calls` exceeds the repeat cap, then builds every
    /// default body callees-first.
    pub(super) fn defaults(&mut self, order: &[NodeId]) -> Result<(), Located> {
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
                NodeKind::LoadBalancer => {
                    let calls: Vec<Call> = self
                        .topo
                        .out_edges(n)
                        .map(|e| self.call_of(e, &mut keys))
                        .collect();
                    if !calls.is_empty() {
                        steps.push(Step::Route(Route {
                            node: n,
                            calls,
                            spread: self.topo.nodes[n].spread.clone(),
                            key: keys.next(n),
                        }));
                    }
                }
                _ => {}
            }
            self.default[n] = Arc::new(Seq::new(steps));
        }
        Ok(())
    }
}

impl Compiler<'_> {
    /// **Private.** Warns about a cache whose `timeout` is below its hit cost's median: the
    /// executor turns a lookup slower than the timeout into a miss, so most hits are cut (R1.64).
    pub(super) fn cache_timeout_warnings(&mut self) {
        for n in &self.topo.nodes {
            let (Some(t), Some(m)) = (n.timeout, n.cache.as_ref()) else {
                continue;
            };
            let hit = m.hit.median();
            if hit > t {
                self.warnings.push(format!(
                    "cache '{}': timeout {t:?} is below its hit cost (about {hit:?}), so most lookups are cut to misses",
                    n.name
                ));
            }
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
pub(super) fn normalize(key: &str) -> Option<String> {
    let (m, p) = key.split_once(' ')?;
    Some(format!("{} {}", m.to_uppercase(), p.trim()))
}

/// **Private.** The resolved-size and depth limits (E13, D9) for a chain at `place`.
pub(super) fn limits(s: &Seq, place: &Place) -> Result<(), Located> {
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
/// `endpoints:` entries and node `chains:` (spec section 5.1).
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
    c.cache_timeout_warnings();
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
