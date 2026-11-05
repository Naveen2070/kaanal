//! The pin delay: plan shifts, spans, failures and the sweep.

use crate::common::*;
use kaanal_core::{
    ChainRequest, ChaosKey, ChaosLayer, ErrorKind, HopKind, PlanItem, execute, sample_plan,
};
use std::time::Duration;

pub(crate) fn req(ep: &kaanal_core::Endpoint, n: u64, seed: u64, d: Duration) -> ChainRequest<'_> {
    ChainRequest {
        at: std::time::Duration::ZERO,
        endpoint: ep,
        n,
        seed,
        pin_delay: d,
        req_bytes: 0,
        resp_bytes: 0,
    }
}

pub(crate) fn chaos_ep(key: &str, kind: ErrorKind) -> kaanal_core::EffectiveChaos {
    let mut s = kaanal_core::ChaosState::default();
    s.set(
        ChaosKey::Endpoint(key.into()),
        ChaosLayer {
            error_rate: Some(1.0),
            error_kind: Some(kind),
            ..ChaosLayer::default()
        },
    );
    s.compile()
}

#[test]
fn pin_delay_zero_plan_is_identical() {
    let (t, e) = build(BASE);
    // chaos on the front door exercises the failure, hang and latency branches
    let mut state = kaanal_core::ChaosState::default();
    state.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(0.4),
            error_kind: Some(ErrorKind::Hang),
            latency_add: Some(Duration::from_millis(7)),
            ..ChaosLayer::default()
        },
    );
    state.set(
        ChaosKey::Node(t.id("db").unwrap()),
        ChaosLayer {
            error_rate: Some(0.3),
            ..ChaosLayer::default()
        },
    );
    let chaos = [
        kaanal_core::ChaosState::default().compile(),
        state.compile(),
    ];
    let mut hung = 0;
    for key in ["GET /users/{id}", "GET /health", "HEAD /users/{id}"] {
        let ep = e.get(key).unwrap();
        for c in &chaos {
            for seed in 0..40 {
                for n in 0..5 {
                    let (zero, zf) = sample_plan(&t, c, &req(ep, n, seed, Duration::ZERO));
                    let (with, wf) = sample_plan(&t, c, &req(ep, n, seed, secs(3)));
                    // the delayed plan is the zero plan shifted, behind one leading hop; the
                    // zero plan is today's plan, so a pin adds no draw and moves nothing else
                    assert_eq!(
                        with.items[1..],
                        shifted(&zero.items, secs(3))[..],
                        "{key} {seed} {n}"
                    );
                    assert_eq!(wf, zf.map(|(i, at)| (i + 1, at + secs(3))));
                    hung += usize::from(zf.is_some());
                }
            }
        }
    }
    assert!(hung > 0, "the sweep must reach a planned failure");
    // with no front-door chaos the zero plan has no delay hop at all
    let ep = e.get("GET /users/{id}").unwrap();
    let r = req(ep, 3, 42, Duration::ZERO);
    let (plan, _) = sample_plan(&t, &chaos[0], &r);
    assert!(
        plan.items
            .iter()
            .all(|i| !matches!(i, PlanItem::Hop(h) if h.kind == HopKind::Delay))
    );
    assert_eq!(execute(&t, &chaos[0], &r).plan, plan);
}

pub(crate) fn shifted(items: &[PlanItem], d: Duration) -> Vec<PlanItem> {
    items
        .iter()
        .map(|i| match i {
            PlanItem::Hop(h) => PlanItem::Hop(kaanal_core::Hop {
                start: h.start + d,
                ..h.clone()
            }),
            PlanItem::Fail {
                node,
                status,
                kind,
                at,
            } => PlanItem::Fail {
                node: *node,
                status: *status,
                kind: *kind,
                at: *at + d,
            },
            PlanItem::Span {
                node,
                timeout,
                at,
                size,
                orphan,
            } => PlanItem::Span {
                node: *node,
                timeout: *timeout,
                at: *at + d,
                size: *size,
                orphan: *orphan,
            },
            other => other.clone(),
        })
        .collect()
}

