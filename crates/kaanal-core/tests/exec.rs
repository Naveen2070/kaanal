use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosState, Config, HopKind, NodeChaos, Op, Topology,
    execute,
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
"#;

fn topo() -> Topology {
    Topology::build(&Config::from_yaml(BASE).unwrap()).unwrap()
}

fn req(t: &Topology, n: u64) -> ChainRequest<'static> {
    ChainRequest {
        entry: t.id("app"),
        op: Op::Read,
        endpoint_key: "GET /users/{id}",
        n,
        seed: 42,
    }
}

fn run(t: &Topology, chaos: &ChaosState, n: u64) -> ChainOutcome {
    execute(t, chaos, &req(t, n))
}

fn chaos_on(t: &Topology, node: &str, c: NodeChaos) -> ChaosState {
    let mut s = ChaosState::default();
    s.nodes.insert(t.id(node).unwrap(), c);
    s
}

#[test]
fn same_inputs_same_outcome() {
    let t = topo();
    assert_eq!(
        run(&t, &ChaosState::default(), 3),
        run(&t, &ChaosState::default(), 3)
    );
}

#[test]
fn different_n_gives_different_timings() {
    let t = topo();
    assert_ne!(
        run(&t, &ChaosState::default(), 0).hops,
        run(&t, &ChaosState::default(), 1).hops
    );
}

#[test]
fn happy_path_visits_gateway_service_db_in_order() {
    let t = topo();
    let out = run(&t, &ChaosState::default(), 0);
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
    assert_eq!(out.result, ChainResult::Ok);
    assert_eq!(
        out.total(),
        out.hops.iter().map(|h| h.dur).sum::<Duration>()
    );
}

#[test]
fn error_rate_one_always_fails_at_that_node() {
    let t = topo();
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
        let out = run(&t, &chaos, n);
        assert_eq!(
            out.result,
            ChainResult::Err {
                node: t.id("db").unwrap(),
                status: 504
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
    let t = topo();
    let chaos = chaos_on(
        &t,
        "app",
        NodeChaos {
            down: true,
            ..Default::default()
        },
    );
    let out = run(&t, &chaos, 0);
    assert_eq!(
        out.result,
        ChainResult::Err {
            node: t.id("app").unwrap(),
            status: 503
        }
    );
    assert_eq!(out.hops.len(), 2);
}

#[test]
fn error_rate_zero_never_fails() {
    let t = topo();
    let chaos = chaos_on(&t, "db", NodeChaos::default());
    assert!((0..1000).all(|n| run(&t, &chaos, n).result == ChainResult::Ok));
}

#[test]
fn latency_add_lands_on_that_node() {
    let t = topo();
    let chaos = chaos_on(
        &t,
        "db",
        NodeChaos {
            latency_add: Duration::from_millis(100),
            ..Default::default()
        },
    );
    let out = run(&t, &chaos, 0);
    let db_op = out
        .hops
        .iter()
        .find(|h| h.kind == HopKind::Op(Op::Read))
        .unwrap();
    assert!(db_op.dur >= Duration::from_millis(100));
}

#[test]
fn inert_chaos_entry_does_not_change_timings() {
    let t = topo();
    let chaos = chaos_on(&t, "db", NodeChaos::default());
    assert_eq!(run(&t, &chaos, 5), run(&t, &ChaosState::default(), 5));
}

#[test]
fn static_route_only_touches_gateway() {
    let t = topo();
    let r = ChainRequest {
        entry: None,
        op: Op::Read,
        endpoint_key: "GET /health",
        n: 0,
        seed: 42,
    };
    let out = execute(&t, &ChaosState::default(), &r);
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
    let t = topo();
    let chaos = chaos_on(
        &t,
        "nginx",
        NodeChaos {
            down: true,
            ..Default::default()
        },
    );
    let r = ChainRequest {
        entry: None,
        op: Op::Read,
        endpoint_key: "GET /health",
        n: 0,
        seed: 42,
    };
    let out = execute(&t, &chaos, &r);
    assert_eq!(
        out.result,
        ChainResult::Err {
            node: t.gateway,
            status: 503
        }
    );
    assert!(out.hops.is_empty());
}
