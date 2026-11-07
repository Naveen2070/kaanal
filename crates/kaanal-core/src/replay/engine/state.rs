//! The engine's data: the error and failure types, the finished-request records, and the per-request and per-node state the step loop reads and writes.

use super::{Arc, Bucket, ChainResult, Duration, FailKind, Gate, Hop, NodeId, Plan, fmt};

/// **Private.** `starts` value of a hop that never started.
pub(super) const NOT_STARTED: u64 = u64::MAX;

/// **Private.** Set on the seq of a deadline event, so it sorts after every other event of the
/// same instant (Z14) without changing the heap key or the simulator's `(time, seq)` merge.
pub(super) const DEADLINE: u64 = 1 << 62;

/// **Private.** Set on the seq of a retry timer (a backoff ending), so it sorts after the plain events of its
/// instant and before a deadline's, and is told apart from both.
pub(super) const RETRY: u64 = 1 << 61;

/// Why the engine stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    /// An event time would pass `u64::MAX` ns (about 584 years). The engine stays failed.
    ClockOverflow,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the replay clock would pass u64::MAX nanoseconds (about 584 years)")
    }
}

impl std::error::Error for EngineError {}

/// Why a replayed request failed; carried on [`Finished::cause`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCause {
    /// A planned failure from the chaos plan (a hang included).
    Chaos,
    /// The node's rate limit refused it (429).
    RateLimit,
    /// The node's queue was full.
    Overflow,
    /// A `timeout:` span's deadline fired.
    Deadline,
    /// The caller's circuit breaker was open and rejected the call (step 7c).
    Breaker,
}

/// What a finished request experienced.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// The caller's key from [`Engine::submit`].
    pub tag: u64,
    /// Submit order, starting at 0.
    pub order: u64,
    /// Submit time (ns).
    pub arrival: u64,
    /// When the client got its answer (ns).
    pub done: u64,
    /// Ok, or where and how the request failed.
    pub result: ChainResult,
    /// Why it failed; `None` when `result` is Ok.
    pub cause: Option<FailCause>,
    /// The hops that ran, in plan order: `start` is the offset from `arrival`; a hop a failure
    /// cancelled while running is cut at the failure instant; one that had not started is absent.
    pub hops: Vec<FinishedHop>,
    /// The slots it held across a call, in the order they were taken.
    pub holds: Vec<FinishedHold>,
    /// The retries it made (attempt 2 and later of a call), in the order they started.
    pub attempts: Vec<FinishedAttempt>,
}

/// A retry that started (step 7e): a second or later attempt of a call, as the Log shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedAttempt {
    /// When it started, as an offset from arrival (after its backoff).
    pub at: Duration,
    /// Which attempt it is, counting the first as 1 (so a first retry is 2).
    pub n: u8,
    /// How many attempts the call may make.
    pub of: u8,
    /// The component the call goes to.
    pub node: NodeId,
}

/// A slot a request held across a call (`hold: call`), as the client's record sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedHold {
    /// The node whose slot it was.
    pub node: NodeId,
    /// The replica whose slot it was.
    pub replica: u16,
    /// When it was taken, as an offset from arrival.
    pub start: Duration,
    /// How long it was held (cut at the answer when it was still held then).
    pub dur: Duration,
}

/// One hop of a [`Finished`] request.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedHop {
    /// Index of the hop in the submitted plan.
    pub idx: usize,
    /// The hop with its real `start` and (cut) `dur`.
    pub hop: Hop,
    /// Time spent in the node's queue before it started.
    pub wait: Duration,
    /// The try of its call this hop ran in, counting the first as 1; also 1 outside any retried call.
    pub attempt: u8,
}

/// Per-node totals. `completed`, `wait`, `limited` and `overflowed` count requests
/// arriving at or after the window's `warmup`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeCounters {
    /// Busy slot-nanoseconds inside the window `[warmup, end]`.
    pub area: u128,
    /// Longest queue seen at or after `warmup`.
    pub max_queue: usize,
    /// Own hops that started.
    pub completed: u64,
    /// Summed queue waits (ns) before those hops.
    pub wait: u128,
    /// Hop starts refused by the rate limit (429).
    pub limited: u64,
    /// Hop starts refused by a full queue.
    pub overflowed: u64,
    /// Requests failed by this node's deadline, counted at the span's node.
    pub timed_out: u64,
    /// Calls the caller's circuit breaker rejected without sending (step 7c), counted at the callee.
    pub short_circuited: u64,
}

