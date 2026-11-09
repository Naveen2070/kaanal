//! What one finished request leaves behind: the `X-Kaanal-Trace` text, the [`LogEntry`] for the
//! request log and the bus [`Event`], all built from the same plan and `Finished` so they cannot
//! disagree (L11).
//!
//! The trace is built once into two texts: `full` (kept in the log, cut only at [`MAX_TRACE`]) and
//! `header` (what is sent, cut at [`TRACE_CAP`]), both by the same bracket-closing and
//! `...+N hops` rule. Secrets never get here: only names, times, statuses and the path (no query
//! string, no headers, no bodies).

use std::fmt::Write as _;
use std::time::Duration;

use kaanal_core::{
    ChainResult, EffectiveChaos, ErrorKind, FailCause, FailKind, Finished, FinishedAttempt,
    HANG_CAP, HopKind, Plan, PlanItem, Resolved, Topology,
};

use crate::log::{ChainErrorRec, HopRec, LogEntry, MAX_HOPS, MAX_TRACE};
use crate::state::Event;

/// Longest `X-Kaanal-Trace` value in bytes (Node's undici rejects headers over
/// about 16 KiB). The error part is always kept, so a long failing node name can pass it.
const TRACE_CAP: usize = 4096;

/// The two renderings of one trace: `full` for the log, `header` for the response.
pub struct TraceParts {
    /// The log copy: uncapped unless it passes [`MAX_TRACE`].
    pub full: String,
    /// The `X-Kaanal-Trace` value, at most [`TRACE_CAP`] bytes (plus a long error part).
    pub header: String,
}

/// **Private.** One piece of the trace: a hop, or the bracket structure of a `parallel`.
enum Piece {
    Hop(String),
    /// A marker between hops (`retry=2/3`): in the text, but not a hop to count, cut or keep a block for.
    Mark(String),
    Open,
    Bar,
    Close,
}

