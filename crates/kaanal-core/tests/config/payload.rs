//! `bandwidth` on an edge and `per_kb` on a node: what a payload costs.

use kaanal_core::{Config, Dist, Topology};
use std::time::Duration;

fn load(nodes: &str, edges: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}edges:\n{edges}"))
        .map_err(|e| e.to_string())
}

const NODES: &str = "  app: { type: service }\n  db: { type: database, flavor: postgres }\n";

#[test]
fn bandwidth_must_be_positive() {
    let err = load(NODES, "  - { from: app, to: db, bandwidth: 0 }\n").unwrap_err();
    assert!(err.contains("bandwidth must be at least 1"), "{err}");
    assert!(err.starts_with("6:"), "positioned on its line: {err}");
    let c = load(NODES, "  - { from: app, to: db, bandwidth: 1000000 }\n").unwrap();
    assert_eq!(c.edges[0].bandwidth, Some(1_000_000));
}

#[test]
fn per_kb_parses_as_a_duration_like_proc_on_every_kind_but_a_cache() {
    let kinds = [
        ("gateway", ""),
        ("service", ""),
        ("monolith", ", flavor: mysql"),
        ("database", ", flavor: postgres"),
        ("external-api", ""),
        ("auth", ""),
    ];
    for (kind, extra) in kinds {
        let src = format!("  n: {{ type: {kind}{extra}, per_kb: 50us }}\n");
        let c = load(&src, "").unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert!(c.nodes["n"].per_kb().is_some(), "{kind}");
        // a lone database, cache or external API is not a valid topology, so build the others
        if matches!(kind, "gateway" | "service" | "monolith" | "auth") {
            let t = Topology::build(&c).unwrap();
            assert_eq!(
                t.nodes[0].per_kb,
                Dist::Fixed(Duration::from_micros(50)),
                "{kind}"
            );
        }
    }
    let c = load("  n: { type: service, per_kb: { mean: 2ms } }\n", "").unwrap();
    assert_eq!(
        Topology::build(&c).unwrap().nodes[0].per_kb,
        Dist::Exp {
            mean: Duration::from_millis(2)
        }
    );
}

#[test]
fn payload_costs_is_false_by_default_and_true_with_either_field() {
    let plain = Topology::build(&load(NODES, "  - { from: app, to: db }\n").unwrap()).unwrap();
    assert!(!plain.payload_costs());
    assert_eq!(plain.nodes[0].per_kb, Dist::ZERO);
    assert_eq!(plain.edges[0].bandwidth, None);
    let link =
        Topology::build(&load(NODES, "  - { from: app, to: db, bandwidth: 5000 }\n").unwrap())
            .unwrap();
    assert!(link.payload_costs());
    let per =
        Topology::build(&load("  app: { type: service, per_kb: 1ms }\n", "").unwrap()).unwrap();
    assert!(per.payload_costs());
}

#[test]
fn edge_id_is_the_config_index() {
    let c = load(
        "  gw: { type: gateway }\n  app: { type: service }\n  db: { type: database, flavor: postgres }\n",
        "  - { from: gw, to: app }\n  - { from: app, to: db }\n",
    )
    .unwrap();
    let t = Topology::build(&c).unwrap();
    let id = |a: &str, b: &str| t.edge_id(t.id(a).unwrap(), t.id(b).unwrap());
    assert_eq!(
        (id("gw", "app"), id("app", "db"), id("gw", "db")),
        (Some(0), Some(1), None)
    );
}

#[test]
fn per_kb_on_a_cache_is_rejected() {
    let err = load("  c: { type: cache, flavor: redis, per_kb: 1ms }\n", "").unwrap_err();
    assert!(err.contains("a cache has no per_kb"), "{err}");
    assert!(err.starts_with("3:"), "positioned on its line: {err}");
}

#[test]
fn node_bandwidth_takes_one_number_or_out_and_in_and_must_be_positive() {
    let c = load("  app: { type: service, bandwidth: 125000 }\n", "").unwrap();
    assert_eq!(c.nodes["app"].bandwidth(), (Some(125_000), Some(125_000)));
    let c = load(
        "  app: { type: service, bandwidth: { out: 10, in: 20 } }\n",
        "",
    )
    .unwrap();
    assert_eq!(c.nodes["app"].bandwidth(), (Some(10), Some(20)));
    let c = load("  app: { type: service, bandwidth: { out: 10 } }\n", "").unwrap();
    assert_eq!(c.nodes["app"].bandwidth(), (Some(10), None));
    let t = Topology::build(&c).unwrap();
    assert_eq!(t.nodes[0].bandwidth, (Some(10), None));
    assert!(
        t.payload_costs(),
        "a node bandwidth makes payload size matter"
    );
    let err = load("  app: { type: service, bandwidth: 0 }\n", "").unwrap_err();
    assert!(err.contains("bandwidth must be at least 1"), "{err}");
    let err = load("  app: { type: service, bandwidth: { out: 0 } }\n", "").unwrap_err();
    assert!(err.contains("bandwidth must be at least 1"), "{err}");
    let err = load("  c: { type: cache, bandwidth: 5 }\n", "").unwrap_err();
    assert!(err.contains("bandwidth"), "a cache has none: {err}");
}
