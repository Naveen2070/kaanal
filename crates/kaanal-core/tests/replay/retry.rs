//! Retries in the engine (RULES 82-84, step 7a): a transient failure inside an attempt ends that
//! attempt, not the request. Plans are built by hand: `scope` lays the attempts out with their sizes.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{
    ChainRequest, ChainResult, ChaosState, ErrorKind, FailKind, HopKind, NodeChaos, NodeId, Op,
    PlanItem, RetryBudget, execute,
};
use std::time::Duration;

const APP: &str = "version: 1\nnodes:\n  app: { type: service }\n";
const MS_D: fn(u64) -> Duration = Duration::from_millis;

/// A `Retry` scope: one `(backoff in ms, items)` per attempt.
fn scope(attempts: Vec<(u64, Vec<PlanItem>)>) -> Vec<PlanItem> {
    scope_with(None, attempts)
}

/// [`scope`] drawing on a retry budget.
fn scope_with(budget: Option<RetryBudget>, attempts: Vec<(u64, Vec<PlanItem>)>) -> Vec<PlanItem> {
    let body: usize = attempts.iter().map(|(_, v)| 1 + v.len()).sum();
    let mut out = vec![PlanItem::Retry {
        attempts: u8::try_from(attempts.len()).unwrap(),
        size: body,
        budget,
    }];
    for (wait, items) in attempts {
        out.push(PlanItem::Attempt {
            backoff: MS_D(wait),
            size: items.len(),
        });
        out.extend(items);
    }
    out
}

fn fail(node: NodeId, status: u16, kind: FailKind, at_ms: u64) -> PlanItem {
    PlanItem::Fail {
        node,
        status,
        kind,
        at: MS_D(at_ms),
    }
}

fn err(f: &kaanal_core::replay::Finished) -> (u16, FailKind) {
    match f.result {
        ChainResult::Err { status, kind, .. } => (status, kind),
        ChainResult::Ok => panic!("expected a failure: {f:?}"),
    }
}

#[test]
fn a_planned_failure_starts_the_next_attempt_after_its_backoff() {
    let t = topo(APP);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    let items = scope(vec![
        (
            0,
            vec![
                hop(app, HopKind::Proc, 5),
                fail(app, 503, FailKind::Status, 5),
            ],
        ),
        (20, vec![hop(app, HopKind::Proc, 7)]),
    ]);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].result, ChainResult::Ok, "the second attempt succeeded");
    assert_eq!(f[0].done, ms(32), "5 ms, the 20 ms backoff, then 7 ms");
    let hops: Vec<_> = f[0].hops.iter().map(|h| (h.hop.start, h.hop.dur)).collect();
    assert_eq!(hops, [(MS_D(0), MS_D(5)), (MS_D(25), MS_D(7))]);
}

#[test]
fn the_last_attempt_failing_fails_the_request_with_its_status() {
    let t = topo(APP);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    let items = scope(vec![
        (
            0,
            vec![
                hop(app, HopKind::Proc, 5),
                fail(app, 503, FailKind::Status, 5),
            ],
        ),
        (
            20,
            vec![
                hop(app, HopKind::Proc, 3),
                fail(app, 502, FailKind::Reset, 28),
            ],
        ),
    ]);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(
        err(&f[0]),
        (502, FailKind::Reset),
        "the last attempt's failure"
    );
    assert_eq!(f[0].done, ms(28));
    assert_eq!(f[0].cause, Some(FailCause::Chaos));
}

#[test]
fn a_failure_that_is_not_transient_fails_the_request_at_once() {
    for (status, kind) in [(404, FailKind::Status), (502, FailKind::Malformed)] {
        let t = topo(APP);
        let app = t.id("app").unwrap();
        let mut e = Engine::new(&t);
        let items = scope(vec![
            (
                0,
                vec![hop(app, HopKind::Proc, 5), fail(app, status, kind, 5)],
            ),
            (20, vec![hop(app, HopKind::Proc, 7)]),
        ]);
        e.submit(0, plan(items), 0).unwrap();
        let f = run_all(&mut e);
        assert_eq!(err(&f[0]), (status, kind));
        assert_eq!(f[0].done, ms(5), "no retry: it fails where it failed");
        assert_eq!(f[0].hops.len(), 1, "the second attempt never started");
    }
}

