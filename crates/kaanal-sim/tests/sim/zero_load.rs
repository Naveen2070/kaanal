//! Zero-load equality: the simulator against `execute` for every failure kind, spans, hangs, endpoint chaos and stacked scenarios.

use crate::common::*;
use crate::shared::compiled;
use kaanal_core::{
    ChainResult, ChaosKey, ChaosLayer, ChaosStack, ChaosState, Config, ErrorKind, HopKind,
    NodeChaos, ScenarioSet, Source,
};
use kaanal_sim::run;
use std::collections::BTreeMap;
use std::time::Duration;

#[test]
fn sim_reports_timeouts_per_node_and_errors_by_kind() {
    // gateway deadline 2ms in front of a db whose op takes 10ms: every request times out
    let (t, e) = build_with("type: gateway, proc: 1ms, timeout: 2ms", SLOW_DB);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 200)).unwrap();
    let nginx = t.id("nginx").unwrap();
    assert!(rep.requests.len() > 5_000);
    assert_eq!(rep.nodes[nginx].timed_out, rep.requests.len() as u64);
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", rep.requests.len() as u64)])
    );
}

#[test]
fn zero_load_equals_execute_for_every_kind() {
    // R7.5: at zero load the sim record equals the independent execute() outcome.
    let db_timeout = format!("{DB}, timeout: 3s");
    // (chaos kind, label, db body, error kind, whether a span deadline fires at db). A hang
    // that ends through its planned Timeout `Fail` (no span: the default cap) is not an
    // engine deadline, so db.timed_out stays 0 although errors_by_kind says `timeout`.
    // With a node timeout the db span's deadline (3 s, from the edge start) comes 1 ms before
    // the hang hop's end (edge 1 ms + 3 s cap), so the deadline fires and is counted at db.
    for (kind, name, db, want, deadline) in [
        (ErrorKind::Status, "status", DB, "status", false),
        (ErrorKind::Reset, "reset", DB, "reset", false),
        (ErrorKind::Malformed, "malformed", DB, "malformed", false),
        (ErrorKind::Hang, "hang default cap", DB, "timeout", false),
        (
            ErrorKind::Hang,
            "hang node timeout",
            &*db_timeout,
            "timeout",
            true,
        ),
    ] {
        let (t, e) = build_with(GW, db);
        let chaos = ChaosState::from([(
            t.id("db").unwrap(),
            NodeChaos {
                error_rate: 1.0,
                error_kind: kind,
                ..NodeChaos::default()
            },
        )]);
        let rep = zero_load(&t, &e, &chaos, name);
        assert_eq!(
            rep.errors_by_kind(),
            BTreeMap::from([(want, all(&rep))]),
            "{name}: every request must fail with that kind"
        );
        let timed_out = rep.nodes[t.id("db").unwrap()].timed_out;
        assert_eq!(timed_out, if deadline { all(&rep) } else { 0 }, "{name}");
    }
}

#[test]
fn zero_load_equals_execute_with_spans() {
    // db's 3 ms deadline covers its 1 ms edge and a 10 ms op: it really times out at zero load.
    let (t, e) = build_with(GW, &format!("{SLOW_DB}, timeout: 3ms"));
    let rep = zero_load(&t, &e, &ChaosState::default(), "db span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));
}

pub(crate) fn hang() -> NodeChaos {
    NodeChaos {
        error_rate: 1.0,
        error_kind: ErrorKind::Hang,
        ..NodeChaos::default()
    }
}

#[test]
fn zero_load_equals_execute_for_a_hang_under_spans() {
    // (a) db span 3 ms (from the edge start) vs a hang hop capped at the same 3 ms that
    // starts after the 1 ms edge: the deadline fires first, 1 ms before the planned Fail (Z6).
    let (t, e) = build_with(GW, &format!("{DB}, timeout: 3ms"));
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let rep = zero_load(&t, &e, &chaos, "hang under db span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));

    // The hanging db has no timeout (60 s cap); the gateway's root span (1 s) fires first.
    let (t, e) = build_with("type: gateway, proc: 1ms, timeout: 1s", DB);
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let rep = zero_load(&t, &e, &chaos, "hang under gateway span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("nginx").unwrap()].timed_out, all(&rep));
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, 0);
}

#[test]
fn zero_load_equals_execute_for_a_deadline_in_a_parallel_branch() {
    // (b) db's 2 ms span fires inside one branch while the other (`slow`, 50 ms) is cut.
    let (t, e) = compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {GW} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {SLOW_DB}, timeout: 2ms }}\n  slow: {{ type: external-api, proc: 50ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: slow, net: 1ms }}\nendpoints:\n  \"GET /x\": {{ chain: [ {{ parallel: [ db, slow ] }} ] }}\n"
        ),
        &[("GET /x", "app")],
    );
    let (rep, cut) = assert_zero_load_equal(
        &t,
        &e,
        &ChaosState::default(),
        "parallel",
        &["GET /x"],
        SEED,
    );
    assert!(cut > 0, "the deadline must cancel the sibling branch");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));
    assert_eq!(rep.nodes[t.id("slow").unwrap()].timed_out, 0);
}

