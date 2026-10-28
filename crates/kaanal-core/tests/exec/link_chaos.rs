//! Chaos on a link in the plan: partition, drops, jitter, extra latency and a bandwidth cap.

use crate::common::*;
use kaanal_core::{
    ChainRequest, ChainResult, ChaosKey, ChaosLayer, ChaosState, ErrorKind, FailKind, Hop, HopKind,
    PlanItem, Topology, execute,
};
use std::time::Duration;

/// `app -> db` is edge 1; `db_f` and `edge_f` are extra fields on `db` and on that edge.
fn cfg(db_f: &str, edge_f: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: 5ms }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }}{db_f} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms{edge_f} }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: [ db ] }}\n"
    )
}

fn on_edge(layer: ChaosLayer) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(ChaosKey::Edge(1), layer);
    s
}

fn plan(
    t: &Topology,
    e: &kaanal_core::Endpoints,
    c: &ChaosState,
    n: u64,
    bytes: u64,
) -> kaanal_core::ChainOutcome {
    execute(
        t,
        &c.compile(),
        &ChainRequest {
            endpoint: e.get("GET /users/{id}").unwrap(),
            n,
            seed: 42,
            pin_delay: Duration::ZERO,
            req_bytes: bytes,
            resp_bytes: 0,
        },
    )
}

fn hops(o: &kaanal_core::ChainOutcome) -> Vec<Hop> {
    o.plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h.clone()),
            _ => None,
        })
        .collect()
}

/// The `Net` hops, in order: nginx to app, then app to db.
fn nets(o: &kaanal_core::ChainOutcome) -> Vec<Duration> {
    hops(o)
        .iter()
        .filter(|h| h.kind == HopKind::Net)
        .map(|h| h.dur)
        .collect()
}

fn down() -> ChaosLayer {
    ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    }
}

#[test]
fn partition_waits_the_callee_timeout_then_504() {
    let (t, e) = build(&cfg(", timeout: 200ms", ""));
    let o = plan(&t, &e, &on_edge(down()), 0, 0);
    let db = t.id("db").unwrap();
    assert_eq!(
        o.result,
        ChainResult::Err {
            node: db,
            status: 504,
            kind: FailKind::Timeout,
            retry_after: None
        }
    );
    assert_eq!(
        *nets(&o).last().unwrap(),
        ms(200),
        "the call waits its callee's timeout"
    );
    // without a timeout it waits the hang cap (1 minute)
    let (t, e) = build(&cfg("", ""));
    let o = plan(&t, &e, &on_edge(down()), 0, 0);
    assert_eq!(*nets(&o).last().unwrap(), Duration::from_secs(60));
    assert!(matches!(o.result, ChainResult::Err { status: 504, .. }));
}

#[test]
fn link_drop_is_a_502_reset() {
    let (t, e) = build(&cfg("", ""));
    let c = on_edge(ChaosLayer {
        error_rate: Some(1.0),
        ..ChaosLayer::default()
    });
    let o = plan(&t, &e, &c, 0, 0);
    let db = t.id("db").unwrap();
    assert_eq!(
        o.result,
        ChainResult::Err {
            node: db,
            status: 502,
            kind: FailKind::Reset,
            retry_after: None
        }
    );
    // it fails at the call: no net hop to the db, no op
    assert_eq!(nets(&o).len(), 1);
    assert!(!hops(&o).iter().any(|h| matches!(h.kind, HopKind::Op(_))));
    // a hang kind waits instead
    let c = on_edge(ChaosLayer {
        error_rate: Some(1.0),
        error_kind: Some(ErrorKind::Hang),
        ..ChaosLayer::default()
    });
    assert!(matches!(
        plan(&t, &e, &c, 0, 0).result,
        ChainResult::Err { status: 504, .. }
    ));
}

#[test]
fn a_drop_rate_hits_about_its_share() {
    let (t, e) = build(&cfg("", ""));
    let c = on_edge(ChaosLayer {
        error_rate: Some(0.3),
        ..ChaosLayer::default()
    });
    let hit = (0..2000)
        .filter(|n| {
            matches!(
                plan(&t, &e, &c, *n, 0).result,
                ChainResult::Err { status: 502, .. }
            )
        })
        .count();
    assert!((500..700).contains(&hit), "{hit} of 2000 at 0.3");
}