#[test]
fn pin_delay_shifts_everything_without_new_draws() {
    let (t, e) = build(BASE);
    let ep = e.get("GET /users/{id}").unwrap();
    let c = kaanal_core::ChaosState::default().compile();
    for seed in 0..20 {
        let base = sample_plan(&t, &c, &req(ep, 1, seed, Duration::ZERO)).0;
        let with = sample_plan(&t, &c, &req(ep, 1, seed, secs(10))).0;
        let PlanItem::Hop(first) = &with.items[0] else {
            panic!("{:?}", with.items[0])
        };
        assert_eq!(
            (first.kind, first.start, first.dur),
            (HopKind::Delay, Duration::ZERO, secs(10))
        );
        assert_eq!(first.node, ep.entry_node.unwrap());
        assert_eq!(with.items[1..], shifted(&base.items, secs(10))[..]);
    }
}

#[test]
fn pin_delay_shifts_the_chain_and_stays_outside_the_timeout_span() {
    let src = BASE.replace(
        "nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }",
        "nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }",
    );
    let (t, e) = build(&src);
    let ep = e.get("GET /users/{id}").unwrap();
    let c = kaanal_core::ChaosState::default().compile();
    let r = req(ep, 0, 7, secs(10));
    let (plan, planned) = sample_plan(&t, &c, &r);
    assert!(planned.is_none());
    // the delay hop comes before the root span, which opens at 10 s
    assert!(
        matches!(&plan.items[0], PlanItem::Hop(h) if h.kind == HopKind::Delay && h.dur == secs(10))
    );
    let PlanItem::Span { at, timeout, .. } = plan.items[1] else {
        panic!("{:?}", plan.items[1])
    };
    assert_eq!((at, timeout), (secs(10), secs(5)));
    let out = execute(&t, &c, &r);
    assert_eq!(
        out.result,
        kaanal_core::ChainResult::Ok,
        "no deadline failure"
    );
    assert!(out.fail.is_none());
    let chain_start = out
        .hops
        .iter()
        .filter(|h| h.kind != HopKind::Delay)
        .map(|h| h.start)
        .min()
        .unwrap();
    assert_eq!(chain_start, secs(10));
    assert!(out.total() > secs(10) && out.total() < secs(11));
}

#[test]
fn pin_delay_survives_endpoint_failure_and_hang() {
    let src = BASE.replace(
        "nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }",
        "nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }",
    );
    let (t, e) = build(&src);
    let ep = e.get("GET /users/{id}").unwrap();
    let key = "GET /users/{id}";

    let fail = chaos_ep(key, ErrorKind::Status);
    let out = execute(&t, &fail, &req(ep, 0, 1, secs(10)));
    assert!(
        matches!(
            out.result,
            kaanal_core::ChainResult::Err { status: 503, .. }
        ),
        "{:?}",
        out.result
    );
    let Some(PlanItem::Fail { at, .. }) = out.fail.map(|i| &out.plan.items[i]) else {
        panic!()
    };
    assert_eq!(*at, secs(10));
    assert_eq!(out.total(), secs(10));

    let hang = chaos_ep(key, ErrorKind::Hang);
    let out = execute(&t, &hang, &req(ep, 0, 1, secs(10)));
    // the hang lasts the gateway timeout (5 s) and starts after the pin delay
    let hops: Vec<_> = out.hops.iter().map(|h| (h.kind, h.start, h.dur)).collect();
    assert_eq!(hops[0], (HopKind::Delay, Duration::ZERO, secs(10)));
    assert_eq!(hops[1], (HopKind::Delay, secs(10), secs(5)));
    assert!(
        matches!(
            out.result,
            kaanal_core::ChainResult::Err { status: 504, .. }
        ),
        "{:?}",
        out.result
    );
    let Some(PlanItem::Fail { at, .. }) = out.fail.map(|i| &out.plan.items[i]) else {
        panic!()
    };
    assert_eq!(*at, secs(15));
}

#[test]
fn endpoint_latency_hop_starts_after_the_pin_delay() {
    let (t, e) = build(BASE);
    let ep = e.get("GET /users/{id}").unwrap();
    let mut s = kaanal_core::ChaosState::default();
    s.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(50)),
            ..ChaosLayer::default()
        },
    );
    let (plan, _) = sample_plan(&t, &s.compile(), &req(ep, 0, 3, secs(2)));
    let delays: Vec<_> = plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) if h.kind == HopKind::Delay => Some((h.start, h.dur)),
            _ => None,
        })
        .collect();
    assert_eq!(
        delays,
        [
            (Duration::ZERO, secs(2)),
            (secs(2), Duration::from_millis(50))
        ]
    );
    let first_chain = plan
        .items
        .iter()
        .find_map(|i| match i {
            PlanItem::Hop(h) if h.kind != HopKind::Delay => Some(h.start),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_chain, secs(2) + Duration::from_millis(50));
}