#[test]
fn zero_load_equals_execute_for_a_hang_in_a_parallel_branch() {
    // A hang in one branch (60 s cap, planned `Fail` at the hang hop's end) while the sibling
    // (`slow`, 50 ms) finishes long before: the failure ends the block at the hang's end.
    let (t, e) = compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {GW} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {DB} }}\n  slow: {{ type: external-api, proc: 50ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: slow, net: 1ms }}\nendpoints:\n  \"GET /x\": {{ chain: [ {{ parallel: [ db, slow ] }} ] }}\n"
        ),
        &[("GET /x", "app")],
    );
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let (rep, _) = assert_zero_load_equal(&t, &e, &chaos, "hang in parallel", &["GET /x"], SEED);
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(
        rep.nodes[t.id("db").unwrap()].timed_out,
        0,
        "no deadline fired, the hang ran to its Fail"
    );
}

#[test]
fn zero_load_equals_execute_for_endpoint_chaos() {
    // (c) endpoint-level chaos: a 502 at the front door, a Delay hop, and a node chaos that
    // applies only through this endpoint.
    let (t, e) = build_with(GW, DB);
    let db = t.id("db").unwrap();
    let with = |key: ChaosKey, layer: ChaosLayer| {
        let mut c = ChaosState::default();
        c.set(key, layer);
        c
    };
    let front = || ChaosKey::Endpoint(KEY.into());
    let fail = with(
        front(),
        ChaosLayer {
            error_rate: Some(1.0),
            error_status: Some(502),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &fail, "endpoint 502");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("status", all(&rep))])
    );
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Err { status: 502, .. }))
    );

    let slow = with(
        front(),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(20)),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &slow, "endpoint latency");
    assert!(rep.errors_by_kind().is_empty());
    assert!(
        rep.requests
            .iter()
            .all(|r| r.hops.iter().any(|(h, _)| h.kind == HopKind::Delay))
    );

    let at = with(
        ChaosKey::NodeAt(db, KEY.into()),
        ChaosLayer {
            error_rate: Some(1.0),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &at, "node-at db");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("status", all(&rep))])
    );
}

#[test]
fn zero_load_equals_live_exactly() {
    for (name, yaml, routes) in sample_configs() {
        let seed = Config::from_yaml(&yaml).unwrap().seed;
        let (t, e) = compiled(&yaml, &routes);
        // Failing databases, external APIs and caches (a failing cache is a miss, E9).
        let flaky: ChaosState = ["db", "pay", "c"]
            .iter()
            .filter_map(|n| t.id(n))
            .map(|id| {
                (
                    id,
                    NodeChaos {
                        error_rate: 0.5,
                        ..NodeChaos::default()
                    },
                )
            })
            .collect();
        for chaos in [ChaosState::default(), flaky] {
            let keys: Vec<&str> = routes.iter().map(|(k, _)| *k).collect();
            let (rep, cut) = assert_zero_load_equal(&t, &e, &chaos, name, &keys, seed);
            assert!(rep.requests.len() > 5_000, "{name}: {}", rep.requests.len());
            if name == "chains" && !chaos.is_empty() {
                assert!(
                    cut > 100,
                    "failures inside parallel must be exercised: {cut}"
                );
            }
        }
    }
}

pub(crate) fn layer(rate: f64, kind: ErrorKind, status: u16) -> ChaosLayer {
    ChaosLayer {
        error_rate: Some(rate),
        error_kind: Some(kind),
        error_status: Some(status),
        ..ChaosLayer::default()
    }
}

