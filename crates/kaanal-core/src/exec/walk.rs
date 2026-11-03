//! The walk behind [`super::sample_plan`]: one request's chain as a timeline of [`PlanItem`]s,
//! with the per-hop draws, spans, forks and failures. Moved out of `mod.rs` unchanged (R3.8).

use std::time::Duration;

use rand::{Rng, SeedableRng};

use super::link::MAX_HOP;
use super::{Hop, HopKind, Op, PlanItem, fail_of, hang_cap};
use crate::balance::{Balance, pick_replica};
use crate::chain::{Seq, Step};
use crate::chaos::{EffectiveChaos, ErrorKind, FailKind, Resolved};
use crate::config::Hold;
use crate::dist::Dist;
use crate::rng::HopRng;
use crate::stable_hash;
use crate::topology::{NodeId, Topology};

/// **Private.** The RNG of one hop's draws (R1.34, D8).
pub(crate) fn draw_rng(path: u64, node: &str, tag: &[u8]) -> HopRng {
    HopRng::seed_from_u64(stable_hash(&[&path.to_le_bytes(), node.as_bytes(), tag]))
}

/// **Private.** The path of one `call`/`cache` step repetition (D8).
fn step_path(parent: u64, target: &str, occ: u32, rep: u32) -> u64 {
    stable_hash(&[
        &parent.to_le_bytes(),
        target.as_bytes(),
        &occ.to_le_bytes(),
        &rep.to_le_bytes(),
    ])
}

/// **Private.** Whether a request takes a `call` step with branch probability `prob`: always
/// when `prob` is 1 (no draw, so the plan is as before `prob` existed), else a Bernoulli draw
/// from the step's own stream under the new tag `prob`, so no existing draw moves.
fn takes_branch(prob: f64, path: u64, target: &str, occ: u32) -> bool {
    if prob >= 1.0 {
        return true;
    }
    let roll: f64 = draw_rng(step_path(path, target, occ, 0), target, b"prob").random();
    roll < prob
}

/// **Private.** How a sequence ended.
pub(super) enum Flow {
    /// It ran to its end at this offset.
    Done(Duration),
    /// A cache hit ended it at this offset (only this sequence, section 5.3).
    Hit(Duration),
    /// A step failed: its instant and the plan index of its `Fail` item.
    Fail { at: Duration, idx: usize },
}

/// **Private.** One walk in progress: borrowed inputs plus the plan so far.
pub(super) struct Walk<'a> {
    pub(super) topo: &'a Topology,
    pub(super) chaos: &'a EffectiveChaos,
    /// Key of the endpoint being walked, for endpoint-scoped chaos.
    pub(super) ep_key: &'a str,
    /// The request counter, for round-robin.
    pub(super) n: u64,
    /// Picks so far per node in this request (round-robin advances per pick).
    pub(super) picks: Vec<u32>,
    /// Request and response body sizes, for link hops and `per_kb`.
    pub(super) bytes: (u64, u64),
    pub(super) items: Vec<PlanItem>,
}

