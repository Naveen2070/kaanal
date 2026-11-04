//! Links in the simulator: payload sizes make link hops, equal to `execute` at zero load. The
//! links are fast (a transfer is microseconds), because a link is one FIFO server: two requests
//! that overlap on it really queue, which is not zero load.

use crate::common::*;
use crate::shared::compiled;
use kaanal_core::{ChainResult, ChaosKey, ChaosLayer, ChaosState, Config, HopKind};

/// Two bandwidth edges and a `per_kb` node, behind a gateway.
const LINKS: &str = "version: 1\nheader_bytes: 0\nseed: 31\nnodes:\n  gw: { type: gateway, proc: { mean: 1ms } }\n  app: { type: service, proc: { mean: 2ms }, per_kb: 100us }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: app, net: 1ms, bandwidth: 1000000000 }\n  - { from: app, to: db, net: { mean: 1ms }, bandwidth: 500000000 }\n";

#[test]
fn link_chain_matches_execute() {
    let seed = Config::from_yaml(LINKS).unwrap().seed;
    let (t, e) = compiled(LINKS, &APP_ROUTES);
    let mut sc = zero_load_config(&["GET /users/{id}", "POST /orders"], seed);
    for (s, (req, resp)) in sc.streams.iter_mut().zip([(2_000, 10_000), (50_000, 0)]) {
        s.req_bytes = req;
        s.resp_bytes = resp;
    }
    // short, so two transfers meeting on a link (which queue, correctly) is out of the picture
    sc.duration = std::time::Duration::from_secs(300);
    let rep = kaanal_sim::run(&t, &e, &ChaosState::default(), &sc).unwrap();
    compare_with_execute(&t, &e, &ChaosState::default().compile(), &rep, &sc, "links");
    let kinds = |resp: bool| {
        rep.requests
            .iter()
            .flat_map(|r| r.hops.iter())
            .filter(|(h, _)| matches!(h.kind, HopKind::Link { resp: x, .. } if x == resp))
            .count()
    };
    assert!(kinds(false) > 100, "request transfers were planned");
    assert!(kinds(true) > 100, "response transfers were planned");
}

fn on_edge(layer: ChaosLayer) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(ChaosKey::Edge(1), layer);
    s
}

/// Runs the two routes (with payloads) and asserts every record equals `execute`.
fn equal_under(src: &str, chaos: &ChaosState, label: &str) -> kaanal_sim::SimReport {
    let seed = Config::from_yaml(src).unwrap().seed;
    let (t, e) = compiled(src, &APP_ROUTES);
    let mut sc = zero_load_config(&["GET /users/{id}", "POST /orders"], seed);
    for (s, bytes) in sc.streams.iter_mut().zip([2_000, 50_000]) {
        s.req_bytes = bytes;
    }
    sc.duration = std::time::Duration::from_secs(300);
    let rep = kaanal_sim::run(&t, &e, chaos, &sc).unwrap();
    compare_with_execute(&t, &e, &chaos.compile(), &rep, &sc, label);
    rep
}

#[test]
fn link_chaos_plan_matches_the_simulator() {
    let chaos = on_edge(ChaosLayer {
        latency_add: Some(std::time::Duration::from_millis(15)),
        jitter: Some(std::time::Duration::from_millis(5)),
        bandwidth_cap: Some(100_000_000),
        error_rate: Some(0.2),
        ..ChaosLayer::default()
    });
    let rep = equal_under(LINKS, &chaos, "link chaos");
    let failed = rep
        .requests
        .iter()
        .filter(|r| r.result != ChainResult::Ok)
        .count();
    assert!(
        failed > 20 && failed < rep.requests.len(),
        "some drops: {failed}"
    );
}

#[test]
fn partition_under_a_span_cancels_like_a_hang() {
    // db has a 200 ms timeout: the partition waits that long, then 504, in the plan and the engine
    let src = LINKS.replace("flavor: postgres }", "flavor: postgres, timeout: 200ms }");
    let down = ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    };
    let rep = equal_under(&src, &on_edge(down), "partition");
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Err { status: 504, .. }))
    );
    let (t, _) = compiled(&src, &APP_ROUTES);
    assert_eq!(
        rep.nodes[t.id("db").unwrap()].timed_out,
        0,
        "the planned failure wins the tie"
    );
}
