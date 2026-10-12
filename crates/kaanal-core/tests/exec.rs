use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosState, Config, Endpoints, HopKind, NodeChaos, Op,
    Topology, execute,
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
        chaos,
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
            retry_after: None
        }
    );
}