#[test]
fn a_rate_limited_hop_retries_and_waits_out_retry_after() {
    // one token a second: the first request takes it, the second is refused with a 429 and told to
    // wait about a second. Its backoff is only 10 ms, so a retry at 10 ms would be refused again and
    // fail: it succeeds because the wait is at least `Retry-After` (RFC 9110 10.2.3).
    let t = topo(LIMITED);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(gw, HopKind::Proc, 1)]), 0)
        .unwrap();
    let items = scope(vec![
        (0, vec![hop(gw, HopKind::Proc, 1)]),
        (10, vec![hop(gw, HopKind::Proc, 1)]),
    ]);
    e.submit(0, plan(items), 1).unwrap();
    let f = run_all(&mut e);
    let b = fin(&f, 1);
    assert_eq!(b.result, ChainResult::Ok);
    assert!(b.done >= ms(1000), "waited the Retry-After: {}", b.done);
    assert!(b.done < ms(1100));
    assert_eq!(
        e.node_counters()[gw].limited,
        1,
        "the refusal is counted once"
    );
}

#[test]
fn a_full_queue_retries_like_a_planned_failure() {
    // a database with one connection and no queue: a second request is refused while the first runs
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: service }\n  db: { type: database, ops: { read: 400ms, write: 400ms, scan: 400ms }, pool: { size: 1, queue: 0 } }\nedges:\n  - { from: gw, to: db }\n",
    );
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(db, HopKind::Op(Op::Read), 400)]), 0)
        .unwrap();
    let items = scope(vec![
        (0, vec![hop(db, HopKind::Op(Op::Read), 400)]),
        (450, vec![hop(db, HopKind::Op(Op::Read), 10)]),
    ]);
    e.submit(0, plan(items), 1).unwrap();
    let f = run_all(&mut e);
    let b = fin(&f, 1);
    assert_eq!(b.result, ChainResult::Ok);
    assert_eq!(
        b.done,
        ms(460),
        "refused at 0, retried at 450 when the connection was free"
    );
    assert_eq!(e.node_counters()[db].overflowed, 1);
}

#[test]
fn a_deadline_inside_the_scope_retries_one_outside_fails_the_request() {
    let t = topo(APP);
    let app = t.id("app").unwrap();
    // each attempt has its own 10 ms deadline; the first attempt's hop would take 50 ms
    let inside = |outer: Option<u64>| {
        let items = scope(vec![
            (0, vec![span(app, 10, 1), hop(app, HopKind::Proc, 50)]),
            (5, vec![span(app, 10, 1), hop(app, HopKind::Proc, 3)]),
        ]);
        match outer {
            None => items,
            Some(d) => {
                let mut v = vec![span(app, d, items.len())];
                v.extend(items);
                v
            }
        }
    };
    let mut e = Engine::new(&t);
    e.submit(0, plan(inside(None)), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].result, ChainResult::Ok);
    assert_eq!(
        f[0].done,
        ms(18),
        "timed out at 10, retried at 15, done at 18"
    );
    assert_eq!(
        f[0].hops[0].hop.dur,
        MS_D(10),
        "the first attempt is cut at its deadline"
    );
    assert_eq!(e.node_counters()[app].timed_out, 1);

    // an enclosing deadline of 12 ms falls during the backoff: that one fails the whole request
    let mut e = Engine::new(&t);
    e.submit(0, plan(inside(Some(12))), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(err(&f[0]), (504, FailKind::Timeout));
    assert_eq!(f[0].done, ms(12));
    assert_eq!(f[0].cause, Some(FailCause::Deadline));
    assert_eq!(f[0].hops.len(), 1, "the second attempt never started");
}

#[test]
fn a_held_slot_of_a_failed_attempt_is_released() {
    // one app slot held across a call: while attempt 0 holds it, request C waits; when attempt 0
    // fails, the slot goes to C at once, long before attempt 1 (30 ms of backoff) comes back for it
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: { limit: 1, queue: 5 }, hold: call }\n  db: { type: database, ops: { read: 40ms, write: 40ms, scan: 40ms } }\nedges:\n  - { from: app, to: db }\n",
    );
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    let held = |inner: Vec<PlanItem>| {
        let mut v = vec![PlanItem::Hold {
            node: app,
            replica: 0,
            size: inner.len(),
        }];
        v.extend(inner);
        v
    };
    let items = scope(vec![
        (
            0,
            held(vec![
                hop(app, HopKind::Proc, 10),
                fail(db, 503, FailKind::Status, 10),
            ]),
        ),
        (
            30,
            held(vec![
                hop(app, HopKind::Proc, 10),
                hop(db, HopKind::Op(Op::Read), 5),
            ]),
        ),
    ]);
    e.submit(0, plan(items), 0).unwrap();
    e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), 1)
        .unwrap();
    let f = run_all(&mut e);
    assert_eq!(
        fin(&f, 1).done,
        ms(20),
        "C took the slot at 10 ms, not after B's whole retry"
    );
    let b = fin(&f, 0);
    assert_eq!(b.result, ChainResult::Ok);
    assert_eq!(
        b.done,
        ms(55),
        "attempt 1 at 40 ms: 10 ms of work and 5 ms at the database"
    );
}

