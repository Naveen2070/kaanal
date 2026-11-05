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

mod conn;
mod link;
mod loss;
#[cfg_attr(not(test), allow(dead_code))] // wired in step 7a task 3
mod retry;
mod route;
mod scope;
mod stream;
mod walk;
mod wire;

pub use retry::Jitter;
pub(crate) use retry::retryable;
pub use scope::ScopedFail;
pub(crate) use walk::draw_rng;
use walk::{Flow, Walk};

use std::ops::Range;
use std::sync::Arc;
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
    /// A new connection's set-up (RULES 68): the TCP and TLS round trips and the slow-start stalls
    /// of the request body (`resp` false, before the callee), or of the response body (`resp`
    /// true, after it). A pure delay like `Net`; it belongs to the edge's target.
    Connect {
        /// False for the handshake and the request's stalls, true for the response's stalls.
        resp: bool,
    },
    /// What loss adds to a transfer (RULES 74): fast retransmits, retransmission timeouts, or for a
    /// long flow the time over the lossless one at the TCP rate. A pure delay after the transfer
    /// (`resp` false: the request, true: the response); it belongs to the callee.
    Retransmit {
        /// False after the request body, true after the response body.
        resp: bool,
    },
    /// The wait for the first byte of a streamed response (RULES 71): the callee is producing and
    /// the transfer has not started. A pure delay in the transfer's branch of a fork; it belongs to
    /// the callee.
    FirstByte,
    /// A payload crossing a bandwidth-limited edge (`resp`: the response direction). It belongs
    /// to the edge's target and shares the edge's rate with other transfers on it.
    Link {
        /// The edge crossed.
        edge: u16,
        /// False for the request body, true for the response body.
        resp: bool,
    },
    /// A payload held up by a node's network card when that is the narrowest pipe on its path
    /// (`bandwidth:` on a node). The hop belongs to the node and replica whose pipe it is, and
    /// shares that pipe's rate with other transfers on it.
    Nic {
        /// True for the node's send pipe, false for its receive pipe.
        send: bool,
        /// False for the request body, true for the response body.
        resp: bool,
    },
}

impl HopKind {
    /// Whether the hop is only time passing (a wire delay, a fault delay, a connection set-up): it
    /// holds no slot, no pipe and no token.
    pub fn is_wait(self) -> bool {
        matches!(
            self,
            HopKind::Net
                | HopKind::Delay
                | HopKind::Connect { .. }
                | HopKind::FirstByte
                | HopKind::Retransmit { .. }
        )
    }
}

/// A pipe a transfer crosses (RULES 62): the edge's wire, or the send or receive pipe of one copy
/// of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipe {
    /// The wire of an edge (`bandwidth` on the edge).
    Wire {
        /// The edge.
        edge: u16,
    },
    /// The send pipe of a copy of a node (`bandwidth` on the node).
    Send {
        /// The node.
        node: NodeId,
        /// The copy.
        replica: u16,
    },
    /// The receive pipe of a copy of a node.
    Recv {
        /// The node.
        node: NodeId,
        /// The copy.
        replica: u16,
    },
}

/// What the engine needs to share pipes fairly (RULES 62): the bytes of a transfer and every pipe
/// with a rate that it crosses. The hop's own `dur` stays `bytes / the narrowest rate`, the time at
/// zero load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xfer {
    /// Bytes to move.
    pub bytes: u64,
    /// The pipes with a rate on the path (at most three, the used ones first).
    pub pipes: [Option<Pipe>; 3],
    /// A ceiling on the rate (bytes a second): a link chaos `bandwidth_cap` on the wire.
    pub limit: Option<u64>,
}