/// The `X-Kaanal-Trace` value: visible hops in plan order with one decimal
/// (`1.0ms`, never a truncated `0ms`); a `parallel` block is `[a,b|c]` (branches split by
/// `|`); cache hops are `node.hit` / `node.miss`; `node.wait=T` comes before a hop that
/// queued; a failure appends `node.error=STATUS`, or `node.error=reset` / `node.error=malformed`
/// for those kinds (they have no status); a pinned request ends with `pinned=STATE`. A text over
/// its cap ([`TRACE_CAP`] for the header, [`MAX_TRACE`] for the log copy) stops before a
/// hop, closes open brackets and adds `,...+N hops` (ASCII, D13), N being the pieces left out.
pub fn trace_parts(plan: &Plan, fin: &Finished, topo: &Topology, pin: Option<&str>) -> TraceParts {
    let (hops, result, attempts) = (&fin.hops[..], &fin.result, &fin.attempts[..]);
    let name = |id: usize| topo.nodes[id].name.as_str();
    // `db#2` for a hop on one copy of a replicated node (human text only; the API keeps `replica` apart)
    let at = |id: usize, replica: u16| {
        if topo.nodes[id].replicas > 1 {
            format!("{}#{replica}", name(id))
        } else {
            name(id).to_string()
        }
    };
    let fmt_ms = |d: Duration| format!("{:.1}ms", d.as_secs_f64() * 1000.0);
    let mut pieces = Vec::new();
    // (end of the fork's items, index of its first branch marker) per open fork.
    let mut forks: Vec<(usize, usize)> = Vec::new();
    let mut next = 0; // index into `hops`
    let mut marks = Marks::new(attempts);
    for (i, item) in plan.items.iter().enumerate() {
        while forks.last().is_some_and(|&(end, _)| end == i) {
            forks.pop();
            pieces.push(Piece::Close);
        }
        match item {
            PlanItem::Fork { size, .. } => {
                forks.push((i + 1 + size, i + 1));
                pieces.push(Piece::Open);
            }
            PlanItem::Branch { .. } => {
                if forks.last().is_some_and(|&(_, first)| first != i) {
                    pieces.push(Piece::Bar);
                }
            }
            PlanItem::Retry { attempts, size, .. } => marks.open(i + 1 + size, *attempts),
            PlanItem::Attempt { .. } => {
                if let Some((n, of)) = marks.start(i) {
                    pieces.push(Piece::Mark(format!("retry={n}/{of}")));
                }
            }
            PlanItem::Hop(_) if hops.get(next).is_some_and(|h| h.idx == i) => {
                let fh = &hops[next];
                let h = &fh.hop;
                next += 1;
                if !fh.wait.is_zero() {
                    pieces.push(Piece::Hop(format!(
                        "{}.wait={}",
                        at(h.node, h.replica),
                        fmt_ms(fh.wait)
                    )));
                }
                let d = fmt_ms(h.dur);
                pieces.push(Piece::Hop(match h.kind {
                    HopKind::Net => format!("net={d}"),
                    HopKind::Connect { resp } => format!("{}={d}", connect_name(resp)),
                    HopKind::FirstByte => format!("{}.first-byte={d}", at(h.node, h.replica)),
                    HopKind::Retransmit { resp } => format!("{}={d}", retransmit_name(resp)),
                    HopKind::Proc => format!("{}.proc={d}", at(h.node, h.replica)),
                    HopKind::Op(op) => format!("{}.{}={d}", at(h.node, h.replica), op.name()),
                    HopKind::CacheHit => format!("{}.hit={d}", at(h.node, h.replica)),
                    HopKind::CacheMiss => format!("{}.miss={d}", at(h.node, h.replica)),
                    HopKind::Delay => format!("{}.delay={d}", at(h.node, h.replica)),
                    HopKind::Link { resp, .. } => {
                        format!("{}.{}={d}", at(h.node, h.replica), link_name(resp))
                    }
                    HopKind::Nic { send, .. } => {
                        format!("{}.{}={d}", at(h.node, h.replica), nic_name(send))
                    }
                }));
            }
            // a hop is a piece through `hops`; the rest draw nothing
            _ => {}
        }
    }
    pieces.extend(forks.iter().map(|_| Piece::Close));
    let pieces = without_empty_blocks(pieces);
    let error = error_part(result, name);
    // The error and the pin are always kept, so they are reserved before the hops are cut.
    let pinned = pin.map_or_else(String::new, |p| format!("pinned={p}"));
    let tails: Vec<&String> = [&error, &pinned]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect();
    let tail: usize = tails.iter().map(|t| t.len() + 1).sum();
    let whole = render(&pieces, usize::MAX, 0);
    let over = |cap: usize| whole.len() + tail > cap;
    let mut header = if over(TRACE_CAP) {
        render(&pieces, TRACE_CAP, tail)
    } else {
        whole.clone()
    };
    let mut full = if over(MAX_TRACE) {
        render(&pieces, MAX_TRACE, tail)
    } else {
        whole
    };
    for t in tails {
        for out in [&mut header, &mut full] {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(t);
        }
    }
    TraceParts { full, header }
}

/// **Private.** The trace's `node.error=...` tail, empty when the request succeeded.
fn error_part<'a>(result: &ChainResult, name: impl Fn(usize) -> &'a str) -> String {
    match result {
        ChainResult::Err {
            node, status, kind, ..
        } => match kind {
            FailKind::Reset | FailKind::Malformed => {
                format!("{}.error={}", name(*node), kind.as_str())
            }
            _ => format!("{}.error={status}", name(*node)),
        },
        ChainResult::Ok => String::new(),
    }
}

/// **Private.** Which retry of a call the plan walk is in. A call's attempts follow its `Retry` item;
/// every attempt after the first has a marker in the trace, but only when it really started, which
/// `Finished::attempts` says (the engine lists each retry it began, by number and call size).
struct Marks<'a> {
    started: &'a [FinishedAttempt],
    used: Vec<bool>,
    /// Per open call: where its items end, how many tries it may make, how many markers were met.
    calls: Vec<(usize, u8, u8)>,
}

impl<'a> Marks<'a> {
    fn new(started: &'a [FinishedAttempt]) -> Self {
        Self {
            started,
            used: vec![false; started.len()],
            calls: Vec::new(),
        }
    }