#[test]
fn jitter_is_bounded_and_deterministic() {
    let (t, e) = build(&cfg("", ""));
    let c = on_edge(ChaosLayer {
        jitter: Some(Duration::from_millis(5)),
        ..ChaosLayer::default()
    });
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..200 {
        let a = nets(&plan(&t, &e, &c, n, 0));
        assert_eq!(
            a,
            nets(&plan(&t, &e, &c, n, 0)),
            "same request, same jitter"
        );
        assert_eq!(a[0], ms(1), "only the jittered edge moves");
        assert!(a[1] <= ms(6), "1 ms plus at most 5 ms: {:?}", a[1]);
        seen.insert(a[1]);
    }
    assert!(seen.len() > 50, "it really varies: {}", seen.len());
    assert!(
        seen.iter().any(|d| *d < ms(1)),
        "and goes below the base too"
    );
}

#[test]
fn latency_add_extends_the_net_hop() {
    let (t, e) = build(&cfg("", ""));
    let c = on_edge(ChaosLayer {
        latency_add: Some(Duration::from_millis(20)),
        ..ChaosLayer::default()
    });
    assert_eq!(nets(&plan(&t, &e, &c, 0, 0)), [ms(1), ms(21)]);
}

#[test]
fn a_bandwidth_cap_lengthens_the_transfer() {
    let (t, e) = build(&cfg("", ", bandwidth: 1000000"));
    let link = |c: &ChaosState| {
        hops(&plan(&t, &e, c, 0, 10_000))
            .iter()
            .find(|h| matches!(h.kind, HopKind::Link { .. }))
            .unwrap()
            .dur
    };
    assert_eq!(link(&ChaosState::default()), ms(10));
    let cap = |bps| {
        on_edge(ChaosLayer {
            bandwidth_cap: Some(bps),
            ..ChaosLayer::default()
        })
    };
    assert_eq!(
        link(&cap(500_000)),
        ms(20),
        "half the bandwidth, twice the time"
    );
    assert_eq!(link(&cap(2_000_000)), ms(10), "a cap never raises it");
}

#[test]
fn link_chaos_off_changes_no_plan() {
    let (t, e) = build(&cfg("", ""));
    let off = ChaosState::default();
    // an inert edge entry, and chaos on another key, plan exactly like none
    let inert = on_edge(ChaosLayer::default());
    let mut other = ChaosState::default();
    other.set(ChaosKey::Edge(9), down());
    for n in 0..50 {
        let base = plan(&t, &e, &off, n, 0).plan;
        assert_eq!(plan(&t, &e, &inert, n, 0).plan, base, "n = {n}");
        // no call crosses edge 9, so its partition changes nothing
        assert_eq!(plan(&t, &e, &other, n, 0).plan, base, "n = {n}");
    }
}

#[test]
fn a_dropped_link_to_a_cache_is_a_miss_not_a_failure() {
    let src = "version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service }\n  c: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 2ms }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: c }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { cache: c } ] }\n";
    let (t, e) = build(src);
    let kinds = |c: &ChaosState| -> Vec<HopKind> {
        let o = plan(&t, &e, c, 0, 0);
        assert_eq!(o.result, ChainResult::Ok, "a cache never fails the request");
        hops(&o)
            .iter()
            .map(|h| h.kind)
            .filter(|k| matches!(k, HopKind::CacheHit | HopKind::CacheMiss))
            .collect()
    };
    assert_eq!(kinds(&ChaosState::default()), [HopKind::CacheHit]);
    // partition: a miss charged the hang cap; reset: a miss at once
    let part = on_edge(down());
    assert_eq!(kinds(&part), [HopKind::CacheMiss]);
    let o = plan(&t, &e, &part, 0, 0);
    assert!(
        hops(&o)
            .iter()
            .any(|h| h.kind == HopKind::CacheMiss && h.dur == Duration::from_secs(60))
    );
}
