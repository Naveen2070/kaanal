//! The chain executor: computes what a request would experience.
//!
//! [`execute`] walks an endpoint's compiled chain ([`crate::chain`]) as a timeline:
//! every hop gets a zero-load `start` and a sampled `dur`, a `parallel` block starts its
//! branches together and ends with the slowest, and the first failure (E14: at its hop
//! start, ties to the lowest branch) cancels its siblings. The sampled tree is returned as a
//! [`Plan`], the one replay format the simulator walks (and sub-project B will). Every
//! draw has its own RNG, seeded from the request, the step's place in the tree and the node
//! (R1.34), so editing one step never changes another step's draws. Pure: no sleeping, no
//! IO; live mode sleeps [`ChainOutcome::total`], the simulator advances virtual time (R2.5).

use std::ops::Range;
use std::time::Duration;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::chain::{Endpoint, Seq, Step};
use crate::chaos::{ChaosState, ErrorKind, FailKind, HANG_CAP, merge};
use crate::dist::Dist;
use crate::stable_hash;
use crate::topology::{NodeId, Topology};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
/// A database operation. Discriminants index `Node::ops` ([read, write, scan]);
/// keep in sync with the topology.
pub enum Op {
    /// Read one row (`GET /users/{id}`).
    Read = 0,
    /// Insert, update or delete.
    Write = 1,
    /// List or search (`GET /users`).
    Scan = 2,
}

impl Op {
    /// GET with a path parameter reads one row; GET without lists; everything else writes.
    pub fn infer(method: &str, path_template: &str) -> Op {
        match method.to_ascii_uppercase().as_str() {
            "GET" | "HEAD" if path_template.contains('{') => Op::Read,
            "GET" | "HEAD" => Op::Scan,
            _ => Op::Write,
        }
    }

    /// Lowercase name used in the `X-Kaanal-Trace` header (`db.read=4.2ms`).
    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Scan => "scan",
        }
    }
}

/// What kind of cost a hop represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopKind {
    /// Network latency on an edge.
    Net,
    /// Processing time on a gateway, service, monolith, external API or auth node.
    Proc,
    /// A database operation, or the inline op of a monolith.
    Op(Op),
    /// A cache lookup that hit: it ends the enclosing sequence.
    CacheHit,
    /// A cache lookup that missed (or a cache that is down or failing, E9).
    CacheMiss,
    /// A fault-injection delay before the entry node: holds no slot and no rate-limit token.
    Delay,
}

/// One timed step of a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    /// The node the cost belongs to (for `Net` hops, the edge's target).
    pub node: NodeId,
    /// Net, proc, op, cache hit or miss, or an endpoint-chaos delay.
    pub kind: HopKind,
    /// Offset from the request's start at zero load.
    pub start: Duration,
    /// Sampled duration (cut at the failure instant in [`ChainOutcome::hops`] when the
    /// hop's branch was cancelled).
    pub dur: Duration,
}

/// One entry of a [`Plan`], in depth-first (pre-)order.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanItem {
    /// A hop with its full sampled duration.
    Hop(Hop),
    /// The step that failed: its sequence stops here.
    Fail {
        /// The failing node.
        node: NodeId,
        /// The status it answers with.
        status: u16,
        /// How the failure looked to the caller.
        kind: FailKind,
        /// Zero-load instant of the failure: the failing hop's start (E14).
        at: Duration,
    },
    /// A `parallel` block: `branches` [`PlanItem::Branch`] items follow, `size` items in all.
    Fork {
        /// Number of branches.
        branches: usize,
        /// Items after this one that belong to the block (branch markers included).
        size: usize,
    },
    /// One branch of the enclosing fork: its `size` items follow.
    Branch {
        /// Items after this one that belong to the branch.
        size: usize,
    },
    /// A deadline scope (pre-order): the `size` items after it are the call (edge hop and body)
    /// that must end within `timeout` of `at`.
    Span {
        /// The node whose `timeout` this is.
        node: NodeId,
        /// The deadline, counted from `at`.
        timeout: Duration,
        /// Zero-load instant the call started.
        at: Duration,
        /// Items after this one that belong to the span.
        size: usize,
    },
}

