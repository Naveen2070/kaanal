//! Calls and the load balancer's pick, split out of `walk.rs`: one `call:` step run as a method,
//! and the choice of a callee for a [`Step::Route`].

use std::time::Duration;

use rand::Rng;

use super::retry::{backoff, jitter, retryable};
use super::walk::{Flow, Walk, draw_rng, step_path, takes_branch};
use super::{BreakerRef, Op, PlanItem, RetryBudget, ScopedFail, scope};
use crate::balance::pick_replica;
use crate::chain::{Call, Route};
use crate::chaos::FailKind;
use crate::config::{NodeKind, RetryConfig};
use crate::stable_hash;
use crate::topology::NodeId;

impl Walk<'_> {
    /// **Private.** Runs one `call:` step from `*now`: the network hop, the callee's body, the
    /// response, `repeat` times in sequence. Moves `*now` to the end; `Some` is the failure that
    /// ended the request.
    pub(super) fn call_step(
        &mut self,
        call: &Call,
        path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        // The call carries its own sizes when it has them (`sends`, `returns`); the callee's body
        // and its own calls see them, and the caller's go back once the call is over.
        let saved = self.bytes;
        self.bytes = (
            call.sends.unwrap_or(saved.0),
            call.returns.unwrap_or(saved.1),
        );
        let flow = self.call_inner(call, path, op, now, run);
        self.bytes = saved;
        flow
    }

    /// **Private.** [`Walk::call_step`] with the call's sizes already in `bytes`.
    fn call_inner(
        &mut self,
        call: &Call,
        path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        let topo = self.topo;
        let target = topo.nodes[call.node].name.as_str();
        // One draw per step: a skipped call has no hop, span or repeat.
        if !takes_branch(call.prob, path, target, call.key.occ) {
            return None;
        }
        let op = call.op.unwrap_or(op);
        for rep in 0..call.repeat {
            let rep_path = step_path(path, target, call.key.occ, rep);
            let flow = match self.retry_of(call, op) {
                Some(rc) => self.attempts(call, rc, rep_path, op, now, run),
                // a breaker with no retry: a scope of one attempt, which draws what the call draws
                None if call.breaker.is_some() => {
                    let once = RetryConfig {
                        max_attempts: 1,
                        ..RetryConfig::default()
                    };
                    self.attempts(call, &once, rep_path, op, now, run)
                }
                None => self.try_once(call, rep_path, op, now, run),
            };
            if flow.is_some() {
                return flow;
            }
        }
        None
    }

    /// **Private.** One try of a call from `*now`: the replica pick, the wire, the callee's body and the
    /// way back (everything of one `repeat` of a call). Moves `*now` to the end; `Some` is the failure.
    #[inline]
    fn try_once(
        &mut self,
        call: &Call,
        rep_path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        let topo = self.topo;
        let chaos = self.chaos;
        let Some(r) = self.pick(call.node, call.balance.as_ref(), rep_path) else {
            return Some(self.no_replica(call.node, *now));
        };
        let span = self.open_span(call.node, *now, call.orphan);
        let edge = topo.edge_id(run.0, call.node);
        let lc = edge.and_then(|e| chaos.link(e));
        let cap = lc.and_then(|l| l.bandwidth_cap);
        if let Some(k) = lc.and_then(|l| self.link_drop(l, call.node, rep_path)) {
            let fail = self.link_fail(k, call.node, r, *now);
            self.close_span(span);
            return Some(fail);
        }
        let net = self.net_sample(call.node, &call.net, rep_path, lc);
        let mut wire = self.wire((edge, cap), net, ((call.node, r), run), rep_path);
        *now += self.wire_connect(&mut wire, *now);
        // an upload streams into the callee, so the request's transfer runs beside it (RULES 76)
        let mode = self.stream_mode(call, op);
        let upload = mode.request() && self.can_stream(edge, (run.0, call.node), false, cap);
        if !upload {
            *now += self.wire_send(&mut wire, *now);
        }
        *now += self.net_hop(call.node, net, (*now, r));
        let up = upload.then(|| self.begin_upload(&mut wire, *now));
        let stream = (mode.response() && self.can_stream(edge, (call.node, run.0), true, cap))
            .then(|| self.begin_stream());
        let t0 = *now;
        let hold = self.open_hold(call.node, r);
        let flow = self.seq(&call.body, rep_path, op, *now, (call.node, r));
        self.close_hold(hold);
        let done = if let Some(fork) = stream {
            self.end_stream(fork, &flow, &mut wire, t0)
        } else if let Flow::Done(end) | Flow::Hit(end) = flow {
            Some(end + self.wire_back(&mut wire, end))
        } else {
            None
        };
        let sent = up.as_ref().map(|u| u.sent);
        if let Some(u) = &up {
            self.end_upload(u);
        }
        if let Some(end) = done {
            *now = end.max(sent.unwrap_or(Duration::ZERO));
            *now += self.wire_stalls(&mut wire, *now);
        }
        self.close_span(span);
        match flow {
            Flow::Done(_) | Flow::Hit(_) => {}
            fail @ Flow::Fail { .. } => return Some(fail),
        }
        None
    }

    /// **Private.** The retry policy that applies to `call` under `op`, if any: a call that has `retry`,
    /// except a write to a database or service, which a retry could apply twice (RFC 9110 section 9.2.2)
    /// and so needs `retry.writes`. A write to a `storage` node is idempotent (an S3 PUT or DELETE) and
    /// always qualifies (RULES 84).
    #[inline]
    fn retry_of<'c>(&self, call: &'c Call, op: Op) -> Option<&'c RetryConfig> {
        let rc = call.retry.as_ref()?;
        (op != Op::Write || rc.writes || self.topo.nodes[call.node].kind == NodeKind::Storage)
            .then_some(rc)
    }

    /// **Private.** The retry budget and the circuit breaker a call's scope draws on, both kept by the
    /// engine per `(edge, caller replica)`.
    fn scope_refs(
        &self,
        call: &Call,
        rc: &RetryConfig,
        run: (NodeId, u16),
    ) -> (Option<RetryBudget>, Option<BreakerRef>) {
        let edge = self
            .topo
            .edge_id(run.0, call.node)
            .and_then(|e| u16::try_from(e).ok());
        let budget = rc.budget.and_then(|b| {
            Some(RetryBudget {
                edge: edge?,
                replica: run.1,
                max_milli: b.max_tokens * 1000,
                ratio_milli: milli(b.token_ratio),
            })
        });
        let breaker = call.breaker.and_then(|_| {
            Some(BreakerRef {
                edge: edge?,
                replica: run.1,
            })
        });
        (budget, breaker)
    }

    /// **Private.** One `repeat` of a call that may be retried (RULES 82-84): a [`PlanItem::Retry`] holding
    /// up to `rc.max_attempts` [`PlanItem::Attempt`]s, each its own try with its own draws (the first
    /// attempt draws exactly what the call would without `retry`, so a call that succeeds first time
    /// costs the plan only the two markers).
    ///
    /// **Every attempt is sampled**, also after the first success, because the replay engine may need
    /// them when load fails an attempt the sampler thought fine (a full queue, a rate limit, a
    /// deadline). Sampling stops early only after a failure that is not transient, which no engine would
    /// retry either. The sampler itself decides each attempt at zero load: the first success settles
    /// the call (later attempts are marked unreached), a transient failure goes on to the next attempt
    /// after its backoff, and the last attempt's failure (or a non-transient one) is the call's.
    /// Decisions of failed attempts are written to [`Plan::scoped`](super::Plan::scoped).
    /// Moves `*now` to the end of the settling attempt; `Some` is the failure that ended the call.
    #[inline(never)]
    fn attempts(
        &mut self,
        call: &Call,
        rc: &RetryConfig,
        rep_path: u64,
        op: Op,
        now: &mut Duration,
        run: (NodeId, u16),
    ) -> Option<Flow> {
        let name = self.topo.nodes[call.node].name.as_str();
        // one stream of jitter draws, a draw per retry whether or not it is reached
        let mut jitter_rng = draw_rng(rep_path, name, b"retry");
        let retry = self.items.len();
        let (budget, breaker) = self.scope_refs(call, rc, run);
        self.items.push(PlanItem::Retry {
            attempts: rc.max_attempts,
            size: 0,
            budget,
            breaker,
        });
        let (mut ready, mut emitted) = (*now, 0u8);
        // the first success (its end), else the failure that ended the call
        let mut settled: Option<Duration> = None;
        let mut ended: Option<Flow> = None;
        let mut tail = self.tail;
        // the replica counters as the settling attempt left them: attempts sampled after it never ran, so
        // they must not move what the rest of the request picks
        let mut counters: Option<(Vec<u32>, Vec<u32>)> = None;
        self.retried = true;
        for j in 0..rc.max_attempts {
            let wait = if j == 0 {
                Duration::ZERO
            } else {
                let u: f64 = jitter_rng.random();
                jitter(
                    backoff(rc.backoff, rc.multiplier, rc.cap, u32::from(j) - 1),
                    rc.jitter,
                    u,
                )
            };
            let marker = self.items.len();
            self.items.push(PlanItem::Attempt {
                backoff: wait,
                size: 0,
            });
            emitted += 1;
            let mut end = ready + wait;
            let path = if j == 0 {
                rep_path
            } else {
                stable_hash(&[&rep_path.to_le_bytes(), b"retry", &[j]])
            };
            let flow = self.try_once(call, path, op, &mut end, run);
            let size = self.items.len() - marker - 1;
            self.items[marker] = PlanItem::Attempt {
                backoff: wait,
                size,
            };
            if settled.is_some() {
                continue; // sampled for the engine only
            }
            let planned = match flow {
                Some(Flow::Fail { at, idx }) => Some((idx, at)),
                _ => None,
            };
            let decided = scope::deadline_in(
                &self.items,
                &self.scoped,
                marker + 1..self.items.len(),
                planned.map(|(_, t)| t),
            )
            .or(planned);
            tail = self.tail;
            let Some((idx, at)) = decided else {
                settled = Some(end);
                counters = Some((self.picks.clone(), self.routes.clone()));
                continue;
            };
            self.scoped.push(ScopedFail {
                attempt: marker,
                item: idx,
                at,
            });
            ended = Some(Flow::Fail { at, idx });
            let (status, kind) = match self.items[idx] {
                PlanItem::Fail { status, kind, .. } => (status, kind),
                _ => (504, FailKind::Timeout), // a span's deadline
            };
            if !retryable(status, kind) {
                break;
            }
            ready = at;
        }
        self.items[retry] = PlanItem::Retry {
            attempts: emitted,
            size: self.items.len() - retry - 1,
            budget,
            breaker,
        };
        self.tail = tail;
        if let Some(c) = counters {
            (self.picks, self.routes) = c;
        }
        if let Some(end) = settled {
            *now = end;
            return None;
        }
        ended
    }

    /// **Private.** The wire out of a balancer a call takes now: the pure [`pick_replica`] over
    /// its callees by `spread`, a callee with every copy down skipped. The round-robin counter
    /// is the balancer's own (a balancer with copies keeps its replica picks apart), and the
    /// draw is tagged with its own name so it does not repeat the replica pick. `None` when every
    /// callee is down.
    pub(super) fn pick_callee<'c>(&mut self, r: &'c Route, path: u64) -> Option<&'c Call> {
        let lb = &self.topo.nodes[r.node];
        let k = u64::from(self.routes[r.node]);
        self.routes[r.node] += 1;
        let tag = format!("{}#spread", lb.name);
        let offset = stable_hash(&[self.ep_key.as_bytes(), tag.as_bytes()]).wrapping_add(k);
        let up: Vec<bool> = r
            .calls
            .iter()
            .map(|c| {
                self.chaos.is_empty()
                    || (0..self.topo.nodes[c.node].replicas)
                        .any(|rep| !self.chaos.replica_down(c.node, self.ep_key, rep))
            })
            .collect();
        let count = u16::try_from(r.calls.len()).unwrap_or(u16::MAX);
        // each copy of the balancer round-robins over only the calls it gets, one in `replicas`;
        // counting requests in step with the copy pick would lock a callee to a copy
        let n = self.n / u64::from(lb.replicas.max(1));
        let i = pick_replica(&r.spread, count, &up, n, offset, path, &tag)?;
        r.calls.get(usize::from(i))
    }
}

/// **Private.** A `token_ratio` in thousandths of a token, 1 to 1000 (the config allows above 0, at most 1).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 1..=1000"
)]
fn milli(ratio: f64) -> u16 {
    (ratio * 1000.0).round().clamp(1.0, 1000.0) as u16
}
