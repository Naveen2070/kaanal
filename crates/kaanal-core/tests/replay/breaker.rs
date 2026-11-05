//! The circuit breaker in the engine (RULES 82-84, step 7c). Plans are built by hand: a `Retry` scope
//! over the edge `gw -> app`, whose attempts ask the breaker of edge 0 for the caller replica.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause, Finished};
use kaanal_core::{
    BreakerRef, ChainResult, FailKind, HopKind, NodeId, PlanItem, RetryBudget, Topology,
};
use std::time::Duration;

/// `gw -> app` with a breaker that opens at 4 failures of 4, stays open 10 s and decides over 2 probes.
const YAML: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app, breaker: { window: 4, min_calls: 4, open_for: 10s, half_open_calls: 2 } }\n";
const MS_D: fn(u64) -> Duration = Duration::from_millis;
const SEC: u64 = 1000 * MS;

fn breaker(replica: u16) -> BreakerRef {
    BreakerRef { edge: 0, replica }
}

fn fail(node: NodeId, status: u16) -> PlanItem {
    PlanItem::Fail {
        node,
        status,
        kind: FailKind::Status,
        at: MS_D(1),
    }
}

/// One call through the breaker with `tries` attempts: the first `bad` of them fail with `status`, the
/// rest succeed; each attempt is one 1 ms hop, and a retry waits 5 ms.
fn call(app: NodeId, tries: u8, bad: u8, status: u16, replica: u16) -> Vec<PlanItem> {
    let mut items = Vec::new();
    for j in 0..tries {
        let mut a = vec![hop(app, HopKind::Proc, 1)];
        if j < bad {
            a.push(fail(app, status));
        }
        items.push(PlanItem::Attempt {
            backoff: if j == 0 { Duration::ZERO } else { MS_D(5) },
            size: a.len(),
        });
        items.extend(a);
    }
    let mut out = vec![PlanItem::Retry {
        attempts: tries,
        size: items.len(),
        budget: None,
        breaker: Some(breaker(replica)),
    }];
    out.extend(items);
    out
}

fn ok(app: NodeId) -> Vec<PlanItem> {
    call(app, 1, 0, 503, 0)
}

fn bad(app: NodeId) -> Vec<PlanItem> {
    call(app, 1, 1, 503, 0)
}

/// Submits each `(time ns, items)` and returns the engine and what finished.
fn go(t: &Topology, calls: Vec<(u64, Vec<PlanItem>)>) -> (Engine, Vec<Finished>) {
    let mut e = Engine::new(t);
    for (i, (at, items)) in calls.into_iter().enumerate() {
        e.submit(at, plan(items), i as u64).unwrap();
    }
    let f = run_all(&mut e);
    (e, f)
}

fn status(f: &Finished) -> Option<u16> {
    match f.result {
        ChainResult::Err { status, .. } => Some(status),
        ChainResult::Ok => None,
    }
}

#[test]
fn it_opens_after_min_calls_and_rejects_at_once_with_nothing_sent() {
    let top = topo(YAML);
    let app = top.id("app").unwrap();
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    calls.push((5 * SEC, ok(app)));
    let (eng, done) = go(&top, calls);
    for i in 0..4 {
        assert_eq!(status(fin(&done, i)), Some(503));
        assert_eq!(fin(&done, i).cause, Some(FailCause::Chaos));
    }
    let rej = fin(&done, 4);
    assert_eq!(status(rej), Some(503), "the open breaker answers 503");
    assert_eq!(rej.cause, Some(FailCause::Breaker));
    assert_eq!(rej.done, rej.arrival, "a rejection takes no time");
    assert_eq!(rej.hops, [], "and sends nothing");
    let ctr = &eng.node_counters()[app];
    assert_eq!(
        (ctr.completed, ctr.short_circuited),
        (4, 1),
        "the callee never saw the fifth"
    );
}

#[test]
fn http_4xx_does_not_count_as_a_failure() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let mut calls: Vec<_> = (0..8).map(|i| (i * SEC, call(app, 1, 1, 404, 0))).collect();
    calls.push((9 * SEC, ok(app)));
    let (_, f) = go(&t, calls);
    assert_eq!(status(fin(&f, 8)), None, "eight 404s did not open it");
}

#[test]
fn it_half_opens_after_open_for_and_closes_when_the_probes_succeed() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    calls.push((9 * SEC, ok(app))); // it opened at about 3.001 s, so it is open until 13.001 s
    calls.push((14 * SEC, ok(app))); // probe 1
    calls.push((14 * SEC + 10 * MS, ok(app))); // probe 2: closes
    calls.push((15 * SEC, bad(app))); // closed: reaches the callee
    calls.push((16 * SEC, ok(app)));
    let (_, f) = go(&t, calls);
    assert_eq!(fin(&f, 4).cause, Some(FailCause::Breaker));
    assert_eq!(status(fin(&f, 5)), None);
    assert_eq!(status(fin(&f, 6)), None);
    assert_eq!(
        fin(&f, 7).cause,
        Some(FailCause::Chaos),
        "closed again: it reached the callee"
    );
    assert_eq!(status(fin(&f, 8)), None);
}

