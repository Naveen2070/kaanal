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

/// Most exec tests check exact byte arithmetic, so a config that does not say otherwise gets
/// `header_bytes: 0` (RULES 65: the default is 600 B and 400 B); the header tests write their own.
fn without_headers(src: &str) -> String {
    if src.contains("header_bytes") {
        src.to_string()
    } else {
        src.replacen("version: 1\n", "version: 1\nheader_bytes: 0\n", 1)
    }
}

/// Like [`build`], but the config keeps the default headers when it does not set `header_bytes`.
pub(crate) fn build_raw(src: &str) -> (Topology, Endpoints) {
    compile_cfg(&Config::from_yaml(src).unwrap())
}

pub(crate) fn build(src: &str) -> (Topology, Endpoints) {
    compile_cfg(&Config::from_yaml(&without_headers(src)).unwrap())
}

fn compile_cfg(cfg: &Config) -> (Topology, Endpoints) {
    let t = Topology::build(cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(cfg)).unwrap();
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
