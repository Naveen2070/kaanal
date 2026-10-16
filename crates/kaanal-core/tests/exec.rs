use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosKey, ChaosLayer, ChaosState, Config, Endpoints,
    ErrorKind, FailKind, HANG_CAP, HopKind, NodeChaos, Op, PlanItem, Topology, execute,
};
use std::time::Duration;

const BASE: &str = r#"
version: 1
nodes:
  nginx: { type: gateway, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service, spec: openapi.yaml, proc: { p50: 5ms, p99: 20ms } }
  db:    { type: database, ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }
endpoints:
  "GET /health": { override: { body: { ok: true } } }
"#;

fn build(src: &str) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

fn topo() -> (Topology, Endpoints) {
    build(BASE)
}

fn run_key(t: &Topology, e: &Endpoints, key: &str, chaos: &ChaosState, n: u64) -> ChainOutcome {
    execute(
        t,
        &chaos.compile(),
        &ChainRequest {
            endpoint: e.get(key).unwrap(),
            n,
            seed: 42,
        },
    )
}

fn run(t: &Topology, e: &Endpoints, chaos: &ChaosState, n: u64) -> ChainOutcome {
    run_key(t, e, "GET /users/{id}", chaos, n)
}

fn chaos_on(t: &Topology, node: &str, c: NodeChaos) -> ChaosState {
    let mut s = ChaosState::default();
    s.insert(t.id(node).unwrap(), c);
    s
}

#[test]
fn same_inputs_same_outcome() {
    let (t, e) = topo();
    assert_eq!(
        run(&t, &e, &ChaosState::default(), 3),
        run(&t, &e, &ChaosState::default(), 3)
    );
}

#[test]
fn different_n_gives_different_timings() {
    let (t, e) = topo();
    assert_ne!(
        run(&t, &e, &ChaosState::default(), 0).hops,
        run(&t, &e, &ChaosState::default(), 1).hops
    );
}

#[test]
fn happy_path_visits_gateway_service_db_in_order() {
    let (t, e) = topo();
    let out = run(&t, &e, &ChaosState::default(), 0);
    let kinds: Vec<(String, HopKind)> = out
        .hops
        .iter()
        .map(|h| (t.nodes[h.node].name.clone(), h.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("nginx".into(), HopKind::Proc),
            ("app".into(), HopKind::Net),
            ("app".into(), HopKind::Proc),
            ("db".into(), HopKind::Net),
            ("db".into(), HopKind::Op(Op::Read)),
        ]
    );
    for w in out.hops.windows(2) {
        assert_eq!(
            w[1].start,
            w[0].start + w[0].dur,
            "a sequence: each hop starts when the last ends"
        );
    }
    assert_eq!(out.result, ChainResult::Ok);
    assert_eq!(
        out.total(),
        out.hops.iter().map(|h| h.dur).sum::<Duration>()
    );
}

#[test]
fn error_rate_one_always_fails_at_that_node() {
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_status: 504,
            ..Default::default()
        },
    );
    for n in 0..100 {
        let out = run(&t, &e, &chaos, n);
        assert_eq!(
            out.result,
            ChainResult::Err {
                node: t.id("db").unwrap(),
                status: 504,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        assert_eq!(
            out.hops.len(),
            4,
            "time spent before the failing node still counts"
        );
    }
}

#[test]
fn down_node_returns_503() {
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "app",
        NodeChaos {
            down: true,
            ..Default::default()
        },
    );
    let out = run(&t, &e, &chaos, 0);
    assert_eq!(
        out.result,
        ChainResult::Err {
            node: t.id("app").unwrap(),
            status: 503,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(out.hops.len(), 2);
}

#[test]
fn error_rate_zero_never_fails() {
    let (t, e) = topo();
    let chaos = chaos_on(&t, "db", NodeChaos::default());
    assert!((0..1000).all(|n| run(&t, &e, &chaos, n).result == ChainResult::Ok));
}

#[test]
fn latency_add_lands_on_that_node() {
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "db",
        NodeChaos {
            latency_add: Duration::from_millis(100),
            ..Default::default()
        },
    );
    let out = run(&t, &e, &chaos, 0);
    let db_op = out
        .hops
        .iter()
        .find(|h| h.kind == HopKind::Op(Op::Read))
        .unwrap();
    assert!(db_op.dur >= Duration::from_millis(100));
}

#[test]
fn inert_chaos_entry_does_not_change_timings() {
    let (t, e) = topo();
    let chaos = chaos_on(&t, "db", NodeChaos::default());
    assert_eq!(
        run(&t, &e, &chaos, 5),
        run(&t, &e, &ChaosState::default(), 5)
    );
}

#[test]
fn static_route_only_touches_gateway() {
    let (t, e) = topo();
    let out = run_key(&t, &e, "GET /health", &ChaosState::default(), 0);
    assert_eq!(out.hops.len(), 1);
    assert_eq!(out.result, ChainResult::Ok);
}

#[test]
fn op_inference() {
    assert_eq!(Op::infer("GET", "/users/{id}"), Op::Read);
    assert_eq!(Op::infer("GET", "/users"), Op::Scan);
    assert_eq!(Op::infer("POST", "/orders"), Op::Write);
    assert_eq!(Op::infer("DELETE", "/orders/{id}"), Op::Write);
    assert_eq!(Op::infer("get", "/users"), Op::Scan);
    assert_eq!(Op::Read.name(), "read");
}

#[test]
fn gateway_down_fails_static_route_with_no_hops() {
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "nginx",
        NodeChaos {
            down: true,
            ..Default::default()
        },
    );
    let out = run_key(&t, &e, "GET /health", &chaos, 0);
    assert_eq!(
        out.result,
        ChainResult::Err {
            node: t.gateway.unwrap(),
            status: 503,
            kind: FailKind::Status,
            retry_after: None
        }
    );
    assert_eq!(out.hops, []);
}

