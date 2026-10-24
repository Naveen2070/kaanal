//! Shared helpers: the base config and pin, scenario and config builders.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};

use kaanal_core::{
    Config, Endpoints, PinEntry, PinLayer, PinState, Scenario, ScenarioConfig, Topology,
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

pub(crate) fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub(crate) fn pin(state: PinState) -> PinEntry {
    PinEntry {
        state,
        status: None,
        delay: None,
    }
}

pub(crate) fn scenario(name: &str, pins: &[(&str, PinState)]) -> Scenario {
    let mut layer = PinLayer::default();
    for (k, s) in pins {
        layer.insert((*k).to_string(), pin(*s));
    }
    Scenario {
        name: name.into(),
        description: None,
        layers: kaanal_core::ChaosState::default(),
        pins: layer,
    }
}

pub(crate) fn cfg(y: &str) -> ScenarioConfig {
    serde_yaml_ng::from_str(y).unwrap()
}
