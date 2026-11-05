//! `Compiler` methods that compile a whole endpoint: node chains, state overrides, heads.

use super::{
    Arc, BTreeMap, Call, Compiler, Endpoint, EndpointConfig, Job, Keys, Located, NodeId, NodeKind,
    PathSeg, PinState, Place, Responder, Seq, StateOverride, StateOverrides, Step, check_entry,
    limits,
};

impl Compiler<'_> {
    pub(super) fn node_chains(&mut self, n: NodeId) -> Result<(), Located> {
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

    /// **Private.** The checked `states:` of a job's entry: keys `loading|empty|error|slow`,
    /// values in range.
    pub(super) fn states(
        cfg: Option<&EndpointConfig>,
        place: &Place,
    ) -> Result<StateOverrides, Located> {
        let mut out = StateOverrides::default();
        for (k, v) in cfg.into_iter().flat_map(|c| &c.states) {
            let at = place.key("states").key(k);
            let state = match PinState::parse(k) {
                Some(PinState::Success) => {
                    return Err(
                        at.err("states: 'success' is the normal response and cannot be overridden")
                    );
                }
                Some(s) => s,
                None => {
                    return Err(at.err(format!(
                        "states: '{k}' must be one of loading, empty, error, slow"
                    )));
                }
            };
            if let Err((f, m)) = check_entry(state, v.status, v.delay) {
                return Err(at.key(f).bad(format!("states: {m}")));
            }
            if v.body.is_some() && matches!(state, PinState::Loading | PinState::Slow) {
                return Err(at
                    .key("body")
                    .bad(format!("states: a {} override takes no body", state.name())));
            }
            out.set(
                state,
                StateOverride {
                    status: v.status,
                    body: v.body.clone(),
                    delay: v.delay,
                },
            );
        }
        Ok(out)
    }

    pub(super) fn endpoint(&mut self, j: &Job<'_>) -> Result<(), Located> {
        let op = j.cfg.and_then(|c| c.op).unwrap_or(j.op);
        let custom = j.cfg.and_then(|c| c.chain.as_ref());
        let n = self.topo.nodes.len();
        let body = match (j.owner, custom) {
            (Some(o), Some(steps)) => {
                let mut s = self.prefix(o);
                s.extend(self.steps(steps, o, &j.place.key("chain"), 1, &mut Keys::new(n))?);
                Arc::new(Seq::new(s))
            }
            // X12: login walks the auth node's out-edges like a service's default body. JWKS,
            // and the gateway's auth calls on ordinary routes, stay `proc` only (A2 D5).
            (Some(o), None) if j.responder == Some(Responder::Login) => {
                let mut s = self.prefix(o);
                s.extend(self.walk(o, &mut Keys::new(n)));
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
            // X7: the auth node owns its built-in routes; the gateway's own work runs first.
            (Some(gw), Some(o)) if j.responder.is_some() => {
                let mut steps = self.default[gw].steps.clone();
                match steps
                    .iter()
                    .position(|s| matches!(s, Step::Call(c) if c.node == o))
                {
                    // `auth_request` edge: that call runs the route's body, once (P6).
                    Some(i) => {
                        if let Step::Call(c) = &mut steps[i] {
                            c.body = Arc::clone(&body);
                            c.repeat = 1;
                        }
                    }
                    // Local validation: no edge, so no network hop; the body runs inline and
                    // already starts with auth's own `proc` (P7). No call keys can repeat: the
                    // gateway part has no calls without an auth edge (one auth node, X2).
                    None => steps.extend(body.steps.iter().cloned()),
                }
                Arc::new(Seq::new(steps))
            }
            (Some(gw), Some(o)) => self.routed_chain(gw, o, &body, &j.place)?,
            _ => Arc::clone(&body),
        };
        limits(&chain, &j.place)?;
        let ep = Endpoint {
            key: j.key.clone(),
            owner: j.owner,
            entry_node: self.topo.gateway.or(j.owner),
            op,
            chain,
            body,
            error_map: j.error_map.clone(),
            response: j.cfg.and_then(|c| c.response.clone()),
            synthesized: false,
            states: Self::states(j.cfg, &j.place)?,
            responder: j.responder,
            collection: j.cfg.and_then(|c| c.collection.clone()),
            total: j.cfg.and_then(|c| c.total),
            entry: j.cfg.map(|_| (j.place.path.clone(), j.place.file.clone())),
        };
        self.done.insert(j.key.clone(), Arc::new(ep));
        Ok(())
    }

    /// **Private.** D20: every GET without a HEAD gets a `HEAD` copy with its own key.
    pub(super) fn heads(&mut self) {
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

impl Compiler<'_> {
    /// **Private.** The load balancers a request crosses from `from` to reach `to` when `from`
    /// has no wire to it: the shortest chain of wires `from -> lb -> ... -> to` whose middle
    /// nodes are all load balancers. Empty when there is none (the caller then reports the missing
    /// wire as before).
    fn balancers_between(&self, from: NodeId, to: NodeId) -> Vec<NodeId> {
        let lb = |n: NodeId| self.topo.nodes[n].kind == NodeKind::LoadBalancer;
        let mut prev: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(at) = queue.pop_front() {
            for e in self.topo.out_edges(at) {
                if e.to == to {
                    let mut path = Vec::new();
                    let mut cur = at;
                    while cur != from {
                        path.push(cur);
                        cur = prev[&cur];
                    }
                    path.reverse();
                    return path;
                }
                if lb(e.to) && e.to != from && !prev.contains_key(&e.to) {
                    prev.insert(e.to, at);
                    queue.push_back(e.to);
                }
            }
        }
        Vec::new()
    }
}

impl Compiler<'_> {
    /// **Private.** The whole chain of a route owned by `o` behind the gateway `gw`: the gateway's
    /// own work, then the call to the owner, straight over the wire when there is one, else through
    /// the load balancers that reach it.
    fn routed_chain(
        &self,
        gw: NodeId,
        o: NodeId,
        body: &Arc<Seq>,
        place: &Place,
    ) -> Result<Arc<Seq>, Located> {
        let n = self.topo.nodes.len();
        // no wire straight to the owner, but one through load balancers: the request
        // crosses each of them (path-based routing: the route decides the target, so a
        // balancer does not pick) and their cost, queue and bandwidth are paid on the way
        let via = if self.topo.edge(gw, o).is_none() {
            self.balancers_between(gw, o)
        } else {
            Vec::new()
        };
        let mut hops = via.clone();
        hops.push(o);
        let mut inner = Arc::clone(body);
        for w in (1..hops.len()).rev() {
            let (lb, to) = (hops[w - 1], hops[w]);
            let (net, balance, sizes, (retry, orphan, breaker)) = {
                let e = self.edge(lb, to, place)?;
                (
                    e.net.clone(),
                    e.balance.clone(),
                    (e.sends, e.returns, e.stream),
                    (
                        e.retry.clone().map(std::sync::Arc::new),
                        e.abandon == crate::config::Abandon::Continue,
                        e.breaker,
                    ),
                )
            };
            let mut steps = self.prefix(lb);
            let key = Keys::after(&steps, n).next(to);
            steps.push(Step::Call(Call {
                node: to,
                repeat: 1,
                net,
                body: inner,
                op: None,
                prob: 1.0,
                balance,
                sends: sizes.0,
                returns: sizes.1,
                stream: sizes.2,
                retry,
                orphan,
                breaker,
                key,
            }));
            inner = Arc::new(Seq::new(steps));
        }
        let first = hops[0];
        let (net, balance, sizes, (retry, orphan, breaker)) = {
            let e = self.edge(gw, first, place)?;
            (
                e.net.clone(),
                e.balance.clone(),
                (e.sends, e.returns, e.stream),
                (
                    e.retry.clone().map(std::sync::Arc::new),
                    e.abandon == crate::config::Abandon::Continue,
                    e.breaker,
                ),
            )
        };
        let mut steps = self.default[gw].steps.clone();
        let key = Keys::after(&steps, n).next(first);
        steps.push(Step::Call(Call {
            node: first,
            repeat: 1,
            net,
            body: inner,
            op: None,
            prob: 1.0,
            balance,
            sends: sizes.0,
            returns: sizes.1,
            stream: sizes.2,
            retry,
            orphan,
            breaker,
            key,
        }));
        Ok(Arc::new(Seq::new(steps)))
    }
}