impl Xfer {
    /// The pipes used.
    pub fn pipes(&self) -> impl Iterator<Item = Pipe> + '_ {
        self.pipes.iter().flatten().copied()
    }
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
    /// A node holding its slot across a call (`hold: call`, pre-order): the node takes a slot of
    /// `replica` here, queueing like any hop when none is free, and the `size` items after this one
    /// (its own hops and everything it calls) run inside that slot, which is freed when the walk
    /// passes the last of them. The node's own hops in the scope take no slot of their own. Never
    /// emitted for a node without slots.
    Hold {
        /// The node whose slot is held.
        node: NodeId,
        /// The replica whose slot it is.
        replica: u16,
        /// Items after this one that run inside the held slot.
        size: usize,
    },
    /// The copy of `node` the next visit takes is not fixed yet: hops and holds after this item
    /// that name `replica == code` (`>= DYN_BASE`) run on the copy the simulator chooses from
    /// `cands` when it reaches this item, by the live queues. A plan for the mocks and for
    /// `execute` is made static first ([`Plan::resolve_static`]). Only `least-conn` and `p2c`
    /// emit it.
    Pick {
        /// The node whose copy is chosen.
        node: NodeId,
        /// The code the later hops carry in `replica`.
        code: u16,
        /// How the copy is chosen.
        kind: PickKind,
        /// Candidate copies, bit `i` for copy `i` (at most 64 copies).
        cands: u64,
        /// Rotates the tie-break of `least-conn`, like the round-robin offset.
        offset: u16,
        /// Extra work time per copy (ns, indexed by copy) from chaos aimed at one copy: the part
        /// the plan could not add because the copy was not known. `None` when no copy has any.
        skew: Option<Arc<[u64]>>,
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
        /// The caller's edge says `abandon: continue`: when the deadline passes, the work in the span goes on.
        orphan: bool,
    },
    /// A call that may be tried again (RULES 82-84, pre-order): `attempts` [`PlanItem::Attempt`]s follow,
    /// `size` items in all. Every attempt is sampled up front, because whether one fails can depend on
    /// load the sampler cannot see; the replay engine runs them in order until one succeeds, and
    /// [`Plan::scoped`] says which failed at zero load.
    Retry {
        /// Tries, the first included.
        attempts: u8,
        /// Items after this one that belong to the scope (the attempts and their markers).
        size: usize,
        /// The retry budget this call draws on, when it has one.
        budget: Option<RetryBudget>,
        /// The circuit breaker every attempt asks first (step 7c), when the edge has one.
        breaker: Option<BreakerRef>,
    },
    /// One try of the enclosing [`PlanItem::Retry`]: its `size` items follow. A failure inside it, that
    /// is transient, starts the next attempt after `backoff` instead of failing the request. Walking
    /// onto the next `Attempt` means this one succeeded.
    Attempt {
        /// The wait before this try (zero for the first): the sampled, jittered backoff.
        backoff: Duration,
        /// Items after this one that belong to the attempt.
        size: usize,
    },
}

/// The gRPC A6 retry throttling a retried call draws on (RULES 82-84): the engine keeps a token count per
/// `(edge, caller replica)`, as a client process keeps one per channel. Integers (thousandths of a token),
/// as A6 itself allows at most three decimals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryBudget {
    /// The edge the call goes over.
    pub edge: u16,
    /// The caller replica holding the budget (a [`DYN_BASE`] code until its pick is bound).
    pub replica: u16,
    /// `maxTokens` in thousandths: the count starts here and never rises above it.
    pub max_milli: u32,
    /// `tokenRatio` in thousandths: what a success adds back (a failure costs 1000).
    pub ratio_milli: u16,
}

/// Which circuit breaker a scope's attempts ask (step 7c): the engine keeps one per `(edge, caller
/// replica)`, as a client process keeps one per dependency; its settings are the edge's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerRef {
    /// The edge the call goes over.
    pub edge: u16,
    /// The caller replica holding the breaker (a [`DYN_BASE`] code until its pick is bound).
    pub replica: u16,
}

/// First `replica` value that is a [`PlanItem::Pick`] code, not a copy (a node has at most 64).
pub const DYN_BASE: u16 = 0x8000;

/// How a [`PlanItem::Pick`] chooses among its candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    /// The candidate with the fewest requests in it; ties rotate with the offset.
    LeastConn,
    /// The less loaded of two candidates; a tie takes the first.
    P2c,
}