/// **Private.** A walker over one plan range: the whole plan, or one `parallel` branch.
pub(super) struct Token {
    /// Next plan index.
    pub(super) at: usize,
    /// End of the range (exclusive).
    pub(super) end: usize,
    /// The join it reports to at its end; `None` for the root token.
    pub(super) join: Option<usize>,
    /// The replica queue it waits in (node, replica), and since when (ns).
    pub(super) queued_at: Option<(NodeId, u16, u64)>,
    /// Node of the last own hop with no edge hop since: the current visit (Y6).
    pub(super) last_own: Option<NodeId>,
    /// Innermost span this token opened that is still open (`SpanState::parent` chains outward).
    pub(super) open_top: Option<usize>,
    /// Innermost hold this token stands in (`HoldState::parent` chains outward). A branch token
    /// starts with its parent's, which it may look at but never lets go of (`HoldState::owner`).
    pub(super) hold_top: Option<usize>,
}

/// **Private.** A `parallel` block waiting for its branches.
pub(super) struct Join {
    pub(super) left: usize,
    pub(super) parent: usize,
}

/// **Private.** One span a request has entered.
pub(super) struct SpanState {
    /// The enclosing span still open on the same token when this one was entered.
    pub(super) parent: Option<usize>,
    /// Plan index just past the span; its opener closes it on walking on past this.
    pub(super) end: usize,
    /// Plan index of the `Span` item.
    pub(super) idx: usize,
    /// The node whose `timeout` this is; a timeout is counted and reported here.
    pub(super) node: NodeId,
    /// Seq of its deadline event: identifies the span, so a recycled request slot cannot match.
    pub(super) seq: u64,
    /// When the deadline falls (ns).
    pub(super) due: u64,
    /// Not yet walked past by its opening token; only an open span can time the request out.
    pub(super) open: bool,
    /// `abandon: continue`: when the deadline passes, the work inside the span goes on (see `retry.rs`).
    pub(super) orphan: bool,
}

/// What a retry scope did on one wire over the reporting window (step 7f), counted for requests
/// arriving at or after the window's `warmup`. A *call* is one entry into a scope, so a call that
/// succeeds first time is only a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WireCounters {
    /// Calls over the wire that carried a retry scope.
    pub calls: u64,
    /// Calls that made at least one retry.
    pub retried: u64,
    /// Retry attempts, all calls (a call retried twice adds 2).
    pub retries: u64,
    /// Retried calls that ended well.
    pub recovered: u64,
    /// Calls whose last attempt failed, retried or not (retried and not recovered gave up).
    pub failed: u64,
}

/// What a circuit breaker is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Calls go through; outcomes are counted.
    Closed,
    /// A few probe calls decide whether to close or reopen.
    HalfOpen,
    /// Every call is refused until the open time has passed.
    Open,
}

/// A breaker moved to `state` at `at` ns on the virtual clock (step 7f).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerChange {
    /// The edge the breaker is on.
    pub edge: u16,
    /// The caller replica holding the breaker.
    pub replica: u16,
    /// When it changed, ns on the virtual clock.
    pub at: u64,
    /// What it changed to.
    pub state: BreakerState,
}

/// **Private.** A retry scope a request has entered (RULES 82-84, see `retry.rs`).
pub(super) struct RetryState {
    /// Plan index just past the whole scope.
    pub(super) end: usize,
    /// Plan index of the `Attempt` marker of the attempt now running (or just cut, while waiting).
    pub(super) cur_start: usize,
    /// Plan index just past that attempt: the next `Attempt` marker, or `end` for the last one.
    pub(super) cur_end: usize,
    /// What the token that entered the scope looked like: the next attempt's token copies it.
    pub(super) base: Token,
    /// Not yet left: the scope can still end an attempt with a retry.
    pub(super) open: bool,
    /// The attempt was cut and the next one waits for its backoff to pass.
    pub(super) waiting: bool,
    /// The budget the call draws on, if any.
    pub(super) budget: Option<crate::exec::RetryBudget>,
    /// The circuit breaker every attempt asks, if the edge has one.
    pub(super) breaker: Option<crate::exec::BreakerRef>,
    /// Seq of the pending backoff timer; identifies it, so a recycled request slot cannot match.
    pub(super) timer: u64,
    /// How many attempts the scope has, and the plan index of the marker of the first.
    pub(super) attempts: u8,
    pub(super) first: usize,
    /// The edge the call goes over ([`crate::exec::NO_EDGE`] when unknown) and whether it has retried.
    pub(super) edge: u16,
    pub(super) retried: bool,
    /// When each retry started (ns), in order: the Log shows them.
    pub(super) starts: Vec<u64>,
}