    fn open(&mut self, end: usize, attempts: u8) {
        self.calls.push((end, attempts, 0));
    }

    /// Walking onto the attempt marker at `i` (`size` items follow): the `(n, of)` to print when this is a
    /// retry that started.
    fn start(&mut self, i: usize) -> Option<(u8, u8)> {
        self.calls.retain(|c| c.0 > i);
        let call = self.calls.last_mut()?;
        let n = call.2 + 1;
        call.2 = n;
        let of = call.1;
        if n == 1 {
            return None;
        }
        let at = (0..self.started.len())
            .find(|&k| !self.used[k] && self.started[k].n == n && self.started[k].of == of)?;
        self.used[at] = true;
        Some((n, of))
    }
}

/// **Private.** Drops every `[...]` block with no hop inside (a `parallel` that a sibling's
/// failure cancelled before it started would otherwise print as `[|]`).
fn without_empty_blocks(pieces: Vec<Piece>) -> Vec<Piece> {
    // Open positions with the hops seen inside each; a block's range is dropped when it has none.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut drop: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        match p {
            Piece::Open => open.push((i, 0)),
            Piece::Hop(_) => {
                if let Some(top) = open.last_mut() {
                    top.1 += 1;
                }
            }
            Piece::Close => {
                if let Some((start, hops)) = open.pop() {
                    if hops == 0 {
                        drop.push(start..i + 1);
                    } else if let Some(parent) = open.last_mut() {
                        parent.1 += hops;
                    }
                }
            }
            Piece::Bar | Piece::Mark(_) => {}
        }
    }
    pieces
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !drop.iter().any(|r| r.contains(i)))
        .map(|(_, p)| p)
        .collect()
}

/// **Private.** Joins `pieces` (commas between elements, none after `[` or `|`). Stops
/// before the first hop that would not leave room, within `cap`, for the open brackets,
/// the `,...+N hops` suffix and `tail` more bytes.
fn render(pieces: &[Piece], cap: usize, tail: usize) -> String {
    let total = pieces.iter().filter(|p| matches!(p, Piece::Hop(_))).count();
    let suffix = ",...+".len() + total.to_string().len() + " hops".len();
    let (mut out, mut first, mut depth, mut shown) = (String::new(), true, 0usize, 0usize);
    for p in pieces {
        match p {
            Piece::Hop(h) => {
                let sep = usize::from(!first);
                if out.len() + sep + h.len() + depth + suffix + tail > cap {
                    out.extend(std::iter::repeat_n(']', depth));
                    if !out.is_empty() {
                        out.push(',');
                    }
                    let _ = write!(out, "...+{} hops", total - shown);
                    return out;
                }
                if !first {
                    out.push(',');
                }
                out.push_str(h);
                (first, shown) = (false, shown + 1);
            }
            Piece::Mark(m) => {
                let sep = usize::from(!first);
                if out.len() + sep + m.len() + depth + suffix + tail > cap {
                    out.extend(std::iter::repeat_n(']', depth));
                    if !out.is_empty() {
                        out.push(',');
                    }
                    let _ = write!(out, "...+{} hops", total - shown);
                    return out;
                }
                if !first {
                    out.push(',');
                }
                out.push_str(m);
                first = false;
            }
            Piece::Open => {
                if !first {
                    out.push(',');
                }
                out.push('[');
                (first, depth) = (true, depth + 1);
            }
            Piece::Bar => {
                out.push('|');
                first = true;
            }
            Piece::Close => {
                out.push(']');
                (first, depth) = (false, depth.saturating_sub(1));
            }
        }
    }
    out
}

/// What is known about a request before and around the chain.
pub struct Facts<'a> {
    /// Our id text.
    pub id: String,
    /// The client's sanitised `X-Request-Id`.
    pub client_id: Option<String>,
    /// The id's number.
    pub seq: u64,
    /// UTC ms since the epoch.
    pub ts_ms: u64,
    /// Measured wall time, ms.
    pub ms: u64,
    /// Uppercase method.
    pub method: &'a str,
    /// Request path (no query).
    pub path: &'a str,
    /// The matched route key.
    pub endpoint: Option<&'a str>,
    /// Canonical scope key.
    pub scope: Option<&'a str>,
}

