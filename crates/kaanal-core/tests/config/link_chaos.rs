//! Chaos on a link: the `edge:` target, its fields and its rules.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{
    ChaosKey, ChaosLayer, ChaosState, Config, Endpoints, ErrorKind, PathSeg, ScenarioConfig,
    Topology, compile_layers, resolve_scenario,
};
use std::time::Duration;

/// `gw -> app` has a bandwidth (edge 0), `app -> db` has none (edge 1).
const SRC: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: app, bandwidth: 1000000 }\n  - { from: app, to: db }\n";

fn world() -> (Topology, Endpoints) {
    let cfg = Config::from_yaml(SRC).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap())];
    let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap();
    (t, e)
}

fn resolve(
    entry: &str,
) -> Result<(kaanal_core::Scenario, Vec<String>), kaanal_core::ScenarioError> {
    let (t, e) = world();
    let cfg: ScenarioConfig = serde_yaml_ng::from_str(&format!("chaos:\n  - {entry}\n")).unwrap();
    resolve_scenario("s", &cfg, &t, &e, &|_| true)
}

#[test]
fn an_edge_target_parses_as_from_to_fields() {
    let (s, warns) = resolve(
        "{ edge: { from: gw, to: app }, latency_ms: 20, jitter_ms: 5, bandwidth_cap: 500000 }",
    )
    .unwrap();
    assert!(warns.is_empty(), "{warns:?}");
    let layer = s.layers.layer(&ChaosKey::Edge(0)).unwrap();
    assert_eq!(layer.latency_add, Some(Duration::from_millis(20)));
    assert_eq!(layer.jitter, Some(Duration::from_millis(5)));
    assert_eq!(layer.bandwidth_cap, Some(500_000));
    let c = s.layers.compile();
    let r = c.link(0).unwrap();
    assert_eq!(
        (r.jitter, r.bandwidth_cap),
        (Duration::from_millis(5), Some(500_000))
    );
    assert!(c.link(1).is_none());
}

#[test]
fn a_status_kind_on_a_link_is_rejected() {
    for (entry, what) in [
        (
            "{ edge: { from: gw, to: app }, error_rate: 0.5, error_kind: status }",
            "reset or hang",
        ),
        (
            "{ edge: { from: gw, to: app }, error_rate: 0.5, error_kind: malformed }",
            "reset or hang",
        ),
        (
            "{ edge: { from: gw, to: app }, error_rate: 0.5, error_status: 503 }",
            "no status",
        ),
    ] {
        let e = resolve(entry).unwrap_err();
        assert!(e.message.contains(what), "{entry}: {}", e.message);
    }
    // reset and hang are fine, and a link failure defaults to reset
    let (s, _) =
        resolve("{ edge: { from: gw, to: app }, error_rate: 0.5, error_kind: hang }").unwrap();
    assert_eq!(
        s.layers.compile().link(0).unwrap().faults()[0].kind,
        ErrorKind::Hang
    );
    let (s, _) = resolve("{ edge: { from: gw, to: app }, error_rate: 0.5 }").unwrap();
    assert_eq!(
        s.layers.compile().link(0).unwrap().faults()[0].kind,
        ErrorKind::Reset
    );
}

#[test]
fn bandwidth_cap_without_edge_bandwidth_is_rejected() {
    let e = resolve("{ edge: { from: app, to: db }, bandwidth_cap: 100 }").unwrap_err();
    assert!(e.message.contains("no bandwidth to cap"), "{}", e.message);
    let e = resolve("{ edge: { from: gw, to: app }, bandwidth_cap: 2000000 }").unwrap_err();
    assert!(e.message.contains("only lowers"), "{}", e.message);
    assert!(resolve("{ edge: { from: gw, to: app }, bandwidth_cap: 1000000 }").is_ok());
}

#[test]
fn a_scenario_naming_a_missing_edge_is_a_positioned_load_error() {
    let e = resolve("{ edge: { from: gw, to: db }, down: true }").unwrap_err();
    assert!(e.message.contains("no edge 'gw' -> 'db'"), "{}", e.message);
    assert_eq!(
        e.path,
        vec![
            PathSeg::Key("chaos".into()),
            PathSeg::Index(0),
            PathSeg::Key("edge".into())
        ]
    );
}

#[test]
fn link_fields_and_node_targets_do_not_mix() {
    let e = resolve("{ node: db, jitter_ms: 5 }").unwrap_err();
    assert!(e.message.contains("is for an edge entry"), "{}", e.message);
    let e = resolve("{ node: db, bandwidth_cap: 5 }").unwrap_err();
    assert!(e.message.contains("is for an edge entry"), "{}", e.message);
    let e = resolve("{ node: db, edge: { from: gw, to: app }, down: true }").unwrap_err();
    assert!(e.message.contains("takes no node"), "{}", e.message);
}

#[test]
fn edge_layers_merge_like_node_layers() {
    let layer = |ms, jitter, cap| ChaosLayer {
        latency_add: Some(Duration::from_millis(ms)),
        jitter: Some(Duration::from_millis(jitter)),
        bandwidth_cap: Some(cap),
        ..ChaosLayer::default()
    };
    let (mut a, mut b) = (ChaosState::default(), ChaosState::default());
    a.set(ChaosKey::Edge(0), layer(10, 2, 800));
    b.set(ChaosKey::Edge(0), layer(5, 7, 600));
    let r = compile_layers(&[&a, &b]);
    let r = r.link(0).unwrap();
    assert_eq!(r.latency_add, Duration::from_millis(15), "latency adds");
    assert_eq!(r.jitter, Duration::from_millis(7), "the largest jitter");
    assert_eq!(r.bandwidth_cap, Some(600), "the smallest cap");
    // an override drops the lower layer
    b.set(
        ChaosKey::Edge(0),
        ChaosLayer {
            replace: Some(true),
            ..layer(5, 7, 600)
        },
    );
    let r = compile_layers(&[&a, &b]);
    assert_eq!(r.link(0).unwrap().latency_add, Duration::from_millis(5));
    assert_eq!(a.entry_count(), 1);
    assert!(!a.is_empty());
}