/// **Private.** An attempt that was cut short: plan indices `lo < at <= hi` of it stop at `at` (ns).
pub(super) struct Cut {
    pub(super) lo: usize,
    pub(super) hi: usize,
    pub(super) at: u64,
}

/// **Private.** One slot a request holds across a call (`hold: call`, see `hold.rs`).
pub(super) struct HoldState {
    /// The enclosing hold on the same token when this one was taken.
    pub(super) parent: Option<usize>,
    /// The token that took it, and the only one that lets it go.
    pub(super) owner: usize,
    /// Plan index just past the held scope; the owner frees the slot on walking on past it.
    pub(super) end: usize,
    pub(super) node: NodeId,
    pub(super) replica: u16,
    /// The slot is still held.
    pub(super) open: bool,
    /// When it was taken and, once let go, when (ns).
    pub(super) since: u64,
    pub(super) until: u64,
}

/// **Private.** The failure that decided a request: instant, plan index (D17), node, status.
pub(super) struct Failure {
    pub(super) at: u64,
    pub(super) idx: usize,
    pub(super) node: NodeId,
    pub(super) status: u16,
    pub(super) kind: FailKind,
    pub(super) retry_after: Option<Duration>,
    /// Set when the engine itself refused the hop and counted it: the node and replica, and whether it was the
    /// rate limit (`true`) or the queue (`false`). A planned `Fail` that replaces this failure (D17)
    /// takes the count back, so counters match the answers.
    pub(super) refused: Option<(NodeId, u16, bool)>,
    /// Why it failed. Not derivable from `refused` or `kind`: a fired deadline and a planned hang
    /// both have kind `Timeout` and `refused: None`.
    pub(super) cause: FailCause,
}

/// **Private.** One request in flight.
pub(super) struct Req {
    pub(super) tag: u64,
    pub(super) order: u64,
    pub(super) arrival: u64,
    pub(super) plan: Arc<Plan>,
    pub(super) starts: Vec<u64>,
    pub(super) waits: Vec<u64>,
    pub(super) tokens: Vec<Token>,
    pub(super) joins: Vec<Join>,
    pub(super) spans: Vec<SpanState>,
    pub(super) holds: Vec<HoldState>,
    pub(super) retries: Vec<RetryState>,
    pub(super) cuts: Vec<Cut>,
    /// Plan ranges `(span index, span end)` whose work goes on after a deadline passed (`abandon: continue`).
    pub(super) orphans: Vec<(usize, usize)>,
    /// Tokens holding a hop or waiting in a queue.
    pub(super) busy: usize,
    pub(super) fail: Option<Failure>,
    /// When the client got its answer (ns).
    pub(super) done: Option<u64>,
    /// The `Finished` was produced (or skipped for a pre-warmup request).
    pub(super) emitted: bool,
}

/// **Private.** The per-request buffers of a `Req`, recycled between requests (R6.13).
#[derive(Default)]
pub(super) struct Scratch {
    pub(super) starts: Vec<u64>,
    pub(super) waits: Vec<u64>,
    pub(super) tokens: Vec<Token>,
    pub(super) joins: Vec<Join>,
    pub(super) spans: Vec<SpanState>,
    pub(super) holds: Vec<HoldState>,
    pub(super) retries: Vec<RetryState>,
    pub(super) cuts: Vec<Cut>,
    pub(super) orphans: Vec<(usize, usize)>,
}

impl Req {
    /// Whether plan index `at` lies inside the work of a call whose caller gave up but which goes on.
    #[inline]
    pub(super) fn orphan_at(&self, at: usize) -> bool {
        !self.orphans.is_empty() && self.orphans.iter().any(|&(lo, hi)| lo < at && at < hi)
    }
}

/// **Private.** Queueing state of one node.
pub(super) struct NodeState {
    pub(super) gate: Option<Gate>,
    pub(super) bucket: Option<Bucket>,
    /// Last time `busy` changed (ns); `NodeCounters::area` is integrated up to here.
    pub(super) since: u64,
}

/// **Private.** The live request in slot `r`.
pub(super) fn live(reqs: &mut [Option<Req>], r: usize) -> &mut Req {
    reqs[r].as_mut().expect("events only name live requests")
}

/// **Private.** Duration → nanoseconds; validation keeps durations far below `u64::MAX`.
pub(super) fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("validated durations fit in u64 nanoseconds")
}
