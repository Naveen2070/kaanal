//! `detect` (step 7d, RULES 91): a balancer keeps sending to a copy that has gone down until it notices,
//! `detect` after the outage began. Until then the call fails fast at the copy (a refused connection, the
//! existing down roll); after, the copy is skipped. A copy that is down before the run is noticed already.
//! Round-robin gives exact shares: a third of the requests land on one copy of three.

use crate::common::*;
use kaanal_core::{
    ChainRequest, ChainResult, ChaosKey, ChaosLayer, ChaosState, Endpoints, Hop, PlanItem,
    Topology, execute,
};
use std::time::Duration;

const KEY: &str = "GET /users/{id}";
const SEC: Duration = Duration::from_secs(1);

/// `app -> db` (three copies, round-robin); `edge` is extra fields on that edge.
fn cfg(edge: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: 5ms }}\n  db: {{ type: database, replicas: 3, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms{edge} }}\nendpoints:\n  \"{KEY}\": {{ chain: [ db ] }}\n"
    )
}

/// Copy 1 of `db` is down, from `from` (none = before the run).
fn outage(t: &Topology, from: Option<Duration>) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Replica(t.id("db").unwrap(), 1),
        ChaosLayer {
            down: Some(true),
            down_from: from,
            ..ChaosLayer::default()
        },
    );
    s
}

/// Request `n` arriving at `at`.
fn one(
    t: &Topology,
    e: &Endpoints,
    s: &ChaosState,
    at: Duration,
    n: u64,
) -> kaanal_core::ChainOutcome {
    execute(
        t,
        &s.compile(),
        &ChainRequest {
            at,
            endpoint: e.get(KEY).unwrap(),
            n,
            seed: 42,
            pin_delay: Duration::ZERO,
            req_bytes: 0,
            resp_bytes: 0,
        },
    )
}

/// `(failed, hops on copy 1)` over requests 0..300 all arriving at `at`.
fn phase(t: &Topology, e: &Endpoints, s: &ChaosState, at: Duration) -> (usize, usize) {
    let db = t.id("db").unwrap();
    let (mut failed, mut on_one) = (0, 0);
    for n in 0..300 {
        let out = one(t, e, s, at, n);
        failed += usize::from(!matches!(out.result, ChainResult::Ok));
        on_one += out
            .plan
            .items
            .iter()
            .filter(|i| matches!(i, PlanItem::Hop(Hop { node, replica: 1, kind, .. }) if *node == db && !kind.is_wait()))
            .count();
    }
    (failed, on_one)
}

#[test]
fn a_copy_serves_normally_until_it_goes_down() {
    let (t, e) = build(&cfg(", detect: 5s"));
    let s = outage(&t, Some(10 * SEC));
    assert_eq!(
        phase(&t, &e, &s, 9 * SEC),
        (0, 100),
        "before the onset it takes its third"
    );
}

#[test]
fn until_noticed_the_balancer_still_sends_a_third_and_they_fail_fast() {
    let (t, e) = build(&cfg(", detect: 5s"));
    let s = outage(&t, Some(10 * SEC));
    for at in [10 * SEC, 12 * SEC, Duration::from_millis(14_999)] {
        let (failed, on_one) = phase(&t, &e, &s, at);
        assert_eq!(
            failed, 100,
            "at {at:?}: a third of the calls reach the dead copy and fail"
        );
        assert_eq!(on_one, 0, "a refused call does no work on the copy");
    }
    // fast, and transient: a 503 reset after the wire, so a retry or a breaker can react
    let out = one(&t, &e, &s, 10 * SEC, 1); // request 1 lands on copy 1
    assert!(out.total() < Duration::from_millis(50), "{:?}", out.total());
}

#[test]
fn once_noticed_the_copy_is_skipped() {
    let (t, e) = build(&cfg(", detect: 5s"));
    let s = outage(&t, Some(10 * SEC));
    for at in [15 * SEC, 60 * SEC] {
        assert_eq!(phase(&t, &e, &s, at), (0, 0), "at {at:?}");
    }
}

#[test]
fn detect_zero_skips_the_copy_from_the_instant_it_goes_down() {
    let (t, e) = build(&cfg(""));
    let s = outage(&t, Some(10 * SEC));
    assert_eq!(phase(&t, &e, &s, 9 * SEC), (0, 100));
    assert_eq!(
        phase(&t, &e, &s, 10 * SEC),
        (0, 0),
        "no delay: skipped at once, as before"
    );
}

#[test]
fn a_copy_down_before_the_run_is_already_noticed() {
    let (t, e) = build(&cfg(", detect: 5s"));
    let s = outage(&t, None);
    assert_eq!(phase(&t, &e, &s, Duration::ZERO), (0, 0));
}

#[test]
fn a_load_balancers_detect_applies_to_the_copies_behind_it() {
    // app -> lb -> db: the lb's `detect` is the health delay of the pool it fronts
    let y = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 5ms }\n  lb: { type: load-balancer, detect: 5s }\n  db: { type: database, replicas: 3, ops: { read: 4ms, write: 8ms, scan: 30ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: lb, net: 1ms }\n  - { from: lb, to: db, net: 1ms }\nendpoints:\n  \"GET /users/{id}\": { chain: [ lb ] }\n";
    let (t, e) = build(y);
    let s = outage(&t, Some(10 * SEC));
    let during = phase(&t, &e, &s, 12 * SEC);
    assert_eq!(
        during.0, 100,
        "the lb still sends a third to the dead copy: {during:?}"
    );
    assert_eq!(phase(&t, &e, &s, 15 * SEC), (0, 0));
}

#[test]
fn an_edges_detect_wins_over_the_balancers() {
    let y = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 5ms }\n  lb: { type: load-balancer, detect: 60s }\n  db: { type: database, replicas: 3, ops: { read: 4ms, write: 8ms, scan: 30ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: lb, net: 1ms }\n  - { from: lb, to: db, net: 1ms, detect: 1s }\nendpoints:\n  \"GET /users/{id}\": { chain: [ lb ] }\n";
    let (t, e) = build(y);
    let s = outage(&t, Some(10 * SEC));
    assert_eq!(
        phase(&t, &e, &s, 12 * SEC),
        (0, 0),
        "the edge says 1s, not the lb's 60s"
    );
}

#[test]
fn a_bad_detect_and_a_bad_from_name_their_field() {
    for bad in ["detect: 2h", "detect: soon"] {
        let e = kaanal_core::Config::from_yaml(&cfg(&format!(", {bad}"))).unwrap_err();
        assert!(e.to_string().contains("detect"), "{bad}: {e}");
    }
    let sc = |entry: &str| {
        kaanal_core::parse_scenario(&format!("chaos:\n  - {{ {entry} }}\n"))
            .map_err(|e| e.to_string())
            .and_then(|s| kaanal_core::scenario::validate_scenario_shape(&s).map_err(|e| e.message))
    };
    assert!(sc("node: db, replica: 1, down: true, from_ms: 10000").is_ok());
    for bad in [
        "node: db, error_rate: 0.5, from_ms: 10000",
        "edge: { from: app, to: db }, down: true, from_ms: 10000",
        "node: db, endpoint: 'GET /x', down: true, from_ms: 10000",
    ] {
        let e = sc(bad).unwrap_err();
        assert!(e.contains("from_ms"), "{bad}: {e}");
    }
}
