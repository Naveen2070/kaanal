//! The A2 chain config the drift tests share: every step type (cache, repeat, parallel,
//! via, with), lognormal and exponential costs.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{Config, Endpoints, Topology};

pub const CHAINS: &str = r#"version: 1
seed: 11
nodes:
  gw: { type: gateway, proc: { p50: 1ms, p99: 3ms } }
  login: { type: auth, proc: { mean: 1ms } }
  app: { type: service, proc: { mean: 2ms } }
  orders: { type: service, proc: { mean: 3ms }, chains: { reserve: [ { call: db, op: write }, pay ] } }
  db: { type: database, flavor: postgres }
  c: { type: cache, flavor: redis }
  pay: { type: external-api, proc: { p50: 5ms, p99: 30ms } }
edges:
  - { from: gw, to: app, net: { mean: 1ms } }
  - { from: gw, to: orders, net: 1ms }
  - { from: gw, to: login, net: 200us }
  - { from: app, to: orders, net: { mean: 1ms }, calls: 2 }
  - { from: app, to: c, net: 200us }
  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }
  - { from: app, to: pay, net: { mean: 2ms } }
  - { from: orders, to: c }
  - { from: orders, to: db, net: 1ms }
  - { from: orders, to: pay, net: 1ms }
endpoints:
  "GET /users/{id}":
    chain:
      - cache: c
      - parallel: [ pay, [ { call: orders, with: reserve } ], { call: orders, via: "POST /orders" } ]
      - { call: db, repeat: 3 }
  "POST /orders": { chain: [ db, pay ] }
"#;

pub const CHAIN_ROUTES: [(&str, &str); 4] = [
    ("GET /users/{id}", "app"),
    ("GET /users", "app"),
    ("GET /orders/{id}", "orders"),
    ("POST /orders", "orders"),
];

/// Builds `yaml` and compiles `routes` (key, owner) with its inline `endpoints:`.
pub fn compiled(yaml: &str, routes: &[(&str, &str)]) -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(yaml).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let specs: Vec<RouteSpec> = routes
        .iter()
        .map(|(k, o)| RouteSpec::inferred(k, t.id(o).unwrap()))
        .collect();
    let e = compile(&t, &specs, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}
