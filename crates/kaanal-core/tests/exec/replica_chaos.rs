//! Chaos on one replica: down replicas are skipped, latency and failures hit only that copy.
#![expect(
    clippy::many_single_char_names,
    reason = "t, e, s and n are the shorthand of every exec test: topology, endpoints, chaos state, request"
)]

use crate::common::*;
use kaanal_core::{ChainResult, ChaosKey, ChaosLayer, ChaosState, FailKind, Hop, PlanItem};
use std::time::Duration;

const KEY: &str = "GET /users/{id}";

fn cfg(db_fields: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: 5ms }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }}{db_fields} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\nendpoints:\n  \"{KEY}\": {{ chain: [ db ] }}\n"
    )
}

fn layer(f: impl FnOnce(&mut ChaosLayer)) -> ChaosLayer {
    let mut l = ChaosLayer::default();
    f(&mut l);
    l
}

fn down() -> ChaosLayer {
    layer(|l| l.down = Some(true))
}

fn latency(ms: u64) -> ChaosLayer {
    layer(|l| l.latency_add = Some(Duration::from_millis(ms)))
}

fn db_hops(
    t: &kaanal_core::Topology,
    e: &kaanal_core::Endpoints,
    c: &ChaosState,
    n: u64,
) -> Vec<Hop> {
    let db = t.id("db").unwrap();
    run(t, e, c, n)
        .plan
        .items
        .into_iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) if h.node == db && !h.kind.is_wait() => Some(h),
            _ => None,
        })
        .collect()
}

#[test]
fn a_replica_down_is_skipped_by_every_policy() {
    let policies = ["round-robin", "random", "{ weighted: [1, 1, 1] }"];
    for p in policies {
        let (t, e) = build(&cfg(&format!(", replicas: 3, balance: {p}")));
        let db = t.id("db").unwrap();
        let mut s = ChaosState::default();
        s.set(ChaosKey::Replica(db, 1), down());
        let mut seen = std::collections::BTreeSet::new();
        for n in 0..300 {
            let h = db_hops(&t, &e, &s, n);
            assert_eq!(
                h.len(),
                1,
                "{p}: the request still succeeds on a live replica"
            );
            seen.insert(h[0].replica);
        }
        assert_eq!(seen, [0, 2].into(), "{p}");
    }
}

#[test]
fn every_replica_down_fails_with_503() {
    let (t, e) = build(&cfg(", replicas: 3"));
    let db = t.id("db").unwrap();
    let mut s = ChaosState::default();
    for r in 0..3 {
        s.set(ChaosKey::Replica(db, r), down());
    }
    for n in 0..20 {
        let out = run(&t, &e, &s, n);
        match out.result {
            ChainResult::Err {
                node, status, kind, ..
            } => {
                assert_eq!((node, status, kind), (db, 503, FailKind::Status), "n = {n}");
            }
            ChainResult::Ok => panic!("every replica is down, n = {n}"),
        }
        assert!(
            out.hops.iter().all(|h| h.node != db),
            "the call never starts: no hop on db, n = {n}"
        );
    }
}

#[test]
fn a_node_down_takes_every_replica_down_with_it() {
    let (t, e) = build(&cfg(", replicas: 2"));
    let mut s = ChaosState::default();
    s.set(ChaosKey::Node(t.id("db").unwrap()), down());
    assert!(matches!(
        run(&t, &e, &s, 0).result,
        ChainResult::Err { status: 503, .. }
    ));
}

#[test]
fn replica_latency_chaos_hits_only_that_replica() {
    let (t, e) = build(&cfg(", replicas: 3"));
    let db = t.id("db").unwrap();
    let calm = ChaosState::default();
    let mut slow = ChaosState::default();
    slow.set(ChaosKey::Replica(db, 2), latency(30));
    for n in 0..60 {
        let (a, b) = (db_hops(&t, &e, &calm, n), db_hops(&t, &e, &slow, n));
        assert_eq!((a.len(), b.len()), (1, 1));
        assert_eq!(a[0].replica, b[0].replica, "chaos does not move the pick");
        let extra = b[0].dur.checked_sub(a[0].dur).unwrap();
        let want = if b[0].replica == 2 { 30 } else { 0 };
        assert_eq!(
            extra,
            Duration::from_millis(want),
            "n = {n}, replica {}",
            b[0].replica
        );
    }
}

#[test]
fn a_replica_entry_inherits_the_node_entry() {
    let (t, e) = build(&cfg(", replicas: 2"));
    let db = t.id("db").unwrap();
    let calm = ChaosState::default();
    let mut s = ChaosState::default();
    s.set(ChaosKey::Node(db), latency(10));
    s.set(ChaosKey::Replica(db, 0), latency(5));
    for n in 0..40 {
        let (a, b) = (db_hops(&t, &e, &calm, n), db_hops(&t, &e, &s, n));
        let want = if b[0].replica == 0 { 15 } else { 10 };
        assert_eq!(
            b[0].dur.checked_sub(a[0].dur).unwrap(),
            Duration::from_millis(want),
            "n = {n}"
        );
    }
}

#[test]
fn a_replica_entry_also_folds_the_node_at_endpoint_entry() {
    let (t, e) = build(&cfg(", replicas: 2"));
    let db = t.id("db").unwrap();
    let calm = ChaosState::default();
    let mut s = ChaosState::default();
    s.set(ChaosKey::NodeAt(db, KEY.into()), latency(7));
    s.set(ChaosKey::Replica(db, 1), latency(3));
    for n in 0..40 {
        let (a, b) = (db_hops(&t, &e, &calm, n), db_hops(&t, &e, &s, n));
        let want = if b[0].replica == 1 { 10 } else { 7 };
        assert_eq!(
            b[0].dur.checked_sub(a[0].dur).unwrap(),
            Duration::from_millis(want),
            "n = {n}"
        );
    }
}

#[test]
fn a_replica_layer_can_be_read_listed_and_removed() {
    let mut s = ChaosState::default();
    assert!(s.is_empty());
    s.set(ChaosKey::Replica(3, 1), down());
    assert!(!s.is_empty() && s.entry_count() == 1);
    assert!(s.layer(&ChaosKey::Replica(3, 1)).is_some());
    assert!(s.layer(&ChaosKey::Replica(3, 0)).is_none());
    assert_eq!(s.entries().len(), 1);
    assert!(!s.compile().is_empty());
    assert!(s.remove(&ChaosKey::Replica(3, 1)));
    assert!(!s.remove(&ChaosKey::Replica(3, 1)));
    assert!(s.is_empty());
}