/// The sampled tree of one request: what ran, with zero-load times, fixed at arrival.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Items in depth-first order.
    pub items: Vec<PlanItem>,
}

/// How a chain walk ended.
#[derive(Debug, Clone, PartialEq)]
pub enum ChainResult {
    /// Every hop succeeded.
    Ok,
    /// The walk stopped at a node whose chaos failed it.
    Err {
        /// The failing node.
        node: NodeId,
        /// The HTTP status it answered with.
        status: u16,
        /// How the failure looked to the caller.
        kind: FailKind,
        /// For a 429 from a rate limit: how long until a token is available. Always `None` from `execute`.
        retry_after: Option<Duration>,
    },
}

/// The full result of one [`execute`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChainOutcome {
    /// What the client saw, in plan order: every hop except those a failure cancelled
    /// before they started; a cancelled hop that was running is cut at the failure instant.
    pub hops: Vec<Hop>,
    /// `kept[i]` is the plan index of `hops[i]`.
    pub kept: Vec<usize>,
    /// Everything sampled, for replay (the simulator).
    pub plan: Plan,
    /// Ok, or where and how it failed.
    pub result: ChainResult,
    /// Plan index of the item that decided `result`: a `Fail`, or a `Span` whose deadline passed.
    pub fail: Option<usize>,
}

impl ChainOutcome {
    /// What the client waits: the end of the latest hop that counted. On a failure this is
    /// the failure instant. A planned failure starts where an earlier hop ended, or at 0; a span
    /// deadline lands mid-hop, and [`ChainOutcome::hops`] is clipped there.
    pub fn total(&self) -> Duration {
        self.hops
            .iter()
            .map(|h| h.start + h.dur)
            .max()
            .unwrap_or(Duration::ZERO)
    }
}

/// The input to one [`execute`] call.
pub struct ChainRequest<'a> {
    /// The compiled endpoint: key (seed material), op and chain.
    pub endpoint: &'a Endpoint,
    /// This endpoint's request counter: part of the seed, so sequential requests vary.
    pub n: u64,
    /// The chain seed: `seed:` (controlled, and the simulator always) or the startup draw
    /// (realistic). See [`crate::config::SeedSource`].
    pub seed: u64,
}

impl Plan {
    /// Plan ranges a failure at item `fail` cancels: at every fork enclosing it, every
    /// branch except the one holding it (live mode and the simulator share this rule).
    pub fn cancelled_by(&self, fail: usize) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < fail {
            match self.items[i] {
                PlanItem::Fork { size, .. } if fail <= i + size => {
                    let (mut b, end, mut inside) = (i + 1, i + 1 + size, None);
                    while b < end {
                        let PlanItem::Branch { size: bs } = self.items[b] else {
                            break;
                        };
                        let r = b + 1..b + 1 + bs;
                        if r.contains(&fail) {
                            inside = Some(r.start);
                        } else {
                            out.push(r.clone());
                        }
                        b = r.end;
                    }
                    let Some(next) = inside else { break };
                    i = next;
                }
                PlanItem::Fork { size, .. } => i += 1 + size,
                _ => i += 1,
            }
        }
        out
    }

    /// **Private.** The client-visible hops (see [`ChainOutcome::hops`]) and their plan indices.
    ///
    /// `cut_all`: the deciding failure is a span deadline, so every hop is clipped or dropped
    /// at that instant (not just sibling branches).
    fn visible(&self, fail: Option<(usize, Duration)>, cut_all: bool) -> (Vec<Hop>, Vec<usize>) {
        let cut = match fail {
            Some((i, _)) if !cut_all => self.cancelled_by(i),
            _ => Vec::new(),
        };
        let at = fail.map_or(Duration::MAX, |(_, t)| t);
        let (mut hops, mut kept) = (Vec::new(), Vec::new());
        for (i, item) in self.items.iter().enumerate() {
            let PlanItem::Hop(h) = item else { continue };
            let mut h = h.clone();
            // ponytail: linear scan of the cut ranges per hop; ranges are at most the branch count.
            if cut_all || cut.iter().any(|r| r.contains(&i)) {
                if h.start >= at {
                    continue;
                }
                h.dur = h.dur.min(at.saturating_sub(h.start));
            }
            hops.push(h);
            kept.push(i);
        }
        (hops, kept)
    }
}