#[test]
fn failed_probes_reopen_it_for_another_open_for() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    calls.push((14 * SEC, bad(app))); // probe 1 fails
    calls.push((14 * SEC + 10 * MS, bad(app))); // probe 2 fails: 100% >= 50%, reopens
    calls.push((20 * SEC, ok(app))); // open again until about 24 s
    calls.push((30 * SEC, ok(app))); // a probe again
    let (_, f) = go(&t, calls);
    assert_eq!(fin(&f, 6).cause, Some(FailCause::Breaker));
    assert_eq!(
        status(fin(&f, 7)),
        None,
        "the next round of probes got through"
    );
}

#[test]
fn a_retry_meeting_an_open_breaker_waits_its_backoff_and_sends_nothing() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    // three attempts, all rejected: each retry still waits 5 ms (it cannot know), nothing is sent
    calls.push((5 * SEC, call(app, 3, 0, 503, 0)));
    let (e, f) = go(&t, calls);
    let r = fin(&f, 4);
    assert_eq!(r.cause, Some(FailCause::Breaker));
    assert_eq!(
        r.done - r.arrival,
        2 * 5 * MS,
        "two backoffs of 5 ms, no work"
    );
    assert_eq!(r.hops, []);
    assert_eq!(
        e.node_counters()[app].short_circuited,
        3,
        "every attempt was rejected and counted"
    );
}

#[test]
fn a_rejected_attempt_takes_no_budget_token() {
    // 10 tokens, a failed attempt costs 1, a retry needs more than 5 left. Four failed calls cost 4;
    // the three rejected attempts of the fifth must cost nothing, so a later call that fails once
    // (6 tokens left, plus refills) still retries. Had each rejection cost a token, 3 more would be
    // gone (3 left) and it could not.
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let with_budget = |mut items: Vec<PlanItem>| {
        if let PlanItem::Retry { budget, .. } = &mut items[0] {
            *budget = Some(RetryBudget {
                edge: 0,
                replica: 0,
                max_milli: 10_000,
                ratio_milli: 100,
            });
        }
        items
    };
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, with_budget(bad(app)))).collect();
    calls.push((5 * SEC, with_budget(call(app, 3, 0, 503, 0))));
    calls.push((14 * SEC, with_budget(ok(app)))); // probe 1
    calls.push((14 * SEC + 10 * MS, with_budget(ok(app)))); // probe 2: closes
    calls.push((20 * SEC, with_budget(call(app, 2, 1, 503, 0))));
    let (_, f) = go(&t, calls);
    assert_eq!(
        status(fin(&f, 7)),
        None,
        "the retry was afforded: {:?}",
        fin(&f, 7)
    );
}

#[test]
fn an_attempt_with_no_items_is_rejected_and_retried_like_any_other() {
    // a callee that does no work (no hop, no wire) leaves an attempt of size 0; the breaker must still
    // reject it, and the scope must still wait its backoff and try again
    let top = topo(YAML);
    let app = top.id("app").unwrap();
    let empty = vec![
        PlanItem::Retry {
            attempts: 2,
            size: 2,
            budget: None,
            breaker: Some(breaker(0)),
        },
        PlanItem::Attempt {
            backoff: Duration::ZERO,
            size: 0,
        },
        PlanItem::Attempt {
            backoff: MS_D(5),
            size: 0,
        },
    ];
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    calls.push((5 * SEC, empty));
    let (eng, done) = go(&top, calls);
    let rej = fin(&done, 4);
    assert_eq!(rej.cause, Some(FailCause::Breaker));
    assert_eq!(
        rej.done - rej.arrival,
        5 * MS,
        "one backoff between two rejections"
    );
    assert_eq!(eng.node_counters()[app].short_circuited, 2);
}

#[test]
fn each_caller_replica_has_its_own_breaker() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let mut calls: Vec<_> = (0..4).map(|i| (i * SEC, bad(app))).collect();
    calls.push((5 * SEC, ok(app))); // replica 0: rejected
    calls.push((5 * SEC, call(app, 1, 0, 503, 1))); // replica 1: closed, passes
    let (_, f) = go(&t, calls);
    assert_eq!(fin(&f, 4).cause, Some(FailCause::Breaker));
    assert_eq!(status(fin(&f, 5)), None);
}

#[test]
fn a_closed_breaker_is_invisible_to_a_failing_call() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let (_, f) = go(&t, vec![(0, bad(app))]);
    assert_eq!(
        (status(&f[0]), f[0].cause),
        (Some(503), Some(FailCause::Chaos))
    );
    assert_eq!(f[0].done, ms(1));
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
fn the_share_reaching_the_callee_matches_the_duty_cycle() {
    // A callee that always fails, arrivals every 10 ms (lambda = 100/s), open_for T = 1 s, h = 5
    // probes. Each cycle lets h calls through and rejects lambda*T of them, so the fraction that
    // reaches the callee is h / (h + lambda T) = 5/105 (Nygard, Release It!; resilience4j half-open).
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app, breaker: { window: 10, min_calls: 10, open_for: 1s, half_open_calls: 5 } }\n";
    let top = topo(yaml);
    let app = top.id("app").unwrap();
    let count = 20_000u64;
    let calls = (0..count).map(|i| (i * 10 * MS, bad(app))).collect();
    let (eng, done) = go(&top, calls);
    assert_eq!(done.len() as u64, count);
    let ctr = &eng.node_counters()[app];
    let (passed, rejected) = (ctr.completed as f64, ctr.short_circuited as f64);
    assert_eq!(passed + rejected, count as f64);
    let want = 5.0 / 105.0;
    let got = passed / count as f64;
    assert!(
        (got - want).abs() / want < 0.05,
        "fraction reaching the callee {got:.4}, theory {want:.4}"
    );
}