#[test]
fn chaos_validate_enforces_the_shared_limits() {
    // R6.9: one set of limits for live mode and the simulator.
    let ok = |c: NodeChaos| c.validate().is_ok();
    assert!(ok(NodeChaos::default()));
    let at_cap = NodeChaos {
        latency_add: kaanal_core::MAX_LATENCY_ADD,
        error_rate: 1.0,
        error_status: 500,
        down: true,
        ..NodeChaos::default()
    };
    assert!(ok(at_cap.clone()));
    assert_eq!(kaanal_core::MAX_LATENCY_ADD, Duration::from_secs(300));
    for bad in [
        NodeChaos {
            latency_add: Duration::from_millis(300_001),
            ..NodeChaos::default()
        },
        NodeChaos {
            error_rate: 1.5,
            ..NodeChaos::default()
        },
        NodeChaos {
            error_rate: -0.1,
            ..NodeChaos::default()
        },
        NodeChaos {
            error_status: 418,
            ..NodeChaos::default()
        },
        NodeChaos {
            error_status: 505,
            ..NodeChaos::default()
        },
    ] {
        assert!(!ok(bad.clone()), "{bad:?}");
    }
    assert!(ok(NodeChaos {
        error_status: 504,
        ..at_cap
    }));
}

#[test]
fn gatewayless_walk_starts_at_the_owner() {
    let (t, e) = build(
        "version: 1\nnodes:\n  app: { type: service, proc: 2ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: app, to: db, net: 1ms }\n",
    );
    let out = run(&t, &e, &ChaosState::default(), 0);
    let kinds: Vec<(&str, HopKind)> = out
        .hops
        .iter()
        .map(|h| (t.nodes[h.node].name.as_str(), h.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("app", HopKind::Proc),
            ("db", HopKind::Net),
            ("db", HopKind::Op(Op::Read))
        ]
    );
    assert_eq!(out.total(), Duration::from_millis(4));
    assert_eq!(out.result, ChainResult::Ok);
}

#[test]
fn callees_below_the_entry_are_charged() {
    // Default chain (spec §5.2): callees in edge order (shop, ext), then the cache (a read).
    let (t, e) = build(
        "version: 1
nodes:
  app: { type: service, proc: 2ms }
  shop: { type: monolith, flavor: mysql }
  c: { type: cache, flavor: redis }
  ext: { type: external-api }
edges:
  - { from: app, to: shop }
  - { from: app, to: c }
  - { from: app, to: ext }
",
    );
    let out = run(&t, &e, &ChaosState::default(), 0);
    let seen: Vec<(&str, HopKind)> = out
        .hops
        .iter()
        .map(|h| (t.nodes[h.node].name.as_str(), h.kind))
        .collect();
    assert_eq!(
        seen[..7],
        [
            ("app", HopKind::Proc),
            ("shop", HopKind::Net),
            ("shop", HopKind::Proc),
            ("shop", HopKind::Op(Op::Read)),
            ("ext", HopKind::Net),
            ("ext", HopKind::Proc),
            ("c", HopKind::Net),
        ]
    );
    assert!(
        matches!(seen[7], ("c", HopKind::CacheHit | HopKind::CacheMiss)),
        "{seen:?}"
    );
    assert_eq!(seen.len(), 8);
    assert!(
        out.hops[3].dur > Duration::ZERO && out.hops[7].dur > Duration::ZERO,
        "preset costs"
    );
    assert!(out.total() > Duration::from_millis(2));
}

