//! Replicas in the plan: a call binds one replica of its callee for the whole subtree.

use crate::common::*;
use kaanal_core::{ChainResult, ChaosState, Hop, HopKind, PlanItem};

/// A gateway, `app` and `db`; `app_f` and `db_f` are extra node fields, `edge_f` extra fields on
/// the app-to-db edge, and `chain` the route's chain.
fn cfg(gw_f: &str, app_f: &str, db_f: &str, edge_f: &str, chain: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms{gw_f} }}\n  app: {{ type: service, proc: {{ p50: 5ms, p99: 20ms }}{app_f} }}\n  db: {{ type: database, ops: {{ read: {{ p50: 4ms, p99: 20ms }}, write: 8ms, scan: 30ms }}{db_f} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms{edge_f} }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: {chain} }}\n"
    )
}

fn hops(t: &kaanal_core::Topology, e: &kaanal_core::Endpoints, n: u64) -> Vec<Hop> {
    run(t, e, &ChaosState::default(), n)
        .plan
        .items
        .into_iter()
        .filter_map(|i| match i {
            kaanal_core::PlanItem::Hop(h) => Some(h),
            _ => None,
        })
        .collect()
}

fn of(h: &[Hop], t: &kaanal_core::Topology, node: &str, kind: HopKind) -> Vec<u16> {
    let id = t.id(node).unwrap();
    h.iter()
        .filter(|h| h.node == id && h.kind == kind)
        .map(|h| h.replica)
        .collect()
}

#[test]
fn replicas_1_changes_no_plan_and_every_hop_is_replica_0() {
    let (t0, e0) = build(&cfg("", "", "", "", "[ db ]"));
    let (t1, e1) = build(&cfg(
        "",
        ", replicas: 1, balance: random",
        ", replicas: 1",
        "",
        "[ db ]",
    ));
    for n in 0..100 {
        let (a, b) = (hops(&t0, &e0, n), hops(&t1, &e1, n));
        assert_eq!(a, b, "n = {n}");
        assert!(a.iter().all(|h| h.replica == 0));
    }
}

#[test]
fn a_call_binds_its_whole_subtree_to_one_replica() {
    let (t, e) = build(&cfg("", ", replicas: 3, balance: random", "", "", "[ db ]"));
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..200 {
        let h = hops(&t, &e, n);
        let net = of(&h, &t, "app", HopKind::Net);
        let proc_ = of(&h, &t, "app", HopKind::Proc);
        assert_eq!((net.len(), proc_.len()), (1, 1));
        assert_eq!(
            net, proc_,
            "the net hop into app and app's own work share a replica"
        );
        assert!(net[0] < 3);
        seen.insert(net[0]);
        // db has one replica
        assert_eq!(of(&h, &t, "db", HopKind::Op(kaanal_core::Op::Read)), [0]);
    }
    assert_eq!(seen.len(), 3, "random reaches every replica: {seen:?}");
}

#[test]
fn two_calls_to_one_node_advance_round_robin() {
    let (t, e) = build(&cfg("", "", ", replicas: 3", "", "[ db, db ]"));
    let firsts: Vec<(u16, u16)> = (0..6)
        .map(|n| {
            let r = of(&hops(&t, &e, n), &t, "db", HopKind::Net);
            assert_eq!(r.len(), 2);
            (r[0], r[1])
        })
        .collect();
    for (a, b) in &firsts {
        assert_eq!(*b, (a + 1) % 3, "the second call takes the next replica");
    }
    for w in firsts.windows(2) {
        assert_eq!(
            w[1].0,
            (w[0].0 + 1) % 3,
            "the next request starts one further on"
        );
    }
}

#[test]
fn the_replica_never_changes_service_times() {
    let (t0, e0) = build(&cfg("", "", "", "", "[ db, db ]"));
    let (t1, e1) = build(&cfg(
        "",
        ", replicas: 3, balance: random",
        ", replicas: 4",
        "",
        "[ db, db ]",
    ));
    let times = |t, e, n| {
        hops(t, e, n)
            .into_iter()
            .map(|h| (h.node, h.kind, h.start, h.dur))
            .collect::<Vec<_>>()
    };
    for n in 0..100 {
        assert_eq!(times(&t0, &e0, n), times(&t1, &e1, n), "n = {n}");
    }
}