/// What the client was told.
pub struct Answer<'a> {
    /// The status sent.
    pub status: u16,
    /// The UI state a pin forced.
    pub pin: Option<&'static str>,
    /// The node whose failure the client saw, and how.
    pub failed: Option<&'a (String, FailKind)>,
    /// A live payload edit answered it (step 8A).
    pub edited: bool,
}

/// What the chain did: the plan, the engine's verdict and the chaos the plan was sampled under.
pub struct Run<'a> {
    /// The sampled plan.
    pub plan: &'a Plan,
    /// What the engine reports.
    pub fin: &'a Finished,
    /// The chaos snapshot (or scoped stack) the request used.
    pub chaos: &'a EffectiveChaos,
    /// The topology.
    pub topo: &'a Topology,
    /// A pin delay was planned (the leading hop is it).
    pub pin_delay: bool,
}

/// **Private.** The trace and log name of a link hop's direction.
fn link_name(resp: bool) -> &'static str {
    if resp { "link-resp" } else { "link" }
}

/// **Private.** The trace and log name of a connection set-up hop: the handshake and the request's
/// slow start, or the response's slow start.
fn connect_name(resp: bool) -> &'static str {
    if resp { "connect-resp" } else { "connect" }
}

/// **Private.** The trace and log name of a retransmit hop: after the request or after the response.
fn retransmit_name(resp: bool) -> &'static str {
    if resp {
        "retransmit-resp"
    } else {
        "retransmit"
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Longest `path` and `method` an entry keeps; a longer one ends in `…`. A request line is
/// client input, and the ring keeps 1000 of them.
const MAX_PATH: usize = 2048;
const MAX_METHOD: usize = 16;

/// `s` cut to at most `max` bytes on a char boundary, with a `…` marker when it was cut.
fn capped(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max - '…'.len_utf8();
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

fn blank(f: Facts, status: u16) -> LogEntry {
    LogEntry {
        id: f.id,
        client_id: f.client_id,
        seq: f.seq,
        done: 0,
        ts_ms: f.ts_ms,
        method: capped(f.method, MAX_METHOD),
        path: capped(f.path, MAX_PATH),
        endpoint: f.endpoint.map(str::to_string),
        status,
        ms: f.ms,
        trace_ms: 0.0,
        scope: f.scope.map(str::to_string),
        pin: None,
        flags: Vec::new(),
        chain_error: None,
        failed_node: None,
        kind: None,
        rejected: None,
        edited: false,
        trace: String::new(),
        hops: Vec::new(),
        truncated: false,
    }
}

/// The entry of a request answered before any chain ran (`unmatched`, `bad-scope`,
/// `engine-stopped`) or abandoned by its client (`client-gone`).
pub fn reject_entry(f: Facts, status: u16, rejected: &'static str) -> LogEntry {
    LogEntry {
        rejected: Some(rejected),
        ..blank(f, status)
    }
}

/// The entry of a request the chain served. `trace` is [`TraceParts::full`].
pub fn build_entry(f: Facts, a: &Answer, run: &Run, trace: String) -> LogEntry {
    let fin = run.fin;
    let (hops, truncated, delayed, faulted) = hop_recs(f.endpoint.unwrap_or(""), run);
    let cause = fin.cause;
    let mut flags = Vec::new();
    if cause == Some(FailCause::Chaos) || faulted {
        flags.push("FI");
    }
    if delayed {
        flags.push("DI");
    }
    if run.pin_delay {
        flags.push("PD");
    }
    match cause {
        Some(FailCause::RateLimit) => flags.push("RL"),
        Some(FailCause::Overflow) => flags.push("UO"),
        Some(FailCause::Deadline) => flags.push("DT"),
        Some(FailCause::Breaker) => flags.push("CB"),
        Some(FailCause::Chaos) | None => {}
    }
    if a.pin.is_some() {
        flags.push("PN");
    }
    let chain_error = match &fin.result {
        ChainResult::Err {
            node, status, kind, ..
        } => Some(ChainErrorRec {
            node: run.topo.nodes[*node].name.clone(),
            status: *status,
            kind: kind.as_str(),
            cause: match cause {
                Some(FailCause::RateLimit) => "rate-limit",
                Some(FailCause::Overflow) => "overflow",
                Some(FailCause::Deadline) => "deadline",
                Some(FailCause::Breaker) => "breaker",
                Some(FailCause::Chaos) | None => "chaos",
            },
        }),
        ChainResult::Ok => None,
    };
    LogEntry {
        trace_ms: fin
            .hops
            .iter()
            .map(|h| h.hop.start + h.hop.dur)
            .max()
            .map_or(0.0, ms),
        pin: a.pin,
        edited: a.edited,
        flags,
        chain_error,
        failed_node: a.failed.map(|x| x.0.clone()),
        kind: a
            .failed
            .map(|x| x.1)
            .filter(|k| *k != FailKind::Status)
            .map(FailKind::as_str),
        trace,
        hops,
        truncated,
        ..blank(f, a.status)
    }
}

/// **Private.** The chaos that applies to a hop of `kind` at `(node, replica)`: a fault delay takes
/// the endpoint's, a node's own hop the node's, and a wire, card or connection hop none.
fn hop_chaos<'a>(
    run: &'a Run,
    key: &str,
    kind: HopKind,
    (node, replica): (usize, u16),
) -> Option<&'a Resolved> {
    match kind {
        HopKind::Delay => run.chaos.endpoint(key),
        HopKind::Proc | HopKind::Op(_) | HopKind::CacheHit | HopKind::CacheMiss => {
            run.chaos.visit_replica(node, key, replica)
        }
        HopKind::Net
        | HopKind::Connect { .. }
        | HopKind::FirstByte
        | HopKind::Retransmit { .. }
        | HopKind::Link { .. }
        | HopKind::Nic { .. } => None,
    }
}