/// Computes what the request would experience.
///
/// The same inputs always give the same outcome (R4.1). Each hop draws from its own RNG
/// seeded `stable_hash(path, node name, tag)` (tags `net`, `proc`, `op`, `cache`, and `endpoint` for front-door chaos), where
/// `path` starts as `stable_hash(seed, endpoint key, n)` and every `call`/`cache` step
/// extends it with `(target name, occurrence, repetition)` (R1.34, D8). Own hops draw the
/// chaos roll first and caches also their hit roll, always, so chaos never shifts a timing.
pub fn execute(topo: &Topology, chaos: &ChaosState, req: &ChainRequest) -> ChainOutcome {
    let (plan, planned) = sample_plan(topo, chaos, req);
    let decided = deadline_failure(&plan, planned).or(planned);
    let result = match decided.map(|(i, _)| &plan.items[i]) {
        Some(&PlanItem::Fail {
            node, status, kind, ..
        }) => ChainResult::Err {
            node,
            status,
            kind,
            retry_after: None,
        },
        Some(&PlanItem::Span { node, .. }) => ChainResult::Err {
            node,
            status: 504,
            kind: FailKind::Timeout,
            retry_after: None,
        },
        _ => ChainResult::Ok,
    };
    // `deadline_failure` only returns a span, so a decision that differs from the planned one is one.
    let cut_all = decided != planned;
    let (hops, kept) = plan.visible(decided, cut_all);
    ChainOutcome {
        hops,
        kept,
        plan,
        result,
        fail: decided.map(|(i, _)| i),
    }
}

/// **Private.** Zero-load deadline check (Z13): the earliest span whose body ends after its
/// deadline, compared with the planned failure; a tie goes to the planned failure.
fn deadline_failure(plan: &Plan, planned: Option<(usize, Duration)>) -> Option<(usize, Duration)> {
    let mut best = planned.map(|(_, t)| t);
    let mut out = None;
    for (i, item) in plan.items.iter().enumerate() {
        let PlanItem::Span {
            timeout, at, size, ..
        } = item
        else {
            continue;
        };
        let end = plan.items[i + 1..=i + size]
            .iter()
            .filter_map(|x| match x {
                PlanItem::Hop(h) => Some(h.start + h.dur),
                _ => None,
            })
            .max()
            .unwrap_or(*at);
        let due = *at + *timeout;
        if end > due && best.is_none_or(|b| due < b) {
            best = Some(due);
            out = Some((i, due));
        }
    }
    out
}

/// The sampling half of [`execute`]: the [`Plan`] and the deciding failure (plan index,
/// instant), without building the client-visible hops. The simulator replays the plan
/// itself, so it skips that work; [`execute`] returns this same plan.
pub fn sample_plan(
    topo: &Topology,
    chaos: &ChaosState,
    req: &ChainRequest,
) -> (Plan, Option<(usize, Duration)>) {
    let ep = req.endpoint;
    let root = stable_hash(&[
        &req.seed.to_le_bytes(),
        ep.key.as_bytes(),
        &req.n.to_le_bytes(),
    ]);
    let mut w = Walk {
        topo,
        chaos,
        ep_key: ep.key.as_str(),
        items: Vec::new(),
    };
    let entry = ep.entry_node;
    let root_span = entry.and_then(|n| w.open_span(n, Duration::ZERO));
    let mut start = Duration::ZERO;
    let mut flow = None;
    if let (Some(entry), Some(layer)) = (entry, chaos.endpoint(&ep.key)) {
        let c = merge(&[layer]);
        let name = &topo.nodes[entry].name;
        let roll: f64 = draw_rng(root, name, b"endpoint").random();
        if c.down || roll < c.error_rate {
            let (k, s) = if c.down {
                (ErrorKind::Status, 503)
            } else {
                (c.error_kind, c.error_status)
            };
            let (kind, status) = fail_of(k, s);
            let mut at = Duration::ZERO;
            if k == ErrorKind::Hang {
                at = hang_cap(topo.nodes[entry].timeout);
                w.items.push(PlanItem::Hop(Hop {
                    node: entry,
                    kind: HopKind::Delay,
                    start: Duration::ZERO,
                    dur: at,
                }));
            }
            let idx = w.items.len();
            w.items.push(PlanItem::Fail {
                node: entry,
                status,
                kind,
                at,
            });
            flow = Some(Flow::Fail { at, idx });
        } else if !c.latency_add.is_zero() {
            w.items.push(PlanItem::Hop(Hop {
                node: entry,
                kind: HopKind::Delay,
                start: Duration::ZERO,
                dur: c.latency_add,
            }));
            start = c.latency_add;
        }
    }
    let flow = flow.unwrap_or_else(|| w.seq(&ep.chain, root, ep.op, start));
    w.close_span(root_span);
    let fail = match flow {
        Flow::Fail { at, idx } => Some((idx, at)),
        Flow::Done(_) | Flow::Hit(_) => None,
    };
    (Plan { items: w.items }, fail)
}