/// Gateway and db with timeouts, a cache, and a `parallel` block: the constructs the pin delay
/// must shift (spans, forks and branches, cache hits, hangs, deadline cuts).
pub(crate) const RICH: &str = r#"
version: 1
nodes:
  nginx: { type: gateway, timeout: 5s, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
  c:     { type: cache, hit_ratio: 0.5, hit: 1ms, miss: 1ms }
  db:    { type: database, timeout: 12ms, ops: { read: { p50: 8ms, p99: 30ms }, write: 8ms, scan: 30ms } }
  db2:   { type: database, ops: { read: 6ms, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: c, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: app, to: db2, net: 1ms }
endpoints:
  "GET /users/{id}":
    chain: [ { cache: c }, { parallel: [ db, db2 ] } ]
"#;

#[test]
fn pin_delay_sweep_covers_spans_forks_and_cache() {
    let (t, e) = build(RICH);
    let ep = e.get("GET /users/{id}").unwrap();
    let mut front = kaanal_core::ChaosState::default();
    front.set(
        ChaosKey::Endpoint("GET /users/{id}".into()),
        ChaosLayer {
            error_rate: Some(0.2),
            error_kind: Some(ErrorKind::Hang),
            latency_add: Some(Duration::from_millis(3)),
            ..ChaosLayer::default()
        },
    );
    let mut node = kaanal_core::ChaosState::default();
    node.set(
        ChaosKey::Node(t.id("db").unwrap()),
        ChaosLayer {
            error_rate: Some(0.3),
            error_kind: Some(ErrorKind::Hang),
            ..ChaosLayer::default()
        },
    );
    node.set(
        ChaosKey::Node(t.id("db2").unwrap()),
        ChaosLayer {
            error_rate: Some(0.2),
            ..ChaosLayer::default()
        },
    );
    let chaos = [
        kaanal_core::ChaosState::default().compile(),
        front.compile(),
        node.compile(),
    ];
    let (mut spans, mut forks, mut branches, mut hits) = (0, 0, 0, 0);
    let (mut gw_cuts, mut db_cuts) = (0, 0);
    for c in &chaos {
        for seed in 0..60 {
            for n in 0..5 {
                let (zero, zf) = sample_plan(&t, c, &req(ep, n, seed, Duration::ZERO));
                let (with, wf) = sample_plan(&t, c, &req(ep, n, seed, secs(3)));
                assert_eq!(
                    with.items[1..],
                    shifted(&zero.items, secs(3))[..],
                    "{seed} {n}"
                );
                assert_eq!(wf, zf.map(|(i, at)| (i + 1, at + secs(3))));
                // whole-plan equality with what `execute` plans at zero delay
                let r = req(ep, n, seed, Duration::ZERO);
                assert_eq!(execute(&t, c, &r).plan, zero);
                for i in &zero.items {
                    match i {
                        PlanItem::Span { .. } => spans += 1,
                        PlanItem::Fork { .. } => forks += 1,
                        PlanItem::Branch { .. } => branches += 1,
                        PlanItem::Hop(h) if h.kind == HopKind::CacheHit => hits += 1,
                        _ => {}
                    }
                }
                // a span deadline can fire at run time; it must also fire (or not) alike
                let a = execute(&t, c, &req(ep, n, seed, Duration::ZERO));
                let b = execute(&t, c, &req(ep, n, seed, secs(3)));
                assert_eq!(a.result, b.result, "{seed} {n}");
                if let kaanal_core::ChainResult::Err {
                    node, status: 504, ..
                } = a.result
                {
                    if node == t.id("nginx").unwrap() {
                        gw_cuts += 1;
                    } else if node == t.id("db").unwrap() {
                        db_cuts += 1;
                    }
                }
            }
        }
    }
    assert!(
        spans > 0 && forks > 0 && branches > 0 && hits > 0,
        "{spans} {forks} {branches} {hits}"
    );
    assert!(
        gw_cuts > 0 && db_cuts > 0,
        "gateway timeouts {gw_cuts}, db timeouts {db_cuts}"
    );
}