impl Walk<'_> {
    /// **Private.** The replica of `node` a request takes now: the pure [`pick_replica`] with the
    /// edge's `balance` over the node's own. A node with one replica is always 0 (no pick, no
    /// counter). Round-robin advances per pick, so two calls to one node in a request take
    /// consecutive replicas. `None` when every replica is down.
    pub(super) fn pick(&mut self, node: NodeId, edge: Option<&Balance>, path: u64) -> Option<u16> {
        let nd = &self.topo.nodes[node];
        if nd.replicas <= 1 {
            return Some(0);
        }
        let k = u64::from(self.picks[node]);
        self.picks[node] += 1;
        let offset = stable_hash(&[self.ep_key.as_bytes(), nd.name.as_bytes()]).wrapping_add(k);
        let balance = edge.unwrap_or(&nd.balance);
        // a down replica is skipped from the request's arrival on (decided here, in the plan)
        let up: Vec<bool> = if self.chaos.is_empty() {
            Vec::new()
        } else {
            (0..nd.replicas)
                .map(|r| !self.chaos.replica_down(node, self.ep_key, r))
                .collect()
        };
        pick_replica(balance, nd.replicas, &up, self.n, offset, path, &nd.name)
    }

    /// **Private.** Every replica of `node` is down: the call fails at `at` with a 503.
    pub(super) fn no_replica(&mut self, node: NodeId, at: Duration) -> Flow {
        let idx = self.items.len();
        self.items.push(PlanItem::Fail {
            node,
            status: 503,
            kind: FailKind::Status,
            at,
        });
        Flow::Fail { at, idx }
    }

    /// **Private.** The replica `node`'s own hop runs on inside a sequence run by `runner` on
    /// `replica`: the runner's own, or, for another node's work inlined here (an auth node
    /// validating locally, with no gateway-to-auth edge), one picked once per sequence for that
    /// node. `None` when every replica of it is down.
    fn own_replica(
        &mut self,
        node: NodeId,
        (runner, replica): (NodeId, u16),
        path: u64,
        inline: &mut Option<(NodeId, u16)>,
    ) -> Option<u16> {
        if node == runner {
            return Some(replica);
        }
        if let Some((n, r)) = *inline
            && n == node
        {
            return Some(r);
        }
        let r = self.pick(node, None, path)?;
        *inline = Some((node, r));
        Some(r)
    }

    /// **Private.** Runs `body` from offset `at` with the inherited `op` (E10). `runner` is the
    /// node running it and `replica` its replica: the runner's own hops land there, a call to
    /// another node picks that node's.
    pub(super) fn seq(
        &mut self,
        body: &Seq,
        path: u64,
        op: Op,
        at: Duration,
        run: (NodeId, u16),
    ) -> Flow {
        // A copy of the reference, so node names can be held across `&mut self` calls.
        let topo = self.topo;
        let chaos = self.chaos;
        let mut now = at;
        let mut inline = None;
        for step in &body.steps {
            match step {
                Step::Proc { node } => {
                    let Some(r) = self.own_replica(*node, run, path, &mut inline) else {
                        return self.no_replica(*node, now);
                    };
                    match self.own(*node, HopKind::Proc, path, b"proc", now, r) {
                        Ok(dur) => now += dur,
                        Err(fail) => return fail,
                    }
                }
                Step::Op { node, op: own } => {
                    let Some(r) = self.own_replica(*node, run, path, &mut inline) else {
                        return self.no_replica(*node, now);
                    };
                    let kind = HopKind::Op(own.unwrap_or(op));
                    match self.own(*node, kind, path, b"op", now, r) {
                        Ok(dur) => now += dur,
                        Err(fail) => return fail,
                    }
                }
                Step::Call(call) => {
                    let target = topo.nodes[call.node].name.as_str();
                    // One draw per step: a skipped call has no hop, span or repeat.
                    if !takes_branch(call.prob, path, target, call.key.occ) {
                        continue;
                    }
                    let op = call.op.unwrap_or(op);
                    for rep in 0..call.repeat {
                        let rep_path = step_path(path, target, call.key.occ, rep);
                        let Some(r) = self.pick(call.node, call.balance.as_ref(), rep_path) else {
                            return self.no_replica(call.node, now);
                        };
                        let span = self.open_span(call.node, now);
                        let edge = topo.edge_id(run.0, call.node);
                        let lc = edge.and_then(|e| chaos.link(e));
                        let cap = lc.and_then(|l| l.bandwidth_cap);
                        if let Some(k) = lc.and_then(|l| self.link_drop(l, call.node, rep_path)) {
                            let fail = self.link_fail(k, call.node, r, now);
                            self.close_span(span);
                            return fail;
                        }
                        now += self.link(edge, call.node, false, (now, r), cap);
                        now += self.net(call.node, &call.net, rep_path, (now, r), lc);
                        let hold = self.open_hold(call.node, r);
                        let flow = self.seq(&call.body, rep_path, op, now, (call.node, r));
                        self.close_hold(hold);
                        if let Flow::Done(end) | Flow::Hit(end) = flow {
                            now = end + self.link(edge, call.node, true, (end, r), cap);
                        }
                        self.close_span(span);
                        match flow {
                            Flow::Done(_) | Flow::Hit(_) => {}
                            fail @ Flow::Fail { .. } => return fail,
                        }
                    }
                }
                Step::Cache(cache) => {
                    // Section 5.2: the default chain's cache steps serve reads and scans only.
                    if cache.reads_only && op == Op::Write {
                        continue;
                    }
                    let target = topo.nodes[cache.node].name.as_str();
                    for rep in 0..cache.repeat {
                        let rep_path = step_path(path, target, cache.key.occ, rep);
                        // A cache never fails the request (Z17): with no live replica the lookup
                        // goes to replica 0, whose chaos is down, so it is a miss like a down cache.
                        let r = self
                            .pick(cache.node, cache.balance.as_ref(), rep_path)
                            .unwrap_or(0);
                        let edge = topo.edge_id(run.0, cache.node);
                        let lc = edge.and_then(|e| chaos.link(e));
                        let cap = lc.and_then(|l| l.bandwidth_cap);
                        // A cache never fails the request (Z17): a dropped link is a miss, a
                        // reset at once and a hang or partition after the node's timeout.
                        if let Some(k) = lc.and_then(|l| self.link_drop(l, cache.node, rep_path)) {
                            now += self.cache_miss_on_drop(k, cache.node, (now, r));
                            continue;
                        }
                        now += self.link(edge, cache.node, false, (now, r), cap);
                        now += self.net(cache.node, &cache.net, rep_path, (now, r), lc);
                        let (hit, dur) = self.lookup(cache.node, rep_path, now, r);
                        now += dur;
                        if hit {
                            now += self.link(edge, cache.node, true, (now, r), cap);
                            return Flow::Hit(now);
                        }
                    }
                }
                Step::Parallel(branches) => match self.fork(branches, path, op, now, run) {
                    Ok(end) => now = end,
                    Err(fail) => return fail,
                },
            }
        }
        Flow::Done(now)
    }

    /// **Private.** Opens a hold for a visit to `node` (replica `replica`) when it has `hold: call`
    /// and slots to hold; returns its plan index. The caller walks the node's body, then closes it.
    pub(super) fn open_hold(&mut self, node: NodeId, replica: u16) -> Option<usize> {
        let n = &self.topo.nodes[node];
        if n.hold != Hold::Call || n.admission.is_none() {
            return None;
        }
        self.items.push(PlanItem::Hold {
            node,
            replica,
            size: 0,
        });
        Some(self.items.len() - 1)
    }

    /// **Private.** Fixes the size of the hold opened at `hold` (after its body is walked, also
    /// when the body failed).
    pub(super) fn close_hold(&mut self, hold: Option<usize>) {
        if let Some(i) = hold {
            let end = self.items.len() - i - 1;
            if let PlanItem::Hold { size, .. } = &mut self.items[i] {
                *size = end;
            }
        }
    }

    /// **Private.** Opens a span for a call to `node` when it has a `timeout`; returns its plan index.
    pub(super) fn open_span(&mut self, node: NodeId, at: Duration) -> Option<usize> {
        let timeout = self.topo.nodes[node].timeout?;
        self.items.push(PlanItem::Span {
            node,
            timeout,
            at,
            size: 0,
        });
        Some(self.items.len() - 1)
    }

    /// **Private.** Fixes the size of the span opened at `span` (call after its body is walked,
    /// also when the body failed).
    pub(super) fn close_span(&mut self, span: Option<usize>) {
        if let Some(i) = span {
            let size = self.items.len() - i - 1;
            if let PlanItem::Span { size: s, .. } = &mut self.items[i] {
                *s = size;
            }
        }
    }

    /// **Private.** An edge's latency as a `Net` hop, with the link's chaos (extra latency, jitter) on it.
    fn net(
        &mut self,
        to: NodeId,
        d: &Dist,
        path: u64,
        (start, replica): (Duration, u16),
        lc: Option<&Resolved>,
    ) -> Duration {
        let dur = d.sample(&mut draw_rng(path, &self.topo.nodes[to].name, b"net"));
        let dur = self.shake(dur, lc, path, to);
        self.items.push(PlanItem::Hop(Hop {
            node: to,
            replica,
            kind: HopKind::Net,
            start,
            dur,
        }));
        dur
    }

    /// **Private.** A node's own hop (proc or op): chaos roll, then the duration. A failing
    /// node fails at its hop start (E14) and records a `Fail` instead of a hop; a hang records a
    /// hop of the node's `timeout` (else [`HANG_CAP`]) and then a 504 `Fail` at its end (Z6).
    fn own(
        &mut self,
        node: NodeId,
        kind: HopKind,
        path: u64,
        tag: &[u8],
        start: Duration,
        replica: u16,
    ) -> Result<Duration, Flow> {
        let topo = self.topo;
        let n = &topo.nodes[node];
        let mut rng = draw_rng(path, &n.name, tag);
        let roll: f64 = rng.random();
        let c = (!self.chaos.is_empty())
            .then(|| self.chaos.visit_replica(node, self.ep_key, replica))
            .flatten();
        let hit = c.and_then(|c| c.hit(roll));
        if let Some((k, s)) = hit {
            let (fk, status) = fail_of(k, s);
            let mut at = start;
            if k == ErrorKind::Hang {
                // A hang holds the node for its whole cap, then times out (Z6).
                let cap = hang_cap(n.timeout);
                self.items.push(PlanItem::Hop(Hop {
                    node,
                    replica,
                    kind,
                    start,
                    dur: cap,
                }));
                at += cap;
            }
            let idx = self.items.len();
            self.items.push(PlanItem::Fail {
                node,
                status,
                kind: fk,
                at,
            });
            return Err(Flow::Fail { at, idx });
        }
        let dist = match kind {
            HopKind::Op(op) => n.ops.as_ref().map_or(&Dist::ZERO, |o| &o[op as usize]),
            _ => &n.proc,
        };
        let mut dur = dist.sample(&mut rng) + c.map_or(Duration::ZERO, |c| c.latency_add);
        let body = self.bytes.0.saturating_add(self.bytes.1);
        if matches!(kind, HopKind::Proc | HopKind::Op(_)) && body > 0 && n.per_kb != Dist::ZERO {
            // a draw of its own, so no existing draw moves
            let per_kb = n.per_kb.sample(&mut draw_rng(path, &n.name, b"perkb"));
            #[expect(clippy::cast_precision_loss, reason = "a size, not an exact count")]
            let kb = body as f64 / 1024.0;
            let secs = per_kb.as_secs_f64() * kb;
            dur += Duration::from_secs_f64(secs.min(MAX_HOP.as_secs_f64()));
        }
        self.items.push(PlanItem::Hop(Hop {
            node,
            replica,
            kind,
            start,
            dur,
        }));
        Ok(dur)
    }

    /// **Private.** A cache lookup (E9, D12): chaos roll and hit roll are always drawn; a
    /// down or failing cache is a miss, charged its `miss` cost plus `latency_add`. The node's
    /// `timeout` caps the charge (a slower lookup is a miss costing exactly the timeout); a hang
    /// is a miss charged the timeout, else [`HANG_CAP`] (Z17).
    fn lookup(
        &mut self,
        node: NodeId,
        path: u64,
        start: Duration,
        replica: u16,
    ) -> (bool, Duration) {
        let topo = self.topo;
        let n = &topo.nodes[node];
        let mut rng = draw_rng(path, &n.name, b"cache");
        let (roll, hit_roll): (f64, f64) = (rng.random(), rng.random());
        let c = (!self.chaos.is_empty())
            .then(|| self.chaos.visit_replica(node, self.ep_key, replica))
            .flatten();
        let fault = c.and_then(|c| c.hit(roll));
        let failing = fault.is_some();
        let model = n.cache.as_ref();
        // A hanging cache is a miss charged its cap; a cache never fails the request (Z17).
        let hang = fault.is_some_and(|(k, _)| k == ErrorKind::Hang);
        let mut hit = !failing && model.is_some_and(|m| hit_roll < m.hit_ratio);
        let mut dur = if hang {
            hang_cap(n.timeout)
        } else {
            let dist = model.map_or(&Dist::ZERO, |m| if hit { &m.hit } else { &m.miss });
            dist.sample(&mut rng) + c.map_or(Duration::ZERO, |c| c.latency_add)
        };
        if let Some(t) = n.timeout
            && dur > t
        {
            hit = false;
            dur = t;
        }
        let kind = if hit {
            HopKind::CacheHit
        } else {
            HopKind::CacheMiss
        };
        self.items.push(PlanItem::Hop(Hop {
            node,
            replica,
            kind,
            start,
            dur,
        }));
        (hit, dur)
    }

    /// **Private.** E14: branches start together at `t`. The block ends with its slowest
    /// branch, or fails at the earliest failure; equal instants keep the lower branch (and
    /// so the lower plan index). Branches are transparent for the path (D8): their steps'
    /// keys already count occurrences across the block.
    fn fork(
        &mut self,
        branches: &[Seq],
        path: u64,
        op: Op,
        t: Duration,
        run: (NodeId, u16),
    ) -> Result<Duration, Flow> {
        let at = self.items.len();
        self.items.push(PlanItem::Fork {
            branches: branches.len(),
            size: 0,
        });
        let (mut end, mut first): (Duration, Option<Flow>) = (t, None);
        for b in branches {
            let bi = self.items.len();
            self.items.push(PlanItem::Branch { size: 0 });
            let flow = self.seq(b, path, op, t, run);
            self.items[bi] = PlanItem::Branch {
                size: self.items.len() - bi - 1,
            };
            match flow {
                Flow::Done(e) | Flow::Hit(e) => end = end.max(e),
                Flow::Fail { at: f, .. } => {
                    let earlier = match &first {
                        Some(Flow::Fail { at: g, .. }) => f < *g,
                        _ => true,
                    };
                    if earlier {
                        first = Some(flow);
                    }
                }
            }
        }
        self.items[at] = PlanItem::Fork {
            branches: branches.len(),
            size: self.items.len() - at - 1,
        };
        first.map_or(Ok(end), Err)
    }
}