/// **Private.** How a chaos error kind (with its configured status) looks to the caller;
/// a hang ends as a 504 timeout (Z5, Z6).
fn fail_of(kind: ErrorKind, status: u16) -> (FailKind, u16) {
    match kind {
        ErrorKind::Status => (FailKind::Status, status),
        ErrorKind::Reset => (FailKind::Reset, 502),
        ErrorKind::Malformed => (FailKind::Malformed, 502),
        ErrorKind::Hang => (FailKind::Timeout, 504),
    }
}

/// **Private.** How long a hang lasts: the node's `timeout`, else [`HANG_CAP`].
fn hang_cap(timeout: Option<Duration>) -> Duration {
    timeout.unwrap_or(HANG_CAP)
}

/// **Private.** The RNG of one hop's draws (R1.34, D8).
fn draw_rng(path: u64, node: &str, tag: &[u8]) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(stable_hash(&[&path.to_le_bytes(), node.as_bytes(), tag]))
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

/// **Private.** How a sequence ended.
enum Flow {
    /// It ran to its end at this offset.
    Done(Duration),
    /// A cache hit ended it at this offset (only this sequence, §5.3).
    Hit(Duration),
    /// A step failed: its instant and the plan index of its `Fail` item.
    Fail { at: Duration, idx: usize },
}

/// **Private.** One walk in progress: borrowed inputs plus the plan so far.
struct Walk<'a> {
    topo: &'a Topology,
    chaos: &'a ChaosState,
    /// Key of the endpoint being walked, for endpoint-scoped chaos.
    ep_key: &'a str,
    items: Vec<PlanItem>,
}