#[test]
fn execute_never_sets_retry_after() {
    let (t, e) = topo();
    let chaos = chaos_on(
        &t,
        "app",
        NodeChaos {
            error_rate: 1.0,
            error_status: 502,
            ..Default::default()
        },
    );
    let out = run(&t, &e, &chaos, 0);
    assert_eq!(
        out.result,
        ChainResult::Err {
            node: t.id("app").unwrap(),
            status: 502,
            kind: FailKind::Status,
            retry_after: None
        }
    );
}

fn timed(extra_nodes: &str) -> (Topology, Endpoints) {
    build(&BASE.replace(
        "nginx: { type: gateway,",
        &format!("nginx: {{ type: gateway,{extra_nodes}"),
    ))
}

#[test]
fn a_node_without_timeout_makes_no_spans() {
    let (t, e) = topo();
    let out = run(&t, &e, &ChaosState::default(), 0);
    assert!(
        out.plan
            .items
            .iter()
            .all(|i| !matches!(i, PlanItem::Span { .. }))
    );
}

#[test]
fn timeout_on_the_callee_wraps_edge_hop_and_body_in_a_span() {
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 50ms,",
    ));
    let out = run(&t, &e, &ChaosState::default(), 0);
    let i = out
        .plan
        .items
        .iter()
        .position(|i| matches!(i, PlanItem::Span { .. }))
        .unwrap();
    let PlanItem::Span {
        node,
        timeout,
        size,
        ..
    } = &out.plan.items[i]
    else {
        unreachable!()
    };
    assert_eq!(
        (*node, *timeout),
        (t.id("db").unwrap(), Duration::from_millis(50))
    );
    // the net hop to db and the db op are inside
    assert_eq!(*size, 2);
}

#[test]
fn execute_reports_a_zero_load_timeout_at_the_deadline() {
    // db read takes 4ms p50 / 20ms p99; a 1ms deadline on db always passes before the op ends
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 1ms,",
    ));
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err {
        node, status, kind, ..
    } = &out.result
    else {
        panic!("{:?}", out.result)
    };
    assert_eq!(
        (*node, *status, *kind),
        (t.id("db").unwrap(), 504, FailKind::Timeout)
    );
    let span_start = out
        .plan
        .items
        .iter()
        .find_map(|i| match i {
            PlanItem::Span { at, .. } => Some(*at),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        out.total(),
        span_start + Duration::from_millis(1),
        "answered at the deadline"
    );
    assert!(
        out.hops.iter().all(|h| h.start + h.dur <= out.total()),
        "hops are cut at the deadline"
    );
}

#[test]
fn gateway_timeout_covers_the_whole_chain() {
    let (t, e) = timed(" timeout: 2ms,");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("nginx").unwrap(), FailKind::Timeout));
}

#[test]
fn a_planned_fail_earlier_than_the_deadline_wins() {
    // gateway down fails at offset 0, before its own 1 ms deadline; the planned Fail decides
    let (t, e) = build(&BASE.replace(
        "nginx: { type: gateway,",
        "nginx: { type: gateway, timeout: 1ms,",
    ));
    let gw_down = chaos_on(
        &t,
        "nginx",
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &gw_down, 0);
    let ChainResult::Err { kind, status, .. } = &out.result else {
        panic!()
    };
    assert_eq!(
        (*kind, *status),
        (FailKind::Status, 503),
        "fail at offset 0 is earlier than 1ms"
    );
}

#[test]
fn reset_and_malformed_are_planned_fails_with_their_kind() {
    let (t, e) = topo();
    for (k, want) in [
        (ErrorKind::Reset, FailKind::Reset),
        (ErrorKind::Malformed, FailKind::Malformed),
    ] {
        let c = chaos_on(
            &t,
            "db",
            NodeChaos {
                error_rate: 1.0,
                error_kind: k,
                ..NodeChaos::default()
            },
        );
        let out = run(&t, &e, &c, 0);
        let ChainResult::Err {
            node, kind, status, ..
        } = &out.result
        else {
            panic!()
        };
        assert_eq!((*node, *kind, *status), (t.id("db").unwrap(), want, 502));
    }
}

