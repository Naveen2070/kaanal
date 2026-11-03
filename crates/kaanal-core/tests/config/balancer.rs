//! The `load-balancer` node: its fields, who may call it and whom it may call (RULES 34, 50).

use kaanal_core::{Balance, Config, NodeKind, Topology};

fn load(nodes: &str, edges: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}edges:\n{edges}"))
        .map_err(|e| e.to_string())
}

const TIER: &str = "  gw: { type: gateway }\n  lb: { type: load-balancer, proc: 200us, concurrency: 100 }\n  a: { type: service }\n  b: { type: service }\n";
const WIRES: &str = "  - { from: gw, to: lb }\n  - { from: lb, to: a }\n  - { from: lb, to: b }\n";

#[test]
fn a_balancer_builds_in_front_of_two_services() {
    let c = load(TIER, WIRES).unwrap();
    assert_eq!(c.nodes["lb"].kind(), NodeKind::LoadBalancer);
    assert_eq!(c.nodes["lb"].kind().as_str(), "load-balancer");
    let t = Topology::build(&c).unwrap();
    let lb = &t.nodes[t.id("lb").unwrap()];
    assert_eq!(lb.spread, Balance::RoundRobin, "round-robin unless it says");
    assert!(NodeKind::ALL.contains(&NodeKind::LoadBalancer));
}

#[test]
fn spread_takes_the_balance_forms_and_weighted_needs_one_weight_per_wire() {
    let nodes = |spread: &str| TIER.replace("concurrency: 100", &format!("spread: {spread}"));
    let c = load(&nodes("random"), WIRES).unwrap();
    assert_eq!(c.nodes["lb"].spread(), Some(&Balance::Random));
    let c = load(&nodes("{ weighted: [3, 1] }"), WIRES).unwrap();
    assert_eq!(c.nodes["lb"].spread(), Some(&Balance::Weighted(vec![3, 1])));
    let err = load(&nodes("{ weighted: [3, 1, 1] }"), WIRES).unwrap_err();
    assert!(err.contains("3 weights for 2 wires out of it"), "{err}");
    let err = load(&nodes("{ weighted: [0, 0] }"), WIRES).unwrap_err();
    assert!(err.contains("at least one weight above 0"), "{err}");
}

#[test]
fn who_may_call_a_balancer_and_whom_it_may_call() {
    // callers: gateway, service, monolith, auth, another balancer
    let nodes = "  gw: { type: gateway }\n  s: { type: service }\n  lb: { type: load-balancer }\n  lb2: { type: load-balancer }\n  d: { type: database, flavor: postgres }\n";
    let edges = "  - { from: gw, to: s }\n  - { from: s, to: lb }\n  - { from: lb, to: lb2 }\n  - { from: lb2, to: d }\n";
    Topology::build(&load(nodes, edges).unwrap()).unwrap();
    // a balancer may not call a cache or the gateway
    let cache = "  gw: { type: gateway }\n  lb: { type: load-balancer }\n  c: { type: cache, flavor: redis }\n  s: { type: service }\n";
    let err = Topology::build(
        &load(
            cache,
            "  - { from: gw, to: s }\n  - { from: s, to: lb }\n  - { from: lb, to: c }\n",
        )
        .unwrap(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("cannot call cache"), "{err}");
}

#[test]
fn an_edge_out_of_a_balancer_takes_no_prob() {
    let err = load(
        TIER,
        "  - { from: gw, to: lb }\n  - { from: lb, to: a, prob: 0.5 }\n  - { from: lb, to: b }\n",
    )
    .unwrap_err();
    assert!(
        err.contains("a load balancer picks one wire by its spread"),
        "{err}"
    );
}

#[test]
fn spread_belongs_to_the_balancer_only() {
    let err = load("  s: { type: service, spread: random }\n", "").unwrap_err();
    assert!(err.contains("spread"), "{err}");
}