/// **Private.** The structured hops (at most [`MAX_HOPS`]), whether more existed, and whether any
/// hop carried injected delay or a hanging cache lookup (a fault the request survived).
fn hop_recs(key: &str, run: &Run) -> (Vec<HopRec>, bool, bool, bool) {
    let name = |id: usize| run.topo.nodes[id].name.clone();
    let mut recs = Vec::new();
    let (mut truncated, mut delayed, mut faulted) = (false, false, false);
    // (end of the fork's items, the fork's ordinal, the current branch's lane) per open fork.
    let mut forks: Vec<(usize, u16, u16)> = Vec::new();
    let mut ordinal = 0u16;
    let mut next = 0;
    for (i, item) in run.plan.items.iter().enumerate() {
        while forks.last().is_some_and(|f| f.0 == i) {
            forks.pop();
        }
        match item {
            PlanItem::Fork { size, .. } => {
                forks.push((i + 1 + size, ordinal, 0));
                ordinal = ordinal.saturating_add(1);
            }
            PlanItem::Branch { .. } => {
                if let Some(f) = forks.last_mut() {
                    f.2 = f.2.saturating_add(1);
                }
            }
            PlanItem::Hop(planned) if run.fin.hops.get(next).is_some_and(|h| h.idx == i) => {
                let fh = &run.fin.hops[next];
                let h = &fh.hop;
                next += 1;
                let mut why: Vec<String> = Vec::new();
                let lead_pin = run.pin_delay && i == 0 && h.kind == HopKind::Delay;
                // A hang is a capped hop followed by the node's own timeout failure: injected
                // failure (FI), never injected delay.
                let hang = matches!(
                    run.plan.items.get(i + 1),
                    Some(PlanItem::Fail { node, kind: FailKind::Timeout, .. }) if *node == h.node
                );
                let mut added = Duration::ZERO;
                if lead_pin {
                    why.push("pin delay".into());
                    delayed = true;
                } else if !hang && !run.chaos.is_empty() {
                    let c = hop_chaos(run, key, h.kind, (h.node, h.replica));
                    // A cache lookup never emits a `Fail`: a hang is a hop charged the node's
                    // `timeout` (else HANG_CAP) with no latency, and a slow lookup is cut to the
                    // `timeout`. The request still succeeds (Z17) but a fault was injected (FI);
                    // a cut lookup without a hang fault says nothing about `latency_add`.
                    let timeout = run.topo.nodes[h.node].timeout;
                    let cache = matches!(h.kind, HopKind::CacheHit | HopKind::CacheMiss);
                    let at_cap = planned.dur == timeout.unwrap_or(HANG_CAP);
                    let hang_fault =
                        c.is_some_and(|c| c.faults().iter().any(|f| f.kind == ErrorKind::Hang));
                    if cache && at_cap && hang_fault {
                        why.push("chaos hang".into());
                        faulted = true;
                    } else if !(cache && at_cap && timeout.is_some()) {
                        // `h.dur`, not the plan's: a deadline may have cut the hop at run time.
                        added = c.map_or(Duration::ZERO, |c| c.latency_add).min(h.dur);
                    }
                }
                if !hang && !run.chaos.is_empty() && !lead_pin {
                    let (extra, note) = link::note(run, h.kind, h.node, h.dur);
                    added = added.max(extra);
                    if let Some(n) = note {
                        delayed = true;
                        why.push(n);
                    }
                }
                if !added.is_zero() {
                    delayed = true;
                    why.push(format!("chaos +{}ms", added.as_millis()));
                }
                if !fh.wait.is_zero() {
                    why.push(format!("queued {:.1}ms", ms(fh.wait)));
                }
                if recs.len() == MAX_HOPS {
                    truncated = true;
                    continue;
                }
                let top = forks.last();
                recs.push(HopRec {
                    node: name(h.node),
                    replica: (run.topo.nodes[h.node].replicas > 1).then_some(h.replica),
                    op: match h.kind {
                        HopKind::Op(op) => Some(op.name()),
                        _ => None,
                    },
                    kind: kind_str(h.kind),
                    start_ms: ms(h.start),
                    dur_ms: ms(h.dur),
                    wait_ms: ms(fh.wait),
                    lane: top.map_or(0, |f| f.2),
                    fork: top.map(|f| f.1),
                    cut: h.dur < planned.dur,
                    attempt: (fh.attempt > 1).then_some(fh.attempt),
                    why: (!why.is_empty()).then(|| why.join(", ")),
                });
            }
            PlanItem::Hop(_)
            | PlanItem::Fail { .. }
            | PlanItem::Span { .. }
            | PlanItem::Hold { .. }
            | PlanItem::Pick { .. }
            | PlanItem::Retry { .. }
            | PlanItem::Attempt { .. } => {}
        }
    }
    (recs, truncated, delayed, faulted)
}