#[test]
fn hang_is_a_capped_hop_then_a_timeout_fail() {
    let (t, e) = topo();
    let c = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    let ChainResult::Err {
        kind, status, node, ..
    } = &out.result
    else {
        panic!()
    };
    assert_eq!(
        (*kind, *status, *node),
        (FailKind::Timeout, 504, t.id("db").unwrap())
    );
    let hang = out
        .hops
        .iter()
        .find(|h| h.node == t.id("db").unwrap() && matches!(h.kind, HopKind::Op(_)))
        .unwrap();
    assert_eq!(hang.dur, HANG_CAP, "no node timeout: the 60 s default cap");
}

#[test]
fn hang_uses_the_node_timeout_when_set() {
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 3s,",
    ));
    let c = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    // The plan keeps the full cap, but db's span opened at the edge hop (Z6 consequence), so its
    // deadline precedes the hang's own Fail and `out.hops` is clipped there.
    let hang = out.plan.items.iter().find_map(|i| match i {
        PlanItem::Hop(h) if matches!(h.kind, HopKind::Op(_)) => Some(h),
        _ => None,
    });
    assert_eq!(hang.unwrap().dur, Duration::from_secs(3));
    let db = t.id("db").unwrap();
    let span_at = out.plan.items.iter().find_map(|i| match i {
        PlanItem::Span { node, at, .. } if *node == db => Some(*at),
        _ => None,
    });
    assert_eq!(out.total(), span_at.unwrap() + Duration::from_secs(3));
    assert!(matches!(out.result, ChainResult::Err { node, .. } if node == db));
    assert!(matches!(
        out.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
}

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

/// Fixed timings: nginx proc 1 ms, edge nets 1 ms, app proc 5 ms, every db op 10 ms.
fn custom(app: &str, db: &str, db2: &str, chain: &str) -> (Topology, Endpoints) {
    build(&format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, spec: openapi.yaml, proc: 5ms{app} }}\n  db: {{ type: database{db}, ops: {{ read: 10ms, write: 10ms, scan: 10ms }} }}\n  db2: {{ type: database{db2}, ops: {{ read: 10ms, write: 10ms, scan: 10ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: db2, net: 1ms }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: {chain}\n"
    ))
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn a_planned_fail_at_the_deadline_instant_wins_the_tie() {
    // span(app) opens at 1 ms, due 8 ms; db is down and fails at its op start, also 8 ms,
    // while db2's op runs on to 18 ms, so the span is violated.
    let (t, e) = custom(", timeout: 7ms", "", "", "[ { parallel: [ db, db2 ] } ]");
    let down = chaos_on(
        &t,
        "db",
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &down, 0);
    let ChainResult::Err {
        node, status, kind, ..
    } = &out.result
    else {
        panic!("{:?}", out.result)
    };
    assert_eq!(
        (*node, *status, *kind),
        (t.id("db").unwrap(), 503, FailKind::Status)
    );
    assert_eq!(out.total(), ms(8));
}

#[test]
fn the_earliest_deadline_wins_not_the_first_span_in_plan_order() {
    // outer app span: opens 1 ms, due 16 ms, body ends 18 ms (violated).
    // inner db span: opens 7 ms, due 9 ms, body ends 18 ms (violated, and due earlier).
    let (t, e) = custom(", timeout: 15ms", ", timeout: 2ms", "", "[ db ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("db").unwrap(), FailKind::Timeout));
    assert_eq!(out.total(), ms(9));
}

#[test]
fn a_span_in_a_parallel_branch_cuts_every_hop_at_its_deadline() {
    // db2 span: opens 7 ms, due 9 ms; both branches' ops would run to 18 ms.
    let (t, e) = custom("", "", ", timeout: 2ms", "[ { parallel: [ db, db2 ] } ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("db2").unwrap(), FailKind::Timeout));
    assert_eq!(out.total(), ms(9));
    assert!(
        out.hops.iter().all(|h| h.start + h.dur <= ms(9)),
        "{:?}",
        out.hops
    );
}

#[test]
fn repeat_makes_one_span_per_repetition() {
    let (t, e) = custom("", ", timeout: 50ms", "", "[ { call: db, repeat: 2 } ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    assert!(matches!(out.result, ChainResult::Ok));
    let ats: Vec<Duration> = out
        .plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Span { at, size: 2, .. } => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(ats.len(), 2);
    assert!(ats[0] < ats[1], "{ats:?}");
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

fn layer_on(t: &Topology, node: &str, rate: f64, kind: ErrorKind, status: u16) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Node(t.id(node).unwrap()),
        ChaosLayer {
            error_rate: Some(rate),
            error_kind: Some(kind),
            error_status: Some(status),
            ..ChaosLayer::default()
        },
    );
    s
}

fn fail_kind(o: &ChainOutcome) -> Option<(FailKind, u16)> {
    match o.result {
        ChainResult::Err { kind, status, .. } => Some((kind, status)),
        ChainResult::Ok => None,
    }
}

#[test]
fn a_banded_mix_fails_with_each_kind_at_its_share() {
    // 30% reset on top of 30% 503: the top layer owns the first 30% of rolls, the lower
    // layer the next 21% (0.3 + 0.3 - 0.09 = 0.51).
    let (t, e) = topo();
    let (low, top) = (
        layer_on(&t, "db", 0.3, ErrorKind::Status, 503),
        layer_on(&t, "db", 0.3, ErrorKind::Reset, 502),
    );
    let eff = kaanal_core::compile_layers(&[&low, &top]);
    let (mut reset, mut status) = (0u32, 0u32);
    for n in 0..2000 {
        let o = execute(
            &t,
            &eff,
            &ChainRequest {
                endpoint: e.get("GET /users/{id}").unwrap(),
                n,
                seed: 42,
            },
        );
        match fail_kind(&o) {
            Some((FailKind::Reset, 502)) => reset += 1,
            Some((FailKind::Status, 503)) => status += 1,
            None => {}
            other => panic!("unexpected failure {other:?}"),
        }
    }
    let pct = |c: u32| f64::from(c) / 20.0;
    assert!((pct(reset) - 30.0).abs() < 3.0, "reset {}%", pct(reset));
    assert!((pct(status) - 21.0).abs() < 3.0, "status {}%", pct(status));
}

#[test]
fn a_stack_is_deterministic_and_a_single_layer_ignores_the_stack() {
    let (t, e) = topo();
    let ep = e.get("GET /users/{id}").unwrap();
    let out = |eff: &kaanal_core::EffectiveChaos, n| {
        execute(
            &t,
            eff,
            &ChainRequest {
                endpoint: ep,
                n,
                seed: 42,
            },
        )
    };
    let (low, top) = (
        layer_on(&t, "db", 0.3, ErrorKind::Status, 503),
        layer_on(&t, "db", 0.3, ErrorKind::Reset, 502),
    );
    let stacked = kaanal_core::compile_layers(&[&low, &top]);
    let again = kaanal_core::compile_layers(&[&low, &top]);
    for n in 0..500 {
        assert_eq!(out(&stacked, n), out(&again, n), "n={n}");
    }
    // One layer alone (the pre-stack compile) gives the same outcome as that layer inside a
    // stack of empty layers, for every failure kind.
    let empty = ChaosState::default();
    for kind in [
        ErrorKind::Status,
        ErrorKind::Reset,
        ErrorKind::Hang,
        ErrorKind::Malformed,
    ] {
        let one = layer_on(&t, "db", 0.5, kind, 504);
        let alone = one.compile();
        let wrapped = kaanal_core::compile_layers(&[&empty, &one, &empty]);
        let mut failed = 0;
        for n in 0..500 {
            let o = out(&alone, n);
            failed += u32::from(fail_kind(&o).is_some());
            assert_eq!(o, out(&wrapped, n), "{kind:?} n={n}");
        }
        assert!((150..350).contains(&failed), "{kind:?}: {failed}/500");
    }
}

#[test]
fn stacked_latency_adds_up_and_clamps() {
    let (t, _) = topo();
    let lat = |d: u64| {
        let mut s = ChaosState::default();
        s.set(
            ChaosKey::Node(t.id("db").unwrap()),
            ChaosLayer {
                latency_add: Some(Duration::from_secs(d)),
                ..ChaosLayer::default()
            },
        );
        s
    };
    let (ten, twenty) = (lat(10), lat(20));
    let eff = kaanal_core::compile_layers(&[&ten, &twenty]);
    let at = |eff: &kaanal_core::EffectiveChaos| {
        eff.visit(t.id("db").unwrap(), "x").unwrap().latency_add
    };
    assert_eq!(at(&eff), Duration::from_secs(30));
    let (big, bigger) = (lat(200), lat(200));
    let eff = kaanal_core::compile_layers(&[&big, &bigger]);
    assert_eq!(at(&eff), kaanal_core::MAX_LATENCY_ADD);
}
