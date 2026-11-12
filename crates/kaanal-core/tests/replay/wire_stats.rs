//! What the engine counts per wire for a retry scope (calls, retries, recovered, failed) and the state
//! changes of a circuit breaker, for the Studio's Results (step 7f). Plans are built by hand, as in
//! `breaker.rs`: a `Retry` scope over the edge `gw -> app` (edge 0).
//!
//! Definitions (each asserted below): a *call* is one entry into a scope; *retried* is a call that made
//! at least one retry; *retries* counts every retry attempt; *recovered* is a retried call that ended
//! well; *failed* is a call whose last attempt failed (retried or not).

use crate::common::*;
use kaanal_core::replay::{BreakerState, Engine};
use kaanal_core::{BreakerRef, FailKind, HopKind, NodeId, PlanItem, Topology};
use std::time::Duration;

const YAML: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app, breaker: { window: 4, min_calls: 4, open_for: 10s, half_open_calls: 2 } }\n";
const MS_D: fn(u64) -> Duration = Duration::from_millis;
const SEC: u64 = 1000 * MS;

/// One call with `tries` attempts: the first `bad` of them fail with `status`, the rest succeed.
fn call(app: NodeId, tries: u8, bad: u8, status: u16, breaker: bool) -> Vec<PlanItem> {
    let mut items = Vec::new();
    for j in 0..tries {
        let mut a = vec![hop(app, HopKind::Proc, 1)];
        if j < bad {
            a.push(PlanItem::Fail {
                node: app,
                status,
                kind: FailKind::Status,
                at: MS_D(1),
            });
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
        breaker: breaker.then_some(BreakerRef {
            edge: 0,
            replica: 0,
        }),
        edge: 0,
    }];
    out.extend(items);
    out
}

fn go(t: &Topology, warmup: u64, calls: Vec<(u64, Vec<PlanItem>)>) -> Engine {
    let mut e = Engine::new(t);
    e.set_window(warmup, u64::MAX);
    for (i, (at, items)) in calls.into_iter().enumerate() {
        e.submit(at, plan(items), i as u64).unwrap();
    }
    run_all(&mut e);
    e
}

fn counts(e: &Engine) -> (u64, u64, u64, u64, u64) {
    let w = &e.wire_counters()[0];
    (w.calls, w.retried, w.retries, w.recovered, w.failed)
}

#[test]
fn a_call_that_succeeds_first_time_is_one_call_and_nothing_else() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(&t, 0, vec![(0, call(app, 3, 0, 503, false))]);
    assert_eq!(counts(&e), (1, 0, 0, 0, 0));
}

#[test]
fn a_call_that_fails_once_then_succeeds_is_retried_and_recovered() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(&t, 0, vec![(0, call(app, 3, 1, 503, false))]);
    assert_eq!(counts(&e), (1, 1, 1, 1, 0));
}

#[test]
fn a_call_that_fails_every_attempt_is_retried_twice_and_gives_up() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(&t, 0, vec![(0, call(app, 3, 3, 503, false))]);
    assert_eq!(counts(&e), (1, 1, 2, 0, 1));
}

#[test]
fn a_failure_that_is_not_transient_is_failed_without_a_retry() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(&t, 0, vec![(0, call(app, 3, 1, 404, false))]);
    assert_eq!(counts(&e), (1, 0, 0, 0, 1));
}

#[test]
fn a_request_before_the_warmup_is_not_counted() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(
        &t,
        5 * SEC,
        vec![
            (0, call(app, 3, 3, 503, false)),
            (6 * SEC, call(app, 3, 1, 503, false)),
        ],
    );
    assert_eq!(
        counts(&e),
        (1, 1, 1, 1, 0),
        "only the second is in the window"
    );
}

#[test]
fn the_breaker_logs_open_then_half_open_then_closed() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    // four failed calls open it at about 3 s; after open_for (10 s) two probes succeed and close it
    let mut calls: Vec<_> = (0..4)
        .map(|i| (i * SEC, call(app, 1, 1, 503, true)))
        .collect();
    calls.push((14 * SEC, call(app, 1, 0, 503, true))); // probe 1: half-open
    calls.push((14 * SEC + 10 * MS, call(app, 1, 0, 503, true))); // probe 2: closes
    let e = go(&t, 0, calls);
    let log = e.breaker_changes();
    assert_eq!(
        log.iter().map(|c| c.state).collect::<Vec<_>>(),
        [
            BreakerState::Open,
            BreakerState::HalfOpen,
            BreakerState::Closed
        ]
    );
    assert!(log.iter().all(|c| c.edge == 0 && c.replica == 0));
    assert!(
        log[0].at >= 3 * SEC && log[0].at < 4 * SEC,
        "opened after the fourth failure"
    );
    assert_eq!(
        log[1].at,
        14 * SEC,
        "half-open when the first probe arrives"
    );
    assert!(log[2].at > log[1].at);
}

#[test]
fn a_wire_with_no_breaker_logs_no_change() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let e = go(&t, 0, vec![(0, call(app, 3, 3, 503, false))]);
    assert_eq!(e.breaker_changes(), []);
}

fn finished_of(t: &Topology, items: Vec<PlanItem>) -> kaanal_core::replay::Finished {
    let mut e = Engine::new(t);
    e.submit(0, plan(items), 0).unwrap();
    run_all(&mut e).remove(0)
}

#[test]
fn a_finished_hop_says_which_attempt_ran_it() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    // attempt 1 fails, attempt 2 fails, attempt 3 succeeds: one proc hop each
    let fin = finished_of(&t, call(app, 3, 2, 503, false));
    let tries: Vec<u8> = fin.hops.iter().map(|h| h.attempt).collect();
    assert_eq!(tries, vec![1, 2, 3]);
}

#[test]
fn a_hop_outside_any_retry_is_attempt_one() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    let fin = finished_of(&t, call(app, 3, 0, 503, false));
    assert_eq!(
        fin.hops.iter().map(|h| h.attempt).collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn breaker_phases_read_what_each_breaker_is_doing_now() {
    let t = topo(YAML);
    let app = t.id("app").unwrap();
    assert!(
        go(&t, 0, vec![]).breaker_phases().is_empty(),
        "no call, no breaker"
    );
    // four failed calls open it and nothing has asked since
    let open = go(
        &t,
        0,
        (0..4)
            .map(|i| (i * SEC, call(app, 1, 1, 503, true)))
            .collect(),
    );
    assert_eq!(open.breaker_phases(), [(0, 0, BreakerState::Open)]);
    // the same plus two good probes after open_for: closed again
    let mut calls: Vec<_> = (0..4)
        .map(|i| (i * SEC, call(app, 1, 1, 503, true)))
        .collect();
    calls.push((14 * SEC, call(app, 1, 0, 503, true)));
    calls.push((14 * SEC + 10 * MS, call(app, 1, 0, 503, true)));
    assert_eq!(
        go(&t, 0, calls).breaker_phases(),
        [(0, 0, BreakerState::Closed)]
    );
}