/// The sampled tree of one request: what ran, with zero-load times, fixed at arrival.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Items in depth-first order.
    pub items: Vec<PlanItem>,
    /// What the engine shares pipes by for each transfer hop ([`HopKind::Link`], [`HopKind::Nic`]),
    /// as `(index in items, transfer)` in item order. A side table, not a field of [`Hop`]: a hop is
    /// walked a lot and must stay at 48 bytes. A transfer hop without an entry just takes its `dur`.
    pub xfers: Vec<(usize, Xfer)>,
    /// How each failed attempt of a [`PlanItem::Retry`] ended at zero load (the sampler's decision), in
    /// plan order. Empty for a plan without retries. The replay engine ignores it.
    pub scoped: Vec<ScopedFail>,
}

impl Plan {
    /// The transfer of the hop at `idx`, if it has one.
    pub fn xfer(&self, idx: usize) -> Option<&Xfer> {
        let i = self.xfers.binary_search_by_key(&idx, |(i, _)| *i).ok()?;
        Some(&self.xfers[i].1)
    }
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
        let hops = self.hops.iter().map(|h| h.start + h.dur).max();
        // a failure is felt at its instant even when nothing runs then: a call waiting out a retry
        // backoff has no hop, and the caller's deadline or the last attempt's failure ends the request
        let failed = self.fail.and_then(|i| match self.plan.items[i] {
            PlanItem::Fail { at, .. } => Some(at),
            PlanItem::Span { at, timeout, .. } => Some(at + timeout),
            _ => None,
        });
        hops.max(failed).unwrap_or(Duration::ZERO)
    }
}