#[test]
fn a_failing_branch_cuts_the_whole_attempt_and_the_other_branch_stops() {
    let t = topo(APP);
    let app = t.id("app").unwrap();
    // branch A runs 50 ms, branch B fails at 5 ms: the attempt is cut at 5, A is a zombie
    let attempt0 = vec![
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 1 },
        hop(app, HopKind::Proc, 50),
        PlanItem::Branch { size: 2 },
        hop(app, HopKind::Proc, 5),
        fail(app, 503, FailKind::Status, 5),
    ];
    let items = scope(vec![(0, attempt0), (10, vec![hop(app, HopKind::Proc, 7)])]);
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].result, ChainResult::Ok);
    assert_eq!(f[0].done, ms(22), "cut at 5, backoff 10, then 7");
    let a = f[0]
        .hops
        .iter()
        .find(|h| h.hop.dur == MS_D(5) && h.hop.start == MS_D(0));
    assert!(
        a.is_some(),
        "a hop of the cut attempt is clipped to the cut: {:?}",
        f[0].hops
    );
    assert_eq!(f[0].hops.len(), 3);
}

#[test]
fn a_retry_inside_a_branch_reports_to_its_join() {
    // branch A retries once (done at 22 ms), branch B takes 30 ms: the block ends with B
    let t = topo(APP);
    let app = t.id("app").unwrap();
    let a = scope(vec![
        (
            0,
            vec![
                hop(app, HopKind::Proc, 5),
                fail(app, 503, FailKind::Status, 5),
            ],
        ),
        (10, vec![hop(app, HopKind::Proc, 7)]),
    ]);
    let mut items = vec![
        PlanItem::Fork {
            branches: 2,
            size: 0,
        },
        PlanItem::Branch { size: a.len() },
    ];
    items.extend(a);
    items.push(PlanItem::Branch { size: 1 });
    items.push(hop(app, HopKind::Proc, 30));
    let size = items.len() - 1;
    items[0] = PlanItem::Fork { branches: 2, size };
    items.push(hop(app, HopKind::Proc, 4));
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].result, ChainResult::Ok);
    assert_eq!(f[0].done, ms(34), "the block ends at 30 ms, then 4 ms more");
}

#[test]
fn offered_load_follows_the_geometric_series() {
    // gateway -> service -> db, retry on the first edge, the db failing at random with probability p:
    // each attempt reruns the service's work, so the service sees lambda x sum_{k<n} p^k (the retry
    // amplification). Requests are far apart, so nothing queues and `completed` counts the work done.
    for (p, tries) in [(0.5, 3u32), (0.3, 4)] {
        let cfg = format!(
            "version: 1\nheader_bytes: 0\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: gw, to: app, net: 1ms, retry: {{ max_attempts: {tries} }} }}\n  - {{ from: app, to: db, net: 1ms }}\n"
        );
        let t = topo(&cfg);
        let routes = [kaanal_core::chain::RouteSpec::inferred(
            "GET /users/{id}",
            t.id("app").unwrap(),
        )];
        let ep = kaanal_core::chain::compile(
            &t,
            &routes,
            kaanal_core::chain::EndpointsInput::inline(
                &kaanal_core::Config::from_yaml(&cfg).unwrap(),
            ),
        )
        .unwrap();
        let mut s = ChaosState::default();
        s.insert(
            t.id("db").unwrap(),
            NodeChaos {
                error_rate: p,
                error_kind: ErrorKind::Status,
                ..NodeChaos::default()
            },
        );
        let chaos = s.compile();
        let mut e = Engine::new(&t);
        let runs = 5000u64;
        for n in 0..runs {
            let (pl, _) = kaanal_core::sample_plan(
                &t,
                &chaos,
                &ChainRequest {
                    endpoint: ep.get("GET /users/{id}").unwrap(),
                    n,
                    seed: 7,
                    pin_delay: Duration::ZERO,
                    req_bytes: 0,
                    resp_bytes: 0,
                },
            );
            e.submit(n * 10_000 * MS, std::sync::Arc::new(pl.resolved()), n)
                .unwrap();
        }
        let _ = run_all(&mut e);
        let app = t.id("app").unwrap();
        #[allow(clippy::cast_precision_loss)]
        let got = e.node_counters()[app].completed as f64 / runs as f64;
        let want: f64 = (0..tries)
            .map(|k| f64::powi(p, i32::try_from(k).unwrap()))
            .sum();
        assert!(
            (got - want).abs() / want < 0.02,
            "p {p}, {tries} tries: the service did {got} units of work a request, wanted {want}"
        );
    }
    let _ = execute;
}

