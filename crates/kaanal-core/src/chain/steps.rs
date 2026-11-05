//! `Compiler` methods that compile one step: `call`, `cache`, `parallel`, `repeat`.

use crate::config::prob_ok;

use super::{
    Arc, CacheStep, Call, CallStep, Compiler, Edge, Keys, Located, MAX_BRANCHES, MAX_NESTING,
    MAX_REPEAT, MAX_STEPS, NodeId, NodeKind, Place, Seq, Step, StepConfig, normalize,
};

impl Compiler<'_> {
    pub(super) fn steps(
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

    pub(super) fn step(
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
                let edge = self.edge(runner, to, &at)?;
                let (net, balance) = (edge.net.clone(), edge.balance.clone());
                Ok(Step::Cache(CacheStep {
                    node: to,
                    net,
                    reads_only: false,
                    balance,
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
    pub(super) fn edge(&self, runner: NodeId, to: NodeId, at: &Place) -> Result<&Edge, Located> {
        self.topo.edge(runner, to).ok_or_else(|| {
            at.bad(format!(
                "'{}' has no edge to '{}' (add it under edges:)",
                self.topo.nodes[runner].name, self.topo.nodes[to].name
            ))
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one step's checks in the order the messages come"
    )]
    pub(super) fn call(
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
        let (net, calls, edge_prob, balance, edge_sizes) = (
            edge.net.clone(),
            edge.calls.get(),
            edge.prob,
            edge.balance.clone(),
            (edge.sends, edge.returns, edge.stream),
        );
        let edge_retry = edge.retry.clone();
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
            // a storage node takes the op as GET, PUT, LIST: all three cost the same (RULES 61)
            let has_ops = matches!(kind, NodeKind::Database | NodeKind::Storage)
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
        let prob = step_prob(c.prob, edge_prob, at)?;
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
            prob,
            balance,
            sends: c.sends.or(edge_sizes.0),
            returns: c.returns.or(edge_sizes.1),
            stream: c.stream.or(edge_sizes.2),
            retry: c.retry.clone().or(edge_retry).map(std::sync::Arc::new),
            key: keys.next(to),
        })
    }
}

/// **Private.** A call's `prob`: its own, else the edge's; the step's own must be above 0, at most 1.
fn step_prob(own: Option<f64>, edge: f64, at: &Place) -> Result<f64, Located> {
    match own {
        None => Ok(edge),
        Some(p) if prob_ok(p) => Ok(p),
        Some(p) => Err(at
            .key("prob")
            .bad(format!("prob must be above 0 and at most 1 (got {p})"))),
    }
}