/// The bus event of an entry; `nodes` is what the request touched.
pub fn event_of(e: &LogEntry, nodes: Vec<String>) -> Event {
    Event {
        id: e.id.clone(),
        client_id: e.client_id.clone(),
        seq: e.seq,
        ts_ms: e.ts_ms,
        method: e.method.clone(),
        path: e.path.clone(),
        endpoint: e.endpoint.clone(),
        status: e.status,
        ms: e.ms,
        nodes,
        failed_node: e.failed_node.clone(),
        pin: e.pin,
        kind: e.kind,
        scope: e.scope.clone(),
        rejected: e.rejected,
    }
}

mod link;

/// **Private.** The `kind` string of a hop in the log and the trace.
fn kind_str(kind: HopKind) -> &'static str {
    match kind {
        HopKind::Net => "net",
        HopKind::Connect { resp } => connect_name(resp),
        HopKind::FirstByte => "first-byte",
        HopKind::Retransmit { resp } => retransmit_name(resp),
        HopKind::Proc => "proc",
        HopKind::Op(_) => "op",
        HopKind::CacheHit => "hit",
        HopKind::CacheMiss => "miss",
        HopKind::Delay => "delay",
        HopKind::Link { resp, .. } => link_name(resp),
        HopKind::Nic { send, .. } => nic_name(send),
    }
}

/// **Private.** The name of a node's network pipe hop: `nic-out` (send) or `nic-in` (receive).
fn nic_name(send: bool) -> &'static str {
    if send { "nic-out" } else { "nic-in" }
}

#[cfg(test)]
mod tests;
