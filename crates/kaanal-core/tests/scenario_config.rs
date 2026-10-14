use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    Config, Endpoints, PathSeg, ScenarioConfig, Topology, parse_scenario, resolve_scenario,
};

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

fn fixture() -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(BASE).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

#[test]
fn a_scenario_resolves_to_layers() {
    let (topo, eps) = fixture();
    let cfg: ScenarioConfig = serde_yaml_ng::from_str(
        r#"
description: payments down
chaos:
  - { node: db, down: true }
  - { endpoint: "get /users/{id}", latency_ms: 800 }
  - { node: db, endpoint: "GET /users/{id}", error_rate: 0.3, error_kind: reset }
"#,
    )
    .unwrap();
    let (s, warns) = resolve_scenario("pd", &cfg, &topo, &eps).unwrap();
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(s.description.as_deref(), Some("payments down"));
    assert_eq!(s.layers.entry_count(), 3);
    let e = s.layers.compile();
    assert!(
        e.visit(topo.id("db").unwrap(), "GET /users/{id}")
            .unwrap()
            .down
    );
    assert_eq!(
        e.endpoint("GET /users/{id}")
            .unwrap()
            .latency_add
            .as_millis(),
        800
    );
}

#[test]
fn bad_scenarios_are_rejected_with_the_entry_and_field() {
    let (topo, eps) = fixture();
    // (yaml, wanted words, path of the error)
    let cases = [
        ("chaos: [ { down: true } ]", "node or endpoint", "chaos.0"),
        (
            "chaos: [ { node: ghost, down: true } ]",
            "unknown node",
            "chaos.0.node",
        ),
        (
            "chaos: [ { endpoint: \"GET /nope\", down: true } ]",
            "unknown endpoint",
            "chaos.0.endpoint",
        ),
        (
            "chaos: [ { endpoint: nope, down: true } ]",
            "METHOD /path",
            "chaos.0.endpoint",
        ),
        ("chaos: [ { node: db } ]", "sets nothing", "chaos.0"),
        ("chaos: []", "no chaos entries", "chaos"),
        (
            "chaos: [ { node: db, error_kind: status, error_status: 200, error_rate: 0.1 } ]",
            "500-504",
            "chaos.0.error_status",
        ),
        (
            "chaos: [ { node: db, error_kind: boom, error_rate: 0.1 } ]",
            "error_kind",
            "chaos.0.error_kind",
        ),
        (
            "chaos: [ { node: db, error_rate: 2 } ]",
            "error_rate",
            "chaos.0.error_rate",
        ),
        (
            "chaos: [ { node: db, error_kind: reset, error_status: 200, error_rate: 0.1 } ]",
            "500-504",
            "chaos.0.error_status",
        ),
        (
            "chaos: [ { node: db, latency_ms: 400000 } ]",
            "5 min",
            "chaos.0.latency_ms",
        ),
        (
            "chaos: [ { node: db, down: true }, { node: db, latency_ms: 5 } ]",
            "same key",
            "chaos.1",
        ),
        (
            "chaos: [ { node: db, down: true } ]\nlimits: { rate: 1 }",
            "not supported yet",
            "limits",
        ),
        (
            "chaos: [ { node: db, down: true } ]\npins: {}",
            "not supported yet",
            "pins",
        ),
    ];
    for (y, want, at) in cases {
        let cfg: ScenarioConfig = serde_yaml_ng::from_str(y).unwrap();
        let e = resolve_scenario("x", &cfg, &topo, &eps).unwrap_err();
        assert!(e.message.contains(want), "{y}: {}", e.message);
        let path: Vec<String> = e
            .path
            .iter()
            .map(|p| match p {
                PathSeg::Key(k) => k.clone(),
                PathSeg::Index(i) => i.to_string(),
            })
            .collect();
        assert_eq!(path.join("."), at, "{y}");
    }
}

#[test]
fn too_many_raw_entries_are_refused_even_if_keys_collapse() {
    let (topo, eps) = fixture();
    let entries = vec!["{ node: db, down: true }"; 257].join(", ");
    let cfg: ScenarioConfig = serde_yaml_ng::from_str(&format!("chaos: [ {entries} ]")).unwrap();
    let e = resolve_scenario("x", &cfg, &topo, &eps).unwrap_err();
    assert!(e.message.contains("at most 256"), "{}", e.message);
}

#[test]
fn unreachable_pair_and_orphan_fields_are_warnings() {
    let (topo, eps) = fixture();
    // GET /health is a static route: gateway only, so db is never visited through it.
    let cfg: ScenarioConfig = serde_yaml_ng::from_str(
        "chaos: [ { node: db, endpoint: \"GET /health\", down: true }, { node: db, error_kind: reset } ]",
    )
    .unwrap();
    let (_, warns) = resolve_scenario("x", &cfg, &topo, &eps).unwrap();
    assert_eq!(warns.len(), 2, "{warns:?}");
    assert!(warns[0].contains("never visits"), "{warns:?}");
    assert!(warns[1].contains("error_rate"), "{warns:?}");
}

#[test]
fn scenario_files_parse_and_reject_unknown_keys() {
    let ok =
        parse_scenario("\u{feff}description: d\nchaos: [ { node: db, down: true } ]\n").unwrap();
    assert_eq!(ok.chaos.len(), 1);
    let e = parse_scenario("chaos: []\nbogus: 1\n").unwrap_err();
    assert!(e.line.is_some(), "{e}");
    assert!(
        parse_scenario("description: x\n").is_err(),
        "chaos is required"
    );
}

#[test]
fn config_scenarios_key_parses() {
    let cfg = Config::from_yaml(&format!(
        "{BASE}scenarios:\n  pd:\n    chaos: [ {{ node: db, down: true }} ]\n"
    ))
    .unwrap();
    assert_eq!(cfg.scenarios.len(), 1);
}

#[test]
fn duplicate_key_names_the_earlier_entry() {
    let (topo, eps) = fixture();
    let cfg: ScenarioConfig = serde_yaml_ng::from_str(
        "chaos: [ { node: db, down: true }, { endpoint: GET /health, down: true }, { node: db, latency_ms: 5 } ]",
    )
    .unwrap();
    let e = resolve_scenario("x", &cfg, &topo, &eps).unwrap_err();
    assert!(e.message.contains("same key as chaos[0]"), "{}", e.message);
}

#[test]
fn null_limits_and_pins_read_as_absent() {
    let (topo, eps) = fixture();
    let cfg: ScenarioConfig =
        serde_yaml_ng::from_str("chaos: [ { node: db, down: true } ]\nlimits: null\npins: ~\n")
            .unwrap();
    resolve_scenario("x", &cfg, &topo, &eps).unwrap();
}