#[test]
fn an_entry_node_with_replicas_picks_without_a_calling_edge() {
    let (t, e) = build(&cfg(", replicas: 3", "", "", "", "[ db ]"));
    let reps: Vec<u16> = (0..6)
        .map(|n| {
            let r = of(&hops(&t, &e, n), &t, "nginx", HopKind::Proc);
            assert_eq!(r.len(), 1);
            r[0]
        })
        .collect();
    for w in reps.windows(2) {
        assert_eq!(w[1], (w[0] + 1) % 3, "{reps:?}");
    }
}

#[test]
fn an_edge_balance_overrides_the_node() {
    let (t, e) = build(&cfg(
        "",
        "",
        ", replicas: 3, balance: random",
        ", balance: { weighted: [0, 0, 1] }",
        "[ db ]",
    ));
    for n in 0..100 {
        assert_eq!(of(&hops(&t, &e, n), &t, "db", HopKind::Net), [2], "n = {n}");
    }
}

/// A gateway and a login service with no gateway-to-auth edge, so auth's body runs inline in the
/// gateway's sequence (local validation).
fn auth_cfg(gw_f: &str, auth_f: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms{gw_f} }}\n  login: {{ type: auth, proc: 2ms{auth_f} }}\n  app: {{ type: service, proc: 5ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n"
    )
}

#[test]
fn an_inline_auth_node_never_inherits_the_gateways_replica() {
    // gateway on 3 replicas, auth on 1: auth's hops must stay on its only replica
    let (t, e) = build(&auth_cfg(", replicas: 3", ""));
    let login = t.id("login").unwrap();
    for n in 0..60 {
        let out = run_key(&t, &e, "POST /login", &ChaosState::default(), n);
        let on_login: Vec<u16> = plan_hops(&out)
            .iter()
            .filter(|h| h.node == login)
            .map(|h| h.replica)
            .collect();
        assert!(
            !on_login.is_empty(),
            "the login route runs auth's own work, n = {n}"
        );
        assert!(
            on_login.iter().all(|r| *r == 0),
            "login has one replica: {on_login:?}, n = {n}"
        );
    }
}

#[test]
fn an_inline_auth_node_with_replicas_is_balanced_on_its_own() {
    // gateway on 1 replica, auth on 3 random: auth's work spreads over its own replicas
    let (t, e) = build(&auth_cfg("", ", replicas: 3, balance: random"));
    let login = t.id("login").unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..200 {
        let out = run_key(&t, &e, "POST /login", &ChaosState::default(), n);
        for h in plan_hops(&out).iter().filter(|h| h.node == login) {
            assert!(h.replica < 3);
            seen.insert(h.replica);
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

fn plan_hops(out: &kaanal_core::ChainOutcome) -> Vec<Hop> {
    out.plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_down_replicated_cache_is_a_miss_not_a_failure() {
    // Z17: a cache never fails the request. Adding replicas must not change that.
    let src = "version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, proc: 1ms }\n  c: { type: cache, hit_ratio: 0.9, hit: 1ms, miss: 2ms, replicas: 2 }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: c }\n";
    let (t, e) = build(src);
    let c = t.id("c").unwrap();
    let mut state = ChaosState::default();
    state.set(
        kaanal_core::ChaosKey::Node(c),
        kaanal_core::ChaosLayer {
            down: Some(true),
            ..kaanal_core::ChaosLayer::default()
        },
    );
    for n in 0..30 {
        let out = run(&t, &e, &state, n);
        assert_eq!(out.result, ChainResult::Ok, "n = {n}");
        assert!(
            out.hops
                .iter()
                .any(|h| h.node == c && h.kind == HopKind::CacheMiss),
            "a down cache is a miss, n = {n}"
        );
    }
}
