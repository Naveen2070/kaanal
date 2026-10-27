//! Shared helpers: the base config, topology and the run functions.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};

use kaanal_core::{
    ChainOutcome, ChainRequest, ChaosState, Config, Endpoints, NodeChaos, Topology, execute,
};

use std::time::Duration;

pub(crate) const BASE: &str = r#"
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

pub(crate) fn build(src: &str) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

pub(crate) fn topo() -> (Topology, Endpoints) {
    build(BASE)
}

pub(crate) fn run_key(
    t: &Topology,
    e: &Endpoints,
    key: &str,
    chaos: &ChaosState,
    n: u64,
) -> ChainOutcome {
    execute(
        t,
        &chaos.compile(),
        &ChainRequest {
            endpoint: e.get(key).unwrap(),
            n,
            seed: 42,
            pin_delay: std::time::Duration::ZERO,
            req_bytes: 0,
            resp_bytes: 0,
        },
    )
}

pub(crate) fn run(t: &Topology, e: &Endpoints, chaos: &ChaosState, n: u64) -> ChainOutcome {
    run_key(t, e, "GET /users/{id}", chaos, n)
}

pub(crate) fn chaos_on(t: &Topology, node: &str, c: NodeChaos) -> ChaosState {
    let mut s = ChaosState::default();
    s.insert(t.id(node).unwrap(), c);
    s
}

pub(crate) fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
