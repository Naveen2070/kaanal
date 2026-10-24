//! Endpoint and node-at-endpoint chaos, cache timeouts and hangs.

use crate::common::*;
use kaanal_core::{
    ChainResult, ChaosKey, ChaosLayer, ChaosState, ErrorKind, FailKind, HANG_CAP, HopKind,
    NodeChaos, PlanItem,
};
use std::time::Duration;

#[test]
fn endpoint_chaos_failure_belongs_to_the_entry_node_at_offset_zero() {
    let (t, e) = topo();
    let mut c = ChaosState::default();
    c.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(1.0),
            error_status: Some(502),
            ..Default::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    let ChainResult::Err { node, status, .. } = &out.result else {
        panic!()
    };
    assert_eq!((*node, *status), (t.id("nginx").unwrap(), 502));
    assert_eq!(out.hops.len(), 0);
    let other = run_key(&t, &e, "GET /health", &c, 0);
    assert!(
        matches!(other.result, ChainResult::Ok),
        "other endpoints are untouched"
    );
}

#[test]
fn endpoint_latency_is_a_delay_hop_on_the_entry_node() {
    let (t, e) = topo();
    let mut c = ChaosState::default();
    c.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(300)),
            ..Default::default()
        },
    );
    let base = run(&t, &e, &ChaosState::default(), 0);
    let out = run(&t, &e, &c, 0);
    assert_eq!(out.hops[0].kind, HopKind::Delay);
    assert_eq!(out.hops[0].node, t.id("nginx").unwrap());
    assert_eq!(out.total(), base.total() + Duration::from_millis(300));
}

#[test]
fn node_at_chaos_only_hits_its_endpoint() {
    let (t, e) = topo();
    let db = t.id("db").unwrap();
    let mut c = ChaosState::default();
    c.set(
        ChaosKey::NodeAt(db, "GET /users/{id}".into()),
        ChaosLayer {
            down: Some(true),
            ..Default::default()
        },
    );
    assert!(matches!(run(&t, &e, &c, 0).result, ChainResult::Err { .. }));
    let mut c2 = ChaosState::default();
    c2.set(
        ChaosKey::NodeAt(db, "GET /other".into()),
        ChaosLayer {
            down: Some(true),
            ..Default::default()
        },
    );
    assert!(matches!(run(&t, &e, &c2, 0).result, ChainResult::Ok));
}

#[test]
fn cache_timeout_is_a_capped_miss() {
    // a cache with a miss/hit cost above its timeout is a miss charged exactly the timeout
    let src = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  c: { type: cache, hit_ratio: 1.0, hit: 50ms, miss: 50ms, timeout: 10ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: c, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n";
    let (t, e) = build(src);
    let out = run(&t, &e, &ChaosState::default(), 0);
    assert!(
        matches!(out.result, ChainResult::Ok),
        "a cache never fails the request"
    );
    let c = out
        .hops
        .iter()
        .find(|h| h.node == t.id("c").unwrap() && matches!(h.kind, HopKind::CacheMiss))
        .unwrap();
    assert_eq!(c.dur, Duration::from_millis(10));
}

#[test]
fn hang_on_a_cache_is_a_miss_charged_the_cap() {
    let src = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  c: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 1ms, timeout: 7ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: c, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n";
    let (t, e) = build(src);
    let c = chaos_on(
        &t,
        "c",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    assert!(matches!(out.result, ChainResult::Ok));
    let m = out
        .hops
        .iter()
        .find(|h| matches!(h.kind, HopKind::CacheMiss))
        .unwrap();
    assert_eq!(m.dur, Duration::from_millis(7));
}

#[test]
fn execute_ignores_limits_with_spans() {
    // review focus 10: one request alone never contends with itself at zero load
    let src = BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 5s, rate_limit: { rps: 1, burst: 1 }, concurrency: 1,",
    );
    let (t, e) = build(&src);
    assert!(matches!(
        run(&t, &e, &ChaosState::default(), 0).result,
        ChainResult::Ok
    ));
}

#[test]
fn hang_plans_a_capped_own_hop_then_a_timeout_fail() {
    // plan-level: the hang hop is a normal own hop of the node (a slot holder), then a Fail
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &chaos, 0);
    let at = out
        .plan
        .items
        .iter()
        .position(|it| {
            matches!(
                it,
                PlanItem::Fail {
                    kind: FailKind::Timeout,
                    ..
                }
            )
        })
        .unwrap();
    assert!(
        matches!(&out.plan.items[at - 1], PlanItem::Hop(h) if h.kind != HopKind::Net && h.dur == HANG_CAP)
    );
}

#[test]
fn endpoint_chaos_down_reset_and_malformed_fail_at_offset_zero() {
    let (t, e) = topo();
    for (layer, want, status) in [
        (
            ChaosLayer {
                down: Some(true),
                ..Default::default()
            },
            FailKind::Status,
            503,
        ),
        (
            ChaosLayer {
                error_rate: Some(1.0),
                error_kind: Some(ErrorKind::Reset),
                ..Default::default()
            },
            FailKind::Reset,
            502,
        ),
        (
            ChaosLayer {
                error_rate: Some(1.0),
                error_kind: Some(ErrorKind::Malformed),
                ..Default::default()
            },
            FailKind::Malformed,
            502,
        ),
    ] {
        let mut c = ChaosState::default();
        c.set(ChaosKey::Endpoint("GET /users/{id}".into()), layer);
        let out = run(&t, &e, &c, 0);
        let nginx = t.id("nginx").unwrap();
        assert_eq!(
            out.plan.items,
            vec![PlanItem::Fail {
                node: nginx,
                status,
                kind: want,
                at: Duration::ZERO
            }]
        );
        assert!(matches!(out.result, ChainResult::Err { node, .. } if node == nginx));
        assert_eq!(out.total(), Duration::ZERO);
    }
}

#[test]
fn endpoint_chaos_hang_is_a_delay_hop_of_the_cap_then_a_504_at_the_cap() {
    let (t, e) = topo();
    let mut c = ChaosState::default();
    c.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(1.0),
            error_kind: Some(ErrorKind::Hang),
            ..Default::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    let nginx = t.id("nginx").unwrap();
    assert_eq!(
        out.plan.items,
        vec![
            PlanItem::Hop(kaanal_core::Hop {
                node: nginx,
                kind: HopKind::Delay,
                start: Duration::ZERO,
                dur: HANG_CAP
            }),
            PlanItem::Fail {
                node: nginx,
                status: 504,
                kind: FailKind::Timeout,
                at: HANG_CAP
            },
        ]
    );
    assert_eq!(out.total(), HANG_CAP);
}