impl Walk<'_> {
    /// **Private.** Runs `body` from offset `at` with the inherited `op` (E10).
    fn seq(&mut self, body: &Seq, path: u64, op: Op, at: Duration) -> Flow {
        // A copy of the reference, so node names can be held across `&mut self` calls.
        let topo = self.topo;
        let mut now = at;
        for step in &body.steps {
            match step {
                Step::Proc { node } => match self.own(*node, HopKind::Proc, path, b"proc", now) {
                    Ok(dur) => now += dur,
                    Err(fail) => return fail,
                },
                Step::Op { node, op: own } => {
                    let kind = HopKind::Op(own.unwrap_or(op));
                    match self.own(*node, kind, path, b"op", now) {
                        Ok(dur) => now += dur,
                        Err(fail) => return fail,
                    }
                }
                Step::Call(call) => {
                    let target = topo.nodes[call.node].name.as_str();
                    let op = call.op.unwrap_or(op);
                    for rep in 0..call.repeat {
                        let rep_path = step_path(path, target, call.key.occ, rep);
                        let span = self.open_span(call.node, now);
                        now += self.net(call.node, &call.net, rep_path, now);
                        let flow = self.seq(&call.body, rep_path, op, now);
                        self.close_span(span);
                        match flow {
                            Flow::Done(end) | Flow::Hit(end) => now = end,
                            fail @ Flow::Fail { .. } => return fail,
                        }
                    }
                }
                Step::Cache(cache) => {
                    // §5.2: the default chain's cache steps serve reads and scans only.
                    if cache.reads_only && op == Op::Write {
                        continue;
                    }
                    let target = topo.nodes[cache.node].name.as_str();
                    for rep in 0..cache.repeat {
                        let rep_path = step_path(path, target, cache.key.occ, rep);
                        now += self.net(cache.node, &cache.net, rep_path, now);
                        let (hit, dur) = self.lookup(cache.node, rep_path, now);
                        now += dur;
                        if hit {
                            return Flow::Hit(now);
                        }
                    }
                }
                Step::Parallel(branches) => match self.fork(branches, path, op, now) {
                    Ok(end) => now = end,
                    Err(fail) => return fail,
                },
            }
        }
        Flow::Done(now)
    }

    /// **Private.** Opens a span for a call to `node` when it has a `timeout`; returns its plan index.
    fn open_span(&mut self, node: NodeId, at: Duration) -> Option<usize> {
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
    fn close_span(&mut self, span: Option<usize>) {
        if let Some(i) = span {
            let size = self.items.len() - i - 1;
            if let PlanItem::Span { size: s, .. } = &mut self.items[i] {
                *s = size;
            }
        }
    }

    /// **Private.** An edge's latency as a `Net` hop (no chaos on edges, as in A1).
    fn net(&mut self, to: NodeId, d: &Dist, path: u64, start: Duration) -> Duration {
        let dur = d.sample(&mut draw_rng(path, &self.topo.nodes[to].name, b"net"));
        self.items.push(PlanItem::Hop(Hop {
            node: to,
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
    ) -> Result<Duration, Flow> {
        let topo = self.topo;
        let n = &topo.nodes[node];
        let mut rng = draw_rng(path, &n.name, tag);
        let roll: f64 = rng.random();
        let c = (!self.chaos.is_empty()).then(|| self.chaos.resolve(node, self.ep_key));
        let c = c.as_ref();
        let hit = c.and_then(|c| {
            if c.down {
                Some((ErrorKind::Status, 503))
            } else if roll < c.error_rate {
                Some((c.error_kind, c.error_status))
            } else {
                None
            }
        });
        if let Some((k, s)) = hit {
            let (fk, status) = fail_of(k, s);
            let mut at = start;
            if k == ErrorKind::Hang {
                // A hang holds the node for its whole cap, then times out (Z6).
                let cap = hang_cap(n.timeout);
                self.items.push(PlanItem::Hop(Hop {
                    node,
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
        let dur = dist.sample(&mut rng) + c.map_or(Duration::ZERO, |c| c.latency_add);
        self.items.push(PlanItem::Hop(Hop {
            node,
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
    fn lookup(&mut self, node: NodeId, path: u64, start: Duration) -> (bool, Duration) {
        let topo = self.topo;
        let n = &topo.nodes[node];
        let mut rng = draw_rng(path, &n.name, b"cache");
        let (roll, hit_roll): (f64, f64) = (rng.random(), rng.random());
        let c = (!self.chaos.is_empty()).then(|| self.chaos.resolve(node, self.ep_key));
        let c = c.as_ref();
        let failing = c.is_some_and(|c| c.down || roll < c.error_rate);
        let model = n.cache.as_ref();
        // A hanging cache is a miss charged its cap; a cache never fails the request (Z17).
        let hang = failing && c.is_some_and(|c| !c.down && c.error_kind == ErrorKind::Hang);
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
    fn fork(&mut self, branches: &[Seq], path: u64, op: Op, t: Duration) -> Result<Duration, Flow> {
        let at = self.items.len();
        self.items.push(PlanItem::Fork {
            branches: branches.len(),
            size: 0,
        });
        let (mut end, mut first): (Duration, Option<Flow>) = (t, None);
        for b in branches {
            let bi = self.items.len();
            self.items.push(PlanItem::Branch { size: 0 });
            let flow = self.seq(b, path, op, t);
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
