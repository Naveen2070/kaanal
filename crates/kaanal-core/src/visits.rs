//! Expected visits per node: how often a request is expected to enter each node.
//!
//! Computed from the compiled [`Seq`] trees, not from the edge graph, because chains decide the
//! traffic (`with:`/`via:` reuse bodies, a step may repeat or skip). A visit is one entry to a
//! node: the entry node, a `call` into it, or a cache lookup. Values above 1 are normal.
//! Zero-chaos expectation: failures and chaos are ignored.

use std::sync::Arc;

use crate::chain::{Endpoint, Seq, Step};
use crate::exec::Op;
use crate::topology::{NodeId, Topology};

/// Expected entries per request to `ep`, by [`crate::NodeId`].
///
/// The entry node counts 1. A `call` adds `m * prob * repeat` to its callee and runs its body with
/// that multiplier. A cache step adds `m * (1 + (1-h) + ... + (1-h)^(repeat-1))` (a lookup runs only
/// if every earlier one missed) and scales the rest of that sequence by `(1-h)^repeat`: a hit ends
/// only its own sequence, so a `parallel` branch's hit leaves its siblings and what follows alone.
/// A default-chain cache lookup skips writes, like the executor. Work of a node other than the
/// sequence's runner (an auth node validating locally) is one inline visit. Not modelled: chaos,
/// and a cache `timeout` shorter than its `hit` cost (the executor turns those hits into misses;
/// it depends on the sampled cost).
pub fn expected_visits(topo: &Topology, ep: &Endpoint) -> Vec<f64> {
    let mut v = vec![0.0; topo.nodes.len()];
    if let Some(runner) = ep.entry_node {
        v[runner] += 1.0;
        walk(topo, &ep.chain, runner, ep.op, 1.0, &mut v);
    }
    v
}

/// Expected entries per second to each node for a traffic `mix` of `(endpoint, requests per
/// second)`: the sum of each endpoint's [`expected_visits`] times its rate.
pub fn visits_per_s(topo: &Topology, mix: &[(Arc<Endpoint>, f64)]) -> Vec<f64> {
    let mut total = vec![0.0; topo.nodes.len()];
    for (ep, rate) in mix {
        for (t, x) in total.iter_mut().zip(expected_visits(topo, ep)) {
            *t += rate * x;
        }
    }
    total
}

/// **Private.** Adds the visits of running `seq` with multiplier `m` and inherited `op`.
fn walk(topo: &Topology, seq: &Seq, runner: NodeId, op: Op, mut m: f64, v: &mut [f64]) {
    for step in &seq.steps {
        match step {
            // Own work of the runner was counted at the call that entered it. Work of another
            // node in this sequence is inline (an auth node validating locally): one visit.
            Step::Proc { node } | Step::Op { node, .. } => {
                if *node != runner {
                    v[*node] += m;
                }
            }
            Step::Call(c) => {
                let k = m * c.prob * f64::from(c.repeat);
                v[c.node] += k;
                walk(topo, &c.body, c.node, c.op.unwrap_or(op), k, v);
            }
            Step::Cache(c) => {
                if c.reads_only && op == Op::Write {
                    continue;
                }
                let h = topo.nodes[c.node]
                    .cache
                    .as_ref()
                    .map_or(0.0, |x| x.hit_ratio);
                let miss = 1.0 - h;
                let lookups = (0..c.repeat).fold((0.0, 1.0), |(sum, p), _| (sum + p, p * miss));
                v[c.node] += m * lookups.0;
                m *= lookups.1;
            }
            Step::Route(r) => {
                // one callee takes each call: its share of the traffic, by the spread
                for (c, share) in r.calls.iter().zip(r.shares()) {
                    let k = m * share * f64::from(c.repeat);
                    v[c.node] += k;
                    walk(topo, &c.body, c.node, c.op.unwrap_or(op), k, v);
                }
            }
            Step::Parallel(branches) => {
                for b in branches {
                    walk(topo, b, runner, op, m, v);
                }
            }
        }
    }
}