/// A budget of 10 tokens refilling 0.1 a success, on edge 0 for the given caller replica.
fn budget(replica: u16) -> RetryBudget {
    RetryBudget {
        edge: 0,
        replica,
        max_milli: 10_000,
        ratio_milli: 100,
    }
}

/// A call whose every one of `tries` attempts fails (an outage of its callee).
fn outage(app: NodeId, b: Option<RetryBudget>, tries: usize) -> Vec<PlanItem> {
    scope_with(
        b,
        (0..tries)
            .map(|_| {
                (
                    1,
                    vec![
                        hop(app, HopKind::Proc, 1),
                        fail(app, 503, FailKind::Status, 1),
                    ],
                )
            })
            .collect(),
    )
}

/// A call that succeeds at its first attempt (the others are in the plan, unused).
fn healthy(app: NodeId, b: Option<RetryBudget>) -> Vec<PlanItem> {
    scope_with(
        b,
        vec![
            (0, vec![hop(app, HopKind::Proc, 1)]),
            (1, vec![hop(app, HopKind::Proc, 1)]),
        ],
    )
}

/// Attempts (hops that ran) of each request, in submit order, one second apart.
fn attempts_of(t: &kaanal_core::Topology, plans: Vec<Vec<PlanItem>>) -> Vec<usize> {
    let mut e = Engine::new(t);
    for (i, p) in plans.into_iter().enumerate() {
        e.submit(i as u64 * 1000 * MS, plan(p), i as u64).unwrap();
    }
    let f = run_all(&mut e);
    (0..f.len() as u64).map(|i| fin(&f, i).hops.len()).collect()
}

#[test]
fn a_budget_caps_the_amplification_of_an_outage() {
    // gRPC A6: 10 tokens, a retry only while more than 5 are left, a failed attempt costs 1. In an
    // outage the first request spends the budget on its four retries (10 -> 5) and every later request
    // gets its one attempt: 5 + (N - 1) attempts for N requests, not 5 N.
    let t = topo(APP);
    let app = t.id("app").unwrap();
    let n = 40;
    let with = attempts_of(
        &t,
        (0..n).map(|_| outage(app, Some(budget(0)), 5)).collect(),
    );
    assert_eq!(with[0], 5);
    assert!(with[1..].iter().all(|a| *a == 1), "{with:?}");
    assert_eq!(with.iter().sum::<usize>(), 5 + (n - 1));
    let without = attempts_of(&t, (0..n).map(|_| outage(app, None, 5)).collect());
    assert_eq!(
        without.iter().sum::<usize>(),
        5 * n,
        "no budget: every request retries to the end"
    );
}

#[test]
fn the_budget_refills_by_the_token_ratio_per_success() {
    // drained to 0 by an outage, then S successes put S x 0.1 tokens back; a failing request costs 1
    // and may retry only if more than 5 are left: 0.1 S - 1 > 5, so S >= 61 and not S = 60.
    let t = topo(APP);
    let app = t.id("app").unwrap();
    for (s, retries) in [(60usize, false), (61, true)] {
        let mut plans: Vec<_> = (0..12).map(|_| outage(app, Some(budget(0)), 5)).collect();
        plans.extend((0..s).map(|_| healthy(app, Some(budget(0)))));
        plans.push(outage(app, Some(budget(0)), 5));
        let got = attempts_of(&t, plans);
        let last = *got.last().unwrap();
        assert_eq!(
            last > 1,
            retries,
            "after {s} successes the failing request made {last} attempts"
        );
    }
}

#[test]
fn each_caller_replica_has_its_own_budget() {
    // replica 0 drains its budget; replica 1 of the same caller still has all 10 tokens
    let t = topo(APP);
    let app = t.id("app").unwrap();
    let mut plans: Vec<_> = (0..12).map(|_| outage(app, Some(budget(0)), 5)).collect();
    plans.push(outage(app, Some(budget(1)), 5));
    let got = attempts_of(&t, plans);
    assert_eq!(got[11], 1, "replica 0 is throttled");
    assert_eq!(got[12], 5, "replica 1 retries freely");
}
