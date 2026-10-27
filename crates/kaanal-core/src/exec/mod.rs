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

mod walk;

pub(crate) use walk::draw_rng;
use walk::{Flow, Walk};

use std::ops::Range;
use std::time::Duration;

use rand::Rng;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::chain::Endpoint;
use crate::chaos::{EffectiveChaos, ErrorKind, FailKind, HANG_CAP};

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
    /// A payload crossing a bandwidth-limited edge (`resp`: the response direction). It belongs
    /// to the edge's target and queues FIFO behind other transfers on the same edge.
    Link {
        /// The edge crossed.
        edge: crate::topology::EdgeId,
        /// False for the request body, true for the response body.
        resp: bool,
    },
}

/// One timed step of a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    /// The node the cost belongs to (for `Net` hops, the edge's target).
    pub node: NodeId,
    /// Which replica of the node ran the hop (0 for a node with one replica).
    pub replica: u16,
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
        /// Zero-load instant of the failure: the failing hop's start (E14). For a `hang` it is the end\n        /// of the capped hang hop (the failure comes after the hop has run its whole length).
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
    /// A pin's delay: a pre-chain `Delay` hop outside every `timeout:` span, after which the
    /// chain starts. Zero (always, in the simulator) plans exactly as without a pin.
    pub pin_delay: Duration,
    /// Bytes of the request body, for link and `per_kb` costs. 0 (always, unless a driver sets
    /// it) plans exactly as before payload costs existed.
    pub req_bytes: u64,
    /// Bytes of the response body, likewise.
    pub resp_bytes: u64,
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
pub fn execute(topo: &Topology, chaos: &EffectiveChaos, req: &ChainRequest) -> ChainOutcome {
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

/// The sampling half of [`execute`]: the [`Plan`] and the *planned* failure (plan index,
/// instant; a deadline from a `timeout:` span is not planned, it fires at run time), without building the client-visible hops. The simulator replays the plan
/// itself, so it skips that work; [`execute`] returns this same plan.
pub fn sample_plan(
    topo: &Topology,
    chaos: &EffectiveChaos,
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
        n: req.n,
        picks: vec![0; topo.nodes.len()],
        bytes: (req.req_bytes, req.resp_bytes),
        items: Vec::new(),
    };
    let entry = ep.entry_node;
    // The entry node has no calling edge, so its own balance picks its replica.
    let entry_pick = entry.map_or(Some(0), |e| w.pick(e, None, root));
    let er = entry_pick.unwrap_or(0);
    // A pin's delay runs before the root span opens, so no `timeout:` counts it.
    let base = req.pin_delay;
    debug_assert!(
        base.is_zero() || entry.is_some(),
        "a pin delay needs an entry node (resolve_scenario rejects pins without one)"
    );
    if let Some(n) = entry.filter(|_| !base.is_zero()) {
        w.items.push(PlanItem::Hop(Hop {
            node: n,
            replica: er,
            kind: HopKind::Delay,
            start: Duration::ZERO,
            dur: base,
        }));
    }
    let root_span = entry.and_then(|n| w.open_span(n, base));
    let mut start = base;
    let mut flow = None;
    if entry_pick.is_none() {
        flow = entry.map(|e| w.no_replica(e, base));
    }
    if let (Some(entry), Some(c), true) = (entry, chaos.endpoint(&ep.key), flow.is_none()) {
        let name = &topo.nodes[entry].name;
        let roll: f64 = draw_rng(root, name, b"endpoint").random();
        if let Some((k, s)) = c.hit(roll) {
            let (kind, status) = fail_of(k, s);
            let mut at = base;
            if k == ErrorKind::Hang {
                let cap = hang_cap(topo.nodes[entry].timeout);
                at = base + cap;
                w.items.push(PlanItem::Hop(Hop {
                    node: entry,
                    replica: er,
                    kind: HopKind::Delay,
                    start: base,
                    dur: cap,
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
                replica: er,
                kind: HopKind::Delay,
                start: base,
                dur: c.latency_add,
            }));
            start = base + c.latency_add;
        }
    }
    let flow = flow.unwrap_or_else(|| {
        w.seq(
            &ep.chain,
            root,
            ep.op,
            start,
            (entry.unwrap_or_default(), er),
        )
    });
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