/// Two scenarios plus a live override, compiled; asserts sim == execute at zero load.
pub(crate) fn stack_drift(
    label: &str,
    s1: Vec<(ChaosKey, ChaosLayer)>,
    s2: Vec<(ChaosKey, ChaosLayer)>,
    live: Vec<(ChaosKey, ChaosLayer)>,
) -> kaanal_sim::SimReport {
    let (t, e) = build_with(GW, DB);
    let mut set = ScenarioSet::default();
    set.insert(scenario("s1", s1), Source::Runtime).unwrap();
    set.insert(scenario("s2", s2), Source::Runtime).unwrap();
    let mut stack = ChaosStack::default();
    stack.activate(&set, "s1").unwrap();
    stack.activate(&set, "s2").unwrap();
    for (k, l) in live {
        stack.live_mut().set(k, l);
    }
    let eff = stack.compile(&set);
    assert!(!eff.is_empty(), "{label}: the stack must compile to chaos");
    assert_effective_equal(&t, &e, &eff, label, &[KEY], SEED).0
}

#[test]
fn zero_load_equals_execute_for_a_scenario_stack() {
    let (t, _) = build_with(GW, DB);
    let (db, app) = (t.id("db").unwrap(), t.id("app").unwrap());
    // A live latency bump rides on top of every case.
    let live = || {
        vec![(
            ChaosKey::Node(t.id("nginx").unwrap()),
            ChaosLayer {
                latency_add: Some(Duration::from_millis(3)),
                ..ChaosLayer::default()
            },
        )]
    };
    for (kind, name) in [
        (ErrorKind::Status, "status"),
        (ErrorKind::Reset, "reset"),
        (ErrorKind::Hang, "hang"),
        (ErrorKind::Malformed, "malformed"),
    ] {
        let rep = stack_drift(
            name,
            vec![(ChaosKey::Node(db), layer(0.5, kind, 502))],
            vec![(ChaosKey::Node(app), layer(0.2, ErrorKind::Status, 503))],
            live(),
        );
        let want = if name == "hang" { "timeout" } else { name };
        assert!(
            rep.errors_by_kind().contains_key(want),
            "{name}: its fault must fire: {:?}",
            rep.errors_by_kind()
        );
    }

    // Banded mix: 30% reset under 30% 503 on one node (about 51% total, two kinds).
    let rep = stack_drift(
        "banded",
        vec![(ChaosKey::Node(db), layer(0.3, ErrorKind::Status, 503))],
        vec![(ChaosKey::Node(db), layer(0.3, ErrorKind::Reset, 503))],
        live(),
    );
    let by = rep.errors_by_kind();
    assert!(
        by.contains_key("reset") && by.contains_key("status"),
        "{by:?}"
    );

    // node@endpoint pair.
    let rep = stack_drift(
        "node-at",
        vec![(
            ChaosKey::NodeAt(db, KEY.into()),
            layer(0.4, ErrorKind::Reset, 503),
        )],
        vec![(ChaosKey::Node(db), layer(0.2, ErrorKind::Status, 503))],
        live(),
    );
    assert!(rep.errors_by_kind().contains_key("reset"));

    // endpoint-level entry with latency.
    let front = ChaosLayer {
        latency_add: Some(Duration::from_millis(20)),
        ..layer(0.3, ErrorKind::Status, 502)
    };
    let rep = stack_drift(
        "endpoint",
        vec![(ChaosKey::Endpoint(KEY.into()), front)],
        vec![(ChaosKey::Node(db), layer(0.2, ErrorKind::Malformed, 503))],
        live(),
    );
    assert!(rep.errors_by_kind().contains_key("status"));

    // A live override calms a scenario: nothing fails, and the sim still agrees with execute.
    let calm = ChaosLayer {
        replace: Some(true),
        ..ChaosLayer::default()
    };
    let rep = stack_drift(
        "calmed",
        vec![(ChaosKey::Node(db), layer(1.0, ErrorKind::Reset, 503))],
        vec![(ChaosKey::Node(db), layer(1.0, ErrorKind::Status, 503))],
        vec![(ChaosKey::Node(db), calm)],
    );
    assert!(rep.errors_by_kind().is_empty());
}