/// The input to one [`execute`] call.
pub struct ChainRequest<'a> {
    /// The compiled endpoint: key (seed material), op and chain.
    pub endpoint: &'a Endpoint,
    /// The instant the request arrives, from the start of the run: when a chaos outage that begins mid-run
    /// (`from_ms`) and a balancer's `detect` are compared with it (step 7d). Zero where there is no clock
    /// (the live mocks, `execute` tests), which plans exactly as before.
    pub at: Duration,
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
    /// Points every hop and hold of the pick at item `pick` (and the pick itself) at copy `copy`:
    /// the later items that carry the pick's code.
    pub fn bind_pick(&mut self, pick: usize, copy: u16) {
        let PlanItem::Pick { code, skew, .. } = self.items[pick].clone() else {
            return;
        };
        let extra = skew
            .and_then(|s| s.get(usize::from(copy)).copied())
            .map_or(Duration::ZERO, Duration::from_nanos);
        for (i, x) in &mut self.xfers {
            if *i > pick {
                for p in x.pipes.iter_mut().flatten() {
                    if let Pipe::Send { replica, .. } | Pipe::Recv { replica, .. } = p
                        && *replica == code
                    {
                        *replica = copy;
                    }
                }
            }
        }
        for item in &mut self.items[pick + 1..] {
            match item {
                PlanItem::Hop(h) if h.replica == code => {
                    h.replica = copy;
                    // the copy's own chaos latency lands on its work, as in a fixed-copy plan
                    if matches!(
                        h.kind,
                        HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss
                    ) {
                        h.dur += extra;
                    }
                }
                PlanItem::Hold { replica, .. } if *replica == code => *replica = copy,
                PlanItem::Retry {
                    budget: Some(b), ..
                } if b.replica == code => b.replica = copy,
                PlanItem::Retry {
                    breaker: Some(b), ..
                } if b.replica == code => b.replica = copy,
                _ => {}
            }
        }
    }

    /// The copy a [`PlanItem::Pick`] takes with no load to look at (zero load, the mocks): the
    /// rotating offset for `least-conn`, the first candidate for `p2c`.
    pub fn static_copy(kind: PickKind, cands: u64, offset: u16) -> u16 {
        let live: Vec<u16> = (0..64u16).filter(|i| cands >> i & 1 == 1).collect();
        match kind {
            PickKind::LeastConn => live[usize::from(offset) % live.len().max(1)],
            PickKind::P2c => live[0],
        }
    }

    /// This plan with every [`PlanItem::Pick`] settled as for no load: for `execute` and the
    /// live mocks, which have no queues to look at. The simulator uses the plan as it is.
    #[must_use]
    pub fn resolved(mut self) -> Plan {
        for i in 0..self.items.len() {
            if let PlanItem::Pick {
                kind,
                cands,
                offset,
                ..
            } = self.items[i]
            {
                self.bind_pick(i, Plan::static_copy(kind, cands, offset));
            }
        }
        self
    }

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
    fn visible(
        &self,
        fail: Option<(usize, Duration)>,
        cut_all: bool,
        retried: bool,
    ) -> (Vec<Hop>, Vec<usize>) {
        let cut = match fail {
            Some((i, _)) if !cut_all => self.cancelled_by(i),
            _ => Vec::new(),
        };
        let at = fail.map_or(Duration::MAX, |(_, t)| t);
        // no retry scope, nothing to look up (the common case)
        let scopes = if retried {
            scope::ranges(&self.items, &self.scoped)
        } else {
            scope::Ranges::default()
        };
        let (mut hops, mut kept) = (Vec::new(), Vec::new());
        for (i, item) in self.items.iter().enumerate() {
            let PlanItem::Hop(h) = item else { continue };
            let mut h = h.clone();
            if !scopes.is_empty() {
                // an attempt after the one that settled the call never ran; one that failed stops at its failure
                if scopes.unreached.iter().any(|r| r.contains(&i)) {
                    continue;
                }
                if let Some((_, stop)) = scopes.cut.iter().find(|(r, _)| r.contains(&i)) {
                    if h.start >= *stop {
                        continue;
                    }
                    h.dur = h.dur.min(stop.saturating_sub(h.start));
                }
            }
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
    let (plan, planned, retried) = sample(topo, chaos, req);
    // zero load has no queues to look at: `least-conn` and `p2c` are settled by their tie rules
    let plan = plan.resolved();
    let decided = scope::deadline_in(
        &plan.items,
        &plan.scoped,
        0..plan.items.len(),
        planned.map(|(_, t)| t),
    )
    .or(planned);
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
    // a deadline cuts every hop at its instant, not only the cancelled sibling branches
    let cut_all = decided.is_some_and(|(i, _)| matches!(plan.items[i], PlanItem::Span { .. }));
    let (hops, kept) = plan.visible(decided, cut_all, retried);
    ChainOutcome {
        hops,
        kept,
        plan,
        result,
        fail: decided.map(|(i, _)| i),
    }
}

/// The sampling half of [`execute`]: the [`Plan`] and the *planned* failure (plan index,
/// instant; a deadline from a `timeout:` span is not planned, it fires at run time), without building the client-visible hops. The simulator replays the plan
/// itself, so it skips that work; [`execute`] returns this same plan.
pub fn sample_plan(
    topo: &Topology,
    chaos: &EffectiveChaos,
    req: &ChainRequest,
) -> (Plan, Option<(usize, Duration)>) {
    let (plan, fail, _) = sample(topo, chaos, req);
    (plan, fail)
}

/// **Private.** [`sample_plan`], also saying whether the plan has a retry scope.
fn sample(
    topo: &Topology,
    chaos: &EffectiveChaos,
    req: &ChainRequest,
) -> (Plan, Option<(usize, Duration)>, bool) {
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
        at: req.at,
        n: req.n,
        picks: vec![0; topo.nodes.len()],
        routes: vec![0; topo.nodes.len()],
        dyn_picks: 0,
        bytes: (req.req_bytes, req.resp_bytes),
        items: Vec::new(),
        xfers: Vec::new(),
        scoped: Vec::new(),
        retried: false,
        tail: None,
    };
    let entry = ep.entry_node;
    // The entry node has no calling edge, so its own balance picks its replica.
    let entry_pick = entry.map_or(Some(0), |e| w.pick(e, None, e, root));
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
    let root_span = entry.and_then(|n| w.open_span(n, base, false));
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
        let hold = entry.and_then(|n| w.open_hold(n, er));
        let flow = w.seq(
            &ep.chain,
            root,
            ep.op,
            start,
            (entry.unwrap_or_default(), er),
        );
        w.close_hold(hold);
        flow
    });
    w.close_span(root_span);
    let fail = match flow {
        Flow::Fail { at, idx } => Some((idx, at)),
        Flow::Done(_) | Flow::Hit(_) => None,
    };
    let retried = w.retried;
    (w.into_plan(), fail, retried)
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
